use crate::multiplexer::UdtMultiplexer;
use crate::packet::UdtPacket;
use crate::socket::{SocketId, UdtSocket, UdtStatus};
use crate::udt::{SocketRef, Udt};
use nix::sys::socket::{
    recvmmsg, AddressFamily, MsgFlags, RecvMmsgData, SockaddrIn, SockaddrIn6, SockaddrLike,
    SockaddrStorage,
};
use std::collections::{BTreeMap, VecDeque};
use std::io::IoSliceMut;
use std::net::SocketAddr;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex, Weak};
use tokio::io::{Error, ErrorKind, Interest, Result};
use tokio::net::UdpSocket;
use tokio::time::{Duration, Instant};
use tokio_timerfd::sleep;

const TIMERS_CHECK_INTERVAL: Duration = Duration::from_millis(100);
const UDP_RCV_TIMEOUT: Duration = Duration::from_micros(10);

#[derive(Debug)]
pub(crate) struct UdtRcvQueue {
    sockets: Mutex<VecDeque<(Instant, SocketId)>>,
    mss: u32,
    channel: Arc<UdpSocket>,
    multiplexer: Mutex<Weak<UdtMultiplexer>>,
    socket_refs: Mutex<BTreeMap<SocketId, Weak<UdtSocket>>>,
}

impl UdtRcvQueue {
    pub fn new(channel: Arc<UdpSocket>, mss: u32) -> Self {
        Self {
            sockets: Mutex::new(VecDeque::new()),
            mss,
            channel,
            multiplexer: Mutex::new(Weak::new()),
            socket_refs: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn push_back(&self, socket_id: SocketId) {
        self.sockets
            .lock()
            .unwrap()
            .push_back((Instant::now(), socket_id));
    }

    pub fn update(&self, socket_id: SocketId) {
        self.sockets
            .lock()
            .unwrap()
            .retain(|(_, id)| socket_id != *id);
        self.push_back(socket_id);
    }

    pub fn set_multiplexer(&self, mux: &Arc<UdtMultiplexer>) {
        *self.multiplexer.lock().unwrap() = Arc::downgrade(mux);
    }

    async fn get_socket(&self, socket_id: SocketId) -> Option<SocketRef> {
        let known_socket = self.socket_refs.lock().unwrap().get(&socket_id).cloned();
        if let Some(socket) = known_socket {
            socket.upgrade()
        } else if let Some(socket) = Udt::get().read().await.get_socket(socket_id) {
            self.socket_refs
                .lock()
                .unwrap()
                .insert(socket_id, Arc::downgrade(&socket));
            Some(socket)
        } else {
            None
        }
    }

    pub(crate) async fn worker(&self) -> Result<()> {
        let mut buf = vec![0_u8; self.mss as usize * 100];

        loop {
            let packets = {
                let bufs = buf.chunks_exact_mut(self.mss as usize);

                let mut recv_mesg_data: Vec<RecvMmsgData<_>> = bufs
                    .map(|b| RecvMmsgData {
                        iov: [IoSliceMut::new(&mut b[..])],
                        cmsg_buffer: None,
                    })
                    .collect();

                let msgs: Vec<_> = self
                    .channel
                    .try_io(Interest::READABLE, || {
                        let msgs = recvmmsg(
                            self.channel.as_raw_fd(),
                            &mut recv_mesg_data,
                            MsgFlags::MSG_DONTWAIT,
                            None,
                        )
                        .map_err(|err| {
                            if err == nix::errno::Errno::EWOULDBLOCK {
                                return Error::new(ErrorKind::WouldBlock, "recvmmsg would block");
                            }
                            Error::new(ErrorKind::Other, err)
                        })?
                        .iter()
                        .map(|msg| {
                            let addr: SockaddrStorage = msg.address.unwrap();
                            (msg.bytes, addr)
                        })
                        .collect();
                        Ok(msgs)
                    })
                    .unwrap_or_default();

                if !msgs.is_empty() {
                    msgs.into_iter()
                        .zip(buf.chunks_exact_mut(self.mss as usize))
                        .filter_map(|((nbytes, addr), buf)| {
                            let packet = UdtPacket::deserialize(&buf[..nbytes]).ok()?;
                            let addr: SocketAddr = match addr.family() {
                                Some(AddressFamily::Inet) => {
                                    Self::addr_v4_from_sockaddrin(*addr.as_sockaddr_in().unwrap())
                                        .into()
                                }
                                Some(AddressFamily::Inet6) => {
                                    Self::addr_v6_from_sockaddrin6(*addr.as_sockaddr_in6().unwrap())
                                        .into()
                                }
                                _ => unreachable!(),
                            };
                            Some((packet, addr))
                        })
                        .collect()
                } else {
                    tokio::select! {
                        _ = sleep(UDP_RCV_TIMEOUT) => (),
                        _ = self.channel.readable() => ()

                    };
                    vec![]
                }
            };

            for (packet, addr) in packets {
                let socket_id = packet.get_dest_socket_id();
                if socket_id == 0 {
                    if let Some(handshake) = packet.handshake() {
                        let mux = {
                            let lock = self.multiplexer.lock().unwrap();
                            lock.upgrade()
                        };
                        if let Some(mux) = mux {
                            let listener = mux.listener.read().await;
                            if let Some(listener) = &*listener {
                                listener.listen_on_handshake(addr, handshake).await?;
                            }
                        }
                    } else {
                        return Err(Error::new(
                            ErrorKind::InvalidData,
                            "received non-hanshake packet with socket 0",
                        ));
                    }
                } else {
                    // if !self.sockets.contains(&socket_id) {
                    //     eprintln!("socket {} not present in rcv_queue", socket_id);
                    //     continue;
                    // }

                    if let Some(socket) = self.get_socket(socket_id).await {
                        if socket.peer_addr() == Some(addr)
                            && ![UdtStatus::Broken, UdtStatus::Closed, UdtStatus::Closing]
                                .contains(&socket.status())
                        {
                            socket.process_packet(packet).await?;
                            socket.check_timers().await;
                            self.update(socket_id);
                        } else {
                            eprintln!("Ignoring packet {:?}", packet);
                        }
                    } else {
                        eprintln!("socket not found for socket_id {}", socket_id);
                        // TODO: implement rendezvous queue
                    }
                }
            }

            let to_check: Vec<_> = self
                .sockets
                .lock()
                .unwrap()
                .iter()
                .take_while(|(ts, _)| ts.elapsed() > TIMERS_CHECK_INTERVAL)
                .map(|(_, socket_id)| *socket_id)
                .collect();

            for socket_id in to_check {
                if let Some(socket) = self.get_socket(socket_id).await {
                    socket.check_timers().await;
                }
            }
        }
    }

    // TEMP: waiting for "nix" next release (> 0.24.1) to include these conversions
    fn addr_v4_from_sockaddrin(addr: SockaddrIn) -> std::net::SocketAddrV4 {
        std::net::SocketAddrV4::new(std::net::Ipv4Addr::from(addr.ip()), addr.port())
    }

    fn addr_v6_from_sockaddrin6(addr: SockaddrIn6) -> std::net::SocketAddrV6 {
        std::net::SocketAddrV6::new(
            addr.ip(),
            addr.port(),
            u32::from_be(addr.flowinfo()),
            u32::from_be(addr.scope_id()),
        )
    }
}

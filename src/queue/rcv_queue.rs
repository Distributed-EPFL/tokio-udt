use crate::multiplexer::UdtMultiplexer;
use crate::packet::UdtPacket;
use crate::socket::{SocketId, UdtSocket};
use crate::udt::{SocketRef, Udt, UDT_DEBUG};
use nix::sys::socket::{SockaddrIn, SockaddrIn6};
use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Weak};
use tokio::io::{Error, ErrorKind, Result};
use tokio::net::UdpSocket;
use tokio::time::{Duration, Instant};
use tokio_timerfd::sleep;

const TIMERS_CHECK_INTERVAL: Duration = Duration::from_millis(100);
const UDP_RCV_TIMEOUT: Duration = Duration::from_micros(30);

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

    fn update(&self, socket_id: SocketId) {
        let mut queue = self.sockets.lock().unwrap();
        queue.retain(|(_, id)| socket_id != *id);
        queue.push_back((Instant::now(), socket_id));
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

    #[cfg(target_os = "linux")]
    fn receive_packets(&self, buf: &mut [u8]) -> Result<Vec<(usize, SocketAddr)>> {
        use nix::sys::socket::{
            recvmmsg, AddressFamily, MsgFlags, RecvMmsgData, SockaddrLike, SockaddrStorage,
        };
        use std::io::IoSliceMut;
        use std::os::unix::io::AsRawFd;
        use tokio::io::Interest;
        let bufs = buf.chunks_exact_mut(self.mss as usize);
        let mut recv_mesg_data: Vec<RecvMmsgData<_>> = bufs
            .map(|b| RecvMmsgData {
                iov: [IoSliceMut::new(&mut b[..])],
                cmsg_buffer: None,
            })
            .collect();

        self.channel.try_io(Interest::READABLE, || {
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
                let socket_addr: SocketAddr = match addr.family() {
                    Some(AddressFamily::Inet) => {
                        Self::addr_v4_from_sockaddrin(*addr.as_sockaddr_in().unwrap()).into()
                    }
                    Some(AddressFamily::Inet6) => {
                        Self::addr_v6_from_sockaddrin6(*addr.as_sockaddr_in6().unwrap()).into()
                    }
                    _ => unreachable!(),
                };
                (msg.bytes, socket_addr)
            })
            .collect();
            Ok(msgs)
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn receive_packets(&self, buf: &mut [u8]) -> Result<Vec<(usize, SocketAddr)>> {
        let bufs = buf.chunks_exact_mut(self.mss as usize);
        let mut msgs = vec![];
        for mut buf in bufs {
            match self.channel.try_recv_from(&mut buf) {
                Ok(msg) => {
                    msgs.push(msg);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(msgs)
    }

    pub(crate) async fn worker(&self) -> Result<()> {
        let mut buf = vec![0_u8; self.mss as usize * 100];
        loop {
            let packets = {
                let msgs = self.receive_packets(&mut buf).unwrap_or_default();
                if !msgs.is_empty() {
                    let packets: Vec<_> = msgs
                        .into_iter()
                        .zip(buf.chunks_exact_mut(self.mss as usize))
                        .filter_map(|((nbytes, addr), buf)| {
                            let packet = UdtPacket::deserialize(&buf[..nbytes]).ok()?;

                            Some((packet, addr))
                        })
                        .collect();
                    Some(packets)
                } else {
                    tokio::select! {
                        _ = sleep(UDP_RCV_TIMEOUT) => (),
                        _ = self.channel.readable() => ()
                    };
                    None
                }
            };

            for (packet, addr) in packets.into_iter().flatten() {
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
                        if socket.peer_addr() == Some(addr) && socket.status().is_alive() {
                            socket.process_packet(packet).await?;
                            socket.check_timers().await;
                            self.update(socket_id);
                        } else if *UDT_DEBUG {
                            eprintln!("Ignoring packet {:?}", packet);
                        }
                    } else {
                        // TODO: implement rendezvous queue

                        if *UDT_DEBUG {
                            eprintln!("socket not found for socket_id {}", socket_id);
                            dbg!(packet);
                        }
                    }
                }
            }

            let to_check = {
                let mut to_check = vec![];
                let mut sockets = self.sockets.lock().unwrap();
                while sockets
                    .front()
                    .map(|(ts, _)| ts.elapsed() > TIMERS_CHECK_INTERVAL)
                    .unwrap_or(false)
                {
                    to_check.push(sockets.pop_front().unwrap().1);
                }
                to_check
            };

            for socket_id in to_check {
                if let Some(socket) = self.get_socket(socket_id).await {
                    if socket.status().is_alive() {
                        socket.check_timers().await;
                        self.update(socket_id);
                    }
                }
            }
        }
    }

    // TEMP: waiting for "nix" next release (> 0.24.2) to include these conversions
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

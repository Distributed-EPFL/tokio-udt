use crate::multiplexer::UdtMultiplexer;
use crate::packet::UdtPacket;
use crate::socket::{SocketId, UdtStatus};
use crate::udt::{SocketRef, Udt};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use tokio::io::{Error, ErrorKind, Result};
use tokio::net::UdpSocket;
use tokio::time::{Duration, Instant};
use tokio_timerfd::sleep;

const TIMERS_CHECK_INTERVAL: Duration = Duration::from_millis(100);
const UDP_RCV_TIMEOUT: Duration = Duration::from_micros(100);

#[derive(Debug)]
pub(crate) struct UdtRcvQueue {
    sockets: Mutex<VecDeque<(Instant, SocketId)>>,
    payload_size: u32,
    channel: Arc<UdpSocket>,
    multiplexer: Mutex<Weak<UdtMultiplexer>>,
    socket_refs: Mutex<BTreeMap<SocketId, SocketRef>>,
}

impl UdtRcvQueue {
    pub fn new(channel: Arc<UdpSocket>, payload_size: u32) -> Self {
        Self {
            sockets: Mutex::new(VecDeque::new()),
            payload_size,
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
            Some(socket)
        } else if let Some(socket) = Udt::get().read().await.get_socket(socket_id) {
            self.socket_refs
                .lock()
                .unwrap()
                .insert(socket_id, socket.clone());
            Some(socket)
        } else {
            None
        }
    }

    pub(crate) async fn worker(&self) -> Result<()> {
        let mut buf = vec![0_u8; self.payload_size as usize];
        loop {
            if let Some((size, addr)) = {
                let res = self.channel.try_recv_from(&mut buf).ok();
                if res.is_some() {
                    res
                } else {
                    tokio::select! {
                        r = self.channel.recv_from(&mut buf) => Some(r?),
                        _ = sleep(UDP_RCV_TIMEOUT) => None,
                    }
                }
            } {
                let packet = UdtPacket::deserialize(&buf[..size])?;
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
                if let Some(socket) = Udt::get().read().await.get_socket(socket_id) {
                    socket.check_timers().await;
                }
            }
        }
    }
}

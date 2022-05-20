use crate::multiplexer::UdtMultiplexer;
use crate::packet::UdtPacket;
use crate::socket::{SocketId, UdtStatus};
use crate::udt::Udt;
use std::collections::VecDeque;
use std::sync::{Arc, Weak};
use tokio::io::{Error, ErrorKind, Result};
use tokio::net::UdpSocket;
use tokio::sync::{Notify, RwLock};

#[derive(Debug)]
pub(crate) struct UdtRcvQueue<'a> {
    sockets: VecDeque<SocketId>,
    notify: Notify,
    // packets: Vec<UdtPacket>,
    payload_size: u32,
    channel: Arc<UdpSocket>,
    multiplexer: Weak<RwLock<UdtMultiplexer<'a>>>,
}

impl<'a> UdtRcvQueue<'a> {
    pub fn new(channel: Arc<UdpSocket>, payload_size: u32) -> Self {
        Self {
            sockets: VecDeque::new(),
            notify: Notify::new(),
            payload_size,
            channel,
            multiplexer: Weak::new(),
        }
    }

    pub fn push_back(&mut self, socket_id: SocketId) {
        self.sockets.push_back(socket_id);
    }

    pub fn set_multiplexer(&mut self, mux: &Arc<RwLock<UdtMultiplexer<'a>>>) {
        self.multiplexer = Arc::downgrade(mux);
    }

    pub(crate) async fn worker(&self) -> Result<()> {
        loop {
            // TODO: drop packet if no space in packets

            let mut buf = Vec::with_capacity(self.payload_size as usize);
            let (_size, addr) = self.channel.recv_from(&mut buf).await?;
            let packet = UdtPacket::deserialize(buf)?;
            let socket_id = packet.get_dest_socket_id();
            if socket_id == 0 {
                if let Some(handshake) = packet.handshake() {
                    if let Some(lock) = self.multiplexer.upgrade() {
                        let mux = lock.read().await;
                        if let Some(listener) = &mux.listener {
                            listener
                                .read()
                                .await
                                .listen_on_handshake(addr, handshake)
                                .await?;
                        }
                    }
                } else {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "received non-hanshake packet with socket 0",
                    ));
                }
            } else {
                if let Some(socket) = Udt::get().read().await.get_socket(socket_id).await {
                    let mut socket = socket.write().await;
                    if socket.peer_addr == Some(addr) && socket.status == UdtStatus::Connected {
                        socket.process_packet(packet).await?;
                    }
                } else {
                    eprintln!("socket not found for socket_id {}", socket_id);
                    // TODO: implement rendezvous queue
                }
            }
        }
    }
}

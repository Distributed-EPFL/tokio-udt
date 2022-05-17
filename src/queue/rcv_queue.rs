use crate::multiplexer::UdtMultiplexer;
use crate::packet::UdtPacket;
use crate::socket::UdtSocket;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use tokio::io::{Error, ErrorKind, Result};
use tokio::net::UdpSocket;
use tokio::sync::Notify;

#[derive(Debug)]
pub(crate) struct UdtRcvQueue {
    sockets: VecDeque<Rc<RefCell<UdtSocket>>>,
    notify: Notify,
    packets: Vec<UdtPacket>,
    payload_size: u32,
    channel: Rc<UdpSocket>,
    multiplexer: Rc<RefCell<UdtMultiplexer>>,
}

impl UdtRcvQueue {
    pub fn new(
        channel: Rc<UdpSocket>,
        payload_size: u32,
        mux: Rc<RefCell<UdtMultiplexer>>,
    ) -> Self {
        Self {
            sockets: VecDeque::new(),
            notify: Notify::new(),
            packets: vec![],
            payload_size,
            channel,
            multiplexer: mux,
        }
    }

    pub fn push_back(&mut self, socket: Rc<RefCell<UdtSocket>>) {
        self.sockets.push_back(socket);
    }

    async fn worker(&mut self) -> Result<()> {
        loop {
            // TODO: drop packet if no space in packets

            let mut buf = Vec::with_capacity(self.payload_size as usize);
            let (_size, addr) = self.channel.recv_from(&mut buf).await?;
            let packet = UdtPacket::deserialize(buf)?;
            let socket_id = packet.get_dest_socket_id();
            if socket_id == 0 {
                if let Some(handshake) = packet.handshake() {
                    if let Some(ref listener) = self.multiplexer.borrow().listener {
                        listener.borrow().listen_on_handshake(addr, handshake);
                    }
                } else {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "received non-hanshake packet with socket 0",
                    ));
                }
            }
        }
    }
}
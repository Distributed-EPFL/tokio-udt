use super::configuration::UdtConfiguration;
use super::packet::UdtPacket;
use super::socket::UdtSocket;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Result;
use std::net::SocketAddr;
use std::rc::Rc;
use tokio::net::UdpSocket;

pub type MultiplexerId = u32;

#[derive(Debug)]
pub(crate) struct UdtMultiplexer {
    pub id: MultiplexerId,
    pub port: u16,
    pub channel: UdpSocket,
    pub reusable: bool,
    pub mss: u32,

    pub snd_queue: VecDeque<Rc<RefCell<UdtSocket>>>,
    pub rcv_queue: VecDeque<Rc<RefCell<UdtSocket>>>,
}

impl UdtMultiplexer {
    pub async fn bind(
        id: MultiplexerId,
        bind_addr: SocketAddr,
        config: &UdtConfiguration,
    ) -> Result<Self> {
        let port = bind_addr.port();
        let channel = UdpSocket::bind(bind_addr).await?;
        // TODO: set sndBufSize and rcvBufSize

        // TODO: init snd_queue and rcv_queue

        Ok(Self {
            id,
            port,
            reusable: config.reuse_addr,
            mss: config.mss,
            channel,
            snd_queue: VecDeque::new(),
            rcv_queue: VecDeque::new(),
        })
    }

    pub async fn send_to(&self, addr: &SocketAddr, packet: UdtPacket) -> Result<usize> {
        self.channel.send_to(&packet.serialize(), addr).await
    }

    pub fn get_local_addr(&self) -> SocketAddr {
        self.channel
            .local_addr()
            .expect("failed to retrieve udp local addr")
    }
}

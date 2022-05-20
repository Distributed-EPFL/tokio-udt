use super::configuration::UdtConfiguration;
use super::packet::UdtPacket;
use crate::queue::{UdtRcvQueue, UdtSndQueue};
use crate::udt::SocketRef;
use std::io::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

pub type MultiplexerId = u32;

#[derive(Debug)]
pub struct UdtMultiplexer<'a> {
    pub id: MultiplexerId,
    pub port: u16,
    pub channel: Arc<UdpSocket>,
    pub reusable: bool,
    pub mss: u32,

    pub(crate) snd_queue: UdtSndQueue,
    pub(crate) rcv_queue: UdtRcvQueue<'a>,
    pub listener: Option<SocketRef<'a>>,
}

impl<'a> UdtMultiplexer<'a> {
    pub async fn bind(
        id: MultiplexerId,
        bind_addr: SocketAddr,
        config: &UdtConfiguration,
    ) -> Result<(MultiplexerId, Arc<RwLock<UdtMultiplexer<'a>>>)> {
        let port = bind_addr.port();
        let channel = Arc::new(UdpSocket::bind(bind_addr).await?);
        // TODO: set UDP sndBufSize and rcvBufSize ?

        let mux = Self {
            id,
            port,
            reusable: config.reuse_addr,
            mss: config.mss,
            channel: channel.clone(),
            snd_queue: UdtSndQueue::new(),
            rcv_queue: UdtRcvQueue::new(channel, config.mss),
            listener: None,
        };

        let lock = Arc::new(RwLock::new(mux));
        let lock2 = lock.clone();
        let mut mux = lock.write_owned().await;
        mux.rcv_queue.set_multiplexer(&lock2);
        Ok((id, lock2))
    }

    pub(crate) async fn send_to(&self, addr: &SocketAddr, packet: UdtPacket) -> Result<usize> {
        self.channel.send_to(&packet.serialize(), addr).await
    }

    pub fn get_local_addr(&self) -> SocketAddr {
        self.channel
            .local_addr()
            .expect("failed to retrieve udp local addr")
    }

    pub fn run(&'static mut self) {
        tokio::spawn(async { self.rcv_queue.worker().await });
        tokio::spawn(async { self.snd_queue.worker().await });
    }
}

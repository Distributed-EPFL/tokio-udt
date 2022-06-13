use super::configuration::UdtConfiguration;
use super::packet::UdtPacket;
use crate::queue::{UdtRcvQueue, UdtSndQueue};
use crate::udt::SocketRef;
use socket2::{Domain, Socket, Type};
use std::io::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

pub type MultiplexerId = u32;

#[derive(Debug)]
pub struct UdtMultiplexer {
    pub id: MultiplexerId,
    pub port: u16,
    pub channel: Arc<UdpSocket>,
    pub reusable: bool,
    pub mss: u32,

    pub(crate) snd_queue: UdtSndQueue,
    pub(crate) rcv_queue: UdtRcvQueue,
    pub listener: RwLock<Option<SocketRef>>,
}

impl UdtMultiplexer {
    async fn new_udp_socket(
        config: &UdtConfiguration,
        bind_addr: Option<SocketAddr>,
    ) -> Result<UdpSocket> {
        let domain = if bind_addr.map(|addr| addr.ip().is_ipv6()).unwrap_or(false) {
            Domain::IPV6
        } else {
            Domain::IPV4
        };
        tokio::task::spawn_blocking({
            let config = config.clone();
            move || {
                let socket = Socket::new(domain, Type::DGRAM, None)?;
                socket.set_recv_buffer_size(config.udp_rcv_buf_size)?;
                socket.set_send_buffer_size(config.udp_snd_buf_size)?;
                socket.set_nonblocking(true)?;
                if let Some(addr) = bind_addr {
                    socket.bind(&addr.into())?;
                }
                UdpSocket::from_std(socket.into())
            }
        })
        .await?
    }

    pub(crate) async fn new(
        id: MultiplexerId,
        config: &UdtConfiguration,
    ) -> Result<(MultiplexerId, Arc<UdtMultiplexer>)> {
        let udp_socket = Self::new_udp_socket(config, None).await?;
        let channel = Arc::new(udp_socket);
        let port = channel.local_addr()?.port();
        let mux = Self {
            id,
            port,
            reusable: config.reuse_addr,
            mss: config.mss,
            channel: channel.clone(),
            snd_queue: UdtSndQueue::new(),
            rcv_queue: UdtRcvQueue::new(channel, config.mss),
            listener: RwLock::new(None),
        };

        let mux = Arc::new(mux);
        mux.rcv_queue.set_multiplexer(&mux);
        Ok((id, mux))
    }

    pub(crate) async fn bind(
        id: MultiplexerId,
        bind_addr: SocketAddr,
        config: &UdtConfiguration,
    ) -> Result<(MultiplexerId, Arc<UdtMultiplexer>)> {
        let port = bind_addr.port();
        let udp_socket = Self::new_udp_socket(config, Some(bind_addr)).await?;
        let channel = Arc::new(udp_socket);
        let mux = Self {
            id,
            port,
            reusable: config.reuse_addr,
            mss: config.mss,
            channel: channel.clone(),
            snd_queue: UdtSndQueue::new(),
            rcv_queue: UdtRcvQueue::new(channel, config.mss),
            listener: RwLock::new(None),
        };

        let mux = Arc::new(mux);
        mux.rcv_queue.set_multiplexer(&mux);
        Ok((id, mux))
    }

    pub(crate) async fn send_to(&self, addr: &SocketAddr, packet: UdtPacket) -> Result<usize> {
        self.channel.send_to(&packet.serialize(), addr).await
    }

    // pub fn get_local_addr(&self) -> SocketAddr {
    //     self.channel
    //         .local_addr()
    //         .expect("failed to retrieve udp local addr")
    // }

    pub fn run(mux: Arc<Self>) {
        let mux2 = mux.clone();
        tokio::spawn(async move { mux.rcv_queue.worker().await.unwrap() });
        tokio::spawn(async move { mux2.snd_queue.worker().await.unwrap() });
    }
}

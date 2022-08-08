use super::configuration::UdtConfiguration;
use super::packet::UdtPacket;
use crate::queue::{UdtRcvQueue, UdtSndQueue};
use crate::udt::SocketRef;
use socket2::{Domain, Socket, Type};
use std::io::Result;
use std::net::{Ipv4Addr, SocketAddr};
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
        let bind_addr = bind_addr.unwrap_or_else(|| (Ipv4Addr::UNSPECIFIED, 0).into());
        let domain = if bind_addr.is_ipv6() {
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
                socket.set_reuse_port(config.udp_reuse_port)?;
                socket.set_nonblocking(true)?;
                socket.bind(&bind_addr.into())?;
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
            reusable: config.reuse_mux,
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
        let udp_socket = Self::new_udp_socket(config, Some(bind_addr)).await?;
        let port = udp_socket.local_addr()?.port();

        let channel = Arc::new(udp_socket);
        let mux = Self {
            id,
            port,
            reusable: config.reuse_mux,
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

    #[cfg(target_os = "linux")]
    pub(crate) async fn send_mmsg_to(
        &self,
        addr: &SocketAddr,
        packets: impl Iterator<Item = UdtPacket>,
    ) -> Result<usize> {
        use nix::sys::socket::{sendmmsg, MsgFlags, SendMmsgData, SockaddrStorage};
        use std::io::IoSlice;
        use std::os::unix::io::AsRawFd;
        use tokio::io::{Error, ErrorKind, Interest};
        let data: Vec<_> = packets.map(|p| p.serialize()).collect();
        let dest: SockaddrStorage = (*addr).into();
        let buffers: Vec<SendMmsgData<_, _, _>> = data
            .iter()
            .map(|packet| SendMmsgData {
                iov: [IoSlice::new(packet)],
                cmsgs: &[],
                addr: Some(dest),
                _lt: Default::default(),
            })
            .collect();
        self.channel.writable().await?;
        let sent = self
            .channel
            .try_io(Interest::WRITABLE, || {
                let sock_fd = self.channel.as_raw_fd();
                let sent: usize = sendmmsg(sock_fd, &buffers, MsgFlags::MSG_DONTWAIT)
                    .map_err(|err| {
                        if err == nix::errno::Errno::EWOULDBLOCK {
                            return Error::new(ErrorKind::WouldBlock, "sendmmsg would block");
                        }
                        Error::new(ErrorKind::Other, err)
                    })?
                    .into_iter()
                    .sum();
                Ok(sent)
            })
            .unwrap_or(0);
        Ok(sent)
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) async fn send_mmsg_to(
        &self,
        addr: &SocketAddr,
        packets: impl Iterator<Item = UdtPacket>,
    ) -> Result<usize> {
        self.channel.writable().await?;
        let mut sent = 0;
        for data in packets.map(|p| p.serialize()) {
            sent += self.channel.send_to(&data, addr).await?;
        }
        Ok(sent)
    }

    // pub fn get_local_addr(&self) -> SocketAddr {
    //     self.channel
    //         .local_addr()
    //         .expect("failed to retrieve udp local addr")
    // }

    pub fn run(mux: Arc<Self>) {
        tokio::spawn({
            let mux = mux.clone();
            async move { mux.rcv_queue.worker().await.unwrap() }
        });
        tokio::spawn(async move { mux.snd_queue.worker().await.unwrap() });
    }
}

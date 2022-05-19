use super::configuration::UdtConfiguration;
use super::packet::UdtPacket;
use crate::queue::{UdtRcvQueue, UdtSndQueue};
use crate::udt::SocketRef;
use std::cell::RefCell;
use std::io::Result;
use std::net::SocketAddr;
use std::rc::Rc;
use tokio::net::UdpSocket;
use tokio::task::LocalSet;

pub type MultiplexerId = u32;

#[derive(Debug)]
pub struct UdtMultiplexer {
    pub id: MultiplexerId,
    pub port: u16,
    pub channel: Rc<UdpSocket>,
    pub reusable: bool,
    pub mss: u32,

    pub(crate) snd_queue: UdtSndQueue,
    pub(crate) rcv_queue: Option<UdtRcvQueue>,
    pub listener: Option<SocketRef>,
}

impl UdtMultiplexer {
    pub async fn bind(
        id: MultiplexerId,
        bind_addr: SocketAddr,
        config: &UdtConfiguration,
    ) -> Result<Rc<RefCell<Self>>> {
        let port = bind_addr.port();
        let channel = Rc::new(UdpSocket::bind(bind_addr).await?);
        // TODO: set UDP sndBufSize and rcvBufSize ?

        let mux = Self {
            id,
            port,
            reusable: config.reuse_addr,
            mss: config.mss,
            snd_queue: UdtSndQueue::new(),
            rcv_queue: None,
            channel: channel.clone(),
            listener: None,
        };

        let mux_rc = Rc::new(RefCell::new(mux));
        let rcv_queue = UdtRcvQueue::new(channel, config.mss, mux_rc.clone());

        // let local_set = LocalSet::new();
        // {
        //     let rcv_queue = rcv_queue.clone();
        //     local_set.spawn_local(async move {
        //         rcv_queue.worker();
        //     });

        //     local_set.spawn_local(async move {
        //         snd_queue.worker();
        //     });
        // }
        {
            let mut mux = mux_rc.borrow_mut();
            mux.rcv_queue = Some(rcv_queue);
        }
        Ok(mux_rc)
    }

    pub(crate) async fn send_to(&self, addr: &SocketAddr, packet: UdtPacket) -> Result<usize> {
        self.channel.send_to(&packet.serialize(), addr).await
    }

    pub fn get_local_addr(&self) -> SocketAddr {
        self.channel
            .local_addr()
            .expect("failed to retrieve udp local addr")
    }

    // pub fn run(&mut self) {
    //     let local_set = LocalSet::new();
    //     local_set.spawn_local(async {
    //         if let Some(rcv) = self.rcv_queue {
    //             rcv.worker();
    //         }
    //     });
    // }
}

use crate::configuration::UdtConfiguration;
use crate::control_packet::{HandShakeInfo, UdtControlPacket};
use crate::flow::UdtFlow;
use crate::multiplexer::UdtMultiplexer;
use crate::packet::UdtPacket;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::Result;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use tokio::time::Instant;

pub type SocketId = u32;

#[derive(Debug, Copy, Clone)]
pub(crate) enum SocketType {
    Stream = 0,
    Datagram = 1,
}

#[derive(Debug)]
pub(crate) struct UdtSocket {
    pub socket_id: SocketId,
    pub status: UdtStatus,
    socket_type: SocketType,
    listen_socket: Option<SocketId>,
    pub peer_addr: Option<SocketAddr>,
    pub peer_socket_id: Option<SocketId>,
    pub initial_seq_number: u32,

    pub queued_sockets: BTreeSet<SocketId>,
    pub accepted_socket: BTreeSet<SocketId>,
    pub backlog_size: usize,
    pub multiplexer: Option<Rc<RefCell<UdtMultiplexer>>>,
    pub configuration: UdtConfiguration,

    snd_buffer: Vec<u8>,
    rcv_buffer: Vec<u8>,
    flow: UdtFlow,
    self_ip: Option<IpAddr>,

    start_time: Instant,
}

impl UdtSocket {
    pub fn new(socket_id: SocketId, socket_type: SocketType) -> Self {
        Self {
            socket_id,
            socket_type,
            status: UdtStatus::Init,
            initial_seq_number: rand::random(),
            peer_addr: None,
            peer_socket_id: None,
            listen_socket: None,
            queued_sockets: BTreeSet::new(),
            accepted_socket: BTreeSet::new(),
            backlog_size: 0,
            multiplexer: None,
            configuration: UdtConfiguration::default(),
            snd_buffer: vec![],
            rcv_buffer: vec![],
            flow: UdtFlow::default(),
            self_ip: None,
            start_time: Instant::now(),
        }
    }

    pub fn with_peer(mut self, peer: SocketAddr, peer_socket_id: SocketId) -> Self {
        self.peer_addr = Some(peer);
        self.peer_socket_id = Some(peer_socket_id);
        self
    }

    pub fn with_listen_socket(mut self, listen_socket: &UdtSocket) -> Self {
        self.listen_socket = Some(listen_socket.socket_id);
        self.multiplexer = listen_socket.multiplexer.clone();
        self
    }

    pub fn with_initial_seq_number(mut self, isn: u32) -> Self {
        self.initial_seq_number = isn;
        self
    }

    pub fn open(&mut self) {
        // TODO: init packet_size, payload_size, etc.
        self.status = UdtStatus::Opened;
    }

    pub async fn connect_on_handshake(
        mut self,
        peer: SocketAddr,
        mut hs: HandShakeInfo,
    ) -> Result<Rc<RefCell<Self>>> {
        if hs.max_packet_size > self.configuration.mss {
            hs.max_packet_size = self.configuration.mss;
        } else {
            self.configuration.mss = hs.max_packet_size;
        }

        self.flow.window_size = hs.max_window_size;
        hs.max_window_size = std::cmp::min(
            self.configuration.rcv_buf_size,
            self.configuration.flight_flag_size,
        );
        self.set_self_ip(hs.ip_address);
        hs.ip_address = peer.ip();

        // TODO: use network information cache to set RTT, bandwidth, etc.
        // TODO: init congestion control

        self.peer_addr = Some(peer);
        self.status = UdtStatus::Connected;

        let packet = UdtControlPacket::new_handshake(
            hs,
            self.peer_socket_id.expect("peer_socket_id not defined"),
        );
        let socket = Rc::new(RefCell::new(self));

        if let Some(mut mux) = socket.borrow().multiplexer.as_ref().map(|m| m.borrow_mut()) {
            if let Some(ref mut rcv_queue) = mux.rcv_queue {
                rcv_queue.push_back(socket.clone());
            }
            mux.send_to(&peer, UdtPacket::Control(packet)).await?;
        }
        Ok(socket)
    }

    pub fn set_multiplexer(&mut self, mux: Rc<RefCell<UdtMultiplexer>>) {
        self.multiplexer = Some(mux);
    }

    pub fn set_self_ip(&mut self, ip: IpAddr) {
        self.self_ip = Some(ip);
    }

    pub fn self_addr(&self) -> Option<SocketAddr> {
        self.multiplexer
            .as_ref()
            .map(|m| m.borrow().get_local_addr())
    }

    pub fn send_next_packet(&mut self) -> Result<()> {
        !unimplemented!()
    }

    pub fn listen_on_handshake(&self, addr: SocketAddr, handshake: &HandShakeInfo) {
        let timestamp = self.start_time.elapsed().as_secs() / 60; // secret changes every one minute
        let host = addr.ip();
        let port = addr.port();
        let cookie = Sha256::digest(format!("{host}:{port}:{timestamp}").as_bytes());
        if handshake.connection_type == 1 {
            // Regular connection, respond to handshake
        }
    }
}

impl Ord for UdtSocket {
    fn cmp(&self, other: &Self) -> Ordering {
        self.socket_id.cmp(&other.socket_id)
    }
}

impl PartialOrd for UdtSocket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for UdtSocket {
    fn eq(&self, other: &Self) -> bool {
        self.socket_id == other.socket_id
    }
}

impl Eq for UdtSocket {}

#[derive(Debug, PartialEq)]
pub(crate) enum UdtStatus {
    Init,
    Opened,
    Listening,
    Connecting,
    Connected,
    Broken,
    Closing,
    Closed,
}

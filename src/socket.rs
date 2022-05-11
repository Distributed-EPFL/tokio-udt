use crate::configuration::UdtConfiguration;
use crate::control_packet::HandShakeInfo;
use crate::flow::UdtFlow;
use crate::multiplexer::UdtMultiplexer;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;

pub type SocketId = usize;

#[derive(Debug)]
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
    pub self_addr: Option<SocketAddr>,
    pub peer_addr: Option<SocketAddr>,
    pub peer_socket_id: Option<SocketId>,
    initial_seq_number: u32,

    pub queued_sockets: BTreeSet<SocketId>,
    pub accepted_socket: BTreeSet<SocketId>,
    pub backlog_size: usize,
    multiplexer: Option<Rc<RefCell<UdtMultiplexer>>>,
    configuration: UdtConfiguration,
    flow: UdtFlow,
    self_ip: Option<IpAddr>,
}

impl UdtSocket {
    pub fn new(socket_id: SocketId, socket_type: SocketType) -> Self {
        Self {
            socket_id,
            socket_type,
            status: UdtStatus::Init,
            initial_seq_number: rand::random(),
            self_addr: None,
            peer_addr: None,
            peer_socket_id: None,
            listen_socket: None,
            queued_sockets: BTreeSet::new(),
            accepted_socket: BTreeSet::new(),
            backlog_size: 0,
            multiplexer: None,
            configuration: UdtConfiguration::default(),
            flow: UdtFlow::default(),
            self_ip: None,
        }
    }

    pub fn with_peer(mut self, peer: SocketAddr, peer_socket_id: SocketId) -> Self {
        self.peer_addr = Some(peer);
        self.peer_socket_id = Some(peer_socket_id);
        self
    }

    pub fn with_listen_socket(mut self, listen_socket: SocketId) -> Self {
        self.listen_socket = Some(listen_socket);
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

    pub fn connect_on_handshake(
        mut self,
        peer: SocketAddr,
        mut hs: HandShakeInfo,
    ) -> Rc<RefCell<Self>> {
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

        let socket = Rc::new(RefCell::new(self));
        if let Some(mut mux) = socket.borrow().multiplexer.as_ref().map(|m| m.borrow_mut()) {
            mux.rcv_queue.push_back(socket.clone());
        }

        socket
    }

    pub fn set_multiplexer(&mut self, mux: Rc<RefCell<UdtMultiplexer>>) {
        self.multiplexer = Some(mux);
    }

    pub fn set_self_ip(&mut self, ip: IpAddr) {
        self.self_ip = Some(ip);
    }
}

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

use std::net::SocketAddr;

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
    self_addr: Option<SocketAddr>,
    peer_addr: Option<SocketAddr>,
    initial_seq_number: u32,
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
            listen_socket: None,
        }
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

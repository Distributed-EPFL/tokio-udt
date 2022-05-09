use std::net::SocketAddr;

#[derive(Debug)]
pub(crate) struct UdtSocket {
    socket_id: u16,
    status: UdtStatus,
    self_addr: SocketAddr,
    peer_addr: SocketAddr,
    initial_seq_number: u32,
}

#[derive(Debug)]
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

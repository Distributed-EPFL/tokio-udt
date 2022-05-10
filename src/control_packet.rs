use super::socket::SocketType;
use std::net::IpAddr;

#[derive(Debug)]
pub(crate) struct UDTControlPacket {
    // bit 0 = 1
    pub packet_type: ControlPacketType, // bits 1-15 + Control Information Field (bits 128+)
    pub reserved: u16,                  // bits 16-31
    pub additional_info: u32,           // bits 32-63
    pub timestamp: u32,                 // bits 64-95
    pub dest_socket_id: u32,            // bits 96-127
}

#[derive(Debug)]
pub(crate) enum ControlPacketType {
    Handshake(HandShakeInfo),
    KeepAlive,
    Ack(AckInfo),
    Nak(NakInfo),
    Shutdown,
    Ack2,
    MsgDropRequest(DropRequestInfo),
    UserDefined,
}

#[derive(Debug)]
pub(crate) struct HandShakeInfo {
    pub udt_version: u32,
    pub socket_type: SocketType,
    pub initial_seq_number: u32,
    pub max_packet_size: u32,
    pub max_window_size: u32,
    pub connection_type: i32, // regular or rendezvous
    pub socket_id: u32,
    pub syn_cookie: u32,
    pub ip_address: IpAddr,
}

#[derive(Debug)]
pub(crate) struct AckInfo {
    /// The packet sequence number to which all the
    /// previous packets have been received (excluding)
    pub next_seq_number: u32,
    pub info: Option<AckOptionalInfo>,
}

#[derive(Debug)]
pub(crate) struct AckOptionalInfo {
    /// RTT in microseconds
    pub rtt: u32,
    pub rtt_variance: u32,
    pub available_buf_size: u32,
    pub pack_recv_rate: u32,
    pub link_capacity: u32,
}

#[derive(Debug)]
pub(crate) struct NakInfo {
    pub loss_info: Vec<u32>,
}

#[derive(Debug)]
pub(crate) struct DropRequestInfo {
    pub first_seq_number: u32,
    pub last_seq_number: u32,
}

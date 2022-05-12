use super::socket::{SocketId, SocketType};
use crate::common::ip_to_bytes;
use std::net::IpAddr;

#[derive(Debug)]
pub(crate) struct UdtControlPacket {
    // bit 0 = 1
    pub packet_type: ControlPacketType, // bits 1-15 + Control Information Field (bits 128+)
    pub reserved: u16,                  // bits 16-31
    pub additional_info: u32,           // bits 32-63
    pub timestamp: u32,                 // bits 64-95
    pub dest_socket_id: SocketId,       // bits 96-127
}

impl UdtControlPacket {
    pub fn new_handshake(hs: HandShakeInfo, dest_socket_id: SocketId) -> Self {
        Self {
            packet_type: ControlPacketType::Handshake(hs),
            reserved: 0,
            additional_info: 0,
            timestamp: 0, // TODO set timestamp here ?
            dest_socket_id,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::with_capacity(8);
        buffer.extend_from_slice(&(0xf000 + self.packet_type.type_as_u15()).to_be_bytes());
        buffer.extend_from_slice(&self.reserved.to_be_bytes());
        buffer.extend_from_slice(&self.additional_info.to_be_bytes());
        buffer.extend_from_slice(&self.timestamp.to_be_bytes());
        buffer.extend_from_slice(&self.dest_socket_id.to_be_bytes());
        buffer.extend_from_slice(&self.packet_type.control_info_field());
        buffer
    }
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

impl ControlPacketType {
    pub fn type_as_u15(&self) -> u16 {
        match self {
            Self::Handshake(_) => 0x0000,
            Self::KeepAlive => 0x0001,
            Self::Ack(_) => 0x0002,
            Self::Nak(_) => 0x0003,
            Self::Shutdown => 0x0005,
            Self::Ack2 => 0x0006,
            Self::MsgDropRequest(_) => 0x0007,
            Self::UserDefined => 0x7fff,
        }
    }

    pub fn control_info_field(&self) -> Vec<u8> {
        match self {
            Self::Handshake(hs) => hs.serialize(),
            // TODO serialize ACK, NAK and MessageDrop
            _ => vec![],
        }
    }
}

#[derive(Debug)]
pub(crate) struct HandShakeInfo {
    pub udt_version: u32,
    pub socket_type: SocketType,
    pub initial_seq_number: u32,
    pub max_packet_size: u32,
    pub max_window_size: u32,
    pub connection_type: i32, // regular or rendezvous
    pub socket_id: SocketId,
    pub syn_cookie: u32,
    pub ip_address: IpAddr,
}

impl HandShakeInfo {
    pub fn serialize(&self) -> Vec<u8> {
        [
            self.udt_version,
            self.socket_type as u32,
            self.initial_seq_number,
            self.max_packet_size,
            self.max_window_size,
        ]
        .iter()
        .map(|v| v.to_be_bytes())
        .flatten()
        .chain(self.connection_type.to_be_bytes().into_iter())
        .chain(self.socket_id.to_be_bytes().into_iter())
        .chain(self.syn_cookie.to_be_bytes().into_iter())
        .chain(ip_to_bytes(self.ip_address))
        .collect()
    }
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

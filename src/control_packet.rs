use super::socket::{SocketId, SocketType};
use crate::common::ip_to_bytes;
use crate::seq_number::SeqNumber;
use std::net::IpAddr;
use tokio::io::{Error, ErrorKind, Result};

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

    pub fn new_nak(loss_list: Vec<u32>, dest_socket_id: SocketId) -> Self {
        Self {
            packet_type: ControlPacketType::Nak(NakInfo {
                loss_info: loss_list,
            }),
            reserved: 0,
            additional_info: 0,
            timestamp: 0,
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

    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        if raw.len() < 16 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "control packet header is too short",
            ));
        }
        let reserved = u16::from_be_bytes(raw[2..4].try_into().unwrap());
        let additional_info = u32::from_be_bytes(raw[4..8].try_into().unwrap());
        let timestamp = u32::from_be_bytes(raw[8..12].try_into().unwrap());
        let dest_socket_id = u32::from_be_bytes(raw[12..16].try_into().unwrap());
        let packet_type = ControlPacketType::deserialize(raw)?;
        Ok(Self {
            reserved,
            additional_info,
            timestamp,
            dest_socket_id,
            packet_type,
        })
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

    pub fn deserialize(raw_control_packet: &[u8]) -> Result<Self> {
        let type_id = u16::from_be_bytes(raw_control_packet[0..2].try_into().unwrap()) & 0x7FFF;
        let packet = match type_id {
            0x0000 => Self::Handshake(HandShakeInfo::deserialize(&raw_control_packet[16..])?),
            0x0001 => Self::KeepAlive,
            0x0002 => Self::Ack(AckInfo::deserialize(&raw_control_packet[16..])?),
            0x0003 => Self::Nak(NakInfo::deserialize(&raw_control_packet[16..])?),
            0x0005 => Self::Shutdown,
            0x0006 => Self::Ack2,
            0x0007 => {
                Self::MsgDropRequest(DropRequestInfo::deserialize(&raw_control_packet[16..])?)
            }
            0x7fff => Self::UserDefined,
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "unknown control packet type",
                ));
            }
        };
        Ok(packet)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HandShakeInfo {
    pub udt_version: u32,
    pub socket_type: SocketType,
    pub initial_seq_number: SeqNumber,
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
            self.initial_seq_number.number(),
            self.max_packet_size,
            self.max_window_size,
        ]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .chain(self.connection_type.to_be_bytes().into_iter())
        .chain(self.socket_id.to_be_bytes().into_iter())
        .chain(self.syn_cookie.to_be_bytes().into_iter())
        .chain(ip_to_bytes(self.ip_address))
        .collect()
    }

    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        !unimplemented!()
    }
}

#[derive(Debug)]
pub(crate) struct AckInfo {
    /// The packet sequence number to which all the
    /// previous packets have been received (excluding)
    pub next_seq_number: SeqNumber,
    pub info: Option<AckOptionalInfo>,
}

impl AckInfo {
    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        !unimplemented!()
    }
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

impl NakInfo {
    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        !unimplemented!()
    }
}

#[derive(Debug)]
pub(crate) struct DropRequestInfo {
    pub first_seq_number: SeqNumber,
    pub last_seq_number: SeqNumber,
}

impl DropRequestInfo {
    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        !unimplemented!()
    }
}

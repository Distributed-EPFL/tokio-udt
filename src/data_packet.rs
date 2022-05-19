use crate::seq_number::SeqNumber;
use tokio::io::{Error, ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct UdtDataPacket {
    pub header: UdtDataPacketHeader,
    pub data: Vec<u8>,
}

impl UdtDataPacket {
    pub fn deserialize(mut raw: Vec<u8>) -> Result<Self> {
        let header = UdtDataPacketHeader::deserialize(&raw[..16])?;
        let data = raw.drain(16..).collect();
        Ok(Self { header, data })
    }
}

#[derive(Debug)]
pub(crate) struct UdtDataPacketHeader {
    // bit 0 = 0
    pub seq_number: SeqNumber,    // bits 1-31
    pub position: PacketPosition, // bits 32-33
    pub in_order: bool,           // bit 34
    pub msg_number: u32,          // bits 35-63
    pub timestamp: u32,           // bits 64-95
    pub dest_socket_id: u32,      // bits 96-127
}

impl UdtDataPacketHeader {
    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        if raw.len() < 16 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "data packet header is too short",
            ));
        }
        let seq_number = u32::from_be_bytes(raw[0..4].try_into().unwrap()) & 0x7fffffff;
        let position: PacketPosition = ((raw[4] & 0b11000000) >> 6).try_into()?;
        let in_order = (raw[4] & 0b00100000) != 0;
        let msg_number = u32::from_be_bytes(raw[4..8].try_into().unwrap()) & 0x1fffffff;
        let timestamp = u32::from_be_bytes(raw[8..12].try_into().unwrap());
        let dest_socket_id = u32::from_be_bytes(raw[12..16].try_into().unwrap());
        Ok(Self {
            seq_number: seq_number.into(),
            position,
            in_order,
            msg_number,
            timestamp,
            dest_socket_id,
        })
    }
}

#[derive(Debug)]
pub(crate) enum PacketPosition {
    First,
    Last,
    Only,
    Middle,
}

impl TryFrom<u8> for PacketPosition {
    type Error = Error;

    fn try_from(raw_position: u8) -> Result<Self> {
        match raw_position {
            0b10 => Ok(PacketPosition::First),
            0b01 => Ok(PacketPosition::Last),
            0b11 => Ok(PacketPosition::Only),
            0b00 => Ok(PacketPosition::Middle),
            _ => Err(Error::new(
                ErrorKind::InvalidData,
                "invalid packet position",
            )),
        }
    }
}

use crate::seq_number::{MsgNumber, SeqNumber};
use bytes::Bytes;
use tokio::io::{Error, ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct UdtDataPacket {
    pub header: UdtDataPacketHeader,
    pub data: Bytes,
}

impl UdtDataPacket {
    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        let header = UdtDataPacketHeader::deserialize(&raw[..16])?;
        let data = Bytes::copy_from_slice(&raw[16..]);
        Ok(Self { header, data })
    }

    pub fn payload_len(&self) -> u32 {
        self.data.len() as u32
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(1600);
        buffer.extend_from_slice(&self.header.serialize());
        buffer.extend_from_slice(&self.data);
        buffer
    }
}

#[derive(Debug)]
pub(crate) struct UdtDataPacketHeader {
    // bit 0 = 0
    pub seq_number: SeqNumber,    // bits 1-31
    pub position: PacketPosition, // bits 32-33
    pub in_order: bool,           // bit 34
    pub msg_number: MsgNumber,    // bits 35-63
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
            msg_number: msg_number.into(),
            timestamp,
            dest_socket_id,
        })
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::with_capacity(16);
        buffer.extend_from_slice(&self.seq_number.number().to_be_bytes());

        let block: u32 = ((self.position as u32) << 30)
            + ((self.in_order as u32) << 29)
            + self.msg_number.number();

        buffer.extend_from_slice(&block.to_be_bytes());
        buffer.extend_from_slice(&self.timestamp.to_be_bytes());
        buffer.extend_from_slice(&self.dest_socket_id.to_be_bytes());
        buffer
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PacketPosition {
    First = 2,
    Last = 1,
    Only = 3,
    Middle = 0,
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

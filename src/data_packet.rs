#[derive(Debug)]
pub(crate) struct UDTDataPacket {
    pub header: UDTDataPacketHeader,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct UDTDataPacketHeader {
    // bit 0 = 0
    pub seq_number: u32,          // bits 1-31
    pub position: PacketPosition, // bits 32-33
    pub in_order: bool,           // bit 34
    pub msg_number: u32,          // bits 35-63
    pub timestamp: u32,           // bits 64-95
    pub dest_socket_id: u32,      // bits 96-127
}

#[derive(Debug)]
pub(crate) enum PacketPosition {
    First,
    Last,
    Only,
    Middle,
}

impl TryFrom<u8> for PacketPosition {
    type Error = &'static str;

    fn try_from(raw_position: u8) -> Result<Self, Self::Error> {
        match raw_position {
            0b10 => Ok(PacketPosition::First),
            0b01 => Ok(PacketPosition::Last),
            0b11 => Ok(PacketPosition::Only),
            0b00 => Ok(PacketPosition::Middle),
            _ => Err("invalid packet position"),
        }
    }
}

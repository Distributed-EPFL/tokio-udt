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
    FirstPacket,
    LastPacket,
    OnlyPacket,
    MiddlePacket,
}

impl TryFrom<u8> for PacketPosition {
    type Error = &'static str;

    fn try_from(raw_position: u8) -> Result<Self, Self::Error> {
        match raw_position {
            0b10 => Ok(PacketPosition::FirstPacket),
            0b01 => Ok(PacketPosition::LastPacket),
            0b11 => Ok(PacketPosition::OnlyPacket),
            0b00 => Ok(PacketPosition::MiddlePacket),
            _ => Err("invalid packet position"),
        }
    }
}

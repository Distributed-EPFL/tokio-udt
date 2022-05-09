use super::control_packet::UDTControlPacket;
use super::data_packet::UDTDataPacket;

#[derive(Debug)]
pub(crate) enum UdtPacket {
    Control(UDTControlPacket),
    Data(UDTDataPacket),
}

impl UdtPacket {
    fn get_dest_socket_id(&self) -> u32 {
        match self {
            Self::Control(p) => p.dest_socket_id,
            Self::Data(p) => p.header.dest_socket_id,
        }
    }
}

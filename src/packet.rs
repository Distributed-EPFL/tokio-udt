use super::control_packet::UdtControlPacket;
use super::data_packet::UdtDataPacket;

#[derive(Debug)]
pub(crate) enum UdtPacket {
    Control(UdtControlPacket),
    Data(UdtDataPacket),
}

impl UdtPacket {
    pub fn get_dest_socket_id(&self) -> u32 {
        match self {
            Self::Control(p) => p.dest_socket_id,
            Self::Data(p) => p.header.dest_socket_id,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        match self {
            Self::Control(p) => p.serialize(),
            Self::Data(p) => unimplemented!(), // TODO
        }
    }
}

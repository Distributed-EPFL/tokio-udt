use crate::data_packet::UdtDataPacket;
use crate::seq_number::{MsgNumber, SeqNumber};
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use tokio::io::{Error, ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct RcvBuffer {
    packets: BTreeMap<SeqNumber, UdtDataPacket>, // map: seq_number -> packet
    max_size: u32,
}

impl RcvBuffer {
    pub fn new(max_size: u32) -> Self {
        Self {
            max_size,
            packets: BTreeMap::new(),
        }
    }

    pub fn get_available_buf_size(&self) -> u32 {
        self.max_size - self.packets.len() as u32
    }

    pub fn insert(&mut self, packet: UdtDataPacket) -> Result<()> {
        let seq_number = packet.header.seq_number;
        match self.packets.entry(seq_number) {
            Entry::Occupied(_) => Err(Error::new(
                ErrorKind::AlreadyExists,
                "a packet with the same seq number is present in buffer",
            )),
            Entry::Vacant(e) => {
                e.insert(packet);
                Ok(())
            }
        }
    }

    pub fn drop_msg(&mut self, msg: MsgNumber) {
        self.packets
            .retain(|_k, packet| packet.header.msg_number != msg);
    }

    pub fn ack_data(&mut self, from: SeqNumber, to: SeqNumber) {
        if from > to {
            self.ack_data(from, SeqNumber::max());
            self.ack_data(SeqNumber::zero(), to);
            return;
        }
        let keys: Vec<_> = self.packets.range(from..to).map(|(k, _)| *k).collect();
        for key in keys {
            self.packets.remove(&key);
        }
    }
}

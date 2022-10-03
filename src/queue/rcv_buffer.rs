use crate::data_packet::UdtDataPacket;
use crate::seq_number::{MsgNumber, SeqNumber};
use std::collections::BTreeMap;
use tokio::io::ReadBuf;

#[derive(Debug)]
pub(crate) struct RcvBuffer {
    packets: BTreeMap<SeqNumber, UdtDataPacket>,
    max_size: u32,
    next_to_read: SeqNumber,
    next_to_ack: SeqNumber,
}

impl RcvBuffer {
    pub fn new(max_size: u32, initial_seq_number: SeqNumber) -> Self {
        Self {
            max_size,
            packets: BTreeMap::new(),
            next_to_read: initial_seq_number,
            next_to_ack: initial_seq_number,
        }
    }

    pub fn get_available_buf_size(&self) -> u32 {
        self.max_size - self.packets.len() as u32
    }

    pub fn insert(&mut self, packet: UdtDataPacket) {
        let seq_number = packet.header.seq_number;
        self.packets.entry(seq_number).or_insert(packet);
    }

    pub fn drop_msg(&mut self, msg: MsgNumber) {
        self.packets
            .retain(|_k, packet| packet.header.msg_number != msg);
    }

    pub fn ack_data(&mut self, to: SeqNumber) {
        if (to - self.next_to_ack) > 0 {
            self.next_to_ack = to;
        }
    }

    pub fn has_data_to_read(&self) -> bool {
        let first = self.next_to_read;
        let last = self.next_to_ack;
        if first <= last {
            return self.packets.range(first..last).next().is_some();
        } else {
            return self
                .packets
                .range(first..=SeqNumber::max())
                .next()
                .is_some()
                || self.packets.range(SeqNumber::zero()..last).next().is_some();
        }
    }

    pub fn read_buffer(&mut self, buf: &mut ReadBuf<'_>) -> usize {
        if self.next_to_read == self.next_to_ack {
            return 0;
        }

        let packets = {
            if self.next_to_read <= self.next_to_ack {
                self.packets
                    .range(self.next_to_read..self.next_to_ack)
                    .chain(
                        self.packets.range(SeqNumber::zero()..SeqNumber::zero()), //empty
                    )
            } else {
                self.packets
                    .range(self.next_to_read..=SeqNumber::max())
                    .chain(self.packets.range(SeqNumber::zero()..self.next_to_ack))
            }
        };

        let mut written = 0;
        let mut to_remove = vec![];
        for (key, packet) in packets {
            let packet_len = packet.data.len();
            if buf.remaining() < packet_len {
                break;
            }
            buf.put_slice(&packet.data);
            written += packet_len;
            to_remove.push(*key);
            self.next_to_read = *key + 1;
        }

        to_remove.iter().for_each(|k| {
            self.packets.remove(k);
        });

        written
    }
}

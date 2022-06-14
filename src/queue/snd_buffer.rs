use crate::data_packet::{PacketPosition, UdtDataPacket, UdtDataPacketHeader};
use crate::seq_number::MsgNumber;
use crate::seq_number::SeqNumber;
use crate::socket::SocketId;
use bytes::Bytes;
use std::collections::VecDeque;
use tokio::io::{Error, ErrorKind, Result as IoResult};
use tokio::time::{Duration, Instant};

const FETCH_BATCH_SIZE: usize = 100;

#[derive(Debug, Clone)]
pub(crate) struct SndBufferBlock {
    data: Bytes,
    msg_number: MsgNumber,
    origin_time: Instant,
    ttl: Option<u64>, // milliseconds,
    in_order: bool,
    position: PacketPosition,
}

impl SndBufferBlock {
    fn has_expired(&self) -> bool {
        if let Some(ttl) = self.ttl {
            return self.origin_time.elapsed() > Duration::from_millis(ttl);
        }
        false
    }

    fn as_data_packet(
        &self,
        seq_number: SeqNumber,
        dest_socket_id: SocketId,
        start_time: Instant,
    ) -> UdtDataPacket {
        UdtDataPacket {
            data: self.data.clone(),
            header: UdtDataPacketHeader {
                msg_number: self.msg_number,
                dest_socket_id,
                seq_number,
                in_order: self.in_order,
                position: self.position,
                timestamp: (start_time.elapsed().as_micros() & (u32::MAX as u128)) as u32,
            },
        }
    }
}

#[derive(Debug)]
pub(crate) struct SndBuffer {
    max_size: u32,
    buffer: VecDeque<SndBufferBlock>,
    mss: u32,
    next_msg_number: MsgNumber,
    current_position: usize,
}

impl SndBuffer {
    pub fn new(max_size: u32, mss: u32) -> Self {
        Self {
            max_size,
            buffer: VecDeque::new(),
            mss,
            next_msg_number: MsgNumber::zero(),
            current_position: 0,
        }
    }

    pub fn add_message(&mut self, data: &[u8], ttl: Option<u64>, in_order: bool) -> IoResult<()> {
        let msg_number = self.next_msg_number;
        let now = Instant::now();
        let chunks = data.chunks(self.mss as usize);
        let chunks_len = chunks.len();

        if self.buffer.len() + chunks_len > self.max_size as usize {
            return Err(Error::new(ErrorKind::OutOfMemory, "Send buffer is full"));
        }

        self.buffer
            .extend(chunks.enumerate().map(|(idx, chunk)| SndBufferBlock {
                data: Bytes::copy_from_slice(chunk),
                msg_number,
                origin_time: now,
                ttl,
                in_order,
                position: {
                    if idx == 0 && chunks_len == 1 {
                        PacketPosition::Only
                    } else if idx == 0 {
                        PacketPosition::First
                    } else if idx == chunks_len - 1 {
                        PacketPosition::Last
                    } else {
                        PacketPosition::Middle
                    }
                },
            }));
        self.next_msg_number = self.next_msg_number + 1;
        Ok(())
    }

    pub fn ack_data(&mut self, offset: i32) {
        for _ in 0..offset {
            if self.buffer.pop_front().is_some() {
                self.current_position -= 1;
            }
        }
    }

    pub fn read_data(
        &mut self,
        offset: usize,
        seq_number: SeqNumber,
        dest_socket_id: SocketId,
        start_time: Instant,
    ) -> Result<UdtDataPacket, (MsgNumber, usize)> {
        if let Some(block) = self.buffer.get(offset) {
            if block.has_expired() {
                // Move current_position to next message
                let mut pos = offset + 1;
                let mut msg_len = 1;
                while pos < self.buffer.len() {
                    if self.buffer[pos].msg_number == block.msg_number {
                        msg_len += 1;
                    } else {
                        break;
                    }
                    pos += 1;
                }
                if offset <= self.current_position && self.current_position < pos {
                    self.current_position = pos;
                }
                Err((block.msg_number, msg_len))
            } else {
                Ok(block.as_data_packet(seq_number, dest_socket_id, start_time))
            }
        } else {
            Err((MsgNumber::zero(), 0)) // No msg found
        }
    }

    pub fn fetch_batch(
        &mut self,
        mut seq_number: SeqNumber,
        dest_socket_id: SocketId,
        start_time: Instant,
    ) -> Vec<UdtDataPacket> {
        let blocks: Vec<_> = self
            .buffer
            .range(self.current_position..)
            .take(FETCH_BATCH_SIZE)
            .map(|block| {
                let packet = block.as_data_packet(seq_number, dest_socket_id, start_time);
                seq_number = seq_number + 1;
                packet
            })
            .collect();
        self.current_position += blocks.len();
        blocks
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

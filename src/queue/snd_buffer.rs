use crate::seq_number::MsgNumber;
use std::collections::VecDeque;
use tokio::time::Instant;

struct SndBufferBlock {
    data: Vec<u8>,
    msg_number: MsgNumber,
    origin_time: Instant,
    ttl: usize, // milliseconds,
    in_order: bool,
}

pub(crate) struct SndBuffer {
    buffer: VecDeque<SndBufferBlock>,
    mss: usize,
    next_msg_number: MsgNumber,
}

impl SndBuffer {
    pub fn new(mss: usize) -> Self {
        Self {
            buffer: VecDeque::new(),
            mss,
            next_msg_number: MsgNumber::zero(),
        }
    }

    pub fn add_data(&mut self, data: Vec<u8>, ttl: usize, in_order: bool) {
        self.buffer.extend(data.chunks(self.mss).map(|chunk| {
            let msg_number = self.next_msg_number;
            self.next_msg_number = self.next_msg_number + 1;
            SndBufferBlock {
                data: chunk.into(),
                msg_number,
                origin_time: Instant::now(),
                ttl,
                in_order,
            }
        }));
    }
}

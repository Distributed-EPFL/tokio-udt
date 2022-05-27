use crate::seq_number::{AckSeqNumber, SeqNumber};
use std::collections::{BTreeMap, VecDeque};
use tokio::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct AckWindow {
    size: usize,
    acks: BTreeMap<AckSeqNumber, (SeqNumber, Instant)>,
    keys: VecDeque<AckSeqNumber>,
}

impl AckWindow {
    pub fn new(size: usize) -> Self {
        Self {
            size,
            acks: BTreeMap::new(),
            keys: VecDeque::with_capacity(size),
        }
    }

    pub fn store(&mut self, seq: SeqNumber, ack: AckSeqNumber) {
        if self.keys.len() >= self.size {
            let oldest = self.keys.pop_front().unwrap();
            self.acks.remove(&oldest);
        }
        self.keys.push_back(ack);
        self.acks.insert(ack, (seq, Instant::now()));
    }

    pub fn get(&mut self, ack: AckSeqNumber) -> Option<(SeqNumber, Duration)> {
        self.acks.get(&ack).map(|(seq, ts)| (*seq, ts.elapsed()))
    }
}

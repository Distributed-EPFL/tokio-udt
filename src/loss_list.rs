use crate::seq_number::{SeqNumber, MAX_SEQ_NUMBER};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct RcvLossList {
    sequences: BTreeMap<SeqNumber, (SeqNumber, SeqNumber)>,
    max_size: u32,
}

impl RcvLossList {
    pub fn new(max_size: u32) -> Self {
        Self {
            sequences: BTreeMap::new(),
            max_size,
        }
    }

    pub fn insert(&mut self, n1: SeqNumber, n2: SeqNumber) {
        if n1.number() > n2.number() {
            self.insert(n1, SeqNumber::max());
            self.insert(SeqNumber::zero(), n2);
            return;
        }
        if let Some((_, (start, end))) = self.sequences.range_mut(..=n1).next_back() {
            if *end == n1 - 1 {
                *end = n2;
                return;
            }
        }
        self.sequences.insert(n1, (n1, n2));
    }

    pub fn remove(&mut self, num: SeqNumber) {
        if let Some((key, (start, end))) = self.sequences.range_mut(..=num).next_back() {
            if *start == num {
                let key = *key;
                let end = *end;
                self.sequences.remove(&key);
                if end > num {
                    self.sequences.insert(num + 1, (num + 1, end));
                }
            } else if *end >= num {
                let current_end = *end;
                *end = num - 1;
                if current_end > num {
                    self.sequences.insert(num + 1, (num + 1, current_end));
                }
            }
        }
    }

    fn remove_all(&mut self, n1: SeqNumber, n2: SeqNumber) {
        if n1 <= n2 {
            for i in n1.number()..=n2.number() {
                self.remove(i.into());
            }
        } else {
            for i in n1.number()..=MAX_SEQ_NUMBER {
                self.remove(i.into());
            }
            for i in 0..=n2.number() {
                self.remove(i.into());
            }
        }
    }

    fn find(&self, n1: SeqNumber, n2: SeqNumber) -> bool {
        if n1 > n2 {
            return self.find(n1, SeqNumber::max()) || self.find(SeqNumber::zero(), n2);
        }

        if let Some((_, (_start, end))) = self.sequences.range(..n1).next_back() {
            if *end >= n1 {
                return true;
            }
        } else if let Some((_, (start, _end))) = self.sequences.range(n1..).next() {
            if *start <= n2 {
                return true;
            }
        }
        false
    }

    fn is_empty(&self) -> bool {
        self.sequences.is_empty()
    }

    fn get_loss_array(&self, limit: usize) -> Vec<u32> {
        let mut array: Vec<_> = self
            .sequences
            .values()
            .flat_map(|(start, end)| {
                if start == end {
                    vec![start.number()]
                } else {
                    vec![start.number() | 0x8000000, end.number()]
                }
            })
            .take(limit)
            .collect();

        if let Some(v) = array.last() {
            if *v >= 0x8000000 {
                array.pop();
            }
        }
        array
    }
}

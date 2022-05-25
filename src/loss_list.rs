use crate::seq_number::SeqNumber;
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

        let mut keys_to_remove = vec![];
        for (key, (start, end)) in self.sequences.range_mut((n1 + 1)..=n2) {
            if *end > n2 {
                *start = n2 + 1;
            } else {
                keys_to_remove.push(*key);
            }
        }
        for key in keys_to_remove {
            self.sequences.remove(&key);
        }

        if let Some((_, (_start, end))) = self.sequences.range_mut(..=n1).next_back() {
            if *end == n1 - 1 {
                *end = n2;
                return;
            }
        }
        self.sequences
            .entry(n1)
            .and_modify(|(_start, end)| *end = std::cmp::max(*end, n2))
            .or_insert((n1, n2));
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

    pub fn remove_all(&mut self, n1: SeqNumber, n2: SeqNumber) {
        if n1 <= n2 {
            for i in (n1.number()..=n2.number()).rev() {
                self.remove(i.into());
            }
        } else {
            self.remove_all(n1, SeqNumber::max());
            self.remove_all(SeqNumber::zero(), n2);
        }
    }

    // fn find(&self, n1: SeqNumber, n2: SeqNumber) -> bool {
    //     if n1 > n2 {
    //         return self.find(n1, SeqNumber::max()) || self.find(SeqNumber::zero(), n2);
    //     }

    //     if let Some((_, (_start, end))) = self.sequences.range(..n1).next_back() {
    //         if *end >= n1 {
    //             return true;
    //         }
    //     } else if let Some((_, (start, _end))) = self.sequences.range(n1..).next() {
    //         if *start <= n2 {
    //             return true;
    //         }
    //     }
    //     false
    // }

    // pub fn is_empty(&self) -> bool {
    //     self.sequences.is_empty()
    // }

    // pub fn get_loss_array(&self, limit: usize) -> Vec<u32> {
    //     let mut array: Vec<_> = self
    //         .sequences
    //         .values()
    //         .flat_map(|(start, end)| {
    //             if start == end {
    //                 vec![start.number()]
    //             } else {
    //                 vec![start.number() | 0x8000000, end.number()]
    //             }
    //         })
    //         .take(limit)
    //         .collect();

    //     if let Some(v) = array.last() {
    //         if *v >= 0x8000000 {
    //             array.pop();
    //         }
    //     }
    //     array
    // }

    pub fn pop_after(&mut self, after: SeqNumber) -> Option<SeqNumber> {
        if self.sequences.is_empty() {
            return None;
        }
        if let Some((_, (_start, end))) = self.sequences.range(..=after).next_back() {
            if *end >= after {
                self.remove(after);
                return Some(after);
            }
        }
        if let Some((_, (start, _end))) = self.sequences.range(after..).next() {
            let start = *start;
            self.remove(start);
            return Some(start);
        }
        if let Some((_, (start, _end))) = self.sequences.iter().next() {
            let start = *start;
            self.remove(start);
            return Some(start);
        }
        None
    }
}

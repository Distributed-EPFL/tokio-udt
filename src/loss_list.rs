use crate::seq_number::SeqNumber;
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct LossList {
    sequences: BTreeMap<SeqNumber, (SeqNumber, SeqNumber)>,
}

impl LossList {
    pub fn new() -> Self {
        Self {
            sequences: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, n1: SeqNumber, n2: SeqNumber) {
        // TODO: limit size of loss list

        if n1.number() > n2.number() {
            self.insert(n1, SeqNumber::max());
            self.insert(SeqNumber::zero(), n2);
            return;
        }

        let mut n2 = n2;
        if n2.number() > n1.number() {
            let mut keys_to_remove = vec![];
            for (key, (_start, end)) in self.sequences.range((n1 + 1)..=n2) {
                keys_to_remove.push(*key);
                if *end > n2 {
                    n2 = *end;
                }
            }
            for key in keys_to_remove {
                self.sequences.remove(&key);
            }
        }

        if let Some((_, (_start, end))) = self.sequences.range_mut(..=n1).next_back() {
            if *end >= n1 - 1 {
                *end = std::cmp::max(*end, n2);
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

    pub fn is_empty(&self) -> bool {
        self.sequences.is_empty()
    }

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

    pub fn peek_after(&self, after: SeqNumber) -> Option<SeqNumber> {
        if self.sequences.is_empty() {
            return None;
        }
        if let Some((_, (_start, end))) = self.sequences.range(..=after).next_back() {
            if *end >= after {
                return Some(after);
            }
        }
        if let Some((_, (start, _end))) = self.sequences.range(after..).next() {
            return Some(*start);
        }
        if let Some((_, (start, _end))) = self.sequences.iter().next() {
            return Some(*start);
        }
        None
    }
}

#[test]
fn test_insert_sequences() {
    let mut loss_list = crate::loss_list::LossList::new();
    loss_list.insert(5.into(), 10.into());
    loss_list.insert(1.into(), 2.into());
    assert_eq!(loss_list.sequences.len(), 2);

    let items: Vec<_> = loss_list.sequences.clone().into_iter().collect();
    assert_eq!(
        items,
        [
            (1.into(), (1.into(), 2.into())),
            (5.into(), (5.into(), 10.into())),
        ]
    );

    assert_eq!(loss_list.peek_after(1.into()), Some(1.into()));
    assert_eq!(loss_list.peek_after(4.into()), Some(5.into()));
    assert_eq!(loss_list.peek_after(10.into()), Some(10.into()));
    assert_eq!(loss_list.peek_after(11.into()), Some(1.into()));
}

#[test]
fn test_insert_overlapping_sequence() {
    let mut loss_list = crate::loss_list::LossList::new();
    loss_list.insert(1.into(), 10.into());
    loss_list.insert(5.into(), 20.into());
    assert_eq!(loss_list.sequences.len(), 1);
    let items: Vec<_> = loss_list.sequences.into_iter().collect();
    assert_eq!(items, [(1.into(), (1.into(), 20.into())),]);
}

#[test]
fn test_insert_with_multiple_overlapping_sequences() {
    let mut loss_list = crate::loss_list::LossList::new();
    loss_list.insert(6.into(), 10.into());
    loss_list.insert(12.into(), 25.into());
    loss_list.insert(1.into(), 22.into());
    assert_eq!(loss_list.sequences.len(), 1);
    let items: Vec<_> = loss_list.sequences.into_iter().collect();
    assert_eq!(items, [(1.into(), (1.into(), 25.into())),]);
}

#[test]
fn test_insert_with_bigger_existing_sequence() {
    let mut loss_list = crate::loss_list::LossList::new();
    loss_list.insert(10.into(), 30.into());
    loss_list.insert(10.into(), 20.into());
    assert_eq!(loss_list.sequences.len(), 1);
    let items: Vec<_> = loss_list.sequences.into_iter().collect();
    assert_eq!(items, [(10.into(), (10.into(), 30.into())),]);
}

#[test]
fn test_remove_seq_inside_sequence() {
    let mut loss_list = crate::loss_list::LossList::new();
    loss_list.insert(1.into(), 10.into());
    loss_list.remove(5.into());

    assert_eq!(loss_list.sequences.len(), 2);
    let items: Vec<_> = loss_list.sequences.into_iter().collect();
    assert_eq!(
        items,
        [
            (1.into(), (1.into(), 4.into())),
            (6.into(), (6.into(), 10.into())),
        ]
    );
}

#[test]
fn test_remove_first_item() {
    let mut loss_list = crate::loss_list::LossList::new();
    loss_list.insert(1.into(), 10.into());
    loss_list.remove(1.into());

    assert_eq!(loss_list.sequences.len(), 1);
    let items: Vec<_> = loss_list.sequences.into_iter().collect();
    assert_eq!(items, [(2.into(), (2.into(), 10.into())),]);
}

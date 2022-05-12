use super::socket::UdtSocket;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::rc::Rc;
use std::time::Instant;

#[derive(PartialEq, Eq)]
struct SendQueueNode {
    timestamp: Instant,
    socket: Rc<RefCell<UdtSocket>>,
}

impl Ord for SendQueueNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.timestamp.cmp(&other.timestamp).reverse()
    }
}

impl PartialOrd for SendQueueNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct UdtSendQueue {
    sockets: BinaryHeap<SendQueueNode>,
}

impl UdtSendQueue {
    fn new() -> Self {
        unimplemented!()
    }
}

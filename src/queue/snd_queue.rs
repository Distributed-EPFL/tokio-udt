use crate::socket::UdtSocket;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::rc::Rc;
use tokio::io::Result;
use tokio::sync::Notify;
use tokio::time::{sleep_until, Instant};

#[derive(Debug, PartialEq, Eq)]
struct SendQueueNode {
    timestamp: Instant,
    socket: Rc<RefCell<UdtSocket>>,
}

impl Ord for SendQueueNode {
    // Send queue should be sorted by smaller timestamp first
    fn cmp(&self, other: &Self) -> Ordering {
        self.timestamp.cmp(&other.timestamp).reverse()
    }
}

impl PartialOrd for SendQueueNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub(crate) struct UdtSndQueue {
    sockets: BinaryHeap<SendQueueNode>,
    notify: Notify,
}

impl UdtSndQueue {
    pub fn new() -> Self {
        UdtSndQueue {
            sockets: BinaryHeap::new(),
            notify: Notify::new(),
        }
    }

    pub async fn worker(&mut self) -> Result<()> {
        loop {
            if let Some(node) = self.sockets.peek() {
                tokio::select! {
                   _ = sleep_until(node.timestamp) => {}
                   _ = self.notify.notified() => {}
                }
                if let Some(node) = self.sockets.pop() {
                    node.socket.borrow_mut().send_next_packet()?;
                }
            } else {
                self.notify.notified().await
            }
        }
    }
}

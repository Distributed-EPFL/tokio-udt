use crate::socket::SocketId;
use crate::udt::SocketRef;
use crate::udt::Udt;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use tokio::io::Result;
use tokio::sync::Notify;
use tokio::time::{sleep_until, Instant};

#[derive(Debug, PartialEq, Eq)]
struct SendQueueNode {
    timestamp: Instant,
    socket_id: SocketId,
}

impl SendQueueNode {
    pub async fn socket(&self) -> Option<SocketRef<'static>> {
        Udt::get().read().await.get_socket(self.socket_id).await
    }
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
                    if let Some(socket) = node.socket().await {
                        socket.write().await.send_next_packet()?;
                    }
                }
            } else {
                self.notify.notified().await
            }
        }
    }
}

use crate::socket::SocketId;
use crate::udt::SocketRef;
use crate::udt::Udt;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use tokio::io::Result;
use tokio::sync::{Notify, RwLock};
use tokio::time::{sleep_until, Instant};

#[derive(Debug, PartialEq, Eq, Clone)]
struct SendQueueNode {
    timestamp: Instant,
    socket_id: SocketId,
}

impl SendQueueNode {
    pub async fn socket(&self) -> Option<SocketRef> {
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
    sockets: RwLock<BinaryHeap<SendQueueNode>>,
    notify: Notify,
    start_time: Instant,
}

impl UdtSndQueue {
    pub fn new() -> Self {
        UdtSndQueue {
            sockets: RwLock::new(BinaryHeap::new()),
            notify: Notify::new(),
            start_time: Instant::now(),
        }
    }

    pub async fn worker(&self) -> Result<()> {
        loop {
            if let Some(node) = self.sockets.read().await.peek() {
                tokio::select! {
                   _ = sleep_until(node.timestamp) => {}
                   _ = self.notify.notified() => {}
                }
                if let Some(node) = self.sockets.write().await.pop() {
                    if let Some(socket) = node.socket().await {
                        if let Some((packet, ts)) = socket.write().await.next_data_packet().await? {
                            self.insert(ts, node.socket_id).await;
                            socket.read().await.send_packet(packet.into()).await?;
                        }
                    }
                }
            } else {
                self.notify.notified().await
            }
        }
    }

    pub async fn insert(&self, ts: Instant, socket_id: SocketId) {
        self.sockets.write().await.push(SendQueueNode {
            socket_id,
            timestamp: ts,
        });
        if let Some(node) = self.sockets.read().await.peek() {
            if node.socket_id == socket_id {
                self.notify.notify_waiters();
            }
        }
    }

    pub async fn update(&self, socket_id: SocketId, reschedule: bool) {
        let mut remove = false;
        let mut insert = false;
        {
            let mut sockets = self.sockets.write().await;
            if reschedule {
                if let Some(mut node) = sockets.peek_mut() {
                    if node.socket_id == socket_id {
                        node.timestamp = self.start_time;
                        self.notify.notify_waiters();
                        return;
                    }
                }
            }
            if !sockets.iter().any(|n| n.socket_id == socket_id) {
                insert = true;
            } else if reschedule {
                remove = true;
                insert = true;
            }
        }

        if remove {
            self.remove(socket_id).await;
        }
        if insert {
            self.insert(self.start_time, socket_id).await;
        }
    }

    pub async fn remove(&self, socket_id: SocketId) {
        let mut sockets = self.sockets.write().await;
        *sockets = sockets
            .iter()
            .filter(|n| n.socket_id != socket_id)
            .cloned()
            .collect();
    }
}

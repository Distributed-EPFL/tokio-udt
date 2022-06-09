use crate::socket::SocketId;
use crate::udt::SocketRef;
use crate::udt::Udt;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use tokio::io::Result;
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;
use tokio_timerfd::Delay;

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
    sockets: Mutex<BinaryHeap<SendQueueNode>>,
    notify: Notify,
    start_time: Instant,
}

impl UdtSndQueue {
    pub fn new() -> Self {
        UdtSndQueue {
            sockets: Mutex::new(BinaryHeap::new()),
            notify: Notify::new(),
            start_time: Instant::now(),
        }
    }

    pub async fn worker(&self) -> Result<()> {
        loop {
            let next_timestamp = {
                let sockets = self.sockets.lock().await;
                sockets.peek().map(|n| n.timestamp)
            };
            if let Some(timestamp) = next_timestamp {
                tokio::select! {
                   _ = Delay::new(timestamp.into_std())? => {}
                   _ = self.notify.notified() => {}
                }
                let next_node = { self.sockets.lock().await.pop() };
                if let Some(node) = next_node {
                    if let Some(socket) = node.socket().await {
                        if let Some((packet, ts)) = socket.next_data_packet().await? {
                            self.insert(ts, node.socket_id).await;
                            socket.send_packet(packet.into()).await?;
                        }
                    }
                }
            } else {
                self.notify.notified().await;
            }
        }
    }

    pub async fn insert(&self, ts: Instant, socket_id: SocketId) {
        self.sockets.lock().await.push(SendQueueNode {
            socket_id,
            timestamp: ts,
        });
        if let Some(node) = self.sockets.lock().await.peek() {
            if node.socket_id == socket_id {
                self.notify.notify_one();
            }
        }
    }

    pub async fn update(&self, socket_id: SocketId, reschedule: bool) {
        if reschedule {
            let mut sockets = self.sockets.lock().await;
            if let Some(mut node) = sockets.peek_mut() {
                if node.socket_id == socket_id {
                    node.timestamp = self.start_time;
                    self.notify.notify_one();
                    return;
                }
            };
        };
        if !self
            .sockets
            .lock()
            .await
            .iter()
            .any(|n| n.socket_id == socket_id)
        {
            self.insert(self.start_time, socket_id).await;
        } else if reschedule {
            self.remove(socket_id).await;
            self.insert(self.start_time, socket_id).await;
        }
    }

    pub async fn remove(&self, socket_id: SocketId) {
        let mut sockets = self.sockets.lock().await;
        *sockets = sockets
            .iter()
            .filter(|n| n.socket_id != socket_id)
            .cloned()
            .collect();
    }
}

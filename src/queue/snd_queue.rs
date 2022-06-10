use crate::socket::SocketId;
use crate::udt::SocketRef;
use crate::udt::Udt;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Mutex;
use tokio::io::Result;
use tokio::sync::Notify;
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
                let sockets = self.sockets.lock().unwrap();
                sockets.peek().map(|n| n.timestamp)
            };
            if let Some(timestamp) = next_timestamp {

                tokio::select! {
                   _ = Delay::new(timestamp.into_std())? => {}
                   _ = self.notify.notified() => {}
                }
                let next_node = { self.sockets.lock().unwrap().pop() };
                if let Some(node) = next_node {
                    if let Some(socket) = node.socket().await {
                        if let Some((packet, ts)) = socket.next_data_packet().await? {
                            self.insert(ts, node.socket_id);
                            socket.send_packet(packet.into()).await?;
                        }
                    }
                }
            } else {
                self.notify.notified().await;
            }
        }
    }

    pub fn insert(&self, ts: Instant, socket_id: SocketId) {
        let mut sockets = self.sockets.lock().unwrap();
        sockets.push(SendQueueNode {
            socket_id,
            timestamp: ts,
        });
        if let Some(node) = sockets.peek() {
            if node.socket_id == socket_id {
                self.notify.notify_one();
            }
        }
    }

    pub fn update(&self, socket_id: SocketId, reschedule: bool) {
        if reschedule {
            let mut sockets = self.sockets.lock().unwrap();
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
            .unwrap()
            .iter()
            .any(|n| n.socket_id == socket_id)
        {
            self.insert(self.start_time, socket_id);
        } else if reschedule {
            self.remove(socket_id);
            self.insert(self.start_time, socket_id);
        }
    }

    pub fn remove(&self, socket_id: SocketId) {
        let mut sockets = self.sockets.lock().unwrap();
        *sockets = sockets
            .iter()
            .filter(|n| n.socket_id != socket_id)
            .cloned()
            .collect();
    }
}

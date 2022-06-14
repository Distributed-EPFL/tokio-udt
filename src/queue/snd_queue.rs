use crate::socket::SocketId;
use crate::udt::SocketRef;
use crate::udt::Udt;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};
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
    queue: Mutex<BinaryHeap<SendQueueNode>>,
    notify: Notify,
    start_time: Instant,
    socket_refs: Mutex<BTreeMap<SocketId, SocketRef>>,
}

impl UdtSndQueue {
    pub fn new() -> Self {
        UdtSndQueue {
            queue: Mutex::new(BinaryHeap::new()),
            notify: Notify::new(),
            start_time: Instant::now(),
            socket_refs: Mutex::new(BTreeMap::new()),
        }
    }

    async fn get_socket(&self, socket_id: SocketId) -> Option<SocketRef> {
        let known_socket = self.socket_refs.lock().unwrap().get(&socket_id).cloned();
        if let Some(socket) = known_socket {
            Some(socket)
        } else if let Some(socket) = Udt::get().read().await.get_socket(socket_id) {
            self.socket_refs
                .lock()
                .unwrap()
                .insert(socket_id, socket.clone());
            Some(socket)
        } else {
            None
        }
    }

    pub async fn worker(&self) -> Result<()> {
        loop {
            let next_node = {
                let mut sockets = self.queue.lock().unwrap();
                let first_node = sockets.peek();
                match first_node {
                    Some(node) => {
                        if node.timestamp <= Instant::now() {
                            Ok(sockets.pop().unwrap())
                        } else {
                            Err(Some(node.timestamp))
                        }
                    }
                    None => Err(None),
                }
            };
            match next_node {
                Ok(node) => {
                    if let Some(socket) = self.get_socket(node.socket_id).await {
                        if let Some((packets, ts)) = socket.next_data_packets().await? {
                            self.insert(ts, node.socket_id);
                            socket.send_data_packets(packets).await?;
                        }
                    }
                }
                Err(Some(ts)) => {
                    tokio::select! {
                        _ = Delay::new(ts.into_std())? => {}
                        _ = self.notify.notified() => {}
                    }
                }
                _ => {
                    self.notify.notified().await;
                }
            }
        }
    }

    pub fn insert(&self, ts: Instant, socket_id: SocketId) {
        let mut sockets = self.queue.lock().unwrap();
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
            let mut sockets = self.queue.lock().unwrap();
            if let Some(mut node) = sockets.peek_mut() {
                if node.socket_id == socket_id {
                    node.timestamp = self.start_time;
                    self.notify.notify_one();
                    return;
                }
            };
        };
        if !self
            .queue
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
        let mut sockets = self.queue.lock().unwrap();
        *sockets = sockets
            .iter()
            .filter(|n| n.socket_id != socket_id)
            .cloned()
            .collect();
    }
}

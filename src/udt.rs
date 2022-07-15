use super::configuration::UdtConfiguration;
use crate::control_packet::{HandShakeInfo, UdtControlPacket};
use crate::multiplexer::{MultiplexerId, UdtMultiplexer};
use crate::seq_number::SeqNumber;
use crate::socket::{SocketId, SocketType, UdtSocket, UdtStatus};
use once_cell::sync::OnceCell;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, ErrorKind, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_timerfd::sleep;

pub(crate) type SocketRef = Arc<UdtSocket>;

pub(crate) static UDT_INSTANCE: OnceCell<RwLock<Udt>> = OnceCell::new();

#[derive(Default, Debug)]
pub(crate) struct Udt {
    sockets: BTreeMap<SocketId, SocketRef>,
    // closed_sockets: BTreeMap<SocketId, SocketRef>,
    multiplexers: BTreeMap<MultiplexerId, Arc<UdtMultiplexer>>,
    next_socket_id: SocketId,
    peers: BTreeMap<(SocketId, SeqNumber), BTreeSet<SocketId>>, // peer socket id -> local socket id
}

impl Udt {
    fn new() -> Self {
        Self {
            next_socket_id: rand::random(),
            ..Default::default()
        }
    }

    pub fn get() -> &'static RwLock<Self> {
        UDT_INSTANCE.get_or_init(|| {
            Udt::cleanup_worker();
            RwLock::new(Udt::new())
        })
    }

    fn get_new_socket_id(&mut self) -> SocketId {
        let socket_id = self.next_socket_id;
        self.next_socket_id = self.next_socket_id.wrapping_sub(1);
        socket_id
    }

    pub(crate) fn get_socket(&self, socket_id: SocketId) -> Option<SocketRef> {
        if let Some(socket) = self.sockets.get(&socket_id) {
            if socket.status() != UdtStatus::Closed {
                return Some(socket.clone());
            }
        }
        None
    }

    pub(crate) async fn get_peer_socket(
        &self,
        peer: SocketAddr,
        socket_id: SocketId,
        initial_seq_number: SeqNumber,
    ) -> Option<SocketRef> {
        for socket in self
            .peers
            .get(&(socket_id, initial_seq_number))?
            .iter()
            .filter_map(|id| self.sockets.get(id))
        {
            if socket.peer_addr() == Some(peer) {
                return self.sockets.get(&socket.socket_id).cloned();
            }
        }
        None
    }

    pub fn new_socket(
        &mut self,
        socket_type: SocketType,
        config: Option<UdtConfiguration>,
    ) -> Result<&SocketRef> {
        let socket = UdtSocket::new(self.get_new_socket_id(), socket_type, None, config);
        let socket_id = socket.socket_id;
        if let Entry::Vacant(e) = self.sockets.entry(socket_id) {
            return Ok(e.insert(Arc::new(socket)));
        }
        Err(Error::new(
            ErrorKind::AlreadyExists,
            "socket_id already exists",
        ))
    }

    pub(crate) async fn new_connection(
        &mut self,
        listener_socket: &UdtSocket,
        peer: SocketAddr,
        hs: &HandShakeInfo,
    ) -> Result<()> {
        if let Some(existing_peer_socket) = self
            .get_peer_socket(peer, hs.socket_id, hs.initial_seq_number)
            .await
        {
            let socket = existing_peer_socket;
            if socket.status() == UdtStatus::Broken {
                eprintln!("Existing connection to peer {} is broken", peer);
                // last connection from the "peer" address has been broken

                // *socket.status.lock().unwrap() = UdtStatus::Closed;
                /*  TODO:
                    Set timestamp? and remove from queued sockets and accept sockets?
                */
            } else {
                // Respond with existing socket configuration.
                let source_socket_id = hs.socket_id;
                let hs = {
                    let mut hs = hs.clone();
                    let configuration = socket.configuration.read().unwrap();
                    hs.initial_seq_number = socket.initial_seq_number;
                    hs.max_packet_size = configuration.mss;
                    hs.max_window_size = configuration.flight_flag_size;
                    hs.connection_type = -1;
                    hs.socket_id = socket.socket_id;
                    hs
                };
                let packet = UdtControlPacket::new_handshake(hs, source_socket_id);
                socket.send_to(&peer, packet.into()).await?;
                return Ok(());
            }
        }

        let new_socket_id = self.get_new_socket_id();

        let new_socket = {
            let multiplexer = listener_socket
                .multiplexer
                .read()
                .unwrap()
                .upgrade()
                .ok_or_else(|| Error::new(ErrorKind::Other, "Listener has no multiplexer"))?;

            let config = listener_socket.configuration.read().unwrap().clone();
            if listener_socket.queued_sockets.read().await.len() >= config.accept_queue_size {
                return Err(Error::new(ErrorKind::Other, "Too many queued sockets"));
            }

            let new_socket = UdtSocket::new(
                new_socket_id,
                hs.socket_type,
                Some(hs.initial_seq_number),
                Some(config),
            )
            .with_peer(peer, hs.socket_id)
            .with_listen_socket(listener_socket.socket_id, multiplexer);
            new_socket.open();
            new_socket
        };

        let ns_id = new_socket.socket_id;
        let ns_isn = new_socket.initial_seq_number;
        let ns_peer_socket_id = hs.socket_id;
        let new_socket_ref = new_socket.connect_on_handshake(peer, hs.clone()).await?;

        self.peers
            .entry((ns_peer_socket_id, ns_isn))
            .or_default()
            .insert(new_socket_ref.socket_id);
        self.sockets.insert(ns_id, new_socket_ref);

        listener_socket.queued_sockets.write().await.insert(ns_id);
        listener_socket.accept_notify.notify_one();
        Ok(())
    }

    pub async fn bind(&mut self, socket_id: SocketId, addr: SocketAddr) -> Result<()> {
        let socket = self
            .get_socket(socket_id)
            .ok_or_else(|| Error::new(ErrorKind::Other, "unknown socket id"))?;

        if socket.status() != UdtStatus::Init {
            return Err(Error::new(ErrorKind::Other, "socket already binded"));
        }

        self.update_mux(&socket, Some(addr)).await?;
        socket.open();
        Ok(())
    }

    pub(crate) async fn update_mux(
        &mut self,
        socket: &UdtSocket,
        bind_addr: Option<SocketAddr>,
    ) -> Result<()> {
        if socket.configuration.read().unwrap().reuse_mux {
            if let Some(bind_addr) = bind_addr {
                let port = bind_addr.port();
                for mux in self.multiplexers.values() {
                    let socket_mss = socket.configuration.read().unwrap().mss;
                    if mux.reusable && mux.port == port && mux.mss == socket_mss {
                        socket.set_multiplexer(mux);
                        return Ok(());
                    }
                }
            }
        }

        // A new multiplexer is needed
        let mux = {
            let configuration = socket.configuration.read().unwrap().clone();
            let (mux_id, mux) = if let Some(bind_addr) = bind_addr {
                UdtMultiplexer::bind(socket.socket_id, bind_addr, &configuration).await?
            } else {
                UdtMultiplexer::new(socket.socket_id, &configuration).await?
            };
            self.multiplexers.insert(mux_id, mux.clone());
            mux
        };
        socket.set_multiplexer(&mux);
        UdtMultiplexer::run(mux);
        Ok(())
    }

    async fn remove_broken_sockets(&mut self) {
        for (_, sock) in self
            .sockets
            .iter()
            .filter(|(_, s)| s.status() == UdtStatus::Broken)
        {
            if let Some(listen_socket_id) = sock.listen_socket {
                if let Some(listener) = self.sockets.get(&listen_socket_id) {
                    listener
                        .queued_sockets
                        .write()
                        .await
                        .remove(&sock.socket_id);
                }
            }
            tokio::spawn({
                let sock = sock.clone();
                async move { sock.close().await }
            });
        }

        let to_remove: Vec<_> = self
            .sockets
            .iter()
            .filter(|(_, s)| s.status() == UdtStatus::Closing)
            .map(|(socket_id, _)| *socket_id)
            .collect();
        for socket_id in to_remove {
            if let Some(sock) = self.sockets.remove(&socket_id) {
                *sock.status.lock().unwrap() = UdtStatus::Closed;
            }
        }
    }

    fn cleanup_worker() {
        tokio::spawn(async {
            let udt = Self::get();
            loop {
                udt.write().await.remove_broken_sockets().await;
                sleep(std::time::Duration::from_secs(1)).await.unwrap();
            }
        });
    }
}

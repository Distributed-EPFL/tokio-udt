use super::configuration::UdtConfiguration;
use crate::control_packet::HandShakeInfo;
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

pub(crate) type SocketRef = Arc<UdtSocket>;

pub(crate) static UDT_INSTANCE: OnceCell<RwLock<Udt>> = OnceCell::new();

#[derive(Default, Debug)]
pub(crate) struct Udt {
    sockets: BTreeMap<SocketId, SocketRef>,
    // closed_sockets: BTreeMap<SocketId, SocketRef>,
    multiplexers: BTreeMap<MultiplexerId, Arc<UdtMultiplexer>>,
    next_socket_id: SocketId,
    pub configuration: UdtConfiguration,
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
        UDT_INSTANCE.get_or_init(|| RwLock::new(Udt::new()))
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
        backlog_size: usize,
    ) -> Result<&SocketRef> {
        let mut socket = UdtSocket::new(self.get_new_socket_id(), socket_type, None);
        socket.backlog_size = backlog_size;
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
                // last connection from the "peer" address has been broken
                *socket.status.lock().unwrap() = UdtStatus::Closed;
                /*  TODO:
                    Set timestamp? and remove from queued sockets and accept sockets?
                */
            } else {
                /*  TODO:
                    Respond with existing socket configuration.
                    Mutate handshake info?
                */
                unimplemented!()
            }
        }

        let new_socket_id = self.get_new_socket_id();

        let new_socket = {
            let multiplexer = listener_socket
                .multiplexer
                .lock()
                .unwrap()
                .upgrade()
                .ok_or_else(|| Error::new(ErrorKind::Other, "Listener has no multiplexer"))?;

            if listener_socket.queued_sockets.read().await.len() >= listener_socket.backlog_size {
                return Err(Error::new(ErrorKind::Other, "Too many queued sockets"));
            }

            let new_socket =
                UdtSocket::new(new_socket_id, hs.socket_type, Some(hs.initial_seq_number))
                    .with_peer(peer, hs.socket_id)
                    .await
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
        if self.configuration.reuse_addr {
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
}

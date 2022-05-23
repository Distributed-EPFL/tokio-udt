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

pub(crate) type SocketRef = Arc<RwLock<UdtSocket>>;

pub static UDT_INSTANCE: OnceCell<RwLock<Udt>> = OnceCell::new();

#[derive(Default, Debug)]
pub struct Udt {
    sockets: BTreeMap<SocketId, SocketRef>,
    closed_sockets: BTreeMap<SocketId, SocketRef>,
    multiplexers: BTreeMap<MultiplexerId, Arc<RwLock<UdtMultiplexer>>>,
    next_socket_id: SocketId,
    configuration: UdtConfiguration,
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

    pub(crate) async fn get_socket(&self, socket_id: SocketId) -> Option<SocketRef> {
        if let Some(lock) = self.sockets.get(&socket_id) {
            let socket = lock.read().await;
            if socket.status != UdtStatus::Closed {
                return Some(lock.clone());
            }
        }
        None
    }

    pub(crate) async fn get_peer_socket(
        &mut self,
        peer: SocketAddr,
        socket_id: SocketId,
        initial_seq_number: SeqNumber,
    ) -> Option<SocketRef> {
        for socket_lock in self
            .peers
            .get(&(socket_id, initial_seq_number))?
            .iter()
            .filter_map(|id| self.sockets.get(id))
        {
            let socket = socket_lock.read().await;
            if socket.peer_addr == Some(peer) {
                return self.sockets.get(&socket.socket_id).cloned();
            }
        }
        None
    }

    pub fn new_socket(&'static mut self, socket_type: SocketType) -> Result<&'static SocketRef> {
        let socket = UdtSocket::new(self.get_new_socket_id(), socket_type);
        let socket_id = socket.socket_id;
        if let Entry::Vacant(e) = self.sockets.entry(socket_id) {
            return Ok(e.insert(Arc::new(RwLock::new(socket))));
        }
        Err(Error::new(
            ErrorKind::AlreadyExists,
            "socket_id already exists",
        ))
    }

    pub(crate) async fn new_connection(
        &mut self,
        listener_socket_id: SocketId,
        peer: SocketAddr,
        hs: &HandShakeInfo,
    ) -> Result<()> {
        if let Some(existing_peer_socket) = self
            .get_peer_socket(peer, hs.socket_id, hs.initial_seq_number)
            .await
        {
            let mut socket = existing_peer_socket.write().await;
            if socket.status == UdtStatus::Broken {
                // last connection from the "peer" address has been broken
                socket.status = UdtStatus::Closed;
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

        let listener_socket = self
            .sockets
            .get(&listener_socket_id)
            .ok_or_else(|| Error::new(ErrorKind::Other, "Failed to find listener socket"))?
            .clone();

        let new_socket = {
            let listener_socket = listener_socket.write().await;

            let multiplexer = listener_socket
                .multiplexer
                .upgrade()
                .ok_or_else(|| Error::new(ErrorKind::Other, "Listener has no multiplexer"))?;

            if listener_socket.backlog_size >= listener_socket.queued_sockets.len() {
                return Err(Error::new(ErrorKind::Other, "Too many queued sockets"));
            }

            let mut new_socket = UdtSocket::new(new_socket_id, hs.socket_type)
                .with_peer(peer, hs.socket_id)
                .with_listen_socket(listener_socket_id, multiplexer)
                .with_initial_seq_number(hs.initial_seq_number);
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
            .insert(new_socket_ref.read().await.socket_id);
        self.sockets.insert(ns_id, new_socket_ref);
        listener_socket.write().await.queued_sockets.insert(ns_id);

        // TODO: Trigger event? Unblock "accept" in listener socket?

        Ok(())
    }

    pub async fn bind(&'static mut self, socket_id: SocketId, addr: SocketAddr) -> Result<()> {
        let socket = self
            .get_socket(socket_id)
            .await
            .ok_or_else(|| Error::new(ErrorKind::Other, "unknown socket id"))?;

        let mut socket = socket.write().await;

        if socket.status != UdtStatus::Init {
            return Err(Error::new(ErrorKind::Other, "socket already binded"));
        }

        self.update_mux(&mut socket, addr).await?;
        // TODO: continue

        Ok(())
    }

    pub(crate) async fn update_mux(
        &'static mut self,
        socket: &mut UdtSocket,
        bind_addr: SocketAddr,
    ) -> Result<()> {
        if self.configuration.reuse_addr {
            let port = bind_addr.port();
            for m in self.multiplexers.values() {
                let mux = m.read().await;
                if mux.reusable && mux.port == port && mux.mss == socket.configuration.mss {
                    socket.set_multiplexer(m);
                    return Ok(());
                }
            }
        }

        // A new multiplexer is needed
        let (mux_id, mux) =
            UdtMultiplexer::bind(socket.socket_id, bind_addr, &socket.configuration).await?;
        self.multiplexers.insert(mux_id, mux.clone());
        socket.set_multiplexer(&mux);
        tokio::spawn(async move {
            UdtMultiplexer::run(mux).await;
        });
        Ok(())
    }
}

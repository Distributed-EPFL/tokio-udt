use super::configuration::UdtConfiguration;
use crate::control_packet::HandShakeInfo;
use crate::multiplexer::{MultiplexerId, UdtMultiplexer};
use crate::seq_number::SeqNumber;
use crate::socket::{SocketId, SocketType, UdtSocket, UdtStatus};
use std::cell::RefCell;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, ErrorKind, Result};
use std::net::SocketAddr;
use std::rc::Rc;

pub(crate) type SocketRef = Rc<RefCell<UdtSocket>>;

thread_local! {
    pub static UDT_INSTANCE: Rc<RefCell<Udt>> = Rc::new(RefCell::new(Udt::new()));
}

#[derive(Default, Debug)]
pub struct Udt {
    sockets: BTreeMap<SocketId, SocketRef>,
    closed_sockets: BTreeMap<SocketId, SocketRef>,
    multiplexers: BTreeMap<MultiplexerId, Rc<RefCell<UdtMultiplexer>>>,
    next_socket_id: SocketId,
    configuration: UdtConfiguration,
    peers: BTreeMap<(SocketId, SeqNumber), BTreeSet<SocketRef>>,
}

impl Udt {
    fn new() -> Self {
        Self {
            next_socket_id: rand::random(),
            ..Default::default()
        }
    }

    pub fn get() -> Rc<RefCell<Self>> {
        UDT_INSTANCE.with(|udt| udt.clone())
    }

    fn get_new_socket_id(&mut self) -> SocketId {
        let socket_id = self.next_socket_id;
        self.next_socket_id = self.next_socket_id.wrapping_sub(1);
        socket_id
    }

    pub(crate) fn get_socket(&self, socket_id: SocketId) -> Option<Rc<RefCell<UdtSocket>>> {
        self.sockets
            .get(&socket_id)
            .filter(|s| s.borrow().status != UdtStatus::Closed)
            .cloned()
    }

    pub(crate) fn get_peer_socket(
        &mut self,
        peer: SocketAddr,
        socket_id: SocketId,
        initial_seq_number: SeqNumber,
    ) -> Option<SocketRef> {
        self.peers
            .get(&(socket_id, initial_seq_number))?
            .iter()
            .find(|s| s.borrow().peer_addr == Some(peer))
            .map(|s| self.sockets.get(&s.borrow().socket_id))?
            .cloned()
    }

    pub(crate) fn new_socket(&mut self, socket_type: SocketType) -> Result<&SocketRef> {
        let socket = UdtSocket::new(self.get_new_socket_id(), socket_type);
        let socket_id = socket.socket_id;
        if let Entry::Vacant(e) = self.sockets.entry(socket_id) {
            return Ok(e.insert(Rc::new(RefCell::new(socket))));
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
        if let Some(existing_peer_socket) =
            self.get_peer_socket(peer, hs.socket_id, hs.initial_seq_number)
        {
            let mut socket = existing_peer_socket.borrow_mut();
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

        let listener_socket = self
            .sockets
            .get(&listener_socket_id)
            .ok_or_else(|| Error::new(ErrorKind::Other, "Failed to find listener socket"))?
            .clone();

        if listener_socket.borrow().backlog_size >= listener_socket.borrow().queued_sockets.len() {
            return Err(Error::new(ErrorKind::Other, "Too many queued sockets"));
        }

        let new_socket_id = self.get_new_socket_id();
        let mut new_socket = UdtSocket::new(new_socket_id, hs.socket_type)
            .with_peer(peer, hs.socket_id)
            .with_listen_socket(&listener_socket.borrow())
            .with_initial_seq_number(hs.initial_seq_number);
        new_socket.open();

        let ns_id = new_socket.socket_id;
        let ns_isn = new_socket.initial_seq_number;
        let ns_peer_socket_id = hs.socket_id;
        let new_socket_rc = new_socket.connect_on_handshake(peer, hs.clone()).await?;
        self.peers
            .entry((ns_peer_socket_id, ns_isn))
            .or_default()
            .insert(new_socket_rc.clone());
        self.sockets.insert(ns_id, new_socket_rc);
        listener_socket.borrow_mut().queued_sockets.insert(ns_id);

        // TODO: Trigger event? Unblock "accept" in listener socket?

        Ok(())
    }

    pub async fn bind(&mut self, socket_id: SocketId, addr: SocketAddr) -> Result<()> {
        let socket = self
            .get_socket(socket_id)
            .ok_or_else(|| Error::new(ErrorKind::Other, "unknown socket id"))?;
        let mut socket = socket.borrow_mut();

        if socket.status != UdtStatus::Init {
            return Err(Error::new(ErrorKind::Other, "socket already binded"));
        }

        self.update_mux(&mut socket, addr).await?;
        // TODO: continue

        Ok(())
    }

    pub(crate) async fn update_mux(
        &mut self,
        socket: &mut UdtSocket,
        bind_addr: SocketAddr,
    ) -> Result<()> {
        if self.configuration.reuse_addr {
            let port = bind_addr.port();
            if let Some(mux) = self.multiplexers.values().find(|m| {
                let mux = m.borrow();
                mux.reusable && mux.port == port && mux.mss == socket.configuration.mss
            }) {
                socket.set_multiplexer(mux.clone());
                return Ok(());
            }
        }

        // A new multiplexer is needed
        let mux_rc =
            UdtMultiplexer::bind(socket.socket_id, bind_addr, &socket.configuration).await?;
        let mux_id = mux_rc.borrow().id;
        self.multiplexers.insert(mux_id, mux_rc.clone());
        socket.set_multiplexer(mux_rc);
        Ok(())
    }
}

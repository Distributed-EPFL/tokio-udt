use super::configuration::UdtConfiguration;
use crate::control_packet::HandShakeInfo;
use crate::multiplexer::{MultiplexerId, UdtMultiplexer};
use crate::socket::{SocketId, SocketType, UdtSocket, UdtStatus};
use std::cell::RefCell;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, ErrorKind, Result};
use std::net::SocketAddr;
use std::ops::DerefMut;
use std::rc::Rc;

#[derive(Default)]
struct Udt<'a> {
    sockets: BTreeMap<SocketId, Rc<RefCell<UdtSocket>>>,
    closed_sockets: BTreeMap<SocketId, &'a UdtSocket>,
    multiplexers: BTreeMap<MultiplexerId, Rc<RefCell<UdtMultiplexer>>>,
    next_socket_id: SocketId,
    configuration: UdtConfiguration,
    peers: BTreeMap<(SocketId, u32), BTreeSet<&'a UdtSocket>>,
}

impl Udt<'_> {
    fn new() -> Self {
        Self {
            next_socket_id: rand::random(),
            ..Default::default()
        }
    }

    fn get_new_socket_id(&mut self) -> SocketId {
        let socket_id = self.next_socket_id;
        self.next_socket_id = self.next_socket_id.wrapping_sub(1);
        socket_id
    }

    pub fn get_socket(&self, socket_id: SocketId) -> Option<Rc<RefCell<UdtSocket>>> {
        self.sockets
            .get(&socket_id)
            .filter(|s| s.borrow().status != UdtStatus::Closed)
            .cloned()
    }

    pub fn get_peer_socket(
        &mut self,
        peer: SocketAddr,
        socket_id: SocketId,
        initial_seq_number: u32,
    ) -> Option<Rc<RefCell<UdtSocket>>> {
        self.peers
            .get(&(socket_id, initial_seq_number))?
            .iter()
            .find(|s| s.peer_addr == Some(peer))
            .map(|s| self.sockets.get(&s.socket_id))?
            .cloned()
    }

    pub fn new_socket(&mut self, socket_type: SocketType) -> Result<&Rc<RefCell<UdtSocket>>> {
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

    pub fn new_connection(
        &mut self,
        listener_socket_id: SocketId,
        peer: SocketAddr,
        hs: HandShakeInfo,
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
            .borrow();

        if listener_socket.backlog_size >= listener_socket.queued_sockets.len() {
            return Err(Error::new(ErrorKind::Other, "Too many queued sockets"));
        }
        let listener_port = listener_socket
            .self_addr
            .map(|listener_addr| listener_addr.port());

        drop(listener_socket);

        let mut new_socket = UdtSocket::new(self.get_new_socket_id(), hs.socket_type)
            .with_peer(peer, hs.socket_id)
            .with_listen_socket(listener_socket_id)
            .with_initial_seq_number(hs.initial_seq_number);
        new_socket.open();

        if let Some(port) = listener_port {
            if let Some(mux) = self
                .multiplexers
                .values()
                .find(|mux| mux.borrow().port == port)
            {
                new_socket.set_multiplexer(mux.clone());
            }
        }

        Ok(())
    }
}

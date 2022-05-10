use super::configuration::UdtConfiguration;
use crate::control_packet::HandShakeInfo;
use crate::multiplexer::{MultiplexerId, UDTMultiplexer};
use crate::socket::{SocketId, SocketType, UdtSocket, UdtStatus};
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::io::{Error, ErrorKind, Result};
use std::net::SocketAddr;

#[derive(Default)]
struct Udt<'a> {
    sockets: BTreeMap<SocketId, UdtSocket>,
    closed_sockets: BTreeMap<SocketId, &'a UdtSocket>,
    multiplexers: BTreeMap<MultiplexerId, UDTMultiplexer<'a>>,
    next_socket_id: SocketId,
    configuration: UdtConfiguration,
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

    pub fn get_socket(&self, socket_id: SocketId) -> Option<&UdtSocket> {
        self.sockets
            .get(&socket_id)
            .filter(|s| s.status != UdtStatus::Closed)
    }

    pub fn new_socket(&mut self, socket_type: SocketType) -> Result<&UdtSocket> {
        let socket = UdtSocket::new(self.get_new_socket_id(), socket_type);
        let socket_id = socket.socket_id;
        if let Entry::Vacant(e) = self.sockets.entry(socket_id) {
            return Ok(e.insert(socket));
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
        let listener_socket = self.sockets.get(&listener_socket_id).ok_or(Error::new(
            ErrorKind::Other,
            "Faild to find listener socket",
        ))?;
        Ok(())
    }
}

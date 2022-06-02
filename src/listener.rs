use crate::connection::UdtConnection;
use crate::socket::{SocketType, UdtStatus};
use crate::udt::{SocketRef, Udt};
use std::net::SocketAddr;
use tokio::io::{Error, ErrorKind, Result};

pub struct UdtListener {
    socket: SocketRef,
}

impl UdtListener {
    pub async fn new(bind_addr: SocketAddr, backlog: usize) -> Result<Self> {
        let socket = {
            let mut udt = Udt::get().write().await;
            udt.new_socket(SocketType::Stream)?.clone()
        };

        if socket.read().await.configuration.rendezvous {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "listen is not supported in rendezvous connection setup",
            ));
        }

        let socket_id = socket.read().await.socket_id;

        {
            let mut udt = Udt::get().write().await;
            udt.bind(socket_id, bind_addr).await?;
        }

        {
            let socket_ref = socket.clone();
            let mut socket = socket.write().await;
            socket.backlog_size = backlog;
            let mux_lock = socket
                .multiplexer
                .upgrade()
                .expect("multiplexer is not initialized");
            let mut mux = mux_lock.write().await;
            mux.listener = Some(socket_ref);
            socket.status = UdtStatus::Listening;
        }

        Ok(Self { socket })
    }

    pub async fn accept(&self) -> Result<(SocketAddr, UdtConnection)> {
        let socket = self.socket.read().await;
        if socket.configuration.rendezvous {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "no 'accept' in rendezvous connection setup",
            ));
        }

        let accepted_socket_id = loop {
            let mut socket = self.socket.write().await;
            if socket.status != UdtStatus::Listening {
                return Err(Error::new(
                    ErrorKind::Other,
                    "socket is not in listening state",
                ));
            }

            if let Some(socket_id) = socket.queued_sockets.iter().next() {
                let socket_id = *socket_id;
                socket.queued_sockets.remove(&socket_id);
                break socket_id;
            }

            {
                let socket = self.socket.read().await;
                socket.accept_notify.notified().await;
            }
        };

        let udt = Udt::get().read().await;
        let accepted_socket = udt.get_socket(accepted_socket_id).await.ok_or_else(|| {
            Error::new(
                ErrorKind::Other,
                "invalid socket id when accepting connection",
            )
        })?;

        let peer_addr = accepted_socket.read().await.peer_addr.ok_or_else(|| {
            Error::new(
                ErrorKind::Other,
                "unknown peer address for accepted connection",
            )
        })?;

        Ok((peer_addr, UdtConnection::new(accepted_socket)))
    }
}

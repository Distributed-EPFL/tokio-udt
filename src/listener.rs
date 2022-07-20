use crate::configuration::UdtConfiguration;
use crate::connection::UdtConnection;
use crate::socket::{SocketType, UdtStatus};
use crate::udt::{SocketRef, Udt};
use std::net::SocketAddr;
use tokio::io::{Error, ErrorKind, Result};

pub struct UdtListener {
    socket: SocketRef,
}

impl UdtListener {
    pub async fn bind(bind_addr: SocketAddr, config: Option<UdtConfiguration>) -> Result<Self> {
        let socket = {
            let mut udt = Udt::get().write().await;
            udt.new_socket(SocketType::Stream, config)?.clone()
        };

        if socket.configuration.read().unwrap().rendezvous {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "listen is not supported in rendezvous connection setup",
            ));
        }

        let socket_id = socket.socket_id;

        {
            let mut udt = Udt::get().write().await;
            udt.bind(socket_id, bind_addr).await?;
        }

        {
            let socket_ref = socket.clone();
            let mux = socket
                .multiplexer()
                .expect("multiplexer is not initialized");
            *mux.listener.write().await = Some(socket_ref);
            *socket.status.lock().unwrap() = UdtStatus::Listening;

            println!("Now listening on {:?}", bind_addr);
        }

        Ok(Self { socket })
    }

    pub async fn accept(&self) -> Result<(SocketAddr, UdtConnection)> {
        {
            if self.socket.configuration.read().unwrap().rendezvous {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "no 'accept' in rendezvous connection setup",
                ));
            }
        }

        let accepted_socket_id = loop {
            let notified = {
                if self.socket.status() != UdtStatus::Listening {
                    return Err(Error::new(
                        ErrorKind::Other,
                        "socket is not in listening state",
                    ));
                }

                let mut queue = self.socket.queued_sockets.write().await;
                if let Some(socket_id) = queue.iter().next() {
                    let socket_id = *socket_id;
                    queue.remove(&socket_id);
                    break socket_id;
                }
                self.socket.accept_notify.notified()
            };
            notified.await
        };

        let udt = Udt::get().read().await;
        let accepted_socket = udt.get_socket(accepted_socket_id).ok_or_else(|| {
            Error::new(
                ErrorKind::Other,
                "invalid socket id when accepting connection",
            )
        })?;

        let peer_addr = accepted_socket.peer_addr().ok_or_else(|| {
            Error::new(
                ErrorKind::Other,
                "unknown peer address for accepted connection",
            )
        })?;

        Ok((peer_addr, UdtConnection::new(accepted_socket)))
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket.multiplexer().unwrap().channel.local_addr()
    }

    pub fn socket_id(&self) -> u32 {
        self.socket.socket_id
    }
}

use crate::socket::{SocketType, UdtStatus};
use crate::udt::{SocketRef, Udt};
use std::net::SocketAddr;
use tokio::io::Result;

pub struct UdtConnection {
    socket: SocketRef,
}

impl UdtConnection {
    pub(crate) fn new(socket: SocketRef) -> Self {
        Self { socket }
    }

    pub async fn connect(addr: SocketAddr) -> Result<Self> {
        let socket = {
            let mut udt = Udt::get().write().await;
            udt.new_socket(SocketType::Stream)?.clone()
        };
        {
            let mut socket = socket.write().await;
            socket.connect(addr).await?;
        }
        let connection = Self {
            socket: socket.clone(),
        };
        loop {
            if connection.socket.read().await.status().await == UdtStatus::Connected {
                break;
            }
            connection
                .socket
                .read()
                .await
                .connect_notify
                .notified()
                .await;
        }
        Ok(connection)
    }

    pub async fn send(&self, msg: &[u8]) -> Result<()> {
        let socket = self.socket.read().await;
        socket.send(msg).await?;
        Ok(())
    }

    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let socket = self.socket.read().await;
        let nbytes = socket.recv(buf).await?;
        Ok(nbytes)
    }
}

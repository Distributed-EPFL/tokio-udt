use crate::socket::{SocketType, UdtStatus};
use crate::udt::{SocketRef, Udt};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, Result};

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
            udt.new_socket(SocketType::Stream, 10)?.clone()
        };
        socket.connect(addr).await?;
        let connection = Self::new(socket.clone());
        loop {
            if connection.socket.status() == UdtStatus::Connected {
                break;
            }
            connection.socket.connect_notify.notified().await;
        }
        Ok(connection)
    }

    pub async fn send(&self, msg: &[u8]) -> Result<()> {
        self.socket.send(msg).await?;
        Ok(())
    }

    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let nbytes = self.socket.recv(buf).await?;
        Ok(nbytes)
    }

    pub async fn recv_buf(&self, buf: &mut ReadBuf<'_>) -> Result<()> {
        println!("Recv buf...");

        let rcv_buf = buf.initialize_unfilled();
        println!("Rcv buf size {}", rcv_buf.len());
        let nbytes = self.socket.recv(rcv_buf).await?;
        if nbytes > 0 {
            buf.advance(nbytes);
        }
        println!("Recv buf done.");
        Ok(())
    }
}

impl AsyncRead for UdtConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        match self.socket.poll_recv(buf) {
            Poll::Ready(_) => Poll::Ready(Ok(())),
            Poll::Pending => {
                let waker = cx.waker().clone();
                let socket = self.socket.clone();
                tokio::spawn(async move {
                    socket.rcv_notify.notified().await;
                    waker.wake();
                });
                Poll::Pending
            }
        }
    }
}

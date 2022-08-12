/*!
An implementation of UDP-based Data Transfer Protocol (UDT) based on Tokio primitives.

UDT is a high performance data transport protocol. It was designed for data intensive
applications over high speed wide area networks, to overcome the efficiency and fairness
problems of TCP. As its names indicates, UDT is built on top of UDP and it provides both
reliable data streaming and messaging services.


# Usage

##  UDT server example

Bind a port with [`UdtListener`]:

```no_run
use std::net::Ipv4Addr;
use tokio::io::{AsyncReadExt, Result};
use tokio_udt::UdtListener;

#[tokio::main]
async fn main() -> Result<()> {
    let port = 9000;
    let listener = UdtListener::bind((Ipv4Addr::UNSPECIFIED, port).into(), None).await?;

    println!("Waiting for connections...");

    loop {
        let (addr, mut connection) = listener.accept().await?;
        println!("Accepted connection from {}", addr);
        let mut buffer = Vec::with_capacity(1_000_000);
        tokio::task::spawn({
            async move {
                loop {
                    match connection.read_buf(&mut buffer).await {
                        Ok(_size) => {}
                        Err(e) => {
                            eprintln!("Connnection with {} failed: {}", addr, e);
                            break;
                        }
                    }
                }
            }
        });
    }
}
```

## UDT client example

Open a connection with [`UdtConnection`]

```no_run
use std::net::Ipv4Addr;
use tokio::io::{AsyncWriteExt, Result};
use tokio_udt::UdtConnection;

#[tokio::main]
async fn main() -> Result<()> {
    let port = 9000;
    let mut connection = UdtConnection::connect((Ipv4Addr::LOCALHOST, port), None).await?;
    loop {
        connection.write_all(b"Hello World!").await?;
    }
}
```
*/
mod ack_window;
mod common;
mod configuration;
mod connection;
mod control_packet;
mod data_packet;
mod flow;
mod listener;
mod loss_list;
mod multiplexer;
mod packet;
mod queue;
mod rate_control;
mod seq_number;
mod socket;
mod state;
mod udt;

pub use configuration::UdtConfiguration;
pub use connection::UdtConnection;
pub use listener::UdtListener;
pub use rate_control::RateControl;
pub use seq_number::SeqNumber;

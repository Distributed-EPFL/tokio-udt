# tokio-udt

 An implementation of UDP-based Data Transfer Protocol (UDT) based on Tokio primitives.

[![Crates.io][crates-badge]][crates-url]
[![Docs][docs-badge]][docs-url]

[crates-badge]: https://img.shields.io/crates/v/tokio-udt.svg
[crates-url]: https://crates.io/crates/tokio-udt
[docs-badge]: https://img.shields.io/docsrs/tokio-udt.svg
[docs-url]: https://docs.rs/tokio-udt/

## What is UDT?

UDT is a high performance data transport protocol. It was designed specifically for data intensive
applications over high speed wide area networks, to overcome the efficiency and fairness
problems of TCP. As its names indicates, UDT is built on top of UDP and it provides both
reliable data streaming and messaging services.

To learn more about UDT, see https://udt.sourceforge.io/


## Examples

### UDT listener

```rust,no_run
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

### UDT client

```rust,no_run
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

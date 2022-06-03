use std::time::{Duration, Instant};
use tokio::io::ErrorKind;
use tokio::time::sleep;
use tokio_udt::UdtConnection;

#[tokio::main]
async fn main() {
    // console_subscriber::init();

    let connection = UdtConnection::connect("127.0.0.1:9000".parse().unwrap())
        .await
        .unwrap();

    let buffer: Vec<u8> = std::iter::repeat(b"Hello World!")
        .take(10000)
        .flat_map(|b| *b)
        .collect();
    println!("Message length: {}", buffer.len());

    let mut last = Instant::now();

    sleep(Duration::from_secs(3)).await;
    let mut count = 0;

    loop {
        connection
            .send(&buffer[..])
            .await
            .map(|_| {
                count += 1;
            })
            .or_else(|err| match err.kind() {
                ErrorKind::OutOfMemory => Ok(()),
                _ => Err(err),
            })
            .unwrap();

        if last.elapsed() > Duration::new(1, 0) {
            last = Instant::now();

            println!("Sent {} messages", count);
        }
    }
}

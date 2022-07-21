use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio_udt::UdtConnection;

#[tokio::main]
async fn main() {
    let mut connection = UdtConnection::connect("127.0.0.1:9000", None)
        .await
        .unwrap();

    println!("Connected!");

    let buffer: Vec<u8> = std::iter::repeat(b"Hello World!")
        .take(100000)
        .flat_map(|b| *b)
        .collect();
    println!("Message length: {}", buffer.len());

    let mut last = Instant::now();
    let mut count = 0;

    loop {
        connection
            .write_all(&buffer)
            .await
            .map(|_| {
                count += 1;
            })
            .unwrap();

        if last.elapsed() > Duration::new(1, 0) {
            last = Instant::now();
            println!("Sent {} messages", count);
            println!(
                "Period {:?}",
                connection.rate_control().get_pkt_send_period()
            );
            println!(
                "Window {:?}",
                connection.rate_control().get_congestion_window_size()
            );
        }
    }

    // connection.close().await
}

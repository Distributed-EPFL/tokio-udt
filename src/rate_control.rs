use crate::seq_number::SeqNumber;
use crate::socket::SYN_INTERVAL;
use tokio::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct RateControl {
    pkt_send_period: Duration,
    congestion_window_size: f64,

    rc_interval: Duration,
    last_rate_increase: Instant,
    slow_start: bool,
    last_ack: SeqNumber,
    loss: bool, // has lost happeneind since last rate increase
    last_dec_seq: SeqNumber,
    last_dec_period: Duration,
    nak_count: usize,
    dec_random: usize,
    avg_nak_num: usize,
    dec_count: usize,
}

impl RateControl {
    pub fn new() -> Self {
        Self {
            pkt_send_period: Duration::from_micros(1),
            congestion_window_size: 16.0,

            rc_interval: SYN_INTERVAL,
            last_rate_increase: Instant::now(),
            slow_start: true,
            last_ack: SeqNumber::zero(),
            loss: false,
            last_dec_seq: SeqNumber::zero() - 1,
            last_dec_period: Duration::from_micros(1),
            nak_count: 0,
            avg_nak_num: 0,
            dec_random: 0,
            dec_count: 0,
        }
    }
}

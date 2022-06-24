use crate::packet::UdtPacket;
use crate::seq_number::SeqNumber;
use crate::socket::SYN_INTERVAL;
use tokio::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct RateControl {
    pkt_send_period: Duration,
    congestion_window_size: f64,
    recv_rate: usize,
    rtt: Duration,

    curr_snd_seq_number: SeqNumber,
    rc_interval: Duration,
    last_rate_increase: Instant,
    slow_start: bool,
    last_ack: SeqNumber,
    loss: bool, // has lost happenened since last rate increase
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
            recv_rate: 0,
            rtt: Duration::default(),

            curr_snd_seq_number: SeqNumber::zero(),
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

    pub fn init(&mut self) {
        unimplemented!()
    }

    pub fn get_pkt_send_period(&self) -> Duration {
        self.pkt_send_period
    }

    pub fn get_congestion_window_size(&self) -> u32 {
        self.congestion_window_size as u32
    }

    pub fn get_ack_pkts_interval(&self) -> usize {
        unimplemented!()
    }

    pub fn get_ack_period(&self) -> Duration {
        unimplemented!()
    }

    pub fn set_rtt(&mut self, rtt: Duration) {
        unimplemented!()
    }

    pub fn set_rcv_rate(&mut self, pkt_per_sec: usize) {
        unimplemented!()
    }

    pub fn set_bandwidth(&mut self, pkt_per_sec: usize) {
        unimplemented!()
    }

    pub fn on_ack(&mut self, ack: SeqNumber) {
        unimplemented!()
    }

    pub fn on_loss(&mut self, loss_list: &[SeqNumber]) {
        unimplemented!()
    }

    pub fn set_curr_snd_seq_number(&mut self, seq: SeqNumber) {
        self.curr_snd_seq_number = seq;
    }

    pub fn on_timeout(&mut self) {
        if self.slow_start {
            self.slow_start = false;
            if self.recv_rate > 0 {
                self.pkt_send_period = Duration::from_secs_f64(1.0 / self.recv_rate as f64);
            } else {
                self.pkt_send_period =
                    (self.rtt + self.rc_interval).div_f64(self.congestion_window_size);
            }
        }
    }
}

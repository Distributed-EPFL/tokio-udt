use crate::flow::UdtFlow;
use crate::seq_number::SeqNumber;
use crate::socket::SYN_INTERVAL;
use rand::Rng;
use tokio::time::{Duration, Instant};

#[derive(Debug)]
pub struct RateControl {
    pkt_send_period: Duration,
    congestion_window_size: f64,
    max_window_size: f64,
    recv_rate: u32,
    bandwidth: u32,
    rtt: Duration,
    mss: f64,

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

    ack_period: Duration,
    ack_pkt_interval: usize,
}

impl RateControl {
    pub(crate) fn new() -> Self {
        Self {
            pkt_send_period: Duration::from_micros(1),
            congestion_window_size: 16.0,
            max_window_size: 16.0,
            recv_rate: 0,
            bandwidth: 0,
            rtt: Duration::default(),
            mss: 1500.0,

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
            dec_random: 1,
            dec_count: 0,

            ack_period: Duration::ZERO,
            ack_pkt_interval: 0,
        }
    }

    pub(crate) fn init(&mut self, mss: u32, flow: &UdtFlow, seq_number: SeqNumber) {
        self.last_rate_increase = Instant::now();
        self.mss = mss as f64;
        self.max_window_size = flow.flow_window_size as f64;

        self.slow_start = true;
        self.loss = false;
        self.curr_snd_seq_number = seq_number;
        self.last_ack = seq_number;
        self.last_dec_seq = seq_number - 1;

        self.recv_rate = flow.peer_delivery_rate;
        self.bandwidth = flow.peer_bandwidth;
        self.rtt = flow.rtt;
    }

    pub fn get_pkt_send_period(&self) -> Duration {
        self.pkt_send_period
    }

    pub fn get_congestion_window_size(&self) -> u32 {
        self.congestion_window_size as u32
    }

    pub fn get_ack_pkt_interval(&self) -> usize {
        self.ack_pkt_interval
    }

    pub fn get_ack_period(&self) -> Duration {
        self.ack_period
    }

    pub fn set_rtt(&mut self, rtt: Duration) {
        self.rtt = rtt;
    }

    pub fn set_rcv_rate(&mut self, pkt_per_sec: u32) {
        self.recv_rate = pkt_per_sec;
    }

    pub fn set_bandwidth(&mut self, pkt_per_sec: u32) {
        self.bandwidth = pkt_per_sec;
    }

    pub fn set_pkt_interval(&mut self, nb_pkts: usize) {
        self.ack_pkt_interval = nb_pkts;
    }

    pub fn on_ack(&mut self, ack: SeqNumber) {
        const MIN_INC: f64 = 0.01;

        let now = Instant::now();
        if (now - self.last_rate_increase) < self.rc_interval {
            return;
        }
        self.last_rate_increase = now;

        if self.slow_start {
            self.congestion_window_size += (ack - self.last_ack) as f64;
            self.last_ack = ack;
            if self.congestion_window_size > self.max_window_size {
                self.slow_start = false;
                if self.recv_rate > 0 {
                    self.pkt_send_period = Duration::from_secs(1) / self.recv_rate;
                } else {
                    self.pkt_send_period =
                        (self.rtt + self.rc_interval).div_f64(self.congestion_window_size);
                }
            }
        } else {
            self.congestion_window_size =
                self.recv_rate as f64 * (self.rtt + self.rc_interval).as_secs_f64() + 16.0
        }

        if self.slow_start {
            return;
        }

        if self.loss {
            self.loss = false;
            return;
        }

        let mut b = self.bandwidth as f64 - 1.0 / self.pkt_send_period.as_secs_f64();
        if (self.pkt_send_period > self.last_dec_period) && (self.bandwidth as f64 / 9.0 < b) {
            b = self.bandwidth as f64 / 9.0;
        }
        let increase = if b <= 0.0 {
            MIN_INC
        } else {
            let inc = 10.0_f64.powf((b * self.mss as f64 * 8.0).log10().ceil() * 1.5e-6 / self.mss);
            if inc < MIN_INC {
                MIN_INC
            } else {
                inc
            }
        };
        self.pkt_send_period = Duration::from_secs_f64(
            (self.pkt_send_period.as_secs_f64() * self.rc_interval.as_secs_f64())
                / (self.pkt_send_period.mul_f64(increase) + self.rc_interval).as_secs_f64(),
        );
    }

    pub fn on_loss(&mut self, loss_seq: SeqNumber) {
        if self.slow_start {
            self.slow_start = false;
            if self.recv_rate > 0 {
                self.pkt_send_period = Duration::from_secs(1) / self.recv_rate;
                return;
            }
            self.pkt_send_period =
                (self.rtt + self.rc_interval).div_f64(self.congestion_window_size);
        }

        self.loss = true;
        if (loss_seq - self.last_dec_seq) > 0 {
            self.last_dec_period = self.pkt_send_period;
            self.pkt_send_period = self.pkt_send_period.mul_f64(1.125);
            self.avg_nak_num =
                (self.avg_nak_num as f64 * 0.875 + self.nak_count as f64 * 0.125).ceil() as usize;
            self.nak_count = 1;
            self.dec_count = 1;
            self.last_dec_seq = self.curr_snd_seq_number;

            self.dec_random = if self.avg_nak_num == 0 {
                1
            } else {
                rand::thread_rng().gen_range(1..=self.avg_nak_num)
            };
        } else {
            self.dec_count += 1;
            if self.dec_random <= 5 {
                self.nak_count += 1;
                if self.nak_count % self.dec_random == 0 {
                    self.pkt_send_period = self.pkt_send_period.mul_f64(1.125);
                    self.last_dec_seq = self.curr_snd_seq_number;
                }
            }
        }
    }

    pub fn set_curr_snd_seq_number(&mut self, seq: SeqNumber) {
        self.curr_snd_seq_number = seq;
    }

    pub fn on_timeout(&mut self) {
        if self.slow_start {
            self.slow_start = false;
            if self.recv_rate > 0 {
                self.pkt_send_period = Duration::from_secs(1) / self.recv_rate;
            } else {
                self.pkt_send_period =
                    (self.rtt + self.rc_interval).div_f64(self.congestion_window_size);
            }
        }
    }
}

use std::collections::VecDeque;
use tokio::time::{Duration, Instant};

const ARRIVAL_WINDOW_SIZE: usize = 16;
const PROBE_WINDOW_SIZE: usize = 64;
pub const PROBE_MODULO: u32 = 16;

#[derive(Debug)]
pub(crate) struct UdtFlow {
    pub flow_window_size: u32,

    arrival_window: VecDeque<Duration>,
    probe_window: VecDeque<Duration>,
    last_arrival_time: Instant,
    probe_time: Instant,
    pub rtt: Duration,
    pub rtt_var: Duration,
    pub peer_bandwidth: u32,
    pub peer_delivery_rate: u32,
}

impl Default for UdtFlow {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            flow_window_size: 0,
            last_arrival_time: now,
            arrival_window: VecDeque::new(),
            probe_time: now,
            probe_window: VecDeque::new(),
            rtt: Duration::from_millis(100),
            rtt_var: Duration::from_millis(50),
            peer_bandwidth: 1,
            peer_delivery_rate: 16,
        }
    }
}

impl UdtFlow {
    pub fn on_pkt_arrival(&mut self) {
        let now = Instant::now();
        self.arrival_window.push_back(now - self.last_arrival_time);
        if self.arrival_window.len() > ARRIVAL_WINDOW_SIZE {
            self.arrival_window.pop_front();
        }
        self.last_arrival_time = now;
    }

    pub fn on_probe1_arrival(&mut self) {
        self.probe_time = Instant::now();
    }

    pub fn on_probe2_arrival(&mut self) {
        let now = Instant::now();
        self.probe_window.push_back(now - self.probe_time);
        if self.probe_window.len() > PROBE_WINDOW_SIZE {
            self.probe_window.pop_front();
        }
    }

    /// Returns a number of packets per second
    pub fn get_pkt_rcv_speed(&self) -> u32 {
        let length = self.arrival_window.len();
        let mut values = self.arrival_window.clone();
        let (_, median, _) = values.make_contiguous().select_nth_unstable(length / 2);
        let median = *median;
        let values: Vec<_> = values
            .into_iter()
            .filter(|x| *x > median / 8 && *x < median * 8)
            .collect();
        if values.len() < ARRIVAL_WINDOW_SIZE / 2 {
            return 0;
        }
        let total_duration: Duration = values.iter().sum();
        (values.len() as f64 / total_duration.as_secs_f64()).ceil() as u32
    }

    pub fn get_bandwidth(&self) -> u32 {
        if self.probe_window.is_empty() {
            return 0;
        }
        let length = self.probe_window.len();
        let mut values = self.probe_window.clone();
        let (_, median, _) = values.make_contiguous().select_nth_unstable(length / 2);
        let median = *median;
        let values: Vec<_> = values
            .into_iter()
            .filter(|x| *x > median / 8 && *x < median * 8)
            .collect();
        let total_duration: Duration = values.iter().sum();
        if total_duration.is_zero() {
            return 0;
        }
        (values.len() as f64 / total_duration.as_secs_f64()).ceil() as u32
    }

    pub fn update_rtt(&mut self, new_val: Duration) {
        self.rtt = (7 * self.rtt + new_val) / 8;
    }

    pub fn update_rtt_var(&mut self, new_val: Duration) {
        self.rtt_var = (3 * self.rtt_var + new_val) / 4;
    }

    pub fn update_bandwidth(&mut self, new_val: u32) {
        self.peer_bandwidth = (7 * self.peer_bandwidth + new_val) / 8;
    }

    pub fn update_peer_delivery_rate(&mut self, new_val: u32) {
        self.peer_delivery_rate = (7 * self.peer_delivery_rate + new_val) / 8;
    }
}

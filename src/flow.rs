use std::collections::VecDeque;
use tokio::time::{Duration, Instant};

const ARRIVAL_WINDOW_SIZE: usize = 16;
const PROBE_WINDOW_SIZE: usize = 64;
pub const PROBE_MODULO: u32 = 16;

#[derive(Debug)]
pub(crate) struct UdtFlow {
    arrival_window: VecDeque<Duration>,
    probe_window: VecDeque<Duration>,
    last_arrival_time: Instant,
    probe_time: Instant,
}

impl Default for UdtFlow {
    fn default() -> Self {
        Self {
            last_arrival_time: Instant::now(),
            ..Default::default()
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
    pub fn get_pkt_rcv_speed(&self) -> usize {
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
        (values.len() as f64 / total_duration.as_secs_f64()).ceil() as usize
    }

    pub fn get_bandwidth(&self) -> usize {
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
        (values.len() as f64 / total_duration.as_secs_f64()).ceil() as usize
    }
}

use crate::ack_window::AckWindow;
use crate::configuration::UdtConfiguration;
use crate::loss_list::RcvLossList;
use crate::seq_number::{AckSeqNumber, SeqNumber};
use crate::socket::SYN_INTERVAL;
use tokio::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct SocketState {
    pub last_rsp_time: Instant,

    // Receiving related,
    pub last_sent_ack: SeqNumber,
    pub last_sent_ack_time: Instant,
    pub curr_rcv_seq_number: SeqNumber,
    pub last_ack_seq_number: AckSeqNumber,
    pub rcv_loss_list: RcvLossList,
    pub last_ack2_received: SeqNumber,

    // Sending related
    pub last_ack_received: SeqNumber,
    pub last_data_ack_processed: SeqNumber,
    pub last_ack2_sent_back: AckSeqNumber,
    pub curr_snd_seq_number: SeqNumber,
    pub last_ack2_time: Instant,
    pub snd_loss_list: RcvLossList,

    pub next_ack_time: Instant,
    pub interpacket_interval: Duration,
    pub ack_packet_counter: usize,
    pub light_ack_counter: usize,
    pub exp_count: u32,

    pub ack_window: AckWindow,
}

impl SocketState {
    pub fn new(isn: SeqNumber, configuration: &UdtConfiguration) -> Self {
        let now = Instant::now();

        Self {
            last_rsp_time: now,
            last_ack_seq_number: AckSeqNumber::zero(),
            rcv_loss_list: RcvLossList::new(configuration.flight_flag_size),
            curr_rcv_seq_number: isn - 1,

            next_ack_time: now + SYN_INTERVAL,
            interpacket_interval: Duration::from_micros(10),
            ack_packet_counter: 0,
            light_ack_counter: 0,

            exp_count: 1,
            last_ack_received: isn - 1,
            last_sent_ack: isn - 1,
            last_sent_ack_time: now,
            last_ack2_received: isn.number().into(),

            curr_snd_seq_number: isn - 1,
            last_ack2_sent_back: isn.number().into(),
            last_ack2_time: now,
            last_data_ack_processed: isn,
            snd_loss_list: RcvLossList::new(configuration.flight_flag_size * 2),
            ack_window: AckWindow::new(1024),
        }
    }
}

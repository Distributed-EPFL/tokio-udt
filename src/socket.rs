use crate::configuration::UdtConfiguration;
use crate::control_packet::{AckOptionalInfo, ControlPacketType, HandShakeInfo, UdtControlPacket};
use crate::data_packet::{UdtDataPacket, UDT_DATA_HEADER_SIZE};
use crate::flow::{UdtFlow, PROBE_MODULO};
use crate::multiplexer::UdtMultiplexer;
use crate::packet::UdtPacket;
use crate::queue::{RcvBuffer, SndBuffer};
use crate::rate_control::RateControl;
use crate::seq_number::SeqNumber;
use crate::state::SocketState;
use crate::udt::{SocketRef, Udt};
use once_cell::sync::Lazy;
use rand::distributions::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::task::Poll;
use tokio::io::{Error, ErrorKind, ReadBuf, Result};
use tokio::sync::{Notify, RwLock as TokioRwLock};
use tokio::time::{Duration, Instant};

pub(crate) const SYN_INTERVAL: Duration = Duration::from_millis(10);
const MIN_EXP_INTERVAL: Duration = Duration::from_millis(300);
const ACK_INTERVAL: Duration = SYN_INTERVAL;
const PACKETS_BETWEEN_LIGHT_ACK: usize = 64;

static SALT: Lazy<String> = Lazy::new(|| {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(30)
        .map(char::from)
        .collect()
});

pub type SocketId = u32;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum SocketType {
    Stream = 1,
    Datagram = 2,
}

impl TryFrom<u32> for SocketType {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            1 => Ok(SocketType::Stream),
            2 => Ok(SocketType::Datagram),
            _ => Err(Error::new(
                ErrorKind::InvalidData,
                "unknown value for socket type",
            )),
        }
    }
}

#[derive(Debug)]
pub struct UdtSocket {
    pub socket_id: SocketId,
    pub status: Mutex<UdtStatus>,
    pub socket_type: SocketType,
    listen_socket: Option<SocketId>,
    peer_addr: Mutex<Option<SocketAddr>>,
    peer_socket_id: Mutex<Option<SocketId>>,
    pub initial_seq_number: SeqNumber,

    pub(crate) queued_sockets: TokioRwLock<BTreeSet<SocketId>>,
    pub(crate) accept_notify: Notify,
    pub(crate) multiplexer: RwLock<Weak<UdtMultiplexer>>,
    pub configuration: RwLock<UdtConfiguration>,

    rcv_buffer: Mutex<RcvBuffer>,
    snd_buffer: Mutex<SndBuffer>,
    flow: RwLock<UdtFlow>,
    rate_control: RwLock<RateControl>,
    // self_ip: Option<IpAddr>,
    start_time: Instant,

    state: Mutex<SocketState>,

    pub(crate) connect_notify: Notify,
    pub(crate) rcv_notify: Notify,
    pub(crate) ack_notify: Notify,
}

impl UdtSocket {
    pub(crate) fn new(
        socket_id: SocketId,
        socket_type: SocketType,
        isn: Option<SeqNumber>,
        configuration: Option<UdtConfiguration>,
    ) -> Self {
        let now = Instant::now();
        let initial_seq_number = isn.unwrap_or_else(SeqNumber::random);
        let configuration = configuration.unwrap_or_default();
        Self {
            socket_id,
            socket_type,
            status: Mutex::new(UdtStatus::Init),
            initial_seq_number,
            peer_addr: Mutex::new(None),
            peer_socket_id: Mutex::new(None),
            listen_socket: None,
            queued_sockets: TokioRwLock::new(BTreeSet::new()),
            accept_notify: Notify::new(),
            multiplexer: RwLock::new(Weak::new()),
            snd_buffer: Mutex::new(SndBuffer::new(configuration.snd_buf_size)),
            rcv_buffer: Mutex::new(RcvBuffer::new(
                configuration.rcv_buf_size,
                initial_seq_number,
            )),
            flow: RwLock::new(UdtFlow::default()),
            rate_control: RwLock::new(RateControl::new()),
            // self_ip: None,
            start_time: now,

            state: Mutex::new(SocketState::new(initial_seq_number, &configuration)),
            connect_notify: Notify::new(),
            rcv_notify: Notify::new(),
            ack_notify: Notify::new(),
            configuration: RwLock::new(configuration),
        }
    }

    pub fn with_peer(self, peer: SocketAddr, peer_socket_id: SocketId) -> Self {
        self.set_peer_addr(peer);
        *self.peer_socket_id.lock().unwrap() = Some(peer_socket_id);
        self
    }

    fn set_peer_addr(&self, peer: SocketAddr) {
        *self.peer_addr.lock().unwrap() = Some(peer);
        self.snd_buffer
            .lock()
            .unwrap()
            .set_payload_size(self.get_max_payload_size() as usize);
    }

    pub fn with_listen_socket(
        mut self,
        listen_socket_id: SocketId,
        mux: Arc<UdtMultiplexer>,
    ) -> Self {
        self.listen_socket = Some(listen_socket_id);
        *self.multiplexer.write().unwrap() = Arc::downgrade(&mux);
        self
    }

    pub fn open(&self) {
        *self.status.lock().unwrap() = UdtStatus::Opened;
    }

    fn rcv_buffer(&self) -> std::sync::MutexGuard<RcvBuffer> {
        self.rcv_buffer.lock().unwrap()
    }

    pub(crate) fn peer_addr(&self) -> Option<SocketAddr> {
        *self.peer_addr.lock().unwrap()
    }

    pub(crate) fn peer_socket_id(&self) -> Option<SocketId> {
        *self.peer_socket_id.lock().unwrap()
    }

    fn state(&self) -> std::sync::MutexGuard<SocketState> {
        self.state.lock().unwrap()
    }

    pub(crate) async fn connect_on_handshake(
        self,
        peer: SocketAddr,
        mut hs: HandShakeInfo,
    ) -> Result<SocketRef> {
        {
            let mut configuration = self.configuration.write().unwrap();
            if hs.max_packet_size > configuration.mss {
                hs.max_packet_size = configuration.mss;
            } else {
                configuration.mss = hs.max_packet_size;
            }

            self.flow.write().unwrap().flow_window_size = hs.max_window_size;
            hs.max_window_size =
                std::cmp::min(configuration.rcv_buf_size, configuration.flight_flag_size);
        }
        // self.set_self_ip(hs.ip_address);
        hs.ip_address = peer.ip();
        hs.socket_id = self.socket_id;

        // TODO: use network information cache to set RTT, bandwidth, etc.

        {
            let rate_control = self.rate_control.write().unwrap();
            // TODO: init congestion control
        }

        *self.status.lock().unwrap() = UdtStatus::Connected;

        let packet = UdtControlPacket::new_handshake(
            hs,
            self.peer_socket_id().expect("peer_socket_id not defined"),
        );

        if let Some(mux) = self.multiplexer() {
            mux.rcv_queue.push_back(self.socket_id);
            mux.send_to(&peer, packet.into()).await?;
        }

        let socket = Arc::new(self);
        Ok(socket)
    }

    pub fn set_multiplexer(&self, mux: &Arc<UdtMultiplexer>) {
        *self.multiplexer.write().unwrap() = Arc::downgrade(mux);
    }

    pub(crate) fn multiplexer(&self) -> Option<Arc<UdtMultiplexer>> {
        self.multiplexer.read().unwrap().upgrade()
    }

    // pub fn set_self_ip(&mut self, ip: IpAddr) {
    //     self.self_ip = Some(ip);
    // }

    // pub async fn self_addr(&self) -> Option<SocketAddr> {
    //     if let Some(mux) = self.multiplexer.lock().unwrap().upgrade() {
    //         return Some(mux.get_local_addr());
    //     }
    //     None
    // }

    pub(crate) async fn next_data_packets(&self) -> Result<Option<(Vec<UdtDataPacket>, Instant)>> {
        if [UdtStatus::Broken, UdtStatus::Closed, UdtStatus::Closing].contains(&self.status()) {
            return Err(Error::new(
                ErrorKind::BrokenPipe,
                "socket is closed or broken",
            ));
        }
        let now = Instant::now();
        let mut probe = false;

        let to_resend = {
            let mut state = self.state();
            let last_data_ack_processed = state.last_data_ack_processed;
            state
                .snd_loss_list
                .pop_after(last_data_ack_processed)
                .map(|seq| (seq, seq - last_data_ack_processed))
        };

        let packets = match to_resend {
            Some((seq, offset)) => {
                // Loss retransmission has priority
                if offset < 0 {
                    eprintln!("unexpected offset in sender loss list");
                    return Ok(None);
                }
                let to_send = self.snd_buffer.lock().unwrap().read_data(
                    offset as usize,
                    seq,
                    self.peer_socket_id().unwrap(),
                    self.start_time,
                );
                match to_send {
                    Err((msg_number, msg_len)) => {
                        if msg_len == 0 {
                            return Ok(None);
                        }
                        let (start, end) = (seq, seq + msg_len as i32 - 1);
                        let drop = UdtControlPacket::new_drop(
                            msg_number,
                            start,
                            end,
                            self.peer_socket_id().unwrap(),
                        );
                        self.send_packet(drop.into()).await?;

                        let mut state = self.state();
                        let last_data_ack_processed = state.last_data_ack_processed;
                        state.snd_loss_list.remove_all(last_data_ack_processed, end);
                        if (end + 1) - state.curr_snd_seq_number > 0 {
                            state.curr_snd_seq_number = end + 1;
                        }
                        return Ok(None);
                    }
                    Ok(packet) => vec![packet],
                }
            }
            None => {
                // TODO: check congestion window
                let window_size = self.flow.read().unwrap().flow_window_size;
                let mut state = self.state();
                if (state.curr_snd_seq_number - state.last_ack_received) > window_size as i32 {
                    return Ok(None);
                }
                match self.snd_buffer.lock().unwrap().fetch_batch(
                    state.curr_snd_seq_number + 1,
                    self.peer_socket_id().unwrap(),
                    self.start_time,
                ) {
                    packets if !packets.is_empty() => {
                        state.curr_snd_seq_number =
                            state.curr_snd_seq_number + packets.len() as i32;
                        if state.curr_snd_seq_number.number() % 16 == 0 {
                            probe = true;
                        }
                        packets
                    }
                    _ => return Ok(None),
                }
            }
        };

        // update CC and stats
        if probe {
            return Ok(Some((packets, now)));
        }

        // TODO keep track of difference with target time
        let packets_len = packets.len();
        Ok(Some((
            packets,
            now + self.state().interpacket_interval * packets_len as u32,
        )))
    }

    fn compute_cookie(&self, addr: &SocketAddr, offset: Option<isize>) -> u32 {
        let timestamp = (self.start_time.elapsed().as_secs() / 60) + offset.unwrap_or(0) as u64; // secret changes every one minute
        let host = addr.ip();
        let port = addr.port();
        let salt: &str = &(*SALT);
        u32::from_be_bytes(
            Sha256::digest(format!("{salt}:{host}:{port}:{timestamp}").as_bytes())[..4]
                .try_into()
                .unwrap(),
        )
    }

    pub(crate) async fn send_to(&self, addr: &SocketAddr, packet: UdtPacket) -> Result<()> {
        self.multiplexer()
            .expect("multiplexer not initialized")
            .send_to(addr, packet)
            .await?;
        Ok(())
    }

    pub(crate) async fn listen_on_handshake(
        &self,
        addr: SocketAddr,
        hs: &HandShakeInfo,
    ) -> Result<()> {
        let status = self.status();
        if status == UdtStatus::Closing || status == UdtStatus::Closed {
            return Err(Error::new(ErrorKind::ConnectionRefused, "socket closed"));
        }

        if hs.connection_type == 1 {
            // Regular connection, respond to handshake
            let mut hs_response = hs.clone();
            let dest_socket_id = hs_response.socket_id;
            hs_response.syn_cookie = self.compute_cookie(&addr, None);
            let hs_packet = UdtControlPacket::new_handshake(hs_response, dest_socket_id);
            self.send_to(&addr, hs_packet.into()).await?;
            return Ok(());
        }

        if hs.connection_type != -1 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("invalid connection_type: {}", hs.connection_type),
            ));
        }

        // Validate client response
        let syn_cookie = hs.syn_cookie;
        if syn_cookie != self.compute_cookie(&addr, None)
            && syn_cookie != self.compute_cookie(&addr, Some(-1))
        {
            // Invalid cookie;
            return Err(Error::new(ErrorKind::PermissionDenied, "invalid cookie"));
        }

        let dest_socket_id = hs.socket_id;
        let udt_version = self.configuration.read().unwrap().udt_version();
        if hs.udt_version != udt_version || hs.socket_type != self.socket_type {
            // Reject request
            let mut hs_response = hs.clone();
            hs_response.connection_type = 1002; // Error codes defined in C++ implementation
            let hs_packet = UdtControlPacket::new_handshake(hs_response, dest_socket_id);
            self.send_to(&addr, hs_packet.into()).await?;
            return Err(Error::new(
                ErrorKind::ConnectionRefused,
                "configuration mismatch",
            ));
        }

        Udt::get()
            .write()
            .await
            .new_connection(self, addr, hs)
            .await?;
        // Send handshake packet in case of errors on connection?

        Ok(())
    }

    pub(crate) async fn process_packet(&self, packet: UdtPacket) -> Result<()> {
        match packet {
            UdtPacket::Control(ctrl) => self.process_ctrl(ctrl).await,
            UdtPacket::Data(data) => self.process_data(data).await,
        }
    }

    async fn process_ctrl(&self, packet: UdtControlPacket) -> Result<()> {
        {
            let mut state = self.state();
            state.exp_count = 1;
            state.last_rsp_time = Instant::now();
        }

        match packet.packet_type {
            ControlPacketType::Handshake(hs) => {
                if self.status() != UdtStatus::Connecting {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "unexpected handshake for socket with status {:?}",
                            self.status
                        ),
                    ));
                }

                // TODO: handle rendezvous mode
                if hs.connection_type > 0 {
                    let mut hs = hs.clone();
                    hs.connection_type = -1;
                    hs.socket_id = self.socket_id;
                    let hs_packet = UdtControlPacket::new_handshake(hs, 0);
                    self.send_packet(hs_packet.into()).await?;
                } else {
                    // post connect
                    let mut configuration = self.configuration.write().unwrap();
                    configuration.mss = hs.max_packet_size;
                    configuration.flight_flag_size = hs.max_window_size;
                    let mut state = self.state();
                    state.last_sent_ack = hs.initial_seq_number;
                    state.last_ack2_received = hs.initial_seq_number;
                    state.curr_rcv_seq_number = hs.initial_seq_number - 1;
                    *self.peer_socket_id.lock().unwrap() = Some(hs.socket_id);
                    // self.self_ip = Some(hs.ip_address);

                    // TODO: check size of loss lists

                    // TODO: init CC
                    *self.status.lock().unwrap() = UdtStatus::Connected;
                    self.connect_notify.notify_waiters();
                }
            }
            ControlPacketType::KeepAlive => (),
            ControlPacketType::Ack(ref ack) => {
                match &ack.info {
                    None => {
                        let mut state = self.state();
                        let seq = ack.next_seq_number;
                        let nb_acked = seq - state.last_ack_received;
                        if nb_acked >= 0 {
                            state.last_ack_received = seq;
                            self.flow.write().unwrap().flow_window_size -= (nb_acked) as u32;
                        }
                    }
                    Some(extra) => {
                        let ack_seq = packet.ack_seq_number().unwrap();
                        let send_ack2 = {
                            let state = self.state();
                            state.last_ack2_time.elapsed() > SYN_INTERVAL
                                || ack_seq == state.last_ack2_sent_back
                        };
                        if send_ack2 {
                            if let Some(peer) = self.peer_socket_id() {
                                let ack2_packet = UdtControlPacket::new_ack2(ack_seq, peer);
                                self.send_packet(ack2_packet.into()).await?;
                                let mut state = self.state();
                                state.last_ack2_sent_back = ack_seq;
                                state.last_ack2_time = Instant::now();
                            }
                        }

                        let seq = ack.next_seq_number;

                        {
                            let mut state = self.state();
                            if (seq - state.curr_snd_seq_number) > 1 {
                                // This should not happen
                                eprintln!("Udt socket broken: seq number is larger than expected");
                                *self.status.lock().unwrap() = UdtStatus::Broken;
                            }

                            if (seq - state.last_ack_received) >= 0 {
                                self.flow.write().unwrap().flow_window_size =
                                    extra.available_buf_size;
                                state.last_ack_received = seq;
                            }

                            let offset = seq - state.last_data_ack_processed;
                            if offset <= 0 {
                                // Ignore Repeated acks
                                return Ok(());
                            }

                            self.snd_buffer.lock().unwrap().ack_data(offset);
                            let last_data_ack_processed = state.last_data_ack_processed;
                            state
                                .snd_loss_list
                                .remove_all(last_data_ack_processed, seq - 1);
                            // TODO record times for monitoring purposes
                            state.last_data_ack_processed = seq;
                            self.update_snd_queue(false);
                            self.ack_notify.notify_waiters();
                        }

                        let mut flow = self.flow.write().unwrap();
                        flow.update_rtt(Duration::from_micros(extra.rtt.into()));
                        flow.update_rtt_var(Duration::from_micros(extra.rtt_variance.into()));
                        if extra.pack_recv_rate > 0 {
                            flow.update_peer_delivery_rate(extra.pack_recv_rate);
                        }
                        if extra.link_capacity > 0 {
                            flow.update_bandwidth(extra.link_capacity);
                        }
                        // TODO: CCC
                    }
                }
            }
            ControlPacketType::Ack2 => {
                let ack_seq = packet.ack_seq_number().unwrap();
                let window = self.state().ack_window.get(ack_seq);
                if let Some((seq, rtt)) = window {
                    let mut flow = self.flow.write().unwrap();
                    let rtt_abs_diff = {
                        if rtt > flow.rtt {
                            rtt - flow.rtt
                        } else {
                            flow.rtt - rtt
                        }
                    };
                    flow.update_rtt_var(rtt_abs_diff);
                    flow.update_rtt(rtt);
                    drop(flow);
                    let mut state = self.state();
                    if (seq - state.last_ack2_received) > 0 {
                        state.last_ack2_received = seq;
                    }
                }
            }
            ControlPacketType::Nak(ref nak) => {
                let mut broken = false;
                let loss_iter = &mut nak.loss_info.iter();
                let mut state = self.state();
                while let Some(loss) = loss_iter.next() {
                    let (seq_start, seq_end) = {
                        if loss & 0x8000_0000 != 0 {
                            if let Some(seq_end) = loss_iter.next() {
                                let seq_start: SeqNumber = (loss & 0x7fff_ffff).into();
                                let seq_end: SeqNumber = (*seq_end).into();
                                (seq_start, seq_end)
                            } else {
                                broken = true;
                                break;
                            }
                        } else {
                            ((*loss).into(), (*loss).into())
                        }
                    };
                    if (seq_start - seq_end > 0) || (seq_end - state.curr_snd_seq_number > 0) {
                        broken = true;
                        break;
                    }
                    if seq_start - state.last_ack_received >= 0 {
                        state.snd_loss_list.insert(seq_start, seq_end);
                    } else if seq_end - state.last_ack_received >= 0 {
                        let last_ack_received = state.last_ack_received;
                        state.snd_loss_list.insert(last_ack_received, seq_end);
                    }
                }

                if broken {
                    println!("NAK is broken: {:?} {:?}", nak, state);
                    *self.status.lock().unwrap() = UdtStatus::Broken;
                    return Ok(());
                }

                self.update_snd_queue(true);
            }
            ControlPacketType::Shutdown => {
                *self.status.lock().unwrap() = UdtStatus::Closing;
            }
            ControlPacketType::MsgDropRequest(ref drop) => {
                let msg_number = packet.msg_seq_number().unwrap();
                self.rcv_buffer.lock().unwrap().drop_msg(msg_number);
                let mut state = self.state();
                state
                    .rcv_loss_list
                    .remove_all(drop.first_seq_number, drop.last_seq_number);
                if (drop.first_seq_number - (state.curr_rcv_seq_number + 1)) <= 0
                    && (drop.last_seq_number - state.curr_rcv_seq_number) > 0
                {
                    state.curr_rcv_seq_number = drop.last_seq_number;
                }
            }
            ControlPacketType::UserDefined => unimplemented!(),
        }
        Ok(())
    }

    async fn process_data(&self, packet: UdtDataPacket) -> Result<()> {
        let now = Instant::now();
        self.state().last_rsp_time = now;

        // CC onPktReceived
        // pktCount++

        let seq_number = packet.header.seq_number;

        {
            let mut flow = self.flow.write().unwrap();
            flow.on_pkt_arrival(now);

            if seq_number.number() % PROBE_MODULO == 0 {
                flow.on_probe1_arrival();
            } else if seq_number.number() % PROBE_MODULO == 1 {
                flow.on_probe2_arrival();
            }
        }

        // trace_rcv++
        // recv_total++
        let offset = seq_number - self.state().last_sent_ack;
        if offset < 0 {
            // seq number is too late
            return Ok(());
        }

        let payload_len = {
            let mut rcv_buffer = self.rcv_buffer();
            let available_buf_size = rcv_buffer.get_available_buf_size();
            if available_buf_size < offset as u32 {
                eprintln!("not enough space in rcv buffer");
                return Ok(());
            }

            let payload_len = packet.payload_len();
            rcv_buffer.insert(packet);
            payload_len
        };

        if (seq_number - self.state().curr_rcv_seq_number) > 1 {
            // some packets have been lost in between
            let nak_packet = {
                let mut state = self.state();
                let curr_rcv_seq_number = state.curr_rcv_seq_number;
                state
                    .rcv_loss_list
                    .insert(curr_rcv_seq_number + 1, seq_number - 1);

                // send NAK immediately
                let loss_list = {
                    if state.curr_rcv_seq_number + 1 == seq_number - 1 {
                        vec![(seq_number - 1).number()]
                    } else {
                        vec![
                            (state.curr_rcv_seq_number + 1).number() | 0x8000_0000,
                            (seq_number - 1).number(),
                        ]
                    }
                };
                UdtControlPacket::new_nak(loss_list, self.peer_socket_id().unwrap_or(0))
            };
            self.send_packet(nak_packet.into()).await?;
            // TODO increment NAK stats
        }

        if payload_len < self.get_max_payload_size() {
            self.state().next_ack_time = Instant::now();
        }

        let mut state = self.state();

        if seq_number - state.curr_rcv_seq_number > 0 {
            state.curr_rcv_seq_number = seq_number;
        } else {
            state.rcv_loss_list.remove(seq_number);
        }

        Ok(())
    }

    pub fn get_max_payload_size(&self) -> u32 {
        let configuration = self.configuration.read().unwrap();
        match self.peer_addr().map(|a| a.ip()) {
            Some(IpAddr::V6(_)) => configuration.mss - 40 - UDT_DATA_HEADER_SIZE as u32,
            _ => configuration.mss - 28 - UDT_DATA_HEADER_SIZE as u32,
        }
    }

    pub(crate) async fn send_packet(&self, packet: UdtPacket) -> Result<()> {
        if let Some(addr) = self.peer_addr() {
            self.send_to(&addr, packet).await?;
        }
        Ok(())
    }

    pub(crate) async fn send_data_packets(&self, packets: Vec<UdtDataPacket>) -> Result<()> {
        if let Some(addr) = self.peer_addr() {
            self.multiplexer()
                .expect("multiplexer not initialized")
                .send_mmsg_to(&addr, packets.into_iter().map(|p| p.into()))
                .await?;
        }
        Ok(())
    }

    async fn send_ack(&self, light: bool) -> Result<()> {
        let seq_number = {
            let state = self.state();
            let seq_number = match state
                .rcv_loss_list
                .peek_after(state.curr_rcv_seq_number + 1)
            {
                Some(num) => num,
                None => state.curr_rcv_seq_number + 1,
            };
            if seq_number == state.last_ack2_received {
                return Ok(());
            }
            seq_number
        };

        if light {
            // Save time on buffer procesing and bandwith measurement
            let ack_packet = UdtControlPacket::new_ack(
                0.into(),
                seq_number,
                self.peer_socket_id().unwrap(),
                None,
            );
            self.send_packet(ack_packet.into()).await?;
            return Ok(());
        }

        {
            let mut state = self.state();
            let to_ack: i32 = seq_number - state.last_sent_ack;
            match to_ack.cmp(&0) {
                Ordering::Greater => {
                    self.rcv_buffer().ack_data(seq_number);
                    state.last_sent_ack = seq_number;
                    self.rcv_notify.notify_waiters();
                }
                Ordering::Equal => {
                    let last_sent_ack_elapsed = state.last_sent_ack_time.elapsed();
                    drop(state);
                    let flow = self.flow.read().unwrap();
                    if last_sent_ack_elapsed < (flow.rtt + 4 * flow.rtt_var) {
                        return Ok(());
                    }
                }
                _ => {
                    return Ok(());
                }
            }
        }

        let ack_packet = {
            let mut state = self.state();
            if (state.last_sent_ack - state.last_ack2_received) > 0 {
                state.last_ack_seq_number = state.last_ack_seq_number + 1;
                drop(state);
                let mut ack_info = {
                    let flow = self.flow.read().unwrap();
                    AckOptionalInfo {
                        rtt: flow.rtt.as_micros().try_into().unwrap_or(u32::MAX),
                        rtt_variance: flow.rtt_var.as_micros().try_into().unwrap_or(u32::MAX),
                        available_buf_size: std::cmp::max(
                            self.rcv_buffer().get_available_buf_size(),
                            2,
                        ),
                        // TODO: use rcv_time_window to keep track of bandwidth / recv speed
                        pack_recv_rate: 0,
                        link_capacity: 0,
                    }
                };
                if self.state().last_sent_ack_time.elapsed() > SYN_INTERVAL {
                    let flow = self.flow.read().unwrap();
                    ack_info.pack_recv_rate = flow.get_pkt_rcv_speed();
                    ack_info.link_capacity = flow.get_bandwidth();
                    self.state().last_sent_ack_time = Instant::now();
                }
                let state = self.state();
                Some(UdtControlPacket::new_ack(
                    state.last_ack_seq_number,
                    state.last_sent_ack,
                    self.peer_socket_id().unwrap(),
                    Some(ack_info),
                ))
            } else {
                None
            }
        };

        if let Some(ack_packet) = ack_packet {
            self.send_packet(ack_packet.into()).await?;
            let mut state = self.state();
            let last_sent_ack = state.last_sent_ack;
            let last_ack_seq_number = state.last_ack_seq_number;
            state.ack_window.store(last_sent_ack, last_ack_seq_number);
        }

        Ok(())
    }

    fn cc_update(&self) {
        // TODO update CC parameters

        //self.interpacket_interval = ...
    }

    pub(crate) async fn check_timers(&self) {
        self.cc_update();
        let now = Instant::now();

        if now > self.state().next_ack_time {
            // TODO: use CC ack interval too
            self.send_ack(false).await.unwrap_or_else(|err| {
                eprintln!("failed to send ack: {:?}", err);
            });
            let mut state = self.state();
            state.next_ack_time = now + ACK_INTERVAL;
            state.ack_packet_counter = 0;
            state.light_ack_counter = 0;
        } else {
            let send_light_ack = {
                let state = self.state();
                (state.light_ack_counter + 1) * PACKETS_BETWEEN_LIGHT_ACK
                    <= state.ack_packet_counter
            };
            if send_light_ack {
                self.send_ack(true).await.unwrap_or_else(|err| {
                    eprintln!("failed to send ack: {:?}", err);
                });
                self.state().light_ack_counter += 1;
            }
        }

        // TODO use user-defined RTO
        let next_exp_time = {
            let (rtt, rtt_var) = {
                let flow = self.flow.read().unwrap();
                (flow.rtt, flow.rtt_var)
            };
            let state = self.state();
            let exp_int = state.exp_count * (rtt + 4 * rtt_var) + SYN_INTERVAL;
            let next_exp = std::cmp::max(exp_int, state.exp_count * MIN_EXP_INTERVAL);
            state.last_rsp_time + next_exp
        };
        if now > next_exp_time {
            {
                let state = self.state();
                if state.exp_count > 16 && state.last_rsp_time.elapsed() > Duration::from_secs(5) {
                    // Connection is broken
                    *self.status.lock().unwrap() = UdtStatus::Broken;
                    self.update_snd_queue(true);
                    return;
                }
            }

            if self.snd_buffer.lock().unwrap().is_empty() {
                if let Some(peer_socket_id) = self.peer_socket_id() {
                    let keep_alive = UdtControlPacket::new_keep_alive(peer_socket_id);
                    self.send_packet(keep_alive.into())
                        .await
                        .unwrap_or_else(|err| {
                            eprintln!("failed to send keep alive: {:?}", err);
                        });
                }
            } else {
                let mut state = self.state();
                if (state.last_ack_received != state.curr_snd_seq_number + 1)
                    && state.snd_loss_list.is_empty()
                {
                    let last_ack_received = state.last_ack_received;
                    let curr_snd_seq_number = state.curr_snd_seq_number;
                    state
                        .snd_loss_list
                        .insert(last_ack_received, curr_snd_seq_number);
                }

                // TODO: CC onTimeout
                self.cc_update();
                self.update_snd_queue(true);
            }

            let mut state = self.state();
            state.exp_count += 1;
            // Reset last response time since we just sent a heart-beat.
            state.last_rsp_time = now;
        }
    }

    fn update_snd_queue(&self, reschedule: bool) {
        if let Some(mux) = self.multiplexer() {
            mux.snd_queue.update(self.socket_id, reschedule);
        }
    }

    pub fn send(&self, data: &[u8]) -> Result<()> {
        if self.socket_type != SocketType::Stream {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "socket needs to be configured in stream mode to send data buffer",
            ));
        }
        if self.status() != UdtStatus::Connected {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket is not connected",
            ));
        }

        if data.is_empty() {
            return Ok(());
        }

        if self.snd_buffer.lock().unwrap().is_empty() {
            // delay the EXP timer to avoid mis-fired timeout
            self.state().last_rsp_time = Instant::now();
        }

        self.snd_buffer
            .lock()
            .unwrap()
            .add_message(data, None, false)?;
        self.update_snd_queue(false);
        Ok(())
    }

    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        if self.socket_type != SocketType::Stream {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "cannot recv on non-stream socket",
            ));
        }
        let status = self.status();
        if status == UdtStatus::Broken || status == UdtStatus::Closing {
            if !self.rcv_buffer().has_data_to_read() {
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "connection was closed or broken",
                ));
            }
        } else if status != UdtStatus::Connected {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket not connected",
            ));
        }

        if buf.is_empty() {
            return Ok(0);
        }

        if !self.rcv_buffer().has_data_to_read() {
            self.rcv_notify.notified().await;
        }

        let status = self.status();
        if status == UdtStatus::Broken || status == UdtStatus::Closing {
            if !self.rcv_buffer().has_data_to_read() {
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "connection was closed or broken",
                ));
            }
        } else if status != UdtStatus::Connected {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket not connected",
            ));
        }

        let mut buf = ReadBuf::new(buf);
        let written = self.rcv_buffer().read_buffer(&mut buf);

        // TODO: handle UDT timeout
        Ok(written)
    }

    pub(crate) fn poll_recv(&self, buf: &mut ReadBuf<'_>) -> Poll<Result<usize>> {
        if self.socket_type != SocketType::Stream {
            return Poll::Ready(Err(Error::new(
                ErrorKind::InvalidInput,
                "cannot recv on non-stream socket",
            )));
        }
        let status = self.status();
        if status == UdtStatus::Broken || status == UdtStatus::Closing {
            if !self.rcv_buffer().has_data_to_read() {
                return Poll::Ready(Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "connection was closed or broken",
                )));
            }
        } else if status != UdtStatus::Connected {
            return Poll::Ready(Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket not connected",
            )));
        }

        if !self.rcv_buffer().has_data_to_read() {
            return Poll::Pending;
        }

        if buf.remaining() == 0 {
            return Poll::Ready(Ok(0));
        }
        let written = self.rcv_buffer().read_buffer(buf);
        Poll::Ready(Ok(written))
    }

    pub(crate) async fn connect(&self, addr: SocketAddr) -> Result<()> {
        if self.status() != UdtStatus::Init {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("expected status Init, found {:?}", self.status),
            ));
        }

        self.open();
        {
            let mut udt = Udt::get().write().await;
            udt.update_mux(self, None).await?;
        }

        *self.status.lock().unwrap() = UdtStatus::Connecting;
        self.set_peer_addr(addr);

        // TODO: use rendezvous queue?

        let hs_packet = {
            let configuration = self.configuration.read().unwrap();
            let hs = HandShakeInfo {
                udt_version: configuration.udt_version(),
                initial_seq_number: self.initial_seq_number,
                max_packet_size: configuration.mss,
                max_window_size: std::cmp::min(
                    self.flow.read().unwrap().flow_window_size,
                    self.rcv_buffer().get_available_buf_size(),
                ),
                connection_type: 1,
                socket_type: self.socket_type,
                socket_id: self.socket_id,
                ip_address: addr.ip(),
                syn_cookie: 0,
            };
            UdtControlPacket::new_handshake(hs, 0)
        };
        self.send_to(&addr, hs_packet.into()).await?;

        Ok(())
    }

    pub fn status(&self) -> UdtStatus {
        *self.status.lock().unwrap()
    }

    pub fn snd_buffer_is_empty(&self) -> bool {
        self.snd_buffer.lock().unwrap().is_empty()
    }
}

impl Ord for UdtSocket {
    fn cmp(&self, other: &Self) -> Ordering {
        self.socket_id.cmp(&other.socket_id)
    }
}

impl PartialOrd for UdtSocket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for UdtSocket {
    fn eq(&self, other: &Self) -> bool {
        self.socket_id == other.socket_id
    }
}

impl Eq for UdtSocket {}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum UdtStatus {
    Init,
    Opened,
    Listening,
    Connecting,
    Connected,
    Broken,
    Closing,
    Closed,
}

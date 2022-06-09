use crate::configuration::UdtConfiguration;
use crate::control_packet::{AckOptionalInfo, ControlPacketType, HandShakeInfo, UdtControlPacket};
use crate::data_packet::UdtDataPacket;
use crate::flow::{UdtFlow, PROBE_MODULO};
use crate::multiplexer::UdtMultiplexer;
use crate::packet::{UdtPacket, UDT_HEADER_SIZE};
use crate::queue::{RcvBuffer, SndBuffer};
use crate::seq_number::SeqNumber;
use crate::state::SocketState;
use crate::udt::{SocketRef, Udt};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Weak};
use tokio::io::{Error, ErrorKind, Result};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::time::{Duration, Instant};

pub(crate) const SYN_INTERVAL: Duration = Duration::from_millis(10);
const MIN_EXP_INTERVAL: Duration = Duration::from_millis(300);
const ACK_INTERVAL: Duration = SYN_INTERVAL;
const PACKETS_BETWEEN_LIGHT_ACK: usize = 64;

pub type SocketId = u32;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum SocketType {
    Stream = 0,
    Datagram = 1,
}

impl TryFrom<u32> for SocketType {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            0 => Ok(SocketType::Stream),
            1 => Ok(SocketType::Datagram),
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
    pub status: RwLock<UdtStatus>,
    pub socket_type: SocketType,
    listen_socket: Option<SocketId>,
    pub peer_addr: Option<SocketAddr>,
    pub peer_socket_id: RwLock<Option<SocketId>>,
    pub initial_seq_number: SeqNumber,

    pub(crate) queued_sockets: RwLock<BTreeSet<SocketId>>,
    pub(crate) accept_notify: Notify,
    pub(crate) backlog_size: usize,
    pub(crate) multiplexer: Weak<RwLock<UdtMultiplexer>>,
    pub configuration: RwLock<UdtConfiguration>,

    rcv_buffer: RwLock<RcvBuffer>,
    snd_buffer: RwLock<SndBuffer>,
    flow: RwLock<UdtFlow>,
    // self_ip: Option<IpAddr>,
    start_time: Instant,

    state: Mutex<SocketState>,

    pub(crate) connect_notify: Notify,
    rcv_notify: Notify,
}

impl UdtSocket {
    pub(crate) fn new(
        socket_id: SocketId,
        socket_type: SocketType,
        isn: Option<SeqNumber>,
    ) -> Self {
        let now = Instant::now();
        let initial_seq_number = isn.unwrap_or_else(SeqNumber::random);
        let configuration = UdtConfiguration::default();
        Self {
            socket_id,
            socket_type,
            status: RwLock::new(UdtStatus::Init),
            initial_seq_number,
            peer_addr: None,
            peer_socket_id: RwLock::new(None),
            listen_socket: None,
            queued_sockets: RwLock::new(BTreeSet::new()),
            accept_notify: Notify::new(),
            backlog_size: 0,
            multiplexer: Weak::new(),
            snd_buffer: RwLock::new(SndBuffer::new(
                configuration.snd_buf_size,
                configuration.mss,
            )),
            rcv_buffer: RwLock::new(RcvBuffer::new(
                configuration.rcv_buf_size,
                initial_seq_number,
            )),
            flow: RwLock::new(UdtFlow::default()),
            // self_ip: None,
            start_time: now,

            state: Mutex::new(SocketState::new(initial_seq_number, &configuration)),
            connect_notify: Notify::new(),
            rcv_notify: Notify::new(),
            configuration: RwLock::new(configuration),
        }
    }

    pub async fn with_peer(mut self, peer: SocketAddr, peer_socket_id: SocketId) -> Self {
        self.peer_addr = Some(peer);
        *self.peer_socket_id.write().await = Some(peer_socket_id);
        self
    }

    pub fn with_listen_socket(
        mut self,
        listen_socket_id: SocketId,
        mux: Arc<RwLock<UdtMultiplexer>>,
    ) -> Self {
        self.listen_socket = Some(listen_socket_id);
        self.multiplexer = Arc::downgrade(&mux);
        self
    }

    pub async fn open(&self) {
        *self.status.write().await = UdtStatus::Opened;
    }

    pub(crate) async fn connect_on_handshake(
        mut self,
        peer: SocketAddr,
        mut hs: HandShakeInfo,
    ) -> Result<SocketRef> {
        {
            let mut configuration = self.configuration.write().await;
            if hs.max_packet_size > configuration.mss {
                hs.max_packet_size = configuration.mss;
            } else {
                configuration.mss = hs.max_packet_size;
            }

            self.flow.write().await.flow_window_size = hs.max_window_size;
            hs.max_window_size =
                std::cmp::min(configuration.rcv_buf_size, configuration.flight_flag_size);
        }
        // self.set_self_ip(hs.ip_address);
        hs.ip_address = peer.ip();
        hs.socket_id = self.socket_id;

        // TODO: use network information cache to set RTT, bandwidth, etc.
        // TODO: init congestion control

        self.peer_addr = Some(peer);
        *self.status.write().await = UdtStatus::Connected;

        let packet = UdtControlPacket::new_handshake(
            hs,
            self.peer_socket_id
                .read()
                .await
                .expect("peer_socket_id not defined"),
        );

        if let Some(lock) = self.multiplexer.upgrade() {
            let mux = lock.read().await;
            mux.rcv_queue.push_back(self.socket_id).await;
            mux.send_to(&peer, packet.into()).await?;
        }

        let socket = Arc::new(RwLock::new(self));
        Ok(socket)
    }

    pub fn set_multiplexer(&mut self, mux: &Arc<RwLock<UdtMultiplexer>>) {
        self.multiplexer = Arc::downgrade(mux);
    }

    // pub fn set_self_ip(&mut self, ip: IpAddr) {
    //     self.self_ip = Some(ip);
    // }

    pub async fn self_addr(&self) -> Option<SocketAddr> {
        if let Some(mux) = self.multiplexer.upgrade() {
            return Some(mux.read().await.get_local_addr());
        }
        None
    }

    pub(crate) async fn next_data_packet(&self) -> Result<Option<(UdtDataPacket, Instant)>> {
        if [UdtStatus::Broken, UdtStatus::Closed, UdtStatus::Closing].contains(&self.status().await)
        {
            return Err(Error::new(
                ErrorKind::BrokenPipe,
                "socket is closed or broken",
            ));
        }
        let now = Instant::now();
        let mut probe = false;

        let mut state = self.state.lock().await;
        let last_data_ack_processed = state.last_data_ack_processed;
        let packet = match state.snd_loss_list.pop_after(last_data_ack_processed) {
            Some(seq) => {
                // Loss retransmission has priority
                let offset = seq - state.last_data_ack_processed;
                if offset < 0 {
                    eprintln!("unexpected offset in sender loss list");
                    return Ok(None);
                }
                match self.snd_buffer.write().await.read_data(
                    offset as usize,
                    seq,
                    self.peer_socket_id.read().await.unwrap(),
                    self.start_time,
                ) {
                    Err((msg_number, msg_len)) => {
                        if msg_len == 0 {
                            return Ok(None);
                        }
                        let (start, end) = (seq, seq + msg_len as i32 - 1);
                        let drop = UdtControlPacket::new_drop(
                            msg_number,
                            start,
                            end,
                            self.peer_socket_id.read().await.unwrap(),
                        );
                        self.send_packet(drop.into()).await?;

                        let last_data_ack_processed = state.last_data_ack_processed;
                        state.snd_loss_list.remove_all(last_data_ack_processed, end);
                        if (end + 1) - state.curr_snd_seq_number > 0 {
                            state.curr_snd_seq_number = end + 1;
                        }
                        return Ok(None);
                    }
                    Ok(packet) => packet,
                }
            }
            None => {
                // TODO: check congestion window
                let window_size = self.flow.read().await.flow_window_size;
                if (state.curr_snd_seq_number - state.last_ack_received) > window_size as i32 {
                    return Ok(None);
                }
                match self.snd_buffer.write().await.fetch(
                    state.curr_snd_seq_number + 1,
                    self.peer_socket_id.read().await.unwrap(),
                    self.start_time,
                ) {
                    Some(packet) => {
                        state.curr_snd_seq_number = state.curr_snd_seq_number + 1;
                        if state.curr_snd_seq_number.number() % 16 == 0 {
                            probe = true;
                        }
                        packet
                    }
                    None => return Ok(None),
                }
            }
        };

        // update CC and stats
        if probe {
            return Ok(Some((packet, now)));
        }

        // TODO keep track of difference with target time
        Ok(Some((packet, now + state.interpacket_interval)))
    }

    fn compute_cookie(&self, addr: &SocketAddr, offset: Option<isize>) -> u32 {
        let timestamp = (self.start_time.elapsed().as_secs() / 60) + offset.unwrap_or(0) as u64; // secret changes every one minute
        let host = addr.ip();
        let port = addr.port();
        u32::from_be_bytes(
            Sha256::digest(format!("{host}:{port}:{timestamp}").as_bytes())[..4]
                .try_into()
                .unwrap(),
        )
    }

    pub(crate) async fn send_to(&self, addr: &SocketAddr, packet: UdtPacket) -> Result<()> {
        self.multiplexer
            .upgrade()
            .expect("multiplexer not initialized")
            .read()
            .await
            .send_to(addr, packet)
            .await?;
        Ok(())
    }

    pub(crate) async fn listen_on_handshake(
        &self,
        addr: SocketAddr,
        hs: &HandShakeInfo,
    ) -> Result<()> {
        let status = self.status().await;
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
        let configuration = self.configuration.read().await;
        if hs.udt_version != configuration.udt_version() || hs.socket_type != self.socket_type {
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
        let mut state = self.state.lock().await;
        state.exp_count = 1;
        state.last_rsp_time = Instant::now();

        match packet.packet_type {
            ControlPacketType::Handshake(hs) => {
                if self.status().await != UdtStatus::Connecting {
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
                    let mut configuration = self.configuration.write().await;
                    configuration.mss = hs.max_packet_size;
                    configuration.flight_flag_size = hs.max_window_size;
                    state.last_sent_ack = hs.initial_seq_number;
                    state.last_ack2_received = hs.initial_seq_number;
                    state.curr_rcv_seq_number = hs.initial_seq_number - 1;
                    *self.peer_socket_id.write().await = Some(hs.socket_id);
                    // self.self_ip = Some(hs.ip_address);

                    // TODO: check size of loss lists

                    // TODO: init CC
                    *self.status.write().await = UdtStatus::Connected;
                    self.connect_notify.notify_waiters();
                }
            }
            ControlPacketType::KeepAlive => (),
            ControlPacketType::Ack(ref ack) => {
                match &ack.info {
                    None => {
                        let seq = ack.next_seq_number;
                        if (seq - state.last_ack_received) >= 0 {
                            state.last_ack_received = seq;
                            self.flow.write().await.flow_window_size -=
                                (seq - state.last_ack_received) as u32;
                        }
                    }
                    Some(extra) => {
                        let ack_seq = packet.ack_seq_number().unwrap();
                        if state.last_ack2_time.elapsed() > SYN_INTERVAL
                            || ack_seq == state.last_ack2_sent_back
                        {
                            if let Some(peer) = *self.peer_socket_id.read().await {
                                let ack2_packet = UdtControlPacket::new_ack2(ack_seq, peer);
                                self.send_packet(ack2_packet.into()).await?;
                                state.last_ack2_sent_back = ack_seq;
                                state.last_ack2_time = Instant::now();
                            }
                        }

                        let seq = ack.next_seq_number;

                        if (seq - state.curr_snd_seq_number) > 1 {
                            // This should not happen
                            eprintln!("Udt socket broken: seq number is larger than expected");
                            *self.status.write().await = UdtStatus::Broken;
                        }

                        if (seq - state.last_ack_received) >= 0 {
                            self.flow.write().await.flow_window_size = extra.available_buf_size;
                            state.last_ack_received = seq;
                        }

                        let offset = seq - state.last_data_ack_processed;
                        if offset <= 0 {
                            // Ignore Repeated acks
                            return Ok(());
                        }

                        self.snd_buffer.write().await.ack_data(offset);
                        let last_data_ack_processed = state.last_data_ack_processed;
                        state
                            .snd_loss_list
                            .remove_all(last_data_ack_processed, seq - 1);
                        // TODO record times for monitoring purposes
                        state.last_data_ack_processed = seq;
                        self.update_snd_queue(false).await;

                        self.flow
                            .write()
                            .await
                            .update_rtt(Duration::from_micros(extra.rtt.into()));
                        self.flow
                            .write()
                            .await
                            .update_rtt_var(Duration::from_micros(extra.rtt_variance.into()));
                        if extra.pack_recv_rate > 0 {
                            self.flow
                                .write()
                                .await
                                .update_peer_delivery_rate(extra.pack_recv_rate);
                        }
                        if extra.link_capacity > 0 {
                            self.flow
                                .write()
                                .await
                                .update_bandwidth(extra.link_capacity);
                        }
                        // TODO: CCC
                    }
                }
            }
            ControlPacketType::Ack2 => {
                let ack_seq = packet.ack_seq_number().unwrap();
                if let Some((seq, rtt)) = state.ack_window.get(ack_seq) {
                    let mut flow = self.flow.write().await;
                    let rtt_abs_diff = {
                        if rtt > flow.rtt {
                            rtt - flow.rtt
                        } else {
                            flow.rtt - rtt
                        }
                    };
                    flow.update_rtt_var(rtt_abs_diff);
                    flow.update_rtt(rtt);
                    if (seq - state.last_ack2_received) > 0 {
                        state.last_ack2_received = seq;
                    }
                }
            }
            ControlPacketType::Nak(ref nak) => {
                let mut broken = false;
                let loss_iter = &mut nak.loss_info.iter();
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
                    *self.status.write().await = UdtStatus::Broken;
                    return Ok(());
                }

                self.update_snd_queue(true).await;
            }
            ControlPacketType::Shutdown => {
                *self.status.write().await = UdtStatus::Closing;
            }
            ControlPacketType::MsgDropRequest(ref drop) => {
                let msg_number = packet.msg_seq_number().unwrap();
                self.rcv_buffer.write().await.drop_msg(msg_number);
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
        let mut state = self.state.lock().await;
        state.last_rsp_time = Instant::now();

        // CC onPktReceived
        // pktCount++

        let seq_number = packet.header.seq_number;

        {
            let mut flow = self.flow.write().await;
            flow.on_pkt_arrival();

            if seq_number.number() % PROBE_MODULO == 0 {
                flow.on_probe1_arrival();
            } else if seq_number.number() % PROBE_MODULO == 1 {
                flow.on_probe2_arrival();
            }
        }

        // trace_rcv++
        // recv_total++
        let offset = seq_number - state.last_sent_ack;
        if offset < 0 {
            // eprintln!!("seq number is too late");
            return Ok(());
        }
        if self.rcv_buffer.read().await.get_available_buf_size() < offset as u32 {
            return Err(Error::new(
                ErrorKind::OutOfMemory,
                "not enough space in rcv buffer",
            ));
        }

        let payload_len = packet.payload_len();
        self.rcv_buffer.write().await.insert(packet)?;
        if (seq_number - state.curr_rcv_seq_number) > 1 {
            // some packets have been lost in between
            let curr_rcv_seq_number = state.curr_rcv_seq_number;
            state
                .rcv_loss_list
                .insert(curr_rcv_seq_number + 1, seq_number - 1);

            // send NAK immedialtely
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
            let nak_packet =
                UdtControlPacket::new_nak(loss_list, self.peer_socket_id.read().await.unwrap_or(0));
            self.send_packet(nak_packet.into()).await?;
            // TODO increment NAK stats
        }

        if payload_len < self.get_max_payload_size().await {
            state.next_ack_time = Instant::now();
        }

        if seq_number - state.curr_rcv_seq_number > 0 {
            state.curr_rcv_seq_number = seq_number;
        } else {
            state.rcv_loss_list.remove(seq_number);
        }

        Ok(())
    }

    pub async fn get_max_payload_size(&self) -> u32 {
        let configuration = self.configuration.read().await;
        match self.peer_addr.map(|a| a.ip()) {
            Some(IpAddr::V6(_)) => configuration.mss - 40 - UDT_HEADER_SIZE,
            _ => configuration.mss - 28 - UDT_HEADER_SIZE,
        }
    }

    pub(crate) async fn send_packet(&self, packet: UdtPacket) -> Result<()> {
        if let Some(addr) = self.peer_addr {
            self.send_to(&addr, packet).await?;
        }
        Ok(())
    }

    async fn send_ack(&self, state: &mut SocketState, light: bool) -> Result<()> {
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

        if light {
            // Save time on buffer procesing and bandwith measurement
            let ack_packet = UdtControlPacket::new_ack(
                0.into(),
                seq_number,
                self.peer_socket_id.read().await.unwrap(),
                None,
            );
            self.send_packet(ack_packet.into()).await?;
            return Ok(());
        }

        let to_ack: i32 = seq_number - state.last_sent_ack;
        if to_ack > 0 {
            self.rcv_buffer.write().await.ack_data(seq_number);
            state.last_sent_ack = seq_number;
            self.rcv_notify.notify_waiters();
        } else if seq_number == state.last_sent_ack {
            let flow = self.flow.read().await;
            if state.last_sent_ack_time.elapsed() < (flow.rtt + 4 * flow.rtt_var) {
                return Ok(());
            }
        } else {
            return Ok(());
        }

        if (state.last_sent_ack - state.last_ack2_received) > 0 {
            state.last_ack_seq_number = state.last_ack_seq_number + 1;
            let flow = self.flow.read().await;
            let mut ack_info = AckOptionalInfo {
                rtt: flow.rtt.as_micros().try_into().unwrap_or(u32::MAX),
                rtt_variance: flow.rtt_var.as_micros().try_into().unwrap_or(u32::MAX),
                available_buf_size: std::cmp::max(
                    self.rcv_buffer.read().await.get_available_buf_size(),
                    2,
                ),
                // TODO: use rcv_time_window to keep track of bandwidth / recv speed
                pack_recv_rate: 0,
                link_capacity: 0,
            };
            if state.last_sent_ack_time.elapsed() > SYN_INTERVAL {
                ack_info.pack_recv_rate = flow.get_pkt_rcv_speed();
                ack_info.link_capacity = flow.get_bandwidth();
                state.last_sent_ack_time = Instant::now();
            }
            let ack_packet = UdtControlPacket::new_ack(
                state.last_ack_seq_number,
                state.last_sent_ack,
                self.peer_socket_id.read().await.unwrap(),
                Some(ack_info),
            );
            self.send_packet(ack_packet.into()).await?;
            state
                .ack_window
                .store(state.last_sent_ack, state.last_ack_seq_number);
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
        let mut state = self.state.lock().await;
        if now > state.next_ack_time {
            // TODO: use CC ack interval too
            self.send_ack(&mut state, false)
                .await
                .unwrap_or_else(|err| {
                    eprintln!("failed to send ack: {:?}", err);
                });
            state.next_ack_time = now + ACK_INTERVAL;
            state.ack_packet_counter = 0;
            state.light_ack_counter = 0;
        } else if (state.light_ack_counter + 1) * PACKETS_BETWEEN_LIGHT_ACK
            <= state.ack_packet_counter
        {
            self.send_ack(&mut state, true).await.unwrap_or_else(|err| {
                eprintln!("failed to send ack: {:?}", err);
            });
            state.light_ack_counter += 1;
        }

        // TODO use user-defined RTO
        let next_exp = {
            let flow = self.flow.read().await;
            let exp_int = state.exp_count * (flow.rtt + 4 * flow.rtt_var) + SYN_INTERVAL;
            std::cmp::max(exp_int, state.exp_count * MIN_EXP_INTERVAL)
        };
        let next_exp_time = state.last_rsp_time + next_exp;
        if now > next_exp_time {
            if state.exp_count > 16 && state.last_rsp_time.elapsed() > Duration::from_secs(5) {
                // Connection is broken
                *self.status.write().await = UdtStatus::Broken;
                self.update_snd_queue(true).await;
                return;
            }

            if self.snd_buffer.read().await.is_empty() {
                if let Some(peer_socket_id) = *self.peer_socket_id.read().await {
                    let keep_alive = UdtControlPacket::new_keep_alive(peer_socket_id);
                    self.send_packet(keep_alive.into())
                        .await
                        .unwrap_or_else(|err| {
                            eprintln!("failed to send keep alive: {:?}", err);
                        });
                }
            } else {
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
                self.update_snd_queue(true).await;
            }

            state.exp_count += 1;
            // Reset last response time since we just sent a heart-beat.
            state.last_rsp_time = now;
        }
    }

    async fn update_snd_queue(&self, reschedule: bool) {
        if let Some(lock) = self.multiplexer.upgrade() {
            let mux = lock.read().await;
            mux.snd_queue.update(self.socket_id, reschedule).await;
        }
    }

    pub async fn send(&self, data: &[u8]) -> Result<()> {
        if self.socket_type != SocketType::Stream {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "socket needs to be configured in stream mode to send data buffer",
            ));
        }
        if self.status().await != UdtStatus::Connected {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket is not connected",
            ));
        }

        if data.is_empty() {
            return Ok(());
        }

        if self.snd_buffer.read().await.is_empty() {
            // delay the EXP timer to avoid mis-fired timeout
            self.state.lock().await.last_rsp_time = Instant::now();
        }

        self.snd_buffer
            .write()
            .await
            .add_message(data, None, false)?;
        self.update_snd_queue(false).await;
        Ok(())
    }

    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        if self.socket_type != SocketType::Stream {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "cannot recv on non-stream socket",
            ));
        }
        let status = self.status().await;
        if status == UdtStatus::Broken || status == UdtStatus::Closing {
            if !self.rcv_buffer.read().await.has_data_to_read() {
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

        if !self.rcv_buffer.read().await.has_data_to_read() {
            self.rcv_notify.notified().await;
        }

        let status = self.status().await;
        if status == UdtStatus::Broken || status == UdtStatus::Closing {
            if !self.rcv_buffer.read().await.has_data_to_read() {
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

        let written = self.rcv_buffer.write().await.read_buffer(buf);

        // TODO: handle UDT timeout

        Ok(written)
    }

    pub(crate) async fn connect(&mut self, addr: SocketAddr) -> Result<()> {
        if self.status().await != UdtStatus::Init {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("expected status Init, found {:?}", self.status),
            ));
        }

        self.open().await;
        {
            let mut udt = Udt::get().write().await;
            udt.update_mux(self, None).await?;
        }

        *self.status.write().await = UdtStatus::Connecting;
        self.peer_addr = Some(addr);

        // TODO: use rendezvous queue?

        let configuration = self.configuration.read().await;

        let hs = HandShakeInfo {
            udt_version: configuration.udt_version(),
            initial_seq_number: self.initial_seq_number,
            max_packet_size: configuration.mss,
            max_window_size: std::cmp::min(
                self.flow.read().await.flow_window_size,
                self.rcv_buffer.read().await.get_available_buf_size(),
            ),
            connection_type: 1,
            socket_type: self.socket_type,
            socket_id: self.socket_id,
            ip_address: addr.ip(),
            syn_cookie: 0,
        };
        let hs_packet = UdtControlPacket::new_handshake(hs, 0);
        self.send_to(&addr, hs_packet.into()).await?;

        Ok(())
    }

    pub async fn status(&self) -> UdtStatus {
        *self.status.read().await
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

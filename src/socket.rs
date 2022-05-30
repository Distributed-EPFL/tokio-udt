use crate::ack_window::AckWindow;
use crate::configuration::UdtConfiguration;
use crate::control_packet::{AckOptionalInfo, ControlPacketType, HandShakeInfo, UdtControlPacket};
use crate::data_packet::UdtDataPacket;
use crate::flow::{UdtFlow, PROBE_MODULO};
use crate::loss_list::RcvLossList;
use crate::multiplexer::UdtMultiplexer;
use crate::packet::{UdtPacket, UDT_HEADER_SIZE};
use crate::queue::{RcvBuffer, SndBuffer};
use crate::seq_number::{AckSeqNumber, SeqNumber};
use crate::udt::{SocketRef, Udt};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Weak};
use tokio::io::{Error, ErrorKind, Result};
use tokio::sync::{Notify, RwLock};
use tokio::time::{Duration, Instant};

const SYN_INTERVAL: Duration = Duration::from_millis(10);
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
    pub status: UdtStatus,
    socket_type: SocketType,
    listen_socket: Option<SocketId>,
    pub peer_addr: Option<SocketAddr>,
    pub peer_socket_id: Option<SocketId>,
    pub initial_seq_number: SeqNumber,

    pub queued_sockets: BTreeSet<SocketId>,
    pub accepted_socket: BTreeSet<SocketId>,
    pub backlog_size: usize,
    pub multiplexer: Weak<RwLock<UdtMultiplexer>>,
    pub configuration: UdtConfiguration,

    rcv_buffer: RcvBuffer,
    snd_buffer: SndBuffer,
    flow_window_size: u32,
    flow: UdtFlow,
    self_ip: Option<IpAddr>,
    start_time: Instant,
    last_rsp_time: Instant,

    // Receiving related,
    last_sent_ack: SeqNumber,
    last_sent_ack_time: Instant,
    curr_rcv_seq_number: SeqNumber,
    last_ack_seq_number: AckSeqNumber,
    rcv_loss_list: RcvLossList,
    last_ack2_received: SeqNumber,
    rcv_notify: Notify,

    // Sending related
    last_ack_received: SeqNumber,
    last_data_ack_processed: SeqNumber,
    last_ack2_sent_back: AckSeqNumber,
    curr_snd_seq_number: SeqNumber,
    last_ack2_time: Instant,
    snd_loss_list: RcvLossList,

    next_ack_time: Instant,
    interpacket_interval: Duration,
    ack_packet_counter: usize,
    light_ack_counter: usize,

    ack_window: AckWindow,

    exp_count: u32,
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
            status: UdtStatus::Init,
            initial_seq_number,
            peer_addr: None,
            peer_socket_id: None,
            listen_socket: None,
            queued_sockets: BTreeSet::new(),
            accepted_socket: BTreeSet::new(),
            backlog_size: 0,
            multiplexer: Weak::new(),
            snd_buffer: SndBuffer::new(configuration.mss),
            rcv_buffer: RcvBuffer::new(configuration.rcv_buf_size, initial_seq_number),
            flow: UdtFlow::default(),
            flow_window_size: 0,
            self_ip: None,
            start_time: now,
            last_rsp_time: now,
            last_ack_seq_number: AckSeqNumber::zero(),
            rcv_loss_list: RcvLossList::new(configuration.flight_flag_size),
            curr_rcv_seq_number: initial_seq_number - 1,

            next_ack_time: now + SYN_INTERVAL,
            interpacket_interval: Duration::from_micros(10),
            ack_packet_counter: 0,
            light_ack_counter: 0,

            exp_count: 1,
            last_ack_received: initial_seq_number - 1,
            last_sent_ack: initial_seq_number - 1,
            last_sent_ack_time: now,
            last_ack2_received: initial_seq_number.number().into(),

            curr_snd_seq_number: initial_seq_number - 1,
            last_ack2_sent_back: initial_seq_number.number().into(),
            last_ack2_time: now,
            last_data_ack_processed: initial_seq_number,
            snd_loss_list: RcvLossList::new(configuration.flight_flag_size * 2),
            ack_window: AckWindow::new(1024),
            configuration,
            rcv_notify: Notify::new(),
        }
    }

    pub fn with_peer(mut self, peer: SocketAddr, peer_socket_id: SocketId) -> Self {
        self.peer_addr = Some(peer);
        self.peer_socket_id = Some(peer_socket_id);
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

    pub fn open(&mut self) {
        self.status = UdtStatus::Opened;
    }

    pub(crate) async fn connect_on_handshake(
        mut self,
        peer: SocketAddr,
        mut hs: HandShakeInfo,
    ) -> Result<SocketRef> {
        if hs.max_packet_size > self.configuration.mss {
            hs.max_packet_size = self.configuration.mss;
        } else {
            self.configuration.mss = hs.max_packet_size;
        }

        self.flow_window_size = hs.max_window_size;
        hs.max_window_size = std::cmp::min(
            self.configuration.rcv_buf_size,
            self.configuration.flight_flag_size,
        );
        self.set_self_ip(hs.ip_address);
        hs.ip_address = peer.ip();

        // TODO: use network information cache to set RTT, bandwidth, etc.
        // TODO: init congestion control

        self.peer_addr = Some(peer);
        self.status = UdtStatus::Connected;

        let packet = UdtControlPacket::new_handshake(
            hs,
            self.peer_socket_id.expect("peer_socket_id not defined"),
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

    pub fn set_self_ip(&mut self, ip: IpAddr) {
        self.self_ip = Some(ip);
    }

    pub async fn self_addr(&self) -> Option<SocketAddr> {
        if let Some(mux) = self.multiplexer.upgrade() {
            return Some(mux.read().await.get_local_addr());
        }
        None
    }

    pub(crate) async fn next_data_packet(&mut self) -> Result<Option<(UdtDataPacket, Instant)>> {
        if [UdtStatus::Broken, UdtStatus::Closed, UdtStatus::Closing].contains(&self.status) {
            return Err(Error::new(
                ErrorKind::BrokenPipe,
                "socket is closed or broken",
            ));
        }
        let now = Instant::now();
        let mut probe = false;

        let packet = match self.snd_loss_list.pop_after(self.last_data_ack_processed) {
            Some(seq) => {
                // Loss retransmission has priority
                let offset = seq - self.last_data_ack_processed;
                if offset < 0 {
                    eprintln!("unexpected offset in sender loss list");
                    return Ok(None);
                }
                match self.snd_buffer.read_data(
                    offset as usize,
                    seq,
                    self.peer_socket_id.unwrap(),
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
                            self.peer_socket_id.unwrap(),
                        );
                        self.send_packet(drop.into()).await?;
                        self.snd_loss_list
                            .remove_all(self.last_data_ack_processed, end);
                        if end + 1 - self.curr_snd_seq_number > 0 {
                            self.curr_snd_seq_number = end + 1;
                        }
                        return Ok(None);
                    }
                    Ok(packet) => packet,
                }
            }
            None => {
                // TODO: check congestion window
                let window_size = self.flow_window_size;
                if (self.curr_snd_seq_number - self.last_ack_received) < window_size as i32 {
                    return Ok(None);
                }
                match self.snd_buffer.fetch(
                    self.curr_snd_seq_number + 1,
                    self.peer_socket_id.unwrap(),
                    self.start_time,
                ) {
                    Some(packet) => {
                        self.curr_snd_seq_number = self.curr_snd_seq_number + 1;
                        if self.curr_snd_seq_number.number() % 16 == 0 {
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
        Ok(Some((packet, now + self.interpacket_interval)))
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
        if self.status == UdtStatus::Closing || self.status == UdtStatus::Closed {
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
        if hs.udt_version != self.configuration.udt_version() || hs.socket_type != self.socket_type
        {
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
            .new_connection(self.socket_id, addr, hs)
            .await?;
        // Send handshake packet in case of errors on connection?

        Ok(())
    }

    pub(crate) async fn process_packet(&mut self, packet: UdtPacket) -> Result<()> {
        match packet {
            UdtPacket::Control(ctrl) => self.process_ctrl(ctrl).await,
            UdtPacket::Data(data) => self.process_data(data).await,
        }
    }

    async fn process_ctrl(&mut self, packet: UdtControlPacket) -> Result<()> {
        self.exp_count = 1;
        let now = Instant::now();
        self.last_rsp_time = now;

        match packet.packet_type {
            ControlPacketType::Handshake(_) => {
                // TODO Handle repeated handshake
            }
            ControlPacketType::KeepAlive => (),
            ControlPacketType::Ack(ref ack) => {
                match &ack.info {
                    None => {
                        let seq = ack.next_seq_number;
                        if (seq - self.last_ack_received) >= 0 {
                            self.last_ack_received = seq;
                            self.flow_window_size -= (seq - self.last_ack_received) as u32;
                        }
                    }
                    Some(extra) => {
                        let ack_seq = packet.ack_seq_number().unwrap();
                        if self.last_ack2_time.elapsed() > SYN_INTERVAL
                            || ack_seq == self.last_ack2_sent_back
                        {
                            if let Some(peer) = self.peer_socket_id {
                                let ack2_packet = UdtControlPacket::new_ack2(ack_seq, peer);
                                self.send_packet(ack2_packet.into()).await?;
                                self.last_ack2_sent_back = ack_seq;
                                self.last_ack2_time = Instant::now();
                            }
                        }

                        let seq = ack.next_seq_number;

                        if (seq - self.curr_rcv_seq_number) > 1 {
                            // This should not happen
                            eprintln!("Udt socket broken: seq number is larger than expected");
                            self.status = UdtStatus::Broken;
                        }

                        if (seq - self.last_ack_received) >= 0 {
                            self.flow_window_size = extra.available_buf_size;
                            self.last_ack_received = seq;
                        }

                        let offset = seq - self.last_data_ack_processed;
                        if offset <= 0 {
                            // Ignore Repeated acks
                            return Ok(());
                        }

                        self.snd_buffer.ack_data(offset);
                        self.snd_loss_list
                            .remove_all(self.last_data_ack_processed, seq - 1);
                        // TODO record times for monitoring purposes
                        self.last_data_ack_processed = seq;
                        self.update_snd_queue(false).await;

                        self.flow
                            .update_rtt(Duration::from_millis(extra.rtt.into()));
                        self.flow
                            .update_rtt_var(Duration::from_millis(extra.rtt_variance.into()));
                        if extra.pack_recv_rate > 0 {
                            self.flow.update_peer_delivery_rate(extra.pack_recv_rate);
                        }
                        if extra.link_capacity > 0 {
                            self.flow.update_bandwidth(extra.link_capacity);
                        }
                        // TODO: CCC
                    }
                }
            }
            ControlPacketType::Ack2 => {
                let ack_seq = packet.ack_seq_number().unwrap();
                if let Some((seq, rtt)) = self.ack_window.get(ack_seq) {
                    let rtt_abs_diff = {
                        if rtt > self.flow.rtt {
                            rtt - self.flow.rtt
                        } else {
                            self.flow.rtt - rtt
                        }
                    };
                    self.flow.update_rtt_var(rtt_abs_diff);
                    self.flow.update_rtt(rtt);
                    if (seq - self.last_ack2_received) > 0 {
                        self.last_ack2_received = seq;
                    }
                }
            }
            ControlPacketType::Nak(ref nak) => {
                let mut broken = false;
                let loss_iter = &mut nak.loss_info.iter();
                while let Some(loss) = loss_iter.next() {
                    let (seq_start, seq_end) = {
                        if loss & 0x8000000 != 0 {
                            if let Some(seq_end) = loss_iter.next() {
                                let seq_start: SeqNumber = (loss & 0x7fffffff).into();
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
                    if (seq_start - seq_end > 0) || (seq_end - self.curr_snd_seq_number > 0) {
                        broken = true;
                        break;
                    }
                    if seq_start - self.last_ack_received >= 0 {
                        self.snd_loss_list.insert(seq_start, seq_end);
                    } else if seq_end - self.last_ack_received >= 0 {
                        self.snd_loss_list.insert(self.last_ack_received, seq_end);
                    }
                }

                if broken {
                    self.status = UdtStatus::Broken;
                    return Ok(());
                }

                self.update_snd_queue(true).await;
            }
            ControlPacketType::Shutdown => {
                self.status = UdtStatus::Closing;
            }
            ControlPacketType::MsgDropRequest(ref drop) => {
                let msg_number = packet.msg_seq_number().unwrap();
                self.rcv_buffer.drop_msg(msg_number);
                self.rcv_loss_list
                    .remove_all(drop.first_seq_number, drop.last_seq_number);
                if (drop.first_seq_number - (self.curr_rcv_seq_number + 1)) <= 0
                    && (drop.last_seq_number - self.curr_rcv_seq_number) > 0
                {
                    self.curr_rcv_seq_number = drop.last_seq_number;
                }
            }
            ControlPacketType::UserDefined => unimplemented!(),
        }
        Ok(())
    }

    async fn process_data(&mut self, packet: UdtDataPacket) -> Result<()> {
        self.last_rsp_time = Instant::now();

        // CC onPktReceived
        // pktCount++

        self.flow.on_pkt_arrival();

        let seq_number = packet.header.seq_number;
        if seq_number.number() % PROBE_MODULO == 0 {
            self.flow.on_probe1_arrival();
        } else if seq_number.number() % PROBE_MODULO == 1 {
            self.flow.on_probe2_arrival();
        }

        // trace_rcv++
        // recv_total++
        let offset = seq_number - self.last_sent_ack;
        if offset < 0 {
            return Err(Error::new(ErrorKind::InvalidData, "seq number is too late"));
        }
        if self.rcv_buffer.get_available_buf_size() < offset as u32 {
            return Err(Error::new(
                ErrorKind::OutOfMemory,
                "not enough space in rcv buffer",
            ));
        }

        let payload_len = packet.payload_len();
        self.rcv_buffer.insert(packet)?;
        if seq_number > self.curr_rcv_seq_number + 1 {
            // some packets have been lost in between
            self.rcv_loss_list
                .insert(self.curr_rcv_seq_number + 1, seq_number - 1);

            // send NAK immedialtely
            let loss_list = {
                if self.curr_rcv_seq_number + 1 == seq_number - 1 {
                    vec![(seq_number - 1).number()]
                } else {
                    vec![
                        (self.curr_rcv_seq_number + 1).number() | 0x80000000,
                        (seq_number - 1).number(),
                    ]
                }
            };
            let nak_packet = UdtControlPacket::new_nak(loss_list, self.peer_socket_id.unwrap_or(0));
            self.send_packet(nak_packet.into()).await?;
            // TODO increment NAK stats
        }

        if payload_len < self.get_max_payload_size() {
            self.next_ack_time = Instant::now();
        }

        if seq_number - self.curr_rcv_seq_number > 0 {
            self.curr_rcv_seq_number = seq_number;
        } else {
            self.rcv_loss_list.remove(seq_number);
        }

        Ok(())
    }

    pub fn get_max_payload_size(&self) -> u32 {
        match self.peer_addr.map(|a| a.ip()) {
            Some(IpAddr::V6(_)) => self.configuration.mss - 40 - UDT_HEADER_SIZE,
            _ => self.configuration.mss - 28 - UDT_HEADER_SIZE,
        }
    }

    pub(crate) async fn send_packet(&self, packet: UdtPacket) -> Result<()> {
        if let Some(addr) = self.peer_addr {
            self.send_to(&addr, packet).await?;
        }
        Ok(())
    }

    async fn send_ack(&mut self, light: bool) -> Result<()> {
        let seq_number = match self.rcv_loss_list.peek_after(self.curr_rcv_seq_number + 1) {
            Some(num) => num,
            None => self.curr_rcv_seq_number + 1,
        };

        if seq_number == self.last_ack2_received {
            return Ok(());
        }

        if light {
            // Save time on buffer procesing and bandwith measurement
            let ack_packet =
                UdtControlPacket::new_ack(0.into(), seq_number, self.peer_socket_id.unwrap(), None);
            self.send_packet(ack_packet.into()).await?;
            return Ok(());
        }

        let to_ack = seq_number - self.last_sent_ack;
        if to_ack > 0 {
            self.rcv_buffer.ack_data(seq_number);
            self.last_sent_ack = seq_number;
            self.rcv_notify.notify_waiters();
        } else if seq_number == self.last_sent_ack {
            if self.last_sent_ack_time.elapsed() < (self.flow.rtt + 4 * self.flow.rtt_var) {
                return Ok(());
            }
        } else {
            return Ok(());
        }

        if (self.last_sent_ack - self.last_ack2_received) > 0 {
            self.last_ack_seq_number = self.last_ack_seq_number + 1;
            let mut ack_info = AckOptionalInfo {
                rtt: self.flow.rtt.as_micros().try_into().unwrap_or(u32::MAX),
                rtt_variance: self.flow.rtt_var.as_micros().try_into().unwrap_or(u32::MAX),
                available_buf_size: std::cmp::max(self.rcv_buffer.get_available_buf_size(), 2),
                // TODO: use rcv_time_window to keep track of bandwidth / recv speed
                pack_recv_rate: 0,
                link_capacity: 0,
            };
            if self.last_sent_ack_time.elapsed() > SYN_INTERVAL {
                ack_info.pack_recv_rate = self.flow.get_pkt_rcv_speed();
                ack_info.link_capacity = self.flow.get_bandwidth();
                self.last_sent_ack_time = Instant::now();
            }
            let ack_packet = UdtControlPacket::new_ack(
                self.last_ack_seq_number,
                self.last_sent_ack,
                self.peer_socket_id.unwrap(),
                Some(ack_info),
            );
            self.send_packet(ack_packet.into()).await?;
            self.ack_window
                .store(self.last_sent_ack, self.last_ack_seq_number);
        }

        Ok(())
    }

    fn cc_update(&mut self) {
        // TODO update CC parameters

        //self.interpacket_interval = ...
    }

    pub(crate) async fn check_timers(&mut self) {
        self.cc_update();
        let now = Instant::now();
        if now > self.next_ack_time {
            // TODO: use CC ack interval too
            self.send_ack(false).await.unwrap_or_else(|err| {
                eprintln!("failed to send ack: {:?}", err);
            });
            self.next_ack_time = now + ACK_INTERVAL;

            self.ack_packet_counter = 0;
            self.light_ack_counter = 0;
        } else if (self.light_ack_counter + 1) * PACKETS_BETWEEN_LIGHT_ACK
            <= self.ack_packet_counter
        {
            self.send_ack(true).await.unwrap_or_else(|err| {
                eprintln!("failed to send ack: {:?}", err);
            });
            self.light_ack_counter += 1;
        }

        // TODO use user-defined RTO
        let next_exp = {
            let exp_int = self.exp_count * (self.flow.rtt + 4 * self.flow.rtt_var) + SYN_INTERVAL;
            std::cmp::max(exp_int, self.exp_count * MIN_EXP_INTERVAL)
        };
        let next_exp_time = self.last_rsp_time + next_exp;
        if now > next_exp_time {
            if self.exp_count > 16 && self.last_rsp_time.elapsed() > Duration::from_secs(5) {
                // Connection is broken
                self.status = UdtStatus::Broken;
                self.update_snd_queue(true).await;
                return;
            }

            if self.snd_buffer.is_empty() {
                let keep_alive = UdtControlPacket::new_keep_alive(self.peer_socket_id.unwrap());
                self.send_packet(keep_alive.into())
                    .await
                    .unwrap_or_else(|err| {
                        eprintln!("failed to send keep alive: {:?}", err);
                    });
            } else {
                if (self.last_ack_received != self.curr_snd_seq_number + 1)
                    && self.snd_loss_list.is_empty()
                {
                    self.snd_loss_list
                        .insert(self.last_ack_received, self.curr_snd_seq_number);
                }

                // TODO: CC onTimeout
                self.cc_update();
                self.update_snd_queue(true).await;
            }

            self.exp_count += 1;
            // Reset last response time since we just sent a heart-beat.
            self.last_rsp_time = now;
        }
    }

    async fn update_snd_queue(&self, reschedule: bool) {
        if let Some(lock) = self.multiplexer.upgrade() {
            let mux = lock.read().await;
            mux.snd_queue.update(self.socket_id, reschedule).await;
        }
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        if self.socket_type != SocketType::Stream {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "socket needs to be configured in stream mode to send data buffer",
            ));
        }
        if self.status != UdtStatus::Connected {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket is not connected",
            ));
        }

        if data.is_empty() {
            return Ok(());
        }

        if self.snd_buffer.is_empty() {
            // delay the EXP timer to avoid mis-fired timeout
            self.last_rsp_time = Instant::now();
        }

        // TODO limit snd buffer size
        self.snd_buffer.add_message(data, None, false);
        self.update_snd_queue(false).await;
        Ok(())
    }

    pub async fn recv(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.socket_type != SocketType::Stream {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "cannot recv on non-stream socket",
            ));
        }
        if self.status == UdtStatus::Broken || self.status == UdtStatus::Closing {
            if !self.rcv_buffer.has_data_to_read() {
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "connection was closed or broken",
                ));
            }
        } else if self.status != UdtStatus::Connected {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket not connected",
            ));
        }

        if buf.is_empty() {
            return Ok(0);
        }

        if !self.rcv_buffer.has_data_to_read() {
            self.rcv_notify.notified().await;
        }

        if self.status == UdtStatus::Broken || self.status == UdtStatus::Closing {
            if !self.rcv_buffer.has_data_to_read() {
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "connection was closed or broken",
                ));
            }
        } else if self.status != UdtStatus::Connected {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "UDT socket not connected",
            ));
        }

        let written = self.rcv_buffer.read_buffer(buf);

        // TODO: handle UDT timeout

        Ok(written)
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

#[derive(Debug, PartialEq)]
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

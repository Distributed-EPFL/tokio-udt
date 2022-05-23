use crate::configuration::UdtConfiguration;
use crate::control_packet::{ControlPacketType, HandShakeInfo, UdtControlPacket};
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
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

const SYN_INTERVAL: Duration = Duration::from_millis(10);
const MIN_NAK_INTERVAL: Duration = Duration::from_millis(300);

pub type SocketId = u32;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum SocketType {
    Stream = 0,
    Datagram = 1,
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
    curr_rcv_seq_number: SeqNumber,
    last_ack_seq_number: AckSeqNumber,
    rcv_loss_list: RcvLossList,
    last_ack2_received: AckSeqNumber,

    // Sending related
    last_ack_received: SeqNumber,
    last_data_ack_processed: SeqNumber,
    last_ack2_sent_back: AckSeqNumber,
    curr_snd_seq_number: SeqNumber,
    last_ack2_time: Instant,
    snd_loss_list: RcvLossList,

    next_ack_time: Instant,
    next_nak_time: Instant,
    interpacket_interval: Duration,

    exp_count: usize,
}

impl UdtSocket {
    pub(crate) fn new(socket_id: SocketId, socket_type: SocketType) -> Self {
        let now = Instant::now();
        let initial_seq_number = SeqNumber::random();
        let configuration = UdtConfiguration::default();
        Self {
            socket_id,
            socket_type,
            status: UdtStatus::Init,
            initial_seq_number: initial_seq_number,
            peer_addr: None,
            peer_socket_id: None,
            listen_socket: None,
            queued_sockets: BTreeSet::new(),
            accepted_socket: BTreeSet::new(),
            backlog_size: 0,
            multiplexer: Weak::new(),
            snd_buffer: SndBuffer::new(configuration.mss),
            rcv_buffer: RcvBuffer::new(configuration.rcv_buf_size),
            flow: UdtFlow::default(),
            flow_window_size: 0,
            self_ip: None,
            start_time: now,
            last_rsp_time: now,
            last_ack_seq_number: AckSeqNumber::zero(),
            rcv_loss_list: RcvLossList::new(configuration.flight_flag_size),
            curr_rcv_seq_number: initial_seq_number - 1,

            next_ack_time: now + SYN_INTERVAL,
            next_nak_time: now + MIN_NAK_INTERVAL,
            interpacket_interval: Duration::from_micros(10),

            exp_count: 1,
            last_ack_received: initial_seq_number - 1,
            last_sent_ack: initial_seq_number - 1,
            last_ack2_received: initial_seq_number.number().into(),

            curr_snd_seq_number: initial_seq_number - 1,
            last_ack2_sent_back: initial_seq_number.number().into(),
            last_ack2_time: now,
            last_data_ack_processed: initial_seq_number,
            snd_loss_list: RcvLossList::new(configuration.flight_flag_size * 2),
            configuration: configuration,
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

    pub fn with_initial_seq_number(mut self, isn: SeqNumber) -> Self {
        self.initial_seq_number = isn;
        self
    }

    pub fn open(&mut self) {
        // TODO: init packet_size, payload_size, etc.
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
            let mut mux = lock.write().await;
            mux.rcv_queue.push_back(self.socket_id);
            mux.send_to(&peer, UdtPacket::Control(packet)).await?;
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

    pub fn send_next_packet(&mut self) -> Result<()> {
        !unimplemented!()
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
            ControlPacketType::Handshake(_) => (),
            ControlPacketType::KeepAlive => (),
            ControlPacketType::Ack(ref ack) => {
                match &ack.info {
                    None => {
                        let seq = ack.next_seq_number;
                        if seq >= self.last_ack_received {
                            self.last_ack_received = seq;
                            // TODO: Update flow window size?
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

                        let offset = self.last_data_ack_processed - seq;
                        if offset <= 0 {
                            // Ignore Repeated acks
                            return Ok(());
                        }

                        self.snd_buffer.ack_data(offset);
                        self.snd_loss_list
                            .remove_all(self.last_data_ack_processed, seq - 1);
                        // TODO record times for monitoring purposes
                        self.last_data_ack_processed = seq;
                        if let Some(lock) = self.multiplexer.upgrade() {
                            let mux = lock.read().await;
                            mux.snd_queue.update(self.socket_id, false).await;
                        }

                        // TODO: CCC
                    }
                }
            }
            ControlPacketType::Ack2 => {
                let ack_seq = packet.ack_seq_number().unwrap();
                // TODO: Compute RTT
                if (ack_seq - self.last_ack2_received) > 0 {
                    self.last_ack2_received = ack_seq;
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

                if let Some(lock) = self.multiplexer.upgrade() {
                    let mux = lock.read().await;
                    mux.snd_queue.update(self.socket_id, true).await;
                }
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
        let offset = self.last_sent_ack - seq_number;
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

    async fn send_packet(&self, packet: UdtPacket) -> Result<()> {
        if let Some(addr) = self.peer_addr {
            self.send_to(&addr, packet).await?;
        }
        Ok(())
    }

    fn cc_update(&mut self) {
        // TODO update CC parameters

        //self.interpacket_interval = ...
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

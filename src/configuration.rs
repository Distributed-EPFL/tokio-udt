use std::time::Duration;

const DEFAULT_MSS: u32 = 1500;
const DEFAULT_UDT_BUF_SIZE: u32 = 81920;
const DEFAULT_UDP_BUF_SIZE: usize = 8_000_000;
const UDT_VERSION: u32 = 4;

/// Options for UDT protocol
#[derive(Debug, Clone)]
pub struct UdtConfiguration {
    /// Packet size: the optimal size is the network MTU size. The default value is 1500 bytes.
    /// A UDT connection will choose the smaller value of the MSS between the two peer sides.
    pub mss: u32,
    /// Maximum window size (nb of packets).  
    /// Internal parameter: you should set it to not less than `rcv_buf_size`.
    /// Default: 256000
    pub flight_flag_size: u32,
    /// Size of temporary storage for packets to send (nb of packets)
    pub snd_buf_size: u32,
    /// Size of temporary storage for packets to receive (nb of packets)
    pub rcv_buf_size: u32,
    /// UDT uses UDP as the data channel, so the UDP buffer size may affect the performance.
    /// The sending buffer size is applied on the UDP socket. The actual value used
    /// by the kernel is bounded by "net.core.wmem_max".
    pub udp_snd_buf_size: usize,
    /// UDT uses UDP as the data channel, so the UDP buffer size may affect the performance.
    /// The receiving buffer size is applied on the UDP socket. The actual value used
    /// by the kernel is bounded by "net.core.rmem_max".
    pub udp_rcv_buf_size: usize,
    /// Whether SO_REUSEPORT option should be set on the UDP socket.
    /// On Linux, this option can be useful to load-balance packets
    /// from multiple clients to distinct threads and distinct UDT multiplexers.
    /// Default: false.
    pub udp_reuse_port: bool,
    /// Whether a potential existing UDT multiplexer (and associated UDP socket)
    /// should be reused when binding the same port. The preexisting listener
    /// must have been created with this option set to true.
    /// For optimal throughput from multiple clients, using
    /// `udp_reuse_port` may be preferable.
    /// Default: true
    pub reuse_mux: bool,
    /// UDT rendez-vous mode. (NOT IMPLEMENTED)
    pub rendezvous: bool,
    /// Maximum number of pending UDT connections to accept. Default: 1000
    pub accept_queue_size: usize,
    /// Linger time on close(). Default: 10 seconds
    pub linger_timeout: Option<Duration>,
}

impl UdtConfiguration {
    pub fn udt_version(&self) -> u32 {
        UDT_VERSION
    }
}

impl Default for UdtConfiguration {
    fn default() -> Self {
        Self {
            mss: DEFAULT_MSS,
            flight_flag_size: 256000,
            snd_buf_size: DEFAULT_UDT_BUF_SIZE,
            rcv_buf_size: DEFAULT_UDT_BUF_SIZE * 2,
            udp_snd_buf_size: DEFAULT_UDP_BUF_SIZE,
            udp_rcv_buf_size: DEFAULT_UDP_BUF_SIZE,
            udp_reuse_port: false,
            linger_timeout: Some(Duration::from_secs(10)),
            reuse_mux: true,
            rendezvous: false,
            accept_queue_size: 1000,
        }
    }
}

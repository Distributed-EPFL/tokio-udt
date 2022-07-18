const DEFAULT_MSS: u32 = 1500;
const DEFAULT_UDT_BUF_SIZE: u32 = 81920;
const DEFAULT_UDP_BUF_SIZE: usize = 8_000_000;
const UDT_VERSION: u32 = 4;

#[derive(Debug, Clone)]
pub struct UdtConfiguration {
    pub mss: u32,
    pub flight_flag_size: u32,
    pub snd_buf_size: u32,
    pub rcv_buf_size: u32,
    pub udp_snd_buf_size: usize,
    pub udp_rcv_buf_size: usize,
    pub udp_reuse_port: bool,
    pub reuse_mux: bool,
    // snd_timeout
    // rcv_timeout
    // socktype
    // ip_version
    pub rendezvous: bool,
    pub accept_queue_size: usize,
    pub linger_timeout: Option<u32>,
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
            linger_timeout: Some(180),
            reuse_mux: true,
            rendezvous: false,
            accept_queue_size: 100,
        }
    }
}

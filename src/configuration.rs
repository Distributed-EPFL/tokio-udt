const DEFAULT_MSS: u32 = 1500;
const DEFAULT_UDT_BUF_SIZE: u32 = 8192;
const DEFAULT_UDP_BUF_SIZE: usize = 8_000_000;
const UDT_VERSION: u32 = 4;

#[derive(Debug, Clone)]
pub struct UdtConfiguration {
    pub(crate) mss: u32,
    pub(crate) flight_flag_size: u32,
    pub snd_buf_size: u32,
    pub rcv_buf_size: u32,
    linger_timeout: Option<u32>,
    pub(crate) udp_snd_buf_size: usize,
    pub(crate) udp_rcv_buf_size: usize,
    pub reuse_addr: bool,
    // snd_timeout
    // rcv_timeout
    // socktype
    // ip_version
    pub(crate) rendezvous: bool,
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
            flight_flag_size: 25600,
            snd_buf_size: DEFAULT_UDT_BUF_SIZE,
            rcv_buf_size: DEFAULT_UDT_BUF_SIZE * 2,
            udp_snd_buf_size: DEFAULT_UDP_BUF_SIZE,
            udp_rcv_buf_size: DEFAULT_UDP_BUF_SIZE,
            linger_timeout: Some(180),
            reuse_addr: true,
            rendezvous: false,
        }
    }
}

const DEFAULT_MSS: u16 = 1500;
const DEFAULT_UDT_BUF_SIZE: u32 = 8192;

#[derive(Debug)]
pub(crate) struct UdtConfiguration {
    mss: u16,
    syn_sending: bool,
    syn_recving: bool,
    flight_flag_size: u32,
    snd_buf_size: u32,
    rcv_buf_size: u32,
    linger_timeout: Option<u32>,
    udp_snd_buf_size: u32,
    udp_rcv_buf_size: u32,
    reuse_addr: bool,
    // snd_timeout
    // rcv_timeout
    // socktype
    // ip_version
    // rendezvous: bool
}

impl Default for UdtConfiguration {
    fn default() -> Self {
        Self {
            mss: DEFAULT_MSS,
            syn_sending: true,
            syn_recving: true,
            flight_flag_size: 25600,
            snd_buf_size: DEFAULT_UDT_BUF_SIZE,
            rcv_buf_size: DEFAULT_UDT_BUF_SIZE,
            udp_snd_buf_size: 65536,
            udp_rcv_buf_size: DEFAULT_MSS as u32 * DEFAULT_UDT_BUF_SIZE,
            linger_timeout: Some(180),
            reuse_addr: true,
        }
    }
}

use std::net::IpAddr;

pub fn ip_to_bytes(ip: IpAddr) -> [u8; 16] {
    match ip {
        IpAddr::V4(addr) => {
            let mut bytes = [0; 16];
            bytes[0..4].copy_from_slice(&addr.octets());
            bytes
        }
        IpAddr::V6(addr) => addr.octets(),
    }
}

use std::net::IpAddr;

const MAX_SEQ_NUMBER: i32 = 0x7fffffff;
const SEQ_NUMBER_OFFSET_THRESHOLD: u32 = 0x3fffffff;

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

pub fn seq_number_offset(seq1: u32, seq2: u32) -> i32 {
    if seq1.abs_diff(seq2) < SEQ_NUMBER_OFFSET_THRESHOLD {
        return seq2 as i32 - seq1 as i32;
    }
    if seq1 < seq2 {
        return (seq2 as i32) - (seq1 as i32) - MAX_SEQ_NUMBER - 1;
    }
    (seq2 as i32) - (seq1 as i32) + MAX_SEQ_NUMBER + 1
}

use super::socket::UdtSocket;
use std::collections::VecDeque;

pub type MultiplexerId = usize;

#[derive(Debug)]
pub(crate) struct UDTMultiplexer<'a> {
    id: usize,
    port: u16,

    snd_queue: VecDeque<&'a UdtSocket>,
    rcv_queue: VecDeque<&'a UdtSocket>,
}

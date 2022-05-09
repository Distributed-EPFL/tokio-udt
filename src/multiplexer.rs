use super::configuration::UdtConfiguration;
use super::socket::UdtSocket;
use std::collections::VecDeque;

#[derive(Debug)]
struct UDTMultiplexer<'a> {
    configuration: UdtConfiguration,

    snd_queue: VecDeque<&'a UdtSocket>,
    rcv_queue: VecDeque<&'a UdtSocket>,
}

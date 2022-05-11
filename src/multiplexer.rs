use super::socket::UdtSocket;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub type MultiplexerId = usize;

#[derive(Debug)]
pub(crate) struct UdtMultiplexer {
    pub id: MultiplexerId,
    pub port: u16,

    pub snd_queue: VecDeque<Rc<RefCell<UdtSocket>>>,
    pub rcv_queue: VecDeque<Rc<RefCell<UdtSocket>>>,
}

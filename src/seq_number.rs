use rand::Rng;
use std::marker::PhantomData;

pub trait SeqConstants: Clone {
    const MAX_NUMBER: u32;

    fn threshold() -> u32 {
        Self::MAX_NUMBER / 2
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct GenericSeqNumber<T>
where
    T: SeqConstants,
{
    number: u32,
    phantom: PhantomData<T>,
}

impl<T: SeqConstants> From<u32> for GenericSeqNumber<T> {
    fn from(number: u32) -> Self {
        Self {
            number,
            phantom: PhantomData,
        }
    }
}

impl<T: SeqConstants> GenericSeqNumber<T> {
    pub const MAX_NUMBER: u32 = T::MAX_NUMBER;

    pub fn number(self) -> u32 {
        self.number
    }

    pub fn random() -> Self {
        rand::thread_rng().gen_range(0..=T::MAX_NUMBER).into()
    }

    pub fn zero() -> Self {
        Self {
            number: 0,
            phantom: PhantomData,
        }
    }

    pub fn max() -> Self {
        Self {
            number: T::MAX_NUMBER,
            phantom: PhantomData,
        }
    }
}

impl<T: SeqConstants> std::ops::Sub for GenericSeqNumber<T> {
    type Output = i32;

    #[allow(clippy::neg_multiply)]
    fn sub(self, other: Self) -> Self::Output {
        if self.number.abs_diff(other.number) <= T::threshold() {
            self.number as i32 - other.number as i32
        } else if self.number < T::threshold() {
            (self.number + T::MAX_NUMBER + 1 - other.number) as i32 * -1
        } else {
            (other.number + T::MAX_NUMBER + 1 - self.number) as i32
        }
    }
}

impl<T: SeqConstants> std::ops::Add<i32> for GenericSeqNumber<T> {
    type Output = GenericSeqNumber<T>;

    fn add(self, rhs: i32) -> Self {
        let resp = ((self.number as i64 + rhs as i64).rem_euclid(T::MAX_NUMBER as i64 + 1)) as u32;
        resp.into()
    }
}

impl<T: SeqConstants> std::ops::Sub<i32> for GenericSeqNumber<T> {
    type Output = GenericSeqNumber<T>;

    #[allow(clippy::neg_multiply)]
    fn sub(self, rhs: i32) -> Self {
        self + (rhs * -1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Copy)]
pub struct SeqNumberConstants;
impl SeqConstants for SeqNumberConstants {
    const MAX_NUMBER: u32 = 0x7fffffff;
}
pub type SeqNumber = GenericSeqNumber<SeqNumberConstants>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Copy)]
pub struct AckSeqNumberConstants;
impl SeqConstants for AckSeqNumberConstants {
    const MAX_NUMBER: u32 = 0x7fffffff;
}
pub type AckSeqNumber = GenericSeqNumber<AckSeqNumberConstants>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Copy)]
pub struct MsgNumberConstants;
impl SeqConstants for MsgNumberConstants {
    const MAX_NUMBER: u32 = 0x1fffffff;
}
pub type MsgNumber = GenericSeqNumber<MsgNumberConstants>;

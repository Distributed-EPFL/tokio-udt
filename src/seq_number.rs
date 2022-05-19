use rand::Rng;

const MAX_SEQ_NUMBER: u32 = 0x7fffffff;
const SEQ_NUMBER_OFFSET_THRESHOLD: u32 = 0x3fffffff;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) struct SeqNumber(u32);

impl From<u32> for SeqNumber {
    fn from(num: u32) -> Self {
        Self(num)
    }
}

impl SeqNumber {
    pub fn number(self) -> u32 {
        self.0
    }

    pub fn random() -> Self {
        rand::thread_rng().gen_range(0..=MAX_SEQ_NUMBER).into()
    }
}

impl std::ops::Sub for SeqNumber {
    type Output = i32;

    #[allow(clippy::neg_multiply)]
    fn sub(self, other: Self) -> Self::Output {
        if self.0.abs_diff(other.0) <= SEQ_NUMBER_OFFSET_THRESHOLD {
            self.0 as i32 - other.0 as i32
        } else if self.0 < SEQ_NUMBER_OFFSET_THRESHOLD {
            (self.0 + MAX_SEQ_NUMBER + 1 - other.0) as i32 * -1
        } else {
            (other.0 + MAX_SEQ_NUMBER + 1 - self.0) as i32
        }
    }
}

impl std::ops::Add<i32> for SeqNumber {
    type Output = SeqNumber;

    fn add(self, rhs: i32) -> Self {
        let resp = ((self.0 as i64 + rhs as i64).rem_euclid(MAX_SEQ_NUMBER as i64 + 1)) as u32;
        resp.into()
    }
}

impl std::ops::Sub<i32> for SeqNumber {
    type Output = SeqNumber;

    #[allow(clippy::neg_multiply)]
    fn sub(self, rhs: i32) -> Self {
        self + (rhs * -1)
    }
}

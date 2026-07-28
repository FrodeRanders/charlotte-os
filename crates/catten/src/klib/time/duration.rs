use crate::klib::constants::{
    MICROS_PER_SEC,
    MILLIS_PER_SEC,
    NANOS_PER_SEC,
    PICOS_PER_SEC,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtDuration {
    picos: u128,
}

impl ExtDuration {
    pub fn from_secs(secs: u128) -> Self {
        ExtDuration {
            picos: secs * PICOS_PER_SEC,
        }
    }

    pub fn from_millis(millis: u128) -> Self {
        ExtDuration {
            picos: millis * 1_000_000_000u128,
        }
    }

    pub fn from_micros(micros: u128) -> Self {
        ExtDuration {
            picos: micros * 1_000_000u128,
        }
    }

    pub fn from_nanos(nanos: u128) -> Self {
        ExtDuration {
            picos: nanos * 1_000u128,
        }
    }

    pub fn from_picos(picos: u128) -> Self {
        ExtDuration {
            picos,
        }
    }

    pub fn as_secs(&self) -> u128 {
        self.picos / PICOS_PER_SEC
    }

    pub fn as_millis(&self) -> u128 {
        self.picos / MILLIS_PER_SEC
    }

    pub fn as_micros(&self) -> u128 {
        self.picos / MICROS_PER_SEC
    }

    pub fn as_nanos(&self) -> u128 {
        self.picos / NANOS_PER_SEC
    }

    pub fn as_picos(&self) -> u128 {
        self.picos
    }
}

impl core::ops::Div for ExtDuration {
    type Output = ExtDuration;

    fn div(self, rhs: Self) -> Self::Output {
        ExtDuration {
            picos: self.picos / rhs.picos,
        }
    }
}

impl core::ops::Rem for ExtDuration {
    type Output = ExtDuration;

    fn rem(self, rhs: Self) -> Self::Output {
        ExtDuration {
            picos: self.picos % rhs.picos,
        }
    }
}

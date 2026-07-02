const FLAGS_POLARITY_MASK: u16 = 0b11;
const FLAGS_POLARITY_SHIFT: u16 = 0;
const FLAGS_TRIGGER_MASK: u16 = 0b11;
const FLAGS_TRIGGER_SHIFT: u16 = 2;

#[derive(Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum NmiSrcPolarity {
    BusSpec = 0b00,
    ActiveHigh = 0b01,
    Reserved = 0b10,
    ActiveLow = 0b11,
}

#[derive(Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum NmiSrcTrigger {
    BusSpec = 0b00,
    Edge = 0b01,
    Reserved = 0b10,
    Level = 0b11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(super) struct NmiSrcFlags(u16);

impl NmiSrcFlags {
    pub fn polarity(&self) -> NmiSrcPolarity {
        match (self.0 & FLAGS_POLARITY_MASK) >> FLAGS_POLARITY_SHIFT {
            0b00 => NmiSrcPolarity::BusSpec,
            0b01 => NmiSrcPolarity::ActiveHigh,
            0b10 => NmiSrcPolarity::Reserved,
            0b11 => NmiSrcPolarity::ActiveLow,
            _ => unreachable!(),
        }
    }

    pub fn trigger(&self) -> NmiSrcTrigger {
        match (self.0 & FLAGS_TRIGGER_MASK) >> FLAGS_TRIGGER_SHIFT {
            0b00 => NmiSrcTrigger::BusSpec,
            0b01 => NmiSrcTrigger::Edge,
            0b10 => NmiSrcTrigger::Reserved,
            0b11 => NmiSrcTrigger::Level,
            _ => unreachable!(),
        }
    }
}

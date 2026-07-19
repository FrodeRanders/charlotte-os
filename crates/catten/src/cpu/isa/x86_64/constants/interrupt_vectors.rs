pub const FIXED_INTERRUPT_VECTOR_COUNT: u8 = 36;

pub const LAPIC_TIMER_VECTOR: u8 = 32;
/// x86 scheduler wakeups may use the LAPIC timer vector directly; unlike GIC
/// PPIs, APIC vectors are valid IPI payloads.
pub const SCHEDULER_IPI_VECTOR: u8 = LAPIC_TIMER_VECTOR;
pub const ASYNC_IPI_VECTOR: u8 = 33;
pub const SYNC_IPI_VECTOR: u8 = 34;
pub const SPURIOUS_INTERRUPT_VECTOR_NUM: u8 = 255;

pub const FIXED_INTERRUPT_VECTOR_COUNT: u8 = 37;

pub const LAPIC_TIMER_VECTOR: u8 = 32;
pub const ASYNC_IPI_VECTOR: u8 = 33;
pub const SYNC_IPI_VECTOR: u8 = 34;
/// Keep scheduler wakeups distinct from the one-shot timer. The local APIC
/// records pending interrupts by vector, so sharing a vector lets a timer
/// expiry and a scheduler IPI coalesce into one delivery.
pub const SCHEDULER_IPI_VECTOR: u8 = 35;
pub const SPURIOUS_INTERRUPT_VECTOR_NUM: u8 = 255;

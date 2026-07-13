use limine::{
    BaseRevision,
    RequestsEndMarker,
    RequestsStartMarker,
    request::{
        ExecutableAddressRequest,
        FramebufferRequest,
        HhdmRequest,
        MemmapRequest,
        MpRequest,
        RsdpRequest,
        StackSizeRequest,
        TscFrequencyRequest,
    },
};

use crate::memory::allocators::memory::PageSize;

// The Limine boot protocol locates requests by scanning the loaded executable
// for their magic numbers, bounded by the start and end markers. Requests that
// are never referenced from Rust code (e.g. the base revision and the markers
// themselves) would otherwise be removed by dead-code elimination and
// `--gc-sections`, causing Limine to fall back to defaults (notably base
// revision 0, which is rejected on AArch64). We therefore mark every request
// `#[used]` and place them in dedicated `.limine_requests*` sections that the
// linker scripts explicitly `KEEP`.

#[used]
#[unsafe(link_section = ".limine_requests_start")]
pub static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static EXECUTABLE_ADDRESS_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

const MP_X2APIC_ENABLE: u64 = 1 << 0;
#[used]
#[unsafe(link_section = ".limine_requests")]
pub static MP_REQUEST: MpRequest = MpRequest::new(
    if cfg!(target_arch = "x86_64") {
        MP_X2APIC_ENABLE
    } else {
        0
    },
);

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static STACK_SIZE: StackSizeRequest =
    StackSizeRequest::new(PageSize::Standard.num_bytes() as u64 * 4);

#[used]
#[unsafe(link_section = ".limine_requests")]
pub static TSC_FREQUENCY_REQUEST: TscFrequencyRequest = TscFrequencyRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
pub static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

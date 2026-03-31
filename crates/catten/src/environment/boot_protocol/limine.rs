use limine::request::{
    ExecutableAddressRequest,
    FramebufferRequest,
    HhdmRequest,
    MemmapRequest,
    MpRequest,
    RsdpRequest,
    StackSizeRequest,
};
use limine::{BaseRevision, RequestsEndMarker, RequestsStartMarker};

use crate::cpu::isa::memory::MemoryInterfaceImpl;
use crate::memory::MemoryInterface as _;

pub static BASE_REVISION: BaseRevision = BaseRevision::new();

pub static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

pub static MEMORY_MAP_REQUEST: MemmapRequest = MemmapRequest::new();

pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

pub static EXECUTABLE_ADDRESS_REQUEST: ExecutableAddressRequest = ExecutableAddressRequest::new();

pub static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

const MP_X2APIC_ENABLE: u64 = 1 << 0;
pub static MP_REQUEST: MpRequest = MpRequest::new(
    if cfg!(target_arch = "x86_64") {
        MP_X2APIC_ENABLE
    } else {
        0
    },
);

pub static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

pub static STACK_SIZE: StackSizeRequest =
    StackSizeRequest::new((MemoryInterfaceImpl::PAGE_SIZE * 4) as u64);

pub static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

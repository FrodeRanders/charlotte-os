use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::ptr::NonNull;

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

use crate::memory::allocators::memory::PageSize;

const LIMINE_COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];
const LIMINE_TSC_FREQUENCY_REQUEST_ID: [u64; 2] = [0x10f2ee1d87d195e4, 0xf747a2b78f6ddb31];

#[repr(C, align(8))]
pub struct LimineRequest {
    id: [u64; 4],
    revision: u64,
    response: UnsafeCell<*const c_void>,
}

impl LimineRequest {
    const fn new(id: [u64; 2]) -> Self {
        Self {
            id: [LIMINE_COMMON_MAGIC[0], LIMINE_COMMON_MAGIC[1], id[0], id[1]],
            revision: 0,
            response: UnsafeCell::new(core::ptr::null()),
        }
    }

    pub unsafe fn get_response<T>(&self) -> Option<NonNull<T>> {
        let ptr = unsafe { self.response.get().read_volatile() };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { NonNull::new_unchecked(ptr as *mut T) })
        }
    }
}

#[repr(C)]
pub struct TscFrequencyResponse {
    pub revision:  u64,
    pub frequency: u64,
}

unsafe impl Sync for LimineRequest {}

pub static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

pub static BASE_REVISION: BaseRevision = BaseRevision::new();

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
    StackSizeRequest::new(PageSize::Standard.num_bytes() as u64 * 4);

#[unsafe(no_mangle)]
#[used]
pub static TSC_FREQUENCY_REQUEST: LimineRequest =
    LimineRequest::new(LIMINE_TSC_FREQUENCY_REQUEST_ID);

pub static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

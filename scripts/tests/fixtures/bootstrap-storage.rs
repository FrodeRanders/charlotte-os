//! Host synchronization adapters; production phase, calibration and parser code.
#![allow(dead_code)]
extern crate alloc;
extern crate self as spin;
pub struct Once<T>(std::sync::OnceLock<T>);
impl<T> Once<T> {
    pub const fn new() -> Self { Self(std::sync::OnceLock::new()) }
    pub fn get(&self) -> Option<&T> { self.0.get() }
    pub fn call_once(&self, f: impl FnOnce() -> T) -> &T { self.0.get_or_init(f) }
}
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    // Serial host controls use a nonallocating borrow adapter. Darwin's std
    // mutex lazily allocates OS lock backing; the kernel spin mutex does not.
    pub struct Mutex<T>(core::cell::RefCell<T>);
    impl<T> Mutex<T> {
        pub fn new(value:T)->Self { Self(core::cell::RefCell::new(value)) }
        pub fn lock(&self)->core::cell::RefMut<'_,T> { self.0.borrow_mut() }
    }
}}}}
mod memory { pub mod physical {
    pub trait PhysicalAddress { unsafe fn into_hhdm_ptr<T>(&self)->*const T; }
}}
mod environment { pub mod acpi {
    pub const MAX_ACPI_TABLE_LEN:u32=65536;
    pub enum AcpiTableType { IORT }
    #[repr(C,packed)]
    pub struct SdtHeader { signature:[u8;4],pub length:u32,rest:[u8;28] }
    impl SdtHeader {
        pub fn validate(&self)->bool {
            let length=self.length;
            if !(36..=MAX_ACPI_TABLE_LEN).contains(&length) {return false;}
            let bytes=unsafe {core::slice::from_raw_parts(core::ptr::from_ref(self).cast::<u8>(),length as usize)};
            bytes.iter().fold(0u8,|sum,byte|sum.wrapping_add(*byte))==0
        }
    }
    #[derive(Clone, Copy)] pub struct TableAddress;
    static ADDRESSES:[TableAddress;1]=[TableAddress];
    pub static TABLE:core::sync::atomic::AtomicPtr<u8>=core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());
    impl crate::memory::physical::PhysicalAddress for TableAddress {
        unsafe fn into_hhdm_ptr<T>(&self)->*const T {TABLE.load(core::sync::atomic::Ordering::Acquire).cast()}
    }
    pub fn find_table_type(_:AcpiTableType)->Result<&'static[TableAddress],()> {Ok(&ADDRESSES)}
}}
#[path="@BOOT_TIMER@"] mod boot_timer;
#[path="@ROOT@/crates/catten/src/cpu/isa/x86_64/timers/apic_timer/calibration.rs"] mod calibration;
#[path="@ROOT@/crates/catten/src/environment/acpi/sdt/iort.rs"] mod iort;

use core::{cell::Cell,sync::atomic::{AtomicUsize,Ordering}};
use std::alloc::{GlobalAlloc,Layout,System};
thread_local! { static TRACK:Cell<bool>=const{Cell::new(false)}; }
thread_local! { static FAIL_NEXT:Cell<bool>=const{Cell::new(false)}; }
static ALLOCATIONS:AtomicUsize=AtomicUsize::new(0);
struct Tracer;
#[global_allocator] static ALLOCATOR:Tracer=Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self,layout:Layout)->*mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {ALLOCATIONS.fetch_add(1,Ordering::Relaxed);}
        if FAIL_NEXT.try_with(|fail|fail.replace(false)).unwrap_or(false) {return core::ptr::null_mut();}
        unsafe {System.alloc(layout)}
    }
    unsafe fn dealloc(&self,pointer:*mut u8,layout:Layout) {unsafe {System.dealloc(pointer,layout)}}
}
fn measure(f:impl FnOnce())->usize {
    ALLOCATIONS.store(0,Ordering::Relaxed);TRACK.with(|flag|flag.set(true));f();
    TRACK.with(|flag|flag.set(false));ALLOCATIONS.load(Ordering::Relaxed)
}

#[test] fn local_initialization_and_runtime_access_allocate_no_backing() {
    let timers:Vec<_>=(0..256).map(|_|boot_timer::BootTimer::prepare(0usize)).collect();
    assert!(timers.iter().all(|timer|!timer.ready()));
    let allocations=measure(|| {
        for (lp,timer) in timers.iter().enumerate() {
            timer.initialize(|state|*state=lp+1);
            let first=timer.get();let second=timer.get();
            assert!(std::sync::Arc::ptr_eq(&first,&second));
            assert_eq!(*first.lock(),lp+1);
        }
    });
    assert_eq!(allocations,0,"runtime timer backing allocation");
    assert!(timers.iter().all(boot_timer::BootTimer::ready));
}

#[test] fn missing_repeated_and_uncertain_setup_cannot_be_used_or_replayed() {
    use std::panic::{catch_unwind,AssertUnwindSafe};
    let timer=boot_timer::BootTimer::prepare(0usize);
    assert!(catch_unwind(AssertUnwindSafe(||timer.get())).is_err());
    timer.initialize(|state|*state=1);
    assert!(catch_unwind(AssertUnwindSafe(||timer.initialize(|state|*state=2))).is_err());
    assert_eq!(*timer.get().lock(),1);
    let failed=boot_timer::BootTimer::prepare(0usize);
    assert!(catch_unwind(AssertUnwindSafe(||failed.initialize(|_|panic!("withheld hardware setup")))).is_err());
    assert!(!failed.ready());
    assert!(catch_unwind(AssertUnwindSafe(||failed.get())).is_err());
    assert!(catch_unwind(AssertUnwindSafe(||failed.initialize(|_|panic!("must not run")))).is_err());
}

#[test] fn calibration_requires_ten_complete_nonzero_samples_without_heap() {
    let allocations=measure(|| {
        let mut count=0;
        assert_eq!(calibration::resolution(||{count+=1;Some(count*10)}),Ok(55));
        assert_eq!(count,calibration::NUM_SAMPLES as u128);
        for failure in 0..calibration::NUM_SAMPLES {
            let mut sample=0;
            assert_eq!(calibration::resolution(||{let index=sample;sample+=1;(index!=failure).then_some(100)}),Err(calibration::Error::Incomplete));
            assert_eq!(sample,failure+1);
        }
        assert_eq!(calibration::resolution(||Some(0)),Err(calibration::Error::Zero));
        assert_eq!(calibration::resolution(||Some(u128::MAX)),Err(calibration::Error::Overflow));
    });
    assert_eq!(allocations,0);
}

#[test] fn cached_iort_translation_never_reparses_or_allocates_at_runtime() {
    let mut bytes=vec![0u8;152];
    bytes[..4].copy_from_slice(b"IORT");bytes[4..8].copy_from_slice(&152u32.to_le_bytes());
    bytes[36..40].copy_from_slice(&2u32.to_le_bytes());bytes[40..44].copy_from_slice(&48u32.to_le_bytes());
    bytes[48]=2;bytes[49..51].copy_from_slice(&36u16.to_le_bytes());
    for (offset,value) in [(56,1u32),(60,16),(64,0x100),(68,16),(72,0x200),(76,84)] {bytes[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
    bytes[84]=4;bytes[85..87].copy_from_slice(&68u16.to_le_bytes());
    bytes[100..108].copy_from_slice(&0x900_0000u64.to_le_bytes());
    bytes[9]=0u8.wrapping_sub(bytes.iter().fold(0u8,|sum,byte|sum.wrapping_add(*byte)));
    environment::acpi::TABLE.store(bytes.as_mut_ptr(),Ordering::Release);
    iort::prepare_boot_discovery();
    // Invalidate the fixture's firmware pointer after copying immutable metadata.
    environment::acpi::TABLE.store(core::ptr::null_mut(),Ordering::Release);drop(bytes);
    assert_eq!(measure(||for _ in 0..1000 {assert_eq!(iort::discover_smmuv3().unwrap().stream_id(0x107),Some(0x207));}),0);
}


type HwAsid = u16;
#[path="@HW_ASID@"] mod hw_asid;

#[test] fn hardware_asid_allocation_failure_precedes_any_issued_tag() {
    FAIL_NEXT.with(|fail|fail.set(true));
    let rejected=hw_asid::HwAsidAllocator::try_new(256,0);
    assert!(rejected.is_err());
    assert!(!FAIL_NEXT.with(Cell::get));
    let mut prepared=hw_asid::HwAsidAllocator::try_new(256,0).unwrap();
    assert_eq!(prepared.allocate(),Some(1));
}

#[test] fn hardware_asid_return_and_exhaustion_allocate_no_backing() {
    for (limit,reserved) in [(4u32,2u16),(256,0),(256,128),(65536,0),(65536,65535)] {
        let mut allocator=hw_asid::HwAsidAllocator::try_new(limit,reserved).unwrap();
        let capacity=allocator.capacity();
        let mut issued=Vec::with_capacity(limit as usize);
        let mut seen=vec![false;limit as usize];
        assert_eq!(measure(|| {
            while let Some(tag)=allocator.allocate() {
                assert!(tag!=0 && tag!=reserved && !seen[tag as usize]);
                seen[tag as usize]=true;issued.push(tag);
            }
            assert_eq!(issued.len(),limit as usize-1-usize::from(reserved!=0));
            assert_eq!(allocator.allocate(),None);
            for &tag in &issued {allocator.release(tag);}
            for &tag in issued.iter().rev() {assert_eq!(allocator.allocate(),Some(tag));}
            assert_eq!(allocator.allocate(),None);
        }),0,"hardware tag retirement allocated backing");
        assert_eq!(allocator.capacity(),capacity);
    }
}

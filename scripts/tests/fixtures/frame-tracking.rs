#![feature(allocator_api)]
#![allow(dead_code, unused_imports, unused_attributes)]
extern crate alloc;
extern crate self as catten_syscall;
extern crate self as charlotte_lifecycle;
#[path = SDK]
mod syscall_format;
pub use syscall_format::*;
#[path = POLICY]
mod resource_policy;
#[path = RESOURCES]
pub mod resources;
mod cpu {
    pub mod multiprocessor {
        pub mod spin {
            pub mod mutex {
                pub struct Mutex<T>(std::sync::Mutex<T>);
                impl<T> Mutex<T> {
                    pub const fn new(value: T) -> Self {
                        Self(std::sync::Mutex::new(value))
                    }

                    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
                        self.0.lock().unwrap()
                    }

                    pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> {
                        self.0.try_lock().ok()
                    }
                }
            }
        }
    }
}
mod klib {
    #[path = ADMISSION]
    pub mod admission;
}
mod memory {
    pub type PAddr = usize; // Identity only: host controls perform no physical release.
    pub mod translation {
        pub struct Account;
        impl Account {
            pub fn is_platform(&self) -> bool {
                false
            }

            pub fn principal(&self) -> Option<u64> {
                None
            }
        }
    }
    pub mod kernel_buffers {
        use core::{
            alloc::AllocError,
            sync::atomic::{
                AtomicU64,
                Ordering,
            },
        };

        use crate::klib::admission::ByteSponsor;
        static USED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
        static LIMIT: [AtomicU64; 5] = [const { AtomicU64::new(u64::MAX) }; 5];
        #[derive(Clone, Copy, Debug)]
        pub struct Sponsor(Option<u64>);
        impl Sponsor {
            pub fn unidentified() -> Self {
                Self(None)
            }

            pub fn verified(principal: u64) -> Self {
                assert!((1..5).contains(&principal));
                Self(Some(principal))
            }

            pub fn principal(self) -> Option<u64> {
                self.0
            }

            pub fn from_account(account: &super::translation::Account) -> Self {
                Self(account.principal())
            }
        }
        impl ByteSponsor for Sponsor {
            fn reserve(&self, bytes: u64) -> Result<(), AllocError> {
                if let Some(id) = self.0 {
                    USED[id as usize]
                        .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                            used.checked_add(bytes)
                                .filter(|next| *next <= LIMIT[id as usize].load(Ordering::Acquire))
                        })
                        .map_err(|_| AllocError)?;
                }
                Ok(())
            }

            fn release(&self, bytes: u64) {
                if let Some(id) = self.0 {
                    assert!(USED[id as usize].fetch_sub(bytes, Ordering::AcqRel) >= bytes);
                }
            }
        }
        pub fn test_used(id: u64) -> u64 {
            USED[id as usize].load(Ordering::Acquire)
        }
        pub fn test_limit(id: u64, bytes: u64) {
            assert!(test_used(id) <= bytes);
            LIMIT[id as usize].store(bytes, Ordering::Release);
        }
        pub fn total_used() -> u64 {
            USED.iter().map(|value| value.load(Ordering::Acquire)).sum()
        }
    }
    #[path = TRACKER]
    pub mod frame_tracking;
}

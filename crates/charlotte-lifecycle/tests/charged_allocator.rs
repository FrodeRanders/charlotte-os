#![feature(allocator_api)]

//! Execute the kernel's actual allocator owner without booting an OS. The
//! generic adapter has no kernel dependencies; kernel fixtures test sponsorship.
extern crate alloc;
#[path = "../../catten/src/klib/charged_allocator.rs"]
mod charged_allocator;

#![feature(allocator_ext)]

//! Execute the kernel's actual allocator owner without booting an OS. The
//! generic adapter has no kernel dependencies; kernel fixtures test sponsorship.
extern crate alloc;
#[expect(dead_code, reason = "standalone owner tests do not exercise kernel charge borrowers")]
#[path = "../../catten/src/klib/charged_allocator.rs"]
mod charged_allocator;

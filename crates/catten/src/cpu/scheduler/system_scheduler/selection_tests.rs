//! Serialized boot probes of the production, allocation-free load scan.

use super::*;
use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    {
        // Allocation or destruction by selection would deadlock against this
        // actual heap hold. Arrays and the result contain only scalar samples.
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(LoadExtremes::sample([]), None);
        assert_eq!(
            LoadExtremes::sample([(17, 9)]),
            Some(LoadExtremes {
                least: (17, 9),
                most: (17, 9)
            })
        );
        for loads in [
            [(7, 5), (2, 0), (255, 5), (1, 0)],
            [(1, 0), (255, 5), (2, 0), (7, 5)],
            [(255, 5), (7, 5), (1, 0), (2, 0)],
        ] {
            assert_eq!(
                LoadExtremes::sample(loads),
                Some(LoadExtremes {
                    least: (1, 0),
                    most: (255, 5)
                })
            );
        }
        assert_eq!(
            LoadExtremes::sample([(8, usize::MAX), (3, usize::MAX), (5, usize::MAX)]),
            Some(LoadExtremes {
                least: (3, usize::MAX),
                most: (8, usize::MAX)
            })
        );
        // Exercise the actual summary iterator as well. Do not overwrite live
        // LP summaries to manufacture an imbalance or claim migration evidence.
        let scheduler = SYSTEM_SCHEDULER.read();
        let extremes = scheduler.load_extremes();
        assert_eq!(extremes.is_some(), !scheduler.lp_schedulers.is_empty());
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
}

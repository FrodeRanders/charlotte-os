use super::*;

fn raw(value: usize) -> VAddr {
    unsafe { VAddr::from_raw_unchecked(value) }
}

#[test]
fn every_software_region_has_exact_boundaries_in_all_layouts() {
    for bits in [39, 48, 57] {
        for arm in [false, true] {
            let map = LinearAddressMap::for_width(bits, arm);
            for region in [
                &map.null_page,
                &map.application,
                &map.direct_mapping,
                &map.kernel_stack_arena,
                &map.kernel_mmio,
                &map.kernel_allocator_arena,
                &map.kernel_image,
            ] {
                let base: usize = region.base.into();
                assert!(region.contains(raw(base)));
                let last = base.checked_add(region.length - 1).unwrap();
                assert!(region.contains(raw(last)));
                if let Some(end) = last.checked_add(1) {
                    assert!(!region.contains(raw(end)));
                    let (_, actual): (VAddr, VAddr) = (*region).into();
                    assert_eq!(usize::from(actual), end);
                }
                if base > 0 {
                    assert!(!region.contains(raw(base - 1)));
                }
            }
            assert!(map.application.contains(raw(0x10000)));
            let (_, end): (VAddr, VAddr) = map.application.into();
            assert_eq!(usize::from(end), 1usize << (bits - 1));
            assert_eq!(map.region_type(raw(usize::MAX)), RegionType::KernelImage);
        }
    }
}

#[test]
fn active_arena_bases_and_raw_arm_hhdm_are_preserved() {
    for arm in [false, true] {
        let map = LinearAddressMap::for_width(48, arm);
        assert_eq!(usize::from(map.kernel_stack_arena.base), 0xffff_8100_0000_0000);
        assert_eq!(usize::from(map.kernel_mmio.base), 0xffff_8200_0000_0000);
        assert_eq!(usize::from(map.kernel_allocator_arena.base), 0xffff_8400_0000_0000);
        let hhdm = if arm {
            0xffff_0000_0000_0000
        } else {
            0xffff_8000_0000_0000
        };
        assert_eq!(usize::from(map.direct_mapping.base), hhdm);
        assert_eq!(map.region_type(raw(hhdm + 0x1000)), RegionType::DirectMapping);
    }
}

#[test]
fn invalid_tiles_and_unrepresentable_end_reject_instead_of_wrapping() {
    let mut map = LinearAddressMap::for_width(48, false);
    map.kernel_mmio.length += 4096;
    assert!(std::panic::catch_unwind(|| map.validate(1u128 << 47, 0xffff_8000_0000_0000)).is_err());
    let image = LinearAddressMap::for_width(48, false).kernel_image;
    assert!(image.contains(raw(usize::MAX)));
    assert!(
        std::panic::catch_unwind(|| {
            let _: (VAddr, VAddr) = image.into();
        })
        .is_err()
    );
    assert!(std::panic::catch_unwind(|| LinearAddressMap::for_width(52, false)).is_err());
}

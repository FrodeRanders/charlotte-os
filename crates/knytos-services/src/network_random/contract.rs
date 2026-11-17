// Public Python hashlib/hmac vectors, shared by host tests and the signed
// opt-in EL0 security probe. Never seed the production network this way.
pub(super) fn seeded() -> super::NetworkRandom {
    super::NetworkRandom::initialize(|bytes| {
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = index as u8;
        }
        Ok::<_, ()>(())
    })
    .unwrap()
}

pub(crate) fn check() {
    let mut random = seeded();
    assert_eq!(random.interface_seed(), u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]));
    for expected in [64443, 60648, 61719, 64261, 51295, 53828, 61106, 49164] {
        assert_eq!(random.ephemeral_port(), expected);
    }
}

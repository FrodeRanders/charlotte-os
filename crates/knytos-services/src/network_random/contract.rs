// Public Python hashlib/hmac vectors, shared by host tests and the signed
// opt-in EL0 security probe. Never seed the production network this way.
// For each counter c, independently generate the expected port with:
// digest = hmac.digest(bytes(range(8, 40)),
//     b"Knytos TCP/IP ephemeral port v1" + c.to_bytes(8, "little"), "sha256")
// port = 49152 + (int.from_bytes(digest[:2], "little") & 0x3fff)
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
    for expected in [65131, 61996, 59304, 60673, 64507, 52136, 54698, 60056] {
        assert_eq!(random.ephemeral_port(), expected);
    }
}

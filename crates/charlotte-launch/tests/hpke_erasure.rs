//! Public fixture bytes exercise the exact X25519 owners used by HPKE.

use core::{
    mem::MaybeUninit,
    ptr,
};

use x25519_dalek::{
    PublicKey,
    StaticSecret,
};

fn assert_drop_erases(value: impl AsRef<[u8]>) {
    let mut storage = MaybeUninit::new(value);
    let owner = storage.as_mut_ptr();
    // Keep storage allocated after destroying its one initialized owner.
    // Locate the initialized 32-byte field with AsRef, then end that borrow.
    // Post-Drop reads derive from the storage pointer, avoiding reuse of a
    // pointer from the shared borrow. No padding, freed memory or second owner.
    unsafe {
        let bytes = (*owner).as_ref();
        assert_eq!(bytes.len(), 32);
        assert!(bytes.iter().any(|byte| *byte != 0));
        let offset = bytes.as_ptr().addr().checked_sub(owner.addr()).unwrap();
        assert!(offset + 32 <= core::mem::size_of_val(&*owner));
        ptr::drop_in_place(owner);
        let data = owner.cast::<u8>().add(offset);
        for index in 0..32 {
            assert_eq!(ptr::read_volatile(data.add(index)), 0, "retained byte {index}");
        }
    }
}

#[test]
fn x25519_private_owner_erases_on_drop() {
    assert_drop_erases(StaticSecret::from([0x5a; 32]));
}

#[test]
fn x25519_shared_owner_erases_on_drop() {
    let private = StaticSecret::from([0x5a; 32]);
    let peer = PublicKey::from(&StaticSecret::from([0x6b; 32]));
    assert_drop_erases(private.diffie_hellman(&peer));
}

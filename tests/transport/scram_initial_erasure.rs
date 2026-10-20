//! Initial SCRAM allocations use only public credential fixtures.
use core::hint::black_box;

use charlotte_kafka::scram::{
    Client,
    Error,
};

mod support;
use support::{
    observe,
    observe_nth,
};

const USER: &str = "ZZZZZZZZ,=user";
const PASSWORD: &[u8] = b"ZZZZZZZZ-password";
const NONCE: &str = "ZZZZZZZZ-nonce";
const CHALLENGE: &[u8] = b"r=ZZZZZZZZ-nonce-server,s=c2FsdA==,i=4096";

#[test]
fn transcript_is_built_without_username_scratch() {
    let observed = observe(0, 0, 2, || {
        let client = Client::new(USER, PASSWORD, NONCE).unwrap();
        drop(black_box(client));
    });
    assert!(observed.copy_erased, "transcript retained: {observed:?}");
    assert_eq!(observed.allocations, 3, "unexpected username copy: {observed:?}");
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn nonce_erases_on_initial_drop() {
    // Transcript, password, nonce: all three are live at constructor return.
    let observed = observe_nth(3, 0, 0, 0, || {
        let client = Client::new(USER, PASSWORD, NONCE).unwrap();
        drop(black_box(client));
    });
    assert!(observed.copy_erased, "nonce retained: {observed:?}");
    assert_eq!(observed.allocations, 3);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn nonce_erases_on_challenge_rejection() {
    let observed = observe_nth(3, 0, 0, 0, || {
        let client = Client::new(USER, PASSWORD, NONCE).unwrap();
        assert!(matches!(
            client.receive_server_first(b"malformed"),
            Err(Error::InvalidServerMessage)
        ));
    });
    assert!(observed.copy_erased, "rejected nonce retained: {observed:?}");
    assert_eq!(observed.allocations, 3);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn nonce_erases_before_final_phase_use() {
    let mut final_phase = None;
    let observed = observe_nth(3, 0, 0, 0, || {
        let client = Client::new(USER, PASSWORD, NONCE).unwrap();
        final_phase = Some(client.receive_server_first(CHALLENGE).unwrap());
    });
    assert!(observed.copy_erased, "nonce survived transition: {observed:?}");
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
    let final_phase = final_phase.unwrap();
    assert!(final_phase.message().starts_with(b"c=biws,r=ZZZZZZZZ-nonce-server,p="));
    drop(black_box(final_phase));
}

#[test]
fn password_keeps_existing_erasure_policy() {
    let observed = observe_nth(2, 0, 0, 0, || {
        let client = Client::new(USER, PASSWORD, NONCE).unwrap();
        drop(black_box(client));
    });
    assert!(observed.copy_erased, "password retained: {observed:?}");
    assert_eq!(observed.allocations, 3);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn escaping_preserves_utf8_and_adjacent_delimiters() {
    for (username, expected) in [
        ("user", "n,,n=user,r=ZZZZZZZZ-nonce"),
        (",=", "n,,n==2C=3D,r=ZZZZZZZZ-nonce"),
        ("雪,=user", "n,,n=雪=2C=3Duser,r=ZZZZZZZZ-nonce"),
        ("a,,==z", "n,,n=a=2C=2C=3D=3Dz,r=ZZZZZZZZ-nonce"),
    ] {
        let client = Client::new(username, PASSWORD, NONCE).unwrap();
        assert_eq!(client.client_first().as_slice(), expected.as_bytes());
    }
    let username = ",=".repeat(1024);
    let observed = observe(0, 0, 2, || {
        let client = Client::new(&username, PASSWORD, NONCE).unwrap();
        drop(black_box(client));
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 3);
    assert_eq!(observed.growths, 0);
}

#[test]
fn invalid_initial_inputs_reject_before_allocation() {
    for (username, password, nonce) in [
        ("", PASSWORD, NONCE),
        (USER, b"".as_slice(), NONCE),
        (USER, PASSWORD, ""),
        (USER, PASSWORD, "bad,nonce"),
        (USER, PASSWORD, "bad nonce"),
        (USER, PASSWORD, "bad\nnonce"),
        (USER, PASSWORD, "雪"),
    ] {
        let observed = observe(0, 0, 0, || {
            assert!(matches!(
                Client::new(username, password, nonce),
                Err(Error::InvalidCredential)
            ));
        });
        assert_eq!(observed.allocations, 0);
    }
}

#[test]
fn escaped_username_exchange_matches_python_reference() {
    let mut expected = include_str!("fixtures/scram-initial-reference.txt").lines();
    let client = Client::new(USER, PASSWORD, NONCE).unwrap();
    assert_eq!(client.client_first().as_slice(), expected.next().unwrap().as_bytes());
    let final_phase = client.receive_server_first(CHALLENGE).unwrap();
    assert_eq!(final_phase.message(), expected.next().unwrap().as_bytes());
    final_phase.receive_server_final(expected.next().unwrap().as_bytes()).unwrap();
    assert!(expected.next().is_none());
}

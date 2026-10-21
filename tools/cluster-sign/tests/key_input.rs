//! Public fixtures check the production private-key text owners.
use std::io::{
    self,
    Cursor,
    Read,
};

use zeroize::Zeroizing;

#[path = "../src/key_input.rs"]
mod key_input;
#[path = "../../../tests/transport/support.rs"]
mod support;
use key_input::{
    normalize_key_text,
    read_text,
    MAX_KEY_FILE_BYTES,
};
use support::observe;

fn erased(observed: support::Observation) {
    assert_eq!(observed.copy_releases, 1, "{observed:?}");
    assert_eq!(observed.growths, 0, "{observed:?}");
    assert!(observed.copy_erased, "public fixture text retained: {observed:?}");
}

#[test]
fn accepted_input_keeps_one_erasing_allocation() {
    for len in [8, 128, MAX_KEY_FILE_BYTES] {
        let input = vec![b'Z'; len];
        let observed = observe(0, 0, 0, || {
            let text: Zeroizing<Vec<u8>> = read_text(Cursor::new(&input)).unwrap();
            assert_eq!(text.as_slice(), input);
            drop(text);
        });
        assert_eq!(observed.allocations, 1);
        erased(observed);
    }
    assert!(read_text(io::empty()).unwrap().is_empty());
}

struct PartialError(bool);
impl Read for PartialError {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Err(io::ErrorKind::Other.into());
        }
        self.0 = true;
        output[..8].fill(b'Z');
        Ok(8)
    }
}

#[test]
fn partial_error_erases_accepted_prefix() {
    erased(observe(0, 0, 0, || {
        assert_eq!(read_text(PartialError(false)).unwrap_err(), "cannot read private-key file");
    }));
}

struct DirtyResult(bool);
impl Read for DirtyResult {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(output.len() >= 16);
        output.fill(b'Z');
        if self.0 {
            Err(io::ErrorKind::Other.into())
        } else {
            Ok(0)
        }
    }
}

#[test]
fn dirty_error_and_eof_erase_unreported_destination() {
    for error in [false, true] {
        erased(observe(0, 0, 8, || {
            let text = read_text(DirtyResult(error));
            if error {
                assert!(text.is_err());
            } else {
                assert!(text.as_ref().unwrap().is_empty());
            }
            drop(text);
        }));
    }
}

struct InterruptedInput(u8);
impl Read for InterruptedInput {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.0 += 1;
        match self.0 {
            1 => {
                output.fill(b'Z');
                Err(io::ErrorKind::Interrupted.into())
            }
            2 => {
                output[..8].fill(b'A');
                Ok(8)
            }
            _ => Ok(0),
        }
    }
}

#[test]
fn interrupted_read_preserves_owner_and_accepted_bytes() {
    erased(observe(0, 0, 0, || {
        let text = read_text(InterruptedInput(0)).unwrap();
        assert_eq!(text.as_slice(), b"AAAAAAAA");
        drop(text);
    }));
}

#[test]
fn sentinel_rejects_growth_without_reallocation_or_excess_reading() {
    for len in [MAX_KEY_FILE_BYTES, MAX_KEY_FILE_BYTES + 1, MAX_KEY_FILE_BYTES * 2] {
        let input = vec![b'Z'; len];
        let mut source = Cursor::new(&input);
        erased(observe(0, 0, 0, || {
            let result = read_text(&mut source);
            if len == MAX_KEY_FILE_BYTES {
                assert_eq!(result.as_ref().unwrap().as_slice(), input);
            } else {
                assert_eq!(result.as_ref().unwrap_err(), &"private-key file exceeds 4096 bytes");
            }
            drop(result);
        }));
        assert_eq!(source.position() as usize, len.min(MAX_KEY_FILE_BYTES + 1));
    }
}

#[test]
fn normalization_returns_owner_without_growth_and_preserves_grammar() {
    for (input, expected) in [
        ("  # public fixture\n\n  ZZZZZZZZ \r\n\t0011Aa\n", "ZZZZZZZZ0011Aa"),
        ("\u{2003}ZZZZZZZZ\u{2003}\n# ignored", "ZZZZZZZZ"),
        ("ZZZZZZZZ#inline-comment", "ZZZZZZZZ#inline-comment"),
    ] {
        let observed = observe(0, 0, 0, || {
            let encoded: Zeroizing<String> = normalize_key_text(input);
            assert_eq!(encoded.as_str(), expected);
            drop(encoded);
        });
        assert_eq!(observed.allocations, 1);
        erased(observed);
    }
    assert!(normalize_key_text("\n # comment\n\t").is_empty());
}

#[test]
fn consuming_normalized_text_on_rejection_erases_same_allocation() {
    fn reject(text: Zeroizing<String>, address: usize) -> Result<(), ()> {
        assert_eq!(text.as_ptr().addr(), address);
        Err(())
    }
    erased(observe(0, 0, 0, || {
        let text = normalize_key_text("ZZZZZZZZ");
        let address = text.as_ptr().addr();
        assert_eq!(reject(text, address), Err(()));
    }));
}

#[test]
fn pinned_zeroize_erases_initialized_bytes_beyond_vector_length() {
    // The fixture initializes the complete allocation before truncating. This
    // probe never reads uninitialized capacity or storage after deallocation.
    erased(observe(0, 0, 128, || {
        let mut bytes = Zeroizing::new(vec![b'Z'; MAX_KEY_FILE_BYTES + 1]);
        bytes.truncate(8);
        drop(bytes);
    }));
}

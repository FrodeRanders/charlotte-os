//! Exact deferred TCP-send frame and once-only enqueue receipt.
pub const FRAME_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 16;
pub const MAX_PAYLOAD: usize = FRAME_BYTES - HEADER_BYTES;
const MAGIC: &[u8; 8] = b"CSOCKTX1";

pub fn encode(payload: &[u8], frame: &mut [u8]) -> bool {
    if frame.len() != FRAME_BYTES || payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return false;
    }
    frame.fill(0);
    frame[..8].copy_from_slice(MAGIC);
    frame[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    frame[HEADER_BYTES..HEADER_BYTES + payload.len()].copy_from_slice(payload);
    true
}

pub fn decode(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() != FRAME_BYTES || &frame[..8] != MAGIC || frame[12..16] != [0; 4] {
        return None;
    }
    let length = u32::from_le_bytes(frame[8..12].try_into().ok()?) as usize;
    if !(1..=MAX_PAYLOAD).contains(&length)
        || frame[HEADER_BYTES + length..].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    Some(&frame[HEADER_BYTES..HEADER_BYTES + length])
}

/// Zero confirms that no bytes entered the TCP buffer. Any positive receipt or
/// error fences another enqueue attempt with this accepted request owner.
#[derive(Default)]
pub struct Enqueue {
    terminal: Option<Result<usize, i64>>,
}
impl Enqueue {
    pub fn attempt(&mut self, write: impl FnOnce() -> Result<usize, i64>) -> Result<usize, i64> {
        if let Some(result) = self.terminal {
            return result;
        }
        let result = write();
        if result != Ok(0) {
            self.terminal = Some(result);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_frame_roundtrips_empty_and_oversized_payloads_reject() {
        let mut frame = [0xff; FRAME_BYTES];
        assert!(encode(b"hello", &mut frame));
        assert_eq!(decode(&frame), Some(&b"hello"[..]));
        assert!(!encode(&[], &mut frame));
        assert!(!encode(&[0; FRAME_BYTES], &mut frame));
        assert!(encode(&[7; MAX_PAYLOAD], &mut frame));
        assert_eq!(decode(&frame).unwrap().len(), MAX_PAYLOAD);
    }
    #[test]
    fn retired_truncated_extended_reserved_and_dirty_tail_frames_reject() {
        let mut frame = [0; FRAME_BYTES];
        encode(b"x", &mut frame);
        assert!(decode(&frame[..FRAME_BYTES - 1]).is_none());
        let mut extended = [0; FRAME_BYTES + 1];
        extended[..FRAME_BYTES].copy_from_slice(&frame);
        assert!(decode(&extended).is_none());
        for index in [0, 12, HEADER_BYTES + 1, FRAME_BYTES - 1] {
            let mut bad = frame;
            bad[index] ^= 0x80;
            assert!(decode(&bad).is_none());
        }
        for length in [0u32, MAX_PAYLOAD as u32 + 1, u32::MAX] {
            let mut bad = frame;
            bad[8..12].copy_from_slice(&length.to_le_bytes());
            assert!(decode(&bad).is_none());
        }
        assert!(decode(&[0; FRAME_BYTES]).is_none());
    }
    #[test]
    fn enqueue_never_repeats_a_partial_receipt_or_ambiguous_error() {
        for result in [Ok(5), Err(-99)] {
            let mut state = Enqueue::default();
            let mut calls = 0;
            assert_eq!(
                state.attempt(|| {
                    calls += 1;
                    result
                }),
                result
            );
            assert_eq!(
                state.attempt(|| {
                    calls += 1;
                    Ok(12)
                }),
                result
            );
            assert_eq!(calls, 1);
        }
        let mut state = Enqueue::default();
        assert_eq!(state.attempt(|| Ok(0)), Ok(0));
        assert_eq!(state.attempt(|| Ok(7)), Ok(7));
    }
}

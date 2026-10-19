//! Owned assembly of length-prefixed broker replies across stream chunks.
use alloc::vec::Vec;

use zeroize::{
    Zeroize,
    Zeroizing,
};

use crate::{
    Error,
    MAX_FRAME_LEN,
};

#[derive(Debug, Eq, PartialEq)]
pub enum ReceiveError<E> {
    Transport(E),
    Protocol(Error),
    ChunkTooLarge,
    EmptyChunk,
    Poisoned,
}

/// One partial reply and the remaining bytes of its existing transport chunk.
/// A failed read fences this owner until the caller discards the old stream
/// and explicitly resets it. Reset discards bytes; it grants no replay right.
pub struct FrameReceiver {
    header: Zeroizing<[u8; 4]>,
    header_len: usize,
    frame: Option<Zeroizing<Vec<u8>>>,
    frame_len: usize,
    pending: Zeroizing<Vec<u8>>,
    offset: usize,
    failed: bool,
}

impl Default for FrameReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameReceiver {
    pub fn new() -> Self {
        Self {
            header: Zeroizing::new([0; 4]),
            header_len: 0,
            frame: None,
            frame_len: 0,
            pending: Zeroizing::new(Vec::new()),
            offset: 0,
            failed: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The producer supplies an owned chunk using its existing blocking/readiness
    /// contract. Buffered bytes are consumed before requesting another chunk.
    /// Reserve the complete frame only after validating its inline length header.
    pub fn read_frame<E>(
        &mut self,
        mut receive: impl FnMut() -> Result<Zeroizing<Vec<u8>>, E>,
    ) -> Result<Zeroizing<Vec<u8>>, ReceiveError<E>> {
        if self.failed {
            return Err(ReceiveError::Poisoned);
        }
        // An interrupted producer must leave this episode fenced too.
        self.failed = true;
        let result = self.read_inner(&mut receive);
        if result.is_ok() {
            self.failed = false;
        }
        result
    }

    fn read_inner<E>(
        &mut self,
        receive: &mut impl FnMut() -> Result<Zeroizing<Vec<u8>>, E>,
    ) -> Result<Zeroizing<Vec<u8>>, ReceiveError<E>> {
        loop {
            if self.offset == self.pending.len() {
                let pending = receive().map_err(ReceiveError::Transport)?;
                if pending.is_empty() {
                    return Err(ReceiveError::EmptyChunk);
                }
                if pending.len() > MAX_FRAME_LEN + 4 {
                    return Err(ReceiveError::ChunkTooLarge);
                }
                self.pending = pending;
                self.offset = 0;
            }
            if self.header_len < 4 {
                let count = (4 - self.header_len).min(self.pending.len() - self.offset);
                let end = self.offset + count;
                self.header[self.header_len..self.header_len + count]
                    .copy_from_slice(&self.pending[self.offset..end]);
                self.pending[self.offset..end].zeroize();
                self.offset = end;
                self.header_len += count;
                if self.header_len < 4 {
                    continue;
                }
                let payload = i32::from_be_bytes(*self.header);
                if payload < 4 {
                    return Err(ReceiveError::Protocol(Error::Invalid));
                }
                if payload as usize > MAX_FRAME_LEN {
                    return Err(ReceiveError::Protocol(Error::TooLarge));
                }
                self.frame_len = payload as usize + 4;
                let mut frame = Zeroizing::new(Vec::with_capacity(self.frame_len));
                frame.extend_from_slice(&self.header[..]);
                self.frame = Some(frame);
            }
            let frame = self.frame.as_mut().expect("validated header owns its frame");
            let count = (self.frame_len - frame.len()).min(self.pending.len() - self.offset);
            let end = self.offset + count;
            frame.extend_from_slice(&self.pending[self.offset..end]);
            self.pending[self.offset..end].zeroize();
            self.offset = end;
            if frame.len() == self.frame_len {
                self.header.zeroize();
                self.header_len = 0;
                self.frame_len = 0;
                return Ok(self.frame.take().expect("complete frame owner"));
            }
        }
    }
}

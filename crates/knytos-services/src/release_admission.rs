//! Owned destination prepared before release admission or follower relay.
use catten_rt::owned::{
    MappedMemory,
    OwnedMemory,
    ReplyToken,
    Writable,
};
use charlotte_launch::{
    release,
    release_admission::{
        MAX_LEN,
        Receipt,
        decode_result,
    },
    sha256,
};

pub struct PreparedReply {
    token: ReplyToken,
    mapping: MappedMemory<Writable>,
    digest: [u8; 32],
    count: usize,
}

impl PreparedReply {
    pub fn prepare(token: ReplyToken, envelope: &[u8]) -> Result<Self, (ReplyToken, i64)> {
        let Some(release) = release::decode(envelope) else {
            return Err((token, crate::clusterctl::ERR_UNTRUSTED_DESCRIPTOR));
        };
        let memory = match OwnedMemory::allocate(1) {
            Ok(memory) => memory,
            Err(_) => return Err((token, crate::clusterctl::ERR_UPLOAD_FAILED)),
        };
        let mapping = match memory.map_writable() {
            Ok(mapping) => mapping,
            Err((_memory, _error)) => return Err((token, crate::clusterctl::ERR_UPLOAD_FAILED)),
        };
        Ok(Self {
            token,
            mapping,
            digest: sha256::digest(envelope),
            count: release.descriptors().count(),
        })
    }

    pub fn reject(self, code: i64) {
        let Self {
            token,
            mapping,
            ..
        } = self;
        drop(mapping);
        let _ = token.reply(code);
    }

    pub fn complete(mut self, bytes: &[u8]) {
        let receipt = match decode_result(bytes) {
            Some(Ok(receipt)) => receipt,
            Some(Err(code)) => {
                self.reject(code);
                return;
            }
            None => {
                self.reject(crate::dns::ERR_UNCERTAIN);
                return;
            }
        };
        if receipt.digest() != self.digest
            || receipt.components().len() != self.count
            || bytes.len() > MAX_LEN
        {
            self.reject(crate::dns::ERR_UNCERTAIN);
            return;
        }
        self.mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        let Self {
            token,
            mapping,
            ..
        } = self;
        match mapping.unmap() {
            Ok(memory) => {
                let _ = token.reply_move(memory, bytes.len() as i64);
            }
            Err((_mapping, _error)) => {
                let _ = token.reply(crate::dns::ERR_UNCERTAIN);
            }
        }
    }
}

/// Scalar operational admission needs only the release generation, but still
/// validates the complete result before discarding its component metadata.
pub fn generation_result(bytes: &[u8]) -> i64 {
    match decode_result(bytes) {
        Some(Ok(receipt)) => receipt.generation() as i64,
        Some(Err(code)) => code,
        None => crate::dns::ERR_UNCERTAIN,
    }
}

pub fn valid_receipt(bytes: &[u8], envelope: &[u8]) -> Option<Receipt> {
    Receipt::decode(bytes).filter(|receipt| receipt.matches(envelope))
}

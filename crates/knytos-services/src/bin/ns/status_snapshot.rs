//! Own a complete active-publication observation through allocation and reply.

use alloc::vec::Vec;

use catten_services::ns;
use charlotte_protocol_ns::snapshot;

enum StatusBacking {
    Empty,
    Memory(catten_rt::owned::OwnedMemory),
    Mapping(catten_rt::owned::MappedMemory<catten_rt::owned::Writable>),
}

/// One owner for every transient field from preparation through publication.
/// The output is destroyed before unused reply authority on abandonment.
struct StatusReply {
    backing: StatusBacking,
    bytes: Vec<u8>,
    reply: Option<catten_rt::owned::ReplyToken>,
}

impl StatusReply {
    fn reject(mut self, error: i64) {
        drop(core::mem::replace(&mut self.backing, StatusBacking::Empty));
        if let Some(reply) = self.reply.take() {
            let _ = reply.reply(error);
        }
    }

    fn prepare(&mut self) -> Result<(), i64> {
        use catten_rt::owned::OwnedMemory;
        self.backing = StatusBacking::Memory(
            OwnedMemory::allocate(self.bytes.len().div_ceil(4096)).map_err(|_| ns::ERR_INVALID)?,
        );
        let StatusBacking::Memory(memory) =
            core::mem::replace(&mut self.backing, StatusBacking::Empty)
        else {
            unreachable!()
        };
        self.backing = StatusBacking::Mapping(match memory.map_writable() {
            Ok(mapping) => mapping,
            Err((memory, _)) => {
                self.backing = StatusBacking::Memory(memory);
                return Err(ns::ERR_INVALID);
            }
        });
        let StatusBacking::Mapping(mapping) = &mut self.backing else {
            unreachable!()
        };
        mapping.as_mut_slice()[..self.bytes.len()].copy_from_slice(&self.bytes);
        let StatusBacking::Mapping(mapping) =
            core::mem::replace(&mut self.backing, StatusBacking::Empty)
        else {
            unreachable!()
        };
        self.backing = StatusBacking::Memory(match mapping.unmap() {
            Ok(memory) => memory,
            Err((mapping, _)) => {
                self.backing = StatusBacking::Mapping(mapping);
                return Err(ns::ERR_INVALID);
            }
        });
        Ok(())
    }

    fn publish(mut self) {
        let StatusBacking::Memory(memory) =
            core::mem::replace(&mut self.backing, StatusBacking::Empty)
        else {
            unreachable!()
        };
        // Rejected move returns the memory owner; dropping its result releases
        // it through the existing runtime policy exactly once, without replay.
        let _ = self
            .reply
            .take()
            .expect("status reply authority")
            .reply_move(memory, self.bytes.len() as i64);
    }
}

/// Consume the catalog observation's received transaction before any scalar
/// attachment fallback can touch it. No new cleanup/retry policy is added.
pub(super) fn reply_status(
    mut request: catten_rt::owned::IncomingMessage,
    registry: &super::Registry,
    pending: usize,
) {
    let invalid = request.arg0 != 0 || request.memory.is_some() || request.connection.is_some();
    let mut operation = StatusReply {
        backing: StatusBacking::Empty,
        bytes: Vec::new(),
        reply: request.reply.take(),
    };
    drop(request);
    if operation.reply.is_none() {
        return;
    }
    if invalid {
        operation.reject(ns::ERR_INVALID);
        return;
    }
    let Ok(pending) = u32::try_from(pending) else {
        operation.reject(ns::ERR_INVALID);
        return;
    };
    operation.bytes = match snapshot::encode(
        registry.iter().filter(|(_, entry)| entry.publication.is_some()).map(|(name, entry)| {
            snapshot::Record {
                name,
                generation: entry.generation as u64,
            }
        }),
        pending,
    ) {
        Ok(bytes) => bytes,
        Err(_) => {
            operation.reject(ns::ERR_INVALID);
            return;
        }
    };
    if let Err(error) = operation.prepare() {
        operation.reject(error);
        return;
    }
    operation.publish();
}

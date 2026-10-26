//! Caller-funded RNG replies. No reply backing is allocated by the producer.

use catten_rt::owned::{
    IncomingMessage,
    MappedMemory,
    Writable,
};

use crate::entropy;

/// Handle one complete received transaction. The fill callback accesses only
/// the validated requested prefix; the hardware adapter retains its own DMA
/// backing. A true result means that the exact count/buffer was published.
pub fn serve_fill(message: IncomingMessage, fill: impl FnOnce(&mut [u8]) -> bool) -> bool {
    FillReply {
        message,
        mapping: None,
    }
    .serve(fill)
}

// Retain the entire request, reply/loan authority and any transient mapping
// together. Local access must end before cancellation can release a loan.
struct FillReply {
    message: IncomingMessage,
    mapping: Option<MappedMemory<Writable>>,
}

impl FillReply {
    fn serve(mut self, fill: impl FnOnce(&mut [u8]) -> bool) -> bool {
        if self.message.reply.is_none() {
            return false;
        }
        let error = if self.message.opcode != entropy::OP_FILL {
            Some(entropy::ERR_BAD_OPCODE)
        } else if self.message.connection.is_some()
            || self.message.arg0 == 0
            || self.message.arg0 > entropy::MAX_REQUEST as u64
            || !self.message.memory.as_ref().is_some_and(|m| m.len() == entropy::BUFFER_BYTES)
        {
            Some(entropy::ERR_INVALID)
        } else {
            None
        };
        if let Some(error) = error {
            return self.reject(error);
        }
        let memory = self.message.memory.take().expect("validated caller buffer");
        self.mapping = Some(match memory.map_writable() {
            Ok(mapping) => mapping,
            Err((memory, _)) => {
                self.message.memory = Some(memory);
                return self.reject(entropy::ERR_MEMORY);
            }
        });
        let length = self.message.arg0 as usize;
        if !fill(&mut self.mapping.as_mut().expect("mapped caller buffer").as_mut_slice()[..length])
        {
            return self.reject(entropy::ERR_DEVICE);
        }
        self.message.memory =
            Some(match self.mapping.take().expect("mapped caller buffer").unmap() {
                Ok(memory) => memory,
                Err((mapping, _)) => {
                    self.mapping = Some(mapping);
                    return self.reject(entropy::ERR_MEMORY);
                }
            });
        let memory = self.message.memory.take().expect("unmapped caller buffer");
        self.message
            .reply
            .take()
            .expect("received reply authority")
            .reply_move(memory, length as i64)
            .is_ok()
    }

    fn reject(mut self, error: i64) -> bool {
        // Mapping and received memory precede reply/loan cancellation. Failed
        // unmap retains its typed owner until this same disposal boundary.
        drop(self.mapping.take());
        drop(self.message.memory.take());
        if let Some(reply) = self.message.reply.take() {
            let _ = reply.reply(error);
        }
        false
    }
}

impl Drop for FillReply {
    fn drop(&mut self) {
        drop(self.mapping.take());
        drop(self.message.memory.take());
    }
}

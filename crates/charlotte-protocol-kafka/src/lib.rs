//! Wire protocol for the CharlotteOS Kafka data-plane service.
//!
//! One service profile grants authority to one broker endpoint, topic,
//! partition, consumer group, and transactional identity. Broker credentials
//! and Kafka producer epochs never cross this application-facing boundary.
#![no_std]

pub const INTERFACE: u64 = u64::from_le_bytes(*b"KAFKA\0\0\0");
pub const VERSION: u32 = 1;
pub const NAME: u64 = u64::from_le_bytes(*b"kafka\0\0\0");

pub mod manifest {
    pub const IP: u64 = u64::from_le_bytes(*b"kfk_ip\0\0");
    pub const HOST: u64 = u64::from_le_bytes(*b"kfk_host");
    pub const PORT: u64 = u64::from_le_bytes(*b"kfk_port");
    pub const TLS: u64 = u64::from_le_bytes(*b"kfk_tls\0");
    pub const CA_DER: u64 = u64::from_le_bytes(*b"kfk_ca\0\0");
    pub const TOPIC: u64 = u64::from_le_bytes(*b"kfktopic");
    pub const PARTITION: u64 = u64::from_le_bytes(*b"kfkpart\0");
    pub const GROUP: u64 = u64::from_le_bytes(*b"kfkgroup");
    pub const TRANSACTIONAL_ID: u64 = u64::from_le_bytes(*b"kfktxn\0\0");
    pub const RIGHTS: u64 = u64::from_le_bytes(*b"kfkright");
    pub const TRANSACTION_TIMEOUT_MS: u64 = u64::from_le_bytes(*b"kfktout\0");
}

/// Produce one record outside an application transaction. The service still
/// uses Kafka's idempotent producer sequence. `arg0` is the request length.
pub const OP_PRODUCE: u32 = 1;
/// Open the profile's fixed topic/partition consumer at its committed group
/// offset (or the earliest retained offset). Returns a positive consumer ID.
pub const OP_CONSUMER_OPEN: u32 = 2;
/// Fetch one record. `arg0` is the consumer ID. A positive result is a
/// delivery ID and the reply moves an encoded [`DeliveredRecord`].
pub const OP_CONSUMER_POLL: u32 = 3;
/// Commit the delivery's next offset outside a transaction. Consumes the
/// delivery resource.
pub const OP_DELIVERY_COMMIT: u32 = 4;
/// Release a delivery without advancing the committed offset. The next poll
/// may redeliver it. Used by the application owner's `Drop` fallback.
pub const OP_DELIVERY_RELEASE: u32 = 5;
/// Close a consumer and release any outstanding delivery.
pub const OP_CONSUMER_CLOSE: u32 = 6;
/// Begin a Kafka transaction. Returns a positive transaction ID.
pub const OP_TX_BEGIN: u32 = 7;
/// Produce within a transaction. `arg0` packs the transaction ID in the low
/// 32 bits and request length in the high 32 bits.
pub const OP_TX_PRODUCE: u32 = 8;
/// Move a delivery's group offset into a transaction. `arg0` packs the
/// transaction ID low and delivery ID high. Consumes the delivery resource.
pub const OP_TX_INCLUDE_DELIVERY: u32 = 9;
/// Commit and consume a transaction resource.
pub const OP_TX_COMMIT: u32 = 10;
/// Abort and consume a transaction resource.
pub const OP_TX_ABORT: u32 = 11;
pub const OP_STATUS: u32 = 12;

pub const RIGHT_PRODUCE: u64 = 1 << 0;
pub const RIGHT_CONSUME: u64 = 1 << 1;
pub const RIGHT_TRANSACTION: u64 = 1 << 2;
pub const ALL_RIGHTS: u64 = RIGHT_PRODUCE | RIGHT_CONSUME | RIGHT_TRANSACTION;

pub const ERR_INVALID: i64 = -1;
pub const ERR_DENIED: i64 = -2;
pub const ERR_TRANSPORT: i64 = -3;
pub const ERR_PROTOCOL: i64 = -4;
pub const ERR_BROKER: i64 = -5;
pub const ERR_BUSY: i64 = -6;
pub const ERR_FENCED: i64 = -7;
pub const ERR_TIMEOUT: i64 = -8;
pub const ERR_TOO_LARGE: i64 = -9;
pub const ERR_BAD_OPCODE: i64 = -10;
pub const ERR_TLS_REQUIRED: i64 = -11;
pub const ERR_UNSUPPORTED: i64 = -12;

pub const MAX_RECORD_BYTES: usize = 3_840;
pub const MAX_KEY_BYTES: usize = 1_024;
pub const RECORD_REQUEST_MAGIC: u32 = 0x3152_464b; // "KFR1" LE
pub const RECORD_REQUEST_HEADER_LEN: usize = 24;
pub const FLAG_NULL_KEY: u16 = 1 << 0;
pub const FLAG_NULL_VALUE: u16 = 1 << 1;
pub const VALID_RECORD_FLAGS: u16 = FLAG_NULL_KEY | FLAG_NULL_VALUE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordRequest<'a> {
    pub timestamp_ms: i64,
    pub key: Option<&'a [u8]>,
    pub value: Option<&'a [u8]>,
}

impl RecordRequest<'_> {
    pub const fn new<'a>(key: Option<&'a [u8]>, value: Option<&'a [u8]>) -> RecordRequest<'a> {
        RecordRequest {
            timestamp_ms: -1,
            key,
            value,
        }
    }

    pub const fn with_timestamp(mut self, timestamp_ms: i64) -> Self {
        self.timestamp_ms = timestamp_ms;
        self
    }

    pub fn encoded_len(&self) -> usize {
        RECORD_REQUEST_HEADER_LEN
            + self.key.map_or(0, <[u8]>::len)
            + self.value.map_or(0, <[u8]>::len)
    }

    pub fn encode(&self, output: &mut [u8]) -> Option<usize> {
        let key_len = self.key.map_or(0, <[u8]>::len);
        let value_len = self.value.map_or(0, <[u8]>::len);
        let len = self.encoded_len();
        if key_len > MAX_KEY_BYTES
            || key_len > u16::MAX as usize
            || value_len > MAX_RECORD_BYTES
            || len > MAX_RECORD_BYTES
            || output.len() < len
        {
            return None;
        }
        let mut flags = 0;
        if self.key.is_none() {
            flags |= FLAG_NULL_KEY;
        }
        if self.value.is_none() {
            flags |= FLAG_NULL_VALUE;
        }
        output[..len].fill(0);
        output[0..4].copy_from_slice(&RECORD_REQUEST_MAGIC.to_le_bytes());
        output[4..6].copy_from_slice(&(VERSION as u16).to_le_bytes());
        output[6..8].copy_from_slice(&flags.to_le_bytes());
        output[8..16].copy_from_slice(&self.timestamp_ms.to_le_bytes());
        output[16..18].copy_from_slice(&(key_len as u16).to_le_bytes());
        output[18..22].copy_from_slice(&(value_len as u32).to_le_bytes());
        let key_end = RECORD_REQUEST_HEADER_LEN + key_len;
        if let Some(key) = self.key {
            output[RECORD_REQUEST_HEADER_LEN..key_end].copy_from_slice(key);
        }
        if let Some(value) = self.value {
            output[key_end..key_end + value_len].copy_from_slice(value);
        }
        Some(len)
    }

    pub fn decode(input: &'_ [u8]) -> Option<RecordRequest<'_>> {
        if input.len() < RECORD_REQUEST_HEADER_LEN
            || u32::from_le_bytes(input[0..4].try_into().ok()?) != RECORD_REQUEST_MAGIC
            || u16::from_le_bytes(input[4..6].try_into().ok()?) != VERSION as u16
        {
            return None;
        }
        let flags = u16::from_le_bytes(input[6..8].try_into().ok()?);
        let key_len = u16::from_le_bytes(input[16..18].try_into().ok()?) as usize;
        let value_len = u32::from_le_bytes(input[18..22].try_into().ok()?) as usize;
        let key_end = RECORD_REQUEST_HEADER_LEN.checked_add(key_len)?;
        let end = key_end.checked_add(value_len)?;
        if flags & !VALID_RECORD_FLAGS != 0
            || key_len > MAX_KEY_BYTES
            || value_len > MAX_RECORD_BYTES
            || end != input.len()
            || end > MAX_RECORD_BYTES
            || flags & FLAG_NULL_KEY != 0 && key_len != 0
            || flags & FLAG_NULL_VALUE != 0 && value_len != 0
        {
            return None;
        }
        Some(RecordRequest {
            timestamp_ms: i64::from_le_bytes(input[8..16].try_into().ok()?),
            key: (flags & FLAG_NULL_KEY == 0).then_some(&input[RECORD_REQUEST_HEADER_LEN..key_end]),
            value: (flags & FLAG_NULL_VALUE == 0).then_some(&input[key_end..end]),
        })
    }
}

pub const DELIVERED_MAGIC: u32 = 0x3144_464b; // "KFD1" LE
pub const DELIVERED_HEADER_LEN: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveredRecord<'a> {
    pub partition: i32,
    pub offset: i64,
    pub timestamp_ms: i64,
    pub key: Option<&'a [u8]>,
    pub value: Option<&'a [u8]>,
}

impl DeliveredRecord<'_> {
    pub fn encoded_len(&self) -> usize {
        DELIVERED_HEADER_LEN + self.key.map_or(0, <[u8]>::len) + self.value.map_or(0, <[u8]>::len)
    }

    pub fn encode(&self, output: &mut [u8]) -> Option<usize> {
        let request = RecordRequest {
            timestamp_ms: self.timestamp_ms,
            key: self.key,
            value: self.value,
        };
        let len = DELIVERED_HEADER_LEN + request.encoded_len() - RECORD_REQUEST_HEADER_LEN;
        if len > MAX_RECORD_BYTES || output.len() < len {
            return None;
        }
        let mut flags = 0;
        if self.key.is_none() {
            flags |= FLAG_NULL_KEY;
        }
        if self.value.is_none() {
            flags |= FLAG_NULL_VALUE;
        }
        let key_len = self.key.map_or(0, <[u8]>::len);
        let value_len = self.value.map_or(0, <[u8]>::len);
        output[..len].fill(0);
        output[0..4].copy_from_slice(&DELIVERED_MAGIC.to_le_bytes());
        output[4..6].copy_from_slice(&(VERSION as u16).to_le_bytes());
        output[6..8].copy_from_slice(&flags.to_le_bytes());
        output[8..12].copy_from_slice(&self.partition.to_le_bytes());
        output[12..16].copy_from_slice(&(len as u32).to_le_bytes());
        output[16..24].copy_from_slice(&self.offset.to_le_bytes());
        output[24..32].copy_from_slice(&self.timestamp_ms.to_le_bytes());
        output[32..34].copy_from_slice(&(key_len as u16).to_le_bytes());
        output[34..38].copy_from_slice(&(value_len as u32).to_le_bytes());
        let key_end = DELIVERED_HEADER_LEN + key_len;
        if let Some(key) = self.key {
            output[DELIVERED_HEADER_LEN..key_end].copy_from_slice(key);
        }
        if let Some(value) = self.value {
            output[key_end..key_end + value_len].copy_from_slice(value);
        }
        Some(len)
    }

    pub fn decode(input: &'_ [u8]) -> Option<DeliveredRecord<'_>> {
        if input.len() < DELIVERED_HEADER_LEN
            || u32::from_le_bytes(input[0..4].try_into().ok()?) != DELIVERED_MAGIC
            || u16::from_le_bytes(input[4..6].try_into().ok()?) != VERSION as u16
        {
            return None;
        }
        let flags = u16::from_le_bytes(input[6..8].try_into().ok()?);
        let encoded_len = u32::from_le_bytes(input[12..16].try_into().ok()?) as usize;
        let key_len = u16::from_le_bytes(input[32..34].try_into().ok()?) as usize;
        let value_len = u32::from_le_bytes(input[34..38].try_into().ok()?) as usize;
        let key_end = DELIVERED_HEADER_LEN.checked_add(key_len)?;
        let end = key_end.checked_add(value_len)?;
        if flags & !VALID_RECORD_FLAGS != 0
            || key_len > MAX_KEY_BYTES
            || value_len > MAX_RECORD_BYTES
            || end != encoded_len
            || end > input.len()
            || flags & FLAG_NULL_KEY != 0 && key_len != 0
            || flags & FLAG_NULL_VALUE != 0 && value_len != 0
        {
            return None;
        }
        Some(DeliveredRecord {
            partition: i32::from_le_bytes(input[8..12].try_into().ok()?),
            offset: i64::from_le_bytes(input[16..24].try_into().ok()?),
            timestamp_ms: i64::from_le_bytes(input[24..32].try_into().ok()?),
            key: (flags & FLAG_NULL_KEY == 0).then_some(&input[DELIVERED_HEADER_LEN..key_end]),
            value: (flags & FLAG_NULL_VALUE == 0).then_some(&input[key_end..end]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_preserves_null_and_empty() {
        let request = RecordRequest::new(None, Some(b"")).with_timestamp(7);
        let mut page = [0; 4096];
        let len = request.encode(&mut page).unwrap();
        assert_eq!(RecordRequest::decode(&page[..len]), Some(request));
    }

    #[test]
    fn delivered_round_trip() {
        let record = DeliveredRecord {
            partition: 2,
            offset: 41,
            timestamp_ms: 99,
            key: Some(b"key"),
            value: Some(b"value"),
        };
        let mut page = [0; 4096];
        let len = record.encode(&mut page).unwrap();
        assert_eq!(DeliveredRecord::decode(&page[..len]), Some(record));
    }
}

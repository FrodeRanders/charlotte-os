//! Wire protocol for the CharlotteOS Kafka data-plane service.
//!
//! One service profile grants authority to a bounded broker-destination
//! allow-list, one fixed consume topic/partition, an allow-listed set of
//! produce routes, one consumer group, and one transactional identity. It
//! publishes separately named access points whose Kafka-rights masks attenuate
//! that connector authority. Broker credentials and Kafka producer epochs
//! never cross this application-facing boundary.
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

pub const INTERFACE: u64 = u64::from_le_bytes(*b"KAFKA\0\0\0");
pub const VERSION: u32 = 1;
pub const DEFAULT_NAME: &[u8] = b"kafka";
/// Legacy packed form of [`DEFAULT_NAME`]. Profiles may provision any valid
/// instance name instead.
pub const NAME: u64 = u64::from_le_bytes(*b"kafka\0\0\0");

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
/// Produce outside a transaction to an allow-listed route. `arg0` uses
/// [`pack_routed_record_arg`] with a zero resource id.
pub const OP_PRODUCE_TO: u32 = 13;
/// Produce within a transaction to an allow-listed route. `arg0` uses
/// [`pack_routed_record_arg`] with the transaction id as its resource id.
pub const OP_TX_PRODUCE_TO: u32 = 14;

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
pub const ERR_AUTHENTICATION: i64 = -13;

pub const MAX_RECORD_BYTES: usize = 3_840;
pub const MAX_KEY_BYTES: usize = 1_024;
pub const MAX_TOPIC_BYTES: usize = 249;
/// Hard implementation ceiling. A profile carries a lower operator-selected
/// limit and is rejected if either its declared limit or route count exceeds
/// this value.
pub const MAX_PRODUCE_ROUTES: usize = 64;
/// Maximum number of explicitly authorized broker destinations in one
/// connector profile. Kafka metadata may select only one of these endpoints.
pub const MAX_BROKER_ENDPOINTS: usize = 32;
pub const MAX_SASL_USERNAME_BYTES: usize = 256;
pub const MAX_SASL_PASSWORD_BYTES: usize = 1_024;
pub const MAX_MTLS_CERTIFICATE_BYTES: usize = 16 * 1024;
pub const MAX_MTLS_PRIVATE_KEY_BYTES: usize = 4 * 1024;
/// Matches the name service's one-page bounded name ABI.
pub const MAX_INSTANCE_NAME_BYTES: usize = 256;
/// Bounded number of separately published capability-bearing access points.
pub const MAX_AUTHORITY_ENDPOINTS: usize = 64;
pub const DEFAULT_ROUTE: u16 = 0;
pub const PROFILE_MAGIC: [u8; 8] = *b"CHKAFP5\0";
pub const PROFILE_VERSION: u16 = 5;
pub const PROFILE_HEADER_LEN: usize = 100;
pub const PROFILE_DIGEST_OFFSET: usize = 16;
pub const PROFILE_DIGEST_LEN: usize = 32;
pub const PROFILE_ROUTE_HEADER_LEN: usize = 8;
pub const PROFILE_BROKER_HEADER_LEN: usize = 8;
pub const PROFILE_AUTHORITY_HEADER_LEN: usize = 16;
pub const MAX_PROFILE_BYTES: usize = 64 * 1024;
pub const PROFILE_FLAG_TLS: u16 = 1;
pub const AUTH_NONE: u16 = 0;
pub const AUTH_SCRAM_SHA_256: u16 = 1;
pub const AUTH_MTLS_P256: u16 = 2;
pub const AUTH_SCRAM_SHA_256_AND_MTLS_P256: u16 = 3;
pub const PROFILE_MTLS_HEADER_LEN: usize = 8;
pub const RECORD_REQUEST_MAGIC: u32 = 0x3152_464b; // "KFR1" LE
pub const RECORD_REQUEST_HEADER_LEN: usize = 24;
pub const FLAG_NULL_KEY: u16 = 1 << 0;
pub const FLAG_NULL_VALUE: u16 = 1 << 1;
pub const VALID_RECORD_FLAGS: u16 = FLAG_NULL_KEY | FLAG_NULL_VALUE;

/// A provisioned topic/partition route. Route zero always denotes the
/// profile's fixed consume topic; profile routes are numbered from one in
/// declaration order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProduceRoute<'a> {
    pub topic: &'a [u8],
    pub partition: i32,
}

/// A network destination the connector may select after reading Kafka
/// metadata. The hostname and port must exactly match the broker-advertised
/// endpoint; the provisioned IPv4 address is never learned from the broker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerEndpoint<'a> {
    pub endpoint_ipv4: [u8; 4],
    pub host: &'a [u8],
    pub port: u16,
}

/// One published connector endpoint and its Kafka operation ceiling.
///
/// IPC rights only decide whether a caller may issue a call. These rights are
/// checked again by the connector against the opcode received on this exact
/// endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityEndpoint<'a> {
    pub service_name: &'a [u8],
    pub rights: u64,
}

/// Connector-only Kafka authentication material. This is part of the
/// read-only launch profile and never appears in the application IPC ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authentication<'a> {
    None,
    ScramSha256 {
        username: &'a [u8],
        password: &'a [u8],
    },
    MtlsP256 {
        certificate_der: &'a [u8],
        private_key_der: &'a [u8],
    },
    ScramSha256AndMtlsP256 {
        username: &'a [u8],
        password: &'a [u8],
        certificate_der: &'a [u8],
        private_key_der: &'a [u8],
    },
}

impl BrokerEndpoint<'_> {
    fn valid(&self) -> bool {
        self.port != 0
            && !self.host.is_empty()
            && self.host.len() <= 255
            && core::str::from_utf8(self.host).is_ok()
    }
}

impl ProduceRoute<'_> {
    fn valid(&self) -> bool {
        self.partition >= 0 && !self.topic.is_empty() && self.topic.len() <= MAX_TOPIC_BYTES
    }
}

/// Immutable launch profile carried in a kernel-enforced read-only memory
/// capability. All variable-length data is covered by the SHA-256 digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile<'a> {
    /// Stable connector instance identity used for status and diagnostics.
    /// Applications use one of `authority_endpoints`, never this label.
    pub instance_name: &'a [u8],
    /// Separately published endpoints whose rights are bounded by `rights`.
    pub authority_endpoints: Vec<AuthorityEndpoint<'a>>,
    pub endpoint_ipv4: [u8; 4],
    pub host: &'a [u8],
    pub port: u16,
    /// Additional metadata-selectable destinations. The primary
    /// `endpoint_ipv4`/`host`/`port` tuple is always endpoint zero.
    pub broker_endpoints: Vec<BrokerEndpoint<'a>>,
    pub tls: bool,
    pub ca_certificate_der: &'a [u8],
    pub topic: &'a [u8],
    pub partition: i32,
    pub produce_routes: Vec<ProduceRoute<'a>>,
    pub max_produce_routes: u16,
    pub group: &'a [u8],
    pub transactional_id: &'a [u8],
    pub authentication: Authentication<'a>,
    pub rights: u64,
    pub transaction_timeout_ms: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileError {
    TooLarge,
    InvalidHeader,
    UnsupportedVersion,
    DigestMismatch,
    InvalidField,
    TooManyRoutes,
    DuplicateRoute,
    TooManyBrokers,
    DuplicateBroker,
    TooManyAuthorities,
    DuplicateAuthority,
}

impl Profile<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, ProfileError> {
        validate_profile(self)?;
        let route_bytes = self.produce_routes.iter().try_fold(0usize, |total, route| {
            total.checked_add(PROFILE_ROUTE_HEADER_LEN + route.topic.len())
        });
        let broker_bytes = self.broker_endpoints.iter().try_fold(0usize, |total, broker| {
            total.checked_add(PROFILE_BROKER_HEADER_LEN + broker.host.len())
        });
        let authority_bytes =
            self.authority_endpoints.iter().try_fold(0usize, |total, endpoint| {
                total.checked_add(PROFILE_AUTHORITY_HEADER_LEN + endpoint.service_name.len())
            });
        let total_len = PROFILE_HEADER_LEN
            .checked_add(self.instance_name.len())
            .and_then(|len| len.checked_add(self.host.len()))
            .and_then(|len| len.checked_add(self.ca_certificate_der.len()))
            .and_then(|len| len.checked_add(self.topic.len()))
            .and_then(|len| len.checked_add(self.group.len()))
            .and_then(|len| len.checked_add(self.transactional_id.len()))
            .and_then(|len| {
                let (_, username, password, certificate, private_key) =
                    authentication_fields(self.authentication);
                len.checked_add(username.len())?
                    .checked_add(password.len())?
                    .checked_add(
                        if certificate.is_empty() {
                            0
                        } else {
                            PROFILE_MTLS_HEADER_LEN
                        },
                    )?
                    .checked_add(certificate.len())?
                    .checked_add(private_key.len())
            })
            .and_then(|len| len.checked_add(route_bytes?))
            .and_then(|len| len.checked_add(broker_bytes?))
            .and_then(|len| len.checked_add(authority_bytes?))
            .ok_or(ProfileError::TooLarge)?;
        if total_len > MAX_PROFILE_BYTES || total_len > u32::MAX as usize {
            return Err(ProfileError::TooLarge);
        }
        let mut output = alloc::vec![0; total_len];
        output[0..8].copy_from_slice(&PROFILE_MAGIC);
        put_u16(&mut output, 8, PROFILE_VERSION);
        put_u16(&mut output, 10, PROFILE_HEADER_LEN as u16);
        put_u32(&mut output, 12, total_len as u32);
        output[48..52].copy_from_slice(&self.endpoint_ipv4);
        put_u16(&mut output, 52, self.port);
        put_u16(
            &mut output,
            54,
            if self.tls {
                PROFILE_FLAG_TLS
            } else {
                0
            },
        );
        put_u64(&mut output, 56, self.rights);
        put_u32(&mut output, 64, self.transaction_timeout_ms);
        put_i32(&mut output, 68, self.partition);
        put_u16(&mut output, 72, self.produce_routes.len() as u16);
        put_u16(&mut output, 74, self.max_produce_routes);
        put_u16(&mut output, 76, self.host.len() as u16);
        put_u16(&mut output, 78, self.topic.len() as u16);
        put_u16(&mut output, 80, self.group.len() as u16);
        put_u16(&mut output, 82, self.transactional_id.len() as u16);
        put_u32(&mut output, 84, self.ca_certificate_der.len() as u32);
        put_u16(&mut output, 88, self.broker_endpoints.len() as u16);
        let (auth_kind, username, password, certificate, private_key) =
            authentication_fields(self.authentication);
        put_u16(&mut output, 90, auth_kind);
        put_u16(&mut output, 92, username.len() as u16);
        put_u16(&mut output, 94, password.len() as u16);
        put_u16(&mut output, 96, self.instance_name.len() as u16);
        put_u16(&mut output, 98, self.authority_endpoints.len() as u16);
        let mut offset = PROFILE_HEADER_LEN;
        for field in [
            self.instance_name,
            self.host,
            self.ca_certificate_der,
            self.topic,
            self.group,
            self.transactional_id,
        ] {
            output[offset..offset + field.len()].copy_from_slice(field);
            offset += field.len();
        }
        for field in [username, password] {
            output[offset..offset + field.len()].copy_from_slice(field);
            offset += field.len();
        }
        if !certificate.is_empty() {
            put_u32(&mut output, offset, certificate.len() as u32);
            put_u32(&mut output, offset + 4, private_key.len() as u32);
            offset += PROFILE_MTLS_HEADER_LEN;
            for field in [certificate, private_key] {
                output[offset..offset + field.len()].copy_from_slice(field);
                offset += field.len();
            }
        }
        for route in &self.produce_routes {
            put_i32(&mut output, offset, route.partition);
            put_u16(&mut output, offset + 4, route.topic.len() as u16);
            offset += PROFILE_ROUTE_HEADER_LEN;
            output[offset..offset + route.topic.len()].copy_from_slice(route.topic);
            offset += route.topic.len();
        }
        for broker in &self.broker_endpoints {
            output[offset..offset + 4].copy_from_slice(&broker.endpoint_ipv4);
            put_u16(&mut output, offset + 4, broker.port);
            put_u16(&mut output, offset + 6, broker.host.len() as u16);
            offset += PROFILE_BROKER_HEADER_LEN;
            output[offset..offset + broker.host.len()].copy_from_slice(broker.host);
            offset += broker.host.len();
        }
        for endpoint in &self.authority_endpoints {
            put_u64(&mut output, offset, endpoint.rights);
            put_u16(&mut output, offset + 8, endpoint.service_name.len() as u16);
            offset += PROFILE_AUTHORITY_HEADER_LEN;
            output[offset..offset + endpoint.service_name.len()]
                .copy_from_slice(endpoint.service_name);
            offset += endpoint.service_name.len();
        }
        let digest = charlotte_launch::sha256::digest(&output);
        output[PROFILE_DIGEST_OFFSET..PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN]
            .copy_from_slice(&digest);
        Ok(output)
    }

    pub fn decode(input: &'_ [u8]) -> Result<Profile<'_>, ProfileError> {
        if input.len() < PROFILE_HEADER_LEN
            || input.len() > MAX_PROFILE_BYTES
            || input[0..8] != PROFILE_MAGIC
        {
            return Err(ProfileError::InvalidHeader);
        }
        if get_u16(input, 8)? != PROFILE_VERSION {
            return Err(ProfileError::UnsupportedVersion);
        }
        if get_u16(input, 10)? as usize != PROFILE_HEADER_LEN
            || get_u32(input, 12)? as usize != input.len()
        {
            return Err(ProfileError::InvalidHeader);
        }
        let expected: [u8; 32] = input
            [PROFILE_DIGEST_OFFSET..PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN]
            .try_into()
            .map_err(|_| ProfileError::InvalidHeader)?;
        let mut hasher = charlotte_launch::sha256::Sha256::new();
        hasher.update(&input[..PROFILE_DIGEST_OFFSET]);
        hasher.update(&[0; PROFILE_DIGEST_LEN]);
        hasher.update(&input[PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN..]);
        if hasher.finalize() != expected {
            return Err(ProfileError::DigestMismatch);
        }
        let flags = get_u16(input, 54)?;
        let route_count = get_u16(input, 72)? as usize;
        let broker_count = get_u16(input, 88)? as usize;
        let auth_kind = get_u16(input, 90)?;
        let username_len = get_u16(input, 92)? as usize;
        let password_len = get_u16(input, 94)? as usize;
        let instance_name_len = get_u16(input, 96)? as usize;
        let authority_count = get_u16(input, 98)? as usize;
        let max_produce_routes = get_u16(input, 74)?;
        if flags & !PROFILE_FLAG_TLS != 0
            || route_count > MAX_PRODUCE_ROUTES
            || route_count > usize::from(max_produce_routes)
            || usize::from(max_produce_routes) > MAX_PRODUCE_ROUTES
        {
            return Err(ProfileError::TooManyRoutes);
        }
        if !matches!(
            auth_kind,
            AUTH_NONE | AUTH_SCRAM_SHA_256 | AUTH_MTLS_P256 | AUTH_SCRAM_SHA_256_AND_MTLS_P256
        ) || auth_kind == AUTH_NONE && (username_len != 0 || password_len != 0)
            || matches!(auth_kind, AUTH_SCRAM_SHA_256 | AUTH_SCRAM_SHA_256_AND_MTLS_P256)
                && (username_len == 0
                    || username_len > MAX_SASL_USERNAME_BYTES
                    || password_len == 0
                    || password_len > MAX_SASL_PASSWORD_BYTES)
            || auth_kind == AUTH_MTLS_P256 && (username_len != 0 || password_len != 0)
        {
            return Err(ProfileError::InvalidField);
        }
        if broker_count > MAX_BROKER_ENDPOINTS.saturating_sub(1) {
            return Err(ProfileError::TooManyBrokers);
        }
        if authority_count == 0 || authority_count > MAX_AUTHORITY_ENDPOINTS {
            return Err(ProfileError::TooManyAuthorities);
        }
        let lengths = [
            instance_name_len,
            get_u16(input, 76)? as usize,
            get_u32(input, 84)? as usize,
            get_u16(input, 78)? as usize,
            get_u16(input, 80)? as usize,
            get_u16(input, 82)? as usize,
        ];
        let mut offset = PROFILE_HEADER_LEN;
        let mut fields: [&[u8]; 6] = [&[]; 6];
        for (slot, len) in fields.iter_mut().zip(lengths) {
            let end = offset.checked_add(len).ok_or(ProfileError::TooLarge)?;
            *slot = input.get(offset..end).ok_or(ProfileError::InvalidField)?;
            offset = end;
        }
        let username_end = offset.checked_add(username_len).ok_or(ProfileError::TooLarge)?;
        let username = input.get(offset..username_end).ok_or(ProfileError::InvalidField)?;
        offset = username_end;
        let password_end = offset.checked_add(password_len).ok_or(ProfileError::TooLarge)?;
        let password = input.get(offset..password_end).ok_or(ProfileError::InvalidField)?;
        offset = password_end;
        let (certificate_der, private_key_der) =
            if matches!(auth_kind, AUTH_MTLS_P256 | AUTH_SCRAM_SHA_256_AND_MTLS_P256) {
                let header = input
                    .get(offset..offset + PROFILE_MTLS_HEADER_LEN)
                    .ok_or(ProfileError::InvalidField)?;
                let certificate_len = get_u32(header, 0)? as usize;
                let private_key_len = get_u32(header, 4)? as usize;
                if certificate_len == 0
                    || certificate_len > MAX_MTLS_CERTIFICATE_BYTES
                    || private_key_len == 0
                    || private_key_len > MAX_MTLS_PRIVATE_KEY_BYTES
                {
                    return Err(ProfileError::InvalidField);
                }
                offset += PROFILE_MTLS_HEADER_LEN;
                let certificate_end =
                    offset.checked_add(certificate_len).ok_or(ProfileError::TooLarge)?;
                let certificate =
                    input.get(offset..certificate_end).ok_or(ProfileError::InvalidField)?;
                offset = certificate_end;
                let private_key_end =
                    offset.checked_add(private_key_len).ok_or(ProfileError::TooLarge)?;
                let private_key =
                    input.get(offset..private_key_end).ok_or(ProfileError::InvalidField)?;
                offset = private_key_end;
                (certificate, private_key)
            } else {
                (&[][..], &[][..])
            };
        let mut produce_routes = Vec::with_capacity(route_count);
        for _ in 0..route_count {
            let header = input
                .get(offset..offset + PROFILE_ROUTE_HEADER_LEN)
                .ok_or(ProfileError::InvalidField)?;
            let partition = get_i32(header, 0)?;
            let topic_len = get_u16(header, 4)? as usize;
            if get_u16(header, 6)? != 0 {
                return Err(ProfileError::InvalidField);
            }
            offset += PROFILE_ROUTE_HEADER_LEN;
            let end = offset.checked_add(topic_len).ok_or(ProfileError::TooLarge)?;
            let topic = input.get(offset..end).ok_or(ProfileError::InvalidField)?;
            offset = end;
            produce_routes.push(ProduceRoute {
                topic,
                partition,
            });
        }
        let mut broker_endpoints = Vec::with_capacity(broker_count);
        for _ in 0..broker_count {
            let header = input
                .get(offset..offset + PROFILE_BROKER_HEADER_LEN)
                .ok_or(ProfileError::InvalidField)?;
            let endpoint_ipv4 = header[0..4].try_into().map_err(|_| ProfileError::InvalidField)?;
            let port = get_u16(header, 4)?;
            let host_len = get_u16(header, 6)? as usize;
            offset += PROFILE_BROKER_HEADER_LEN;
            let end = offset.checked_add(host_len).ok_or(ProfileError::TooLarge)?;
            let host = input.get(offset..end).ok_or(ProfileError::InvalidField)?;
            offset = end;
            broker_endpoints.push(BrokerEndpoint {
                endpoint_ipv4,
                host,
                port,
            });
        }
        let mut authority_endpoints = Vec::with_capacity(authority_count);
        for _ in 0..authority_count {
            let header = input
                .get(offset..offset + PROFILE_AUTHORITY_HEADER_LEN)
                .ok_or(ProfileError::InvalidField)?;
            let rights = get_u64(header, 0)?;
            let service_name_len = get_u16(header, 8)? as usize;
            if header[10..PROFILE_AUTHORITY_HEADER_LEN].iter().any(|byte| *byte != 0) {
                return Err(ProfileError::InvalidField);
            }
            offset += PROFILE_AUTHORITY_HEADER_LEN;
            let end = offset.checked_add(service_name_len).ok_or(ProfileError::TooLarge)?;
            let service_name = input.get(offset..end).ok_or(ProfileError::InvalidField)?;
            offset = end;
            authority_endpoints.push(AuthorityEndpoint {
                service_name,
                rights,
            });
        }
        if offset != input.len() {
            return Err(ProfileError::InvalidField);
        }
        let profile = Profile {
            instance_name: fields[0],
            authority_endpoints,
            endpoint_ipv4: input[48..52].try_into().map_err(|_| ProfileError::InvalidHeader)?,
            host: fields[1],
            port: get_u16(input, 52)?,
            broker_endpoints,
            tls: flags & PROFILE_FLAG_TLS != 0,
            ca_certificate_der: fields[2],
            topic: fields[3],
            partition: get_i32(input, 68)?,
            produce_routes,
            max_produce_routes,
            group: fields[4],
            transactional_id: fields[5],
            authentication: match auth_kind {
                AUTH_NONE => Authentication::None,
                AUTH_SCRAM_SHA_256 => Authentication::ScramSha256 {
                    username,
                    password,
                },
                AUTH_MTLS_P256 => Authentication::MtlsP256 {
                    certificate_der,
                    private_key_der,
                },
                AUTH_SCRAM_SHA_256_AND_MTLS_P256 => Authentication::ScramSha256AndMtlsP256 {
                    username,
                    password,
                    certificate_der,
                    private_key_der,
                },
                _ => return Err(ProfileError::InvalidField),
            },
            rights: get_u64(input, 56)?,
            transaction_timeout_ms: get_u32(input, 64)?,
        };
        validate_profile(&profile)?;
        Ok(profile)
    }
}

fn validate_profile(profile: &Profile<'_>) -> Result<(), ProfileError> {
    if profile.instance_name.is_empty()
        || profile.instance_name.len() > MAX_INSTANCE_NAME_BYTES
        || !profile.instance_name.iter().all(u8::is_ascii_graphic)
        || profile.port == 0
        || profile.host.is_empty()
        || profile.host.len() > 255
        || core::str::from_utf8(profile.host).is_err()
        || profile.topic.is_empty()
        || profile.topic.len() > MAX_TOPIC_BYTES
        || profile.partition < 0
        || profile.group.is_empty()
        || profile.group.len() > u16::MAX as usize
        || profile.transactional_id.is_empty()
        || profile.transactional_id.len() > u16::MAX as usize
        || profile.rights == 0
        || profile.rights & !ALL_RIGHTS != 0
        || !(1_000..=900_000).contains(&profile.transaction_timeout_ms)
        || profile.tls != !profile.ca_certificate_der.is_empty()
    {
        return Err(ProfileError::InvalidField);
    }
    if profile.authority_endpoints.is_empty()
        || profile.authority_endpoints.len() > MAX_AUTHORITY_ENDPOINTS
    {
        return Err(ProfileError::TooManyAuthorities);
    }
    for (index, endpoint) in profile.authority_endpoints.iter().enumerate() {
        if endpoint.service_name.is_empty()
            || endpoint.service_name.len() > MAX_INSTANCE_NAME_BYTES
            || !endpoint.service_name.iter().all(u8::is_ascii_graphic)
            || endpoint.rights == 0
            || endpoint.rights & !profile.rights != 0
        {
            return Err(ProfileError::InvalidField);
        }
        if profile.authority_endpoints[..index]
            .iter()
            .any(|candidate| candidate.service_name == endpoint.service_name)
        {
            return Err(ProfileError::DuplicateAuthority);
        }
    }
    match profile.authentication {
        Authentication::None => {}
        Authentication::ScramSha256 {
            username,
            password,
        } => {
            if !profile.tls || !valid_scram(username, password) {
                return Err(ProfileError::InvalidField);
            }
        }
        Authentication::MtlsP256 {
            certificate_der,
            private_key_der,
        } => {
            if !profile.tls || !valid_mtls(certificate_der, private_key_der) {
                return Err(ProfileError::InvalidField);
            }
        }
        Authentication::ScramSha256AndMtlsP256 {
            username,
            password,
            certificate_der,
            private_key_der,
        } => {
            if !profile.tls
                || !valid_scram(username, password)
                || !valid_mtls(certificate_der, private_key_der)
            {
                return Err(ProfileError::InvalidField);
            }
        }
    }
    if profile.broker_endpoints.len() >= MAX_BROKER_ENDPOINTS {
        return Err(ProfileError::TooManyBrokers);
    }
    for (index, broker) in profile.broker_endpoints.iter().enumerate() {
        if !broker.valid() {
            return Err(ProfileError::InvalidField);
        }
        if broker.host == profile.host && broker.port == profile.port
            || profile.broker_endpoints[..index]
                .iter()
                .any(|candidate| candidate.host == broker.host && candidate.port == broker.port)
        {
            return Err(ProfileError::DuplicateBroker);
        }
    }
    if profile.produce_routes.len() > MAX_PRODUCE_ROUTES
        || profile.produce_routes.len() > usize::from(profile.max_produce_routes)
        || usize::from(profile.max_produce_routes) > MAX_PRODUCE_ROUTES
    {
        return Err(ProfileError::TooManyRoutes);
    }
    for (index, route) in profile.produce_routes.iter().enumerate() {
        if !route.valid() {
            return Err(ProfileError::InvalidField);
        }
        if route.topic == profile.topic && route.partition == profile.partition
            || profile.produce_routes[..index].iter().any(|candidate| candidate == route)
        {
            return Err(ProfileError::DuplicateRoute);
        }
    }
    Ok(())
}

fn valid_scram(username: &[u8], password: &[u8]) -> bool {
    !username.is_empty()
        && username.len() <= MAX_SASL_USERNAME_BYTES
        && !password.is_empty()
        && password.len() <= MAX_SASL_PASSWORD_BYTES
        && core::str::from_utf8(username).is_ok()
        && core::str::from_utf8(password).is_ok()
        && username.iter().all(|byte| (0x20..=0x7e).contains(byte))
        && password.iter().all(|byte| (0x20..=0x7e).contains(byte))
}

fn valid_mtls(certificate_der: &[u8], private_key_der: &[u8]) -> bool {
    !certificate_der.is_empty()
        && certificate_der.len() <= MAX_MTLS_CERTIFICATE_BYTES
        && !private_key_der.is_empty()
        && private_key_der.len() <= MAX_MTLS_PRIVATE_KEY_BYTES
}

fn authentication_fields(authentication: Authentication<'_>) -> (u16, &[u8], &[u8], &[u8], &[u8]) {
    match authentication {
        Authentication::None => (AUTH_NONE, &[], &[], &[], &[]),
        Authentication::ScramSha256 {
            username,
            password,
        } => (AUTH_SCRAM_SHA_256, username, password, &[], &[]),
        Authentication::MtlsP256 {
            certificate_der,
            private_key_der,
        } => (AUTH_MTLS_P256, &[], &[], certificate_der, private_key_der),
        Authentication::ScramSha256AndMtlsP256 {
            username,
            password,
            certificate_der,
            private_key_der,
        } => {
            (AUTH_SCRAM_SHA_256_AND_MTLS_P256, username, password, certificate_der, private_key_der)
        }
    }
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(output: &mut [u8], offset: usize, value: i32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(input: &[u8], offset: usize) -> Result<u16, ProfileError> {
    Ok(u16::from_le_bytes(
        input.get(offset..offset + 2).ok_or(ProfileError::InvalidHeader)?.try_into().unwrap(),
    ))
}

fn get_u32(input: &[u8], offset: usize) -> Result<u32, ProfileError> {
    Ok(u32::from_le_bytes(
        input.get(offset..offset + 4).ok_or(ProfileError::InvalidHeader)?.try_into().unwrap(),
    ))
}

fn get_i32(input: &[u8], offset: usize) -> Result<i32, ProfileError> {
    Ok(i32::from_le_bytes(
        input.get(offset..offset + 4).ok_or(ProfileError::InvalidHeader)?.try_into().unwrap(),
    ))
}

fn get_u64(input: &[u8], offset: usize) -> Result<u64, ProfileError> {
    Ok(u64::from_le_bytes(
        input.get(offset..offset + 8).ok_or(ProfileError::InvalidHeader)?.try_into().unwrap(),
    ))
}

/// Pack a routed record call into the scalar IPC argument.
///
/// Layout: resource id in bits 0..31, encoded record length in bits 32..47,
/// and the provisioned route index in bits 48..63.
pub fn pack_routed_record_arg(resource_id: u32, route: u16, len: usize) -> Option<u64> {
    let len = u16::try_from(len).ok()?;
    Some(u64::from(resource_id) | (u64::from(len) << 32) | (u64::from(route) << 48))
}

pub fn unpack_routed_record_arg(arg: u64) -> (u32, u16, usize) {
    (arg as u32, (arg >> 48) as u16, ((arg >> 32) as u16) as usize)
}

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

    fn authorities() -> Vec<AuthorityEndpoint<'static>> {
        alloc::vec![AuthorityEndpoint {
            service_name: DEFAULT_NAME,
            rights: ALL_RIGHTS,
        }]
    }

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

    #[test]
    fn profile_round_trip_and_integrity() {
        let profile = Profile {
            instance_name: b"orders-worker-1",
            authority_endpoints: alloc::vec![
                AuthorityEndpoint {
                    service_name: b"kafka/orders/worker-1/producer",
                    rights: RIGHT_PRODUCE,
                },
                AuthorityEndpoint {
                    service_name: b"kafka/orders/worker-1/step",
                    rights: ALL_RIGHTS,
                },
            ],
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka.test",
            port: 9093,
            broker_endpoints: alloc::vec![BrokerEndpoint {
                endpoint_ipv4: [10, 0, 2, 3],
                host: b"kafka-2.test",
                port: 9093,
            }],
            tls: true,
            ca_certificate_der: b"certificate",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![ProduceRoute {
                topic: b"results",
                partition: 3,
            }],
            max_produce_routes: 64,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::ScramSha256AndMtlsP256 {
                username: b"worker",
                password: b"secret",
                certificate_der: b"client-certificate",
                private_key_der: b"client-private-key",
            },
            rights: ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        };
        let encoded = profile.encode().unwrap();
        assert_eq!(Profile::decode(&encoded).unwrap(), profile);
        let mut corrupt = encoded;
        *corrupt.last_mut().unwrap() ^= 1;
        assert_eq!(Profile::decode(&corrupt), Err(ProfileError::DigestMismatch));

        let mut invalid_name = profile;
        invalid_name.authority_endpoints[0].service_name = b"kafka/orders/bad name";
        assert_eq!(invalid_name.encode(), Err(ProfileError::InvalidField));
    }

    #[test]
    fn profile_rejects_unusable_and_duplicate_broker_destinations() {
        let mut profile = Profile {
            instance_name: DEFAULT_NAME,
            authority_endpoints: authorities(),
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka-1.test",
            port: 9093,
            broker_endpoints: alloc::vec![BrokerEndpoint {
                endpoint_ipv4: [10, 0, 2, 3],
                host: b"kafka-1.test",
                port: 9093,
            }],
            tls: false,
            ca_certificate_der: b"",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![],
            max_produce_routes: 64,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::None,
            rights: ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        };
        assert_eq!(profile.encode(), Err(ProfileError::DuplicateBroker));

        profile.broker_endpoints[0].host = b"\xff";
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));
        profile.broker_endpoints = alloc::vec![
            BrokerEndpoint {
                endpoint_ipv4: [10, 0, 2, 3],
                host: b"broker.test",
                port: 9093,
            };
            MAX_BROKER_ENDPOINTS
        ];
        assert_eq!(profile.encode(), Err(ProfileError::TooManyBrokers));
    }

    #[test]
    fn profile_rejects_non_utf8_primary_hostname() {
        let profile = Profile {
            instance_name: DEFAULT_NAME,
            authority_endpoints: authorities(),
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"\xff",
            port: 9093,
            broker_endpoints: alloc::vec![],
            tls: false,
            ca_certificate_der: b"",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![],
            max_produce_routes: 64,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::None,
            rights: ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        };
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));
    }

    #[test]
    fn profile_requires_tls_and_bounded_ascii_for_scram() {
        let mut profile = Profile {
            instance_name: DEFAULT_NAME,
            authority_endpoints: authorities(),
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka.test",
            port: 9093,
            broker_endpoints: alloc::vec![],
            tls: false,
            ca_certificate_der: b"",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![],
            max_produce_routes: 64,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::ScramSha256 {
                username: b"worker",
                password: b"secret",
            },
            rights: ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        };
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));
        profile.tls = true;
        profile.ca_certificate_der = b"certificate";
        profile.authentication = Authentication::ScramSha256 {
            username: b"worker",
            password: b"line\nbreak",
        };
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));
    }

    #[test]
    fn profile_requires_tls_and_bounded_identity_for_mtls() {
        let mut profile = Profile {
            instance_name: DEFAULT_NAME,
            authority_endpoints: authorities(),
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka.test",
            port: 9093,
            broker_endpoints: alloc::vec![],
            tls: false,
            ca_certificate_der: b"",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![],
            max_produce_routes: 64,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::MtlsP256 {
                certificate_der: b"client-certificate",
                private_key_der: b"client-private-key",
            },
            rights: ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        };
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));

        profile.tls = true;
        profile.ca_certificate_der = b"certificate";
        profile.authentication = Authentication::MtlsP256 {
            certificate_der: b"",
            private_key_der: b"client-private-key",
        };
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));

        let oversized_private_key = alloc::vec![0; MAX_MTLS_PRIVATE_KEY_BYTES + 1];
        profile.authentication = Authentication::MtlsP256 {
            certificate_der: b"client-certificate",
            private_key_der: &oversized_private_key,
        };
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));
    }

    #[test]
    fn profile_rejects_duplicate_routes() {
        let route = ProduceRoute {
            topic: b"results",
            partition: 0,
        };
        let profile = Profile {
            instance_name: DEFAULT_NAME,
            authority_endpoints: authorities(),
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka.test",
            port: 9093,
            broker_endpoints: alloc::vec![],
            tls: false,
            ca_certificate_der: b"",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![route, route],
            max_produce_routes: 64,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::None,
            rights: ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        };
        assert_eq!(profile.encode(), Err(ProfileError::DuplicateRoute));
    }

    #[test]
    fn profile_enforces_declared_and_hard_route_limits() {
        let route = ProduceRoute {
            topic: b"results",
            partition: 0,
        };
        let mut profile = Profile {
            instance_name: DEFAULT_NAME,
            authority_endpoints: authorities(),
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka.test",
            port: 9093,
            broker_endpoints: alloc::vec![],
            tls: false,
            ca_certificate_der: b"",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![route],
            max_produce_routes: 0,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::None,
            rights: ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        };
        assert_eq!(profile.encode(), Err(ProfileError::TooManyRoutes));
        profile.produce_routes = alloc::vec![route; MAX_PRODUCE_ROUTES + 1];
        profile.max_produce_routes = (MAX_PRODUCE_ROUTES + 1) as u16;
        assert_eq!(profile.encode(), Err(ProfileError::TooManyRoutes));
    }

    #[test]
    fn profile_rejects_duplicate_or_escalated_authority_endpoints() {
        let mut profile = Profile {
            instance_name: DEFAULT_NAME,
            authority_endpoints: alloc::vec![
                AuthorityEndpoint {
                    service_name: b"kafka/producer",
                    rights: RIGHT_PRODUCE,
                },
                AuthorityEndpoint {
                    service_name: b"kafka/producer",
                    rights: RIGHT_PRODUCE,
                },
            ],
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka.test",
            port: 9093,
            broker_endpoints: alloc::vec![],
            tls: false,
            ca_certificate_der: b"",
            topic: b"events",
            partition: 0,
            produce_routes: alloc::vec![],
            max_produce_routes: 64,
            group: b"workers",
            transactional_id: b"worker-1",
            authentication: Authentication::None,
            rights: RIGHT_PRODUCE,
            transaction_timeout_ms: 60_000,
        };
        assert_eq!(profile.encode(), Err(ProfileError::DuplicateAuthority));

        profile.authority_endpoints.truncate(1);
        profile.authority_endpoints[0].rights = ALL_RIGHTS;
        assert_eq!(profile.encode(), Err(ProfileError::InvalidField));

        profile.authority_endpoints.clear();
        assert_eq!(profile.encode(), Err(ProfileError::TooManyAuthorities));
    }

    #[test]
    fn routed_record_argument_round_trip() {
        let encoded = pack_routed_record_arg(0x1234_5678, 7, MAX_RECORD_BYTES).unwrap();
        assert_eq!(unpack_routed_record_arg(encoded), (0x1234_5678, 7, MAX_RECORD_BYTES));
    }
}

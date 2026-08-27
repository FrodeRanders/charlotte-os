//! Bounded SCRAM-SHA-256 client state for Kafka SASL authentication.

use alloc::{
    string::String,
    vec,
    vec::Vec,
};

use charlotte_launch::sha256::{
    Sha256,
    digest,
};
use zeroize::Zeroizing;

pub const MECHANISM: &[u8] = b"SCRAM-SHA-256";
pub const MIN_ITERATIONS: u32 = 4_096;
pub const MAX_ITERATIONS: u32 = 1_000_000;
const MAX_SERVER_MESSAGE_BYTES: usize = 4_096;
const MAX_SALT_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidCredential,
    InvalidNonce,
    InvalidServerMessage,
    UnsupportedExtension,
    IterationLimit,
    ServerRejected,
    SignatureMismatch,
}

/// One SCRAM exchange. Secret material and derived keys are erased when the
/// exchange completes or an error path drops it.
pub struct Client {
    password: Zeroizing<Vec<u8>>,
    nonce: String,
    client_first_bare: Zeroizing<Vec<u8>>,
    expected_server_signature: Option<Zeroizing<[u8; 32]>>,
}

impl Client {
    pub fn new(username: &str, password: &[u8], nonce: &str) -> Result<Self, Error> {
        if username.is_empty()
            || password.is_empty()
            || nonce.is_empty()
            || !nonce.bytes().all(|byte| (0x21..=0x7e).contains(&byte) && byte != b',')
        {
            return Err(Error::InvalidCredential);
        }
        let escaped = escape_username(username);
        let mut client_first_bare =
            Zeroizing::new(Vec::with_capacity(2 + escaped.len() + 3 + nonce.len()));
        client_first_bare.extend_from_slice(b"n=");
        client_first_bare.extend_from_slice(escaped.as_bytes());
        client_first_bare.extend_from_slice(b",r=");
        client_first_bare.extend_from_slice(nonce.as_bytes());
        Ok(Self {
            password: Zeroizing::new(password.to_vec()),
            nonce: String::from(nonce),
            client_first_bare,
            expected_server_signature: None,
        })
    }

    pub fn client_first(&self) -> Zeroizing<Vec<u8>> {
        let mut message = Zeroizing::new(Vec::with_capacity(3 + self.client_first_bare.len()));
        message.extend_from_slice(b"n,,");
        message.extend_from_slice(&self.client_first_bare);
        message
    }

    pub fn receive_server_first(&mut self, message: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        if message.is_empty() || message.len() > MAX_SERVER_MESSAGE_BYTES {
            return Err(Error::InvalidServerMessage);
        }
        let text = core::str::from_utf8(message).map_err(|_| Error::InvalidServerMessage)?;
        let mut combined_nonce = None;
        let mut salt = None;
        let mut iterations = None;
        for attribute in text.split(',') {
            let (name, value) = attribute.split_once('=').ok_or(Error::InvalidServerMessage)?;
            match name {
                "r" if combined_nonce.is_none() => combined_nonce = Some(value),
                "s" if salt.is_none() => salt = Some(value),
                "i" if iterations.is_none() => {
                    iterations =
                        Some(value.parse::<u32>().map_err(|_| Error::InvalidServerMessage)?);
                }
                "m" => return Err(Error::UnsupportedExtension),
                _ => return Err(Error::InvalidServerMessage),
            }
        }
        let combined_nonce = combined_nonce.ok_or(Error::InvalidServerMessage)?;
        if !combined_nonce.starts_with(&self.nonce)
            || combined_nonce.len() <= self.nonce.len()
            || !combined_nonce.bytes().all(|byte| (0x21..=0x7e).contains(&byte) && byte != b',')
        {
            return Err(Error::InvalidNonce);
        }
        let iterations = iterations.ok_or(Error::InvalidServerMessage)?;
        if !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations) {
            return Err(Error::IterationLimit);
        }
        let salt = base64_decode(salt.ok_or(Error::InvalidServerMessage)?)?;
        if salt.is_empty() || salt.len() > MAX_SALT_BYTES {
            return Err(Error::InvalidServerMessage);
        }

        let mut final_without_proof = Zeroizing::new(Vec::with_capacity(9 + combined_nonce.len()));
        final_without_proof.extend_from_slice(b"c=biws,r=");
        final_without_proof.extend_from_slice(combined_nonce.as_bytes());

        let mut auth_message = Zeroizing::new(Vec::with_capacity(
            self.client_first_bare.len() + message.len() + final_without_proof.len() + 2,
        ));
        auth_message.extend_from_slice(&self.client_first_bare);
        auth_message.push(b',');
        auth_message.extend_from_slice(message);
        auth_message.push(b',');
        auth_message.extend_from_slice(&final_without_proof);

        let salted_password = Zeroizing::new(pbkdf2_sha256(&self.password, &salt, iterations));
        let client_key = Zeroizing::new(hmac_sha256(&salted_password[..], b"Client Key"));
        let stored_key = Zeroizing::new(digest(&client_key[..]));
        let client_signature = Zeroizing::new(hmac_sha256(&stored_key[..], &auth_message));
        let mut proof = Zeroizing::new([0u8; 32]);
        for index in 0..proof.len() {
            proof[index] = client_key[index] ^ client_signature[index];
        }
        let server_key = Zeroizing::new(hmac_sha256(&salted_password[..], b"Server Key"));
        self.expected_server_signature =
            Some(Zeroizing::new(hmac_sha256(&server_key[..], &auth_message)));

        let proof = Zeroizing::new(base64_encode(&proof[..], true));
        let mut client_final =
            Zeroizing::new(Vec::with_capacity(final_without_proof.len() + 3 + proof.len()));
        client_final.extend_from_slice(&final_without_proof);
        client_final.extend_from_slice(b",p=");
        client_final.extend_from_slice(proof.as_bytes());
        Ok(client_final)
    }

    pub fn receive_server_final(&mut self, message: &[u8]) -> Result<(), Error> {
        if message.is_empty() || message.len() > MAX_SERVER_MESSAGE_BYTES {
            return Err(Error::InvalidServerMessage);
        }
        let expected = self.expected_server_signature.take().ok_or(Error::InvalidServerMessage)?;
        let text = core::str::from_utf8(message).map_err(|_| Error::InvalidServerMessage)?;
        if text.starts_with("e=") {
            return Err(Error::ServerRejected);
        }
        let encoded = text.strip_prefix("v=").ok_or(Error::InvalidServerMessage)?;
        if encoded.contains(',') {
            return Err(Error::InvalidServerMessage);
        }
        let signature = Zeroizing::new(base64_decode(encoded)?);
        if signature.len() != expected.len() || !constant_time_eq(&signature, &expected[..]) {
            return Err(Error::SignatureMismatch);
        }
        Ok(())
    }
}

fn escape_username(username: &str) -> String {
    let mut escaped = String::with_capacity(username.len());
    for character in username.chars() {
        match character {
            ',' => escaped.push_str("=2C"),
            '=' => escaped.push_str("=3D"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut salt_block = Zeroizing::new(Vec::with_capacity(salt.len() + 4));
    salt_block.extend_from_slice(salt);
    salt_block.extend_from_slice(&1u32.to_be_bytes());
    let mut current = Zeroizing::new(hmac_sha256(password, &salt_block));
    let mut result = Zeroizing::new(*current);
    for _ in 1..iterations {
        *current = hmac_sha256(password, &current[..]);
        for index in 0..result.len() {
            result[index] ^= current[index];
        }
    }
    *result
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = Zeroizing::new([0u8; 64]);
    if key.len() > block.len() {
        let hashed_key = Zeroizing::new(digest(key));
        block[..32].copy_from_slice(&hashed_key[..]);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = Zeroizing::new([0x36u8; 64]);
    let mut outer_pad = Zeroizing::new([0x5cu8; 64]);
    for index in 0..block.len() {
        inner_pad[index] ^= block[index];
        outer_pad[index] ^= block[index];
    }
    let mut inner = Sha256::new();
    inner.update(&inner_pad[..]);
    inner.update(message);
    let inner_digest = Zeroizing::new(inner.finalize());
    let mut outer = Sha256::new();
    outer.update(&outer_pad[..]);
    outer.update(&inner_digest[..]);
    outer.finalize()
}

pub fn base64_encode(bytes: &[u8], padding: bool) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        result.push(ALPHABET[((value >> 18) & 0x3f) as usize] as char);
        result.push(ALPHABET[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((value >> 6) & 0x3f) as usize] as char);
        } else if padding {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(value & 0x3f) as usize] as char);
        } else if padding {
            result.push('=');
        }
    }
    result
}

fn base64_decode(value: &str) -> Result<Vec<u8>, Error> {
    if value.is_empty() || value.len() % 4 == 1 {
        return Err(Error::InvalidServerMessage);
    }
    let mut output = vec![];
    let bytes = value.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        let count = remaining.min(4);
        let mut digits = [0u8; 4];
        let mut padding = 0;
        for index in 0..count {
            if bytes[offset + index] == b'=' {
                if offset + count != bytes.len()
                    || !bytes[offset + index..offset + count].iter().all(|byte| *byte == b'=')
                {
                    return Err(Error::InvalidServerMessage);
                }
                padding = count - index;
                break;
            } else {
                digits[index] = base64_digit(bytes[offset + index])?;
            }
        }
        if count < 2 || padding > 2 || count < 4 && padding != 0 {
            return Err(Error::InvalidServerMessage);
        }
        let packed = u32::from(digits[0]) << 18
            | u32::from(digits[1]) << 12
            | u32::from(digits[2]) << 6
            | u32::from(digits[3]);
        output.push((packed >> 16) as u8);
        if count > 2 && padding < 2 {
            output.push((packed >> 8) as u8);
        }
        if count > 3 && padding == 0 {
            output.push(packed as u8);
        }
        offset += count;
    }
    Ok(output)
}

fn base64_digit(byte: u8) -> Result<u8, Error> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(Error::InvalidServerMessage),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip_and_padding() {
        for value in [b"f".as_slice(), b"fo", b"foo", b"SCRAM-SHA-256"] {
            let encoded = base64_encode(value, true);
            assert_eq!(base64_decode(&encoded).unwrap(), value);
        }
        assert_eq!(base64_encode(b"foo", false), "Zm9v");
    }

    #[test]
    fn scram_sha_256_matches_rfc_7677_exchange() {
        let mut client = Client::new("user", b"pencil", "rOprNGfwEbeRWgbNEkqO").unwrap();
        assert_eq!(&*client.client_first(), b"n,,n=user,r=rOprNGfwEbeRWgbNEkqO");
        let final_message = client
            .receive_server_first(
                b"r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096",
            )
            .unwrap();
        assert_eq!(
            &*final_message,
            b"c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        client.receive_server_final(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=").unwrap();
    }

    #[test]
    fn rejects_nonce_downgrade_and_excessive_work() {
        let mut client = Client::new("user", b"secret", "client").unwrap();
        assert_eq!(
            client.receive_server_first(b"r=other,s=c2FsdA==,i=4096"),
            Err(Error::InvalidNonce)
        );
        assert_eq!(
            client.receive_server_first(b"r=client-server,s=c2FsdA==,i=1000001"),
            Err(Error::IterationLimit)
        );

        let mut client = Client::new("user", b"secret", "client").unwrap();
        client.receive_server_first(b"r=client-server,s=c2FsdA==,i=4096").unwrap();
        assert_eq!(
            client.receive_server_final(&vec![b'x'; MAX_SERVER_MESSAGE_BYTES + 1]),
            Err(Error::InvalidServerMessage)
        );
    }
}

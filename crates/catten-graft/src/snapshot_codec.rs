use alloc::{
    format,
    string::String,
    vec::Vec,
};

use crate::types::{
    Peer,
    Role,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEnvelope {
    pub current_members: Vec<Peer>,
    pub next_members: Vec<Peer>,
    pub state_machine_snapshot: Vec<u8>,
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        }
    }
    out
}

fn base64_decode(value: &str) -> Option<Vec<u8>> {
    fn digit(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some((byte - b'A') as u32),
            b'a'..=b'z' => Some((byte - b'a' + 26) as u32),
            b'0'..=b'9' => Some((byte - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(value.len() * 3 / 4);
    for chunk in value.as_bytes().chunks(4) {
        if chunk.len() < 2 {
            return None;
        }
        let v0 = digit(chunk[0])?;
        let v1 = digit(chunk[1])?;
        let v2 = chunk.get(2).and_then(|byte| digit(*byte));
        let v3 = chunk.get(3).and_then(|byte| digit(*byte));
        out.push(((v0 << 2) | (v1 >> 4)) as u8);
        if let Some(v2) = v2 {
            out.push(((v1 << 4) | (v2 >> 2)) as u8);
            if let Some(v3) = v3 {
                out.push(((v2 << 6) | v3) as u8);
            }
        }
    }
    Some(out)
}

fn quote_json(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

/// Java/C++/Rust Graft-compatible membership snapshot envelope.
pub fn wrap_snapshot_payload(
    current_members: &[Peer],
    next_members: &[Peer],
    state_machine_snapshot: &[u8],
) -> Vec<u8> {
    let members_json = |members: &[Peer]| {
        members
            .iter()
            .map(|peer| {
                let role = if peer.is_voter() {
                    "VOTER"
                } else {
                    "LEARNER"
                };
                format!(
                    "{{\"id\":{},\"role\":\"{role}\",\"address\":{{\"host\":\"charlotte:{:016x}\",\
                     \"port\":0}}}}",
                    quote_json(&peer.id),
                    peer.service_name
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{{\"version\":1,\"currentMembers\":[{}],\"nextMembers\":[{}],\"stateMachineSnapshot\":\
         {}}}",
        members_json(current_members),
        members_json(next_members),
        quote_json(&base64_encode(state_machine_snapshot))
    )
    .into_bytes()
}

pub fn unwrap_snapshot_payload(payload: &[u8]) -> Vec<u8> {
    decode_snapshot_payload(payload)
        .map(|envelope| envelope.state_machine_snapshot)
        .unwrap_or_else(|| payload.to_vec())
}

pub fn decode_snapshot_payload(payload: &[u8]) -> Option<SnapshotEnvelope> {
    let Ok(value) = core::str::from_utf8(payload) else {
        return None;
    };
    Some(SnapshotEnvelope {
        current_members: member_ids(value, "currentMembers")?,
        next_members: member_ids(value, "nextMembers")?,
        state_machine_snapshot: base64_decode(string_field(value, "stateMachineSnapshot")?)?,
    })
}

fn member_ids(value: &str, field: &str) -> Option<Vec<Peer>> {
    let marker = format!("\"{field}\":[");
    let start = value.find(&marker)? + marker.len();
    let end = matching_array_end(value.as_bytes(), start)?;
    let array = &value[start..end];
    let mut members = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = array[cursor..].find("\"id\":") {
        cursor += relative + "\"id\":".len();
        let object_start = array[..cursor].rfind('{')?;
        let object_end = array[cursor..].find('}').map(|offset| cursor + offset)?;
        let object = &array[object_start..=object_end];
        let (id, consumed) = parse_json_string(&array[cursor..])?;
        let role = string_field(object, "role").unwrap_or("VOTER");
        let service_name = string_field(object, "host")
            .and_then(|host| host.strip_prefix("charlotte:"))
            .and_then(|value| u64::from_str_radix(value, 16).ok())
            .unwrap_or(0);
        members.push(Peer {
            id,
            service_name,
            role: if role.eq_ignore_ascii_case("LEARNER") {
                Role::Learner
            } else {
                Role::Voter
            },
        });
        cursor += consumed;
    }
    Some(members)
}

fn string_field<'a>(value: &'a str, field: &str) -> Option<&'a str> {
    let marker = format!("\"{field}\":");
    let start = value.find(&marker)? + marker.len();
    let rest = &value[start..];
    let (_, consumed) = parse_json_string(rest)?;
    Some(&rest[1..consumed - 1])
}

fn matching_array_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in bytes.get(start..)?.iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b']' {
            return Some(start + offset);
        }
    }
    None
}

fn parse_json_string(value: &str) -> Option<(String, usize)> {
    let bytes = value.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for (offset, byte) in bytes.iter().copied().enumerate().skip(1) {
        if escaped {
            out.push(match byte {
                b'"' => '"',
                b'\\' => '\\',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                _ => return None,
            });
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some((out, offset + 1));
        } else if byte.is_ascii_control() {
            return None;
        } else if byte.is_ascii() {
            out.push(byte as char);
        } else {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use alloc::{
        string::ToString,
        vec,
    };

    use super::{
        decode_snapshot_payload,
        wrap_snapshot_payload,
    };
    use crate::types::Peer;

    #[test]
    fn membership_and_application_state_round_trip() {
        let encoded = wrap_snapshot_payload(
            &[Peer::voter("n1".to_string(), 1), Peer::learner("n2".to_string(), 2)],
            &[Peer::voter("n2".to_string(), 2), Peer::voter("n3".to_string(), 3)],
            &[0, 1, 2, 255],
        );
        let decoded = decode_snapshot_payload(&encoded).unwrap();
        assert_eq!(decoded.current_members[0], Peer::voter("n1".to_string(), 1));
        assert_eq!(decoded.current_members[1], Peer::learner("n2".to_string(), 2));
        assert_eq!(decoded.next_members[1], Peer::voter("n3".to_string(), 3));
        assert_eq!(decoded.state_machine_snapshot, vec![0, 1, 2, 255]);
    }
}

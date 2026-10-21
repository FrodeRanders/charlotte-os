//! Decode deployd's current JSON records without substring matching.

fn trim(value: &str) -> &str {
    value.trim_matches([' ', '\t', '\r', '\n'])
}

// Require every current field once. Nested values, escaped keys, unknown fields
// and extra records cannot supply an alternative generation or ready marker.
fn fields<'a, const N: usize>(body: &'a str, names: [&str; N]) -> Option<[&'a str; N]> {
    let body = trim(body).strip_prefix('{')?.strip_suffix('}')?;
    let mut values = [""; N];
    let mut seen = [false; N];
    for field in body.split(',') {
        let (key, value) = field.split_once(':')?;
        let key = trim(key).strip_prefix('"')?.strip_suffix('"')?;
        let index = names.iter().position(|name| *name == key)?;
        if seen[index] {
            return None;
        }
        seen[index] = true;
        values[index] = trim(value);
    }
    seen.iter().all(|seen| *seen).then_some(values)
}

fn number(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

pub(super) fn admitted_generation(body: &str) -> Option<u64> {
    let [generation] = fields(body, ["generation"])?;
    number(generation).filter(|generation| *generation > 0 && *generation <= i64::MAX as u64)
}

/// The complete bounded receipt binds component order to the exact signed envelope.
pub(super) fn admitted_release(
    body: &str,
    envelope: &[u8],
) -> Option<charlotte_launch::release_admission::Receipt> {
    let [value] = fields(body, ["admission"])?;
    let hex = value.strip_prefix('"')?.strip_suffix('"')?;
    if hex.len() % 2 != 0 || hex.len() > 2 * charlotte_launch::release_admission::MAX_LEN {
        return None;
    }
    let mut bytes = [0; charlotte_launch::release_admission::MAX_LEN];
    fn digit(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        }
    }
    for (slot, pair) in bytes.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0.iter()) {
        *slot = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    let receipt = charlotte_launch::release_admission::Receipt::decode(&bytes[..hex.len() / 2])?;
    receipt.matches(envelope).then_some(receipt)
}

/// Structural observation of a committed policy, without a local UTC or
/// authority decision. Zero assignments is the current explicit clear policy.
pub(super) fn policy_status(body: &str) -> Option<()> {
    let [generation, sequence, assignments, start, end] = fields(
        body,
        [
            "generation",
            "sequence",
            "assignments",
            "not_before_unix_seconds",
            "expires_unix_seconds",
        ],
    )?;
    let generation = number(generation)?;
    let sequence = number(sequence)?;
    let assignments = number(assignments)?;
    let start = number(start)?;
    let end = number(end)?;
    (generation > 0
        && generation <= i64::MAX as u64
        && sequence > 0
        && assignments <= charlotte_launch::ingress::MAX_SERVICES as u64
        && start > 0
        && end.checked_sub(start)? <= charlotte_launch::ingress_policy::MAX_VALIDITY_SECONDS)
        .then_some(())
}

pub(super) struct Status {
    pub(super) generation: u64,
    pub(super) ready: bool,
}

pub(super) fn status(body: &str) -> Option<Status> {
    let [state, generation, service, node, desired, ready] = fields(
        body,
        [
            "state",
            "deployment_generation",
            "service_generation",
            "node_key",
            "desired_replicas",
            "ready_replicas",
        ],
    )?;
    let generation = number(generation)?;
    if generation == 0 || generation > i64::MAX as u64 {
        return None;
    }
    let service = number(service)?;
    number(node)?;
    let desired = u16::try_from(number(desired)?).ok()?;
    let ready = u16::try_from(number(ready)?).ok()?;
    if ready > desired {
        return None;
    }
    let complete = match state {
        "\"ready\"" if desired > 0 && ready == desired && service > 0 => true,
        "\"committed\"" if ready == 0 => false,
        "\"replacing\"" if ready > 0 && ready < desired => false,
        _ => return None,
    };
    Some(Status {
        generation,
        ready: complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: &str = concat!(
        "{\"state\":\"ready\",\"deployment_generation\":2,\"service_generation\":7,",
        "\"node_key\":1,\"desired_replicas\":1,\"ready_replicas\":1}"
    );

    #[test]
    fn policy_observation_accepts_current_bounds_without_utc_admission() {
        for assignments in [0, charlotte_launch::ingress::MAX_SERVICES] {
            let body = format!(
                "{{\"generation\":1,\"sequence\":{},\"assignments\":{assignments},\"\
                 not_before_unix_seconds\":1,\"expires_unix_seconds\":{}}}",
                u64::MAX,
                1 + charlotte_launch::ingress_policy::MAX_VALIDITY_SECONDS
            );
            assert!(policy_status(&body).is_some());
        }
    }
    #[test]
    fn policy_observation_rejects_ambiguous_and_invalid_fields() {
        let valid = concat!(
            r#"{"generation":2,"sequence":3,"assignments":1,"#,
            r#""not_before_unix_seconds":1,"expires_unix_seconds":301}"#
        );
        for body in [
            valid.replace("\"generation\":2", "\"generation\":0"),
            valid.replace("\"sequence\":3", "\"sequence\":0"),
            valid.replace("\"assignments\":1", "\"assignments\":17"),
            valid.replace("\"not_before_unix_seconds\":1", "\"not_before_unix_seconds\":0"),
            valid.replace("\"expires_unix_seconds\":301", "\"expires_unix_seconds\":0"),
            valid.replace(
                "\"expires_unix_seconds\":301",
                &format!(
                    "\"expires_unix_seconds\":{}",
                    2 + charlotte_launch::ingress_policy::MAX_VALIDITY_SECONDS
                ),
            ),
            valid.replace("\"generation\":2", "\"generation\":2,\"generation\":3"),
            valid.replace("\"assignments\":1", "\"extra\":1"),
            format!("{valid} trailing"),
        ] {
            assert!(policy_status(&body).is_none(), "{body}");
        }
    }

    #[test]
    fn admission_requires_one_positive_signed_result() {
        for input in ["{\"generation\":2}\n", " { \"generation\" : 2 } "] {
            assert_eq!(admitted_generation(input), Some(2));
        }
        for input in [
            "{}",
            "{\"generation\":0}",
            "{\"generation\":-1}",
            "{\"generation\":+2}",
            "{\"generation\":02}",
            "{\"generation\":9223372036854775808}",
            "{\"generation\":2,\"generation\":3}",
            "{\"generation\":2} trailing",
        ] {
            assert_eq!(admitted_generation(input), None, "{input}");
        }
    }

    #[test]
    fn ready_record_preserves_generation() {
        let result = status(READY).unwrap();
        assert_eq!(result.generation, 2);
        assert!(result.ready);
    }

    #[test]
    fn field_order_and_json_whitespace_are_accepted() {
        let result = status(concat!(
            " { \"ready_replicas\" : 1, \"desired_replicas\":1, \"node_key\":1,",
            "\"service_generation\":7,\"deployment_generation\":2,\"state\":\"ready\" }\n"
        ))
        .unwrap();
        assert!(result.ready);
    }

    #[test]
    fn pending_records_are_distinct_from_ready() {
        for input in [
            READY
                .replace("ready\"", "committed\"")
                .replace("\"ready_replicas\":1", "\"ready_replicas\":0"),
            READY
                .replace("ready\"", "replacing\"")
                .replace("\"desired_replicas\":1", "\"desired_replicas\":2"),
        ] {
            assert!(!status(&input).unwrap().ready);
        }
    }

    #[test]
    fn ambiguous_extended_and_incomplete_records_reject() {
        for input in [
            format!("{READY} trailing"),
            READY.replace(
                "\"deployment_generation\":2",
                "\"deployment_generation\":2,\"deployment_generation\":3",
            ),
            READY.replace("\"deployment_generation\":2,", ""),
            READY.replace("\"node_key\":1", "\"unknown\":1"),
            READY.replace("\"state\":\"ready\"", "\"state\":{\"state\":\"ready\"}"),
            READY.replace("\"node_key\":1", "\"node_key\":1.0"),
        ] {
            assert!(status(&input).is_none(), "{input}");
        }
    }

    #[test]
    fn invalid_ready_counts_generations_and_states_reject() {
        for input in [
            READY.replace("\"ready_replicas\":1", "\"ready_replicas\":0"),
            READY.replace("\"desired_replicas\":1", "\"desired_replicas\":0"),
            READY.replace("\"service_generation\":7", "\"service_generation\":0"),
            READY.replace("\"deployment_generation\":2", "\"deployment_generation\":0"),
            READY.replace("\"desired_replicas\":1", "\"desired_replicas\":65536"),
            READY.replace("\"state\":\"ready\"", "\"state\":\"unknown\""),
        ] {
            assert!(status(&input).is_none(), "{input}");
        }
    }
}

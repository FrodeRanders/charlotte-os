//! Decode deployd's current scalar JSON records without substring matching.

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

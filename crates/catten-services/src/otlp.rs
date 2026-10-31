//! Bounded OTLP/HTTP protobuf metrics. Diagnostic export never supplies placement input.

use alloc::{
    format,
    vec::Vec,
};

use crate::observability::{
    self,
    history_header as h,
    history_record as r,
};

pub const MAX_REQUEST: usize = 8192;
pub const MAX_RESPONSE: usize = 8192;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub ticks: u64,
    pub values: [u64; r::WORDS],
}

pub fn latest_history(bytes: &[u8]) -> Option<(u64, Sample, Option<Sample>)> {
    fn word(b: &[u8], i: usize) -> Option<u64> {
        Some(u64::from_le_bytes(
            b.get(i.checked_mul(8)?..i.checked_add(1)?.checked_mul(8)?)?.try_into().ok()?,
        ))
    }
    if word(bytes, h::MAGIC)? != observability::HISTORY_MAGIC
        || word(bytes, h::VERSION)? != observability::HISTORY_VERSION
        || word(bytes, h::RECORD_BYTES)? != (r::WORDS * 8) as u64
    {
        return None;
    }
    let n = usize::try_from(word(bytes, h::RECORD_COUNT)?).ok()?;
    let freq = word(bytes, h::COUNTER_FREQUENCY_HZ)?;
    if n == 0
        || n > observability::HISTORY_CAPACITY
        || freq == 0
        || bytes.len() != (h::WORDS + n * r::WORDS) * 8
    {
        return None;
    }
    let sample = |i| -> Option<Sample> {
        let mut values = [0; r::WORDS];
        for (j, v) in values.iter_mut().enumerate() {
            *v = word(bytes, h::WORDS + i * r::WORDS + j)?;
        }
        Some(Sample {
            ticks: values[r::MONOTONIC_TICKS],
            values,
        })
    };
    Some((
        freq,
        sample(n - 1)?,
        if n > 1 {
            Some(sample(n - 2)?)
        } else {
            None
        },
    ))
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 128 {
        out.push(v as u8 | 128);
        v >>= 7;
    }
    out.push(v as u8);
}
fn message(out: &mut Vec<u8>, field: u8, bytes: &[u8]) {
    varint(out, u64::from(field) * 8 + 2);
    varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}
fn attr(out: &mut Vec<u8>, name: &str, value: &str) {
    let mut kv = Vec::new();
    message(&mut kv, 1, name.as_bytes());
    let mut any = Vec::new();
    message(&mut any, 1, value.as_bytes());
    message(&mut kv, 2, &any);
    message(out, 1, &kv);
}

fn gauge(out: &mut Vec<u8>, name: &str, unit: &str, time: u64, value: u64, double: bool) {
    let mut point = Vec::new();
    point.push(3 * 8 + 1);
    point.extend_from_slice(&time.to_le_bytes());
    point.push(
        if double {
            4 * 8 + 1
        } else {
            6 * 8 + 1
        },
    );
    point.extend_from_slice(&value.to_le_bytes());
    let mut gauge = Vec::new();
    message(&mut gauge, 1, &point);
    let mut metric = Vec::new();
    message(&mut metric, 1, name.as_bytes());
    message(&mut metric, 3, unit.as_bytes());
    message(&mut metric, 5, &gauge);
    message(out, 2, &metric);
}

/// Gauge timestamps describe the sampled instant, not the export attempt.
pub fn metrics(
    instance: &str,
    unix_ns: u64,
    frequency: u64,
    current: Sample,
    previous: Option<Sample>,
) -> Option<Vec<u8>> {
    if unix_ns == 0 || frequency == 0 || instance.is_empty() || instance.len() > 64 {
        return None;
    }
    let mut resource = Vec::new();
    attr(&mut resource, "service.name", "charlotte-os");
    attr(&mut resource, "service.instance.id", instance);
    let mut scope = Vec::new();
    let mut instrumentation = Vec::new();
    message(&mut instrumentation, 1, b"charlotte.telemetry");
    message(&mut scope, 1, &instrumentation);
    for (index, name, unit) in [
        (r::THREADS, "charlotte.threads", "{thread}"),
        (r::DOMAINS, "charlotte.domains", "{domain}"),
        (r::FREE_FRAMES, "charlotte.memory.frames.free", "{frame}"),
        (r::USABLE_FRAMES, "charlotte.memory.frames.usable", "{frame}"),
        (r::OWNED_FRAMES, "charlotte.memory.frames.owned", "{frame}"),
        (r::HEAP_ALLOCATED_BYTES, "charlotte.heap.allocated", "By"),
        (r::HEAP_PEAK_BYTES, "charlotte.heap.peak", "By"),
        (r::STACK_PAGES, "charlotte.stack.reserved", "{page}"),
        (r::STACK_USED_HIGH_WATER, "charlotte.stack.used_high_water", "{page}"),
        (r::THREADS_HIGH_WATER, "charlotte.threads.high_water", "{thread}"),
        (r::LOGICAL_PROCESSORS, "charlotte.cpu.logical_processors", "{cpu}"),
    ] {
        let value = current.values[index];
        if value > i64::MAX as u64 {
            return None;
        }
        gauge(&mut scope, name, unit, unix_ns, value, false);
    }
    if let Some(old) = previous
        && current.ticks > old.ticks
        && current.values[r::CPU_BUSY_TICKS] >= old.values[r::CPU_BUSY_TICKS]
        && current.values[r::LOGICAL_PROCESSORS] > 0
    {
        let busy = (current.values[r::CPU_BUSY_TICKS] - old.values[r::CPU_BUSY_TICKS]) as f64;
        let span =
            (current.ticks - old.ticks) as f64 * current.values[r::LOGICAL_PROCESSORS] as f64;
        gauge(
            &mut scope,
            "charlotte.cpu.utilization",
            "1",
            unix_ns,
            (busy / span).clamp(0.0, 1.0).to_bits(),
            true,
        );
    }
    let mut rm = Vec::new();
    message(&mut rm, 1, &resource);
    message(&mut rm, 2, &scope);
    let mut request = Vec::new();
    message(&mut request, 1, &rm);
    (request.len() <= MAX_REQUEST).then_some(request)
}

pub fn request(profile: &charlotte_launch::otlp::Profile, body: &[u8]) -> Option<Vec<u8>> {
    if !profile.valid() || body.len() > MAX_REQUEST {
        return None;
    }
    let [a, b, c, d] = profile.address;
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {a}.{b}.{c}.{d}:{}\r\nContent-Type: \
         application/x-protobuf\r\nAccept: application/x-protobuf\r\nContent-Length: \
         {}\r\nConnection: close\r\n\r\n",
        profile.path,
        profile.port,
        body.len()
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    (out.len() <= MAX_REQUEST).then_some(out)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Acknowledgement {
    Full,
    Partial {
        rejected: u64,
    },
}

fn take_varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut v = 0;
    for shift in (0..70).step_by(7) {
        let (&b, rest) = bytes.split_first()?;
        *bytes = rest;
        if shift == 63 && b > 1 {
            return None;
        }
        v |= u64::from(b & 127) << shift;
        if b < 128 {
            return Some(v);
        }
    }
    None
}
fn take_field<'a>(bytes: &mut &'a [u8]) -> Option<(u64, &'a [u8])> {
    let tag = take_varint(bytes)?;
    if tag >> 3 == 0 {
        return None;
    }
    let len = match tag & 7 {
        0 => {
            let original = *bytes;
            take_varint(bytes)?;
            return Some((tag, &original[..original.len() - bytes.len()]));
        }
        1 => 8,
        2 => usize::try_from(take_varint(bytes)?).ok()?,
        5 => 4,
        _ => return None,
    };
    let (value, rest) = bytes.split_at_checked(len)?;
    *bytes = rest;
    Some((tag, value))
}

/// Empty initialized partial-success messages carry default success (including
/// Collector 0.164). A nonempty warning or rejection is terminal, never replayed.
pub fn acknowledge(mut body: &[u8]) -> Option<Acknowledgement> {
    if body.len() > MAX_RESPONSE {
        return None;
    }
    let mut ack = Acknowledgement::Full;
    let mut partial_seen = false;
    while !body.is_empty() {
        let (tag, mut value) = take_field(&mut body)?;
        if tag >> 3 == 1 {
            if tag & 7 != 2 || partial_seen {
                return None;
            }
            partial_seen = true;
            let mut rejected = 0;
            let mut warning = false;
            while !value.is_empty() {
                let (t, mut field) = take_field(&mut value)?;
                if t >> 3 == 1 {
                    if t & 7 != 0 {
                        return None;
                    }
                    rejected = take_varint(&mut field)?;
                    if rejected > i64::MAX as u64 {
                        return None;
                    }
                } else if t >> 3 == 2 {
                    if t & 7 != 2 || core::str::from_utf8(field).is_err() {
                        return None;
                    }
                    warning |= !field.is_empty();
                }
            }
            ack = if rejected == 0 && !warning {
                Acknowledgement::Full
            } else {
                Acknowledgement::Partial {
                    rejected,
                }
            };
        }
    }
    Some(ack)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_success_is_terminal_and_malformed_replies_reject() {
        assert_eq!(acknowledge(&[]), Some(Acknowledgement::Full));
        assert_eq!(
            acknowledge(&[10, 2, 8, 3]),
            Some(Acknowledgement::Partial {
                rejected: 3
            })
        );
        assert_eq!(acknowledge(&[10, 0]), Some(Acknowledgement::Full));
        assert_eq!(
            acknowledge(&[10, 4, 18, 2, b'o', b'k']),
            Some(Acknowledgement::Partial {
                rejected: 0
            })
        );
        for bad in [&[10, 3, 8, 3][..], &[0], &[10, 0, 10, 0], &[10, 1, 8]] {
            assert!(acknowledge(bad).is_none());
        }
    }
    #[test]
    fn metric_values_and_timestamp_use_otlp_fixed64_fields() {
        let mut sample = Sample {
            ticks: 20,
            values: [1; r::WORDS],
        };
        sample.values[r::CPU_BUSY_TICKS] = 10;
        let bytes = metrics("node", 123456, 1000, sample, None).unwrap();
        assert!(
            bytes
                .windows(b"charlotte.heap.allocated".len())
                .any(|b| b == b"charlotte.heap.allocated")
        );
        let mut timestamp = alloc::vec![25];
        timestamp.extend_from_slice(&123456u64.to_le_bytes());
        assert!(bytes.windows(timestamp.len()).any(|b| b == timestamp));
        sample.values[r::THREADS] = u64::MAX;
        assert!(metrics("node", 1, 1000, sample, None).is_none());
    }
}

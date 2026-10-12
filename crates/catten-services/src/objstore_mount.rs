//! Superblock selection: unreadable metadata is never evidence of a blank disk.

pub fn select<T, E>(
    mut read: impl FnMut(u64) -> Result<Option<T>, E>,
    choose: impl FnOnce(T, T) -> T,
) -> Result<Option<T>, E> {
    let first = read(0)?;
    let second = read(1)?;
    Ok(match (first, second) {
        (Some(a), Some(b)) => Some(choose(a, b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    })
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    #[test]
    fn rejection_never_selects_formatting_or_a_partial_candidate() {
        for failure in ["allocation", "submission", "observation", "device", "mapping"] {
            for failed_slot in [0, 1] {
                let calls = Cell::new(0);
                let result = select(
                    |slot| {
                        calls.set(calls.get() + 1);
                        if slot == failed_slot {
                            Err(failure)
                        } else {
                            Ok(Some(7))
                        }
                    },
                    |_, _| panic!("must not select after unreadable metadata"),
                );
                assert_eq!(result, Err(failure));
                assert_eq!(calls.get(), failed_slot + 1);
            }
        }
    }

    #[test]
    fn successfully_read_candidates_preserve_redundancy_and_blank_selection() {
        for (first, second, expected) in [
            (None, None, None),
            (Some(1), None, Some(1)),
            (None, Some(2), Some(2)),
            (Some(1), Some(2), Some(2)),
            (Some(3), Some(2), Some(3)),
        ] {
            let result = select(
                |slot| {
                    Ok::<_, ()>(
                        if slot == 0 {
                            first
                        } else {
                            second
                        },
                    )
                },
                core::cmp::max,
            );
            assert_eq!(result, Ok(expected));
        }
    }
}

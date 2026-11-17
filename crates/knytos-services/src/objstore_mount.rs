//! Only two successfully read, zero-filled slots permit first-boot formatting.

pub enum Slot<T> {
    Blank,
    Valid(T),
    Corrupt,
    Unsupported,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Error<E> {
    Read(E),
    Unsupported,
    Corrupt,
}

pub fn select<T, E>(
    mut read: impl FnMut(u64) -> Result<Slot<T>, E>,
    choose: impl FnOnce(T, T) -> T,
) -> Result<Option<T>, Error<E>> {
    use Slot::*;
    let first = read(0).map_err(Error::Read)?;
    let second = read(1).map_err(Error::Read)?;
    Ok(match (first, second) {
        (Unsupported, _) | (_, Unsupported) => return Err(Error::Unsupported),
        (Valid(a), Valid(b)) => Some(choose(a, b)),
        (Valid(a), _) | (_, Valid(a)) => Some(a),
        (Blank, Blank) => None,
        _ => return Err(Error::Corrupt),
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
                            Ok(Slot::Valid(7))
                        }
                    },
                    |_, _| panic!("must not select after unreadable metadata"),
                );
                assert_eq!(result, Err(Error::Read(failure)));
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
                            first.map_or(Slot::Blank, Slot::Valid)
                        } else {
                            second.map_or(Slot::Blank, Slot::Valid)
                        },
                    )
                },
                core::cmp::max,
            );
            assert_eq!(result, Ok(expected));
        }
    }

    #[test]
    fn unsupported_or_nonblank_without_a_current_copy_never_formats() {
        for (a, b, expected) in [
            (Slot::Unsupported, Slot::Valid(7), Err(Error::Unsupported)),
            (Slot::Valid(7), Slot::Unsupported, Err(Error::Unsupported)),
            (Slot::Corrupt, Slot::Blank, Err(Error::Corrupt)),
            (Slot::Blank, Slot::Corrupt, Err(Error::Corrupt)),
            (Slot::Corrupt, Slot::Corrupt, Err(Error::Corrupt)),
            (Slot::Corrupt, Slot::Valid(7), Ok(Some(7))),
            (Slot::Valid(7), Slot::Corrupt, Ok(Some(7))),
        ] {
            let mut slots = [a, b].into_iter();
            assert_eq!(select(|_| Ok::<_, ()>(slots.next().unwrap()), core::cmp::max), expected);
        }
    }
}

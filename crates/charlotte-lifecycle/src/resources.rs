//! Checked admission counters. Callers serialize reservations and retain their
//! charge until the resource is actually released, not merely transferred.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Amount {
    pub pages: u64,
    pub objects: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Limit,
    Underflow,
}

/// Atomic admission across independently measured resource dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VectorBudget<const N: usize> {
    limit: [u64; N],
    used: [u64; N],
}

impl<const N: usize> VectorBudget<N> {
    pub const fn new(limit: [u64; N]) -> Self {
        Self {
            limit,
            used: [0; N],
        }
    }

    pub const fn used(&self) -> [u64; N] {
        self.used
    }

    pub fn reserve(&mut self, amount: [u64; N]) -> Result<(), Error> {
        let mut next = self.used;
        for ((used, increment), limit) in next.iter_mut().zip(amount).zip(self.limit) {
            *used = used.checked_add(increment).ok_or(Error::Limit)?;
            if *used > limit {
                return Err(Error::Limit);
            }
        }
        self.used = next;
        Ok(())
    }

    pub fn release(&mut self, amount: [u64; N]) -> Result<(), Error> {
        let mut next = self.used;
        for (used, decrement) in next.iter_mut().zip(amount) {
            *used = used.checked_sub(decrement).ok_or(Error::Underflow)?;
        }
        self.used = next;
        Ok(())
    }
}

/// One-dimensional admission for records/events rather than backing pages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CountBudget {
    limit: usize,
    used: usize,
}

impl CountBudget {
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            used: 0,
        }
    }

    pub const fn used(&self) -> usize {
        self.used
    }

    pub fn reserve(&mut self) -> Result<(), Error> {
        let next = self.used.checked_add(1).ok_or(Error::Limit)?;
        if next > self.limit {
            return Err(Error::Limit);
        }
        self.used = next;
        Ok(())
    }

    pub fn release(&mut self) -> Result<(), Error> {
        self.used = self.used.checked_sub(1).ok_or(Error::Underflow)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budget {
    limit: Amount,
    used: Amount,
}

impl Budget {
    pub const fn new(limit: Amount) -> Self {
        Self {
            limit,
            used: Amount {
                pages: 0,
                objects: 0,
            },
        }
    }

    pub const fn used(&self) -> Amount {
        self.used
    }

    pub const fn limit(&self) -> Amount {
        self.limit
    }

    pub fn reserve(&mut self, amount: Amount) -> Result<(), Error> {
        let pages = self.used.pages.checked_add(amount.pages).ok_or(Error::Limit)?;
        let objects = self.used.objects.checked_add(amount.objects).ok_or(Error::Limit)?;
        if pages > self.limit.pages || objects > self.limit.objects {
            return Err(Error::Limit);
        }
        self.used = Amount {
            pages,
            objects,
        };
        Ok(())
    }

    pub fn release(&mut self, amount: Amount) -> Result<(), Error> {
        let pages = self.used.pages.checked_sub(amount.pages).ok_or(Error::Underflow)?;
        let objects = self.used.objects.checked_sub(amount.objects).ok_or(Error::Underflow)?;
        self.used = Amount {
            pages,
            objects,
        };
        Ok(())
    }

    pub fn set_limit(&mut self, limit: Amount) -> Result<(), Error> {
        if self.used.pages > limit.pages || self.used.objects > limit.objects {
            return Err(Error::Limit);
        }
        self.limit = limit;
        Ok(())
    }
}

/// Keep one eighth of usable physical frames beyond reach of memory-object
/// allocations. The allocator must check this while holding its frame lock.
/// This is a progress reserve, not an entitlement for every other subsystem.
pub const fn frames_available(free: u64, usable: u64, request: u64) -> bool {
    let reserve = if usable / 8 == 0 {
        1
    } else {
        usable / 8
    };
    match free.checked_sub(request) {
        Some(remaining) => remaining >= reserve,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_reservations_and_releases_are_atomic() {
        let mut budget = VectorBudget::new([2, 8, u64::MAX]);
        budget.reserve([1, 4, 1]).unwrap();
        let before = budget;
        for amount in [[2, 0, 0], [0, 5, 0], [0, 0, u64::MAX]] {
            assert_eq!(budget.reserve(amount), Err(Error::Limit));
            assert_eq!(budget, before);
        }
        assert_eq!(budget.release([1, 0, 2]), Err(Error::Underflow));
        assert_eq!(budget, before);
        budget.release([1, 4, 1]).unwrap();
        budget.reserve([2, 8, u64::MAX]).unwrap();
        budget.release([2, 8, u64::MAX]).unwrap();
        assert_eq!(budget.used(), [0; 3]);
    }

    #[test]
    fn event_count_rejection_and_release_are_atomic() {
        let mut budget = CountBudget::new(2);
        budget.reserve().unwrap();
        budget.reserve().unwrap();
        let full = budget;
        assert_eq!(budget.reserve(), Err(Error::Limit));
        assert_eq!(budget, full);
        budget.release().unwrap();
        budget.reserve().unwrap();
        budget.release().unwrap();
        budget.release().unwrap();
        let empty = budget;
        assert_eq!(budget.release(), Err(Error::Underflow));
        assert_eq!(budget, empty);
        assert_eq!(budget.used(), 0);
        let mut overflowing = CountBudget {
            limit: usize::MAX,
            used: usize::MAX,
        };
        assert_eq!(overflowing.reserve(), Err(Error::Limit));
        assert_eq!(overflowing.used(), usize::MAX);
    }

    #[test]
    fn rejection_is_atomic_for_each_dimension_and_overflow() {
        let mut budget = Budget::new(Amount {
            pages: 8,
            objects: 2,
        });
        budget
            .reserve(Amount {
                pages: 4,
                objects: 1,
            })
            .unwrap();
        let before = budget;
        for amount in [
            Amount {
                pages: 5,
                objects: 0,
            },
            Amount {
                pages: 0,
                objects: 2,
            },
            Amount {
                pages: u64::MAX,
                objects: 1,
            },
            Amount {
                pages: 0,
                objects: u64::MAX,
            },
        ] {
            assert_eq!(budget.reserve(amount), Err(Error::Limit));
            assert_eq!(budget, before);
        }
        budget
            .reserve(Amount {
                pages: 4,
                objects: 1,
            })
            .unwrap();
        assert_eq!(budget.used(), budget.limit());
    }

    #[test]
    fn release_and_limit_changes_are_checked_and_reusable() {
        let mut budget = Budget::new(Amount {
            pages: 8,
            objects: 2,
        });
        let amount = Amount {
            pages: 4,
            objects: 1,
        };
        budget.reserve(amount).unwrap();
        let before = budget;
        assert_eq!(
            budget.release(Amount {
                pages: 0,
                objects: 2
            }),
            Err(Error::Underflow)
        );
        assert_eq!(budget, before);
        assert_eq!(
            budget.set_limit(Amount {
                pages: 3,
                objects: 2
            }),
            Err(Error::Limit)
        );
        assert_eq!(budget, before);
        budget.release(amount).unwrap();
        budget.reserve(budget.limit()).unwrap();
        budget.release(budget.limit()).unwrap();
        assert_eq!(budget.used(), Amount::default());
    }

    #[test]
    fn reserve_pool_cannot_be_consumed_by_ordinary_admission() {
        let mut total = Budget::new(Amount {
            pages: 16,
            objects: 8,
        });
        let mut ordinary = Budget::new(Amount {
            pages: 12,
            objects: 6,
        });
        let amount = ordinary.limit();
        ordinary.reserve(amount).unwrap();
        total.reserve(amount).unwrap();
        assert_eq!(
            ordinary.reserve(Amount {
                pages: 1,
                objects: 0
            }),
            Err(Error::Limit)
        );
        total
            .reserve(Amount {
                pages: 4,
                objects: 2,
            })
            .unwrap();
        assert_eq!(total.used(), total.limit());
    }

    #[test]
    fn physical_progress_reserve_includes_boundary_and_underflow() {
        assert!(frames_available(20, 80, 10));
        assert!(!frames_available(20, 80, 11));
        assert!(!frames_available(20, 80, 21));
        assert!(!frames_available(0, 0, 0));
        assert!(frames_available(1, 0, 0));
    }
}

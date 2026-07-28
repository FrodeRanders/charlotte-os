pub fn nearest_multiple_of<T>(value: T, multiple: T) -> T
where
    T: From<u64> + Into<u64> + Copy,
{
    let value: u64 = value.into();
    let multiple: u64 = multiple.into();
    match value.saturating_add(multiple / 2).checked_div(multiple) {
        Some(quotient) => T::from(quotient.saturating_mul(multiple)),
        None => T::from(value),
    }
}

use super::*;

#[test]
fn cycles_diamonds_empty_revisits_and_out_of_range_buses_are_bounded() {
    let mut buses = Buses::new(10, 13);
    buses.discover(9);
    buses.discover(14);
    let mut order = Vec::new();
    while let Some(bus) = buses.next() {
        order.push(bus);
        match bus {
            10 => {
                buses.discover(11);
                buses.discover(12)
            }
            11 | 12 => buses.discover(13),
            13 => buses.discover(10),
            _ => unreachable!(),
        }
    }
    assert_eq!(order, vec![10, 12, 13, 11]);
    buses.discover(11);
    assert_eq!(buses.next(), None);
}

#[test]
fn all_256_buses_fit_once_and_deep_chain_does_not_need_recursion() {
    let mut buses = Buses::new(0, 255);
    for number in 0..=255 {
        buses.discover(number);
    }
    for number in (0..=255).rev() {
        assert_eq!(buses.next(), Some(number));
        buses.discover(number);
    }
    assert_eq!(buses.next(), None);
    let mut buses = Buses::new(0, 255);
    let mut count = 0;
    while let Some(number) = buses.next() {
        count += 1;
        if number < 255 {
            buses.discover(number + 1);
        }
        buses.discover(0);
    }
    assert_eq!(count, 256);
}

#[test]
fn exact_function_and_device_limits_reject_without_wrapping_or_reset() {
    let mut budget = Budget::new();
    for _ in 0..MAX_FUNCTIONS {
        budget.accept_device().unwrap();
        budget.accept_function().unwrap();
    }
    for _ in 0..2 {
        assert_eq!(budget.accept_device(), Err(Error::DeviceCapacity));
        assert_eq!(budget.accept_function(), Err(Error::FunctionCapacity));
    }
}

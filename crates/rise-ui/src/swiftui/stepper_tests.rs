use crate::swiftui::stepper::stepped_value;

#[test]
fn a_press_moves_the_value_by_one() {
    assert_eq!(stepped_value(4, 1, &(0..=10)), Some(5));
    assert_eq!(stepped_value(4, -1, &(0..=10)), Some(3));
}

#[test]
fn the_ends_of_the_range_disable_their_segment() {
    assert_eq!(stepped_value(0, -1, &(0..=10)), None);
    assert_eq!(stepped_value(10, 1, &(0..=10)), None);
}

#[test]
fn a_value_outside_a_narrowed_range_comes_back_into_it() {
    assert_eq!(stepped_value(12, -1, &(0..=8)), Some(8));
}

#[test]
fn typed_digits_clamp_to_the_range_and_overflow_is_ignored() {
    use crate::swiftui::stepper::typed_value;

    assert_eq!(typed_value("7", &(0..=10)), Some(7));
    assert_eq!(typed_value("64", &(16..=32)), Some(32));
    assert_eq!(typed_value("2", &(16..=32)), Some(16));
    assert_eq!(typed_value("99999999999999999999", &(0..=10)), None);
}

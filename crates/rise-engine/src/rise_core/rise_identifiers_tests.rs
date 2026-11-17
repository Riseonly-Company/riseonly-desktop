use rise_canon::rise_rpc::REQUEST_ID_SEQUENCE_BITS as SEQUENCE_BITS;
use rise_core::foundation::foundation_date::Date;

use crate::rise_core::rise_identifiers::{
    RiseRequestID, RiseRequestIDAllocator, RiseRequestIDOutOfRange,
};

#[test]
fn the_first_id_of_a_millisecond_has_sequence_one() {
    let allocator = RiseRequestIDAllocator::new();
    let id = allocator.next(Date::from_ms(1_000));
    assert_eq!(id.raw_value(), (1_000 << SEQUENCE_BITS) | 1);
    let next = allocator.next(Date::from_ms(1_000));
    assert_eq!(next.raw_value(), (1_000 << SEQUENCE_BITS) | 2);
}

#[test]
fn a_backwards_clock_never_regresses_the_id() {
    let allocator = RiseRequestIDAllocator::new();
    let first = allocator.next(Date::from_ms(10_000));
    let second = allocator.next(Date::from_ms(5_000));
    let third = allocator.next(Date::from_ms(-5));
    assert!(first < second && second < third);
}

#[test]
fn the_timestamp_seconds_come_back_out_of_the_id() {
    let allocator = RiseRequestIDAllocator::new();
    let id = allocator.next(Date::from_ms(1_700_000_123_456));
    assert_eq!(RiseRequestIDAllocator::timestamp_seconds(id), 1_700_000_123);
}

#[test]
fn a_clock_before_1970_still_issues_a_valid_id() {
    let allocator = RiseRequestIDAllocator::new();
    assert_eq!(allocator.next(Date::from_ms(-1)).raw_value(), 1);
}

#[test]
fn the_shared_allocator_is_one_instance() {
    assert!(std::ptr::eq(RiseRequestIDAllocator::shared(), RiseRequestIDAllocator::shared()));
}

#[test]
fn description_is_the_raw_value() {
    assert_eq!(RiseRequestID::new(42).map(|id| id.to_string()).as_deref(), Some("42"));
}

#[test]
fn coding_is_the_bare_number_and_zero_is_refused() {
    let id = RiseRequestID::new(1_048_577).unwrap();
    assert_eq!(serde_json::to_string(&id).unwrap(), "1048577");
    assert_eq!(serde_json::from_str::<RiseRequestID>("1048577").unwrap(), id);
    assert!(serde_json::from_str::<RiseRequestID>("0").is_err());
    assert_eq!(RiseRequestID::try_from(0), Err(RiseRequestIDOutOfRange(0)));
}

#[test]
fn a_zero_raw_value_is_refused_where_the_precondition_traps() {
    assert_eq!(RiseRequestID::new(0), None);
    assert_eq!(RiseRequestID::new(1).map(|id| id.raw_value()), Some(1));
}

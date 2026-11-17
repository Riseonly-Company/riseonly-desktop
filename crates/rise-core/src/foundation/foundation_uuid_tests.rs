use super::foundation_uuid::Uuid;

#[test]
fn uuid_string_is_upper_case() {
    let uuid = Uuid::from_bytes([
        0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
    ]);
    assert_eq!(uuid.uuid_string(), "6BA7B811-9DAD-11D1-80B4-00C04FD430C8");
    assert_eq!(uuid.to_string(), uuid.uuid_string());
}

#[test]
fn new_uuid_is_version_four_upper_case() {
    let uuid = Uuid::new_v4();
    let text = uuid.uuid_string();
    assert_eq!(text.len(), 36);
    assert_eq!(text, text.to_uppercase());
    assert_eq!(uuid.uuid()[6] >> 4, 4);
    assert_eq!(uuid.uuid()[8] & 0xc0, 0x80);
    assert_ne!(Uuid::new_v4(), uuid);
}

#[test]
fn parses_either_case() {
    let upper = Uuid::from_uuid_string("6BA7B811-9DAD-11D1-80B4-00C04FD430C8");
    let lower = Uuid::from_uuid_string("6ba7b811-9dad-11d1-80b4-00c04fd430c8");
    assert!(upper.is_some());
    assert_eq!(upper, lower);
}

#[test]
fn rejects_malformed_strings() {
    assert_eq!(Uuid::from_uuid_string(""), None);
    assert_eq!(Uuid::from_uuid_string("6BA7B8119DAD11D180B400C04FD430C8"), None);
    assert_eq!(Uuid::from_uuid_string("{6BA7B811-9DAD-11D1-80B4-00C04FD430C8}"), None);
    assert_eq!(Uuid::from_uuid_string("6BA7B811-9DAD-11D1-80B4-00C04FD430CG"), None);
    assert_eq!(Uuid::from_uuid_string("6BA7B811-9DAD-11D1-80B4_00C04FD430C8"), None);
    assert_eq!(Uuid::from_uuid_string(" 6BA7B811-9DAD-11D1-80B4-00C04FD430C"), None);
}

#[test]
fn round_trips_through_uuid_string() {
    let uuid = Uuid::new_v4();
    assert_eq!(Uuid::from_uuid_string(&uuid.uuid_string()), Some(uuid));
}

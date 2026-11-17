use super::foundation_data::{
    base64_encoded_string, data_base64_encoded, data_base64_encoded_ignoring_unknown_characters,
};

#[test]
fn decodes_padded_standard_base64() {
    assert_eq!(data_base64_encoded("aGVsbG8="), Some(b"hello".to_vec()));
    assert_eq!(data_base64_encoded("aGk="), Some(b"hi".to_vec()));
    assert_eq!(data_base64_encoded("+/8="), Some(vec![0xfb, 0xff]));
}

#[test]
fn empty_string_is_empty_data() {
    assert_eq!(data_base64_encoded(""), Some(Vec::new()));
}

#[test]
fn rejects_whitespace_and_line_breaks() {
    assert_eq!(data_base64_encoded("aGVs bG8="), None);
    assert_eq!(data_base64_encoded("aGVs\nbG8="), None);
    assert_eq!(data_base64_encoded(" aGVsbG8="), None);
}

#[test]
fn rejects_missing_padding_and_url_alphabet() {
    assert_eq!(data_base64_encoded("aGVsbG8"), None);
    assert_eq!(data_base64_encoded("-_8="), None);
    assert_eq!(data_base64_encoded("aGk=="), None);
}

#[test]
fn ignore_unknown_characters_skips_whitespace() {
    assert_eq!(data_base64_encoded_ignoring_unknown_characters("aGVs bG8=\n"), Some(b"hello".to_vec()));
    assert_eq!(data_base64_encoded_ignoring_unknown_characters("aG*Vs\tbG8="), Some(b"hello".to_vec()));
}

#[test]
fn encodes_padded_without_line_breaks() {
    assert_eq!(base64_encoded_string(b"hello"), "aGVsbG8=");
    assert_eq!(base64_encoded_string(&[0xfb, 0xff]), "+/8=");
    assert_eq!(base64_encoded_string(b""), "");
    let long = vec![0u8; 120];
    assert!(!base64_encoded_string(&long).contains('\n'));
}

#[test]
fn round_trips_bytes() {
    let bytes: Vec<u8> = (0u8..=255).collect();
    assert_eq!(data_base64_encoded(&base64_encoded_string(&bytes)), Some(bytes));
}

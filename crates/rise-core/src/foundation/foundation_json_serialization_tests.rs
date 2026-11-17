use serde_json::{json, Value};

use super::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};

fn write(value: &Value, options: JSONWritingOptions) -> String {
    let data = JSONSerialization::data_with_json_object(value, options).expect("a writable value");
    String::from_utf8(data).expect("UTF-8 output")
}

const PRETTY: JSONWritingOptions = JSONWritingOptions {
    sorted_keys: false,
    pretty_printed: true,
    without_escaping_slashes: false,
    fragments_allowed: false,
};

const SORTED: JSONWritingOptions = JSONWritingOptions {
    sorted_keys: true,
    pretty_printed: false,
    without_escaping_slashes: false,
    fragments_allowed: false,
};

#[test]
fn reads_objects_and_arrays_only() {
    assert_eq!(JSONSerialization::json_object(br#"{"a":1}"#), Some(json!({ "a": 1 })));
    assert_eq!(JSONSerialization::json_object(b" [1, 2] \n"), Some(json!([1, 2])));
    assert_eq!(JSONSerialization::json_object(b"1"), None);
    assert_eq!(JSONSerialization::json_object(br#""text""#), None);
    assert_eq!(JSONSerialization::json_object(b"null"), None);
    assert_eq!(JSONSerialization::json_object(b"{"), None);
    assert_eq!(JSONSerialization::json_object(b""), None);
}

#[test]
fn reads_bom_and_utf16_input() {
    assert_eq!(JSONSerialization::json_object(b"\xEF\xBB\xBF[1]"), Some(json!([1])));
    let utf16_le: Vec<u8> = "[true]".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(JSONSerialization::json_object(&utf16_le), Some(json!([true])));
    let mut utf16_be = vec![0xFE, 0xFF];
    utf16_be.extend("[false]".encode_utf16().flat_map(u16::to_be_bytes));
    assert_eq!(JSONSerialization::json_object(&utf16_be), Some(json!([false])));
    let utf32_le: Vec<u8> = "[2]".chars().flat_map(|c| u32::from(c).to_le_bytes()).collect();
    assert_eq!(JSONSerialization::json_object(&utf32_le), Some(json!([2])));
}

#[test]
fn rejects_invalid_utf8() {
    assert_eq!(JSONSerialization::json_object(b"[\"\xFF\"]"), None);
}

#[test]
fn valid_json_object_is_a_container() {
    assert!(JSONSerialization::is_valid_json_object(&json!({})));
    assert!(JSONSerialization::is_valid_json_object(&json!([])));
    assert!(!JSONSerialization::is_valid_json_object(&json!(1)));
    assert!(!JSONSerialization::is_valid_json_object(&json!("a")));
    assert!(!JSONSerialization::is_valid_json_object(&Value::Null));
}

#[test]
fn fragments_need_the_option() {
    assert_eq!(JSONSerialization::data_with_json_object(&json!(1), JSONWritingOptions::default()), None);
    let options = JSONWritingOptions { fragments_allowed: true, ..JSONWritingOptions::default() };
    assert_eq!(write(&json!("a"), options), "\"a\"");
    assert_eq!(write(&Value::Null, options), "null");
    assert_eq!(write(&json!(true), options), "true");
}

#[test]
fn compact_output_has_no_spaces() {
    let value = json!({ "a": [1, false, null, "x"] });
    assert_eq!(write(&value, JSONWritingOptions::default()), r#"{"a":[1,false,null,"x"]}"#);
    assert_eq!(write(&json!({}), JSONWritingOptions::default()), "{}");
    assert_eq!(write(&json!([]), JSONWritingOptions::default()), "[]");
}

#[test]
fn pretty_output_matches_darwin() {
    let value = json!({ "a": [1, 2] });
    assert_eq!(write(&value, PRETTY), "{\n  \"a\" : [\n    1,\n    2\n  ]\n}");
}

#[test]
fn pretty_empty_containers_keep_a_blank_line() {
    assert_eq!(write(&json!({}), PRETTY), "{\n\n}");
    assert_eq!(write(&json!([]), PRETTY), "[\n\n]");
    assert_eq!(write(&json!({ "a": {} }), PRETTY), "{\n  \"a\" : {\n\n  }\n}");
    assert_eq!(write(&json!([[]]), PRETTY), "[\n  [\n\n  ]\n]");
}

#[test]
fn slashes_escape_unless_asked() {
    assert_eq!(write(&json!(["a/b"]), JSONWritingOptions::default()), r#"["a\/b"]"#);
    let options = JSONWritingOptions { without_escaping_slashes: true, ..JSONWritingOptions::default() };
    assert_eq!(write(&json!(["a/b"]), options), r#"["a/b"]"#);
}

#[test]
fn control_characters_escape_in_lowercase_hex() {
    let value = json!(["\u{1}\u{1f}\"\\\n\r\t\u{8}\u{c}"]);
    assert_eq!(
        write(&value, JSONWritingOptions::default()),
        "[\"\\u0001\\u001f\\\"\\\\\\n\\r\\t\\b\\f\"]"
    );
}

#[test]
fn non_ascii_is_written_raw() {
    assert_eq!(write(&json!(["\u{e9}\u{1f600}"]), JSONWritingOptions::default()), "[\"\u{e9}\u{1f600}\"]");
}

#[test]
fn doubles_use_seventeen_significant_digits() {
    let options = JSONWritingOptions::default();
    assert_eq!(write(&json!([0.1]), options), "[0.10000000000000001]");
    assert_eq!(write(&json!([1.5]), options), "[1.5]");
    assert_eq!(write(&json!([1e20]), options), "[1e+20]");
    assert_eq!(write(&json!([-0.0]), options), "[-0]");
    assert_eq!(write(&json!([3.0]), options), "[3]");
    assert_eq!(write(&json!([0.0001]), options), "[0.0001]");
    assert_eq!(write(&json!([0.00001]), options), "[1.0000000000000001e-05]");
    assert_eq!(write(&json!([1e16]), options), "[10000000000000000]");
    assert_eq!(write(&json!([1e17]), options), "[1e+17]");
}

#[test]
fn integers_print_exactly() {
    let options = JSONWritingOptions::default();
    assert_eq!(write(&json!([0, -7, u64::MAX, i64::MIN]), options), format!("[0,-7,{},{}]", u64::MAX, i64::MIN));
}

#[test]
fn sorted_keys_are_numeric_and_case_insensitive() {
    let value = json!({ "Z": 1, "B": 2, "b": 3, "a10": 4, "a2": 5, "_x": 6 });
    assert_eq!(write(&value, SORTED), r#"{"_x":6,"a2":5,"a10":4,"b":3,"B":2,"Z":1}"#);
}

#[test]
fn sorted_keys_apply_at_every_level() {
    let value = json!({ "b": { "y": 1, "x": 2 }, "a": [{ "d": 1, "c": 2 }] });
    assert_eq!(write(&value, SORTED), r#"{"a":[{"c":2,"d":1}],"b":{"x":2,"y":1}}"#);
}

#[test]
fn sorted_pretty_combines_both() {
    let options = JSONWritingOptions { sorted_keys: true, pretty_printed: true, ..JSONWritingOptions::default() };
    let value = json!({ "b": 1, "a": "/" });
    assert_eq!(write(&value, options), "{\n  \"a\" : \"\\/\",\n  \"b\" : 1\n}");
}

#[test]
fn round_trips_through_the_reader() {
    let value = json!({ "list": [1, 2.5, "s", null, { "k": true }] });
    let data = JSONSerialization::data_with_json_object(&value, SORTED).expect("a writable value");
    assert_eq!(JSONSerialization::json_object(&data), Some(value));
}

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::user_defaults::{DefaultsValue, UserDefaults};

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rise-user-defaults-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test sandbox is writable");
        Self { root }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_missing_key_reads_as_foundation_zero_values() {
    let defaults = UserDefaults::in_memory();
    assert!(!defaults.bool("missing"));
    assert_eq!(defaults.integer("missing"), 0);
    assert_eq!(defaults.double("missing"), 0.0);
    assert_eq!(defaults.string("missing"), None);
    assert_eq!(defaults.data("missing"), None);
    assert_eq!(defaults.object("missing"), None);
    assert_eq!(defaults.string_array("missing"), None);
}

#[test]
fn registered_defaults_answer_until_a_value_is_set_and_return_after_remove() {
    let defaults = UserDefaults::in_memory();
    defaults.register([("flag", DefaultsValue::Bool(true)), ("count", DefaultsValue::Integer(7))]);
    assert!(defaults.bool("flag"));
    assert_eq!(defaults.integer("count"), 7);

    defaults.set("flag", false);
    assert!(!defaults.bool("flag"));

    defaults.remove_object("flag");
    assert!(defaults.bool("flag"));
    assert_eq!(defaults.dictionary_representation().get("count"), Some(&DefaultsValue::Integer(7)));
}

#[test]
fn values_convert_between_numbers_and_strings_like_foundation() {
    let defaults = UserDefaults::in_memory();
    defaults.set("number", 42_i64);
    defaults.set("text", " 17 apples");
    defaults.set("yes", "YES");
    defaults.set("fraction", 2.5_f64);

    assert_eq!(defaults.string("number").as_deref(), Some("42"));
    assert_eq!(defaults.integer("text"), 17);
    assert!(defaults.bool("yes"));
    assert!(defaults.bool("text"));
    assert_eq!(defaults.integer("fraction"), 2);
    assert_eq!(defaults.double("text"), 17.0);
    assert_eq!(defaults.data("text"), None);
    assert_eq!(defaults.object("fraction").and_then(|v| v.as_i64()), None);
    assert_eq!(defaults.object("number").and_then(|v| v.as_bool()), None);
}

#[test]
fn a_string_array_with_a_non_string_element_is_nil() {
    let defaults = UserDefaults::in_memory();
    defaults.set("names", vec!["a".to_owned(), "b".to_owned()]);
    defaults.set("mixed", vec![DefaultsValue::from("a"), DefaultsValue::Integer(1)]);
    assert_eq!(defaults.string_array("names"), Some(vec!["a".to_owned(), "b".to_owned()]));
    assert_eq!(defaults.string_array("mixed"), None);
    assert_eq!(defaults.array("mixed").map(|items| items.len()), Some(2));
}

#[test]
fn a_write_is_read_at_once_and_reaches_disk_after_synchronize() {
    let sandbox = Sandbox::new("deferred");
    let file = sandbox.file("standard.defaults.json");
    let defaults = UserDefaults::persistent(file.clone());
    defaults.register([("registered", DefaultsValue::Bool(true))]);

    defaults.set("token", "abc");
    defaults.set("blob", vec![0_u8, 1, 2, 255]);
    let mut flags = BTreeMap::new();
    flags.insert("push".to_owned(), true);
    defaults.set("settings", flags);
    assert_eq!(defaults.string("token").as_deref(), Some("abc"));

    assert!(defaults.synchronize());
    let reopened = UserDefaults::persistent(file);
    assert_eq!(reopened.string("token").as_deref(), Some("abc"));
    assert_eq!(reopened.data("blob"), Some(vec![0_u8, 1, 2, 255]));
    assert_eq!(
        reopened.dictionary("settings").and_then(|d| d.get("push").and_then(DefaultsValue::as_bool)),
        Some(true)
    );
    assert!(!reopened.bool("registered"));
}

#[test]
fn a_removed_value_is_gone_from_disk_too() {
    let sandbox = Sandbox::new("remove");
    let file = sandbox.file("standard.defaults.json");
    let defaults = UserDefaults::persistent(file.clone());
    defaults.set("a", 1_i64);
    defaults.set("b", 2_i64);
    defaults.remove_object("a");
    assert!(defaults.synchronize());

    let reopened = UserDefaults::persistent(file);
    assert_eq!(reopened.object("a"), None);
    assert_eq!(reopened.integer("b"), 2);
}

#[test]
fn two_domains_keep_separate_files() {
    let sandbox = Sandbox::new("suites");
    let first = UserDefaults::persistent(sandbox.file("one.suite.json"));
    let second = UserDefaults::persistent(sandbox.file("two.suite.json"));
    first.set("shared", 1_i64);
    second.set("shared", 2_i64);
    assert!(first.synchronize());
    assert!(second.synchronize());

    assert_eq!(UserDefaults::persistent(sandbox.file("one.suite.json")).integer("shared"), 1);
    assert_eq!(UserDefaults::persistent(sandbox.file("two.suite.json")).integer("shared"), 2);
}

#[test]
fn a_damaged_entry_is_skipped_and_the_rest_loads() {
    let sandbox = Sandbox::new("damaged");
    let file = sandbox.file("standard.defaults.json");
    std::fs::write(&file, br#"{"good":{"integer":5},"bad":{"no_such_kind":1}}"#)
        .expect("test file is writable");
    let defaults = UserDefaults::persistent(file);
    assert_eq!(defaults.integer("good"), 5);
    assert_eq!(defaults.object("bad"), None);
}

#[test]
fn a_suite_name_that_cannot_be_a_file_is_nil_and_a_valid_one_is_shared() {
    assert!(UserDefaults::suite("").is_none());
    assert!(UserDefaults::suite("../escape").is_none());
    let name = format!("group.test.{}", std::process::id());
    let first = UserDefaults::suite(&name).expect("valid suite name");
    let second = UserDefaults::suite(&name).expect("valid suite name");
    first.set("badge", 3_i64);
    assert_eq!(second.integer("badge"), 3);
}

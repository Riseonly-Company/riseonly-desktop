use crate::foundation::foundation_data_detector::{NSDataDetector, NSTextCheckingType, LINK_DETECTOR};
use crate::foundation::foundation_regex::NSMatchingOptions;
use crate::foundation::ns_range::{substring, NSRange};

fn detector() -> &'static NSDataDetector {
    LINK_DETECTOR.as_ref().expect("link detector builds")
}

fn links(text: &str) -> Vec<(String, String)> {
    detector()
        .matches(text, NSMatchingOptions::NONE, NSRange::full(text))
        .into_iter()
        .map(|m| {
            let found = substring(text, m.range).unwrap_or_default().to_owned();
            let url = m.url.map(|u| u.absolute_string().to_owned()).unwrap_or_default();
            (found, url)
        })
        .collect()
}

#[test]
fn only_link_type_builds() {
    assert!(NSDataDetector::new(NSTextCheckingType::LINK).is_ok());
    assert!(NSDataDetector::new(NSTextCheckingType(1 << 3)).is_err());
    assert!(NSDataDetector::new(NSTextCheckingType(0)).is_err());
    assert_eq!(NSTextCheckingType::LINK.raw_value(), 32);
}

#[test]
fn scheme_links_keep_their_text() {
    assert_eq!(
        links("see https://rise.example.com/a?b=1#c now"),
        vec![("https://rise.example.com/a?b=1#c".to_owned(), "https://rise.example.com/a?b=1#c".to_owned())]
    );
    assert_eq!(links("open riseonly://chat/42")[0].1, "riseonly://chat/42");
}

#[test]
fn www_and_bare_hosts_get_http() {
    assert_eq!(links("go to www.example.zz"), vec![("www.example.zz".to_owned(), "http://www.example.zz".to_owned())]);
    assert_eq!(links("apple.com/mac"), vec![("apple.com/mac".to_owned(), "http://apple.com/mac".to_owned())]);
    assert!(links("version 1.5 and file.unknowntld").is_empty());
}

#[test]
fn addresses_become_mailto() {
    assert_eq!(links("write to a.b+c@mail.example.org."), vec![("a.b+c@mail.example.org".to_owned(), "mailto:a.b+c@mail.example.org".to_owned())]);
    assert_eq!(links("mailto:x@y.com")[0].1, "mailto:x@y.com");
}

#[test]
fn trailing_punctuation_and_unopened_brackets_are_dropped() {
    assert_eq!(links("(see https://x.com/a).")[0].0, "https://x.com/a");
    assert_eq!(links("https://en.wikipedia.org/wiki/Rust_(language)!")[0].0, "https://en.wikipedia.org/wiki/Rust_(language)");
    assert_eq!(links("example.com, then")[0].0, "example.com");
}

#[test]
fn ranges_are_utf16() {
    let text = "\u{1F600} example.com";
    let m = detector().first_match(text, NSMatchingOptions::NONE, NSRange::full(text)).expect("link");
    assert_eq!(m.range, NSRange::new(3, 11));
    assert_eq!(m.result_type, NSTextCheckingType::LINK);
}

#[test]
fn hosts_inside_words_or_paths_are_not_links() {
    assert!(links("x_example.com").is_empty());
    assert_eq!(links("https://a.com/b.com").len(), 1);
}

#[test]
fn anchored_requires_link_at_range_start() {
    let text = "hi example.com";
    assert!(detector().first_match(text, NSMatchingOptions::ANCHORED, NSRange::full(text)).is_none());
    assert!(detector().first_match(text, NSMatchingOptions::ANCHORED, NSRange::new(3, 11)).is_some());
}

#[test]
fn whole_address_match_equals_range() {
    let text = "user@example.com";
    let full = NSRange::full(text);
    let m = detector().first_match(text, NSMatchingOptions::NONE, full).expect("address");
    assert_eq!(m.range, full);
    assert_eq!(m.url.and_then(|u| u.scheme().map(str::to_owned)).as_deref(), Some("mailto"));
}

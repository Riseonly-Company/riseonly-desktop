use super::foundation_url::{
    Url, UrlAllowedCharacters, UrlComponents, UrlQueryItem, adding_percent_encoding,
    removing_percent_encoding,
};

fn url(string: &str) -> Url {
    match Url::from_string(string) {
        Some(url) => url,
        None => panic!("expected a URL for {string:?}"),
    }
}

#[test]
fn parses_every_component() {
    let u = url("https://user:pa%20ss@example.com:8443/a/b%20c.tar.gz?x=1&y=%2B#frag");
    assert_eq!(u.scheme(), Some("https"));
    assert_eq!(u.user().as_deref(), Some("user"));
    assert_eq!(u.password().as_deref(), Some("pa ss"));
    assert_eq!(u.host().as_deref(), Some("example.com"));
    assert_eq!(u.port(), Some(8443));
    assert_eq!(u.path(), "/a/b c.tar.gz");
    assert_eq!(u.query(), Some("x=1&y=%2B"));
    assert_eq!(u.fragment(), Some("frag"));
    assert_eq!(u.last_path_component(), "b c.tar.gz");
    assert_eq!(u.path_extension(), "gz");
    assert_eq!(u.absolute_string(), "https://user:pa%20ss@example.com:8443/a/b%20c.tar.gz?x=1&y=%2B#frag");
}

#[test]
fn space_without_encoding_is_none() {
    assert!(Url::from_string_encoding_invalid_characters("https://a.com/a b", false).is_none());
    assert!(Url::from_string_encoding_invalid_characters("a b", false).is_none());
    assert!(UrlComponents::from_string("https://a.com/a b").is_none());
}

#[test]
fn space_is_encoded_like_ios_17() {
    assert_eq!(url("https://a.com/a b?q=c d").absolute_string(), "https://a.com/a%20b?q=c%20d");
    assert_eq!(url("https://a.com/100%").absolute_string(), "https://a.com/100%25");
    assert_eq!(url("https://a.com/%20").absolute_string(), "https://a.com/%20");
}

#[test]
fn a_repair_makes_every_percent_of_its_component_literal() {
    assert_eq!(url("file:///a b%20c.png").absolute_string(), "file:///a%20b%2520c.png");
    assert_eq!(url("file:///a%20b%ZZ.png").absolute_string(), "file:///a%2520b%25ZZ.png");
    assert_eq!(url("https://h.com/a%20b/%ZZ?q=%20#f%20").absolute_string(), "https://h.com/a%2520b/%25ZZ?q=%20#f%20");
    assert_eq!(url("https://h.com/a%20b/c?q=%ZZ%20#f%20").absolute_string(), "https://h.com/a%20b/c?q=%25ZZ%2520#f%20");
    assert_eq!(url("https://h.com/a%2").absolute_string(), "https://h.com/a%252");
    assert_eq!(url("https://h.com/%20\u{65e5}").absolute_string(), "https://h.com/%2520%E6%97%A5");
}

#[test]
fn empty_string_bad_scheme_and_bad_port_are_none() {
    assert!(Url::from_string("").is_none());
    assert!(Url::from_string("1http://a.com").is_none());
    assert!(Url::from_string("http://a.com:8x/").is_none());
    assert!(Url::from_string("http://[::1/").is_none());
}

#[test]
fn relative_and_scheme_only_strings_parse() {
    let relative = url("folder/file.png");
    assert_eq!(relative.scheme(), None);
    assert_eq!(relative.path(), "folder/file.png");
    assert_eq!(relative.path_extension(), "png");
    assert!(Url::from_string("riseonly:").is_some());
}

#[test]
fn ip_literal_host_drops_brackets() {
    let u = url("http://[::1]:8080/x");
    assert_eq!(u.host().as_deref(), Some("::1"));
    assert_eq!(u.port(), Some(8080));
}

#[test]
fn path_strips_trailing_slash_except_root() {
    assert_eq!(url("https://a.com/dir/").path(), "/dir");
    assert_eq!(url("https://a.com/").path(), "/");
    assert_eq!(url("https://a.com").path(), "");
    assert_eq!(url("https://a.com/").last_path_component(), "/");
    assert_eq!(url("https://a.com/dir/").last_path_component(), "dir");
}

#[test]
fn path_extension_edge_cases() {
    assert_eq!(url("file:///tmp/.bashrc").path_extension(), "");
    assert_eq!(url("file:///tmp/name.").path_extension(), "");
    assert_eq!(url("file:///tmp/name").path_extension(), "");
}

#[test]
fn file_url_detection_ignores_case() {
    assert!(url("file:///tmp/a.txt").is_file_url());
    assert!(url("FILE:///tmp/a.txt").is_file_url());
    assert!(!url("https://a.com/a.txt").is_file_url());
    assert_eq!(url("file:///tmp/a.txt").host(), None);
}

#[test]
fn appending_and_deleting_path_components() {
    let base = url("https://a.com/api");
    assert_eq!(base.appending_path_component("v1").absolute_string(), "https://a.com/api/v1");
    assert_eq!(url("https://a.com/api/").appending_path_component("v1").absolute_string(), "https://a.com/api/v1");
    assert_eq!(url("https://a.com").appending_path_component("x y").absolute_string(), "https://a.com/x%20y");
    assert_eq!(url("https://a.com/a/b").deleting_last_path_component().absolute_string(), "https://a.com/a/");
    assert_eq!(url("https://a.com/a/b/").deleting_last_path_component().absolute_string(), "https://a.com/a/");
    assert_eq!(url("file:///").deleting_last_path_component().absolute_string(), "file:///../");
}

#[test]
fn path_extension_append_and_delete() {
    let u = url("file:///tmp/clip.mov");
    assert_eq!(u.deleting_path_extension().absolute_string(), "file:///tmp/clip");
    assert_eq!(u.deleting_path_extension().appending_path_extension("mp4").absolute_string(), "file:///tmp/clip.mp4");
}

#[test]
fn path_components_start_with_root() {
    assert_eq!(url("https://a.com/a/b").path_components(), vec!["/", "a", "b"]);
}

#[test]
fn relative_to_resolves_by_rfc_3986() {
    let base = url("http://a/b/c/d;p?q");
    let resolve = |r: &str| match Url::from_string_relative_to(r, Some(&base)) {
        Some(u) => u.absolute_string().to_string(),
        None => panic!("expected a URL for {r:?}"),
    };
    assert_eq!(resolve("g"), "http://a/b/c/g");
    assert_eq!(resolve("./g"), "http://a/b/c/g");
    assert_eq!(resolve("g/"), "http://a/b/c/g/");
    assert_eq!(resolve("/g"), "http://a/g");
    assert_eq!(resolve("//g"), "http://g");
    assert_eq!(resolve("?y"), "http://a/b/c/d;p?y");
    assert_eq!(resolve("#s"), "http://a/b/c/d;p?q#s");
    assert_eq!(resolve("../g"), "http://a/b/g");
    assert_eq!(resolve("../../g"), "http://a/g");
    assert_eq!(resolve("../../../g"), "http://a/g");
    let relative = Url::from_string_relative_to("g", Some(&base));
    assert_eq!(relative.as_ref().map(Url::relative_string), Some("g"));
    assert_eq!(relative.as_ref().and_then(Url::base_url), Some(&base));
}

#[test]
fn standardized_removes_dot_segments() {
    assert_eq!(url("https://a.com/a/./b/../c").standardized().absolute_string(), "https://a.com/a/c");
}

#[test]
fn components_query_items_decode_and_keep_plus() {
    let components = UrlComponents::from_string("https://a.com/s?q=a%20b&tag=c+d&flag&empty=");
    let items = components.and_then(|c| c.query_items());
    assert_eq!(
        items,
        Some(vec![
            UrlQueryItem::new("q", Some("a b".to_string())),
            UrlQueryItem::new("tag", Some("c+d".to_string())),
            UrlQueryItem::new("flag", None),
            UrlQueryItem::new("empty", Some(String::new())),
        ])
    );
}

#[test]
fn components_build_url_with_encoded_query_items() {
    let mut components = UrlComponents::new();
    components.set_scheme(Some("https"));
    components.set_host(Some("api.example.com"));
    components.set_path("/search files");
    components.set_query_items(Some(vec![
        UrlQueryItem::new("q", Some("a&b=c d".to_string())),
        UrlQueryItem::new("plus", Some("1+1".to_string())),
        UrlQueryItem::new("flag", None),
    ]));
    let built = components.url().map(|u| u.absolute_string().to_string());
    assert_eq!(built.as_deref(), Some("https://api.example.com/search%20files?q=a%26b%3Dc%20d&plus=1+1&flag"));
    assert_eq!(components.path(), "/search files");
    assert_eq!(components.percent_encoded_path(), "/search%20files");
}

#[test]
fn components_reject_relative_path_next_to_host() {
    let mut components = UrlComponents::new();
    components.set_host(Some("a.com"));
    components.set_path("relative");
    assert_eq!(components.string(), None);
    assert!(components.url().is_none());
}

#[test]
fn components_from_url_round_trips() {
    let source = url("https://a.com:8080/p?x=1#f");
    let components = UrlComponents::from_url(&source, false);
    assert_eq!(components.as_ref().map(UrlComponents::port), Some(Some(8080)));
    assert_eq!(components.and_then(|c| c.url()), Some(source));
}

#[test]
fn empty_query_items_set_empty_query() {
    let mut components = UrlComponents::from_string("https://a.com").unwrap_or_default();
    components.set_query_items(Some(Vec::new()));
    assert_eq!(components.string().as_deref(), Some("https://a.com?"));
    components.set_query_items(None);
    assert_eq!(components.string().as_deref(), Some("https://a.com"));
}

#[test]
fn percent_encoding_helpers_match_foundation() {
    assert_eq!(adding_percent_encoding("a b/c?d", UrlAllowedCharacters::URL_QUERY_ALLOWED), "a%20b/c?d");
    assert_eq!(adding_percent_encoding("ab;c", UrlAllowedCharacters::URL_PATH_ALLOWED), "ab%3Bc");
    let with_percent = UrlAllowedCharacters::URL_QUERY_ALLOWED.union("%");
    assert_eq!(adding_percent_encoding("50%", with_percent), "50%");
    assert_eq!(adding_percent_encoding("\u{e9}", UrlAllowedCharacters::URL_QUERY_ALLOWED), "%C3%A9");
    assert_eq!(removing_percent_encoding("a%20b%C3%A9").as_deref(), Some("a b\u{e9}"));
    assert_eq!(removing_percent_encoding("100%"), None);
    assert_eq!(removing_percent_encoding("%FF"), None);
}

#[test]
fn non_ascii_host_becomes_punycode() {
    assert_eq!(url("https://b\u{fc}cher.example/").host().as_deref(), Some("xn--bcher-kva.example"));
}

use super::foundation_file_url::{
    PathFlavor, file_path_string, file_url_string, is_absolute_path_in, posix_file_url_string_from_bytes,
};
use super::foundation_url::Url;

fn url(string: &str) -> Url {
    match Url::from_string(string) {
        Some(url) => url,
        None => panic!("expected a URL for {string:?}"),
    }
}

#[test]
fn posix_path_becomes_file_url_with_encoded_segments() {
    assert_eq!(file_url_string("/Users/a/x y%.jpg", PathFlavor::Posix).as_deref(), Some("file:///Users/a/x%20y%25.jpg"));
    assert_eq!(file_url_string("/", PathFlavor::Posix).as_deref(), Some("file:///"));
    assert_eq!(file_url_string("/a/\u{444}#?.m4a", PathFlavor::Posix).as_deref(), Some("file:///a/%D1%84%23%3F.m4a"));
    assert_eq!(file_url_string("relative/x.jpg", PathFlavor::Posix), None);
}

#[test]
fn windows_drive_path_becomes_file_url_with_drive_segment() {
    assert_eq!(file_url_string(r"C:\Users\a\x y.jpg", PathFlavor::Windows).as_deref(), Some("file:///C:/Users/a/x%20y.jpg"));
    assert_eq!(file_url_string("d:/media/v.mp4", PathFlavor::Windows).as_deref(), Some("file:///d:/media/v.mp4"));
    assert_eq!(file_url_string(r"C:\", PathFlavor::Windows).as_deref(), Some("file:///C:/"));
    assert_eq!(file_url_string("C:", PathFlavor::Windows).as_deref(), Some("file:///C:/"));
    assert_eq!(file_url_string(r"\\?\C:\a\b.png", PathFlavor::Windows).as_deref(), Some("file:///C:/a/b.png"));
}

#[test]
fn windows_unc_path_becomes_file_url_with_host() {
    assert_eq!(file_url_string(r"\\server\share\a b.txt", PathFlavor::Windows).as_deref(), Some("file://server/share/a%20b.txt"));
    assert_eq!(file_url_string(r"\\?\UNC\server\share\x", PathFlavor::Windows).as_deref(), Some("file://server/share/x"));
    assert_eq!(file_url_string(r"\\server", PathFlavor::Windows).as_deref(), Some("file://server/"));
}

#[test]
fn windows_relative_and_drive_relative_paths_are_rejected() {
    assert_eq!(file_url_string(r"Users\a.jpg", PathFlavor::Windows), None);
    assert_eq!(file_url_string(r"C:a.jpg", PathFlavor::Windows), None);
    assert_eq!(file_url_string(r"\\", PathFlavor::Windows), None);
    assert_eq!(file_url_string("/Users/a.jpg", PathFlavor::Windows), None);
}

#[test]
fn posix_file_url_gives_decoded_path_and_ignores_host() {
    assert_eq!(file_path_string(&url("file:///Users/a/x%20y.jpg"), PathFlavor::Posix).as_deref(), Some("/Users/a/x y.jpg"));
    assert_eq!(file_path_string(&url("file://localhost/tmp/a"), PathFlavor::Posix).as_deref(), Some("/tmp/a"));
    assert_eq!(file_path_string(&url("file:///"), PathFlavor::Posix).as_deref(), Some("/"));
    assert_eq!(file_path_string(&url("https://example.com/a.jpg"), PathFlavor::Posix), None);
}

#[test]
fn windows_file_url_with_drive_gives_backslash_path() {
    assert_eq!(file_path_string(&url("file:///C:/Users/a/x%20y.jpg"), PathFlavor::Windows).as_deref(), Some(r"C:\Users\a\x y.jpg"));
    assert_eq!(file_path_string(&url("file://localhost/D:/v.mp4"), PathFlavor::Windows).as_deref(), Some(r"D:\v.mp4"));
    assert_eq!(file_path_string(&url("file:///C:/"), PathFlavor::Windows).as_deref(), Some(r"C:\"));
    assert_eq!(file_path_string(&url("file:///C|/a"), PathFlavor::Windows).as_deref(), Some(r"C:\a"));
}

#[test]
fn windows_file_url_with_host_gives_unc_path() {
    assert_eq!(file_path_string(&url("file://server/share/a%20b.txt"), PathFlavor::Windows).as_deref(), Some(r"\\server\share\a b.txt"));
    assert_eq!(file_path_string(&url("file://server/"), PathFlavor::Windows), None);
}

#[test]
fn windows_file_url_without_drive_is_rejected() {
    assert_eq!(file_path_string(&url("file:///Users/a.jpg"), PathFlavor::Windows), None);
    assert_eq!(file_path_string(&url("file:///C:a"), PathFlavor::Windows), None);
    assert_eq!(file_path_string(&url("file:///"), PathFlavor::Windows), None);
}

#[test]
fn round_trip_keeps_the_path_in_both_flavors() {
    let cases = [
        ("/var/mobile/a b/%c;d.ogg", PathFlavor::Posix),
        (r"C:\Users\a b\%c;d.ogg", PathFlavor::Windows),
        (r"\\nas\media\clip.mp4", PathFlavor::Windows),
    ];
    for (path, flavor) in cases {
        let string = match file_url_string(path, flavor) {
            Some(string) => string,
            None => panic!("expected a file URL for {path:?}"),
        };
        assert_eq!(file_path_string(&url(&string), flavor).as_deref(), Some(path), "round trip of {path:?}");
    }
}

#[test]
fn absolute_path_check_follows_the_flavor() {
    assert!(is_absolute_path_in("/a", PathFlavor::Posix));
    assert!(!is_absolute_path_in(r"C:\a", PathFlavor::Posix));
    assert!(is_absolute_path_in(r"C:\a", PathFlavor::Windows));
    assert!(is_absolute_path_in("c:/a", PathFlavor::Windows));
    assert!(is_absolute_path_in(r"\\host\share", PathFlavor::Windows));
    assert!(!is_absolute_path_in("/a", PathFlavor::Windows));
    assert!(!is_absolute_path_in("content://x", PathFlavor::Windows));
}

#[test]
fn host_flavor_matches_the_separator() {
    let expected = if std::path::MAIN_SEPARATOR == '\\' { PathFlavor::Windows } else { PathFlavor::Posix };
    assert_eq!(PathFlavor::HOST, expected);
}

#[test]
fn posix_bytes_outside_utf8_are_escaped_in_the_url() {
    let string = posix_file_url_string_from_bytes(b"/a b/\xFF\xC0.jpg");
    assert_eq!(string.as_deref(), Some("file:///a%20b/%FF%C0.jpg"));
    assert_eq!(posix_file_url_string_from_bytes(b"relative/\xFF"), None);
}

#[test]
fn a_url_built_from_a_non_utf8_path_gives_that_path_back() {
    let original = std::path::PathBuf::from("/a/original");
    let kept = url("file:///a/%FF.jpg").keeping_non_utf8_file_path(original.clone());
    assert_eq!(kept.to_file_path().as_ref(), Some(&original));
    assert_eq!(kept.clone().to_file_path().as_ref(), Some(&original));
}

#[test]
fn a_file_url_with_non_utf8_escapes_names_those_bytes_on_posix() {
    let path = url("file:///a%20b/%FF.png").to_file_path();
    match PathFlavor::HOST {
        PathFlavor::Posix => {
            let bytes = path.as_ref().map(|path| path.as_os_str().as_encoded_bytes());
            assert_eq!(bytes, Some(&b"/a b/\xFF.png"[..]));
        }
        PathFlavor::Windows => assert_eq!(path, None),
    }
}

#[test]
fn a_nul_beside_non_utf8_escapes_names_no_path() {
    assert_eq!(url("file:///a%FF%00.png").to_file_path(), None);
}

#[test]
fn a_malformed_escape_makes_the_non_utf8_one_literal_as_foundation_does() {
    let parsed = url("file:///a%FF%ZZ.png");
    assert_eq!(parsed.absolute_string(), "file:///a%25FF%25ZZ.png");
    let expected = match PathFlavor::HOST {
        PathFlavor::Posix => Some(std::path::PathBuf::from("/a%FF%ZZ.png")),
        PathFlavor::Windows => None,
    };
    assert_eq!(parsed.to_file_path(), expected);
    assert!(Url::from_string_encoding_invalid_characters("file:///a%FF%ZZ.png", false).is_none());
}

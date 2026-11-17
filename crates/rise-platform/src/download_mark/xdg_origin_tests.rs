use crate::download_mark::xdg_origin::*;
use crate::download_mark::DownloadOrigin;

#[test]
fn the_attribute_names_follow_common_extended_attributes() {
    assert_eq!(ORIGIN_URL_ATTRIBUTE, "user.xdg.origin.url");
    assert_eq!(REFERRER_URL_ATTRIBUTE, "user.xdg.referrer.url");
}

#[test]
fn both_urls_are_written_origin_first() {
    let origin = DownloadOrigin {
        host_url: Some("https://cdn.example.com/files/report.pdf".to_string()),
        referrer_url: Some("https://example.com/chat/42".to_string()),
    };
    assert_eq!(
        xdg_origin_attributes(&origin),
        vec![
            (ORIGIN_URL_ATTRIBUTE, "https://cdn.example.com/files/report.pdf".to_string()),
            (REFERRER_URL_ATTRIBUTE, "https://example.com/chat/42".to_string()),
        ]
    );
}

#[test]
fn tokens_in_the_url_are_not_kept() {
    let origin = DownloadOrigin {
        host_url: Some("https://user:secret@cdn.example.com/a.zip?signature=abc#part".to_string()),
        referrer_url: None,
    };
    assert_eq!(
        xdg_origin_attributes(&origin),
        vec![(ORIGIN_URL_ATTRIBUTE, "https://cdn.example.com/a.zip".to_string())]
    );
}

#[test]
fn a_file_without_a_known_url_gets_no_attribute() {
    assert!(xdg_origin_attributes(&DownloadOrigin::default()).is_empty());
    let origin = DownloadOrigin {
        host_url: Some("not a url".to_string()),
        referrer_url: Some("https://example.com/\r\nx".to_string()),
    };
    assert!(xdg_origin_attributes(&origin).is_empty());
}

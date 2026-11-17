use crate::download_mark::*;

fn origin(host_url: Option<&str>, referrer_url: Option<&str>) -> DownloadOrigin {
    DownloadOrigin {
        host_url: host_url.map(str::to_owned),
        referrer_url: referrer_url.map(str::to_owned),
    }
}

#[test]
fn the_zone_alone_is_written_without_an_origin() {
    assert_eq!(zone_identifier_text(&DownloadOrigin::default()), "[ZoneTransfer]\r\nZoneId=3\r\n");
}

#[test]
fn the_referrer_comes_before_the_host_url_as_browsers_write_them() {
    let text = zone_identifier_text(&origin(
        Some("https://cdn.riseonly.net/files/report.pdf"),
        Some("https://riseonly.net/chat/42"),
    ));

    assert_eq!(
        text,
        "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://riseonly.net/chat/42\r\nHostUrl=https://cdn.riseonly.net/files/report.pdf\r\n"
    );
}

#[test]
fn tokens_in_the_query_fragment_and_user_info_never_reach_the_disk() {
    assert_eq!(
        recorded_url("https://user:secret@cdn.riseonly.net/f/a.zip?X-Amz-Signature=abc#part").as_deref(),
        Some("https://cdn.riseonly.net/f/a.zip")
    );
}

#[test]
fn a_host_without_a_path_is_kept_as_is() {
    assert_eq!(recorded_url("https://riseonly.net").as_deref(), Some("https://riseonly.net"));
    assert_eq!(recorded_url("https://riseonly.net?x=1").as_deref(), Some("https://riseonly.net"));
}

#[test]
fn values_that_are_not_one_line_absolute_urls_are_not_recorded() {
    assert_eq!(recorded_url("report.pdf"), None);
    assert_eq!(recorded_url("https://"), None);
    assert_eq!(recorded_url("://riseonly.net/a"), None);
    assert_eq!(recorded_url("https://riseonly.net/a\r\nZoneId=0"), None);
    assert_eq!(recorded_url("ht tp://riseonly.net/a"), None);
}

#[test]
fn a_rejected_url_leaves_only_the_zone_line() {
    let text = zone_identifier_text(&origin(Some("https://riseonly.net/a\nZoneId=0"), None));

    assert_eq!(text, "[ZoneTransfer]\r\nZoneId=3\r\n");
}

#[test]
fn the_internet_zone_is_three() {
    assert_eq!(INTERNET_ZONE_ID, 3);
    assert_eq!(ZONE_IDENTIFIER_STREAM, "Zone.Identifier");
}

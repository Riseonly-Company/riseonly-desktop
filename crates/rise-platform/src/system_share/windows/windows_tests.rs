use std::path::PathBuf;

use gpui::{bounds, point, px, size};

use crate::system_share::*;
use super::{PackageContent, PendingReports, share_title};

fn anchor() -> gpui::Bounds<gpui::Pixels> {
    bounds(point(px(0.0), px(0.0)), size(px(10.0), px(10.0)))
}

#[test]
fn a_file_titles_the_share_with_its_name() {
    let items = [
        ShareItem::Link("https://riseonly.net/p/1".into()),
        ShareItem::File(PathBuf::from(r"C:\Users\me\Pictures\photo.jpg")),
    ];
    assert_eq!(share_title(&items), "photo.jpg");
}

#[test]
fn a_text_titles_the_share_with_its_first_non_empty_line() {
    let items = [ShareItem::Text("\n  first line  \nsecond".into())];
    assert_eq!(share_title(&items), "first line");
}

#[test]
fn a_long_line_is_shortened_with_an_ellipsis() {
    let long = "x".repeat(200);
    let title = share_title(&[ShareItem::Text(long)]);
    assert_eq!(title.chars().count(), 80);
    assert!(title.ends_with('\u{2026}'));
}

#[test]
fn the_package_carries_the_first_link_all_text_and_every_file() {
    let request = ShareRequest::new(
        vec![
            ShareItem::Link("https://riseonly.net/a".into()),
            ShareItem::Text("hello".into()),
            ShareItem::Link("https://riseonly.net/b".into()),
            ShareItem::File(PathBuf::from(r"C:\tmp\a.txt")),
        ],
        anchor(),
    )
    .expect("valid request");
    let content = PackageContent::of(&request);
    assert_eq!(content.web_link.as_deref(), Some("https://riseonly.net/a"));
    assert_eq!(
        content.text.as_deref(),
        Some("https://riseonly.net/a\nhello\nhttps://riseonly.net/b")
    );
    assert_eq!(content.files, vec![PathBuf::from(r"C:\tmp\a.txt")]);
    assert_eq!(content.title, "a.txt");
}

#[test]
fn files_alone_carry_no_text_and_no_link() {
    let request = ShareRequest::new(
        vec![ShareItem::File(PathBuf::from(r"C:\tmp\clip.mp4"))],
        anchor(),
    )
    .expect("valid request");
    let content = PackageContent::of(&request);
    assert_eq!(content.web_link, None);
    assert_eq!(content.text, None);
}

#[test]
fn a_receiver_dropped_before_its_frame_waits_for_the_next_share() {
    let pending = PendingReports::take().expect("the receiver is free before any share");
    assert!(PendingReports::take().is_none(), "one drain task per process");
    drop(pending);
    let again = PendingReports::take();
    assert!(again.is_some(), "a window closed before its frame gives the receiver back");
}

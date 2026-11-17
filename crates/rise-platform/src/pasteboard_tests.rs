use std::path::PathBuf;
use std::time::Duration;

use gpui::{ClipboardEntry, ClipboardItem, ExternalPaths, Image, ImageFormat};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_url::Url;

use crate::host_os::HostOs;
use crate::pasteboard::{
    UIPasteboardItem, clipboard_item_of, expired_item, expiry_delay, file_paths_in,
    file_paths_in_text, has_images_in,
    has_strings_in, image_in, still_holds, string_in, strings_in, url_in,
};

fn url(raw: &str) -> Url {
    let Some(url) = Url::from_string(raw) else {
        panic!("{raw} is not a URL");
    };
    url
}

fn png() -> Image {
    Image::from_bytes(ImageFormat::Png, vec![0x89, 0x50, 0x4e, 0x47])
}

#[test]
fn copied_text_reads_back_as_the_string() {
    let item = ClipboardItem::new_string("hello".to_owned());
    assert_eq!(string_in(&item).as_deref(), Some("hello"));
    assert!(has_strings_in(&item));
    assert!(!has_images_in(&item));
}

#[test]
fn copied_file_paths_are_not_text() {
    let paths = ExternalPaths(std::iter::once(PathBuf::from("/tmp/a.txt")).collect());
    let item = ClipboardItem { entries: vec![ClipboardEntry::ExternalPaths(paths)] };
    assert_eq!(string_in(&item), None);
    assert!(!has_strings_in(&item));
    assert!(strings_in(&item).is_empty());
}

#[test]
fn an_empty_string_is_no_string() {
    assert_eq!(string_in(&expired_item()), None);
    assert!(!has_strings_in(&expired_item()));
}

#[test]
fn image_reads_back_and_is_not_text() {
    let item = ClipboardItem::new_image(&png());
    assert_eq!(image_in(&item), Some(png()));
    assert!(has_images_in(&item));
    assert!(!has_strings_in(&item));
}

#[test]
fn set_items_keeps_every_value_in_order() {
    let item = clipboard_item_of(vec![
        UIPasteboardItem::PlainText("token".to_owned()),
        UIPasteboardItem::Url(url("https://riseonly.net/p/1")),
        UIPasteboardItem::Image(png()),
    ]);
    assert_eq!(
        strings_in(&item),
        vec!["token".to_owned(), "https://riseonly.net/p/1".to_owned()]
    );
    assert_eq!(image_in(&item), Some(png()));
}

#[test]
fn url_is_reported_only_for_one_link() {
    let link = ClipboardItem::new_string("  https://riseonly.net/p/1\n".to_owned());
    assert_eq!(
        url_in(&link).map(|u| u.absolute_string().to_owned()).as_deref(),
        Some("https://riseonly.net/p/1")
    );
    assert_eq!(url_in(&ClipboardItem::new_string("just words".to_owned())), None);
    assert_eq!(url_in(&ClipboardItem::new_string("see https://riseonly.net".to_owned())), None);
    assert_eq!(url_in(&expired_item()), None);
}

#[test]
fn expiry_waits_until_the_date_and_never_negative() {
    let now = Date::from_ms(1_000_000);
    assert_eq!(expiry_delay(now.adding_time_interval(120.0), now), Duration::from_secs(120));
    assert_eq!(expiry_delay(now.adding_time_interval(-5.0), now), Duration::ZERO);
    assert_eq!(expiry_delay(now, now), Duration::ZERO);
}

#[test]
fn expiry_leaves_a_later_copy_alone() {
    let written = clipboard_item_of(vec![UIPasteboardItem::PlainText("secret".to_owned())]);
    assert!(still_holds(Some(&written.clone()), &written));
    let later = ClipboardItem::new_string("user copy".to_owned());
    assert!(!still_holds(Some(&later), &written));
    assert!(!still_holds(None, &written));
}

#[test]
fn a_file_list_in_text_reads_as_paths_on_linux_only() {
    let item = ClipboardItem::new_string("/tmp/a b.png\nfile:///tmp/c%20d.pdf\n".to_owned());
    let expected = vec![PathBuf::from("/tmp/a b.png"), PathBuf::from("/tmp/c d.pdf")];
    assert_eq!(file_paths_in(&item, HostOs::Linux), expected);
    assert!(file_paths_in(&item, HostOs::MacOs).is_empty());
    assert!(file_paths_in(&item, HostOs::Windows).is_empty());
}

#[test]
fn path_entries_are_the_files_on_every_host() {
    let paths = vec![PathBuf::from("/tmp/a.png")];
    let item = ClipboardItem {
        entries: vec![
            ClipboardEntry::ExternalPaths(ExternalPaths(paths.iter().cloned().collect())),
            ClipboardEntry::String(gpui::ClipboardString::new("/tmp/other.png".to_owned())),
        ],
    };
    for host in HostOs::ALL {
        assert_eq!(file_paths_in(&item, host), paths);
    }
}

#[test]
fn a_uri_list_skips_comments_and_crlf() {
    let text = "# copied\r\nfile://localhost/tmp/a.png\r\n\r\nfile:///tmp/b.png\r\n";
    assert_eq!(
        file_paths_in_text(text),
        vec![PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")]
    );
}

#[test]
fn text_with_anything_else_lists_no_files() {
    assert!(file_paths_in_text("look at /tmp/a.png").is_empty());
    assert!(file_paths_in_text("/tmp/a.png\nand this one").is_empty());
    assert!(file_paths_in_text("https://example.com/a.png").is_empty());
    assert!(file_paths_in_text("file://server/share/a.png").is_empty());
    assert!(file_paths_in_text("relative/a.png").is_empty());
    assert!(file_paths_in_text("").is_empty());
}

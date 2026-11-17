use std::path::{Path, PathBuf};

use rise_core::foundation::foundation_url::Url;

use crate::app::shell::file_drop::{self, DropRejection};
use crate::modules::share_extension::entry::share_extension_models::ShareExtensionError;
use crate::modules::share_extension::entry::share_item_loader::ShareItemProvider;
use crate::modules::share_extension::entry::share_view_controller::ShareViewController;
use crate::modules::system_share::desktop_share_intake::{
    DesktopShareArguments, accepted_files, is_shareable_link, items_for_files, items_for_text, split_launch_arguments,
    split_opened_urls,
};

fn arguments(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|value| (*value).to_owned()).collect()
}

fn url(raw: &str) -> Url {
    Url::from_string(raw).expect("a valid test URL")
}

#[test]
fn a_path_in_the_arguments_is_a_file_and_the_executable_is_not() {
    let split = split_launch_arguments(&arguments(&["/usr/bin/riseonly", "/tmp/photo one.png"]));
    assert_eq!(split.files, vec![PathBuf::from("/tmp/photo one.png")]);
    assert!(split.links.is_empty());
    assert!(split.texts.is_empty());
}

#[test]
fn a_launch_with_nothing_shared_is_empty() {
    assert!(split_launch_arguments(&arguments(&["riseonly"])).is_empty());
}

#[test]
fn a_file_url_in_the_arguments_is_decoded_to_its_path() {
    let split = split_launch_arguments(&arguments(&["riseonly", "file:///tmp/photo%20one.png"]));
    assert_eq!(split.files, vec![PathBuf::from("/tmp/photo one.png")]);
    assert!(split.links.is_empty());
}

#[test]
fn a_flag_and_its_value_are_not_shared_but_later_arguments_are() {
    let split = split_launch_arguments(&arguments(&["riseonly", "--drive", "script", "hello there"]));
    assert_eq!(split.texts, vec!["hello there".to_owned()]);
}

#[test]
fn links_stay_links_so_the_share_handoffs_and_the_parser_see_them_first() {
    let split = split_launch_arguments(&arguments(&["riseonly", "riseonly://share-send?id=1", "/tmp/a.png"]));
    assert_eq!(split.links, vec![url("riseonly://share-send?id=1")]);
    assert_eq!(split.files, vec![PathBuf::from("/tmp/a.png")]);
}

#[test]
fn text_with_a_colon_is_text_not_a_link() {
    let split = split_launch_arguments(&arguments(&["riseonly", "note: buy milk"]));
    assert_eq!(split.texts, vec!["note: buy milk".to_owned()]);
    assert!(split.links.is_empty());
}

#[test]
fn opened_file_urls_are_files_and_other_urls_are_links() {
    let split = split_opened_urls(&arguments(&["file:///tmp/a.mov", "riseonly://chat/42"]));
    assert_eq!(split.files, vec![PathBuf::from("/tmp/a.mov")]);
    assert_eq!(split.links, vec![url("riseonly://chat/42")]);
}

#[test]
fn batches_keep_their_order_when_merged() {
    let mut merged = DesktopShareArguments::default();
    merged.extend(split_opened_urls(&arguments(&["file:///tmp/1.png"])));
    merged.extend(split_opened_urls(&arguments(&["file:///tmp/2.png"])));
    assert_eq!(merged.files, vec![PathBuf::from("/tmp/1.png"), PathBuf::from("/tmp/2.png")]);
}

#[test]
fn files_become_one_item_named_after_each_file() {
    let items = items_for_files(&[PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.mov")]);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].attachments,
        Some(vec![
            ShareItemProvider::File { path: PathBuf::from("/tmp/a.png"), suggested_name: Some("a.png".to_owned()) },
            ShareItemProvider::File { path: PathBuf::from("/tmp/b.mov"), suggested_name: Some("b.mov".to_owned()) },
        ])
    );
}

#[test]
fn text_and_a_link_go_through_as_text() {
    for text in ["hello there", "https://example.com/a"] {
        let items = items_for_text(text);
        assert_eq!(items[0].attachments, Some(vec![ShareItemProvider::Text(text.to_owned())]));
    }
}

#[test]
fn more_files_than_the_canon_allows_are_refused_whole() {
    let limit = file_drop::max_files();
    assert!(limit > 0, "the chat_attachments preset is in the canon");
    let paths: Vec<PathBuf> = (0..=limit).map(|n| PathBuf::from(format!("/tmp/{n}.png"))).collect();
    assert_eq!(
        accepted_files(&paths, |_: &Path| true),
        Err(DropRejection::TooMany { offered: limit + 1, limit })
    );
}

#[test]
fn a_directory_is_refused_by_name() {
    let paths = vec![PathBuf::from("/tmp/folder")];
    assert_eq!(
        accepted_files(&paths, |_: &Path| false),
        Err(DropRejection::NotAFile(PathBuf::from("/tmp/folder")))
    );
}

#[test]
fn only_web_links_are_shared_on_when_nothing_claims_them() {
    assert!(is_shareable_link(&url("https://example.com/a")));
    assert!(is_shareable_link(&url("http://example.com/a")));
    assert!(!is_shareable_link(&url("mailto:someone@example.com")));
}

#[test]
fn signed_out_and_a_missing_snapshot_still_offer_the_story_and_other_errors_do_not() {
    let signed_out = anyhow::Error::from(ShareExtensionError::AuthenticationUnavailable);
    let no_snapshot = anyhow::Error::from(ShareExtensionError::SnapshotUnavailable);
    let changed = anyhow::Error::from(ShareExtensionError::AccountChanged);
    assert!(ShareViewController::is_signed_out(&signed_out));
    assert!(ShareViewController::can_offer_story_without_session(&signed_out));
    assert!(!ShareViewController::is_signed_out(&no_snapshot));
    assert!(ShareViewController::can_offer_story_without_session(&no_snapshot));
    assert!(!ShareViewController::can_offer_story_without_session(&changed));
}

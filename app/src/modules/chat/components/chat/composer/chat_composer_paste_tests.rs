use std::path::{Path, PathBuf};

use gpui::{ClipboardEntry, ClipboardItem, ClipboardString, ExternalPaths, Image, ImageFormat};
use rise_platform::host_os::HostOs;

use crate::modules::chat::components::chat::composer::chat_composer_paste::ChatComposerPaste;

fn png() -> Image {
    Image::from_bytes(ImageFormat::Png, vec![1, 2, 3])
}

fn file_entry(path: &str) -> ClipboardEntry {
    ClipboardEntry::ExternalPaths(ExternalPaths(std::iter::once(PathBuf::from(path)).collect()))
}

fn string_entry(text: &str) -> ClipboardEntry {
    ClipboardEntry::String(ClipboardString::new(text.to_owned()))
}

fn every_path_is_a_file(_: &Path) -> bool {
    true
}

fn no_path_is_a_file(_: &Path) -> bool {
    false
}

#[test]
fn copied_files_win_over_the_names_beside_them() {
    let item = ClipboardItem { entries: vec![file_entry("/tmp/a.png"), string_entry("a.png")] };
    let paste = ChatComposerPaste::of(&item, HostOs::MacOs, every_path_is_a_file);
    assert!(matches!(paste, ChatComposerPaste::Files(paths) if paths == vec![PathBuf::from("/tmp/a.png")]));
}

#[test]
fn an_image_alone_goes_as_an_image() {
    let paste = ChatComposerPaste::of(&ClipboardItem::new_image(&png()), HostOs::Windows, every_path_is_a_file);
    assert!(matches!(paste, ChatComposerPaste::Image(_)));
}

#[test]
fn text_beside_an_image_stays_text() {
    let item = ClipboardItem { entries: vec![string_entry("caption"), ClipboardEntry::Image(png())] };
    let paste = ChatComposerPaste::of(&item, HostOs::MacOs, every_path_is_a_file);
    assert!(matches!(paste, ChatComposerPaste::Text));
}

#[test]
fn a_path_typed_as_text_on_linux_stays_text_unless_it_is_a_file() {
    let item = ClipboardItem::new_string("/usr/share/doc".to_owned());
    assert!(matches!(ChatComposerPaste::of(&item, HostOs::Linux, no_path_is_a_file), ChatComposerPaste::Text));
    assert!(matches!(ChatComposerPaste::of(&item, HostOs::Linux, every_path_is_a_file), ChatComposerPaste::Files(_)));
    assert!(matches!(ChatComposerPaste::of(&item, HostOs::MacOs, every_path_is_a_file), ChatComposerPaste::Text));
}

#[test]
fn files_that_are_gone_reach_the_drop_path_to_be_refused() {
    let item = ClipboardItem { entries: vec![file_entry("/tmp/gone.png")] };
    let paste = ChatComposerPaste::of(&item, HostOs::Windows, no_path_is_a_file);
    assert!(matches!(paste, ChatComposerPaste::Files(_)));
}

#[test]
fn plain_text_stays_text() {
    let item = ClipboardItem::new_string("hello".to_owned());
    assert!(matches!(ChatComposerPaste::of(&item, HostOs::MacOs, every_path_is_a_file), ChatComposerPaste::Text));
}

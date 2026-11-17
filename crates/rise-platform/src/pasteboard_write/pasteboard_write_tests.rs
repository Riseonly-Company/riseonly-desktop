use std::cell::RefCell;

use crate::host_os::HostOs;
use crate::pasteboard_write::*;

struct Unwritten;

impl NativePasteboardWriter for Unwritten {}

#[derive(Default)]
struct Recording {
    host_only: RefCell<Vec<String>>,
    urls: RefCell<Vec<String>>,
}

impl NativePasteboardWriter for Recording {
    fn write_host_only_text(&self, text: &str) -> Result<(), PasteboardWriteUnavailable> {
        self.host_only.borrow_mut().push(text.to_owned());
        Ok(())
    }

    fn write_url(&self, absolute_string: &str) -> Result<(), PasteboardWriteUnavailable> {
        self.urls.borrow_mut().push(absolute_string.to_owned());
        Err(PasteboardWriteUnavailable::SystemRefused("public.url".to_owned()))
    }
}

#[test]
fn a_layer_without_a_branch_names_the_host() {
    let expected = Err(PasteboardWriteUnavailable::BranchNotWritten(HostOs::current()));
    assert_eq!(write_host_only_text_with(&Unwritten, "token"), expected);
    assert_eq!(write_url_with(&Unwritten, "https://riseonly.net/p/1"), expected);
    assert_eq!(write_image_png_with(&Unwritten, b"\x89PNG\r\n\x1a\n"), expected);
}

#[test]
fn writes_reach_the_layer_unchanged_and_its_refusal_comes_back() {
    let recording = Recording::default();
    assert_eq!(write_host_only_text_with(&recording, " bot:token \n"), Ok(()));
    assert_eq!(
        write_url_with(&recording, "https://riseonly.net/s/pack"),
        Err(PasteboardWriteUnavailable::SystemRefused("public.url".to_owned()))
    );
    assert_eq!(*recording.host_only.borrow(), vec![" bot:token \n".to_owned()]);
    assert_eq!(*recording.urls.borrow(), vec!["https://riseonly.net/s/pack".to_owned()]);
}

#[test]
fn the_default_answer_is_branch_not_written() {
    assert_eq!(
        unavailable_by_default(HostOs::Linux),
        PasteboardWriteUnavailable::BranchNotWritten(HostOs::Linux)
    );
}

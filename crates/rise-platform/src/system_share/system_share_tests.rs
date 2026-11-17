use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{bounds, point, px, size};

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::system_share::*;

fn anchor() -> gpui::Bounds<gpui::Pixels> {
    bounds(point(px(40.0), px(100.0)), size(px(24.0), px(20.0)))
}

fn absolute(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

#[test]
fn empty_items_leave_nothing_to_share() {
    let items = vec![ShareItem::Text(String::new()), ShareItem::Link(String::new())];
    assert_eq!(
        ShareRequest::new(items, anchor()),
        Err(ShareUnavailable::NothingToShare)
    );
    assert_eq!(
        ShareRequest::new(Vec::new(), anchor()),
        Err(ShareUnavailable::NothingToShare)
    );
}

#[test]
fn duplicates_are_dropped_in_order_and_text_goes_out_as_given() {
    let request = ShareRequest::new(
        vec![
            ShareItem::Link("https://riseonly.net/c/1".into()),
            ShareItem::Text(" hello ".into()),
            ShareItem::Link("https://riseonly.net/c/1".into()),
            ShareItem::Text(" hello ".into()),
        ],
        anchor(),
    )
    .expect("two distinct items");
    assert_eq!(
        request.items(),
        &[
            ShareItem::Link("https://riseonly.net/c/1".into()),
            ShareItem::Text(" hello ".into()),
        ]
    );
}

#[test]
fn a_link_that_is_not_web_is_shared_as_text() {
    let request = ShareRequest::new(vec![ShareItem::Link("riseonly://chat/7".into())], anchor())
        .expect("one item");
    assert_eq!(request.items(), &[ShareItem::Text("riseonly://chat/7".into())]);

    let upper = ShareRequest::new(vec![ShareItem::Link("HTTPS://Riseonly.net".into())], anchor())
        .expect("one item");
    assert_eq!(upper.items(), &[ShareItem::Link("HTTPS://Riseonly.net".into())]);
}

#[test]
fn a_relative_file_is_refused_by_name() {
    let relative = PathBuf::from("photo.jpg");
    assert_eq!(
        ShareRequest::new(vec![ShareItem::File(relative.clone())], anchor()),
        Err(ShareUnavailable::UnusableFile(relative))
    );
}

#[test]
fn clipboard_text_joins_links_and_text_and_skips_files() {
    let request = ShareRequest::new(
        vec![
            ShareItem::File(absolute("a.jpg")),
            ShareItem::Link("https://riseonly.net/p/2".into()),
            ShareItem::Text("look".into()),
        ],
        anchor(),
    )
    .expect("three items");
    assert_eq!(
        request.clipboard_text().as_deref(),
        Some("https://riseonly.net/p/2\nlook")
    );

    let files_only =
        ShareRequest::new(vec![ShareItem::File(absolute("b.jpg"))], anchor()).expect("one file");
    assert_eq!(files_only.clipboard_text(), None);
}

#[test]
fn every_host_has_a_named_default_answer() {
    assert_eq!(
        unavailable_by_default(HostOs::Linux),
        ShareUnavailable::NoSystemSheet
    );
    assert_eq!(
        unavailable_by_default(HostOs::Windows),
        ShareUnavailable::BranchNotWritten(HostOs::Windows)
    );
    for host in HostOs::ALL {
        assert!(!unavailable_by_default(host).to_string().is_empty());
    }
}

#[test]
fn an_unavailable_sheet_hands_the_feature_text_to_copy() {
    let request = ShareRequest::new(
        vec![ShareItem::Link("https://riseonly.net/c/3".into())],
        anchor(),
    )
    .expect("one link");

    let shown = outcome_of(Ok(()), &request);
    assert_eq!(shown, ShareOutcome::Presented);
    assert_eq!(shown.support(), PlatformSupport::Performed);

    let refused = outcome_of(Err(ShareUnavailable::NoSystemSheet), &request);
    assert_eq!(
        refused,
        ShareOutcome::Unavailable {
            reason: ShareUnavailable::NoSystemSheet,
            copy: Some("https://riseonly.net/c/3".into()),
        }
    );
    assert_eq!(refused.support(), PlatformSupport::Unsupported);
}

#[test]
fn the_anchor_flips_into_a_bottom_left_view() {
    let rect = anchor_in_view(anchor(), 600.0, false);
    assert_eq!(rect.x, 40.0);
    assert_eq!(rect.y, 600.0 - 100.0 - 20.0);
    assert_eq!((rect.width, rect.height), (24.0, 20.0));
    assert!(rect.below_is_min_y);

    let flipped = anchor_in_view(anchor(), 600.0, true);
    assert_eq!(flipped.y, 100.0);
    assert!(!flipped.below_is_min_y);
}

#[test]
fn a_zero_sized_anchor_still_gives_the_sheet_a_point() {
    let point_anchor = bounds(point(px(10.0), px(10.0)), size(px(0.0), px(0.0)));
    let rect = anchor_in_view(point_anchor, 300.0, false);
    assert_eq!((rect.width, rect.height), (1.0, 1.0));
    assert_eq!(rect.y, 300.0 - 10.0 - 1.0);
}

fn recorder() -> (Rc<RefCell<Vec<ShareFinished>>>, ShareCompletion) {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&heard);
    let completion = ShareCompletion::new(move |finished| sink.borrow_mut().push(finished));
    (heard, completion)
}

#[test]
fn a_finished_share_reports_once_with_its_result() {
    let (heard, completion) = recorder();
    completion.finish(true);
    assert_eq!(*heard.borrow(), vec![ShareFinished { succeeded: true }]);
}

#[test]
fn a_completion_dropped_unfinished_reports_failure_once() {
    let (heard, completion) = recorder();
    drop(completion);
    assert_eq!(*heard.borrow(), vec![ShareFinished { succeeded: false }]);
}

#[test]
fn a_refusal_finishes_unsuccessful_before_it_returns() {
    let (heard, completion) = recorder();
    let answer = refuse(completion, unavailable_by_default(HostOs::Linux));
    assert_eq!(answer, Err(ShareUnavailable::NoSystemSheet));
    assert_eq!(*heard.borrow(), vec![ShareFinished { succeeded: false }]);
}

#[test]
fn an_incoming_item_is_its_file_when_it_has_one_and_blank_text_is_dropped() {
    let mut incoming = IncomingShare::default();
    assert!(incoming.is_empty());

    let file = absolute("clip.mp4");
    incoming.push_item(Some(file.clone()), Some(file.display().to_string()));
    incoming.push_item(None, Some(" \n".to_owned()));
    incoming.push_item(None, Some("hello".to_owned()));
    incoming.push_item(None, None);

    assert_eq!(incoming.files, vec![file]);
    assert_eq!(incoming.texts, vec!["hello".to_owned()]);
}

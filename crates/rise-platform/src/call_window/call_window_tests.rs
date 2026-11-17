use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Bounds, WindowBounds, WindowKind, point, px, size};

use crate::call_window::*;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::notifications::{InMemoryNotificationPresenter, NotificationTag};

fn screen() -> Bounds<gpui::Pixels> {
    Bounds::new(point(px(0.0), px(25.0)), size(px(1440.0), px(875.0)))
}

#[test]
fn floating_window_keeps_sixteen_by_nine() {
    let window = floating_window_size();
    let ratio = window.width.as_f32() / window.height.as_f32();
    assert!((ratio - 16.0 / 9.0).abs() < 0.001);
}

#[test]
fn floating_window_sits_inset_in_each_corner() {
    let visible = screen();
    let inset = px(SCREEN_EDGE_INSET);
    let top_leading = floating_window_bounds(visible, FloatingCorner::TopLeading);
    assert_eq!(top_leading.origin, point(inset, px(25.0) + inset));
    let bottom_trailing = floating_window_bounds(visible, FloatingCorner::BottomTrailing);
    assert_eq!(bottom_trailing.right(), visible.right() - inset);
    assert_eq!(bottom_trailing.bottom(), visible.bottom() - inset);
    let top_trailing = floating_window_bounds(visible, FloatingCorner::TopTrailing);
    assert_eq!(top_trailing.origin.y, top_leading.origin.y);
    assert_eq!(top_trailing.right(), visible.right() - inset);
    let bottom_leading = floating_window_bounds(visible, FloatingCorner::BottomLeading);
    assert_eq!(bottom_leading.origin.x, top_leading.origin.x);
    assert_eq!(bottom_leading.bottom(), visible.bottom() - inset);
}

#[test]
fn floating_window_stays_on_a_display_smaller_than_it() {
    let tiny = Bounds::new(point(px(100.0), px(100.0)), size(px(200.0), px(100.0)));
    for corner in FloatingCorner::ALL {
        let bounds = floating_window_bounds(tiny, corner);
        assert!(bounds.origin.x >= tiny.origin.x, "{corner:?}");
        assert!(bounds.origin.y >= tiny.origin.y, "{corner:?}");
    }
}

#[test]
fn each_corner_maps_back_to_itself() {
    let visible = screen();
    for corner in FloatingCorner::ALL {
        let bounds = floating_window_bounds(visible, corner);
        assert_eq!(nearest_corner(visible, bounds), corner);
    }
}

#[test]
fn a_dragged_window_takes_the_quarter_its_center_is_in() {
    let visible = screen();
    let dragged = Bounds::new(point(px(1000.0), px(60.0)), floating_window_size());
    assert_eq!(nearest_corner(visible, dragged), FloatingCorner::TopTrailing);
    assert_eq!(
        remember_floating_corner(visible, dragged),
        FloatingCorner::TopTrailing
    );
    assert_eq!(remembered_floating_corner(), FloatingCorner::TopTrailing);
}

#[test]
fn call_windows_float_by_kind_except_on_linux() {
    assert_eq!(call_window_kind(HostOs::MacOs), WindowKind::PopUp);
    assert_eq!(call_window_kind(HostOs::Windows), WindowKind::PopUp);
    assert_eq!(call_window_kind(HostOs::Linux), WindowKind::Normal);
}

#[test]
fn floating_window_opens_without_focus_and_fixed_size() {
    let options = floating_window_options_for(
        HostOs::MacOs,
        screen(),
        None,
        FloatingCorner::BottomLeading,
    );
    assert!(!options.focus);
    assert!(!options.is_resizable);
    assert!(!options.is_minimizable);
    assert!(options.titlebar.is_none());
    assert_eq!(
        options.window_bounds,
        Some(WindowBounds::Windowed(floating_window_bounds(
            screen(),
            FloatingCorner::BottomLeading
        )))
    );
}

#[test]
fn incoming_window_leaves_the_keyboard_with_the_active_app() {
    assert!(!incoming_window_options_for(HostOs::MacOs, screen(), None).focus);
    assert!(!incoming_window_options_for(HostOs::Windows, screen(), None).focus);
    assert!(!incoming_window_options_for(HostOs::Linux, screen(), None).focus);
}

#[test]
fn incoming_window_opens_without_focus_top_trailing() {
    let options = incoming_window_options_for(HostOs::Windows, screen(), None);
    assert!(!options.focus);
    let Some(WindowBounds::Windowed(bounds)) = options.window_bounds else {
        panic!("the incoming window opens windowed");
    };
    assert_eq!(bounds.size, incoming_window_size());
    assert_eq!(bounds.right(), screen().right() - px(SCREEN_EDGE_INSET));
    assert_eq!(bounds.origin.y, screen().origin.y + px(SCREEN_EDGE_INSET));
}

#[test]
fn enter_answers_and_escape_declines() {
    assert_eq!(
        IncomingCallResponse::from_key("enter"),
        Some(IncomingCallResponse::Answer)
    );
    assert_eq!(
        IncomingCallResponse::from_key("escape"),
        Some(IncomingCallResponse::Decline)
    );
    assert_eq!(IncomingCallResponse::from_key("space"), None);
}

#[test]
fn call_tag_carries_the_call_id() {
    let tag = call_notification_tag("c-42");
    assert!(tag.as_str().contains(&format!("{CALL_TAG_KEY_PREFIX}c-42")));
    assert_eq!(call_id_from_notification_tag(&tag), Some("c-42"));
    assert_eq!(
        call_id_from_notification_tag(&NotificationTag::conversation("call:c-42")),
        None
    );
    assert_eq!(
        call_id_from_notification_tag(&NotificationTag::unique("c-42")),
        None
    );
    assert_eq!(call_id_from_notification_tag(&call_notification_tag("")), None);
}

fn notice() -> IncomingCallNotice {
    IncomingCallNotice {
        call_id: "c-7".to_owned(),
        caller_name: "Caller".to_owned(),
        body: "Incoming call".to_owned(),
        answer_label: "Answer".to_owned(),
        decline_label: "Decline".to_owned(),
        avatar: None,
    }
}

#[test]
fn incoming_notification_has_answer_and_decline_under_the_call_tag() {
    let notification = notice().notification();
    assert_eq!(notification.tag, call_notification_tag("c-7"));
    let ids: Vec<&str> = notification
        .actions
        .iter()
        .map(|action| action.id.as_str())
        .collect();
    assert_eq!(ids, [CALL_ANSWER_ACTION_ID, CALL_DECLINE_ACTION_ID]);
    for id in ids {
        let response = IncomingCallResponse::from_action_id(id);
        assert_eq!(response.map(IncomingCallResponse::action_id), Some(id));
    }
}

#[test]
fn answering_anywhere_retracts_the_call_notification() {
    let presenter = InMemoryNotificationPresenter::new();
    assert_eq!(
        present_incoming_call(&presenter, &notice()),
        PlatformSupport::Performed
    );
    assert_eq!(
        retract_call_notification(&presenter, "c-7"),
        PlatformSupport::Performed
    );
    assert_eq!(presenter.presented().len(), 1);
    assert_eq!(presenter.dismissed(), vec![call_notification_tag("c-7")]);
}

fn fake_request(log: &Rc<RefCell<Vec<String>>>, call_id: &str) -> AttentionRequest {
    log.borrow_mut().push(format!("request {call_id}"));
    let log = Rc::clone(log);
    let call_id = call_id.to_owned();
    AttentionRequest::new(move || log.borrow_mut().push(format!("cancel {call_id}")))
}

#[test]
fn answer_or_end_cancels_the_attention_request() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut attention = CallAttention::new();
    assert_eq!(
        attention.begin_with("a", || Ok(fake_request(&log, "a"))),
        PlatformSupport::Performed
    );
    assert!(attention.is_requesting("a"));
    attention.end("a");
    assert!(!attention.is_requesting("a"));
    assert_eq!(*log.borrow(), ["request a", "cancel a"]);
}

#[test]
fn the_same_call_does_not_ask_twice() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut attention = CallAttention::new();
    let _ = attention.begin_with("a", || Ok(fake_request(&log, "a")));
    let _ = attention.begin_with("a", || Ok(fake_request(&log, "a")));
    assert_eq!(*log.borrow(), ["request a"]);
}

#[test]
fn a_new_call_cancels_the_old_request_first() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut attention = CallAttention::new();
    let _ = attention.begin_with("a", || Ok(fake_request(&log, "a")));
    let _ = attention.begin_with("b", || Ok(fake_request(&log, "b")));
    attention.end("a");
    assert_eq!(attention.requesting_call_id(), Some("b"));
    attention.end_all();
    assert_eq!(
        *log.borrow(),
        ["request a", "cancel a", "request b", "cancel b"]
    );
}

#[test]
fn a_refused_request_is_reported_and_cleared_on_end() {
    let mut attention = CallAttention::new();
    let support = attention.begin_with("a", || {
        Err(CallWindowUnavailable::WindowSystemLimit("no activation token"))
    });
    assert_eq!(support, PlatformSupport::Unsupported);
    assert!(attention.failure().is_some());
    assert!(!attention.is_requesting("a"));
    attention.end("a");
    assert!(attention.failure().is_none());
}

#[test]
fn dropping_the_tracker_cancels_the_held_request() {
    let log = Rc::new(RefCell::new(Vec::new()));
    {
        let mut attention = CallAttention::new();
        let _ = attention.begin_with("a", || Ok(fake_request(&log, "a")));
    }
    assert_eq!(*log.borrow(), ["request a", "cancel a"]);
}

#[test]
fn screen_point_lparam_packs_x_low_and_y_high() {
    assert_eq!(screen_point_lparam(0, 0), 0);
    assert_eq!(screen_point_lparam(100, 200), (200 << 16) | 100);
}

#[test]
fn screen_point_lparam_keeps_negative_monitor_coordinates() {
    let lparam = screen_point_lparam(-1920, -8);
    assert_eq!((lparam & 0xffff) as u16 as i16, -1920);
    assert_eq!(((lparam >> 16) & 0xffff) as u16 as i16, -8);
}

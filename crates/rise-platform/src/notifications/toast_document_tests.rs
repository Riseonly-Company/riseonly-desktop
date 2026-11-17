use std::path::Path;

use super::toast_document::*;
use crate::notifications::{LocalNotification, NotificationAction, NotificationTag};

fn message() -> LocalNotification {
    LocalNotification::new(NotificationTag::conversation("chat-1"), "Ayan", "a < b & \"c\"")
        .with_action(NotificationAction::reply("Reply"))
        .with_action(NotificationAction::mark_read("Mark read"))
}

#[test]
fn a_plain_toast_carries_title_body_and_buttons_and_no_rich_parts() {
    let xml = toast_xml(&message(), false);
    assert!(xml.starts_with(r#"<toast><visual><binding template="ToastGeneric">"#));
    assert!(xml.contains("<text>Ayan</text><text>a &lt; b &amp; &quot;c&quot;</text>"));
    assert!(xml.contains(&format!(
        r#"<action content="Reply" arguments="{}"/>"#,
        action_arguments(NotificationAction::REPLY_ID)
    )));
    assert!(xml.contains(&format!(
        r#"<action content="Mark read" arguments="{}"/>"#,
        action_arguments(NotificationAction::MARK_READ_ID)
    )));
    assert!(!xml.contains("<header"));
    assert!(!xml.contains("<image"));
    assert!(!xml.contains("<audio"));
    assert!(xml.ends_with("</toast>"));
}

#[test]
fn a_toast_without_buttons_has_no_actions_element() {
    let notification = LocalNotification::new(NotificationTag::unique("n-1"), "Title", "Body");
    assert!(!toast_xml(&notification, false).contains("<actions>"));
}

#[test]
fn a_silent_toast_turns_the_default_sound_off() {
    assert!(toast_xml(&message(), true).contains(r#"<audio silent="true"/></toast>"#));
}

#[test]
fn a_thread_becomes_a_header_named_after_the_title() {
    let xml = toast_xml(&message().with_thread("chat-1"), false);
    assert!(xml.starts_with(&format!(
        r#"<toast><header id="{}" title="Ayan" arguments="{HEADER_ARGUMENTS}"/><visual>"#,
        header_id("chat-1")
    )));
}

#[test]
fn an_avatar_becomes_a_circular_app_logo_override() {
    let xml = toast_xml(&message().with_avatar(r"C:\Users\a b\avatar.png"), false);
    assert!(xml.contains(
        r#"<image placement="appLogoOverride" hint-crop="circle" src="file:///C:/Users/a%20b/avatar.png"/></binding>"#
    ));
}

#[test]
fn an_avatar_windows_cannot_load_is_dropped_not_sent() {
    let xml = toast_xml(&message().with_avatar("avatars/relative.png"), false);
    assert!(!xml.contains("<image"));
}

#[test]
fn the_sender_adds_no_line_because_the_title_already_names_the_author() {
    let plain = toast_xml(&message(), false);
    assert_eq!(toast_xml(&message().with_sender("Ayan"), false), plain);
}

#[test]
fn control_characters_are_dropped_so_the_document_still_loads() {
    let notification =
        LocalNotification::new(NotificationTag::unique("n-1"), "T\u{1}", "line\nnext\u{7}");
    assert!(toast_xml(&notification, false).contains("<text>T</text><text>line\nnext</text>"));
}

#[test]
fn toast_tags_fit_the_cap_and_never_change_between_launches() {
    let tag = NotificationTag::conversation("chat-1");
    let windows_tag = toast_tag(&tag);
    assert_eq!(windows_tag.len(), 16);
    assert_eq!(windows_tag, toast_tag(&NotificationTag::conversation("chat-1")));
    assert_ne!(windows_tag, toast_tag(&NotificationTag::conversation("chat-2")));
    assert_ne!(windows_tag, toast_tag(&NotificationTag::unique("chat-1")));
    assert_eq!(header_id(""), "cbf29ce484222325");
}

#[test]
fn a_button_click_gives_its_id_and_body_or_header_clicks_give_none() {
    assert_eq!(
        action_from_arguments(&action_arguments(NotificationAction::REPLY_ID)).as_deref(),
        Some(NotificationAction::REPLY_ID)
    );
    assert_eq!(action_from_arguments(""), None);
    assert_eq!(action_from_arguments(HEADER_ARGUMENTS), None);
    assert_eq!(action_from_arguments(&action_arguments("")), None);
}

#[test]
fn file_uris_cover_drive_unc_and_verbatim_paths() {
    assert_eq!(
        file_uri(Path::new(r"D:\cache\a.png")).as_deref(),
        Some("file:///D:/cache/a.png")
    );
    assert_eq!(
        file_uri(Path::new(r"\\server\share\a #1.png")).as_deref(),
        Some("file://server/share/a%20%231.png")
    );
    assert_eq!(
        file_uri(Path::new(r"\\?\C:\cache\a.png")).as_deref(),
        Some("file:///C:/cache/a.png")
    );
    assert_eq!(
        file_uri(Path::new(r"\\?\UNC\server\share\a.png")).as_deref(),
        Some("file://server/share/a.png")
    );
    assert_eq!(
        file_uri(Path::new("C:\\cache\\\u{430}.png")).as_deref(),
        Some("file:///C:/cache/%D0%B0.png")
    );
}

#[test]
fn paths_that_are_not_absolute_windows_paths_give_no_uri() {
    assert_eq!(file_uri(Path::new("relative.png")), None);
    assert_eq!(file_uri(Path::new("/usr/share/a.png")), None);
    assert_eq!(file_uri(Path::new("C:relative.png")), None);
    assert_eq!(file_uri(Path::new(r"\\server")), None);
}

#[test]
fn a_text_input_button_gets_a_field_ahead_of_every_action() {
    let toast = toast_xml(
        &LocalNotification::new(NotificationTag::conversation("chat-1"), "Ayan", "hi")
            .with_action(NotificationAction::mark_read("Mark read"))
            .with_action(NotificationAction::reply("Reply").with_text_input("Send", "Say <hi>")),
        false,
    );
    let id = input_id(NotificationAction::REPLY_ID);
    let input = format!(r#"<input id="{id}" type="text" placeHolderContent="Say &lt;hi&gt;"/>"#);
    let send = format!(
        r#"<action content="Send" arguments="{}" hint-inputId="{id}"/>"#,
        action_arguments(NotificationAction::REPLY_ID)
    );
    assert!(toast.contains(&input), "{toast}");
    assert!(toast.contains(&send), "{toast}");
    assert!(
        toast.find(&input) < toast.find("<action "),
        "the schema orders inputs before actions: {toast}"
    );
    assert_eq!(toast.matches("<input ").count(), 1, "a plain button has no field");
}

#[test]
fn a_plain_button_stays_without_a_field() {
    let toast = toast_xml(&message(), false);
    assert!(!toast.contains("<input "), "{toast}");
    assert!(!toast.contains("hint-inputId"), "{toast}");
}

#[test]
fn an_empty_field_sends_no_text() {
    assert_eq!(typed_reply(""), None);
    assert_eq!(typed_reply("  \n"), None);
    assert_eq!(typed_reply(" on my way").as_deref(), Some(" on my way"));
}

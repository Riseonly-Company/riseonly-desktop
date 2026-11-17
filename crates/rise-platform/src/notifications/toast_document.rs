use std::path::Path;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::notifications::{LocalNotification, NotificationTag};

pub const TOAST_GROUP: &str = "riseonly";

pub const HEADER_ARGUMENTS: &str = "rise-header";

const ACTION_ARGUMENTS_PREFIX: &str = "rise-action-";

const INPUT_ID_PREFIX: &str = "rise-input-";

/// windows tag for a seam tag, a stable fnv1a 64 bit digest in 16 hex digits
/// fits the 64 char cap and survives restarts so removal by tag still finds the toast
pub fn toast_tag(tag: &NotificationTag) -> String {
    stable_digest(tag.as_str())
}

/// header id of a conversation, under the same cap
pub fn header_id(thread: &str) -> String {
    stable_digest(thread)
}

fn stable_digest(text: &str) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let digest = text
        .bytes()
        .fold(OFFSET_BASIS, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(PRIME));
    format!("{digest:016x}")
}

/// toast xml for one notification, title and body stay as given
/// title already names the person or chat so sender adds no line
/// silent drops the default sound
pub fn toast_xml(notification: &LocalNotification, silent: bool) -> String {
    let mut xml = String::from("<toast>");
    if let Some(thread) = notification.thread.as_deref() {
        xml.push_str(&format!(
            r#"<header id="{}" title="{}" arguments="{}"/>"#,
            escape(&header_id(thread)),
            escape(&notification.title),
            HEADER_ARGUMENTS,
        ));
    }
    xml.push_str(r#"<visual><binding template="ToastGeneric">"#);
    for text in [&notification.title, &notification.body] {
        xml.push_str(&format!("<text>{}</text>", escape(text)));
    }
    if let Some(source) = notification.avatar.as_deref().and_then(file_uri) {
        xml.push_str(&format!(
            r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#,
            escape(&source),
        ));
    }
    xml.push_str("</binding></visual>");
    if !notification.actions.is_empty() {
        xml.push_str("<actions>");
        for action in &notification.actions {
            if let Some(input) = &action.text_input {
                xml.push_str(&format!(
                    r#"<input id="{}" type="text" placeHolderContent="{}"/>"#,
                    escape(&input_id(&action.id)),
                    escape(&input.placeholder),
                ));
            }
        }
        for action in &notification.actions {
            match &action.text_input {
                Some(input) => xml.push_str(&format!(
                    r#"<action content="{}" arguments="{}" hint-inputId="{}"/>"#,
                    escape(&input.button_title),
                    escape(&action_arguments(&action.id)),
                    escape(&input_id(&action.id)),
                )),
                None => xml.push_str(&format!(
                    r#"<action content="{}" arguments="{}"/>"#,
                    escape(&action.label),
                    escape(&action_arguments(&action.id)),
                )),
            }
        }
        xml.push_str("</actions>");
    }
    if silent {
        xml.push_str(r#"<audio silent="true"/>"#);
    }
    xml.push_str("</toast>");
    xml
}

/// arguments a button carries, kept apart from the header ones
pub fn action_arguments(action_id: &str) -> String {
    format!("{ACTION_ARGUMENTS_PREFIX}{action_id}")
}

/// seam action id behind activation arguments
/// a button gives its id, a body or header click gives none
pub fn action_from_arguments(arguments: &str) -> Option<String> {
    arguments
        .strip_prefix(ACTION_ARGUMENTS_PREFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// toast input id of a text input button, key of its text in the activation UserInput
pub fn input_id(action_id: &str) -> String {
    format!("{INPUT_ID_PREFIX}{action_id}")
}

/// what the person typed, a send with an empty field has no text
/// so the feature opens the chat like on a host with no reply field
pub fn typed_reply(text: &str) -> Option<String> {
    (!trimming_whitespaces_and_newlines(text).is_empty()).then(|| text.to_owned())
}

/// file uri for an absolute windows path (drive or unc), the form unpackaged apps give toast images
/// anything else cant be loaded so its dropped
pub fn file_uri(path: &Path) -> Option<String> {
    let raw = path.to_str()?.replace('\\', "/");
    let raw = if let Some(unc) = raw.strip_prefix("//?/UNC/") {
        format!("//{unc}")
    } else if let Some(local) = raw.strip_prefix("//?/") {
        local.to_owned()
    } else {
        raw
    };
    if let Some(unc) = raw.strip_prefix("//") {
        let (host, rest) = unc.split_once('/')?;
        if host.is_empty() || rest.is_empty() {
            return None;
        }
        return Some(format!("file://{}/{}", percent_encode(host), percent_encode(rest)));
    }
    let bytes = raw.as_bytes();
    let has_drive = bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'/';
    if !has_drive {
        return None;
    }
    Some(format!("file:///{}:/{}", &raw[..1], percent_encode(&raw[3..])))
}

fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            control if control < ' ' && !matches!(control, '\t' | '\n' | '\r') => {}
            other => escaped.push(other),
        }
    }
    escaped
}

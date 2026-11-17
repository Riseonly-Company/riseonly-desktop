use super::clipboard_offer::*;

#[test]
fn host_only_text_offers_every_text_type_and_the_secret_hint() {
    let offer = ClipboardOffer::host_only_text("bot:token");
    let mimes: Vec<&str> = offer.mime_types().collect();
    assert_eq!(
        mimes,
        vec![
            "text/plain;charset=utf-8",
            "text/plain;charset=UTF-8",
            "UTF8_STRING",
            "text/plain",
            "x-kde-passwordManagerHint",
        ]
    );
    for mime in TEXT_MIMES {
        assert_eq!(offer.payload(mime), Some(b"bot:token".as_slice()));
    }
    assert_eq!(offer.payload(PASSWORD_MANAGER_HINT_MIME), Some(b"secret".as_slice()));
}

#[test]
fn url_offers_a_uri_list_line_first_and_the_text_after() {
    let offer = ClipboardOffer::url("https://riseonly.net/s/pack");
    assert_eq!(offer.mime_types().next(), Some("text/uri-list"));
    assert_eq!(
        offer.payload(URI_LIST_MIME),
        Some(b"https://riseonly.net/s/pack\r\n".as_slice())
    );
    assert_eq!(
        offer.payload("UTF8_STRING"),
        Some(b"https://riseonly.net/s/pack".as_slice())
    );
    assert_eq!(offer.payload(PASSWORD_MANAGER_HINT_MIME), None);
}

#[test]
fn a_type_not_offered_has_no_payload() {
    let offer = ClipboardOffer::host_only_text("x");
    assert_eq!(offer.payload("text/html"), None);
    assert_eq!(offer.payload("TEXT/PLAIN"), None);
}

#[test]
fn text_keeps_its_bytes_unchanged() {
    let offer = ClipboardOffer::host_only_text(" tok\u{e9}n \n");
    assert_eq!(offer.payload("text/plain"), Some(" tok\u{e9}n \n".as_bytes()));
}

#[test]
fn the_session_follows_the_gpui_choice() {
    assert_eq!(clipboard_session(false, true, true), ClipboardSession::Wayland);
    assert_eq!(clipboard_session(false, true, false), ClipboardSession::Wayland);
    assert_eq!(clipboard_session(false, false, true), ClipboardSession::X11);
    assert_eq!(clipboard_session(true, true, true), ClipboardSession::Unreachable);
    assert_eq!(clipboard_session(false, false, false), ClipboardSession::Unreachable);
}

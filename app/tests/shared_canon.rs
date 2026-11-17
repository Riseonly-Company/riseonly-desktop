use rise_perm::{ALL_PERMISSIONS, BIT_COUNT, BITS, ChatPermissions, DEFAULT_EVERYONE, bit_by_name};

#[test]
fn canon_reaches_the_desktop() {
    assert_eq!(BIT_COUNT, BITS.len(), "the bit table and its length disagree");
    assert!(BIT_COUNT > 0, "the canon arrived empty");
}

#[test]
fn default_everyone_cannot_mention_everyone() {
    let mention = bit_by_name("mention_everyone").expect("the canon has the `mention_everyone` bit");
    let everyone = ChatPermissions::from_bits(DEFAULT_EVERYONE);
    assert!(
        !everyone.contains(ChatPermissions::from_bits(1 << mention.position)),
        "DEFAULT_EVERYONE gained the right to mention everyone"
    );
}

#[test]
fn every_bit_is_inside_the_full_mask() {
    let all = ChatPermissions::from_bits(ALL_PERMISSIONS);
    for bit in &BITS {
        let single = ChatPermissions::from_bits(1 << bit.position);
        assert!(
            all.contains(single),
            "bit `{}` is outside ALL_PERMISSIONS",
            bit.name
        );
    }
}

#[test]
fn every_connected_crate_is_reachable() {
    assert!(rise_moderation::ReportReason::ALL.len() > 1, "moderation");
    assert!(!rise_rtc::ROOM_MODE_RULES.is_empty(), "rtc");
    assert!(rise_music::EFFECT_COUNT > 1, "music");
    assert!(rise_limits::LIMIT_COUNT > 1, "limits");
    assert!(rise_events::WS_EVENT_COUNT > 1, "events");
    assert!(
        !rise_catalog::VACANCY_CURRENCIES_DEFAULT.wire().is_empty(),
        "catalog"
    );
    assert!(!rise_chat::MESSAGE_DECODE_KEYS.is_empty(), "chat");
    assert!(rise_endpoints::TIMEOUT_DEFAULT_MS > 0, "endpoints");
    assert!(!rise_regions::FLAG_FALLBACK.is_empty(), "regions");
}

#[test]
fn locales_arrived_and_parse() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/locales");
    let mut files = 0;
    for entry in std::fs::read_dir(directory).expect("the locale directory exists")
    {
        let path = entry.expect("the directory entry is readable").path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("the locale is readable");
        let value: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: does not parse: {error}", path.display()));
        assert!(
            value.as_object().is_some_and(|map| !map.is_empty()),
            "{}: empty dictionary",
            path.display()
        );
        files += 1;
    }
    assert!(
        files >= 41,
        "{files} locales arrived, expected at least 41"
    );
}

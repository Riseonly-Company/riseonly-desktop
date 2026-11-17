use super::*;

#[test]
fn colour_follows_the_canon_hash_and_palette() {
    for key in ["u-42", "none", "0hjyxbg8", "\u{1F600}"] {
        let index = rise_avatar::palette_index(key);
        assert_eq!(AvatarPalette::color_for_chat(key), AvatarPalette::palette()[index], "{key:?}");
    }
}

#[test]
fn a_rename_keeps_the_colour_and_a_blank_id_falls_back_to_the_name() {
    assert_eq!(
        AvatarPalette::color_for_user(Some("u-1"), Some("Alice")),
        AvatarPalette::color_for_user(Some("u-1"), Some("Bob"))
    );
    assert_eq!(
        AvatarPalette::color_for_user(Some("   "), Some("Alice")),
        AvatarPalette::color_for_chat("Alice")
    );
    assert_eq!(AvatarPalette::color_for_user(None, None), AvatarPalette::color_for_chat("none"));
}

#[test]
fn the_scoped_colour_hashes_scope_separator_and_entity_as_one_key() {
    let joined = AvatarPalette::color_for_chat("c-1:u-2");
    assert_eq!(AvatarPalette::color_in("c-1", Some("u-2")), joined);
    assert_eq!(AvatarPalette::color_in_chat("c-1", Some("u-2")), joined);
    assert_eq!(
        AvatarPalette::color_in_chat("c-1", Some("")),
        AvatarPalette::color_in("c-1", None)
    );
}

#[test]
fn the_letter_is_the_first_grapheme_uppercased() {
    assert_eq!(AvatarPalette::display_letter(Some("  aianov")), "A");
    assert_eq!(AvatarPalette::display_letter(Some("\u{434}\u{443}\u{43b}\u{430}\u{442}")), "\u{414}");
    assert_eq!(AvatarPalette::display_letter(Some("   ")), "?");
    assert_eq!(AvatarPalette::display_letter(None), "?");
}

#[test]
fn generated_server_avatars_are_drawn_locally() {
    let generated = "https://cdn.example/public/avatars/generated/2b1f.3_03_d094.png";
    assert_eq!(AvatarPalette::resolved_logo_url(Some(generated)), None);
    assert_eq!(AvatarPalette::generated_avatar_owner_id(Some(generated)), Some("2b1f".to_owned()));
    assert_eq!(AvatarPalette::generated_avatar_owner_id(Some("https://cdn.example/a.png")), None);
}

#[test]
fn only_http_and_local_logos_resolve() {
    assert_eq!(
        AvatarPalette::resolved_logo_url(Some(" https://cdn.example/a.png ")),
        Some("https://cdn.example/a.png".to_owned())
    );
    assert_eq!(AvatarPalette::resolved_logo_url(Some("/tmp/a.png")), Some("/tmp/a.png".to_owned()));
    assert_eq!(AvatarPalette::resolved_logo_url(Some("avatars/1.png")), None);
    assert_eq!(AvatarPalette::resolved_logo_url(Some("")), None);
    assert_eq!(AvatarPalette::resolved_logo_url(None), None);
}

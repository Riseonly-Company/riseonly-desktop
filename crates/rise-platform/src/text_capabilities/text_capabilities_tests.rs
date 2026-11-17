use std::path::PathBuf;

use crate::host_os::HostOs;
use crate::text_capabilities::*;

#[test]
fn macos_draws_flag_emoji() {
    assert!(
        renders_flag_emoji_on(HostOs::MacOs),
        "Apple Color Emoji is a system font; the flags are always there"
    );
}

#[test]
fn everywhere_else_falls_back_to_artwork() {
    for host in [HostOs::Linux, HostOs::Windows] {
        assert!(
            !renders_flag_emoji_on(host),
            "a machine with no flag glyphs must get the shipped SVG, not two letters in boxes"
        );
    }
}

#[test]
fn only_the_cbdt_noto_build_counts_as_a_flag_font() {
    assert!(is_flag_emoji_font_file("NotoColorEmoji.ttf"));
    assert!(is_flag_emoji_font_file("notocoloremoji.TTF"));
    assert!(!is_flag_emoji_font_file("Noto-COLRv1.ttf"), "swash draws no COLRv1");
    assert!(!is_flag_emoji_font_file("NotoEmoji-Regular.ttf"), "the one-colour cut");
    assert!(!is_flag_emoji_font_file("TwemojiMozilla.ttf"), "gpui masks it in one colour");
    assert!(!is_flag_emoji_font_file("NotoColorEmoji.ttf.bak"), "not a font file");
}

#[test]
fn walk_finds_a_nested_colour_emoji_font() {
    let root = std::env::temp_dir().join(format!("rise-flag-{}", std::process::id()));
    let nested = root.join("truetype").join("noto");
    std::fs::create_dir_all(&nested).expect("temp dir");
    std::fs::write(root.join("DejaVuSans.ttf"), b"").expect("font file");
    let before = has_flag_emoji_font(&[root.join("missing"), root.clone()]);
    std::fs::write(nested.join("NotoColorEmoji.ttf"), b"").expect("font file");
    let after = has_flag_emoji_font(&[root.clone()]);
    let _ = std::fs::remove_dir_all(&root);
    assert!(!before, "no colour emoji font keeps the shipped artwork");
    assert!(after, "Noto Color Emoji draws the flags");
}

#[test]
fn every_host_names_a_real_monospaced_family() {
    assert_eq!(monospaced_family_on(HostOs::MacOs), "Menlo", "SF Mono is not public");
    assert_eq!(monospaced_family_on(HostOs::Windows), "Consolas");
    assert_eq!(monospaced_family_on(HostOs::Linux), "DejaVu Sans Mono");
    for host in HostOs::ALL {
        assert_ne!(
            monospaced_family_on(host),
            "monospace",
            "gpui matches names exactly; a generic alias falls back to the UI font"
        );
    }
}

#[test]
fn linux_takes_the_best_installed_family() {
    assert_eq!(
        pick_linux_monospaced(&["Noto Sans Mono", "Liberation Mono"]),
        "Liberation Mono"
    );
    assert_eq!(pick_linux_monospaced(&["Noto Sans Mono"]), "Noto Sans Mono");
    assert_eq!(
        pick_linux_monospaced(&[]),
        "DejaVu Sans Mono",
        "nothing found keeps the first choice; gpui draws the UI font in its place"
    );
}

#[test]
fn font_file_names_map_to_families() {
    assert_eq!(monospaced_family_of_file("DejaVuSansMono.ttf"), Some("DejaVu Sans Mono"));
    assert_eq!(monospaced_family_of_file("DejaVuSansMono-Bold.ttf"), Some("DejaVu Sans Mono"));
    assert_eq!(
        monospaced_family_of_file("LiberationMono-Regular.ttf"),
        Some("Liberation Mono")
    );
    assert_eq!(monospaced_family_of_file("NotoSansMono-Regular.otf"), Some("Noto Sans Mono"));
    assert_eq!(monospaced_family_of_file("DejaVuSans.ttf"), None, "the proportional cut");
    assert_eq!(monospaced_family_of_file("DejaVuSansMono.ttf.bak"), None, "not a font file");
    assert_eq!(monospaced_family_of_file("NotoMono-Regular.ttf"), None, "family Noto Mono");
}

#[test]
fn font_dirs_follow_fontdb_xdg_and_flatpak() {
    let dirs = linux_font_dirs(
        Some("/home/a/"),
        Some("/home/a/.local/share"),
        Some("/usr/local/share:/usr/share/:relative/dir::/var/lib/flatpak/exports/share"),
    );
    let expected: Vec<PathBuf> = [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/home/a/.fonts",
        "/home/a/.local/share/fonts",
        "/var/lib/flatpak/exports/share/fonts",
        "/run/host/fonts",
        "/run/host/user-fonts",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    assert_eq!(dirs, expected, "duplicates dropped, a relative XDG entry ignored by the spec");
}

#[test]
fn font_dirs_without_home_keep_system_dirs() {
    let dirs = linux_font_dirs(None, Some("relative"), None);
    assert_eq!(dirs.len(), 4, "system dirs and the two Flatpak host dirs: {dirs:?}");
}

#[test]
fn walk_finds_nested_and_ignores_missing_dirs() {
    let root = std::env::temp_dir().join(format!("rise-mono-{}", std::process::id()));
    let nested = root.join("truetype").join("liberation");
    std::fs::create_dir_all(&nested).expect("temp dir");
    std::fs::write(nested.join("LiberationMono-Regular.ttf"), b"").expect("font file");
    std::fs::write(root.join("DejaVuSans.ttf"), b"").expect("font file");
    let found = installed_monospaced_families(&[root.join("missing"), root.clone()]);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(found, vec!["Liberation Mono"]);
}

#[test]
fn an_empty_text_draws() {
    assert!(draws_text(""), "iOS walks no character and answers true");
}

#[test]
fn a_failed_read_shows_the_text() {
    let host = HostOs::current();
    assert!(drawable_or_assumed(Ok(true)));
    assert!(!drawable_or_assumed(Ok(false)), "a box is a box");
    assert!(
        drawable_or_assumed(Err(TextCoverageUnavailable::BranchNotWritten { host })),
        "a host that cannot tell keeps the symbol, the behaviour before the check"
    );
    assert!(drawable_or_assumed(Err(TextCoverageUnavailable::SystemCallFailed {
        host,
        call: "CTFontCreateForString",
    })));
}

#[test]
fn the_answer_is_stable_across_calls() {
    for text in ["$", "\u{20B8}", "\u{0378}", "AED \u{062F}.\u{0625}"] {
        assert_eq!(draws_text(text), draws_text(text), "the cache returns the first answer: {text}");
    }
}

#[test]
fn macos_draws_latin_and_boxes_an_unassigned_code_point() {
    if HostOs::current() != HostOs::MacOs {
        return;
    }
    assert!(draws_text("$"), "the system font draws a dollar sign");
    assert!(draws_text("\u{20AC}"), "the euro sign is in SF");
    assert!(!draws_text("\u{0378}"), "an unassigned code point reaches LastResort");
}


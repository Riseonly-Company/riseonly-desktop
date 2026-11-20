use crate::host_os::HostOs;

struct FallbackFace {
    asset: &'static str,
    sample: char,
}

const FALLBACK_FACES: &[FallbackFace] = &[
    FallbackFace { asset: "fonts/NotoSansCJK-Regular.ttc", sample: '\u{4e2d}' },
    FallbackFace { asset: "fonts/NotoSansThai-Regular.ttf", sample: '\u{0e01}' },
    FallbackFace { asset: "fonts/NotoSansDevanagari-Regular.ttf", sample: '\u{0915}' },
    FallbackFace { asset: "fonts/NotoSansBengali-Regular.ttf", sample: '\u{0995}' },
    FallbackFace { asset: "fonts/NotoSansTamil-Regular.ttf", sample: '\u{0b95}' },
    FallbackFace { asset: "fonts/NotoSansGeorgian-Regular.ttf", sample: '\u{10d0}' },
    FallbackFace { asset: "fonts/NotoSansHebrew-Regular.ttf", sample: '\u{05d0}' },
    FallbackFace { asset: "fonts/NotoSansArabic-Regular.ttf", sample: '\u{0628}' },
];

pub fn uncovered_faces() -> Vec<&'static str> {
    let samples: Vec<char> = FALLBACK_FACES.iter().map(|face| face.sample).collect();
    FALLBACK_FACES
        .iter()
        .zip(super::os::covered(&samples))
        .filter(|(_, covered)| !covered)
        .map(|(face, _)| face.asset)
        .collect()
}

/// color emoji face the app bundles for a host whose own emoji set the app replaces
pub const fn bundled_emoji_face(host: HostOs) -> Option<&'static str> {
    match host {
        HostOs::Windows => Some("fonts/TwemojiMozilla.ttf"),
        HostOs::MacOs | HostOs::Linux => None,
    }
}

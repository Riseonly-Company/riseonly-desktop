use std::borrow::Cow;

use gpui::{App, Font, FontFeatures, FontStyle, FontWeight};
use rise_theme::{FONT_FAMILY, SHIPPED_WEIGHTS};

/// One face the app bundle is expected to carry.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BundledFace {
    pub asset_path: &'static str,
    pub weight: FontWeight,
}

/// The faces the product ships, and the only weights `rise_theme::Typography`
/// will ever hand to gpui.
///
/// Six because the reference draws with six: light, regular, medium, semibold,
/// bold and heavy. A face missing here does not fail loudly at the call site —
/// the text stack quietly substitutes another family, and the substitute differs
/// per OS, which is the exact failure shipping our own font is meant to remove.
/// That is why [`load_bundled_fonts`] returns what is missing instead of `()`.
pub const BUNDLED_FACES: [BundledFace; 6] = [
    BundledFace {
        asset_path: "fonts/Inter-Light.ttf",
        weight: FontWeight::LIGHT,
    },
    BundledFace {
        asset_path: "fonts/Inter-Regular.ttf",
        weight: FontWeight::NORMAL,
    },
    BundledFace {
        asset_path: "fonts/Inter-Medium.ttf",
        weight: FontWeight::MEDIUM,
    },
    BundledFace {
        asset_path: "fonts/Inter-SemiBold.ttf",
        weight: FontWeight::SEMIBOLD,
    },
    BundledFace {
        asset_path: "fonts/Inter-Bold.ttf",
        weight: FontWeight::BOLD,
    },
    BundledFace {
        asset_path: "fonts/Inter-Black.ttf",
        weight: FontWeight::BLACK,
    },
];

/// What actually reached the text system.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct FontLoad {
    pub loaded: Vec<&'static str>,
    pub missing: Vec<&'static str>,
    /// Set when the text system refused the bytes it was given — a truncated
    /// download or a file the bundler replaced with an HTML error page reaches
    /// here rather than being discovered as blank text three screens later.
    pub rejected: Option<String>,
    /// Weights that came back as another weight's face.
    ///
    /// The one failure in this file that a unit test cannot see. Inter's Light,
    /// Medium, SemiBold and Black each carry their own legacy family name (see
    /// `crates/rise-ui/tests/font_faces.rs`), so a text stack that reads the
    /// legacy record rather than the typographic one answers Regular for weight
    /// 500 and 600 — without an error, and with no way to notice except by
    /// looking at the pixels. This is measured against the real text system at
    /// startup, because `TestAppContext` installs a no-op one that answers
    /// `FontId(1)` to every query including a family that does not exist.
    pub misresolved: Vec<(FontWeight, FontWeight)>,
}

impl FontLoad {
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty() && self.rejected.is_none() && self.misresolved.is_empty()
    }
}

/// Reads every bundled face through the app's asset source and hands them to the
/// text system.
///
/// Runs once at startup, before the first window: adding a font later does not
/// re-shape text that has already been laid out.
pub fn load_bundled_fonts(cx: &App) -> FontLoad {
    let source = cx.asset_source().clone();
    let mut report = FontLoad::default();
    let mut bytes: Vec<Cow<'static, [u8]>> = Vec::with_capacity(BUNDLED_FACES.len());

    for face in BUNDLED_FACES {
        match source.load(face.asset_path) {
            Ok(Some(data)) if !data.is_empty() => {
                bytes.push(data);
                report.loaded.push(face.asset_path);
            }
            _ => report.missing.push(face.asset_path),
        }
    }

    if !bytes.is_empty()
        && let Err(error) = cx.text_system().add_fonts(bytes)
    {
        report.rejected = Some(error.to_string());
    }

    if report.rejected.is_none() && report.missing.is_empty() {
        report.misresolved = collapsed_weights(cx);
    }

    report
}

/// Which shipped weights the text system answered with somebody else's face.
///
/// Compared by `FontId` rather than by reading the weight back: an id is minted
/// per distinct resolved face, so two weights sharing one id is exactly the
/// collapse this looks for, and it does not depend on the order the weights are
/// asked for.
fn collapsed_weights(cx: &App) -> Vec<(FontWeight, FontWeight)> {
    let resolved: Vec<_> = SHIPPED_WEIGHTS
        .iter()
        .map(|weight| (*weight, cx.text_system().resolve_font(&font_for(*weight))))
        .collect();

    let mut collapsed = Vec::new();
    for (index, (weight, id)) in resolved.iter().enumerate() {
        for (other_weight, other_id) in &resolved[index + 1..] {
            if id == other_id {
                collapsed.push((*weight, *other_weight));
            }
        }
    }
    collapsed
}

fn font_for(weight: FontWeight) -> Font {
    Font {
        family: FONT_FAMILY.into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("assets")
    }

    #[test]
    fn a_face_is_bundled_for_every_weight_the_theme_will_ask_for() {
        for weight in SHIPPED_WEIGHTS {
            assert!(
                BUNDLED_FACES.iter().any(|face| face.weight == weight),
                "{weight:?} is a shipped weight with no bundled face; \
                 gpui would substitute a different family on each OS"
            );
        }
    }

    #[test]
    fn no_two_faces_claim_the_same_weight_or_the_same_file() {
        for (index, face) in BUNDLED_FACES.iter().enumerate() {
            for other in &BUNDLED_FACES[index + 1..] {
                assert_ne!(face.weight, other.weight);
                assert_ne!(face.asset_path, other.asset_path);
            }
        }
    }

    #[test]
    fn every_bundled_face_exists_in_the_repository_and_is_a_real_font() {
        let assets = repo_assets();
        if !assets.join("fonts").is_dir() {
            eprintln!("skipping: {} is not populated yet", assets.display());
            return;
        }

        for face in BUNDLED_FACES {
            let path = assets.join(face.asset_path);
            let bytes =
                std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));

            assert!(
                bytes.len() > 10_000,
                "{} is {} bytes, which is an error page rather than a font",
                path.display(),
                bytes.len()
            );

            // The sfnt tag. A TrueType outline file starts 0x00010000, an
            // Apple-flavoured one "true", a CFF/OpenType one "OTTO". Anything
            // else is not something the text system will accept.
            let tag = &bytes[..4];
            assert!(
                tag == [0x00, 0x01, 0x00, 0x00] || tag == b"true" || tag == b"OTTO",
                "{} does not start with an sfnt tag: {tag:?}",
                path.display()
            );
        }
    }

    #[test]
    fn the_licence_that_lets_us_redistribute_the_font_ships_with_it() {
        let assets = repo_assets();
        if !assets.join("fonts").is_dir() {
            eprintln!("skipping: {} is not populated yet", assets.display());
            return;
        }

        let licence = assets.join("fonts/OFL.txt");
        let text = std::fs::read_to_string(&licence)
            .unwrap_or_else(|error| panic!("{}: {error}", licence.display()));

        assert!(
            text.contains("SIL OPEN FONT LICENSE"),
            "{} is not the licence text",
            licence.display()
        );
    }
}

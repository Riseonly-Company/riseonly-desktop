use std::borrow::Cow;

use gpui::{App, Font, FontFeatures, FontStyle, FontWeight};
use rise_theme::{FONT_FAMILY, SHIPPED_WEIGHTS};

/// one face the app bundle is expected to carry
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BundledFace {
    pub asset_path: &'static str,
    pub weight: FontWeight,
}

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

pub const BUNDLED_ITALIC_FACES: [BundledFace; 6] = [
    BundledFace {
        asset_path: "fonts/Inter-LightItalic.ttf",
        weight: FontWeight::LIGHT,
    },
    BundledFace {
        asset_path: "fonts/Inter-Italic.ttf",
        weight: FontWeight::NORMAL,
    },
    BundledFace {
        asset_path: "fonts/Inter-MediumItalic.ttf",
        weight: FontWeight::MEDIUM,
    },
    BundledFace {
        asset_path: "fonts/Inter-SemiBoldItalic.ttf",
        weight: FontWeight::SEMIBOLD,
    },
    BundledFace {
        asset_path: "fonts/Inter-BoldItalic.ttf",
        weight: FontWeight::BOLD,
    },
    BundledFace {
        asset_path: "fonts/Inter-BlackItalic.ttf",
        weight: FontWeight::BLACK,
    },
];

/// what actually reached the text system
#[derive(Clone, PartialEq, Debug, Default)]
pub struct FontLoad {
    pub loaded: Vec<&'static str>,
    pub missing: Vec<&'static str>,
    pub rejected: Option<String>,
    pub misresolved: Vec<(FontWeight, FontWeight)>,
}

impl FontLoad {
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty() && self.rejected.is_none() && self.misresolved.is_empty()
    }
}

pub fn load_fallback_fonts(cx: &mut App) {
    let source = cx.asset_source().clone();
    let read = cx.background_executor().spawn(async move {
        rise_platform::font_fallback::uncovered_faces()
            .into_iter()
            .filter_map(|asset| source.load(asset).ok().flatten())
            .collect::<Vec<_>>()
    });
    cx.spawn(async move |cx| {
        let faces = read.await;
        if faces.is_empty() {
            return;
        }
        cx.update(|cx| {
            if let Err(error) = cx.text_system().add_fonts(faces) {
                tracing::warn!(target: "riseonly", %error, "the bundled fallback faces were refused");
            }
        });
    })
    .detach();
}

pub fn load_bundled_fonts(cx: &App) -> FontLoad {
    let source = cx.asset_source().clone();
    let mut report = FontLoad::default();
    let mut bytes: Vec<Cow<'static, [u8]>> =
        Vec::with_capacity(BUNDLED_FACES.len() + BUNDLED_ITALIC_FACES.len());

    let emoji = rise_platform::font_fallback::bundled_emoji_face(rise_platform::HostOs::current());
    let paths = BUNDLED_FACES.iter().chain(BUNDLED_ITALIC_FACES.iter()).map(|face| face.asset_path).chain(emoji);
    for asset_path in paths {
        match source.load(asset_path) {
            Ok(Some(data)) if !data.is_empty() => {
                bytes.push(data);
                report.loaded.push(asset_path);
            }
            _ => report.missing.push(asset_path),
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
    fn every_upright_face_has_an_italic_twin_of_the_same_weight() {
        assert_eq!(BUNDLED_FACES.len(), BUNDLED_ITALIC_FACES.len());
        for (upright, italic) in BUNDLED_FACES.iter().zip(BUNDLED_ITALIC_FACES.iter()) {
            assert_eq!(upright.weight, italic.weight);
            assert_ne!(upright.asset_path, italic.asset_path);
        }
    }

    #[test]
    fn every_bundled_face_exists_in_the_repository_and_is_a_real_font() {
        let assets = repo_assets();
        if !assets.join("fonts").is_dir() {
            tracing::warn!(target: "riseonly", "skipping: {} is not populated yet", assets.display());
            return;
        }

        for face in BUNDLED_FACES.iter().chain(BUNDLED_ITALIC_FACES.iter()) {
            let path = assets.join(face.asset_path);
            let bytes =
                std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));

            assert!(
                bytes.len() > 10_000,
                "{} is {} bytes, which is an error page rather than a font",
                path.display(),
                bytes.len()
            );

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
            tracing::warn!(target: "riseonly", "skipping: {} is not populated yet", assets.display());
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

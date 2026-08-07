use gpui::{App, AppContext, Entity, Pixels};
use rise_media::lottie::lottie_view::{LottieRequest, LottieView, spawn_lottie};

/// The animations the product ships, by the name the reference calls them.
///
/// Names, not paths, because that is what a screen ports across: the Swift says
/// `LottieView(name: "magic_crystal_ball")` and `model.animationName` returns
/// `"hello"`, so a screen that spelled a path would stop diffing against its
/// reference.
pub struct Animation;

impl Animation {
    pub const CRYSTAL_BALL: &'static str = "magic_crystal_ball";
    pub const COOL_EMOJI: &'static str = "cool_emoji";
    pub const SHIELD: &'static str = "shield";
    pub const PARTY: &'static str = "party";

    /// The sign-in / sign-up wizard's mascot, one pose per step.
    pub const HELLO: &'static str = "red_panda/hello";
    pub const QUITE: &'static str = "red_panda/quite";
    pub const GREETINGS: &'static str = "red_panda/greetings";
    pub const WHAT: &'static str = "red_panda/what";
    pub const LIKE: &'static str = "red_panda/like";
    pub const NO: &'static str = "red_panda/no";
    pub const WORK: &'static str = "red_panda/work";
    pub const HELP: &'static str = "red_panda/help";
    pub const GIFT: &'static str = "red_panda/gift";

    pub const ALL: [&'static str; 13] = [
        Self::CRYSTAL_BALL,
        Self::COOL_EMOJI,
        Self::SHIELD,
        Self::PARTY,
        Self::HELLO,
        Self::QUITE,
        Self::GREETINGS,
        Self::WHAT,
        Self::LIKE,
        Self::NO,
        Self::WORK,
        Self::HELP,
        Self::GIFT,
    ];

    pub fn asset_path(name: &str) -> String {
        format!("animations/{name}.json")
    }
}

/// What one on-screen animation may hold in decoded frames.
///
/// One plays at a time on both screens that use these, so this is the feature's
/// ceiling rather than a per-instance hint. `crates/rise-media/tests/
/// lottie_assets.rs` fails if a shipped animation does not fit it.
const BUDGET_BYTES: u64 = 64 * 1024 * 1024;

/// Starts an animation and hands back the view it will appear in.
///
/// The view exists immediately at its final size and fills in when the raster
/// finishes, so the screen's layout never moves — the same contract a skeleton
/// has. A missing or unrenderable animation leaves it empty rather than drawing
/// a placeholder that would read as a design decision.
pub fn lottie(name: &str, side: Pixels, scale_factor: f32, cx: &mut App) -> Entity<LottieView> {
    let path = Animation::asset_path(name);
    let json = cx
        .asset_source()
        .load(&path)
        .ok()
        .flatten()
        .map(|bytes| bytes.into_owned());

    let Some(json) = json else {
        tracing::error!(target: "riseonly", "no animation at {path}");
        return cx.new(|_| LottieView::empty(side));
    };

    spawn_lottie(
        LottieRequest {
            json,
            cache_key: name.to_owned(),
            side,
            scale_factor,
            budget_bytes: BUDGET_BYTES,
        },
        open_rasterizer,
        cx,
    )
}

/// The one place the rasteriser is chosen.
///
/// Behind the `rlottie` feature: without the vendored source there is no
/// rasteriser, and the screens draw nothing rather than failing to build. That
/// is stated here rather than discovered — `cargo make vendor-rlottie` is the
/// fix, and the log line says so.
#[allow(unused_variables)]
fn open_rasterizer(
    json: &[u8],
    cache_key: &str,
) -> Option<Box<dyn rise_media::lottie::sequence::LottieRasterizer>> {
    #[cfg(feature = "rlottie")]
    {
        match rise_media::lottie::rlottie_backend::RlottieRasterizer::open(json, cache_key) {
            Ok(rasterizer) => Some(Box::new(rasterizer)),
            Err(error) => {
                tracing::error!(target: "riseonly", "rlottie refused {cache_key}: {error:?}");
                None
            }
        }
    }
    #[cfg(not(feature = "rlottie"))]
    {
        tracing::warn!(
            target: "riseonly",
            "built without the rlottie feature; {cache_key} will not animate — \
             run `cargo make vendor-rlottie` and rebuild"
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn every_named_animation_has_a_file_in_the_repository() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
        if !root.is_dir() {
            return;
        }

        for name in Animation::ALL {
            let path = root.join(Animation::asset_path(name));
            assert!(
                path.is_file(),
                "{name} is named by a screen but has no file at {}",
                path.display()
            );
        }
    }

    #[test]
    fn the_asset_path_stays_inside_the_asset_root() {
        for name in Animation::ALL {
            let path = Animation::asset_path(name);
            assert!(!path.contains(".."), "{path} escapes the asset root");
            assert!(path.starts_with("animations/"));
            assert!(path.ends_with(".json"));
        }
    }

    #[test]
    fn no_two_names_collide() {
        let mut names = Animation::ALL.to_vec();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}

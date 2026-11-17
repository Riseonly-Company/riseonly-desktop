#![cfg(feature = "rlottie")]

use std::path::PathBuf;
use std::sync::Arc;

use rise_media::lottie::raster_policy::SequenceKey;
use rise_media::lottie::rlottie_backend::RlottieRasterizer;
use rise_media::lottie::sequence::{DecodedFrameCache, FrameSequence, LottieRasterizer};
use rise_media::lottie::{container, raster_policy};

fn animations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/animations")
}

const SHIPPED: &[&str] = &[
    "magic_crystal_ball",
    "cool_emoji",
    "shield",
    "party",
    "red_panda/hello",
    "red_panda/quite",
    "red_panda/greetings",
    "red_panda/what",
    "red_panda/like",
    "red_panda/no",
    "red_panda/work",
    "red_panda/help",
    "red_panda/gift",
];

fn open(name: &str) -> Option<FrameSequence> {
    let path = animations_dir().join(format!("{name}.json"));
    let bytes = std::fs::read(&path).ok()?;
    let json = container::to_animation_json(&bytes).expect("a shipped animation must parse");

    let dimension = raster_policy::dimension(168.0, 2.0);
    let rasterizer = RlottieRasterizer::open(&json, name).expect("rlottie must accept it");

    Some(
        FrameSequence::open(
            SequenceKey::new(name.to_owned(), dimension),
            Box::new(rasterizer) as Box<dyn LottieRasterizer>,
            Arc::new(DecodedFrameCache::new(64 * 1024 * 1024)),
        )
        .expect("a plausible timeline"),
    )
}

#[test]
fn every_animation_a_screen_names_is_present_and_draws() {
    if !animations_dir().is_dir() {
        return;
    }

    for name in SHIPPED {
        let sequence = open(name).unwrap_or_else(|| {
            panic!("{name}.json is missing from assets/animations; the screen would draw a hole")
        });

        assert!(sequence.frame_count() > 1, "{name} is a still image");
        assert!(
            sequence.first_visible_frame().is_some(),
            "{name} rasterises to nothing — rlottie does not implement something it uses, \
             and it would render as an empty box rather than fail"
        );
    }
}

#[test]
fn a_whole_animation_fits_the_budget_the_screen_gives_it() {
    if !animations_dir().is_dir() {
        return;
    }

    const BUDGET: u64 = 64 * 1024 * 1024;

    for name in SHIPPED {
        let Some(sequence) = open(name) else { continue };
        let stride = sequence.presentation_stride().max(1);
        let sampled = sequence.frame_count().div_ceil(stride);
        let cost = sampled as u64 * sequence.frame_bytes();

        assert!(
            cost <= BUDGET,
            "{name} needs {} MB of decoded frames at 168pt@2x ({sampled} frames of {} KB); \
             the screen budget is {} MB",
            cost / (1024 * 1024),
            sequence.frame_bytes() / 1024,
            BUDGET / (1024 * 1024)
        );
    }
}

#[test]
fn the_presentation_stride_actually_drops_frames_on_a_high_rate_source() {
    if !animations_dir().is_dir() {
        return;
    }

    let strided = SHIPPED
        .iter()
        .filter_map(|name| open(name))
        .any(|sequence| sequence.presentation_stride() > 1);

    assert!(
        strided || SHIPPED.is_empty(),
        "no shipped animation is sampled down; check raster_policy::presentation_stride"
    );
}

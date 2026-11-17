use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::video_export::*;

fn close(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

fn landscape() -> VideoProbe {
    VideoProbe {
        duration_seconds: 10.0,
        video_start_seconds: 0.0,
        video_duration_seconds: 0.0,
        natural_size: FrameSize::new(1920.0, 1080.0),
        preferred_transform: Affine::IDENTITY,
        nominal_frame_rate: 30.0,
        has_video: true,
        has_audio: true,
        byte_count: 1_000_000,
    }
}

/// phone portrait clip, stored landscape and turned a quarter clockwise for display
fn portrait() -> VideoProbe {
    VideoProbe {
        preferred_transform: Affine { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 1080.0, ty: 0.0 },
        ..landscape()
    }
}

#[test]
fn the_preset_follows_the_profile_short_side() {
    assert_eq!(ExportPreset::for_short_side(360), ExportPreset::Size640x480);
    assert_eq!(ExportPreset::for_short_side(480), ExportPreset::Size640x480);
    assert_eq!(ExportPreset::for_short_side(720), ExportPreset::Size1280x720);
    assert_eq!(ExportPreset::for_short_side(1080), ExportPreset::Size1920x1080);
    assert_eq!(ExportPreset::for_short_side(1440), ExportPreset::Size3840x2160);
    assert_eq!(ExportPreset::HighestQuality.bounds(), None);
}

#[test]
fn a_turned_clip_displays_upright() {
    let probe = portrait();
    let size = probe.display_size();
    assert!(close(size.width, 1080.0) && close(size.height, 1920.0));
    let (min_x, min_y, _) = probe.upright().bounds_of(probe.natural_size);
    assert!(close(min_x, 0.0) && close(min_y, 0.0));
}

#[test]
fn a_clip_without_a_size_renders_at_the_fallback() {
    let probe = VideoProbe { natural_size: FrameSize::default(), ..landscape() };
    assert_eq!(probe.display_size(), FALLBACK_RENDER_SIZE);
}

#[test]
fn the_render_size_is_even_and_capped() {
    let probe = landscape();
    assert_eq!(render_size(&probe, FrameGeometry::default(), 1280.0), (1280, 720));
    let turned = FrameGeometry { rotation_steps: 1, ..FrameGeometry::default() };
    assert_eq!(render_size(&probe, turned, 1920.0), (1080, 1920));
    let cropped = FrameGeometry {
        crop: Some(RelativeRect { x: 0.0, y: 0.0, width: 0.5, height: 0.5 }),
        ..FrameGeometry::default()
    };
    assert_eq!(render_size(&probe, cropped, 4096.0), (960, 540));
    let odd = VideoProbe { natural_size: FrameSize::new(641.0, 361.0), ..landscape() };
    assert_eq!(render_size(&odd, FrameGeometry::default(), 4096.0), (640, 360));
}

#[test]
fn a_full_crop_and_no_turn_is_the_identity() {
    let full = FrameGeometry {
        rotation_steps: 4,
        crop: Some(RelativeRect { x: 0.0, y: 0.0, width: 1.0, height: 1.0 }),
        ..FrameGeometry::default()
    };
    assert!(full.is_identity());
    assert!(!FrameGeometry { mirrored: true, ..FrameGeometry::default() }.is_identity());
    assert!(!FrameGeometry { free_rotation_degrees: 1.0, ..FrameGeometry::default() }.is_identity());
}

#[test]
fn the_segment_transform_fills_the_render() {
    let probe = landscape();
    let render = render_size(&probe, FrameGeometry::default(), 1920.0);
    let transform = segment_transform(&probe, FrameGeometry::default(), render);
    let (x, y, size) = transform.bounds_of(probe.natural_size);
    assert!(close(x, 0.0) && close(y, 0.0));
    assert!(close(size.width, 1920.0) && close(size.height, 1080.0));
}

#[test]
fn a_quarter_turn_lands_inside_the_render() {
    let probe = landscape();
    let geometry = FrameGeometry { rotation_steps: 1, ..FrameGeometry::default() };
    let render = render_size(&probe, geometry, 1920.0);
    let transform = segment_transform(&probe, geometry, render);
    let (x, y, size) = transform.bounds_of(probe.natural_size);
    assert!(close(x, 0.0) && close(y, 0.0));
    assert!(close(size.width, f64::from(render.0)) && close(size.height, f64::from(render.1)));
    let (top_left_x, _) = transform.apply(0.0, 0.0);
    assert!(close(top_left_x, f64::from(render.0)));
}

#[test]
fn a_mirror_swaps_the_left_and_right_edges() {
    let probe = landscape();
    let geometry = FrameGeometry { mirrored: true, ..FrameGeometry::default() };
    let transform = segment_transform(&probe, geometry, (1920, 1080));
    let (left, _) = transform.apply(0.0, 0.0);
    assert!(close(left, 1920.0));
}

#[test]
fn the_frame_rate_is_rounded_defaulted_and_capped() {
    assert_eq!(render_frame_rate(29.97), 30);
    assert_eq!(render_frame_rate(0.0), DEFAULT_FRAME_RATE);
    assert_eq!(render_frame_rate(f32::NAN), DEFAULT_FRAME_RATE);
    assert_eq!(render_frame_rate(240.0), CAPPED_FRAME_RATE);
    assert_eq!(render_frame_rate(60.0), 60);
    assert!(VideoProbe { nominal_frame_rate: 240.0, ..landscape() }.needs_frame_rate_cap());
    assert!(!landscape().needs_frame_rate_cap());
}

#[test]
fn a_trim_is_clamped_into_the_clip() {
    let trim = clamp_trim(10.0, 2.0, 5.0).expect("trim");
    assert!(close(trim.span.start_seconds, 2.0) && close(trim.span.duration_seconds, 3.0));
    assert!(trim.cuts);
    let whole = clamp_trim(10.0, 0.0, 10.0).expect("whole");
    assert!(!whole.cuts);
    let past_end = clamp_trim(10.0, 12.0, 15.0).expect("end");
    assert!(close(past_end.span.end_seconds(), 10.0));
    assert!(close(past_end.span.duration_seconds, MINIMUM_CLIP_SECONDS));
    let backwards = clamp_trim(10.0, 4.0, 1.0).expect("backwards");
    assert!(close(backwards.span.duration_seconds, MINIMUM_CLIP_SECONDS));
    assert_eq!(clamp_trim(0.0, 0.0, 1.0), None);
    assert_eq!(clamp_trim(f64::NAN, 0.0, 1.0), None);
}

#[test]
fn music_after_the_trim_start_is_led_in() {
    let trim = TimeSpan { start_seconds: 2.0, duration_seconds: 6.0 };
    let placed = music_placement(trim, 3.0, 0, 30_000, 60.0).expect("placed");
    assert!(close(placed.at_seconds, 1.0));
    assert!(close(placed.span.start_seconds, 0.0));
    assert!(close(placed.span.duration_seconds, 5.0));
}

#[test]
fn music_before_the_trim_start_is_cut_from_its_head() {
    let trim = TimeSpan { start_seconds: 4.0, duration_seconds: 6.0 };
    let placed = music_placement(trim, 1.0, 10_000, 40_000, 60.0).expect("placed");
    assert!(close(placed.at_seconds, 0.0));
    assert!(close(placed.span.start_seconds, 13.0));
    assert!(close(placed.span.duration_seconds, 3.0));
}

#[test]
fn music_too_short_to_hear_is_not_mixed() {
    let trim = TimeSpan { start_seconds: 0.0, duration_seconds: 5.0 };
    assert_eq!(music_placement(trim, 4.99, 0, 10_000, 60.0), None);
    assert_eq!(music_placement(trim, 0.0, 5_000, 5_000, 60.0), None);
    assert_eq!(music_placement(trim, 0.0, 0, 10_000, f64::NAN), None);
}

#[test]
fn video_note_segments_never_overlap() {
    let starts = segment_starts(&[(0.0, 2.0), (1.5, 1.0), (5.0, 1.0)]);
    assert_eq!(starts, vec![0.0, 2.0, 5.0]);
    assert!(segment_starts(&[]).is_empty());
}

fn plain_job(target: &Path) -> ExportJob {
    ExportJob::transcode(
        Path::new("/tmp/source.mov"),
        &landscape(),
        ExportPreset::Size1280x720,
        target,
        None,
        Some(5_000_000),
    )
}

#[test]
fn a_transcode_is_a_single_source() {
    let job = plain_job(Path::new("/tmp/out.mp4"));
    assert!(job.is_single_source());
    assert_eq!(job.validate(), Ok(()));
    assert_eq!(job.segments[0].volume, Some(1.0));
    let silent = ExportJob::transcode(
        Path::new("/tmp/source.mov"),
        &VideoProbe { has_audio: false, ..landscape() },
        ExportPreset::Size1280x720,
        Path::new("/tmp/out.mp4"),
        None,
        None,
    );
    assert_eq!(silent.segments[0].volume, None);
}

#[test]
fn a_job_with_music_is_not_a_single_source() {
    let mut job = plain_job(Path::new("/tmp/out.mp4"));
    job.audio.push(AudioLayer { source: PathBuf::from("/tmp/music.m4a"), placement: None, volume: 0.5 });
    assert!(!job.is_single_source());
    assert_eq!(job.validate(), Ok(()));
}

#[test]
fn a_broken_job_is_rejected() {
    let target = Path::new("/tmp/out.mp4");
    let mut empty = plain_job(target);
    empty.segments.clear();
    assert!(matches!(empty.validate(), Err(VideoExportError::InvalidJob(_))));

    let mut loud = plain_job(target);
    loud.segments[0].volume = Some(1.5);
    assert!(matches!(loud.validate(), Err(VideoExportError::InvalidJob(_))));

    let mut odd = plain_job(target);
    odd.render = Some(RenderPlan { size: (641, 360), frame_rate: 30, adjustments: ColorAdjustments::IDENTITY, overlay: None, loop_layers: Vec::new(), badge: None });
    assert!(matches!(odd.validate(), Err(VideoExportError::InvalidJob(_))));

    let mut wrong_overlay = plain_job(target);
    wrong_overlay.render = Some(RenderPlan {
        size: (4, 2),
        frame_rate: 30,
        adjustments: ColorAdjustments::IDENTITY,
        overlay: Some(Overlay { width: 4, height: 2, rgba: Arc::from(vec![0u8; 4]) }),
        loop_layers: Vec::new(),
        badge: None,
    });
    assert!(matches!(wrong_overlay.validate(), Err(VideoExportError::InvalidJob(_))));

    let mut nan_colour = plain_job(target);
    nan_colour.render = Some(RenderPlan {
        size: (4, 2),
        frame_rate: 30,
        adjustments: ColorAdjustments { brightness: f64::NAN, ..ColorAdjustments::IDENTITY },
        overlay: None,
        loop_layers: Vec::new(),
        badge: None,
    });
    assert!(matches!(nan_colour.validate(), Err(VideoExportError::InvalidJob(_))));

    let mut zero_ceiling = plain_job(target);
    zero_ceiling.file_length_limit = Some(0);
    assert!(matches!(zero_ceiling.validate(), Err(VideoExportError::InvalidJob(_))));

    let mut empty_span = plain_job(target);
    empty_span.segments[0].span = Some(TimeSpan { start_seconds: 1.0, duration_seconds: 0.0 });
    assert!(matches!(empty_span.validate(), Err(VideoExportError::InvalidJob(_))));
}

#[test]
fn a_matching_overlay_is_accepted() {
    let mut job = plain_job(Path::new("/tmp/out.mp4"));
    job.render = Some(RenderPlan {
        size: (4, 2),
        frame_rate: 30,
        adjustments: ColorAdjustments::IDENTITY,
        overlay: Some(Overlay { width: 4, height: 2, rgba: Arc::from(vec![0u8; 32]) }),
        loop_layers: Vec::new(),
        badge: None,
    });
    assert_eq!(job.validate(), Ok(()));
}

struct FixedProgress(f32);

impl ExportControl for FixedProgress {
    fn progress(&self) -> f32 {
        self.0
    }

    fn cancel(&self) {}
}

#[test]
fn progress_never_says_done_before_the_outcome() {
    assert!((ExportTicket::new(Arc::new(FixedProgress(1.0))).progress() - PROGRESS_CEILING).abs() < f32::EPSILON);
    assert!(ExportTicket::new(Arc::new(FixedProgress(-0.5))).progress().abs() < f32::EPSILON);
    assert!(ExportTicket::new(Arc::new(FixedProgress(f32::NAN))).progress().abs() < f32::EPSILON);
}

#[test]
fn progress_is_reported_only_after_it_moved() {
    let mut throttle = ProgressThrottle::default();
    assert_eq!(throttle.next(0.0), Some(0.0));
    assert_eq!(throttle.next(0.001), None);
    assert_eq!(throttle.next(0.006), Some(0.006));
}

#[test]
fn affine_composition_applies_left_to_right() {
    let moved_then_scaled = Affine::translation(1.0, 2.0).then(Affine::scale(2.0, 3.0));
    let (x, y) = moved_then_scaled.apply(0.0, 0.0);
    assert!(close(x, 2.0) && close(y, 6.0));
    let quarter = Affine::rotation(std::f64::consts::FRAC_PI_2);
    let (x, y) = quarter.apply(1.0, 0.0);
    assert!(close(x, 0.0) && close(y, 1.0));
}

#[test]
fn a_poster_needs_a_time_and_a_side() {
    assert!(matches!(poster(Path::new("/tmp/source.mov"), -1.0, 720), Err(VideoExportError::InvalidJob(_))));
    assert!(matches!(poster(Path::new("/tmp/source.mov"), 0.0, 0), Err(VideoExportError::InvalidJob(_))));
}

#[test]
fn colour_adjustments_derive_the_core_image_parameters() {
    assert!(ColorAdjustments::IDENTITY.is_identity());
    assert_eq!(ColorAdjustments::default(), ColorAdjustments::IDENTITY);
    assert_eq!(ColorAdjustments::IDENTITY.color_controls(), None);
    let edit = ColorAdjustments {
        enhance: 1.4,
        brightness: 0.2,
        contrast: -0.5,
        saturation: 1.0,
        temperature: 0.5,
        highlights: -1.0,
        shadows: 0.5,
        vignette: 0.4,
    };
    assert!(!edit.is_identity());
    assert_eq!(edit.enhance_fraction(), Some(1.0));
    let controls = edit.color_controls().expect("controls");
    assert!((controls.brightness - 0.1).abs() < 1e-12);
    assert!((controls.contrast - 0.7).abs() < 1e-12);
    assert!((controls.saturation - 1.7).abs() < 1e-12);
    assert_eq!(edit.target_neutral_kelvin(), Some(5400.0));
    let bend = edit.highlight_shadow().expect("highlight and shadow");
    assert!((bend.highlight_amount - 0.4).abs() < 1e-12);
    assert!((bend.shadow_amount - 0.3).abs() < 1e-12);
    assert!((edit.vignette_intensity().expect("vignette") - 0.6).abs() < 1e-12);
    let only_vignette = ColorAdjustments { vignette: -0.3, ..ColorAdjustments::IDENTITY };
    assert_eq!(only_vignette.vignette_intensity(), None);
    assert_eq!(only_vignette.enhance_fraction(), None);
}

#[test]
fn an_inverted_transform_undoes_it() {
    let transform = Affine { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 1080.0, ty: 0.0 }
        .then(Affine::scale(0.5, 0.5))
        .then(Affine::translation(3.0, -7.0));
    let back = transform.inverted().expect("invertible");
    let (x, y) = transform.apply(120.0, 40.0);
    let (bx, by) = back.apply(x, y);
    assert!((bx - 120.0).abs() < 1e-9 && (by - 40.0).abs() < 1e-9);
    assert_eq!(Affine::scale(0.0, 1.0).inverted(), None);
}

fn still(width: u32, height: u32, duration_seconds: f64) -> StillVideoJob {
    StillVideoJob {
        frame: PosterFrame { width, height, bgra: vec![0; width as usize * height as usize * 4] },
        duration_seconds,
        average_bit_rate: 5_000_000,
        target: PathBuf::from("still.mp4"),
    }
}

#[test]
fn a_still_video_needs_even_sides_one_frame_and_a_duration() {
    assert_eq!(still(4, 2, 3.0).validate(), Ok(()));
    assert!(still(3, 2, 3.0).validate().is_err());
    assert!(still(0, 0, 3.0).validate().is_err());
    assert!(still(4, 2, 0.0).validate().is_err());
    assert!(still(4, 2, f64::NAN).validate().is_err());
    assert!(still(4, 2, f64::INFINITY).validate().is_err());
    let mut short = still(4, 2, 3.0);
    short.frame.bgra.pop();
    assert!(short.validate().is_err());
    let mut silent = still(4, 2, 3.0);
    silent.average_bit_rate = 0;
    assert!(silent.validate().is_err());
}

#[test]
fn a_still_repeats_one_thirtieth_before_its_end() {
    assert_eq!(still(2, 2, 3.0).ticks(), StillVideoTicks { repeat: Some(1_780), end: 1_800 });
    assert_eq!(still(2, 2, 15.5).ticks(), StillVideoTicks { repeat: Some(9_280), end: 9_300 });
    assert_eq!(still(2, 2, 0.02).ticks(), StillVideoTicks { repeat: None, end: 12 });
}

fn frame_spec(width: u32, height: u32, frames_per_second: u32, average_bit_rate: u32) -> FrameWriterSpec {
    FrameWriterSpec { target: PathBuf::from("canvas.mp4"), width, height, frames_per_second, average_bit_rate }
}

#[test]
fn a_frame_video_needs_even_sides_a_rate_and_a_bit_rate() {
    assert_eq!(frame_spec(1080, 1920, 60, 6_000_000).validate(), Ok(()));
    assert_eq!(frame_spec(1080, 1920, 60, 6_000_000).frame_bytes(), 1080 * 1920 * 4);
    assert!(frame_spec(1081, 1920, 60, 6_000_000).validate().is_err());
    assert!(frame_spec(0, 0, 60, 6_000_000).validate().is_err());
    assert!(frame_spec(2, 2, 0, 6_000_000).validate().is_err());
    assert!(frame_spec(2, 2, u32::MAX, 6_000_000).validate().is_err());
    assert!(frame_spec(2, 2, 60, 0).validate().is_err());
}

#[test]
fn rgba_turns_into_premultiplied_bgra() {
    let mut out = vec![9; 3];
    premultiplied_bgra(&[10, 20, 30, 255, 200, 100, 50, 128, 255, 255, 255, 0], &mut out);
    assert_eq!(out, vec![30, 20, 10, 255, 25, 50, 100, 128, 0, 0, 0, 0]);
}

#[test]
fn frame_indices_start_at_zero_or_later_and_only_grow() {
    assert!(frame_index_follows(None, 0));
    assert!(frame_index_follows(None, 5));
    assert!(!frame_index_follows(None, -1));
    assert!(frame_index_follows(Some(3), 4));
    assert!(!frame_index_follows(Some(3), 3));
    assert!(!frame_index_follows(Some(3), 2));
}

fn loop_of(count: usize, frame_duration: f64) -> LoopLayer {
    let frame = LoopFrame { width: 1, height: 1, rgba: Arc::from(vec![0_u8; 4]), placement: Affine::IDENTITY };
    LoopLayer { frames: vec![frame; count], frame_duration }
}

#[test]
fn a_loop_shows_the_frame_of_the_output_time_round_and_round() {
    let layer = loop_of(4, 0.1);
    assert_eq!(layer.frame_index_at(0.0), Some(0));
    assert_eq!(layer.frame_index_at(0.25), Some(2));
    assert_eq!(layer.frame_index_at(0.45), Some(0));
    assert_eq!(layer.frame_index_at(-0.05), Some(3));
    assert_eq!(layer.frame_index_at(f64::NAN), Some(0));
    assert_eq!(loop_of(1, 0.1).frame_index_at(7.3), Some(0));
    assert_eq!(loop_of(0, 0.1).frame_index_at(1.0), None);
    assert_eq!(loop_frame_index(3, 0.0, 1.0), Some(0));
}

#[test]
fn loop_layers_and_the_badge_are_checked_before_the_export() {
    let target = Path::new("/tmp/out.mp4");
    let plan = |loop_layers: Vec<LoopLayer>, badge: Option<MovingBadge>| RenderPlan {
        size: (4, 2),
        frame_rate: 30,
        adjustments: ColorAdjustments::IDENTITY,
        overlay: None,
        loop_layers,
        badge,
    };
    let stretch = BadgeStretch { start_seconds: 0.0, end_seconds: 5.0, x: 1.0, y: 1.0, fade_in_seconds: 0.5, fade_out_seconds: 0.5 };
    let badge = |stretch: BadgeStretch| MovingBadge {
        image: Overlay { width: 2, height: 1, rgba: Arc::from(vec![0_u8; 8]) },
        stretches: vec![stretch],
    };
    let mut job = plain_job(target);
    job.render = Some(plan(vec![loop_of(2, 0.04)], Some(badge(stretch))));
    assert_eq!(job.validate(), Ok(()));
    assert!(job.render.as_ref().is_some_and(RenderPlan::draws_every_frame));

    job.render = Some(plan(vec![loop_of(0, 0.04)], None));
    assert!(matches!(job.validate(), Err(VideoExportError::InvalidJob(_))));
    job.render = Some(plan(vec![loop_of(2, 0.0)], None));
    assert!(matches!(job.validate(), Err(VideoExportError::InvalidJob(_))));
    let mut skewed = loop_of(1, 0.04);
    skewed.frames[0].placement.tx = f64::INFINITY;
    job.render = Some(plan(vec![skewed], None));
    assert!(matches!(job.validate(), Err(VideoExportError::InvalidJob(_))));
    job.render = Some(plan(Vec::new(), Some(badge(BadgeStretch { end_seconds: 0.0, ..stretch }))));
    assert!(matches!(job.validate(), Err(VideoExportError::InvalidJob(_))));

    let still = plan(Vec::new(), None);
    assert!(!still.draws_every_frame());
}

#[test]
fn the_badge_fades_in_and_out_inside_its_stretch() {
    let stretch = BadgeStretch { start_seconds: 2.0, end_seconds: 6.0, x: 10.0, y: 20.0, fade_in_seconds: 1.0, fade_out_seconds: 0.5 };
    let badge = MovingBadge { image: Overlay { width: 1, height: 1, rgba: Arc::from(vec![0_u8; 4]) }, stretches: vec![stretch] };
    assert_eq!(badge.spot_at(1.0), None);
    assert_eq!(badge.spot_at(2.0), None);
    let rising = badge.spot_at(2.5).map(|spot| spot.opacity).unwrap_or(0.0);
    assert!(close(rising, 0.5));
    assert_eq!(badge.spot_at(4.0), Some(BadgeSpot { x: 10.0, y: 20.0, opacity: 1.0 }));
    let falling = badge.spot_at(5.75).map(|spot| spot.opacity).unwrap_or(0.0);
    assert!(close(falling, 0.5));
    assert_eq!(badge.spot_at(6.0), None);
    assert_eq!(badge.spot_at(f64::NAN), None);
}

#[test]
fn colour_tags_keep_a_standard_definition_clip_on_its_matrix() {
    let hd = ColorTags::of_source(None, None, None, (1920, 1080));
    assert_eq!(hd, ColorTags { primaries: TAG_ITU_R_709_2, transfer: TAG_ITU_R_709_2, matrix: TAG_ITU_R_709_2 });
    let sd = ColorTags::of_source(None, None, None, (720, 576));
    assert_eq!(sd, ColorTags { primaries: TAG_SMPTE_C, transfer: TAG_ITU_R_709_2, matrix: TAG_ITU_R_601_4 });
    let bt470 = ColorTags::of_source(Some(TAG_EBU_3213), Some(TAG_ITU_R_709_2), Some("YCbCrMatrix#5"), (1920, 1080));
    assert_eq!(bt470, ColorTags { primaries: TAG_EBU_3213, transfer: TAG_ITU_R_709_2, matrix: TAG_ITU_R_601_4 });
    let hdr = ColorTags::of_source(Some("ITU_R_2020"), Some("SMPTE_ST_2084_PQ"), Some("ITU_R_2020"), (3840, 2160));
    assert_eq!(hdr, hd);
    let odd_primaries = ColorTags::of_source(Some("P3_D65"), None, Some(TAG_ITU_R_709_2), (1920, 1080));
    assert_eq!(odd_primaries.primaries, TAG_ITU_R_709_2);
}

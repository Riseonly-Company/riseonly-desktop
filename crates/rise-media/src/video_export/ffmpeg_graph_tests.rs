use std::f64::consts::FRAC_PI_6;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::ffmpeg_graph::*;
use super::video_export::*;

fn probe(width: f64, height: f64, transform: Affine, seconds: f64, has_audio: bool) -> VideoProbe {
    VideoProbe {
        duration_seconds: seconds,
        video_start_seconds: 0.0,
        video_duration_seconds: 0.0,
        natural_size: FrameSize::new(width, height),
        preferred_transform: transform,
        nominal_frame_rate: 30.0,
        has_video: true,
        has_audio,
        byte_count: 1_000,
    }
}

const PORTRAIT: Affine = Affine { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 1080.0, ty: 0.0 };

#[test]
fn an_identity_layer_is_a_plain_scale_at_the_origin() {
    let placement = Placement::of(Affine::IDENTITY, FrameSize::new(1920.0, 1080.0)).expect("placement");
    assert_eq!(placement.turn, Turn::None);
    assert!(!placement.mirrored);
    assert_eq!((placement.x, placement.y), (0, 0));
    assert_eq!(placement.filters(), "scale=w=1920:h=1080:flags=bicubic");
}

#[test]
fn the_iphone_portrait_transform_is_one_clockwise_transpose() {
    let placement = Placement::of(PORTRAIT, FrameSize::new(1920.0, 1080.0)).expect("placement");
    assert_eq!(placement.turn, Turn::Clockwise);
    assert_eq!(placement.size, (1080, 1920));
    assert_eq!((placement.x, placement.y), (0, 0));
    assert_eq!(placement.filters(), "transpose=clock");
}

#[test]
fn a_front_camera_mirror_is_an_hflip() {
    let mirror = Affine { a: -1.0, b: 0.0, c: 0.0, d: 1.0, tx: 640.0, ty: 0.0 };
    let placement = Placement::of(mirror, FrameSize::new(640.0, 480.0)).expect("placement");
    assert!(placement.mirrored);
    assert_eq!(placement.turn, Turn::None);
    assert_eq!(placement.filters(), "hflip");
}

#[test]
fn a_half_turn_flips_both_ways() {
    let half = Affine { a: -1.0, b: 0.0, c: 0.0, d: -1.0, tx: 640.0, ty: 480.0 };
    let placement = Placement::of(half, FrameSize::new(640.0, 480.0)).expect("placement");
    assert_eq!(placement.turn, Turn::Half);
    assert_eq!(placement.filters(), "hflip,vflip");
}

#[test]
fn a_free_rotation_rotates_into_its_bounding_box_and_lands_at_its_corner() {
    let placement = Placement::of(Affine::rotation(FRAC_PI_6), FrameSize::new(100.0, 100.0)).expect("placement");
    assert!(matches!(placement.turn, Turn::Free(_)));
    assert_eq!(placement.turned_size, (137, 137));
    assert_eq!((placement.x, placement.y), (-50, 0));
    assert_eq!(placement.filters(), "rotate=a=0.523599:ow=137:oh=137:c=black");
}

#[test]
fn a_degenerate_transform_has_no_placement() {
    assert_eq!(Placement::of(Affine::scale(0.0, 1.0), FrameSize::new(10.0, 10.0)), None);
    assert_eq!(Placement::of(Affine::IDENTITY, FrameSize::new(0.0, 10.0)), None);
}

#[test]
fn numbers_print_fixed_without_negative_zero() {
    assert_eq!(number(-0.0), "0.000000");
    assert_eq!(number(1.5), "1.500000");
    assert_eq!(number(f64::NAN), "0.000000");
}

#[test]
fn presets_scale_down_into_their_sides_and_never_up() {
    let landscape = probe(1920.0, 1080.0, Affine::IDENTITY, 1.0, false);
    assert_eq!(preset_size(&landscape, ExportPreset::Size1280x720), (1280, 720));
    assert_eq!(preset_size(&landscape, ExportPreset::HighestQuality), (1920, 1080));
    let portrait = probe(1920.0, 1080.0, PORTRAIT, 1.0, false);
    assert_eq!(preset_size(&portrait, ExportPreset::Size640x480), (360, 640));
    let small = probe(640.0, 360.0, Affine::IDENTITY, 1.0, false);
    assert_eq!(preset_size(&small, ExportPreset::Size1920x1080), (640, 360));
}

#[test]
fn a_plain_transcode_is_one_direct_chain_with_its_own_sound() {
    let source = probe(1920.0, 1080.0, Affine::IDENTITY, 10.0, true);
    let job = ExportJob::transcode(
        Path::new("clip.mov"),
        &source,
        ExportPreset::Size1920x1080,
        Path::new("out.mp4"),
        None,
        None,
    );
    let plan = plan_graph(&job, &[source], &[], VAAPI_ENCODER).expect("plan");
    assert_eq!(
        plan.description,
        "[in_v0]trim=start=0.000000:end=10.000000,setpts=PTS-STARTPTS,scale=w=1920:h=1080:flags=bicubic[v0];\
         [v0]format=nv12,hwupload[out_v];\
         [in_a0]atrim=start=0.000000:end=10.000000,asetpts=PTS-STARTPTS,aresample=44100,aformat=channel_layouts=stereo,adelay=delays=0:all=1,volume=1.000000[a0];\
         [a0]anull,atrim=end=10.000000,aformat=sample_fmts=fltp:sample_rates=44100:channel_layouts=stereo[out_a]"
    );
    assert_eq!(
        plan.inputs,
        vec![
            GraphInput { name: "in_v0".into(), stream: InputStream::SegmentVideo(0) },
            GraphInput { name: "in_a0".into(), stream: InputStream::SegmentAudio(0) },
        ]
    );
    assert_eq!(plan.size, (1920, 1080));
    assert_eq!(plan.frame_rate, None);
    assert!(plan.has_audio);
    assert_eq!(plan.total_seconds, 10.0);
}

fn note_segment(at: Option<f64>) -> VideoSegment {
    VideoSegment {
        source: PathBuf::from("note.mov"),
        span: None,
        at_seconds: at,
        transform: Affine::IDENTITY,
        volume: None,
    }
}

#[test]
fn a_rendered_note_puts_segments_on_a_black_canvas_under_the_overlay() {
    let job = ExportJob {
        segments: vec![note_segment(Some(0.0)), note_segment(Some(2.5))],
        audio: vec![AudioLayer { source: PathBuf::from("mic.m4a"), placement: None, volume: 0.5 }],
        render: Some(RenderPlan {
            size: (640, 640),
            frame_rate: 30,
            adjustments: ColorAdjustments::IDENTITY,
            overlay: Some(Overlay { width: 640, height: 640, rgba: Arc::from(vec![0_u8; 640 * 640 * 4]) }),
            loop_layers: Vec::new(),
            badge: None,
        }),
        preset: ExportPreset::MediumQuality,
        target: PathBuf::from("note.mp4"),
        file_length_limit: None,
        cap_frame_rate: false,
    };
    let probes = [
        probe(640.0, 640.0, Affine::IDENTITY, 2.0, true),
        probe(640.0, 640.0, Affine::IDENTITY, 1.0, true),
    ];
    let plan = plan_graph(&job, &probes, &[9.0], "other").expect("plan");
    assert!(plan.description.starts_with(
        "color=c=black:s=640x640:r=30:d=3.500000[base0];\
         [in_v0]trim=start=0.000000:end=2.000000,setpts=PTS-STARTPTS,scale=w=640:h=640:flags=bicubic,fps=30,setpts=PTS+0.000000/TB[v0];\
         [base0][v0]overlay=x=0:y=0:eof_action=pass[base1];"
    ));
    assert!(plan.description.contains("setpts=PTS+2.500000/TB[v1];[base1][v1]overlay=x=0:y=0:eof_action=pass[base2];"));
    assert!(plan.description.contains("[base2][in_o]overlay=x=0:y=0:format=auto[composed];[composed]format=yuv420p[out_v]"));
    assert!(plan.description.contains(
        "[in_m0]atrim=start=0.000000:end=3.500000,asetpts=PTS-STARTPTS,aresample=44100,aformat=channel_layouts=stereo,adelay=delays=0:all=1,volume=0.500000[m0]"
    ));
    assert!(plan.description.ends_with("[m0]anull,atrim=end=3.500000,aformat=sample_fmts=fltp:sample_rates=44100:channel_layouts=stereo[out_a]"));
    assert_eq!(plan.frame_rate, Some(30));
    assert_eq!(plan.total_seconds, 3.5);
    assert!(plan.inputs.contains(&GraphInput { name: "in_o".into(), stream: InputStream::Overlay }));
    assert!(!plan.inputs.iter().any(|input| matches!(input.stream, InputStream::SegmentAudio(_))));
}

#[test]
fn several_sounds_mix_without_normalising() {
    let mut job = ExportJob::transcode(
        Path::new("clip.mov"),
        &probe(1280.0, 720.0, Affine::IDENTITY, 4.0, true),
        ExportPreset::HighestQuality,
        Path::new("out.mp4"),
        None,
        None,
    );
    job.audio.push(AudioLayer {
        source: PathBuf::from("music.m4a"),
        placement: Some(AudioPlacement { span: TimeSpan { start_seconds: 1.0, duration_seconds: 2.0 }, at_seconds: 0.5 }),
        volume: 0.25,
    });
    let plan = plan_graph(&job, &[probe(1280.0, 720.0, Affine::IDENTITY, 4.0, true)], &[30.0], VAAPI_ENCODER).expect("plan");
    assert!(plan.description.contains("adelay=delays=500:all=1,volume=0.250000[m0]"));
    assert!(plan.description.contains("[a0][m0]amix=inputs=2:duration=longest:normalize=0,atrim=end=4.000000"));
    assert_eq!(plan.frame_rate, None);
    assert!(plan.description.starts_with("[in_v0]trim=start=0.000000:end=4.000000,"));
}

#[test]
fn the_plan_rejects_missing_probes_and_silent_pictures() {
    let source = probe(640.0, 480.0, Affine::IDENTITY, 1.0, false);
    let job = ExportJob::transcode(Path::new("a.mov"), &source, ExportPreset::MediumQuality, Path::new("b.mp4"), None, None);
    assert!(matches!(plan_graph(&job, &[], &[], VAAPI_ENCODER), Err(VideoExportError::InvalidJob(_))));
    let mut blind = source.clone();
    blind.has_video = false;
    assert_eq!(plan_graph(&job, &[blind], &[], VAAPI_ENCODER), Err(VideoExportError::NoVideoTrack(PathBuf::from("a.mov"))));
}

#[test]
fn the_poster_turns_upright_and_fits_the_long_side() {
    let (filters, size) = poster_filters(&probe(1920.0, 1080.0, PORTRAIT, 1.0, false), 720).expect("poster");
    assert_eq!(filters, "transpose=clock,scale=w=405:h=720:flags=bicubic,format=bgra");
    assert_eq!(size, (405, 720));
}

#[test]
fn the_encoder_needs_aac_and_a_vaapi_device() {
    assert_eq!(pick_video_encoder(|_| true, true), Ok(VAAPI_ENCODER));
    assert!(matches!(pick_video_encoder(|_| true, false), Err(VideoExportError::EncoderUnavailable(_))));
    match pick_video_encoder(|name| name != AAC_ENCODER, true) {
        Err(VideoExportError::EncoderUnavailable(reason)) => assert!(reason.contains(AAC_ENCODER)),
        other => panic!("expected EncoderUnavailable, got {other:?}"),
    }
}

#[test]
fn bit_rates_follow_the_preset() {
    assert_eq!(video_bit_rate(ExportPreset::Size1280x720, (1280, 720), 30), 3_317_760);
    assert_eq!(video_bit_rate(ExportPreset::MediumQuality, (2, 2), 30), 250_000);
    assert_eq!(audio_bit_rate(ExportPreset::MediumQuality), 64_000);
    assert_eq!(audio_bit_rate(ExportPreset::HighestQuality), 128_000);
}

#[test]
fn the_byte_ceiling_keeps_room_for_the_trailer() {
    assert!(!byte_ceiling_reached(934_463, Some(1_000_000)));
    assert!(byte_ceiling_reached(934_464, Some(1_000_000)));
    assert!(!byte_ceiling_reached(u64::MAX, None));
}

#[test]
fn progress_is_clamped() {
    assert_eq!(progress(5.0, 10.0), 0.5);
    assert_eq!(progress(20.0, 10.0), 1.0);
    assert_eq!(progress(1.0, 0.0), 0.0);
}

#[test]
fn the_display_matrix_is_the_preferred_transform() {
    let turned = affine_from_display_matrix(Some([0, 65_536, 0, -65_536, 0, 0, 0, 0, 1 << 30]));
    assert_eq!(turned, Affine { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: 0.0, ty: 0.0 });
    assert_eq!(affine_from_display_matrix(None), Affine::IDENTITY);
    assert_eq!(affine_from_display_matrix(Some([0; 9])), Affine::IDENTITY);
}

#[test]
fn identity_adjustments_add_no_colour_pass() {
    assert_eq!(color_filters(ColorAdjustments::IDENTITY), None);
}

#[test]
fn colour_adjustments_become_lgpl_filters_in_core_image_order() {
    let adjustments = ColorAdjustments {
        enhance: 0.5,
        brightness: 0.2,
        contrast: 0.0,
        saturation: -0.5,
        temperature: 1.0,
        highlights: 0.0,
        shadows: 1.0,
        vignette: 1.0,
    };
    let filters = color_filters(adjustments).expect("filters");
    assert_eq!(
        filters,
        "normalize=strength=0.500000,hue=s=0.650000,curves=all='0.000000/0.100000 0.900000/1.000000',\
colortemperature=temperature=4300.000000,curves=all='0/0 0.25/0.400000 0.75/0.750000 1/1',vignette=angle=0.942478"
    );
    assert!(!filters.contains("eq="), "eq is GPL only");
}

#[test]
fn colour_pass_runs_before_the_segment_transform() {
    let mut job = ExportJob::transcode(
        Path::new("clip.mov"),
        &probe(1920.0, 1080.0, Affine::IDENTITY, 4.0, false),
        ExportPreset::Size1280x720,
        Path::new("out.mp4"),
        None,
        None,
    );
    job.render = Some(RenderPlan {
        size: (1280, 720),
        frame_rate: 30,
        adjustments: ColorAdjustments { saturation: 1.0, ..ColorAdjustments::IDENTITY },
        overlay: None,
        loop_layers: Vec::new(),
        badge: None,
    });
    job.segments[0].transform = Affine::scale(2.0 / 3.0, 2.0 / 3.0);
    let plan = plan_graph(&job, &[probe(1920.0, 1080.0, Affine::IDENTITY, 4.0, false)], &[], "h264_vaapi")
        .expect("plan");
    let colour = plan.description.find("hue=s=1.700000").expect("colour pass");
    let scale = plan.description.find("scale=w=1280:h=720").expect("transform");
    assert!(colour < scale);
}

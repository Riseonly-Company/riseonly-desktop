use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::GzEncoder;
use rise_limits::cache_budget::LOTTIE_DECODED_BYTES;
use rise_limits::limit::STICKER_IMAGE_SOURCE;
use rise_limits::service_limit::{API_GATEWAY_MAX_MEDIA_DURATION_SECS, MAX_MEDIA_FPS};
use rise_platform::HostOs;

use super::decode_guard::*;
use crate::lottie::container::{ContainerError, to_animation_json};
use crate::texture::frame::MAX_DIMENSION;
use crate::video::decoder::{
    DecodedFrame, DecoderError, DecoderPlan, DecoderRequest, GuardedDecoder, VideoCodec, VideoDecoder,
};

const FIXED_POINT_ONE: u32 = 1 << 16;

/// the broken test set lives in tests/data and gets written here, nothing is downloaded
fn data_file(name: &str, bytes: &[u8]) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(payload.len() + 8);
    bytes.extend_from_slice(&(payload.len() as u32 + 8).to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

fn movie_header(time_scale: u32, duration: u32) -> Vec<u8> {
    let mut payload = vec![0_u8; 100];
    payload[12..16].copy_from_slice(&time_scale.to_be_bytes());
    payload[16..20].copy_from_slice(&duration.to_be_bytes());
    mp4_box(b"mvhd", &payload)
}

fn track_header(width: u32, height: u32) -> Vec<u8> {
    let mut payload = vec![0_u8; 84];
    payload[76..80].copy_from_slice(&(width * FIXED_POINT_ONE).to_be_bytes());
    payload[80..84].copy_from_slice(&(height * FIXED_POINT_ONE).to_be_bytes());
    mp4_box(b"tkhd", &payload)
}

fn mp4(width: u32, height: u32, time_scale: u32, duration: u32) -> Vec<u8> {
    let track = mp4_box(b"trak", &track_header(width, height));
    let movie = [movie_header(time_scale, duration), track].concat();
    [
        mp4_box(b"ftyp", b"isom\0\0\0\0isom"),
        mp4_box(b"moov", &movie),
        mp4_box(b"mdat", &[0_u8; 16]),
    ]
    .concat()
}

fn lottie(frame_rate: f64, first_frame: i64, last_frame: i64, side: i64) -> Vec<u8> {
    format!(
        r#"{{"v":"5.5.7","fr":{frame_rate},"ip":{first_frame},"op":{last_frame},"w":{side},"h":{side},"layers":[]}}"#
    )
    .into_bytes()
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

struct PanickingDecoder {
    plan: DecoderPlan,
}

impl VideoDecoder for PanickingDecoder {
    fn plan(&self) -> &DecoderPlan {
        &self.plan
    }

    fn send_packet(&mut self, _packet: &[u8], _timestamp_us: i64) -> Result<(), DecoderError> {
        panic!("the backend failed while sending");
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, DecoderError> {
        panic!("the backend failed while receiving");
    }

    fn flush(&mut self) -> Result<(), DecoderError> {
        panic!("the backend failed while flushing");
    }
}

fn panicking_decoder() -> GuardedDecoder {
    let request = DecoderRequest {
        codec: VideoCodec::H264,
        width: 1280,
        height: 720,
        bit_depth: 8,
    };
    let plan = DecoderPlan::resolve(HostOs::current(), &request).unwrap();
    GuardedDecoder::new(Box::new(PanickingDecoder { plan }))
}

#[test]
fn every_client_key_is_in_the_canon() {
    for key in [
        LOTTIE_JSON_DEPTH_KEY,
        LOTTIE_FRAMES_KEY,
        VIDEO_SIDE_KEY,
        VIDEO_FRAME_BYTES_KEY,
        VIDEO_FILE_BYTES_KEY,
        MP4_BOXES_KEY,
    ] {
        assert!(client_limit(key) > 0, "{key} is missing from the canon");
    }
}

#[test]
fn the_video_side_ceiling_is_the_one_frame_geometry_enforces() {
    assert_eq!(client_limit(VIDEO_SIDE_KEY), u64::from(MAX_DIMENSION));
}

#[test]
fn an_ordinary_mp4_passes() {
    let path = data_file("ordinary.mp4", &mp4(1280, 720, 600, 6000));
    assert_eq!(check_video_file(&path), Ok(()));
}

#[test]
fn a_truncated_mp4_is_refused() {
    let whole = mp4(1280, 720, 600, 6000);
    let path = data_file("truncated.mp4", &whole[..64]);
    assert_eq!(check_video_file(&path), Err(DecodeGuardError::ContainerTruncated));
}

#[test]
fn an_mp4_without_a_movie_box_is_refused() {
    let bytes = [mp4_box(b"ftyp", b"isom"), mp4_box(b"mdat", &[0_u8; 16])].concat();
    let path = data_file("no_movie.mp4", &bytes);
    assert_eq!(check_video_file(&path), Err(DecodeGuardError::ContainerTruncated));
}

#[test]
fn a_final_data_box_longer_than_the_file_is_an_unfinished_write_not_a_refusal() {
    let whole = mp4(1280, 720, 600, 6000);
    let mut cursor = Cursor::new(&whole[..whole.len() - 4]);
    assert_eq!(check_mp4(&mut cursor, whole.len() as u64 - 4), Ok(()));
}

#[test]
fn huge_dimensions_in_the_header_are_refused() {
    let path = data_file("huge_dimensions.mp4", &mp4(30000, 30000, 600, 6000));
    assert!(matches!(
        check_video_file(&path),
        Err(DecodeGuardError::FrameTooLarge { width: 30000, height: 30000, .. })
    ));
}

#[test]
fn a_duration_over_the_ceiling_is_refused() {
    let seconds = u32::try_from(API_GATEWAY_MAX_MEDIA_DURATION_SECS + 1).unwrap();
    let mut cursor = Cursor::new(mp4(1280, 720, 1, seconds));
    let length = cursor.get_ref().len() as u64;
    assert!(matches!(
        check_mp4(&mut cursor, length),
        Err(DecodeGuardError::DurationTooLong { .. })
    ));
}

#[test]
fn a_zero_time_scale_is_malformed() {
    let mut cursor = Cursor::new(mp4(1280, 720, 0, 6000));
    let length = cursor.get_ref().len() as u64;
    assert_eq!(check_mp4(&mut cursor, length), Err(DecodeGuardError::ContainerMalformed));
}

#[test]
fn a_box_smaller_than_its_header_is_malformed() {
    let bytes = [mp4_box(b"ftyp", b"isom"), vec![0, 0, 0, 4], b"moov".to_vec()].concat();
    let length = bytes.len() as u64;
    assert_eq!(check_mp4(&mut Cursor::new(bytes), length), Err(DecodeGuardError::ContainerMalformed));
}

#[test]
fn more_boxes_than_the_ceiling_are_refused() {
    let limit = client_limit(MP4_BOXES_KEY);
    let free = mp4_box(b"free", &[]);
    let bytes = free.repeat(limit as usize);
    let bytes = [mp4_box(b"ftyp", b"isom"), bytes].concat();
    let length = bytes.len() as u64;
    assert_eq!(
        check_mp4(&mut Cursor::new(bytes), length),
        Err(DecodeGuardError::TooManyBoxes { limit })
    );
}

#[test]
fn a_file_of_another_format_passes_to_its_demuxer() {
    let path = data_file("not_mp4.webm", &[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0, 0, 0]);
    assert_eq!(check_video_file(&path), Ok(()));
}

#[test]
fn frame_memory_and_rate_ceilings_hold() {
    assert_eq!(check_video_frame(1920, 1080), Ok(()));
    assert!(check_video_frame(MAX_DIMENSION + 2, 16).is_err());
    let limit = client_limit(VIDEO_FRAME_BYTES_KEY);
    assert!(check_frame_bytes(limit + 1).is_err());
    assert_eq!(check_video_timeline(Some(60.0), Some(30.0)), Ok(()));
    assert_eq!(
        check_video_timeline(Some(MAX_MEDIA_FPS as f64 + 1.0), None),
        Err(DecodeGuardError::FrameRateOutOfRange)
    );
    assert_eq!(check_video_timeline(Some(f64::NAN), None), Err(DecodeGuardError::FrameRateOutOfRange));
}

#[test]
fn a_gzip_bomb_is_refused_at_the_inflation_ceiling() {
    let bomb = gzip(&vec![b' '; STICKER_IMAGE_SOURCE as usize + 4096]);
    assert!((bomb.len() as u64) < STICKER_IMAGE_SOURCE);
    let path = data_file("bomb.tgs", &bomb);
    let read = fs::read(path).unwrap();
    assert_eq!(
        to_animation_json(&read),
        Err(ContainerError::TooLarge { bytes: STICKER_IMAGE_SOURCE + 1 })
    );
    assert_eq!(
        inflate_lottie(&read),
        Err(DecodeGuardError::InflationRefused { limit: STICKER_IMAGE_SOURCE })
    );
}

#[test]
fn a_truncated_gzip_is_refused() {
    let whole = gzip(&lottie(30.0, 0, 30, 512));
    let truncated = &whole[..whole.len() / 2];
    assert_eq!(to_animation_json(truncated), Err(ContainerError::Malformed));
    assert_eq!(inflate_lottie(truncated), Err(DecodeGuardError::CompressedStreamBroken));
}

#[test]
fn an_ordinary_sticker_inflates_and_passes() {
    let json = lottie(60.0, 0, 180, 512);
    assert_eq!(to_animation_json(&gzip(&json)).as_deref(), Ok(json.as_slice()));
    assert_eq!(check_lottie_json(&json), Ok(()));
}

#[test]
fn a_deeply_nested_lottie_is_refused_before_it_is_parsed() {
    let levels = client_limit(LOTTIE_JSON_DEPTH_KEY) as usize + 1;
    let deep = format!(r#"{{"layers":{}{}}}"#, "[".repeat(levels), "]".repeat(levels));
    let path = data_file("deep.json", deep.as_bytes());
    assert!(matches!(
        check_lottie_json(&fs::read(path).unwrap()),
        Err(DecodeGuardError::LottieTooDeep { .. })
    ));
}

#[test]
fn brackets_inside_a_string_do_not_count_as_nesting() {
    let levels = client_limit(LOTTIE_JSON_DEPTH_KEY) as usize + 1;
    let json = format!(
        r#"{{"nm":"\"{}","fr":30,"ip":0,"op":30,"w":512,"h":512,"layers":[]}}"#,
        "[".repeat(levels)
    );
    assert_eq!(check_lottie_json(json.as_bytes()), Ok(()));
}

#[test]
fn an_oversized_lottie_is_refused_by_its_length() {
    let huge = vec![b' '; STICKER_IMAGE_SOURCE as usize + 1];
    assert_eq!(
        check_lottie_json(&huge),
        Err(DecodeGuardError::FileTooLarge {
            bytes: STICKER_IMAGE_SOURCE + 1,
            limit: STICKER_IMAGE_SOURCE
        })
    );
}

#[test]
fn a_lottie_header_out_of_range_is_refused() {
    let frames = client_limit(LOTTIE_FRAMES_KEY) as i64 + 1;
    assert!(matches!(
        check_lottie_json(&lottie(30.0, 0, frames, 512)),
        Err(DecodeGuardError::TooManyFrames { .. })
    ));
    assert_eq!(
        check_lottie_json(&lottie(0.0, 0, 30, 512)),
        Err(DecodeGuardError::FrameRateOutOfRange)
    );
    assert_eq!(check_lottie_json(&lottie(30.0, 30, 0, 512)), Err(DecodeGuardError::LottieHeaderInvalid));
    assert_eq!(check_lottie_json(&lottie(30.0, 0, 30, -1)), Err(DecodeGuardError::LottieHeaderInvalid));
    assert_eq!(check_lottie_json(&lottie(30.0, 0, 30, 100_000)), Err(DecodeGuardError::LottieHeaderInvalid));
    assert_eq!(check_lottie_json(b"{}"), Err(DecodeGuardError::LottieHeaderInvalid));
}

#[test]
fn a_rasterised_frame_stays_within_the_decoded_budget() {
    assert_eq!(check_lottie_frame(512), Ok(()));
    let over = (LOTTIE_DECODED_BYTES / 4).isqrt() as u32 + 1;
    assert!(check_lottie_frame(over).is_err());
    assert!(check_lottie_frame(u32::MAX).is_err());
}

#[test]
fn a_panic_on_the_way_to_a_native_call_becomes_an_error() {
    assert_eq!(native_call(|| 7), Ok(7));
    let panicked: Result<(), _> = native_call(|| panic!("native boundary"));
    assert_eq!(panicked, Err(DecodeGuardError::NativePanicked));
}

#[test]
fn a_panicking_backend_reports_an_error_through_the_decoder() {
    let mut decoder = panicking_decoder();
    assert!(matches!(decoder.receive_frame(), Err(DecoderError::Backend(_))));
    assert!(matches!(decoder.flush(), Err(DecoderError::Backend(_))));
    assert!(matches!(decoder.send_packet(&[0_u8; 4], 0), Err(DecoderError::Backend(_))));
}

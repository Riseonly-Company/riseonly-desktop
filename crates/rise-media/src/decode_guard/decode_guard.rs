use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use rise_limits::cache_budget::LOTTIE_DECODED_BYTES;
use rise_limits::limit::STICKER_IMAGE_SOURCE;
use rise_limits::service_limit::{
    API_GATEWAY_MAX_MEDIA_DURATION_SECS, MAX_MEDIA_FPS, MAX_PROBED_IMAGE_DIMENSION,
};
use serde_json::Value;
use thiserror::Error;

use crate::lottie::zlib::{InflateRefusal, inflate_bounded};
use crate::texture::frame::FrameFormat;

pub(crate) const LOTTIE_JSON_DEPTH_KEY: &str = "decode_lottie_json_depth";
pub(crate) const LOTTIE_FRAMES_KEY: &str = "decode_lottie_frames";
pub(crate) const VIDEO_SIDE_KEY: &str = "decode_video_side";
pub(crate) const VIDEO_FRAME_BYTES_KEY: &str = "decode_video_frame_bytes";
pub(crate) const VIDEO_FILE_BYTES_KEY: &str = "decode_video_file_bytes";
pub(crate) const MP4_BOXES_KEY: &str = "decode_mp4_boxes";

const BOX_HEADER_BYTES: u64 = 8;
const BOX_LARGE_SIZE_BYTES: u64 = 8;
const FIXED_POINT_ONE: u32 = 1 << 16;

/// why media was refused before reaching a decoder
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub enum DecodeGuardError {
    #[error("the file holds {bytes} bytes over a ceiling of {limit}")]
    FileTooLarge { bytes: u64, limit: u64 },
    #[error("a frame of {width}x{height} is over the side ceiling of {limit}")]
    FrameTooLarge { width: u32, height: u32, limit: u64 },
    #[error("a frame needs {bytes} bytes over a ceiling of {limit}")]
    FrameMemoryTooLarge { bytes: u64, limit: u64 },
    #[error("the frame rate is out of range")]
    FrameRateOutOfRange,
    #[error("the duration is over the ceiling of {limit} seconds")]
    DurationTooLong { limit: u64 },
    #[error("{frames} frames are over the ceiling of {limit}")]
    TooManyFrames { frames: u64, limit: u64 },
    #[error("the Lottie document nests deeper than {limit}")]
    LottieTooDeep { limit: u64 },
    #[error("the Lottie document has no valid header")]
    LottieHeaderInvalid,
    #[error("the compressed sticker is broken")]
    CompressedStreamBroken,
    #[error("the compressed sticker inflates past {limit} bytes")]
    InflationRefused { limit: u64 },
    #[error("the container ends before its boxes do")]
    ContainerTruncated,
    #[error("the container has a box that cannot be read")]
    ContainerMalformed,
    #[error("the container holds more than {limit} boxes")]
    TooManyBoxes { limit: u64 },
    #[error("the native decoder call panicked")]
    NativePanicked,
}

pub(crate) fn client_limit(name: &str) -> u64 {
    rise_limits::CLIENT_LIMITS
        .iter()
        .find(|spec| spec.name == name)
        .map_or(0, |spec| spec.value)
}

/// runs one native decoder call and turns a panic into an error
pub fn native_call<T>(call: impl FnOnce() -> T) -> Result<T, DecodeGuardError> {
    catch_unwind(AssertUnwindSafe(call)).map_err(|_| {
        tracing::warn!(target: "riseonly", "a native decoder call panicked");
        DecodeGuardError::NativePanicked
    })
}

fn check_bytes(bytes: u64, limit: u64) -> Result<(), DecodeGuardError> {
    if bytes > limit {
        return Err(DecodeGuardError::FileTooLarge { bytes, limit });
    }
    Ok(())
}

/// checks sides and worst case memory of one frame
/// a zero side passes since the container may not know the size yet
pub fn check_video_frame(width: u32, height: u32) -> Result<(), DecodeGuardError> {
    let limit = client_limit(VIDEO_SIDE_KEY);
    if u64::from(width) > limit || u64::from(height) > limit {
        return Err(DecodeGuardError::FrameTooLarge { width, height, limit });
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    check_frame_bytes(pixels.saturating_mul(u64::from(FrameFormat::Bgra8.bytes_per_luma_sample())))
}

/// checks the memory one decoded or imported frame takes
pub fn check_frame_bytes(bytes: u64) -> Result<(), DecodeGuardError> {
    let limit = client_limit(VIDEO_FRAME_BYTES_KEY);
    if bytes > limit {
        return Err(DecodeGuardError::FrameMemoryTooLarge { bytes, limit });
    }
    Ok(())
}

/// checks the frame rate and length a container declares, none means it doesnt know
pub fn check_video_timeline(
    frame_rate: Option<f64>,
    duration_seconds: Option<f64>,
) -> Result<(), DecodeGuardError> {
    if let Some(rate) = frame_rate
        && !(0.0..=MAX_MEDIA_FPS as f64).contains(&rate)
    {
        return Err(DecodeGuardError::FrameRateOutOfRange);
    }
    if let Some(seconds) = duration_seconds
        && !(seconds.is_finite() && seconds <= API_GATEWAY_MAX_MEDIA_DURATION_SECS as f64)
    {
        return Err(DecodeGuardError::DurationTooLong {
            limit: API_GATEWAY_MAX_MEDIA_DURATION_SECS,
        });
    }
    Ok(())
}

/// one compressed packet can never be bigger than the whole file
pub fn check_video_packet(bytes: u64) -> Result<(), DecodeGuardError> {
    check_bytes(bytes, client_limit(VIDEO_FILE_BYTES_KEY))
}

/// checks a local video before any player opens it, size and for mp4 family the box tree
/// an unopenable file passes since the decoder reports that the usual way
/// reads disk as often as the boxes ask so never call it on the main thread
pub fn check_video_file(path: &Path) -> Result<(), DecodeGuardError> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Ok(());
    };
    let Ok(metadata) = file.metadata() else {
        return Ok(());
    };
    check_bytes(metadata.len(), client_limit(VIDEO_FILE_BYTES_KEY))?;
    check_mp4(&mut file, metadata.len())
}

/// walks mp4 boxes without allocating, only box headers and fixed mvhd and tkhd fields get read
/// other formats pass, their demuxer header is checked after opening
pub fn check_mp4<R: Read + Seek>(reader: &mut R, length: u64) -> Result<(), DecodeGuardError> {
    if length < BOX_HEADER_BYTES {
        return Err(DecodeGuardError::ContainerTruncated);
    }
    let mut walk = Mp4Walk {
        reader,
        boxes: 0,
        limit: client_limit(MP4_BOXES_KEY),
    };
    let mut kind = [0_u8; 4];
    walk.read_at(4, &mut kind)?;
    if &kind != b"ftyp" {
        return Ok(());
    }

    let mut has_movie = false;
    let mut position = 0;
    while position < length {
        let current = walk.next_box(position, length, has_movie)?;
        if &current.kind == b"moov" {
            walk.check_movie(current.payload, current.end)?;
            has_movie = true;
        }
        position = current.end;
    }
    if has_movie {
        Ok(())
    } else {
        Err(DecodeGuardError::ContainerTruncated)
    }
}

struct Mp4Box {
    kind: [u8; 4],
    payload: u64,
    end: u64,
}

struct Mp4Walk<'a, R> {
    reader: &'a mut R,
    boxes: u64,
    limit: u64,
}

impl<R: Read + Seek> Mp4Walk<'_, R> {
    fn read_at(&mut self, position: u64, buffer: &mut [u8]) -> Result<(), DecodeGuardError> {
        self.reader
            .seek(SeekFrom::Start(position))
            .and_then(|_| self.reader.read_exact(buffer))
            .map_err(|_| DecodeGuardError::ContainerTruncated)
    }

    fn next_box(
        &mut self,
        start: u64,
        parent_end: u64,
        tolerates_long_data: bool,
    ) -> Result<Mp4Box, DecodeGuardError> {
        self.boxes += 1;
        if self.boxes > self.limit {
            return Err(DecodeGuardError::TooManyBoxes { limit: self.limit });
        }
        if parent_end - start < BOX_HEADER_BYTES {
            return Err(DecodeGuardError::ContainerTruncated);
        }

        let mut header = [0_u8; BOX_HEADER_BYTES as usize];
        self.read_at(start, &mut header)?;
        let kind = [header[4], header[5], header[6], header[7]];
        let short_size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);

        let mut payload = start + BOX_HEADER_BYTES;
        let end = match short_size {
            0 => parent_end,
            1 => {
                let mut large = [0_u8; BOX_LARGE_SIZE_BYTES as usize];
                self.read_at(payload, &mut large)?;
                payload += BOX_LARGE_SIZE_BYTES;
                start
                    .checked_add(u64::from_be_bytes(large))
                    .ok_or(DecodeGuardError::ContainerMalformed)?
            }
            size => start + u64::from(size),
        };
        if end < payload {
            return Err(DecodeGuardError::ContainerMalformed);
        }
        if end > parent_end {
            if tolerates_long_data && &kind == b"mdat" {
                return Ok(Mp4Box { kind, payload, end: parent_end });
            }
            return Err(DecodeGuardError::ContainerTruncated);
        }
        Ok(Mp4Box { kind, payload, end })
    }

    fn check_movie(&mut self, start: u64, end: u64) -> Result<(), DecodeGuardError> {
        let mut position = start;
        while position < end {
            let child = self.next_box(position, end, false)?;
            match &child.kind {
                b"mvhd" => self.check_movie_header(&child)?,
                b"trak" => self.check_track(child.payload, child.end)?,
                _ => {}
            }
            position = child.end;
        }
        Ok(())
    }

    fn check_track(&mut self, start: u64, end: u64) -> Result<(), DecodeGuardError> {
        let mut position = start;
        while position < end {
            let child = self.next_box(position, end, false)?;
            if &child.kind == b"tkhd" {
                self.check_track_header(&child)?;
            }
            position = child.end;
        }
        Ok(())
    }

    fn read_field<const N: usize>(
        &mut self,
        owner: &Mp4Box,
        offset: u64,
    ) -> Result<[u8; N], DecodeGuardError> {
        let position = owner.payload + offset;
        if position + N as u64 > owner.end {
            return Err(DecodeGuardError::ContainerTruncated);
        }
        let mut field = [0_u8; N];
        self.read_at(position, &mut field)?;
        Ok(field)
    }

    fn check_movie_header(&mut self, movie_header: &Mp4Box) -> Result<(), DecodeGuardError> {
        let [version] = self.read_field::<1>(movie_header, 0)?;
        let (time_scale, duration) = if version == 1 {
            let scale = self.read_field::<4>(movie_header, 20)?;
            let duration = self.read_field::<8>(movie_header, 24)?;
            (u32::from_be_bytes(scale), u64::from_be_bytes(duration))
        } else {
            let scale = self.read_field::<4>(movie_header, 12)?;
            let duration = u32::from_be_bytes(self.read_field::<4>(movie_header, 16)?);
            let widened = if duration == u32::MAX { u64::MAX } else { u64::from(duration) };
            (u32::from_be_bytes(scale), widened)
        };
        if time_scale == 0 {
            return Err(DecodeGuardError::ContainerMalformed);
        }
        if duration == u64::MAX {
            return Ok(());
        }
        check_video_timeline(None, Some(duration as f64 / f64::from(time_scale)))
    }

    fn check_track_header(&mut self, track_header: &Mp4Box) -> Result<(), DecodeGuardError> {
        let [version] = self.read_field::<1>(track_header, 0)?;
        let offset = if version == 1 { 88 } else { 76 };
        let width = u32::from_be_bytes(self.read_field::<4>(track_header, offset)?);
        let height = u32::from_be_bytes(self.read_field::<4>(track_header, offset + 4)?);
        check_video_frame(width / FIXED_POINT_ONE, height / FIXED_POINT_ONE)
    }
}

/// bounded inflate of a tgs or ros sticker with the same ceiling as the old loader
pub fn inflate_lottie(compressed: &[u8]) -> Result<Vec<u8>, DecodeGuardError> {
    check_bytes(compressed.len() as u64, STICKER_IMAGE_SOURCE)?;
    let limit = usize::try_from(STICKER_IMAGE_SOURCE).unwrap_or(usize::MAX);
    inflate_bounded(compressed, limit).map_err(|refusal| match refusal {
        InflateRefusal::Broken => DecodeGuardError::CompressedStreamBroken,
        InflateRefusal::OverCeiling => DecodeGuardError::InflationRefused { limit: STICKER_IMAGE_SOURCE },
    })
}

/// checks a lottie doc before rlottie parses it, size and nesting and header fields
/// nesting goes first because the header read is a recursive parser too
pub fn check_lottie_json(json: &[u8]) -> Result<(), DecodeGuardError> {
    check_bytes(json.len() as u64, STICKER_IMAGE_SOURCE)?;
    let depth_limit = client_limit(LOTTIE_JSON_DEPTH_KEY);
    if !nesting_is_within(json, depth_limit) {
        return Err(DecodeGuardError::LottieTooDeep { limit: depth_limit });
    }

    let root: Value =
        serde_json::from_slice(json).map_err(|_| DecodeGuardError::LottieHeaderInvalid)?;
    let number = |field: &str| {
        root.get(field)
            .and_then(Value::as_f64)
            .ok_or(DecodeGuardError::LottieHeaderInvalid)
    };
    let (width, height) = (number("w")?, number("h")?);
    let (first_frame, last_frame) = (number("ip")?, number("op")?);
    let side_limit = MAX_PROBED_IMAGE_DIMENSION as f64;
    if !(0.0..=side_limit).contains(&width) || !(0.0..=side_limit).contains(&height) {
        return Err(DecodeGuardError::LottieHeaderInvalid);
    }
    if last_frame < first_frame {
        return Err(DecodeGuardError::LottieHeaderInvalid);
    }
    check_lottie_timeline((last_frame - first_frame) as u64, number("fr")?)
}

/// checks frame count and rate of an animation
pub fn check_lottie_timeline(frames: u64, frame_rate: f64) -> Result<(), DecodeGuardError> {
    let limit = client_limit(LOTTIE_FRAMES_KEY);
    if frames > limit {
        return Err(DecodeGuardError::TooManyFrames { frames, limit });
    }
    if frame_rate <= 0.0 || !(0.0..=MAX_MEDIA_FPS as f64).contains(&frame_rate) {
        return Err(DecodeGuardError::FrameRateOutOfRange);
    }
    Ok(())
}

/// checks the buffer of one rasterised frame, a square of 32 bit words
pub fn check_lottie_frame(dimension: u32) -> Result<(), DecodeGuardError> {
    let side = u64::from(dimension);
    let bytes = side
        .saturating_mul(side)
        .saturating_mul(u64::from(FrameFormat::Bgra8.bytes_per_luma_sample()));
    if bytes > LOTTIE_DECODED_BYTES {
        return Err(DecodeGuardError::FrameMemoryTooLarge { bytes, limit: LOTTIE_DECODED_BYTES });
    }
    Ok(())
}

fn nesting_is_within(json: &[u8], limit: u64) -> bool {
    let mut depth = 0_u64;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in json {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > limit {
                    return false;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}

/// checks every video stream a demuxer found before a codec opens
#[cfg(feature = "ffmpeg")]
pub fn check_ffmpeg_input(
    input: &rsmpeg::avformat::AVFormatContextInput,
) -> Result<(), DecodeGuardError> {
    use rsmpeg::ffi;

    for stream in input.streams() {
        let parameters = stream.codecpar();
        if parameters.codec_type != ffi::AVMEDIA_TYPE_VIDEO {
            continue;
        }
        check_video_frame(parameters.width.unsigned_abs(), parameters.height.unsigned_abs())?;
        let rate = stream.avg_frame_rate;
        let known = rate.num > 0 && rate.den > 0;
        check_video_timeline(known.then(|| f64::from(rate.num) / f64::from(rate.den)), None)?;
    }
    let known = input.duration > 0 && input.duration != ffi::AV_NOPTS_VALUE;
    let seconds = known.then(|| input.duration as f64 / f64::from(ffi::AV_TIME_BASE));
    check_video_timeline(None, seconds)
}

/// sets max_pixels so libavcodec refuses oversized frames, even a size change mid stream
/// call before avcodec_open2
#[cfg(feature = "ffmpeg")]
pub fn limit_codec_pixels(context: &mut rsmpeg::avcodec::AVCodecContext) {
    let pixels = client_limit(VIDEO_FRAME_BYTES_KEY)
        / u64::from(FrameFormat::Bgra8.bytes_per_luma_sample());
    unsafe {
        (*context.as_mut_ptr()).max_pixels = i64::try_from(pixels).unwrap_or(i64::MAX);
    }
}

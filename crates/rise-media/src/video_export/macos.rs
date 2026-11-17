use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_long, c_ulong, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use core_foundation::base::TCFType;
use core_foundation::base::CFGetTypeID;
use core_foundation::data::CFData;
use core_foundation::string::CFString;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{Message, msg_send};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSPoint, NSRect, NSSize, NSString, NSURL};
use parking_lot::Mutex;

use super::video_export::*;

/// coremedia time as laid out in CMTime.h
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CmTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

unsafe impl Encode for CmTime {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[Encoding::LongLong, Encoding::Int, Encoding::UInt, Encoding::LongLong]);
}

unsafe impl RefEncode for CmTime {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// time range, start and duration
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CmTimeRange {
    start: CmTime,
    duration: CmTime,
}

unsafe impl Encode for CmTimeRange {
    const ENCODING: Encoding = Encoding::Struct("?", &[CmTime::ENCODING, CmTime::ENCODING]);
}

/// affine transform a b c d tx ty
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CgAffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

unsafe impl Encode for CgAffineTransform {
    const ENCODING: Encoding = Encoding::Struct(
        "CGAffineTransform",
        &[
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
        ],
    );
}

impl From<Affine> for CgAffineTransform {
    fn from(value: Affine) -> Self {
        Self { a: value.a, b: value.b, c: value.c, d: value.d, tx: value.tx, ty: value.ty }
    }
}

impl From<CgAffineTransform> for Affine {
    fn from(value: CgAffineTransform) -> Self {
        Self { a: value.a, b: value.b, c: value.c, d: value.d, tx: value.tx, ty: value.ty }
    }
}

#[repr(C)]
struct CgImage {
    _private: [u8; 0],
}

unsafe impl RefEncode for CgImage {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

/// opaque pixel buffer ref, encoded as the method signatures declare it
#[repr(C)]
struct CvPixelBuffer {
    _private: [u8; 0],
}

unsafe impl RefEncode for CvPixelBuffer {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CVBuffer", &[]));
}

/// opaque pixel buffer pool ref, encoded as the adaptor property declares it
#[repr(C)]
struct CvPixelBufferPool {
    _private: [u8; 0],
}

unsafe impl RefEncode for CvPixelBufferPool {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CVPixelBufferPool", &[]));
}

const TIMESCALE: i32 = 600;
const INVALID_TRACK_ID: i32 = 0;
const AV_EXPORT_COMPLETED: isize = 3;
const AV_EXPORT_CANCELLED: isize = 5;
const BGRX: u32 = 6 | (2 << 12);
const RGBA_STRAIGHT: u32 = 3;
const RENDERING_INTENT_DEFAULT: i32 = 0;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: &'static NSString;
    static AVMediaTypeAudio: &'static NSString;
    static AVFileTypeMPEG4: &'static NSString;
    static AVAssetExportPreset640x480: &'static NSString;
    static AVAssetExportPreset1280x720: &'static NSString;
    static AVAssetExportPreset1920x1080: &'static NSString;
    static AVAssetExportPreset3840x2160: &'static NSString;
    static AVAssetExportPresetMediumQuality: &'static NSString;
    static AVAssetExportPresetHighestQuality: &'static NSString;
    static AVVideoCodecKey: &'static NSString;
    static AVVideoCodecTypeH264: &'static NSString;
    static AVVideoWidthKey: &'static NSString;
    static AVVideoHeightKey: &'static NSString;
    static AVVideoCompressionPropertiesKey: &'static NSString;
    static AVVideoAverageBitRateKey: &'static NSString;
    static AVVideoProfileLevelKey: &'static NSString;
    static AVVideoProfileLevelH264HighAutoLevel: &'static NSString;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMTimeZero: CmTime;
    static kCMTimePositiveInfinity: CmTime;
    fn CMTimeMakeWithSeconds(seconds: f64, preferred_timescale: i32) -> CmTime;
    fn CMTimeGetSeconds(time: CmTime) -> f64;
    fn CMTimeMake(value: i64, timescale: i32) -> CmTime;
    static kCMFormatDescriptionExtension_ColorPrimaries: *const c_void;
    static kCMFormatDescriptionExtension_TransferFunction: *const c_void;
    static kCMFormatDescriptionExtension_YCbCrMatrix: *const c_void;
    fn CMFormatDescriptionGetExtension(description: *const c_void, key: *const c_void) -> *const c_void;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: &'static NSString;
    static kCVPixelBufferWidthKey: &'static NSString;
    static kCVPixelBufferHeightKey: &'static NSString;
    static kCVPixelBufferCGImageCompatibilityKey: &'static NSString;
    static kCVPixelBufferCGBitmapContextCompatibilityKey: &'static NSString;
    static kCVPixelBufferIOSurfacePropertiesKey: &'static NSString;
    fn CVPixelBufferCreate(
        allocator: *const c_void,
        width: usize,
        height: usize,
        pixel_format: u32,
        attributes: *const c_void,
        buffer: *mut *mut CvPixelBuffer,
    ) -> i32;
    fn CVPixelBufferLockBaseAddress(buffer: *mut CvPixelBuffer, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut CvPixelBuffer, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut CvPixelBuffer) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut CvPixelBuffer) -> usize;
    fn CVPixelBufferRelease(buffer: *mut CvPixelBuffer);
    fn CVPixelBufferPoolCreatePixelBuffer(
        allocator: *const c_void,
        pool: *mut CvPixelBufferPool,
        buffer: *mut *mut CvPixelBuffer,
    ) -> i32;
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCAGravityResize: &'static NSString;
}

#[link(name = "CoreImage", kind = "framework")]
unsafe extern "C" {
    static kCIImageAutoAdjustEnhance: &'static NSString;
    static kCIImageAutoAdjustRedEye: &'static NSString;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: *const CgImage) -> usize;
    fn CGImageGetHeight(image: *const CgImage) -> usize;
    fn CGImageRelease(image: *const CgImage);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
        provider: *mut c_void,
        decode: *const f64,
        should_interpolate: bool,
        intent: i32,
    ) -> *const CgImage;
    fn CGDataProviderCreateWithCFData(data: *const c_void) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(context: *mut c_void, rect: NSRect, image: *const CgImage);
    fn CGContextRelease(context: *mut c_void);
}

unsafe extern "C" {
    static _NSConcreteStackBlock: [u8; 0];
    fn dispatch_get_global_queue(identifier: c_long, flags: c_ulong) -> *mut AnyObject;
    fn dispatch_async_f(queue: *mut AnyObject, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

fn seconds(value: f64) -> CmTime {
    unsafe { CMTimeMakeWithSeconds(value, TIMESCALE) }
}

fn positive_infinity() -> CmTime {
    unsafe { kCMTimePositiveInfinity }
}

fn zero() -> CmTime {
    unsafe { kCMTimeZero }
}

fn to_seconds(time: CmTime) -> f64 {
    unsafe { CMTimeGetSeconds(time) }
}

fn range(span: TimeSpan) -> CmTimeRange {
    CmTimeRange { start: seconds(span.start_seconds), duration: seconds(span.duration_seconds) }
}

fn class(name: &'static std::ffi::CStr) -> Result<&'static AnyClass, VideoExportError> {
    AnyClass::get(name).ok_or_else(|| {
        VideoExportError::EncoderUnavailable(format!("AVFoundation has no {}", name.to_string_lossy()))
    })
}

fn new_object(name: &'static std::ffi::CStr) -> Result<Retained<AnyObject>, VideoExportError> {
    let class = class(name)?;
    let object: Option<Retained<AnyObject>> = unsafe { msg_send![class, new] };
    object.ok_or_else(|| VideoExportError::CompositionFailed(format!("{} new returned nil", name.to_string_lossy())))
}

fn file_url(path: &Path) -> Result<Retained<NSURL>, VideoExportError> {
    let text = path
        .to_str()
        .ok_or_else(|| VideoExportError::SourceUnreadable(path.to_path_buf(), "the path is not UTF-8".into()))?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(text)))
}

/// url asset over a local file
fn url_asset(path: &Path) -> Result<Retained<AnyObject>, VideoExportError> {
    std::fs::metadata(path).map_err(|error| VideoExportError::SourceUnreadable(path.to_path_buf(), error.to_string()))?;
    let url = file_url(path)?;
    let asset_class = class(c"AVURLAsset")?;
    let none: Option<&AnyObject> = None;
    let asset: Option<Retained<AnyObject>> = unsafe { msg_send![asset_class, URLAssetWithURL: &*url, options: none] };
    asset.ok_or_else(|| VideoExportError::SourceUnreadable(path.to_path_buf(), "AVURLAsset returned nil".into()))
}

fn remote_asset(url: &str) -> Result<Retained<AnyObject>, VideoExportError> {
    let url = NSURL::URLWithString(&NSString::from_str(url))
        .ok_or_else(|| VideoExportError::RemoteUnreadable("NSURL does not parse the URL".into()))?;
    let asset_class = class(c"AVURLAsset")?;
    let none: Option<&AnyObject> = None;
    let asset: Option<Retained<AnyObject>> = unsafe { msg_send![asset_class, URLAssetWithURL: &*url, options: none] };
    asset.ok_or_else(|| VideoExportError::RemoteUnreadable("AVURLAsset returned nil".into()))
}

/// how far from the asked time the generator may take its frame
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tolerance {
    Exact,
    Any,
    Filmstrip,
}

/// why the image generator gave no frame
enum GeneratorFailure {
    Class(VideoExportError),
    NoImage(Option<Retained<NSError>>),
    Undrawable,
}

/// one upright frame from the image generator, no side longer than max_side
fn generated_frame(
    asset: &AnyObject,
    at_seconds: f64,
    max_side: u32,
    tolerance: Tolerance,
) -> Result<PosterFrame, GeneratorFailure> {
    let generator_class = class(c"AVAssetImageGenerator").map_err(GeneratorFailure::Class)?;
    let side = f64::from(max_side);
    let mut error: *mut NSError = std::ptr::null_mut();
    let image: *const CgImage = unsafe {
        let generator: Retained<AnyObject> = msg_send![generator_class, assetImageGeneratorWithAsset: asset];
        let _: () = msg_send![&*generator, setAppliesPreferredTrackTransform: Bool::YES];
        let _: () = msg_send![&*generator, setMaximumSize: NSSize::new(side, side)];
        if tolerance == Tolerance::Any {
            let _: () = msg_send![&*generator, setRequestedTimeToleranceBefore: positive_infinity()];
            let _: () = msg_send![&*generator, setRequestedTimeToleranceAfter: positive_infinity()];
        } else if tolerance == Tolerance::Filmstrip {
            let _: () = msg_send![&*generator, setRequestedTimeToleranceBefore: seconds(0.2)];
            let _: () = msg_send![&*generator, setRequestedTimeToleranceAfter: seconds(0.2)];
        }
        msg_send![
            &*generator,
            copyCGImageAtTime: seconds(at_seconds.max(0.0)),
            actualTime: std::ptr::null_mut::<CmTime>(),
            error: &mut error
        ]
    };
    if image.is_null() {
        let error = unsafe { Retained::retain(error) };
        return Err(GeneratorFailure::NoImage(error));
    }
    let frame = bgrx_of(image);
    unsafe { CGImageRelease(image) };
    frame.ok_or(GeneratorFailure::Undrawable)
}

/// one frame of a local file through the image generator with the given tolerance
fn local_frame(source: &Path, at_seconds: f64, max_side: u32, tolerance: Tolerance) -> Result<PosterFrame, VideoExportError> {
    let asset = url_asset(source)?;
    if tracks(&asset, video_type()).is_empty() {
        return Err(VideoExportError::NoVideoTrack(source.to_path_buf()));
    }
    generated_frame(&asset, at_seconds, max_side, tolerance).map_err(|failure| match failure {
        GeneratorFailure::Class(error) => error,
        GeneratorFailure::NoImage(error) => {
            VideoExportError::PosterFailed(describe(error, || "AVAssetImageGenerator returned no image".into()))
        }
        GeneratorFailure::Undrawable => VideoExportError::PosterFailed("the frame could not be drawn".into()),
    })
}

fn remote_reason(error: Option<Retained<NSError>>, url: &str) -> String {
    let Some(error) = error else {
        return "AVAssetImageGenerator returned no image".into();
    };
    let head = format!("{} {}", error.domain(), error.code());
    let text = error.localizedDescription().to_string();
    let host = url.split_once("://").map_or(url, |(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(rest));
    if text.contains("://") || (!host.is_empty() && text.contains(host)) {
        head
    } else {
        format!("{head}: {text}")
    }
}

fn tracks(asset: &AnyObject, media_type: &NSString) -> Vec<Retained<AnyObject>> {
    let list: Retained<NSArray<AnyObject>> = unsafe { msg_send![asset, tracksWithMediaType: media_type] };
    list.to_vec()
}

fn video_type() -> &'static NSString {
    unsafe { AVMediaTypeVideo }
}

fn audio_type() -> &'static NSString {
    unsafe { AVMediaTypeAudio }
}

fn asset_duration(asset: &AnyObject) -> f64 {
    let duration: CmTime = unsafe { msg_send![asset, duration] };
    to_seconds(duration)
}

fn preset_name(preset: ExportPreset) -> &'static NSString {
    unsafe {
        match preset {
            ExportPreset::Size640x480 => AVAssetExportPreset640x480,
            ExportPreset::Size1280x720 => AVAssetExportPreset1280x720,
            ExportPreset::Size1920x1080 => AVAssetExportPreset1920x1080,
            ExportPreset::Size3840x2160 => AVAssetExportPreset3840x2160,
            ExportPreset::MediumQuality => AVAssetExportPresetMediumQuality,
            ExportPreset::HighestQuality => AVAssetExportPresetHighestQuality,
        }
    }
}

fn describe(error: Option<Retained<NSError>>, fallback: impl FnOnce() -> String) -> String {
    error.map_or_else(fallback, |error| error.localizedDescription().to_string())
}

pub(super) struct NativeVideoExport;

impl VideoExportSource for NativeVideoExport {
    fn support(&self) -> Result<(), VideoExportError> {
        class(c"AVAssetExportSession")?;
        class(c"AVMutableComposition")?;
        Ok(())
    }

    fn probe(&self, source: &Path) -> Result<VideoProbe, VideoExportError> {
        let byte_count = std::fs::metadata(source)
            .map_err(|error| VideoExportError::SourceUnreadable(source.to_path_buf(), error.to_string()))?
            .len();
        let asset = url_asset(source)?;
        let readable: bool = unsafe { msg_send![&*asset, isReadable] };
        if !readable {
            return Err(VideoExportError::SourceUnreadable(source.to_path_buf(), "AVFoundation cannot read it".into()));
        }
        let video = tracks(&asset, video_type()).into_iter().next();
        let has_audio = !tracks(&asset, audio_type()).is_empty();
        let (video_start_seconds, video_duration_seconds) = match &video {
            Some(track) => {
                let span: CmTimeRange = unsafe { msg_send![&**track, timeRange] };
                let known = |value: f64| if value.is_finite() && value >= 0.0 { value } else { 0.0 };
                (known(to_seconds(span.start)), known(to_seconds(span.duration)))
            }
            None => (0.0, 0.0),
        };
        let (natural_size, preferred_transform, nominal_frame_rate) = match &video {
            Some(track) => {
                let (size, transform, rate): (NSSize, CgAffineTransform, f32) = unsafe {
                    (
                        msg_send![&**track, naturalSize],
                        msg_send![&**track, preferredTransform],
                        msg_send![&**track, nominalFrameRate],
                    )
                };
                (FrameSize::new(size.width, size.height), Affine::from(transform), rate)
            }
            None => (FrameSize::default(), Affine::IDENTITY, 0.0),
        };
        Ok(VideoProbe {
            duration_seconds: asset_duration(&asset),
            video_start_seconds,
            video_duration_seconds,
            natural_size,
            preferred_transform,
            nominal_frame_rate,
            has_video: video.is_some(),
            has_audio,
            byte_count,
        })
    }

    fn poster(&self, source: &Path, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
        local_frame(source, at_seconds, max_side, Tolerance::Exact)
    }

    fn filmstrip_frame(&self, source: &Path, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
        local_frame(source, at_seconds, max_side, Tolerance::Filmstrip)
    }

    fn remote_poster(&self, url: &str, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
        let asset = remote_asset(url)?;
        generated_frame(&asset, at_seconds, max_side, Tolerance::Any).map_err(|failure| match failure {
            GeneratorFailure::Class(error) => error,
            GeneratorFailure::NoImage(error) => VideoExportError::RemoteUnreadable(remote_reason(error, url)),
            GeneratorFailure::Undrawable => VideoExportError::PosterFailed("the frame could not be drawn".into()),
        })
    }

    fn start_export(&self, job: ExportJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        clear_target(&job.target)?;
        let built = if job.is_single_source() { single_source(&job)? } else { composed(&job)? };
        let session = export_session(&job, &built)?;
        let control = Arc::new(SessionControl(session.clone()));
        let id = NEXT_EXPORT.fetch_add(1, Ordering::Relaxed);
        RUNNING.lock().get_or_insert_with(HashMap::new).insert(
            id,
            Running { session: SessionControl(session.clone()), target: job.target.clone(), sink: done },
        );
        let block = IdBlock {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: BLOCK_HAS_SIGNATURE,
            reserved: 0,
            invoke: export_answered as *const c_void,
            descriptor: &EXPORT_DESCRIPTOR,
            id,
        };
        unsafe {
            let _: () = msg_send![&*session, exportAsynchronouslyWithCompletionHandler: &raw const block];
        }
        Ok(ExportTicket::new(control))
    }

    fn start_still_video(&self, job: StillVideoJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        autoreleasepool(|_| start_still(job, done))
    }

    fn frame_writer(&self, spec: &FrameWriterSpec) -> Result<Box<dyn FrameSink>, VideoExportError> {
        let sink = autoreleasepool(|_| frame_sink(spec))?;
        Ok(Box::new(sink))
    }
}

/// removes an earlier output, the writer and export session refuse an existing file
fn clear_target(target: &Path) -> Result<(), VideoExportError> {
    match std::fs::remove_file(target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(VideoExportError::TargetFailed(error.to_string())),
    }
}

/// what the export session reads, the asset and its time range and the optional compositions
struct Built {
    asset: Retained<AnyObject>,
    time_range: Option<CmTimeRange>,
    video_composition: Option<Retained<AnyObject>>,
    audio_mix: Option<Retained<AnyObject>>,
}

fn single_source(job: &ExportJob) -> Result<Built, VideoExportError> {
    let segment = &job.segments[0];
    let asset = url_asset(&segment.source)?;
    let video = tracks(&asset, video_type())
        .into_iter()
        .next()
        .ok_or_else(|| VideoExportError::NoVideoTrack(segment.source.clone()))?;
    let volume = segment.volume.unwrap_or(0.0);
    let audio_mix = if volume < 0.999 {
        let parameters = tracks(&asset, audio_type())
            .iter()
            .map(|track| mix_parameters(track, volume))
            .collect::<Result<Vec<_>, _>>()?;
        Some(audio_mix(&parameters)?)
    } else {
        None
    };
    let duration = asset_duration(&asset);
    let video_composition = match &job.render {
        Some(render) if render.draws_every_frame() => Some(filtered_composition(
            &asset,
            render,
            &[(segment.transform, 0.0)],
            track_upright(&video),
            &source_tags(&video),
        )?),
        Some(render) => Some(rendered_composition(render, &[(video, segment.transform, 0.0)], duration)?),
        None if job.cap_frame_rate => Some(capped_composition(&asset)?),
        None => None,
    };
    Ok(Built { asset, time_range: segment.span.map(range), video_composition, audio_mix })
}

/// segments, their sound and extra audio layers on one composition timeline
fn composed(job: &ExportJob) -> Result<Built, VideoExportError> {
    let composition = new_object(c"AVMutableComposition")?;
    let composition_video = add_track(&composition, video_type())?;
    let mut parameters = Vec::new();
    let mut layers: Vec<(Retained<AnyObject>, Affine, f64)> = Vec::new();
    let mut first_preferred: Option<CgAffineTransform> = None;
    let mut first_upright = Affine::IDENTITY;
    let mut first_tags = SourceTags::default();
    let mut cursor = 0.0_f64;
    for segment in &job.segments {
        let asset = url_asset(&segment.source)?;
        let video = tracks(&asset, video_type())
            .into_iter()
            .next()
            .ok_or_else(|| VideoExportError::NoVideoTrack(segment.source.clone()))?;
        let span = segment
            .span
            .unwrap_or(TimeSpan { start_seconds: 0.0, duration_seconds: asset_duration(&asset) });
        let at = segment.at_seconds.unwrap_or(cursor);
        insert(&composition_video, span, &video, at)?;
        if first_preferred.is_none() {
            first_preferred = Some(unsafe { msg_send![&*video, preferredTransform] });
            first_upright = track_upright(&video);
            first_tags = source_tags(&video);
        }
        layers.push((composition_video.clone(), segment.transform, at));
        if let (Some(volume), Some(sound)) = (segment.volume, tracks(&asset, audio_type()).into_iter().next()) {
            let composition_audio = add_track(&composition, audio_type())?;
            insert(&composition_audio, span, &sound, at)?;
            parameters.push(mix_parameters(&composition_audio, volume)?);
        }
        cursor = at + span.duration_seconds;
    }
    for layer in &job.audio {
        let asset = url_asset(&layer.source)?;
        let Some(sound) = tracks(&asset, audio_type()).into_iter().next() else {
            return Err(VideoExportError::SourceUnreadable(layer.source.clone(), "no audio track".into()));
        };
        let placement = layer.placement.unwrap_or(AudioPlacement {
            span: TimeSpan { start_seconds: 0.0, duration_seconds: asset_duration(&asset).min(cursor) },
            at_seconds: 0.0,
        });
        if placement.span.duration_seconds.is_nan() || placement.span.duration_seconds <= 0.0 {
            continue;
        }
        let composition_audio = add_track(&composition, audio_type())?;
        insert(&composition_audio, placement.span, &sound, placement.at_seconds)?;
        parameters.push(mix_parameters(&composition_audio, layer.volume)?);
    }
    if let Some(preferred) = first_preferred {
        unsafe {
            let _: () = msg_send![&*composition_video, setPreferredTransform: preferred];
        }
    }
    let total = asset_duration(&composition);
    let video_composition = match &job.render {
        Some(render) if render.draws_every_frame() => {
            let timeline: Vec<(Affine, f64)> = layers.iter().map(|(_, transform, at)| (*transform, *at)).collect();
            Some(filtered_composition(&composition, render, &timeline, first_upright, &first_tags)?)
        }
        Some(render) => Some(rendered_composition(render, &layers, total)?),
        None if job.cap_frame_rate => Some(capped_composition(&composition)?),
        None => None,
    };
    let audio_mix = if parameters.is_empty() { None } else { Some(audio_mix(&parameters)?) };
    Ok(Built { asset: composition, time_range: None, video_composition, audio_mix })
}

fn add_track(composition: &AnyObject, media_type: &NSString) -> Result<Retained<AnyObject>, VideoExportError> {
    let track: Option<Retained<AnyObject>> =
        unsafe { msg_send![composition, addMutableTrackWithMediaType: media_type, preferredTrackID: INVALID_TRACK_ID] };
    track.ok_or_else(|| VideoExportError::CompositionFailed("the composition refused a track".into()))
}

fn insert(target: &AnyObject, span: TimeSpan, source: &AnyObject, at_seconds: f64) -> Result<(), VideoExportError> {
    let mut error: *mut NSError = std::ptr::null_mut();
    let inserted: bool = unsafe {
        msg_send![target, insertTimeRange: range(span), ofTrack: source, atTime: seconds(at_seconds), error: &mut error]
    };
    if inserted {
        return Ok(());
    }
    let error = unsafe { Retained::retain(error) };
    Err(VideoExportError::CompositionFailed(describe(error, || "insertTimeRange failed".into())))
}

fn mix_parameters(track: &AnyObject, volume: f32) -> Result<Retained<AnyObject>, VideoExportError> {
    let class = class(c"AVMutableAudioMixInputParameters")?;
    unsafe {
        let parameters: Retained<AnyObject> = msg_send![class, audioMixInputParametersWithTrack: track];
        let _: () = msg_send![&*parameters, setVolume: volume, atTime: zero()];
        Ok(parameters)
    }
}

fn audio_mix(parameters: &[Retained<AnyObject>]) -> Result<Retained<AnyObject>, VideoExportError> {
    let mix = new_object(c"AVMutableAudioMix")?;
    let list = NSArray::from_retained_slice(parameters);
    unsafe {
        let _: () = msg_send![&*mix, setInputParameters: &*list];
    }
    Ok(mix)
}

/// frame rate cap and nothing else changed, frame duration 1/30
fn capped_composition(asset: &AnyObject) -> Result<Retained<AnyObject>, VideoExportError> {
    let class = class(c"AVMutableVideoComposition")?;
    unsafe {
        let composition: Option<Retained<AnyObject>> = msg_send![class, videoCompositionWithPropertiesOfAsset: asset];
        let composition =
            composition.ok_or_else(|| VideoExportError::CompositionFailed("no video composition for the asset".into()))?;
        let rate = CmTime { value: 1, timescale: CAPPED_FRAME_RATE as i32, flags: 1, epoch: 0 };
        let _: () = msg_send![&*composition, setFrameDuration: rate];
        Ok(composition)
    }
}

/// the edit rendered, each segment drawn through its layer transform from its start
/// at render size and frame rate, with the overlay over every frame
fn rendered_composition(
    render: &RenderPlan,
    layers: &[(Retained<AnyObject>, Affine, f64)],
    total_seconds: f64,
) -> Result<Retained<AnyObject>, VideoExportError> {
    let composition = new_object(c"AVMutableVideoComposition")?;
    let instruction = new_object(c"AVMutableVideoCompositionInstruction")?;
    let layer_class = class(c"AVMutableVideoCompositionLayerInstruction")?;
    let mut layer_instructions: Vec<Retained<AnyObject>> = Vec::new();
    let mut by_track: Vec<(*const AnyObject, Retained<AnyObject>)> = Vec::new();
    for (track, transform, at) in layers {
        let key = Retained::as_ptr(track);
        let existing = by_track.iter().find(|(seen, _)| *seen == key).map(|(_, layer)| layer.clone());
        let layer = match existing {
            Some(layer) => layer,
            None => {
                let layer: Retained<AnyObject> =
                    unsafe { msg_send![layer_class, videoCompositionLayerInstructionWithAssetTrack: &**track] };
                by_track.push((key, layer.clone()));
                layer_instructions.push(layer.clone());
                layer
            }
        };
        unsafe {
            let _: () = msg_send![&*layer, setTransform: CgAffineTransform::from(*transform), atTime: seconds(*at)];
        }
    }
    let rate = i32::try_from(render.frame_rate).unwrap_or(DEFAULT_FRAME_RATE as i32);
    let list = NSArray::from_retained_slice(&layer_instructions);
    let instructions = NSArray::from_retained_slice(std::slice::from_ref(&instruction));
    let size = NSSize::new(f64::from(render.size.0), f64::from(render.size.1));
    unsafe {
        let whole = CmTimeRange { start: zero(), duration: seconds(total_seconds) };
        let _: () = msg_send![&*instruction, setTimeRange: whole];
        let _: () = msg_send![&*instruction, setLayerInstructions: &*list];
        let _: () = msg_send![&*composition, setInstructions: &*instructions];
        let _: () = msg_send![&*composition, setRenderSize: size];
        let duration = CmTime { value: 1, timescale: rate.max(1), flags: 1, epoch: 0 };
        let _: () = msg_send![&*composition, setFrameDuration: duration];
    }
    if let Some(overlay) = &render.overlay {
        let tool = overlay_tool(overlay, size)?;
        unsafe {
            let _: () = msg_send![&*composition, setAnimationTool: &*tool];
        }
    }
    Ok(composition)
}

/// preferred transform of the track moved so the displayed frame starts at the origin
fn track_upright(track: &AnyObject) -> Affine {
    let (size, preferred): (NSSize, CgAffineTransform) =
        unsafe { (msg_send![track, naturalSize], msg_send![track, preferredTransform]) };
    let preferred = Affine::from(preferred);
    let (min_x, min_y, _) = preferred.bounds_of(FrameSize::new(size.width, size.height));
    if min_x.is_finite() && min_y.is_finite() {
        preferred.then(Affine::translation(-min_x, -min_y))
    } else {
        preferred
    }
}

/// the edit drawn by core image on every frame, color adjustments and geometry and crop
/// overlay, looping stickers and the watermark badge on top
/// segment layer transform is applied after undoing upright
fn filtered_composition(
    asset: &AnyObject,
    render: &RenderPlan,
    layers: &[(Affine, f64)],
    upright: Affine,
    tags: &SourceTags,
) -> Result<Retained<AnyObject>, VideoExportError> {
    let undo_upright = upright
        .inverted()
        .ok_or_else(|| VideoExportError::CompositionFailed("the preferred transform collapses the frame".into()))?;
    let mut segments: Vec<(f64, Affine)> = layers.iter().map(|(transform, at)| (*at, undo_upright.then(*transform))).collect();
    segments.sort_by(|left, right| left.0.total_cmp(&right.0));
    let overlay = match &render.overlay {
        Some(overlay) => Some(overlay_image(overlay)?),
        None => None,
    };
    let render_height = f64::from(render.size.1);
    let loops = render
        .loop_layers
        .iter()
        .map(|layer| placed_loop(layer, render_height))
        .collect::<Result<Vec<_>, _>>()?;
    let badge = match &render.badge {
        Some(badge) => Some(PlacedBadge {
            mark: overlay_image(&badge.image)?,
            height: f64::from(badge.image.height),
            path: badge.clone(),
        }),
        None => None,
    };
    let filter = Arc::new(FrameFilter { adjustments: render.adjustments, size: render.size, segments, overlay, loops, badge });
    let class = class(c"AVMutableVideoComposition")?;
    let block = FilterBlock {
        isa: (&raw const _NSConcreteStackBlock).cast(),
        flags: BLOCK_HAS_COPY_DISPOSE | BLOCK_HAS_SIGNATURE,
        reserved: 0,
        invoke: filter_frame as *const c_void,
        descriptor: &FILTER_DESCRIPTOR,
        filter: Arc::into_raw(filter),
    };
    let composition: Option<Retained<AnyObject>> = unsafe {
        msg_send![class, videoCompositionWithAsset: asset, applyingCIFiltersWithHandler: &raw const block]
    };
    unsafe { drop(Arc::from_raw(block.filter)) };
    let composition =
        composition.ok_or_else(|| VideoExportError::CompositionFailed("no filtering composition for the asset".into()))?;
    let rate = i32::try_from(render.frame_rate).unwrap_or(DEFAULT_FRAME_RATE as i32);
    unsafe {
        let size = NSSize::new(f64::from(render.size.0), f64::from(render.size.1));
        let _: () = msg_send![&*composition, setRenderSize: size];
        let duration = CmTime { value: 1, timescale: rate.max(1), flags: 1, epoch: 0 };
        let _: () = msg_send![&*composition, setFrameDuration: duration];
    }
    if render.badge.is_some() {
        let colors = ColorTags::of_source(
            tags.primaries.as_deref(),
            tags.transfer.as_deref(),
            tags.matrix.as_deref(),
            render.size,
        );
        set_color_tags(&composition, colors);
    }
    Ok(composition)
}

/// colour tags of the first format description of a track, each only when its a string
#[derive(Default)]
struct SourceTags {
    primaries: Option<String>,
    transfer: Option<String>,
    matrix: Option<String>,
}

fn source_tags(track: &AnyObject) -> SourceTags {
    let first: Option<Retained<AnyObject>> = unsafe {
        let list: Retained<NSArray<AnyObject>> = msg_send![track, formatDescriptions];
        msg_send![&*list, firstObject]
    };
    let Some(description) = first else { return SourceTags::default() };
    let read = |key: *const c_void| -> Option<String> {
        unsafe {
            let value = CMFormatDescriptionGetExtension(Retained::as_ptr(&description).cast(), key);
            if value.is_null() || CFGetTypeID(value) != CFString::type_id() {
                return None;
            }
            Some(CFString::wrap_under_get_rule(value.cast()).to_string())
        }
    };
    let keys = unsafe {
        (
            kCMFormatDescriptionExtension_ColorPrimaries,
            kCMFormatDescriptionExtension_TransferFunction,
            kCMFormatDescriptionExtension_YCbCrMatrix,
        )
    };
    SourceTags { primaries: read(keys.0), transfer: read(keys.1), matrix: read(keys.2) }
}

fn set_color_tags(composition: &AnyObject, tags: ColorTags) {
    let primaries = NSString::from_str(tags.primaries);
    let transfer = NSString::from_str(tags.transfer);
    let matrix = NSString::from_str(tags.matrix);
    unsafe {
        let _: () = msg_send![composition, setColorPrimaries: &*primaries];
        let _: () = msg_send![composition, setColorTransferFunction: &*transfer];
        let _: () = msg_send![composition, setColorYCbCrMatrix: &*matrix];
    }
}

/// frames of a looping layer made core images once, each already placed in the y-up render space
struct PlacedLoop {
    frame_duration: f64,
    frames: Vec<SharedImage>,
}

fn placed_loop(layer: &LoopLayer, render_height: f64) -> Result<PlacedLoop, VideoExportError> {
    let frames = layer
        .frames
        .iter()
        .map(|frame| {
            let picture = overlay_image(&Overlay { width: frame.width, height: frame.height, rgba: frame.rgba.clone() })?;
            let into = flip(f64::from(frame.height)).then(frame.placement).then(flip(render_height));
            Ok(SharedImage(transformed(&picture.0, into)))
        })
        .collect::<Result<Vec<_>, VideoExportError>>()?;
    Ok(PlacedLoop { frame_duration: layer.frame_duration, frames })
}

/// the watermark as a core image and the stretches it stands in
struct PlacedBadge {
    mark: SharedImage,
    height: f64,
    path: MovingBadge,
}

impl PlacedBadge {
    fn drawn_over(&self, image: Retained<AnyObject>, at_seconds: f64, render_height: f64) -> Retained<AnyObject> {
        let Some(spot) = self.path.spot_at(at_seconds) else { return image };
        let mut mark = self.mark.0.clone();
        if spot.opacity < 1.0 {
            mark = filtered(mark, "CIColorMatrix", |filter| set_vector4(filter, "inputAVector", 0.0, 0.0, 0.0, spot.opacity));
        }
        let mark = transformed(&mark, Affine::translation(spot.x, render_height - spot.y - self.height));
        unsafe { msg_send![&*mark, imageByCompositingOverImage: &*image] }
    }
}

/// the overlay as an image, drawn over every filtered frame
fn overlay_image(overlay: &Overlay) -> Result<SharedImage, VideoExportError> {
    let image = rgba_image(overlay)
        .ok_or_else(|| VideoExportError::CompositionFailed("the overlay could not be made an image".into()))?;
    let ci_class = class(c"CIImage");
    let picture: Option<Retained<AnyObject>> = match ci_class {
        Ok(ci_class) => unsafe { msg_send![ci_class, imageWithCGImage: image] },
        Err(_) => None,
    };
    unsafe { CGImageRelease(image) };
    picture
        .map(SharedImage)
        .ok_or_else(|| VideoExportError::CompositionFailed("the overlay could not be made a Core Image".into()))
}

struct SharedImage(Retained<AnyObject>);

unsafe impl Send for SharedImage {}
unsafe impl Sync for SharedImage {}

struct FrameFilter {
    adjustments: ColorAdjustments,
    size: (u32, u32),
    segments: Vec<(f64, Affine)>,
    overlay: Option<SharedImage>,
    loops: Vec<PlacedLoop>,
    badge: Option<PlacedBadge>,
}

impl FrameFilter {
    fn geometry_at(&self, at_seconds: f64) -> Affine {
        self.segments
            .iter()
            .rev()
            .find(|(start, _)| *start <= at_seconds)
            .or(self.segments.first())
            .map_or(Affine::IDENTITY, |(_, transform)| *transform)
    }

    fn render(&self, source: &Retained<AnyObject>, at_seconds: f64) -> Option<Retained<AnyObject>> {
        let extent = image_extent(source);
        if !finite_positive(extent) {
            return None;
        }
        let (render_width, render_height) = (f64::from(self.size.0), f64::from(self.size.1));
        let adjusted = adjusted_image(source, self.adjustments);
        let (mut image, geometry) = if finite_positive(image_extent(&adjusted)) {
            (cropped(&adjusted, extent), self.geometry_at(at_seconds))
        } else {
            let (width, height) = (extent.size.width, extent.size.height);
            let scale = (render_width / width).max(render_height / height);
            if !scale.is_finite() || scale <= 0.0 {
                return None;
            }
            let fill = Affine::scale(scale, scale)
                .then(Affine::translation((render_width - width * scale) / 2.0, (render_height - height * scale) / 2.0));
            (source.clone(), fill)
        };
        let into_core_image = Affine::translation(-extent.origin.x, -extent.origin.y)
            .then(flip(extent.size.height))
            .then(geometry)
            .then(flip(render_height));
        image = transformed(&image, into_core_image);
        let target = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(render_width, render_height));
        image = cropped(&image, target);
        if let Some(overlay) = &self.overlay {
            image = unsafe { msg_send![&*overlay.0, imageByCompositingOverImage: &*image] };
        }
        for placed in &self.loops {
            let frame = loop_frame_index(placed.frames.len(), placed.frame_duration, at_seconds)
                .and_then(|index| placed.frames.get(index));
            if let Some(frame) = frame {
                image = unsafe { msg_send![&*frame.0, imageByCompositingOverImage: &*image] };
            }
        }
        if let Some(badge) = &self.badge {
            image = badge.drawn_over(image, at_seconds, render_height);
        }
        Some(cropped(&image, target))
    }
}

fn flip(height: f64) -> Affine {
    Affine::scale(1.0, -1.0).then(Affine::translation(0.0, height))
}

fn finite_positive(rect: NSRect) -> bool {
    [rect.origin.x, rect.origin.y, rect.size.width, rect.size.height].iter().all(|value| value.is_finite())
        && rect.size.width > 0.0
        && rect.size.height > 0.0
}

fn image_extent(image: &AnyObject) -> NSRect {
    unsafe { msg_send![image, extent] }
}

fn cropped(image: &AnyObject, rect: NSRect) -> Retained<AnyObject> {
    unsafe { msg_send![image, imageByCroppingToRect: rect] }
}

fn transformed(image: &AnyObject, transform: Affine) -> Retained<AnyObject> {
    unsafe { msg_send![image, imageByApplyingTransform: CgAffineTransform::from(transform)] }
}

fn named_filter(name: &str) -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"CIFilter")?;
    let name = NSString::from_str(name);
    unsafe { msg_send![class, filterWithName: &*name] }
}

fn set_input<T: Message>(filter: &AnyObject, key: &str, value: &T) {
    let key = NSString::from_str(key);
    unsafe {
        let _: () = msg_send![filter, setValue: value, forKey: &*key];
    }
}

fn set_number(filter: &AnyObject, key: &str, value: f64) {
    set_input(filter, key, &*NSNumber::new_f32(value as f32));
}

fn set_vector4(filter: &AnyObject, key: &str, x: f64, y: f64, z: f64, w: f64) {
    let Some(class) = AnyClass::get(c"CIVector") else { return };
    let vector: Retained<AnyObject> = unsafe { msg_send![class, vectorWithX: x, Y: y, Z: z, W: w] };
    set_input(filter, key, &*vector);
}

fn set_vector(filter: &AnyObject, key: &str, x: f64, y: f64) {
    let Some(class) = AnyClass::get(c"CIVector") else { return };
    let vector: Retained<AnyObject> = unsafe { msg_send![class, vectorWithX: x, Y: y] };
    set_input(filter, key, &*vector);
}

fn output_image(filter: &AnyObject) -> Option<Retained<AnyObject>> {
    unsafe { msg_send![filter, outputImage] }
}

/// one named filter over image with inputs set
/// the image itself if the filter is missing or gives nothing
fn filtered(image: Retained<AnyObject>, name: &str, inputs: impl FnOnce(&AnyObject)) -> Retained<AnyObject> {
    let Some(filter) = named_filter(name) else { return image };
    set_input(&filter, "inputImage", &*image);
    inputs(&filter);
    output_image(&filter).unwrap_or(image)
}

fn adjusted_image(source: &Retained<AnyObject>, adjustments: ColorAdjustments) -> Retained<AnyObject> {
    let mut image = source.clone();
    if let Some(fraction) = adjustments.enhance_fraction() {
        for filter in auto_adjustment_filters(source) {
            set_input(&filter, "inputImage", &*image);
            if let Some(enhanced) = output_image(&filter) {
                image = filtered(image, "CIDissolveTransition", |dissolve| {
                    set_input(dissolve, "inputTargetImage", &*enhanced);
                    set_number(dissolve, "inputTime", fraction);
                });
            }
        }
    }
    if let Some(controls) = adjustments.color_controls() {
        image = filtered(image, "CIColorControls", |filter| {
            set_number(filter, "inputBrightness", controls.brightness);
            set_number(filter, "inputContrast", controls.contrast);
            set_number(filter, "inputSaturation", controls.saturation);
        });
    }
    if let Some(kelvin) = adjustments.target_neutral_kelvin() {
        image = filtered(image, "CITemperatureAndTint", |filter| {
            set_vector(filter, "inputNeutral", NEUTRAL_KELVIN, 0.0);
            set_vector(filter, "inputTargetNeutral", kelvin, 0.0);
        });
    }
    if let Some(bend) = adjustments.highlight_shadow() {
        image = filtered(image, "CIHighlightShadowAdjust", |filter| {
            set_number(filter, "inputHighlightAmount", bend.highlight_amount);
            set_number(filter, "inputShadowAmount", bend.shadow_amount);
        });
    }
    if let Some(intensity) = adjustments.vignette_intensity() {
        image = filtered(image, "CIVignette", |filter| {
            set_number(filter, "inputIntensity", intensity);
            set_number(filter, "inputRadius", VIGNETTE_RADIUS);
        });
    }
    image
}

fn auto_adjustment_filters(source: &AnyObject) -> Vec<Retained<AnyObject>> {
    let yes = NSNumber::new_bool(true);
    let no = NSNumber::new_bool(false);
    let keys: [&NSString; 2] = unsafe { [kCIImageAutoAdjustEnhance, kCIImageAutoAdjustRedEye] };
    let options = NSDictionary::<NSString, NSNumber>::from_slices(&keys, &[&*yes, &*no]);
    let list: Retained<NSArray<AnyObject>> = unsafe { msg_send![source, autoAdjustmentFiltersWithOptions: &*options] };
    list.to_vec()
}

fn overlay_tool(overlay: &Overlay, size: NSSize) -> Result<Retained<AnyObject>, VideoExportError> {
    let image = rgba_image(overlay)
        .ok_or_else(|| VideoExportError::CompositionFailed("the overlay could not be made an image".into()))?;
    let layer_class = class(c"CALayer")?;
    let tool_class = class(c"AVVideoCompositionCoreAnimationTool")?;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
    unsafe {
        let parent: Retained<AnyObject> = msg_send![layer_class, layer];
        let video: Retained<AnyObject> = msg_send![layer_class, layer];
        let picture: Retained<AnyObject> = msg_send![layer_class, layer];
        for layer in [&parent, &video, &picture] {
            let _: () = msg_send![&**layer, setFrame: frame];
        }
        let contents: *const AnyObject = image.cast();
        let _: () = msg_send![&*picture, setContents: contents];
        let _: () = msg_send![&*picture, setContentsGravity: kCAGravityResize];
        CGImageRelease(image);
        let _: () = msg_send![&*parent, addSublayer: &*video];
        let _: () = msg_send![&*parent, addSublayer: &*picture];
        let tool: Option<Retained<AnyObject>> = msg_send![
            tool_class,
            videoCompositionCoreAnimationToolWithPostProcessingAsVideoLayer: &*video,
            inLayer: &*parent
        ];
        tool.ok_or_else(|| VideoExportError::CompositionFailed("no animation tool for the overlay".into()))
    }
}

/// a +1 cgimage over a copy of the straight alpha rgba pixels of the overlay
fn rgba_image(overlay: &Overlay) -> Option<*const CgImage> {
    let width = usize::try_from(overlay.width).ok()?;
    let height = usize::try_from(overlay.height).ok()?;
    let bytes_per_row = width.checked_mul(4)?;
    if overlay.rgba.len() != bytes_per_row.checked_mul(height)? {
        return None;
    }
    let data = CFData::from_buffer(&overlay.rgba[..]);
    unsafe {
        let provider = CGDataProviderCreateWithCFData(data.as_concrete_TypeRef().cast());
        if provider.is_null() {
            return None;
        }
        let space = CGColorSpaceCreateDeviceRGB();
        let image = CGImageCreate(
            width,
            height,
            8,
            32,
            bytes_per_row,
            space,
            RGBA_STRAIGHT,
            provider,
            std::ptr::null(),
            false,
            RENDERING_INTENT_DEFAULT,
        );
        CGColorSpaceRelease(space);
        CGDataProviderRelease(provider);
        (!image.is_null()).then_some(image)
    }
}

fn bgrx_of(image: *const CgImage) -> Option<PosterFrame> {
    unsafe {
        let width = CGImageGetWidth(image);
        let height = CGImageGetHeight(image);
        if width == 0 || height == 0 {
            return None;
        }
        let bytes_per_row = width.checked_mul(4)?;
        let mut bgra = vec![0u8; bytes_per_row.checked_mul(height)?];
        let space = CGColorSpaceCreateDeviceRGB();
        let context = CGBitmapContextCreate(bgra.as_mut_ptr().cast(), width, height, 8, bytes_per_row, space, BGRX);
        CGColorSpaceRelease(space);
        if context.is_null() {
            return None;
        }
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width as f64, height as f64));
        CGContextDrawImage(context, rect, image);
        CGContextRelease(context);
        for pixel in bgra.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Some(PosterFrame { width: u32::try_from(width).ok()?, height: u32::try_from(height).ok()?, bgra })
    }
}

fn export_session(job: &ExportJob, built: &Built) -> Result<Retained<AnyObject>, VideoExportError> {
    let session_class = class(c"AVAssetExportSession")?;
    let url = file_url(&job.target).map_err(|_| VideoExportError::TargetFailed("the target path is not UTF-8".into()))?;
    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![session_class, alloc];
        let session: Option<Retained<AnyObject>> =
            msg_send![allocated, initWithAsset: &*built.asset, presetName: preset_name(job.preset)];
        let session = session.ok_or(VideoExportError::PresetUnavailable(job.preset))?;
        let supported: Retained<NSArray<NSString>> = msg_send![&*session, supportedFileTypes];
        if supported.count() > 0 && !supported.containsObject(AVFileTypeMPEG4) {
            return Err(VideoExportError::Mp4Unsupported);
        }
        let _: () = msg_send![&*session, setOutputURL: &*url];
        let _: () = msg_send![&*session, setOutputFileType: AVFileTypeMPEG4];
        let _: () = msg_send![&*session, setShouldOptimizeForNetworkUse: Bool::YES];
        if let Some(limit) = job.file_length_limit {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let _: () = msg_send![&*session, setFileLengthLimit: limit];
        }
        if let Some(time_range) = built.time_range {
            let _: () = msg_send![&*session, setTimeRange: time_range];
        }
        if let Some(video_composition) = &built.video_composition {
            let _: () = msg_send![&*session, setVideoComposition: &**video_composition];
        }
        if let Some(audio_mix) = &built.audio_mix {
            let _: () = msg_send![&*session, setAudioMix: &**audio_mix];
        }
        Ok(session)
    }
}

struct SessionControl(Retained<AnyObject>);

unsafe impl Send for SessionControl {}
unsafe impl Sync for SessionControl {}

impl ExportControl for SessionControl {
    fn progress(&self) -> f32 {
        unsafe { msg_send![&*self.0, progress] }
    }

    fn cancel(&self) {
        unsafe {
            let _: () = msg_send![&*self.0, cancelExport];
        }
    }
}

struct Running {
    session: SessionControl,
    target: PathBuf,
    sink: ExportSink,
}

static NEXT_EXPORT: AtomicU64 = AtomicU64::new(1);
static RUNNING: Mutex<Option<HashMap<u64, Running>>> = Mutex::new(None);

const BLOCK_HAS_SIGNATURE: c_int = 1 << 30;
const BLOCK_HAS_COPY_DISPOSE: c_int = 1 << 25;

#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
    signature: *const c_char,
}

unsafe impl Sync for BlockDescriptor {}

#[repr(C)]
struct IdBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: *const c_void,
    descriptor: *const BlockDescriptor,
    id: u64,
}

unsafe impl RefEncode for IdBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static EXPORT_DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<IdBlock>() as c_ulong,
    signature: c"v8@?0".as_ptr(),
};

unsafe extern "C" fn export_answered(block: *const IdBlock) {
    let id = unsafe { (*block).id };
    let Some(running) = RUNNING.lock().as_mut().and_then(|table| table.remove(&id)) else {
        return;
    };
    let (status, error): (isize, Option<Retained<NSError>>) =
        unsafe { (msg_send![&*running.session.0, status], msg_send![&*running.session.0, error]) };
    let outcome = match status {
        AV_EXPORT_COMPLETED => match std::fs::metadata(&running.target) {
            Ok(metadata) => ExportOutcome::Completed { path: running.target.clone(), byte_count: metadata.len() },
            Err(error) => ExportOutcome::Failed(VideoExportError::TargetFailed(error.to_string())),
        },
        AV_EXPORT_CANCELLED => {
            let _ = std::fs::remove_file(&running.target);
            ExportOutcome::Cancelled
        }
        _ => {
            let _ = std::fs::remove_file(&running.target);
            ExportOutcome::Failed(VideoExportError::ExportFailed(describe(error, || {
                format!("export session ended with status {status}")
            })))
        }
    };
    let sink = running.sink;
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || sink(outcome))).is_err() {
        tracing::error!(target: "riseonly", "video export sink panicked");
    }
}

#[repr(C)]
struct CopyDisposeDescriptor {
    reserved: c_ulong,
    size: c_ulong,
    copy: unsafe extern "C" fn(*mut FilterBlock, *const FilterBlock),
    dispose: unsafe extern "C" fn(*const FilterBlock),
    signature: *const c_char,
}

unsafe impl Sync for CopyDisposeDescriptor {}

#[repr(C)]
struct FilterBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: *const c_void,
    descriptor: *const CopyDisposeDescriptor,
    filter: *const FrameFilter,
}

unsafe impl RefEncode for FilterBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static FILTER_DESCRIPTOR: CopyDisposeDescriptor = CopyDisposeDescriptor {
    reserved: 0,
    size: std::mem::size_of::<FilterBlock>() as c_ulong,
    copy: filter_block_copy,
    dispose: filter_block_dispose,
    signature: c"v16@?0@8".as_ptr(),
};

unsafe extern "C" fn filter_block_copy(_destination: *mut FilterBlock, source: *const FilterBlock) {
    unsafe { Arc::increment_strong_count((*source).filter) };
}

unsafe extern "C" fn filter_block_dispose(block: *const FilterBlock) {
    unsafe { Arc::decrement_strong_count((*block).filter) };
}

unsafe extern "C" fn filter_frame(block: *const FilterBlock, request: *mut AnyObject) {
    let filter = unsafe { &*(*block).filter };
    let Some(request) = (unsafe { request.as_ref() }) else { return };
    autoreleasepool(|_| {
        let (source, time): (Retained<AnyObject>, CmTime) =
            unsafe { (msg_send![request, sourceImage], msg_send![request, compositionTime]) };
        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| filter.render(&source, to_seconds(time))))
            .unwrap_or_else(|_| {
                tracing::error!(target: "riseonly", "video export frame filter panicked");
                None
            });
        let image = rendered.unwrap_or(source);
        let no_context: Option<&AnyObject> = None;
        unsafe {
            let _: () = msg_send![request, finishWithImage: &*image, context: no_context];
        }
    });
}

const PIXEL_FORMAT_BGRA: u32 = u32::from_be_bytes(*b"BGRA");
const CV_RETURN_SUCCESS: i32 = 0;
const QOS_CLASS_USER_INITIATED: c_long = 0x19;
const AV_WRITER_COMPLETED: isize = 2;

/// objects one still video is written with
struct StillWriter {
    writer: Retained<AnyObject>,
    input: Retained<AnyObject>,
    adaptor: Retained<AnyObject>,
    pixels: *mut CvPixelBuffer,
}

unsafe impl Send for StillWriter {}
unsafe impl Sync for StillWriter {}

impl Drop for StillWriter {
    fn drop(&mut self) {
        unsafe { CVPixelBufferRelease(self.pixels) };
    }
}

struct StillRunning {
    parts: Arc<StillWriter>,
    ticks: StillVideoTicks,
    target: PathBuf,
    sink: ExportSink,
    appended: bool,
}

static STILL_RUNNING: Mutex<Option<HashMap<u64, StillRunning>>> = Mutex::new(None);

fn global_queue() -> *mut AnyObject {
    unsafe { dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0) }
}

fn ticks(value: i64) -> CmTime {
    unsafe { CMTimeMake(value, STILL_VIDEO_TIMESCALE) }
}

fn number(value: u32) -> Retained<AnyObject> {
    NSNumber::new_u32(value).into()
}

/// the 32bgra buffer the picture is drawn into, filled row by row from the packed frame
/// its own row stride may be wider
fn pixel_buffer(frame: &PosterFrame) -> Result<*mut CvPixelBuffer, VideoExportError> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    unsafe {
        let empty = NSDictionary::<NSString, AnyObject>::new();
        let attributes = NSDictionary::<NSString, AnyObject>::from_retained_objects(
            &[
                kCVPixelBufferCGImageCompatibilityKey,
                kCVPixelBufferCGBitmapContextCompatibilityKey,
                kCVPixelBufferIOSurfacePropertiesKey,
            ],
            &[NSNumber::new_bool(true).into(), NSNumber::new_bool(true).into(), empty.into()],
        );
        let mut pixels: *mut CvPixelBuffer = std::ptr::null_mut();
        let created = CVPixelBufferCreate(
            std::ptr::null(),
            width,
            height,
            PIXEL_FORMAT_BGRA,
            Retained::as_ptr(&attributes).cast(),
            &mut pixels,
        );
        if created != CV_RETURN_SUCCESS || pixels.is_null() {
            return Err(VideoExportError::ExportFailed(format!("CVPixelBufferCreate failed with {created}")));
        }
        if let Err(error) = fill_pixels(pixels, &frame.bgra, width, height) {
            CVPixelBufferRelease(pixels);
            return Err(error);
        }
        Ok(pixels)
    }
}

/// copies packed bgra rows into a 32bgra buffer of width x height with a maybe wider stride
/// the caller keeps ownership of the buffer
fn fill_pixels(pixels: *mut CvPixelBuffer, bgra: &[u8], width: usize, height: usize) -> Result<(), VideoExportError> {
    let row = width * 4;
    if bgra.len() != row * height {
        return Err(VideoExportError::InvalidJob("the frame is not one BGRA frame of its size"));
    }
    unsafe {
        if CVPixelBufferLockBaseAddress(pixels, 0) != CV_RETURN_SUCCESS {
            return Err(VideoExportError::ExportFailed("the pixel buffer could not be locked".into()));
        }
        let base = CVPixelBufferGetBaseAddress(pixels).cast::<u8>();
        let stride = CVPixelBufferGetBytesPerRow(pixels);
        if base.is_null() || stride < row {
            CVPixelBufferUnlockBaseAddress(pixels, 0);
            return Err(VideoExportError::ExportFailed("the pixel buffer has no room for the frame".into()));
        }
        for (index, source) in bgra.chunks_exact(row).enumerate().take(height) {
            std::ptr::copy_nonoverlapping(source.as_ptr(), base.add(index * stride), row);
        }
        CVPixelBufferUnlockBaseAddress(pixels, 0);
    }
    Ok(())
}

fn h264_output_settings(width: u32, height: u32, average_bit_rate: u32) -> Retained<NSDictionary<NSString, AnyObject>> {
    unsafe {
        let compression = NSDictionary::<NSString, AnyObject>::from_retained_objects(
            &[AVVideoAverageBitRateKey, AVVideoProfileLevelKey],
            &[number(average_bit_rate), AVVideoProfileLevelH264HighAutoLevel.retain().into()],
        );
        NSDictionary::<NSString, AnyObject>::from_retained_objects(
            &[AVVideoCodecKey, AVVideoWidthKey, AVVideoHeightKey, AVVideoCompressionPropertiesKey],
            &[AVVideoCodecTypeH264.retain().into(), number(width), number(height), compression.into()],
        )
    }
}

/// mpeg4 writer at target with one h264 input, not real time
/// plus the pixel buffer adaptor over it
fn h264_writer(
    target: &Path,
    width: u32,
    height: u32,
    average_bit_rate: u32,
    source_attributes: &NSDictionary<NSString, AnyObject>,
) -> Result<(Retained<AnyObject>, Retained<AnyObject>, Retained<AnyObject>), VideoExportError> {
    let writer_class = class(c"AVAssetWriter")?;
    let input_class = class(c"AVAssetWriterInput")?;
    let adaptor_class = class(c"AVAssetWriterInputPixelBufferAdaptor")?;
    let url = file_url(target).map_err(|_| VideoExportError::TargetFailed("the target path is not UTF-8".into()))?;
    let settings = h264_output_settings(width, height, average_bit_rate);
    unsafe {
        let made: Result<Retained<AnyObject>, Retained<NSError>> =
            msg_send![writer_class, assetWriterWithURL: &*url, fileType: AVFileTypeMPEG4, error: _];
        let writer = made.map_err(|error| VideoExportError::TargetFailed(error.localizedDescription().to_string()))?;
        let input: Option<Retained<AnyObject>> =
            msg_send![input_class, assetWriterInputWithMediaType: AVMediaTypeVideo, outputSettings: &*settings];
        let Some(input) = input else {
            return Err(VideoExportError::EncoderUnavailable("AVAssetWriterInput refused the H.264 settings".into()));
        };
        let _: () = msg_send![&*input, setExpectsMediaDataInRealTime: Bool::NO];
        let adaptor: Option<Retained<AnyObject>> = msg_send![
            adaptor_class,
            assetWriterInputPixelBufferAdaptorWithAssetWriterInput: &*input,
            sourcePixelBufferAttributes: source_attributes
        ];
        let Some(adaptor) = adaptor else {
            return Err(VideoExportError::ExportFailed("the pixel buffer adaptor could not be made".into()));
        };
        Ok((writer, input, adaptor))
    }
}

fn still_writer(job: &StillVideoJob) -> Result<StillWriter, VideoExportError> {
    let source_attributes = unsafe {
        NSDictionary::<NSString, AnyObject>::from_retained_objects(
            &[kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey, kCVPixelBufferHeightKey],
            &[number(PIXEL_FORMAT_BGRA), number(job.frame.width), number(job.frame.height)],
        )
    };
    let (writer, input, adaptor) =
        h264_writer(&job.target, job.frame.width, job.frame.height, job.average_bit_rate, &source_attributes)?;
    let pixels = pixel_buffer(&job.frame)?;
    Ok(StillWriter { writer, input, adaptor, pixels })
}

fn start_still(job: StillVideoJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
    clear_target(&job.target)?;
    let parts = Arc::new(still_writer(&job)?);
    unsafe {
        let can_add: bool = msg_send![&*parts.writer, canAddInput: &*parts.input];
        if !can_add {
            return Err(VideoExportError::ExportFailed("AVAssetWriter cannot add the H.264 input".into()));
        }
        let _: () = msg_send![&*parts.writer, addInput: &*parts.input];
        let started: bool = msg_send![&*parts.writer, startWriting];
        if !started {
            let error: Option<Retained<NSError>> = msg_send![&*parts.writer, error];
            let _ = std::fs::remove_file(&job.target);
            return Err(VideoExportError::ExportFailed(describe(error, || "AVAssetWriter did not start".into())));
        }
        let _: () = msg_send![&*parts.writer, startSessionAtSourceTime: zero()];
    }
    let id = NEXT_EXPORT.fetch_add(1, Ordering::Relaxed);
    STILL_RUNNING.lock().get_or_insert_with(HashMap::new).insert(
        id,
        StillRunning { parts: parts.clone(), ticks: job.ticks(), target: job.target, sink: done, appended: false },
    );
    let block = id_block(id, still_ready);
    unsafe {
        let queue: &AnyObject = &*global_queue();
        let _: () = msg_send![&*parts.input, requestMediaDataWhenReadyOnQueue: queue, usingBlock: &raw const block];
    }
    Ok(ExportTicket::new(Arc::new(StillControl { id })))
}

/// an id block literal for a void handler that carries id
fn id_block(id: u64, invoke: unsafe extern "C" fn(*const IdBlock)) -> IdBlock {
    IdBlock {
        isa: (&raw const _NSConcreteStackBlock).cast(),
        flags: BLOCK_HAS_SIGNATURE,
        reserved: 0,
        invoke: invoke as *const c_void,
        descriptor: &EXPORT_DESCRIPTOR,
        id,
    }
}

/// takes the running still out of the table and sends its outcome once
/// a cancelled or failed writer leaves no partial file
fn finish_still(id: u64, outcome: impl FnOnce(&StillRunning) -> ExportOutcome) {
    let Some(running) = STILL_RUNNING.lock().as_mut().and_then(|table| table.remove(&id)) else {
        return;
    };
    let outcome = outcome(&running);
    if !matches!(outcome, ExportOutcome::Completed { .. }) {
        let _ = std::fs::remove_file(&running.target);
    }
    let sink = running.sink;
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || sink(outcome))).is_err() {
        tracing::error!(target: "riseonly", "still video sink panicked");
    }
}

fn writer_failure(parts: &StillWriter, fallback: &'static str) -> ExportOutcome {
    let error: Option<Retained<NSError>> = unsafe { msg_send![&*parts.writer, error] };
    ExportOutcome::Failed(VideoExportError::ExportFailed(describe(error, || fallback.into())))
}

unsafe extern "C" fn still_ready(block: *const IdBlock) {
    let id = unsafe { (*block).id };
    let claimed = {
        let mut table = STILL_RUNNING.lock();
        match table.as_mut().and_then(|table| table.get_mut(&id)) {
            Some(running) if !running.appended => {
                running.appended = true;
                Some((running.parts.clone(), running.ticks))
            }
            _ => None,
        }
    };
    let Some((parts, still)) = claimed else { return };
    autoreleasepool(|_| {
        let append = |at: CmTime| -> bool {
            unsafe { msg_send![&*parts.adaptor, appendPixelBuffer: parts.pixels, withPresentationTime: at] }
        };
        let ready: bool = unsafe { msg_send![&*parts.input, isReadyForMoreMediaData] };
        let written = ready && append(zero()) && still.repeat.is_none_or(|repeat| append(ticks(repeat)));
        if !written {
            let outcome = writer_failure(&parts, "AVAssetWriter took no frame");
            unsafe {
                let _: () = msg_send![&*parts.writer, cancelWriting];
            }
            finish_still(id, move |_| outcome);
            return;
        }
        let finished = id_block(id, still_written);
        unsafe {
            let _: () = msg_send![&*parts.input, markAsFinished];
            let _: () = msg_send![&*parts.writer, endSessionAtSourceTime: ticks(still.end)];
            let _: () = msg_send![&*parts.writer, finishWritingWithCompletionHandler: &raw const finished];
        }
    });
}

unsafe extern "C" fn still_written(block: *const IdBlock) {
    let id = unsafe { (*block).id };
    autoreleasepool(|_| {
        finish_still(id, |running| {
            let status: isize = unsafe { msg_send![&*running.parts.writer, status] };
            if status != AV_WRITER_COMPLETED {
                return writer_failure(&running.parts, "AVAssetWriter did not complete");
            }
            match std::fs::metadata(&running.target) {
                Ok(metadata) => ExportOutcome::Completed { path: running.target.clone(), byte_count: metadata.len() },
                Err(error) => ExportOutcome::Failed(VideoExportError::TargetFailed(error.to_string())),
            }
        });
    });
}

struct StillControl {
    id: u64,
}

impl ExportControl for StillControl {
    fn progress(&self) -> f32 {
        0.0
    }

    fn cancel(&self) {
        unsafe { dispatch_async_f(global_queue(), std::ptr::without_provenance_mut(self.id as usize), still_cancelled) };
    }
}

extern "C" fn still_cancelled(context: *mut c_void) {
    let id = context.addr() as u64;
    autoreleasepool(|_| {
        finish_still(id, |running| {
            unsafe {
                let _: () = msg_send![&*running.parts.writer, cancelWriting];
            }
            ExportOutcome::Cancelled
        });
    });
}

const AV_WRITER_WRITING: isize = 1;
const FRAME_READY_POLL: std::time::Duration = std::time::Duration::from_millis(2);

static FRAME_FINISHING: Mutex<Option<HashMap<u64, std::sync::mpsc::SyncSender<()>>>> = Mutex::new(None);

struct MacFrameSink {
    writer: Retained<AnyObject>,
    input: Retained<AnyObject>,
    adaptor: Retained<AnyObject>,
    width: usize,
    height: usize,
    frames_per_second: i32,
    finished: bool,
}

unsafe impl Send for MacFrameSink {}

fn frame_sink(spec: &FrameWriterSpec) -> Result<MacFrameSink, VideoExportError> {
    clear_target(&spec.target)?;
    let source_attributes = unsafe {
        NSDictionary::<NSString, AnyObject>::from_retained_objects(
            &[
                kCVPixelBufferPixelFormatTypeKey,
                kCVPixelBufferWidthKey,
                kCVPixelBufferHeightKey,
                kCVPixelBufferCGBitmapContextCompatibilityKey,
            ],
            &[number(PIXEL_FORMAT_BGRA), number(spec.width), number(spec.height), NSNumber::new_bool(true).into()],
        )
    };
    let (writer, input, adaptor) =
        h264_writer(&spec.target, spec.width, spec.height, spec.average_bit_rate, &source_attributes)?;
    let frames_per_second = i32::try_from(spec.frames_per_second)
        .map_err(|_| VideoExportError::InvalidJob("a frame video needs a frame rate"))?;
    let mut sink = MacFrameSink {
        writer,
        input,
        adaptor,
        width: spec.width as usize,
        height: spec.height as usize,
        frames_per_second,
        finished: false,
    };
    unsafe {
        let can_add: bool = msg_send![&*sink.writer, canAddInput: &*sink.input];
        if !can_add {
            sink.finished = true;
            return Err(VideoExportError::ExportFailed("AVAssetWriter cannot add the H.264 input".into()));
        }
        let _: () = msg_send![&*sink.writer, addInput: &*sink.input];
        let started: bool = msg_send![&*sink.writer, startWriting];
        if !started {
            sink.finished = true;
            return Err(sink.failure("AVAssetWriter did not start"));
        }
        let _: () = msg_send![&*sink.writer, startSessionAtSourceTime: zero()];
    }
    Ok(sink)
}

impl MacFrameSink {
    fn failure(&self, fallback: &'static str) -> VideoExportError {
        let error: Option<Retained<NSError>> = unsafe { msg_send![&*self.writer, error] };
        VideoExportError::ExportFailed(describe(error, || fallback.into()))
    }

    fn wait_until_ready(&self) -> Result<(), VideoExportError> {
        loop {
            let ready: bool = unsafe { msg_send![&*self.input, isReadyForMoreMediaData] };
            if ready {
                return Ok(());
            }
            let status: isize = unsafe { msg_send![&*self.writer, status] };
            if status != AV_WRITER_WRITING {
                return Err(self.failure("AVAssetWriter stopped writing"));
            }
            std::thread::sleep(FRAME_READY_POLL);
        }
    }

    fn pooled_pixels(&self, bgra: &[u8]) -> Result<*mut CvPixelBuffer, VideoExportError> {
        unsafe {
            let pool: *mut CvPixelBufferPool = msg_send![&*self.adaptor, pixelBufferPool];
            if pool.is_null() {
                return Err(self.failure("the adaptor has no pixel buffer pool"));
            }
            let mut pixels: *mut CvPixelBuffer = std::ptr::null_mut();
            let created = CVPixelBufferPoolCreatePixelBuffer(std::ptr::null(), pool, &mut pixels);
            if created != CV_RETURN_SUCCESS || pixels.is_null() {
                return Err(VideoExportError::ExportFailed(format!("CVPixelBufferPoolCreatePixelBuffer failed with {created}")));
            }
            if let Err(error) = fill_pixels(pixels, bgra, self.width, self.height) {
                CVPixelBufferRelease(pixels);
                return Err(error);
            }
            Ok(pixels)
        }
    }
}

impl FrameSink for MacFrameSink {
    fn append_bgra(&mut self, bgra: &[u8], index: i64) -> Result<(), VideoExportError> {
        autoreleasepool(|_| {
            self.wait_until_ready()?;
            let pixels = self.pooled_pixels(bgra)?;
            let appended: bool = unsafe {
                let at = CMTimeMake(index, self.frames_per_second);
                let appended: bool = msg_send![&*self.adaptor, appendPixelBuffer: pixels, withPresentationTime: at];
                CVPixelBufferRelease(pixels);
                appended
            };
            if appended { Ok(()) } else { Err(self.failure("AVAssetWriter took no frame")) }
        })
    }

    fn finish(mut self: Box<Self>, duration_seconds: f64) -> Result<(), VideoExportError> {
        self.finished = true;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let id = NEXT_EXPORT.fetch_add(1, Ordering::Relaxed);
        FRAME_FINISHING.lock().get_or_insert_with(HashMap::new).insert(id, sender);
        let finished = id_block(id, frame_written);
        autoreleasepool(|_| {
            unsafe {
                let _: () = msg_send![&*self.input, markAsFinished];
                let _: () = msg_send![&*self.writer, endSessionAtSourceTime: seconds(duration_seconds)];
                let _: () = msg_send![&*self.writer, finishWritingWithCompletionHandler: &raw const finished];
            }
        });
        if receiver.recv().is_err() {
            return Err(VideoExportError::ExportFailed("AVAssetWriter never answered the finish".into()));
        }
        autoreleasepool(|_| {
            let status: isize = unsafe { msg_send![&*self.writer, status] };
            if status == AV_WRITER_COMPLETED { Ok(()) } else { Err(self.failure("AVAssetWriter did not complete")) }
        })
    }
}

impl Drop for MacFrameSink {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        autoreleasepool(|_| {
            unsafe {
                let _: () = msg_send![&*self.writer, cancelWriting];
            }
        });
    }
}

unsafe extern "C" fn frame_written(block: *const IdBlock) {
    let id = unsafe { (*block).id };
    let waiting = FRAME_FINISHING.lock().as_mut().and_then(|table| table.remove(&id));
    if let Some(sender) = waiting {
        let _ = sender.send(());
    }
}

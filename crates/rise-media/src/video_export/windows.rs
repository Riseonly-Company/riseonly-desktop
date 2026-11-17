use std::path::Path;

use super::video_export::*;

pub(super) struct NativeVideoExport;

#[cfg(not(feature = "ffmpeg"))]
const NO_FFMPEG: &str = "this build carries no FFmpeg: the crate feature ffmpeg links the LGPL prefix";

#[cfg(not(feature = "ffmpeg"))]
fn unavailable() -> VideoExportError {
    VideoExportError::EncoderUnavailable(NO_FFMPEG.to_owned())
}

#[cfg(not(feature = "ffmpeg"))]
impl VideoExportSource for NativeVideoExport {
    fn support(&self) -> Result<(), VideoExportError> {
        Err(unavailable())
    }

    fn probe(&self, _source: &Path) -> Result<VideoProbe, VideoExportError> {
        Err(unavailable())
    }

    fn poster(&self, _source: &Path, _at_seconds: f64, _max_side: u32) -> Result<PosterFrame, VideoExportError> {
        Err(unavailable())
    }

    fn remote_poster(&self, _url: &str, _at_seconds: f64, _max_side: u32) -> Result<PosterFrame, VideoExportError> {
        Err(unavailable())
    }

    fn start_export(&self, _job: ExportJob, _done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        Err(unavailable())
    }

    fn start_still_video(&self, _job: StillVideoJob, _done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        Err(unavailable())
    }
}

#[cfg(feature = "ffmpeg")]
impl VideoExportSource for NativeVideoExport {
    fn support(&self) -> Result<(), VideoExportError> {
        windows_encoder().map(|_| ())
    }

    fn probe(&self, source: &Path) -> Result<VideoProbe, VideoExportError> {
        super::ffmpeg_export::probe(source)
    }

    fn poster(&self, source: &Path, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
        super::ffmpeg_export::poster(source, at_seconds, max_side)
    }

    fn remote_poster(&self, url: &str, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
        super::ffmpeg_export::remote_poster(url, at_seconds, max_side)
    }

    fn start_export(&self, job: ExportJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        super::ffmpeg_export::start_export(job, done, windows_encoder()?)
    }

    fn start_still_video(&self, job: StillVideoJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        super::ffmpeg_export::start_still_video(job, done, windows_encoder()?)
    }
}

#[cfg(feature = "ffmpeg")]
const MEDIA_FOUNDATION_ENCODER: &str = "h264_mf";

/// h264 encoder this machine can reach, asked once
/// encoders are fixed at link time and media foundation doesnt come and go while the app runs
#[cfg(feature = "ffmpeg")]
fn windows_encoder() -> Result<&'static str, VideoExportError> {
    use std::sync::OnceLock;

    use super::ffmpeg_export::{components, has_encoder};
    use super::ffmpeg_graph::AAC_ENCODER;

    static PICKED: OnceLock<Result<&'static str, VideoExportError>> = OnceLock::new();
    PICKED
        .get_or_init(|| {
            components(false)?;
            if !has_encoder(AAC_ENCODER) {
                return Err(VideoExportError::EncoderUnavailable(format!("libavcodec has no {AAC_ENCODER} encoder")));
            }
            if !has_encoder(MEDIA_FOUNDATION_ENCODER) {
                return Err(VideoExportError::EncoderUnavailable(format!(
                    "libavcodec has no {MEDIA_FOUNDATION_ENCODER} encoder"
                )));
            }
            media_foundation_present()?;
            Ok(MEDIA_FOUNDATION_ENCODER)
        })
        .clone()
}

#[cfg(feature = "ffmpeg")]
fn media_foundation_present() -> Result<(), VideoExportError> {
    use ::windows::Win32::Foundation::FreeLibrary;
    use ::windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    use ::windows::core::w;

    let module = unsafe { LoadLibraryExW(w!("mfplat.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.map_err(|error| {
        VideoExportError::EncoderUnavailable(format!(
            "Media Foundation is missing (mfplat.dll: {error}); N editions need the Media Feature Pack"
        ))
    })?;
    let _ = unsafe { FreeLibrary(module) };
    Ok(())
}

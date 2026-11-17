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
        linux_encoder().map(|_| ())
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
        super::ffmpeg_export::start_export(job, done, linux_encoder()?)
    }

    fn start_still_video(&self, job: StillVideoJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        super::ffmpeg_export::start_still_video(job, done, linux_encoder()?)
    }
}

/// h264 encoder this machine can reach, asked once
/// encoders are fixed at link time and vaapi doesnt come and go while the app runs
#[cfg(feature = "ffmpeg")]
fn linux_encoder() -> Result<&'static str, VideoExportError> {
    use std::sync::OnceLock;

    use super::ffmpeg_export::{components, has_encoder, vaapi_device_opens};
    use super::ffmpeg_graph::pick_video_encoder;

    static PICKED: OnceLock<Result<&'static str, VideoExportError>> = OnceLock::new();
    PICKED
        .get_or_init(|| {
            components(true)?;
            pick_video_encoder(has_encoder, vaapi_device_opens())
        })
        .clone()
}

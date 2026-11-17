use std::path::Path;

use super::audio_capture::*;

pub(super) struct NativeAacEncoder;

#[cfg(not(feature = "ffmpeg"))]
impl AacEncoderSource for NativeAacEncoder {
    fn support(&self) -> Result<(), AudioCaptureError> {
        Err(AudioCaptureError::EncoderUnavailable(super::ffmpeg_aac::feature_off_reason()))
    }

    fn create(&self, _path: &Path, _settings: &AacSettings) -> Result<Box<dyn AacFileWriter>, AudioCaptureError> {
        Err(AudioCaptureError::EncoderUnavailable(super::ffmpeg_aac::feature_off_reason()))
    }
}

#[cfg(feature = "ffmpeg")]
impl AacEncoderSource for NativeAacEncoder {
    fn support(&self) -> Result<(), AudioCaptureError> {
        lgpl::support()
    }

    fn create(&self, path: &Path, settings: &AacSettings) -> Result<Box<dyn AacFileWriter>, AudioCaptureError> {
        lgpl::support()?;
        let plan = super::ffmpeg_aac::FfmpegAacPlan::for_settings(settings)?;
        Ok(Box::new(lgpl::FfmpegAacWriter::create(path, &plan)?))
    }
}

#[cfg(feature = "ffmpeg")]
mod lgpl {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{self, NonNull};
    use std::sync::OnceLock;

    use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVPacket};
    use rsmpeg::avformat::{AVFormatContextOutput, AVOutputFormat};
    use rsmpeg::avutil::{AVChannelLayout, AVFrame};
    use rsmpeg::error::RsmpegError;
    use rsmpeg::ffi;

    use super::super::audio_capture::{AacFileWriter, AudioCaptureError};
    use super::super::ffmpeg_aac::{
        FFMPEG_AAC_ENCODER, FFMPEG_M4A_MUXER, FfmpegAacPlan, PlanarFrame, PlanarFrames, missing_component_reason,
    };

    const AAC_ENCODER: &std::ffi::CStr = c"aac";
    const M4A_MUXER: &std::ffi::CStr = c"ipod";

    pub(super) fn support() -> Result<(), AudioCaptureError> {
        static SUPPORT: OnceLock<Option<String>> = OnceLock::new();
        let missing = SUPPORT.get_or_init(|| {
            debug_assert_eq!(AAC_ENCODER.to_bytes(), FFMPEG_AAC_ENCODER.as_bytes());
            debug_assert_eq!(M4A_MUXER.to_bytes(), FFMPEG_M4A_MUXER.as_bytes());
            let has_encoder = AVCodec::find_encoder_by_name(AAC_ENCODER).is_some();
            let has_muxer = AVOutputFormat::guess_format(Some(M4A_MUXER), None, None).is_some();
            missing_component_reason(has_encoder, has_muxer)
        });
        match missing {
            None => Ok(()),
            Some(reason) => Err(AudioCaptureError::EncoderUnavailable(reason.clone())),
        }
    }

    fn encoder_failed(call: &str, error: RsmpegError) -> AudioCaptureError {
        AudioCaptureError::EncoderFailed(format!("{call}: {error}"))
    }

    fn file_failed(call: &str, error: RsmpegError) -> AudioCaptureError {
        AudioCaptureError::FileFailed(format!("{call}: {error}"))
    }

    pub(super) struct FfmpegAacWriter {
        encoder: AVCodecContext,
        output: AVFormatContextOutput,
        frames: PlanarFrames,
        plan: FfmpegAacPlan,
        stream_time_base: ffi::AVRational,
        file_open: bool,
    }

    impl FfmpegAacWriter {
        pub(super) fn create(path: &Path, plan: &FfmpegAacPlan) -> Result<Self, AudioCaptureError> {
            let filename = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| AudioCaptureError::FileFailed("the path holds a NUL byte".to_owned()))?;

            let mut raw_output = ptr::null_mut();
            let status = unsafe {
                ffi::avformat_alloc_output_context2(&mut raw_output, ptr::null(), M4A_MUXER.as_ptr(), filename.as_ptr())
            };
            let raw_output = NonNull::new(raw_output).filter(|_| status >= 0).ok_or_else(|| {
                AudioCaptureError::EncoderFailed(format!("avformat_alloc_output_context2 failed ({status})"))
            })?;
            let mut output = unsafe { AVFormatContextOutput::from_raw(raw_output) };
            let global_header = output.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32 != 0;

            let codec = AVCodec::find_encoder_by_name(AAC_ENCODER).ok_or_else(|| {
                AudioCaptureError::EncoderUnavailable(
                    missing_component_reason(false, true).unwrap_or_default(),
                )
            })?;
            let mut encoder = AVCodecContext::new(&codec);
            let sample_rate = i32::try_from(plan.sample_rate)
                .map_err(|_| AudioCaptureError::InvalidSettings("the sample rate does not fit libavcodec"))?;
            encoder.set_sample_rate(sample_rate);
            encoder.set_sample_fmt(ffi::AV_SAMPLE_FMT_FLTP);
            encoder.set_ch_layout(AVChannelLayout::from_nb_channels(i32::from(plan.channels)).into_inner());
            encoder.set_bit_rate(i64::from(plan.bit_rate));
            encoder.set_time_base(ffi::AVRational { num: 1, den: sample_rate });
            if global_header {
                let flags = encoder.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
                encoder.set_flags(flags);
            }
            encoder.open(None).map_err(|error| encoder_failed("avcodec_open2", error))?;
            let frame_length = usize::try_from(encoder.frame_size).unwrap_or(0);

            {
                let mut stream = output.new_stream();
                stream.set_codecpar(encoder.extract_codecpar());
                stream.set_time_base(encoder.time_base);
            }

            let mut file = ptr::null_mut();
            let opened = unsafe { ffi::avio_open(&mut file, filename.as_ptr(), ffi::AVIO_FLAG_WRITE as i32) };
            if opened < 0 || file.is_null() {
                return Err(AudioCaptureError::FileFailed(format!(
                    "avio_open {} failed: {}",
                    path.display(),
                    rsmpeg::avutil::err2str(opened).unwrap_or_else(|| opened.to_string())
                )));
            }
            unsafe {
                (*output.as_mut_ptr()).pb = file;
            }
            let mut writer = Self {
                encoder,
                output,
                frames: PlanarFrames::new(plan.channels, frame_length),
                plan: *plan,
                stream_time_base: ffi::AVRational { num: 1, den: sample_rate },
                file_open: true,
            };
            writer
                .output
                .write_header(&mut None)
                .map_err(|error| file_failed("avformat_write_header", error))?;
            writer.stream_time_base = writer.output.streams()[0].time_base;
            Ok(writer)
        }

        fn encode(&mut self, planar: &PlanarFrame) -> Result<(), AudioCaptureError> {
            let mut frame = AVFrame::new();
            frame.set_format(ffi::AV_SAMPLE_FMT_FLTP);
            frame.set_ch_layout(AVChannelLayout::from_nb_channels(i32::from(self.plan.channels)).into_inner());
            frame.set_sample_rate(self.encoder.sample_rate);
            frame.set_nb_samples(
                i32::try_from(planar.frames)
                    .map_err(|_| AudioCaptureError::EncoderFailed("an encoder frame is too long".to_owned()))?,
            );
            frame.alloc_buffer().map_err(|error| encoder_failed("av_frame_get_buffer", error))?;
            for (channel, plane) in planar.planes.iter().enumerate() {
                let data = frame.data_mut()[channel].cast::<f32>();
                if data.is_null() {
                    return Err(AudioCaptureError::EncoderFailed("av_frame_get_buffer left a plane empty".to_owned()));
                }
                unsafe { ptr::copy_nonoverlapping(plane.as_ptr(), data, plane.len()) };
            }
            frame.set_pts(planar.pts);
            self.encoder
                .send_frame(Some(&frame))
                .map_err(|error| encoder_failed("avcodec_send_frame", error))?;
            self.drain()
        }

        fn drain(&mut self) -> Result<(), AudioCaptureError> {
            loop {
                let mut packet: AVPacket = match self.encoder.receive_packet() {
                    Ok(packet) => packet,
                    Err(RsmpegError::EncoderDrainError | RsmpegError::EncoderFlushedError) => return Ok(()),
                    Err(error) => return Err(encoder_failed("avcodec_receive_packet", error)),
                };
                packet.set_stream_index(0);
                packet.rescale_ts(self.encoder.time_base, self.stream_time_base);
                self.output
                    .interleaved_write_frame(&mut packet)
                    .map_err(|error| file_failed("av_interleaved_write_frame", error))?;
            }
        }

        fn close_file(&mut self) -> Result<(), AudioCaptureError> {
            if !self.file_open {
                return Ok(());
            }
            self.file_open = false;
            let status = unsafe { ffi::avio_closep(&mut (*self.output.as_mut_ptr()).pb) };
            if status < 0 {
                return Err(AudioCaptureError::FileFailed(format!(
                    "avio_closep failed: {}",
                    rsmpeg::avutil::err2str(status).unwrap_or_else(|| status.to_string())
                )));
            }
            Ok(())
        }
    }

    impl AacFileWriter for FfmpegAacWriter {
        fn write(&mut self, interleaved: &[f32]) -> Result<(), AudioCaptureError> {
            self.frames.push(interleaved);
            while let Some(planar) = self.frames.pop_full() {
                self.encode(&planar)?;
            }
            Ok(())
        }

        fn finish(mut self: Box<Self>) -> Result<(), AudioCaptureError> {
            if let Some(last) = self.frames.pop_last() {
                self.encode(&last)?;
            }
            self.encoder
                .send_frame(None)
                .map_err(|error| encoder_failed("avcodec_send_frame(flush)", error))?;
            self.drain()?;
            self.output
                .write_trailer()
                .map_err(|error| file_failed("av_write_trailer", error))?;
            self.close_file()
        }
    }

    impl Drop for FfmpegAacWriter {
        fn drop(&mut self) {
            if let Err(error) = self.close_file() {
                tracing::debug!("aac writer dropped with a failing file: {error}");
            }
        }
    }
}

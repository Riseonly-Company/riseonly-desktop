use std::ffi::{CStr, CString, c_int};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVPacket};
use rsmpeg::avformat::{AVFormatContextInput, AVFormatContextOutput, AVOutputFormat};
use rsmpeg::avutil::{AVDictionary, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::ffmpeg_graph::{
    AAC_ENCODER, AUDIO_SINK, DEFAULT_VAAPI_DEVICE, VAAPI_ENCODER, GraphInput, GraphPlan, InputStream, MP4_MUXER,
    OUTPUT_CHANNEL_LAYOUT, OUTPUT_SAMPLE_RATE, VIDEO_SINK, affine_from_display_matrix, audio_bit_rate,
    byte_ceiling_reached, encoder_tail, plan_graph, poster_filters, progress, video_bit_rate,
};
use super::video_export::*;

const REQUIRED_FILTERS: [&CStr; 24] = [
    c"buffer", c"abuffer", c"buffersink", c"abuffersink", c"trim", c"atrim", c"setpts", c"asetpts", c"fps",
    c"scale", c"format", c"aformat", c"hflip", c"vflip", c"transpose", c"rotate", c"overlay", c"color",
    c"amix", c"adelay", c"volume", c"anull", c"null", c"aresample",
];
const UPLOAD_FILTER: &CStr = c"hwupload";
const DECODE_TAIL_SECONDS: f64 = 0.5;
const SEEK_CUR: c_int = 1;
const MICROSECONDS: ffi::AVRational = ffi::AVRational { num: 1, den: ffi::AV_TIME_BASE as i32 };

fn av_text(status: c_int) -> String {
    rsmpeg::avutil::err2str(status).unwrap_or_else(|| status.to_string())
}

fn path_text(path: &Path) -> Option<CString> {
    path.to_str().and_then(|text| CString::new(text).ok())
}

fn c_path(path: &Path) -> Result<CString, VideoExportError> {
    path_text(path).ok_or_else(|| {
        VideoExportError::SourceUnreadable(path.to_path_buf(), "the path is not UTF-8 or holds a NUL byte".to_owned())
    })
}

fn seconds(value: i64, time_base: ffi::AVRational) -> f64 {
    if value == ffi::AV_NOPTS_VALUE || time_base.den == 0 {
        return f64::NAN;
    }
    value as f64 * f64::from(time_base.num) / f64::from(time_base.den)
}

fn has_filter(filter: &CStr) -> bool {
    !unsafe { ffi::avfilter_get_by_name(filter.as_ptr()) }.is_null()
}

/// true when the linked prefix has the muxer and every filter the graph names
/// needs_upload is for an arm whose encoder takes vaapi surfaces
pub(crate) fn components(needs_upload: bool) -> Result<(), VideoExportError> {
    let muxer = CString::new(MP4_MUXER).unwrap_or_default();
    if AVOutputFormat::guess_format(Some(muxer.as_c_str()), None, None).is_none() {
        return Err(VideoExportError::EncoderUnavailable(format!("libavformat has no {MP4_MUXER} muxer")));
    }
    let upload = needs_upload.then_some(UPLOAD_FILTER);
    if let Some(filter) = REQUIRED_FILTERS.into_iter().chain(upload).find(|filter| !has_filter(filter)) {
        return Err(VideoExportError::EncoderUnavailable(format!(
            "libavfilter has no {} filter",
            filter.to_string_lossy()
        )));
    }
    Ok(())
}

/// true if libavcodec has an encoder with that name
pub(crate) fn has_encoder(name: &str) -> bool {
    CString::new(name).ok().and_then(|name| AVCodec::find_encoder_by_name(&name)).is_some()
}

pub(crate) fn vaapi_device_opens() -> bool {
    HwDevice::open().is_ok()
}

/// a vaapi device ref, the graph hwupload and the encoder each take their own ref
struct HwDevice(NonNull<ffi::AVBufferRef>);

impl HwDevice {
    fn open() -> Result<Self, VideoExportError> {
        let node = CString::new(DEFAULT_VAAPI_DEVICE).unwrap_or_default();
        let mut last = 0;
        for device in [node.as_ptr(), ptr::null()] {
            let mut raw = ptr::null_mut();
            let status = unsafe {
                ffi::av_hwdevice_ctx_create(&mut raw, ffi::AV_HWDEVICE_TYPE_VAAPI, device, ptr::null_mut(), 0)
            };
            if status >= 0 {
                if let Some(raw) = NonNull::new(raw) {
                    return Ok(Self(raw));
                }
            }
            last = status;
        }
        Err(VideoExportError::EncoderUnavailable(format!("no VA-API device: {}", av_text(last))))
    }

    fn new_ref(&self) -> *mut ffi::AVBufferRef {
        unsafe { ffi::av_buffer_ref(self.0.as_ptr()) }
    }
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        let mut raw = self.0.as_ptr();
        unsafe { ffi::av_buffer_unref(&mut raw) };
    }
}

fn open_input(path: &Path) -> Result<AVFormatContextInput, VideoExportError> {
    let url = c_path(path)?;
    let input = AVFormatContextInput::open(&url, None, &mut None)
        .map_err(|error| VideoExportError::SourceUnreadable(path.to_path_buf(), error.to_string()))?;
    crate::decode_guard::decode_guard::check_ffmpeg_input(&input)
        .map_err(|refusal| VideoExportError::SourceUnreadable(path.to_path_buf(), refusal.to_string()))?;
    Ok(input)
}

fn best_stream(input: &AVFormatContextInput, media: ffi::AVMediaType) -> Option<usize> {
    input.find_best_stream(media).ok().flatten().map(|(index, _)| index)
}

pub(crate) fn probe(path: &Path) -> Result<VideoProbe, VideoExportError> {
    let input = open_input(path)?;
    let byte_count = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    Ok(probe_input(&input, byte_count))
}

/// probe of an open input, byte_count is the size of the file or remote resource
fn probe_input(input: &AVFormatContextInput, byte_count: u64) -> VideoProbe {
    let video = best_stream(input, ffi::AVMEDIA_TYPE_VIDEO);
    let audio = best_stream(input, ffi::AVMEDIA_TYPE_AUDIO);
    let mut duration = if input.duration > 0 && input.duration != ffi::AV_NOPTS_VALUE {
        input.duration as f64 / f64::from(ffi::AV_TIME_BASE)
    } else {
        0.0
    };
    let mut natural_size = FrameSize::default();
    let mut preferred_transform = Affine::IDENTITY;
    let mut nominal_frame_rate = 0.0_f32;
    let mut video_start_seconds = 0.0;
    let mut video_duration_seconds = 0.0;
    if let Some(index) = video {
        let stream = &input.streams()[index];
        let origin = if input.start_time == ffi::AV_NOPTS_VALUE {
            0.0
        } else {
            input.start_time as f64 / f64::from(ffi::AV_TIME_BASE)
        };
        let start = seconds(stream.start_time, stream.time_base) - origin;
        if start.is_finite() && start >= 0.0 {
            video_start_seconds = start;
        }
        let own = seconds(stream.duration, stream.time_base);
        if own.is_finite() && own > 0.0 {
            video_duration_seconds = own;
        }
        let parameters = stream.codecpar();
        natural_size = FrameSize::new(f64::from(parameters.width.max(0)), f64::from(parameters.height.max(0)));
        preferred_transform = affine_from_display_matrix(display_matrix(&parameters));
        let rate = stream.guess_framerate().filter(|rate| rate.num > 0 && rate.den > 0).unwrap_or(stream.avg_frame_rate);
        if rate.num > 0 && rate.den > 0 {
            nominal_frame_rate = (f64::from(rate.num) / f64::from(rate.den)) as f32;
        }
        if duration <= 0.0 {
            let own = seconds(stream.duration, stream.time_base);
            if own.is_finite() && own > 0.0 {
                duration = own;
            }
        }
    }
    VideoProbe {
        duration_seconds: duration,
        video_start_seconds,
        video_duration_seconds,
        natural_size,
        preferred_transform,
        nominal_frame_rate,
        has_video: video.is_some(),
        has_audio: audio.is_some(),
        byte_count,
    }
}

fn display_matrix(parameters: &ffi::AVCodecParameters) -> Option<[i32; 9]> {
    let entry = unsafe {
        ffi::av_packet_side_data_get(
            parameters.coded_side_data,
            parameters.nb_coded_side_data,
            ffi::AV_PKT_DATA_DISPLAYMATRIX,
        )
    };
    if entry.is_null() {
        return None;
    }
    let entry = unsafe { &*entry };
    if entry.data.is_null() || entry.size < 36 {
        return None;
    }
    let mut matrix = [0_i32; 9];
    for (index, value) in matrix.iter_mut().enumerate() {
        *value = unsafe { ptr::read_unaligned(entry.data.cast::<i32>().add(index)) };
    }
    Some(matrix)
}

/// one decoded stream of one file, timestamps shifted so it starts at zero
struct Source {
    input: AVFormatContextInput,
    stream: usize,
    decoder: AVCodecContext,
    time_base: ffi::AVRational,
    start: i64,
    stop_seconds: Option<f64>,
    pending: Option<AVFrame>,
    flushed: bool,
    finished: bool,
}

impl Source {
    fn open(path: &Path, media: ffi::AVMediaType, stop_seconds: Option<f64>) -> Result<Option<Self>, VideoExportError> {
        let input = open_input(path)?;
        Self::from_input(input, media, stop_seconds, |what| VideoExportError::SourceUnreadable(path.to_path_buf(), what))
    }

    /// decoder for the best stream of the given media type, none if the input has no such stream
    fn from_input(
        input: AVFormatContextInput,
        media: ffi::AVMediaType,
        stop_seconds: Option<f64>,
        unreadable: impl Fn(String) -> VideoExportError,
    ) -> Result<Option<Self>, VideoExportError> {
        let Some((stream, codec)) = input.find_best_stream(media).ok().flatten() else {
            return Ok(None);
        };
        let (time_base, start, decoder) = {
            let av_stream = &input.streams()[stream];
            let mut decoder = AVCodecContext::new(&codec);
            decoder
                .apply_codecpar(&av_stream.codecpar())
                .map_err(|error| unreadable(format!("avcodec_parameters_to_context: {error}")))?;
            decoder.set_pkt_timebase(av_stream.time_base);
            crate::decode_guard::decode_guard::limit_codec_pixels(&mut decoder);
            let start = if av_stream.start_time == ffi::AV_NOPTS_VALUE { 0 } else { av_stream.start_time };
            (av_stream.time_base, start, decoder)
        };
        let mut decoder = decoder;
        decoder.open(None).map_err(|error| unreadable(format!("avcodec_open2: {error}")))?;
        Ok(Some(Self {
            input,
            stream,
            decoder,
            time_base,
            start,
            stop_seconds,
            pending: None,
            flushed: false,
            finished: false,
        }))
    }

    /// jumps to the key frame at or before at_seconds
    /// if the file cant seek decoding from the start still works, just slower
    fn seek(&mut self, at_seconds: f64) {
        if !at_seconds.is_finite() || at_seconds <= 0.0 {
            return;
        }
        let target = unsafe { ffi::av_rescale_q((at_seconds * 1_000_000.0) as i64, MICROSECONDS, self.time_base) };
        let stream = c_int::try_from(self.stream).unwrap_or(-1);
        let status = unsafe {
            ffi::av_seek_frame(
                self.input.as_mut_ptr(),
                stream,
                target.saturating_add(self.start),
                ffi::AVSEEK_FLAG_BACKWARD as c_int,
            )
        };
        if status < 0 {
            tracing::debug!("video export seek failed: {}", av_text(status));
        }
    }

    /// first frame without consuming it, buffer sources take their format from it
    fn peek(&mut self) -> Result<Option<&AVFrame>, VideoExportError> {
        if self.pending.is_none() {
            self.pending = self.decode()?;
        }
        Ok(self.pending.as_ref())
    }

    fn next(&mut self) -> Result<Option<AVFrame>, VideoExportError> {
        if let Some(frame) = self.pending.take() {
            return Ok(Some(frame));
        }
        self.decode()
    }

    fn decode(&mut self) -> Result<Option<AVFrame>, VideoExportError> {
        if self.finished {
            return Ok(None);
        }
        loop {
            match self.decoder.receive_frame() {
                Ok(mut frame) => {
                    let stamp = if frame.best_effort_timestamp != ffi::AV_NOPTS_VALUE {
                        frame.best_effort_timestamp
                    } else {
                        frame.pts
                    };
                    if stamp != ffi::AV_NOPTS_VALUE {
                        frame.set_pts(stamp - self.start);
                    }
                    let at = seconds(frame.pts, self.time_base);
                    if self.stop_seconds.is_some_and(|stop| at.is_finite() && at > stop) {
                        self.finished = true;
                        return Ok(None);
                    }
                    return Ok(Some(frame));
                }
                Err(RsmpegError::DecoderDrainError) if !self.flushed => {}
                Err(RsmpegError::DecoderDrainError | RsmpegError::DecoderFlushedError) => {
                    self.finished = true;
                    return Ok(None);
                }
                Err(error) => return Err(VideoExportError::ExportFailed(format!("avcodec_receive_frame: {error}"))),
            }
            match self.input.read_packet() {
                Ok(Some(packet)) => {
                    if packet.stream_index as usize != self.stream {
                        continue;
                    }
                    match self.decoder.send_packet(Some(&packet)) {
                        Ok(()) | Err(RsmpegError::DecoderFullError) => {}
                        Err(RsmpegError::SendPacketError(status)) if status == ffi::AVERROR_INVALIDDATA => {}
                        Err(error) => {
                            return Err(VideoExportError::ExportFailed(format!("avcodec_send_packet: {error}")));
                        }
                    }
                }
                Ok(None) => {
                    self.flushed = true;
                    let _ = self.decoder.send_packet(None);
                }
                Err(error) => return Err(VideoExportError::ExportFailed(format!("av_read_frame: {error}"))),
            }
        }
    }
}

/// what a buffer source is created with
enum SourceFormat {
    Video { width: i32, height: i32, format: i32, time_base: ffi::AVRational, aspect: ffi::AVRational },
    Audio { sample_rate: i32, format: i32, layout: String, time_base: ffi::AVRational },
}

impl SourceFormat {
    fn of_video(frame: &AVFrame, time_base: ffi::AVRational) -> Self {
        let aspect = if frame.sample_aspect_ratio.num > 0 && frame.sample_aspect_ratio.den > 0 {
            frame.sample_aspect_ratio
        } else {
            ffi::AVRational { num: 1, den: 1 }
        };
        Self::Video { width: frame.width, height: frame.height, format: frame.format, time_base, aspect }
    }

    fn of_audio(frame: &AVFrame, time_base: ffi::AVRational) -> Self {
        Self::Audio {
            sample_rate: frame.sample_rate,
            format: frame.format,
            layout: layout_text(&frame.ch_layout),
            time_base,
        }
    }

    fn filter(&self) -> &'static CStr {
        match self {
            Self::Video { .. } => c"buffer",
            Self::Audio { .. } => c"abuffer",
        }
    }

    fn args(&self) -> String {
        match self {
            Self::Video { width, height, format, time_base, aspect } => format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}",
                time_base.num, time_base.den, aspect.num, aspect.den
            ),
            Self::Audio { sample_rate, format, layout, time_base } => {
                let name = unsafe { ffi::av_get_sample_fmt_name(*format) };
                let name = if name.is_null() {
                    "fltp".to_owned()
                } else {
                    unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
                };
                format!(
                    "time_base={}/{}:sample_rate={sample_rate}:sample_fmt={name}:channel_layout={layout}",
                    time_base.num, time_base.den
                )
            }
        }
    }
}

fn layout_text(layout: &ffi::AVChannelLayout) -> String {
    let mut known: ffi::AVChannelLayout = unsafe { std::mem::zeroed() };
    let channels = layout.nb_channels.max(1);
    let source = if layout.order == ffi::AV_CHANNEL_ORDER_UNSPEC || layout.nb_channels <= 0 {
        unsafe { ffi::av_channel_layout_default(&mut known, channels) };
        &known
    } else {
        layout
    };
    let mut buffer = [0 as std::ffi::c_char; 64];
    let status = unsafe { ffi::av_channel_layout_describe(source, buffer.as_mut_ptr(), buffer.len()) };
    if status < 0 {
        return OUTPUT_CHANNEL_LAYOUT.to_owned();
    }
    unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned()
}

/// configured filter graph with its named sources and sinks, filter contexts die with the graph
struct Graph {
    raw: NonNull<ffi::AVFilterGraph>,
    sources: Vec<(InputStream, NonNull<ffi::AVFilterContext>, bool)>,
    video_sink: NonNull<ffi::AVFilterContext>,
    audio_sink: Option<NonNull<ffi::AVFilterContext>>,
}

impl Drop for Graph {
    fn drop(&mut self) {
        let mut raw = self.raw.as_ptr();
        unsafe { ffi::avfilter_graph_free(&mut raw) };
    }
}

struct InOut(*mut ffi::AVFilterInOut);

impl Drop for InOut {
    fn drop(&mut self) {
        unsafe { ffi::avfilter_inout_free(&mut self.0) };
    }
}

fn graph_failed(call: &str, status: c_int) -> VideoExportError {
    VideoExportError::CompositionFailed(format!("{call}: {}", av_text(status)))
}

impl Graph {
    fn build(
        description: &str,
        inputs: &[(GraphInput, SourceFormat)],
        device: Option<&HwDevice>,
    ) -> Result<Self, VideoExportError> {
        let text = CString::new(description)
            .map_err(|_| VideoExportError::CompositionFailed("the graph holds a NUL byte".to_owned()))?;
        let raw = NonNull::new(unsafe { ffi::avfilter_graph_alloc() })
            .ok_or_else(|| VideoExportError::CompositionFailed("avfilter_graph_alloc failed".to_owned()))?;
        let mut graph = Self { raw, sources: Vec::new(), video_sink: NonNull::dangling(), audio_sink: None };
        let (mut open_inputs, mut open_outputs) = (InOut(ptr::null_mut()), InOut(ptr::null_mut()));
        let status = unsafe {
            ffi::avfilter_graph_parse2(graph.raw.as_ptr(), text.as_ptr(), &mut open_inputs.0, &mut open_outputs.0)
        };
        if status < 0 {
            return Err(graph_failed("avfilter_graph_parse2", status));
        }
        let mut pad = open_inputs.0;
        while !pad.is_null() {
            let entry = unsafe { &*pad };
            pad = entry.next;
            let name = if entry.name.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(entry.name) }.to_string_lossy().into_owned()
            };
            let (input, format) = inputs.iter().find(|(input, _)| input.name == name).ok_or_else(|| {
                VideoExportError::CompositionFailed(format!("the graph input {name} has no source"))
            })?;
            let source = graph.create(format.filter(), &name, Some(&format.args()))?;
            let pad_index = u32::try_from(entry.pad_idx).unwrap_or(0);
            let status = unsafe { ffi::avfilter_link(source.as_ptr(), 0, entry.filter_ctx, pad_index) };
            if status < 0 {
                return Err(graph_failed("avfilter_link", status));
            }
            graph.sources.push((input.stream, source, false));
        }
        let mut pad = open_outputs.0;
        let mut video_sink = None;
        while !pad.is_null() {
            let entry = unsafe { &*pad };
            pad = entry.next;
            let name = if entry.name.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(entry.name) }.to_string_lossy().into_owned()
            };
            let filter = if name == VIDEO_SINK {
                c"buffersink"
            } else if name == AUDIO_SINK {
                c"abuffersink"
            } else {
                return Err(VideoExportError::CompositionFailed(format!("the graph output {name} has no sink")));
            };
            let sink = graph.create(filter, &name, None)?;
            let pad_index = u32::try_from(entry.pad_idx).unwrap_or(0);
            let status = unsafe { ffi::avfilter_link(entry.filter_ctx, pad_index, sink.as_ptr(), 0) };
            if status < 0 {
                return Err(graph_failed("avfilter_link", status));
            }
            if name == VIDEO_SINK {
                video_sink = Some(sink);
            } else {
                graph.audio_sink = Some(sink);
            }
        }
        graph.video_sink = video_sink
            .ok_or_else(|| VideoExportError::CompositionFailed(format!("the graph has no {VIDEO_SINK}")))?;
        if let Some(device) = device {
            graph.give_device(device);
        }
        let status = unsafe { ffi::avfilter_graph_config(graph.raw.as_ptr(), ptr::null_mut()) };
        if status < 0 {
            return Err(graph_failed("avfilter_graph_config", status));
        }
        Ok(graph)
    }

    fn create(&mut self, filter: &CStr, name: &str, args: Option<&str>) -> Result<NonNull<ffi::AVFilterContext>, VideoExportError> {
        let instance = CString::new(format!("rise_{name}")).unwrap_or_default();
        let args = args.map(|args| CString::new(args).unwrap_or_default());
        let mut context = ptr::null_mut();
        let status = unsafe {
            ffi::avfilter_graph_create_filter(
                &mut context,
                ffi::avfilter_get_by_name(filter.as_ptr()),
                instance.as_ptr(),
                args.as_ref().map_or(ptr::null(), |args| args.as_ptr()),
                ptr::null_mut(),
                self.raw.as_ptr(),
            )
        };
        if status < 0 {
            return Err(graph_failed("avfilter_graph_create_filter", status));
        }
        NonNull::new(context)
            .ok_or_else(|| VideoExportError::CompositionFailed("avfilter_graph_create_filter gave nothing".to_owned()))
    }

    fn give_device(&mut self, device: &HwDevice) {
        let graph = unsafe { self.raw.as_ref() };
        for index in 0..graph.nb_filters as usize {
            let context = unsafe { *graph.filters.add(index) };
            if context.is_null() {
                continue;
            }
            let is_upload = unsafe { CStr::from_ptr((*(*context).filter).name) } == c"hwupload";
            if is_upload {
                unsafe { (*context).hw_device_ctx = device.new_ref() };
            }
        }
    }

    fn feed(&mut self, slot: usize, frame: Option<AVFrame>) -> Result<(), VideoExportError> {
        let (_, source, closed) = &mut self.sources[slot];
        let status = match frame {
            Some(mut frame) => {
                unsafe { ffi::av_buffersrc_add_frame_flags(source.as_ptr(), frame.as_mut_ptr(), 0) }
            }
            None => {
                *closed = true;
                unsafe { ffi::av_buffersrc_add_frame_flags(source.as_ptr(), ptr::null_mut(), 0) }
            }
        };
        if status == ffi::AVERROR_EOF {
            *closed = true;
            return Ok(());
        }
        if status < 0 {
            return Err(graph_failed("av_buffersrc_add_frame", status));
        }
        Ok(())
    }

    fn hungriest(&self) -> Option<usize> {
        self.sources
            .iter()
            .enumerate()
            .filter(|(_, (_, _, closed))| !closed)
            .max_by_key(|(_, (_, source, _))| unsafe { ffi::av_buffersrc_get_nb_failed_requests(source.as_ptr()) })
            .map(|(slot, _)| slot)
    }
}

enum Pulled {
    Frame(AVFrame),
    Again,
    End,
}

fn pull(sink: NonNull<ffi::AVFilterContext>) -> Result<Pulled, VideoExportError> {
    let mut frame = AVFrame::new();
    let status = unsafe { ffi::av_buffersink_get_frame(sink.as_ptr(), frame.as_mut_ptr()) };
    if status >= 0 {
        Ok(Pulled::Frame(frame))
    } else if status == ffi::AVERROR(ffi::EAGAIN) {
        Ok(Pulled::Again)
    } else if status == ffi::AVERROR_EOF {
        Ok(Pulled::End)
    } else {
        Err(graph_failed("av_buffersink_get_frame", status))
    }
}

pub(crate) fn poster(path: &Path, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
    let probe = probe(path)?;
    if !probe.has_video {
        return Err(VideoExportError::NoVideoTrack(path.to_path_buf()));
    }
    let (filters, _) = poster_filters(&probe, max_side)?;
    let mut source =
        Source::open(path, ffi::AVMEDIA_TYPE_VIDEO, None)?.ok_or_else(|| VideoExportError::NoVideoTrack(path.to_path_buf()))?;
    source.seek(at_seconds);
    let mut chosen: Option<AVFrame> = None;
    while let Some(frame) = source.next()? {
        let at = seconds(frame.pts, source.time_base);
        let past = at.is_finite() && at > at_seconds;
        if past && chosen.is_some() {
            break;
        }
        chosen = Some(frame);
        if past {
            break;
        }
    }
    let frame = chosen.ok_or_else(|| VideoExportError::PosterFailed("the file decodes no frame".to_owned()))?;
    poster_picture(frame, source.time_base, &filters)
}

/// runs one decoded frame through the poster filters into an opaque bgra picture
fn poster_picture(frame: AVFrame, time_base: ffi::AVRational, filters: &str) -> Result<PosterFrame, VideoExportError> {
    let name = "in_p".to_owned();
    let format = SourceFormat::of_video(&frame, time_base);
    let input = GraphInput { name: name.clone(), stream: InputStream::SegmentVideo(0) };
    let mut graph = Graph::build(&format!("[{name}]{filters}[{VIDEO_SINK}]"), &[(input, format)], None)?;
    graph.feed(0, Some(frame))?;
    graph.feed(0, None)?;
    let Pulled::Frame(picture) = pull(graph.video_sink)? else {
        return Err(VideoExportError::PosterFailed("the filters gave no frame".to_owned()));
    };
    let (width, height) = (u32::try_from(picture.width).unwrap_or(0), u32::try_from(picture.height).unwrap_or(0));
    let stride = usize::try_from(picture.linesize[0]).unwrap_or(0);
    let row = width as usize * 4;
    if width == 0 || height == 0 || picture.data[0].is_null() || stride < row {
        return Err(VideoExportError::PosterFailed("the filters gave an empty frame".to_owned()));
    }
    let mut bgra = Vec::with_capacity(row * height as usize);
    for line in 0..height as usize {
        let bytes = unsafe { std::slice::from_raw_parts(picture.data[0].add(line * stride), row) };
        bgra.extend_from_slice(bytes);
    }
    for pixel in bgra.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    Ok(PosterFrame { width, height, bgra })
}

const REMOTE_CHUNK_BYTES: i64 = 512 * 1_024;
const REMOTE_IO_BUFFER_BYTES: usize = 32 * 1_024;
const REMOTE_WHOLE_BODY_LIMIT: usize = 256 * 1_024 * 1_024;
const REMOTE_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const REMOTE_AVERROR_EIO: c_int = ffi::AVERROR(5);
const REMOTE_AVERROR_EINVAL: c_int = ffi::AVERROR(22);

fn remote_failed(what: &str, error: reqwest::Error) -> String {
    format!("{what}: {}", error.without_url())
}

fn remote_tls_config() -> Option<std::sync::Arc<rustls::ClientConfig>> {
    use rustls_platform_verifier::BuilderVerifierExt;

    static SHARED: std::sync::OnceLock<Option<std::sync::Arc<rustls::ClientConfig>>> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .and_then(|builder| builder.with_platform_verifier())
                .map(|builder| builder.with_no_client_auth());
            match config {
                Ok(config) => Some(std::sync::Arc::new(config)),
                Err(error) => {
                    tracing::error!("the system certificate verifier did not start: {error}");
                    None
                }
            }
        })
        .clone()
}

fn remote_client(url: &str) -> Result<reqwest::Client, VideoExportError> {
    use rise_platform::network_proxy::{ProxyRoute, ProxyTarget, TargetScheme, system_proxy};

    let parsed = reqwest::Url::parse(url)
        .map_err(|error| VideoExportError::RemoteUnreadable(format!("the URL does not parse: {error}")))?;
    let scheme = TargetScheme::parse(parsed.scheme())
        .ok_or_else(|| VideoExportError::RemoteUnreadable("the URL is not http or https".to_owned()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| VideoExportError::RemoteUnreadable("the URL has no host".to_owned()))?;
    let builder = reqwest::Client::builder().timeout(REMOTE_REQUEST_TIMEOUT);
    let builder = match remote_tls_config() {
        Some(config) => builder.use_preconfigured_tls(config.as_ref().clone()),
        None => builder,
    };
    let builder = match system_proxy().route_for(&ProxyTarget::new(scheme, host, parsed.port())) {
        ProxyRoute::Direct => builder.no_proxy(),
        ProxyRoute::Via(endpoint) => {
            let mut proxy = reqwest::Proxy::all(endpoint.url())
                .map_err(|error| VideoExportError::RemoteUnreadable(remote_failed("the system proxy", error)))?;
            if let Some(credentials) = &endpoint.credentials {
                proxy = proxy.basic_auth(&credentials.user, &credentials.password);
            }
            builder.proxy(proxy)
        }
    };
    builder.build().map_err(|error| VideoExportError::RemoteUnreadable(remote_failed("the HTTP client", error)))
}

fn content_range(value: &str) -> Option<(i64, Option<i64>)> {
    let rest = value.strip_prefix("bytes ")?;
    let (span, complete) = rest.split_once('/')?;
    let (first, _) = span.split_once('-')?;
    let complete = if complete == "*" { None } else { Some(complete.parse::<i64>().ok()?) };
    Some((first.parse::<i64>().ok()?, complete))
}

/// remote file read with ranged gets on a decoder thread
/// these are the read and seek callbacks of a custom avio context
struct RemoteFile {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    url: String,
    position: i64,
    length: Option<i64>,
    chunk_start: i64,
    chunk: Vec<u8>,
    whole: bool,
}

impl RemoteFile {
    /// fetches the first chunk, which also tells the full length
    fn open(url: &str) -> Result<Self, VideoExportError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| VideoExportError::RemoteUnreadable(format!("no runtime for the request: {error}")))?;
        let mut file = Self {
            runtime,
            client: remote_client(url)?,
            url: url.to_owned(),
            position: 0,
            length: None,
            chunk_start: 0,
            chunk: Vec::new(),
            whole: false,
        };
        file.fetch(0).map_err(VideoExportError::RemoteUnreadable)?;
        Ok(file)
    }

    fn holds(&self, offset: i64) -> bool {
        let end = self.chunk_start.saturating_add(i64::try_from(self.chunk.len()).unwrap_or(i64::MAX));
        offset >= self.chunk_start && offset < end
    }

    /// replaces the chunk with bytes from start, an empty chunk is the end of the file
    fn fetch(&mut self, start: i64) -> Result<(), String> {
        let last = start.saturating_add(REMOTE_CHUNK_BYTES - 1);
        let request = self.client.get(self.url.as_str()).header(reqwest::header::RANGE, format!("bytes={start}-{last}"));
        let (whole, first, complete, body) = self.runtime.block_on(async move {
            let mut response = request.send().await.map_err(|error| remote_failed("the request", error))?;
            let status = response.status();
            let (whole, first, complete, limit) = if status == reqwest::StatusCode::PARTIAL_CONTENT {
                let range = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(content_range)
                    .ok_or_else(|| "a partial answer without a usable Content-Range".to_owned())?;
                let limit = usize::try_from(REMOTE_CHUNK_BYTES).unwrap_or(usize::MAX);
                (false, range.0, range.1, limit)
            } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                return Ok((false, start, None, Vec::new()));
            } else if status.is_success() {
                let declared = response.content_length().and_then(|length| i64::try_from(length).ok());
                (true, 0, declared, REMOTE_WHOLE_BODY_LIMIT)
            } else {
                return Err(format!("the server answered {status}"));
            };
            let mut body = Vec::new();
            while let Some(piece) = response.chunk().await.map_err(|error| remote_failed("the body", error))? {
                if body.len().saturating_add(piece.len()) > limit {
                    return Err(format!("the server sent more than {limit} bytes"));
                }
                body.extend_from_slice(&piece);
            }
            Ok::<_, String>((whole, first, complete, body))
        })?;
        if !whole && first != start && !body.is_empty() {
            return Err(format!("asked for offset {start}, the server sent {first}"));
        }
        if whole {
            self.length = Some(i64::try_from(body.len()).unwrap_or(i64::MAX));
        } else if complete.is_some() {
            self.length = complete;
        }
        self.whole = whole;
        self.chunk_start = first;
        self.chunk = body;
        Ok(())
    }

    fn read(&mut self, buffer: &mut [u8]) -> c_int {
        if self.length.is_some_and(|length| self.position >= length) {
            return ffi::AVERROR_EOF;
        }
        if !self.holds(self.position) {
            if self.whole {
                return ffi::AVERROR_EOF;
            }
            if let Err(reason) = self.fetch(self.position) {
                tracing::warn!(target: "riseonly", %reason, position = self.position, "remote poster read failed");
                return REMOTE_AVERROR_EIO;
            }
            if !self.holds(self.position) {
                return ffi::AVERROR_EOF;
            }
        }
        let offset = usize::try_from(self.position - self.chunk_start).unwrap_or(0);
        let count = (self.chunk.len() - offset).min(buffer.len());
        buffer[..count].copy_from_slice(&self.chunk[offset..offset + count]);
        self.position = self.position.saturating_add(i64::try_from(count).unwrap_or(i64::MAX));
        c_int::try_from(count).unwrap_or(c_int::MAX)
    }

    fn seek(&mut self, offset: i64, whence: c_int) -> i64 {
        use crate::video::av_asset::{AVSEEK_SIZE, avio_seek_target};

        match avio_seek_target(offset, whence, self.position, self.length) {
            Some(target) if (whence & AVSEEK_SIZE) != 0 => target,
            Some(target) => {
                self.position = target;
                target
            }
            None => i64::from(REMOTE_AVERROR_EINVAL),
        }
    }

    /// read only custom io context over this file, read and seek callbacks share it
    fn into_io_context(self) -> rsmpeg::avformat::AVIOContextCustom {
        use rsmpeg::avformat::{AVIOContextCustom, ReadPacketCallback, SeekCallback};
        use rsmpeg::avutil::AVMem;

        let file = Arc::new(parking_lot::Mutex::new(self));
        let read_file = Arc::clone(&file);
        let read_packet: ReadPacketCallback = Box::new(move |_opaque, buffer| read_file.lock().read(buffer));
        let seek: SeekCallback = Box::new(move |_opaque, offset, whence| file.lock().seek(offset, whence));
        AVIOContextCustom::alloc_context(AVMem::new(REMOTE_IO_BUFFER_BYTES), false, Vec::new(), Some(read_packet), None, Some(seek))
    }
}

/// read only io context over a remote http or https file, ranged gets on the callers thread
/// the linux clip player opens a bare https item through it, first chunk is fetched before return
pub(crate) fn remote_io_context(url: &str) -> Result<rsmpeg::avformat::AVIOContextCustom, VideoExportError> {
    Ok(RemoteFile::open(url)?.into_io_context())
}

pub(crate) fn remote_poster(url: &str, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
    use rsmpeg::avformat::AVIOContextContainer;

    let file = RemoteFile::open(url)?;
    let byte_count = file.length.and_then(|length| u64::try_from(length).ok()).unwrap_or(0);
    let input = AVFormatContextInput::from_io_context(AVIOContextContainer::Custom(file.into_io_context()))
        .map_err(|error| VideoExportError::RemoteUnreadable(error.to_string()))?;
    crate::decode_guard::decode_guard::check_ffmpeg_input(&input)
        .map_err(|refusal| VideoExportError::RemoteUnreadable(refusal.to_string()))?;
    let probe = probe_input(&input, byte_count);
    if !probe.has_video {
        return Err(VideoExportError::PosterFailed("the remote file has no video track".to_owned()));
    }
    let (filters, _) = poster_filters(&probe, max_side)?;
    let mut source = Source::from_input(input, ffi::AVMEDIA_TYPE_VIDEO, None, VideoExportError::RemoteUnreadable)?
        .ok_or_else(|| VideoExportError::PosterFailed("the remote file has no video track".to_owned()))?;
    source.seek(at_seconds);
    let frame = source
        .next()?
        .ok_or_else(|| VideoExportError::PosterFailed("the remote file decodes no frame".to_owned()))?;
    poster_picture(frame, source.time_base, &filters)
}

/// shared state of the running export
struct Control {
    progress: AtomicU32,
    cancelled: AtomicBool,
}

impl ExportControl for Control {
    fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn start_export(job: ExportJob, done: ExportSink, encoder: &'static str) -> Result<ExportTicket, VideoExportError> {
    let probes = job.segments.iter().map(|segment| probe(&segment.source)).collect::<Result<Vec<_>, _>>()?;
    let layer_seconds = job
        .audio
        .iter()
        .map(|layer| probe(&layer.source).map(|probe| probe.duration_seconds))
        .collect::<Result<Vec<_>, _>>()?;
    let plan = plan_graph(&job, &probes, &layer_seconds, encoder)?;
    let control = Arc::new(Control { progress: AtomicU32::new(0.0_f32.to_bits()), cancelled: AtomicBool::new(false) });
    let worker = Arc::clone(&control);
    std::thread::Builder::new()
        .name("rise-video-export".to_owned())
        .spawn(move || {
            let target = job.target.clone();
            let outcome = match run(&job, &probes, &plan, encoder, &worker) {
                Ok(Some(byte_count)) => ExportOutcome::Completed { path: target.clone(), byte_count },
                Ok(None) => ExportOutcome::Cancelled,
                Err(error) => ExportOutcome::Failed(error),
            };
            if !matches!(outcome, ExportOutcome::Completed { .. }) {
                let _ = std::fs::remove_file(&target);
            }
            done(outcome);
        })
        .map_err(|error| VideoExportError::ExportFailed(format!("the export thread did not start: {error}")))?;
    Ok(ExportTicket::new(control))
}

/// span of a stream thats used, for the decode stop
fn used_until(job: &ExportJob, probes: &[VideoProbe], stream: InputStream, total: f64) -> Option<f64> {
    let end = match stream {
        InputStream::SegmentVideo(index) | InputStream::SegmentAudio(index) => job.segments[index]
            .span
            .map(TimeSpan::end_seconds)
            .unwrap_or(probes[index].duration_seconds),
        InputStream::AudioLayer(index) => {
            job.audio[index].placement.map(|placement| placement.span.end_seconds()).unwrap_or(total)
        }
        InputStream::Overlay => return None,
    };
    end.is_finite().then_some(end + DECODE_TAIL_SECONDS)
}

fn start_of(job: &ExportJob, stream: InputStream) -> f64 {
    match stream {
        InputStream::SegmentVideo(index) | InputStream::SegmentAudio(index) => {
            job.segments[index].span.map_or(0.0, |span| span.start_seconds)
        }
        InputStream::AudioLayer(index) => job.audio[index].placement.map_or(0.0, |placement| placement.span.start_seconds),
        InputStream::Overlay => 0.0,
    }
}

/// one graph input, a decoded stream or the overlay picture still to send
enum Feed {
    Decoded(Source),
    Picture(Option<AVFrame>),
}

fn overlay_frame(overlay: &Overlay) -> Result<AVFrame, VideoExportError> {
    let mut frame = AVFrame::new();
    frame.set_format(ffi::AV_PIX_FMT_RGBA);
    frame.set_width(i32::try_from(overlay.width).unwrap_or(0));
    frame.set_height(i32::try_from(overlay.height).unwrap_or(0));
    frame.alloc_buffer().map_err(|error| VideoExportError::CompositionFailed(format!("av_frame_get_buffer: {error}")))?;
    let row = overlay.width as usize * 4;
    let stride = usize::try_from(frame.linesize[0]).unwrap_or(0);
    let data = frame.data[0];
    if data.is_null() || stride < row {
        return Err(VideoExportError::CompositionFailed("the overlay frame has no buffer".to_owned()));
    }
    for (line, pixels) in overlay.rgba.chunks_exact(row).enumerate() {
        unsafe { ptr::copy_nonoverlapping(pixels.as_ptr(), data.add(line * stride), row) };
    }
    frame.set_pts(0);
    Ok(frame)
}

struct Output {
    context: AVFormatContextOutput,
    file_open: bool,
}

impl Output {
    fn create(target: &Path) -> Result<Self, VideoExportError> {
        let filename = path_text(target)
            .ok_or_else(|| VideoExportError::TargetFailed("the path is not UTF-8 or holds a NUL byte".to_owned()))?;
        let muxer = CString::new(MP4_MUXER).unwrap_or_default();
        let mut raw = ptr::null_mut();
        let status =
            unsafe { ffi::avformat_alloc_output_context2(&mut raw, ptr::null(), muxer.as_ptr(), filename.as_ptr()) };
        let raw = NonNull::new(raw)
            .filter(|_| status >= 0)
            .ok_or_else(|| VideoExportError::Mp4Unsupported)?;
        let context = unsafe { AVFormatContextOutput::from_raw(raw) };
        Ok(Self { context, file_open: false })
    }

    fn open_file(&mut self, target: &Path) -> Result<(), VideoExportError> {
        let filename = path_text(target)
            .ok_or_else(|| VideoExportError::TargetFailed("the path is not UTF-8 or holds a NUL byte".to_owned()))?;
        let mut file = ptr::null_mut();
        let status = unsafe { ffi::avio_open(&mut file, filename.as_ptr(), ffi::AVIO_FLAG_WRITE as c_int) };
        if status < 0 || file.is_null() {
            return Err(VideoExportError::TargetFailed(format!("avio_open {}: {}", target.display(), av_text(status))));
        }
        unsafe { (*self.context.as_mut_ptr()).pb = file };
        self.file_open = true;
        Ok(())
    }

    fn written(&mut self) -> u64 {
        let position = unsafe { ffi::avio_seek((*self.context.as_mut_ptr()).pb, 0, SEEK_CUR) };
        u64::try_from(position).unwrap_or(0)
    }

    fn close_file(&mut self) -> Result<(), VideoExportError> {
        if !self.file_open {
            return Ok(());
        }
        self.file_open = false;
        let status = unsafe { ffi::avio_closep(&mut (*self.context.as_mut_ptr()).pb) };
        if status < 0 {
            return Err(VideoExportError::TargetFailed(format!("avio_closep: {}", av_text(status))));
        }
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Err(error) = self.close_file() {
            tracing::debug!("video export dropped with a failing file: {error}");
        }
    }
}

struct Encoder {
    context: AVCodecContext,
    stream: usize,
    stream_time_base: ffi::AVRational,
    finished: bool,
}

fn encoder_failed(call: &str, error: RsmpegError) -> VideoExportError {
    VideoExportError::ExportFailed(format!("{call}: {error}"))
}

fn open_video_encoder(
    name: &str,
    sink: NonNull<ffi::AVFilterContext>,
    plan: &GraphPlan,
    preset: ExportPreset,
    fallback_rate: u32,
    global_header: bool,
) -> Result<AVCodecContext, VideoExportError> {
    let codec_name = CString::new(name).unwrap_or_default();
    let codec = AVCodec::find_encoder_by_name(&codec_name)
        .ok_or_else(|| VideoExportError::EncoderUnavailable(format!("libavcodec has no {name} encoder")))?;
    let mut context = AVCodecContext::new(&codec);
    let (width, height, format, time_base, rate, frames) = unsafe {
        (
            ffi::av_buffersink_get_w(sink.as_ptr()),
            ffi::av_buffersink_get_h(sink.as_ptr()),
            ffi::av_buffersink_get_format(sink.as_ptr()),
            ffi::av_buffersink_get_time_base(sink.as_ptr()),
            ffi::av_buffersink_get_frame_rate(sink.as_ptr()),
            ffi::av_buffersink_get_hw_frames_ctx(sink.as_ptr()),
        )
    };
    let rate_fps = if rate.num > 0 && rate.den > 0 {
        (f64::from(rate.num) / f64::from(rate.den)).round().max(1.0) as u32
    } else {
        plan.frame_rate.unwrap_or(fallback_rate)
    };
    context.set_width(width);
    context.set_height(height);
    context.set_pix_fmt(format);
    context.set_time_base(time_base);
    if rate.num > 0 && rate.den > 0 {
        context.set_framerate(rate);
    }
    context.set_sample_aspect_ratio(ffi::AVRational { num: 1, den: 1 });
    let size = (u32::try_from(width).unwrap_or(0), u32::try_from(height).unwrap_or(0));
    context.set_bit_rate(i64::try_from(video_bit_rate(preset, size, rate_fps)).unwrap_or(i64::MAX));
    context.set_gop_size(i32::try_from(rate_fps.saturating_mul(2)).unwrap_or(60));
    if global_header {
        let flags = context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        context.set_flags(flags);
    }
    if !frames.is_null() {
        unsafe { (*context.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(frames) };
    }
    context.open(None).map_err(|error| encoder_failed("avcodec_open2 video", error))?;
    Ok(context)
}

fn open_audio_encoder(preset: ExportPreset, global_header: bool) -> Result<AVCodecContext, VideoExportError> {
    let codec_name = CString::new(AAC_ENCODER).unwrap_or_default();
    let codec = AVCodec::find_encoder_by_name(&codec_name)
        .ok_or_else(|| VideoExportError::EncoderUnavailable(format!("libavcodec has no {AAC_ENCODER} encoder")))?;
    let mut context = AVCodecContext::new(&codec);
    let rate = OUTPUT_SAMPLE_RATE as i32;
    context.set_sample_rate(rate);
    context.set_sample_fmt(ffi::AV_SAMPLE_FMT_FLTP);
    context.set_ch_layout(rsmpeg::avutil::AVChannelLayout::from_nb_channels(2).into_inner());
    context.set_bit_rate(i64::from(audio_bit_rate(preset)));
    context.set_time_base(ffi::AVRational { num: 1, den: rate });
    if global_header {
        let flags = context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        context.set_flags(flags);
    }
    context.open(None).map_err(|error| encoder_failed("avcodec_open2 audio", error))?;
    Ok(context)
}

/// runs the export, ok none if cancelled and ok some bytes when done
fn run(
    job: &ExportJob,
    probes: &[VideoProbe],
    plan: &GraphPlan,
    encoder_name: &'static str,
    control: &Control,
) -> Result<Option<u64>, VideoExportError> {
    let device = if encoder_name == VAAPI_ENCODER { Some(HwDevice::open()?) } else { None };
    let mut feeds = Vec::with_capacity(plan.inputs.len());
    let mut formats = Vec::with_capacity(plan.inputs.len());
    for input in &plan.inputs {
        let (path, media) = match input.stream {
            InputStream::SegmentVideo(index) => (job.segments[index].source.as_path(), ffi::AVMEDIA_TYPE_VIDEO),
            InputStream::SegmentAudio(index) => (job.segments[index].source.as_path(), ffi::AVMEDIA_TYPE_AUDIO),
            InputStream::AudioLayer(index) => (job.audio[index].source.as_path(), ffi::AVMEDIA_TYPE_AUDIO),
            InputStream::Overlay => {
                let overlay = job
                    .render
                    .as_ref()
                    .and_then(|render| render.overlay.as_ref())
                    .ok_or_else(|| VideoExportError::InvalidJob("the graph wants an overlay the job lacks"))?;
                let frame = overlay_frame(overlay)?;
                formats.push((input.clone(), SourceFormat::of_video(&frame, ffi::AVRational { num: 1, den: 1 })));
                feeds.push(Feed::Picture(Some(frame)));
                continue;
            }
        };
        let missing = || match media {
            ffi::AVMEDIA_TYPE_VIDEO => VideoExportError::NoVideoTrack(path.to_path_buf()),
            _ => VideoExportError::SourceUnreadable(path.to_path_buf(), "no audio track".to_owned()),
        };
        let mut source = Source::open(path, media, used_until(job, probes, input.stream, plan.total_seconds))?
            .ok_or_else(|| missing())?;
        source.seek(start_of(job, input.stream));
        let time_base = source.time_base;
        let format = match source.peek()? {
            Some(frame) if media == ffi::AVMEDIA_TYPE_VIDEO => SourceFormat::of_video(frame, time_base),
            Some(frame) => SourceFormat::of_audio(frame, time_base),
            None => return Err(missing()),
        };
        formats.push((input.clone(), format));
        feeds.push(Feed::Decoded(source));
    }
    let mut graph = Graph::build(&plan.description, &formats, device.as_ref())?;
    let slots: Vec<usize> = graph
        .sources
        .iter()
        .map(|(stream, _, _)| plan.inputs.iter().position(|input| input.stream == *stream).unwrap_or(0))
        .collect();

    let mut output = Output::create(&job.target)?;
    let global_header = output.context.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32 != 0;
    let fallback_rate = render_frame_rate(probes[0].nominal_frame_rate);
    let video_context =
        open_video_encoder(encoder_name, graph.video_sink, plan, job.preset, fallback_rate, global_header)?;
    let audio_context = match graph.audio_sink {
        Some(sink) => {
            let context = open_audio_encoder(job.preset, global_header)?;
            let frame_size = u32::try_from(context.frame_size).unwrap_or(1024).max(1);
            unsafe { ffi::av_buffersink_set_frame_size(sink.as_ptr(), frame_size) };
            Some(context)
        }
        None => None,
    };
    {
        let mut stream = output.context.new_stream();
        stream.set_codecpar(video_context.extract_codecpar());
        stream.set_time_base(video_context.time_base);
    }
    if let Some(context) = &audio_context {
        let mut stream = output.context.new_stream();
        stream.set_codecpar(context.extract_codecpar());
        stream.set_time_base(context.time_base);
    }
    output.open_file(&job.target)?;
    let mut options = Some(AVDictionary::new(c"movflags", c"+faststart", 0));
    output
        .context
        .write_header(&mut options)
        .map_err(|error| VideoExportError::TargetFailed(format!("avformat_write_header: {error}")))?;
    let mut video = Encoder {
        context: video_context,
        stream: 0,
        stream_time_base: output.context.streams()[0].time_base,
        finished: false,
    };
    let mut audio = match audio_context {
        Some(context) => Some(Encoder {
            context,
            stream: 1,
            stream_time_base: output.context.streams()[1].time_base,
            finished: false,
        }),
        None => None,
    };

    let video_base = unsafe { ffi::av_buffersink_get_time_base(graph.video_sink.as_ptr()) };
    let audio_base = graph
        .audio_sink
        .map(|sink| unsafe { ffi::av_buffersink_get_time_base(sink.as_ptr()) })
        .unwrap_or(ffi::AVRational { num: 1, den: OUTPUT_SAMPLE_RATE as i32 });
    let mut encoded_seconds = 0.0_f64;
    let mut ceiling = false;
    loop {
        if control.cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut moved = false;
        if !video.finished {
            loop {
                match pull(graph.video_sink)? {
                    Pulled::Frame(frame) => {
                        moved = true;
                        encode(&mut video, video_base, Some(frame), &mut output, &mut encoded_seconds)?;
                    }
                    Pulled::Again => break,
                    Pulled::End => {
                        encode(&mut video, video_base, None, &mut output, &mut encoded_seconds)?;
                        break;
                    }
                }
                if byte_ceiling_reached(output.written(), job.file_length_limit) {
                    ceiling = true;
                    break;
                }
            }
        }
        if let (Some(audio), Some(sink)) = (audio.as_mut(), graph.audio_sink) {
            while !audio.finished && !ceiling {
                match pull(sink)? {
                    Pulled::Frame(frame) => {
                        moved = true;
                        let mut ignored = 0.0;
                        encode(audio, audio_base, Some(frame), &mut output, &mut ignored)?;
                    }
                    Pulled::Again => break,
                    Pulled::End => {
                        let mut ignored = 0.0;
                        encode(audio, audio_base, None, &mut output, &mut ignored)?;
                    }
                }
                if byte_ceiling_reached(output.written(), job.file_length_limit) {
                    ceiling = true;
                }
            }
        }
        control.progress.store(progress(encoded_seconds, plan.total_seconds).to_bits(), Ordering::Relaxed);
        if ceiling {
            if !video.finished {
                encode(&mut video, video_base, None, &mut output, &mut encoded_seconds)?;
            }
            if let Some(audio) = audio.as_mut().filter(|audio| !audio.finished) {
                let mut ignored = 0.0;
                encode(audio, audio_base, None, &mut output, &mut ignored)?;
            }
            break;
        }
        if video.finished && audio.as_ref().is_none_or(|audio| audio.finished) {
            break;
        }
        match graph.hungriest() {
            Some(slot) => {
                let frame = match &mut feeds[slots[slot]] {
                    Feed::Decoded(source) => source.next()?,
                    Feed::Picture(picture) => picture.take(),
                };
                graph.feed(slot, frame)?;
            }
            None if !moved => {
                return Err(VideoExportError::CompositionFailed("the filter graph stalled with every source closed".to_owned()));
            }
            None => {}
        }
    }
    output
        .context
        .write_trailer()
        .map_err(|error| VideoExportError::TargetFailed(format!("av_write_trailer: {error}")))?;
    output.close_file()?;
    drop(output);
    let bytes = std::fs::metadata(&job.target)
        .map(|meta| meta.len())
        .map_err(|error| VideoExportError::TargetFailed(error.to_string()))?;
    Ok(Some(bytes))
}

/// sends one filtered frame (or the end) to the encoder and muxes every packet it gives
fn encode(
    encoder: &mut Encoder,
    sink_base: ffi::AVRational,
    frame: Option<AVFrame>,
    output: &mut Output,
    encoded_seconds: &mut f64,
) -> Result<(), VideoExportError> {
    if encoder.finished {
        return Ok(());
    }
    match frame {
        Some(mut frame) => {
            if sink_base.num > 0 && sink_base.den > 0 && frame.pts != ffi::AV_NOPTS_VALUE {
                let pts = unsafe { ffi::av_rescale_q(frame.pts, sink_base, encoder.context.time_base) };
                frame.set_pts(pts);
            }
            encoder.context.send_frame(Some(&frame)).map_err(|error| encoder_failed("avcodec_send_frame", error))?;
        }
        None => {
            encoder.finished = true;
            encoder.context.send_frame(None).map_err(|error| encoder_failed("avcodec_send_frame(flush)", error))?;
        }
    }
    loop {
        let mut packet: AVPacket = match encoder.context.receive_packet() {
            Ok(packet) => packet,
            Err(RsmpegError::EncoderDrainError | RsmpegError::EncoderFlushedError) => return Ok(()),
            Err(error) => return Err(encoder_failed("avcodec_receive_packet", error)),
        };
        let at = seconds(packet.pts, encoder.context.time_base);
        if at.is_finite() {
            *encoded_seconds = encoded_seconds.max(at);
        }
        packet.set_stream_index(encoder.stream as i32);
        packet.rescale_ts(encoder.context.time_base, encoder.stream_time_base);
        output
            .context
            .interleaved_write_frame(&mut packet)
            .map_err(|error| VideoExportError::TargetFailed(format!("av_interleaved_write_frame: {error}")))?;
    }
}

const STILL_INPUT: &str = "still";
const STILL_FRAME_RATE: i32 = 30;

pub(crate) fn start_still_video(
    job: StillVideoJob,
    done: ExportSink,
    encoder: &'static str,
) -> Result<ExportTicket, VideoExportError> {
    let control = Arc::new(Control { progress: AtomicU32::new(0.0_f32.to_bits()), cancelled: AtomicBool::new(false) });
    let worker = Arc::clone(&control);
    std::thread::Builder::new()
        .name("rise-still-video".to_owned())
        .spawn(move || {
            let _ = std::fs::remove_file(&job.target);
            let outcome = match run_still(&job, encoder, &worker) {
                Ok(Some(byte_count)) => ExportOutcome::Completed { path: job.target.clone(), byte_count },
                Ok(None) => ExportOutcome::Cancelled,
                Err(error) => ExportOutcome::Failed(error),
            };
            if !matches!(outcome, ExportOutcome::Completed { .. }) {
                let _ = std::fs::remove_file(&job.target);
            }
            done(outcome);
        })
        .map_err(|error| VideoExportError::ExportFailed(format!("the export thread did not start: {error}")))?;
    Ok(ExportTicket::new(control))
}

/// the still as one bgra frame at pts in the still time base
fn still_frame(picture: &PosterFrame, pts: i64) -> Result<AVFrame, VideoExportError> {
    let mut frame = AVFrame::new();
    frame.set_format(ffi::AV_PIX_FMT_BGRA);
    frame.set_width(i32::try_from(picture.width).unwrap_or(0));
    frame.set_height(i32::try_from(picture.height).unwrap_or(0));
    frame.alloc_buffer().map_err(|error| VideoExportError::CompositionFailed(format!("av_frame_get_buffer: {error}")))?;
    let row = picture.width as usize * 4;
    let stride = usize::try_from(frame.linesize[0]).unwrap_or(0);
    let data = frame.data[0];
    if data.is_null() || stride < row {
        return Err(VideoExportError::CompositionFailed("the still frame has no buffer".to_owned()));
    }
    for (line, pixels) in picture.bgra.chunks_exact(row).enumerate() {
        unsafe { ptr::copy_nonoverlapping(pixels.as_ptr(), data.add(line * stride), row) };
    }
    frame.set_pts(pts);
    Ok(frame)
}

fn open_still_encoder(
    name: &str,
    sink: NonNull<ffi::AVFilterContext>,
    average_bit_rate: u32,
    global_header: bool,
) -> Result<AVCodecContext, VideoExportError> {
    let codec_name = CString::new(name).unwrap_or_default();
    let codec = AVCodec::find_encoder_by_name(&codec_name)
        .ok_or_else(|| VideoExportError::EncoderUnavailable(format!("libavcodec has no {name} encoder")))?;
    let mut context = AVCodecContext::new(&codec);
    let (width, height, format, time_base, frames) = unsafe {
        (
            ffi::av_buffersink_get_w(sink.as_ptr()),
            ffi::av_buffersink_get_h(sink.as_ptr()),
            ffi::av_buffersink_get_format(sink.as_ptr()),
            ffi::av_buffersink_get_time_base(sink.as_ptr()),
            ffi::av_buffersink_get_hw_frames_ctx(sink.as_ptr()),
        )
    };
    context.set_width(width);
    context.set_height(height);
    context.set_pix_fmt(format);
    context.set_time_base(time_base);
    context.set_framerate(ffi::AVRational { num: STILL_FRAME_RATE, den: 1 });
    context.set_sample_aspect_ratio(ffi::AVRational { num: 1, den: 1 });
    context.set_bit_rate(i64::from(average_bit_rate));
    context.set_gop_size(STILL_FRAME_RATE * 2);
    unsafe { (*context.as_mut_ptr()).profile = ffi::AV_PROFILE_H264_HIGH as c_int };
    if global_header {
        let flags = context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        context.set_flags(flags);
    }
    if !frames.is_null() {
        unsafe { (*context.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(frames) };
    }
    context.open(None).map_err(|error| encoder_failed("avcodec_open2 video", error))?;
    Ok(context)
}

fn run_still(job: &StillVideoJob, encoder_name: &'static str, control: &Control) -> Result<Option<u64>, VideoExportError> {
    let ticks = job.ticks();
    let time_base = ffi::AVRational { num: 1, den: STILL_VIDEO_TIMESCALE };
    let device = if encoder_name == VAAPI_ENCODER { Some(HwDevice::open()?) } else { None };
    let first = still_frame(&job.frame, 0)?;
    let input = GraphInput { name: STILL_INPUT.to_owned(), stream: InputStream::Overlay };
    let format = SourceFormat::of_video(&first, time_base);
    let description = format!("[{STILL_INPUT}]{}[{VIDEO_SINK}]", encoder_tail(encoder_name));
    let mut graph = Graph::build(&description, &[(input, format)], device.as_ref())?;

    let mut output = Output::create(&job.target)?;
    let global_header = output.context.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32 != 0;
    let context = open_still_encoder(encoder_name, graph.video_sink, job.average_bit_rate, global_header)?;
    {
        let mut stream = output.context.new_stream();
        stream.set_codecpar(context.extract_codecpar());
        stream.set_time_base(context.time_base);
    }
    output.open_file(&job.target)?;
    output
        .context
        .write_header(&mut None)
        .map_err(|error| VideoExportError::TargetFailed(format!("avformat_write_header: {error}")))?;
    let mut video = Encoder {
        context,
        stream: 0,
        stream_time_base: output.context.streams()[0].time_base,
        finished: false,
    };
    let sink_base = unsafe { ffi::av_buffersink_get_time_base(graph.video_sink.as_ptr()) };

    let mut pictures = vec![Some(first)];
    if let Some(repeat) = ticks.repeat {
        pictures.push(Some(still_frame(&job.frame, repeat)?));
    }
    pictures.push(None);
    let mut encoded_ticks = 0_i64;
    for picture in pictures {
        if control.cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        graph.feed(0, picture)?;
        loop {
            match pull(graph.video_sink)? {
                Pulled::Frame(frame) => {
                    encode_still(&mut video, sink_base, Some(frame), ticks, &mut output, &mut encoded_ticks)?
                }
                Pulled::Again => break,
                Pulled::End => {
                    encode_still(&mut video, sink_base, None, ticks, &mut output, &mut encoded_ticks)?;
                    break;
                }
            }
        }
        let done = progress(encoded_ticks as f64, ticks.end as f64);
        control.progress.store(done.to_bits(), Ordering::Relaxed);
    }
    if control.cancelled.load(Ordering::Relaxed) {
        return Ok(None);
    }
    encode_still(&mut video, sink_base, None, ticks, &mut output, &mut encoded_ticks)?;
    output
        .context
        .write_trailer()
        .map_err(|error| VideoExportError::TargetFailed(format!("av_write_trailer: {error}")))?;
    output.close_file()?;
    drop(output);
    drop(graph);
    let bytes = std::fs::metadata(&job.target)
        .map(|meta| meta.len())
        .map_err(|error| VideoExportError::TargetFailed(error.to_string()))?;
    Ok(Some(bytes))
}

/// sends one still frame (or the end) and muxes packets, each lasts until the next copy or session end
/// so the track is exactly the job duration
fn encode_still(
    encoder: &mut Encoder,
    sink_base: ffi::AVRational,
    frame: Option<AVFrame>,
    ticks: StillVideoTicks,
    output: &mut Output,
    encoded_ticks: &mut i64,
) -> Result<(), VideoExportError> {
    if encoder.finished {
        return Ok(());
    }
    let encoder_base = encoder.context.time_base;
    match frame {
        Some(mut frame) => {
            if sink_base.num > 0 && sink_base.den > 0 && frame.pts != ffi::AV_NOPTS_VALUE {
                let pts = unsafe { ffi::av_rescale_q(frame.pts, sink_base, encoder_base) };
                frame.set_pts(pts);
            }
            encoder.context.send_frame(Some(&frame)).map_err(|error| encoder_failed("avcodec_send_frame", error))?;
        }
        None => {
            encoder.finished = true;
            encoder.context.send_frame(None).map_err(|error| encoder_failed("avcodec_send_frame(flush)", error))?;
        }
    }
    let still_base = ffi::AVRational { num: 1, den: STILL_VIDEO_TIMESCALE };
    loop {
        let mut packet: AVPacket = match encoder.context.receive_packet() {
            Ok(packet) => packet,
            Err(RsmpegError::EncoderDrainError | RsmpegError::EncoderFlushedError) => return Ok(()),
            Err(error) => return Err(encoder_failed("avcodec_receive_packet", error)),
        };
        if packet.pts != ffi::AV_NOPTS_VALUE {
            let at = unsafe { ffi::av_rescale_q(packet.pts, encoder_base, still_base) };
            let until = match ticks.repeat {
                Some(repeat) if at < repeat => repeat,
                _ => ticks.end,
            };
            let duration = unsafe { ffi::av_rescale_q((until - at).max(1), still_base, encoder_base) };
            packet.set_duration(duration);
            *encoded_ticks = (*encoded_ticks).max(until);
        }
        packet.set_stream_index(encoder.stream as i32);
        packet.rescale_ts(encoder_base, encoder.stream_time_base);
        output
            .context
            .interleaved_write_frame(&mut packet)
            .map_err(|error| VideoExportError::TargetFailed(format!("av_interleaved_write_frame: {error}")))?;
    }
}

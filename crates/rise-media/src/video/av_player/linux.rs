use super::av_player::*;

#[cfg(not(feature = "ffmpeg"))]
pub(super) fn player_available() -> bool {
    false
}

#[cfg(not(feature = "ffmpeg"))]
pub(super) fn open_engine(_item: &AVPlayerItem, _ends: EndSink) -> Result<Box<dyn PlayerEngine>, AVPlayerError> {
    let host = rise_platform::HostOs::current();
    Err(AVPlayerError::NotOnThisHost { host, reason: unwired_reason(host) })
}

#[cfg(feature = "ffmpeg")]
pub(super) fn player_available() -> bool {
    true
}

#[cfg(feature = "ffmpeg")]
pub(super) fn open_engine(item: &AVPlayerItem, ends: EndSink) -> Result<Box<dyn PlayerEngine>, AVPlayerError> {
    clip::open(item, ends)
}

#[cfg(not(feature = "ffmpeg"))]
pub(super) fn open_playback_engine() -> Result<Box<dyn PlaybackEngine>, AVPlayerError> {
    let host = rise_platform::HostOs::current();
    Err(AVPlayerError::NotOnThisHost { host, reason: unwired_reason(host) })
}

#[cfg(feature = "ffmpeg")]
pub(super) fn open_playback_engine() -> Result<Box<dyn PlaybackEngine>, AVPlayerError> {
    playback::open()
}

#[cfg(feature = "ffmpeg")]
mod clip {
    use std::any::Any;
    use std::ffi::{c_int, CString};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use gpui::{AnyElement, IntoElement as _, ObjectFit, OwnedSurfaceFrame, Styled as _};
    use parking_lot::Mutex;
    use rise_core::foundation::foundation_date::RiseClock;
    use rise_core::foundation::foundation_url::Url;
    use rsmpeg::avcodec::AVCodecContext;
    use rsmpeg::avformat::{AVFormatContextInput, AVIOContextContainer};
    use rsmpeg::avutil::{AVDictionary, AVFrame};
    use rsmpeg::error::RsmpegError;
    use rsmpeg::ffi;
    use rsmpeg::swscale::SwsContext;

    use crate::texture::frame::FrameFormat;
    use crate::texture::native_surface::ycbcr_to_rgb_columns;
    use crate::video::av_asset::ffmpeg_io::avio_context;
    use crate::video::av_asset::AVURLAsset;
    use crate::video::av_player::clip_pacing::*;
    use crate::video::av_player::media_engine_frames::Nv12Picture;
    use crate::video::av_player::pixel_buffer::Nv12PixelBuffer;
    use crate::video::av_player::*;
    use crate::video::decoder::HostI420Planes;
    use crate::video_export::ffmpeg_export::remote_io_context;

    const STREAM_ID_SPACE: u64 = 1 << 62;
    static NEXT_STREAM: AtomicU64 = AtomicU64::new(0);

    const SWS_BILINEAR: u32 = 2;
    const MICROSECONDS: ffi::AVRational = ffi::AVRational { num: 1, den: 1_000_000 };

    /// one command from the gpui half to the decoder thread
    enum Command {
        Play,
        Pause,
        Loop { source: Option<ClipSource> },
        Unloop,
        Stop,
    }

    /// what the decoder thread opens for one item
    /// the url alone or the url with the asset whose loader serves the bytes
    #[derive(Clone)]
    pub(super) struct ClipSource {
        pub(super) url: Url,
        asset: Option<AVURLAsset>,
        pub(super) preferred_forward_buffer_duration: Duration,
    }

    impl ClipSource {
        pub(super) fn of(item: &AVPlayerItem) -> Result<Self, AVPlayerError> {
            SystemUrl::of(item.url())?;
            Ok(Self {
                url: item.url().clone(),
                asset: item.asset().cloned(),
                preferred_forward_buffer_duration: item.preferred_forward_buffer_duration(),
            })
        }
    }

    /// picture the layer draws, nv12 planes and the conversion of their stream
    #[derive(Clone)]
    pub(super) struct Shown {
        pub(super) picture: Nv12Picture,
        ycbcr_to_rgb: [[f32; 4]; 4],
    }

    impl Shown {
        /// copy of this picture turned clockwise by quarter turns
        pub(super) fn pixel_buffer(&self, clockwise_quarter_turns: u8) -> AVPlayerPixelBuffer {
            AVPlayerPixelBuffer::new(Box::new(Nv12PixelBuffer::new(self.picture.clone(), self.ycbcr_to_rgb, clockwise_quarter_turns)))
        }
    }

    struct LinPlayer {
        commands: Sender<Command>,
        shown: Arc<Mutex<Option<Shown>>>,
        url: Url,
        stream_id: u64,
    }

    struct LoopHolder {
        commands: Sender<Command>,
    }

    impl Drop for LoopHolder {
        fn drop(&mut self) {
            let _ = self.commands.send(Command::Unloop);
        }
    }

    /// stream id of its own for the pictures of one player
    pub(super) fn next_stream_id() -> u64 {
        STREAM_ID_SPACE | NEXT_STREAM.fetch_add(1, Ordering::Relaxed)
    }

    /// element that draws shown into the caller bounds with fit
    pub(super) fn shown_element(stream_id: u64, shown: Shown, fit: ObjectFit) -> Option<AnyElement> {
        let picture = shown.picture;
        let owned = OwnedSurfaceFrame::nv12(
            stream_id,
            picture.generation,
            picture.width,
            picture.height,
            picture.luma,
            picture.luma_stride,
            picture.chroma,
            picture.chroma_stride,
        )?
        .with_ycbcr_to_rgb(shown.ycbcr_to_rgb);
        Some(gpui::surface(owned).object_fit(fit).size_full().into_any_element())
    }

    pub(super) fn open(item: &AVPlayerItem, ends: EndSink) -> Result<Box<dyn PlayerEngine>, AVPlayerError> {
        let source = ClipSource::of(item)?;
        let (commands, receiver) = mpsc::channel();
        let shown = Arc::new(Mutex::new(None));
        let stream_id = next_stream_id();
        let url = source.url.clone();
        let worker = ClipThread { source, ends, receiver, shown: Arc::clone(&shown) };
        thread::Builder::new()
            .name("riseonly-clip-player".to_owned())
            .spawn(move || worker.run())
            .map_err(|_| AVPlayerError::Platform { call: "spawn the clip decoder thread" })?;
        Ok(Box::new(LinPlayer { commands, shown, url, stream_id }))
    }

    impl Drop for LinPlayer {
        fn drop(&mut self) {
            let _ = self.commands.send(Command::Stop);
        }
    }

    impl LinPlayer {
        fn send(&self, command: Command) {
            if self.commands.send(command).is_err() {
                tracing::debug!(target: "riseonly", "clip decoder thread already left");
            }
        }
    }

    impl PlayerEngine for LinPlayer {
        fn set_is_muted(&mut self, muted: bool) {
            tracing::trace!(target: "riseonly", muted, "clip mute");
        }

        fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) {
            tracing::trace!(target: "riseonly", ?action, "clip action at item end");
        }

        fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) {
            tracing::trace!(target: "riseonly", waits, "clip stall policy");
        }

        fn play(&mut self) {
            self.send(Command::Play);
        }

        fn pause(&mut self) {
            self.send(Command::Pause);
        }

        fn start_looping(&mut self, template: &AVPlayerItem) -> Result<Box<dyn Any>, AVPlayerError> {
            let template_source = ClipSource::of(template)?;
            let replaced = template_source.url != self.url;
            self.url = template_source.url.clone();
            let source = replaced.then_some(template_source);
            self.commands
                .send(Command::Loop { source })
                .map_err(|_| AVPlayerError::Platform { call: "start the clip loop" })?;
            Ok(Box::new(LoopHolder { commands: self.commands.clone() }))
        }

        fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
            let shown = self.shown.lock().clone()?;
            shown_element(self.stream_id, shown, fit)
        }
    }

    /// decoded picture waiting for its presentation time
    struct Decoded {
        pts_us: i64,
        shown: Shown,
    }

    struct ClipThread {
        source: ClipSource,
        ends: EndSink,
        receiver: Receiver<Command>,
        shown: Arc<Mutex<Option<Shown>>>,
    }

    /// state of one decoder thread between commands
    struct Playback {
        decoding: Option<Decoding>,
        pending: Option<Decoded>,
        clock: ClipClock,
        playing: bool,
        looping: bool,
        ended: bool,
        has_picture: bool,
        generation: u64,
    }

    impl Playback {
        fn open(source: &ClipSource) -> Self {
            let decoding = Decoding::open(source)
                .inspect_err(|reason| tracing::warn!(target: "riseonly", %reason, "clip player could not open the item"))
                .ok();
            Self {
                decoding,
                pending: None,
                clock: ClipClock::default(),
                playing: false,
                looping: false,
                ended: false,
                has_picture: false,
                generation: 0,
            }
        }

        /// how long the thread may sleep before its next step, none sleeps until a command
        fn wait(&mut self) -> Option<Duration> {
            if self.decoding.is_none() || self.ended {
                return None;
            }
            let still = !self.playing && self.has_picture;
            match &self.pending {
                _ if still => None,
                None => Some(Duration::ZERO),
                Some(_) if !self.playing => Some(Duration::ZERO),
                Some(decoded) => match self.clock.due(decoded.pts_us, RiseClock::monotonic()) {
                    PictureDue::Now => Some(Duration::ZERO),
                    PictureDue::After(wait) => Some(wait),
                },
            }
        }

        /// decodes the next picture or shows the pending one now that it is due
        fn step(&mut self, source: &ClipSource, ends: &EndSink, shown: &Mutex<Option<Shown>>) {
            if let Some(decoded) = self.pending.take() {
                self.generation = self.generation.wrapping_add(1);
                let mut next = decoded.shown;
                next.picture.generation = self.generation;
                *shown.lock() = Some(next);
                self.has_picture = true;
                return;
            }
            let Some(decoding) = &mut self.decoding else {
                return;
            };
            match decoding.next_picture() {
                Ok(Some(decoded)) => self.pending = Some(decoded),
                Ok(None) => {
                    let end = pass_end(self.looping);
                    if end.posts_end {
                        ends.deliver(source.url.absolute_string());
                    }
                    if !end.restarts {
                        self.ended = true;
                        return;
                    }
                    if let Err(reason) = decoding.rewind() {
                        tracing::warn!(target: "riseonly", %reason, "clip loop could not seek to the start");
                        self.ended = true;
                        return;
                    }
                    self.clock.restart();
                }
                Err(reason) => {
                    tracing::warn!(target: "riseonly", %reason, "clip decoding stopped");
                    self.ended = true;
                }
            }
        }

        fn start_loop(&mut self) {
            self.looping = true;
            if !self.ended {
                return;
            }
            let Some(decoding) = &mut self.decoding else {
                return;
            };
            match decoding.rewind() {
                Ok(()) => {
                    self.ended = false;
                    self.clock.restart();
                }
                Err(reason) => tracing::warn!(target: "riseonly", %reason, "clip loop could not seek to the start"),
            }
        }
    }

    impl ClipThread {
        fn run(self) {
            let mut source = self.source;
            let mut playback = Playback::open(&source);
            loop {
                let command = match playback.wait() {
                    None => match self.receiver.recv() {
                        Ok(command) => Some(command),
                        Err(_) => break,
                    },
                    Some(wait) => match self.receiver.recv_timeout(wait) {
                        Ok(command) => Some(command),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    },
                };
                match command {
                    Some(Command::Stop) => break,
                    Some(Command::Play) => {
                        if !playback.playing {
                            playback.playing = true;
                            playback.clock.resume(RiseClock::monotonic());
                        }
                    }
                    Some(Command::Pause) => {
                        playback.playing = false;
                        playback.clock.pause(RiseClock::monotonic());
                    }
                    Some(Command::Loop { source: replacement }) => {
                        if let Some(replacement) = replacement {
                            let playing = playback.playing;
                            source = replacement;
                            playback = Playback::open(&source);
                            playback.playing = playing;
                        }
                        playback.start_loop();
                    }
                    Some(Command::Unloop) => playback.looping = false,
                    None => playback.step(&source, &self.ends, &self.shown),
                }
            }
        }
    }

    /// input and software decoder of the best video stream of one source
    struct Decoding {
        input: AVFormatContextInput,
        stream: usize,
        decoder: AVCodecContext,
        time_base: ffi::AVRational,
        start: i64,
        flushed: bool,
        last_pts_us: i64,
        scaler: Option<SwsContext>,
    }

    /// opens the item through the asset loader or as a local file or through ranged requests
    pub(super) fn open_input(source: &ClipSource) -> Result<AVFormatContextInput, String> {
        if let Some(asset) = &source.asset {
            return AVFormatContextInput::from_io_context(AVIOContextContainer::Custom(avio_context(asset)))
                .map_err(|error| format!("avformat_open_input over the asset loader: {error}"));
        }
        match SystemUrl::of(&source.url).map_err(|error| error.to_string())? {
            SystemUrl::File(path) => {
                let path = CString::new(path).map_err(|_| "the file path holds a nul".to_owned())?;
                AVFormatContextInput::open(&path, None, &mut None).map_err(|error| format!("avformat_open_input: {error}"))
            }
            SystemUrl::Remote(url) => {
                let io = remote_io_context(&url).map_err(|error| error.to_string())?;
                AVFormatContextInput::from_io_context(AVIOContextContainer::Custom(io))
                    .map_err(|error| format!("avformat_open_input over HTTP: {error}"))
            }
        }
    }

    impl Decoding {
        fn open(source: &ClipSource) -> Result<Self, String> {
            let input = open_input(source)?;
            crate::decode_guard::decode_guard::check_ffmpeg_input(&input).map_err(|refusal| refusal.to_string())?;
            let (stream, codec) = input
                .find_best_stream(ffi::AVMEDIA_TYPE_VIDEO)
                .map_err(|error| format!("av_find_best_stream: {error}"))?
                .ok_or_else(|| "the item has no video stream".to_owned())?;
            let (time_base, start, mut decoder) = {
                let av_stream = &input.streams()[stream];
                let mut decoder = AVCodecContext::new(&codec);
                decoder
                    .apply_codecpar(&av_stream.codecpar())
                    .map_err(|error| format!("avcodec_parameters_to_context: {error}"))?;
                decoder.set_pkt_timebase(av_stream.time_base);
                crate::decode_guard::decode_guard::limit_codec_pixels(&mut decoder);
                let start = if av_stream.start_time == ffi::AV_NOPTS_VALUE { 0 } else { av_stream.start_time };
                (av_stream.time_base, start, decoder)
            };
            decoder
                .open(Some(AVDictionary::new(c"threads", c"auto", 0)))
                .map_err(|error| format!("avcodec_open2: {error}"))?;
            Ok(Self { input, stream, decoder, time_base, start, flushed: false, last_pts_us: 0, scaler: None })
        }

        fn next_picture(&mut self) -> Result<Option<Decoded>, String> {
            loop {
                match self.decoder.receive_frame() {
                    Ok(frame) => return self.decoded(&frame).map(Some),
                    Err(RsmpegError::DecoderDrainError) if !self.flushed => {}
                    Err(RsmpegError::DecoderDrainError | RsmpegError::DecoderFlushedError) => return Ok(None),
                    Err(error) => return Err(format!("avcodec_receive_frame: {error}")),
                }
                match self.input.read_packet() {
                    Ok(Some(packet)) => {
                        if usize::try_from(packet.stream_index).ok() != Some(self.stream) {
                            continue;
                        }
                        match self.decoder.send_packet(Some(&packet)) {
                            Ok(()) | Err(RsmpegError::DecoderFullError) => {}
                            Err(RsmpegError::SendPacketError(status)) if status == ffi::AVERROR_INVALIDDATA => {}
                            Err(error) => return Err(format!("avcodec_send_packet: {error}")),
                        }
                    }
                    Ok(None) => {
                        self.flushed = true;
                        let _ = self.decoder.send_packet(None);
                    }
                    Err(error) => return Err(format!("av_read_frame: {error}")),
                }
            }
        }

        /// back to the first picture for the next pass, seek then reset the drained decoder
        fn rewind(&mut self) -> Result<(), String> {
            let stream = c_int::try_from(self.stream).map_err(|_| "the stream index is out of range".to_owned())?;
            let status = unsafe {
                ffi::av_seek_frame(self.input.as_mut_ptr(), stream, self.start, ffi::AVSEEK_FLAG_BACKWARD as c_int)
            };
            if status < 0 {
                return Err(format!("av_seek_frame answered {status}"));
            }
            unsafe { ffi::avcodec_flush_buffers(self.decoder.as_mut_ptr()) };
            self.flushed = false;
            Ok(())
        }

        /// presentation time and drawable planes of one decoded picture
        fn decoded(&mut self, frame: &AVFrame) -> Result<Decoded, String> {
            let stamp = if frame.best_effort_timestamp != ffi::AV_NOPTS_VALUE { frame.best_effort_timestamp } else { frame.pts };
            let pts_us = if stamp == ffi::AV_NOPTS_VALUE {
                self.last_pts_us
            } else {
                unsafe { ffi::av_rescale_q(stamp.saturating_sub(self.start), self.time_base, MICROSECONDS) }
            };
            self.last_pts_us = pts_us;
            let shown = video_picture(frame, &mut self.scaler)?;
            Ok(Decoded { pts_us, shown })
        }
    }

    /// drawable planes and colour conversion of one decoded picture
    /// scaler keeps the libswscale context of a layout that needs one between pictures
    pub(super) fn video_picture(frame: &AVFrame, scaler: &mut Option<SwsContext>) -> Result<Shown, String> {
        let height = u32::try_from(frame.height).unwrap_or(0);
        let full_range_format = frame.format == ffi::AV_PIX_FMT_YUVJ420P;
        let color = picture_color(frame.colorspace as i32, frame.color_range as i32, full_range_format, height);
        let picture = if frame.format == ffi::AV_PIX_FMT_YUV420P || full_range_format {
            i420_picture(frame)
        } else if frame.format == ffi::AV_PIX_FMT_NV12 {
            semi_planar_picture(frame)
        } else {
            scaled_picture(frame, scaler)?
        }
        .ok_or_else(|| format!("a decoded picture of format {} has no usable planes", frame.format))?;
        let ycbcr_to_rgb = ycbcr_to_rgb_columns(&color.params(FrameFormat::Nv12));
        Ok(Shown { picture, ycbcr_to_rgb })
    }

    fn scaled_picture(frame: &AVFrame, cached: &mut Option<SwsContext>) -> Result<Option<Nv12Picture>, String> {
        let scaler = match cached.take() {
            Some(scaler) => scaler.get_cached_context(
                frame.width,
                frame.height,
                frame.format,
                frame.width,
                frame.height,
                ffi::AV_PIX_FMT_NV12,
                SWS_BILINEAR,
                None,
                None,
                None,
            ),
            None => SwsContext::get_context(
                frame.width,
                frame.height,
                frame.format,
                frame.width,
                frame.height,
                ffi::AV_PIX_FMT_NV12,
                SWS_BILINEAR,
                None,
                None,
                None,
            ),
        };
        let mut scaler = scaler.ok_or_else(|| format!("libswscale cannot convert format {} to NV12", frame.format))?;
        let mut nv12 = AVFrame::new();
        nv12.set_format(ffi::AV_PIX_FMT_NV12);
        nv12.set_width(frame.width);
        nv12.set_height(frame.height);
        nv12.alloc_buffer().map_err(|error| format!("av_frame_get_buffer: {error}"))?;
        scaler
            .scale_frame(frame, 0, frame.height, &mut nv12)
            .map_err(|error| format!("sws_scale: {error}"))?;
        *cached = Some(scaler);
        Ok(semi_planar_picture(&nv12))
    }

    /// one plane of a software frame, rows rows of at least row bytes and their stride
    fn plane(frame: &AVFrame, index: usize, row: u32, rows: u32) -> Option<(&[u8], u32)> {
        let stride = u32::try_from(frame.linesize[index]).ok().filter(|stride| *stride >= row)?;
        let pointer = frame.data[index];
        if pointer.is_null() || rows == 0 {
            return None;
        }
        let len = usize::try_from(u64::from(stride) * u64::from(rows - 1) + u64::from(row)).ok()?;
        Some((unsafe { std::slice::from_raw_parts(pointer, len) }, stride))
    }

    fn i420_picture(frame: &AVFrame) -> Option<Nv12Picture> {
        let width = u32::try_from(frame.width).ok().filter(|width| *width > 0)?;
        let height = u32::try_from(frame.height).ok().filter(|height| *height > 0)?;
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let (y, stride_y) = plane(frame, 0, width, height)?;
        let (u, stride_u) = plane(frame, 1, chroma_width, chroma_height)?;
        let (v, stride_v) = plane(frame, 2, chroma_width, chroma_height)?;
        nv12_from_i420(&HostI420Planes { width, height, y, u, v, stride_y, stride_u, stride_v }, 0)
    }

    fn semi_planar_picture(frame: &AVFrame) -> Option<Nv12Picture> {
        let width = u32::try_from(frame.width).ok().filter(|width| *width > 0)?;
        let height = u32::try_from(frame.height).ok().filter(|height| *height > 0)?;
        let (y, stride_y) = plane(frame, 0, width, height)?;
        let (uv, stride_uv) = plane(frame, 1, width.div_ceil(2) * 2, height.div_ceil(2))?;
        nv12_from_semi_planar(width, height, y, stride_y, uv, stride_uv, 0)
    }
}

#[cfg(feature = "ffmpeg")]
mod playback {
    use std::collections::VecDeque;
    use std::ffi::{c_char, CStr, CString};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use cpal::traits::HostTrait as _;
    use gpui::{AnyElement, ObjectFit, Size};
    use parking_lot::Mutex;
    use rise_core::foundation::foundation_date::RiseClock;
    use rsmpeg::avcodec::{AVCodecContext, AVPacket};
    use rsmpeg::avfilter::{AVFilter, AVFilterGraph, AVFilterInOut};
    use rsmpeg::avformat::AVFormatContextInput;
    use rsmpeg::avutil::{AVDictionary, AVFrame};
    use rsmpeg::error::RsmpegError;
    use rsmpeg::ffi;
    use rsmpeg::swscale::SwsContext;

    use super::clip::{next_stream_id, open_input, shown_element, video_picture, ClipSource, Shown};
    use crate::audio::cpal_sink::{supported_configs, CpalSink};
    use crate::audio::format::StreamFormat;
    use crate::audio::output::{choose, ring_samples, AudioSink as _, DeviceConfig};
    use crate::audio::ring::{self, Producer};
    use crate::video::av_player::media_clock::*;
    use crate::video::av_player::pixel_buffer::clockwise_quarter_turns_of_display_rotation;
    use crate::video::av_player::*;

    const MICROSECONDS: ffi::AVRational = ffi::AVRational { num: 1, den: 1_000_000 };
    const GRAPH_SOURCE: &CStr = c"rise_audio_in";
    const GRAPH_SINK: &CStr = c"rise_audio_out";
    const LAYOUT_TEXT_LEN: usize = 64;

    /// one command from the gpui half to the player thread
    enum Command {
        Replace { item: u64, source: Option<ClipSource>, ends: ItemEndSink },
        Play { tempo: f64 },
        Pause,
        Seek { item: u64, window: SeekWindow, done: SeekSink },
        Settings(PlayerSettings),
        Stop,
    }

    /// player settings every item of the thread opens with
    #[derive(Clone, Copy)]
    struct PlayerSettings {
        muted: bool,
        volume: f32,
        action_at_item_end: AVPlayerActionAtItemEnd,
        waits_to_minimize_stalling: bool,
    }

    impl Default for PlayerSettings {
        fn default() -> Self {
            Self {
                muted: false,
                volume: 1.0,
                action_at_item_end: AVPlayerActionAtItemEnd::Pause,
                waits_to_minimize_stalling: true,
            }
        }
    }

    /// what the player thread last read for the item numbered item
    #[derive(Clone, Copy)]
    struct PlaybackTimes {
        item: u64,
        current: Option<f64>,
        duration: Option<f64>,
        status: AVPlayerItemStatus,
        control: AVPlayerTimeControlStatus,
        loaded: Option<CMTimeRange>,
        presentation: Option<Size<f32>>,
    }

    impl PlaybackTimes {
        /// the item numbered item while it opens, or for good when status is failed
        fn opening(item: u64, status: AVPlayerItemStatus) -> Self {
            Self {
                item,
                current: None,
                duration: None,
                status,
                control: AVPlayerTimeControlStatus::Paused,
                loaded: None,
                presentation: None,
            }
        }
    }

    /// newest picture with the number of the item it belongs to
    /// the picture of the item numbered first, with the ffmpeg display rotation of its track
    type ItemPicture = Arc<Mutex<Option<(u64, Shown, f64)>>>;

    struct LinMediaPlayer {
        commands: Sender<Command>,
        picture: ItemPicture,
        times: Arc<Mutex<PlaybackTimes>>,
        stream_id: u64,
        item: u64,
        settings: PlayerSettings,
        copied: Option<(u64, u64)>,
    }

    /// linux player, one player thread that decodes the current item with sound
    pub(super) fn open() -> Result<Box<dyn PlaybackEngine>, AVPlayerError> {
        let (commands, receiver) = mpsc::channel();
        let picture: ItemPicture = Arc::new(Mutex::new(None));
        let times = Arc::new(Mutex::new(PlaybackTimes::opening(0, AVPlayerItemStatus::Unknown)));
        let worker = PlayerThread { receiver, picture: Arc::clone(&picture), times: Arc::clone(&times) };
        thread::Builder::new()
            .name("riseonly-media-player".to_owned())
            .spawn(move || worker.run())
            .map_err(|_| AVPlayerError::Platform { call: "spawn the media player thread" })?;
        Ok(Box::new(LinMediaPlayer {
            commands,
            picture,
            times,
            stream_id: next_stream_id(),
            item: 0,
            settings: PlayerSettings::default(),
            copied: None,
        }))
    }

    impl Drop for LinMediaPlayer {
        fn drop(&mut self) {
            let _ = self.commands.send(Command::Stop);
        }
    }

    impl LinMediaPlayer {
        fn send(&self, command: Command) {
            if self.commands.send(command).is_err() {
                tracing::debug!(target: "riseonly", "media player thread already left");
            }
        }

        /// what the thread last read, when it is about the current item
        fn current_times(&self) -> Option<PlaybackTimes> {
            let times = *self.times.lock();
            (times.item == self.item).then_some(times)
        }

        fn apply(&mut self, settings: PlayerSettings) -> Result<(), AVPlayerError> {
            self.settings = settings;
            self.send(Command::Settings(settings));
            Ok(())
        }
    }

    impl PlaybackEngine for LinMediaPlayer {
        fn replace_current_item(&mut self, item: Option<&AVPlayerItem>, ends: ItemEndSink) -> Result<(), AVPlayerError> {
            self.item = self.item.wrapping_add(1);
            let (source, failure) = match item.map(ClipSource::of).transpose() {
                Ok(source) => (source, None),
                Err(error) => (None, Some(error)),
            };
            self.send(Command::Replace { item: self.item, source, ends });
            failure.map_or(Ok(()), Err)
        }

        fn play_immediately(&mut self, rate: f32) {
            self.set_rate(rate);
        }

        fn set_rate(&mut self, rate: f32) {
            match tempo_of_rate(rate) {
                Some(tempo) => self.send(Command::Play { tempo }),
                None => self.send(Command::Pause),
            }
        }

        fn pause(&mut self) {
            self.send(Command::Pause);
        }

        fn current_time(&self) -> Option<f64> {
            self.current_times().and_then(|times| times.current)
        }

        fn current_item_duration(&self) -> Option<f64> {
            self.current_times().and_then(|times| times.duration)
        }

        fn seek(&mut self, window: SeekWindow, done: SeekSink) {
            self.send(Command::Seek { item: self.item, window, done });
        }

        fn set_is_muted(&mut self, muted: bool) -> Result<(), AVPlayerError> {
            self.apply(PlayerSettings { muted, ..self.settings })
        }

        fn set_volume(&mut self, volume: f32) -> Result<(), AVPlayerError> {
            self.apply(PlayerSettings { volume, ..self.settings })
        }

        fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) -> Result<(), AVPlayerError> {
            self.apply(PlayerSettings { action_at_item_end: action, ..self.settings })
        }

        fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) -> Result<(), AVPlayerError> {
            self.apply(PlayerSettings { waits_to_minimize_stalling: waits, ..self.settings })
        }

        fn time_control_status(&self) -> Option<AVPlayerTimeControlStatus> {
            Some(self.current_times().map_or(AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate, |times| times.control))
        }

        fn item_status(&self) -> Option<AVPlayerItemStatus> {
            Some(self.current_times().map_or(AVPlayerItemStatus::Unknown, |times| times.status))
        }

        fn loaded_time_ranges(&self) -> Vec<CMTimeRange> {
            self.current_times().and_then(|times| times.loaded).into_iter().collect()
        }

        fn presentation_size(&self) -> Option<Size<f32>> {
            self.current_times().and_then(|times| times.presentation)
        }

        fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
            let shown = {
                let slot = self.picture.lock();
                match slot.as_ref() {
                    Some((item, shown, _)) if *item == self.item => shown.clone(),
                    _ => return None,
                }
            };
            shown_element(self.stream_id, shown, fit)
        }

        fn is_ready_for_display(&self) -> Option<bool> {
            Some(self.picture.lock().as_ref().is_some_and(|(item, _, _)| *item == self.item))
        }

        fn copy_pixel_buffer(&mut self) -> Option<AVPlayerPixelBuffer> {
            let slot = self.picture.lock();
            let (item, shown, rotation_degrees) = slot.as_ref()?;
            let key = (*item, shown.picture.generation);
            if *item != self.item || self.copied == Some(key) {
                return None;
            }
            let buffer = shown.pixel_buffer(clockwise_quarter_turns_of_display_rotation(*rotation_degrees));
            drop(slot);
            self.copied = Some(key);
            Some(buffer)
        }
    }

    struct PlayerThread {
        receiver: Receiver<Command>,
        picture: ItemPicture,
        times: Arc<Mutex<PlaybackTimes>>,
    }

    impl PlayerThread {
        fn run(self) {
            let mut current: Option<MediaItem> = None;
            let mut settings = PlayerSettings::default();
            loop {
                let wait = current.as_mut().and_then(MediaItem::wait);
                let command = match wait {
                    None => match self.receiver.recv() {
                        Ok(command) => Some(command),
                        Err(_) => break,
                    },
                    Some(wait) => match self.receiver.recv_timeout(wait) {
                        Ok(command) => Some(command),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    },
                };
                match command {
                    Some(Command::Stop) => break,
                    Some(Command::Replace { item, source, ends }) => {
                        drop(current.take());
                        *self.times.lock() = PlaybackTimes::opening(item, AVPlayerItemStatus::Unknown);
                        let had_source = source.is_some();
                        current = source.and_then(|source| {
                            MediaItem::open(item, &source, ends, settings)
                                .inspect_err(|reason| {
                                    tracing::warn!(target: "riseonly", %reason, "media player could not open the item")
                                })
                                .ok()
                        });
                        if had_source && current.is_none() {
                            *self.times.lock() = PlaybackTimes::opening(item, AVPlayerItemStatus::Failed);
                        }
                    }
                    Some(Command::Seek { item, window, done }) => match &mut current {
                        Some(media) if media.item == item => media.seek(window, done),
                        _ => done.deliver(false),
                    },
                    Some(Command::Settings(next)) => {
                        settings = next;
                        if let Some(media) = &mut current {
                            media.apply(next);
                        }
                    }
                    Some(Command::Play { tempo }) => {
                        if let Some(media) = &mut current {
                            media.play(tempo);
                        }
                    }
                    Some(Command::Pause) => {
                        if let Some(media) = &mut current {
                            media.pause();
                        }
                    }
                    None => {
                        if let Some(media) = &mut current {
                            media.step(&self.picture);
                        }
                    }
                }
                if let Some(media) = &mut current {
                    *self.times.lock() = media.times();
                }
            }
        }
    }

    /// which decoder a packet feeds
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Track {
        Video,
        Audio,
    }

    /// input and the packets read ahead for the decoder that didnt ask for them
    struct Demuxer {
        input: AVFormatContextInput,
        done: bool,
        video_stream: Option<usize>,
        audio_stream: Option<usize>,
        video_packets: VecDeque<AVPacket>,
        audio_packets: VecDeque<AVPacket>,
        start_us: i64,
        range_start_us: i64,
        buffered_end_us: Option<i64>,
    }

    const READ_AHEAD_PACKETS: usize = 32;

    impl Demuxer {
        fn packet_end_us(&self, packet: &AVPacket) -> Option<i64> {
            let index = usize::try_from(packet.stream_index).ok()?;
            let stream = self.input.streams().get(index)?;
            let stamp = if packet.pts != ffi::AV_NOPTS_VALUE { packet.pts } else { packet.dts };
            if stamp == ffi::AV_NOPTS_VALUE {
                return None;
            }
            let us = unsafe { ffi::av_rescale_q(stamp.saturating_add(packet.duration.max(0)), stream.time_base, MICROSECONDS) };
            Some(us.saturating_sub(self.start_us))
        }

        fn note_read(&mut self, packet: &AVPacket) {
            if let Some(end_us) = self.packet_end_us(packet) {
                self.buffered_end_us = Some(self.buffered_end_us.map_or(end_us, |end| end.max(end_us)));
            }
        }

        /// how far the packets read reach past play_head_us
        fn buffered_ahead(&self, play_head_us: i64) -> Duration {
            let ahead = self.buffered_end_us.map_or(0, |end| end.saturating_sub(play_head_us));
            Duration::from_micros(ahead.max(0).unsigned_abs())
        }

        /// whether more should be read to keep target past play_head_us
        fn wants_more(&self, play_head_us: i64, target: Duration) -> bool {
            !self.done && self.buffered_ahead(play_head_us) < target
        }

        /// one loaded range from where reading started to what it reached
        fn loaded(&self) -> Option<CMTimeRange> {
            let end_us = self.buffered_end_us?;
            let start_us = self.range_start_us.min(end_us);
            Some(CMTimeRange::new(start_us as f64 / 1_000_000.0, (end_us - start_us) as f64 / 1_000_000.0))
        }

        fn read_ahead(&mut self, play_head_us: i64, target: Duration) -> Result<(), String> {
            for _ in 0..READ_AHEAD_PACKETS {
                if !self.wants_more(play_head_us, target) {
                    return Ok(());
                }
                let Some(packet) = self.input.read_packet().map_err(|error| format!("av_read_frame: {error}"))? else {
                    self.done = true;
                    return Ok(());
                };
                self.note_read(&packet);
                let index = usize::try_from(packet.stream_index).ok();
                if index.is_some() && index == self.video_stream {
                    self.video_packets.push_back(packet);
                } else if index.is_some() && index == self.audio_stream {
                    self.audio_packets.push_back(packet);
                }
            }
            Ok(())
        }

        /// moves the input to the keyframe at or before window target_us, no later than its upper bound
        /// and forgets everything read before
        fn seek(&mut self, window: SeekWindow) -> Result<(), String> {
            let target = self.start_us.saturating_add(window.target_us);
            let latest = self.start_us.saturating_add(window.max_us);
            let status = unsafe { ffi::avformat_seek_file(self.input.as_mut_ptr(), -1, i64::MIN, target, latest, 0) };
            if status < 0 {
                return Err(format!("avformat_seek_file: {status}"));
            }
            self.done = false;
            self.video_packets.clear();
            self.audio_packets.clear();
            self.range_start_us = window.min_us;
            self.buffered_end_us = None;
            Ok(())
        }
        /// next packet of the track, none at the end of the input
        fn next(&mut self, track: Track) -> Result<Option<AVPacket>, String> {
            let queued = match track {
                Track::Video => self.video_packets.pop_front(),
                Track::Audio => self.audio_packets.pop_front(),
            };
            if queued.is_some() {
                return Ok(queued);
            }
            while !self.done {
                let Some(packet) = self.input.read_packet().map_err(|error| format!("av_read_frame: {error}"))? else {
                    self.done = true;
                    break;
                };
                self.note_read(&packet);
                let index = usize::try_from(packet.stream_index).ok();
                let of = if index.is_some() && index == self.video_stream {
                    Track::Video
                } else if index.is_some() && index == self.audio_stream {
                    Track::Audio
                } else {
                    continue;
                };
                if of == track {
                    return Ok(Some(packet));
                }
                match of {
                    Track::Video => self.video_packets.push_back(packet),
                    Track::Audio => self.audio_packets.push_back(packet),
                }
            }
            Ok(None)
        }
    }

    /// one software decoder of one stream, drained through the demuxer
    struct StreamDecoder {
        track: Track,
        decoder: AVCodecContext,
        time_base: ffi::AVRational,
        flushed: bool,
        finished: bool,
    }

    impl StreamDecoder {
        fn open(input: &AVFormatContextInput, track: Track) -> Result<Option<(usize, Self)>, String> {
            let media_type = match track {
                Track::Video => ffi::AVMEDIA_TYPE_VIDEO,
                Track::Audio => ffi::AVMEDIA_TYPE_AUDIO,
            };
            let Ok(Some((stream, codec))) = input.find_best_stream(media_type) else {
                return Ok(None);
            };
            let av_stream = &input.streams()[stream];
            let mut decoder = AVCodecContext::new(&codec);
            decoder
                .apply_codecpar(&av_stream.codecpar())
                .map_err(|error| format!("avcodec_parameters_to_context: {error}"))?;
            decoder.set_pkt_timebase(av_stream.time_base);
            crate::decode_guard::decode_guard::limit_codec_pixels(&mut decoder);
            let time_base = av_stream.time_base;
            decoder
                .open(Some(AVDictionary::new(c"threads", c"auto", 0)))
                .map_err(|error| format!("avcodec_open2: {error}"))?;
            Ok(Some((stream, Self { track, decoder, time_base, flushed: false, finished: false })))
        }

        /// next decoded frame, none at the end of the stream
        fn next_frame(&mut self, demuxer: &mut Demuxer) -> Result<Option<AVFrame>, String> {
            if self.finished {
                return Ok(None);
            }
            loop {
                match self.decoder.receive_frame() {
                    Ok(frame) => return Ok(Some(frame)),
                    Err(RsmpegError::DecoderDrainError) if !self.flushed => {}
                    Err(RsmpegError::DecoderDrainError | RsmpegError::DecoderFlushedError) => {
                        self.finished = true;
                        return Ok(None);
                    }
                    Err(error) => return Err(format!("avcodec_receive_frame: {error}")),
                }
                match demuxer.next(self.track)? {
                    Some(packet) => match self.decoder.send_packet(Some(&packet)) {
                        Ok(()) | Err(RsmpegError::DecoderFullError) => {}
                        Err(RsmpegError::SendPacketError(status)) if status == ffi::AVERROR_INVALIDDATA => {}
                        Err(error) => return Err(format!("avcodec_send_packet: {error}")),
                    },
                    None => {
                        self.flushed = true;
                        let _ = self.decoder.send_packet(None);
                    }
                }
            }
        }

        /// forgets every packet and frame of the old position after a seek
        fn flush(&mut self) {
            unsafe { ffi::avcodec_flush_buffers(self.decoder.as_mut_ptr()) };
            self.flushed = false;
            self.finished = false;
        }

        /// media time of a frame in microseconds from the start of the item
        fn media_us(&self, frame: &AVFrame, start_us: i64) -> Option<i64> {
            let stamp = if frame.best_effort_timestamp != ffi::AV_NOPTS_VALUE { frame.best_effort_timestamp } else { frame.pts };
            if stamp == ffi::AV_NOPTS_VALUE {
                return None;
            }
            let us = unsafe { ffi::av_rescale_q(stamp, self.time_base, MICROSECONDS) };
            Some(us.saturating_sub(start_us))
        }
    }

    /// decoded video of the item, its decoder and the libswscale context it may need
    struct VideoTrack {
        decoder: StreamDecoder,
        scaler: Option<SwsContext>,
        last_pts_us: i64,
    }

    /// format of the decoded audio as the abuffer source declares it
    struct AudioSourceFormat {
        sample_rate: u32,
        format_name: String,
        layout: String,
    }

    impl AudioSourceFormat {
        fn of(frame: &AVFrame) -> Self {
            let name = unsafe { ffi::av_get_sample_fmt_name(frame.format) };
            let format_name = if name.is_null() {
                "fltp".to_owned()
            } else {
                unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
            };
            Self {
                sample_rate: u32::try_from(frame.sample_rate).unwrap_or(0),
                format_name,
                layout: layout_text(&frame.ch_layout),
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
        let mut buffer = [0 as c_char; LAYOUT_TEXT_LEN];
        let status = unsafe { ffi::av_channel_layout_describe(source, buffer.as_mut_ptr(), buffer.len()) };
        if status < 0 {
            return format!("{channels}c");
        }
        unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned()
    }

    fn build_graph(
        source: &AudioSourceFormat,
        tempo: f64,
        has_atempo: bool,
        device: DeviceConfig,
    ) -> Result<(AVFilterGraph, u32), String> {
        let declared = declared_source_rate(source.sample_rate, tempo, has_atempo);
        let args = CString::new(format!(
            "time_base=1/{declared}:sample_rate={declared}:sample_fmt={}:channel_layout={}",
            source.format_name, source.layout
        ))
        .map_err(|_| "the abuffer arguments hold a nul".to_owned())?;
        let spec = CString::new(audio_filter_spec(tempo, has_atempo, device.sample_rate, device.channels))
            .map_err(|_| "the filter spec holds a nul".to_owned())?;
        let abuffer = AVFilter::get_by_name(c"abuffer").ok_or("the prefix has no abuffer filter")?;
        let abuffersink = AVFilter::get_by_name(c"abuffersink").ok_or("the prefix has no abuffersink filter")?;
        let graph = AVFilterGraph::new();
        {
            let mut source_context = graph
                .create_filter_context(&abuffer, GRAPH_SOURCE, Some(&args))
                .map_err(|error| format!("avfilter_graph_create_filter abuffer: {error}"))?;
            let mut sink_context = graph
                .create_filter_context(&abuffersink, GRAPH_SINK, None)
                .map_err(|error| format!("avfilter_graph_create_filter abuffersink: {error}"))?;
            let outputs = AVFilterInOut::new(c"in", &mut source_context, 0);
            let inputs = AVFilterInOut::new(c"out", &mut sink_context, 0);
            graph
                .parse_ptr(&spec, Some(inputs), Some(outputs))
                .map_err(|error| format!("avfilter_graph_parse_ptr: {error}"))?;
        }
        graph.config().map_err(|error| format!("avfilter_graph_config: {error}"))?;
        Ok((graph, declared))
    }

    /// device stream of the item, the ring it drains and the graph that fills it
    struct AudioOutput {
        sink: CpalSink,
        producer: Producer,
        device: DeviceConfig,
        target_samples: usize,
        timeline: AudioTimeline,
        has_atempo: bool,
        graph: Option<(AVFilterGraph, f64, u32)>,
        fed_samples: i64,
        leftover: Vec<f32>,
        drained: bool,
        flushing: bool,
        muted: bool,
        volume: f32,
    }

    impl AudioOutput {
        fn open(sample_rate: u32, channels: u16) -> Result<Self, String> {
            let channels = channels.clamp(1, crate::audio::format::MAXIMUM_CHANNELS);
            let source = StreamFormat::new(sample_rate, channels).map_err(|error| format!("{error:?}"))?;
            let device = cpal::default_host().default_output_device().ok_or("no default output device")?;
            let supported = supported_configs(&device).map_err(|error| format!("{error:?}"))?;
            let chosen = choose(source, &supported).map_err(|error| format!("{error:?}"))?;
            let (producer, consumer) = ring::ring(ring_samples(chosen));
            let sink = CpalSink::open(source, consumer).map_err(|error| format!("{error:?}"))?;
            let device = sink.config();
            let target_samples = ring_samples(device).min(producer.capacity());
            Ok(Self {
                sink,
                producer,
                device,
                target_samples,
                timeline: AudioTimeline::new(device.sample_rate),
                has_atempo: AVFilter::get_by_name(c"atempo").is_some(),
                graph: None,
                fed_samples: 0,
                leftover: Vec::new(),
                drained: false,
                flushing: false,
                muted: false,
                volume: 1.0,
            })
        }

        /// sound settings of the player, they reach the samples pushed from now on
        fn hear(&mut self, settings: PlayerSettings) {
            self.muted = settings.muted;
            self.volume = settings.volume;
        }

        fn channels(&self) -> usize {
            usize::from(self.device.channels.max(1))
        }

        /// frames the device has played of what was pushed
        fn played_frames(&self) -> u64 {
            let buffered = (self.producer.filled() / self.channels()) as u64;
            self.timeline.pushed_frames().saturating_sub(buffered)
        }

        /// media time the listener hears, none before any sound was pushed
        fn clock_us(&mut self) -> Option<i64> {
            let played = self.played_frames();
            self.timeline.media_us_at(played)
        }

        /// true once everything the graph gave was played
        fn finished(&self) -> bool {
            self.drained && self.leftover.is_empty() && self.producer.filled() == 0
        }

        /// the ring wants more samples and there are some to come
        fn hungry(&self) -> bool {
            if !self.leftover.is_empty() {
                return self.producer.free() >= self.channels();
            }
            !self.drained && self.producer.filled() < self.target_samples
        }

        fn retempo(&mut self, tempo: f64) {
            if self.graph.as_ref().is_some_and(|(_, current, _)| same_tempo(*current, tempo)) {
                return;
            }
            self.graph = None;
            self.leftover.clear();
            self.fed_samples = 0;
        }

        fn push_leftover(&mut self) {
            let channels = self.channels();
            let room = self.producer.free() / channels * channels;
            let count = self.leftover.len().min(room);
            if count == 0 {
                return;
            }
            if self.muted {
                self.leftover[..count].fill(0.0);
            } else if self.volume < 1.0 {
                let volume = self.volume;
                self.leftover[..count].iter_mut().for_each(|sample| *sample *= volume);
            }
            let pushed = self.producer.push(&self.leftover[..count]);
            self.leftover.drain(..pushed);
            self.timeline.pushed((pushed / channels) as u64);
        }

        fn take_from_graph(&mut self) -> Result<Option<()>, String> {
            let Some((graph, _, _)) = &mut self.graph else {
                return Ok(None);
            };
            let mut sink = graph.get_filter(GRAPH_SINK).ok_or("the graph lost its sink")?;
            match sink.buffersink_get_frame(None) {
                Ok(frame) => {
                    let channels = usize::try_from(frame.ch_layout.nb_channels).unwrap_or(0);
                    let frames = usize::try_from(frame.nb_samples).unwrap_or(0);
                    if channels != self.channels() || frame.data[0].is_null() {
                        return Err(format!("the graph gave {channels} channels for a {} channel device", self.channels()));
                    }
                    let samples = unsafe { std::slice::from_raw_parts(frame.data[0].cast::<f32>(), frames * channels) };
                    self.leftover.extend_from_slice(samples);
                    Ok(Some(()))
                }
                Err(RsmpegError::BufferSinkDrainError) => Ok(None),
                Err(RsmpegError::BufferSinkEofError) => {
                    self.drained = true;
                    Ok(None)
                }
                Err(error) => Err(format!("av_buffersink_get_frame: {error}")),
            }
        }

        /// feeds one decoded frame, building the graph for the current tempo first
        fn feed(&mut self, mut frame: AVFrame, media_us: Option<i64>, tempo: f64) -> Result<(), String> {
            if self.graph.is_none() {
                let format = AudioSourceFormat::of(&frame);
                let (graph, declared) = build_graph(&format, tempo, self.has_atempo, self.device)?;
                let origin = media_us.or_else(|| self.timeline.end_us()).unwrap_or(0);
                self.timeline.begin_segment(origin, tempo);
                self.graph = Some((graph, tempo, declared));
                self.fed_samples = 0;
            }
            let Some((graph, _, declared)) = &mut self.graph else {
                return Ok(());
            };
            frame.set_sample_rate(i32::try_from(*declared).unwrap_or(i32::MAX));
            frame.set_pts(self.fed_samples);
            self.fed_samples = self.fed_samples.saturating_add(i64::from(frame.nb_samples));
            let mut source = graph.get_filter(GRAPH_SOURCE).ok_or("the graph lost its source")?;
            source.buffersrc_add_frame(Some(frame), None).map_err(|error| format!("av_buffersrc_add_frame: {error}"))
        }

        /// tells the graph the source ended, once
        fn flush(&mut self) -> Result<(), String> {
            if self.flushing {
                return Ok(());
            }
            self.flushing = true;
            let Some((graph, _, _)) = &mut self.graph else {
                self.drained = true;
                return Ok(());
            };
            let mut source = graph.get_filter(GRAPH_SOURCE).ok_or("the graph lost its source")?;
            source.buffersrc_add_frame(None, None).map_err(|error| format!("av_buffersrc_add_frame: {error}"))
        }
    }

    /// decoded audio of the item and its device
    struct AudioTrack {
        decoder: StreamDecoder,
        output: AudioOutput,
        sample_rate: u32,
        channels: u16,
    }

    /// one item, its input and decoders and device and clocks
    struct MediaItem {
        item: u64,
        ends: ItemEndSink,
        demuxer: Demuxer,
        video: Option<VideoTrack>,
        audio: Option<AudioTrack>,
        start_us: i64,
        duration_us: Option<i64>,
        host_clock: HostMediaClock,
        tempo: f64,
        playing: bool,
        pending: Option<(i64, Shown)>,
        has_picture: bool,
        generation: u64,
        ended: bool,
        settings: PlayerSettings,
        awaiting_start: bool,
        seek: Option<(SeekWindow, SeekSink)>,
        anchor_us: i64,
        forward_target: Duration,
        presentation: Option<Size<f32>>,
        rotation_degrees: f64,
    }

    fn display_rotation(par: &ffi::AVCodecParameters) -> f64 {
        let side = unsafe { ffi::av_packet_side_data_get(par.coded_side_data, par.nb_coded_side_data, ffi::AV_PKT_DATA_DISPLAYMATRIX) };
        if side.is_null() {
            return 0.0;
        }
        let side = unsafe { &*side };
        if side.data.is_null() || side.size < 9 * std::mem::size_of::<i32>() {
            return 0.0;
        }
        unsafe { ffi::av_display_rotation_get(side.data.cast::<i32>().cast_const()) }
    }

    impl MediaItem {
        fn open(item: u64, source: &ClipSource, ends: ItemEndSink, settings: PlayerSettings) -> Result<Self, String> {
            let input = open_input(source)?;
            crate::decode_guard::decode_guard::check_ffmpeg_input(&input).map_err(|refusal| refusal.to_string())?;
            let video = StreamDecoder::open(&input, Track::Video)?;
            let audio = StreamDecoder::open(&input, Track::Audio)?;
            if video.is_none() && audio.is_none() {
                return Err("the item has neither a video nor an audio stream it can decode".to_owned());
            }
            let start_us = if input.start_time == ffi::AV_NOPTS_VALUE { 0 } else { input.start_time };
            let duration_us = (input.duration != ffi::AV_NOPTS_VALUE).then_some(input.duration);
            let (presentation, rotation_degrees) = match &video {
                Some((stream, _)) => {
                    let par = input.streams()[*stream].codecpar();
                    let rotation = display_rotation(&par);
                    let aspect = (par.sample_aspect_ratio.num, par.sample_aspect_ratio.den);
                    let width = u32::try_from(par.width).unwrap_or(0);
                    let height = u32::try_from(par.height).unwrap_or(0);
                    let shown = presentation_size_of(width, height, aspect, rotation);
                    ((shown.width > 0.0).then_some(shown), rotation)
                }
                None => (None, 0.0),
            };
            let audio = audio.and_then(|(stream, decoder)| {
                let sample_rate = u32::try_from(decoder.decoder.sample_rate).unwrap_or(0);
                let channels = u16::try_from(decoder.decoder.ch_layout.nb_channels).unwrap_or(0);
                match AudioOutput::open(sample_rate, channels) {
                    Ok(mut output) => {
                        output.hear(settings);
                        Some((stream, AudioTrack { decoder, output, sample_rate, channels }))
                    }
                    Err(reason) => {
                        tracing::warn!(target: "riseonly", %reason, "media player plays without sound");
                        None
                    }
                }
            });
            let demuxer = Demuxer {
                input,
                done: false,
                video_stream: video.as_ref().map(|(stream, _)| *stream),
                audio_stream: audio.as_ref().map(|(stream, _)| *stream),
                video_packets: VecDeque::new(),
                audio_packets: VecDeque::new(),
                start_us,
                range_start_us: 0,
                buffered_end_us: None,
            };
            Ok(Self {
                item,
                ends,
                demuxer,
                video: video.map(|(_, decoder)| VideoTrack { decoder, scaler: None, last_pts_us: 0 }),
                audio: audio.map(|(_, track)| track),
                start_us,
                duration_us,
                host_clock: HostMediaClock::at(0, RiseClock::monotonic()),
                tempo: 1.0,
                playing: false,
                pending: None,
                has_picture: false,
                generation: 0,
                ended: false,
                settings,
                awaiting_start: false,
                seek: None,
                anchor_us: 0,
                forward_target: forward_buffer_target(source.preferred_forward_buffer_duration),
                presentation,
                rotation_degrees,
            })
        }

        /// new player settings
        /// mute and volume reach the next samples pushed
        /// end action and stalling policy apply from the next play or end
        fn apply(&mut self, settings: PlayerSettings) {
            self.settings = settings;
            if let Some(audio) = &mut self.audio {
                audio.output.hear(settings);
            }
            if !settings.waits_to_minimize_stalling && self.awaiting_start {
                self.start_clocks();
            }
        }

        /// true when the clocks advance, played and not waiting for media to be read
        fn running(&self) -> bool {
            self.playing && !self.awaiting_start
        }

        /// media time a caller sees
        /// the seek target while a seek is in flight, otherwise the leading clock
        /// or where it last was while the sound has not started
        fn play_head_us(&mut self) -> i64 {
            if let Some((window, _)) = &self.seek {
                return window.target_us;
            }
            if let Some(clock_us) = self.clock_us() {
                self.anchor_us = clock_us;
            }
            self.anchor_us
        }

        /// seek of the item
        /// input goes to the keyframe before the target and decoders and device start over
        /// frames before the window are dropped, done hears true once the landing picture shows
        fn seek(&mut self, window: SeekWindow, done: SeekSink) {
            if let Err(reason) = self.demuxer.seek(window) {
                tracing::warn!(target: "riseonly", %reason, "media player could not seek");
                done.deliver(false);
                return;
            }
            if let Some((_, earlier)) = self.seek.replace((window, done)) {
                earlier.deliver(false);
            }
            if let Some(video) = &mut self.video {
                video.decoder.flush();
                video.last_pts_us = window.target_us;
            }
            self.pending = None;
            self.has_picture = false;
            self.ended = false;
            self.anchor_us = window.target_us;
            let now = RiseClock::monotonic();
            self.host_clock = HostMediaClock::at(window.target_us, now);
            if let Some(audio) = &mut self.audio {
                audio.decoder.flush();
                match AudioOutput::open(audio.sample_rate, audio.channels) {
                    Ok(mut output) => {
                        output.hear(self.settings);
                        audio.output = output;
                    }
                    Err(reason) => {
                        tracing::warn!(target: "riseonly", %reason, "media player plays on without sound after a seek");
                        self.audio = None;
                    }
                }
            }
            if self.playing {
                self.awaiting_start = self.settings.waits_to_minimize_stalling;
                if !self.awaiting_start {
                    self.start_clocks();
                }
            }
        }

        /// the seek in flight landed at media_us, the host clock goes on from there
        fn land_seek(&mut self, media_us: i64) {
            let Some((_, done)) = self.seek.take() else {
                return;
            };
            let now = RiseClock::monotonic();
            self.anchor_us = media_us;
            self.host_clock = HostMediaClock::at(media_us, now);
            if self.running() {
                self.host_clock.run(self.tempo, now);
            }
            done.deliver(true);
        }

        fn video_done(&self) -> bool {
            self.pending.is_none() && self.video.as_ref().is_none_or(|video| video.decoder.finished)
        }

        /// media time of the leading clock, none while the sound leads and nothing was pushed
        fn clock_us(&mut self) -> Option<i64> {
            match &mut self.audio {
                Some(audio) => audio.output.clock_us(),
                None => Some(self.host_clock.media_us(RiseClock::monotonic())),
            }
        }

        fn times(&mut self) -> PlaybackTimes {
            let media_us = self.play_head_us();
            let sound_pending = self.audio.is_some() && self.clock_us().is_none();
            let control = if !self.playing {
                AVPlayerTimeControlStatus::Paused
            } else if self.awaiting_start || self.seek.is_some() || sound_pending {
                AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate
            } else {
                AVPlayerTimeControlStatus::Playing
            };
            PlaybackTimes {
                item: self.item,
                current: Some(item_seconds(media_us, self.duration_us)),
                duration: duration_seconds(self.duration_us),
                status: AVPlayerItemStatus::ReadyToPlay,
                control,
                loaded: self.demuxer.loaded(),
                presentation: self.presentation,
            }
        }

        fn play(&mut self, tempo: f64) {
            if self.ended {
                return;
            }
            let was_running = self.running();
            self.playing = true;
            self.tempo = tempo;
            if let Some(audio) = &mut self.audio {
                audio.output.retempo(tempo);
            }
            let play_head = self.play_head_us();
            let ahead = self.demuxer.buffered_ahead(play_head);
            let keeps_up = likely_to_keep_up(ahead, self.demuxer.done, self.forward_target);
            if !was_running && self.settings.waits_to_minimize_stalling && !keeps_up {
                self.awaiting_start = true;
                return;
            }
            self.start_clocks();
        }

        /// starts the clocks, the host clock runs at the tempo and the device plays
        fn start_clocks(&mut self) {
            self.awaiting_start = false;
            let now = RiseClock::monotonic();
            self.host_clock.run(self.tempo, now);
            if let Some(audio) = &mut self.audio {
                if let Err(error) = audio.output.sink.play() {
                    tracing::warn!(target: "riseonly", ?error, "media player could not start the output");
                }
            }
        }

        fn pause(&mut self) {
            self.playing = false;
            self.awaiting_start = false;
            self.host_clock.stop(RiseClock::monotonic());
            if let Some(audio) = &mut self.audio {
                if let Err(error) = audio.output.sink.pause() {
                    tracing::warn!(target: "riseonly", ?error, "media player could not pause the output");
                }
            }
        }

        /// how long the thread may sleep before its next step, none sleeps until a command
        fn wait(&mut self) -> Option<Duration> {
            if self.ended {
                return None;
            }
            let play_head = self.play_head_us();
            let reading = self.demuxer.wants_more(play_head, self.forward_target);
            if reading || self.seek.is_some() {
                return Some(Duration::ZERO);
            }
            if self.playing && self.audio.as_ref().is_some_and(|audio| audio.output.hungry()) {
                return Some(Duration::ZERO);
            }
            if !self.running() {
                let first = !self.has_picture && !self.video_done();
                return (first || self.awaiting_start).then_some(Duration::ZERO);
            }
            if self.pending.is_none() && !self.video_done() {
                return Some(Duration::ZERO);
            }
            let audio_leads = self.audio.is_some();
            if let Some((pts_us, _)) = &self.pending {
                let pts_us = *pts_us;
                return match self.clock_us() {
                    None => Some(AUDIO_CLOCK_POLL),
                    Some(clock_us) => match media_picture_due(pts_us, clock_us, self.tempo, audio_leads) {
                        MediaPictureDue::Show | MediaPictureDue::Drop => Some(Duration::ZERO),
                        MediaPictureDue::After(wait) => Some(wait),
                    },
                };
            }
            Some(if audio_leads { AUDIO_CLOCK_POLL } else { Duration::ZERO })
        }

        /// one step, read ahead and fill the ring and decode or show a picture and land a seek and end the item
        fn step(&mut self, picture: &ItemPicture) {
            let play_head = self.play_head_us();
            if let Err(reason) = self.demuxer.read_ahead(play_head, self.forward_target) {
                tracing::warn!(target: "riseonly", %reason, "media player stopped reading the item");
                self.demuxer.done = true;
            }
            if self.awaiting_start {
                let ahead = self.demuxer.buffered_ahead(play_head);
                if likely_to_keep_up(ahead, self.demuxer.done, self.forward_target) {
                    self.start_clocks();
                }
            }
            if let Err(reason) = self.pump_audio() {
                tracing::warn!(target: "riseonly", %reason, "media player lost its sound");
                self.lose_audio();
            }
            if self.audio.as_ref().is_some_and(|audio| audio.output.finished()) {
                self.lose_audio();
            }
            if let Err(reason) = self.step_video(picture) {
                tracing::warn!(target: "riseonly", %reason, "media player decoding stopped");
                self.video = None;
                self.pending = None;
            }
            if self.seek.is_some() && self.video_done() {
                let target_us = self.seek.as_ref().map_or(self.anchor_us, |(window, _)| window.target_us);
                self.land_seek(target_us);
            }
            if !self.ended && self.running() && self.audio.is_none() && self.video_done() {
                self.end();
            }
        }

        /// the sound ended or failed, the host clock leads on from where the sound was
        fn lose_audio(&mut self) {
            let Some(mut audio) = self.audio.take() else {
                return;
            };
            let now = RiseClock::monotonic();
            let media_us = audio.output.clock_us().or_else(|| audio.output.timeline.end_us()).unwrap_or(self.anchor_us);
            self.host_clock = HostMediaClock::at(media_us, now);
            if self.running() {
                self.host_clock.run(self.tempo, now);
            }
            if let Err(error) = audio.output.sink.pause() {
                tracing::debug!(target: "riseonly", ?error, "media player output already stopped");
            }
        }

        fn pump_audio(&mut self) -> Result<(), String> {
            if !self.playing {
                return Ok(());
            }
            let Some(audio) = &mut self.audio else {
                return Ok(());
            };
            loop {
                audio.output.push_leftover();
                if !audio.output.leftover.is_empty() || !audio.output.hungry() {
                    return Ok(());
                }
                if audio.output.take_from_graph()?.is_some() {
                    continue;
                }
                if audio.output.drained {
                    return Ok(());
                }
                match audio.decoder.next_frame(&mut self.demuxer)? {
                    Some(frame) => {
                        let media_us = audio.decoder.media_us(&frame, self.start_us);
                        if let (Some((window, _)), Some(start_us)) = (&self.seek, media_us) {
                            let rate = i64::from(frame.sample_rate.max(1));
                            let end_us = start_us.saturating_add(i64::from(frame.nb_samples) * 1_000_000 / rate);
                            if window.is_before(end_us) {
                                continue;
                            }
                        }
                        audio.output.feed(frame, media_us, self.tempo)?;
                    }
                    None => {
                        audio.output.flush()?;
                        if audio.output.graph.is_none() {
                            return Ok(());
                        }
                    }
                }
            }
        }

        fn step_video(&mut self, picture: &ItemPicture) -> Result<(), String> {
            let Some(video) = &mut self.video else {
                return Ok(());
            };
            if self.pending.is_none() {
                if !(self.playing && !self.awaiting_start) && self.has_picture {
                    return Ok(());
                }
                loop {
                    let Some(frame) = video.decoder.next_frame(&mut self.demuxer)? else {
                        return Ok(());
                    };
                    let pts_us = video.decoder.media_us(&frame, self.start_us).unwrap_or(video.last_pts_us);
                    video.last_pts_us = pts_us;
                    if self.seek.as_ref().is_some_and(|(window, _)| window.is_before(pts_us)) {
                        continue;
                    }
                    let shown = video_picture(&frame, &mut video.scaler)?;
                    self.pending = Some((pts_us, shown));
                    break;
                }
            }
            let Some((pts_us, _)) = &self.pending else {
                return Ok(());
            };
            let pts_us = *pts_us;
            if !self.has_picture {
                self.show(picture);
                return Ok(());
            }
            if !self.running() {
                return Ok(());
            }
            let audio_leads = self.audio.is_some();
            let Some(clock_us) = self.clock_us() else {
                return Ok(());
            };
            match media_picture_due(pts_us, clock_us, self.tempo, audio_leads) {
                MediaPictureDue::Show => self.show(picture),
                MediaPictureDue::Drop => self.pending = None,
                MediaPictureDue::After(_) => {}
            }
            Ok(())
        }

        /// hands the pending picture to the layer, the first one after a seek is its landing
        fn show(&mut self, picture: &ItemPicture) {
            let Some((pts_us, mut shown)) = self.pending.take() else {
                return;
            };
            if self.presentation.is_none() {
                let decoded = presentation_size_of(shown.picture.width, shown.picture.height, (1, 1), self.rotation_degrees);
                self.presentation = (decoded.width > 0.0).then_some(decoded);
            }
            self.generation = self.generation.wrapping_add(1);
            shown.picture.generation = self.generation;
            *picture.lock() = Some((self.item, shown, self.rotation_degrees));
            self.has_picture = true;
            self.land_seek(pts_us);
        }

        /// did play to end time, once
        /// with end action pause the player stops at the item end
        /// with none it keeps playing at the end where time stands still until a seek
        fn end(&mut self) {
            self.ended = true;
            if self.settings.action_at_item_end != AVPlayerActionAtItemEnd::None {
                self.playing = false;
            }
            let now = RiseClock::monotonic();
            let end_us = self.duration_us.unwrap_or_else(|| self.host_clock.media_us(now));
            self.host_clock = HostMediaClock::at(end_us, now);
            self.anchor_us = end_us;
            self.ends.deliver();
        }
    }
}

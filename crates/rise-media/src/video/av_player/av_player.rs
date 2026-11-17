use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use futures::channel::mpsc::{self, UnboundedSender};
use futures::StreamExt as _;
use gpui::{
    canvas, div, size, AnyElement, App, Context, Entity, EventEmitter, IntoElement, ObjectFit, ParentElement as _,
    Pixels, RenderOnce, Size, Styled as _, SurfaceSource, Task, WeakEntity, Window,
};
use rise_core::foundation::foundation_url::Url;
use rise_platform::{HostOs, PlatformSupport};
use thiserror::Error;

use crate::decode_guard::decode_guard::{DecodeGuardError, check_video_file, native_call};
use crate::video::av_asset::AVURLAsset;
pub use crate::video::av_player::pixel_buffer::{AVPlayerPixelBuffer, PixelBufferImage};

/// why the system player could not be opened or could not loop
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum AVPlayerError {
    #[error("{host:?} cannot play a clip yet: {reason}")]
    NotOnThisHost { host: HostOs, reason: &'static str },
    #[error("the system player cannot open {0}")]
    BadUrl(String),
    #[error("{call} returned nil")]
    Platform { call: &'static str },
    #[error("the media item was refused before decoding: {0}")]
    Undecodable(DecodeGuardError),
    #[error("the media item is still being checked")]
    ItemBeingChecked,
    #[error("{call} is not wired on this host yet")]
    Unsupported { call: &'static str },
}

fn local_file_path(item: &AVPlayerItem) -> Option<PathBuf> {
    let url = item.url();
    url.is_file_url().then(|| url.to_file_path()).flatten()
}

fn spawn_item_check<P: 'static>(
    path: PathBuf,
    cx: &mut Context<P>,
    finish: impl FnOnce(&mut P, Result<(), AVPlayerError>, &mut Context<P>) + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let verdict = cx
            .background_executor()
            .spawn(async move { check_video_file(&path).map_err(AVPlayerError::Undecodable) })
            .await;
        let _ = this.update(cx, |player, cx| finish(player, verdict, cx));
    })
}

fn open_clip_engine(
    item: &AVPlayerItem,
    sender: UnboundedSender<Url>,
) -> Result<Box<dyn PlayerEngine>, AVPlayerError> {
    let mut engine = native_call(|| super::os::open_engine(item, EndSink::new(item.url().clone(), sender)))
        .map_err(AVPlayerError::Undecodable)??;
    if crate::audio::sound_mute::is_sound_muted() {
        engine.set_is_muted(true);
    }
    Ok(engine)
}

type LooperSlot = RefCell<Option<Box<dyn Any>>>;

/// what is missing on each host before a clip plays
pub const fn unwired_reason(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "the macOS branch did not open the system player",
        HostOs::Linux => {
            "the LGPL FFmpeg build this player decodes with is not shipped for this host yet (02 section 5 item 3); the layer draws a transparent placeholder"
        }
        HostOs::Windows => "the Windows branch did not open the Media Engine player",
    }
}

/// one playable asset at a url
/// cloning shares nothing with the system, every player opens its own item from the url
/// an item made from an asset keeps it and reads bytes through its resource loader
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AVPlayerItem {
    url: Url,
    asset: Option<AVURLAsset>,
    preferred_forward_buffer_duration: Duration,
}

impl AVPlayerItem {
    pub fn new(url: Url) -> Self {
        Self { url, asset: None, preferred_forward_buffer_duration: Duration::ZERO }
    }

    pub fn with_asset(asset: AVURLAsset) -> Self {
        Self { url: asset.url().clone(), asset: Some(asset), preferred_forward_buffer_duration: Duration::ZERO }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    /// asset the item was made from if it was, its resource loader serves the bytes
    pub fn asset(&self) -> Option<&AVURLAsset> {
        self.asset.as_ref()
    }

    /// starts the asset resource loader on the main thread
    /// so a backend reading the item has a delegate to answer
    /// items made from a url have nothing to start
    pub fn start_resource_loading(&self, cx: &mut App) {
        if let Some(asset) = &self.asset {
            asset.resource_loader().start(cx);
        }
    }

    /// how much media past the play head the player should keep buffered
    /// zero lets the system choose
    pub fn preferred_forward_buffer_duration(&self) -> Duration {
        self.preferred_forward_buffer_duration
    }

    /// set before the item is handed to AVQueuePlayer new
    /// the system item is opened with this value, copies a looper plays keep it
    pub fn set_preferred_forward_buffer_duration(&mut self, duration: Duration) {
        self.preferred_forward_buffer_duration = duration;
    }
}

/// what a player does at item end
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AVPlayerActionAtItemEnd {
    #[default]
    Advance,
    Pause,
    None,
}

impl AVPlayerActionAtItemEnd {
    /// the NSInteger for the action, advance 0 and pause 1 and none 2
    pub const fn code(self) -> isize {
        match self {
            Self::Advance => 0,
            Self::Pause => 1,
            Self::None => 2,
        }
    }
}

/// how video is placed in the layer
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AVLayerVideoGravity {
    #[default]
    ResizeAspect,
    ResizeAspectFill,
    Resize,
}

impl AVLayerVideoGravity {
    /// gpui fit that draws a picture the way the gravity places it in the layer bounds
    pub const fn object_fit(self) -> ObjectFit {
        match self {
            Self::ResizeAspect => ObjectFit::Contain,
            Self::ResizeAspectFill => ObjectFit::Cover,
            Self::Resize => ObjectFit::Fill,
        }
    }
}

/// notifications the player entity emits
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AVPlayerItemNotification {
    DidPlayToEndTime { asset_url: Url },
}

/// how a url reaches the system player, a file path or the absolute string of a remote asset
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SystemUrl {
    File(String),
    Remote(String),
}

impl SystemUrl {
    pub fn of(url: &Url) -> Result<Self, AVPlayerError> {
        if url.is_file_url() {
            let path = url.path();
            if path.is_empty() {
                return Err(AVPlayerError::BadUrl(url.absolute_string().to_owned()));
            }
            return Ok(Self::File(path));
        }
        match url.scheme() {
            Some(scheme) if !scheme.is_empty() => Ok(Self::Remote(url.absolute_string().to_owned())),
            _ => Err(AVPlayerError::BadUrl(url.absolute_string().to_owned())),
        }
    }
}

/// true if an item the system says played to its end is a copy of template, matched by asset url
pub fn is_end_of(template: &Url, asset_url: &str) -> bool {
    if template.absolute_string() == asset_url {
        return true;
    }
    Url::from_string(asset_url).is_some_and(|ended| ended.standardized() == template.standardized())
}

/// where the os layer posts the asset url of every item that played to its end, any thread
#[derive(Clone)]
pub struct EndSink {
    template: Url,
    sender: UnboundedSender<Url>,
}

impl EndSink {
    pub fn new(template: Url, sender: UnboundedSender<Url>) -> Self {
        Self { template, sender }
    }

    /// url the player was opened with, for the os layer own lookups
    pub fn template(&self) -> &Url {
        &self.template
    }

    /// forwards an end whose asset url matches the opened item, others are not this player
    pub fn deliver(&self, asset_url: &str) {
        if is_end_of(&self.template, asset_url) {
            let _ = self.sender.unbounded_send(self.template.clone());
        }
    }
}

/// os half of the seam, one system player
/// settings are applied as they change and never read back, the hub keeps the values the caller set
pub trait PlayerEngine {
    fn set_is_muted(&mut self, muted: bool);
    fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd);
    fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool);
    fn play(&mut self);
    fn pause(&mut self);
    fn start_looping(&mut self, template: &AVPlayerItem) -> Result<Box<dyn Any>, AVPlayerError>;
    /// newest picture for the current host time drawn into the caller bounds with fit
    /// none before the first picture is decoded
    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement>;
    /// key the drawn surface carries for gpui in place swap
    /// none on hosts whose layer renders again for every picture
    fn swap_key(&self) -> Option<u64> {
        None
    }
    fn take_swapped_picture(&mut self) -> Option<SurfaceSource> {
        None
    }
}

/// whether this host plays clips and media at all
pub fn player_support() -> PlatformSupport {
    if super::os::player_available() {
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    }
}

/// gpui entity around the system player of one item
pub struct AVQueuePlayer {
    item: AVPlayerItem,
    is_muted: bool,
    action_at_item_end: AVPlayerActionAtItemEnd,
    automatically_waits_to_minimize_stalling: bool,
    is_playing: bool,
    engine: Result<Box<dyn PlayerEngine>, AVPlayerError>,
    item_is_checking: bool,
    pending_looper: Option<(AVPlayerItem, Weak<LooperSlot>)>,
    _item_check: Option<Task<()>>,
    _ends: Task<()>,
}

impl EventEmitter<AVPlayerItemNotification> for AVQueuePlayer {}

impl AVQueuePlayer {
    /// player for one item, paused until play
    /// a host without a player keeps the error and draws a placeholder
    pub fn new(item: AVPlayerItem, cx: &mut Context<Self>) -> Self {
        item.start_resource_loading(cx);
        let (sender, mut receiver) = mpsc::unbounded::<Url>();
        let (engine, item_check) = match local_file_path(&item) {
            Some(path) => {
                let check = spawn_item_check(path, cx, move |player: &mut Self, verdict, cx| {
                    player.finish_item_check(verdict, sender, cx);
                });
                (Err(AVPlayerError::ItemBeingChecked), Some(check))
            }
            None => {
                let opened = open_clip_engine(&item, sender);
                if let Err(error) = &opened {
                    tracing::warn!(target: "riseonly", %error, "clip player unavailable");
                }
                (opened, None)
            }
        };
        let item_is_checking = item_check.is_some();
        let ends = cx.spawn(async move |this, cx| {
            while let Some(asset_url) = receiver.next().await {
                let emitted = this.update(cx, |_, cx| {
                    cx.emit(AVPlayerItemNotification::DidPlayToEndTime { asset_url });
                });
                if emitted.is_err() {
                    break;
                }
            }
        });
        Self {
            item,
            is_muted: false,
            action_at_item_end: AVPlayerActionAtItemEnd::Advance,
            automatically_waits_to_minimize_stalling: true,
            is_playing: false,
            engine,
            item_is_checking,
            pending_looper: None,
            _item_check: item_check,
            _ends: ends,
        }
    }

    fn finish_item_check(
        &mut self,
        verdict: Result<(), AVPlayerError>,
        sender: UnboundedSender<Url>,
        cx: &mut Context<Self>,
    ) {
        self.item_is_checking = false;
        let looper = self.pending_looper.take();
        self.engine = match verdict.and_then(|()| open_clip_engine(&self.item, sender)) {
            Ok(mut engine) => {
                if self.is_muted {
                    engine.set_is_muted(true);
                }
                engine.set_action_at_item_end(self.action_at_item_end);
                engine.set_automatically_waits_to_minimize_stalling(self.automatically_waits_to_minimize_stalling);
                if let Some((template, slot)) = looper {
                    if let Some(slot) = slot.upgrade() {
                        match engine.start_looping(&template) {
                            Ok(holder) => *slot.borrow_mut() = Some(holder),
                            Err(error) => tracing::warn!(target: "riseonly", %error, "clip looper unavailable"),
                        }
                    }
                }
                if self.is_playing {
                    engine.play();
                }
                Ok(engine)
            }
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "clip player unavailable");
                Err(error)
            }
        };
        cx.notify();
    }

    pub fn item(&self) -> &AVPlayerItem {
        &self.item
    }

    /// why nothing plays on this host if that is so
    pub fn error(&self) -> Option<&AVPlayerError> {
        if self.item_is_checking {
            return None;
        }
        self.engine.as_ref().err()
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    pub fn set_is_muted(&mut self, muted: bool) {
        self.is_muted = muted;
        if let Ok(engine) = &mut self.engine {
            engine.set_is_muted(muted || crate::audio::sound_mute::is_sound_muted());
        }
    }

    pub fn action_at_item_end(&self) -> AVPlayerActionAtItemEnd {
        self.action_at_item_end
    }

    pub fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) {
        self.action_at_item_end = action;
        if let Ok(engine) = &mut self.engine {
            engine.set_action_at_item_end(action);
        }
    }

    pub fn automatically_waits_to_minimize_stalling(&self) -> bool {
        self.automatically_waits_to_minimize_stalling
    }

    pub fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) {
        self.automatically_waits_to_minimize_stalling = waits;
        if let Ok(engine) = &mut self.engine {
            engine.set_automatically_waits_to_minimize_stalling(waits);
        }
    }

    /// true when the rate is not zero
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    pub fn play(&mut self, cx: &mut Context<Self>) {
        self.is_playing = true;
        if let Ok(engine) = &mut self.engine {
            engine.play();
        }
        cx.notify();
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.is_playing = false;
        if let Ok(engine) = &mut self.engine {
            engine.pause();
        }
        cx.notify();
    }

    fn start_looping(&mut self, template: &AVPlayerItem) -> Result<Box<dyn Any>, AVPlayerError> {
        if self.item_is_checking {
            let slot: Rc<LooperSlot> = Rc::new(RefCell::new(None));
            self.pending_looper = Some((template.clone(), Rc::downgrade(&slot)));
            return Ok(Box::new(slot));
        }
        match &mut self.engine {
            Ok(engine) => engine.start_looping(template),
            Err(error) => Err(error.clone()),
        }
    }

    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
        self.engine.as_mut().ok().and_then(|engine| engine.frame(fit))
    }

    fn swap_key(&self) -> Option<u64> {
        self.engine.as_ref().ok().and_then(|engine| engine.swap_key())
    }

    fn take_swapped_picture(&mut self) -> Option<SurfaceSource> {
        self.engine.as_mut().ok().and_then(|engine| engine.take_swapped_picture())
    }
}

/// replays copies of the template item on the player while this value lives
/// dropped, the clip plays once
pub struct AVPlayerLooper {
    looping: Result<Box<dyn Any>, AVPlayerError>,
}

impl AVPlayerLooper {
    pub fn new(player: &Entity<AVQueuePlayer>, template_item: &AVPlayerItem, cx: &mut App) -> Self {
        template_item.start_resource_loading(cx);
        let looping = player.update(cx, |player, _| player.start_looping(template_item));
        if let Err(error) = &looping {
            tracing::warn!(target: "riseonly", %error, "clip looper unavailable");
        }
        Self { looping }
    }

    /// the failure reason if the looper failed
    pub fn error(&self) -> Option<&AVPlayerError> {
        self.looping.as_ref().err()
    }
}

/// time range in seconds
/// a start or duration that isnt a finite number makes an invalid range
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CMTimeRange {
    pub start: f64,
    pub duration: f64,
}

impl CMTimeRange {
    pub const fn new(start: f64, duration: f64) -> Self {
        Self { start, duration }
    }

    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// both times numeric and the duration not negative
    pub fn is_valid(&self) -> bool {
        self.start.is_finite() && self.duration.is_finite() && self.duration >= 0.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AVPlayerTimeControlStatus {
    Paused,
    WaitingToPlayAtSpecifiedRate,
    Playing,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AVPlayerItemStatus {
    Unknown,
    ReadyToPlay,
    Failed,
}

/// one seek in microseconds of item time, the player may land anywhere in min_us..=max_us
/// as close to target_us as it can, infinite tolerance leaves that side open
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeekWindow {
    pub target_us: i64,
    pub min_us: i64,
    pub max_us: i64,
}

impl SeekWindow {
    /// window of a seek in seconds, none for a target that isnt a number
    /// a negative target seeks to the start and a negative or NaN tolerance counts as zero
    pub fn of(to: f64, tolerance_before: f64, tolerance_after: f64) -> Option<Self> {
        if !to.is_finite() {
            return None;
        }
        let target_us = seconds_to_us(to.max(0.0));
        let side = |tolerance: f64| -> i64 {
            if tolerance.is_nan() || tolerance <= 0.0 {
                0
            } else if tolerance.is_infinite() {
                i64::MAX
            } else {
                seconds_to_us(tolerance)
            }
        };
        let min_us = target_us.saturating_sub(side(tolerance_before)).max(0);
        let max_us = target_us.saturating_add(side(tolerance_after));
        Some(Self { target_us, min_us, max_us })
    }

    /// true if a frame at media_us lies before the window and is decoded only to reach it
    pub fn is_before(&self, media_us: i64) -> bool {
        media_us < self.min_us
    }

    /// true if the seek must land exactly on the target, like zero tolerances ask
    pub fn is_exact(&self) -> bool {
        self.min_us == self.target_us && self.max_us == self.target_us
    }
}

fn seconds_to_us(seconds: f64) -> i64 {
    let us = (seconds * 1_000_000.0).round();
    if us >= i64::MAX as f64 { i64::MAX } else { us as i64 }
}

/// where the os layer posts how a seek ended, any thread
/// finished is false when the seek was interrupted by another seek or a new item or a failure
#[derive(Clone)]
pub struct SeekSink {
    id: u64,
    sender: UnboundedSender<(u64, bool)>,
}

impl SeekSink {
    pub fn new(id: u64, sender: UnboundedSender<(u64, bool)>) -> Self {
        Self { id, sender }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn deliver(&self, finished: bool) {
        let _ = self.sender.unbounded_send((self.id, finished));
    }
}

/// presentation size, the decoded size stretched by the sample aspect ratio
/// and turned by the track rotation so a portrait phone video gives a portrait size
/// zero while unknown
pub fn presentation_size_of(width: u32, height: u32, sample_aspect: (i32, i32), rotation_degrees: f64) -> Size<f32> {
    if width == 0 || height == 0 {
        return size(0.0, 0.0);
    }
    let (num, den) = sample_aspect;
    let stretch = if num > 0 && den > 0 { f64::from(num) / f64::from(den) } else { 1.0 };
    let shown_width = f64::from(width) * stretch;
    let shown_height = f64::from(height);
    let quarter_turns = if rotation_degrees.is_finite() { (rotation_degrees / 90.0).round() as i64 } else { 0 };
    if quarter_turns.rem_euclid(2) == 1 {
        size(shown_height as f32, shown_width as f32)
    } else {
        size(shown_width as f32, shown_height as f32)
    }
}

/// how far past the play head a player keeps media read
/// the preferred duration or its own choice for zero
pub fn forward_buffer_target(preferred: Duration) -> Duration {
    const PLAYER_CHOICE: Duration = Duration::from_secs(8);
    if preferred.is_zero() { PLAYER_CHOICE } else { preferred }
}

/// whether a player that waits to minimize stalling may start
/// enough media is read past the play head or the whole item is
pub fn likely_to_keep_up(buffered_ahead: Duration, read_to_end: bool, target: Duration) -> bool {
    const START_AHEAD: Duration = Duration::from_secs(2);
    read_to_end || buffered_ahead >= START_AHEAD.min(target)
}

/// what a kvo observer of the player reads, the entity notifies whenever it changes
#[derive(Clone, PartialEq, Debug)]
struct ObservedState {
    item_status: AVPlayerItemStatus,
    time_control_status: AVPlayerTimeControlStatus,
    loaded_time_ranges: Vec<CMTimeRange>,
    presentation_size: Size<f32>,
    is_ready_for_display: bool,
}

const OBSERVATION_POLL: Duration = Duration::from_millis(100);

const PAUSED_BUFFERING_POLL: Duration = Duration::from_millis(500);

const PAUSED_BUFFERING_IDLE_TICKS: u32 = 4;

/// why the entity keeps reading the engine
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ObservationNeed {
    None,
    Buffering,
    Active,
}

/// where the os layer posts that the item it got with this sink played to its end, any thread
/// the generation tells a late end of a replaced item from the current one
#[derive(Clone)]
pub struct ItemEndSink {
    generation: u64,
    sender: UnboundedSender<u64>,
}

impl ItemEndSink {
    pub fn new(generation: u64, sender: UnboundedSender<u64>) -> Self {
        Self { generation, sender }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// posts the did play to end time notification with the item
    pub fn deliver(&self) {
        let _ = self.sender.unbounded_send(self.generation);
    }
}

/// os half of the player, one system player with sound and a rate and a replaceable item
/// settings are applied as they change and only times are read back
pub trait PlaybackEngine {
    /// swaps the current item
    /// the engine observes the end of the new item and delivers it to ends
    /// a replaced item delivers nothing more, an item the system cant open leaves the player empty
    fn replace_current_item(&mut self, item: Option<&AVPlayerItem>, ends: ItemEndSink) -> Result<(), AVPlayerError>;
    /// plays at once without waiting to minimize stalling
    fn play_immediately(&mut self, rate: f32);
    fn set_rate(&mut self, rate: f32);
    fn pause(&mut self);
    /// current time in seconds of the current item, none without an item or a valid time
    fn current_time(&self) -> Option<f64>;
    /// duration in seconds of the current item, none while indefinite or invalid
    fn current_item_duration(&self) -> Option<f64>;
    /// newest picture for the current host time drawn into the caller bounds with fit
    /// none before the first picture is decoded
    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement>;
    /// key the drawn surface carries for gpui in place swap
    /// none on hosts whose layer renders again for every picture
    fn swap_key(&self) -> Option<u64> {
        None
    }
    fn take_swapped_picture(&mut self) -> Option<SurfaceSource> {
        None
    }
    /// seeks the current item
    /// engine delivers to done once, true when the play head landed in window
    /// a later seek or a new item leaves the earlier sink undelivered or delivers false
    fn seek(&mut self, window: SeekWindow, done: SeekSink) {
        let _ = window;
        done.deliver(false);
    }
    /// sets mute, err keeps the old value in the hub
    fn set_is_muted(&mut self, muted: bool) -> Result<(), AVPlayerError> {
        let _ = muted;
        Err(AVPlayerError::Unsupported { call: "isMuted" })
    }
    /// player volume in 0..=1 (the hub clamps it), kept across items
    /// err keeps the old value in the hub
    fn set_volume(&mut self, volume: f32) -> Result<(), AVPlayerError> {
        let _ = volume;
        Err(AVPlayerError::Unsupported { call: "volume" })
    }
    fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) -> Result<(), AVPlayerError> {
        let _ = action;
        Err(AVPlayerError::Unsupported { call: "actionAtItemEnd" })
    }
    fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) -> Result<(), AVPlayerError> {
        let _ = waits;
        Err(AVPlayerError::Unsupported { call: "automaticallyWaitsToMinimizeStalling" })
    }
    /// time control status while the rate is not zero, none lets the hub answer playing
    fn time_control_status(&self) -> Option<AVPlayerTimeControlStatus> {
        None
    }
    /// item status, none lets the hub derive it from the known duration
    fn item_status(&self) -> Option<AVPlayerItemStatus> {
        None
    }
    /// loaded time ranges in seconds of item time, empty while unknown
    fn loaded_time_ranges(&self) -> Vec<CMTimeRange> {
        Vec::new()
    }
    /// presentation size, none while unknown
    fn presentation_size(&self) -> Option<Size<f32>> {
        None
    }
    /// true once a picture of the current item is decoded so frame draws it
    /// none lets the hub answer from the item status
    fn is_ready_for_display(&self) -> Option<bool> {
        None
    }
    fn copy_pixel_buffer(&mut self) -> Option<AVPlayerPixelBuffer> {
        None
    }
}

/// seconds a cmtime holds, none unless it is valid and numeric
pub fn cm_time_seconds(value: i64, timescale: i32, flags: u32) -> Option<f64> {
    const VALID: u32 = 1;
    const NOT_NUMERIC: u32 = 4 | 8 | 16;
    if flags & VALID == 0 || flags & NOT_NUMERIC != 0 || timescale <= 0 {
        return None;
    }
    let seconds = value as f64 / f64::from(timescale);
    seconds.is_finite().then_some(seconds)
}

/// volume the player takes, clamped to 0..=1 and none for NaN which leaves the player as it was
pub fn player_volume(volume: f32) -> Option<f32> {
    (!volume.is_nan()).then(|| volume.clamp(0.0, 1.0))
}

/// whether a periodic observer fires for now after it last reported last
/// on the first valid time and whenever the time moved, so a paused player doesnt repeat itself
pub fn time_observer_fires(last: Option<f64>, now: Option<f64>) -> bool {
    match (last, now) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(last), Some(now)) => last != now,
    }
}

/// token a periodic time observer returns
/// the observer stops when the token is handed to remove_time_observer or dropped
pub struct AVPlayerTimeObserver {
    _ticks: Task<()>,
}

/// gpui entity around one system player that plays its item with sound at a rate and replaces it
/// emits DidPlayToEndTime for the current item only
pub struct AVPlayer {
    item: Option<AVPlayerItem>,
    rate: f32,
    default_rate: f32,
    is_muted: bool,
    volume: f32,
    action_at_item_end: AVPlayerActionAtItemEnd,
    automatically_waits_to_minimize_stalling: bool,
    generation: u64,
    item_error: Option<AVPlayerError>,
    item_is_checking: bool,
    deferred_seek: Option<SeekWindow>,
    _item_check: Option<Task<()>>,
    ends: UnboundedSender<u64>,
    engine: Result<Box<dyn PlaybackEngine>, AVPlayerError>,
    seek_id: u64,
    pending_seek: Option<(u64, Box<dyn FnOnce(bool, &mut App)>)>,
    seeks: UnboundedSender<(u64, bool)>,
    _ends: Task<()>,
    _seeks: Task<()>,
    _observation: Option<Task<()>>,
    observing: bool,
}

impl EventEmitter<AVPlayerItemNotification> for AVPlayer {}

impl AVPlayer {
    /// player with no item and rate zero
    /// a host without a player keeps the error, the caller then shows its cover and plays nothing
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut receiver) = mpsc::unbounded::<u64>();
        let mut engine = super::os::open_playback_engine();
        match &mut engine {
            Ok(engine) => {
                if crate::audio::sound_mute::is_sound_muted() {
                    if let Err(error) = engine.set_is_muted(true) {
                        tracing::debug!(target: "riseonly", %error, "media player keeps its sound");
                    }
                }
                if let Err(error) = engine.set_action_at_item_end(AVPlayerActionAtItemEnd::Pause) {
                    tracing::debug!(target: "riseonly", %error, "media player keeps its own end action");
                }
            }
            Err(error) => tracing::warn!(target: "riseonly", %error, "media player unavailable"),
        }
        let ends = cx.spawn(async move |this, cx| {
            while let Some(generation) = receiver.next().await {
                let handled = this.update(cx, |player, cx| player.item_did_play_to_end(generation, cx));
                if handled.is_err() {
                    break;
                }
            }
        });
        let (seek_sender, mut seek_receiver) = mpsc::unbounded::<(u64, bool)>();
        let seeks = cx.spawn(async move |this, cx| {
            while let Some((id, finished)) = seek_receiver.next().await {
                let Ok(completion) = this.update(cx, |player, cx| player.seek_did_finish(id, cx)) else {
                    break;
                };
                if let Some(completion) = completion {
                    cx.update(|cx| completion(finished, cx));
                }
            }
        });
        Self {
            item: None,
            rate: 0.0,
            default_rate: 1.0,
            is_muted: false,
            volume: 1.0,
            action_at_item_end: AVPlayerActionAtItemEnd::Pause,
            automatically_waits_to_minimize_stalling: true,
            generation: 0,
            item_error: None,
            item_is_checking: false,
            deferred_seek: None,
            _item_check: None,
            ends: sender,
            engine,
            seek_id: 0,
            pending_seek: None,
            seeks: seek_sender,
            _ends: ends,
            _seeks: seeks,
            _observation: None,
            observing: false,
        }
    }

    /// why nothing plays, the host has no player or the current item could not be opened
    pub fn error(&self) -> Option<&AVPlayerError> {
        self.engine.as_ref().err().or(self.item_error.as_ref())
    }

    pub fn support(&self) -> PlatformSupport {
        if self.engine.is_ok() {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }

    pub fn current_item(&self) -> Option<&AVPlayerItem> {
        self.item.as_ref()
    }

    /// swaps the current item, rate stays zero until the next play
    /// engine opens it with the preferred forward buffer duration
    /// a seek still running on the old item completes with false
    pub fn replace_current_item(&mut self, item: Option<AVPlayerItem>, cx: &mut Context<Self>) {
        if let Some(item) = &item {
            item.start_resource_loading(cx);
        }
        self.finish_pending_seek(false, cx);
        self.generation = self.generation.wrapping_add(1);
        self.item_error = None;
        self.rate = 0.0;
        self.item_is_checking = false;
        self.deferred_seek = None;
        self._item_check = None;
        if let Ok(engine) = &mut self.engine {
            let ends = ItemEndSink::new(self.generation, self.ends.clone());
            let path = item.as_ref().and_then(local_file_path);
            let next = if path.is_none() { item.as_ref() } else { None };
            let opened = native_call(|| engine.replace_current_item(next, ends))
                .map_err(AVPlayerError::Undecodable)
                .and_then(|opened| opened);
            match (opened, path) {
                (Err(error), _) => {
                    tracing::warn!(target: "riseonly", %error, "media item unavailable");
                    self.item_error = Some(error);
                }
                (Ok(()), Some(path)) => {
                    let generation = self.generation;
                    self.item_is_checking = true;
                    self._item_check = Some(spawn_item_check(path, cx, move |player: &mut Self, verdict, cx| {
                        player.finish_item_check(generation, verdict, cx);
                    }));
                }
                (Ok(()), None) => {}
            }
        }
        self.item = item;
        self._observation = None;
        self.observing = false;
        self.resume_observation(cx);
        cx.notify();
    }

    fn finish_item_check(&mut self, generation: u64, verdict: Result<(), AVPlayerError>, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        self.item_is_checking = false;
        self.resume_observation(cx);
        let ends = ItemEndSink::new(generation, self.ends.clone());
        let item = self.item.as_ref();
        let opened = match (&mut self.engine, verdict) {
            (Ok(engine), Ok(())) => native_call(|| engine.replace_current_item(item, ends))
                .map_err(AVPlayerError::Undecodable)
                .and_then(|opened| opened),
            (_, verdict) => verdict,
        };
        let seek = self.deferred_seek.take();
        match opened {
            Ok(()) => {
                if let Ok(engine) = &mut self.engine {
                    if let Some(window) = seek {
                        engine.seek(window, SeekSink::new(self.seek_id, self.seeks.clone()));
                    }
                    if self.rate != 0.0 {
                        if self.automatically_waits_to_minimize_stalling {
                            engine.set_rate(self.rate);
                        } else {
                            engine.play_immediately(self.rate);
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "media item unavailable");
                self.item_error = Some(error);
                self.finish_pending_seek(false, cx);
            }
        }
        cx.notify();
    }

    /// starts reading the engine again after a change that can move what observers read
    /// the poll stops by itself once needs_observation is none
    fn resume_observation(&mut self, cx: &mut Context<Self>) {
        if self.item.is_some() && !self.observing {
            self.observing = true;
            self._observation = Some(Self::observe(cx));
        }
    }

    fn needs_observation(&self) -> ObservationNeed {
        if self.item.is_none() {
            return ObservationNeed::None;
        }
        let active = self.rate != 0.0
            || self.item_is_checking
            || self.pending_seek.is_some()
            || self.deferred_seek.is_some()
            || self.current_item_status() == AVPlayerItemStatus::Unknown
            || (self.current_item_status() == AVPlayerItemStatus::ReadyToPlay && !self.is_ready_for_display());
        if active {
            ObservationNeed::Active
        } else if self.current_item_status() == AVPlayerItemStatus::ReadyToPlay && !self.is_fully_loaded() {
            ObservationNeed::Buffering
        } else {
            ObservationNeed::None
        }
    }

    fn is_fully_loaded(&self) -> bool {
        let Some(duration) = self.current_item_duration().filter(|d| d.is_finite() && *d > 0.0) else {
            return false;
        };
        let loaded_end = self
            .loaded_time_ranges()
            .iter()
            .filter(|range| range.is_valid())
            .map(CMTimeRange::end)
            .fold(0.0_f64, f64::max);
        loaded_end + 0.05 >= duration
    }

    /// reads the engine while there is an item and notifies on every change of what observers read
    /// status and time control and loaded ranges and size and ready for display
    fn observe(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut last: Option<ObservedState> = None;
            let mut interval = OBSERVATION_POLL;
            let mut idle_ticks = 0_u32;
            loop {
                cx.background_executor().timer(interval).await;
                let observed = this.update(cx, |player, cx| {
                    let now = player.observed_state();
                    let changed = last.as_ref() != Some(&now);
                    if changed {
                        last = Some(now);
                        cx.notify();
                    }
                    let need = match player.needs_observation() {
                        ObservationNeed::Active => {
                            idle_ticks = 0;
                            Some(OBSERVATION_POLL)
                        }
                        ObservationNeed::Buffering => {
                            idle_ticks = if changed { 0 } else { idle_ticks + 1 };
                            (idle_ticks < PAUSED_BUFFERING_IDLE_TICKS).then_some(PAUSED_BUFFERING_POLL)
                        }
                        ObservationNeed::None => None,
                    };
                    if need.is_none() {
                        player.observing = false;
                    }
                    need
                });
                match observed {
                    Ok(Some(next)) => interval = next,
                    _ => break,
                }
            }
        })
    }

    fn observed_state(&self) -> ObservedState {
        ObservedState {
            item_status: self.current_item_status(),
            time_control_status: self.time_control_status(),
            loaded_time_ranges: self.loaded_time_ranges(),
            presentation_size: self.presentation_size(),
            is_ready_for_display: self.is_ready_for_display(),
        }
    }

    /// plays at the default rate
    pub fn play(&mut self, cx: &mut Context<Self>) {
        self.set_rate(self.default_rate, cx);
    }

    /// rate play starts with
    pub fn default_rate(&self) -> f32 {
        self.default_rate
    }

    pub fn set_default_rate(&mut self, rate: f32, cx: &mut Context<Self>) {
        self.default_rate = rate;
        cx.notify();
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    /// sets mute
    /// the app wide sound mute keeps the system player silent either way
    pub fn set_is_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        let applied = match &mut self.engine {
            Ok(engine) => engine.set_is_muted(muted || crate::audio::sound_mute::is_sound_muted()),
            Err(_) => Ok(()),
        };
        match applied {
            Ok(()) => self.is_muted = muted,
            Err(error) => tracing::debug!(target: "riseonly", %error, "media player keeps its sound"),
        }
        cx.notify();
    }

    /// volume, 1 until set
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// sets volume clamped to 0..=1, NaN changes nothing
    /// the player keeps it when the item is replaced so a duck or fade outlives the next track
    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        let Some(volume) = player_volume(volume) else {
            return;
        };
        let applied = match &mut self.engine {
            Ok(engine) => engine.set_volume(volume),
            Err(_) => Ok(()),
        };
        match applied {
            Ok(()) => self.volume = volume,
            Err(error) => tracing::debug!(target: "riseonly", %error, "media player keeps its volume"),
        }
        cx.notify();
    }

    pub fn action_at_item_end(&self) -> AVPlayerActionAtItemEnd {
        self.action_at_item_end
    }

    pub fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd, cx: &mut Context<Self>) {
        let applied = match &mut self.engine {
            Ok(engine) => engine.set_action_at_item_end(action),
            Err(_) => Ok(()),
        };
        match applied {
            Ok(()) => self.action_at_item_end = action,
            Err(error) => tracing::debug!(target: "riseonly", %error, "media player keeps its end action"),
        }
        cx.notify();
    }

    pub fn automatically_waits_to_minimize_stalling(&self) -> bool {
        self.automatically_waits_to_minimize_stalling
    }

    pub fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool, cx: &mut Context<Self>) {
        let applied = match &mut self.engine {
            Ok(engine) => engine.set_automatically_waits_to_minimize_stalling(waits),
            Err(_) => Ok(()),
        };
        match applied {
            Ok(()) => self.automatically_waits_to_minimize_stalling = waits,
            Err(error) => tracing::debug!(target: "riseonly", %error, "media player keeps its stalling policy"),
        }
        cx.notify();
    }

    /// paused at rate zero and waiting while there is no item or it failed
    /// otherwise what the engine reports
    pub fn time_control_status(&self) -> AVPlayerTimeControlStatus {
        if self.rate == 0.0 {
            return AVPlayerTimeControlStatus::Paused;
        }
        if self.item.is_none() || self.item_error.is_some() || self.item_is_checking {
            return AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate;
        }
        match &self.engine {
            Ok(engine) => engine.time_control_status().unwrap_or(AVPlayerTimeControlStatus::Playing),
            Err(_) => AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate,
        }
    }

    /// unknown without an item and failed when the host or item couldnt open
    /// otherwise what the engine reports, ready to play once a duration is known
    pub fn current_item_status(&self) -> AVPlayerItemStatus {
        if self.item.is_none() {
            return AVPlayerItemStatus::Unknown;
        }
        if self.item_error.is_some() {
            return AVPlayerItemStatus::Failed;
        }
        if self.item_is_checking {
            return AVPlayerItemStatus::Unknown;
        }
        match &self.engine {
            Ok(engine) => engine.item_status().unwrap_or_else(|| {
                if engine.current_item_duration().is_some() {
                    AVPlayerItemStatus::ReadyToPlay
                } else {
                    AVPlayerItemStatus::Unknown
                }
            }),
            Err(_) => AVPlayerItemStatus::Failed,
        }
    }

    /// loaded time ranges, empty without an item
    pub fn loaded_time_ranges(&self) -> Vec<CMTimeRange> {
        if self.item.is_none() {
            return Vec::new();
        }
        self.engine.as_ref().map(|engine| engine.loaded_time_ranges()).unwrap_or_default()
    }

    /// presentation size, zero while unknown
    pub fn presentation_size(&self) -> Size<f32> {
        let unknown = size(0.0, 0.0);
        if self.item.is_none() {
            return unknown;
        }
        self.engine.as_ref().ok().and_then(|engine| engine.presentation_size()).unwrap_or(unknown)
    }

    /// copy of the newest picture at the current host time from the video output on the current item
    /// none without an item or before a picture or when none came since the last copy
    /// player keeps playing, convert the copy off the main thread
    pub fn copy_pixel_buffer_for_current_time(&mut self) -> Option<AVPlayerPixelBuffer> {
        if self.item.is_none() {
            return None;
        }
        self.engine.as_mut().ok().and_then(|engine| engine.copy_pixel_buffer())
    }

    /// true once a picture of the current item is decoded, false without an item or with a failed one
    /// an observer of this entity is notified when it turns true
    pub fn is_ready_for_display(&self) -> bool {
        if self.item.is_none() || self.current_item_status() == AVPlayerItemStatus::Failed {
            return false;
        }
        match &self.engine {
            Ok(engine) => engine
                .is_ready_for_display()
                .unwrap_or_else(|| self.current_item_status() == AVPlayerItemStatus::ReadyToPlay),
            Err(_) => false,
        }
    }

    /// seeks in seconds, completion gets finished on the main thread outside this entity update
    /// a seek that cant start (no item or failed item or bad target) completes with false
    /// a new seek completes the one before it with false
    pub fn seek(
        &mut self,
        to: f64,
        tolerance_before: f64,
        tolerance_after: f64,
        completion: Option<Box<dyn FnOnce(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        self.finish_pending_seek(false, cx);
        self.seek_id = self.seek_id.wrapping_add(1);
        let id = self.seek_id;
        let window = SeekWindow::of(to, tolerance_before, tolerance_after);
        self.deferred_seek = None;
        let started = match (&mut self.engine, &self.item, &self.item_error, window) {
            (Ok(_), Some(_), None, Some(window)) if self.item_is_checking => {
                self.deferred_seek = Some(window);
                true
            }
            (Ok(engine), Some(_), None, Some(window)) => {
                engine.seek(window, SeekSink::new(id, self.seeks.clone()));
                true
            }
            _ => false,
        };
        match completion {
            Some(completion) if started => self.pending_seek = Some((id, completion)),
            Some(completion) => cx.defer(move |cx| completion(false, cx)),
            None => {}
        }
        self.resume_observation(cx);
        cx.notify();
    }

    fn seek_did_finish(&mut self, id: u64, cx: &mut Context<Self>) -> Option<Box<dyn FnOnce(bool, &mut App)>> {
        if self.pending_seek.as_ref().is_none_or(|(pending, _)| *pending != id) {
            return None;
        }
        cx.notify();
        self.pending_seek.take().map(|(_, completion)| completion)
    }

    fn finish_pending_seek(&mut self, finished: bool, cx: &mut Context<Self>) {
        if let Some((_, completion)) = self.pending_seek.take() {
            cx.defer(move |cx| completion(finished, cx));
        }
    }

    pub fn play_immediately(&mut self, rate: f32, cx: &mut Context<Self>) {
        self.rate = rate;
        if let Ok(engine) = &mut self.engine {
            engine.play_immediately(rate);
        }
        self.resume_observation(cx);
        cx.notify();
    }

    pub fn set_rate(&mut self, rate: f32, cx: &mut Context<Self>) {
        self.rate = rate;
        if let Ok(engine) = &mut self.engine {
            engine.set_rate(rate);
        }
        self.resume_observation(cx);
        cx.notify();
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.rate = 0.0;
        if let Ok(engine) = &mut self.engine {
            engine.pause();
        }
        cx.notify();
    }

    /// current time in seconds, none without an item or on a host without a player
    pub fn current_time(&self) -> Option<f64> {
        self.item.as_ref()?;
        self.engine.as_ref().ok().and_then(|engine| engine.current_time())
    }

    /// current item duration in seconds, none while unknown
    pub fn current_item_duration(&self) -> Option<f64> {
        self.item.as_ref()?;
        self.engine.as_ref().ok().and_then(|engine| engine.current_item_duration())
    }

    /// block gets the current time in seconds on the main thread every interval while time moves
    /// it runs outside the player update so it may read or update this entity
    pub fn add_periodic_time_observer(
        &mut self,
        interval: Duration,
        block: Box<dyn Fn(f64, &mut App)>,
        cx: &mut Context<Self>,
    ) -> AVPlayerTimeObserver {
        let ticks = cx.spawn(async move |this, cx| {
            let mut last = None;
            loop {
                cx.background_executor().timer(interval).await;
                let Ok(now) = this.update(cx, |player, _| player.current_time()) else {
                    break;
                };
                if !time_observer_fires(last, now) {
                    continue;
                }
                last = now;
                if let Some(seconds) = now {
                    cx.update(|cx| block(seconds, cx));
                }
            }
        });
        AVPlayerTimeObserver { _ticks: ticks }
    }

    pub fn remove_time_observer(&mut self, observer: AVPlayerTimeObserver) {
        drop(observer);
    }

    fn item_did_play_to_end(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        let Some(item) = &self.item else {
            return;
        };
        if self.action_at_item_end != AVPlayerActionAtItemEnd::None {
            self.rate = 0.0;
        }
        let asset_url = item.url().clone();
        cx.emit(AVPlayerItemNotification::DidPlayToEndTime { asset_url });
        cx.notify();
    }

    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
        self.item.as_ref()?;
        self.engine.as_mut().ok().and_then(|engine| engine.frame(fit))
    }

    fn swap_key(&self) -> Option<u64> {
        self.item.as_ref()?;
        self.engine.as_ref().ok().and_then(|engine| engine.swap_key())
    }

    fn take_swapped_picture(&mut self) -> Option<SurfaceSource> {
        self.item.as_ref()?;
        self.engine.as_mut().ok().and_then(|engine| engine.take_swapped_picture())
    }
}

/// player a layer draws, the silent looping clip or the media player
#[derive(Clone)]
enum LayerPlayer {
    Queue(Entity<AVQueuePlayer>),
    Player(Entity<AVPlayer>),
}

/// layer player as the swap pull holds it, without keeping the player alive
#[derive(Clone)]
enum WeakLayerPlayer {
    Queue(WeakEntity<AVQueuePlayer>),
    Player(WeakEntity<AVPlayer>),
}

impl LayerPlayer {
    fn downgrade(&self) -> WeakLayerPlayer {
        match self {
            Self::Queue(player) => WeakLayerPlayer::Queue(player.downgrade()),
            Self::Player(player) => WeakLayerPlayer::Player(player.downgrade()),
        }
    }
}

impl WeakLayerPlayer {
    fn take_swapped_picture(&self, cx: &mut App) -> Option<Option<SurfaceSource>> {
        match self {
            Self::Queue(player) => player
                .update(cx, |player, _| {
                    (player.is_playing && player.error().is_none()).then(|| player.take_swapped_picture())
                })
                .ok()
                .flatten(),
            Self::Player(player) => player
                .update(cx, |player, _| {
                    (player.rate != 0.0 && player.error().is_none()).then(|| player.take_swapped_picture())
                })
                .ok()
                .flatten(),
        }
    }
}

thread_local! {
    static SWAP_PULLS: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

fn pull_swapped_pictures(player: WeakLayerPlayer, swap_key: u64, window: &Window) {
    if SWAP_PULLS.with_borrow_mut(|pulls| pulls.insert(swap_key)) {
        schedule_swap_pull(player, swap_key, window);
    }
}

fn schedule_swap_pull(player: WeakLayerPlayer, swap_key: u64, window: &Window) {
    window.on_next_frame(move |window, cx| {
        let keeps_pulling = match player.take_swapped_picture(cx) {
            None => false,
            Some(None) => true,
            Some(Some(picture)) => window.swap_surface(swap_key, picture),
        };
        if keeps_pulling {
            schedule_swap_pull(player, swap_key, window);
        } else {
            SWAP_PULLS.with_borrow_mut(|pulls| pulls.remove(&swap_key));
        }
    });
}

/// draws the player current picture into its own bounds with the gravity
/// one persistent system surface per stream and never a per frame image upload
#[derive(IntoElement)]
pub struct AVPlayerLayer {
    player: Option<LayerPlayer>,
    video_gravity: AVLayerVideoGravity,
}

impl AVPlayerLayer {
    pub fn new(player: Option<Entity<AVQueuePlayer>>) -> Self {
        Self {
            player: player.map(LayerPlayer::Queue),
            video_gravity: AVLayerVideoGravity::ResizeAspect,
        }
    }

    /// layer for a player
    /// a layer is drawn anew every frame so no reattachment is needed
    pub fn for_player(player: Option<Entity<AVPlayer>>) -> Self {
        Self {
            player: player.map(LayerPlayer::Player),
            video_gravity: AVLayerVideoGravity::ResizeAspect,
        }
    }

    pub fn video_gravity(mut self, gravity: AVLayerVideoGravity) -> Self {
        self.video_gravity = gravity;
        self
    }
}

impl RenderOnce for AVPlayerLayer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fit = self.video_gravity.object_fit();
        let (frame, playing, swap_key) = match &self.player {
            Some(LayerPlayer::Queue(player)) => player.update(cx, |player, _| {
                (player.frame(fit), player.is_playing && player.error().is_none(), player.swap_key())
            }),
            Some(LayerPlayer::Player(player)) => player.update(cx, |player, _| {
                (player.frame(fit), player.rate != 0.0 && player.error().is_none(), player.swap_key())
            }),
            None => (None, false, None),
        };
        let pull = swap_key.zip(self.player.as_ref().map(LayerPlayer::downgrade));
        let layer = div().size_full().overflow_hidden().children(frame);
        if !playing {
            return layer;
        }
        layer.child(
            canvas(
                move |bounds, window: &mut Window, _: &mut App| {
                    let clip = window.content_mask().bounds;
                    if bounds.intersects(&clip) {
                        let visible = bounds.intersect(&clip);
                        if visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO {
                            match pull.clone() {
                                Some((swap_key, player)) => pull_swapped_pictures(player, swap_key, window),
                                None => window.request_animation_frame(),
                            }
                        }
                    }
                },
                |_, (), _: &mut Window, _: &mut App| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}

use std::future::Future;
use std::marker::PhantomData;
use std::num::NonZeroIsize;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use parking_lot::Mutex;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use rise_core::foundation::foundation_date::RiseClock;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Debug, Error)]
pub enum MediaKeysError {
    #[error("no media transport API is present on this system")]
    Unavailable,
    #[error("the now-playing card needs a bundled application with a bundle identifier")]
    NotBundled,
    #[error("{0} is not a valid D-Bus bus name")]
    InvalidBusName(String),
    #[error("media transport: {0}")]
    Backend(String),
}

/// how the app names itself to the os
/// only mpris reads it but every host validates it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MediaIdentity {
    pub identity: String,
    pub bus_suffix: String,
}

pub const MPRIS_OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

const MPRIS_BUS_PREFIX: &str = "org.mpris.MediaPlayer2";

const MAX_BUS_NAME: usize = 255;

impl MediaIdentity {
    pub fn new(identity: impl Into<String>, bus_suffix: impl Into<String>) -> Self {
        Self {
            identity: identity.into(),
            bus_suffix: bus_suffix.into(),
        }
    }

    pub fn bus_name(&self) -> String {
        format!("{MPRIS_BUS_PREFIX}.{}", self.bus_suffix)
    }

    /// bus name element may hold letters digits underscore and hyphen, no digit first
    /// hyphen is fine here but illegal in an interface name
    pub fn validate(&self) -> Result<(), MediaKeysError> {
        let name = self.bus_name();
        let invalid = || MediaKeysError::InvalidBusName(name.clone());

        if name.len() > MAX_BUS_NAME {
            return Err(invalid());
        }

        for element in name.split('.') {
            let mut characters = element.chars();
            let Some(first) = characters.next() else {
                return Err(invalid());
            };
            if !legal_in_bus_name(first)
                || first.is_ascii_digit()
                || !characters.all(legal_in_bus_name)
            {
                return Err(invalid());
            }
        }

        Ok(())
    }
}

fn legal_in_bus_name(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

/// where the cover art lives, a reference and never bytes
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Artwork {
    File(PathBuf),
    Url(String),
}

impl Artwork {
    pub fn as_uri(&self) -> String {
        match self {
            Self::File(path) => format!("file://{}", path.display()),
            Self::Url(url) => url.clone(),
        }
    }
}

/// what the card shows, no os types and no gpui types since it crosses the seam
#[derive(Clone, PartialEq, Debug)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    pub elapsed: Duration,
    pub rate: f64,
    pub artwork: Option<Artwork>,
}

impl NowPlaying {
    pub fn new(title: impl Into<String>, artist: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            duration: Duration::ZERO,
            elapsed: Duration::ZERO,
            rate: 1.0,
            artwork: None,
        }
    }

    pub fn with_album(mut self, album: impl Into<String>) -> Self {
        self.album = album.into();
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    pub fn with_artwork(mut self, artwork: Artwork) -> Self {
        self.artwork = Some(artwork);
        self
    }

    pub fn at(mut self, elapsed: Duration, rate: f64) -> Self {
        self.elapsed = elapsed;
        self.rate = rate;
        self
    }

    /// position is ignored so an update isnt taken for a track change
    pub fn is_same_track(&self, other: &Self) -> bool {
        self.title == other.title
            && self.artist == other.artist
            && self.album == other.album
            && self.duration == other.duration
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Interrupted,
}

/// playback state as the platform api wants it, mpris uses a string enum
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawPlaybackState {
    Integer(i64),
    Text(&'static str),
}

impl RawPlaybackState {
    pub fn integer(self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(value),
            Self::Text(_) => None,
        }
    }

    pub fn text(self) -> Option<&'static str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Integer(_) => None,
        }
    }
}

impl PlaybackState {
    pub const ALL: [Self; 4] = [
        Self::Playing,
        Self::Paused,
        Self::Stopped,
        Self::Interrupted,
    ];

    pub fn is_playing(self) -> bool {
        self == Self::Playing
    }

    /// the int scales differ, mac Playing is 1 and windows is 3 (1 is Changing there)
    pub fn raw(self, host: HostOs) -> RawPlaybackState {
        match host {
            HostOs::MacOs => RawPlaybackState::Integer(match self {
                Self::Playing => 1,
                Self::Paused => 2,
                Self::Stopped => 3,
                Self::Interrupted => 4,
            }),
            HostOs::Windows => RawPlaybackState::Integer(match self {
                Self::Playing => 3,
                Self::Paused | Self::Interrupted => 4,
                Self::Stopped => 2,
            }),
            HostOs::Linux => RawPlaybackState::Text(match self {
                Self::Playing => "Playing",
                Self::Paused | Self::Interrupted => "Paused",
                Self::Stopped => "Stopped",
            }),
        }
    }

    /// rate to give the os, 0 for anything not playing or the card timer keeps running
    pub fn effective_rate(self, nominal: f64) -> f64 {
        if self.is_playing() { nominal } else { 0.0 }
    }
}

/// transport commands the app answers
/// no stop, and seeking is absolute only
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek,
}

pub const MPRIS_STOP_COMMAND: MediaCommand = MediaCommand::Pause;

impl MediaCommand {
    pub const ALL: [Self; 6] = [
        Self::Play,
        Self::Pause,
        Self::Toggle,
        Self::Next,
        Self::Previous,
        Self::Seek,
    ];

    /// true when the platform transport api can deliver this command
    /// windows has no toggle button so waiting for Toggle there waits forever
    pub fn is_deliverable_on(self, host: HostOs) -> bool {
        match host {
            HostOs::MacOs | HostOs::Linux => true,
            HostOs::Windows => self != Self::Toggle,
        }
    }

    /// member name on the mpris player interface, mpris Seek is relative and SetPosition absolute
    pub fn mpris_member(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Toggle => "PlayPause",
            Self::Next => "Next",
            Self::Previous => "Previous",
            Self::Seek => "SetPosition",
        }
    }

    /// what a play pause key becomes on this host, windows has no toggle so the app resolves it
    pub fn toggle_for(host: HostOs, state: PlaybackState) -> Self {
        if Self::Toggle.is_deliverable_on(host) {
            Self::Toggle
        } else if state.is_playing() {
            Self::Pause
        } else {
            Self::Play
        }
    }
}

/// a command as it arrived with whatever the os attached
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MediaCommandEvent {
    pub command: MediaCommand,
    pub position: Option<Duration>,
}

impl MediaCommandEvent {
    pub fn simple(command: MediaCommand) -> Self {
        Self {
            command,
            position: None,
        }
    }

    pub fn seek(position: Duration) -> Self {
        Self {
            command: MediaCommand::Seek,
            position: Some(position),
        }
    }
}

/// what the app did with a command, macos turns it into a handler status
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandOutcome {
    Handled,
    Rejected,
}

/// shared and not boxed since the macos binding needs a copy in a process wide slot
pub type CommandHandler = Arc<dyn Fn(MediaCommandEvent) -> CommandOutcome + Send + Sync + 'static>;

/// brings the app window forward for mpris Raise
/// runs on the service task and not the main thread, so the app hops to its window itself
pub type RaiseHandler = Arc<dyn Fn() + Send + Sync + 'static>;

pub const POSITION_EPSILON: Duration = Duration::from_millis(750);

const RATE_EPSILON: f64 = 0.001;

/// last position and rate pair given to the os and when
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PublishedPosition {
    pub elapsed: Duration,
    pub rate: f64,
    pub at: Duration,
}

impl PublishedPosition {
    /// what the os thinks the position is now, having heard nothing since
    pub fn extrapolated(&self, now: Duration) -> Duration {
        let advanced = now.saturating_sub(self.at).as_secs_f64() * self.rate;
        if advanced >= 0.0 {
            self.elapsed + Duration::from_secs_f64(advanced)
        } else {
            self.elapsed
                .saturating_sub(Duration::from_secs_f64(-advanced))
        }
    }
}

pub fn should_republish_position(
    last: Option<PublishedPosition>,
    now: Duration,
    elapsed: Duration,
    rate: f64,
    epsilon: Duration,
) -> bool {
    let Some(last) = last else {
        return true;
    };

    if (rate - last.rate).abs() > RATE_EPSILON {
        return true;
    }

    let expected = last.extrapolated(now);

    expected.abs_diff(elapsed) > epsilon
}

/// republish check with the bookkeeping attached
#[derive(Clone, Copy, Debug)]
pub struct PositionPublisher {
    last: Option<PublishedPosition>,
    epsilon: Duration,
}

impl Default for PositionPublisher {
    fn default() -> Self {
        Self::new(POSITION_EPSILON)
    }
}

impl PositionPublisher {
    pub fn new(epsilon: Duration) -> Self {
        Self {
            last: None,
            epsilon,
        }
    }

    pub fn should_publish(&mut self, now: Duration, elapsed: Duration, rate: f64) -> bool {
        if !should_republish_position(self.last, now, elapsed, rate, self.epsilon) {
            return false;
        }

        self.last = Some(PublishedPosition {
            elapsed,
            rate,
            at: now,
        });
        true
    }

    /// drops the extrapolation base, needed on any track or state change
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    pub fn last(&self) -> Option<PublishedPosition> {
        self.last
    }
}

/// true when the reported position breaks from what the os extrapolates
/// a scrub or seek or jump back, not steady playback or small drift
pub fn is_seek(base: Option<PublishedPosition>, now: Duration, elapsed: Duration, epsilon: Duration) -> bool {
    base.is_some_and(|base| base.extrapolated(now).abs_diff(elapsed) > epsilon)
}

/// one entry of the mpris Metadata map before it becomes a zvariant value
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MprisMetadata {
    Text(String),
    TextList(Vec<String>),
    Path(String),
    Micros(i64),
}

pub(crate) fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

/// object path published as mpris trackid for a track ordinal
/// the staleness check of SetPosition compares against it
pub fn track_path(ordinal: u64) -> String {
    format!("/net/riseonly/track/{ordinal}")
}

/// the mpris Metadata map for a track
/// ordinal must differ between tracks or clients read the next one as a seek
pub fn mpris_metadata(track: &NowPlaying, ordinal: u64) -> Vec<(&'static str, MprisMetadata)> {
    let mut entries = vec![("mpris:trackid", MprisMetadata::Path(track_path(ordinal)))];

    if track.duration > Duration::ZERO {
        entries.push((
            "mpris:length",
            MprisMetadata::Micros(micros(track.duration)),
        ));
    }
    if !track.title.is_empty() {
        entries.push(("xesam:title", MprisMetadata::Text(track.title.clone())));
    }
    if !track.artist.is_empty() {
        entries.push((
            "xesam:artist",
            MprisMetadata::TextList(vec![track.artist.clone()]),
        ));
    }
    if !track.album.is_empty() {
        entries.push(("xesam:album", MprisMetadata::Text(track.album.clone())));
    }
    if let Some(artwork) = &track.artwork {
        entries.push(("mpris:artUrl", MprisMetadata::Text(artwork.as_uri())));
    }

    entries
}

pub fn transport_support(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Linux | HostOs::Windows => PlatformSupport::Performed,
    }
}

/// true when the transport hangs off an app window and not the process
/// windows has no process wide transport controls for desktop apps
pub fn transport_needs_window(host: HostOs) -> bool {
    host == HostOs::Windows
}

/// duration as winrt ticks of 100 ns, saturates instead of wrapping
pub fn timespan_ticks(duration: Duration) -> i64 {
    i64::try_from(duration.as_nanos() / 100).unwrap_or(i64::MAX)
}

/// winrt ticks back to a duration, negative means before the start so it seeks to zero
pub fn duration_from_ticks(ticks: i64) -> Duration {
    let ticks = u64::try_from(ticks).unwrap_or(0);
    Duration::from_nanos(ticks.saturating_mul(100))
}

/// true when the now playing card refuses to show for an unbundled process
/// mac shows nothing and no error, so install fails loudly instead
pub fn card_requires_bundle_identity(host: HostOs) -> bool {
    host == HostOs::MacOs
}

#[derive(Clone, PartialEq, Debug)]
pub struct MediaSnapshot {
    pub now: Option<NowPlaying>,
    pub state: PlaybackState,
    pub generation: u64,
    pub track: u64,
    pub seeks: u64,
}

struct Shared {
    now: Option<NowPlaying>,
    state: PlaybackState,
    generation: u64,
    track: u64,
    seeks: u64,
    attached: bool,
    window: Option<NonZeroIsize>,
    raise: Option<RaiseHandler>,
    waker: Option<Waker>,
}

/// state both bindings publish from, the only thing the service task and app share
pub struct MediaShare {
    handler: CommandHandler,
    inner: Mutex<Shared>,
}

impl MediaShare {
    pub fn new(handler: CommandHandler) -> Self {
        Self {
            handler,
            inner: Mutex::new(Shared {
                now: None,
                state: PlaybackState::Stopped,
                generation: 0,
                track: 0,
                seeks: 0,
                attached: false,
                window: None,
                raise: None,
                waker: None,
            }),
        }
    }

    pub fn handler(&self) -> CommandHandler {
        Arc::clone(&self.handler)
    }

    pub fn deliver(&self, event: MediaCommandEvent) -> CommandOutcome {
        (self.handler)(event)
    }

    pub fn generation(&self) -> u64 {
        self.inner.lock().generation
    }

    /// true when this mpris TrackId still names the loaded track
    /// seeking a changed track scrubs the wrong song, no track loaded matches nothing
    pub fn is_current_track(&self, path: &str) -> bool {
        let inner = self.inner.lock();
        inner.now.is_some() && path == track_path(inner.track)
    }

    pub fn snapshot(&self) -> MediaSnapshot {
        let inner = self.inner.lock();
        MediaSnapshot {
            now: inner.now.clone(),
            state: inner.state,
            generation: inner.generation,
            track: inner.track,
            seeks: inner.seeks,
        }
    }

    pub fn state(&self) -> PlaybackState {
        self.inner.lock().state
    }

    /// set before or after the service runs, mpris reads CanRaise from it
    pub fn set_raise_handler(&self, handler: RaiseHandler) {
        let waker = {
            let mut inner = self.inner.lock();
            inner.raise = Some(handler);
            inner.generation += 1;
            inner.waker.take()
        };
        wake(waker);
    }

    pub fn can_raise(&self) -> bool {
        self.inner.lock().raise.is_some()
    }

    /// runs the raise handler outside the lock, false if the app gave none
    pub fn raise(&self) -> bool {
        let handler = self.inner.lock().raise.clone();
        let Some(handler) = handler else {
            return false;
        };
        handler();
        true
    }

    /// false until a binding claims the os resource, setters say Unsupported till then
    pub fn is_attached(&self) -> bool {
        self.inner.lock().attached
    }

    /// called by the arm that owns the os resource, on linux once the service has the bus name
    pub fn set_attached(&self, attached: bool) {
        let waker = {
            let mut inner = self.inner.lock();
            inner.attached = attached;
            inner.generation += 1;
            inner.waker.take()
        };
        wake(waker);
    }

    /// native handle of the window the transport hangs off, only a win32 hwnd is kept
    pub fn window(&self) -> Option<NonZeroIsize> {
        self.inner.lock().window
    }

    /// gives the service task the window to bind to, a different window rebinds
    pub fn set_window(&self, window: NonZeroIsize) {
        let waker = {
            let mut inner = self.inner.lock();
            if inner.window == Some(window) {
                return;
            }
            inner.window = Some(window);
            inner.generation += 1;
            inner.waker.take()
        };
        wake(waker);
    }

    pub fn is_new_track(&self, next: &NowPlaying) -> bool {
        let inner = self.inner.lock();
        inner
            .now
            .as_ref()
            .is_none_or(|current| !current.is_same_track(next))
    }

    /// resolves next time the published state differs from seen
    pub fn changed(&self, seen: u64) -> Changed<'_> {
        Changed { share: self, seen }
    }

    fn publish(&self, now: Option<NowPlaying>, state: PlaybackState) {
        let waker = {
            let mut inner = self.inner.lock();
            let new_track = match (&inner.now, &now) {
                (Some(current), Some(next)) => !current.is_same_track(next),
                _ => now.is_some(),
            };
            if new_track {
                inner.track += 1;
            }
            inner.now = now;
            inner.state = state;
            inner.generation += 1;
            inner.waker.take()
        };
        wake(waker);
    }

    fn set_state(&self, state: PlaybackState) -> Option<NowPlaying> {
        let (track, waker) = {
            let mut inner = self.inner.lock();
            inner.state = state;
            inner.generation += 1;
            let track = inner.now.clone();
            (track, inner.waker.take())
        };
        wake(waker);
        track
    }

    fn mark_seeked(&self) {
        let waker = {
            let mut inner = self.inner.lock();
            inner.seeks += 1;
            inner.generation += 1;
            inner.waker.take()
        };
        wake(waker);
    }

    fn advance(&self, elapsed: Duration, rate: f64) -> Option<(NowPlaying, PlaybackState)> {
        let (updated, state, waker) = {
            let mut inner = self.inner.lock();
            let state = inner.state;
            let track = inner.now.as_mut()?;
            track.elapsed = elapsed;
            track.rate = rate;
            let updated = track.clone();
            inner.generation += 1;
            (updated, state, inner.waker.take())
        };
        wake(waker);
        Some((updated, state))
    }
}

fn wake(waker: Option<Waker>) {
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// resolves with the new generation once the shared state moves past seen
pub struct Changed<'a> {
    share: &'a MediaShare,
    seen: u64,
}

impl Future for Changed<'_> {
    type Output = u64;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<u64> {
        let mut inner = self.share.inner.lock();
        if inner.generation == self.seen {
            inner.waker = Some(context.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(inner.generation)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionUpdate {
    Published,
    Skipped,
    Unsupported,
}

/// transport keys and the now playing card
pub struct MediaKeys {
    share: Arc<MediaShare>,
    identity: MediaIdentity,
    publisher: PositionPublisher,
    seek_base: Option<PublishedPosition>,
    transport: super::os::Transport,
    _main_thread_only: PhantomData<*const ()>,
}

impl MediaKeys {
    pub fn install(
        identity: &MediaIdentity,
        handler: CommandHandler,
    ) -> Result<Self, MediaKeysError> {
        identity.validate()?;

        let share = Arc::new(MediaShare::new(handler));

        let transport = super::os::Transport::install(share.handler())?;

        Ok(Self {
            share,
            identity: identity.clone(),
            publisher: PositionPublisher::default(),
            seek_base: None,
            transport,
            _main_thread_only: PhantomData,
        })
    }

    pub fn published_snapshot(&self) -> MediaSnapshot {
        self.share.snapshot()
    }

    /// the part of the seam that needs an executor
    /// mac has none, mpris is a dbus service so the app drives a future itself
    pub fn service(&self) -> MediaService {
        MediaService {
            share: Arc::clone(&self.share),
            identity: self.identity.clone(),
        }
    }

    pub fn share(&self) -> &Arc<MediaShare> {
        &self.share
    }

    pub fn bind_window(&self, window: &gpui::Window) -> PlatformSupport {
        if !transport_needs_window(HostOs::current()) {
            return self.transport();
        }
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return PlatformSupport::Unsupported;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return PlatformSupport::Unsupported;
        };
        self.share.set_window(win32.hwnd);
        self.transport()
    }

    pub fn on_raise(&self, handler: RaiseHandler) {
        self.share.set_raise_handler(handler);
    }

    pub fn set_now_playing(&mut self, track: &NowPlaying, state: PlaybackState) -> PlatformSupport {
        let now = RiseClock::monotonic();
        let new_track = self.share.is_new_track(track);
        if new_track {
            self.publisher.invalidate();
        }
        let jumped = !new_track && is_seek(self.seek_base, now, track.elapsed, POSITION_EPSILON);
        self.seek_base = Some(PublishedPosition {
            elapsed: track.elapsed,
            rate: state.effective_rate(track.rate),
            at: now,
        });
        self.share.publish(Some(track.clone()), state);
        if jumped {
            self.share.mark_seeked();
        }
        self.publish_to_os(track, state);
        self.transport()
    }

    pub fn set_playback_state(&mut self, state: PlaybackState) -> PlatformSupport {
        self.publisher.invalidate();
        let now = RiseClock::monotonic();
        if let Some(track) = self.share.set_state(state) {
            self.seek_base = self.seek_base.map(|base| PublishedPosition {
                elapsed: base.extrapolated(now),
                rate: state.effective_rate(track.rate),
                at: now,
            });
            self.publish_to_os(&track, state);
        }
        self.transport()
    }

    pub fn set_position(&mut self, elapsed: Duration, rate: f64) -> PositionUpdate {
        let now = RiseClock::monotonic();
        let jumped = is_seek(self.seek_base, now, elapsed, POSITION_EPSILON);
        if self.seek_base.is_some() {
            self.seek_base = Some(PublishedPosition {
                elapsed,
                rate: self.share.state().effective_rate(rate),
                at: now,
            });
        }
        if jumped {
            self.publisher.invalidate();
        }
        if !self.publisher.should_publish(now, elapsed, rate) {
            return PositionUpdate::Skipped;
        }

        let Some((track, state)) = self.share.advance(elapsed, rate) else {
            return PositionUpdate::Skipped;
        };
        if jumped {
            self.share.mark_seeked();
        }
        self.publish_to_os(&track, state);

        match self.transport() {
            PlatformSupport::Performed => PositionUpdate::Published,
            PlatformSupport::Unsupported => PositionUpdate::Unsupported,
        }
    }

    pub fn clear(&mut self) -> PlatformSupport {
        self.publisher.invalidate();
        self.seek_base = None;
        self.share.publish(None, PlaybackState::Stopped);
        self.clear_os();
        self.transport()
    }

    fn transport(&self) -> PlatformSupport {
        match HostOs::current() {
            HostOs::MacOs => PlatformSupport::Performed,
            HostOs::Linux | HostOs::Windows if self.share.is_attached() => {
                PlatformSupport::Performed
            }
            HostOs::Linux | HostOs::Windows => PlatformSupport::Unsupported,
        }
    }

    fn publish_to_os(&self, track: &NowPlaying, state: PlaybackState) {
        self.transport.publish(track, state);
    }

    fn clear_os(&self) {
        self.transport.clear();
    }
}

/// drives the parts of the seam that must run and not just be called
pub struct MediaService {
    share: Arc<MediaShare>,
    identity: MediaIdentity,
}

impl MediaService {
    /// runs until the connection drops, returns at once on mac
    /// on windows it waits for bind_window then serves the card until a winrt call fails
    pub async fn run(self) -> Result<(), MediaKeysError> {
        super::os::serve(self.share, self.identity).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Wake;

    fn accepting() -> CommandHandler {
        Arc::new(|_| CommandOutcome::Handled)
    }

    fn recording() -> (CommandHandler, Arc<Mutex<Vec<MediaCommandEvent>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        (
            Arc::new(move |event| {
                sink.lock().push(event);
                CommandOutcome::Handled
            }),
            log,
        )
    }

    fn track() -> NowPlaying {
        NowPlaying::new("Nightcall", "Kavinsky")
            .with_album("OutRun")
            .with_duration(Duration::from_secs(258))
    }

    #[test]
    fn a_seek_aimed_at_the_previous_track_is_ignored() {
        let share = MediaShare::new(accepting());
        share.publish(Some(track()), PlaybackState::Playing);
        let first = share.snapshot().track;

        assert!(share.is_current_track(&track_path(first)));

        share.publish(
            Some(NowPlaying::new("Odd Look", "Kavinsky")),
            PlaybackState::Playing,
        );

        assert!(
            !share.is_current_track(&track_path(first)),
            "the path of the track that was replaced must no longer match"
        );
        assert!(share.is_current_track(&track_path(share.snapshot().track)));
    }

    #[test]
    fn no_track_id_matches_while_nothing_is_loaded() {
        let share = MediaShare::new(accepting());

        assert!(!share.is_current_track(&track_path(0)));
        assert!(!share.is_current_track("/net/riseonly/track/1"));
    }

    #[test]
    fn a_stream_reports_no_duration_to_either_host() {
        let stream = NowPlaying::new("Live", "Riseonly Radio");
        assert_eq!(stream.duration, Duration::ZERO);

        let keys: Vec<&str> = mpris_metadata(&stream, 1)
            .iter()
            .map(|(key, _)| *key)
            .collect();

        assert!(!keys.contains(&"mpris:length"));
    }

    struct CountingWaker(AtomicUsize);

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn every_state_maps_to_something_on_every_host() {
        for host in HostOs::ALL {
            for state in PlaybackState::ALL {
                let raw = state.raw(host);
                match host {
                    HostOs::Linux => assert!(raw.text().is_some(), "{state:?} on {host:?}"),
                    _ => assert!(raw.integer().is_some(), "{state:?} on {host:?}"),
                }
            }
        }
    }

    #[test]
    fn macos_and_windows_do_not_agree_on_the_number_for_playing() {
        assert_eq!(
            PlaybackState::Playing.raw(HostOs::MacOs).integer(),
            Some(1),
            "MPNowPlayingPlaybackStatePlaying"
        );
        assert_eq!(
            PlaybackState::Playing.raw(HostOs::Windows).integer(),
            Some(3),
            "MediaPlaybackStatus.Playing; macOS's 1 means Changing here"
        );
    }

    #[test]
    fn windows_paused_and_stopped_use_the_media_playback_status_numbers() {
        assert_eq!(PlaybackState::Paused.raw(HostOs::Windows).integer(), Some(4));
        assert_eq!(PlaybackState::Interrupted.raw(HostOs::Windows).integer(), Some(4));
        assert_eq!(PlaybackState::Stopped.raw(HostOs::Windows).integer(), Some(2));
    }

    #[test]
    fn a_timespan_counts_hundred_nanosecond_ticks() {
        assert_eq!(timespan_ticks(Duration::from_secs(1)), 10_000_000);
        assert_eq!(timespan_ticks(Duration::from_nanos(150)), 1);
        assert_eq!(timespan_ticks(Duration::MAX), i64::MAX);
        assert_eq!(duration_from_ticks(25_000_000), Duration::from_millis(2_500));
        assert_eq!(duration_from_ticks(-5), Duration::ZERO);
    }

    #[test]
    fn a_window_handed_twice_does_not_wake_the_service_again() {
        let share = MediaShare::new(accepting());
        let window = NonZeroIsize::new(0x1234).expect("non-zero");
        let before = share.generation();

        share.set_window(window);
        let bound = share.generation();
        assert_eq!(bound, before + 1);
        assert_eq!(share.window(), Some(window));

        share.set_window(window);
        assert_eq!(share.generation(), bound);
    }

    #[test]
    fn stopped_and_playing_are_never_confused_within_one_host() {
        for host in HostOs::ALL {
            assert_ne!(
                PlaybackState::Playing.raw(host),
                PlaybackState::Stopped.raw(host),
                "{host:?}"
            );
        }
    }

    #[test]
    fn an_interruption_degrades_to_paused_where_the_os_has_no_word_for_it() {
        assert_eq!(
            PlaybackState::Interrupted.raw(HostOs::MacOs).integer(),
            Some(4),
            "macOS has a real interrupted state and must keep it"
        );
        assert_eq!(
            PlaybackState::Interrupted.raw(HostOs::Windows),
            PlaybackState::Paused.raw(HostOs::Windows)
        );
        assert_eq!(
            PlaybackState::Interrupted.raw(HostOs::Linux),
            PlaybackState::Paused.raw(HostOs::Linux)
        );
    }

    #[test]
    fn mpris_reports_a_status_string_rather_than_a_number() {
        assert_eq!(
            PlaybackState::Playing.raw(HostOs::Linux).text(),
            Some("Playing")
        );
        assert_eq!(PlaybackState::Playing.raw(HostOs::Linux).integer(), None);
    }

    #[test]
    fn a_paused_track_reports_a_rate_of_zero_so_the_card_stops_counting() {
        assert_eq!(PlaybackState::Playing.effective_rate(1.5), 1.5);
        for state in [
            PlaybackState::Paused,
            PlaybackState::Stopped,
            PlaybackState::Interrupted,
        ] {
            assert_eq!(state.effective_rate(1.5), 0.0, "{state:?}");
        }
    }

    #[test]
    fn windows_cannot_deliver_a_toggle_so_the_app_must_resolve_it() {
        assert!(!MediaCommand::Toggle.is_deliverable_on(HostOs::Windows));

        assert_eq!(
            MediaCommand::toggle_for(HostOs::Windows, PlaybackState::Playing),
            MediaCommand::Pause
        );
        assert_eq!(
            MediaCommand::toggle_for(HostOs::Windows, PlaybackState::Paused),
            MediaCommand::Play
        );
    }

    #[test]
    fn macos_and_mpris_own_the_toggle_themselves() {
        for host in [HostOs::MacOs, HostOs::Linux] {
            for state in PlaybackState::ALL {
                assert_eq!(
                    MediaCommand::toggle_for(host, state),
                    MediaCommand::Toggle,
                    "{host:?} resolves the toggle at the moment of the keypress"
                );
            }
        }
    }

    #[test]
    fn every_command_except_the_toggle_reaches_every_host() {
        for command in MediaCommand::ALL {
            for host in HostOs::ALL {
                let expected = !(host == HostOs::Windows && command == MediaCommand::Toggle);
                assert_eq!(
                    command.is_deliverable_on(host),
                    expected,
                    "{command:?} on {host:?}"
                );
            }
        }
    }

    #[test]
    fn the_command_set_is_the_one_ios_enables_on_the_remote_command_center() {
        assert_eq!(
            MediaCommand::ALL,
            [
                MediaCommand::Play,
                MediaCommand::Pause,
                MediaCommand::Toggle,
                MediaCommand::Next,
                MediaCommand::Previous,
                MediaCommand::Seek,
            ]
        );
    }

    #[test]
    fn an_mpris_stop_pauses_and_keeps_the_queue() {
        assert_eq!(MPRIS_STOP_COMMAND, MediaCommand::Pause);
        assert!(
            !MediaCommand::ALL
                .iter()
                .any(|command| command.mpris_member() == "Stop"),
            "Stop must not be offered as a command of its own"
        );
        assert!(
            !MediaCommand::ALL
                .iter()
                .any(|command| command.mpris_member() == "Seek"),
            "the relative MPRIS Seek is disabled, as iOS disables seekForward and seekBackward"
        );
    }

    #[test]
    fn a_pause_key_resolves_against_the_state_on_windows_only() {
        for state in PlaybackState::ALL {
            let expected = if state.is_playing() {
                MediaCommand::Pause
            } else {
                MediaCommand::Play
            };
            assert_eq!(MediaCommand::toggle_for(HostOs::Windows, state), expected, "{state:?}");
        }
    }

    #[test]
    fn a_seek_event_carries_its_absolute_position() {
        let event = MediaCommandEvent::seek(Duration::from_secs(42));
        assert_eq!(event.command, MediaCommand::Seek);
        assert_eq!(event.position, Some(Duration::from_secs(42)));
        assert_eq!(MediaCommandEvent::simple(MediaCommand::Next).position, None);
    }

    #[test]
    fn absolute_seeking_maps_to_set_position_not_seek() {
        assert_eq!(MediaCommand::Seek.mpris_member(), "SetPosition");
        assert_eq!(MediaCommand::Toggle.mpris_member(), "PlayPause");

        let members: Vec<&str> = MediaCommand::ALL
            .iter()
            .map(|command| command.mpris_member())
            .collect();
        for member in &members {
            assert_eq!(
                members.iter().filter(|other| *other == member).count(),
                1,
                "{member} is claimed by two commands"
            );
        }
    }

    #[test]
    fn the_first_position_is_always_published() {
        let mut publisher = PositionPublisher::default();
        assert!(publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.0));
    }

    #[test]
    fn steady_playback_at_one_times_speed_never_republishes() {
        let mut publisher = PositionPublisher::default();
        let mut published = 0;

        for tick in 0..240u64 {
            let now = Duration::from_millis(tick * 250);
            let jitter = if tick % 2 == 0 { 18 } else { 0 };
            let elapsed = Duration::from_millis(tick * 250 + jitter);
            if publisher.should_publish(now, elapsed, 1.0) {
                published += 1;
            }
        }

        assert_eq!(
            published, 1,
            "the OS runs the same clock we do; confirming it costs an IPC round trip per tick"
        );
    }

    #[test]
    fn a_seek_republishes_immediately() {
        let mut publisher = PositionPublisher::default();
        assert!(publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.0));

        let now = Duration::from_secs(1);
        assert!(
            publisher.should_publish(now, Duration::from_secs(90), 1.0),
            "a jump of 89 seconds is not something the OS could have extrapolated"
        );
    }

    #[test]
    fn pausing_republishes_because_the_rate_changed() {
        let mut publisher = PositionPublisher::default();
        publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.0);

        let now = Duration::from_millis(250);
        assert!(
            publisher.should_publish(now, Duration::from_millis(250), 0.0),
            "the position matches, but a card that keeps counting while paused is wrong"
        );
    }

    #[test]
    fn a_faster_rate_republishes_at_the_same_position() {
        let mut publisher = PositionPublisher::default();
        publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.0);
        assert!(publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.25));
    }

    #[test]
    fn noise_in_the_reported_rate_does_not_republish() {
        let mut publisher = PositionPublisher::default();
        publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.0);
        assert!(!publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.0000005));
    }

    #[test]
    fn a_drifting_clock_republishes_once_it_passes_the_epsilon() {
        let mut publisher = PositionPublisher::default();
        let mut republished_at = None;

        for tick in 0..40u64 {
            let now = Duration::from_millis(tick * 500);
            let elapsed = Duration::from_millis((tick * 500) * 11 / 10);
            if publisher.should_publish(now, elapsed, 1.0) && tick > 0 {
                republished_at = Some(tick);
                break;
            }
        }

        assert_eq!(
            republished_at,
            Some(16),
            "0.1x drift crosses 750 ms at 7.5 s, which is tick 16 at 500 ms"
        );
    }

    #[test]
    fn invalidating_forces_the_next_position_out() {
        let mut publisher = PositionPublisher::default();
        let hundred = Duration::from_millis(100);
        let two_hundred = Duration::from_millis(200);

        publisher.should_publish(Duration::ZERO, Duration::ZERO, 1.0);
        assert!(!publisher.should_publish(hundred, hundred, 1.0));

        publisher.invalidate();
        assert!(publisher.should_publish(two_hundred, two_hundred, 1.0));
    }

    #[test]
    fn extrapolation_never_runs_below_zero() {
        let published = PublishedPosition {
            elapsed: Duration::from_secs(1),
            rate: -2.0,
            at: Duration::ZERO,
        };
        assert_eq!(
            published.extrapolated(Duration::from_secs(5)),
            Duration::ZERO
        );
    }

    #[test]
    fn extrapolation_scales_with_the_rate() {
        let published = PublishedPosition {
            elapsed: Duration::from_secs(10),
            rate: 2.0,
            at: Duration::from_secs(100),
        };
        assert_eq!(
            published.extrapolated(Duration::from_secs(105)),
            Duration::from_secs(20)
        );
    }

    #[test]
    fn mpris_length_is_microseconds_not_seconds() {
        let entries = mpris_metadata(&track(), 1);
        let length = entries
            .iter()
            .find(|(key, _)| *key == "mpris:length")
            .map(|(_, value)| value.clone());

        assert_eq!(length, Some(MprisMetadata::Micros(258_000_000)));
    }

    #[test]
    fn the_track_id_is_a_valid_object_path() {
        let entries = mpris_metadata(&track(), 7);
        let Some((_, MprisMetadata::Path(path))) =
            entries.iter().find(|(key, _)| *key == "mpris:trackid")
        else {
            panic!("mpris:trackid must be present, clients key their state on it");
        };

        assert!(path.starts_with('/'));
        assert!(!path.ends_with('/'));
        assert!(
            path.split('/').skip(1).all(|element| !element.is_empty()
                && element
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')),
            "{path} is not a legal D-Bus object path"
        );
    }

    #[test]
    fn a_new_track_gets_a_new_track_id() {
        let first = mpris_metadata(&track(), 1);
        let second = mpris_metadata(&track(), 2);
        assert_ne!(first[0], second[0], "reusing a path reads as a seek");
    }

    #[test]
    fn the_artist_is_a_list_because_mpris_says_so() {
        let entries = mpris_metadata(&track(), 1);
        assert!(entries.iter().any(|(key, value)| *key == "xesam:artist"
            && *value == MprisMetadata::TextList(vec!["Kavinsky".to_owned()])));
    }

    #[test]
    fn empty_fields_are_omitted_rather_than_sent_blank() {
        let bare = NowPlaying::new("Untitled", "");
        let entries = mpris_metadata(&bare, 1);
        let keys: Vec<&str> = entries.iter().map(|(key, _)| *key).collect();

        assert!(!keys.contains(&"xesam:album"));
        assert!(!keys.contains(&"xesam:artist"));
        assert!(
            !keys.contains(&"mpris:length"),
            "a stream has no length and zero is not the same claim"
        );
        assert!(keys.contains(&"xesam:title"));
    }

    #[test]
    fn a_file_artwork_becomes_a_file_uri() {
        let artwork = Artwork::File(PathBuf::from("/tmp/cover.jpg"));
        assert_eq!(artwork.as_uri(), "file:///tmp/cover.jpg");

        let remote = Artwork::Url("https://cdn.riseonly.net/a.jpg".to_owned());
        assert_eq!(remote.as_uri(), "https://cdn.riseonly.net/a.jpg");
    }

    #[test]
    fn artwork_reaches_the_metadata_as_a_uri() {
        let with_art = track().with_artwork(Artwork::File(PathBuf::from("/tmp/cover.jpg")));
        let entries = mpris_metadata(&with_art, 1);
        assert!(entries.iter().any(|(key, value)| *key == "mpris:artUrl"
            && *value == MprisMetadata::Text("file:///tmp/cover.jpg".to_owned())));
    }

    #[test]
    fn a_hyphen_is_legal_in_a_bus_name_even_though_it_is_not_in_an_interface() {
        assert!(
            MediaIdentity::new("Riseonly", "riseonly-desktop")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn a_bus_suffix_linux_would_reject_is_refused_on_every_host() {
        let too_long = "a".repeat(240);
        let refused = [
            "",
            "rise only",
            "2fast",
            "rise..only",
            "riseonly.",
            "rise/only",
            "rise+only",
            too_long.as_str(),
        ];

        for suffix in refused {
            let identity = MediaIdentity::new("Riseonly", suffix);
            assert!(
                matches!(identity.validate(), Err(MediaKeysError::InvalidBusName(_))),
                "{suffix:?} must not reach a Linux machine unnoticed"
            );
        }
    }

    #[test]
    fn a_valid_suffix_produces_the_mpris_bus_name() {
        let identity = MediaIdentity::new("Riseonly", "riseonly");
        assert_eq!(identity.bus_name(), "org.mpris.MediaPlayer2.riseonly");
        assert!(identity.validate().is_ok());
    }

    #[test]
    fn only_macos_demands_bundle_identity_for_the_card() {
        assert!(card_requires_bundle_identity(HostOs::MacOs));
        assert!(!card_requires_bundle_identity(HostOs::Linux));
        assert!(!card_requires_bundle_identity(HostOs::Windows));
    }

    #[test]
    fn every_host_drives_a_transport_but_only_windows_needs_a_window() {
        for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
            assert!(transport_support(host).is_supported(), "{host:?}");
        }
        assert!(transport_needs_window(HostOs::Windows));
        assert!(!transport_needs_window(HostOs::MacOs));
        assert!(!transport_needs_window(HostOs::Linux));
    }

    #[test]
    fn a_share_starts_detached_so_a_setter_cannot_claim_success() {
        let share = MediaShare::new(accepting());
        assert!(!share.is_attached());

        share.set_attached(true);
        assert!(share.is_attached());
    }

    #[test]
    fn a_delivered_command_reaches_the_handler() {
        let (handler, log) = recording();
        let share = MediaShare::new(handler);

        share.deliver(MediaCommandEvent::simple(MediaCommand::Next));
        share.deliver(MediaCommandEvent::seek(Duration::from_secs(30)));

        let seen = log.lock().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].command, MediaCommand::Next);
        assert_eq!(seen[1].position, Some(Duration::from_secs(30)));
    }

    #[test]
    fn the_track_ordinal_advances_only_on_a_real_track_change() {
        let share = MediaShare::new(accepting());

        share.publish(Some(track()), PlaybackState::Playing);
        let first = share.snapshot().track;

        share.advance(Duration::from_secs(30), 1.0);
        assert_eq!(
            share.snapshot().track,
            first,
            "a position update is not a new track"
        );

        share.publish(
            Some(NowPlaying::new("Rubber", "Zombie Zombie")),
            PlaybackState::Playing,
        );
        assert_ne!(share.snapshot().track, first);
    }

    #[test]
    fn every_change_bumps_the_generation_so_the_service_can_tell() {
        let share = MediaShare::new(accepting());
        let start = share.generation();

        share.publish(Some(track()), PlaybackState::Playing);
        let published = share.generation();
        assert!(published > start);

        share.set_state(PlaybackState::Paused);
        assert!(share.generation() > published);
    }

    #[test]
    fn a_position_update_on_an_empty_card_changes_nothing() {
        let share = MediaShare::new(accepting());
        let start = share.generation();

        assert!(share.advance(Duration::from_secs(5), 1.0).is_none());
        assert_eq!(
            share.generation(),
            start,
            "an update with nothing playing must not wake the service"
        );
    }

    #[test]
    fn the_service_waits_until_something_actually_changes() {
        let share = MediaShare::new(accepting());
        let seen = share.generation();

        let mut waiting = share.changed(seen);
        let mut context = Context::from_waker(Waker::noop());
        assert_eq!(Pin::new(&mut waiting).poll(&mut context), Poll::Pending);

        share.publish(Some(track()), PlaybackState::Playing);
        assert_eq!(
            Pin::new(&mut waiting).poll(&mut context),
            Poll::Ready(share.generation())
        );
    }

    #[test]
    fn a_change_wakes_the_waiting_service_exactly_once() {
        let share = MediaShare::new(accepting());
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&counter));
        let mut context = Context::from_waker(&waker);

        let mut waiting = share.changed(share.generation());
        assert_eq!(Pin::new(&mut waiting).poll(&mut context), Poll::Pending);

        share.publish(Some(track()), PlaybackState::Playing);
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);

        share.set_state(PlaybackState::Paused);
        assert_eq!(
            counter.0.load(Ordering::Relaxed),
            1,
            "a consumed waker must not be woken again before it re-registers"
        );
    }

    #[test]
    fn a_position_update_is_not_mistaken_for_a_track_change() {
        let playing = track().at(Duration::from_secs(10), 1.0);
        let later = track().at(Duration::from_secs(200), 1.0);
        assert!(playing.is_same_track(&later));

        let next = NowPlaying::new("Nightcall", "Kavinsky").with_album("OutRun");
        assert!(
            !playing.is_same_track(&next),
            "a different duration is a different recording"
        );
    }

    fn base(elapsed: u64, rate: f64, at: u64) -> Option<PublishedPosition> {
        Some(PublishedPosition {
            elapsed: Duration::from_secs(elapsed),
            rate,
            at: Duration::from_secs(at),
        })
    }

    #[test]
    fn steady_playback_and_small_drift_are_not_seeks() {
        let now = Duration::from_secs(10);
        assert!(!is_seek(base(30, 1.0, 0), now, Duration::from_secs(40), POSITION_EPSILON));
        assert!(!is_seek(
            base(30, 1.0, 0),
            now,
            Duration::from_millis(40_500),
            POSITION_EPSILON
        ));
    }

    #[test]
    fn a_jump_forward_or_back_is_a_seek() {
        let now = Duration::from_secs(10);
        assert!(is_seek(base(30, 1.0, 0), now, Duration::from_secs(90), POSITION_EPSILON));
        assert!(is_seek(base(30, 1.0, 0), now, Duration::from_secs(5), POSITION_EPSILON));
    }

    #[test]
    fn a_scrub_while_paused_is_a_seek_and_a_still_card_is_not() {
        let now = Duration::from_secs(60);
        assert!(!is_seek(base(30, 0.0, 0), now, Duration::from_secs(30), POSITION_EPSILON));
        assert!(is_seek(base(30, 0.0, 0), now, Duration::from_secs(120), POSITION_EPSILON));
    }

    #[test]
    fn with_no_base_nothing_is_a_seek() {
        assert!(!is_seek(None, Duration::ZERO, Duration::from_secs(90), POSITION_EPSILON));
    }

    #[test]
    fn a_marked_seek_bumps_the_counter_and_wakes_the_service() {
        let share = MediaShare::new(accepting());
        share.publish(Some(track()), PlaybackState::Playing);
        let before = share.snapshot();

        share.mark_seeked();

        let after = share.snapshot();
        assert_eq!(after.seeks, before.seeks + 1);
        assert!(after.generation > before.generation);
        assert_eq!(after.track, before.track, "a seek is not a track change");
    }

    #[test]
    fn raise_runs_the_handler_only_once_one_is_set() {
        let share = MediaShare::new(accepting());
        assert!(!share.can_raise());
        assert!(!share.raise());

        let raised = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&raised);
        share.set_raise_handler(Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));

        assert!(share.can_raise());
        assert!(share.raise());
        assert_eq!(raised.load(Ordering::Relaxed), 1);
    }
}

//! The transport keys on the keyboard, and the now-playing card the OS shows.
//!
//! Three unrelated OS APIs sit behind this seam — MediaPlayer on macOS, MPRIS
//! over D-Bus on Linux, SystemMediaTransportControls on Windows — and they
//! disagree about almost everything: whether a playback state is a number or a
//! string, which number, whether a play/pause toggle exists at all, and whether
//! publishing is a synchronous call or a service the process has to run. All of
//! those disagreements are resolved here as pure functions of [`HostOs`], so
//! they are exercised by the suite on a Mac; only the final call into the OS is
//! behind a `cfg`.
//!
//! The one policy worth stating up front is the elapsed-time rule. The card is
//! not a progress bar: every OS extrapolates the position itself from the last
//! `(elapsed, rate)` pair it was given. Pushing a new pair on every frame, or
//! even on every half-second progress tick, buys nothing and costs an IPC round
//! trip (a D-Bus signal to every listening shell, on Linux) each time. See
//! [`PositionPublisher`].

use std::future::Future;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[cfg(target_os = "macos")]
use std::sync::OnceLock;

#[cfg(target_os = "macos")]
use objc2::msg_send;
#[cfg(target_os = "macos")]
use objc2::rc::{Allocated, Retained};
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSNumber, NSString};

#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

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

/// How the app names itself to the OS.
///
/// Only MPRIS actually reads this, but it is validated on every host on
/// purpose: an invalid bus name is a runtime failure that would otherwise
/// surface for the first time on a Linux machine nobody is sitting at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MediaIdentity {
    /// Human-readable, shown by MPRIS clients as `Identity`.
    pub identity: String,
    /// Appended to `org.mpris.MediaPlayer2.` to form the bus name.
    pub bus_suffix: String,
}

/// Where MPRIS requires both interfaces to live. Not configurable by the spec.
pub const MPRIS_OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

const MPRIS_BUS_PREFIX: &str = "org.mpris.MediaPlayer2";

/// D-Bus refuses a name longer than this, and the error arrives as a connection
/// failure rather than anything that mentions the name.
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

    /// A bus name element may hold `[A-Za-z0-9_-]` and may not begin with a
    /// digit. The hyphen is the trap: it is legal in a *bus* name and illegal
    /// in an *interface* name, so `riseonly-desktop` is fine here and would be
    /// rejected if the same string were ever reused as an interface.
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

/// Where the cover art can be found.
///
/// Deliberately a reference and not bytes: the card is rebuilt on every track
/// change and copying a megabyte of JPEG through this seam each time is the
/// kind of cost that only shows up as battery.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Artwork {
    /// Already on disk. The only form the macOS card can use, because building
    /// an `MPMediaItemArtwork` from anything else needs an image the caller
    /// does not have at this layer.
    File(PathBuf),
    /// A remote URL. MPRIS hands it to the shell untouched.
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

/// What the card shows. No OS types, no gpui types — this crosses the seam.
#[derive(Clone, PartialEq, Debug)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    pub elapsed: Duration,
    /// The nominal rate of the track, not the current one: a paused track still
    /// has a rate of 1.0 here, and [`PlaybackState`] carries the pause.
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

    /// Position is excluded on purpose: it is what changes constantly, and a
    /// position update must not be mistaken for a track change or the OS card
    /// restarts its artwork fetch several times a second.
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
    /// Something else took the audio device — a call, a screen share. Only
    /// macOS has a word for it; elsewhere it degrades to paused.
    Interrupted,
}

/// The value the platform's own API wants for a playback state.
///
/// Not a bare integer, because MPRIS does not use one: `PlaybackStatus` is a
/// string enum on the wire, and flattening it to a number would mean inventing
/// a number no D-Bus client would recognise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawPlaybackState {
    /// macOS `MPNowPlayingPlaybackState`, Windows `MediaPlaybackStatus`.
    Integer(i64),
    /// MPRIS `org.mpris.MediaPlayer2.Player.PlaybackStatus`.
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

    /// The two integer scales run in opposite directions — macOS numbers
    /// `Playing` 1 and Windows numbers it 4, where macOS's 4 means
    /// `Interrupted` and Windows's 1 means `Changing`. Sharing a constant
    /// between the two arms would produce a card that silently shows the wrong
    /// state rather than failing.
    pub fn raw(self, host: HostOs) -> RawPlaybackState {
        match host {
            // MPNowPlayingPlaybackState: Unknown 0, Playing 1, Paused 2,
            // Stopped 3, Interrupted 4.
            HostOs::MacOs => RawPlaybackState::Integer(match self {
                Self::Playing => 1,
                Self::Paused => 2,
                Self::Stopped => 3,
                Self::Interrupted => 4,
            }),
            // MediaPlaybackStatus: Closed 0, Changing 1, Stopped 2, Paused 3,
            // Playing 4. There is no interrupted state.
            HostOs::Windows => RawPlaybackState::Integer(match self {
                Self::Playing => 4,
                Self::Paused | Self::Interrupted => 3,
                Self::Stopped => 2,
            }),
            HostOs::Linux => RawPlaybackState::Text(match self {
                Self::Playing => "Playing",
                Self::Paused | Self::Interrupted => "Paused",
                Self::Stopped => "Stopped",
            }),
        }
    }

    /// The rate to hand the OS, which is 0 for anything not playing. The OS
    /// extrapolates the position from this number, so reporting the nominal
    /// rate while paused makes the card's timer run on its own.
    pub fn effective_rate(self, nominal: f64) -> f64 {
        if self.is_playing() { nominal } else { 0.0 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaCommand {
    Play,
    Pause,
    /// One key that means "the other thing".
    Toggle,
    Next,
    Previous,
    Stop,
    /// Position carried separately, in [`MediaCommandEvent`].
    Seek,
}

impl MediaCommand {
    pub const ALL: [Self; 7] = [
        Self::Play,
        Self::Pause,
        Self::Toggle,
        Self::Next,
        Self::Previous,
        Self::Stop,
        Self::Seek,
    ];

    /// Whether the platform's own transport API can hand this command over at
    /// all — a statement about the API, not about how much of it this crate has
    /// bound yet. See [`transport_support`] for the latter.
    ///
    /// `SystemMediaTransportControls` has no toggle button in
    /// `SystemMediaTransportControlsButton`, so a Windows build that waits for
    /// `Toggle` waits forever; [`MediaCommand::toggle_for`] is the way out.
    pub fn is_deliverable_on(self, host: HostOs) -> bool {
        match host {
            HostOs::MacOs | HostOs::Linux => true,
            HostOs::Windows => self != Self::Toggle,
        }
    }

    /// The member name on `org.mpris.MediaPlayer2.Player`. Seeking is
    /// `SetPosition` rather than `Seek` because MPRIS's `Seek` is a *relative*
    /// offset and this command carries an absolute one.
    pub fn mpris_member(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Toggle => "PlayPause",
            Self::Next => "Next",
            Self::Previous => "Previous",
            Self::Stop => "Stop",
            Self::Seek => "SetPosition",
        }
    }

    /// What a play/pause key has to become on `host`.
    ///
    /// macOS and MPRIS both own a real toggle and resolve it themselves, which
    /// is the correct answer because they know the state at the instant the key
    /// was pressed. Windows cannot, so the app resolves it and accepts the race.
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

/// A command as it arrived, with whatever the OS attached to it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MediaCommandEvent {
    pub command: MediaCommand,
    /// Set only for [`MediaCommand::Seek`].
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

/// What the app did with a command. macOS turns this back into an
/// `MPRemoteCommandHandlerStatus`, which is what stops the key from being
/// reported as working when nothing happened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandOutcome {
    Handled,
    Rejected,
}

/// Shared, not boxed: the macOS binding is called from Objective-C with no
/// receiver of ours, so the handler has to live in a process-wide slot as well
/// as in the shared state, and cloning an `Arc` is how both get one.
pub type CommandHandler = Arc<dyn Fn(MediaCommandEvent) -> CommandOutcome + Send + Sync + 'static>;

/// How far the OS card may drift from the truth before republishing is worth
/// the round trip. Three quarters of a second is under the threshold at which a
/// reader notices a seconds counter is wrong, and well above the jitter of a
/// player that reports its position from an audio callback.
pub const POSITION_EPSILON: Duration = Duration::from_millis(750);

/// Rates are compared with a tolerance because they arrive as `f64` from a
/// player that computes them; an exact comparison republishes on noise.
const RATE_EPSILON: f64 = 0.001;

/// The last `(position, rate)` pair the OS was given, and when.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PublishedPosition {
    pub elapsed: Duration,
    pub rate: f64,
    /// A reading of any monotonic clock, as long as it is the same clock the
    /// predicate is later called with.
    pub at: Duration,
}

impl PublishedPosition {
    /// What the OS believes the position is at `now`, having been told nothing
    /// since. This is the number the republish decision is made against, and
    /// getting it wrong in either direction is expensive: too generous and the
    /// card lies, too strict and it republishes every tick.
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

/// Whether the OS card is far enough from the truth to be worth updating.
///
/// Steady playback must answer `false` forever: the OS is already running the
/// same clock we are, so a tick that merely confirms it is pure cost.
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

/// [`should_republish_position`] with the bookkeeping attached.
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

    /// Drops the extrapolation base. Required on a track change and on any
    /// state change, because the OS is about to be told a new pair anyway and
    /// extrapolating across the boundary produces a position from the old track.
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    pub fn last(&self) -> Option<PublishedPosition> {
        self.last
    }
}

/// One entry of the MPRIS `Metadata` map, before it becomes a `zvariant` value.
///
/// Modelled separately so the projection — which is where the real traps are —
/// is testable on a machine with no D-Bus.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MprisMetadata {
    Text(String),
    /// `xesam:artist` is `as`, not `s`. A single artist is still a list.
    TextList(Vec<String>),
    /// `mpris:trackid` is `o`: a D-Bus object path, whose elements accept only
    /// `[A-Za-z0-9_]` — no hyphens, unlike the bus name.
    Path(String),
    /// `mpris:length` and `Position` are microseconds. macOS wants seconds for
    /// the same numbers, which is where a factor of a million goes missing.
    Micros(i64),
}

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

/// The object path published as `mpris:trackid` for a track ordinal.
///
/// Built in one place because `SetPosition`'s staleness check compares against
/// it: if the two ever disagreed about the format, every seek would be rejected
/// as stale and the scrubber would appear simply broken.
pub fn track_path(ordinal: u64) -> String {
    format!("/net/riseonly/track/{ordinal}")
}

/// The MPRIS `Metadata` map for a track.
///
/// `ordinal` distinguishes successive tracks: MPRIS clients key their state off
/// `mpris:trackid`, so reusing one path across two tracks makes a client treat
/// the second as a seek within the first.
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

/// Which hosts this crate can currently drive.
///
/// Windows reports `Unsupported`: `SystemMediaTransportControls` is WinRT, and
/// the only way to obtain one for a desktop app is
/// `ISystemMediaTransportControlsInterop::GetForWindow`, which needs the `HWND`
/// of the app window. This layer does not own a window — gpui does — so binding
/// it means threading a raw window handle down here, which is a different seam's
/// decision. Nothing else is missing.
pub fn transport_support(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Linux => PlatformSupport::Performed,
        HostOs::Windows => PlatformSupport::Unsupported,
    }
}

/// Whether the now-playing card refuses to appear for an unbundled process.
///
/// macOS keys the card off bundle identity: `MPNowPlayingInfoCenter` accepts
/// the dictionary from a bare binary and simply shows nothing, with no error
/// anywhere. This is why the app always runs from `Riseonly.app`, and why
/// [`MediaKeys::install`] fails loudly instead of reproducing that silence.
pub fn card_requires_bundle_identity(host: HostOs) -> bool {
    host == HostOs::MacOs
}

#[derive(Clone, PartialEq, Debug)]
pub struct MediaSnapshot {
    pub now: Option<NowPlaying>,
    pub state: PlaybackState,
    /// Bumped on every change, so a service task can tell whether it is stale
    /// without comparing whole structs.
    pub generation: u64,
    /// Bumped only when the track itself changes.
    pub track: u64,
}

struct Shared {
    now: Option<NowPlaying>,
    state: PlaybackState,
    generation: u64,
    track: u64,
    attached: bool,
    waker: Option<Waker>,
}

/// The state both bindings publish from, and the only thing the service task
/// and the app share.
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
                attached: false,
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

    /// Whether an MPRIS `TrackId` still names the track that is loaded.
    ///
    /// A client computes a seek against the track it last saw. If the track
    /// changed in between, obeying that seek scrubs the wrong song — which is
    /// why the spec makes the argument mandatory and says to ignore a mismatch.
    /// With no track loaded nothing can match.
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
        }
    }

    /// False until a binding has actually claimed the OS-side resource. The
    /// setters report `Unsupported` while it is false rather than claiming a
    /// success that went nowhere.
    pub fn is_attached(&self) -> bool {
        self.inner.lock().attached
    }

    /// Called by the arm that owns the OS resource — on Linux, once the MPRIS
    /// service task holds the bus name.
    pub fn set_attached(&self, attached: bool) {
        let waker = {
            let mut inner = self.inner.lock();
            inner.attached = attached;
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

    /// Resolves the next time the published state differs from `seen`.
    ///
    /// This crate has no async runtime and cannot acquire one — `zbus` with the
    /// `tokio` feature borrows the caller's. So the Linux arm cannot poll on a
    /// timer, and instead waits here for the app to change something.
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

/// The waker is taken out from under the lock before it is called: a waker is
/// arbitrary code, and on a single-threaded executor it can poll the very
/// future that is about to lock this mutex again.
fn wake(waker: Option<Waker>) {
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Resolves with the new generation once the shared state moves past `seen`.
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

/// The outcome of a position update, which is richer than supported/not
/// because the interesting answer is the third one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionUpdate {
    Published,
    /// Within the epsilon of what the OS already extrapolates. Not a failure —
    /// this is the expected answer for nearly every tick of steady playback.
    Skipped,
    Unsupported,
}

/// The transport keys and the now-playing card.
pub struct MediaKeys {
    share: Arc<MediaShare>,
    identity: MediaIdentity,
    publisher: PositionPublisher,
    clock: Instant,
    #[cfg(target_os = "macos")]
    mac: MacTransport,
    /// Held on every platform, not only the one where the binding is genuinely
    /// main-thread-only, so that a Linux build cannot compile a `Send` bound
    /// that the macOS build then rejects.
    _main_thread_only: PhantomData<*const ()>,
}

impl MediaKeys {
    /// Claims the OS transport. On macOS this registers the remote command
    /// targets immediately; on Linux nothing reaches the bus until
    /// [`MediaKeys::service`] is running.
    pub fn install(
        identity: &MediaIdentity,
        handler: CommandHandler,
    ) -> Result<Self, MediaKeysError> {
        identity.validate()?;

        let share = Arc::new(MediaShare::new(handler));

        #[cfg(target_os = "macos")]
        let mac = MacTransport::install(share.handler())?;

        Ok(Self {
            share,
            identity: identity.clone(),
            publisher: PositionPublisher::default(),
            clock: Instant::now(),
            #[cfg(target_os = "macos")]
            mac,
            _main_thread_only: PhantomData,
        })
    }

    /// The part of the seam that needs an executor.
    ///
    /// macOS has none: the card is a synchronous main-thread call. MPRIS is a
    /// D-Bus *service* — the process owns a bus name and answers method calls —
    /// which cannot be done from a synchronous call site, so the Linux work is
    /// handed to a future the app drives on the engine's runtime.
    pub fn service(&self) -> MediaService {
        MediaService {
            share: Arc::clone(&self.share),
            identity: self.identity.clone(),
        }
    }

    pub fn share(&self) -> &Arc<MediaShare> {
        &self.share
    }

    pub fn set_now_playing(&mut self, track: &NowPlaying, state: PlaybackState) -> PlatformSupport {
        if self.share.is_new_track(track) {
            self.publisher.invalidate();
        }
        self.share.publish(Some(track.clone()), state);
        self.publish_to_os(track, state);
        self.transport()
    }

    pub fn set_playback_state(&mut self, state: PlaybackState) -> PlatformSupport {
        // A state change is a rate change, and the OS extrapolates from the
        // rate: the next position has to go out whatever the epsilon says.
        self.publisher.invalidate();
        if let Some(track) = self.share.set_state(state) {
            self.publish_to_os(&track, state);
        }
        self.transport()
    }

    /// Call this as often as the player reports progress. It is cheap by design
    /// — most calls answer [`PositionUpdate::Skipped`] without touching the OS.
    pub fn set_position(&mut self, elapsed: Duration, rate: f64) -> PositionUpdate {
        let now = self.clock.elapsed();
        if !self.publisher.should_publish(now, elapsed, rate) {
            return PositionUpdate::Skipped;
        }

        let Some((track, state)) = self.share.advance(elapsed, rate) else {
            return PositionUpdate::Skipped;
        };
        self.publish_to_os(&track, state);

        match self.transport() {
            PlatformSupport::Performed => PositionUpdate::Published,
            PlatformSupport::Unsupported => PositionUpdate::Unsupported,
        }
    }

    pub fn clear(&mut self) -> PlatformSupport {
        self.publisher.invalidate();
        self.share.publish(None, PlaybackState::Stopped);
        self.clear_os();
        self.transport()
    }

    fn transport(&self) -> PlatformSupport {
        match HostOs::current() {
            // `install` returns an error rather than a half-installed seam, so
            // reaching here on macOS means the binding is live.
            HostOs::MacOs => PlatformSupport::Performed,
            HostOs::Linux if self.share.is_attached() => PlatformSupport::Performed,
            HostOs::Linux | HostOs::Windows => PlatformSupport::Unsupported,
        }
    }

    fn publish_to_os(&self, track: &NowPlaying, state: PlaybackState) {
        #[cfg(target_os = "macos")]
        self.mac.publish(track, state);
        #[cfg(not(target_os = "macos"))]
        {
            // Linux publishes from the service task, which the generation bump
            // has already woken.
            let _ = (track, state);
        }
    }

    fn clear_os(&self) {
        #[cfg(target_os = "macos")]
        self.mac.clear();
    }
}

/// Drives the parts of the seam that need to run, rather than be called.
pub struct MediaService {
    share: Arc<MediaShare>,
    identity: MediaIdentity,
}

impl MediaService {
    /// Runs until the connection drops. On macOS and Windows it returns
    /// immediately: neither has anything to run.
    pub async fn run(self) -> Result<(), MediaKeysError> {
        #[cfg(target_os = "linux")]
        {
            serve_mpris(self.share, self.identity).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (&self.share, &self.identity);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// macOS: MPNowPlayingInfoCenter and MPRemoteCommandCenter.
//
// Neither has a binding crate in this workspace's dependency set, so the
// classes are looked up at runtime and the framework is linked by hand. A
// missing class means the framework is absent on this OS version, which is an
// honest `Unavailable`, not a panic.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[link(name = "MediaPlayer", kind = "framework")]
unsafe extern "C" {}

/// `MPRemoteCommandHandlerStatusSuccess`.
#[cfg(target_os = "macos")]
const REMOTE_SUCCESS: isize = 0;

/// `MPRemoteCommandHandlerStatusCommandFailed`. Anything the app declines has
/// to come back as this, or macOS keeps offering the key.
#[cfg(target_os = "macos")]
const REMOTE_COMMAND_FAILED: isize = 200;

/// The remote command targets are Objective-C objects with no Rust receiver, so
/// the handler lives here. An `Arc` is cloned out and the lock released before
/// the handler runs, so a handler that reinstalls the seam cannot deadlock.
#[cfg(target_os = "macos")]
static MAC_HANDLER: Mutex<Option<CommandHandler>> = Mutex::new(None);

#[cfg(target_os = "macos")]
struct MacTransport {
    info_center: Retained<AnyObject>,
    command_center: Retained<AnyObject>,
    target: Retained<AnyObject>,
}

/// The seven commands, and the selector wired to each. One array so that
/// registration and unregistration cannot drift apart: a command added here but
/// forgotten in `Drop` is a dangling pointer in a process-wide singleton.
#[cfg(target_os = "macos")]
const MAC_COMMANDS: [(&std::ffi::CStr, &std::ffi::CStr); 7] = [
    (c"playCommand", c"riseHandlePlay:"),
    (c"pauseCommand", c"riseHandlePause:"),
    (c"togglePlayPauseCommand", c"riseHandleToggle:"),
    (c"nextTrackCommand", c"riseHandleNext:"),
    (c"previousTrackCommand", c"riseHandlePrevious:"),
    (c"stopCommand", c"riseHandleStop:"),
    (c"changePlaybackPositionCommand", c"riseHandleSeek:"),
];

/// # Safety
///
/// `center` must be an `MPRemoteCommandCenter`, and `getter` one of its
/// documented command properties.
#[cfg(target_os = "macos")]
unsafe fn command_of(center: &AnyObject, getter: &std::ffi::CStr) -> Option<Retained<AnyObject>> {
    let getter = Sel::register(getter);
    // SAFETY: each getter is a zero-argument property returning an
    // autoreleased `MPRemoteCommand`, and none belongs to a +1 selector family.
    unsafe { msg_send![center, performSelector: getter] }
}

#[cfg(target_os = "macos")]
impl MacTransport {
    fn install(handler: CommandHandler) -> Result<Self, MediaKeysError> {
        if bundle_identifier().is_none() {
            return Err(MediaKeysError::NotBundled);
        }

        let info_class =
            AnyClass::get(c"MPNowPlayingInfoCenter").ok_or(MediaKeysError::Unavailable)?;
        let command_class =
            AnyClass::get(c"MPRemoteCommandCenter").ok_or(MediaKeysError::Unavailable)?;
        let target_class = command_target_class().ok_or(MediaKeysError::Unavailable)?;

        // SAFETY: both selectors are documented class methods returning a
        // shared singleton, and neither belongs to a +1 selector family.
        let (info_center, command_center, target) = unsafe {
            let info_center: Option<Retained<AnyObject>> = msg_send![info_class, defaultCenter];
            let command_center: Option<Retained<AnyObject>> =
                msg_send![command_class, sharedCommandCenter];
            let target: Option<Retained<AnyObject>> = msg_send![target_class, new];
            (info_center, command_center, target)
        };

        let info_center = info_center.ok_or(MediaKeysError::Unavailable)?;
        let command_center = command_center.ok_or(MediaKeysError::Unavailable)?;
        let target = target.ok_or(MediaKeysError::Unavailable)?;

        *MAC_HANDLER.lock() = Some(handler);

        // SAFETY: every accessor is a documented `MPRemoteCommand` property on
        // `MPRemoteCommandCenter`, and every action is a selector this process
        // registered on `target`'s class with a matching signature.
        unsafe {
            let center = &*command_center;

            for (getter, action) in MAC_COMMANDS {
                wire(command_of(center, getter), &target, action);
            }
        }

        Ok(Self {
            info_center,
            command_center,
            target,
        })
    }

    fn publish(&self, track: &NowPlaying, state: PlaybackState) {
        let Some(dictionary_class) = AnyClass::get(c"NSMutableDictionary") else {
            return;
        };

        // SAFETY: `+dictionary` returns an autoreleased NSMutableDictionary.
        let info: Option<Retained<AnyObject>> = unsafe { msg_send![dictionary_class, dictionary] };
        let Some(info) = info else {
            return;
        };

        // These are the literal runtime values of the MediaPlayer constants:
        // the MPMediaItem keys are the bare property names, while the
        // MPNowPlayingInfoProperty keys are their own constant names verbatim.
        // Reading them as symbols would need an import this crate does not have.
        //
        // SAFETY: `info` is a mutable dictionary and every value is an object.
        unsafe {
            set_entry(&info, "title", &*NSString::from_str(&track.title));
            set_entry(&info, "artist", &*NSString::from_str(&track.artist));
            set_entry(&info, "albumTitle", &*NSString::from_str(&track.album));
            // Omitted at zero, exactly as `mpris_metadata` omits `mpris:length`:
            // a live stream has no length, and claiming a duration of zero draws
            // a scrubber that is permanently at its end. The reference guards
            // every one of its own `MPMediaItemPropertyPlaybackDuration` writes
            // the same way.
            if track.duration > Duration::ZERO {
                set_entry(
                    &info,
                    "playbackDuration",
                    &*NSNumber::new_f64(track.duration.as_secs_f64()),
                );
            }
            set_entry(
                &info,
                "MPNowPlayingInfoPropertyElapsedPlaybackTime",
                &*NSNumber::new_f64(track.elapsed.as_secs_f64()),
            );
            set_entry(
                &info,
                "MPNowPlayingInfoPropertyPlaybackRate",
                &*NSNumber::new_f64(state.effective_rate(track.rate)),
            );
            // MPNowPlayingInfoMediaTypeAudio.
            set_entry(
                &info,
                "MPNowPlayingInfoPropertyMediaType",
                &*NSNumber::new_isize(1),
            );

            if let Some(artwork) = track.artwork.as_ref().and_then(artwork_object) {
                set_entry(&info, "artwork", &*artwork);
            }
        }

        let raw = state
            .raw(HostOs::MacOs)
            .integer()
            .unwrap_or_default()
            .max(0) as usize;

        // SAFETY: `setNowPlayingInfo:` takes an NSDictionary and
        // `setPlaybackState:` an NSUInteger-backed MPNowPlayingPlaybackState.
        unsafe {
            let _: () = msg_send![&*self.info_center, setNowPlayingInfo: &*info];
            let _: () = msg_send![&*self.info_center, setPlaybackState: raw];
        }
    }

    fn clear(&self) {
        let stopped = PlaybackState::Stopped
            .raw(HostOs::MacOs)
            .integer()
            .unwrap_or_default()
            .max(0) as usize;

        // SAFETY: a nil dictionary is the documented way to remove the card.
        unsafe {
            let nil: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![&*self.info_center, setNowPlayingInfo: nil];
            let _: () = msg_send![&*self.info_center, setPlaybackState: stopped];
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacTransport {
    /// `MPRemoteCommand.addTarget:action:` does **not** retain its target, and
    /// `MPRemoteCommandCenter` is a process-wide singleton that outlives this
    /// struct. Releasing the target without unregistering it would leave seven
    /// unowned pointers to freed memory inside a live OS object, and the next
    /// press of a media key — by this app or any other — would message it.
    ///
    /// The card is torn down too: an app that has exited must not still be what
    /// the system shows as playing.
    fn drop(&mut self) {
        self.clear();

        // SAFETY: the same documented properties `install` wired, and
        // `removeTarget:` drops every action registered for this target.
        unsafe {
            let center = &*self.command_center;

            for (getter, _) in MAC_COMMANDS {
                if let Some(command) = command_of(center, getter) {
                    let _: () = msg_send![&*command, removeTarget: &*self.target];
                }
            }
        }

        *MAC_HANDLER.lock() = None;
    }
}

/// # Safety
///
/// `command` must be an `MPRemoteCommand`, `target` an instance of the class
/// built by [`command_target_class`], and `action` one of the selectors
/// registered on it.
#[cfg(target_os = "macos")]
unsafe fn wire(command: Option<Retained<AnyObject>>, target: &AnyObject, action: &std::ffi::CStr) {
    let Some(command) = command else {
        return;
    };
    let action = Sel::register(action);
    unsafe {
        let _: () = msg_send![&*command, setEnabled: Bool::YES];
        let _: () = msg_send![&*command, addTarget: target, action: action];
    }
}

/// # Safety
///
/// `dictionary` must be an `NSMutableDictionary`.
#[cfg(target_os = "macos")]
unsafe fn set_entry<V: objc2::Message>(dictionary: &AnyObject, key: &str, value: &V) {
    let key = NSString::from_str(key);
    unsafe {
        let _: () = msg_send![dictionary, setObject: value, forKey: &*key];
    }
}

/// `MPMediaItemArtwork`'s current initialiser takes a block, and no block crate
/// is in this workspace's dependency set. The image initialiser it replaced is
/// deprecated rather than removed, and is what this uses — after checking the
/// method is really still there, because a missing selector raises an
/// Objective-C exception rather than answering nil, and losing the cover art is
/// not worth losing the process over.
#[cfg(target_os = "macos")]
fn artwork_object(artwork: &Artwork) -> Option<Retained<AnyObject>> {
    let Artwork::File(path) = artwork else {
        return None;
    };

    let image_class = AnyClass::get(c"NSImage")?;
    let artwork_class = AnyClass::get(c"MPMediaItemArtwork")?;
    if !artwork_class.responds_to(Sel::register(c"initWithImage:")) {
        return None;
    }

    let path = NSString::from_str(path.to_str()?);

    // SAFETY: alloc/init pairs on their own classes, with the initialiser
    // verified to exist. Both answer nil rather than raising when the file
    // cannot be read.
    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![image_class, alloc];
        let image: Option<Retained<AnyObject>> =
            msg_send![allocated, initWithContentsOfFile: &*path];
        let image = image?;

        let allocated: Allocated<AnyObject> = msg_send![artwork_class, alloc];
        msg_send![allocated, initWithImage: &*image]
    }
}

#[cfg(target_os = "macos")]
fn bundle_identifier() -> Option<Retained<AnyObject>> {
    let bundle_class = AnyClass::get(c"NSBundle")?;

    // SAFETY: `+mainBundle` always answers, and `-bundleIdentifier` is nil for
    // a process that is not inside a bundle — which is exactly what is tested.
    unsafe {
        let bundle: Option<Retained<AnyObject>> = msg_send![bundle_class, mainBundle];
        let bundle = bundle?;
        msg_send![&*bundle, bundleIdentifier]
    }
}

/// Registered once per process. `ClassBuilder::new` answers `None` for a name
/// that already exists, so the `OnceLock` is not an optimisation.
#[cfg(target_os = "macos")]
fn command_target_class() -> Option<&'static AnyClass> {
    static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();

    *CLASS.get_or_init(|| {
        let superclass = AnyClass::get(c"NSObject")?;
        let mut builder = ClassBuilder::new(c"RiseMediaCommandTarget", superclass)?;

        // SAFETY: every implementation has the signature MediaPlayer invokes an
        // `addTarget:action:` handler with — one object argument, an
        // MPRemoteCommandHandlerStatus return.
        unsafe {
            builder.add_method(
                Sel::register(c"riseHandlePlay:"),
                handle_play as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandlePause:"),
                handle_pause as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandleToggle:"),
                handle_toggle as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandleNext:"),
                handle_next as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandlePrevious:"),
                handle_previous as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandleStop:"),
                handle_stop as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandleSeek:"),
                handle_seek as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
        }

        Some(builder.register())
    })
}

/// A panic crossing back into Objective-C from an `extern "C"` function aborts
/// the process, so a media key would take the whole app down with it. The
/// handler is app code; it is caught here and reported as a failed command.
#[cfg(target_os = "macos")]
fn dispatch(event: MediaCommandEvent) -> isize {
    let handler = MAC_HANDLER.lock().clone();
    let Some(handler) = handler else {
        return REMOTE_COMMAND_FAILED;
    };

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || handler(event)));

    match outcome {
        Ok(CommandOutcome::Handled) => REMOTE_SUCCESS,
        Ok(CommandOutcome::Rejected) | Err(_) => REMOTE_COMMAND_FAILED,
    }
}

#[cfg(target_os = "macos")]
extern "C" fn handle_play(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Play))
}

#[cfg(target_os = "macos")]
extern "C" fn handle_pause(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Pause))
}

#[cfg(target_os = "macos")]
extern "C" fn handle_toggle(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Toggle))
}

#[cfg(target_os = "macos")]
extern "C" fn handle_next(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Next))
}

#[cfg(target_os = "macos")]
extern "C" fn handle_previous(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Previous))
}

#[cfg(target_os = "macos")]
extern "C" fn handle_stop(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Stop))
}

#[cfg(target_os = "macos")]
extern "C" fn handle_seek(_this: *mut AnyObject, _cmd: Sel, event: *mut AnyObject) -> isize {
    // SAFETY: `changePlaybackPositionCommand` delivers an
    // `MPChangePlaybackPositionCommandEvent`, whose `positionTime` is an
    // NSTimeInterval. A nil event is refused rather than assumed.
    let position: f64 = unsafe {
        let Some(event) = event.as_ref() else {
            return REMOTE_COMMAND_FAILED;
        };
        msg_send![event, positionTime]
    };

    if !position.is_finite() || position < 0.0 {
        return REMOTE_COMMAND_FAILED;
    }

    dispatch(MediaCommandEvent::seek(Duration::from_secs_f64(position)))
}

// ---------------------------------------------------------------------------
// Linux: MPRIS over D-Bus. Written blind — nothing here has been run.
//
// The shape follows the spec: two interfaces on one object path, the bus name
// owned for as long as the connection lives. `Seeked` is not emitted; clients
// read `Position` on demand, which the spec allows and which avoids a signal
// this arm has no way to test.
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn backend(error: impl std::fmt::Display) -> MediaKeysError {
    MediaKeysError::Backend(error.to_string())
}

#[cfg(target_os = "linux")]
struct MprisRoot {
    identity: String,
}

#[cfg(target_os = "linux")]
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl MprisRoot {
    fn raise(&self) {}

    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
struct MprisPlayer {
    share: Arc<MediaShare>,
}

#[cfg(target_os = "linux")]
impl MprisPlayer {
    fn send(&self, command: MediaCommand) {
        self.share.deliver(MediaCommandEvent::simple(command));
    }
}

#[cfg(target_os = "linux")]
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl MprisPlayer {
    fn play(&self) {
        self.send(MediaCommand::Play);
    }

    fn pause(&self) {
        self.send(MediaCommand::Pause);
    }

    fn play_pause(&self) {
        self.send(MediaCommand::Toggle);
    }

    fn next(&self) {
        self.send(MediaCommand::Next);
    }

    fn previous(&self) {
        self.send(MediaCommand::Previous);
    }

    fn stop(&self) {
        self.send(MediaCommand::Stop);
    }

    /// MPRIS `Seek` is a signed offset in microseconds; this seam only carries
    /// absolute positions, so the offset is resolved against the last published
    /// one here rather than at the app.
    fn seek(&self, offset: i64) {
        let Some(track) = self.share.snapshot().now else {
            return;
        };
        let current = micros(track.elapsed);
        let target = current.saturating_add(offset).max(0);
        self.share
            .deliver(MediaCommandEvent::seek(Duration::from_micros(
                target as u64,
            )));
    }

    /// MPRIS puts two staleness rules on this method and both are ignorable only
    /// at the user's expense. A negative position must be *dropped*, not clamped
    /// — clamping restarts the track the user was scrubbing. And the `TrackId`
    /// exists because the client may have computed the seek against a track that
    /// has since changed; obeying it then seeks the wrong song. The macOS handler
    /// for the same command already rejects a negative position, so ignoring both
    /// here made one file disagree with itself about the same gesture.
    fn set_position(&self, track: OwnedObjectPath, position: i64) {
        let Ok(position) = u64::try_from(position) else {
            return;
        };

        if !self.share.is_current_track(track.as_str()) {
            return;
        }

        self.share
            .deliver(MediaCommandEvent::seek(Duration::from_micros(position)));
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.share
            .snapshot()
            .state
            .raw(HostOs::Linux)
            .text()
            .unwrap_or("Stopped")
            .to_owned()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.share.snapshot().now.map_or(1.0, |track| track.rate)
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let snapshot = self.share.snapshot();
        let Some(track) = snapshot.now else {
            return HashMap::new();
        };

        mpris_metadata(&track, snapshot.track)
            .into_iter()
            .filter_map(|(key, entry)| Some((key.to_owned(), mpris_value(entry)?)))
            .collect()
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.share
            .snapshot()
            .now
            .map_or(0, |track| micros(track.elapsed))
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

#[cfg(target_os = "linux")]
fn mpris_value(entry: MprisMetadata) -> Option<OwnedValue> {
    let value = match entry {
        MprisMetadata::Text(text) => Value::from(text),
        MprisMetadata::TextList(items) => Value::from(items),
        MprisMetadata::Path(path) => Value::from(ObjectPath::try_from(path).ok()?),
        MprisMetadata::Micros(value) => Value::from(value),
    };
    OwnedValue::try_from(value).ok()
}

#[cfg(target_os = "linux")]
async fn serve_mpris(
    share: Arc<MediaShare>,
    identity: MediaIdentity,
) -> Result<(), MediaKeysError> {
    let connection = zbus::connection::Builder::session()
        .map_err(backend)?
        .name(identity.bus_name())
        .map_err(backend)?
        .serve_at(
            MPRIS_OBJECT_PATH,
            MprisRoot {
                identity: identity.identity.clone(),
            },
        )
        .map_err(backend)?
        .serve_at(
            MPRIS_OBJECT_PATH,
            MprisPlayer {
                share: Arc::clone(&share),
            },
        )
        .map_err(backend)?
        .build()
        .await
        .map_err(backend)?;

    share.set_attached(true);

    let mut seen = share.generation();
    loop {
        seen = share.changed(seen).await;

        let player = connection
            .object_server()
            .interface::<_, MprisPlayer>(MPRIS_OBJECT_PATH)
            .await
            .map_err(backend)?;
        let emitter = player.signal_emitter();

        // A read guard, not a write one: nothing here mutates the interface,
        // and a write guard would block every incoming method call for as long
        // as three signals take to go out.
        let interface = player.get().await;

        // Position is deliberately absent: MPRIS forbids it in
        // PropertiesChanged, because a position that changes by itself would
        // make every client redraw at whatever rate we chose to emit at.
        interface
            .playback_status_changed(emitter)
            .await
            .map_err(backend)?;
        interface.metadata_changed(emitter).await.map_err(backend)?;
        interface.rate_changed(emitter).await.map_err(backend)?;
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

    /// MPRIS makes the `TrackId` mandatory precisely so a seek computed against
    /// a track that has since changed can be discarded. Obeying it scrubs the
    /// song the user is now listening to, to a position they chose for the
    /// previous one.
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

    /// The card and the MPRIS map must make the same claim about a stream: a
    /// duration of zero is not a duration, and publishing it draws a scrubber
    /// pinned to its own end.
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
            Some(4),
            "MediaPlaybackStatus.Playing — macOS's 1 means Changing here"
        );
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

        // Four ticks a second for a minute, with the jitter a real player has.
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

        // The player advances 10% faster than the rate it declared.
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
    fn windows_reports_unsupported_until_smtc_is_bound() {
        assert_eq!(
            transport_support(HostOs::Windows),
            PlatformSupport::Unsupported
        );
        assert!(transport_support(HostOs::MacOs).is_supported());
        assert!(transport_support(HostOs::Linux).is_supported());
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

    #[cfg(target_os = "macos")]
    #[test]
    fn the_mediaplayer_framework_is_linked_and_its_classes_are_present() {
        assert!(AnyClass::get(c"MPNowPlayingInfoCenter").is_some());
        assert!(AnyClass::get(c"MPRemoteCommandCenter").is_some());
        assert!(AnyClass::get(c"MPMediaItemArtwork").is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_remote_command_target_class_registers_once() {
        let first = command_target_class();
        assert!(first.is_some());
        assert_eq!(
            first.map(std::ptr::from_ref),
            command_target_class().map(std::ptr::from_ref),
            "a second registration under the same name would answer None"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unbundled_process_is_refused_the_card_instead_of_being_ignored() {
        // The test binary is not inside Riseonly.app, which is exactly the
        // situation macOS answers by showing nothing at all.
        let identity = MediaIdentity::new("Riseonly", "riseonly.test");
        assert!(
            matches!(
                MediaKeys::install(&identity, accepting()),
                Err(MediaKeysError::NotBundled)
            ),
            "silently publishing to a card that cannot appear is the failure this prevents"
        );
    }
}

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AsyncApp, Global, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, point,
    px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_media::audio::sound_mute;
use rise_platform::system_share::IncomingShare;
use serde::{Deserialize, Serialize};

use crate::app::shell::handover;
use crate::core::engine_bridge::network_transport_config;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::system_share::desktop_share_intake::{DesktopShareArguments, split_opened_urls};

pub const MUTE_ARGUMENT: &str = "-riseMuteSound";
pub const DRIVE_DIR_VARIABLE: &str = "RISE_DEV_DRIVE_DIR";
pub const STEALTH_VARIABLE: &str = "RISE_DEV_STEALTH";
pub const PHOTO_DIR_VARIABLE: &str = "RISE_DEV_PHOTO_DIR";
pub const DEFAULT_PHOTO_DIR: &str = "RiseTestPhotos";
pub const LOCATION_VARIABLE: &str = "RISE_DEV_LOCATION";
pub const DEFAULT_TESTER_LOCATION: (f64, f64) = (41.3111, 69.2797);
pub const TESTER_SAVE_DIR: &str = "Downloads";
pub const SNAPSHOT_REQUEST: &str = "snapshot.request";
pub const SNAPSHOT_DONE: &str = "snapshot.done";
pub const COMMAND_REQUEST: &str = "command.request";
pub const COMMAND_DONE: &str = "command.done";
pub const COORDINATES: &str = "logical pixels from the window's top left: snapshot pixels / scale";

const SNAPSHOT_DONE_PARTIAL: &str = "snapshot.done.partial";
const DEFAULT_STEM: &str = "snapshot";
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const COMMAND_DONE_PARTIAL: &str = "command.done.partial";
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STEP_GAP: Duration = Duration::from_millis(16);
const KEY_GAP: Duration = Duration::from_millis(8);
const HOLD_FRAME: Duration = Duration::from_millis(50);
const DRAG_STEPS: usize = 12;
const SWIPE_STEPS: usize = 12;
const SWIPE_MS: u64 = 180;
const MOMENTUM_GAP: Duration = Duration::from_millis(16);
const MOMENTUM_DECAY: f32 = 0.9;
const MOMENTUM_MIN_STEP: f32 = 0.5;
const MOMENTUM_MAX_FRAMES: usize = 90;
const RESIZE_FRAMES: usize = 90;
const RESIZE_MAX_SIDE: f32 = 8192.0;
#[cfg(not(any(test, feature = "dev-drive")))]
const NOT_BUILT: &str = "built without the dev-drive feature";

pub const ENABLED: bool = cfg!(all(feature = "dev-drive", debug_assertions));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevDrive {
    sound_muted: bool,
    drive_dir: Option<PathBuf>,
    stealth: bool,
}

impl Global for DevDrive {}

impl DevDrive {
    pub fn parse(arguments: impl IntoIterator<Item = String>, drive_dir: Option<OsString>) -> Self {
        Self {
            sound_muted: arguments.into_iter().any(|argument| argument == MUTE_ARGUMENT),
            drive_dir: drive_dir
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from),
            stealth: false,
        }
    }

    /// stealth mode mutes sound too, same as the -riseMuteSound flag
    pub fn with_stealth(mut self, stealth: bool) -> Self {
        self.stealth = stealth;
        if stealth {
            self.sound_muted = true;
        }
        self
    }

    /// read again by storage, which is built before any global can be asked
    pub fn stealth_from_launch() -> bool {
        ENABLED && stealth_requested(std::env::var_os(STEALTH_VARIABLE).as_deref())
    }

    pub fn from_launch() -> Self {
        if !ENABLED {
            return Self::default();
        }
        let arguments = std::env::args_os().filter_map(|argument| argument.into_string().ok());
        Self::parse(arguments, std::env::var_os(DRIVE_DIR_VARIABLE))
            .with_stealth(Self::stealth_from_launch())
    }

    pub fn sound_muted(&self) -> bool {
        self.sound_muted
    }

    pub fn drive_dir(&self) -> Option<&Path> {
        self.drive_dir.as_deref()
    }

    pub fn stealth(&self) -> bool {
        self.stealth
    }

    pub fn install(self, cx: &mut App) {
        sound_mute::set_sound_muted(self.sound_muted);
        if ENABLED && self.stealth {
            install_tester_sources();
        }
        if let Some(dir) = self.drive_dir.clone() {
            if ENABLED {
                cx.set_prompt_builder(rise_ui::in_window_prompt::in_window_prompt_renderer);
                serve_commands(dir.clone(), cx);
            }
            serve_snapshots(dir, cx);
        }
        #[cfg(feature = "dev-drive")]
        {
            if ENABLED && self.stealth {
                conceal_windows(cx);
            }
        }
        cx.set_global(self);
    }

    /// asked by audio before it starts anything, false when no drive was installed
    pub fn is_sound_muted(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(Self::sound_muted)
    }

    /// asked before the app activates itself or posts a notification
    /// always false outside a debug build with dev-drive
    pub fn is_stealth(cx: &App) -> bool {
        ENABLED && cx.try_global::<Self>().is_some_and(Self::stealth)
    }
}

pub fn tester_photo_dir(value: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    value
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.filter(|home| !home.is_empty()).map(|home| PathBuf::from(home).join(DEFAULT_PHOTO_DIR)))
}

fn install_tester_sources() {
    rise_platform::capture_permission::set_tester_sources(true);
    rise_platform::notifications::set_tester_receiver(true);
    rise_media::audio_capture::set_tester_signal(true);
    rise_media::camera_capture::tester_camera::set_tester_camera(true);
    match tester_photo_dir(std::env::var_os(PHOTO_DIR_VARIABLE), std::env::var_os("HOME")) {
        Some(dir) => {
            rise_platform::photo_library::tester_folder_library::install_tester_library(dir, tester_video_poster);
        }
        None => tracing::warn!(target: "riseonly", "no tester photo folder: neither RISE_DEV_PHOTO_DIR nor HOME is set"),
    }
    let (latitude, longitude) = tester_location(std::env::var_os(LOCATION_VARIABLE));
    if !rise_platform::location::install_tester_location(latitude, longitude) {
        tracing::warn!(target: "riseonly", latitude, longitude, "tester location not installed");
    }
    rise_platform::file_dialog::enable_tester_open_answers();
    rise_platform::open_document::set_tester_receiver(true);
    match std::env::var_os("HOME").filter(|home| !home.is_empty()) {
        Some(home) => {
            rise_platform::file_dialog::set_tester_save_directory(PathBuf::from(home).join(TESTER_SAVE_DIR));
        }
        None => tracing::warn!(target: "riseonly", "no tester save folder: HOME is not set"),
    }
}

pub fn tester_location(value: Option<OsString>) -> (f64, f64) {
    value
        .and_then(|value| {
            let value = value.to_str()?.to_owned();
            let (latitude, longitude) = value.split_once(',')?;
            let latitude = trimming_whitespaces_and_newlines(latitude).parse::<f64>().ok()?;
            let longitude = trimming_whitespaces_and_newlines(longitude).parse::<f64>().ok()?;
            (latitude.is_finite() && longitude.is_finite()).then_some((latitude, longitude))
        })
        .unwrap_or(DEFAULT_TESTER_LOCATION)
}

fn tester_video_poster(path: &Path, max_side: u32) -> Option<rise_platform::photo_library::LibraryImage> {
    let poster = rise_media::video_export::poster(path, 0.0, max_side.max(1)).ok()?;
    Some(rise_platform::photo_library::LibraryImage {
        width: poster.width,
        height: poster.height,
        bgra: poster.bgra,
    })
}

/// only the exact value 1 turns stealth on
pub fn stealth_requested(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| value == "1")
}

#[cfg(feature = "dev-drive")]
fn conceal_windows(cx: &mut App) {
    let (wake, mut woken) = futures::channel::mpsc::unbounded::<()>();
    let wake = std::sync::Mutex::new(wake);
    let watch = rise_platform::window_chrome::watch_windows_for_dev_drive(Box::new(move || {
        if let Ok(wake) = wake.lock() {
            let _ = wake.unbounded_send(());
        }
    }));
    cx.defer(conceal_pass);
    cx.spawn(async move |cx| {
        let _watch = watch;
        while futures::StreamExt::next(&mut woken).await.is_some() {
            cx.update(conceal_pass);
        }
    })
    .detach();
}

#[cfg(feature = "dev-drive")]
fn conceal_pass(cx: &mut App) {
    let _ = rise_platform::window_chrome::keep_app_in_background_for_dev_drive();
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, _| {
            rise_platform::window_chrome::conceal_window_for_dev_drive(window)
        });
    }
}

/// the request body names the snapshot
/// it is cut to a bare file stem so nothing writes outside the drive dir
pub fn snapshot_stem(body: &str) -> String {
    let stem: String = body
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if stem.is_empty() {
        DEFAULT_STEM.to_owned()
    } else {
        stem
    }
}

pub fn snapshot_path(dir: &Path, stem: &str, index: usize) -> PathBuf {
    dir.join(format!("{stem}-{index}.png"))
}

pub fn take_request(dir: &Path) -> Option<String> {
    let body = take_file(&dir.join(SNAPSHOT_REQUEST))?;
    Some(snapshot_stem(&String::from_utf8_lossy(&body)))
}

pub fn take_command(dir: &Path) -> Option<Vec<u8>> {
    take_file(&dir.join(COMMAND_REQUEST))
}

fn take_file(request: &Path) -> Option<Vec<u8>> {
    let body = fs::read(request).ok()?;
    if let Err(error) = fs::remove_file(request) {
        tracing::warn!(target: "riseonly", "dev-drive: cannot remove {}: {error}", request.display());
        return None;
    }
    Some(body)
}

pub fn snapshot_windows(dir: &Path, stem: &str, cx: &mut App) -> Vec<Result<PathBuf, String>> {
    cx.windows()
        .into_iter()
        .enumerate()
        .map(|(index, handle)| {
            let path = snapshot_path(dir, stem, index);
            render_window(handle, &path, cx).map(|()| path)
        })
        .collect()
}

/// writes the report whole or not at all, written aside then renamed so scripts never read half a line
pub fn write_report(dir: &Path, results: &[Result<PathBuf, String>]) -> std::io::Result<()> {
    let mut report = String::new();
    for result in results {
        match result {
            Ok(path) => report.push_str(&format!("ok {}\n", path.display())),
            Err(error) => report.push_str(&format!("error {error}\n")),
        }
    }
    let partial = dir.join(SNAPSHOT_DONE_PARTIAL);
    fs::write(&partial, report)?;
    fs::rename(partial, dir.join(SNAPSHOT_DONE))
}

#[cfg(any(test, feature = "dev-drive"))]
fn render_window(handle: AnyWindowHandle, path: &Path, cx: &mut App) -> Result<(), String> {
    let image = handle
        .update(cx, |_, window, _| window.render_to_image())
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    image.save(path).map_err(|error| error.to_string())
}

#[cfg(not(any(test, feature = "dev-drive")))]
fn render_window(_handle: AnyWindowHandle, _path: &Path, _cx: &mut App) -> Result<(), String> {
    Err(NOT_BUILT.to_owned())
}

fn serve_snapshots(dir: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let Some(stem) = take_request(&dir) else {
                continue;
            };
            let results = cx.update(|cx| snapshot_windows(&dir, &stem, cx));
            if let Err(error) = write_report(&dir, &results) {
                tracing::warn!(
                    target: "riseonly",
                    "dev-drive: snapshot report not written to {}: {error}",
                    dir.display()
                );
            }
        }
    })
    .detach();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl DriveButton {
    pub fn mouse_button(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle,
        }
    }
}

fn one_click() -> usize {
    1
}

fn drag_steps() -> usize {
    DRAG_STEPS
}

fn swipe_steps() -> usize {
    SWIPE_STEPS
}

fn swipe_ms() -> u64 {
    SWIPE_MS
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DriveCommand {
    Click {
        x: f32,
        y: f32,
        #[serde(default)]
        button: DriveButton,
        #[serde(default = "one_click")]
        count: usize,
        #[serde(default)]
        hold_ms: u64,
        #[serde(default)]
        snap_at_ms: Option<u64>,
        #[serde(default)]
        snap_stem: Option<String>,
        #[serde(default)]
        minimise_at_ms: Option<u64>,
        #[serde(default)]
        restore_at_ms: Option<u64>,
    },
    Move {
        x: f32,
        y: f32,
    },
    Scroll {
        x: f32,
        y: f32,
        dy: f32,
        #[serde(default)]
        dx: f32,
    },
    Swipe {
        x: f32,
        y: f32,
        #[serde(default)]
        dx: f32,
        #[serde(default)]
        dy: f32,
        #[serde(default = "swipe_steps")]
        steps: usize,
        #[serde(default = "swipe_ms")]
        ms: u64,
        #[serde(default)]
        momentum: bool,
        #[serde(default)]
        lines: bool,
    },
    Type {
        text: String,
    },
    Key {
        keys: String,
    },
    Snapshot {
        #[serde(default)]
        stem: String,
    },
    Info,
    OpenUrl {
        url: String,
    },
    Share {
        #[serde(default)]
        paths: Vec<String>,
    },
    ShareText {
        text: String,
    },
    Files {
        #[serde(default)]
        paths: Vec<String>,
    },
    Net {
        state: DriveNetState,
    },
    Drag {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        #[serde(default = "drag_steps")]
        steps: usize,
        #[serde(default)]
        hold_ms: u64,
        #[serde(default)]
        then: Vec<(f32, f32)>,
        #[serde(default)]
        minimise_at_ms: Option<u64>,
        #[serde(default)]
        restore_at_ms: Option<u64>,
    },
    Location {
        #[serde(default)]
        latitude: Option<f64>,
        #[serde(default)]
        longitude: Option<f64>,
    },
    Window {
        action: DriveWindowAction,
    },
    ResizeWindow {
        width: f32,
        height: f32,
    },
    Notifications {
        #[serde(default)]
        op: DriveNotificationOp,
        #[serde(default, rename = "notification")]
        id: Option<u64>,
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        text: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveNotificationOp {
    #[default]
    List,
    Click,
    Action,
    Reply,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DriveNotificationButton {
    pub id: String,
    pub label: String,
    pub reply: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DriveNotificationInfo {
    pub id: u64,
    pub tag: String,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<PathBuf>,
    pub buttons: Vec<DriveNotificationButton>,
    pub sound: bool,
}

impl From<rise_platform::notifications::TesterNotification> for DriveNotificationInfo {
    fn from(held: rise_platform::notifications::TesterNotification) -> Self {
        Self {
            id: held.id,
            tag: held.tag,
            title: held.title,
            body: held.body,
            category: held.category,
            thread: held.thread,
            sender: held.sender,
            avatar: held.avatar,
            attachment: held.attachment,
            buttons: held
                .actions
                .into_iter()
                .map(|action| DriveNotificationButton {
                    reply: action.text_input.is_some(),
                    id: action.id,
                    label: action.label,
                })
                .collect(),
            sound: held.sound,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveWindowAction {
    Minimise,
    Restore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveNetState {
    Off,
    On,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct DriveRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub window: Option<usize>,
    #[serde(flatten)]
    pub command: DriveCommand,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DriveWindowInfo {
    pub index: usize,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub active: bool,
    pub minimised: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DriveReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub snapshot_errors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handled: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<DriveWindowInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<&'static str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub open_panel_waiting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifications: Option<Vec<DriveNotificationInfo>>,
}

impl DriveReply {
    pub fn failure(id: Option<String>, error: String) -> Self {
        Self { id, error: Some(error), ..Self::default() }
    }
}

#[derive(Deserialize)]
struct RequestIdOnly {
    #[serde(default)]
    id: Option<String>,
}

/// parses a drive command, the error has the request id when it could be read
pub fn parse_command(body: &[u8]) -> Result<DriveRequest, (Option<String>, String)> {
    serde_json::from_slice(body).map_err(|error| {
        let id = serde_json::from_slice::<RequestIdOnly>(body).ok().and_then(|only| only.id);
        (id, format!("bad command: {error}"))
    })
}

/// one character as one keystroke
/// key is what a keymap sees and key_char is what reaches the focused field
pub fn typed_keystroke(character: char) -> Keystroke {
    let (key, key_char) = match character {
        '\n' => ("enter".to_owned(), "\n".to_owned()),
        '\t' => ("tab".to_owned(), "\t".to_owned()),
        ' ' => ("space".to_owned(), " ".to_owned()),
        other => (other.to_lowercase().collect(), other.to_string()),
    };
    Keystroke {
        modifiers: Modifiers { shift: character.is_uppercase(), ..Modifiers::default() },
        key,
        key_char: Some(key_char),
    }
}

/// splits keystrokes by whitespace, eg cmd-shift-d or tab enter
pub fn parse_keys(keys: &str) -> Result<Vec<Keystroke>, String> {
    let parsed = keys
        .split_whitespace()
        .map(|source| Keystroke::parse(source).map_err(|error| format!("bad key {source}: {error:?}")))
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.is_empty() {
        return Err("no keys given".to_owned());
    }
    Ok(parsed)
}

pub fn position_in_viewport(x: f32, y: f32, width: f32, height: f32) -> bool {
    (0.0..width).contains(&x) && (0.0..height).contains(&y)
}

pub fn parse_link(raw: &str) -> Result<Url, String> {
    let owned = raw.to_owned();
    handover::urls_from_links([&owned])
        .into_iter()
        .next()
        .ok_or_else(|| format!("not a link: {raw}"))
}

/// writes the reply whole or not at all, same as the snapshot report
pub fn write_reply(dir: &Path, reply: &DriveReply) -> std::io::Result<()> {
    let partial = dir.join(COMMAND_DONE_PARTIAL);
    fs::write(&partial, serde_json::to_vec(reply)?)?;
    fs::rename(partial, dir.join(COMMAND_DONE))
}

/// picks the window, explicit index first then the active one
/// the app is often in background so it falls back to top of stack then first open window
pub fn target_window(requested: Option<usize>, cx: &App) -> Result<(usize, AnyWindowHandle), String> {
    let open = cx.windows();
    if let Some(index) = requested {
        return open
            .get(index)
            .map(|handle| (index, *handle))
            .ok_or_else(|| format!("no window {index}: {} open", open.len()));
    }
    cx.active_window()
        .into_iter()
        .chain(cx.window_stack().unwrap_or_default())
        .chain(open.iter().copied())
        .find_map(|handle| {
            open.iter().position(|window| *window == handle).map(|index| (index, handle))
        })
        .ok_or_else(|| "no open window".to_owned())
}

pub fn resize_size_valid(width: f32, height: f32) -> bool {
    [width, height].iter().all(|side| side.is_finite() && *side >= 1.0 && *side <= RESIZE_MAX_SIDE)
}

async fn drive_resize(handle: AnyWindowHandle, width: f32, height: f32, cx: &AsyncApp) -> Result<(), String> {
    let wanted = size(px(width), px(height));
    cx.update(|cx| handle.update(cx, |_, window, _| window.resize(wanted)))
        .map_err(|error| error.to_string())?;
    for frame in 1..=RESIZE_FRAMES {
        settle(handle, cx).await?;
        let now = cx
            .update(|cx| handle.update(cx, |_, window, _| window.viewport_size()))
            .map_err(|error| error.to_string())?;
        let reached = (f32::from(now.width) - width).abs() < 0.5 && (f32::from(now.height) - height).abs() < 0.5;
        if reached || frame == RESIZE_FRAMES {
            tracing::info!(
                target: "riseonly",
                width,
                height,
                got_width = f32::from(now.width),
                got_height = f32::from(now.height),
                "dev-drive: window resized"
            );
            break;
        }
    }
    Ok(())
}

pub fn window_infos(cx: &mut App) -> Vec<DriveWindowInfo> {
    cx.windows()
        .into_iter()
        .enumerate()
        .filter_map(|(index, handle)| {
            handle
                .update(cx, |_, window, _| {
                    let size = window.viewport_size();
                    DriveWindowInfo {
                        index,
                        width: f32::from(size.width),
                        height: f32::from(size.height),
                        scale: window.scale_factor(),
                        active: window.is_window_active(),
                        minimised: window_minimised(window),
                    }
                })
                .ok()
        })
        .collect()
}

#[cfg(feature = "dev-drive")]
fn window_minimised(window: &gpui::Window) -> bool {
    rise_platform::window_minimise::minimised_for_dev_drive(window).unwrap_or(false)
}

#[cfg(not(feature = "dev-drive"))]
fn window_minimised(_window: &gpui::Window) -> bool {
    false
}

#[cfg(feature = "dev-drive")]
fn drive_window(handle: AnyWindowHandle, action: DriveWindowAction, cx: &mut App) -> Result<(), String> {
    let minimised = action == DriveWindowAction::Minimise;
    let reported = handle
        .update(cx, |_, window, _| rise_platform::window_minimise::report_for_dev_drive(&*window, minimised))
        .map_err(|error| error.to_string())?;
    if !reported {
        return Err("window: minimise is not observed for this window on this host".to_owned());
    }
    tracing::info!(target: "riseonly", minimised, "dev-drive: window minimise switched");
    Ok(())
}

#[cfg(not(feature = "dev-drive"))]
fn drive_window(_handle: AnyWindowHandle, _action: DriveWindowAction, _cx: &mut App) -> Result<(), String> {
    Err("window: built without the dev-drive feature".to_owned())
}

fn music_playing(cx: &App) -> bool {
    MusicPlayerState::try_read(cx).is_some_and(|player| player.is_playing)
}

fn draw_window(handle: AnyWindowHandle, cx: &mut App) -> Result<(), String> {
    handle
        .update(cx, |_, window, cx| {
            #[cfg(any(test, feature = "dev-drive"))]
            if DevDrive::is_stealth(cx) {
                window.simulate_next_frame(cx);
            }
            if window.needs_draw() {
                window.draw(cx).clear(cx);
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(any(test, feature = "dev-drive"))]
const FOLLOW_FRAMES: u32 = 180;

#[cfg(any(test, feature = "dev-drive"))]
thread_local! {
    static FOLLOW_LEFT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "dev-drive"))]
fn follow_frames(cx: &AsyncApp) {
    if !cx.update(|cx| DevDrive::is_stealth(cx)) {
        return;
    }
    let running = FOLLOW_LEFT.with(|left| left.replace(FOLLOW_FRAMES)) > 0;
    if running {
        return;
    }
    cx.spawn(async move |cx| {
        while FOLLOW_LEFT.with(|left| {
            let frames = left.get();
            left.set(frames.saturating_sub(1));
            frames > 0
        }) {
            cx.background_executor().timer(STEP_GAP).await;
            let busy = cx.update(|cx| {
                let mut busy = false;
                for handle in cx.windows() {
                    busy |= handle
                        .update(cx, |_, window, cx| {
                            let callbacks = window.simulate_next_frame(cx);
                            let dirty = window.needs_draw();
                            if dirty {
                                window.draw(cx).clear(cx);
                            }
                            callbacks > 0 || dirty
                        })
                        .unwrap_or(false);
                }
                busy
            });
            if !busy {
                FOLLOW_LEFT.with(|left| left.set(0));
            }
        }
    })
    .detach();
}

#[cfg(not(any(test, feature = "dev-drive")))]
fn follow_frames(_cx: &AsyncApp) {}

async fn settle(handle: AnyWindowHandle, cx: &AsyncApp) -> Result<(), String> {
    cx.background_executor().timer(STEP_GAP).await;
    cx.update(|cx| draw_window(handle, cx))
}

async fn dispatch(handle: AnyWindowHandle, event: PlatformInput, cx: &AsyncApp) -> Result<(), String> {
    cx.update(|cx| {
        handle.update(cx, |_, window, cx| {
            let _ = window.dispatch_event(event, cx);
        })
    })
    .map_err(|error| error.to_string())?;
    settle(handle, cx).await
}

fn check_position(handle: AnyWindowHandle, x: f32, y: f32, cx: &mut App) -> Result<(), String> {
    let size = handle
        .update(cx, |_, window, _| window.viewport_size())
        .map_err(|error| error.to_string())?;
    let (width, height) = (f32::from(size.width), f32::from(size.height));
    if position_in_viewport(x, y, width, height) {
        Ok(())
    } else {
        Err(format!("point {x},{y} is outside the window {width}x{height}: {COORDINATES}"))
    }
}

fn move_event(x: f32, y: f32) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position: point(px(x), px(y)),
        pressed_button: None,
        modifiers: Modifiers::default(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn drive_click(
    handle: AnyWindowHandle,
    (x, y): (f32, f32),
    button: MouseButton,
    count: usize,
    hold: Duration,
    mut snap: Option<HoldSnapshot<'_>>,
    mut window_plan: HoldWindowPlan,
    cx: &AsyncApp,
) -> Result<Option<DriveReply>, String> {
    let position = point(px(x), px(y));
    let mut snapshot = None;
    settle(handle, cx).await?;
    dispatch(handle, move_event(x, y), cx).await?;
    let pressed_at = RiseClock::monotonic();
    for click_count in 1..=count.max(1) {
        let down = PlatformInput::MouseDown(MouseDownEvent {
            button,
            position,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        dispatch(handle, down, cx).await?;
        if let Some(reply) = hold_down(handle, hold, snap.take(), &mut window_plan, pressed_at, cx).await? {
            snapshot = Some(reply);
        }
        let up = PlatformInput::MouseUp(MouseUpEvent {
            button,
            position,
            modifiers: Modifiers::default(),
            click_count,
        });
        dispatch(handle, up, cx).await?;
    }
    apply_window_plan(handle, &mut window_plan, None, cx)?;
    Ok(snapshot)
}

/// plan for minimising and restoring the window inside a held press, times counted from the press
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HoldWindowPlan {
    pub minimise_at: Option<Duration>,
    pub restore_at: Option<Duration>,
}

impl HoldWindowPlan {
    pub fn from_ms(minimise_at_ms: Option<u64>, restore_at_ms: Option<u64>) -> Self {
        Self {
            minimise_at: minimise_at_ms.map(Duration::from_millis),
            restore_at: restore_at_ms.map(Duration::from_millis),
        }
    }

    /// returns the window switches due after held time, each only once
    /// a restore never goes before a minimise still waiting
    pub fn take_due(&mut self, held: Duration) -> Vec<DriveWindowAction> {
        let mut due = Vec::new();
        if self.minimise_at.is_some_and(|at| held >= at) {
            self.minimise_at = None;
            due.push(DriveWindowAction::Minimise);
        }
        if self.minimise_at.is_none() && self.restore_at.is_some_and(|at| held >= at) {
            self.restore_at = None;
            due.push(DriveWindowAction::Restore);
        }
        due
    }
}

fn apply_window_plan(
    handle: AnyWindowHandle,
    plan: &mut HoldWindowPlan,
    since: Option<Duration>,
    cx: &AsyncApp,
) -> Result<(), String> {
    let held = since.map_or(Duration::MAX, |since| RiseClock::monotonic().saturating_sub(since));
    for action in plan.take_due(held) {
        cx.update(|cx| drive_window(handle, action, cx))?;
    }
    Ok(())
}

/// one wheel event of a driven gesture and the wait after it
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwipeStep {
    pub phase: TouchPhase,
    pub delta: (f32, f32),
    pub gap: Duration,
}

/// wheel events of a swipe, finger part spread evenly over ms
/// the last move lands the total exactly then a momentum tail decays from the last speed
pub fn swipe_events(dx: f32, dy: f32, steps: usize, ms: u64, momentum: bool, lines: bool) -> Vec<SwipeStep> {
    let steps = steps.max(1);
    let gap = Duration::from_millis(ms) / steps as u32;
    let step = (dx / steps as f32, dy / steps as f32);
    let before_last = (steps - 1) as f32;
    let moved = (1..=steps).map(|index| {
        let delta = if index == steps { (dx - step.0 * before_last, dy - step.1 * before_last) } else { step };
        SwipeStep { phase: TouchPhase::Moved, delta, gap }
    });
    if lines {
        return moved.collect();
    }
    let mut events = vec![SwipeStep { phase: TouchPhase::Started, delta: (0.0, 0.0), gap }];
    events.extend(moved);
    events.push(SwipeStep { phase: TouchPhase::Ended, delta: (0.0, 0.0), gap: MOMENTUM_GAP });
    if momentum {
        let finger_gap = gap.as_secs_f32();
        let scale = if finger_gap > 0.0 { MOMENTUM_GAP.as_secs_f32() / finger_gap } else { 1.0 };
        let mut speed = (step.0 * scale, step.1 * scale);
        for _ in 0..MOMENTUM_MAX_FRAMES {
            speed = (speed.0 * MOMENTUM_DECAY, speed.1 * MOMENTUM_DECAY);
            if speed.0.abs().max(speed.1.abs()) < MOMENTUM_MIN_STEP {
                break;
            }
            events.push(SwipeStep { phase: TouchPhase::Moved, delta: speed, gap: MOMENTUM_GAP });
        }
    }
    events
}

async fn drive_swipe(
    handle: AnyWindowHandle,
    (x, y): (f32, f32),
    events: &[SwipeStep],
    lines: bool,
    cx: &AsyncApp,
) -> Result<(), String> {
    settle(handle, cx).await?;
    dispatch(handle, move_event(x, y), cx).await?;
    let position = point(px(x), px(y));
    for event in events {
        let (dx, dy) = event.delta;
        let delta = if lines { ScrollDelta::Lines(point(dx, dy)) } else { ScrollDelta::Pixels(point(px(dx), px(dy))) };
        let wheel = PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta,
            modifiers: Modifiers::default(),
            touch_phase: event.phase,
        });
        cx.update(|cx| {
            handle.update(cx, |_, window, cx| {
                let _ = window.dispatch_event(wheel, cx);
            })
        })
        .map_err(|error| error.to_string())?;
        cx.background_executor().timer(event.gap).await;
        cx.update(|cx| draw_window(handle, cx))?;
    }
    Ok(())
}

/// snapshot taken while a held button is still down
struct HoldSnapshot<'a> {
    at: Duration,
    stem: &'a str,
    dir: &'a Path,
    window: usize,
}

async fn hold_down(
    handle: AnyWindowHandle,
    hold: Duration,
    mut snap: Option<HoldSnapshot<'_>>,
    window_plan: &mut HoldWindowPlan,
    gesture_from: Duration,
    cx: &AsyncApp,
) -> Result<Option<DriveReply>, String> {
    let pressed_at = RiseClock::monotonic();
    let held = || RiseClock::monotonic().saturating_sub(pressed_at);
    let mut snapshot = None;
    while let Some(left) = hold.checked_sub(held()).filter(|left| !left.is_zero()) {
        cx.background_executor().timer(left.min(HOLD_FRAME)).await;
        if held() >= hold {
            break;
        }
        apply_window_plan(handle, window_plan, Some(gesture_from), cx)?;
        cx.update(|cx| draw_window(handle, cx))?;
        if snap.as_ref().is_some_and(|snap| held() >= snap.at)
            && let Some(snap) = snap.take()
        {
            snapshot = Some(cx.update(|cx| drive_snapshot(snap.dir, snap.stem, snap.window, cx))?);
        }
    }
    if let Some(snap) = snap.take() {
        snapshot = Some(cx.update(|cx| drive_snapshot(snap.dir, snap.stem, snap.window, cx))?);
    }
    Ok(snapshot)
}

/// points a drag passes after the press, the last one is exactly to
pub fn drag_points(from: (f32, f32), to: (f32, f32), steps: usize) -> Vec<(f32, f32)> {
    let steps = steps.max(1);
    (1..=steps)
        .map(|step| {
            if step == steps {
                return to;
            }
            let part = step as f32 / steps as f32;
            (from.0 + (to.0 - from.0) * part, from.1 + (to.1 - from.1) * part)
        })
        .collect()
}

/// points of a drag along waypoints after the press, steps per segment
pub fn drag_path(from: (f32, f32), waypoints: &[(f32, f32)], steps: usize) -> Vec<(f32, f32)> {
    let mut start = from;
    let mut points = Vec::new();
    for &waypoint in waypoints {
        points.extend(drag_points(start, waypoint, steps));
        start = waypoint;
    }
    points
}

async fn drive_drag(
    handle: AnyWindowHandle,
    from: (f32, f32),
    waypoints: &[(f32, f32)],
    steps: usize,
    hold: Duration,
    mut window_plan: HoldWindowPlan,
    cx: &AsyncApp,
) -> Result<(), String> {
    let button = MouseButton::Left;
    settle(handle, cx).await?;
    dispatch(handle, move_event(from.0, from.1), cx).await?;
    let down = PlatformInput::MouseDown(MouseDownEvent {
        button,
        position: point(px(from.0), px(from.1)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    dispatch(handle, down, cx).await?;
    let pressed_at = RiseClock::monotonic();
    hold_down(handle, hold, None, &mut window_plan, pressed_at, cx).await?;
    let to = waypoints.last().copied().unwrap_or(from);
    for (x, y) in drag_path(from, waypoints, steps) {
        let moved = PlatformInput::MouseMove(MouseMoveEvent {
            position: point(px(x), px(y)),
            pressed_button: Some(button),
            modifiers: Modifiers::default(),
        });
        dispatch(handle, moved, cx).await?;
        apply_window_plan(handle, &mut window_plan, Some(pressed_at), cx)?;
    }
    let up = PlatformInput::MouseUp(MouseUpEvent {
        button,
        position: point(px(to.0), px(to.1)),
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    dispatch(handle, up, cx).await?;
    apply_window_plan(handle, &mut window_plan, None, cx)
}

async fn drive_keystrokes(
    handle: AnyWindowHandle,
    keystrokes: Vec<Keystroke>,
    cx: &AsyncApp,
) -> Result<usize, String> {
    let mut handled = 0;
    for keystroke in keystrokes {
        let taken = cx
            .update(|cx| {
                handle.update(cx, |_, window, cx| {
                    let taken = window.dispatch_keystroke(keystroke.clone(), cx);
                    if !taken {
                        tracing::warn!(
                            target: "riseonly::X735",
                            "drive key {:?} not taken, focused {:?} active {}",
                            keystroke.key,
                            window.focused(cx),
                            window.is_window_active(),
                        );
                    }
                    taken
                })
            })
            .map_err(|error| error.to_string())?;
        if taken {
            handled += 1;
        }
        cx.background_executor().timer(KEY_GAP).await;
    }
    Ok(handled)
}

fn drive_snapshot(dir: &Path, stem: &str, target: usize, cx: &mut App) -> Result<DriveReply, String> {
    for handle in cx.windows() {
        draw_window(handle, cx)?;
    }
    let results = snapshot_windows(dir, &snapshot_stem(stem), cx);
    let path = match results.get(target) {
        Some(Ok(path)) => path.clone(),
        Some(Err(error)) => return Err(error.clone()),
        None => return Err(format!("no snapshot of window {target}")),
    };
    let mut reply = DriveReply { window: Some(target), path: Some(path), ..DriveReply::default() };
    for result in results {
        match result {
            Ok(path) => reply.paths.push(path),
            Err(error) => reply.snapshot_errors.push(error),
        }
    }
    Ok(reply)
}

fn drive_net(state: DriveNetState, cx: &mut App) -> Result<(), String> {
    let up = state == DriveNetState::On;
    if !network_transport_config::set_tester_network_down(!up) {
        return Err("net: built without the dev-drive feature".to_owned());
    }
    SaiWebSocketManager::shared(cx)
        .update(cx, |manager, cx| manager.tester_network_changed(up, cx));
    tracing::info!(target: "riseonly", up, "dev-drive: network switched");
    Ok(())
}

fn drive_location(latitude: Option<f64>, longitude: Option<f64>) -> Result<(), String> {
    let applied = match (latitude, longitude) {
        (Some(latitude), Some(longitude)) => {
            if !(latitude.is_finite() && longitude.is_finite()) {
                return Err("loc: latitude and longitude must be numbers".to_owned());
            }
            rise_platform::location::move_tester_location(latitude, longitude)
        }
        (None, None) => rise_platform::location::refuse_tester_location(),
        _ => return Err("loc: both latitude and longitude, or neither for off".to_owned()),
    };
    if !applied {
        return Err("loc: the location is not driven or the point is out of range, start the session with RISE_DEV_STEALTH".to_owned());
    }
    tracing::info!(target: "riseonly", ?latitude, ?longitude, "dev-drive: location switched");
    Ok(())
}

fn drive_open_url(raw: &str, cx: &mut App) -> Result<(), String> {
    let url = parse_link(raw)?;
    let shell = handover::answering_shell(cx).ok_or_else(|| "no shell window".to_owned())?;
    shell
        .update(cx, |view, _, cx| view.on_open_url(url, cx))
        .map_err(|error| error.to_string())
}

/// absolute paths pass as they are, anything else has to be a file url like open with sends
pub fn share_arguments(paths: &[String]) -> Result<DesktopShareArguments, String> {
    let mut split = DesktopShareArguments::default();
    for raw in paths {
        if Path::new(raw).is_absolute() {
            split.files.push(PathBuf::from(raw));
            continue;
        }
        let opened = split_opened_urls(std::slice::from_ref(raw));
        if opened.files.is_empty() {
            return Err(format!("share: not an absolute path or a file url: {raw}"));
        }
        split.extend(opened);
    }
    if split.files.is_empty() {
        return Err("share: no paths".to_owned());
    }
    Ok(split)
}

pub fn tester_response(
    op: DriveNotificationOp,
    button: Option<String>,
    text: Option<String>,
) -> Result<Option<rise_platform::notifications::TesterResponse>, String> {
    use rise_platform::notifications::TesterResponse;
    match op {
        DriveNotificationOp::List => Ok(None),
        DriveNotificationOp::Click => Ok(Some(TesterResponse::Click)),
        DriveNotificationOp::Action => button
            .filter(|button| !button.is_empty())
            .map(|button| Some(TesterResponse::Action(button)))
            .ok_or_else(|| "notifications action: name the button by id or label".to_owned()),
        DriveNotificationOp::Reply => text
            .map(|text| Some(TesterResponse::Reply(text)))
            .ok_or_else(|| "notifications reply: the text to send".to_owned()),
    }
}

fn drive_notifications(
    op: DriveNotificationOp,
    id: Option<u64>,
    button: Option<String>,
    text: Option<String>,
    cx: &mut App,
) -> Result<DriveReply, String> {
    if !rise_platform::notifications::uses_tester_receiver() {
        return Err("notifications: the receiver is off, start the session with RISE_DEV_STEALTH".to_owned());
    }
    if let Some(response) = tester_response(op, button, text)? {
        let id = id.ok_or_else(|| "notifications: the id of the notification from list".to_owned())?;
        rise_platform::notifications::respond_to_tester_notification(id, response, cx)?;
    }
    let held = rise_platform::notifications::tester_notifications();
    Ok(DriveReply {
        notifications: Some(held.into_iter().map(DriveNotificationInfo::from).collect()),
        ..DriveReply::default()
    })
}

fn drive_share_text(text: String, cx: &mut App) -> Result<(), String> {
    if trimming_whitespaces_and_newlines(&text).is_empty() {
        return Err("share-text: empty text".to_owned());
    }
    handover::deliver_incoming_share(IncomingShare { files: Vec::new(), texts: vec![text] }, cx);
    Ok(())
}

async fn run_command(dir: &Path, request: DriveRequest, cx: &AsyncApp) -> Result<DriveReply, String> {
    let requested = request.window;
    let target = || cx.update(|cx| target_window(requested, cx));
    match request.command {
        DriveCommand::Click { x, y, button, count, hold_ms, snap_at_ms, snap_stem, minimise_at_ms, restore_at_ms } => {
            let (index, handle) = target()?;
            cx.update(|cx| check_position(handle, x, y, cx))?;
            let stem = snap_stem.unwrap_or_default();
            let snap = snap_at_ms.map(|at_ms| HoldSnapshot {
                at: Duration::from_millis(at_ms),
                stem: stem.as_str(),
                dir,
                window: index,
            });
            let hold = Duration::from_millis(hold_ms);
            let plan = HoldWindowPlan::from_ms(minimise_at_ms, restore_at_ms);
            let snapshot = drive_click(handle, (x, y), button.mouse_button(), count, hold, snap, plan, cx).await?;
            Ok(snapshot.unwrap_or(DriveReply { window: Some(index), ..DriveReply::default() }))
        }
        DriveCommand::Drag { x1, y1, x2, y2, steps, hold_ms, then, minimise_at_ms, restore_at_ms } => {
            let (index, handle) = target()?;
            let mut waypoints = vec![(x2, y2)];
            waypoints.extend(then);
            cx.update(|cx| check_position(handle, x1, y1, cx))?;
            for &(x, y) in &waypoints {
                cx.update(|cx| check_position(handle, x, y, cx))?;
            }
            let plan = HoldWindowPlan::from_ms(minimise_at_ms, restore_at_ms);
            drive_drag(handle, (x1, y1), &waypoints, steps, Duration::from_millis(hold_ms), plan, cx).await?;
            Ok(DriveReply { window: Some(index), ..DriveReply::default() })
        }
        DriveCommand::Location { latitude, longitude } => {
            drive_location(latitude, longitude)?;
            Ok(DriveReply::default())
        }
        DriveCommand::Move { x, y } => {
            let (index, handle) = target()?;
            cx.update(|cx| check_position(handle, x, y, cx))?;
            settle(handle, cx).await?;
            dispatch(handle, move_event(x, y), cx).await?;
            Ok(DriveReply { window: Some(index), ..DriveReply::default() })
        }
        DriveCommand::Scroll { x, y, dy, dx } => {
            let (index, handle) = target()?;
            cx.update(|cx| check_position(handle, x, y, cx))?;
            settle(handle, cx).await?;
            dispatch(handle, move_event(x, y), cx).await?;
            let wheel = PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: point(px(x), px(y)),
                delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            });
            dispatch(handle, wheel, cx).await?;
            Ok(DriveReply { window: Some(index), ..DriveReply::default() })
        }
        DriveCommand::Swipe { x, y, dx, dy, steps, ms, momentum, lines } => {
            let (index, handle) = target()?;
            cx.update(|cx| check_position(handle, x, y, cx))?;
            let events = swipe_events(dx, dy, steps, ms, momentum, lines);
            drive_swipe(handle, (x, y), &events, lines, cx).await?;
            Ok(DriveReply { window: Some(index), ..DriveReply::default() })
        }
        DriveCommand::Type { text } => {
            let (index, handle) = target()?;
            settle(handle, cx).await?;
            let keystrokes: Vec<Keystroke> = text.chars().map(typed_keystroke).collect();
            let total = keystrokes.len();
            let handled = drive_keystrokes(handle, keystrokes, cx).await?;
            settle(handle, cx).await?;
            if total > 0 && handled == 0 {
                return Err("nothing took the text: no focused input in this window".to_owned());
            }
            Ok(DriveReply {
                window: Some(index),
                handled: Some(handled),
                total: Some(total),
                ..DriveReply::default()
            })
        }
        DriveCommand::Key { keys } => {
            let keystrokes = parse_keys(&keys)?;
            let (index, handle) = target()?;
            settle(handle, cx).await?;
            let total = keystrokes.len();
            let handled = drive_keystrokes(handle, keystrokes, cx).await?;
            settle(handle, cx).await?;
            Ok(DriveReply {
                window: Some(index),
                handled: Some(handled),
                total: Some(total),
                ..DriveReply::default()
            })
        }
        DriveCommand::Snapshot { stem } => {
            let (index, _) = target()?;
            cx.update(|cx| drive_snapshot(dir, &stem, index, cx))
        }
        DriveCommand::Info => {
            let index = target().ok().map(|(index, _)| index);
            Ok(DriveReply {
                window: index,
                windows: cx.update(window_infos),
                coordinates: Some(COORDINATES),
                open_panel_waiting: rise_platform::file_dialog::tester_open_panel_waiting(),
                playing: Some(cx.update(|cx| music_playing(cx))),
                ..DriveReply::default()
            })
        }
        DriveCommand::Window { action } => {
            let (index, handle) = target()?;
            cx.update(|cx| drive_window(handle, action, cx))?;
            Ok(DriveReply { window: Some(index), ..DriveReply::default() })
        }
        DriveCommand::ResizeWindow { width, height } => {
            if !resize_size_valid(width, height) {
                return Err(format!("resize: width and height from 1 to {RESIZE_MAX_SIDE} logical pixels"));
            }
            let (index, handle) = target()?;
            drive_resize(handle, width, height, cx).await?;
            let windows = cx.update(window_infos).into_iter().filter(|info| info.index == index).collect();
            Ok(DriveReply { window: Some(index), windows, ..DriveReply::default() })
        }
        DriveCommand::OpenUrl { url } => {
            cx.update(|cx| drive_open_url(&url, cx))?;
            Ok(DriveReply::default())
        }
        DriveCommand::Share { paths } => {
            let arguments = share_arguments(&paths)?;
            tracing::info!(target: "riseonly", files = arguments.files.len(), "dev-drive: share");
            cx.update(|cx| handover::deliver_arguments(arguments, cx));
            Ok(DriveReply::default())
        }
        DriveCommand::ShareText { text } => {
            cx.update(|cx| drive_share_text(text, cx))?;
            Ok(DriveReply::default())
        }
        DriveCommand::Files { paths } => {
            let open_panel_waiting = rise_platform::file_dialog::tester_open_panel_waiting();
            if !rise_platform::file_dialog::push_tester_open_answer(paths.into_iter().map(PathBuf::from).collect()) {
                return Err("files: the open panel is not driven, start the session with RISE_DEV_STEALTH".to_owned());
            }
            Ok(DriveReply { open_panel_waiting, ..DriveReply::default() })
        }
        DriveCommand::Net { state } => {
            cx.update(|cx| drive_net(state, cx))?;
            Ok(DriveReply::default())
        }
        DriveCommand::Notifications { op, id, button, text } => {
            cx.update(|cx| drive_notifications(op, id, button, text, cx))
        }
    }
}

async fn answer_command(dir: &Path, body: &[u8], cx: &AsyncApp) -> DriveReply {
    let request = match parse_command(body) {
        Ok(request) => request,
        Err((id, error)) => return DriveReply::failure(id, error),
    };
    let id = request.id.clone();
    match run_command(dir, request, cx).await {
        Ok(reply) => DriveReply { id, ok: true, ..reply },
        Err(error) => DriveReply::failure(id, error),
    }
}

fn serve_commands(dir: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(COMMAND_POLL_INTERVAL).await;
            let Some(body) = take_command(&dir) else {
                continue;
            };
            let reply = answer_command(&dir, &body, cx).await;
            if let Err(error) = write_reply(&dir, &reply) {
                tracing::warn!(
                    target: "riseonly",
                    "dev-drive: command answer not written to {}: {error}",
                    dir.display()
                );
            }
            follow_frames(cx);
        }
    })
    .detach();
}

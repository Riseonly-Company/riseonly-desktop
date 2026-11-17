use std::cell::RefCell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    App, Bounds, Context, Div, ElementId, Entity, EntityId, FontWeight, Hsla, IntoElement, ParentElement, Path, PathBuilder,
    Pixels, Point, RenderOnce, SharedString, Styled, Subscription, TransformationMatrix, Window, canvas, div, fill, point, px,
    size, transparent_black,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_ui::IconUi;
use rise_ui::frame_request::is_painted_on_screen;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};
use rise_ui::theme;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::media::rise_media_transfer_hub::{RiseMediaCacheIdentity, RiseMediaTransferKey};
use crate::core::ui::rise_media_transfer_chip::{RiseMediaTransferChip, RiseMediaTransferChipViewMetrics};
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::message_content::chat_message_viewport_width;
use crate::modules::chat::components::chat::bubble::rise_exact_upload_progress_ring::RiseExactUploadProgressRing;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, RemoteMediaURL};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

pub type MessageVoiceViewAction = Rc<dyn Fn(&mut Window, &mut App)>;

const BAR_COUNT: usize = 36;
const BAR_WIDTH: f32 = 2.5;
const BAR_SPACING: f32 = 2.0;
const WAVE_HEIGHT: f32 = 18.0;

const FALLBACK_PEAKS: [f32; 36] = [
    0.28, 0.42, 0.64, 0.36, 0.52, 0.78, 0.44, 0.68, 0.32, 0.56, 0.86, 0.48, 0.72, 0.38, 0.58, 0.92, 0.54, 0.74, 0.40, 0.62, 0.82,
    0.46, 0.70, 0.34, 0.60, 0.88, 0.50, 0.76, 0.42, 0.66, 0.80, 0.36, 0.56, 0.72, 0.44, 0.30,
];

static NEXT_WAVEFORM_SWAP_KEY: AtomicU64 = AtomicU64::new(9 << 60);
const BAR_CORNER_STEPS: usize = 4;

/// takes count swap keys from the waveform range for another waveform of the voice player
pub(crate) fn reserve_waveform_swap_keys(count: usize) -> u64 {
    NEXT_WAVEFORM_SWAP_KEY.fetch_add(count as u64, Ordering::Relaxed)
}

fn scoped_id(parent: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(parent.clone()), SharedString::new_static(name))
}

fn noop_action() -> MessageVoiceViewAction {
    Rc::new(|_: &mut Window, _: &mut App| {})
}

#[derive(IntoElement)]
pub struct MessageVoiceView {
    id: ElementId,
    pub item: MessageMediaItem,
    pub upload_owner_local_id: String,
    pub is_own: bool,
    pub progress: f64,
    pub is_playing: bool,
    pub is_loading: bool,
    pub is_waiting_for_network: bool,
    pub unread: bool,
    pub constrained_content_width: Option<f32>,
    pub live_progress_message_id: Option<String>,
    pub on_toggle_play: MessageVoiceViewAction,
    pub on_cancel_upload: MessageVoiceViewAction,
}

impl MessageVoiceView {
    pub fn new(id: impl Into<ElementId>, item: MessageMediaItem) -> Self {
        Self {
            id: id.into(),
            item,
            upload_owner_local_id: String::new(),
            is_own: false,
            progress: 0.0,
            is_playing: false,
            is_loading: false,
            is_waiting_for_network: false,
            unread: false,
            constrained_content_width: None,
            live_progress_message_id: None,
            on_toggle_play: noop_action(),
            on_cancel_upload: noop_action(),
        }
    }

    pub fn upload_owner_local_id(mut self, upload_owner_local_id: impl Into<String>) -> Self {
        self.upload_owner_local_id = upload_owner_local_id.into();
        self
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn progress(mut self, progress: f64) -> Self {
        self.progress = progress;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn is_loading(mut self, is_loading: bool) -> Self {
        self.is_loading = is_loading;
        self
    }

    pub fn is_waiting_for_network(mut self, is_waiting_for_network: bool) -> Self {
        self.is_waiting_for_network = is_waiting_for_network;
        self
    }

    pub fn unread(mut self, unread: bool) -> Self {
        self.unread = unread;
        self
    }

    pub fn live_progress_message_id(mut self, message_id: impl Into<String>) -> Self {
        self.live_progress_message_id = Some(message_id.into());
        self
    }

    pub fn constrained_content_width(mut self, constrained_content_width: Option<f32>) -> Self {
        self.constrained_content_width = constrained_content_width;
        self
    }

    pub fn on_toggle_play(mut self, on_toggle_play: MessageVoiceViewAction) -> Self {
        self.on_toggle_play = on_toggle_play;
        self
    }

    pub fn on_cancel_upload(mut self, on_cancel_upload: MessageVoiceViewAction) -> Self {
        self.on_cancel_upload = on_cancel_upload;
        self
    }

    fn is_transferring(&self) -> bool {
        self.item.is_uploading || self.item.is_compressing
    }

    fn glyph(sf_symbol: &str, side: f32, color: Hsla, is_filled: bool) -> Option<gpui::Svg> {
        IconUi::sized(sf_symbol, px(side), color, is_filled)
    }

    fn play_button(&self, accent: Hsla, is_layout_measurement: bool) -> Div {
        let circle = accent.opacity(if self.is_own { 0.18 } else { 0.15 });
        let button = div().relative().size(px(44.0)).flex().items_center().justify_center().rounded_full().bg(circle);
        let layer = || div().absolute().top_0().left_0().size(px(44.0));
        if self.is_transferring() {
            return button
                .child(
                    layer().child(
                        RiseExactUploadProgressRing::new(
                            scoped_id(&self.id, "upload-ring"),
                            self.upload_owner_local_id.clone(),
                            self.item.clone(),
                            is_layout_measurement,
                        )
                        .progress_color(accent)
                        .track_color(transparent_black())
                        .line_width(2.5),
                    ),
                )
                .children(Self::glyph("xmark", 12.0, accent, false));
        }
        if self.is_loading && self.is_waiting_for_network {
            return button
                .child(layer().child(VoiceWaitingRing {
                    id: scoped_id(&self.id, "waiting-ring"),
                    color: accent,
                    animates: !is_layout_measurement,
                }))
                .children(Self::glyph("xmark", 12.0, accent, false));
        }
        if self.is_loading {
            return match self.transfer_key() {
                Some(key) => button.child(
                    layer().child(
                        RiseMediaTransferChip::new(scoped_id(&self.id, "transfer"), Some(key))
                            .declared_byte_count(self.item.effective_total_bytes())
                            .metrics(RiseMediaTransferChipViewMetrics::ring(
                                44.0,
                                2.5,
                                Some(accent),
                                Some(accent.opacity(0.24)),
                            ))
                            .shows_byte_counts(false)
                            .shows_idle_affordance(false)
                            .is_interactive(false)
                            .hides_small_transfers(false)
                            .is_layout_measurement(is_layout_measurement),
                    ),
                ),
                None => button.child(progress_view().tint(accent).control_size(ControlSize::Small)),
            };
        }
        let nudge = if self.is_playing { 0.0 } else { 1.0 };
        let symbol = if self.is_playing { "pause.fill" } else { "play.fill" };
        button.child(div().relative().left(px(nudge)).children(Self::glyph(symbol, 16.0, accent, true)))
    }

    /// waveform canvas, bars scale with the width the bubble gives it
    fn waveform(
        id: ElementId,
        peaks: Vec<f32>,
        display_progress: f64,
        accent: Hsla,
        dim: Hsla,
        live_message_id: Option<String>,
    ) -> impl IntoElement {
        let swap_slot = WaveformSwapSlot { id };
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let swap_state = swap_slot.resolve(window);
                paint_waveform(&swap_state, &peaks, display_progress, accent, dim, live_message_id, bounds, window);
            },
        )
        .size_full()
    }

    fn transfer_key(&self) -> Option<RiseMediaTransferKey> {
        if self.is_transferring() || self.item.resolved_local_preview_url().is_some() {
            return None;
        }
        let normalized = RemoteMediaURL::optional_normalized(Some(self.item.url.as_str()))?;
        let remote = Url::from_string(&normalized)?;
        let scheme = remote.scheme()?.to_lowercase();
        if scheme != "https" && scheme != "http" {
            return None;
        }
        Some(RiseMediaTransferKey::for_active_account(RiseMediaCacheIdentity::key(&remote)))
    }

    fn normalized_peaks(&self) -> Vec<f32> {
        let source: &[f32] = if self.item.waveform.is_empty() { &FALLBACK_PEAKS } else { &self.item.waveform };
        if source.len() <= 1 {
            let peak = source.first().copied().unwrap_or(0.35).clamp(0.12, 1.0);
            return vec![peak; BAR_COUNT];
        }
        (0..BAR_COUNT)
            .map(|index| {
                let position = index as f64 / (BAR_COUNT - 1).max(1) as f64 * (source.len() - 1) as f64;
                source[position.round() as usize].clamp(0.12, 1.0)
            })
            .collect()
    }

    fn voice_width(&self, message_viewport_width: f32) -> f32 {
        let min_width = 120.0_f32;
        let available_max_width = min_width.max(
            self.constrained_content_width
                .map_or_else(|| 264.0_f32.min(min_width.max(message_viewport_width - 111.0)), |width| width - 36.0),
        );
        let duration = self.item.duration.unwrap_or(0.0) as f32;
        let normalized_duration = (duration.clamp(2.0, 30.0) - 2.0) / 28.0;
        (min_width + (available_max_width - min_width) * normalized_duration).floor()
    }
}

impl RenderOnce for MessageVoiceView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let current_theme = theme(cx);
        let text_secondary = current_theme.text.secondary;
        let primary = current_theme.primary._100;
        let on_primary = current_theme.content_on(primary);
        let accent = if self.is_own { on_primary } else { primary };
        let dim = if self.is_own { on_primary.opacity(0.42) } else { text_secondary.opacity(0.55) };
        let is_layout_measurement = is_message_layout_measurement(cx);
        let peaks = self.normalized_peaks();
        let width = self.voice_width(chat_message_viewport_width(cx));
        let waveform_width = (width - 60.0).max(1.0);
        let display_progress = if self.is_transferring() { self.item.normalized_transfer_progress() } else { self.progress };
        let live_message_id =
            if self.is_playing && !self.is_transferring() { self.live_progress_message_id.clone() } else { None };
        let duration_color = if self.is_own { on_primary.opacity(0.85) } else { text_secondary };

        let is_transferring = self.is_transferring();
        let on_cancel_upload = self.on_cancel_upload.clone();
        let on_toggle_play = self.on_toggle_play.clone();

        div()
            .relative()
            .w(px(width))
            .h(px(38.0))
            .cursor_pointer()
            .child(div().absolute().left(px(3.0)).top(px(-3.0)).child(self.play_button(accent, is_layout_measurement)))
            .child(
                div()
                    .absolute()
                    .left(px(57.0))
                    .top(px(1.0))
                    .w(px(waveform_width))
                    .h(px(WAVE_HEIGHT))
                    .child(Self::waveform(scoped_id(&self.id, "waveform"), peaks, display_progress, accent, dim, live_message_id)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(57.0))
                    .top(px(22.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        MainText::new(RiseFormat::duration_time_interval(self.item.duration.unwrap_or(0.0)))
                            .px(11.0)
                            .font_weight(FontWeight::NORMAL)
                            .monospaced_digits(true)
                            .color(duration_color),
                    )
                    .children(self.unread.then(|| div().size(px(6.0)).rounded_full().bg(accent))),
            )
            .bubble_tap_target(scoped_id(&self.id, "tap"), move |window: &mut Window, cx: &mut App| {
                if is_transferring {
                    on_cancel_upload(window, cx);
                } else {
                    on_toggle_play(window, cx);
                }
            })
    }
}

/// the part of the voice player a row reads
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct VoiceMessagePlaybackRowPlayback {
    progress: f64,
    is_playing: bool,
    is_loading: bool,
    is_waiting_for_network: bool,
}

impl VoiceMessagePlaybackRowPlayback {
    fn of(player: &ChatVoiceMessagePlayer, message_id: &str) -> Self {
        Self {
            progress: player.progress_for(message_id),
            is_playing: player.is_playing_message_id(message_id),
            is_loading: player.is_loading_message_id(message_id),
            is_waiting_for_network: player.is_waiting_for_network(message_id),
        }
    }
}

struct VoiceMessagePlaybackRowObservation {
    message_id: String,
    playback: VoiceMessagePlaybackRowPlayback,
    _subscription: Subscription,
}

impl VoiceMessagePlaybackRowObservation {
    fn new(message_id: String, player: &Entity<ChatVoiceMessagePlayer>, cx: &mut Context<Self>) -> Self {
        let playback = VoiceMessagePlaybackRowPlayback::of(player.read(cx), &message_id);
        Self {
            message_id,
            playback,
            _subscription: cx.observe(player, |this, player, cx| {
                let playback = VoiceMessagePlaybackRowPlayback::of(player.read(cx), &this.message_id);
                if playback != this.playback {
                    this.playback = playback;
                    cx.notify();
                }
            }),
        }
    }
}

#[derive(IntoElement)]
pub struct VoiceMessagePlaybackRow {
    id: ElementId,
    pub message_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub sender_name: String,
    pub item: MessageMediaItem,
    pub is_own: bool,
    pub unread: bool,
    pub constrained_content_width: Option<f32>,
    pub on_cancel_upload: MessageVoiceViewAction,
}

impl VoiceMessagePlaybackRow {
    pub fn new(
        id: impl Into<ElementId>,
        message_id: impl Into<String>,
        chat_id: impl Into<String>,
        chat_name: impl Into<String>,
        sender_name: impl Into<String>,
        item: MessageMediaItem,
    ) -> Self {
        Self {
            id: id.into(),
            message_id: message_id.into(),
            chat_id: chat_id.into(),
            chat_name: chat_name.into(),
            sender_name: sender_name.into(),
            item,
            is_own: false,
            unread: false,
            constrained_content_width: None,
            on_cancel_upload: noop_action(),
        }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn unread(mut self, unread: bool) -> Self {
        self.unread = unread;
        self
    }

    pub fn constrained_content_width(mut self, constrained_content_width: Option<f32>) -> Self {
        self.constrained_content_width = constrained_content_width;
        self
    }

    pub fn on_cancel_upload(mut self, on_cancel_upload: MessageVoiceViewAction) -> Self {
        self.on_cancel_upload = on_cancel_upload;
        self
    }
}

impl RenderOnce for VoiceMessagePlaybackRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = ChatVoiceMessagePlayer::shared(cx);
        let observed_player = player.clone();
        let observed_message_id = self.message_id.clone();
        let observation = window.use_keyed_state(scoped_id(&self.id, "player"), cx, move |_, cx| {
            VoiceMessagePlaybackRowObservation::new(observed_message_id, &observed_player, cx)
        });
        let playback = VoiceMessagePlaybackRowPlayback::of(player.read(cx), &self.message_id);
        let current_message_id = self.message_id.clone();
        observation.update(cx, |observation, _| {
            observation.message_id = current_message_id;
            observation.playback = playback;
        });

        let is_own = self.is_own;
        let (message_id, chat_id, chat_name, sender_name, item) =
            (self.message_id.clone(), self.chat_id, self.chat_name, self.sender_name, self.item.clone());
        let on_toggle_play: MessageVoiceViewAction = Rc::new(move |_window: &mut Window, cx: &mut App| {
            let player = ChatVoiceMessagePlayer::shared(cx);
            let will_start = !player.read(cx).is_playing_message_id(&message_id);
            player.update(cx, |player, cx| {
                player.toggle(
                    message_id.clone(),
                    chat_id.clone(),
                    chat_name.clone(),
                    Some(sender_name.clone()),
                    item.clone(),
                    cx,
                );
            });
            if will_start && !is_own {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.mark_media_played(&chat_id, &message_id, cx));
            }
        });

        MessageVoiceView::new(scoped_id(&self.id, "view"), self.item)
            .live_progress_message_id(self.message_id.clone())
            .upload_owner_local_id(self.message_id)
            .is_own(is_own)
            .progress(playback.progress)
            .is_playing(playback.is_playing)
            .is_loading(playback.is_loading)
            .is_waiting_for_network(playback.is_waiting_for_network)
            .unread(self.unread)
            .constrained_content_width(self.constrained_content_width)
            .on_toggle_play(on_toggle_play)
            .on_cancel_upload(self.on_cancel_upload)
    }
}

/// base swap key of the newest paint of one waveform
/// element state so nothing observes it and a playing message never notifies the chat screen
struct WaveformSwapState {
    base_key: u64,
}

struct WaveformSwapSlot {
    id: ElementId,
}

impl WaveformSwapSlot {
    fn resolve(&self, window: &mut Window) -> Rc<RefCell<WaveformSwapState>> {
        window.with_global_id(self.id.clone(), |global_id, window| {
            window.with_element_state::<Rc<RefCell<WaveformSwapState>>, _>(global_id, |state, _| {
                let state = state.unwrap_or_else(|| Rc::new(RefCell::new(WaveformSwapState { base_key: 0 })));
                (state.clone(), state)
            })
        })
    }
}

fn waveform_bar_rects(peaks: &[f32], width: f32, height: f32) -> Vec<Bounds<Pixels>> {
    let total_width = BAR_COUNT as f32 * BAR_WIDTH + (BAR_COUNT - 1) as f32 * BAR_SPACING;
    let scale = (width / total_width.max(1.0)).max(0.3);
    let bar_width = BAR_WIDTH * scale;
    let bar_spacing = BAR_SPACING * scale;
    let middle = height / 2.0;
    let mut x = 0.0_f32;
    peaks
        .iter()
        .map(|peak| {
            let bar_height = (peak * height).max(2.0);
            let rect = Bounds::new(point(px(x), px(middle - bar_height / 2.0)), size(px(bar_width), px(bar_height)));
            x += bar_width + bar_spacing;
            rect
        })
        .collect()
}

fn waveform_played_count(display_progress: f64) -> usize {
    (0..BAR_COUNT).filter(|index| *index as f64 / (BAR_COUNT - 1).max(1) as f64 <= display_progress).count()
}

/// one bar as a capsule (radius half its width) moved by origin
pub(crate) fn waveform_bar_path(origin: Point<Pixels>, rect: Bounds<Pixels>) -> Option<Path<Pixels>> {
    let left = f32::from(origin.x + rect.left());
    let top = f32::from(origin.y + rect.top());
    let width = f32::from(rect.size.width);
    let height = f32::from(rect.size.height);
    let radius = (width / 2.0).min(height / 2.0).max(0.0);
    let corners = [
        (left + width - radius, top + radius, -PI / 2.0),
        (left + width - radius, top + height - radius, 0.0),
        (left + radius, top + height - radius, PI / 2.0),
        (left + radius, top + radius, PI),
    ];
    let mut points = Vec::with_capacity(corners.len() * (BAR_CORNER_STEPS + 1));
    for (center_x, center_y, start) in corners {
        for step in 0..=BAR_CORNER_STEPS {
            let angle = start + (PI / 2.0) * step as f32 / BAR_CORNER_STEPS as f32;
            points.push(point(px(center_x + radius * angle.cos()), px(center_y + radius * angle.sin())));
        }
    }
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&points, true);
    builder.build().ok()
}

#[allow(clippy::too_many_arguments)]
fn paint_waveform(
    swap_state: &Rc<RefCell<WaveformSwapState>>,
    peaks: &[f32],
    display_progress: f64,
    accent: Hsla,
    dim: Hsla,
    live_message_id: Option<String>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let rects = Rc::new(waveform_bar_rects(peaks, f32::from(bounds.size.width), f32::from(bounds.size.height)));
    let played = waveform_played_count(display_progress);
    let base_key = NEXT_WAVEFORM_SWAP_KEY.fetch_add(BAR_COUNT as u64, Ordering::Relaxed);
    swap_state.borrow_mut().base_key = base_key;
    for (index, rect) in rects.iter().enumerate() {
        if let Some(path) = waveform_bar_path(bounds.origin, *rect) {
            let color = if index < played { accent } else { dim };
            window.paint_swappable_path(path, color, base_key + index as u64, bounds.origin);
        }
    }
    let Some(message_id) = live_message_id else {
        return;
    };
    if !is_painted_on_screen(bounds, window) || window.transformation() != TransformationMatrix::unit() {
        return;
    }
    WaveformSwaps { swap_state: swap_state.clone(), base_key, rects, accent, dim, message_id, played, view: window.current_view() }
        .schedule(window);
}

struct WaveformSwaps {
    swap_state: Rc<RefCell<WaveformSwapState>>,
    base_key: u64,
    rects: Rc<Vec<Bounds<Pixels>>>,
    accent: Hsla,
    dim: Hsla,
    message_id: String,
    played: usize,
    view: EntityId,
}

impl WaveformSwaps {
    fn schedule(self, window: &Window) {
        window.on_next_frame(move |window, cx| self.tick(window, cx));
    }

    fn tick(mut self, window: &mut Window, cx: &mut App) {
        if self.swap_state.borrow().base_key != self.base_key {
            return;
        }
        let (is_playing, progress) = {
            let player = ChatVoiceMessagePlayer::shared(cx);
            let player = player.read(cx);
            (player.is_playing_message_id(&self.message_id), player.progress_for(&self.message_id))
        };
        if !is_playing {
            return;
        }
        let played = waveform_played_count(progress);
        if played == self.played {
            self.schedule(window);
            return;
        }
        let origin = point(px(0.), px(0.));
        let swaps: Vec<(u64, Path<Pixels>, gpui::Background)> = self
            .rects
            .iter()
            .enumerate()
            .filter_map(|(index, rect)| {
                let color = if index < played { self.accent } else { self.dim };
                waveform_bar_path(origin, *rect).map(|path| (self.base_key + index as u64, path, color.into()))
            })
            .collect();
        if window.swap_paths(swaps) {
            self.played = played;
            self.schedule(window);
            return;
        }
        cx.notify(self.view);
    }
}

#[derive(IntoElement)]
struct VoiceWaitingRing {
    id: ElementId,
    color: Hsla,
    animates: bool,
}

impl VoiceWaitingRing {
    const LINE_WIDTH: f32 = 2.5;
    const INSET: f32 = 1.25 + Self::LINE_WIDTH / 2.0;
    const TRIM_END: f32 = 0.16;

    fn oval_points(bounds: Bounds<Pixels>, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
        let center = bounds.center();
        let radius = f32::from(bounds.size.width) / 2.0 - Self::INSET;
        let steps = ((sweep.abs() / (2.0 * PI)) * 96.0).ceil().max(2.0) as usize;
        (0..=steps)
            .map(|step| {
                let angle = start + sweep * step as f32 / steps as f32;
                point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
            })
            .collect()
    }

    fn stroke(points: &[Point<Pixels>], closed: bool, color: Hsla, window: &mut Window) {
        let mut builder = PathBuilder::stroke(px(Self::LINE_WIDTH));
        builder.add_polygon(points, closed);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }

    /// round line cap since gpui strokes have butt ends
    fn round_cap(at: Point<Pixels>, color: Hsla, window: &mut Window) {
        let half = Self::LINE_WIDTH / 2.0;
        let dot = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(Self::LINE_WIDTH), px(Self::LINE_WIDTH)));
        window.paint_quad(fill(dot, color).corner_radii(px(half)));
    }
}

impl RenderOnce for VoiceWaitingRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let started = window.use_keyed_state(self.id.clone(), cx, |_, _| RiseClock::monotonic());
        let color = self.color;
        let turn = if self.animates {
            window.request_animation_frame();
            RiseClock::monotonic().saturating_sub(*started.read(cx)).as_secs_f32().fract()
        } else {
            0.0
        };
        let rotation = (-90.0 + 360.0 * turn).to_radians();
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let track = Self::oval_points(bounds, 0.0, 2.0 * PI);
                Self::stroke(&track, true, color.opacity(0.24), window);
                let arc = Self::oval_points(bounds, rotation, 2.0 * PI * Self::TRIM_END);
                Self::stroke(&arc, false, color, window);
                if let (Some(first), Some(last)) = (arc.first(), arc.last()) {
                    Self::round_cap(*first, color, window);
                    Self::round_cap(*last, color, window);
                }
            },
        )
        .size_full()
    }
}

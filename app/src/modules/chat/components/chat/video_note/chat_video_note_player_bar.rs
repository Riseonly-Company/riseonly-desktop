use std::cell::RefCell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Background, Bounds, ElementId, EntityId, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Path, PathBuilder, Pixels, Point, RenderOnce, SharedString, Size, StatefulInteractiveElement, Styled,
    TransformationMatrix, Window, canvas, div, point, px, transparent_black,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::frame_request::is_painted_on_screen;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::linear;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Transition};

use crate::modules::chat::components::chat::video_note::video_note_floating_pip::paint_progress_trim;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::{
    ChatMediaPlaybackRate, ChatVoiceMessagePlayer,
};

const ROW_SPACING: f32 = 10.0;
const TITLE_SPACING: f32 = 1.0;
const HORIZONTAL_PADDING: f32 = 8.0;
const VERTICAL_PADDING: f32 = 6.5;
const SENDER_FONT_SIZE: f32 = 13.0;
const LABEL_FONT_SIZE: f32 = 11.0;
const RATE_FONT_SIZE: f32 = 12.0;
const RATE_WIDTH: f32 = 34.0;
const CONTROL_HEIGHT: f32 = 30.0;
const CLOSE_WIDTH: f32 = 44.0;
const CLOSE_ICON_SIZE: f32 = 12.0;
const PLAYBACK_BUTTON_SIZE: f32 = 32.0;
const PLAYBACK_ICON_SIZE: f32 = 14.0;
const PLAY_ICON_OFFSET_X: f32 = 1.0;
const SPINNER_SCALE: f32 = 0.72;
const RING_SIZE: f32 = 25.0;
const RING_LINE_WIDTH: f32 = 2.0;
const RING_MOTION_DURATION: f64 = 0.05;
const FLOATING_STROKE_OPACITY: f64 = 0.12;
const RING_TRIM_STEPS: f32 = 96.0;
const RING_CAP_STEPS: usize = 12;
const RING_MIN_FRACTION: f32 = 0.001;

static NEXT_VOICE_RING_SWAP_KEY: AtomicU64 = AtomicU64::new(0xB << 60);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChatMediaPlaybackKind {
    Voice,
    VideoNote,
}

/// strip above the feed for the voice message or video note playing in this chat
/// same strip on a wide and a narrow window
#[derive(IntoElement)]
pub struct ChatMediaPlaybackBar {
    pub chat_id: String,
}

impl ChatMediaPlaybackBar {
    pub fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into() }
    }

    pub fn active_kind(chat_id: &str, cx: &mut App) -> Option<ChatMediaPlaybackKind> {
        let video_note = VideoNotePlayerStore::shared(cx);
        let video_note = video_note.read(cx);
        if video_note.is_active() && video_note.active_chat_id() == chat_id {
            return Some(ChatMediaPlaybackKind::VideoNote);
        }
        let voice = ChatVoiceMessagePlayer::shared(cx);
        let voice = voice.read(cx);
        if voice.is_active() && voice.active_chat_id() == chat_id {
            return Some(ChatMediaPlaybackKind::Voice);
        }
        None
    }

    pub fn is_active(chat_id: &str, cx: &mut App) -> bool {
        Self::active_kind(chat_id, cx).is_some()
    }

    /// transition of the strip for the presenter presence
    pub fn transition() -> Transition {
        Transition::move_edge(Edge::Top).combined(Transition::opacity())
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(
            Arc::new(ElementId::Name(SharedString::from(format!("chat-media-playback-bar-{}", self.chat_id)))),
            SharedString::from(name),
        )
    }

    fn sender_name(kind: ChatMediaPlaybackKind, cx: &mut App) -> String {
        match kind {
            ChatMediaPlaybackKind::Voice => ChatVoiceMessagePlayer::shared(cx).read(cx).sender_name().to_owned(),
            ChatMediaPlaybackKind::VideoNote => VideoNotePlayerStore::shared(cx).read(cx).sender_name().to_owned(),
        }
    }

    fn media_label(kind: ChatMediaPlaybackKind) -> String {
        match kind {
            ChatMediaPlaybackKind::Voice => tr("chat_inline_type_voice"),
            ChatMediaPlaybackKind::VideoNote => tr("video_note_label"),
        }
    }

    fn progress(kind: ChatMediaPlaybackKind, cx: &mut App) -> f32 {
        match kind {
            ChatMediaPlaybackKind::Voice => ChatVoiceMessagePlayer::shared(cx).read(cx).progress() as f32,
            ChatMediaPlaybackKind::VideoNote => VideoNotePlayerStore::shared(cx).read(cx).progress(),
        }
    }

    fn playback_rate(kind: ChatMediaPlaybackKind, cx: &mut App) -> f32 {
        match kind {
            ChatMediaPlaybackKind::Voice => ChatVoiceMessagePlayer::shared(cx).read(cx).playback_rate(),
            ChatMediaPlaybackKind::VideoNote => VideoNotePlayerStore::shared(cx).read(cx).playback_rate(),
        }
    }

    fn is_playing(kind: ChatMediaPlaybackKind, cx: &mut App) -> bool {
        match kind {
            ChatMediaPlaybackKind::Voice => ChatVoiceMessagePlayer::shared(cx).read(cx).is_playing(),
            ChatMediaPlaybackKind::VideoNote => VideoNotePlayerStore::shared(cx).read(cx).is_playing(),
        }
    }

    fn toggle_playback(kind: ChatMediaPlaybackKind, cx: &mut App) {
        match kind {
            ChatMediaPlaybackKind::Voice => {
                let voice_player = ChatVoiceMessagePlayer::shared(cx);
                if voice_player.read(cx).is_playing() {
                    voice_player.update(cx, |player, cx| player.pause(cx));
                } else {
                    voice_player.update(cx, |player, cx| player.resume(cx)).detach();
                }
            }
            ChatMediaPlaybackKind::VideoNote => {
                VideoNotePlayerStore::shared(cx).update(cx, |store, cx| store.pause_or_resume(cx));
            }
        }
    }

    fn cycle_rate(kind: ChatMediaPlaybackKind, cx: &mut App) {
        match kind {
            ChatMediaPlaybackKind::Voice => {
                ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| player.cycle_playback_rate(cx));
            }
            ChatMediaPlaybackKind::VideoNote => {
                VideoNotePlayerStore::shared(cx).update(cx, |store, cx| store.cycle_playback_rate(cx));
            }
        }
    }

    fn stop(kind: ChatMediaPlaybackKind, cx: &mut App) {
        match kind {
            ChatMediaPlaybackKind::Voice => {
                ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| player.stop(cx));
            }
            ChatMediaPlaybackKind::VideoNote => {
                VideoNotePlayerStore::shared(cx).update(cx, |store, cx| store.stop(cx));
            }
        }
    }

    fn playback_button(&self, kind: ChatMediaPlaybackKind, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let is_loading = kind == ChatMediaPlaybackKind::Voice && ChatVoiceMessagePlayer::shared(cx).read(cx).is_loading();
        let is_playing = Self::is_playing(kind, cx);
        let symbol = if is_playing { "pause.fill" } else { "play.fill" };
        let offset_x = if is_playing { 0.0 } else { PLAY_ICON_OFFSET_X };
        let accent = theme.primary._100;

        div()
            .id(self.child_id("playback"))
            .size(px(PLAYBACK_BUTTON_SIZE))
            .flex_none()
            .rounded_full()
            .bg(theme.bg._300)
            .flex()
            .items_center()
            .justify_center()
            .when(!is_loading, |button| {
                button.cursor_pointer().on_click(move |_, _, cx| Self::toggle_playback(kind, cx))
            })
            .map(|button| {
                if is_loading {
                    button.child(div().child(progress_view().tint(accent).scale_effect(SPINNER_SCALE)))
                } else {
                    button.child(
                        div()
                            .relative()
                            .left(px(offset_x))
                            .children(IconUi::sized(symbol, px(PLAYBACK_ICON_SIZE), accent, true)),
                    )
                }
            })
            .into_any_element()
    }

    fn voice_progress_ring(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let (progress, is_playing) = {
            let player = ChatVoiceMessagePlayer::shared(cx);
            let player = player.read(cx);
            (player.progress() as f32, player.is_playing())
        };
        let track = theme.bg._300;
        let accent = theme.primary._100;
        let swap_slot = VoiceRingSwapSlot { id: self.child_id("voice-ring") };

        div()
            .size(px(RING_SIZE))
            .flex_none()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        paint_progress_trim(bounds, 1.0, RING_LINE_WIDTH, track, window);
                        let swap_state = swap_slot.resolve(window);
                        paint_voice_ring(&swap_state, progress, is_playing, accent, bounds, window);
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn progress_ring(&self, kind: ChatMediaPlaybackKind, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        if kind == ChatMediaPlaybackKind::Voice {
            return self.voice_progress_ring(theme, cx);
        }
        let progress = Self::progress(kind, cx);
        let state = window.use_keyed_state(self.child_id("ring"), cx, |_, _| Animated::new(progress));
        let now = RiseClock::monotonic();
        let shown = state.update(cx, |animated, cx| {
            animated.set_in(progress, linear(RING_MOTION_DURATION), now, cx);
            animated.frame_value(now, window)
        });
        let track = theme.bg._300;
        let accent = theme.primary._100;

        div().size(px(RING_SIZE)).flex_none().child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    paint_progress_trim(bounds, 1.0, RING_LINE_WIDTH, track, window);
                    paint_progress_trim(bounds, shown, RING_LINE_WIDTH, accent, window);
                },
            )
            .size_full(),
        )
        .into_any_element()
    }
}

impl RenderOnce for ChatMediaPlaybackBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(kind) = Self::active_kind(&self.chat_id, cx) else {
            return div().into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        let sender_name = Self::sender_name(kind, cx);
        let rate_label = ChatMediaPlaybackRate::label(Self::playback_rate(kind, cx));

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ROW_SPACING))
            .px(px(HORIZONTAL_PADDING))
            .py(px(VERTICAL_PADDING))
            .child(self.playback_button(kind, &theme, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(TITLE_SPACING))
                    .min_w_0()
                    .flex_1()
                    .child(
                        MainText::new(sender_name)
                            .px(SENDER_FONT_SIZE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .child(
                        MainText::new(Self::media_label(kind))
                            .px(LABEL_FONT_SIZE)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
            )
            .child(self.progress_ring(kind, &theme, window, cx))
            .child(
                div()
                    .id(self.child_id("rate"))
                    .w(px(RATE_WIDTH))
                    .h(px(CONTROL_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| Self::cycle_rate(kind, cx))
                    .child(
                        MainText::new(rate_label)
                            .px(RATE_FONT_SIZE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.secondary),
                    ),
            )
            .child(
                div()
                    .id(self.child_id("close"))
                    .w(px(CLOSE_WIDTH))
                    .h(px(CONTROL_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| Self::stop(kind, cx))
                    .tooltip_label(tr("close"))
                    .children(IconUi::sized("xmark", px(CLOSE_ICON_SIZE), theme.text.secondary, false)),
            )
            .into_any_element()
    }
}

/// a voice message keeps playing after its chat is closed
/// outside the chat its controls take the music mini player place
#[derive(IntoElement, Default)]
pub struct ChatVoiceFloatingPlayerBar;

impl ChatVoiceFloatingPlayerBar {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for ChatVoiceFloatingPlayerBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let voice_player = ChatVoiceMessagePlayer::shared(cx);
        let voice_player = voice_player.read(cx);
        if !voice_player.is_active_in_chat() {
            return div().into_any_element();
        }
        let chrome = div()
            .child(ChatMediaPlaybackBar::new(voice_player.active_chat_id()))
            .liquid_chrome_capsule_with(FLOATING_STROKE_OPACITY, true, None, false);
        div().rounded_full().overflow_hidden().child(chrome).into_any_element()
    }
}

#[derive(IntoElement, Default)]
pub struct ChatVideoNotePlayerBar;

impl ChatVideoNotePlayerBar {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for ChatVideoNotePlayerBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = VideoNotePlayerStore::shared(cx);
        let store = store.read(cx);
        if !store.is_active() {
            return div().into_any_element();
        }
        ChatMediaPlaybackBar::new(store.active_chat_id()).into_any_element()
    }
}

/// base swap key of the newest paint of the voice ring, element state nothing observes
struct VoiceRingSwapState {
    base_key: u64,
}

struct VoiceRingSwapSlot {
    id: ElementId,
}

impl VoiceRingSwapSlot {
    fn resolve(&self, window: &mut Window) -> Rc<RefCell<VoiceRingSwapState>> {
        window.with_global_id(self.id.clone(), |global_id, window| {
            window.with_element_state::<Rc<RefCell<VoiceRingSwapState>>, _>(global_id, |state, _| {
                let state = state.unwrap_or_else(|| Rc::new(RefCell::new(VoiceRingSwapState { base_key: 0 })));
                (state.clone(), state)
            })
        })
    }
}

/// progress trim and its two round caps as paths moved by origin
fn voice_ring_paths(origin: Point<Pixels>, ring: Size<Pixels>, progress: f32) -> Option<(Path<Pixels>, Path<Pixels>)> {
    let fraction = progress.clamp(RING_MIN_FRACTION, 1.0);
    let center = point(origin.x + ring.width / 2.0, origin.y + ring.height / 2.0);
    let radius = f32::from(ring.width.min(ring.height)) / 2.0;
    let sweep = 2.0 * PI * fraction;
    let start = -PI / 2.0;
    let steps = ((fraction * RING_TRIM_STEPS).ceil() as usize).max(2);
    let points: Vec<Point<Pixels>> = (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect();
    let mut trim = PathBuilder::stroke(px(RING_LINE_WIDTH));
    trim.add_polygon(&points, false);
    let trim = trim.build().ok()?;
    let half = RING_LINE_WIDTH / 2.0;
    let mut caps = PathBuilder::fill();
    for end in [points.first(), points.last()].into_iter().flatten() {
        let disc: Vec<Point<Pixels>> = (0..RING_CAP_STEPS)
            .map(|step| {
                let angle = 2.0 * PI * step as f32 / RING_CAP_STEPS as f32;
                point(end.x + px(half * angle.cos()), end.y + px(half * angle.sin()))
            })
            .collect();
        caps.add_polygon(&disc, true);
    }
    let caps = caps.build().ok()?;
    Some((trim, caps))
}

fn voice_ring_color(progress: f32, accent: Hsla) -> Hsla {
    if progress > 0.0 { accent } else { transparent_black() }
}

fn paint_voice_ring(
    swap_state: &Rc<RefCell<VoiceRingSwapState>>,
    progress: f32,
    is_playing: bool,
    accent: Hsla,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let base_key = NEXT_VOICE_RING_SWAP_KEY.fetch_add(2, Ordering::Relaxed);
    swap_state.borrow_mut().base_key = base_key;
    let Some((trim, caps)) = voice_ring_paths(bounds.origin, bounds.size, progress) else {
        return;
    };
    let color = voice_ring_color(progress, accent);
    window.paint_swappable_path(trim, color, base_key, bounds.origin);
    window.paint_swappable_path(caps, color, base_key + 1, bounds.origin);
    if !is_playing || !is_painted_on_screen(bounds, window) || window.transformation() != TransformationMatrix::unit() {
        return;
    }
    VoiceRingSwaps {
        swap_state: swap_state.clone(),
        base_key,
        ring: bounds.size,
        accent,
        shown: progress,
        from: progress,
        target: progress,
        retargeted_at: RiseClock::monotonic(),
        view: window.current_view(),
    }
    .schedule(window);
}

struct VoiceRingSwaps {
    swap_state: Rc<RefCell<VoiceRingSwapState>>,
    base_key: u64,
    ring: Size<Pixels>,
    accent: Hsla,
    shown: f32,
    from: f32,
    target: f32,
    retargeted_at: Duration,
    view: EntityId,
}

impl VoiceRingSwaps {
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
            (player.is_playing(), player.progress() as f32)
        };
        if !is_playing {
            return;
        }
        let now = RiseClock::monotonic();
        if progress != self.target {
            self.from = self.shown;
            self.target = progress;
            self.retargeted_at = now;
        }
        let elapsed = now.saturating_sub(self.retargeted_at).as_secs_f64();
        let t = (elapsed / RING_MOTION_DURATION).clamp(0.0, 1.0) as f32;
        let shown = self.from + (self.target - self.from) * t;
        if shown == self.shown {
            self.schedule(window);
            return;
        }
        let Some((trim, caps)) = voice_ring_paths(point(px(0.), px(0.)), self.ring, shown) else {
            self.schedule(window);
            return;
        };
        let color: Background = voice_ring_color(shown, self.accent).into();
        if window.swap_paths(vec![(self.base_key, trim, color), (self.base_key + 1, caps, color)]) {
            self.shown = shown;
            self.schedule(window);
            return;
        }
        cx.notify(self.view);
    }
}

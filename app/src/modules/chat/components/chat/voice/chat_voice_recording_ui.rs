use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, App, AsyncWindowContext, Context, Render, Bounds, BoxShadow, Element, Entity, GlobalElementId, InspectorElementId,
    LayoutId, WeakEntity, Corners, ElementId, FontWeight, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Size,
    Subscription, Window, canvas, div, fill, point, prelude::*, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::error_text_ui::system_red;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::motion::{ease_out, spring};
use rise_ui::swiftui::stacks::{hstack, spacer, spacer_min_w};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatBottomBarMetrics;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatVoiceDraft;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceRecordingTimeFormat;

type ChatVoiceAction = Rc<dyn Fn(&mut Window, &mut App)>;

const TRACK_STROKE_OPACITY: f64 = 0.14;
const TOAST_STROKE_OPACITY: f64 = 0.14;
const LOCK_HINT_STROKE_OPACITY: f64 = 0.12;
const CANCEL_TINT_MOTION_DURATION: f64 = 0.15;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// reads the elapsed time of the take on screen, none once the take is over
pub type ChatRecordingClockReader = Rc<dyn Fn(&App) -> Option<i64>>;

fn recording_clock(
    id: ElementId,
    milliseconds: i64,
    dot_opacity: f32,
    theme: &AppTheme,
    reader: ChatRecordingClockReader,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Div {
    let timeline = window.use_keyed_state(id, cx, |_, _| ChatRecordingClockTimeline { newest_base: 0 });
    let _ = timeline.read(cx).newest_base;
    let label = VoiceRecordingTimeFormat::centiseconds_label(milliseconds);
    let shown = theme.text.primary;
    let hidden = theme.text.primary.opacity(0.0);
    let mut slots = 0_u32;
    let mut row = div().flex().flex_row();
    for character in label.chars() {
        let Some(value) = character.to_digit(10) else {
            row = row.child(div().child(SharedString::from(character.to_string())));
            continue;
        };
        let slot = slots;
        slots += 1;
        let mut cell = div().relative();
        for digit in 0..10_u32 {
            let glyph = div()
                .text_color(if digit == value { shown } else { hidden })
                .child(SharedString::from(digit.to_string()));
            let glyph = if digit == 0 { glyph } else { glyph.absolute().top_0().left_0() };
            cell = cell.child(ChatRecordingClockDigit { child: glyph.into_any_element(), offset: slot * 10 + digit });
        }
        row = row.child(cell);
    }
    let digits = div()
        .swiftui_font(Font::system(16.0, FontWeight::SEMIBOLD, FontDesign::Rounded).monospaced_digit())
        .text_color(theme.text.primary)
        .child(ChatRecordingClockScope { child: row.into_any_element(), timeline, reader, slots, label });
    hstack(8.0_f32).child(div().size(px(8.0)).rounded_full().bg(system_red(cx)).opacity(dot_opacity)).child(digits)
}

/// swap key family of the newest paint of one recording clock
struct ChatRecordingClockTimeline {
    newest_base: u32,
}

static NEXT_CLOCK_SWAP_BASE: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static CLOCK_PAINT_BASE: Cell<u32> = const { Cell::new(0) };
}

fn next_clock_swap_base() -> u32 {
    ((NEXT_CLOCK_SWAP_BASE.fetch_add(1, Ordering::Relaxed) & 0x00FF_FFFF) << 6) | (1 << 30)
}

const CLOCK_SWAP_INTERVAL: Duration = Duration::from_millis(50);

struct ChatRecordingClockScope {
    child: AnyElement,
    timeline: Entity<ChatRecordingClockTimeline>,
    reader: ChatRecordingClockReader,
    slots: u32,
    label: String,
}

impl IntoElement for ChatRecordingClockScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ChatRecordingClockScope {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let base = next_clock_swap_base();
        let previous = CLOCK_PAINT_BASE.with(|cell| cell.replace(base));
        self.child.paint(window, cx);
        CLOCK_PAINT_BASE.with(|cell| cell.set(previous));
        self.timeline.update(cx, |timeline, _| timeline.newest_base = base);
        let visible = bounds.intersect(&window.content_mask().bounds);
        if visible.size.width <= Pixels::ZERO || visible.size.height <= Pixels::ZERO {
            return;
        }
        ChatRecordingClockSwaps {
            base,
            slots: self.slots,
            shown_label: std::mem::take(&mut self.label),
            reader: self.reader.clone(),
            timeline: self.timeline.downgrade(),
        }
        .schedule(window, cx);
    }
}

struct ChatRecordingClockDigit {
    child: AnyElement,
    offset: u32,
}

impl IntoElement for ChatRecordingClockDigit {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ChatRecordingClockDigit {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let base = CLOCK_PAINT_BASE.with(Cell::get);
        if base == 0 {
            self.child.paint(window, cx);
            return;
        }
        window.with_glyph_swap_key(base + self.offset, |window| self.child.paint(window, cx));
    }
}

struct ChatRecordingClockSwaps {
    base: u32,
    slots: u32,
    shown_label: String,
    reader: ChatRecordingClockReader,
    timeline: WeakEntity<ChatRecordingClockTimeline>,
}

impl ChatRecordingClockSwaps {
    fn schedule(self, window: &Window, cx: &App) {
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(CLOCK_SWAP_INTERVAL).await;
                let _ = cx.update(move |window, _| window.on_next_frame(move |window, cx| self.tick(window, cx)));
            })
            .detach();
    }

    fn tick(mut self, window: &mut Window, cx: &mut App) {
        let Some(timeline) = self.timeline.upgrade() else {
            return;
        };
        if timeline.read(cx).newest_base != self.base {
            return;
        }
        let Some(milliseconds) = (self.reader)(cx) else {
            return;
        };
        let label = VoiceRecordingTimeFormat::centiseconds_label(milliseconds);
        if label == self.shown_label {
            self.schedule(window, cx);
            return;
        }
        let values: Vec<u32> = label.chars().filter_map(|character| character.to_digit(10)).collect();
        if values.len() as u32 != self.slots {
            timeline.update(cx, |_, cx| cx.notify());
            return;
        }
        let mut shown = Vec::with_capacity(values.len());
        let mut hidden = Vec::with_capacity(values.len() * 9);
        for (slot, value) in values.iter().enumerate() {
            for digit in 0..10_u32 {
                let key = self.base + slot as u32 * 10 + digit;
                if digit == *value { shown.push(key) } else { hidden.push(key) }
            }
        }
        if window.swap_glyph_visibility(&shown, &hidden) {
            self.shown_label = label;
            self.schedule(window, cx);
            return;
        }
        let timeline = self.timeline.clone();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = cx.update(|_, cx| timeline.update(cx, |_, cx| cx.notify()).ok());
            })
            .detach();
    }
}

struct ChatVoiceHoldTrackState {
    shows_cancel_hint: bool,
    cancel_tint: Animated<Hsla>,
    offset_x: Animated<f32>,
}

#[derive(IntoElement)]
pub struct ChatVoiceHoldTrack {
    id: ElementId,
    theme: AppTheme,
    min_height: f32,
    elapsed_milliseconds: i64,
    clock: ChatRecordingClockReader,
    shows_cancel_hint: bool,
    drag_translation: Size<Pixels>,
}

impl ChatVoiceHoldTrack {
    pub fn new(
        id: impl Into<ElementId>,
        theme: AppTheme,
        min_height: f32,
        elapsed_milliseconds: i64,
        clock: ChatRecordingClockReader,
        shows_cancel_hint: bool,
        drag_translation: Size<Pixels>,
    ) -> Self {
        Self { id: id.into(), theme, min_height, elapsed_milliseconds, clock, shows_cancel_hint, drag_translation }
    }

    fn cancel_tint(&self, red: Hsla) -> Hsla {
        if self.shows_cancel_hint { red.opacity(0.95) } else { self.theme.text.secondary.opacity(0.55) }
    }
}

impl RenderOnce for ChatVoiceHoldTrack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let red = system_red(cx);
        let target_tint = self.cancel_tint(red);
        let target_offset = (f32::from(self.drag_translation.width) * 0.35).min(0.0);
        let shows_cancel_hint = self.shows_cancel_hint;
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| ChatVoiceHoldTrackState {
            shows_cancel_hint,
            cancel_tint: Animated::new(target_tint),
            offset_x: Animated::new(target_offset),
        });
        let now = RiseClock::monotonic();
        let (tint, offset_x) = state.update(cx, |state, cx| {
            let flipped = state.shows_cancel_hint != shows_cancel_hint;
            state.shows_cancel_hint = shows_cancel_hint;
            let spec = || flipped.then(|| ease_out(CANCEL_TINT_MOTION_DURATION));
            state.cancel_tint.set_in(target_tint, spec(), now, cx);
            state.offset_x.set_in(target_offset, spec(), now, cx);
            (state.cancel_tint.frame_value(now, window), state.offset_x.frame_value(now, window))
        });

        hstack(10.0_f32)
            .w_full()
            .min_h(px(self.min_height))
            .px(px(16.0))
            .child(
                hstack(6.0_f32)
                    .relative()
                    .left(px(offset_x))
                    .children(IconUi::sized("chevron.left", px(13.0), tint, false))
                    .child(MainText::new(tr("cancel")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(tint)),
            )
            .child(spacer_min_w(8.0))
            .child(recording_clock(
                child_id(&self.id, "clock"),
                self.elapsed_milliseconds,
                1.0,
                &self.theme,
                self.clock.clone(),
                window,
                cx,
            ))
            .liquid_chrome_rounded_rectangle_with(ChatBottomBarMetrics::INPUT_CORNER, TRACK_STROKE_OPACITY, None, false)
    }
}

#[derive(IntoElement)]
pub struct ChatVoiceLockedTrack {
    id: ElementId,
    theme: AppTheme,
    min_height: f32,
    elapsed_milliseconds: i64,
    clock: ChatRecordingClockReader,
    is_paused: bool,
    on_cancel: ChatVoiceAction,
}

impl ChatVoiceLockedTrack {
    pub fn new(
        id: impl Into<ElementId>,
        theme: AppTheme,
        min_height: f32,
        elapsed_milliseconds: i64,
        clock: ChatRecordingClockReader,
        is_paused: bool,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), theme, min_height, elapsed_milliseconds, clock, is_paused, on_cancel: Rc::new(on_cancel) }
    }
}

impl RenderOnce for ChatVoiceLockedTrack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let on_cancel = self.on_cancel;
        let dot_opacity = if self.is_paused { 0.45 } else { 1.0 };
        hstack(10.0_f32)
            .w_full()
            .min_h(px(self.min_height))
            .px(px(16.0))
            .child(recording_clock(
                child_id(&self.id, "clock"),
                self.elapsed_milliseconds,
                dot_opacity,
                &self.theme,
                self.clock.clone(),
                window,
                cx,
            ))
            .child(spacer_min_w(8.0))
            .child(
                plain(child_id(&self.id, "cancel")).on_press(move |window, cx| on_cancel(window, cx)).child(
                    MainText::new(tr("cancel"))
                        .px(16.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(self.theme.text.secondary.opacity(0.72)),
                ),
            )
            .liquid_chrome_rounded_rectangle_with(ChatBottomBarMetrics::INPUT_CORNER, TRACK_STROKE_OPACITY, None, false)
    }
}

/// composer row of a parked voice note, listen to it before it goes
#[derive(IntoElement)]
pub struct ChatVoiceDraftTrack {
    id: ElementId,
    theme: AppTheme,
    min_height: f32,
    draft: ChatVoiceDraft,
    playback_id: String,
    on_toggle_playback: ChatVoiceAction,
}

impl ChatVoiceDraftTrack {
    const BAR_COUNT: usize = 40;

    pub fn new(
        id: impl Into<ElementId>,
        theme: AppTheme,
        min_height: f32,
        draft: ChatVoiceDraft,
        playback_id: impl Into<String>,
        on_toggle_playback: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            theme,
            min_height,
            draft,
            playback_id: playback_id.into(),
            on_toggle_playback: Rc::new(on_toggle_playback),
        }
    }

    fn waveform(&self, progress: f64) -> impl IntoElement + use<> {
        let peaks = Self::peaks(&self.draft.metadata.waveform);
        let accent = self.theme.primary._100;
        let dim = self.theme.text.secondary.opacity(0.45);
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let count = peaks.len() as f32;
                let spacing = 2.0_f32;
                let bounds_width = f32::from(bounds.size.width);
                let bounds_height = f32::from(bounds.size.height);
                let bar_width = ((bounds_width - spacing * (count - 1.0)) / count).max(1.5);
                let mid = bounds_height / 2.0;
                let mut x = 0.0_f32;
                for (index, peak) in peaks.iter().enumerate() {
                    let height = (peak * bounds_height).max(2.0);
                    let rect = Bounds::new(
                        point(bounds.origin.x + px(x), bounds.origin.y + px(mid - height / 2.0)),
                        size(px(bar_width), px(height)),
                    );
                    let played =
                        index as f64 / peaks.len().saturating_sub(1).max(1) as f64 <= progress && progress > 0.0;
                    window.paint_quad(
                        fill(rect, if played { accent } else { dim }).corner_radii(Corners::all(px(bar_width / 2.0))),
                    );
                    x += bar_width + spacing;
                }
            },
        )
        .flex_1()
        .min_w(px(0.0))
        .h(px(22.0))
    }

    fn peaks(source: &[f32]) -> Vec<f32> {
        if source.len() <= 1 {
            let peak = source.first().copied().unwrap_or(0.35).clamp(0.12, 1.0);
            return vec![peak; Self::BAR_COUNT];
        }
        (0..Self::BAR_COUNT)
            .map(|index| {
                let position = index as f64 / (Self::BAR_COUNT - 1) as f64 * (source.len() - 1) as f64;
                source[position.round() as usize].clamp(0.12, 1.0)
            })
            .collect()
    }
}

impl RenderOnce for ChatVoiceDraftTrack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.current_view();
        let _player_observer = window.use_keyed_state(child_id(&self.id, "player-observer"), cx, move |_, cx| -> [Subscription; 2] {
            let player = ChatVoiceMessagePlayer::shared(cx);
            let tick = player.read(cx).progress_tick();
            [
                App::observe(cx, &player, move |_, cx: &mut App| cx.notify(host)),
                App::observe(cx, &tick, move |_, cx: &mut App| cx.notify(host)),
            ]
        });

        let player = ChatVoiceMessagePlayer::shared(cx);
        let (is_playing, is_current, progress, elapsed_seconds) = {
            let player = player.read(cx);
            (
                player.is_playing_message_id(&self.playback_id),
                player.playing_message_id() == Some(self.playback_id.as_str()),
                player.progress(),
                player.elapsed_seconds(),
            )
        };
        let duration = self.draft.metadata.duration_milliseconds as f64 / 1000.0;
        let symbol = if is_playing { "pause.fill" } else { "play.fill" };
        let label = tr(if is_playing { "media_viewer_pause" } else { "media_viewer_play" });
        let waveform = self.waveform(if is_current { progress } else { 0.0 });
        let on_toggle_playback = self.on_toggle_playback;

        hstack(10.0_f32)
            .w_full()
            .min_h(px(self.min_height))
            .pl(px(8.0))
            .pr(px(14.0))
            .child(
                plain(child_id(&self.id, "toggle")).on_press(move |window, cx| on_toggle_playback(window, cx)).child(
                    div()
                        .id(child_id(&self.id, "toggle-label"))
                        .tooltip_label(label)
                        .size(px(28.0))
                        .rounded_full()
                        .bg(self.theme.primary._100)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .relative()
                                .left(px(if is_playing { 0.0 } else { 1.0 }))
                                .children(IconUi::sized(symbol, px(12.0), self.theme.content_on(self.theme.primary._100), true)),
                        ),
                ),
            )
            .child(waveform)
            .child(
                div()
                    .swiftui_font(Font::system(14.0, FontWeight::SEMIBOLD, FontDesign::Rounded).monospaced_digit())
                    .text_color(self.theme.text.primary)
                    .child(RiseFormat::duration_time_interval(if is_current && elapsed_seconds > 0.0 {
                        elapsed_seconds
                    } else {
                        duration
                    })),
            )
            .liquid_chrome_rounded_rectangle_with(ChatBottomBarMetrics::INPUT_CORNER, TRACK_STROKE_OPACITY, None, false)
    }
}

#[derive(IntoElement)]
pub struct ChatVoiceLockHint {
    theme: AppTheme,
}

impl ChatVoiceLockHint {
    pub fn new(theme: AppTheme) -> Self {
        Self { theme }
    }
}

impl RenderOnce for ChatVoiceLockHint {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tint = self.theme.text.secondary;
        hstack(6.0_f32)
            .px(px(12.0))
            .py(px(7.0))
            .children(IconUi::sized("lock.fill", px(11.0), tint, true))
            .children(IconUi::sized("chevron.up", px(11.0), tint, false))
            .child(MainText::new(tr("chat_voice_slide_up_to_lock")).px(12.0).font_weight(FontWeight::SEMIBOLD))
            .liquid_chrome_capsule_with(LOCK_HINT_STROKE_OPACITY, true, None, false)
    }
}

/// the lock hint as its own cached view
/// its slide asked frames of the view it was rendered in
/// which was the chat screen so every spring frame rerendered it
pub struct ChatVoiceLockHintHost {
    presence: Presence<()>,
    theme: Option<AppTheme>,
    on_gone: Option<Rc<dyn Fn(&mut App)>>,
    was_shown: bool,
}

impl ChatVoiceLockHintHost {
    pub fn new() -> Self {
        Self {
            presence: Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity())),
            theme: None,
            on_gone: None,
            was_shown: false,
        }
    }

    pub fn configure(&mut self, visible: bool, theme: AppTheme, on_gone: Rc<dyn Fn(&mut App)>, cx: &mut Context<Self>) {
        let theme_changed = self.theme.as_ref().is_none_or(|held| held.text.secondary != theme.text.secondary);
        self.theme = Some(theme);
        self.on_gone = Some(on_gone);
        if self.presence.is_present() != visible {
            self.presence.set_in(visible.then_some(()), spring(0.28, 0.82), RiseClock::monotonic(), cx);
            cx.notify();
        } else if theme_changed {
            cx.notify();
        }
    }

    /// whether the hint is on screen at now
    /// arriving or resting or leaving
    pub fn is_shown(&self, now: std::time::Duration) -> bool {
        self.presence.value(now).is_some()
    }
}

impl Default for ChatVoiceLockHintHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for ChatVoiceLockHintHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let theme = self.theme.clone();
        let hint = theme.and_then(|theme| {
            self.presence.render_transitioned(now, window, move |_| {
                hstack(None::<f32>)
                    .w_full()
                    .pb(px(8.0))
                    .child(spacer())
                    .child(ChatVoiceLockHint::new(theme))
                    .child(spacer())
            })
        });
        if hint.is_some() {
            self.was_shown = true;
        } else if self.was_shown {
            self.was_shown = false;
            if let Some(on_gone) = self.on_gone.clone() {
                cx.defer(move |cx| on_gone(cx));
            }
        }
        div().size_full().children(hint)
    }
}

#[derive(IntoElement)]
pub struct ChatVoicePauseButton {
    id: ElementId,
    size: f32,
    is_paused: bool,
    theme: AppTheme,
    action: ChatVoiceAction,
}

impl ChatVoicePauseButton {
    pub fn new(
        id: impl Into<ElementId>,
        size: f32,
        is_paused: bool,
        theme: AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), size, is_paused, theme, action: Rc::new(action) }
    }
}

impl RenderOnce for ChatVoicePauseButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let action = self.action;
        let symbol = if self.is_paused { "play.fill" } else { "pause.fill" };
        LiquidGlassButton::new(self.id, LiquidGlassButtonShape::Circle)
            .size(self.size)
            .on_press(move |window, cx| action(window, cx))
            .children(IconUi::sized(symbol, px(14.0), self.theme.text.primary, true))
    }
}

#[derive(IntoElement)]
pub struct ChatVoiceSendButton {
    id: ElementId,
    size: f32,
    theme: AppTheme,
    action: ChatVoiceAction,
}

impl ChatVoiceSendButton {
    pub fn new(
        id: impl Into<ElementId>,
        size: f32,
        theme: AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), size, theme, action: Rc::new(action) }
    }
}

impl RenderOnce for ChatVoiceSendButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let action = self.action;
        let primary = self.theme.primary._100;
        plain(self.id)
            .on_press(move |window, cx| action(window, cx))
            .relative()
            .size(px(self.size))
            .child(div().absolute().inset_0().rounded_full().bg(primary).shadow(vec![BoxShadow {
                color: primary.opacity(0.45),
                offset: point(px(0.0), px(2.0)),
                blur_radius: px(10.0),
                spread_radius: px(0.0),
                inset: false,
            }]))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("arrow.up", px(18.0), self.theme.content_on(primary), false)),
            )
    }
}

#[derive(IntoElement)]
pub struct ChatVoiceTooShortToast {
    theme: AppTheme,
}

impl ChatVoiceTooShortToast {
    pub fn new(theme: AppTheme) -> Self {
        Self { theme }
    }

    /// transition of the toast for the presenter presence
    pub fn transition() -> Transition {
        Transition::move_edge(Edge::Bottom).combined(Transition::opacity())
    }
}

impl RenderOnce for ChatVoiceTooShortToast {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .px(px(14.0))
            .py(px(10.0))
            .child(
                MainText::new(tr("chat_voice_too_short"))
                    .px(14.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(self.theme.text.primary),
            )
            .liquid_chrome_capsule_with(TOAST_STROKE_OPACITY, true, None, false)
    }
}

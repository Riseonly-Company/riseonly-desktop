use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, AsyncApp, Context, ElementId, Entity, FontWeight, Hsla, Render, SharedString, StyleRefinement, Task,
    WeakEntity, Window,
};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::{tr, tr_plural};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::topic_icon_glyph::TopicIconGlyph;
use crate::modules::chat::engine::forum::rise_forum_voice_presence::RiseForumVoicePresenceSnapshot;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::MemberTagChip;
use crate::modules::chat::pages::chats::chat::chat_topics::channel_kind_editor::ChatChannelKindPresentation;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatTopic, VoiceChannelParticipant, VoiceChannelState,
};

const ROLE_CHIP_MAX_WIDTH: f32 = 140.0;
const ROLE_CHIP_MIN_WIDTH: f32 = 28.0;
const ROLE_CHIP_SHRINK_WEIGHT: f32 = 1000.0;
const UNLIMITED_SEATS: &str = "\u{221E}";

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn bottom_hairline(theme: &AppTheme) -> Div {
    div().absolute().left_0().right_0().bottom_0().h(px(0.5)).bg(theme.border._100)
}

pub enum ForumVoiceChannelMetrics {}

impl ForumVoiceChannelMetrics {
    pub const HEADER_HEIGHT: f32 = 52.0;
    pub const PARTICIPANT_HEIGHT: f32 = 38.0;
    pub const OVERFLOW_HEIGHT: f32 = 30.0;
    pub const ICON_SLOT: f32 = 36.0;
    pub const AVATAR_SIZE: f32 = 26.0;
    pub const HORIZONTAL_PADDING: f32 = 12.0;
    pub const PARTICIPANT_LEADING_INSET: f32 = 20.0;

    /// block height once participants are known
    /// the skeleton uses it so the list doesnt resize when live data lands
    pub fn block_height(participant_count: i64, has_overflow: bool) -> f32 {
        Self::HEADER_HEIGHT
            + participant_count as f32 * Self::PARTICIPANT_HEIGHT
            + if has_overflow { Self::OVERFLOW_HEIGHT } else { 0.0 }
    }
}

/// timer phased on the join second so the row flips at the same instant as the room screen
#[derive(IntoElement)]
struct ForumVoiceSessionTimer {
    id: ElementId,
    started_at: Date,
}

impl RenderOnce for ForumVoiceSessionTimer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = rise_ui::theme(cx).text.secondary;
        let started_at = self.started_at;
        let label = window.with_global_id(named_child(&self.id, "label"), |global_id, window| {
            window.with_element_state(global_id, |held: Option<Entity<ForumVoiceSessionTimerLabel>>, _| {
                let label = held.unwrap_or_else(|| cx.new(|cx| ForumVoiceSessionTimerLabel::new(started_at, color, cx)));
                (label.clone(), label)
            })
        });
        label.update(cx, |label, cx| label.configure(started_at, color, cx));
        let shown = ForumVoiceSessionTimerLabel::text(started_at);
        let slot_width = ForumVoiceSessionTimerLabel::slot_width(&shown);
        label.cached(StyleRefinement::default().w(px(slot_width)).h(px(16.0)).flex_none())
    }
}

/// session duration text
/// ticks by itself on the join second and notifies only when the shown text changes
struct ForumVoiceSessionTimerLabel {
    started_at: Date,
    color: Hsla,
    shown: String,
    _tick: Option<Task<()>>,
}

impl ForumVoiceSessionTimerLabel {
    const DIGIT_WIDTH: f32 = 7.5;
    const SEPARATOR_WIDTH: f32 = 4.0;
    const HOUR_TEMPLATE: &'static str = "0:00:00";

    fn new(started_at: Date, color: Hsla, cx: &mut Context<Self>) -> Self {
        let mut label = Self { started_at, color, shown: Self::text(started_at), _tick: None };
        label.restart_tick(cx);
        label
    }

    fn text(started_at: Date) -> String {
        RiseFormat::duration_time_interval(Date::now().time_interval_since(&started_at).max(0.0))
    }

    fn slot_width(shown: &str) -> f32 {
        let width = |text: &str| {
            text.chars()
                .map(|ch| if ch.is_ascii_digit() { Self::DIGIT_WIDTH } else { Self::SEPARATOR_WIDTH })
                .sum::<f32>()
        };
        width(shown).max(width(Self::HOUR_TEMPLATE)) + 2.0
    }

    fn configure(&mut self, started_at: Date, color: Hsla, cx: &mut Context<Self>) {
        let mut changed = false;
        if self.started_at != started_at {
            self.started_at = started_at;
            self.shown = Self::text(started_at);
            self.restart_tick(cx);
            changed = true;
        }
        if self.color != color {
            self.color = color;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// restarts the tick, phased on the join second so the row flips with the room screen
    fn restart_tick(&mut self, cx: &mut Context<Self>) {
        let started_at = self.started_at;
        self._tick = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                let elapsed = Date::now().time_interval_since(&started_at).max(0.0);
                cx.background_executor().timer(Duration::from_secs_f64(1.0 - elapsed.fract())).await;
                let alive = this.update(cx, |label, cx| {
                    let shown = Self::text(label.started_at);
                    if shown != label.shown {
                        label.shown = shown;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        }));
    }
}

impl Render for ForumVoiceSessionTimerLabel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().items_center().justify_end().child(
            MainText::new(self.shown.clone())
                .px(12.0)
                .font_weight(FontWeight::MEDIUM)
                .color(self.color)
                .number_of_lines(1)
                .monospaced_digits(true)
                .flex_none(),
        )
    }
}

/// separate from the participant lines so the channel hold menu wraps only this row
/// a right click on a member must not open channel actions
#[derive(IntoElement)]
pub struct ForumVoiceChannelHeaderRow {
    topic: ChatTopic,
    is_muted: bool,
}

impl ForumVoiceChannelHeaderRow {
    pub fn new(topic: ChatTopic, is_muted: bool) -> Self {
        Self { topic, is_muted }
    }

    fn row_id(&self) -> ElementId {
        ElementId::Name(format!("forum-voice-header-{}", self.topic.id).into())
    }

    fn header(&self, theme: &AppTheme, state: Option<&VoiceChannelState>) -> Div {
        hstack(8.0)
            .w_full()
            .h(px(ForumVoiceChannelMetrics::HEADER_HEIGHT))
            .px(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING))
            .bg(theme.bg._100)
            .child(self.channel_icon(theme))
            .child(
                MainText::new(self.topic.title.clone())
                    .px(16.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary)
                    .number_of_lines(1)
                    .min_w_0(),
            )
            .when(self.is_muted, |row| {
                row.children(IconUi::sized("bell.slash.fill", px(11.0), theme.text.secondary, true))
            })
            .child(spacer_min_w(8.0))
            .children(self.session_timer(state))
            .children(self.occupancy_pill(theme, state))
    }

    fn channel_icon(&self, theme: &AppTheme) -> Div {
        let glyph = self.topic.channel_kind.glyph_name();
        div()
            .relative()
            .flex_none()
            .w(px(ForumVoiceChannelMetrics::ICON_SLOT))
            .h(px(ForumVoiceChannelMetrics::ICON_SLOT))
            .flex()
            .items_center()
            .justify_center()
            .child(TopicIconGlyph::new(self.topic.icon.clone()).size(22.0))
            .child(
                div()
                    .absolute()
                    .left(px(-3.0))
                    .bottom(px(-2.0))
                    .w(px(16.0))
                    .h(px(16.0))
                    .circle()
                    .bg(theme.bg._100)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(glyph, px(10.0), theme.primary._100, glyph.ends_with(".fill")))
                    .child(centred_stroke(theme.bg._100, 1.5, Shape::Circle)),
            )
    }

    fn session_timer(&self, state: Option<&VoiceChannelState>) -> Option<ForumVoiceSessionTimer> {
        let started_at = state?.session_started_at()?;
        Some(ForumVoiceSessionTimer { id: named_child(&self.row_id(), "session-timer"), started_at })
    }

    fn occupancy_pill(&self, theme: &AppTheme, state: Option<&VoiceChannelState>) -> Option<impl IntoElement + use<>> {
        let occupied = state.map_or(0, |state| state.participant_count);
        if occupied <= 0 {
            return None;
        }
        let state_limit = state.map_or(0, |state| state.user_limit);
        let limit = if state_limit > 0 {
            state_limit
        } else {
            self.topic.voice_config.as_ref().map_or(0, |config| config.user_limit)
        };
        let label = if limit > 0 { format!("{occupied} {} {limit}", tr("rtc_of")) } else { occupied.to_string() };
        let limit_text = if limit > 0 { limit.to_string() } else { UNLIMITED_SEATS.to_owned() };
        let segment = |text: String, color: Hsla| {
            div()
                .h(px(22.0))
                .px(px(7.0))
                .flex()
                .items_center()
                .child(MainText::new(text).px(12.0).font_weight(FontWeight::SEMIBOLD).color(color))
        };
        Some(
            hstack(0.0)
                .id(named_child(&self.row_id(), "occupancy"))
                .flex_none()
                .bg(theme.bg._300)
                .capsule()
                .overflow_hidden()
                .tooltip_label(label)
                .child(segment(occupied.to_string(), theme.text.primary))
                .child(div().w(px(0.5)).h(px(22.0)).bg(theme.border._100))
                .child(segment(limit_text, theme.text.secondary)),
        )
    }
}

impl RenderOnce for ForumVoiceChannelHeaderRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        self.header(&theme, self.topic.voice_state.as_ref())
    }
}

/// tail row of a channel whose participant preview is capped, the N more row
/// and the show less row under it once expanded
#[derive(IntoElement)]
pub struct ForumVoiceOverflowRow {
    id: ElementId,
    text: SharedString,
    on_tap: ButtonAction,
}

impl ForumVoiceOverflowRow {
    pub fn new(
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), text: text.into(), on_tap: std::rc::Rc::new(on_tap) }
    }
}

impl RenderOnce for ForumVoiceOverflowRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let on_tap = self.on_tap;
        plain(self.id)
            .w_full()
            .h(px(ForumVoiceChannelMetrics::OVERFLOW_HEIGHT))
            .flex()
            .items_center()
            .pl(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING + ForumVoiceChannelMetrics::PARTICIPANT_LEADING_INSET))
            .pr(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING))
            .bg(theme.bg._100)
            .on_press(move |window, cx| on_tap(window, cx))
            .child(
                MainText::new(self.text)
                    .px(12.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.text.secondary)
                    .number_of_lines(1)
                    .min_w_0(),
            )
    }
}

#[derive(IntoElement)]
pub struct ForumVoiceParticipantRow {
    id: ElementId,
    participant: VoiceChannelParticipant,
    is_stage: bool,
    pub hand_expiry_revision: i64,
    on_tap: ButtonAction,
}

impl ForumVoiceParticipantRow {
    pub fn new(
        id: impl Into<ElementId>,
        participant: VoiceChannelParticipant,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), participant, is_stage: false, hand_expiry_revision: 0, on_tap: std::rc::Rc::new(on_tap) }
    }

    pub fn is_stage(mut self, is_stage: bool) -> Self {
        self.is_stage = is_stage;
        self
    }

    pub fn hand_expiry_revision(mut self, hand_expiry_revision: i64) -> Self {
        self.hand_expiry_revision = hand_expiry_revision;
        self
    }

    fn shows_hand(&self) -> bool {
        self.is_stage && self.participant.is_hand_raised(RiseForumVoicePresenceSnapshot::now_ms_for_display())
    }

    fn shows_stage_seat(&self) -> bool {
        self.is_stage && self.participant.is_on_stage()
    }

    fn state_icons(&self, theme: &AppTheme) -> Div {
        let error = theme.semantic.error_100;
        hstack(6.0)
            .flex_none()
            .when(self.shows_stage_seat(), |row| {
                row.children(IconUi::sized("person.wave.2.fill", px(10.0), theme.primary._100, true))
            })
            .when(self.participant.camera_enabled, |row| {
                row.children(IconUi::sized("video.fill", px(11.0), theme.text.secondary, true))
            })
            .when(self.participant.is_deafened(), |row| {
                row.children(IconUi::sized(RiseSymbol::DEAFENED, px(12.0), error, false))
            })
            .when(self.participant.is_microphone_muted(), |row| {
                row.children(IconUi::sized("mic.slash.fill", px(12.0), error, true))
            })
            .when(self.shows_hand(), |row| {
                row.children(IconUi::sized("hand.raised.fill", px(11.0), theme.primary._100, true))
            })
            .when(self.participant.is_live(), |row| {
                row.child(
                    div().h(px(18.0)).px(px(6.0)).flex().items_center().capsule().bg(theme.primary._100).child(
                        MainText::new(tr("rtc_on_air"))
                            .px(10.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme.content_on(theme.primary._100))
                            .number_of_lines(1),
                    ),
                )
            })
    }

    fn accessibility_label(&self) -> String {
        let mut parts = vec![self.participant.name.clone()];
        if self.shows_stage_seat() {
            parts.push(tr("rtc_speaker"));
        }
        if self.shows_hand() {
            parts.push(tr("rtc_hand_raised"));
        }
        if self.participant.camera_enabled {
            parts.push(tr("rtc_camera"));
        }
        if self.participant.is_deafened() {
            parts.push(tr("rtc_deafened"));
        }
        if self.participant.is_microphone_muted() {
            parts.push(tr("rtc_muted"));
        }
        if self.participant.is_live() {
            parts.push(tr("rtc_on_air"));
        }
        parts.join(", ")
    }
}

impl RenderOnce for ForumVoiceParticipantRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let participant = &self.participant;
        let is_quiet = participant.is_deafened() || participant.is_microphone_muted();
        let role_chip = div()
            .flex()
            .flex_shrink(ROLE_CHIP_SHRINK_WEIGHT)
            .min_w(px(ROLE_CHIP_MIN_WIDTH))
            .max_w(px(ROLE_CHIP_MAX_WIDTH))
            .overflow_hidden();
        let on_tap = self.on_tap.clone();

        let row = hstack(8.0)
            .w_full()
            .h(px(ForumVoiceChannelMetrics::PARTICIPANT_HEIGHT))
            .pl(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING + ForumVoiceChannelMetrics::PARTICIPANT_LEADING_INSET))
            .pr(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING))
            .bg(theme.bg._100)
            .child(
                div()
                    .flex_none()
                    .w(px(ForumVoiceChannelMetrics::AVATAR_SIZE))
                    .h(px(ForumVoiceChannelMetrics::AVATAR_SIZE))
                    .child(
                        UserLogoUi::new(named_child(&self.id, "logo"))
                            .source(SharedString::from(participant.avatar_url.clone()))
                            .name(SharedString::from(participant.name.clone()))
                            .user_id(SharedString::from(participant.user_id.clone()))
                            .size(ForumVoiceChannelMetrics::AVATAR_SIZE)
                            .show_story_ring(false)
                            .open_story_on_press(false),
                    ),
            )
            .child(
                MainText::new(participant.name.clone())
                    .px(14.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(if is_quiet { theme.text.secondary } else { theme.text.primary })
                    .number_of_lines(1)
                    .flex_shrink(1.0)
                    .min_w_0(),
            )
            .when(participant.is_official, |row| {
                row.child(div().flex_none().child(OfficialBadge::new().size(px(12.0))))
            })
            .when(participant.is_premium, |row| row.child(div().flex_none().child(PremiumDiamond::new().size(12.0))))
            .when(!participant.role_name.is_empty(), |row| {
                row.child(role_chip.child(
                    MemberTagChip::new(participant.role_name.clone()).color_hex(participant.role_badge_color_hex()),
                ))
            })
            .child(spacer_min_w(6.0))
            .child(self.state_icons(&theme));

        div().id(named_child(&self.id, "hover")).w_full().tooltip_label(self.accessibility_label()).child(
            plain(self.id.clone())
                .w_full()
                .on_press(move |window, cx| on_tap(window, cx))
                .child(row),
        )
    }
}

#[derive(IntoElement)]
pub struct ForumVoiceChannelSkeletonRow {
    participant_count: i64,
}

impl Default for ForumVoiceChannelSkeletonRow {
    fn default() -> Self {
        Self { participant_count: 3 }
    }
}

impl ForumVoiceChannelSkeletonRow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn participant_count(mut self, participant_count: i64) -> Self {
        self.participant_count = participant_count;
        self
    }
}

impl RenderOnce for ForumVoiceChannelSkeletonRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        vstack(0.0)
            .w_full()
            .child(
                hstack(8.0)
                    .relative()
                    .w_full()
                    .h(px(ForumVoiceChannelMetrics::HEADER_HEIGHT))
                    .px(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING))
                    .child(SkeletonBlock::new(
                        ForumVoiceChannelMetrics::ICON_SLOT,
                        ForumVoiceChannelMetrics::ICON_SLOT,
                        10.0,
                    ))
                    .child(SkeletonLine::new(128.0, 11.0))
                    .child(spacer_min_w(8.0))
                    .child(SkeletonBlock::new(54.0, 22.0, 11.0))
                    .child(bottom_hairline(&theme)),
            )
            .children((0..self.participant_count.max(0)).map(ForumVoiceParticipantPlaceholderRow::new))
    }
}

/// a member line whose person isnt known yet, the cold skeleton
/// or a live roster that lost a listed member before the server named the next one
#[derive(IntoElement)]
pub struct ForumVoiceParticipantPlaceholderRow {
    index: i64,
}

impl ForumVoiceParticipantPlaceholderRow {
    pub fn new(index: i64) -> Self {
        Self { index }
    }
}

impl RenderOnce for ForumVoiceParticipantPlaceholderRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(8.0)
            .w_full()
            .h(px(ForumVoiceChannelMetrics::PARTICIPANT_HEIGHT))
            .pl(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING + ForumVoiceChannelMetrics::PARTICIPANT_LEADING_INSET))
            .pr(px(ForumVoiceChannelMetrics::HORIZONTAL_PADDING))
            .child(SkeletonBlock::new(
                ForumVoiceChannelMetrics::AVATAR_SIZE,
                ForumVoiceChannelMetrics::AVATAR_SIZE,
                ForumVoiceChannelMetrics::AVATAR_SIZE / 2.0,
            ))
            .child(SkeletonLine::new(if self.index % 2 == 0 { 96.0 } else { 132.0 }, 9.0))
            .child(spacer_min_w(0.0))
    }
}

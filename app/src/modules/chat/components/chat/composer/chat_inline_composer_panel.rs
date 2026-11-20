use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, Pixels, Point, RenderOnce, Render, ScrollDelta,
    ScrollHandle, ScrollStrategy, ScrollWheelEvent, SharedString, UniformListScrollHandle, Window, black, uniform_list, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, alpha};
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};

use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::{
    BotInlineCommandsModel, BotInlineResultModel, BotInlineResultsPage, BotMentionCandidateModel,
};
use crate::modules::bot::stores::bot::bot_actions::bot_group_command_types::{ChatGroupBotCommandModel, ChatGroupBotCommands};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotCommandModel;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerInlineContext;
use crate::modules::chat::components::chat::topic_icon_glyph::TopicIconGlyph;
use crate::modules::chat::pages::chats::chat::chat_topics::channel_kind_editor::ChatChannelKindPresentation;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatRole, ChatTopic};

pub enum ChatInlineComposerPanelMetrics {}

impl ChatInlineComposerPanelMetrics {
    pub const COLLAPSED_HEIGHT: f32 = 264.0;
    pub const MINIMUM_HEIGHT: f32 = 172.0;

    pub fn preferred_collapsed_height(
        context: &ChatComposerInlineContext,
        candidate_count: usize,
        role_count: usize,
        command_count: usize,
        results: &[BotInlineResultModel],
        group_command_count: usize,
        topic_count: usize,
    ) -> f32 {
        match context {
            ChatComposerInlineContext::Inactive => 0.0,
            ChatComposerInlineContext::Topics { .. } => {
                if topic_count == 0 {
                    return Self::MINIMUM_HEIGHT;
                }
                Self::COLLAPSED_HEIGHT.min((topic_count * 44 + 4) as f32)
            }
            ChatComposerInlineContext::GroupCommands { .. } => {
                if group_command_count == 0 {
                    return Self::MINIMUM_HEIGHT;
                }
                Self::COLLAPSED_HEIGHT.min((group_command_count * 52 + 4) as f32)
            }
            ChatComposerInlineContext::Candidates { .. } => {
                let count = candidate_count + role_count;
                if count == 0 {
                    return Self::MINIMUM_HEIGHT;
                }
                Self::COLLAPSED_HEIGHT.min((count * 44 + 4) as f32)
            }
            ChatComposerInlineContext::Bot { query, .. } => {
                if trimming_whitespaces_and_newlines(query).is_empty() {
                    if command_count == 0 {
                        return Self::MINIMUM_HEIGHT;
                    }
                    return Self::COLLAPSED_HEIGHT.min((command_count * 48 + 4) as f32);
                }
                if ChatComposerInlineContext::command_awaiting_argument(query).is_some() {
                    return Self::MINIMUM_HEIGHT;
                }
                if results.is_empty() {
                    return Self::MINIMUM_HEIGHT;
                }
                let visual_height = if results.iter().any(BotInlineResultModel::is_visual) { 112.0 } else { 0.0 };
                let compact_count = results.iter().filter(|result| !result.is_visual()).count();
                let compact_height = (Self::COLLAPSED_HEIGHT - visual_height).min((compact_count * 58) as f32);
                Self::COLLAPSED_HEIGHT.min(58.0_f32.max(visual_height + compact_height))
            }
        }
    }

    /// height for the panel, collapsed height defaults to COLLAPSED_HEIGHT
    pub fn height(available_height: f32, is_expanded: bool, preferred_collapsed_height: f32) -> f32 {
        let maximum = available_height.max(0.0);
        if maximum <= 0.0 {
            return 0.0;
        }
        if is_expanded {
            return maximum;
        }
        maximum.min(preferred_collapsed_height.max(0.0))
    }
}

pub enum ChatInlineComposerPresentationContract {}

impl ChatInlineComposerPresentationContract {
    pub const FEED_BOTTOM_SPACING: f32 = -3.0;

    pub fn inline_controls_are_active(context_presented: bool, composer_eligible: bool) -> bool {
        context_presented && composer_eligible
    }

    pub fn panel_is_visible(inline_controls_are_active: bool, picker_visible: bool) -> bool {
        inline_controls_are_active && !picker_visible
    }

    /// bottom inset of the feed, spacing defaults to FEED_BOTTOM_SPACING
    pub fn feed_bottom_inset(base_composer_height: f32, spacing: f32) -> f32 {
        (base_composer_height.max(0.0) + spacing).max(0.0)
    }

    /// should an empty page auto load, max attempts defaults to 3
    pub fn should_auto_load_empty_page(
        item_count: usize,
        has_more: bool,
        cursor: Option<&str>,
        attempts: usize,
        maximum_attempts: usize,
    ) -> bool {
        item_count == 0 && has_more && cursor.is_some_and(|cursor| !cursor.is_empty()) && attempts < maximum_attempts
    }

    /// true when an empty page needs a manual continue, max attempts defaults to 3
    pub fn requires_manual_empty_page_continuation(
        item_count: usize,
        has_more: bool,
        cursor: Option<&str>,
        attempts: usize,
        maximum_attempts: usize,
    ) -> bool {
        item_count == 0 && has_more && cursor.is_some_and(|cursor| !cursor.is_empty()) && attempts >= maximum_attempts
    }
}

const MAXIMUM_EMPTY_PAGE_ATTEMPTS: usize = 3;
const VISUAL_CARD_SIZE: f32 = 104.0;
const VISUAL_SPACING: f32 = 4.0;
const VISUAL_PADDING: f32 = 4.0;
const VISUAL_OVERSCAN: usize = 4;
const VISUAL_DEFAULT_VIEWPORT: f32 = 480.0;

pub type ChatInlineComposerPanelAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type ChatInlineComposerPanelItemAction<T> = Rc<dyn Fn(&T, &mut Window, &mut App)>;

#[derive(Clone, IntoElement)]
pub struct ChatInlineComposerPanel {
    pub id: ElementId,
    pub context: ChatComposerInlineContext,
    pub candidates: Vec<BotMentionCandidateModel>,
    pub roles: Vec<ChatRole>,
    pub highlighted: Option<usize>,
    pub commands: Option<BotInlineCommandsModel>,
    pub results: BotInlineResultsPage,
    pub is_loading: bool,
    pub error: Option<String>,
    pub available_height: f32,
    pub collapsed_height: f32,
    pub pagination_cache_key: String,
    pub candidates_has_more: bool,
    pub candidates_cursor: Option<String>,
    pub is_result_sending: Rc<dyn Fn(&BotInlineResultModel, &App) -> bool>,
    pub on_candidate: ChatInlineComposerPanelItemAction<BotMentionCandidateModel>,
    pub on_role: ChatInlineComposerPanelItemAction<ChatRole>,
    pub on_command: ChatInlineComposerPanelItemAction<BotCommandModel>,
    pub on_result: ChatInlineComposerPanelItemAction<BotInlineResultModel>,
    pub on_retry: ChatInlineComposerPanelAction,
    pub on_load_more_candidates: ChatInlineComposerPanelAction,
    pub on_load_more_results: ChatInlineComposerPanelAction,
    pub on_height_change: Rc<dyn Fn(Option<f32>, &mut Window, &mut App)>,
    pub on_dismiss: ChatInlineComposerPanelAction,
    pub is_detached: bool,
    pub group_commands: Vec<ChatGroupBotCommandModel>,
    pub on_group_command: ChatInlineComposerPanelItemAction<ChatGroupBotCommandModel>,
    pub topics: Vec<ChatTopic>,
    pub on_topic: ChatInlineComposerPanelItemAction<ChatTopic>,
    pub is_expanded: Binding<bool>,
}

/// the panel as its own cached view
/// mention list scroll notifies the view it renders in, that was the whole chat screen
/// so the host notifies only when panel data changes
pub struct ChatInlineComposerPanelHost {
    panel: Option<ChatInlineComposerPanel>,
    last_expanded: Option<bool>,
    presence: Presence<()>,
    insets: (f32, f32),
    on_gone: Option<Rc<dyn Fn(&mut App)>>,
    was_shown: bool,
}

impl ChatInlineComposerPanelHost {
    /// visible is the panel state when the screen first draws the host
    /// a panel there from the start appears in place with no insertion transition
    pub fn new(visible: bool) -> Self {
        Self {
            panel: None,
            last_expanded: None,
            presence: Presence::new(visible.then_some(())).with_transition(Self::transition()),
            insets: (0.0, 0.0),
            on_gone: None,
            was_shown: false,
        }
    }

    /// transition of the inline panel
    pub fn transition() -> Transition {
        Transition::scale_anchored(0.96, UnitPoint::BOTTOM).combined(Transition::opacity())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        panel: ChatInlineComposerPanel,
        is_expanded: bool,
        visible: bool,
        motion: MotionSpec,
        insets: (f32, f32),
        on_gone: Rc<dyn Fn(&mut App)>,
        cx: &mut Context<Self>,
    ) {
        let expanded_changed = self.last_expanded != Some(is_expanded);
        self.last_expanded = Some(is_expanded);
        let mut changed = expanded_changed
            || self.insets != insets
            || self.panel.as_ref().is_none_or(|held| !held.same_data(&panel));
        self.insets = insets;
        self.panel = Some(panel);
        self.on_gone = Some(on_gone);
        if self.presence.is_present() != visible {
            self.presence.set_in(visible.then_some(()), motion, RiseClock::monotonic(), cx);
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
}

impl Render for ChatInlineComposerPanelHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let (leading, trailing) = self.insets;
        let panel = self.panel.clone();
        let content = panel.and_then(|panel| {
            self.presence.render_transitioned(now, window, move |_| {
                div().size_full().pl(px(leading)).pr(px(trailing)).child(panel)
            })
        });
        if content.is_some() {
            self.was_shown = true;
        } else if self.was_shown {
            self.was_shown = false;
            if let Some(on_gone) = self.on_gone.clone() {
                cx.defer(move |cx| on_gone(cx));
            }
        }
        div().size_full().children(content)
    }
}

impl ChatInlineComposerPanel {
    fn same_data(&self, other: &Self) -> bool {
        self.id == other.id
            && self.context == other.context
            && self.candidates == other.candidates
            && self.roles == other.roles
            && self.highlighted == other.highlighted
            && self.commands == other.commands
            && self.results == other.results
            && self.is_loading == other.is_loading
            && self.error == other.error
            && self.available_height == other.available_height
            && self.collapsed_height == other.collapsed_height
            && self.pagination_cache_key == other.pagination_cache_key
            && self.candidates_has_more == other.candidates_has_more
            && self.candidates_cursor == other.candidates_cursor
            && self.is_detached == other.is_detached
            && self.group_commands == other.group_commands
            && self.topics == other.topics
    }
}

/// state of the panel, kept per panel id
struct ChatInlineComposerPanelState {
    drag_translation: f32,
    candidates_scroll_handle: UniformListScrollHandle,
    visual_scroll_handle: ScrollHandle,
    candidates_drag_start: Option<(Point<Pixels>, bool)>,
    empty_page_attempt_count: usize,
    requested_empty_page_keys: HashSet<String>,
    visible_height: Animated<f32>,
    animates_next_height: bool,
    last_trace: Option<String>,
    scrolled_highlight: Option<usize>,
}

impl ChatInlineComposerPanelState {
    fn new() -> Self {
        Self {
            drag_translation: 0.0,
            candidates_scroll_handle: UniformListScrollHandle::new(),
            visual_scroll_handle: ScrollHandle::new(),
            candidates_drag_start: None,
            empty_page_attempt_count: 0,
            requested_empty_page_keys: HashSet::new(),
            visible_height: Animated::new(0.0),
            animates_next_height: false,
            last_trace: None,
            scrolled_highlight: None,
        }
    }

    /// true when the candidates list rests at its top
    /// a zero height sentinel above the first row appears and disappears
    fn candidates_list_at_top(&self) -> bool {
        self.candidates_scroll_handle.0.borrow().base_handle.offset().y >= gpui::px(0.0)
    }
}

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn non_empty(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

struct ChatInlineComposerPanelPass {
    panel: ChatInlineComposerPanel,
    state: Entity<ChatInlineComposerPanelState>,
    theme: AppTheme,
    is_expanded: bool,
    visible_height: f32,
}

impl RenderOnce for ChatInlineComposerPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| ChatInlineComposerPanelState::new());
        let is_expanded = self.is_expanded.get(cx);

        let resetter = state.clone();
        on_change(
            child_id(&self.id, "pagination-cache-key"),
            self.pagination_cache_key.clone(),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                resetter.update(cx, |state, cx| {
                    state.empty_page_attempt_count = 0;
                    state.requested_empty_page_keys.clear();
                    cx.notify();
                });
            },
        );

        let corner = 22.0;
        let bottom_corner = if self.is_detached { corner } else { 0.0 };
        let collapsed = ChatInlineComposerPanelMetrics::height(self.available_height, false, self.collapsed_height);
        let expanded = ChatInlineComposerPanelMetrics::height(
            self.available_height,
            true,
            ChatInlineComposerPanelMetrics::COLLAPSED_HEIGHT,
        );
        let base = if is_expanded { expanded } else { collapsed };
        let target = expanded.min(collapsed.max(base - state.read(cx).drag_translation));

        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let visible_height = state.update(cx, |state, _| {
            if (*state.visible_height.target() - target).abs() > f32::EPSILON {
                if state.animates_next_height && *state.visible_height.target() > 0.0 {
                    state.visible_height.set(target, spring(0.34, 0.86), now, reduce_motion);
                } else {
                    state.visible_height.jump(target);
                }
            }
            state.animates_next_height = false;
            state.visible_height.value(now)
        });
        if state.read(cx).visible_height.is_running(now) {
            window.request_animation_frame();
        }

        let panel_drag_is_supported = Self::panel_drag_is_supported(&self.context);
        let trace = Self::trace_probe(&self, &state, visible_height, target);
        let pass = Rc::new(ChatInlineComposerPanelPass { panel: self, state, theme, is_expanded, visible_height });
        let content = pass.panel_content(window, cx);

        let drag_pass = pass.clone();
        let end_pass = pass.clone();
        let expand_pass = pass.clone();
        let collapse_pass = pass.clone();
        div()
            .w_full()
            .h(gpui::px(visible_height))
            .flex()
            .flex_col()
            .justify_start()
            .overflow_hidden()
            .rounded_tl(gpui::px(corner))
            .rounded_tr(gpui::px(corner))
            .rounded_bl(gpui::px(bottom_corner))
            .rounded_br(gpui::px(bottom_corner))
            .child(content)
            .child(trace)
            .simultaneous_gesture(
                child_id(&pass.panel.id, "panel-drag"),
                DragGesture::new(8.0)
                    .on_changed(move |value, window, cx| {
                        drag_pass.panel_drag_changed(value, collapsed, expanded, window, cx);
                    })
                    .on_ended(move |value, window, cx| end_pass.panel_drag_ended(value, window, cx)),
                if panel_drag_is_supported { GestureMask::All } else { GestureMask::Subviews },
            )
            .keyboard_dismiss_excluded()
            .accessibility_action(tr("chat_inline_expand"), move |window, cx| {
                expand_pass.set_expanded(true, window, cx);
            })
            .accessibility_action(tr("chat_inline_collapse"), move |window, cx| {
                collapse_pass.set_expanded(false, window, cx);
            })
    }
}

impl ChatInlineComposerPanel {
    fn trace_probe(
        &self,
        state: &Entity<ChatInlineComposerPanelState>,
        visible_height: f32,
        target: f32,
    ) -> impl IntoElement + use<> {
        let kind = match &self.context {
            ChatComposerInlineContext::Candidates { .. } => "candidates",
            ChatComposerInlineContext::Topics { .. } => "topics",
            ChatComposerInlineContext::GroupCommands { .. } => "group_commands",
            ChatComposerInlineContext::Bot { .. } => "bot",
            ChatComposerInlineContext::Inactive => "inactive",
        };
        let summary = format!(
            "kind={kind} candidates={} roles={} loading={} error={} has_more={} available={:.1} collapsed={:.1} target={target:.1} visible={visible_height:.1}",
            self.candidates.len(),
            self.roles.len(),
            self.is_loading,
            self.error.is_some(),
            self.candidates_has_more,
            self.available_height,
            self.collapsed_height,
        );
        let state = state.clone();
        gpui::canvas(
            move |bounds, window, cx| {
                let mask = window.content_mask().bounds;
                let line = format!(
                    "{summary} box=({:.1},{:.1} {:.1}x{:.1}) clip=({:.1},{:.1} {:.1}x{:.1})",
                    f32::from(bounds.origin.x),
                    f32::from(bounds.origin.y),
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                    f32::from(mask.origin.x),
                    f32::from(mask.origin.y),
                    f32::from(mask.size.width),
                    f32::from(mask.size.height),
                );
                state.update(cx, |state, _| {
                    if state.last_trace.as_deref() != Some(line.as_str()) {
                        tracing::info!(target: "riseonly::chat_inline_panel", "{line}");
                        state.last_trace = Some(line);
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// the mention list is a plain scroller so a drag over it belongs to the list
    /// the panel gesture there pushed a height into the chat screen every scroll frame
    fn panel_drag_is_supported(context: &ChatComposerInlineContext) -> bool {
        !matches!(context, ChatComposerInlineContext::Candidates { .. } | ChatComposerInlineContext::Topics { .. })
    }
}

impl ChatInlineComposerPanelPass {
    fn panel_content(self: &Rc<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        match &self.panel.context {
            ChatComposerInlineContext::Candidates { .. } => self.candidates_content(window, cx),
            ChatComposerInlineContext::Topics { .. } => self.topics_content(cx),
            ChatComposerInlineContext::GroupCommands { .. } => self.group_commands_content(cx),
            ChatComposerInlineContext::Bot { query, .. } => {
                if trimming_whitespaces_and_newlines(query).is_empty() {
                    self.commands_content(cx)
                } else if let Some(command) = ChatComposerInlineContext::command_awaiting_argument(query) {
                    ChatInlinePanelState::new(
                        child_id(&self.panel.id, "command-ready"),
                        "text.cursor",
                        tr("chat_inline_command_ready").replace("{command}", &command),
                        tr("chat_inline_command_argument_hint"),
                    )
                    .into_any_element()
                } else {
                    self.results_content(window, cx)
                }
            }
            ChatComposerInlineContext::Inactive => div().into_any_element(),
        }
    }

    fn candidates_content(self: &Rc<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        let panel = &self.panel;
        let is_empty = panel.candidates.is_empty() && panel.roles.is_empty();
        if is_empty && panel.is_loading {
            return ChatInlinePanelState::new(
                child_id(&panel.id, "candidates-loading"),
                "person.2",
                tr("chat_inline_loading"),
                tr("chat_inline_loading_hint"),
            )
            .shows_progress(true)
            .into_any_element();
        }
        if is_empty && let Some(error) = &panel.error {
            let on_retry = panel.on_retry.clone();
            return ChatInlinePanelState::new(
                child_id(&panel.id, "candidates-error"),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("chat_inline_error"),
                error.clone(),
            )
            .action(Some(Rc::new(move |window: &mut Window, cx: &mut App| on_retry(window, cx))))
            .into_any_element();
        }
        if is_empty {
            let attempts = self.state.read(cx).empty_page_attempt_count;
            let requires_manual_continuation =
                ChatInlineComposerPresentationContract::requires_manual_empty_page_continuation(
                    panel.candidates.len() + panel.roles.len(),
                    panel.candidates_has_more,
                    panel.candidates_cursor.as_deref(),
                    attempts,
                    MAXIMUM_EMPTY_PAGE_ATTEMPTS,
                );
            let cursor = panel.candidates_cursor.clone();
            self.auto_load_empty_page(
                "candidates",
                cursor,
                panel.candidates.len() + panel.roles.len(),
                panel.candidates_has_more,
                panel.on_load_more_candidates.clone(),
                window,
                cx,
            );
            return self
                .empty_page_state(
                    "candidates-empty",
                    requires_manual_continuation,
                    panel.candidates_has_more,
                    "person.crop.circle.badge.questionmark",
                    (tr("chat_inline_loading"), tr("chat_inline_loading_hint")),
                    (tr("chat_inline_no_candidates"), tr("chat_inline_no_candidates_hint")),
                    panel.on_load_more_candidates.clone(),
                )
                .into_any_element();
        }

        let role_count = panel.roles.len();
        let row_count = role_count + panel.candidates.len() + usize::from(panel.candidates_has_more);
        let rows = self.clone();
        let scroll_handle = self.state.read(cx).candidates_scroll_handle.clone();
        if let Some(index) = self.takes_highlight(cx).filter(|index| *index < row_count) {
            scroll_handle.scroll_to_item(index, ScrollStrategy::Nearest);
        }
        let list = uniform_list(child_id(&panel.id, "candidates"), row_count, move |range, _, _| {
            range.map(|index| rows.candidate_list_row(index)).collect::<Vec<_>>()
        })
        .track_scroll(&scroll_handle)
        .size_full()
        .py(gpui::px(1.0));

        let start_pass = self.clone();
        let end_pass = self.clone();
        div()
            .w_full()
            .h(gpui::px(self.visible_height))
            .child(list)
            .simultaneous_gesture(
                child_id(&panel.id, "candidates-drag"),
                DragGesture::new(12.0)
                    .on_changed(move |value, _, cx| start_pass.candidates_list_drag_changed(value, cx))
                    .on_ended(move |value, window, cx| end_pass.candidates_list_drag_ended(value, window, cx)),
                GestureMask::All,
            )
            .into_any_element()
    }

    fn highlighted_row(&self, position: usize, row: AnyElement) -> AnyElement {
        if self.panel.highlighted != Some(position) {
            return row;
        }
        div().w_full().bg(alpha(self.theme.text.primary, 0.08)).child(row).into_any_element()
    }

    fn takes_highlight(&self, cx: &mut App) -> Option<usize> {
        let highlighted = self.panel.highlighted;
        let changed = self.state.update(cx, |state, _| {
            let changed = state.scrolled_highlight != highlighted;
            state.scrolled_highlight = highlighted;
            changed
        });
        if changed { highlighted } else { None }
    }

    fn candidate_list_row(&self, index: usize) -> AnyElement {
        let panel = &self.panel;
        let role_count = panel.roles.len();
        let row = if index < role_count {
            Some(self.role_row(index))
        } else if index < role_count + panel.candidates.len() {
            Some(self.candidate_row(index - role_count))
        } else {
            None
        };
        if let Some(row) = row {
            return self.highlighted_row(index, row);
        }
        div()
            .w_full()
            .flex()
            .justify_center()
            .padding_edges(EdgeSet::VERTICAL, 12.)
            .child(progress_view().tint(self.theme.primary._100))
            .into_any_element()
    }

    fn role_row(&self, index: usize) -> AnyElement {
        let panel = &self.panel;
        let theme = &self.theme;
        let role = panel.roles[index].clone();
        let role_color = role.swift_color();
        let icon_color = role_color.map(|color| theme.legible(color)).unwrap_or(theme.primary._100);
        let fill = alpha(role_color.unwrap_or(theme.primary._100), 0.12);
        let is_first = panel.roles.first().map(|first| first.id.as_str()) == Some(role.id.as_str());
        let on_role = panel.on_role.clone();
        let pressed = role.clone();
        let row_id = child_id(&panel.id, format!("role-{}", role.id));
        let button = plain(child_id(&row_id, "button"))
            .on_press(move |window, cx| on_role(&pressed, window, cx))
            .w_full()
            .child(
                hstack(8.)
                    .w_full()
                    .padding_edges(EdgeSet::HORIZONTAL, 12.)
                    .min_h(gpui::px(44.))
                    .child(
                        div()
                            .size(gpui::px(33.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(fill)
                            .rounded_full()
                            .children(IconUi::sized("person.3.fill", gpui::px(14.), icon_color, false)),
                    )
                    .child(self.candidate_labels(role.name.clone(), format!("@{}", role.name)))
                    .child(div().flex_none().w(gpui::px(8.))),
            );
        div()
            .id(row_id)
            .w_full()
            .aria_label(format!("@{}, {}", role.name, tr("chat_inline_role")))
            .child(button.guide_anchor(if is_first { Some(RiseGuideAnchor::ChatInlineFirstRole) } else { None }))
            .into_any_element()
    }

    fn candidate_row(&self, index: usize) -> AnyElement {
        let panel = &self.panel;
        let candidate = panel.candidates[index].clone();
        let on_candidate = panel.on_candidate.clone();
        let pressed = candidate.clone();
        let on_load_more = panel.on_load_more_candidates.clone();
        let row_id = child_id(&panel.id, format!("candidate-{}", candidate.id));
        let button = plain(child_id(&row_id, "button"))
            .on_press(move |window, cx| on_candidate(&pressed, window, cx))
            .w_full()
            .child(
                hstack(8.)
                    .w_full()
                    .padding_edges(EdgeSet::HORIZONTAL, 12.)
                    .min_h(gpui::px(44.))
                    .child(
                        UserLogoUi::new(child_id(&row_id, "logo"))
                            .source(Some(SharedString::from(candidate.avatar_url.clone())))
                            .name(Some(SharedString::from(candidate.name.clone())))
                            .user_id(Some(SharedString::from(candidate.user_id.clone())))
                            .size(33.)
                            .show_story_ring(false),
                    )
                    .child(self.candidate_stacked_labels(&candidate))
                    .child(div().flex_none().w(gpui::px(8.))),
            );
        div()
            .id(row_id.clone())
            .w_full()
            .aria_label(Self::candidate_accessibility_label(&candidate))
            .child(button)
            .sai_pagination_prefetch(
                child_id(&row_id, "prefetch"),
                SaiPaginationPrefetchOptions::new(
                    index,
                    panel.candidates.len(),
                    panel.pagination_cache_key.clone(),
                    panel.candidates_cursor.clone(),
                    panel.candidates_has_more,
                )
                .threshold(8),
                move |window, cx| on_load_more(window, cx),
            )
            .into_any_element()
    }

    fn topics_content(self: &Rc<Self>, cx: &mut App) -> AnyElement {
        let panel = &self.panel;
        if panel.topics.is_empty() {
            return ChatInlinePanelState::new(
                child_id(&panel.id, "topics-empty"),
                "number",
                tr("chat_inline_no_topics"),
                String::new(),
            )
            .into_any_element();
        }
        let rows = self.clone();
        let scroll_handle = self.state.read(cx).candidates_scroll_handle.clone();
        if let Some(index) = self.takes_highlight(cx).filter(|index| *index < panel.topics.len()) {
            scroll_handle.scroll_to_item(index, ScrollStrategy::Nearest);
        }
        let list = uniform_list(child_id(&panel.id, "topics"), panel.topics.len(), move |range, _, _| {
            range.map(|index| rows.highlighted_row(index, rows.topic_row(index))).collect::<Vec<_>>()
        })
        .track_scroll(&scroll_handle)
        .size_full()
        .py(gpui::px(1.0));

        let start_pass = self.clone();
        let end_pass = self.clone();
        div()
            .w_full()
            .h(gpui::px(self.visible_height))
            .child(list)
            .simultaneous_gesture(
                child_id(&panel.id, "topics-drag"),
                DragGesture::new(12.0)
                    .on_changed(move |value, _, cx| start_pass.candidates_list_drag_changed(value, cx))
                    .on_ended(move |value, window, cx| end_pass.candidates_list_drag_ended(value, window, cx)),
                GestureMask::All,
            )
            .into_any_element()
    }

    fn topic_row(&self, index: usize) -> AnyElement {
        let panel = &self.panel;
        let topic = panel.topics[index].clone();
        let on_topic = panel.on_topic.clone();
        let pressed = topic.clone();
        let row_id = child_id(&panel.id, format!("topic-{}", topic.id));
        div()
            .id(row_id.clone())
            .w_full()
            .aria_label(format!("#{}", topic.title))
            .child(
                plain(child_id(&row_id, "button")).on_press(move |window, cx| on_topic(&pressed, window, cx)).w_full().child(
                    hstack(8.)
                        .w_full()
                        .padding_edges(EdgeSet::HORIZONTAL, 12.)
                        .min_h(gpui::px(44.))
                        .child(self.topic_glyph(&topic))
                        .child(self.candidate_labels(topic.title.clone(), format!("#{}", topic.title)))
                        .child(div().flex_none().w(gpui::px(8.))),
                ),
            )
            .into_any_element()
    }

    fn topic_glyph(&self, topic: &ChatTopic) -> AnyElement {
        let theme = &self.theme;
        let icon = if topic.icon.is_empty() { ChatTopic::DEFAULT_ICON.to_owned() } else { topic.icon.clone() };
        div()
            .relative()
            .flex_none()
            .size(gpui::px(33.))
            .flex()
            .items_center()
            .justify_center()
            .bg(alpha(theme.primary._100, 0.12))
            .rounded_full()
            .child(TopicIconGlyph::new(icon).size(20.))
            .when(topic.is_rtc_channel(), |glyph| {
                glyph.child(
                    div()
                        .absolute()
                        .right_0()
                        .bottom_0()
                        .size(gpui::px(15.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.bg._100)
                        .rounded_full()
                        .children(IconUi::sized(topic.channel_kind.glyph_name(), gpui::px(9.), theme.primary._100, false)),
                )
            })
            .into_any_element()
    }

    fn group_commands_content(self: &Rc<Self>, cx: &mut App) -> AnyElement {
        let panel = &self.panel;
        if panel.group_commands.is_empty() && panel.is_loading {
            return ChatInlinePanelState::new(
                child_id(&panel.id, "group-commands-loading"),
                "command",
                tr("chat_inline_commands_loading"),
                tr("chat_inline_commands_loading_hint"),
            )
            .shows_progress(true)
            .into_any_element();
        }
        if panel.group_commands.is_empty() && let Some(error) = &panel.error {
            let on_retry = panel.on_retry.clone();
            return ChatInlinePanelState::new(
                child_id(&panel.id, "group-commands-error"),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("chat_inline_error"),
                error.clone(),
            )
            .action(Some(Rc::new(move |window: &mut Window, cx: &mut App| on_retry(window, cx))))
            .into_any_element();
        }
        if panel.group_commands.is_empty() {
            return ChatInlinePanelState::new(
                child_id(&panel.id, "group-commands-empty"),
                "command.circle",
                tr("chat_inline_no_commands"),
                tr("chat_inline_no_commands_hint"),
            )
            .into_any_element();
        }
        let shows_tag = ChatGroupBotCommands::bot_count(&panel.group_commands) > 1;
        let rows = self.clone();
        let fresh = self.takes_highlight(cx);
        self.scroll_disabled_list(
            "group-commands",
            panel.group_commands.len(),
            52.0,
            Rc::new(move |index| rows.group_command_row(index, shows_tag)),
            2.0,
            (0, fresh),
            cx,
        )
    }

    fn group_command_row(&self, index: usize, shows_tag: bool) -> AnyElement {
        let item = self.panel.group_commands[index].clone();
        let on_group_command = self.panel.on_group_command.clone();
        let pressed = item.clone();
        let row_id = child_id(&self.panel.id, format!("group-command-{}", item.id()));
        let label = format!("/{}, {}, @{}", item.command.command, item.command.description, item.bot.tag);
        div()
            .id(row_id.clone())
            .w_full()
            .aria_label(label)
            .child(
                plain(child_id(&row_id, "button"))
                    .on_press(move |window, cx| on_group_command(&pressed, window, cx))
                    .w_full()
                    .child(
                        hstack(10.)
                            .w_full()
                            .padding_edges(EdgeSet::HORIZONTAL, 12.)
                            .min_h(gpui::px(52.))
                            .child(
                                UserLogoUi::new(child_id(&row_id, "logo"))
                                    .source(Some(SharedString::from(item.bot.avatar_url.clone())))
                                    .name(Some(SharedString::from(item.bot.name.clone())))
                                    .user_id(Some(SharedString::from(item.bot.bot_user_id.clone())))
                                    .size(33.)
                                    .show_story_ring(false),
                            )
                            .child(
                                vstack(1.)
                                    .align(HorizontalAlignment::Leading)
                                    .flex_shrink(1.0)
                                    .min_w_0()
                                    .child(
                                        MainText::new(format!("/{}", item.command.command))
                                            .px(15.)
                                            .font_weight(FontWeight::MEDIUM)
                                            .color(self.theme.text.primary)
                                            .number_of_lines(1),
                                    )
                                    .when_some(non_empty(&item.command.description).map(str::to_owned), |labels, description| {
                                        labels.child(SecondaryText::new(description).px(13.).line_limit(1))
                                    }),
                            )
                            .child(spacer_min_w(8.))
                            .when(shows_tag, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .child(SecondaryText::new(format!("@{}", item.bot.tag)).px(13.).line_limit(1)),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn candidate_labels(&self, title: String, tag: String) -> AnyElement {
        hstack(6.)
            .child(
                div().flex_shrink_0().child(
                    MainText::new(title)
                        .px(15.)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(self.theme.text.primary)
                        .number_of_lines(1),
                ),
            )
            .child(div().flex_shrink(1.0).min_w_0().child(SecondaryText::new(tag).px(13.).line_limit(1)))
            .frame_max_width(Alignment::Leading)
            .flex_1()
            .min_w_0()
            .into_any_element()
    }

    fn candidate_stacked_labels(&self, candidate: &BotMentionCandidateModel) -> AnyElement {
        let title = non_empty(&candidate.name).map(str::to_owned).unwrap_or_else(|| format!("@{}", candidate.tag));
        vstack(1.)
            .align(HorizontalAlignment::Leading)
            .child(
                MainText::new(title)
                    .px(15.)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(self.theme.text.primary)
                    .number_of_lines(1),
            )
            .child(
                hstack(4.)
                    .child(
                        div()
                            .flex_shrink(1.0)
                            .min_w_0()
                            .child(SecondaryText::new(format!("@{}", candidate.tag)).px(13.).line_limit(1)),
                    )
                    .when(candidate.is_inline_bot(), |row| {
                        row.child(div().flex_none().child(
                            SecondaryText::new(format!("· {}", tr("chat_inline_bot"))).px(13.).line_limit(1),
                        ))
                    }),
            )
            .padding_edges(EdgeSet::VERTICAL, 4.)
            .frame_max_width(Alignment::Leading)
            .flex_1()
            .min_w_0()
            .into_any_element()
    }

    fn commands_content(self: &Rc<Self>, cx: &mut App) -> AnyElement {
        let panel = &self.panel;
        let values = panel.commands.as_ref().map(|commands| commands.commands.clone()).unwrap_or_default();
        if values.is_empty() && panel.is_loading {
            return ChatInlinePanelState::new(
                child_id(&panel.id, "commands-loading"),
                "command",
                tr("chat_inline_commands_loading"),
                tr("chat_inline_commands_loading_hint"),
            )
            .shows_progress(true)
            .into_any_element();
        }
        if values.is_empty() && let Some(error) = &panel.error {
            let on_retry = panel.on_retry.clone();
            return ChatInlinePanelState::new(
                child_id(&panel.id, "commands-error"),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("chat_inline_error"),
                error.clone(),
            )
            .action(Some(Rc::new(move |window: &mut Window, cx: &mut App| on_retry(window, cx))))
            .into_any_element();
        }
        if values.is_empty() {
            return ChatInlinePanelState::new(
                child_id(&panel.id, "commands-empty"),
                "command.circle",
                tr("chat_inline_no_commands"),
                tr("chat_inline_no_commands_hint"),
            )
            .into_any_element();
        }
        let values = Rc::new(values);
        let rows = self.clone();
        let row_values = values.clone();
        let fresh = self.takes_highlight(cx);
        self.scroll_disabled_list(
            "commands",
            values.len(),
            48.0,
            Rc::new(move |index| rows.command_row(&row_values[index])),
            2.0,
            (0, fresh),
            cx,
        )
    }

    fn command_row(&self, command: &BotCommandModel) -> AnyElement {
        let title = non_empty(&command.description)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("/{}", command.command));
        let on_command = self.panel.on_command.clone();
        let pressed = command.clone();
        let row_id = child_id(&self.panel.id, format!("command-{}", command.id()));
        div()
            .id(row_id.clone())
            .w_full()
            .aria_label(format!("/{}, {}", command.command, command.description))
            .child(
                plain(child_id(&row_id, "button")).on_press(move |window, cx| on_command(&pressed, window, cx)).w_full().child(
                    hstack(8.)
                        .w_full()
                        .padding_edges(EdgeSet::HORIZONTAL, 16.)
                        .h(gpui::px(48.))
                        .child(
                            div().flex_shrink(1.0).min_w_0().child(
                                MainText::new(title)
                                    .px(15.)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(self.theme.text.primary)
                                    .number_of_lines(1),
                            ),
                        )
                        .child(spacer_min_w(8.)),
                ),
            )
            .into_any_element()
    }

    fn results_content(self: &Rc<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        let panel = &self.panel;
        let results = &panel.results;
        if results.results.is_empty() && panel.is_loading {
            return ChatInlinePanelState::new(
                child_id(&panel.id, "results-loading"),
                "sparkle.magnifyingglass",
                tr("chat_inline_results_loading"),
                tr("chat_inline_results_loading_hint"),
            )
            .shows_progress(true)
            .into_any_element();
        }
        if results.results.is_empty() && let Some(error) = &panel.error {
            let on_retry = panel.on_retry.clone();
            return ChatInlinePanelState::new(
                child_id(&panel.id, "results-error"),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("chat_inline_error"),
                error.clone(),
            )
            .action(Some(Rc::new(move |window: &mut Window, cx: &mut App| on_retry(window, cx))))
            .into_any_element();
        }
        if results.results.is_empty() {
            let attempts = self.state.read(cx).empty_page_attempt_count;
            let requires_manual_continuation =
                ChatInlineComposerPresentationContract::requires_manual_empty_page_continuation(
                    results.results.len(),
                    results.has_more,
                    results.next_offset.as_deref(),
                    attempts,
                    MAXIMUM_EMPTY_PAGE_ATTEMPTS,
                );
            self.auto_load_empty_page(
                "results",
                results.next_offset.clone(),
                results.results.len(),
                results.has_more,
                panel.on_load_more_results.clone(),
                window,
                cx,
            );
            return self
                .empty_page_state(
                    "results-empty",
                    requires_manual_continuation,
                    results.has_more,
                    "photo.on.rectangle.angled",
                    (tr("chat_inline_results_loading"), tr("chat_inline_results_loading_hint")),
                    (tr("chat_inline_no_results"), tr("chat_inline_no_results_hint")),
                    panel.on_load_more_results.clone(),
                )
                .into_any_element();
        }

        let visual: Vec<usize> = (0..results.results.len()).filter(|&index| results.results[index].is_visual()).collect();
        let compact: Vec<usize> =
            (0..results.results.len()).filter(|&index| !results.results[index].is_visual()).collect();

        let fresh = self.takes_highlight(cx);
        let mut stack = vstack(0.).align(HorizontalAlignment::Leading).size_full();
        if !visual.is_empty() {
            stack = stack.child(self.visual_results_row(&visual, fresh, cx));
        }
        let visual_count = visual.len();
        if !compact.is_empty() {
            let shows_progress = results.has_more && visual.is_empty();
            let compact = Rc::new(compact);
            let rows = self.clone();
            let row_indices = compact.clone();
            let theme = self.theme.clone();
            let row_count = compact.len() + usize::from(shows_progress);
            stack = stack.child(div().flex_1().min_h_0().w_full().child(self.scroll_disabled_list(
                "compact-results",
                row_count,
                58.0,
                Rc::new(move |position| {
                    if position < row_indices.len() {
                        rows.compact_result_card(row_indices[position])
                    } else {
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .padding_edges(EdgeSet::VERTICAL, 12.)
                            .child(progress_view().tint(theme.primary._100))
                            .into_any_element()
                    }
                }),
                0.0,
                (visual_count, fresh),
                cx,
            )));
        }
        stack.into_any_element()
    }

    /// scroll view disabled when collapsed
    /// expanded it scrolls as a uniform list, collapsed it stays at the top and draws only rows that fit
    fn scroll_disabled_list(
        self: &Rc<Self>,
        name: &str,
        row_count: usize,
        row_height: f32,
        row: Rc<dyn Fn(usize) -> AnyElement>,
        vertical_padding: f32,
        highlight: (usize, Option<usize>),
        cx: &mut App,
    ) -> AnyElement {
        let id = child_id(&self.panel.id, name.to_owned());
        let (first_position, fresh) = highlight;
        let highlighted = self.panel.highlighted.and_then(|position| position.checked_sub(first_position));
        let pass = self.clone();
        let row: Rc<dyn Fn(usize) -> AnyElement> =
            Rc::new(move |index| pass.highlighted_row(first_position + index, row(index)));
        if self.is_expanded || highlighted.is_some_and(|index| index < row_count) {
            let handle = self.state.read(cx).candidates_scroll_handle.clone();
            if let Some(index) =
                fresh.and_then(|position| position.checked_sub(first_position)).filter(|index| *index < row_count)
            {
                handle.scroll_to_item(index, ScrollStrategy::Nearest);
            }
            return uniform_list(id, row_count, move |range, _, _| range.map(|index| row(index)).collect::<Vec<_>>())
                .track_scroll(&handle)
                .size_full()
                .py(gpui::px(vertical_padding))
                .into_any_element();
        }
        let visible_rows = (ChatInlineComposerPanelMetrics::COLLAPSED_HEIGHT / row_height).ceil() as usize + 1;
        let expand_pass = self.clone();
        div()
            .id(id)
            .size_full()
            .overflow_hidden()
            .py(gpui::px(vertical_padding))
            .child(vstack(0.).w_full().children((0..row_count.min(visible_rows)).map(|index| row(index))))
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                let scrolls_down = match event.delta {
                    ScrollDelta::Pixels(delta) => f32::from(delta.y) < 0.0,
                    ScrollDelta::Lines(delta) => delta.y < 0.0,
                };
                cx.stop_propagation();
                if scrolls_down && !expand_pass.is_expanded {
                    expand_pass.set_expanded(true, window, cx);
                }
            })
            .into_any_element()
    }

    fn visual_results_row(&self, visual: &[usize], fresh: Option<usize>, cx: &App) -> AnyElement {
        let panel = &self.panel;
        let handle = self.state.read(cx).visual_scroll_handle.clone();
        let stride = VISUAL_CARD_SIZE + VISUAL_SPACING;
        let viewport = match f32::from(handle.bounds().size.width) {
            width if width > 0.0 => width,
            _ => VISUAL_DEFAULT_VIEWPORT,
        };
        let end = (visual.len() as f32 * stride - viewport).max(0.0);
        if let Some(position) = fresh.filter(|position| *position < visual.len()) {
            let shown = (-f32::from(handle.offset().x) - VISUAL_PADDING).clamp(0.0, end);
            let leading = position as f32 * stride;
            let target = if leading < shown {
                leading
            } else if leading + VISUAL_CARD_SIZE > shown + viewport {
                leading + VISUAL_CARD_SIZE - viewport + VISUAL_PADDING * 2.0
            } else {
                shown
            };
            handle.set_offset(gpui::point(gpui::px(-(target.clamp(0.0, end) + VISUAL_PADDING)), gpui::px(0.0)));
        }
        let scrolled = (-f32::from(handle.offset().x) - VISUAL_PADDING).clamp(0.0, end);
        let first = ((scrolled / stride) as usize).saturating_sub(VISUAL_OVERSCAN).min(visual.len());
        let last = (first + (viewport / stride).ceil() as usize + VISUAL_OVERSCAN * 2).min(visual.len());

        let mut row = div()
            .id(child_id(&panel.id, "visual-results"))
            .w_full()
            .h(gpui::px(VISUAL_CARD_SIZE + VISUAL_PADDING * 2.0))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .p(gpui::px(VISUAL_PADDING))
            .gap(gpui::px(VISUAL_SPACING))
            .overflow_x_scroll()
            .track_scroll(&handle);
        if first > 0 {
            row = row.child(div().flex_none().w(gpui::px(first as f32 * stride - VISUAL_SPACING)).h(gpui::px(1.0)));
        }
        row = row.children(visual[first..last].iter().enumerate().map(|(offset, &index)| {
            let ringed = panel.highlighted == Some(first + offset);
            div()
                .flex_none()
                .when(ringed, |card| {
                    card.rounded(gpui::px(10.0)).border_2().border_color(self.theme.primary._100)
                })
                .child(self.visual_result_card(index, cx))
        }));
        let remaining = visual.len() - last;
        if remaining > 0 {
            row = row.child(div().flex_none().w(gpui::px(remaining as f32 * stride - VISUAL_SPACING)).h(gpui::px(1.0)));
        }
        row.when(panel.results.has_more, |row| {
            row.child(
                div()
                    .id(child_id(&panel.id, "visual-results-more"))
                    .flex_none()
                    .w(gpui::px(44.))
                    .h(gpui::px(VISUAL_CARD_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .aria_label(tr("chat_inline_loading"))
                    .child(progress_view().tint(self.theme.primary._100)),
            )
        })
        .into_any_element()
    }

    fn visual_result_card(&self, index: usize, cx: &App) -> AnyElement {
        let panel = &self.panel;
        let results = &panel.results;
        let result = results.results[index].clone();
        let theme = &self.theme;
        let is_sending = (panel.is_result_sending)(&result, cx);
        let preview_url = result.preview_url();
        let shape = Shape::RoundedRectangle { corner_radius: 8., style: RoundedCornerStyle::Continuous };
        let image: AnyElement = if !preview_url.is_empty() {
            div()
                .size(gpui::px(104.))
                .overflow_hidden()
                .child(
                    CleverImageUi::new(preview_url)
                        .id(child_id(&panel.id, format!("visual-image-{}", result.id)))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(8.)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(312),
                )
                .into_any_element()
        } else {
            div()
                .size(gpui::px(104.))
                .flex()
                .items_center()
                .justify_center()
                .bg(alpha(theme.bg._300, 0.8))
                .clip_shape(shape)
                .children(IconUi::sized(
                    Self::visual_icon(&result.normalized_type()),
                    gpui::px(28.),
                    theme.text.secondary,
                    true,
                ))
                .into_any_element()
        };
        let on_result = panel.on_result.clone();
        let pressed = result.clone();
        let on_load_more = panel.on_load_more_results.clone();
        let card_id = child_id(&panel.id, format!("visual-{}", result.id));
        let disabled = is_sending || result.effective_query_id(&results.query_id).is_empty();
        div()
            .id(card_id.clone())
            .aria_label(self.result_accessibility_label(&result))
            .aria_description(tr("chat_inline_send_hint"))
            .child(
                plain(child_id(&card_id, "button")).on_press(move |window, cx| on_result(&pressed, window, cx)).disabled(disabled).child(
                    zstack(Alignment::Center)
                        .size(gpui::px(104.))
                        .clip_shape(shape)
                        .child(image)
                        .when(is_sending, |card| {
                            card.child(
                                div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(black().opacity(0.38))
                                    .clip_shape(shape)
                                    .child(progress_view().tint(white())),
                            )
                        }),
                ),
            )
            .sai_pagination_prefetch(
                child_id(&card_id, "prefetch"),
                SaiPaginationPrefetchOptions::new(
                    index,
                    results.results.len(),
                    panel.pagination_cache_key.clone(),
                    results.next_offset.clone(),
                    results.has_more,
                )
                .threshold(6),
                move |window, cx| on_load_more(window, cx),
            )
            .into_any_element()
    }

    fn compact_result_card(&self, index: usize) -> AnyElement {
        let panel = &self.panel;
        let results = &panel.results;
        let result = results.results[index].clone();
        let theme = &self.theme;
        let preview_url = result.preview_url();
        let shape = Shape::RoundedRectangle { corner_radius: 12., style: RoundedCornerStyle::Continuous };
        let thumbnail = zstack(Alignment::Center)
            .size(gpui::px(48.))
            .flex_none()
            .clip_shape(shape)
            .child(div().size_full().bg(alpha(theme.primary._100, 0.12)).clip_shape(shape))
            .child(if !preview_url.is_empty() {
                CleverImageUi::new(preview_url)
                    .id(child_id(&panel.id, format!("compact-image-{}", result.id)))
                    .content_mode(ContentMode::Fill)
                    .corner_radius(12.)
                    .persist_to_disk(true)
                    .maximum_decode_pixel_dimension(144)
                    .into_any_element()
            } else {
                div()
                    .children(IconUi::sized(
                        Self::compact_icon(&result.normalized_type()),
                        gpui::px(20.),
                        theme.primary._100,
                        true,
                    ))
                    .into_any_element()
            });
        let title = non_empty(&result.title)
            .or_else(|| result.send_message.as_ref().and_then(|message| non_empty(&message.content)))
            .or_else(|| non_empty(&result.content))
            .map(str::to_owned)
            .unwrap_or_else(|| tr("chat_inline_result"));
        let subtitle = non_empty(&result.description)
            .or_else(|| non_empty(&result.caption))
            .map(str::to_owned)
            .unwrap_or_else(|| Self::localized_result_type(&result.normalized_type()));
        let is_result_sending = self.panel.is_result_sending.clone();
        let sending_result = result.clone();
        let trailing_color = theme.primary._100;
        let on_result = panel.on_result.clone();
        let pressed = result.clone();
        let on_load_more = panel.on_load_more_results.clone();
        let card_id = child_id(&panel.id, format!("compact-{}", result.id));
        let query_id = results.query_id.clone();
        div()
            .id(card_id.clone())
            .w_full()
            .aria_label(self.result_accessibility_label(&result))
            .aria_description(tr("chat_inline_send_hint"))
            .child(ChatInlineCompactResultButton {
                id: child_id(&card_id, "button"),
                result: sending_result,
                query_id,
                is_result_sending,
                on_result,
                pressed,
                thumbnail: thumbnail.into_any_element(),
                title,
                subtitle,
                title_color: theme.text.primary,
                trailing_color,
            })
            .sai_pagination_prefetch(
                child_id(&card_id, "prefetch"),
                SaiPaginationPrefetchOptions::new(
                    index,
                    results.results.len(),
                    panel.pagination_cache_key.clone(),
                    results.next_offset.clone(),
                    results.has_more,
                )
                .threshold(6),
                move |window, cx| on_load_more(window, cx),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn empty_page_state(
        &self,
        name: &str,
        requires_manual_continuation: bool,
        has_more: bool,
        system_image: &'static str,
        loading: (String, String),
        empty: (String, String),
        load_page: ChatInlineComposerPanelAction,
    ) -> ChatInlinePanelState {
        let (title, message) = if requires_manual_continuation {
            (tr("chat_inline_continue_search"), tr("chat_inline_continue_search_hint"))
        } else if has_more {
            loading
        } else {
            empty
        };
        let state = self.state.clone();
        ChatInlinePanelState::new(
            child_id(&self.panel.id, name.to_owned()),
            if requires_manual_continuation { "arrow.down.circle" } else { system_image },
            title,
            message,
        )
        .shows_progress(has_more && !requires_manual_continuation)
        .action_title(requires_manual_continuation.then(|| tr("chat_inline_continue_search_action")))
        .action(requires_manual_continuation.then(|| {
            Rc::new(move |window: &mut Window, cx: &mut App| {
                Self::continue_empty_page_search(&state, &load_page, window, cx);
            }) as ChatInlineComposerPanelAction
        }))
    }

    fn panel_drag_changed(
        &self,
        value: &DragGestureValue,
        collapsed_height: f32,
        expanded_height: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        if expanded_height <= collapsed_height {
            return;
        }
        if !self.panel_drag_is_eligible(value) {
            self.state.update(cx, |state, cx| {
                state.drag_translation = 0.0;
                cx.notify();
            });
            (self.panel.on_height_change)(None, window, cx);
            return;
        }
        let translation = f32::from(value.translation.height);
        self.state.update(cx, |state, cx| {
            state.drag_translation = translation;
            cx.notify();
        });
        let base = if self.is_expanded { expanded_height } else { collapsed_height };
        (self.panel.on_height_change)(Some(expanded_height.min(collapsed_height.max(base - translation))), window, cx);
    }

    fn panel_drag_ended(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        if !self.panel_drag_is_eligible(value) {
            self.state.update(cx, |state, cx| {
                state.drag_translation = 0.0;
                cx.notify();
            });
            (self.panel.on_height_change)(None, window, cx);
            return;
        }
        let projected = f32::from(value.predicted_end_translation.height);
        self.state.update(cx, |state, cx| {
            state.drag_translation = 0.0;
            cx.notify();
        });
        (self.panel.on_height_change)(None, window, cx);
        if projected < -44.0 {
            self.set_expanded(true, window, cx);
        } else if projected > 70.0 && self.is_expanded {
            self.set_expanded(false, window, cx);
        } else if projected > 125.0 {
            (self.panel.on_dismiss)(window, cx);
        } else {
            self.set_expanded(self.is_expanded, window, cx);
        }
    }

    fn candidates_list_drag_changed(&self, value: &DragGestureValue, cx: &mut App) {
        self.state.update(cx, |state, _| {
            if state.candidates_drag_start.map(|(location, _)| location) != Some(value.start_location) {
                state.candidates_drag_start = Some((value.start_location, state.candidates_list_at_top()));
            }
        });
    }

    /// settles the mention panel only when the finger lifts
    /// swipe up opens to full height, swipe down from the top of the list closes
    fn candidates_list_drag_ended(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        let started_at_top = self.state.update(cx, |state, _| {
            let start = state.candidates_drag_start.take();
            match start {
                Some((location, at_top)) if location == value.start_location => at_top,
                _ => state.candidates_list_at_top(),
            }
        });
        let height = f32::from(value.translation.height);
        let width = f32::from(value.translation.width);
        if height.abs() <= width.abs() {
            return;
        }
        let projected = f32::from(value.predicted_end_translation.height);
        if projected < -44.0 && !self.is_expanded {
            self.set_expanded(true, window, cx);
        } else if projected > 70.0 && started_at_top && self.is_expanded {
            self.set_expanded(false, window, cx);
        } else if projected > 125.0 && started_at_top && !self.is_expanded {
            (self.panel.on_dismiss)(window, cx);
        }
    }

    fn panel_drag_is_eligible(&self, value: &DragGestureValue) -> bool {
        if !ChatInlineComposerPanel::panel_drag_is_supported(&self.panel.context) {
            return false;
        }
        let height = f32::from(value.translation.height);
        if height.abs() <= f32::from(value.translation.width).abs() {
            return false;
        }
        if !self.is_expanded {
            return true;
        }
        f32::from(value.start_location.y) <= 28.0 && height > 0.0
    }

    #[allow(clippy::too_many_arguments)]
    fn auto_load_empty_page(
        &self,
        kind: &'static str,
        cursor: Option<String>,
        item_count: usize,
        has_more: bool,
        load_page: ChatInlineComposerPanelAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        let cache_key = self.panel.pagination_cache_key.clone();
        let task_id = format!("{kind}::{cache_key}::{}", cursor.clone().unwrap_or_default());
        let state = self.state.clone();
        use_task(child_id(&self.panel.id, format!("{kind}-empty-page")), task_id, window, cx, async move |cx| {
            let should_load = state.update(cx, |state, cx| {
                if !ChatInlineComposerPresentationContract::should_auto_load_empty_page(
                    item_count,
                    has_more,
                    cursor.as_deref(),
                    state.empty_page_attempt_count,
                    MAXIMUM_EMPTY_PAGE_ATTEMPTS,
                ) {
                    return false;
                }
                let Some(cursor) = cursor.as_deref() else {
                    return false;
                };
                let request_key = format!("{kind}::{cache_key}::{cursor}");
                if !state.requested_empty_page_keys.insert(request_key) {
                    return false;
                }
                state.empty_page_attempt_count += 1;
                cx.notify();
                true
            });
            if !should_load {
                return;
            }
            cx.background_executor().timer(std::time::Duration::ZERO).await;
            let _ = cx.update(|window, cx| load_page(window, cx));
        });
    }

    fn set_expanded(&self, value: bool, window: &mut Window, cx: &mut App) {
        (self.panel.on_height_change)(None, window, cx);
        self.state.update(cx, |state, cx| {
            state.animates_next_height = true;
            cx.notify();
        });
        self.panel.is_expanded.set(value, window, cx);
    }

    fn continue_empty_page_search(
        state: &Entity<ChatInlineComposerPanelState>,
        load_page: &ChatInlineComposerPanelAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| {
            state.empty_page_attempt_count = 0;
            cx.notify();
        });
        load_page(window, cx);
    }

    fn candidate_accessibility_label(candidate: &BotMentionCandidateModel) -> String {
        let kind = if candidate.is_inline_bot() { tr("chat_inline_bot") } else { tr("chat_inline_user") };
        format!("{}, @{}, {}", candidate.name, candidate.tag, kind)
    }

    fn result_accessibility_label(&self, result: &BotInlineResultModel) -> String {
        let title = non_empty(&result.title)
            .or_else(|| non_empty(&result.description))
            .or_else(|| non_empty(&result.content))
            .map(str::to_owned)
            .unwrap_or_else(|| tr("chat_inline_result"));
        format!("{}, {}", title, Self::localized_result_type(&result.normalized_type()))
    }

    fn localized_result_type(value: &str) -> String {
        match value {
            "photo" | "image" => tr("chat_inline_type_photo"),
            "gif" | "animation" => tr("chat_inline_type_gif"),
            "video" => tr("chat_inline_type_video"),
            "audio" => tr("chat_inline_type_audio"),
            "voice" => tr("chat_inline_type_voice"),
            "document" | "file" => tr("chat_inline_type_document"),
            "contact" => tr("chat_inline_type_contact"),
            "location" | "venue" => tr("chat_inline_type_location"),
            "article" | "link" => tr("chat_inline_type_article"),
            _ => tr("chat_inline_type_text"),
        }
    }

    fn visual_icon(value: &str) -> &'static str {
        match value {
            "video" => "play.rectangle.fill",
            "gif" | "animation" => "sparkles.tv.fill",
            _ => "photo.fill",
        }
    }

    fn compact_icon(value: &str) -> &'static str {
        match value {
            "audio" => "music.note",
            "voice" => "waveform",
            "document" | "file" => "doc.fill",
            "contact" => "person.crop.circle.fill",
            "location" | "venue" => "mappin.and.ellipse",
            "article" | "link" => "link",
            _ => "text.bubble.fill",
        }
    }
}

#[derive(IntoElement)]
struct ChatInlineCompactResultButton {
    id: ElementId,
    result: BotInlineResultModel,
    query_id: String,
    is_result_sending: Rc<dyn Fn(&BotInlineResultModel, &App) -> bool>,
    on_result: ChatInlineComposerPanelItemAction<BotInlineResultModel>,
    pressed: BotInlineResultModel,
    thumbnail: AnyElement,
    title: String,
    subtitle: String,
    title_color: Hsla,
    trailing_color: Hsla,
}

impl RenderOnce for ChatInlineCompactResultButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_sending = (self.is_result_sending)(&self.result, cx);
        let disabled = is_sending || self.result.effective_query_id(&self.query_id).is_empty();
        let on_result = self.on_result;
        let pressed = self.pressed;
        let trailing: AnyElement = if is_sending {
            progress_view().control_size(ControlSize::Small).tint(self.trailing_color).into_any_element()
        } else {
            div()
                .children(IconUi::sized("paperplane.fill", gpui::px(14.), self.trailing_color, true))
                .into_any_element()
        };
        plain(self.id).on_press(move |window, cx| on_result(&pressed, window, cx)).disabled(disabled).w_full().child(
            hstack(12.)
                .w_full()
                .padding_edges(EdgeSet::HORIZONTAL, 12.)
                .padding_edges(EdgeSet::VERTICAL, 5.)
                .min_h(gpui::px(58.))
                .child(self.thumbnail)
                .child(
                    vstack(3.)
                        .align(HorizontalAlignment::Leading)
                        .flex_shrink(1.0)
                        .min_w_0()
                        .child(
                            MainText::new(self.title)
                                .px(14.)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(self.title_color)
                                .number_of_lines(1),
                        )
                        .child(SecondaryText::new(self.subtitle).px(12.).line_limit(2)),
                )
                .child(spacer_min_w(8.))
                .child(trailing),
        )
    }
}

#[derive(IntoElement)]
struct ChatInlinePanelState {
    id: ElementId,
    system_image: &'static str,
    title: String,
    message: String,
    shows_progress: bool,
    action_title: Option<String>,
    action: Option<ChatInlineComposerPanelAction>,
}

impl ChatInlinePanelState {
    fn new(id: ElementId, system_image: &'static str, title: String, message: String) -> Self {
        Self { id, system_image, title, message, shows_progress: false, action_title: None, action: None }
    }

    fn shows_progress(mut self, shows_progress: bool) -> Self {
        self.shows_progress = shows_progress;
        self
    }

    fn action_title(mut self, action_title: Option<String>) -> Self {
        self.action_title = action_title;
        self
    }

    fn action(mut self, action: Option<ChatInlineComposerPanelAction>) -> Self {
        self.action = action;
        self
    }
}

impl RenderOnce for ChatInlinePanelState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let indicator: AnyElement = if self.shows_progress {
            progress_view().tint(theme.primary._100).into_any_element()
        } else {
            div().children(IconUi::sized(self.system_image, gpui::px(24.), theme.text.secondary, true)).into_any_element()
        };
        vstack(8.)
            .child(indicator)
            .child(
                MainText::new(self.title)
                    .px(14.)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary)
                    .number_of_lines(2),
            )
            .child(SecondaryText::new(self.message).px(12.).tac(TextAlignT::Center).line_limit(3))
            .when_some(self.action, |stack, action| {
                let title = self.action_title.unwrap_or_else(|| tr("common_retry"));
                stack.child(
                    system_button(ElementId::NamedChild(Arc::new(self.id.clone()), "action".into()), BorderedStyle::Bordered)
                        .control_size(ControlSize::Small)
                        .on_press(move |window, cx| action(window, cx))
                        .child(SharedString::from(title)),
                )
            })
            .p(gpui::px(16.))
            .frame_max(Alignment::Center)
    }
}

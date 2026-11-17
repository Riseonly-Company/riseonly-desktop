use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AsyncWindowContext, Entity, FontWeight, ListAlignment, ListOffset, ListState, Pixels, Point, SharedString, Size,
    Subscription, Window,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::{tr, tr_plural};
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::list_ui::ListScrollAnimation;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::{IconUi, ListUi};
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAction, HoldContextMenuAlignment, HoldSourceBubbleClip,
};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuContainer, HoldContextMenuControllerPreviewFactory, HoldContextMenuPointHandler,
    HoldContextMenuSourceContent, HoldContextMenuStringHandler,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatWallpaperPrewarm;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::ChatWallpaperSheet;
use crate::modules::chat::components::chat::composer::chat_draft_store::{ChatDraftPreview, ChatDraftStore};
use crate::modules::chat::components::chat::topic_icon_glyph::TopicIconGlyph;
use crate::modules::chat::components::chat_item::chat_context_preview::ChatContextPreview;
use crate::modules::chat::components::chat_item::chat_list_preview_row::{
    ChatListPreviewFactory, ChatListPreviewPresentation, ChatListPreviewRow,
};
use crate::modules::chat::engine::forum::rise_forum_voice_presence::{
    RiseForumVoicePresenceSnapshot, RiseForumVoicePresentation,
};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile_more_sheets::TopicNotificationSheet;
use crate::modules::chat::pages::chats::chat::chat_topics::topic_editor_sheet::TopicEditorSheet;
use crate::modules::chat::pages::chats::chat::forum_home::channel_visibility_editor::{
    ChannelVisibilityEditor, VisibilityTarget, VisibilityTargetKind,
};
use crate::modules::chat::pages::chats::chat::forum_home::forum_topic_focus_presenter::{
    ForumTopicFocusPolicy, ForumTopicFocusPresenter, ForumTopicFocusPresenterRequest,
};
use crate::modules::chat::pages::chats::chat::forum_home::forum_section_collapse_preferences::ForumSectionCollapsePreferences;
use crate::modules::chat::pages::chats::chat::forum_home::forum_section_name_sheet::{
    ForumSectionNameSheetHost, ForumSectionNameTarget, ForumSectionTitleText,
};
use crate::modules::chat::pages::chats::chat::forum_home::forum_voice_channel_row::{
    ForumVoiceChannelHeaderRow, ForumVoiceOverflowRow, ForumVoiceParticipantPlaceholderRow, ForumVoiceParticipantRow,
};
use crate::modules::chat::pages::chats::embedded_forum_horizontal_scroll_gate::EmbeddedForumHorizontalScrollGate;
use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumAction, EmbeddedForumOpenTopicAction, EmbeddedForumTransitionMetrics, EmbeddedForumTransitionState,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatInfo, ChatSection, ChatTopic, ChatWallpaperPlacement,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_roles_services::ForumTopicListSnapshot;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageCustomEmojiEntity;
use crate::modules::notify::components::notifications_hub_sheet_store::NotificationsHubSheetStore;
use crate::modules::rtc::pages::rise_rtc_voice_invite_sheet::RiseRTCVoiceInviteBottomSheet;

pub enum ForumHomeInitialRefreshPolicy {}

impl ForumHomeInitialRefreshPolicy {
    pub const SETTLED_DELAY_NANOSECONDS: u64 =
        (EmbeddedForumTransitionMetrics::BACKGROUND_REFRESH_DELAY_DURATION * 1_000_000_000.0) as u64;
    pub const INTERACTION_RETRY_DELAY_NANOSECONDS: u64 =
        (EmbeddedForumTransitionMetrics::BACKGROUND_WORK_RETRY_INTERVAL * 1_000_000_000.0) as u64;

    pub fn delay_nanoseconds(_has_cached_structure: bool) -> u64 {
        Self::SETTLED_DELAY_NANOSECONDS
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForumHomeColdPresentationPhase {
    Content,
    Loading,
    TimedOut,
}

pub enum ForumHomeColdPresentationPolicy {}

impl ForumHomeColdPresentationPolicy {
    pub const TIMEOUT_NANOSECONDS: u64 = 8_000_000_000;
    pub const PLACEHOLDER_ROW_COUNT: usize = 5;
    pub const PLACEHOLDER_ROW_HEIGHT: f32 = 70.0;
    pub const STATUS_ROW_HEIGHT: f32 = 36.0;

    pub fn phase(has_cached_structure: bool, did_time_out: bool) -> ForumHomeColdPresentationPhase {
        if has_cached_structure {
            return ForumHomeColdPresentationPhase::Content;
        }
        if did_time_out { ForumHomeColdPresentationPhase::TimedOut } else { ForumHomeColdPresentationPhase::Loading }
    }
}

/// non observable last committed value
/// during a panel pan the view stops reading revision backed caches
/// so completions cant invalidate the topic hierarchy
#[derive(Default)]
struct ForumTopicPresentationSnapshotBuffer {
    chat_id: Option<String>,
    snapshot: Option<ForumTopicListSnapshot>,
}

impl ForumTopicPresentationSnapshotBuffer {
    fn store(&mut self, snapshot: ForumTopicListSnapshot, chat_id: &str) {
        self.chat_id = Some(chat_id.to_owned());
        self.snapshot = Some(snapshot);
    }

    fn snapshot(&self, chat_id: &str) -> Option<ForumTopicListSnapshot> {
        if self.chat_id.as_deref() == Some(chat_id) { self.snapshot.clone() } else { None }
    }
}

/// a short presentation can disappear before its deferred subscription starts
/// release only an observation owned by this view and not another screen
#[derive(Default)]
pub struct ForumVoiceObservationLifecycle {
    chat_id: Option<String>,
    channel_signature: String,
}

impl ForumVoiceObservationLifecycle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(
        &mut self,
        chat_id: &str,
        channel_signature: &str,
        accepts_live_snapshot: bool,
        begin: impl FnOnce(&str),
        end: impl FnOnce(&str),
    ) {
        if !accepts_live_snapshot || chat_id.is_empty() || channel_signature.is_empty() {
            self.stop(end);
            return;
        }
        if self.chat_id.as_deref().is_some_and(|previous| previous != chat_id) {
            self.stop(end);
        }
        if self.chat_id.as_deref() == Some(chat_id) && self.channel_signature == channel_signature {
            return;
        }
        self.chat_id = Some(chat_id.to_owned());
        self.channel_signature = channel_signature.to_owned();
        begin(chat_id);
    }

    pub fn stop(&mut self, end: impl FnOnce(&str)) {
        let Some(chat_id) = self.chat_id.take() else {
            return;
        };
        self.channel_signature.clear();
        end(&chat_id);
    }
}

/// the item clears the moment the sheet starts to close
/// the sheet keeps showing it while it slides out
struct ForumSheetItem<T> {
    item: Option<T>,
    retained: Option<T>,
}

impl<T: Clone> ForumSheetItem<T> {
    fn empty() -> Self {
        Self { item: None, retained: None }
    }

    fn present(&mut self, value: T) {
        self.retained = Some(value.clone());
        self.item = Some(value);
    }
}

/// state of the topic list view
/// stores it observes redraw the owner only when something the body reads moved
/// transition and menu presenter notify on every gesture frame
struct ForumTopicListContentState {
    visibility_target: ForumSheetItem<VisibilityTarget>,
    notification_topic: ForumSheetItem<ChatTopic>,
    voice_invite_topic: Option<ChatTopic>,
    wallpaper_topic: ForumSheetItem<ChatTopic>,
    show_chat_wallpaper: bool,
    show_general_wallpaper: bool,
    show_topic_editor: bool,
    has_mounted_topic_editor: bool,
    voice_observation: Rc<RefCell<ForumVoiceObservationLifecycle>>,
    editor_topic_id: Option<String>,
    pending_section_id: Option<String>,
    section_name_target: Option<ForumSectionNameTarget>,
    accepts_live_presentation_snapshot: bool,
    cold_load_attempt_scope: Option<String>,
    cold_load_attempt_generation: u64,
    cold_load_timed_out_scope: Option<String>,
    presentation_snapshot_buffer: ForumTopicPresentationSnapshotBuffer,
    has_active_hold_menu: bool,
    is_connected: bool,
    structural_refresh_generation: u64,
    list: ListUi,
    row_keys: Vec<String>,
    focus_request: Option<ForumTopicFocusPresenterRequest>,
    focus_scroll: ListScrollAnimation,
    _subscriptions: Vec<Subscription>,
}

impl ForumTopicListContentState {
    fn new(presentation_transition: Option<Entity<EmbeddedForumTransitionState>>, cx: &mut gpui::Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let drafts = ChatDraftStore::shared(cx);
        let forum_voice = RiseForumVoicePresentation::shared(cx);
        let presenter = HoldContextMenuPresenter::shared(cx);
        let manager = SaiWs::manager(cx);
        let collapse = ForumSectionCollapsePreferences::shared(cx);
        let has_active_hold_menu = presenter.read(cx).active_request().is_some();
        let is_connected = manager.read(cx).is_connected();
        let structural_refresh_generation =
            presentation_transition.as_ref().map_or(0, |transition| transition.read(cx).structural_refresh_generation());
        let mut subscriptions = vec![
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&drafts, |_, _, cx| cx.notify()),
            cx.observe(&forum_voice, |_, _, cx| cx.notify()),
            cx.observe(&collapse, |_, _, cx| cx.notify()),
            cx.observe(&presenter, |this: &mut Self, presenter, cx| {
                let has_active = presenter.read(cx).active_request().is_some();
                if this.has_active_hold_menu != has_active {
                    this.has_active_hold_menu = has_active;
                    cx.notify();
                }
            }),
            cx.observe(&manager, |this: &mut Self, manager, cx| {
                let connected = manager.read(cx).is_connected();
                if this.is_connected != connected {
                    this.is_connected = connected;
                    cx.notify();
                }
            }),
        ];
        if let Some(transition) = &presentation_transition {
            subscriptions.push(cx.observe(transition, |this: &mut Self, transition, cx| {
                let generation = transition.read(cx).structural_refresh_generation();
                if this.structural_refresh_generation != generation {
                    this.structural_refresh_generation = generation;
                    cx.notify();
                }
            }));
        }
        Self {
            visibility_target: ForumSheetItem::empty(),
            notification_topic: ForumSheetItem::empty(),
            voice_invite_topic: None,
            wallpaper_topic: ForumSheetItem::empty(),
            show_chat_wallpaper: false,
            show_general_wallpaper: false,
            show_topic_editor: false,
            has_mounted_topic_editor: false,
            voice_observation: Rc::new(RefCell::new(ForumVoiceObservationLifecycle::new())),
            editor_topic_id: None,
            pending_section_id: None,
            section_name_target: None,
            accepts_live_presentation_snapshot: false,
            cold_load_attempt_scope: None,
            cold_load_attempt_generation: 0,
            cold_load_timed_out_scope: None,
            presentation_snapshot_buffer: ForumTopicPresentationSnapshotBuffer::default(),
            has_active_hold_menu,
            is_connected,
            structural_refresh_generation,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            focus_request: None,
            focus_scroll: ListScrollAnimation::default(),
            _subscriptions: subscriptions,
        }
    }

    fn register_cold_load_attempt(&mut self, scope: &str) {
        self.cold_load_timed_out_scope = None;
        self.cold_load_attempt_scope = Some(scope.to_owned());
        self.cold_load_attempt_generation = self.cold_load_attempt_generation.wrapping_add(1);
    }

    fn apply_row_keys(&mut self, keys: Vec<String>) {
        if self.row_keys == keys {
            return;
        }
        let (old_range, new_count) = changed_row_span(&self.row_keys, &keys);
        self.list.state().splice(old_range, new_count);
        self.row_keys = keys;
    }
}

/// rows between the common head and the common tail of two row lists
/// the old range and how many rows replace it
pub(crate) fn changed_row_span(old: &[String], new: &[String]) -> (std::ops::Range<usize>, usize) {
    let head = old.iter().zip(new).take_while(|(old, new)| old == new).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    (head..old.len() - tail, new.len() - head - tail)
}

/// one row of the list, a section hands its header and rows and footer over one by one
/// so a category isnt built whole before the first frame
enum ForumTopicListRow {
    TopInset,
    SectionHeader(Option<ChatSection>),
    All,
    General,
    Topic(ChatTopic),
    SectionEmpty(String),
    CreateTopic(Option<String>),
    SectionFooter(Option<String>),
    CreateSection,
    Cold,
    BottomInset,
}

impl ForumTopicListRow {
    fn rows(
        cold_phase: ForumHomeColdPresentationPhase,
        all_topics_view_hidden: bool,
        snapshot: &ForumTopicListSnapshot,
        collapsed_section_ids: &[String],
    ) -> Vec<Self> {
        let mut rows = vec![Self::TopInset];
        match cold_phase {
            ForumHomeColdPresentationPhase::Content => {
                rows.push(Self::SectionHeader(None));
                if !all_topics_view_hidden {
                    rows.push(Self::All);
                }
                if !snapshot.general_hidden {
                    rows.push(Self::General);
                }
                rows.extend(snapshot.grouping.uncategorized.iter().cloned().map(Self::Topic));
                let top_block_empty = all_topics_view_hidden
                    && snapshot.general_hidden
                    && snapshot.grouping.uncategorized.is_empty()
                    && snapshot.sections.is_empty();
                if top_block_empty && !snapshot.can_create_channels() {
                    rows.push(Self::SectionEmpty(String::new()));
                }
                if snapshot.can_create_channels() {
                    rows.push(Self::CreateTopic(None));
                }
                rows.push(Self::SectionFooter(None));

                for section in &snapshot.sections {
                    rows.push(Self::SectionHeader(Some(section.clone())));
                    if collapsed_section_ids.contains(&section.id) {
                        rows.push(Self::SectionFooter(Some(section.id.clone())));
                        continue;
                    }
                    let section_topics = snapshot.grouping.topics(&section.id);
                    rows.extend(section_topics.iter().cloned().map(Self::Topic));
                    if section_topics.is_empty() && !snapshot.can_create_channels() {
                        rows.push(Self::SectionEmpty(section.id.clone()));
                    }
                    if snapshot.can_create_channels() {
                        rows.push(Self::CreateTopic(Some(section.id.clone())));
                    }
                    rows.push(Self::SectionFooter(Some(section.id.clone())));
                }

                if snapshot.can_manage_sections() {
                    rows.push(Self::CreateSection);
                }
            }
            ForumHomeColdPresentationPhase::Loading | ForumHomeColdPresentationPhase::TimedOut => {
                rows.push(Self::Cold);
            }
        }
        rows.push(Self::BottomInset);
        rows
    }

    /// identity of the row, a row that changes height keeps its key
    /// the list measures every row it shows on each frame
    fn key(&self) -> String {
        match self {
            Self::TopInset => "top".to_owned(),
            Self::SectionHeader(None) => "header".to_owned(),
            Self::SectionHeader(Some(section)) => format!("header-{}", section.id),
            Self::All => "all".to_owned(),
            Self::General => "general".to_owned(),
            Self::Topic(topic) => format!("topic-{}", topic.id),
            Self::SectionEmpty(section_id) => format!("empty-{section_id}"),
            Self::CreateTopic(None) => "create-topic".to_owned(),
            Self::CreateTopic(Some(section_id)) => format!("create-topic-{section_id}"),
            Self::SectionFooter(None) => "footer".to_owned(),
            Self::SectionFooter(Some(section_id)) => format!("footer-{section_id}"),
            Self::CreateSection => "create-section".to_owned(),
            Self::Cold => "cold".to_owned(),
            Self::BottomInset => "bottom".to_owned(),
        }
    }
}

/// what every row reads from one pass of the body
struct ForumTopicListRowContext {
    snapshot: Rc<ForumTopicListSnapshot>,
    first_voice_channel_id: Option<String>,
    first_text_topic_id: Option<String>,
    theme: AppTheme,
    top_inset: f32,
    bottom_inset: f32,
    cold_phase: ForumHomeColdPresentationPhase,
    cold_view_id: String,
    on_cold_retry: EmbeddedForumAction,
}

#[derive(IntoElement)]
pub struct ForumTopicListContent {
    chat_id: String,
    top_inset: f32,
    bottom_inset: f32,
    interaction_scroll_locked: bool,
    installs_horizontal_swipe_gate: bool,
    initial_presentation_snapshot: Option<ForumTopicListSnapshot>,
    presentation_epoch: u64,
    presentation_transition: Option<Entity<EmbeddedForumTransitionState>>,
    on_open_all: EmbeddedForumAction,
    on_open_topic: EmbeddedForumOpenTopicAction,
}

impl ForumTopicListContent {
    pub fn new(
        chat_id: String,
        on_open_all: EmbeddedForumAction,
        on_open_topic: EmbeddedForumOpenTopicAction,
    ) -> Self {
        Self {
            chat_id,
            top_inset: 0.0,
            bottom_inset: 32.0,
            interaction_scroll_locked: false,
            installs_horizontal_swipe_gate: false,
            initial_presentation_snapshot: None,
            presentation_epoch: 0,
            presentation_transition: None,
            on_open_all,
            on_open_topic,
        }
    }

    pub fn top_inset(mut self, top_inset: f32) -> Self {
        self.top_inset = top_inset;
        self
    }

    pub fn bottom_inset(mut self, bottom_inset: f32) -> Self {
        self.bottom_inset = bottom_inset;
        self
    }

    pub fn interaction_scroll_locked(mut self, interaction_scroll_locked: bool) -> Self {
        self.interaction_scroll_locked = interaction_scroll_locked;
        self
    }

    pub fn installs_horizontal_swipe_gate(mut self, installs_horizontal_swipe_gate: bool) -> Self {
        self.installs_horizontal_swipe_gate = installs_horizontal_swipe_gate;
        self
    }

    pub fn initial_presentation_snapshot(mut self, snapshot: Option<ForumTopicListSnapshot>) -> Self {
        self.initial_presentation_snapshot = snapshot;
        self
    }

    pub fn presentation_epoch(mut self, presentation_epoch: u64) -> Self {
        self.presentation_epoch = presentation_epoch;
        self
    }

    pub fn presentation_transition(mut self, transition: Option<Entity<EmbeddedForumTransitionState>>) -> Self {
        self.presentation_transition = transition;
        self
    }

    /// kept for hosts that still pass it
    /// creating a section now opens the shared name sheet, the same one renaming uses
    pub fn on_create_section(self, _on_create_section: EmbeddedForumAction) -> Self {
        self
    }

    fn resolved_presentation_snapshot(
        &self,
        state: &Entity<ForumTopicListContentState>,
        structural_updates_blocked: bool,
        cx: &mut App,
    ) -> ForumTopicListSnapshot {
        let chat_id = self.chat_id.as_str();
        let accepts_live = state.read(cx).accepts_live_presentation_snapshot;
        if !accepts_live {
            if let Some(initial) = self.initial_presentation_snapshot.as_ref().filter(|initial| initial.chat.id == chat_id)
            {
                let initial = initial.clone();
                state.update(cx, |state, _| state.presentation_snapshot_buffer.store(initial.clone(), chat_id));
                return initial;
            }
        }
        if structural_updates_blocked {
            if let Some(buffered) = state.read(cx).presentation_snapshot_buffer.snapshot(chat_id) {
                return buffered;
            }
        }

        let live = ChatServicesStore::shared(cx).update(cx, |services, cx| services.forum_topic_list_snapshot(chat_id, cx));
        let stored = live.clone();
        state.update(cx, |state, _| state.presentation_snapshot_buffer.store(stored, chat_id));
        live
    }

    pub fn picture_wallpapers(snapshot: &ForumTopicListSnapshot) -> Vec<ChatWallpaperPlacement> {
        let mut topics: Vec<&ChatTopic> = snapshot.general_topic.iter().collect();
        topics.extend(snapshot.grouping.uncategorized.iter());
        for section in &snapshot.sections {
            topics.extend(snapshot.grouping.topics(&section.id).iter());
        }
        topics
            .into_iter()
            .filter_map(|topic| topic.wallpaper.clone())
            .chain(snapshot.chat.wallpaper.clone())
            .filter(|wallpaper| !wallpaper.image_url.is_empty())
            .collect()
    }

    fn topic(topic_id: &str, snapshot: &ForumTopicListSnapshot) -> Option<ChatTopic> {
        if let Some(topic) = snapshot.grouping.uncategorized.iter().find(|topic| topic.id == topic_id) {
            return Some(topic.clone());
        }
        snapshot
            .sections
            .iter()
            .find_map(|section| snapshot.grouping.topics(&section.id).iter().find(|topic| topic.id == topic_id))
            .cloned()
    }

    fn first_voice_channel_id(snapshot: &ForumTopicListSnapshot) -> Option<String> {
        if let Some(topic) = snapshot.grouping.uncategorized.iter().find(|topic| topic.is_rtc_channel()) {
            return Some(topic.id.clone());
        }
        snapshot.sections.iter().find_map(|section| {
            snapshot.grouping.topics(&section.id).iter().find(|topic| topic.is_rtc_channel()).map(|topic| topic.id.clone())
        })
    }

    /// text topic a guide points at in reading order, general isnt one
    /// it has its own row and no menu of its own
    fn first_text_topic_id(snapshot: &ForumTopicListSnapshot) -> Option<String> {
        if let Some(topic) = snapshot.grouping.uncategorized.iter().find(|topic| !topic.is_rtc_channel()) {
            return Some(topic.id.clone());
        }
        snapshot.sections.iter().find_map(|section| {
            snapshot.grouping.topics(&section.id).iter().find(|topic| !topic.is_rtc_channel()).map(|topic| topic.id.clone())
        })
    }

    fn voice_channel_signature(snapshot: &ForumTopicListSnapshot) -> String {
        let mut ids: Vec<&str> = Vec::new();
        for topic in snapshot.grouping.uncategorized.iter().filter(|topic| topic.is_rtc_channel()) {
            ids.push(&topic.id);
        }
        for section in &snapshot.sections {
            for topic in snapshot.grouping.topics(&section.id).iter().filter(|topic| topic.is_rtc_channel()) {
                ids.push(&topic.id);
            }
        }
        ids.sort_unstable();
        ids.join(",")
    }
}

fn begin_voice_observation(chat_id: &str, cx: &mut App) {
    let operation = ChatPerformanceTrace::shared().begin_operation("forum.voice_start", HashMap::new());
    ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
        interactions.begin_observing_voice_channels(chat_id, cx);
    });
    ChatPerformanceTrace::shared().end_operation(operation, 0.0, HashMap::new());
}

fn end_voice_observation(chat_id: &str, cx: &mut App) {
    let operation = ChatPerformanceTrace::shared().begin_operation("forum.voice_stop", HashMap::new());
    ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
        interactions.end_observing_voice_channels(chat_id, cx);
    });
    ChatPerformanceTrace::shared().end_operation(operation, 0.0, HashMap::new());
}

/// yields so the rest of the task lands on a later main turn
async fn task_yield(cx: &mut AsyncWindowContext) {
    cx.background_executor().timer(Duration::ZERO).await;
}

/// motion fence of the task, waits until the panel stops moving
/// answers false when it isnt presented anymore
/// polls coarsely so it doesnt fight the gesture
async fn wait_for_settled_presentation(
    transition: &Entity<EmbeddedForumTransitionState>,
    cx: &mut AsyncWindowContext,
) -> bool {
    wait_for_settled_presentation_while(transition, cx, |_| true).await
}

async fn wait_for_settled_presentation_while(
    transition: &Entity<EmbeddedForumTransitionState>,
    cx: &mut AsyncWindowContext,
    still_current: impl Fn(&mut AsyncWindowContext) -> bool,
) -> bool {
    loop {
        let (is_presented, is_moving) = transition.read_with(cx, |transition, _| {
            (
                transition.is_presented(),
                transition.is_interactive_gesture_active()
                    || transition.is_interaction_settling()
                    || transition.has_interactive_offset(),
            )
        });
        if !is_presented {
            return false;
        }
        if !is_moving {
            return true;
        }
        cx.background_executor()
            .timer(Duration::from_nanos(ForumHomeInitialRefreshPolicy::INTERACTION_RETRY_DELAY_NANOSECONDS))
            .await;
        if !still_current(cx) {
            return false;
        }
    }
}

#[derive(Clone)]
struct ForumTopicListParts {
    chat_id: String,
    state: Entity<ForumTopicListContentState>,
    nav_store: Option<Entity<NavigationStore>>,
    on_open_all: EmbeddedForumAction,
    on_open_topic: EmbeddedForumOpenTopicAction,
}

impl ForumTopicListParts {
    async fn focus_topic(self, request: ForumTopicFocusPresenterRequest, cx: &mut AsyncWindowContext) {
        let polls = (ForumTopicFocusPolicy::TOPIC_WAIT.as_millis() / ForumTopicFocusPolicy::POLL.as_millis()).max(1);
        let mut found: Option<ChatTopic> = None;
        for _ in 0..polls {
            let step = cx.update(|_, cx| {
                if ForumTopicFocusPresenter::shared(cx).read(cx).pending() != Some(&request) {
                    return None;
                }
                if !self.state.read(cx).accepts_live_presentation_snapshot {
                    return Some(None);
                }
                let snapshot = ChatServicesStore::shared(cx)
                    .update(cx, |services, cx| services.forum_topic_list_snapshot(&self.chat_id, cx));
                Some(ForumTopicListContent::topic(&request.topic_id, &snapshot))
            });
            match step {
                Ok(Some(Some(topic))) => {
                    found = Some(topic);
                    break;
                }
                Ok(Some(None)) => {}
                Ok(None) | Err(_) => break,
            }
            cx.background_executor().timer(ForumTopicFocusPolicy::POLL).await;
        }
        let Some(topic) = found else {
            let _ = cx.update(|_, cx| {
                let presenter = ForumTopicFocusPresenter::shared(cx);
                if !presenter.update(cx, |presenter, cx| presenter.finish(&request, cx)) {
                    return;
                }
                if let Some(nav_store) = &self.nav_store {
                    let route = RootRoute::ChatTopic {
                        chat_id: Some(self.chat_id.clone()),
                        topic_id: Some(request.topic_id.clone()),
                    };
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Wait, cx));
                }
            });
            return;
        };
        let row_key = ForumTopicListRow::Topic(topic.clone()).key();
        let _ = cx.update(|_, cx| {
            self.state.update(cx, |state, cx| {
                let Some(index) = state.row_keys.iter().position(|key| *key == row_key) else { return };
                let target = centered_offset(state.list.state(), index);
                state.focus_scroll.scroll_to(
                    state.list.state(),
                    target,
                    ease_in_out(ForumTopicFocusPolicy::SCROLL_SECONDS),
                    cx,
                );
                cx.notify();
            });
        });
        cx.background_executor().timer(ForumTopicFocusPolicy::OPEN_AFTER_SCROLL).await;
        let _ = cx.update(|window, cx| {
            let presenter = ForumTopicFocusPresenter::shared(cx);
            if !presenter.update(cx, |presenter, cx| presenter.finish(&request, cx)) {
                return;
            }
            ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.topic_opened(&self.chat_id, Some(&topic.id), cx);
            });
            (self.on_open_topic)(Some(topic), window, cx);
        });
    }

    fn update_state(&self, cx: &mut App, action: impl FnOnce(&mut ForumTopicListContentState)) {
        self.state.update(cx, |state, cx| {
            action(state);
            cx.notify();
        });
    }

    fn can_manage_channel(&self, topic: &ChatTopic, cx: &mut App) -> bool {
        ChatServicesStore::shared(cx).update(cx, |services, cx| {
            services.can_manage_channel(&self.chat_id, Some(&topic.id), topic.channel_kind, cx)
        })
    }

    fn present_notifications_hub(&self, cx: &mut App) {
        let services = ChatServicesStore::shared(cx);
        let chat_scopes = services.read(cx).chat_info(&self.chat_id, cx).map(|chat| chat.notification_scopes());
        NotificationsHubSheetStore::shared(cx).update(cx, |store, cx| {
            store.present(Some(&self.chat_id), chat_scopes, None, cx);
        });
    }

    fn begin_cold_load_attempt(&self, scope: &str, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.register_cold_load_attempt(scope);
            cx.notify();
        });
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_forum_home(&self.chat_id, cx);
        });
    }

    fn topic_draft(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Option<ChatDraftPreview> {
        let drafts = ChatDraftStore::shared(cx);
        drafts.read(cx).previews_by_scope().get(&ChatDraftStore::scope_key(chat_id, topic_id, None)).cloned()
    }

    fn hold_row(
        id: String,
        actions: Vec<HoldContextMenuAction>,
        content: HoldContextMenuSourceContent,
        on_action: HoldContextMenuStringHandler,
        on_tap: HoldContextMenuPointHandler,
    ) -> HoldContextMenuContainer {
        HoldContextMenuContainer::new(
            SharedString::from(id),
            HoldContextMenuAlignment::Trailing,
            Rc::new(move |_: &App| actions.clone()),
            content,
        )
        .show_emoji_list(false)
        .source_bubble_clip(Some(ForumRowLift::clip()))
        .lift_inset(ForumRowLift::INSET)
        .on_action(on_action)
        .on_tap(on_tap)
    }

    fn all_row(&self, chat: &ChatInfo, cx: &mut App) -> AnyElement {
        let draft = Self::topic_draft(&chat.id, None, cx);
        let presentation = ChatListPreviewFactory::synthetic_topic(
            &tr("chat_topics_all"),
            chat.last_message.as_ref(),
            chat.unread_count,
            chat.unread_mentions_count,
            chat.unread_replies_count,
            chat.unread_reactions_count,
            chat.is_muted,
            chat.updated_at,
            draft.as_ref(),
            cx,
        );
        let (action_parts, tap_parts) = (self.clone(), self.clone());
        let row_id = format!("forum-all-{}", self.chat_id);
        Self::hold_row(
            format!("{row_id}-hold"),
            Self::chat_notification_actions(),
            Rc::new(move |_: &mut Window, _: &mut App| {
                ForumTopicPreviewRow::new(row_id.clone(), "tray.full.fill", true, presentation.clone()).into_any_element()
            }),
            Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                if id == "notifications" {
                    action_parts.present_notifications_hub(cx);
                } else if id == "wallpaper" {
                    action_parts.update_state(cx, |state| state.show_chat_wallpaper = true);
                }
            }),
            Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| (tap_parts.on_open_all)(window, cx)),
        )
        .with_row_divider(&rise_ui::theme(cx).clone())
    }

    fn general_row(&self, snapshot: &ForumTopicListSnapshot, cx: &mut App) -> AnyElement {
        let topic = snapshot.general_topic.clone();
        let inherited_muted = topic.as_ref().map_or(snapshot.chat.is_muted, |topic| snapshot.is_effectively_muted(topic));
        let general_draft = Self::topic_draft(&snapshot.chat.id, Some(""), cx);
        let mut presentation = if let Some(topic) = &topic {
            let unread_count = ChatServicesStore::shared(cx).read(cx).displayed_unread_count(&snapshot.chat.id, topic);
            ChatListPreviewFactory::topic(
                topic,
                topic.last_message.as_ref(),
                unread_count,
                topic.is_muted || inherited_muted,
                topic.updated_at,
                general_draft.as_ref(),
                cx,
            )
        } else {
            let general_fallback = snapshot
                .chat
                .last_message
                .as_ref()
                .filter(|message| message.topic_id.as_deref().is_some_and(str::is_empty));
            ChatListPreviewFactory::synthetic_topic(
                &tr("chat_topics_general"),
                general_fallback,
                0,
                0,
                0,
                0,
                inherited_muted,
                snapshot.chat.updated_at,
                general_draft.as_ref(),
                cx,
            )
        };
        presentation.title = tr("chat_topics_general");

        let actions = match &topic {
            Some(topic) => self.topic_actions(topic, snapshot, cx),
            None => Self::chat_notification_actions(),
        };
        let (action_parts, tap_parts) = (self.clone(), self.clone());
        let row_id = format!("forum-general-{}", self.chat_id);
        let typing_chat_id = self.chat_id.clone();
        Self::hold_row(
            format!("{row_id}-hold"),
            actions,
            Rc::new(move |_: &mut Window, _: &mut App| {
                ForumTopicPreviewRow::new(row_id.clone(), "number", true, presentation.clone())
                    .typing(Some(typing_chat_id.clone()), Some(String::new()))
                    .into_any_element()
            }),
            Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                if let Some(topic) = &topic {
                    action_parts.handle_topic_action(&id, topic, cx);
                } else if id == "notifications" {
                    action_parts.present_notifications_hub(cx);
                } else if id == "wallpaper" {
                    action_parts.update_state(cx, |state| state.show_general_wallpaper = true);
                }
            }),
            Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.topic_opened(&tap_parts.chat_id, Some(""), cx);
                });
                (tap_parts.on_open_topic)(None, window, cx);
            }),
        )
        .with_row_divider(&rise_ui::theme(cx).clone())
    }

    fn topic_row(
        &self,
        topic: &ChatTopic,
        snapshot: &ForumTopicListSnapshot,
        first_voice_channel_id: Option<&str>,
        first_text_topic_id: Option<&str>,
        cx: &mut App,
    ) -> AnyElement {
        if topic.is_rtc_channel() {
            return self
                .voice_channel_row(topic, snapshot, first_voice_channel_id == Some(topic.id.as_str()), cx)
                .into_any_element();
        }
        let unread_count = ChatServicesStore::shared(cx).read(cx).displayed_unread_count(&snapshot.chat.id, topic);
        let draft = Self::topic_draft(&snapshot.chat.id, Some(&topic.id), cx);
        let presentation = ChatListPreviewFactory::topic(
            topic,
            topic.last_message.as_ref(),
            unread_count,
            snapshot.is_effectively_muted(topic),
            topic.updated_at,
            draft.as_ref(),
            cx,
        );
        let actions = self.topic_actions(topic, snapshot, cx);
        let icon = if topic.icon.is_empty() { ChatTopic::DEFAULT_ICON.to_owned() } else { topic.icon.clone() };
        let anchor =
            (first_text_topic_id == Some(topic.id.as_str())).then_some(RiseGuideAnchor::ForumFirstTextTopic);
        let (action_parts, action_topic) = (self.clone(), topic.clone());
        let (tap_parts, tap_topic) = (self.clone(), topic.clone());
        let row_id = format!("forum-topic-{}", topic.id);
        let (typing_chat_id, typing_topic_id) = (snapshot.chat.id.clone(), topic.id.clone());
        let controller_preview = self.controller_preview(&topic.id);
        Self::hold_row(
            format!("{row_id}-hold"),
            actions,
            Rc::new(move |_: &mut Window, _: &mut App| {
                ForumTopicPreviewRow::new(row_id.clone(), icon.clone(), false, presentation.clone())
                    .typing(Some(typing_chat_id.clone()), Some(typing_topic_id.clone()))
                    .guide_anchor(anchor)
                    .into_any_element()
            }),
            Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                action_parts.handle_topic_action(&id, &action_topic, cx);
            }),
            Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.topic_opened(&tap_parts.chat_id, Some(&tap_topic.id), cx);
                });
                (tap_parts.on_open_topic)(Some(tap_topic.clone()), window, cx);
            }),
        )
        .controller_preview(controller_preview)
        .with_row_divider(&rise_ui::theme(cx).clone())
    }

    /// the held topic shows its own chat page as the lifted card
    fn controller_preview(&self, topic_id: &str) -> Option<HoldContextMenuControllerPreviewFactory> {
        let navigation = self.nav_store.clone()?;
        let (chat_id, topic_id) = (self.chat_id.clone(), topic_id.to_owned());
        let factory: HoldContextMenuControllerPreviewFactory =
            Rc::new(move |size: Size<Pixels>, _: &mut Window, cx: &mut App| {
                ChatContextPreview::make_with_chat_id(&chat_id, Some(topic_id.clone()), size, navigation.clone(), cx)
            });
        Some(factory)
    }

    fn voice_channel_row(
        &self,
        topic: &ChatTopic,
        snapshot: &ForumTopicListSnapshot,
        carries_anchor: bool,
        cx: &mut App,
    ) -> Div {
        let actions = self.topic_actions(topic, snapshot, cx);
        let is_muted = snapshot.is_effectively_muted(topic);
        let anchor = carries_anchor.then_some(RiseGuideAnchor::ForumFirstVoiceChannel);
        let header_topic = topic.clone();
        let (action_parts, action_topic) = (self.clone(), topic.clone());
        let (tap_parts, tap_topic) = (self.clone(), topic.clone());
        let header = Self::hold_row(
            format!("forum-voice-{}-hold", topic.id),
            actions,
            Rc::new(move |_: &mut Window, _: &mut App| {
                ForumVoiceChannelHeaderRow::new(header_topic.clone(), is_muted).guide_anchor(anchor).into_any_element()
            }),
            Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                action_parts.handle_topic_action(&id, &action_topic, cx);
            }),
            Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
                (tap_parts.on_open_topic)(Some(tap_topic.clone()), window, cx);
            }),
        );

        let theme = rise_ui::theme(cx).clone();
        let mut block = vstack(0.0).w_full().child(header.with_row_divider(&theme));
        if let Some(state) = &topic.voice_state {
            let is_stage = topic.channel_kind == ChatChannelKind::Stage;
            let hand_expiry_revision = RiseForumVoicePresentation::shared(cx).read(cx).hand_expiry_revision();
            let participants =
                state.display_participants(is_stage, RiseForumVoicePresenceSnapshot::now_ms_for_display());
            for participant in participants {
                let nav_store = self.nav_store.clone();
                let (user_id, tag) = (participant.user_id.clone(), participant.tag.clone());
                let row_id = SharedString::from(format!("forum-voice-{}-participant-{user_id}", topic.id));
                block = block.child(
                    ForumVoiceParticipantRow::new(row_id, participant, move |_: &mut Window, cx: &mut App| {
                        if let Some(nav_store) = &nav_store {
                            let route =
                                RootRoute::UserPage { user_id: Some(user_id.clone()), tag: Some(tag.clone()) };
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    })
                    .is_stage(is_stage)
                    .hand_expiry_revision(hand_expiry_revision),
                );
            }
            for index in 0..state.missing_preview_count() {
                block = block.child(div().w_full().bg(theme.bg._100).child(ForumVoiceParticipantPlaceholderRow::new(index)));
            }
            if state.hidden_participant_count() > 0 {
                let (chat_id, topic_id) = (self.chat_id.clone(), topic.id.clone());
                block = block.child(ForumVoiceOverflowRow::new(
                    SharedString::from(format!("forum-voice-{}-overflow", topic.id)),
                    tr_plural("forum_voice_more_participants", state.hidden_participant_count()),
                    move |_: &mut Window, cx: &mut App| {
                        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.expand_voice_channel_preview(&chat_id, &topic_id, cx);
                        });
                    },
                ));
            }
            if state.is_expanded() {
                let (chat_id, topic_id) = (self.chat_id.clone(), topic.id.clone());
                block = block.child(ForumVoiceOverflowRow::new(
                    SharedString::from(format!("forum-voice-{}-collapse", topic.id)),
                    tr("common_show_less"),
                    move |_: &mut Window, cx: &mut App| {
                        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.collapse_voice_channel_preview(&chat_id, &topic_id, cx);
                        });
                    },
                ));
            }
        }
        block
    }

    fn row(&self, row: &ForumTopicListRow, context: &ForumTopicListRowContext, cx: &mut App) -> AnyElement {
        let snapshot = context.snapshot.as_ref();
        let theme = &context.theme;
        match row {
            ForumTopicListRow::TopInset => div().w_full().h(px(context.top_inset + 10.0)).into_any_element(),
            ForumTopicListRow::SectionHeader(section) => {
                self.section_header(section.as_ref(), snapshot, cx).into_any_element()
            }
            ForumTopicListRow::All => self.all_row(&snapshot.chat, cx),
            ForumTopicListRow::General => self.general_row(snapshot, cx),
            ForumTopicListRow::Topic(topic) => self.topic_row(
                topic,
                snapshot,
                context.first_voice_channel_id.as_deref(),
                context.first_text_topic_id.as_deref(),
                cx,
            ),
            ForumTopicListRow::SectionEmpty(_) => Self::section_empty_row(theme).into_any_element(),
            ForumTopicListRow::CreateTopic(None) => {
                self.create_topic_row(None, theme).guide_anchor(RiseGuideAnchor::ForumAddTopic).into_any_element()
            }
            ForumTopicListRow::CreateTopic(Some(section_id)) => {
                self.create_topic_row(Some(section_id.as_str()), theme).into_any_element()
            }
            ForumTopicListRow::SectionFooter(_) => div().w_full().h(px(20.0)).into_any_element(),
            ForumTopicListRow::CreateSection => div()
                .w_full()
                .pb(px(20.0))
                .child(div().w_full().bg(theme.bg._100).child(self.create_section_row(theme)).guide_anchor(RiseGuideAnchor::ForumCreateSection))
                .into_any_element(),
            ForumTopicListRow::Cold => ForumTopicColdPresentationView {
                id: context.cold_view_id.clone(),
                phase: context.cold_phase,
                on_retry: context.on_cold_retry.clone(),
            }
            .into_any_element(),
            ForumTopicListRow::BottomInset => div().w_full().h(px(context.bottom_inset)).into_any_element(),
        }
    }

    fn section_header(
        &self,
        section: Option<&ChatSection>,
        snapshot: &ForumTopicListSnapshot,
        cx: &mut App,
    ) -> ForumPreviewSectionHeader {
        let Some(section) = section else {
            return ForumPreviewSectionHeader {
                id: format!("forum-section-root-{}", self.chat_id),
                title: None,
                title_emojis: Vec::new(),
                section_id: None,
                is_collapsed: false,
                header_actions: Vec::new(),
                on_header_action: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
            };
        };
        let (action_parts, action_section) = (self.clone(), section.clone());
        let is_collapsed = ForumSectionCollapsePreferences::shared(cx).read(cx).is_collapsed(&section.id);
        ForumPreviewSectionHeader {
            id: format!("forum-section-{}", section.id),
            title: Some(section.name.clone()),
            title_emojis: section.name_custom_emojis(),
            section_id: Some(section.id.clone()),
            is_collapsed,
            header_actions: Self::section_actions(snapshot),
            on_header_action: Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                action_parts.handle_section_action(&id, &action_section, cx);
            }),
        }
    }

    fn section_empty_row(theme: &AppTheme) -> Div {
        div()
            .relative()
            .w_full()
            .px(px(14.0))
            .py(px(18.0))
            .child(MainText::new(tr("forum_section_empty")).px(13.0).color(theme.text.secondary).number_of_lines(2))
            .child(bottom_hairline(theme))
    }

    fn topic_actions(
        &self,
        topic: &ChatTopic,
        snapshot: &ForumTopicListSnapshot,
        cx: &mut App,
    ) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if !topic.is_general && !topic.id.is_empty() && ChatProfilePermissions::shares_place_links(&snapshot.chat) {
            actions.push(HoldContextMenuAction::new("invite", tr("voice_invite_tile")).system_image("person.badge.plus"));
        }
        if !topic.is_rtc_channel() {
            let displayed_unread = ChatServicesStore::shared(cx).read(cx).displayed_unread_count(&self.chat_id, topic);
            let has_unread = displayed_unread > 0
                || topic.unread_mentions_count > 0
                || topic.unread_replies_count > 0
                || topic.unread_reactions_count > 0;
            actions.push(if has_unread {
                HoldContextMenuAction::new("mark_read", tr("ctx_mark_read")).system_image("checkmark.circle")
            } else {
                HoldContextMenuAction::new("mark_unread", tr("ctx_mark_unread")).system_image("envelope.badge")
            });
        }
        actions.push(HoldContextMenuAction::new("notifications", tr("chat_mute_advanced_title")).system_image("bell"));
        actions.push(
            HoldContextMenuAction::new("wallpaper", tr("wallpaper_topic_title"))
                .system_image("photo.on.rectangle.angled"),
        );
        let can_manage_channel = self.can_manage_channel(topic, cx);
        if can_manage_channel && !topic.is_general {
            actions.push(HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"));
            actions.push(if topic.is_closed {
                HoldContextMenuAction::new("open", tr("chat_topics_open")).system_image("lock.open")
            } else {
                HoldContextMenuAction::new("close", tr("chat_topics_close")).system_image("lock")
            });
        }
        if snapshot.can_manage_roles() && !topic.is_general && !topic.id.is_empty() {
            actions.push(
                HoldContextMenuAction::new("visibility", tr("forum_visibility_action"))
                    .system_image("lock.shield")
                    .guide_anchor(RiseGuideAnchor::ForumMenuVisibility),
            );
        }
        if can_manage_channel && !topic.is_general {
            actions.push(
                HoldContextMenuAction::new("delete", tr("chat_topics_delete")).system_image("trash").destructive(true),
            );
        }
        actions
    }

    fn chat_notification_actions() -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new("notifications", tr("chat_mute_advanced_title")).system_image("bell"),
            HoldContextMenuAction::new("wallpaper", tr("wallpaper_chat_sheet_title"))
                .system_image("photo.on.rectangle.angled"),
        ]
    }

    fn section_actions(snapshot: &ForumTopicListSnapshot) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if snapshot.can_manage_roles() {
            actions.push(HoldContextMenuAction::new("visibility", tr("forum_visibility_action")).system_image("lock.shield"));
        }
        if snapshot.can_manage_sections() {
            actions.push(HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"));
            actions.push(
                HoldContextMenuAction::new("delete", tr("forum_section_delete")).system_image("trash").destructive(true),
            );
        }
        actions
    }

    fn handle_topic_action(&self, id: &str, topic: &ChatTopic, cx: &mut App) {
        match id {
            "mark_read" => {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.mark_topic_read(&self.chat_id, topic, cx);
                });
            }
            "mark_unread" => {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.mark_topic_unread(&self.chat_id, topic, cx);
                });
            }
            "edit" => {
                if !self.can_manage_channel(topic, cx) {
                    return;
                }
                self.update_state(cx, |state| {
                    state.editor_topic_id = Some(topic.id.clone());
                    state.pending_section_id =
                        if topic.section_id.is_empty() { None } else { Some(topic.section_id.clone()) };
                    state.show_topic_editor = true;
                    state.has_mounted_topic_editor = true;
                });
            }
            "invite" => {
                let (parts, topic) = (self.clone(), topic.clone());
                HoldContextMenuPresenter::after_dismiss(
                    move |cx| parts.update_state(cx, |state| state.voice_invite_topic = Some(topic)),
                    cx,
                );
            }
            "notifications" => self.update_state(cx, |state| state.notification_topic.present(topic.clone())),
            "wallpaper" => self.update_state(cx, |state| state.wallpaper_topic.present(topic.clone())),
            "visibility" => {
                let target = VisibilityTarget {
                    kind: VisibilityTargetKind::Topic,
                    id: topic.id.clone(),
                    title: topic.title.clone(),
                };
                self.update_state(cx, |state| state.visibility_target.present(target));
            }
            "close" | "open" => {
                if !self.can_manage_channel(topic, cx) {
                    return;
                }
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.set_topic_closed(&self.chat_id, &topic.id, id == "close", cx);
                });
            }
            "delete" => {
                if !self.can_manage_channel(topic, cx) {
                    return;
                }
                let (chat_id, topic_id) = (self.chat_id.clone(), topic.id.clone());
                NativeConfirmAlert::present_delete(
                    &tr("topic_settings_delete_confirm"),
                    &topic.title,
                    Some(tr("chat_topics_delete")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.delete_topic(&chat_id, &topic_id, None, cx);
                        });
                    },
                    cx,
                );
            }
            _ => {}
        }
    }

    fn handle_section_action(&self, id: &str, section: &ChatSection, cx: &mut App) {
        match id {
            "visibility" => {
                let target = VisibilityTarget {
                    kind: VisibilityTargetKind::Section,
                    id: section.id.clone(),
                    title: section.name.clone(),
                };
                self.update_state(cx, |state| state.visibility_target.present(target));
            }
            "edit" => {
                let target = ForumSectionNameTarget {
                    chat_id: self.chat_id.clone(),
                    section_id: Some(section.id.clone()),
                    name: section.name.clone(),
                    name_entities: section.name_entities.clone(),
                };
                self.update_state(cx, |state| state.section_name_target = Some(target));
            }
            "delete" => {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.delete_section(&self.chat_id, &section.id, cx);
                });
            }
            _ => {}
        }
    }

    fn create_topic_row(&self, section_id: Option<&str>, theme: &AppTheme) -> PlainButton {
        let parts = self.clone();
        let target = section_id.map(str::to_owned);
        plain(SharedString::from(format!(
            "forum-add-topic-{}-{}",
            self.chat_id,
            section_id.unwrap_or_default()
        )))
        .w_full()
        .on_press(move |_, cx| {
            let target = target.clone();
            parts.update_state(cx, |state| {
                state.editor_topic_id = None;
                state.pending_section_id = target;
                state.show_topic_editor = true;
                state.has_mounted_topic_editor = true;
            });
        })
        .child(
            hstack(12.0)
                .relative()
                .w_full()
                .px(px(12.0))
                .py(px(7.0))
                .bg(theme.bg._100)
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(34.0))
                        .children(IconUi::sized("plus", px(15.0), theme.primary._100, false)),
                )
                .child(
                    MainText::new(tr("forum_add_topic"))
                        .px(14.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.primary._100),
                )
                .child(spacer_min_w(0.0))
                .child(bottom_hairline(theme)),
        )
    }

    fn create_section_row(&self, theme: &AppTheme) -> PlainButton {
        let parts = self.clone();
        plain(SharedString::from(format!("forum-create-section-{}", self.chat_id)))
            .w_full()
            .on_press(move |_, cx| {
                let target = ForumSectionNameTarget {
                    chat_id: parts.chat_id.clone(),
                    section_id: None,
                    name: String::new(),
                    name_entities: None,
                };
                parts.update_state(cx, |state| state.section_name_target = Some(target));
            })
            .child(
                hstack(10.0)
                    .relative()
                    .w_full()
                    .px(px(16.0))
                    .min_h(px(46.0))
                    .children(IconUi::sized("folder.badge.plus", px(15.0), theme.primary._100, false))
                    .child(
                        MainText::new(tr("forum_create_section"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    )
                    .child(div().absolute().top_0().left_0().right_0().h(px(0.5)).bg(theme.border._100))
                    .child(bottom_hairline(theme)),
            )
    }
}

const FOCUS_FALLBACK_ROW_HEIGHT: f32 = 56.0;

fn centered_offset(list: &ListState, index: usize) -> ListOffset {
    let viewport = f32::from(list.viewport_bounds().size.height);
    let row = list
        .bounds_for_item(index)
        .map(|bounds| f32::from(bounds.size.height))
        .filter(|height| *height > 0.0)
        .unwrap_or(FOCUS_FALLBACK_ROW_HEIGHT);
    let above = ((viewport - row) / 2.0).max(0.0);
    let rows_above = (above / row).floor() as usize;
    let rest = above - rows_above as f32 * row;
    if index <= rows_above {
        return ListOffset { item_ix: 0, offset_in_item: px(0.0) };
    }
    let top = index - rows_above;
    if rest <= 0.5 {
        ListOffset { item_ix: top, offset_in_item: px(0.0) }
    } else {
        ListOffset { item_ix: top - 1, offset_in_item: px(row - rest) }
    }
}

fn bottom_hairline(theme: &AppTheme) -> Div {
    div().absolute().bottom_0().left_0().right_0().h(px(0.5)).bg(theme.border._100)
}

/// a held row rises as a rounded card of its content, the whole row still takes the press
enum ForumRowLift {}

impl ForumRowLift {
    const INSET: f32 = 8.0;

    fn clip() -> HoldSourceBubbleClip {
        HoldSourceBubbleClip::new(px(16.0), px(16.0), px(16.0), px(16.0))
    }
}

trait ForumRowDivider: IntoElement {
    fn with_row_divider(self, theme: &AppTheme) -> AnyElement {
        div().relative().w_full().child(self).child(bottom_hairline(theme)).into_any_element()
    }
}

impl ForumRowDivider for HoldContextMenuContainer {}

impl RenderOnce for ForumTopicListContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let chat_id = self.chat_id.clone();
        let epoch = self.presentation_epoch;
        let view_id = format!("forum-topic-list-{chat_id}-{epoch}");
        let transition_for_state = self.presentation_transition.clone();
        let state: Entity<ForumTopicListContentState> =
            window.use_keyed_state(SharedString::from(format!("{view_id}-state")), cx, move |_, cx| {
                ForumTopicListContentState::new(transition_for_state, cx)
            });

        let transition = self.presentation_transition.clone();
        let structural_updates_blocked =
            transition.as_ref().is_some_and(|transition| transition.read(cx).structural_updates_blocked());
        let snapshot = self.resolved_presentation_snapshot(&state, structural_updates_blocked, cx);
        let has_cached_structure = snapshot.has_cached_structure;
        let presentation_scope = format!("{chat_id}\u{1F}{epoch}");

        let (did_time_out, timeout_attempt_scope, timeout_attempt_generation, accepts_live) = {
            let state = state.read(cx);
            (
                state.cold_load_timed_out_scope.as_deref() == Some(presentation_scope.as_str()),
                state.cold_load_attempt_scope.clone(),
                state.cold_load_attempt_generation,
                state.accepts_live_presentation_snapshot,
            )
        };
        let cold_phase = ForumHomeColdPresentationPolicy::phase(has_cached_structure, did_time_out);
        let first_voice_channel_id = Self::first_voice_channel_id(&snapshot);
        let first_text_topic_id = Self::first_text_topic_id(&snapshot);
        let voice_signature = Self::voice_channel_signature(&snapshot);
        let has_active_hold_menu = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();

        let parts = ForumTopicListParts {
            chat_id: chat_id.clone(),
            state: state.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
            on_open_all: self.on_open_all.clone(),
            on_open_topic: self.on_open_topic.clone(),
        };

        let all_topics_view_hidden = cold_phase == ForumHomeColdPresentationPhase::Content
            && ChatServicesStore::shared(cx).update(cx, |services, cx| services.all_topics_view_hidden(&chat_id, cx));
        let collapsed_section_ids: Vec<String> = {
            let collapse = ForumSectionCollapsePreferences::shared(cx);
            let collapse = collapse.read(cx);
            snapshot.sections.iter().filter(|section| collapse.is_collapsed(&section.id)).map(|section| section.id.clone()).collect()
        };
        let rows = ForumTopicListRow::rows(cold_phase, all_topics_view_hidden, &snapshot, &collapsed_section_ids);
        let row_keys: Vec<String> = rows.iter().map(ForumTopicListRow::key).collect();
        state.update(cx, |state, _| state.apply_row_keys(row_keys));

        let (retry_parts, retry_scope) = (parts.clone(), presentation_scope.clone());
        let row_parts = parts.clone();
        let row_context = ForumTopicListRowContext {
            snapshot: Rc::new(snapshot.clone()),
            first_voice_channel_id,
            first_text_topic_id,
            theme: theme.clone(),
            top_inset: self.top_inset,
            bottom_inset: self.bottom_inset,
            cold_phase,
            cold_view_id: format!("{view_id}-cold"),
            on_cold_retry: Rc::new(move |_: &mut Window, cx: &mut App| {
                retry_parts.begin_cold_load_attempt(&retry_scope, cx);
            }),
        };
        state.update(cx, |state, _| state.focus_scroll.frame(state.list.state(), window));
        let mut list = state
            .read(cx)
            .list
            .hosted(SharedString::from(format!("{view_id}-scroll")), move |index, _, cx| match rows.get(index) {
                Some(row) => row_parts.row(row, &row_context, cx),
                None => div().into_any_element(),
            })
            .scroll_disabled(has_active_hold_menu || self.interaction_scroll_locked);
        if self.installs_horizontal_swipe_gate {
            list = list.background(
                div().absolute().size_full().child(EmbeddedForumHorizontalScrollGate::new(!has_active_hold_menu)),
            );
        }

        let refresh_state = state.clone();
        let refresh_transition = transition.clone();
        let refresh_chat_id = chat_id.clone();
        let refresh_scope = presentation_scope.clone();
        use_task(
            SharedString::from(format!("{view_id}-refresh")),
            presentation_scope.clone(),
            window,
            cx,
            async move |cx| {
                task_yield(cx).await;
                let delay = ForumHomeInitialRefreshPolicy::delay_nanoseconds(has_cached_structure);
                if delay > 0 {
                    cx.background_executor().timer(Duration::from_nanos(delay)).await;
                }

                if let Some(transition) = &refresh_transition {
                    if !wait_for_settled_presentation(transition, cx).await {
                        return;
                    }
                }

                let accepted = cx.update(|_, cx| {
                    if let Some(transition) = &refresh_transition {
                        transition.update(cx, |transition, cx| transition.release_structural_updates_if_idle(cx));
                    }
                    refresh_state.update(cx, |state, cx| {
                        state.accepts_live_presentation_snapshot = true;
                        cx.notify();
                    });
                });
                if accepted.is_err() {
                    return;
                }
                task_yield(cx).await;
                if let Some(transition) = &refresh_transition {
                    if !wait_for_settled_presentation(transition, cx).await {
                        return;
                    }
                }
                let started_cold = !has_cached_structure;
                let _ = cx.update(|_, cx| {
                    if started_cold {
                        refresh_state.update(cx, |state, cx| {
                            state.register_cold_load_attempt(&refresh_scope);
                            cx.notify();
                        });
                    }
                    ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.load_forum_home(&refresh_chat_id, cx);
                    });
                });
            },
        );

        let voice_observation = state.read(cx).voice_observation.clone();
        let voice_task_observation = voice_observation.clone();
        let voice_transition = transition.clone();
        let voice_chat_id = chat_id.clone();
        let voice_task_signature = voice_signature.clone();
        use_task(
            SharedString::from(format!("{view_id}-voice")),
            (chat_id.clone(), accepts_live, voice_signature),
            window,
            cx,
            async move |cx| {
                let _ = cx.update(|_, cx| {
                    let is_presented =
                        voice_transition.as_ref().is_none_or(|transition| transition.read(cx).is_presented());
                    let cx = RefCell::new(cx);
                    voice_task_observation.borrow_mut().update(
                        &voice_chat_id,
                        &voice_task_signature,
                        accepts_live && is_presented,
                        |chat_id| begin_voice_observation(chat_id, &mut cx.borrow_mut()),
                        |chat_id| end_voice_observation(chat_id, &mut cx.borrow_mut()),
                    );
                });
            },
        );

        let timeout_state = state.clone();
        let timeout_transition = transition.clone();
        let timeout_chat_id = chat_id.clone();
        let timeout_scope = presentation_scope.clone();
        let attempt_scope = timeout_attempt_scope.clone();
        use_task(
            SharedString::from(format!("{view_id}-cold-timeout")),
            (presentation_scope.clone(), timeout_attempt_scope, timeout_attempt_generation),
            window,
            cx,
            async move |cx| {
                if timeout_attempt_generation == 0 || attempt_scope.as_deref() != Some(timeout_scope.as_str()) {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_nanos(ForumHomeColdPresentationPolicy::TIMEOUT_NANOSECONDS))
                    .await;
                let attempt_is_current = |cx: &mut AsyncWindowContext| {
                    timeout_state.read_with(cx, |state, _| {
                        state.cold_load_attempt_scope.as_deref() == Some(timeout_scope.as_str())
                            && state.cold_load_attempt_generation == timeout_attempt_generation
                    })
                };
                if !attempt_is_current(cx) {
                    return;
                }
                if let Some(transition) = &timeout_transition {
                    if !wait_for_settled_presentation_while(transition, cx, attempt_is_current).await {
                        return;
                    }
                }
                let _ = cx.update(|_, cx| {
                    let latest = ChatServicesStore::shared(cx)
                        .update(cx, |services, cx| services.forum_topic_list_snapshot(&timeout_chat_id, cx));
                    if latest.has_cached_structure {
                        return;
                    }
                    timeout_state.update(cx, |state, cx| {
                        state.cold_load_timed_out_scope = Some(timeout_scope.clone());
                        cx.notify();
                    });
                });
            },
        );

        let connected_parts = parts.clone();
        let connected_transition = transition.clone();
        let connected_scope = presentation_scope;
        let is_connected = RiseTransportStatus::is_connected(cx);
        on_change(
            SharedString::from(format!("{view_id}-connected")),
            is_connected,
            false,
            window,
            cx,
            move |_, connected, _, cx| {
                if !*connected || !connected_parts.state.read(cx).accepts_live_presentation_snapshot {
                    return;
                }
                if let Some(transition) = &connected_transition {
                    let transition = transition.read(cx);
                    if !transition.is_presented() || transition.is_interactive_gesture_active() {
                        return;
                    }
                }
                let has_cached = ChatServicesStore::shared(cx)
                    .update(cx, |services, cx| services.forum_topic_list_snapshot(&connected_parts.chat_id, cx))
                    .has_cached_structure;
                if !has_cached {
                    connected_parts.begin_cold_load_attempt(&connected_scope, cx);
                }
            },
        );

        let pictures = Self::picture_wallpapers(&snapshot);
        let picture_chat_id = chat_id.clone();
        use_task(
            SharedString::from(format!("{view_id}-wallpaper-prewarm")),
            pictures.clone(),
            window,
            cx,
            async move |cx| {
                ChatWallpaperPrewarm::pictures(picture_chat_id, pictures, cx).await;
            },
        );

        let focus_pending = ForumTopicFocusPresenter::shared(cx).read(cx).pending().cloned();
        let focus_parts = parts.clone();
        on_change(
            SharedString::from(format!("{view_id}-topic-focus")),
            focus_pending,
            true,
            window,
            cx,
            move |_, _, window, cx| {
                let presenter = ForumTopicFocusPresenter::shared(cx);
                let Some(request) = presenter.update(cx, |presenter, _| presenter.request(&focus_parts.chat_id)) else {
                    return;
                };
                if focus_parts.state.read(cx).focus_request.as_ref() == Some(&request) {
                    return;
                }
                focus_parts.state.update(cx, |state, _| state.focus_request = Some(request.clone()));
                window.spawn(cx, async move |cx: &mut AsyncWindowContext| focus_parts.focus_topic(request, cx).await).detach();
            },
        );

        let sheets = ForumTopicListSheets { view_id: view_id.clone(), chat_id: chat_id.clone(), state: state.clone(), chat: snapshot.chat.clone() };

        let (show_topic_editor, has_mounted_topic_editor, editor_topic_id, pending_section_id) = {
            let state = state.read(cx);
            (
                state.show_topic_editor,
                state.has_mounted_topic_editor,
                state.editor_topic_id.clone(),
                state.pending_section_id.clone(),
            )
        };
        let topic_editor = (show_topic_editor || has_mounted_topic_editor).then(|| {
            TopicEditorSheet::new(
                Binding::entity(
                    state.clone(),
                    |state: &ForumTopicListContentState| state.show_topic_editor,
                    |state, value, cx| {
                        state.show_topic_editor = value;
                        cx.notify();
                    },
                ),
                chat_id.clone(),
            )
            .topic_id(editor_topic_id)
            .section_id(pending_section_id)
        });

        div()
            .size_full()
            .bg(theme.bg._100)
            .child(list)
            .child(sheets)
            .children(topic_editor)
            .on_disappear(SharedString::from(format!("{view_id}-disappear")), move |_, cx| {
                ChatPerformanceTrace::shared().record("forum.topic_list_disappeared", HashMap::new());
                voice_observation.borrow_mut().stop(|chat_id| end_voice_observation(chat_id, cx));
            })
    }
}

#[derive(IntoElement)]
struct ForumTopicListSheets {
    view_id: String,
    chat_id: String,
    state: Entity<ForumTopicListContentState>,
    chat: ChatInfo,
}

impl RenderOnce for ForumTopicListSheets {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { view_id, chat_id, state, chat } = self;

        let voice_invite = state.read(cx).voice_invite_topic.clone().map(|topic| {
            let text = !topic.is_rtc_channel();
            RiseRTCVoiceInviteBottomSheet::new(
                Binding::entity(
                    state.clone(),
                    |state: &ForumTopicListContentState| state.voice_invite_topic.is_some(),
                    |state, value, cx| {
                        if !value {
                            state.voice_invite_topic = None;
                            cx.notify();
                        }
                    },
                ),
                chat_id.clone(),
                Some(topic.id),
            )
            .text(text)
        });
        let section_name = ForumSectionNameSheetHost::new(
            SharedString::from(format!("{view_id}-section-name")),
            Binding::entity(
                state.clone(),
                |state: &ForumTopicListContentState| state.section_name_target.clone(),
                |state, value, cx| {
                    state.section_name_target = value;
                    cx.notify();
                },
            ),
        );

        let visibility_state = state.clone();
        let visibility_chat_id = chat_id.clone();
        let visibility = NativeBottomSheetPresentationHost::new(
            SharedString::from(format!("{view_id}-visibility")),
            Binding::entity(
                state.clone(),
                |state: &ForumTopicListContentState| state.visibility_target.item.is_some(),
                |state, value, cx| {
                    if !value {
                        state.visibility_target.item = None;
                        cx.notify();
                    }
                },
            ),
            move |_, cx| match visibility_state.read(cx).visibility_target.retained.clone() {
                Some(target) => ChannelVisibilityEditor::new(visibility_chat_id.clone(), target).into_any_element(),
                None => div().into_any_element(),
            },
        )
        .presentation_detents([PresentationDetent::Large]);

        let notification_state = state.clone();
        let notification_chat_id = chat_id.clone();
        let notification = NativeBottomSheetPresentationHost::new(
            SharedString::from(format!("{view_id}-notification")),
            Binding::entity(
                state.clone(),
                |state: &ForumTopicListContentState| state.notification_topic.item.is_some(),
                |state, value, cx| {
                    if !value {
                        state.notification_topic.item = None;
                        cx.notify();
                    }
                },
            ),
            move |_, cx| match notification_state.read(cx).notification_topic.retained.clone() {
                Some(topic) => TopicNotificationSheet::new(notification_chat_id.clone(), topic.id).into_any_element(),
                None => div().into_any_element(),
            },
        )
        .presentation_drag_indicator(true);

        let (chat_wallpaper_id, chat_wallpaper_chat) = (chat_id.clone(), chat.clone());
        let chat_wallpaper = NativeBottomSheetPresentationHost::new(
            SharedString::from(format!("{view_id}-chat-wallpaper")),
            Binding::entity(
                state.clone(),
                |state: &ForumTopicListContentState| state.show_chat_wallpaper,
                |state, value, cx| {
                    state.show_chat_wallpaper = value;
                    cx.notify();
                },
            ),
            move |_, _| ChatWallpaperSheet::new(chat_wallpaper_id.clone(), chat_wallpaper_chat.clone()),
        )
        .presentation_detents([])
        .presentation_drag_indicator(true);

        let (general_wallpaper_id, general_wallpaper_chat) = (chat_id.clone(), chat.clone());
        let general_wallpaper = NativeBottomSheetPresentationHost::new(
            SharedString::from(format!("{view_id}-general-wallpaper")),
            Binding::entity(
                state.clone(),
                |state: &ForumTopicListContentState| state.show_general_wallpaper,
                |state, value, cx| {
                    state.show_general_wallpaper = value;
                    cx.notify();
                },
            ),
            move |_, _| {
                ChatWallpaperSheet::new(general_wallpaper_id.clone(), general_wallpaper_chat.clone())
                    .topic_id(Some(ChatTopic::GENERAL_WALLPAPER_SCOPE_ID.to_owned()))
            },
        )
        .presentation_detents([])
        .presentation_drag_indicator(true);

        let wallpaper_state = state.clone();
        let topic_wallpaper = NativeBottomSheetPresentationHost::new(
            SharedString::from(format!("{view_id}-topic-wallpaper")),
            Binding::entity(
                state,
                |state: &ForumTopicListContentState| state.wallpaper_topic.item.is_some(),
                |state, value, cx| {
                    if !value {
                        state.wallpaper_topic.item = None;
                        cx.notify();
                    }
                },
            ),
            move |_, cx| match wallpaper_state.read(cx).wallpaper_topic.retained.clone() {
                Some(topic) => ChatWallpaperSheet::new(chat_id.clone(), chat.clone())
                    .topic_id(Some(topic.wallpaper_scope_id().to_owned()))
                    .into_any_element(),
                None => div().into_any_element(),
            },
        )
        .presentation_detents([])
        .presentation_drag_indicator(true);

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(visibility)
            .child(notification)
            .child(chat_wallpaper)
            .child(general_wallpaper)
            .child(topic_wallpaper)
            .children(voice_invite)
            .child(section_name)
    }
}

#[derive(IntoElement)]
struct ForumTopicColdPresentationView {
    id: String,
    phase: ForumHomeColdPresentationPhase,
    on_retry: EmbeddedForumAction,
}

impl RenderOnce for ForumTopicColdPresentationView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let stable_height = ForumHomeColdPresentationPolicy::STATUS_ROW_HEIGHT
            + ForumHomeColdPresentationPolicy::PLACEHOLDER_ROW_COUNT as f32
                * ForumHomeColdPresentationPolicy::PLACEHOLDER_ROW_HEIGHT;

        let body = if self.phase == ForumHomeColdPresentationPhase::TimedOut {
            let on_retry = self.on_retry.clone();
            vstack(14.0)
                .w_full()
                .min_h(px(stable_height))
                .justify_center()
                .px(px(24.0))
                .children(IconUi::sized(
                    "exclamationmark.arrow.triangle.2.circlepath",
                    px(24.0),
                    theme.text.secondary,
                    false,
                ))
                .child(
                    MainText::new(tr("error_fetching_data"))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                )
                .child(
                    plain(SharedString::from(format!("{}-retry", self.id)))
                        .on_press(move |window, cx| on_retry(window, cx))
                        .child(
                            hstack(0.0)
                                .h(px(38.0))
                                .px(px(18.0))
                                .bg(theme.bg._300)
                                .rounded_full()
                                .child(
                                    MainText::new(tr("common_retry"))
                                        .px(14.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.text.primary),
                                ),
                        ),
                )
        } else {
            let placeholder_rows = (0..ForumHomeColdPresentationPolicy::PLACEHOLDER_ROW_COUNT).map(|index| {
                hstack(10.0)
                    .relative()
                    .w_full()
                    .px(px(12.0))
                    .h(px(ForumHomeColdPresentationPolicy::PLACEHOLDER_ROW_HEIGHT))
                    .child(SkeletonBlock::new(36.0, 36.0, 10.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(px(8.0))
                            .child(SkeletonLine::new(if index % 2 == 0 { 132.0 } else { 176.0 }, 10.0))
                            .child(SkeletonLine::new(if index % 3 == 0 { 190.0 } else { 228.0 }, 8.0)),
                    )
                    .child(spacer_min_w(0.0))
                    .child(bottom_hairline(&theme))
            });
            div()
                .flex()
                .flex_col()
                .w_full()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .w_full()
                        .px(px(14.0))
                        .h(px(ForumHomeColdPresentationPolicy::STATUS_ROW_HEIGHT))
                        .child(
                            MainText::new(tr("chat_inline_loading"))
                                .px(12.0)
                                .color(theme.text.secondary)
                                .number_of_lines(1),
                        ),
                )
                .children(placeholder_rows)
        };
        div().flex().flex_col().w_full().min_h(px(stable_height)).child(body)
    }
}

#[derive(IntoElement)]
struct ForumPreviewSectionHeader {
    id: String,
    title: Option<String>,
    title_emojis: Vec<MessageCustomEmojiEntity>,
    section_id: Option<String>,
    is_collapsed: bool,
    header_actions: Vec<HoldContextMenuAction>,
    on_header_action: HoldContextMenuStringHandler,
}

impl ForumPreviewSectionHeader {
    fn toggle_collapse(section_id: Option<&str>, cx: &mut App) {
        let Some(section_id) = section_id else {
            return;
        };
        let _ = selection_changed();
        ForumSectionCollapsePreferences::shared(cx).update(cx, |collapse, cx| collapse.toggle(section_id, cx));
    }

    fn section_label(
        title: &str,
        title_emojis: &[MessageCustomEmojiEntity],
        has_section: bool,
        is_collapsed: bool,
        theme: &AppTheme,
    ) -> Div {
        let chevron = has_section.then(|| {
            let glyph = if is_collapsed { "chevron.right" } else { "chevron.down" };
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.0))
                .children(IconUi::sized(glyph, px(12.0), theme.text.secondary, false))
        });
        hstack(8.0)
            .w_full()
            .min_h(px(44.0))
            .items_end()
            .px(px(14.0))
            .pb(px(4.0))
            .child(ForumSectionTitleText::new(title.to_owned(), title_emojis.to_vec(), theme.text.secondary))
            .child(spacer_min_w(0.0))
            .children(chevron)
    }
}

impl RenderOnce for ForumPreviewSectionHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (title_emojis, section_id, is_collapsed) = (self.title_emojis, self.section_id, self.is_collapsed);
        let has_section = section_id.is_some();
        let id = self.id;
        let label = self.title.filter(|title| !title.is_empty()).map(|title| {
            if self.header_actions.is_empty() {
                plain(SharedString::from(format!("{id}-toggle")))
                    .w_full()
                    .on_press(move |_, cx| Self::toggle_collapse(section_id.as_deref(), cx))
                    .child(Self::section_label(&title, &title_emojis, has_section, is_collapsed, &theme))
                    .into_any_element()
            } else {
                let actions = self.header_actions;
                let label_theme = theme.clone();
                HoldContextMenuContainer::new(
                    SharedString::from(format!("{id}-hold")),
                    HoldContextMenuAlignment::Leading,
                    Rc::new(move |_: &App| actions.clone()),
                    Rc::new(move |_: &mut Window, _: &mut App| {
                        Self::section_label(&title, &title_emojis, has_section, is_collapsed, &label_theme)
                            .into_any_element()
                    }),
                )
                .show_emoji_list(false)
                .on_action(self.on_header_action)
                .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                    Self::toggle_collapse(section_id.as_deref(), cx);
                }))
                .into_any_element()
            }
        });
        div()
            .flex()
            .flex_col()
            .w_full()
            .bg(theme.bg._100)
            .children(label)
            .child(div().w_full().h(px(0.5)).bg(theme.border._100))
    }
}

#[derive(IntoElement)]
struct ForumTopicPreviewRow {
    id: String,
    icon: SharedString,
    icon_is_system: bool,
    typing_chat_id: Option<String>,
    typing_topic_id: Option<String>,
    presentation: ChatListPreviewPresentation,
}

impl ForumTopicPreviewRow {
    fn new(
        id: String,
        icon: impl Into<SharedString>,
        icon_is_system: bool,
        presentation: ChatListPreviewPresentation,
    ) -> Self {
        Self { id, icon: icon.into(), icon_is_system, typing_chat_id: None, typing_topic_id: None, presentation }
    }

    fn typing(mut self, typing_chat_id: Option<String>, typing_topic_id: Option<String>) -> Self {
        self.typing_chat_id = typing_chat_id;
        self.typing_topic_id = typing_topic_id;
        self
    }
}

impl RenderOnce for ForumTopicPreviewRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let glyph = if self.icon_is_system {
            let is_filled = self.icon.ends_with(".fill");
            IconUi::sized(&self.icon, px(20.0), theme.text.secondary, is_filled)
                .map(IntoElement::into_any_element)
                .unwrap_or_else(|| div().into_any_element())
        } else {
            TopicIconGlyph::new(self.icon.clone()).size(22.0).into_any_element()
        };
        div().relative().w_full().child(
            ChatListPreviewRow::new(SharedString::from(self.id), self.presentation)
                .horizontal_padding(12.0)
                .vertical_padding(9.0)
                .leading_spacing(8.0)
                .typing_chat_id(self.typing_chat_id)
                .typing_topic_id(self.typing_topic_id)
                .typing_shows_names(true)
                .leading(div().flex().flex_none().items_center().justify_center().size(px(36.0)).child(glyph)),
        )
    }
}

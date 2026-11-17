use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, App, Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, Pixels, Point, ScrollHandle,
    SharedString, StyleRefinement, Subscription, Window,
};
use rise_canon::rise_key::chat as key;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_errors::ErrorCode;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::ui::formatted_text_ui::FormattedTextStyle;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_text::InlineCustomEmojiText;
use crate::modules::chat::components::chat::topic_icon_glyph::TopicIconGlyph;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::pages::chats::chat::chat_topics::channel_kind_editor::ChatChannelKindPresentation;
use crate::modules::chat::pages::chats::chat::chat_topics::member_tag_chip_hooks::MemberTagChip;
use crate::modules::chat::pages::chats::chat::chat_topics::topic_editor_sheet::TopicEditorSheet;
use crate::modules::chat::pages::chats::chat::forum_home::forum_section_name_sheet::{
    ForumSectionNameSheetHost, ForumSectionNameTarget, ForumSectionTitleText,
};
use crate::modules::chat::pages::chats::chats_reorder_toggle_button::ChatsReorderToggleButton;
use crate::modules::chat::shared::chat_forum_ordering_policy::ForumOrderEntry;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatPerm, ChatPermExt, ChatSection, ChatTopic,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

const ACTION_RENAME: &str = "rename";
const ACTION_DELETE: &str = "delete";
const ACTION_EDIT: &str = "edit";
const ACTION_OPEN: &str = "open";
const ACTION_CLOSE: &str = "close";
const ESCAPE_KEY: &str = "escape";

/// what is being reordered
/// sections and topics cant be dragged at once since they use different order units
/// sections have reorder_sections, topics a chat wide position
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForumStructureEditScope {
    Topics,
    Sections,
}

impl ForumStructureEditScope {
    pub const ALL: [Self; 2] = [Self::Topics, Self::Sections];

    pub fn id(self) -> &'static str {
        match self {
            Self::Topics => "topics",
            Self::Sections => "sections",
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Topics => tr("forum_reorder_scope_topics"),
            Self::Sections => tr("forum_reorder_scope_sections"),
        }
    }

    pub fn hint(self) -> String {
        match self {
            Self::Topics => tr("forum_reorder_topics_hint"),
            Self::Sections => tr("forum_reorder_sections_hint"),
        }
    }
}

/// one read of the store per frame
struct ChatTopicsSnapshot {
    enabled: bool,
    can_create_channel: bool,
    can_manage_sections: bool,
    sections: Vec<ChatSection>,
    general_topic: Option<ChatTopic>,
    topics: Vec<ChatTopic>,
    has_forum_topics: bool,
    has_effective_permissions: bool,
    is_chat_owner: bool,
    all_topics_view_hidden: bool,
    manageable_topic_ids: HashSet<String>,
}

impl ChatTopicsSnapshot {
    fn read(chat_id: &str, cx: &mut App) -> Self {
        ChatServicesStore::shared(cx).update(cx, |store, cx| {
            let topics = store.forum_manageable_topics(chat_id, cx);
            let manageable_topic_ids = topics
                .iter()
                .filter(|topic| store.can_manage_channel(chat_id, Some(&topic.id), topic.channel_kind, cx))
                .map(|topic| topic.id.clone())
                .collect();
            let role = store.chat_info(chat_id, cx).and_then(|info| info.role);
            Self {
                enabled: store.topics_enabled(chat_id, cx),
                can_create_channel: store.can_create_channel(chat_id, cx),
                can_manage_sections: store.can(chat_id, ChatPerm::MANAGE_SECTIONS, cx),
                sections: store.sections(chat_id, cx),
                general_topic: store.general_topic(chat_id, cx),
                has_forum_topics: !store.forum_topics(chat_id, cx).is_empty(),
                has_effective_permissions: store.effective_permissions(chat_id, None, cx).is_some(),
                is_chat_owner: ChatPerm::is_owner_role(role.as_deref()),
                all_topics_view_hidden: store.all_topics_view_hidden(chat_id, cx),
                manageable_topic_ids,
                topics,
            }
        })
    }

    /// a topic whose section hasnt arrived or is deleted falls into the loose list instead of vanishing
    /// only get_forum_structure knows sections so until its response the section_id is unknown
    fn grouped_topics(&self) -> (Vec<&ChatTopic>, HashMap<&str, Vec<&ChatTopic>>) {
        let known: HashSet<&str> = self.sections.iter().map(|section| section.id.as_str()).collect();
        let mut loose = Vec::new();
        let mut by_section: HashMap<&str, Vec<&ChatTopic>> = HashMap::new();
        for topic in &self.topics {
            if topic.section_id.is_empty() || !known.contains(topic.section_id.as_str()) {
                loose.push(topic);
            } else {
                by_section.entry(topic.section_id.as_str()).or_default().push(topic);
            }
        }
        (loose, by_section)
    }

    fn topics_by_id(&self) -> HashMap<&str, &ChatTopic> {
        let mut by_id = HashMap::with_capacity(self.topics.len());
        for topic in &self.topics {
            by_id.entry(topic.id.as_str()).or_insert(topic);
        }
        by_id
    }

    fn section_names_by_id(&self) -> HashMap<&str, String> {
        let mut names = HashMap::with_capacity(self.sections.len() + 1);
        names.insert("", tr("forum_no_section"));
        for section in &self.sections {
            names.insert(section.id.as_str(), section.name.clone());
        }
        names
    }

    fn structure_loaded(&self) -> bool {
        !self.sections.is_empty() || self.has_forum_topics
    }

    fn has_reorder_right(&self) -> bool {
        self.can_create_channel || self.can_manage_sections
    }

    fn reorder_right_revoked(&self) -> bool {
        self.has_effective_permissions && !self.has_reorder_right()
    }

    fn can_reorder(&self) -> bool {
        if !self.structure_loaded() {
            return false;
        }
        (self.can_create_channel && !self.topics.is_empty()) || (self.can_manage_sections && !self.sections.is_empty())
    }

    /// only what the viewer may drag
    /// topics on manage topics or manage voice channels, sections on manage sections
    /// chat-service refuses the other one
    fn reorder_scopes(&self) -> Vec<ForumStructureEditScope> {
        let mut scopes = Vec::new();
        if self.can_create_channel {
            scopes.push(ForumStructureEditScope::Topics);
        }
        if self.can_manage_sections {
            scopes.push(ForumStructureEditScope::Sections);
        }
        scopes
    }

    fn base_entries(&self) -> Vec<ForumOrderEntry> {
        let (loose, by_section) = self.grouped_topics();
        let mut entries = Vec::new();
        if !self.sections.is_empty() {
            entries.push(ForumOrderEntry::SectionHeader(String::new()));
        }
        for topic in loose {
            entries.push(ForumOrderEntry::Topic(topic.id.clone()));
        }
        for section in &self.sections {
            entries.push(ForumOrderEntry::SectionHeader(section.id.clone()));
            for topic in by_section.get(section.id.as_str()).into_iter().flatten() {
                entries.push(ForumOrderEntry::Topic(topic.id.clone()));
            }
        }
        entries
    }
}

/// builds a reorder row and its drag gesture, called again whenever the row view renders
type ChatTopicsReorderRowParts = Rc<dyn Fn() -> (Div, Option<DragGesture>)>;

/// what a reorder row shows, the cached row view rebuilds only when this changes
#[derive(Clone, PartialEq)]
enum ChatTopicsReorderRowSignature {
    Header { name: String, is_named: bool },
    Topic(ChatTopic),
    Section(ChatSection, usize),
}

/// a row of the reorder card before the card gives it a slot
struct ChatTopicsReorderRow {
    id: String,
    signature: ChatTopicsReorderRowSignature,
    parts: ChatTopicsReorderRowParts,
}

/// palette a cached reorder row was drawn with
#[derive(Clone, PartialEq)]
struct ChatTopicsReorderThemeKey {
    background_hex: String,
    row_fill: Hsla,
    header_fill: Hsla,
    secondary: Hsla,
}

impl ChatTopicsReorderThemeKey {
    fn of(theme: &AppTheme) -> Self {
        Self {
            background_hex: theme.background_hex.clone(),
            row_fill: theme.bg._200,
            header_fill: theme.bg._300,
            secondary: theme.text.secondary,
        }
    }
}

/// one reorder row as its own view
/// so a drag frame replays settled rows instead of laying them out again
struct ChatTopicsReorderRowView {
    build: Rc<dyn Fn() -> AnyElement>,
}

impl Render for ChatTopicsReorderRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child((self.build)())
    }
}

struct ChatTopicsReorderRowSlot {
    view: Entity<ChatTopicsReorderRowView>,
    signature: ChatTopicsReorderRowSignature,
    edges: (bool, bool),
    theme_key: ChatTopicsReorderThemeKey,
}

/// reorder card rows on their slots as one view
/// a drag sample or slot spring redraws only this surface and not the whole screen
struct ChatTopicsReorderSurface {
    rows: Vec<(String, Entity<ChatTopicsReorderRowView>)>,
    divider: Hsla,
    is_fading: bool,
    drag_offset_y: Animated<f32>,
    offset_row_id: Option<String>,
    slot_offsets: HashMap<String, Animated<f32>>,
}

impl ChatTopicsReorderSurface {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            divider: Hsla::default(),
            is_fading: false,
            drag_offset_y: Animated::new(0.0),
            offset_row_id: None,
            slot_offsets: HashMap::new(),
        }
    }

    fn set_props(&mut self, rows: Vec<(String, Entity<ChatTopicsReorderRowView>)>, divider: Hsla, is_fading: bool) -> bool {
        if self.rows == rows && self.divider == divider && self.is_fading == is_fading {
            return false;
        }
        self.slot_offsets.retain(|id, _| rows.iter().any(|(row_id, _)| row_id == id));
        self.rows = rows;
        self.divider = divider;
        self.is_fading = is_fading;
        true
    }

    fn animate_move(&mut self, dragged_id: &str, target: usize, displaced: Vec<(String, usize)>, cx: &App) {
        if let Some(slot) = self.slot_offsets.get_mut(dragged_id) {
            slot.jump(target as f32 * ChatTopicsScreen::REORDER_ROW_PITCH);
        }
        for (id, slot_index) in displaced {
            if let Some(slot) = self.slot_offsets.get_mut(&id) {
                let slot_target = slot_index as f32 * ChatTopicsScreen::REORDER_ROW_PITCH;
                slot.set_in(slot_target, spring(0.28, 0.86), RiseClock::monotonic(), cx);
            }
        }
    }
}

impl Render for ChatTopicsReorderSurface {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let pitch = ChatTopicsScreen::REORDER_ROW_PITCH;
        let row_height = ChatTopicsScreen::REORDER_ROW_HEIGHT;
        let now = RiseClock::monotonic();
        let scale = window.scale_factor().max(1.0);
        let count = self.rows.len();
        let mut settled = Vec::with_capacity(count);
        let mut lifted = None;
        for (index, (id, view)) in self.rows.iter().enumerate() {
            let target = index as f32 * pitch;
            let slot = self.slot_offsets.entry(id.clone()).or_insert_with(|| Animated::new(target));
            if *slot.target() != target {
                slot.jump(target);
            }
            let is_lifted = self.offset_row_id.as_deref() == Some(id.as_str());
            let offset = if is_lifted { self.drag_offset_y.frame_value(now, window) } else { 0.0 };
            let y = ((slot.frame_value(now, window) + offset) * scale).round() / scale;
            let view = AnyView::from(view.clone());
            let content = if is_lifted || self.is_fading {
                view.into_any_element()
            } else {
                view.cached(StyleRefinement::default().w_full().h(px(row_height))).replay_when_moved().into_any_element()
            };
            let placed = div().absolute().left_0().right_0().top(px(y)).h(px(row_height)).child(content);
            if is_lifted {
                lifted = Some(placed);
            } else {
                settled.push(placed);
            }
        }
        let divider = self.divider;
        let dividers = (1..count).map(move |index| {
            div().absolute().left(px(52.0)).right_0().top(px(index as f32 * pitch - 0.5)).h(px(0.5)).bg(divider)
        });
        div().relative().size_full().overflow_hidden().children(dividers).children(settled).children(lifted)
    }
}

/// topics screen, a view and not an element
/// reorder mode and a running drag live in its fields and the router keeps the entity under a push
pub struct ChatTopicsScreen {
    chat_id: Option<String>,
    focus_handle: FocusHandle,
    show_topic_editor: bool,
    editor_topic_id: Option<String>,
    editor_section_id: Option<String>,
    is_reordering: bool,
    scope: ForumStructureEditScope,
    dragging_id: Option<String>,
    accumulated_shift: f32,
    working_entries: Vec<ForumOrderEntry>,
    working_section_ids: Vec<String>,
    section_name_target: Option<ForumSectionNameTarget>,
    offset_row_id: Option<String>,
    reorder_surface: Entity<ChatTopicsReorderSurface>,
    reorder_row_views: HashMap<String, ChatTopicsReorderRowSlot>,
    content_opacity: Animated<f32>,
    scroll_handle: ScrollHandle,
    drag_pointer: Option<ChatTopicsDragPointer>,
    autoscroll_running: bool,
    _subscriptions: [Subscription; 2],
}

/// which row a running drag moves
#[derive(Clone, PartialEq)]
enum ChatTopicsDragTarget {
    Topic(String),
    Section(String),
}

/// last pointer sample of a running drag
/// kept so edge autoscroll can replay it after the page moved under a still pointer
#[derive(Clone)]
struct ChatTopicsDragPointer {
    target: ChatTopicsDragTarget,
    translation_y: f32,
    pointer_y: f32,
    start_offset_y: f32,
}

impl ChatTopicsScreen {
    const REORDER_ROW_HEIGHT: f32 = 56.0;

    const REORDER_ROW_PITCH: f32 = 56.5;

    const CARD_CORNER_RADIUS: f32 = 20.0;
    const DRAG_MINIMUM_DISTANCE: f32 = 2.0;

    const AUTOSCROLL_BOTTOM_EDGE: f32 = 56.0;
    const AUTOSCROLL_TOP_EDGE: f32 = 120.0;
    const AUTOSCROLL_MAX_STEP: f32 = 14.0;
    const AUTOSCROLL_TICK: Duration = Duration::from_millis(16);

    pub fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            chat_id,
            focus_handle: cx.focus_handle(),
            show_topic_editor: false,
            editor_topic_id: None,
            editor_section_id: None,
            is_reordering: false,
            scope: ForumStructureEditScope::Topics,
            dragging_id: None,
            accumulated_shift: 0.0,
            working_entries: Vec::new(),
            working_section_ids: Vec::new(),
            section_name_target: None,
            offset_row_id: None,
            reorder_surface: cx.new(|_| ChatTopicsReorderSurface::new()),
            reorder_row_views: HashMap::new(),
            content_opacity: Animated::new(1.0),
            scroll_handle: ScrollHandle::new(),
            drag_pointer: None,
            autoscroll_running: false,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn resolved_chat_id(&self) -> String {
        self.chat_id.clone().unwrap_or_default()
    }

    fn active_scope(&self, snapshot: &ChatTopicsSnapshot) -> ForumStructureEditScope {
        let scopes = snapshot.reorder_scopes();
        if scopes.contains(&self.scope) {
            self.scope
        } else {
            scopes.first().copied().unwrap_or(ForumStructureEditScope::Topics)
        }
    }

    fn set_reordering(&mut self, is_reordering: bool, cx: &mut Context<Self>) {
        self.is_reordering = is_reordering;
        self.content_opacity.jump(0.0);
        self.content_opacity.set_in(1.0, spring(0.32, 0.85), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn toggle_reordering(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = selection_changed();
        let is_reordering = !self.is_reordering;
        if is_reordering {
            window.focus(&self.focus_handle, cx);
        }
        self.set_reordering(is_reordering, cx);
    }

    fn reset_drag(&mut self, cx: &mut Context<Self>) {
        self.drag_pointer = None;
        self.dragging_id = None;
        self.offset_row_id = None;
        self.reorder_surface.update(cx, |surface, cx| {
            surface.offset_row_id = None;
            surface.drag_offset_y.jump(0.0);
            cx.notify();
        });
        self.accumulated_shift = 0.0;
        self.working_entries = Vec::new();
        self.working_section_ids = Vec::new();
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_reordering
            || !self.focus_handle.is_focused(window)
            || event.keystroke.modifiers.modified()
            || event.keystroke.key != ESCAPE_KEY
        {
            return;
        }
        cx.stop_propagation();
        self.reset_drag(cx);
        self.set_reordering(false, cx);
    }

    fn reorder_right_changed(&mut self, revoked: bool, cx: &mut Context<Self>) {
        if !revoked || !self.is_reordering {
            return;
        }
        self.reset_drag(cx);
        self.set_reordering(false, cx);
        let snapshot = ChatTopicsSnapshot::read(&self.resolved_chat_id(), cx);
        if !snapshot.can_create_channel
            && !snapshot.can_manage_sections
            && let Some(text) = CanonErrors::text(Some(ErrorCode::ChatPermissionDenied.wire()), &HashMap::new())
        {
            let options = NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            };
            Notifier::send(text, options, cx);
        }
    }

    fn present_topic_editor(&mut self, topic_id: Option<String>, section_id: Option<String>, cx: &mut Context<Self>) {
        self.editor_topic_id = topic_id;
        self.editor_section_id = section_id;
        self.show_topic_editor = true;
        cx.notify();
    }

    fn present_create_section(&mut self, cx: &mut Context<Self>) {
        self.section_name_target = Some(ForumSectionNameTarget {
            chat_id: self.resolved_chat_id(),
            section_id: None,
            name: String::new(),
            name_entities: None,
        });
        cx.notify();
    }

    fn section_actions() -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new(ACTION_RENAME, tr("forum_section_rename")).system_image("pencil"),
            HoldContextMenuAction::new(ACTION_DELETE, tr("forum_section_delete"))
                .system_image("trash")
                .destructive(true),
        ]
    }

    fn handle_section_action(&mut self, id: &str, section: &ChatSection, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id();
        let can_manage = ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.can(&chat_id, ChatPerm::MANAGE_SECTIONS, cx));
        if !can_manage {
            return;
        }
        match id {
            ACTION_RENAME => {
                self.section_name_target = Some(ForumSectionNameTarget {
                    chat_id,
                    section_id: Some(section.id.clone()),
                    name: section.name.clone(),
                    name_entities: section.name_entities.clone(),
                });
                cx.notify();
            }
            ACTION_DELETE => {
                let section_id = section.id.clone();
                NativeConfirmAlert::present_delete(
                    &section.name,
                    "",
                    Some(tr("forum_section_delete")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.delete_section(&chat_id, &section_id, cx);
                        });
                    },
                    cx,
                );
            }
            _ => {}
        }
    }

    fn topic_actions(topic: &ChatTopic, can_manage: bool) -> Vec<HoldContextMenuAction> {
        if !can_manage || topic.is_general {
            return Vec::new();
        }
        let (toggle_id, toggle_title, toggle_image) = if topic.is_closed {
            (ACTION_OPEN, tr("chat_topics_open"), "lock.open")
        } else {
            (ACTION_CLOSE, tr("chat_topics_close"), "lock")
        };
        vec![
            HoldContextMenuAction::new(ACTION_EDIT, tr("ctx_edit")).system_image("pencil"),
            HoldContextMenuAction::new(toggle_id, toggle_title).system_image(toggle_image),
            HoldContextMenuAction::new(ACTION_DELETE, tr("chat_topics_delete"))
                .system_image("trash")
                .destructive(true),
        ]
    }

    fn handle_topic_action(&mut self, id: &str, topic: &ChatTopic, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id();
        let can_manage = ChatServicesStore::shared(cx).update(cx, |store, cx| {
            store.can_manage_channel(&chat_id, Some(&topic.id), topic.channel_kind, cx)
        });
        if !can_manage {
            return;
        }
        match id {
            ACTION_EDIT => {
                let section_id = (!topic.section_id.is_empty()).then(|| topic.section_id.clone());
                self.present_topic_editor(Some(topic.id.clone()), section_id, cx);
            }
            ACTION_DELETE => {
                let topic_id = topic.id.clone();
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
            ACTION_CLOSE | ACTION_OPEN => {
                let closed = id == ACTION_CLOSE;
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.set_topic_closed(&chat_id, &topic.id, closed, cx);
                });
            }
            _ => {}
        }
    }

    /// swaps neighbours for every threshold the offset crossed in one event
    /// a mouse reports sparser samples than a finger so one swap per event fell short
    /// returns the new index
    fn drag_step(&mut self, translation_y: f32, index: usize, count: usize) -> usize {
        let threshold = Self::REORDER_ROW_PITCH * 0.6;
        let mut target = index;
        loop {
            let offset = translation_y - self.accumulated_shift;
            if offset > threshold && target + 1 < count {
                self.accumulated_shift += Self::REORDER_ROW_PITCH;
                target += 1;
            } else if offset < -threshold && target > 0 {
                self.accumulated_shift -= Self::REORDER_ROW_PITCH;
                target -= 1;
            } else {
                break;
            }
        }
        target
    }

    /// puts the lifted row under the pointer and after a swap slides the rows it passed
    /// only the surface redraws, a sample that leaves the offset alone redraws nothing
    fn move_lifted_row(&self, row_id: &str, offset: f32, moved: Option<(usize, Vec<(String, usize)>)>, cx: &mut Context<Self>) {
        let row_id = row_id.to_owned();
        self.reorder_surface.update(cx, |surface, cx| {
            let unchanged = moved.is_none()
                && surface.offset_row_id.as_deref() == Some(row_id.as_str())
                && *surface.drag_offset_y.target() == offset
                && !surface.drag_offset_y.is_running(RiseClock::monotonic());
            if unchanged {
                return;
            }
            surface.offset_row_id = Some(row_id.clone());
            surface.drag_offset_y.jump(offset);
            if let Some((target, displaced)) = moved {
                surface.animate_move(&row_id, target, displaced, cx);
            }
            cx.notify();
        });
    }

    /// the released row settles back to its slot on the drop spring
    fn settle_lifted_row(&self, cx: &mut Context<Self>) {
        self.reorder_surface.update(cx, |surface, cx| {
            surface.drag_offset_y.set_in(0.0, spring(0.3, 0.86), RiseClock::monotonic(), cx);
            cx.notify();
        });
    }

    /// successive neighbour swaps from index to target are one remove and insert
    /// the ids between them with new indices are the rows that slid
    fn move_item<T>(items: &mut Vec<T>, index: usize, target: usize, id_of: impl Fn(&T) -> String) -> Vec<(String, usize)> {
        let moved = items.remove(index);
        items.insert(target, moved);
        (index.min(target)..=index.max(target))
            .filter(|position| *position != target)
            .filter_map(|position| items.get(position).map(|item| (id_of(item), position)))
            .collect()
    }

    fn drag_changed(&mut self, target: ChatTopicsDragTarget, value: &DragGestureValue, pointer_y: f32, cx: &mut Context<Self>) {
        let translation_y = f32::from(value.translation.height);
        match self.drag_pointer.as_mut() {
            Some(pointer) if pointer.target == target => {
                pointer.translation_y = translation_y;
                pointer.pointer_y = pointer_y;
            }
            _ => {
                let start_offset_y = f32::from(self.scroll_handle.offset().y);
                self.drag_pointer = Some(ChatTopicsDragPointer { target, translation_y, pointer_y, start_offset_y });
            }
        }
        self.apply_drag_pointer(cx);
        self.start_autoscroll_if_needed(cx);
    }

    fn apply_drag_pointer(&mut self, cx: &mut Context<Self>) {
        let Some(pointer) = self.drag_pointer.clone() else {
            return;
        };
        let scrolled = pointer.start_offset_y - f32::from(self.scroll_handle.offset().y);
        let translation_y = pointer.translation_y + scrolled;
        match &pointer.target {
            ChatTopicsDragTarget::Topic(topic_id) => self.update_topic_drag(topic_id, translation_y, cx),
            ChatTopicsDragTarget::Section(section_id) => self.update_section_drag(section_id, translation_y, cx),
        }
    }

    /// points to scroll this tick, negative up and positive down
    /// zero outside the edge zones, speed grows with how deep the pointer sits
    /// a zone acts only on a drag heading into it
    fn autoscroll_step(&self) -> f32 {
        let Some(pointer) = self.drag_pointer.as_ref() else {
            return 0.0;
        };
        let bounds = self.scroll_handle.bounds();
        let top = f32::from(bounds.top());
        let bottom = f32::from(bounds.bottom());
        if bottom - top <= Self::AUTOSCROLL_TOP_EDGE + Self::AUTOSCROLL_BOTTOM_EDGE {
            return 0.0;
        }
        let from_bottom = bottom - pointer.pointer_y;
        let from_top = pointer.pointer_y - top;
        if from_bottom < Self::AUTOSCROLL_BOTTOM_EDGE && pointer.translation_y > 0.0 {
            let depth = (1.0 - from_bottom / Self::AUTOSCROLL_BOTTOM_EDGE).clamp(0.0, 1.0);
            depth * Self::AUTOSCROLL_MAX_STEP
        } else if from_top < Self::AUTOSCROLL_TOP_EDGE && pointer.translation_y < 0.0 {
            let depth = (1.0 - from_top / Self::AUTOSCROLL_TOP_EDGE).clamp(0.0, 1.0);
            -depth * Self::AUTOSCROLL_MAX_STEP
        } else {
            0.0
        }
    }

    fn start_autoscroll_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.autoscroll_running || self.autoscroll_step() == 0.0 {
            return;
        }
        self.autoscroll_running = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Self::AUTOSCROLL_TICK).await;
                let keeps = this.update(cx, |this, cx| this.autoscroll_tick(cx)).unwrap_or(false);
                if !keeps {
                    break;
                }
            }
        })
        .detach();
    }

    fn autoscroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let step = self.autoscroll_step();
        let offset = self.scroll_handle.offset();
        let max_y = f32::from(self.scroll_handle.max_offset().y).max(0.0);
        let current_y = f32::from(offset.y);
        let next_y = (current_y - step).clamp(-max_y, 0.0);
        if step == 0.0 || self.dragging_id.is_none() || next_y == current_y {
            self.autoscroll_running = false;
            return false;
        }
        self.scroll_handle.set_offset(Point { x: offset.x, y: px(next_y) });
        self.apply_drag_pointer(cx);
        cx.notify();
        true
    }

    fn update_topic_drag(&mut self, topic_id: &str, translation_y: f32, cx: &mut Context<Self>) {
        let entry = ForumOrderEntry::Topic(topic_id.to_owned());
        let entry_id = entry.id();
        if self.dragging_id.as_deref() != Some(entry_id.as_str()) {
            self.dragging_id = Some(entry_id.clone());
            self.offset_row_id = Some(entry_id.clone());
            self.working_entries = ChatTopicsSnapshot::read(&self.resolved_chat_id(), cx).base_entries();
            self.accumulated_shift = 0.0;
            cx.notify();
        }
        let Some(index) = self.working_entries.iter().position(|candidate| *candidate == entry) else {
            self.move_lifted_row(&entry_id, translation_y - self.accumulated_shift, None, cx);
            return;
        };
        let target = self.drag_step(translation_y, index, self.working_entries.len());
        let mut moved = None;
        if target != index {
            moved = Some((target, Self::move_item(&mut self.working_entries, index, target, ForumOrderEntry::id)));
            cx.notify();
        }
        self.move_lifted_row(&entry_id, translation_y - self.accumulated_shift, moved, cx);
    }

    fn end_topic_drag(&mut self, cx: &mut Context<Self>) {
        self.drag_pointer = None;
        let entries = std::mem::take(&mut self.working_entries);
        self.settle_lifted_row(cx);
        self.dragging_id = None;
        self.accumulated_shift = 0.0;
        cx.notify();
        if entries.is_empty() {
            return;
        }
        let chat_id = self.resolved_chat_id();
        let placements = ForumOrderEntry::placements(&entries);
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.reorder_topics(&chat_id, &placements, cx);
        });
    }

    fn update_section_drag(&mut self, section_id: &str, translation_y: f32, cx: &mut Context<Self>) {
        if self.dragging_id.as_deref() != Some(section_id) {
            self.dragging_id = Some(section_id.to_owned());
            self.offset_row_id = Some(section_id.to_owned());
            let snapshot = ChatTopicsSnapshot::read(&self.resolved_chat_id(), cx);
            self.working_section_ids = snapshot.sections.into_iter().map(|section| section.id).collect();
            self.accumulated_shift = 0.0;
            cx.notify();
        }
        let Some(index) = self.working_section_ids.iter().position(|id| id == section_id) else {
            self.move_lifted_row(section_id, translation_y - self.accumulated_shift, None, cx);
            return;
        };
        let target = self.drag_step(translation_y, index, self.working_section_ids.len());
        let mut moved = None;
        if target != index {
            moved = Some((target, Self::move_item(&mut self.working_section_ids, index, target, String::clone)));
            cx.notify();
        }
        self.move_lifted_row(section_id, translation_y - self.accumulated_shift, moved, cx);
    }

    fn end_section_drag(&mut self, cx: &mut Context<Self>) {
        self.drag_pointer = None;
        let ordered = std::mem::take(&mut self.working_section_ids);
        self.settle_lifted_row(cx);
        self.dragging_id = None;
        self.accumulated_shift = 0.0;
        cx.notify();
        if ordered.is_empty() {
            return;
        }
        let chat_id = self.resolved_chat_id();
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.reorder_sections(&chat_id, &ordered, cx);
        });
    }

    fn displayed_entries(&self, snapshot: &ChatTopicsSnapshot) -> Vec<ForumOrderEntry> {
        if self.dragging_id.is_none() || self.working_entries.is_empty() {
            snapshot.base_entries()
        } else {
            self.working_entries.clone()
        }
    }

    fn displayed_sections(&self, snapshot: &ChatTopicsSnapshot) -> Vec<ChatSection> {
        if self.dragging_id.is_none() || self.working_section_ids.is_empty() {
            return snapshot.sections.clone();
        }
        let by_id: HashMap<&str, &ChatSection> =
            snapshot.sections.iter().rev().map(|section| (section.id.as_str(), section)).collect();
        self.working_section_ids.iter().filter_map(|id| by_id.get(id.as_str()).map(|section| (*section).clone())).collect()
    }

    fn card(content: Div, theme: &AppTheme) -> Div {
        div()
            .w_full()
            .px(px(12.0))
            .child(content.w_full().bg(theme.bg._200).rounded(px(Self::CARD_CORNER_RADIUS)))
    }

    fn divider(theme: &AppTheme) -> Div {
        div().w_full().pl(px(52.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
    }

    fn glyph_tile(theme: &AppTheme) -> Div {
        div()
            .flex_none()
            .w(px(29.0))
            .h(px(29.0))
            .rounded(px(8.0))
            .bg(theme.bg._300)
            .flex()
            .items_center()
            .justify_center()
    }

    fn topic_glyph(topic: &ChatTopic, theme: &AppTheme) -> Div {
        let color = theme.text.secondary;
        let tile = Self::glyph_tile(theme).text_color(color);
        if topic.is_general {
            tile.children(IconUi::sized("number", px(15.0), color, false))
        } else if topic.channel_kind == ChatChannelKind::Text {
            let icon = if topic.icon.is_empty() { "#".to_owned() } else { topic.icon.clone() };
            tile.child(TopicIconGlyph::new(icon))
        } else {
            tile.children(IconUi::sized(topic.channel_kind.glyph_name(), px(15.0), color, true))
        }
    }

    fn topic_row(topic: &ChatTopic, theme: &AppTheme) -> Div {
        let muted = theme.text.secondary.opacity(0.7);
        let title = if topic.is_general { tr("chat_topics_general") } else { topic.title.clone() };
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .py(px(11.0))
            .bg(theme.bg._200)
            .child(Self::topic_glyph(topic, theme))
            .child(MainText::new(title).px(15.0).number_of_lines(1))
            .child(spacer_min_w(4.0))
            .when(topic.is_closed, |row| row.children(IconUi::sized("lock.fill", px(12.0), muted, true)))
            .when(topic.is_private, |row| row.children(IconUi::sized("lock.shield.fill", px(12.0), muted, false)))
            .when(topic.is_general, |row| row.child(MemberTagChip::new(tr("chat_topics_general_badge"))))
    }

    fn general_placeholder_row(theme: &AppTheme) -> Div {
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .py(px(11.0))
            .bg(theme.bg._200)
            .child(Self::glyph_tile(theme).children(IconUi::sized("number", px(16.0), theme.text.secondary, false)))
            .child(MainText::new(tr("chat_topics_general")).px(15.0).number_of_lines(1))
            .child(spacer_min_w(4.0))
            .child(MemberTagChip::new(tr("chat_topics_general_badge")))
    }

    fn manageable_topic_row(
        topic: &ChatTopic,
        snapshot: &ChatTopicsSnapshot,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !snapshot.manageable_topic_ids.contains(&topic.id) || topic.is_general {
            return Self::topic_row(topic, theme).into_any_element();
        }
        let actions = Self::topic_actions(topic, true);
        let entity = cx.entity();
        let (action_entity, action_topic) = (entity.clone(), topic.clone());
        let on_action: Rc<dyn Fn(String, &mut Window, &mut App)> = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            action_entity.update(cx, |this, cx| this.handle_topic_action(&id, &action_topic, cx));
        });
        let tap_topic = topic.clone();
        let on_tap: Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)> =
            Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                let section_id = (!tap_topic.section_id.is_empty()).then(|| tap_topic.section_id.clone());
                entity.update(cx, |this, cx| this.present_topic_editor(Some(tap_topic.id.clone()), section_id, cx));
            });
        let (row_topic, row_theme) = (topic.clone(), theme.clone());
        div()
            .w_full()
            .child(
                HoldContextMenuContainer::new(
                    SharedString::from(format!("chat-topic-hold-{}", topic.id)),
                    HoldContextMenuAlignment::Leading,
                    Rc::new(move |_: &App| actions.clone()),
                    Rc::new(move |_: &mut Window, _: &mut App| Self::topic_row(&row_topic, &row_theme).into_any_element()),
                )
                .show_emoji_list(false)
                .lifted_background(Some(theme.bg._200))
                .on_action(on_action)
                .on_tap(on_tap),
            )
            .into_any_element()
    }

    fn add_topic_row(section_id: &str, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let target = section_id.to_owned();
        plain(SharedString::from(format!("chat-topic-add-{section_id}")))
            .w_full()
            .on_press(move |_, cx| {
                entity.update(cx, |this, cx| this.present_topic_editor(None, Some(target.clone()), cx));
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(11.0))
                    .bg(theme.bg._200)
                    .child(Self::glyph_tile(theme).children(IconUi::sized("plus", px(15.0), theme.primary._100, false)))
                    .child(
                        MainText::new(tr("forum_add_topic"))
                            .px(15.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(theme.primary._100),
                    )
                    .child(spacer_min_w(4.0)),
            )
    }

    fn reorder_section_title(section: &ChatSection, theme: &AppTheme) -> AnyElement {
        let emojis = section.name_custom_emojis();
        if emojis.is_empty() {
            return MainText::new(section.name.clone()).px(15.0).number_of_lines(1).into_any_element();
        }
        div()
            .line_clamp(1)
            .text_ellipsis()
            .child(InlineCustomEmojiText::new(
                SharedString::from(format!("chat-topics-reorder-section-{}", section.id)),
                section.name.clone(),
                FormattedTextStyle::app_standard(theme, 15.0),
                emojis,
            ))
            .into_any_element()
    }

    fn section_label(section: &ChatSection, theme: &AppTheme) -> Div {
        div().w_full().px(px(28.0)).py(px(4.0)).child(ForumSectionTitleText::new(
            section.name.clone(),
            section.name_custom_emojis(),
            theme.text.secondary,
        ))
    }

    fn section_header(
        section: &ChatSection,
        can_manage_sections: bool,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !can_manage_sections {
            return Self::section_label(section, theme).into_any_element();
        }
        let entity = cx.entity();
        let action_section = section.clone();
        let on_action: Rc<dyn Fn(String, &mut Window, &mut App)> = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            entity.update(cx, |this, cx| this.handle_section_action(&id, &action_section, cx));
        });
        let (row_section, row_theme) = (section.clone(), theme.clone());
        HoldContextMenuContainer::new(
            SharedString::from(format!("chat-section-hold-{}", section.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| Self::section_actions()),
            Rc::new(move |_: &mut Window, _: &mut App| Self::section_label(&row_section, &row_theme).into_any_element()),
        )
        .show_emoji_list(false)
        .on_action(on_action)
        .into_any_element()
    }

    fn section_card(
        section: &ChatSection,
        section_topics: &[&ChatTopic],
        snapshot: &ChatTopicsSnapshot,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut body = vstack(0.0);
        if section_topics.is_empty() {
            body = body.child(div().w_full().px(px(16.0)).py(px(16.0)).child(
                MainText::new(tr("forum_section_empty")).px(13.0).color(theme.text.secondary).number_of_lines(2),
            ));
        } else {
            for (index, topic) in section_topics.iter().enumerate() {
                if index > 0 {
                    body = body.child(Self::divider(theme));
                }
                body = body.child(Self::manageable_topic_row(topic, snapshot, theme, cx));
            }
        }
        if snapshot.can_create_channel {
            body = body.child(Self::divider(theme)).child(Self::add_topic_row(&section.id, theme, cx));
        }
        vstack(7.0)
            .w_full()
            .items_start()
            .child(Self::section_header(section, snapshot.can_manage_sections, theme, cx))
            .child(Self::card(body, theme))
    }

    fn browse_list(snapshot: &ChatTopicsSnapshot, theme: &AppTheme, cx: &mut Context<Self>) -> Div {
        let (loose, by_section) = snapshot.grouped_topics();
        let mut top = vstack(0.0);
        top = match &snapshot.general_topic {
            Some(general) => top.child(Self::topic_row(general, theme)),
            None => top.child(Self::general_placeholder_row(theme)),
        };
        for topic in loose {
            top = top.child(Self::divider(theme)).child(Self::manageable_topic_row(topic, snapshot, theme, cx));
        }
        let mut list = vstack(16.0).w_full().child(Self::card(top, theme));
        for section in &snapshot.sections {
            let section_topics = by_section.get(section.id.as_str()).map(Vec::as_slice).unwrap_or_default();
            list = list.child(Self::section_card(section, section_topics, snapshot, theme, cx));
        }
        list
    }

    fn hide_all_topics_card(&self, snapshot: &ChatTopicsSnapshot) -> GroupedBtns {
        let hidden = snapshot.all_topics_view_hidden;
        let chat_id = self.resolved_chat_id();
        let (switch_chat_id, row_chat_id) = (chat_id.clone(), chat_id.clone());
        GroupedBtns::new(
            SharedString::from(format!("chat-topics-hide-all-{chat_id}")),
            vec![
                GroupedBtn::new(key::HIDE_ALL_TOPICS, tr("chat_topics_hide_all"))
                    .icon("tray.full.fill")
                    .icon_background(ChatTileColor::orange())
                    .trailing_slot(SwitchUi::new(
                        SharedString::from(format!("chat-topics-hide-all-switch-{chat_id}")),
                        Binding::new(
                            move |_| hidden,
                            move |value, _, cx| {
                                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                    interactions.set_all_topics_view_hidden(&switch_chat_id, value, cx);
                                });
                            },
                        ),
                    ))
                    .callback(move |_, cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.set_all_topics_view_hidden(&row_chat_id, !hidden, cx);
                        });
                    }),
            ],
        )
    }

    fn enable_card(&self, theme: &AppTheme) -> Div {
        let chat_id = self.resolved_chat_id();
        let button_id = SharedString::from(format!("chat-topics-enable-{chat_id}"));
        vstack(14.0)
            .w_full()
            .px(px(24.0))
            .pt(px(24.0))
            .children(IconUi::sized(
                "list.bullet.rectangle.portrait",
                px(40.0),
                theme.text.secondary.opacity(0.6),
                false,
            ))
            .child(div().max_w(px(300.0)).child(
                MainText::new(tr("chat_topics_hint")).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary),
            ))
            .child(
                plain(button_id)
                    .w_full()
                    .on_press(move |_, cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.enable_topics(&chat_id, cx);
                        });
                    })
                    .child(
                        div()
                            .w_full()
                            .min_h(px(48.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.primary._100)
                            .rounded_full()
                            .child(
                                MainText::new(tr("chat_topics_enable"))
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.content_on(theme.primary._100)),
                            ),
                    )
                    .guide_anchor(RiseGuideAnchor::ChatTopicsEnable),
            )
    }

    fn reorder_scope_picker(&self, snapshot: &ChatTopicsSnapshot, theme: &AppTheme, cx: &mut Context<Self>) -> Div {
        let scopes = snapshot.reorder_scopes();
        let active = self.active_scope(snapshot);
        let is_dragging = self.dragging_id.is_some();
        let mut picker = vstack(8.0).w_full().px(px(16.0));
        if scopes.len() > 1 {
            let entity = cx.entity();
            let titles: Vec<SharedString> = scopes.iter().map(|scope| scope.title().into()).collect();
            let selected = scopes.iter().position(|scope| *scope == active).unwrap_or(0);
            let on_select_scopes = scopes.clone();
            picker = picker.child(
                div().w_full().when(is_dragging, |wrapper| wrapper.opacity(0.5)).child(SegmentedPicker::new(
                    "chat-topics-reorder-scope",
                    titles,
                    selected,
                    move |index, _, cx| {
                        let Some(scope) = on_select_scopes.get(index).copied() else {
                            return;
                        };
                        entity.update(cx, |this, cx| {
                            if this.dragging_id.is_none() {
                                this.scope = scope;
                                cx.notify();
                            }
                        });
                    },
                )),
            );
        }
        picker.child(
            MainText::new(active.hint())
                .px(12.0)
                .tac(TextAlignT::Center)
                .color(theme.text.secondary)
                .number_of_lines(2),
        )
    }

    fn drag_handle(theme: &AppTheme) -> Div {
        div()
            .flex_none()
            .w(px(36.0))
            .h(px(Self::REORDER_ROW_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("line.3.horizontal", px(18.0), theme.text.secondary, false))
    }

    fn reorder_entry_row(
        entry: &ForumOrderEntry,
        topics: &HashMap<&str, &ChatTopic>,
        section_names: &HashMap<&str, String>,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> Option<ChatTopicsReorderRow> {
        match entry {
            ForumOrderEntry::SectionHeader(section_id) => {
                let is_named = !section_id.is_empty();
                let symbol = if is_named { "folder.fill" } else { "tray" };
                let name = section_names
                    .get(section_id.as_str())
                    .cloned()
                    .unwrap_or_else(|| tr("forum_no_section"))
                    .to_uppercase();
                let signature = ChatTopicsReorderRowSignature::Header { name: name.clone(), is_named };
                let theme = theme.clone();
                let parts: ChatTopicsReorderRowParts = Rc::new(move || {
                    let row = hstack(12.0)
                        .w_full()
                        .px(px(16.0))
                        .h(px(Self::REORDER_ROW_HEIGHT))
                        .bg(theme.bg._300.opacity(0.55))
                        .child(Self::glyph_tile(&theme).children(IconUi::sized(
                            symbol,
                            px(14.0),
                            theme.text.secondary,
                            is_named,
                        )))
                        .child(
                            MainText::new(name.clone())
                                .px(12.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.secondary)
                                .number_of_lines(1),
                        )
                        .child(spacer_min_w(4.0));
                    (row, None)
                });
                Some(ChatTopicsReorderRow { id: entry.id(), signature, parts })
            }
            ForumOrderEntry::Topic(topic_id) => {
                let topic = (*topics.get(topic_id.as_str())?).clone();
                let signature = ChatTopicsReorderRowSignature::Topic(topic.clone());
                let theme = theme.clone();
                let entity = cx.entity();
                let dragged_topic_id = topic_id.clone();
                let parts: ChatTopicsReorderRowParts = Rc::new(move || {
                    let row = hstack(12.0)
                        .w_full()
                        .px(px(16.0))
                        .h(px(Self::REORDER_ROW_HEIGHT))
                        .bg(theme.bg._200)
                        .child(Self::topic_glyph(&topic, &theme))
                        .child(MainText::new(topic.title.clone()).px(15.0).number_of_lines(1))
                        .child(spacer_min_w(4.0))
                        .child(Self::drag_handle(&theme));
                    let (changed, ended) = (entity.clone(), entity.clone());
                    let dragged_topic_id = dragged_topic_id.clone();
                    let gesture = DragGesture::new(Self::DRAG_MINIMUM_DISTANCE)
                        .on_changed(move |value, window, cx| {
                            let target = ChatTopicsDragTarget::Topic(dragged_topic_id.clone());
                            let pointer_y = f32::from(window.mouse_position().y);
                            changed.update(cx, |this, cx| this.drag_changed(target, value, pointer_y, cx));
                        })
                        .on_ended(move |_, _, cx| ended.update(cx, |this, cx| this.end_topic_drag(cx)));
                    (row, Some(gesture))
                });
                Some(ChatTopicsReorderRow { id: entry.id(), signature, parts })
            }
        }
    }

    fn section_reorder_row(
        section: &ChatSection,
        topic_count: usize,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> ChatTopicsReorderRow {
        let signature = ChatTopicsReorderRowSignature::Section(section.clone(), topic_count);
        let section = section.clone();
        let theme = theme.clone();
        let entity = cx.entity();
        let id = section.id.clone();
        let parts: ChatTopicsReorderRowParts = Rc::new(move || {
            let row = hstack(12.0)
                .w_full()
                .px(px(16.0))
                .h(px(Self::REORDER_ROW_HEIGHT))
                .bg(theme.bg._200)
                .child(Self::glyph_tile(&theme).children(IconUi::sized(
                    "folder.fill",
                    px(14.0),
                    theme.text.secondary,
                    true,
                )))
                .child(Self::reorder_section_title(&section, &theme))
                .child(spacer_min_w(4.0))
                .child(MemberTagChip::new(topic_count.to_string()))
                .child(Self::drag_handle(&theme));
            let (changed, ended) = (entity.clone(), entity.clone());
            let section_id = section.id.clone();
            let gesture = DragGesture::new(Self::DRAG_MINIMUM_DISTANCE)
                .on_changed(move |value, window, cx| {
                    let target = ChatTopicsDragTarget::Section(section_id.clone());
                    let pointer_y = f32::from(window.mouse_position().y);
                    changed.update(cx, |this, cx| this.drag_changed(target, value, pointer_y, cx));
                })
                .on_ended(move |_, _, cx| ended.update(cx, |this, cx| this.end_section_drag(cx)));
            (row, Some(gesture))
        });
        ChatTopicsReorderRow { id, signature, parts }
    }

    fn reorder_row_view(
        &mut self,
        item: ChatTopicsReorderRow,
        edges: (bool, bool),
        theme_key: &ChatTopicsReorderThemeKey,
        is_lifted: bool,
        cx: &mut Context<Self>,
    ) -> Entity<ChatTopicsReorderRowView> {
        let ChatTopicsReorderRow { id, signature, parts } = item;
        let radius = px(Self::CARD_CORNER_RADIUS);
        let gesture_id = SharedString::from(format!("chat-topics-reorder-{id}"));
        let build: Rc<dyn Fn() -> AnyElement> = Rc::new(move || {
            let (row, gesture) = parts();
            let row = row.when(edges.0, |row| row.rounded_t(radius)).when(edges.1, |row| row.rounded_b(radius));
            match gesture {
                Some(gesture) => row.cursor_grab().gesture(gesture_id.clone(), gesture).into_any_element(),
                None => row.into_any_element(),
            }
        });
        if let Some(slot) = self.reorder_row_views.get_mut(&id) {
            let unchanged = slot.signature == signature && slot.edges == edges && slot.theme_key == *theme_key;
            if unchanged || is_lifted {
                slot.signature = signature;
                slot.edges = edges;
                slot.theme_key = theme_key.clone();
                slot.view.update(cx, |view, _| view.build = build);
                return slot.view.clone();
            }
        }
        let view = cx.new(|_| ChatTopicsReorderRowView { build });
        let slot = ChatTopicsReorderRowSlot { view: view.clone(), signature, edges, theme_key: theme_key.clone() };
        self.reorder_row_views.insert(id, slot);
        view
    }

    /// card around the reorder surface
    /// rows go to the surface, it replays while they and their order stay
    /// and renders alone for drag samples and slot springs
    fn reorder_card(&mut self, rows: Vec<ChatTopicsReorderRow>, theme: &AppTheme, cx: &mut Context<Self>) -> Div {
        let count = rows.len();
        self.reorder_row_views.retain(|id, _| rows.iter().any(|row| row.id == *id));
        let theme_key = ChatTopicsReorderThemeKey::of(theme);
        let is_fading = self.content_opacity.is_running(RiseClock::monotonic());
        let mut views = Vec::with_capacity(count);
        for (index, item) in rows.into_iter().enumerate() {
            let id = item.id.clone();
            let is_lifted = self.offset_row_id.as_deref() == Some(id.as_str());
            let view = self.reorder_row_view(item, (index == 0, index + 1 == count), &theme_key, is_lifted, cx);
            views.push((id, view));
        }
        let divider = theme.border._200.opacity(0.55);
        let changed = self.reorder_surface.update(cx, |surface, _| surface.set_props(views, divider, is_fading));
        let height = (count as f32 * Self::REORDER_ROW_PITCH - 0.5).max(0.0);
        let surface = AnyView::from(self.reorder_surface.clone());
        let surface = if changed || is_fading {
            surface.into_any_element()
        } else {
            surface.cached(StyleRefinement::default().w_full().h(px(height))).into_any_element()
        };
        Self::card(div().relative().h(px(height)).overflow_hidden().child(surface), theme)
    }

    fn reorder_list(&mut self, snapshot: &ChatTopicsSnapshot, theme: &AppTheme, cx: &mut Context<Self>) -> Div {
        match self.active_scope(snapshot) {
            ForumStructureEditScope::Topics => {
                let entries = self.displayed_entries(snapshot);
                let by_id = snapshot.topics_by_id();
                let names = snapshot.section_names_by_id();
                let rows = entries
                    .iter()
                    .filter_map(|entry| Self::reorder_entry_row(entry, &by_id, &names, theme, cx))
                    .collect();
                self.reorder_card(rows, theme, cx)
            }
            ForumStructureEditScope::Sections => {
                let ordered = self.displayed_sections(snapshot);
                if ordered.is_empty() {
                    return Self::card(
                        div().w_full().px(px(16.0)).py(px(16.0)).child(
                            MainText::new(tr("forum_section_empty"))
                                .px(13.0)
                                .color(theme.text.secondary)
                                .number_of_lines(2),
                        ),
                        theme,
                    );
                }
                let (_, by_section) = snapshot.grouped_topics();
                let rows = ordered
                    .iter()
                    .map(|section| {
                        let topic_count = by_section.get(section.id.as_str()).map_or(0, Vec::len);
                        Self::section_reorder_row(section, topic_count, theme, cx)
                    })
                    .collect();
                self.reorder_card(rows, theme, cx)
            }
        }
    }

    fn create_buttons(&self, snapshot: &ChatTopicsSnapshot, content: Div, chat_id: &str, cx: &mut Context<Self>) -> Div {
        let mut content = content;
        if snapshot.can_create_channel {
            let entity = cx.entity();
            content = content.child(GroupedBtns::new(
                SharedString::from(format!("chat-topics-create-{chat_id}")),
                vec![
                    GroupedBtn::new("create", tr("chat_topics_create"))
                        .icon("plus")
                        .icon_background(ChatTileColor::blue())
                        .show_chevron(false)
                        .callback(move |_, cx| {
                            entity.update(cx, |this, cx| this.present_topic_editor(None, None, cx));
                        }),
                ],
            ));
        }
        if snapshot.can_manage_sections {
            let entity = cx.entity();
            content = content.child(GroupedBtns::new(
                SharedString::from(format!("chat-topics-create-section-{chat_id}")),
                vec![
                    GroupedBtn::new("create_section", tr("forum_create_section"))
                        .icon("folder.badge.plus")
                        .icon_background(ChatTileColor::indigo())
                        .show_chevron(false)
                        .callback(move |_, cx| {
                            entity.update(cx, |this, cx| this.present_create_section(cx));
                        }),
                ],
            ));
        }
        content
    }
}

impl Render for ChatTopicsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let resolved_chat_id = self.resolved_chat_id();
        let screen_id = format!("chat-topics-{resolved_chat_id}");
        let snapshot = ChatTopicsSnapshot::read(&resolved_chat_id, cx);
        let entity = cx.entity();
        let nav_store = env::<NavigationStoreKey>(cx);

        let interactions = ChatInteractionsStore::shared(cx);
        let task_chat_id = resolved_chat_id.clone();
        use_task(SharedString::from(format!("{screen_id}-task")), (), window, cx, async move |cx| {
            interactions.update(cx, |interactions, cx| interactions.load_forum_home(&task_chat_id, cx));
        });

        let revoked_entity = entity.clone();
        on_change(
            SharedString::from(format!("{screen_id}-revoked")),
            snapshot.reorder_right_revoked(),
            false,
            window,
            cx,
            move |_, revoked, _, cx| {
                let revoked = *revoked;
                revoked_entity.update(cx, |this, cx| this.reorder_right_changed(revoked, cx));
            },
        );
        on_change(
            SharedString::from(format!("{screen_id}-enabled")),
            snapshot.enabled,
            true,
            window,
            cx,
            |_, on, _, cx| GuideFacts::set(RiseGuideSignal::ChatTopicsEnabled, *on, cx),
        );

        let mut body = vstack(16.0).w_full();
        if snapshot.enabled {
            if self.is_reordering {
                body = body
                    .child(self.reorder_scope_picker(&snapshot, &theme, cx))
                    .child(self.reorder_list(&snapshot, &theme, cx));
            } else {
                body = body.child(Self::browse_list(&snapshot, &theme, cx));
                if snapshot.is_chat_owner {
                    body = body.child(self.hide_all_topics_card(&snapshot));
                }
                body = self.create_buttons(&snapshot, body, &resolved_chat_id, cx);
            }
        } else {
            body = body.child(self.enable_card(&theme));
        }
        let opacity = self.content_opacity.frame_value(RiseClock::monotonic(), window).clamp(0.0, 1.0);
        let content = vstack(16.0).w_full().pt(px(12.0)).child(body.opacity(opacity)).child(spacer_min_h(60.0));

        let scroll = scroll_view(SharedString::from(format!("{screen_id}-scroll")))
            .scroll_disabled(self.dragging_id.is_some())
            .scroll_handle(self.scroll_handle.clone())
            .list_shell_scroll_under_chrome(cx)
            .child(content);

        let section_alert = ForumSectionNameSheetHost::new(
            SharedString::from(format!("{screen_id}-section-name")),
            Binding::entity(
                entity.clone(),
                |this: &ChatTopicsScreen| this.section_name_target.clone(),
                |this, value, cx| {
                    this.section_name_target = value;
                    cx.notify();
                },
            ),
        );

        let topic_editor = TopicEditorSheet::new(
            Binding::entity(
                entity.clone(),
                |this: &ChatTopicsScreen| this.show_topic_editor,
                |this, value, _| this.show_topic_editor = value,
            ),
            resolved_chat_id,
        )
        .topic_id(self.editor_topic_id.clone())
        .section_id(self.editor_section_id.clone());

        let mut wrapper = SettingsWrapperUi::new(tr("chat_topics_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        });
        if snapshot.enabled && (snapshot.can_reorder() || self.is_reordering) {
            let toggle_entity = entity;
            wrapper = wrapper.right_jsx(ChatsReorderToggleButton::new(
                SharedString::from(format!("{screen_id}-reorder-toggle")),
                self.is_reordering,
                move |window, cx| toggle_entity.update(cx, |this, cx| this.toggle_reordering(window, cx)),
            ));
        }

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .child(
                wrapper.content(
                    div().size_full().bg(theme.bg._100).child(scroll).child(section_alert).child(topic_editor),
                ),
            )
            .on_disappear(SharedString::from(format!("{screen_id}-disappear")), |_, cx| {
                GuideFacts::set(RiseGuideSignal::ChatTopicsEnabled, false, cx);
            })
    }
}

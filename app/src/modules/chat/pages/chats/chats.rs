use std::collections::hash_map::DefaultHasher;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{
    App, BoxShadow, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, Pixels, Point, Rgba, SharedString, Size,
    Subscription, Task, Window, anchored, black, container_query, deferred, point, relative, white,
};
use rise_canon::{RiseGuideAnchor, RiseLimits};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_platform::system_share::ShareItem;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig, TabHeaderCustomEmoji};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::guide_anchor_ui::guide_anchor_registry::{GuideAnchorCoverKind, GuideAnchorCoverModifiers};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::{
    PageHeaderBackButton, PageHeaderIconButton, PageHeaderMetrics, PageHeaderToolbarSlot,
};
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::motion::{ease_in_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_theme::theme::theme_environment::theme_store;
use rise_theme::RiseThemeAppearance;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition, TransitionModifiers, UnitPoint};
use rise_ui::text_field_alert::TextFieldAlert;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::app::router::main_tab_chrome::{MAIN_TAB_BAR_HEIGHT, main_tab_bar_underlap};
use crate::app::router::navigation_store::{
    EmbeddedForumRequest, NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush, SaiActiveScreenOverlay,
};
use crate::app::router::navigation_types::{MainTab, RiseRTCRoomTarget, RootRoute};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::{
    SaiDataRender, SaiDataRenderDefaultsEmpty, SaiDataRenderDefaultsError,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::maintenance::maintenance_gate::{MaintenanceGate, MaintenanceSnapshot};
use crate::core::ui::animated_tabs_ui::animated_tabs_header::{AnimatedTabsHeader, AnimatedTabsTapTab};
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerLiveState};
use crate::core::ui::clever_image_disk_cache::{
    CleverImageDiskPrefetch, CleverImagePrefetchItem, CleverImagePrefetchPolicy,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::HoldContextMenuTapPresentationModifiers;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::core::widgets::wrappers::list_shell_header_collapse::{
    ListShellHeaderCollapse, ListShellHeaderCollapseContentLift,
    ListShellHeaderCollapseModifiers,
};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatWallpaperPrewarm;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperStore;
use crate::modules::chat::components::chat::chat_logo::{ChatLogo, ChatLogoImagePolicy};
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatShellInsetsReader;
use crate::modules::chat::components::chat::composer::chat_draft_store::{
    ChatDraftOrdering, ChatDraftPreview, ChatDraftStore,
};
use crate::modules::chat::components::chat_item::chat_item::{ChatItem, ChatsListRowActionsSuppressorKey};
use crate::modules::chat::components::chat_item::chat_item_skeleton::ChatListSkeleton;
use crate::modules::chat::components::chat_item::chat_list_preview_row::{ChatListPreviewFactory, ChatListRowMetrics};
use crate::modules::chat::components::chat_search::chat_search_overlay::{
    ChatSearchHero, ChatSearchHeroNamespace, ChatSearchOverlay,
};
use crate::modules::chat::components::chat_search::chat_search_overlay_presenter::ChatSearchOverlayPresenter;
use crate::modules::chat::components::chat_search::chats_search_context::{ChatsSearchContext, ChatsSearchContextKey};
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation, RiseConversationPresentationChange,
};
use crate::modules::chat::pages::chats::chat_list_history_warmup_controller::ChatListHistoryWarmupController;
use crate::modules::chat::pages::chats::chats_aside_hooks::chats_aside_hooks;
use crate::modules::chat::pages::chats::chats_archive_pull_gate::ChatsArchivePullEvents;
use crate::modules::chat::pages::chats::chats_archive_reveal::{
    ChatsArchiveRevealPhase, ChatsArchiveRevealPolicy, ChatsArchiveSettleTarget,
};
use crate::modules::chat::pages::chats::chats_archive_reveal_row::ChatsArchiveRevealRow;
use crate::modules::chat::pages::chats::chats_menu_glyph::{ChatsMenuGlassSchemePin, ChatsMenuGlyph};
use crate::modules::chat::pages::chats::chats_page_viewport_store::ChatsPageViewportStore;
use crate::modules::chat::pages::chats::chats_reorder_toggle_button::ChatsReorderToggleButton;
use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumPresentationSessionPolicy, EmbeddedForumPresentationStore, EmbeddedForumProgressReader,
    EmbeddedForumRailContext, EmbeddedForumRailContextKey, EmbeddedForumRailFade, EmbeddedForumTransitionLayer,
    EmbeddedForumTransitionMetrics, EmbeddedForumTransitionState,
};
use crate::modules::chat::pages::chats::forum_members_subtitle_hooks::ForumMembersSubtitle;
use crate::modules::chat::pages::chats::rise_chat_native_list::{
    RiseChatNativeList, RiseChatNativeListControl, RiseChatNativeListRow,
};
use crate::modules::chat::engine::chats::rise_chat_viewport_repository::RiseChatViewportSnapshot;
use crate::modules::chat::pages::chats::rise_chat_viewport::{RiseChatViewport, RiseChatViewportReader};
use crate::modules::chat::shared::utils::chat_list_utils::ChatListUtils;
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::{ChatActionCallback, ChatActionsStore};
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatTopic};
use crate::modules::chat::stores::chats::chat_interactions::chat_archive_interactions::ChatArchiveOnError;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::{
    ChatInteractionsDone, ChatInteractionsStore,
};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_roles_services::ForumTopicListSnapshot;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::CreateChatSheetState;
use crate::modules::presence::stores::presence::presence_interactions::presence_interactions::PresenceInteractionsStore;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;
use crate::modules::user::components::premium_badge_picker::premium_badge_picker_popover::{
    PremiumBadgePickerPopover, PremiumBadgePopoverLayout,
};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use rise_chat::ChatType;

pub use crate::modules::story::stores::story::story_services::chats_header_status_hooks::ChatsHeaderStatus;

const FIND_KEY: &str = "f";
const EMBEDDED_FORUM_CLOSE_CLEANUP_DELAY: f64 = 0.48;

pub enum ChatListAvatarPrefetchPolicy {}

impl ChatListAvatarPrefetchPolicy {
    pub fn lookahead() -> i64 {
        RiseLimits::client_value("chat_list_avatar_prefetch_lookahead")
    }

    pub fn candidate_indices(item_count: i64, center: i64, previous_center: Option<i64>) -> Vec<i64> {
        if item_count <= 0 || center < 0 || center >= item_count {
            return Vec::new();
        }
        let lookahead = Self::lookahead();
        let moves_toward_newer = previous_center.is_none_or(|previous| center >= previous);
        let (lower, upper) = if previous_center.is_none() {
            ((center - 2).max(0), item_count.min(center + lookahead + 1))
        } else if moves_toward_newer {
            (center, item_count.min(center + lookahead + 1))
        } else {
            ((center - lookahead).max(0), item_count.min(center + 1))
        };
        (lower..upper).collect()
    }
}

pub enum ChatsArchiveScrollResetPolicy {}

impl ChatsArchiveScrollResetPolicy {
    /// interaction locks must not move the mounted list
    /// eg forum presentation or search or hold menus or reordering
    /// normalize only when the archive affordance itself is displaced
    pub fn should_normalize_to_normal_top(phase: ChatsArchiveRevealPhase, marker_y: f32) -> bool {
        phase != ChatsArchiveRevealPhase::Hidden || marker_y > ChatsArchiveRevealPolicy::TOP_TOLERANCE
    }
}

struct ChatListAvatarPrefetchController {
    scope: String,
    active_page_id: Option<String>,
    center: Option<i64>,
    generation: u64,
    preview_preparation: Option<Task<()>>,
}

impl ChatListAvatarPrefetchController {
    fn new() -> Self {
        Self {
            scope: format!("chats-avatars:{}", Uuid::new_v4().uuid_string()),
            active_page_id: None,
            center: None,
            generation: 0,
            preview_preparation: None,
        }
    }

    fn activate(&mut self, page_id: Option<&str>, cx: &mut App) {
        if self.active_page_id.as_deref() == page_id {
            return;
        }
        self.cancel_all(cx);
        self.active_page_id = page_id.map(str::to_owned);
    }

    fn update(&mut self, page_id: &str, visible_index: i64, chats: &[ChatInfo], cx: &mut App) {
        if !CleverImagePrefetchPolicy::allows_prefetch_now(cx) {
            self.cancel_all(cx);
            return;
        }
        self.activate(Some(page_id), cx);
        if let Some(center) = self.center
            && (center - visible_index).abs() < 3
        {
            return;
        }
        let indices =
            ChatListAvatarPrefetchPolicy::candidate_indices(chats.len() as i64, visible_index, self.center);
        self.center = Some(visible_index);
        self.generation = self.generation.wrapping_add(1);
        let messages = indices
            .iter()
            .filter_map(|index| chats[*index as usize].last_message.clone())
            .collect::<Vec<_>>();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let (preview_user_id, preview_epoch) = (account_user_id.clone(), account_epoch.clone());
        self.preview_preparation = Some(cx.spawn(async move |cx| {
            if !SaiAccountCacheScope::is_current_account_epoch(preview_user_id.as_deref(), preview_epoch.as_deref()) {
                return;
            }
            MessagePreview::prepare_list_snippets(&messages, cx).await;
        }));

        let mut seen = HashSet::new();
        let mut items = Vec::with_capacity(indices.len() * 2);
        let mut append = |item: CleverImagePrefetchItem| {
            if item.cache_policy != CleverImageCachePolicy::PrivateEphemeral
                && item.url_string.to_lowercase().starts_with("http")
                && seen.insert(item.clone())
            {
                items.push(item);
            }
        };
        for index in &indices {
            let chat = &chats[*index as usize];
            if let Some(image) = ChatLogoImagePolicy::descriptor(chat, ChatListRowMetrics::AVATAR_SIZE, cx) {
                append(image);
            }
            if let Some(thumbnail) = ChatListPreviewFactory::media_thumbnail_source(chat.last_message.as_ref()) {
                let mut item = CleverImagePrefetchItem::new(thumbnail);
                item.maximum_pixel_dimension = Some(ChatListRowMetrics::THUMBNAIL_DECODE_PIXEL_DIMENSION);
                append(item);
            }
        }
        CleverImageDiskPrefetch::replace_directional(
            items,
            &self.scope,
            self.generation,
            account_user_id,
            account_epoch,
            cx,
        );
    }

    fn cancel_all(&mut self, cx: &mut App) {
        self.generation = self.generation.wrapping_add(1);
        self.preview_preparation = None;
        CleverImageDiskPrefetch::cancel_directional(&self.scope, self.generation, cx);
        self.active_page_id = None;
        self.center = None;
    }
}

pub struct ChatsScreenState {
    pub query: String,
    pub active_tab: i64,
    pub search_overlay: bool,
    pub chat_row_local: HashMap<String, ChatsListRowLocalState>,
    pub deleted_chat_ids: HashSet<String>,
    pub pending_delete_chat: Option<ChatInfo>,
    pub is_reorder_mode: bool,
    pub selected_for_reorder: HashSet<String>,
    pub selected_chat_info: HashMap<String, ChatInfo>,
    pub selected_order: Vec<String>,
    pub pending_chat_open_id: Option<String>,
    pub archive_reveal_phase: ChatsArchiveRevealPhase,
    pub archive_marker_y: f32,
    pub archive_pull_eligible: bool,
    pub archive_gesture_active: bool,
    pub archive_gesture_eligibility_resolved: bool,
    pub archive_gesture_start_marker_y: f32,
    pub archive_initial_positioned: bool,
    pub archive_settling: bool,
    archive_settle_generation: u64,
    archive_settle_task: Option<Task<()>>,
}

impl Default for ChatsScreenState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatsScreenState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            active_tab: 0,
            search_overlay: false,
            chat_row_local: HashMap::new(),
            deleted_chat_ids: HashSet::new(),
            pending_delete_chat: None,
            is_reorder_mode: false,
            selected_for_reorder: HashSet::new(),
            selected_chat_info: HashMap::new(),
            selected_order: Vec::new(),
            pending_chat_open_id: None,
            archive_reveal_phase: ChatsArchiveRevealPhase::Hidden,
            archive_marker_y: 0.0,
            archive_pull_eligible: false,
            archive_gesture_active: false,
            archive_gesture_eligibility_resolved: false,
            archive_gesture_start_marker_y: 0.0,
            archive_initial_positioned: false,
            archive_settling: false,
            archive_settle_generation: 0,
            archive_settle_task: None,
        }
    }

    pub fn exit_reorder_mode(&mut self) -> bool {
        if !self.is_reorder_mode && self.selected_for_reorder.is_empty() {
            return false;
        }
        self.is_reorder_mode = false;
        self.selected_for_reorder.clear();
        self.selected_chat_info.clear();
        self.selected_order.clear();
        true
    }

    pub fn reset_archive_reveal(&mut self) {
        self.archive_reveal_phase = ChatsArchiveRevealPhase::Hidden;
        self.archive_marker_y = 0.0;
        self.archive_pull_eligible = false;
        self.archive_gesture_active = false;
        self.archive_gesture_eligibility_resolved = false;
        self.archive_gesture_start_marker_y = 0.0;
        self.archive_initial_positioned = false;
        self.cancel_archive_settle();
    }

    /// holds rest normalization off while the programmatic settle still moves the list
    /// so the two never scroll it in the same frame, then commits the destination phase on arrival
    pub fn begin_archive_settle(&mut self, final_phase: ChatsArchiveRevealPhase, cx: &mut Context<Self>) {
        self.archive_settling = true;
        self.archive_settle_generation = self.archive_settle_generation.wrapping_add(1);
        let generation = self.archive_settle_generation;
        self.archive_settle_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(ChatsArchiveRevealPolicy::SETTLE_DURATION))
                .await;
            let _ = this.update(cx, |state, cx| {
                if state.archive_settle_generation != generation {
                    return;
                }
                state.archive_settling = false;
                if state.archive_reveal_phase != final_phase {
                    state.archive_reveal_phase = final_phase;
                }
                cx.notify();
            });
        }));
    }

    /// a new touch stops the settle animation so its deferred phase must not land on top of the gesture
    pub fn cancel_archive_settle(&mut self) {
        self.archive_settling = false;
        self.archive_settle_generation = self.archive_settle_generation.wrapping_add(1);
        self.archive_settle_task = None;
    }

    pub fn reset_account_scoped_row_state(&mut self) {
        self.chat_row_local.clear();
        self.deleted_chat_ids.clear();
        self.pending_delete_chat = None;
        self.pending_chat_open_id = None;
        let _ = self.exit_reorder_mode();
        self.reset_archive_reveal();
    }

    pub fn display_chat(this: &Entity<Self>, base: &ChatInfo, cx: &mut App) -> ChatInfo {
        let mut chat = base.clone();
        let mute_all = UserServicesStore::shared(cx).read(cx).mute_all_chats_enabled();
        if mute_all {
            let services = ChatServicesStore::shared(cx);
            chat.is_muted = services.read(cx).effective_is_muted(base, cx);
        }
        if let Some(local) = this.read(cx).chat_row_local.get(&base.id) {
            if let Some(pinned) = local.pinned {
                chat.is_pinned = pinned;
            }
            if local.unread_cleared {
                chat.unread_count = 0;
                chat.unread_mentions_count = 0;
                chat.unread_replies_count = 0;
                chat.unread_reactions_count = 0;
            }
            if local.unread_marked {
                chat.unread_count = chat.unread_count.max(1);
            }
        }
        chat
    }

    pub fn visible_chats(this: &Entity<Self>, filter_query: &str, source: &[ChatInfo], cx: &mut App) -> Vec<ChatInfo> {
        let q = trimming_whitespaces_and_newlines(filter_query).to_lowercase();
        let tag_query = q.trim_start_matches('@').to_owned();
        let deleting = ChatServicesStore::shared(cx).read(cx).pending_delete_chat_ids.clone();
        let deleted = this.read(cx).deleted_chat_ids.clone();
        let mut visible = Vec::with_capacity(source.len());
        for chat in source {
            if deleted.contains(&chat.id) || deleting.contains(&chat.id) {
                continue;
            }
            if !q.is_empty() {
                let is_direct = chat.r#type == ChatType::Direct;
                let matches = ChatListPreviewFactory::chat_title(chat, cx).to_lowercase().contains(&q)
                    || (is_direct && chat.participant.name.to_lowercase().contains(&q))
                    || (is_direct
                        && !tag_query.is_empty()
                        && chat.participant.tag.to_lowercase().trim_start_matches('@').contains(&tag_query))
                    || chat.last_message.as_ref().is_some_and(|message| message.text.to_lowercase().contains(&q));
                if !matches {
                    continue;
                }
            }
            visible.push(Self::display_chat(this, chat, cx));
        }
        visible
    }

    /// folder_id names the folder page the row was held on, none is all chats
    pub fn apply_hold_action(
        this: &Entity<Self>,
        chat_id: &str,
        action_id: &str,
        source: &[ChatInfo],
        folder_id: Option<&str>,
        cx: &mut App,
    ) {
        let Some(base) = source.iter().find(|chat| chat.id == chat_id) else {
            return;
        };
        if action_id == "select" {
            let is_full = this.read(cx).selected_for_reorder.len() as i64 >= ChatsSelectionChrome::selection_limit();
            let selectable = Self::is_selectable(base);
            this.update(cx, |state, cx| {
                state.is_reorder_mode = true;
                if selectable && !is_full {
                    state.note_selected(base);
                }
                cx.notify();
            });
            if selectable && is_full {
                Self::announce_selection_limit(cx);
            }
            return;
        }

        let current = Self::display_chat(this, base, cx);
        let mut row = this.read(cx).chat_row_local.get(chat_id).cloned().unwrap_or_default();
        let (unread_state, unread_chat_id) = (this.downgrade(), chat_id.to_owned());
        let (read_state, read_chat_id) = (this.downgrade(), chat_id.to_owned());
        let result = ChatListCommonHoldActionHandler::apply(
            action_id,
            base,
            &current,
            &mut row,
            move |cx: &mut App| {
                let _ = unread_state.update(cx, |state, cx| {
                    if let Some(local) = state.chat_row_local.get_mut(&unread_chat_id) {
                        local.unread_marked = false;
                        cx.notify();
                    }
                });
            },
            move |cx: &mut App| {
                let _ = read_state.update(cx, |state, cx| {
                    if let Some(local) = state.chat_row_local.get_mut(&read_chat_id) {
                        local.unread_cleared = false;
                        cx.notify();
                    }
                });
            },
            cx,
        );
        match result {
            ChatListCommonHoldActionResult::Handled => return,
            ChatListCommonHoldActionResult::UpdateLocalState => {
                this.update(cx, |state, cx| {
                    state.chat_row_local.insert(chat_id.to_owned(), row);
                    cx.notify();
                });
                return;
            }
            ChatListCommonHoldActionResult::NotHandled => {}
        }

        match action_id {
            "pin" => {
                let (chat_id, folder_id) = (chat_id.to_owned(), folder_id.map(str::to_owned));
                cx.defer(move |cx| {
                    ChatInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.toggle_pin_chat(&chat_id, folder_id.as_deref(), cx));
                });
            }
            "reorder" => {
                this.update(cx, |state, cx| {
                    state.is_reorder_mode = true;
                    cx.notify();
                });
            }
            "archive" => {
                this.update(cx, |state, cx| {
                    state.chat_row_local.remove(chat_id);
                    cx.notify();
                });
                let on_error: ChatArchiveOnError = Rc::new(|cx: &mut App| {
                    Notifier::send(
                        tr("chat_archive_action_failed_title"),
                        NotifierSendOptions {
                            content: Some(tr("chat_archive_action_failed")),
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                });
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.archive_chat(&current, Some(on_error), cx));
            }
            "leave" => {
                HoldContextMenuPresenter::dismiss(cx);
                let (state, removed_chat_id) = (this.downgrade(), chat_id.to_owned());
                let on_confirm: ChatInteractionsDone = Box::new(move |cx: &mut App| {
                    let _ = state.update(cx, |state, cx| {
                        state.deleted_chat_ids.insert(removed_chat_id.clone());
                        state.chat_row_local.remove(&removed_chat_id);
                        cx.notify();
                    });
                });
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.confirm_leave_chat(&current, Some(on_confirm), None, cx));
            }
            "history_clear" => {
                HoldContextMenuPresenter::dismiss(cx);
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.confirm_clear_history(&current, cx));
            }
            "delete" => {
                HoldContextMenuPresenter::dismiss(cx);
                this.update(cx, |state, cx| {
                    state.pending_delete_chat = Some(current);
                    cx.notify();
                });
            }
            _ => {}
        }
    }

    /// a tap or hold past the selection limit marks nothing and says why
    pub fn announce_selection_limit(cx: &mut App) {
        let _ = impact(ImpactStyle::Light);
        Notifier::send(
            tr_args("chats_selection_limit", &[FormatArg::Int(ChatsSelectionChrome::selection_limit())]),
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    /// saved messages cant be deleted and contact join rows are not chats
    /// so neither takes part in a bulk action
    pub fn is_selectable(chat: &ChatInfo) -> bool {
        chat.r#type != ChatType::Favourites && !UserServicesStore::is_contact_join_virtual_chat_id(&chat.id)
    }

    pub fn note_selected(&mut self, chat: &ChatInfo) {
        self.selected_for_reorder.insert(chat.id.clone());
        self.selected_chat_info.insert(chat.id.clone(), chat.clone());
        if !self.selected_order.iter().any(|id| *id == chat.id) {
            self.selected_order.push(chat.id.clone());
        }
    }

    /// selected chats in pick order, current row first and the picked one if the list no longer holds it
    fn selected_chats(&self, source: &[ChatInfo]) -> Vec<ChatInfo> {
        if self.selected_for_reorder.is_empty() {
            return Vec::new();
        }
        let mut by_id: HashMap<&str, &ChatInfo> = HashMap::with_capacity(source.len());
        for chat in source {
            by_id.entry(chat.id.as_str()).or_insert(chat);
        }
        self.selected_order
            .iter()
            .filter(|id| self.selected_for_reorder.contains(*id))
            .filter_map(|id| by_id.get(id.as_str()).copied().or_else(|| self.selected_chat_info.get(id)))
            .filter(|chat| Self::is_selectable(chat))
            .cloned()
            .collect()
    }

    /// every bulk action leaves selection mode at once, the tab bar comes back while the calls run
    fn end_selection(this: &Entity<Self>, cx: &mut App) {
        this.update(cx, |state, cx| {
            if state.exit_reorder_mode() {
                cx.notify();
            }
        });
    }

    pub fn mark_selected_read(this: &Entity<Self>, source: &[ChatInfo], cx: &mut App) {
        let chats = this.read(cx).selected_chats(source);
        Self::end_selection(this, cx);
        for chat in &chats {
            Self::apply_hold_action(this, &chat.id, "mark_read", &chats, None, cx);
        }
    }

    pub fn archive_selected(this: &Entity<Self>, source: &[ChatInfo], cx: &mut App) {
        let chats = this.read(cx).selected_chats(source);
        Self::end_selection(this, cx);
        for chat in &chats {
            Self::apply_hold_action(this, &chat.id, "archive", &chats, None, cx);
        }
    }

    pub fn delete_selected(this: &Entity<Self>, source: &[ChatInfo], cx: &mut App) {
        let selected = this.read(cx).selected_chats(source);
        let chats = selected.iter().map(|chat| Self::display_chat(this, chat, cx)).collect::<Vec<_>>();
        if chats.is_empty() {
            Self::end_selection(this, cx);
            return;
        }
        let title = tr_args("chats_batch_delete_title", &[FormatArg::Int(chats.len() as i64)]);
        let state = this.downgrade();
        NativeConfirmAlert::present_delete(
            &title,
            &tr("chats_batch_delete_body"),
            None,
            None,
            false,
            None,
            None,
            move |cx: &mut App| {
                if let Some(this) = state.upgrade() {
                    Self::end_selection(&this, cx);
                }
                let removed_state = state.clone();
                let on_removed: Rc<dyn Fn(&str, &mut App)> = Rc::new(move |chat_id: &str, cx: &mut App| {
                    let _ = removed_state.update(cx, |state, cx| {
                        state.deleted_chat_ids.insert(chat_id.to_owned());
                        state.chat_row_local.remove(chat_id);
                        cx.notify();
                    });
                });
                ChatsBulkDeleteRun::new(chats, on_removed).start(cx);
            },
            cx,
        );
    }
}

struct ChatsBulkDeleteRun {
    queue: RefCell<VecDeque<ChatInfo>>,
    on_removed: Rc<dyn Fn(&str, &mut App)>,
    removed: Cell<i64>,
    failure_shown: Cell<bool>,
}

impl ChatsBulkDeleteRun {
    const ALERT_GAP: Duration = Duration::from_millis(350);

    fn new(chats: Vec<ChatInfo>, on_removed: Rc<dyn Fn(&str, &mut App)>) -> Rc<Self> {
        Rc::new(Self {
            queue: RefCell::new(chats.into()),
            on_removed,
            removed: Cell::new(0),
            failure_shown: Cell::new(false),
        })
    }

    fn start(self: &Rc<Self>, cx: &mut App) {
        self.next(true, cx);
    }

    fn next(self: &Rc<Self>, after_alert: bool, cx: &mut App) {
        loop {
            let Some(chat) = self.queue.borrow_mut().pop_front() else {
                break;
            };
            let community = matches!(chat.r#type, ChatType::Group | ChatType::Channel);
            if community && ChatProfilePermissions::is_owner(&chat) {
                self.ask(chat, after_alert, cx);
                return;
            }
            if community {
                ChatActionsStore::shared(cx).update(cx, |store, cx| store.leave_chat_action(&chat.id, cx));
                self.note_removed(&chat.id, true, cx);
            } else {
                self.delete(&chat, false, cx);
                self.note_removed(&chat.id, false, cx);
            }
        }
        self.finish(cx);
    }

    fn ask(self: &Rc<Self>, chat: ChatInfo, after_alert: bool, cx: &mut App) {
        let run = self.clone();
        cx.spawn(async move |cx| {
            if after_alert {
                cx.background_executor().timer(Self::ALERT_GAP).await;
            }
            let _ = cx.update(|cx| run.present_owner_question(chat, cx));
        })
        .detach();
    }

    fn present_owner_question(self: &Rc<Self>, chat: ChatInfo, cx: &mut App) {
        let title = ChatInteractionsStore::chat_delete_dialog_title(&chat);
        let body = ChatInteractionsStore::chat_delete_dialog_body(&chat);
        let (cancel_run, confirm_run) = (self.clone(), self.clone());
        let on_cancel: NativeConfirmAlertAction = Box::new(move |cx: &mut App| cancel_run.next(true, cx));
        NativeConfirmAlert::present_delete(
            &title,
            &body,
            Some(tr("chat_delete_for_everyone")),
            None,
            false,
            None,
            Some(on_cancel),
            move |cx: &mut App| {
                confirm_run.delete(&chat, true, cx);
                confirm_run.note_removed(&chat.id, false, cx);
                confirm_run.next(true, cx);
            },
            cx,
        );
    }

    fn delete(self: &Rc<Self>, chat: &ChatInfo, for_everyone: bool, cx: &mut App) {
        let run = self.clone();
        let on_error: ChatActionCallback = Rc::new(move |cx: &mut App| run.show_failure_once(cx));
        ChatActionsStore::shared(cx).update(cx, |store, cx| {
            let pending = store.begin_chat_delete_action(&chat.id, Some(chat.clone()), false, cx);
            store.delete_chat_action(&chat.id, for_everyone, false, Some(pending), Some(on_error), cx);
        });
    }

    /// a left chat is hidden by the screen like a single leave
    /// a deleted one is hidden by the pending delete and returns on its own if the server refuses
    fn note_removed(&self, chat_id: &str, hides_row: bool, cx: &mut App) {
        self.removed.set(self.removed.get() + 1);
        if hides_row {
            (self.on_removed)(chat_id, cx);
        }
    }

    fn show_failure_once(&self, cx: &mut App) {
        if self.failure_shown.get() {
            return;
        }
        self.failure_shown.set(true);
        Notifier::send(
            tr("chat_delete_failed_title"),
            NotifierSendOptions {
                content: Some(tr("chat_delete_failed_text")),
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn finish(&self, cx: &mut App) {
        let removed = self.removed.get();
        if removed > 0 {
            Notifier::send(
                tr_args("chats_batch_deleted", &[FormatArg::Int(removed)]),
                NotifierSendOptions {
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Bottom,
                    icon: NotifierIcon::System("trash".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatsListRowLocalState {
    pub pinned: Option<bool>,
    pub unread_cleared: bool,
    pub unread_marked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatListCommonHoldActionResult {
    NotHandled,
    Handled,
    UpdateLocalState,
}

pub enum ChatListCommonHoldActionHandler {}

impl ChatListCommonHoldActionHandler {
    pub fn apply(
        action_id: &str,
        base: &ChatInfo,
        displayed: &ChatInfo,
        local_state: &mut ChatsListRowLocalState,
        on_unread_mark_settled: impl Fn(&mut App) + 'static,
        on_read_settled: impl Fn(&mut App) + 'static,
        cx: &mut App,
    ) -> ChatListCommonHoldActionResult {
        let chat_id = base.id.clone();
        let set_auto_delete = |seconds: i64, chat_id: String, cx: &mut App| {
            cx.defer(move |cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.set_auto_delete(&chat_id, seconds, cx));
            });
        };
        match action_id {
            "mute" => {
                if UserServicesStore::shared(cx).read(cx).mute_all_chats_enabled() {
                    let services = ChatServicesStore::shared(cx);
                    let was_muted = services.read(cx).effective_is_muted(base, cx);
                    cx.defer(move |cx| {
                        UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                            if was_muted {
                                store.add_chat_mute_exception(&chat_id, cx);
                            } else {
                                store.remove_chat_mute_exception(&chat_id, cx);
                            }
                        });
                    });
                    return ChatListCommonHoldActionResult::Handled;
                }

                let was_muted = displayed.is_muted;
                cx.defer(move |cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                        if was_muted {
                            store.unmute_chat(&chat_id, cx);
                        } else {
                            store.mute_chat(&chat_id, None, cx);
                        }
                    });
                });
                ChatListCommonHoldActionResult::Handled
            }
            "autodelete_off" => {
                set_auto_delete(0, chat_id, cx);
                ChatListCommonHoldActionResult::Handled
            }
            "autodelete_1d" => {
                set_auto_delete(86_400, chat_id, cx);
                ChatListCommonHoldActionResult::Handled
            }
            "autodelete_1w" => {
                set_auto_delete(604_800, chat_id, cx);
                ChatListCommonHoldActionResult::Handled
            }
            "autodelete_1m" => {
                set_auto_delete(2_592_000, chat_id, cx);
                ChatListCommonHoldActionResult::Handled
            }
            "mark_read" => {
                local_state.unread_cleared = true;
                local_state.unread_marked = false;
                let read = MessageInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.mark_chat_as_read(&chat_id, None, true, cx));
                cx.spawn(async move |cx| {
                    if let Some(read) = read {
                        read.await;
                    }
                    cx.update(on_read_settled);
                })
                .detach();
                ChatListCommonHoldActionResult::UpdateLocalState
            }
            "mark_unread" => {
                local_state.unread_cleared = false;
                local_state.unread_marked = true;
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.set_chat_unread_mark(&chat_id, true, Some(Rc::new(on_unread_mark_settled)), cx);
                });
                ChatListCommonHoldActionResult::UpdateLocalState
            }
            _ => ChatListCommonHoldActionResult::NotHandled,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ChatsPinnedReorderStateRowKey {
    page_id: String,
    chat_id: String,
}

struct ChatsPinnedReorderState {
    dragging_chat_id: Option<String>,
    page_id: Option<String>,
    ordered_ids: Vec<String>,
    base_order: Vec<String>,
    control: Option<Weak<RiseChatNativeListControl>>,
    row_heights: HashMap<ChatsPinnedReorderStateRowKey, f32>,
    page_pinned_ids: HashSet<String>,
}

impl ChatsPinnedReorderState {
    const DROP_GLIDE: Duration = Duration::from_millis(300);

    fn new() -> Self {
        Self {
            dragging_chat_id: None,
            page_id: None,
            ordered_ids: Vec::new(),
            base_order: Vec::new(),
            control: None,
            row_heights: HashMap::new(),
            page_pinned_ids: HashSet::new(),
        }
    }

    fn row_key(page_id: &str, chat_id: &str) -> ChatsPinnedReorderStateRowKey {
        ChatsPinnedReorderStateRowKey { page_id: page_id.to_owned(), chat_id: chat_id.to_owned() }
    }

    fn measure(&mut self, page_id: &str, chat_id: &str, height: f32) {
        if !height.is_finite() || height <= 0.0 {
            return;
        }
        self.row_heights.insert(Self::row_key(page_id, chat_id), height);
    }

    /// measured height of a pinned row, else its frame in the list without the separator
    fn row_height(&self, page_id: &str, chat_id: &str, control: &RiseChatNativeListControl, cx: &App) -> Option<f32> {
        if let Some(height) = self.row_heights.get(&Self::row_key(page_id, chat_id)).copied() {
            return Some(height);
        }
        let frame = control.list()?.read(cx).viewport_frame(chat_id)?;
        let height = f32::from(frame.size.height) - ChatListRowMetrics::SEPARATOR_HEIGHT;
        (height.is_finite() && height > 0.0).then_some(height)
    }

    fn folder_id(from_page_id: &str) -> Option<String> {
        let id = from_page_id.strip_prefix("folder:")?;
        (!id.is_empty()).then(|| id.to_owned())
    }

    fn control(&self) -> Option<Rc<RiseChatNativeListControl>> {
        self.control.as_ref().and_then(Weak::upgrade)
    }

    /// rows never change places in the native list while the pointer is down
    /// the dragged row follows the pointer and neighbours slide by offsets
    /// the new order lands once
    fn update(
        &mut self,
        chat_id: &str,
        page_id: &str,
        translation_y: f32,
        pinned_ids: &[String],
        control: &Rc<RiseChatNativeListControl>,
        cx: &mut Context<Self>,
    ) {
        if self.dragging_chat_id.as_deref() != Some(chat_id) || self.page_id.as_deref() != Some(page_id) {
            tracing::warn!(target: "riseonly::X2051", chat_id, page_id, translation_y, "reorder drag start");
            if self.row_height(page_id, chat_id, control, cx).is_none() {
                tracing::warn!(target: "riseonly::X2051", chat_id, page_id, "reorder refused: no row height");
                return;
            }
            let services = ChatServicesStore::shared(cx);
            let stored = match Self::folder_id(page_id) {
                Some(folder_id) => {
                    services.read(cx).folder(&folder_id, cx).map(|folder| folder.pinned_chat_ids).unwrap_or_default()
                }
                None => services.read(cx).pinned_chat_ids_in_order(cx),
            };
            if !stored.iter().any(|id| id == chat_id) {
                tracing::warn!(target: "riseonly::X2051", chat_id, ?stored, "reorder refused: not in the stored pinned order");
                return;
            }
            if let Some(previous) = self.control() {
                previous.clear_row_shifts(cx);
            }
            self.page_pinned_ids = pinned_ids.iter().cloned().collect();
            self.base_order = stored.iter().filter(|id| self.page_pinned_ids.contains(*id)).cloned().collect();
            self.ordered_ids = stored;
            self.page_id = Some(page_id.to_owned());
            self.control = Some(Rc::downgrade(control));
            self.dragging_chat_id = Some(chat_id.to_owned());
            let fallback = self.row_height(page_id, chat_id, control, cx).unwrap_or_default();
            let strides: HashMap<String, f32> = self
                .base_order
                .iter()
                .map(|id| {
                    let height = self.row_height(page_id, id, control, cx).unwrap_or(fallback);
                    (id.clone(), height + ChatListRowMetrics::SEPARATOR_HEIGHT)
                })
                .collect();
            control.begin_reorder(self.base_order.clone(), strides, cx);
            control.lift_row(Some(chat_id), cx);
            cx.notify();
        }
        if control.drag_reorder(chat_id, translation_y, cx) {
            tracing::warn!(target: "riseonly::X2051", chat_id, translation_y, "reorder slot changed");
        }
    }

    /// glides the dragged row into its slot then commits the order to save
    /// nothing is committed when nothing moved
    fn finish(&mut self, commit: impl FnOnce(Vec<String>, Option<String>, &mut App) + 'static, cx: &mut Context<Self>) {
        let (Some(_), Some(control)) = (self.dragging_chat_id.clone(), self.control()) else {
            self.reset(cx);
            cx.notify();
            return;
        };
        let page_order = control.end_reorder(cx).unwrap_or_default();
        let moved = !page_order.is_empty() && page_order != self.base_order;
        let mut next_page = page_order.into_iter();
        let ids: Vec<String> = std::mem::take(&mut self.ordered_ids)
            .into_iter()
            .map(|id| if self.page_pinned_ids.contains(&id) { next_page.next().unwrap_or(id) } else { id })
            .collect();
        let folder_id = self.page_id.as_deref().and_then(Self::folder_id);
        self.dragging_chat_id = None;
        self.page_id = None;
        self.control = None;
        cx.notify();
        if !moved {
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(Self::DROP_GLIDE).await;
                let _ = cx.update(|cx| control.clear_row_shifts(cx));
            })
            .detach();
            return;
        }
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Self::DROP_GLIDE).await;
            let _ = cx.update(|cx| {
                control.settle_row_shifts(ids.clone(), cx);
                commit(ids, folder_id, cx);
            });
        })
        .detach();
    }

    fn reset(&mut self, cx: &mut App) {
        if let Some(control) = self.control() {
            control.clear_row_shifts(cx);
        }
        self.control = None;
        self.dragging_chat_id = None;
        self.page_id = None;
        self.ordered_ids.clear();
        self.base_order.clear();
        self.page_pinned_ids.clear();
    }

    fn clear_geometry(&mut self) {
        self.row_heights.clear();
    }
}

#[derive(IntoElement)]
struct ChatsPinnedReorderMotion {
    state: Entity<ChatsPinnedReorderState>,
    page_id: String,
    chat_id: String,
    measures_height: bool,
    content: AnyElement,
}

impl RenderOnce for ChatsPinnedReorderMotion {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let probe = self.measures_height.then(|| {
            let (state, page_id, chat_id) = (self.state.clone(), self.page_id.clone(), self.chat_id.clone());
            let id = SharedString::from(format!("chats-reorder-measure-{}-{}", self.page_id, self.chat_id));
            measure(id, move |bounds, _, cx| {
                state.update(cx, |state, _| state.measure(&page_id, &chat_id, f32::from(bounds.size.height)));
            })
        });
        div().relative().w_full().child(self.content).children(probe)
    }
}

/// state of the chats page, what it draws from the active conversation engine
/// reread when a store it comes from changes
/// no render of the page reads the presentation
#[derive(PartialEq)]
struct ChatsEngineLists {
    source: Rc<Vec<ChatInfo>>,
    requests_count: i64,
    inbox_load_state: Option<RiseEntityViewLoadState>,
    inbox_render_status: SaiWsSliceStatus,
    inbox_render_error: Option<String>,
    archived_chats: Rc<Vec<ChatInfo>>,
    archived_count: i64,
    archived_unread_chats_count: i64,
}

impl ChatsEngineLists {
    /// the presented account, and the lists while the engine is active
    fn read(cx: &mut App) -> (Option<String>, Option<Self>) {
        let presentation = RiseConversationPresentation::shared(cx);
        let (presented_account, is_active) = {
            let presentation = presentation.read(cx);
            (presentation.account_id().map(str::to_owned), presentation.is_active())
        };
        if !is_active {
            return (presented_account, None);
        }
        let services = ChatServicesStore::shared(cx);
        let source = Rc::new(services.read(cx).chat_list_rows(cx));
        let (requests_state_count, inbox_load_state, inbox_render_status, inbox_render_error) = {
            let presentation = presentation.read(cx);
            (
                presentation.chat_state(RiseChatListKind::Requests).map(ChatServicesStore::message_requests_badge_count),
                presentation.chat_state(RiseChatListKind::Inbox).map(|state| state.load_state),
                presentation.chat_render_status(RiseChatListKind::Inbox),
                presentation.chat_render_error(RiseChatListKind::Inbox),
            )
        };
        let (requests_count, archived_chats, archived_count, archived_unread_chats_count) =
            services.update(cx, |services, cx| {
                let requests_count = match requests_state_count {
                    Some(count) => count,
                    None => services.message_requests(cx).len() as i64,
                };
                (
                    requests_count,
                    services.archived_chats(cx),
                    services.archived_chats_total_count(cx),
                    services.archived_chats_unread_chats_count(cx),
                )
            });
        let lists = Self {
            source,
            requests_count,
            inbox_load_state,
            inbox_render_status,
            inbox_render_error,
            archived_chats: Rc::new(archived_chats),
            archived_count,
            archived_unread_chats_count,
        };
        (presented_account, Some(lists))
    }
}

struct ChatsViewState {
    state: Entity<ChatsScreenState>,
    reorder: Entity<ChatsPinnedReorderState>,
    page_width: f32,
    tab_pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    page_viewports: Rc<ChatsPageViewportStore>,
    retained_tab_page_ids: HashSet<String>,
    performance_trace_revision: u64,
    is_chats_menu_presented: bool,
    hold_menu_active: bool,
    hold_menu_gates_archive: Rc<Cell<bool>>,
    chats_request_in_flight: bool,
    avatar_prefetch: ChatListAvatarPrefetchController,
    history_warmup: ChatListHistoryWarmupController,
    embedded_forum_chat_id: Option<String>,
    embedded_forum_chat_snapshot: Option<ChatInfo>,
    embedded_forum_initial_snapshot: Option<ForumTopicListSnapshot>,
    embedded_forum_allows_detached_snapshot: bool,
    embedded_forum_presentation_epoch: u64,
    embedded_forum_transition: Entity<EmbeddedForumTransitionState>,
    show_embedded_section_create: bool,
    embedded_section_name: String,
    chrome_collapse: Entity<ListShellHeaderCollapse>,
    search_ns: Entity<ChatSearchHeroNamespace>,
    search_presence: Entity<Presence<()>>,
    pending_delete_presence: Entity<Presence<ChatInfo>>,
    focus: FocusHandle,
    presented_account: Option<String>,
    engine_lists: Option<Rc<ChatsEngineLists>>,
    page_models: Rc<RefCell<HashMap<String, ChatsPageModel>>>,
    page_rows: Rc<RefCell<HashMap<String, ChatsRowsMemo>>>,
    archive_merge: Rc<RefCell<Option<ChatsArchiveMerge>>>,
    nav_reads: ChatsNavReads,
    nav_on_chats: bool,
    rows_held_for_entry: bool,
    mark_redraw_pending: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChatsViewState {
    /// rereads what the page draws from the engine, true when any of it moved
    fn refresh_engine_lists(&mut self, cx: &mut Context<Self>) -> bool {
        let (presented_account, engine_lists) = ChatsEngineLists::read(cx);
        let changed =
            presented_account != self.presented_account || engine_lists.as_ref() != self.engine_lists.as_deref();
        self.presented_account = presented_account;
        if engine_lists.as_ref() != self.engine_lists.as_deref() {
            self.engine_lists = engine_lists.map(Rc::new);
        }
        changed
    }

    fn page_reads_presentation_change(change: &RiseConversationPresentationChange) -> bool {
        !matches!(
            change,
            RiseConversationPresentationChange::MessageScope(_)
                | RiseConversationPresentationChange::UnreadMarks(_)
                | RiseConversationPresentationChange::PinnedMessages(_)
                | RiseConversationPresentationChange::MessageTabs
        )
    }

    /// inbox request state the network status title draws, read the same way
    fn chats_request_in_flight(cx: &mut App) -> bool {
        let services = ChatServicesStore::shared(cx);
        let cache_key = services.read(cx).chats_cache_key(cx);
        SaiWsPendingRegistry::shared(cx).read(cx).has_in_flight_request(cache_key.as_deref())
    }

    fn new(nav_store: Option<&Entity<NavigationStore>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|_| ChatsScreenState::new());
        let reorder = cx.new(|_| ChatsPinnedReorderState::new());
        let tab_pager_live_state = cx.new(|_| AnimatedTabsPagerLiveState::new());
        let chrome_collapse = cx.new(|_| ListShellHeaderCollapse::new());

        let services = ChatServicesStore::shared(cx);
        let actions = ChatActionsStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let user = UserServicesStore::shared(cx);
        let presentation = RiseConversationPresentation::shared(cx);
        let drafts = ChatDraftStore::shared(cx);
        let socket = RiseTransportStatus::shown_store(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let maintenance = MaintenanceGate::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let embedded_forum_presentation = EmbeddedForumPresentationStore::shared(cx);
        let page_models: Rc<RefCell<HashMap<String, ChatsPageModel>>> = Rc::default();
        let page_rows: Rc<RefCell<HashMap<String, ChatsRowsMemo>>> = Rc::default();
        let forget_models = {
            let (page_models, page_rows) = (page_models.clone(), page_rows.clone());
            move || {
                page_models.borrow_mut().clear();
                page_rows.borrow_mut().clear();
            }
        };
        let (forget_on_state, forget_on_services, forget_on_user, forget_on_drafts, forget_on_lesson) = (
            forget_models.clone(),
            forget_models.clone(),
            forget_models.clone(),
            forget_models.clone(),
            forget_models,
        );
        let sandbox = GuideSandbox::shared(cx);
        let lesson_listed = Rc::new(std::cell::Cell::new(sandbox.read(cx).owns_mute_scene()));
        let sandbox_forum_id = GuideSandbox::forum_chat_id();
        let forum_owned = std::cell::Cell::new(sandbox.read(cx).owns_forum_chat_id(Some(sandbox_forum_id.as_str())));
        let mut subscriptions = vec![
            cx.observe(&state, move |_, _, cx| {
                forget_on_state();
                cx.notify();
            }),
            cx.observe(&reorder, |_, _, cx| cx.notify()),
            cx.observe(&tab_pager_live_state, |_, _, cx| cx.notify()),
            cx.observe(&services, move |this, _, cx| {
                forget_on_services();
                this.refresh_engine_lists(cx);
                cx.notify();
            }),
            cx.observe(&actions, |this, _, cx| {
                this.refresh_engine_lists(cx);
                cx.notify();
            }),
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&user, move |_, _, cx| {
                forget_on_user();
                cx.notify();
            }),
            cx.subscribe(&presentation, |this, _, change: &RiseConversationPresentationChange, cx| {
                if Self::page_reads_presentation_change(change) && this.refresh_engine_lists(cx) {
                    cx.notify();
                }
            }),
            cx.observe(&drafts, move |_, _, cx| {
                forget_on_drafts();
                cx.notify();
            }),
            cx.observe(&socket, |_, _, cx| cx.notify()),
            cx.observe(&pending_requests, |view, _, cx| {
                let in_flight = Self::chats_request_in_flight(cx);
                if in_flight != view.chats_request_in_flight {
                    view.chats_request_in_flight = in_flight;
                    cx.notify();
                }
            }),
            cx.observe(&maintenance, |_, _, cx| cx.notify()),
            cx.observe(&sandbox, move |_, sandbox, cx| {
                let listed = sandbox.read(cx).owns_mute_scene();
                if lesson_listed.replace(listed) != listed {
                    forget_on_lesson();
                    cx.notify();
                }
                let owned = sandbox.read(cx).owns_forum_chat_id(Some(sandbox_forum_id.as_str()));
                if forum_owned.replace(owned) != owned {
                    cx.notify();
                }
            }),
            cx.observe(&hold_context_menu, |view, menu, cx| {
                let active = menu.read(cx).active_request().is_some();
                if active != view.hold_menu_active {
                    view.hold_menu_active = active;
                    if view.hold_menu_gates_archive.get() {
                        cx.notify();
                    }
                }
            }),
            cx.observe(&embedded_forum_presentation, |_, _, cx| cx.notify()),
        ];
        if let Some(nav_store) = nav_store {
            subscriptions.push(cx.observe_in(nav_store, window, |view, nav_store, window, cx| {
                let on_chats = nav_store.read(cx).current_tab == MainTab::Chats;
                let reads = ChatsNavReads::read(nav_store.read(cx));
                if !view.nav_on_chats && reads.is_beside_on_chats && reads.marked_chat_id.is_some() {
                    view.rows_held_for_entry = true;
                } else if !on_chats {
                    view.rows_held_for_entry = false;
                }
                let stayed_on_chats = view.nav_on_chats && on_chats;
                view.nav_on_chats = on_chats;
                if reads != view.nav_reads {
                    let only_mark_moved = reads.is_chats_root == view.nav_reads.is_chats_root
                        && reads.is_beside_on_chats == view.nav_reads.is_beside_on_chats
                        && reads.embedded_forum_request == view.nav_reads.embedded_forum_request;
                    view.nav_reads = reads;
                    if stayed_on_chats && only_mark_moved {
                        if !view.mark_redraw_pending {
                            view.mark_redraw_pending = true;
                            cx.on_next_frame(window, |view, _, cx| {
                                view.mark_redraw_pending = false;
                                cx.notify();
                            });
                        }
                    } else {
                        cx.notify();
                    }
                }
            }));
        }
        subscriptions.push(cx.on_release(|_, cx| {
            ChatsSelectionChrome::shared(cx).update(cx, |chrome, cx| chrome.deactivate(cx));
        }));

        let (presented_account, engine_lists) = ChatsEngineLists::read(cx);
        let hold_menu_active = hold_context_menu.read(cx).active_request().is_some();
        let nav_reads = nav_store.map(|nav_store| ChatsNavReads::read(nav_store.read(cx))).unwrap_or_default();
        let nav_on_chats = nav_store.is_some_and(|nav_store| nav_store.read(cx).current_tab == MainTab::Chats);
        Self {
            state,
            reorder,
            page_width: 0.0,
            tab_pager_live_state,
            page_viewports: Rc::new(ChatsPageViewportStore::new()),
            retained_tab_page_ids: HashSet::from([ChatsTabPage::All.stable_id()]),
            performance_trace_revision: 0,
            is_chats_menu_presented: false,
            hold_menu_active,
            hold_menu_gates_archive: Rc::new(Cell::new(true)),
            chats_request_in_flight: Self::chats_request_in_flight(cx),
            avatar_prefetch: ChatListAvatarPrefetchController::new(),
            history_warmup: ChatListHistoryWarmupController::new(),
            embedded_forum_chat_id: None,
            embedded_forum_chat_snapshot: None,
            embedded_forum_initial_snapshot: None,
            embedded_forum_allows_detached_snapshot: false,
            embedded_forum_presentation_epoch: 0,
            embedded_forum_transition: cx.new(|_| EmbeddedForumTransitionState::new()),
            show_embedded_section_create: false,
            embedded_section_name: String::new(),
            chrome_collapse,
            search_ns: cx.new(|_| ChatSearchHeroNamespace::new()),
            search_presence: cx.new(|_| Presence::new(None).with_transition(Transition::opacity())),
            pending_delete_presence: cx
                .new(|_| Presence::new(None).with_transition(Transition::opacity().combined(Transition::scale(0.96)))),
            focus: cx.focus_handle(),
            presented_account,
            engine_lists: engine_lists.map(Rc::new),
            page_models,
            page_rows,
            archive_merge: Rc::default(),
            nav_reads,
            nav_on_chats,
            rows_held_for_entry: false,
            mark_redraw_pending: false,
            _subscriptions: subscriptions,
        }
    }
}

/// what the page draws from navigation, taps read the store themselves
#[derive(Clone, PartialEq, Default)]
struct ChatsNavReads {
    is_chats_root: bool,
    is_beside_on_chats: bool,
    marked_chat_id: Option<String>,
    embedded_forum_request: Option<EmbeddedForumRequest>,
}

impl ChatsNavReads {
    fn read(nav_store: &NavigationStore) -> Self {
        let on_chats = nav_store.current_tab == MainTab::Chats;
        let is_beside_on_chats = on_chats && nav_store.chat_list_beside_stack();
        let marked_chat_id = if is_beside_on_chats {
            nav_store.path.iter().rev().find_map(|route| match route {
                RootRoute::Chat { chat_id } | RootRoute::ChatTopic { chat_id, .. } => chat_id.clone(),
                _ => None,
            })
        } else {
            None
        };
        Self {
            is_chats_root: on_chats && nav_store.path.is_empty(),
            is_beside_on_chats,
            marked_chat_id,
            embedded_forum_request: nav_store.embedded_forum_request().cloned(),
        }
    }
}

#[derive(IntoElement, Default)]
pub struct Chats;

impl Chats {
    pub fn new() -> Self {
        Self
    }
}

/// what the row and header closures see of the view
#[derive(Clone)]
struct ChatsActions {
    view: Entity<ChatsViewState>,
    state: Entity<ChatsScreenState>,
    reorder: Entity<ChatsPinnedReorderState>,
    tab_pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    page_viewports: Rc<ChatsPageViewportStore>,
    embedded_forum_transition: Entity<EmbeddedForumTransitionState>,
    chrome_collapse: Entity<ListShellHeaderCollapse>,
    search_ns: Entity<ChatSearchHeroNamespace>,
    search_presence: Entity<Presence<()>>,
    pending_delete_presence: Entity<Presence<ChatInfo>>,
    nav_store: Option<Entity<NavigationStore>>,
    page_models: Rc<RefCell<HashMap<String, ChatsPageModel>>>,
    page_rows: Rc<RefCell<HashMap<String, ChatsRowsMemo>>>,
    archive_merge: Rc<RefCell<Option<ChatsArchiveMerge>>>,
}

/// what one tab page takes from the screen body
#[derive(Clone)]
struct ChatsPageFrame {
    source: Rc<Vec<ChatInfo>>,
    is_loading: bool,
    is_active_page: bool,
    collapse_range: f32,
    has_embedded_forum: bool,
    rail_page_id: Option<String>,
    selected_page_id: Option<String>,
}

/// what every row of one list takes from the all tab
#[derive(Clone)]
struct ChatsListFrame {
    viewport: Entity<RiseChatViewport>,
    display_chats: Rc<Vec<ChatInfo>>,
    source: Rc<Vec<ChatInfo>>,
    page_id: String,
    is_active_page: bool,
    has_before: bool,
    practice_listed: bool,
    guide_chat_id: String,
    is_reorder_mode: bool,
    selected_for_reorder: HashSet<String>,
    pinned_count: i64,
    pinned_ids: Rc<Vec<String>>,
    folder_id: Option<String>,
    separator: Hsla,
    open_chat_id: Option<String>,
    open_fill: Hsla,
}

impl RenderOnce for Chats {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let observed_nav = nav_store.clone();
        let view =
            window.use_keyed_state("chats-view-state", cx, move |window, cx| {
            ChatsViewState::new(observed_nav.as_ref(), window, cx)
        });
        let actions = {
            let current = view.read(cx);
            ChatsActions {
                view: view.clone(),
                state: current.state.clone(),
                reorder: current.reorder.clone(),
                tab_pager_live_state: current.tab_pager_live_state.clone(),
                page_viewports: current.page_viewports.clone(),
                embedded_forum_transition: current.embedded_forum_transition.clone(),
                chrome_collapse: current.chrome_collapse.clone(),
                search_ns: current.search_ns.clone(),
                search_presence: current.search_presence.clone(),
                pending_delete_presence: current.pending_delete_presence.clone(),
                nav_store,
                page_models: current.page_models.clone(),
                page_rows: current.page_rows.clone(),
                archive_merge: current.archive_merge.clone(),
            }
        };
        actions.body(window, cx)
    }
}

impl ChatsActions {
    fn snap_embedded_forum_rail(
        viewports: &ChatsPageViewportStore,
        collapse: &Entity<ListShellHeaderCollapse>,
        page_id: &str,
        chat_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let control = viewports.control(page_id);
        let Some(list) = control.list() else {
            return;
        };
        let fold = {
            let collapse = collapse.read(cx);
            if collapse.is_held() { collapse.offset() } else { collapse.range() }
        };
        control.normal_top(window, cx);
        list.update(cx, |list, cx| {
            let viewport_height = list.scroll_metrics().viewport_size.height;
            let row_fits = list
                .viewport_frame(chat_id)
                .is_some_and(|row| row.origin.y - gpui::px(fold) + row.size.height <= viewport_height);
            if row_fits {
                return;
            }
            let moved = list.smart_scroll_to_item_at_top(
                chat_id,
                crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListTransactionJumpAnimationDirection::TowardOlder,
                false,
                window,
                cx,
            );
            if !moved {
                return;
            }
            let offset_y = list.scroll_metrics().content_offset_y;
            let _ = list.set_content_offset_y(offset_y + gpui::px(fold), false, true, window, cx);
        });
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::default(), cx));
        }
    }

    fn open_chat(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.open_from_chat_list(route, cx));
        }
    }

    fn set_search_overlay(&self, value: bool, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.search_overlay = value;
            if !value {
                state.query.clear();
            }
            cx.notify();
        });
        ChatSearchOverlayPresenter::shared(cx).update(cx, |presenter, cx| presenter.set_is_open(value, cx));
        let now = RiseClock::monotonic();
        self.search_ns.update(cx, |namespace, cx| namespace.set_presented(value, now, cx));
        self.search_presence.update(cx, |presence, cx| {
            presence.set_in(value.then_some(()), ChatSearchHero::animation(), now, cx);
            cx.notify();
        });
    }

    fn body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let services = ChatServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let engine_lists = self.view.read(cx).engine_lists.clone();
        let (source, requests_count, chats_did_load) = match engine_lists.as_deref() {
            Some(lists) => (
                lists.source.clone(),
                lists.requests_count,
                !lists.source.is_empty() || lists.inbox_load_state == Some(RiseEntityViewLoadState::Loaded),
            ),
            None => {
                let source = Rc::new(services.read(cx).chat_list_rows(cx));
                let requests_state_count = RiseConversationPresentation::shared(cx)
                    .read(cx)
                    .chat_state(RiseChatListKind::Requests)
                    .map(ChatServicesStore::message_requests_badge_count);
                let requests_count = match requests_state_count {
                    Some(count) => count,
                    None => services.update(cx, |services, cx| services.message_requests(cx).len() as i64),
                };
                let chats_did_load = !source.is_empty() || {
                    let slice = ChatActionsStore::shared(cx).read(cx).get_chats_sai.clone();
                    slice.status() == SaiWsSliceStatus::Fulfilled || slice.data().is_some()
                };
                (source, requests_count, chats_did_load)
            }
        };
        let folders = services.read(cx).folders_in_order(cx);
        let pages = Rc::new(if folders.is_empty() {
            vec![ChatsTabPage::All, ChatsTabPage::CreatePrompt]
        } else {
            std::iter::once(ChatsTabPage::All).chain(folders.into_iter().map(ChatsTabPage::Folder)).collect()
        });
        let tab_configs = pages
            .iter()
            .enumerate()
            .map(|(index, page)| TabHeaderConfig::new(index as i64, page.tab_title()).custom_emojis(page.tab_custom_emojis()))
            .collect::<Vec<_>>();
        let page_ids = pages.iter().map(ChatsTabPage::stable_id).collect::<Vec<_>>();
        let tabs_count = tab_configs.len() as i64;

        let (active_tab, is_reorder_mode, search_overlay, pending_delete_id) = {
            let state = self.state.read(cx);
            (
                state.active_tab,
                state.is_reorder_mode,
                state.search_overlay,
                state.pending_delete_chat.as_ref().map(|chat| chat.id.clone()),
            )
        };
        let active_page_index = active_tab.clamp(0, tabs_count - 1);
        let (
            embedded_forum_chat_id,
            embedded_forum_chat_snapshot,
            embedded_forum_initial_snapshot,
            embedded_forum_epoch,
            allows_detached_snapshot,
            page_width,
            focus,
            show_section_create,
        ) = {
            let view = self.view.read(cx);
            (
                view.embedded_forum_chat_id.clone(),
                view.embedded_forum_chat_snapshot.clone(),
                view.embedded_forum_initial_snapshot.clone(),
                view.embedded_forum_presentation_epoch,
                view.embedded_forum_allows_detached_snapshot,
                view.page_width,
                view.focus.clone(),
                view.show_embedded_section_create,
            )
        };

        let is_chats_root = self.nav_store.as_ref().is_some_and(|nav_store| {
            let nav_store = nav_store.read(cx);
            nav_store.current_tab == MainTab::Chats && nav_store.path.is_empty()
        });
        let is_list_on_screen = is_chats_root
            || self.nav_store.as_ref().is_some_and(|nav_store| {
                let nav_store = nav_store.read(cx);
                nav_store.current_tab == MainTab::Chats && nav_store.chat_list_beside_stack()
            });
        let is_authenticated = auth.read(cx).is_authenticated();
        let active_prefetch_page_id = (is_authenticated
            && is_list_on_screen
            && !search_overlay
            && embedded_forum_chat_id.is_none())
        .then(|| page_ids[active_page_index as usize].clone());
        let rail_page_id = (is_authenticated && is_list_on_screen && !search_overlay && embedded_forum_chat_id.is_some())
            .then(|| page_ids[active_page_index as usize].clone());
        let embedded_forum_chat = embedded_forum_chat_id.as_ref().and_then(|id| {
            source
                .iter()
                .find(|chat| &chat.id == id)
                .cloned()
                .or_else(|| embedded_forum_chat_snapshot.filter(|chat| &chat.id == id))
        });
        let embedded_forum_width = self.embedded_forum_container_width(page_width);
        let header_theme_revision =
            format!("{:?}|{:?}|{:?}|{:?}", theme.bg._100, theme.primary._100, theme.text.primary, theme.text.secondary);
        let embedded_forum_center_revision = format!(
            "{}|{}|{}|{header_theme_revision}",
            embedded_forum_chat.as_ref().map_or("", |chat| chat.id.as_str()),
            embedded_forum_chat.as_ref().map_or("", |chat| chat.title.as_str()),
            embedded_forum_chat.as_ref().map(|chat| chat.member_count).unwrap_or_default(),
        );
        let embedded_forum_right_revision = format!(
            "{}|{requests_count}|{is_reorder_mode}|{header_theme_revision}",
            embedded_forum_chat.as_ref().map_or("", |chat| chat.id.as_str()),
        );
        let embedded_forum_left_revision =
            format!("{}|{is_reorder_mode}|{header_theme_revision}", embedded_forum_chat.is_some());

        let active_tab_binding = {
            let (get_state, set_state, set_view) = (self.state.clone(), self.state.clone(), self.view.clone());
            Binding::new(
                move |cx: &App| get_state.read(cx).active_tab.clamp(0, tabs_count - 1),
                move |value: i64, _: &mut Window, cx: &mut App| {
                    if set_view.read(cx).embedded_forum_chat_id.is_some() {
                        return;
                    }
                    let clamped = value.clamp(0, tabs_count - 1);
                    set_state.update(cx, |state, cx| {
                        if clamped != state.active_tab {
                            state.active_tab = clamped;
                            cx.notify();
                        }
                    });
                },
            )
        };
        let presence_partner_ids = source
            .iter()
            .filter(|chat| chat.r#type == ChatType::Direct)
            .map(|chat| chat.participant.id.clone())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();

        let collapse_range = ChatsChromeMetrics::collapse_range(tabs_count > 1);
        let frame = ChatsPageFrame {
            source: source.clone(),
            is_loading: !chats_did_load,
            is_active_page: false,
            collapse_range,
            has_embedded_forum: embedded_forum_chat_id.is_some(),
            rail_page_id,
            selected_page_id: (is_authenticated && !search_overlay && embedded_forum_chat_id.is_none())
                .then(|| page_ids[active_page_index as usize].clone()),
        };
        let mount_anchor = self.mount_anchor_index(pages.len() as i64, cx);

        self.install_changes(
            ChatsChangesFrame {
                pages: pages.clone(),
                page_ids: page_ids.clone(),
                mount_anchor,
                active_prefetch_page_id: active_prefetch_page_id.clone(),
                active_page_index,
                has_tabs: tabs_count > 1,
                embedded_forum_chat_id: embedded_forum_chat_id.clone(),
                allows_detached_snapshot,
                is_reorder_mode,
                selected_count: self.state.read(cx).selected_for_reorder.len() as i64,
                is_chats_tab_active: is_list_on_screen,
                pending_delete_id,
                presence_partner_ids,
                source_ids: source.iter().map(|chat| chat.id.clone()).collect(),
                source: source.clone(),
            },
            window,
            cx,
        );

        let shell = self.shell(
            ChatsShellFrame {
                pages: pages.clone(),
                tab_configs,
                tabs_count,
                active_tab_binding,
                active_prefetch_page_id,
                frame,
                mount_anchor,
                embedded_forum_chat,
                embedded_forum_initial_snapshot,
                embedded_forum_epoch,
                embedded_forum_width,
                requests_count,
                embedded_forum_center_revision,
                embedded_forum_right_revision,
                embedded_forum_left_revision,
                search_overlay,
            },
            cx,
        );

        let now = RiseClock::monotonic();
        let search_overlay_layer = {
            let search_presence = self.search_presence.clone();
            let actions = self.clone();
            let hero = self.search_ns.clone();
            search_presence.read(cx).render_transitioned(now, window, move |_| {
                let (get_state, set_actions) = (actions.state.clone(), actions.clone());
                let is_presented = Binding::new(
                    move |cx: &App| get_state.read(cx).search_overlay,
                    move |value: bool, _: &mut Window, cx: &mut App| set_actions.set_search_overlay(value, cx),
                );
                let query = Binding::entity(
                    actions.state.clone(),
                    |state: &ChatsScreenState| state.query.clone(),
                    |state: &mut ChatsScreenState, value: String, _| state.query = value,
                );
                div().absolute().inset_0().child(
                    div()
                        .size_full()
                        .child(
                            env_scope::<ChatsSearchContextKey>(Some(actions.search_context()))
                                .child(ChatSearchOverlay::new("chats-search-overlay", is_presented, query, hero)),
                        )
                        .guide_anchor_cover("chats-search-guide-cover", GuideAnchorCoverKind::Layer),
                )
            })
        };

        let delete_dialog = {
            let pending_delete_presence = self.pending_delete_presence.clone();
            let actions = self.clone();
            pending_delete_presence.read(cx).render_transitioned(now, window, move |chat| {
                let (for_me, for_everyone, cancel) = (actions.clone(), actions.clone(), actions.clone());
                let (me_chat, everyone_chat) = (chat.clone(), chat.clone());
                div().absolute().inset_0().child(ChatDeleteDialog::new(
                    chat.clone(),
                    move |_, cx| {
                        if me_chat.r#type == ChatType::Favourites {
                            for_me.clear_favourites_history(&me_chat, cx);
                        } else {
                            for_me.delete_chat(&me_chat, false, cx);
                        }
                    },
                    move |_, cx| {
                        if everyone_chat.r#type == ChatType::Favourites {
                            for_everyone.clear_favourites_history(&everyone_chat, cx);
                        } else {
                            for_everyone.delete_chat(&everyone_chat, true, cx);
                        }
                    },
                    move |_, cx| cancel.dismiss_delete_dialog(cx),
                ))
            })
        };

        let section_alert = show_section_create.then(|| self.section_create_alert());

        let key_actions = self.clone();
        let appear_actions = self.clone();
        let disappear_actions = self.clone();
        let appearing = focus.clone();
        div()
            .id("chats-keys")
            .relative()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key == FIND_KEY && keystroke.modifiers.secondary() {
                    cx.stop_propagation();
                    if !key_actions.state.read(cx).search_overlay {
                        key_actions.set_search_overlay(true, cx);
                    }
                }
            })
            .child(shell)
            .children(search_overlay_layer)
            .children(delete_dialog)
            .children(section_alert)
            .on_appear("chats-lifecycle", move |window, cx| {
                ScreenFocus::claim(&appearing, window, cx);
                if appear_actions.state.read(cx).search_overlay {
                    ChatSearchOverlayPresenter::shared(cx).update(cx, |presenter, cx| presenter.set_is_open(true, cx));
                }
            })
            .and_on_disappear(move |_, cx| {
                disappear_actions.page_viewports.capture_all(cx);
                ChatPerformanceTrace::shared().stop_if_recording("chat-list", "chat_list_disappeared");
                PremiumBadgePickerPopover::shared(cx).update(cx, |popover, cx| popover.dismiss(cx));
                ChatSearchOverlayPresenter::shared(cx).update(cx, |presenter, cx| presenter.set_is_open(false, cx));
                disappear_actions.view.update(cx, |view, cx| {
                    view.avatar_prefetch.cancel_all(cx);
                    view.history_warmup.cancel_all(cx);
                });
            })
            .into_any_element()
    }

    fn search_context(&self) -> ChatsSearchContext {
        let (visible_state, hold_state, query_state, overlay_actions) =
            (self.state.clone(), self.state.clone(), self.state.clone(), self.clone());
        ChatsSearchContext {
            visible_chats: Rc::new(move |filter_query: &str, source: &[ChatInfo], cx: &mut App| {
                ChatsScreenState::visible_chats(&visible_state, filter_query, source, cx)
            }),
            apply_hold_action: Rc::new(
                move |chat_id: &str, action_id: &str, source: &[ChatInfo], _: &mut Window, cx: &mut App| {
                    ChatsScreenState::apply_hold_action(&hold_state, chat_id, action_id, source, None, cx);
                },
            ),
            set_search_overlay: Rc::new(move |value: bool, _: &mut Window, cx: &mut App| {
                overlay_actions.set_search_overlay(value, cx);
            }),
            set_query: Rc::new(move |value: String, _: &mut Window, cx: &mut App| {
                query_state.update(cx, |state, cx| {
                    state.query = value;
                    cx.notify();
                });
            }),
        }
    }

    fn section_create_alert(&self) -> AnyElement {
        let is_presented = Binding::entity(
            self.view.clone(),
            |view: &ChatsViewState| view.show_embedded_section_create,
            |view: &mut ChatsViewState, value: bool, _| view.show_embedded_section_create = value,
        );
        let text = Binding::entity(
            self.view.clone(),
            |view: &ChatsViewState| view.embedded_section_name.clone(),
            |view: &mut ChatsViewState, value: String, _| view.embedded_section_name = value,
        );
        let view = self.view.clone();
        TextFieldAlert::new(
            "chats-section-create",
            is_presented,
            tr("forum_section_create_title"),
            tr("forum_section_name"),
            text,
            tr("common_cancel"),
            tr("forum_section_create"),
            move |_, cx| {
                let (name, chat_id) = view.update(cx, |view, cx| {
                    let name = trimming_whitespaces_and_newlines(&view.embedded_section_name).to_owned();
                    view.embedded_section_name.clear();
                    cx.notify();
                    (name, view.embedded_forum_chat_id.clone())
                });
                let Some(chat_id) = chat_id.filter(|_| !name.is_empty()) else {
                    return;
                };
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.create_section(&chat_id, &name, None, None, cx));
            },
        )
        .into_any_element()
    }

    fn shell(&self, shell: ChatsShellFrame, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let center_actions = self.clone();
        let center_chat = shell.embedded_forum_chat.clone();
        let center_slot = EmbeddedForumProgressReader::new(
            self.embedded_forum_transition.clone(),
            shell.embedded_forum_width,
            shell.embedded_forum_center_revision.clone(),
            move |progress, _, cx| center_actions.chats_header_center(center_chat, progress, cx),
        );
        let right_actions = self.clone();
        let right_chat = shell.embedded_forum_chat.clone();
        let requests_count = shell.requests_count;
        let right_jsx = EmbeddedForumProgressReader::new(
            self.embedded_forum_transition.clone(),
            shell.embedded_forum_width,
            shell.embedded_forum_right_revision.clone(),
            move |progress, _, cx| right_actions.chats_header_right(right_chat, requests_count, progress, cx),
        );
        let left_actions = self.clone();
        let has_forum = shell.embedded_forum_chat.is_some();
        let left_jsx = EmbeddedForumProgressReader::new(
            self.embedded_forum_transition.clone(),
            shell.embedded_forum_width,
            shell.embedded_forum_left_revision.clone(),
            move |progress, _, cx| left_actions.chats_header_left(has_forum, progress, cx),
        );

        let below_header = self.below_header(&shell);
        let content = self.shell_content(shell, theme.bg._100, cx);
        ListShellUi::new()
            .center_slot(center_slot)
            .center_slot_self_styled(true)
            .use_safe_zone(true)
            .shows_back_button(false)
            .right_jsx(right_jsx)
            .left_jsx(left_jsx)
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_blurred_header()))
            .header_collapse(self.chrome_collapse.clone())
            .below_header(below_header)
            .content(content)
            .into_any_element()
    }

    fn below_header(&self, shell: &ChatsShellFrame) -> AnyElement {
        let collapse = self.chrome_collapse.clone();
        let search_slot = if shell.search_overlay {
            div().w_full().h(px(ChatsChromeMetrics::SEARCH_FIELD_HEIGHT))
        } else {
            let activate = self.clone();
            let namespace = self.search_ns.clone();
            div()
                .relative()
                .w_full()
                .h(px(ChatsChromeMetrics::SEARCH_FIELD_HEIGHT))
                .child(
                    SearchFieldUi::new("chats-search-field", Binding::constant(String::new()))
                        .on_activate(move |_, cx| activate.set_search_overlay(true, cx))
                        .guide_anchor(RiseGuideAnchor::ChatsSearchField),
                )
                .child(measure("chats-search-field-frame", move |bounds, _, cx| {
                    namespace.update(cx, |namespace, _| namespace.collapsed_field_frame = Some(bounds));
                }))
        };
        let search = div()
            .w_full()
            .h(px(ChatsChromeMetrics::SEARCH_FIELD_HEIGHT))
            .child(search_slot.list_shell_header_collapse_slide(&collapse, true));

        let tabs = (shell.tabs_count > 1).then(|| {
            let tap_actions = self.clone();
            let tap_binding = shell.active_tab_binding.clone();
            let tap_pages = shell.pages.clone();
            let header = ChatsTabsHeader {
                tabs: shell.tab_configs.clone(),
                active_tab: shell.active_tab_binding.clone(),
                live_drag_state: self.tab_pager_live_state.clone(),
                on_tap_tab: Rc::new(move |id: i64, window: &mut Window, cx: &mut App| {
                    tap_actions.on_tap_tab(id, &tap_binding, &tap_pages, window, cx);
                }),
            };
            div().w_full().child(
                div()
                    .w_full()
                    .pt(px(ChatsChromeMetrics::TABS_TOP_GAP))
                    .child(div().w_full().h(AnimatedTabsMetrics::BAR_HEIGHT).capsule().overflow_hidden().child(header))
                    .list_shell_header_collapse_slide(&collapse, false),
            )
        });

        vstack(0.0)
            .w_full()
            .px(px(10.0))
            .pt(px(8.0))
            .pb(px(10.0))
            .overflow_hidden()
            .child(search)
            .children(tabs)
            .into_any_element()
    }

    fn on_tap_tab(
        &self,
        id: i64,
        active_tab: &Binding<i64>,
        pages: &Rc<Vec<ChatsTabPage>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_tab = active_tab.get(cx);
        let closes_embedded_forum = self.view.read(cx).embedded_forum_chat_id.is_some();
        if !closes_embedded_forum && id == current_tab {
            return;
        }
        if id != current_tab {
            self.retain_tab_pages_through(current_tab, id, pages, cx);
        }
        let _ = selection_changed();
        if closes_embedded_forum {
            self.state.update(cx, |state, cx| {
                state.active_tab = id;
                cx.notify();
            });
            self.close_embedded_forum(cx);
            return;
        }
        active_tab.set(id, window, cx);
    }

    fn shell_content(&self, shell: ChatsShellFrame, background: Hsla, cx: &mut App) -> AnyElement {
        let (width_reader, width_writer) = (self.view.clone(), self.view.clone());
        let page_width = Binding::new(
            move |cx: &App| width_reader.read(cx).page_width,
            move |width: f32, _: &mut Window, cx: &mut App| {
                width_writer.update(cx, |view, _| view.page_width = width);
            },
        );
        let suppressed_state = self.tab_pager_live_state.clone();
        let mut pager = AnimatedTabsPager::new(
            "chats-tabs-pager",
            shell.tabs_count,
            shell.active_tab_binding.clone(),
            Binding::constant(0.0),
            page_width,
        )
        .live_drag_state(Some(self.tab_pager_live_state.clone()))
        .yields_first_tab_back_swipe_to_navigation(true)
        .drag_locked(shell.frame.has_embedded_forum || shell.search_overlay)
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed: bool, _: &mut Window, cx: &mut App| {
            suppressed_state.update(cx, |state, cx| {
                state.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
            });
        })));

        for (index, page) in shell.pages.iter().enumerate() {
            let content = if self.should_materialize_tab_page(page, index as i64, shell.mount_anchor, cx) {
                let frame = ChatsPageFrame {
                    is_active_page: shell.active_prefetch_page_id.as_deref() == Some(page.stable_id().as_str()),
                    ..shell.frame.clone()
                };
                self.chats_tab_page_view(page, frame, cx)
            } else {
                div().into_any_element()
            };
            pager = pager.child(div().id(SharedString::from(page.stable_id())).size_full().child(content));
        }

        let lifted = ListShellHeaderCollapseContentLift::new(self.chrome_collapse.clone(), pager);
        let rail = EmbeddedForumRailContext::new(
            self.embedded_forum_transition.clone(),
            shell.embedded_forum_width,
            cx,
        );
        let pages_layer = env_scope::<EmbeddedForumRailContextKey>(Some(rail)).child(lifted);

        let forum_layer = self.embedded_forum_layer(&shell, cx);
        env_scope::<ChatsListRowActionsSuppressorKey>(Some(self.tab_pager_live_state.clone()))
            .child(
                div()
                    .relative()
                    .size_full()
                    .bg(background)
                    .child(pages_layer)
                    .children(forum_layer),
            )
            .into_any_element()
    }

    fn embedded_forum_layer(&self, shell: &ChatsShellFrame, cx: &mut App) -> Option<AnyElement> {
        let forum_chat = shell.embedded_forum_chat.clone()?;
        let initial_snapshot = shell.embedded_forum_initial_snapshot.clone()?;
        if initial_snapshot.chat.id != forum_chat.id {
            return None;
        }
        let epoch = shell.embedded_forum_epoch;
        let bottom_inset = main_tab_bar_underlap(cx).max(32.0);
        let (all_actions, all_chat_id) = (self.clone(), forum_chat.id.clone());
        let (topic_actions, topic_chat_id) = (self.clone(), forum_chat.id.clone());
        let section_view = self.view.clone();
        let close_actions = self.clone();
        let (mounted_actions, mounted_chat_id) = (self.clone(), forum_chat.id.clone());
        let layer = EmbeddedForumTransitionLayer {
            chat_id: forum_chat.id.clone(),
            presentation_epoch: epoch,
            initial_snapshot,
            container_width: shell.embedded_forum_width,
            bottom_inset,
            chrome_collapse: self.chrome_collapse.clone(),
            transition: self.embedded_forum_transition.clone(),
            on_open_all: Rc::new(move |_: &mut Window, cx: &mut App| {
                all_actions.open_chat(RootRoute::ForumAll { chat_id: Some(all_chat_id.clone()) }, cx);
            }),
            on_open_topic: Rc::new(move |topic: Option<ChatTopic>, _: &mut Window, cx: &mut App| {
                topic_actions.open_topic(&topic_chat_id, topic, cx);
            }),
            on_create_section: Rc::new(move |_: &mut Window, cx: &mut App| {
                section_view.update(cx, |view, cx| {
                    view.embedded_section_name.clear();
                    view.show_embedded_section_create = true;
                    cx.notify();
                });
            }),
            on_close: Rc::new(move |_: &mut Window, cx: &mut App| close_actions.close_embedded_forum(cx)),
            on_mounted: Rc::new(move |_: &mut Window, cx: &mut App| {
                mounted_actions.begin_embedded_forum_presentation(&mounted_chat_id, epoch, cx);
            }),
        };
        Some(
            div()
                .id(("chats-embedded-forum", epoch))
                .absolute()
                .inset_0()
                .child(layer)
                .into_any_element(),
        )
    }

    fn open_topic(&self, chat_id: &str, topic: Option<ChatTopic>, cx: &mut App) {
        if let Some(topic) = topic.as_ref().filter(|topic| topic.is_rtc_channel()) {
            if let Some(nav_store) = &self.nav_store {
                RiseRTCNavigationGate::open(
                    RiseRTCRoomTarget::Topic { chat_id: chat_id.to_owned(), topic_id: topic.id.clone() },
                    nav_store,
                    cx,
                );
            }
            return;
        }
        let topic_id = topic.map(|topic| topic.id).filter(|topic_id| !topic_id.is_empty());
        let open_info = match &topic_id {
            Some(topic_id) => {
                RootRoute::TopicProfile { chat_id: Some(chat_id.to_owned()), topic_id: Some(topic_id.clone()) }
            }
            None => RootRoute::GeneralProfile { chat_id: Some(chat_id.to_owned()) },
        };
        if let Some(hooks) = chats_aside_hooks() {
            (hooks.follow_topic)(chat_id, Some(&open_info), cx);
        }
        self.open_chat(RootRoute::ChatTopic { chat_id: Some(chat_id.to_owned()), topic_id }, cx);
    }

    fn install_changes(&self, frame: ChatsChangesFrame, window: &mut Window, cx: &mut App) {
        let actions = self.clone();
        on_change(
            "chats-selection-chrome-sync",
            (frame.is_reorder_mode, frame.is_chats_tab_active),
            true,
            window,
            cx,
            move |_, (_, is_tab_active), _, cx| actions.sync_selection_chrome(*is_tab_active, cx),
        );
        on_change("chats-selection-count", frame.selected_count, false, window, cx, move |_, count, _, cx| {
            ChatsSelectionChrome::shared(cx).update(cx, |chrome, cx| chrome.update_selected_count(*count, cx));
        });

        let (actions, pages) = (self.clone(), frame.pages.clone());
        on_change("chats-mount-anchor", frame.mount_anchor, true, window, cx, move |_, anchor, _, cx| {
            actions.retain_tab_pages_around(*anchor, &pages, cx);
        });

        let actions = self.clone();
        on_change(
            "chats-active-prefetch-page",
            frame.active_prefetch_page_id.clone(),
            true,
            window,
            cx,
            move |_, page_id, _, cx| {
                actions.view.update(cx, |view, cx| view.avatar_prefetch.activate(page_id.as_deref(), cx));
                actions.reorder.update(cx, |reorder, cx| {
                    if reorder.dragging_chat_id.is_some() && &reorder.page_id != page_id {
                        reorder.reset(cx);
                        cx.notify();
                    }
                });
                actions.view.update(cx, |view, cx| match page_id {
                    Some(page_id) => view.history_warmup.activate(page_id, cx),
                    None if view.embedded_forum_chat_id.is_none() => view.history_warmup.cancel_all(cx),
                    None => {}
                });
            },
        );

        let (collapse, pages) = (self.chrome_collapse.clone(), frame.pages.clone());
        on_change("chats-active-tab", frame.active_page_index, true, window, cx, move |_, tab, _, cx| {
            let Some(page) = usize::try_from(*tab).ok().and_then(|tab| pages.get(tab)) else {
                return;
            };
            let page_id = page.stable_id();
            collapse.update(cx, |collapse, cx| {
                collapse.activate(&page_id, Some(AnimatedTabsMetrics::release_animation()), cx);
            });
        });

        let collapse = self.chrome_collapse.clone();
        on_change("chats-has-tabs", frame.has_tabs, true, window, cx, move |_, has_tabs, _, cx| {
            collapse.update(cx, |collapse, cx| collapse.set_range(ChatsChromeMetrics::collapse_range(*has_tabs), cx));
        });

        let (actions, pages) = (self.clone(), frame.pages.clone());
        on_change("chats-page-ids", frame.page_ids.clone(), false, window, cx, move |_, page_ids, _, cx| {
            let last = (pages.len() as i64 - 1).max(0);
            let clamped = actions.state.read(cx).active_tab.clamp(0, last);
            actions.state.update(cx, |state, cx| {
                if state.active_tab != clamped {
                    state.active_tab = clamped;
                    cx.notify();
                }
            });
            actions.retain_tab_pages_around(clamped, &pages, cx);
            let retained = page_ids.iter().cloned().collect::<HashSet<_>>();
            actions.chrome_collapse.update(cx, |collapse, _| collapse.retain(&retained));
            let Some(page) = usize::try_from(clamped).ok().and_then(|tab| pages.get(tab)) else {
                return;
            };
            let page_id = page.stable_id();
            actions.chrome_collapse.update(cx, |collapse, cx| collapse.activate(&page_id, None, cx));
        });

        let (presence, pending_state) = (self.pending_delete_presence.clone(), self.state.clone());
        on_change("chats-pending-delete", frame.pending_delete_id.clone(), false, window, cx, move |_, _, _, cx| {
            let chat = pending_state.read(cx).pending_delete_chat.clone();
            let now = RiseClock::monotonic();
            presence.update(cx, |presence, cx| {
                if chat.is_none() && !presence.is_present() {
                    return;
                }
                presence.set_in(chat, spring(0.26, 0.88), now, cx);
                cx.notify();
            });
        });

        let auth_store = AuthServicesStore::shared(cx);
        let is_authenticated = auth_store.read(cx).is_authenticated();
        let effective_user_id = auth_store.read(cx).effective_user_id_for_wire();
        let presented_account = self.view.read(cx).presented_account.clone();
        let task_key = format!(
            "{is_authenticated}-{}-{}",
            effective_user_id.as_deref().unwrap_or_default(),
            presented_account.as_deref().unwrap_or_default()
        );
        let (viewports, task_user_id) = (self.page_viewports.clone(), effective_user_id.clone());
        use_task("chats-load-task", task_key, window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                if let Some(presented) = presented_account.as_deref()
                    && !SaiAccountCacheScope::is_current_account(Some(presented))
                {
                    return;
                }
                viewports.activate_account(&format!(
                    "{}|{}",
                    task_user_id.as_deref().unwrap_or_default(),
                    SaiAccountCacheScope::active_epoch().as_deref().unwrap_or_default()
                ));
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_chats(cx);
                    store.load_initial_archived_chats(cx);
                    store.load_folders(cx);
                    store.load_message_requests(cx);
                });
                UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_chat_mute_overrides_loaded(cx));
            });
        });

        let _ = window.use_keyed_state("chats-wallpaper-prewarm", cx, ChatWallpaperPrewarmDriver::new);

        let is_connected = RiseTransportStatus::shown(cx).is_connected();
        on_change("chats-socket-connected", is_connected, false, window, cx, move |_, connected, _, cx| {
            if !*connected {
                return;
            }
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.retry_rejected_chat_list_reads(cx));
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_chat_mute_overrides_loaded(cx));
        });

        on_change(
            "chats-presence-partners",
            frame.presence_partner_ids.clone(),
            true,
            window,
            cx,
            move |_, ids, _, cx| {
                PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.observe(ids, "chats", cx));
            },
        );

        let requested_chat = EmbeddedForumPresentationStore::shared(cx).read(cx).requested_chat.clone();
        let (actions, request_source) = (self.clone(), frame.source.clone());
        on_change(
            "chats-embedded-forum-requested",
            requested_chat.as_ref().map(|chat| chat.id.clone()),
            true,
            window,
            cx,
            move |_, requested_chat_id, _, cx| {
                let Some(requested_chat_id) = requested_chat_id else {
                    return;
                };
                let Some(requested_chat) = requested_chat.filter(|chat| &chat.id == requested_chat_id) else {
                    return;
                };
                let is_detached = !request_source.iter().any(|chat| &chat.id == requested_chat_id);
                EmbeddedForumPresentationStore::shared(cx)
                    .update(cx, |store, cx| store.consume(requested_chat_id, cx));
                actions.leave_foreign_center_stack(requested_chat_id, cx);
                actions.open_embedded_forum(&requested_chat, is_detached, cx);
            },
        );

        let sandbox_forum_withdrawn = frame.embedded_forum_chat_id.as_deref()
            == Some(GuideSandbox::forum_chat_id().as_str())
            && !GuideSandbox::existing(cx).is_some_and(|sandbox| {
                sandbox
                    .read(cx)
                    .owns_forum_chat_id(frame.embedded_forum_chat_id.as_deref())
            });
        let actions = self.clone();
        on_change(
            "chats-embedded-forum-sandbox-withdrawn",
            sandbox_forum_withdrawn,
            true,
            window,
            cx,
            move |_, withdrawn, _, cx| {
                if *withdrawn {
                    tracing::warn!(target: "riseonly::X2285", "sandbox forum closed after the guide");
                    actions.close_embedded_forum(cx);
                }
            },
        );

        if let Some(nav_store) = &self.nav_store {
            let request = nav_store
                .read(cx)
                .embedded_forum_request()
                .map(|request| (request.id, request.chat_id.clone()));
            let actions = self.clone();
            on_change(
                "chats-embedded-forum-request",
                request,
                false,
                window,
                cx,
                move |_, request, _, cx| {
                    let Some((_, chat_id)) = request else {
                        return;
                    };
                    let open_chat_id = actions.view.read(cx).embedded_forum_chat_id.clone();
                    if let Some(chat_id) = chat_id {
                        if open_chat_id.as_ref() == Some(chat_id) {
                            return;
                        }
                        let services = ChatServicesStore::shared(cx);
                        let Some(chat) = services.read(cx).chat_info(chat_id, cx) else {
                            return;
                        };
                        actions.open_embedded_forum(&chat, false, cx);
                    } else if open_chat_id.is_some() {
                        actions.close_embedded_forum(cx);
                    }
                },
            );

            let nav_store = nav_store.clone();
            on_change(
                "chats-embedded-forum-overlay",
                frame.embedded_forum_chat_id.clone(),
                true,
                window,
                cx,
                move |_, chat_id, _, cx| {
                    let overlay = chat_id.as_ref().map(|chat_id| SaiActiveScreenOverlay {
                        tab: MainTab::Chats,
                        screen: RootRoute::ForumHome { chat_id: Some(chat_id.clone()) }.sai_active_screen(),
                    });
                    nav_store.update(cx, |nav_store, cx| nav_store.set_sai_active_screen_overlay(overlay, cx));
                },
            );
        }

        let rail_page_id = usize::try_from(frame.active_page_index)
            .ok()
            .and_then(|index| frame.page_ids.get(index).cloned());
        let rail_viewports = self.page_viewports.clone();
        let rail_collapse = self.chrome_collapse.clone();
        on_change(
            "chats-embedded-forum-rail-top",
            frame.embedded_forum_chat_id.clone(),
            false,
            window,
            cx,
            move |_, chat_id, window, _| {
                let (Some(chat_id), Some(page_id)) = (chat_id.clone(), rail_page_id) else {
                    return;
                };
                let rail_viewports = rail_viewports.clone();
                let rail_collapse = rail_collapse.clone();
                window.on_next_frame(move |window, cx| {
                    Self::snap_embedded_forum_rail(&rail_viewports, &rail_collapse, &page_id, &chat_id, window, cx);
                });
            },
        );

        let reorder = self.reorder.clone();
        on_change("chats-reorder-mode", frame.is_reorder_mode, false, window, cx, move |_, is_reordering, _, cx| {
            if *is_reordering {
                return;
            }
            reorder.update(cx, |reorder, cx| {
                reorder.reset(cx);
                reorder.clear_geometry();
                cx.notify();
            });
        });

        let actions = self.clone();
        on_change("chats-authenticated", is_authenticated, false, window, cx, move |_, is_authenticated, _, cx| {
            if !*is_authenticated {
                MessagePreview::invalidate_list_cache();
                actions.dismiss_embedded_forum_immediately(cx);
            }
        });

        let actions = self.clone();
        on_change("chats-effective-user", effective_user_id, false, window, cx, move |_, _, _, cx| {
            MessagePreview::invalidate_list_cache();
            actions.view.update(cx, |view, cx| {
                view.avatar_prefetch.cancel_all(cx);
                view.history_warmup.cancel_all(cx);
            });
            actions.state.update(cx, |state, cx| {
                state.reset_account_scoped_row_state();
                cx.notify();
            });
            actions.reorder.update(cx, |reorder, cx| {
                reorder.reset(cx);
                reorder.clear_geometry();
                cx.notify();
            });
            actions.dismiss_embedded_forum_immediately(cx);
        });

        let (actions, open_chat_id, allows_detached) =
            (self.clone(), frame.embedded_forum_chat_id, frame.allows_detached_snapshot);
        on_change("chats-source-ids", frame.source_ids, false, window, cx, move |_, chat_ids, _, cx| {
            let Some(chat_id) = open_chat_id else {
                return;
            };
            if !chat_ids.contains(&chat_id) && !allows_detached {
                actions.dismiss_embedded_forum_immediately(cx);
            }
        });
    }

    fn chats_header_center(&self, forum_chat: Option<ChatInfo>, progress: f32, cx: &mut App) -> AnyElement {
        let status = div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .opacity(1.0 - progress)
            .child(ChatsNetworkStatusTitle::new(self.view.read(cx).engine_lists.as_ref().is_some_and(|lists| {
                lists.inbox_load_state == Some(RiseEntityViewLoadState::Loading)
            })))
            .scale_effect(1.0 - progress * 0.06);
        let forum = forum_chat.map(|forum_chat| {
            let secondary = rise_ui::theme(cx).text.secondary;
            let actions = self.clone();
            let chat_id = forum_chat.id.clone();
            let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
                if progress > 0.92 {
                    actions.push(RootRoute::ChatProfile { chat_id: Some(chat_id.clone()) }, cx);
                }
            });
            button_chrome(div().id("chats-forum-title"), false, Some(action))
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .opacity(progress)
                .tooltip_label(forum_chat.title.clone())
                .child(
                    hstack(4.0)
                        .max_w(px(220.0))
                        .justify_center()
                        .child(
                            div().min_w(px(0.0)).child(
                                MainText::new(forum_chat.title.clone())
                                    .px(15.0)
                                    .tac(TextAlignT::Center)
                                    .font_weight(FontWeight::BOLD)
                                    .number_of_lines(1),
                            ),
                        )
                        .children(IconUi::sized("person.3.fill", px(12.0), secondary, true).map(|icon| icon.flex_none())),
                )
                .child(div().max_w(px(220.0)).child(ForumMembersSubtitle::new(
                    forum_chat.id.clone(),
                    forum_chat.member_count,
                    secondary,
                )))
                .scale_effect(0.94 + progress * 0.06)
        });
        div()
            .relative()
            .w(px(228.0))
            .max_w(relative(1.0))
            .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .child(status)
            .children(forum)
            .into_any_element()
    }

    fn chats_header_left(&self, has_forum: bool, progress: f32, cx: &mut App) -> AnyElement {
        let is_reorder_mode = self.state.read(cx).is_reorder_mode;
        let toggle_state = self.state.clone();
        let hit_tests = !has_forum && progress < 0.05;
        let toggle = div()
            .absolute()
            .inset_0()
            .opacity(1.0 - progress)
            .child(ChatsReorderToggleButton::new("chats-reorder-toggle", is_reorder_mode, move |_, cx| {
                if !hit_tests {
                    return;
                }
                toggle_state.update(cx, |state, cx| {
                    if state.is_reorder_mode {
                        let _ = state.exit_reorder_mode();
                    } else {
                        state.is_reorder_mode = true;
                    }
                    cx.notify();
                });
            }))
            .scale_effect(1.0 - progress * 0.12);
        let back = has_forum.then(|| {
            let actions = self.clone();
            div()
                .absolute()
                .inset_0()
                .opacity(progress)
                .child(PageHeaderBackButton::new("chats-forum-back", move |_, cx| {
                    if progress > 0.92 {
                        actions.close_embedded_forum(cx);
                    }
                }))
                .scale_effect(0.88 + progress * 0.12)
        });
        div()
            .relative()
            .w(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .child(toggle)
            .children(back)
            .into_any_element()
    }

    fn chats_menu_actions(&self) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::with_capacity(3);
        if ChatPerformanceTrace::is_control_available() {
            let trace = ChatPerformanceTrace::shared();
            let symbol = if trace.is_recording() { "stop.circle.fill" } else { "waveform.path.ecg" };
            actions.push(
                HoldContextMenuAction::new("perf_trace", tr("nav_debug_perf"))
                    .system_image(symbol)
                    .disabled(trace.is_finishing()),
            );
        }
        actions.push(
            HoldContextMenuAction::new("create_chat", tr("create_chat_sheet_title")).system_image("square.and.pencil"),
        );
        actions.push(HoldContextMenuAction::new("message_requests", tr("message_requests_title")).system_image("tray"));
        actions
    }

    fn handle_chats_menu_action(&self, id: &str, window: &mut Window, cx: &mut App) {
        match id {
            "perf_trace" => {
                let trace = ChatPerformanceTrace::shared();
                if trace.is_finishing() {
                    return;
                }
                let view = self.view.clone();
                let bump = move |cx: &mut App| {
                    view.update(cx, |view, cx| {
                        view.performance_trace_revision = view.performance_trace_revision.wrapping_add(1);
                        cx.notify();
                    });
                };
                if trace.is_recording() {
                    let (sender, receiver) = oneshot::channel();
                    trace.stop(
                        "chat_list_menu",
                        Some(Box::new(move |url: Option<PathBuf>| {
                            let _ = sender.send(url);
                        })),
                    );
                    let bump = bump.clone();
                    cx.spawn(async move |cx| {
                        let Ok(url) = receiver.await else {
                            return;
                        };
                        cx.update(|cx| {
                            bump(cx);
                            if let Some(url) = url {
                                let _ = SystemShare::present(vec![ShareItem::File(url)], None, cx);
                            }
                        });
                    })
                    .detach();
                } else {
                    let _ = trace.start("chat-list", true, window, cx);
                }
                bump(cx);
            }
            "create_chat" => CreateChatSheetState::shared(cx).update(cx, |sheet, cx| sheet.open(cx)),
            "message_requests" => self.push(RootRoute::MessageRequests, cx),
            _ => {}
        }
    }

    fn chats_header_right(
        &self,
        forum_chat: Option<ChatInfo>,
        requests_count: i64,
        progress: f32,
        cx: &mut App,
    ) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let is_reorder_mode = self.state.read(cx).is_reorder_mode;
        let menu = (!is_reorder_mode).then(|| {
            let badge = (requests_count > 0).then(|| {
                let text = if requests_count > 99 { "99+".to_owned() } else { requests_count.to_string() };
                pass_through().absolute().top(px(-3.0)).right(px(-3.0)).child(
                    div()
                        .relative()
                        .min_w(px(16.0))
                        .min_h(px(16.0))
                        .px(px(4.0))
                        .capsule()
                        .bg(theme.primary._100)
                        .flex()
                        .items_center()
                        .justify_center()
                        .swiftui_font(Font::system(9.0, FontWeight::BOLD, FontDesign::Rounded))
                        .text_color(theme.content_on(theme.primary._100))
                        .child(SharedString::from(text))
                        .child(centred_stroke(theme.bg._100, 1.5, Shape::Capsule)),
                )
            });
            let hit_tests = forum_chat.is_none() && progress < 0.05;
            let opener = self.view.clone();
            let button = LiquidGlassButton::new("chats-header-menu", LiquidGlassButtonShape::Circle)
                .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
                .accessibility_label(Some(tr("more")))
                .on_press(move |_, cx| {
                    opener.update(cx, |view, cx| {
                        view.is_chats_menu_presented = true;
                        cx.notify();
                    });
                })
                .child(ChatsMenuGlyph::new());
            let (reader, writer) = (self.view.clone(), self.view.clone());
            let is_presented = Binding::new(
                move |cx: &App| reader.read(cx).is_chats_menu_presented,
                move |value: bool, _: &mut Window, cx: &mut App| {
                    writer.update(cx, |view, cx| {
                        view.is_chats_menu_presented = value;
                        cx.notify();
                    });
                },
            );
            let (menu_actions, action_handler) = (self.clone(), self.clone());
            let control = div()
                .id("chats-header-menu-tooltip")
                .relative()
                .tooltip_label(tr("more"))
                .child(
                    ChatsMenuGlassSchemePin::new(button).tap_context_menu(
                        "chats-header-tap-menu",
                        is_presented,
                        HoldContextMenuAlignment::Trailing,
                        false,
                        false,
                        false,
                        None,
                        Rc::new(move |_: &App| menu_actions.chats_menu_actions()),
                        Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                            action_handler.handle_chats_menu_action(&id, window, cx);
                        }),
                        Rc::new(|_: &mut Window, _: &mut App| {
                            PageHeaderToolbarSlot::new(ChatsMenuGlyph::new()).into_any_element()
                        }),
                    ),
                )
                .children(badge)
                .guide_anchor(RiseGuideAnchor::ChatsMenu);
            let control = if hit_tests { control.into_any_element() } else { pass_through().child(control).into_any_element() };
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_end()
                .opacity(1.0 - progress)
                .child(control)
                .scale_effect_anchored(1.0 - progress * 0.08, UnitPoint::TRAILING)
        });
        let forum = forum_chat.map(|forum_chat| {
            let actions = self.clone();
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_end()
                .opacity(progress)
                .child(
                    PageHeaderIconButton::new("chats-forum-more", "ellipsis", move |_, cx| {
                        if progress <= 0.92 {
                            return;
                        }
                        let chat_id = Some(forum_chat.id.clone());
                        if ChatProfilePermissions::can_manage(&forum_chat) {
                            actions.push(RootRoute::ChatSettings { chat_id }, cx);
                        } else {
                            actions.push(RootRoute::ChatProfile { chat_id }, cx);
                        }
                    })
                    .font_size(16.0),
                )
                .scale_effect(0.9 + progress * 0.1)
        });
        div()
            .relative()
            .w(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .children(menu)
            .children(forum)
            .into_any_element()
    }

    fn embedded_forum_container_width(&self, page_width: f32) -> f32 {
        if page_width > 0.0 { page_width } else { 320.0 }
    }

    /// beside the split list the center keeps the stack of whatever chat was open before
    /// a forum opened from a row or search folds into the list
    /// so a stack of another chat must go
    fn leave_foreign_center_stack(&self, forum_chat_id: &str, cx: &mut App) {
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        nav_store.update(cx, |nav_store, cx| {
            if !nav_store.chat_list_beside_stack() || nav_store.current_tab != MainTab::Chats {
                return;
            }
            let Some(bottom) = nav_store.path.first() else {
                return;
            };
            let owner = match bottom {
                RootRoute::Chat { chat_id }
                | RootRoute::ChatTopic { chat_id, .. }
                | RootRoute::ChatThread { chat_id, .. }
                | RootRoute::ForumHome { chat_id }
                | RootRoute::ForumAll { chat_id } => chat_id.as_deref(),
                _ => None,
            };
            if owner == Some(forum_chat_id) {
                return;
            }
            nav_store.set_sai_active_screen_overlay(
                Some(SaiActiveScreenOverlay {
                    tab: MainTab::Chats,
                    screen: RootRoute::ForumHome { chat_id: Some(forum_chat_id.to_owned()) }.sai_active_screen(),
                }),
                cx,
            );
            tracing::warn!(target: "riseonly::X2208", chat = forum_chat_id, "forum overlay set before leaving the centre stack");
            nav_store.pop_to_root(cx);
        });
    }

    fn open_embedded_forum(&self, chat: &ChatInfo, allows_detached_snapshot: bool, cx: &mut App) {
        let trace = ChatPerformanceTrace::shared();
        trace.record("forum.open_requested", HashMap::new());
        let operation = trace.begin_operation("forum.prepare", HashMap::new());
        self.view.update(cx, |view, cx| view.history_warmup.will_present_forum(&chat.id, cx));
        self.state.update(cx, |state, cx| {
            if state.exit_reorder_mode() {
                cx.notify();
            }
        });
        self.chrome_collapse.update(cx, |collapse, _| collapse.hold());
        let services = ChatServicesStore::shared(cx);
        let initial_snapshot = services.update(cx, |services, cx| services.forum_topic_list_snapshot(&chat.id, cx));
        self.embedded_forum_transition.update(cx, |transition, cx| transition.prepare_for_presentation(cx));
        self.view.update(cx, |view, cx| {
            view.embedded_forum_presentation_epoch = view.embedded_forum_presentation_epoch.wrapping_add(1);
            view.embedded_forum_chat_snapshot = Some(chat.clone());
            view.embedded_forum_initial_snapshot = Some(initial_snapshot);
            view.embedded_forum_allows_detached_snapshot = allows_detached_snapshot;
            view.embedded_forum_chat_id = Some(chat.id.clone());
            cx.notify();
        });
        trace.end_operation(operation, 0.0, HashMap::new());
    }

    fn is_current_presentation(&self, chat_id: &str, epoch: u64, cx: &App) -> bool {
        let view = self.view.read(cx);
        EmbeddedForumPresentationSessionPolicy::is_current(
            view.embedded_forum_chat_id.as_deref(),
            view.embedded_forum_presentation_epoch,
            chat_id,
            epoch,
        )
    }

    fn begin_embedded_forum_presentation(&self, chat_id: &str, epoch: u64, cx: &mut App) {
        if !self.is_current_presentation(chat_id, epoch, cx) || self.embedded_forum_transition.read(cx).is_presented() {
            return;
        }
        ChatPerformanceTrace::shared().record("forum.panel_mounted", HashMap::new());
        let (actions, chat_id) = (self.clone(), chat_id.to_owned());
        cx.defer(move |cx| {
            if !actions.is_current_presentation(&chat_id, epoch, cx)
                || actions.embedded_forum_transition.read(cx).is_presented()
            {
                return;
            }
            actions.embedded_forum_transition.update(cx, |transition, cx| transition.set_presented(true, cx));
            actions.settle_embedded_forum_presentation_after(
                chat_id,
                epoch,
                EmbeddedForumTransitionMetrics::BACKGROUND_WORK_DELAY_DURATION,
                cx,
            );
        });
    }

    fn settle_embedded_forum_presentation_after(&self, chat_id: String, epoch: u64, delay: f64, cx: &mut App) {
        let actions = self.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            cx.update(|cx| actions.settle_embedded_forum_presentation(chat_id, epoch, cx));
        })
        .detach();
    }

    fn settle_embedded_forum_presentation(&self, chat_id: String, epoch: u64, cx: &mut App) {
        if !self.is_current_presentation(&chat_id, epoch, cx) {
            return;
        }
        let transition = self.embedded_forum_transition.read(cx);
        if !transition.is_presented() {
            return;
        }
        if transition.is_interactive_gesture_active()
            || transition.is_interaction_settling()
            || transition.has_interactive_offset()
        {
            self.settle_embedded_forum_presentation_after(
                chat_id,
                epoch,
                EmbeddedForumTransitionMetrics::BACKGROUND_WORK_RETRY_INTERVAL,
                cx,
            );
            return;
        }
        self.embedded_forum_transition.update(cx, |transition, cx| transition.release_structural_updates_if_idle(cx));
        self.view.update(cx, |view, cx| view.history_warmup.did_settle_forum_presentation(&chat_id, cx));
    }

    fn close_embedded_forum(&self, cx: &mut App) {
        let (closing_chat_id, closing_epoch) = {
            let view = self.view.read(cx);
            (view.embedded_forum_chat_id.clone(), view.embedded_forum_presentation_epoch)
        };
        let Some(closing_chat_id) = closing_chat_id else {
            return;
        };
        self.chrome_collapse
            .update(cx, |collapse, cx| collapse.release(Some(EmbeddedForumTransitionMetrics::animation()), cx));
        self.embedded_forum_transition.update(cx, |transition, cx| {
            transition.set_presented(false, cx);
            transition.reset_drag_offset(cx);
        });
        let actions = self.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(EMBEDDED_FORUM_CLOSE_CLEANUP_DELAY)).await;
            cx.update(|cx| {
                let is_presented = actions.embedded_forum_transition.read(cx).is_presented();
                let should_cleanup = {
                    let view = actions.view.read(cx);
                    EmbeddedForumPresentationSessionPolicy::should_cleanup(
                        view.embedded_forum_chat_id.as_deref(),
                        view.embedded_forum_presentation_epoch,
                        is_presented,
                        &closing_chat_id,
                        closing_epoch,
                    )
                };
                if !should_cleanup {
                    return;
                }
                ChatPerformanceTrace::shared().record("forum.close_cleanup", HashMap::new());
                actions.view.update(cx, |view, cx| {
                    view.embedded_forum_chat_id = None;
                    view.embedded_forum_chat_snapshot = None;
                    view.embedded_forum_initial_snapshot = None;
                    view.embedded_forum_allows_detached_snapshot = false;
                    view.history_warmup.did_settle_forum_presentation(&closing_chat_id, cx);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn dismiss_embedded_forum_immediately(&self, cx: &mut App) {
        self.chrome_collapse.update(cx, |collapse, cx| collapse.release(None, cx));
        self.embedded_forum_transition.update(cx, |transition, cx| transition.dismiss_immediately(cx));
        self.view.update(cx, |view, cx| {
            let dismissed_chat_id = view.embedded_forum_chat_id.take();
            view.embedded_forum_presentation_epoch = view.embedded_forum_presentation_epoch.wrapping_add(1);
            view.embedded_forum_chat_snapshot = None;
            view.embedded_forum_initial_snapshot = None;
            view.embedded_forum_allows_detached_snapshot = false;
            if let Some(dismissed_chat_id) = dismissed_chat_id {
                view.history_warmup.did_settle_forum_presentation(&dismissed_chat_id, cx);
            }
            view.show_embedded_section_create = false;
            view.embedded_section_name.clear();
            cx.notify();
        });
    }

    fn should_materialize_tab_page(&self, page: &ChatsTabPage, index: i64, anchor_index: i64, cx: &App) -> bool {
        self.view.read(cx).retained_tab_page_ids.contains(&page.stable_id()) || (index - anchor_index).abs() <= 1
    }

    fn mount_anchor_index(&self, pages_count: i64, cx: &App) -> i64 {
        let last = (pages_count - 1).max(0);
        let active_tab = self.state.read(cx).active_tab.clamp(0, last);
        self.tab_pager_live_state.read(cx).resolved_mount_anchor(active_tab).clamp(0, last)
    }

    fn retain_tab_pages_around(&self, index: i64, pages: &[ChatsTabPage], cx: &mut App) {
        if pages.is_empty() {
            self.view.update(cx, |view, _| view.retained_tab_page_ids.clear());
            return;
        }
        let last = pages.len() as i64 - 1;
        let clamped = index.clamp(0, last);
        let lower = (clamped - 1).max(0);
        let upper = last.min(clamped + 1);
        self.page_viewports.capture_all(cx);
        self.page_viewports.retain(&pages.iter().map(ChatsTabPage::stable_id).collect());
        let retained = (lower..=upper).map(|index| pages[index as usize].stable_id()).collect::<HashSet<_>>();
        self.view.update(cx, |view, cx| {
            view.retained_tab_page_ids = retained;
            cx.notify();
        });
    }

    fn retain_tab_pages_through(&self, current_index: i64, target_index: i64, pages: &[ChatsTabPage], cx: &mut App) {
        if pages.is_empty() {
            return;
        }
        let last = pages.len() as i64 - 1;
        let current = current_index.clamp(0, last);
        let target = target_index.clamp(0, last);
        let lower = (current.min(target) - 1).max(0);
        let upper = last.min(current.max(target) + 1);
        self.update_retained_tab_pages(lower, upper, pages, cx);
    }

    fn update_retained_tab_pages(&self, lower: i64, upper: i64, pages: &[ChatsTabPage], cx: &mut App) {
        let valid_ids = pages.iter().map(ChatsTabPage::stable_id).collect::<HashSet<_>>();
        self.view.update(cx, |view, cx| {
            let mut updated =
                view.retained_tab_page_ids.intersection(&valid_ids).cloned().collect::<HashSet<String>>();
            updated.extend((lower..=upper).map(|index| pages[index as usize].stable_id()));
            if updated != view.retained_tab_page_ids {
                view.retained_tab_page_ids = updated;
                cx.notify();
            }
        });
    }

    fn chats_tab_page_view(&self, page: &ChatsTabPage, frame: ChatsPageFrame, cx: &mut App) -> AnyElement {
        match page {
            ChatsTabPage::All | ChatsTabPage::Folder(_) => {
                let folder = match page {
                    ChatsTabPage::Folder(folder) => Some(folder.clone()),
                    _ => None,
                };
                let page_id = page.stable_id();
                let restore_chat_ids = self.page_viewports.restoration_ids(&page_id);
                let active = frame.is_active_page || frame.rail_page_id.as_deref() == Some(page_id.as_str());
                let actions = self.clone();
                let reader_folder = folder.clone();
                let reader_id = SharedString::from(format!("chats-viewport-{page_id}"));
                RiseChatViewportReader::new(reader_id, RiseChatListKind::Inbox, move |viewport, _, cx| {
                    actions.chats_viewport_page(folder, page_id, frame, viewport, cx)
                })
                .folder(reader_folder)
                .active(active)
                .restore_chat_ids(restore_chat_ids)
                .into_any_element()
            }
            ChatsTabPage::CreatePrompt => self.create_folder_prompt_tab(cx),
        }
    }

    fn chats_viewport_page(
        &self,
        folder: Option<ChatFolder>,
        page_id: String,
        frame: ChatsPageFrame,
        viewport: Entity<RiseChatViewport>,
        cx: &mut App,
    ) -> AnyElement {
        let engine_archived = self.view.read(cx).engine_lists.as_ref().map(|lists| lists.archived_chats.clone());
        let snapshot = viewport.update(cx, |viewport, cx| viewport.snapshot(cx));
        let has_before = viewport.update(cx, |viewport, cx| viewport.has_before(cx));
        let in_lesson = GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).is_armed());
        let keyable = !in_lesson && (folder.is_none() || engine_archived.is_some());
        let model_key = keyable.then(|| ChatsPageModelKey {
            source: frame.source.clone(),
            archived: folder.as_ref().and(engine_archived.clone()),
            folder: folder.clone(),
            snapshot: snapshot.clone(),
            has_before,
        });
        let kept = model_key.as_ref().and_then(|key| {
            let models = self.page_models.borrow();
            let kept = models.get(&page_id);
            if kept.is_none_or(|model| !model.key.matches(key)) {
                tracing::warn!(
                    target: "riseonly::X1358",
                    page = page_id.as_str(),
                    reason = kept.map_or("dropped", |model| model.key.mismatch(key)),
                    "chats page model rebuilt"
                );
            }
            kept.filter(|model| model.key.matches(key)).cloned()
        });
        let model = match kept {
            Some(model) => model,
            None => {
                let cached = match &folder {
                    Some(folder) => {
                        let services = ChatServicesStore::shared(cx);
                        let merged = match &engine_archived {
                            Some(archived) => self.archive_merged(archived, &frame.source),
                            None => {
                                let archived = services.update(cx, |services, cx| services.archived_chats(cx));
                                Rc::new(ChatListUtils::merging(&archived, &frame.source))
                            }
                        };
                        Rc::new(services.read(cx).chats(folder, &merged))
                    }
                    None => frame.source.clone(),
                };
                let model = self.build_page_model(folder.as_ref(), &cached, snapshot.clone(), has_before, &viewport, cx);
                if let Some(key) = model_key {
                    self.page_models.borrow_mut().insert(page_id.clone(), ChatsPageModel { key, ..model.clone() });
                }
                model
            }
        };
        self.chats_all_tab(
            ChatsAllTabArgs {
                display_chats: model.display_chats,
                filtered_is_empty: model.filtered_is_empty,
                drafts: model.drafts,
                source: model.page_source,
                is_loading: frame.is_loading,
                shows_archive_reveal: folder.is_none() && !has_before,
                folder_id: folder.as_ref().map(|folder| folder.id.clone()),
                is_active_page: frame.is_active_page,
                rows_active: frame.is_active_page
                    || frame.selected_page_id.as_deref() == Some(page_id.as_str())
                    || frame.rail_page_id.as_deref() == Some(page_id.as_str()),
                avatar_prefetch_page_id: page_id,
                collapse_range: frame.collapse_range,
                has_embedded_forum: frame.has_embedded_forum,
                restoration_ready: snapshot.is_some(),
                has_before,
            },
            viewport,
            cx,
        )
    }

    fn archive_merged(&self, archived: &Rc<Vec<ChatInfo>>, source: &Rc<Vec<ChatInfo>>) -> Rc<Vec<ChatInfo>> {
        let mut kept = self.archive_merge.borrow_mut();
        if let Some(merge) = kept.as_ref()
            && Rc::ptr_eq(&merge.archived, archived)
            && Rc::ptr_eq(&merge.source, source)
        {
            return merge.merged.clone();
        }
        let merged = Rc::new(ChatListUtils::merging(archived, source));
        *kept = Some(ChatsArchiveMerge { archived: archived.clone(), source: source.clone(), merged: merged.clone() });
        merged
    }

    fn build_page_model(
        &self,
        folder: Option<&ChatFolder>,
        cached: &Rc<Vec<ChatInfo>>,
        snapshot: Option<Rc<RiseChatViewportSnapshot>>,
        has_before: bool,
        viewport: &Entity<RiseChatViewport>,
        cx: &mut App,
    ) -> ChatsPageModel {
        let window_chats = if snapshot.is_none() {
            let limit = usize::try_from(RiseChatRepository::default_page_limit()).unwrap_or_default();
            cached.iter().take(limit).cloned().collect::<Vec<_>>()
        } else {
            viewport.update(cx, |viewport, cx| viewport.chats(cx))
        };
        let local_only = cached
            .iter()
            .filter(|chat| {
                (GetSaiServerId::is_temp_id(Some(&chat.id))
                    || UserServicesStore::is_contact_join_virtual_chat_id(&chat.id))
                    && !window_chats.iter().any(|candidate| candidate.id == chat.id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let listed = if has_before || local_only.is_empty() {
            window_chats
        } else {
            ChatListUtils::merging(&local_only, &window_chats)
        };
        let guided = match GuideSandbox::existing(cx) {
            Some(sandbox) if folder.is_none() && !has_before => sandbox.read(cx).listed_chats(listed),
            _ => listed,
        };
        let page_source = match folder {
            Some(folder) => Self::folder_pinned_order(guided, folder, cached, !has_before),
            None => guided,
        };
        let filtered = ChatsScreenState::visible_chats(&self.state, "", &page_source, cx);
        let drafts = ChatDraftStore::shared(cx).read(cx).previews().clone();
        let mut seen_ids = HashSet::new();
        let filtered_is_empty = filtered.is_empty();
        let ordered = ChatDraftOrdering::applied(filtered, &drafts);
        let display_chats =
            Rc::new(ordered.into_iter().filter(|chat| seen_ids.insert(chat.id.clone())).collect::<Vec<_>>());
        ChatsPageModel {
            key: ChatsPageModelKey {
                source: cached.clone(),
                archived: None,
                folder: folder.cloned(),
                snapshot,
                has_before,
            },
            page_source: Rc::new(page_source),
            display_chats,
            filtered_is_empty,
            drafts: Rc::new(drafts),
        }
    }

    fn folder_pinned_order(rows: Vec<ChatInfo>, folder: &ChatFolder, cached: &[ChatInfo], at_head: bool) -> Vec<ChatInfo> {
        let mut pin_index: HashMap<&str, i64> = HashMap::with_capacity(folder.pinned_chat_ids.len());
        for (offset, id) in folder.pinned_chat_ids.iter().enumerate() {
            pin_index.entry(id.as_str()).or_insert(offset as i64);
        }
        let any_pinned = rows.iter().any(|chat| chat.is_pinned);
        let mut pinned = Vec::new();
        let mut rest = Vec::new();
        for mut chat in rows.iter().cloned() {
            if let Some(position) = pin_index.get(chat.id.as_str()).copied() {
                chat.is_pinned = true;
                chat.pin_position = Some(position);
                pinned.push(chat);
            } else {
                chat.is_pinned = false;
                chat.pin_position = None;
                rest.push(chat);
            }
        }
        if at_head && pinned.len() < pin_index.len() {
            let present: HashSet<String> = pinned.iter().map(|chat: &ChatInfo| chat.id.clone()).collect();
            for chat in cached {
                let Some(position) = pin_index.get(chat.id.as_str()).copied() else {
                    continue;
                };
                if present.contains(&chat.id) {
                    continue;
                }
                let mut chat = chat.clone();
                chat.is_pinned = true;
                chat.pin_position = Some(position);
                pinned.push(chat);
            }
        }
        if pinned.is_empty() && !any_pinned {
            return rows;
        }
        let tail = ChatListUtils::sort(&rest);
        if !at_head {
            return tail;
        }
        pinned.sort_by_key(|chat| chat.pin_position.unwrap_or(0));
        pinned.extend(tail);
        pinned
    }

    fn create_folder_prompt_tab(&self, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let actions = self.clone();
        let underlap = main_tab_bar_underlap(cx).max(32.0);
        let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            actions.push(RootRoute::ChatFoldersSettings, cx);
        });
        scroll_view("chats-create-folder-prompt")
            .list_shell_scroll_under_chrome(cx)
            .shows_indicators(false)
            .size_full()
            .child(
                vstack(16.0)
                    .w_full()
                    .items_center()
                    .pt(px(60.0))
                    .child(
                        div()
                            .w(px(92.0))
                            .h(px(92.0))
                            .circle()
                            .bg(theme.primary._100.opacity(0.14))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(
                                "folder.fill.badge.plus",
                                px(38.0),
                                theme.primary._100,
                                false,
                            )),
                    )
                    .child(
                        div().px(px(36.0)).child(
                            MainText::new(tr("folder_empty_prompt"))
                                .px(15.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.secondary)
                                .number_of_lines(3),
                        ),
                    )
                    .child(
                        div()
                            .px(px(22.0))
                            .py(px(12.0))
                            .capsule()
                            .bg(theme.primary._100)
                            .child(
                                MainText::new(tr("folder_create"))
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.content_on(theme.primary._100)),
                            )
                            .liquid_press(
                                "chats-create-folder",
                                LiquidPressShape::Capsule,
                                true,
                                false,
                                false,
                                Some(action),
                            ),
                    ),
            )
            .child(div().w_full().h(px(underlap)))
            .into_any_element()
    }

    fn chats_all_tab(
        &self,
        args: ChatsAllTabArgs,
        viewport: Entity<RiseChatViewport>,
        cx: &mut App,
    ) -> AnyElement {
        let border = rise_ui::theme(cx).border._100;
        let open_fill = rise_ui::theme(cx).primary._100.opacity(0.15);
        let open_chat_id = self.split_open_chat_id(cx);
        let pinned_count = args.source.iter().filter(|chat| chat.is_pinned).count() as i64;
        let filtered_is_empty = args.filtered_is_empty;
        let display_chats = args.display_chats;
        let (is_reorder_mode, selected_for_reorder, search_overlay) = {
            let state = self.state.read(cx);
            (state.is_reorder_mode, state.selected_for_reorder.clone(), state.search_overlay)
        };
        let pinned_ids = Rc::new(if is_reorder_mode {
            display_chats.iter().filter(|chat| chat.is_pinned).map(|chat| chat.id.clone()).collect::<Vec<_>>()
        } else {
            Vec::new()
        });
        let services = ChatServicesStore::shared(cx);
        let engine_lists = self.view.read(cx).engine_lists.clone();
        let (archived_chats, archived_count, archived_unread_chats_count) = if !args.shows_archive_reveal {
            (Rc::new(Vec::new()), 0, 0)
        } else if let Some(lists) = engine_lists.as_deref() {
            (lists.archived_chats.clone(), lists.archived_count, lists.archived_unread_chats_count)
        } else {
            services.update(cx, |services, cx| {
                (
                    Rc::new(services.archived_chats(cx)),
                    services.archived_chats_total_count(cx),
                    services.archived_chats_unread_chats_count(cx),
                )
            })
        };
        let pins_archive = archived_count > 0 && !(display_chats.is_empty() && args.is_loading);
        let is_dragging = self.reorder.read(cx).dragging_chat_id.is_some();
        let archive_gate_live = args.shows_archive_reveal
            && self.state.read(cx).active_tab == 0
            && !search_overlay
            && !is_reorder_mode
            && !is_dragging
            && !args.has_embedded_forum;
        let view = self.view.read(cx);
        if args.shows_archive_reveal {
            view.hold_menu_gates_archive.set(archive_gate_live);
        }
        let archive_interactions_enabled = archive_gate_live && !view.hold_menu_active;

        let (render_status, render_error) = if let Some(lists) = engine_lists.as_deref() {
            (lists.inbox_render_status, lists.inbox_render_error.clone())
        } else {
            let slice = ChatActionsStore::shared(cx).read(cx).get_chats_sai.clone();
            (slice.status(), slice.error())
        };
        let is_empty = filtered_is_empty && !args.shows_archive_reveal;
        let needs_pending = args.source.is_empty() && args.is_loading;
        let page_id = args.avatar_prefetch_page_id.clone();
        let restoration_ready = args.restoration_ready || !self.page_viewports.has_bookmark(&page_id);
        let practice_listed = GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_mute_scene());

        let list_frame = ChatsListFrame {
            viewport,
            display_chats,
            source: args.source,
            page_id: page_id.clone(),
            is_active_page: args.is_active_page,
            has_before: args.has_before,
            practice_listed,
            guide_chat_id: GuideSandbox::chat_id(),
            is_reorder_mode,
            selected_for_reorder,
            pinned_count,
            pinned_ids,
            folder_id: args.folder_id.clone(),
            separator: border,
            open_chat_id,
            open_fill,
        };
        let list_args = ChatsListArgs {
            drafts: args.drafts,
            shows_archive_reveal: args.shows_archive_reveal,
            pins_archive,
            archived_chats,
            archived_count,
            archived_unread_chats_count,
            archive_interactions_enabled,
            restoration_ready,
            collapse_range: args.collapse_range,
            rows_active: args.rows_active,
            is_dragging,
        };

        let actions = self.clone();
        let error_text = render_error.clone();
        let empty_id = page_id.clone();
        SaiDataRender::new(render_status, move |_, _| {
            ChatShellInsetsReader::new(move |_, floating_top_inset| {
                container_query(move |size: Size<Pixels>, window: &mut Window, cx: &mut App| {
                    actions.native_list(list_frame, list_args, floating_top_inset, f32::from(size.height), window, cx)
                })
                .into_any_element()
            })
        })
        .id(SharedString::from(format!("chats-render-{page_id}")))
        .is_empty(is_empty)
        .no_data_text(tr("no_chats"))
        .error_text(error_text)
        .need_pending(needs_pending)
        .message_height_percent(20.0)
        .no_data_height_percent(20.0)
        .sai_data_render_loading(|_, cx| {
            scroll_view("chats-skeleton")
                .list_shell_scroll_under_chrome(cx)
                .scroll_disabled(true)
                .shows_indicators(false)
                .size_full()
                .child(ChatListSkeleton::new())
        })
        .sai_data_render_empty(move |_, cx| {
            ChatsBelowChrome.body(
                SaiDataRenderDefaultsEmpty {
                    text: tr("no_chats"),
                    no_data_height_percent: 20.0,
                    geometry_key: SharedString::from(format!("chats-empty-{empty_id}")).into(),
                },
                cx,
            )
        })
        .sai_data_render_error(move |_, cx| {
            ChatsBelowChrome.body(
                SaiDataRenderDefaultsError {
                    text: render_error.clone().unwrap_or_else(|| tr("error_fetching_data")),
                    message_height_percent: 20.0,
                },
                cx,
            )
        })
        .into_any_element()
    }

    fn native_list(
        &self,
        frame: ChatsListFrame,
        args: ChatsListArgs,
        floating_top_inset: f32,
        geometry_height: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let page_id = frame.page_id.clone();
        let control = self.page_viewports.control(&page_id);
        let first_rows = control.first_rows(cx);
        let host = window.current_view();
        let observed_first_rows = first_rows.clone();
        let _first_rows_observer = window.use_keyed_state(
            SharedString::from(format!("chats-first-rows-observer-{page_id}")),
            cx,
            move |_, cx| -> Subscription { App::observe(cx, &observed_first_rows, move |_, cx: &mut App| cx.notify(host)) },
        );
        let first_rows_installed = first_rows.read(cx).installed();
        let rows_held = self.view.read(cx).rows_held_for_entry;
        if rows_held {
            let held_view = self.view.clone();
            window.on_next_frame(move |_, cx| {
                held_view.update(cx, |view, cx| {
                    if view.rows_held_for_entry {
                        view.rows_held_for_entry = false;
                        cx.notify();
                    }
                });
            });
        }
        let viewport_revision = frame.viewport.read(cx).revision();
        let kept_rows = self.page_rows.borrow().get(&page_id).and_then(|memo| {
            memo.matches(&frame, &args.drafts, viewport_revision)
                .then(|| (memo.rows.clone(), memo.visible_index_by_id.clone()))
        });
        let (rows, visible_index_by_id) = match kept_rows {
            Some(kept) => kept,
            None => {
                let shared_actions = Rc::new(self.clone());
                let shared_frame = Rc::new(frame.clone());
                let rows = Rc::new(
                    frame
                        .display_chats
                        .iter()
                        .enumerate()
                        .map(|(index, chat)| {
                            Self::row(&shared_actions, &shared_frame, index, chat, args.drafts.get(&chat.id).cloned(), cx)
                        })
                        .collect::<Vec<_>>(),
                );
                let visible_index_by_id = Rc::new(
                    frame
                        .display_chats
                        .iter()
                        .enumerate()
                        .map(|(index, chat)| (chat.id.clone(), index as i64))
                        .collect::<HashMap<_, _>>(),
                );
                self.page_rows.borrow_mut().insert(
                    page_id.clone(),
                    ChatsRowsMemo {
                        frame: frame.clone(),
                        drafts: args.drafts.clone(),
                        viewport_revision,
                        rows: rows.clone(),
                        visible_index_by_id: visible_index_by_id.clone(),
                    },
                );
                (rows, visible_index_by_id)
            }
        };
        let header = args.shows_archive_reveal.then(|| self.list_header(&frame, &args, cx));
        let rests_behind_pull = args.shows_archive_reveal && !args.pins_archive;
        let archive_offset = if rests_behind_pull { ChatsArchiveRevealPolicy::ROW_HEIGHT } else { 0.0 };
        let footer_height = main_tab_bar_underlap(cx).max(32.0);

        if args.shows_archive_reveal {
            self.install_archive_changes(&page_id, &control, &args, window, cx);
        }

        let (metrics_state, metrics_control) = (self.state.clone(), control.clone());
        let shows_metrics = rests_behind_pull;
        let visible_viewport = frame.viewport.clone();
        let (visible_view, visible_page_id, visible_chats) =
            (self.view.clone(), page_id.clone(), frame.display_chats.clone());
        let is_active_page = frame.is_active_page;

        let list = RiseChatNativeList::new(SharedString::from(format!("chats-list-{page_id}")), rows)
            .header(header)
            .footer_height(footer_height)
            .minimum_content_height(geometry_height - floating_top_inset + args.collapse_range + archive_offset)
            .top_inset(floating_top_inset)
            .initial_top_offset(archive_offset)
            .scroll_enabled(!args.is_dragging)
            .is_active(is_active_page)
            .rows_active(args.rows_active)
            .restoration_ready(args.restoration_ready)
            .page_id(page_id)
            .hold_rows(rows_held)
            .bookmarks(Some(self.page_viewports.clone()))
            .control(Some(control))
            .collapse(Some(self.chrome_collapse.clone()))
            .archive_events(Some(self.archive_pull_events(args.archive_interactions_enabled)))
            .archive_enabled(args.archive_interactions_enabled && !args.pins_archive)
            .on_metrics(move |metrics, window, cx| {
                if !shows_metrics {
                    return;
                }
                let Some(marker_y) = metrics_control.archive_marker_y(cx) else {
                    return;
                };
                let normalize = metrics_state.update(cx, |state, cx| {
                    if (marker_y - state.archive_marker_y).abs() > ChatsArchiveRevealPolicy::MARKER_EPSILON {
                        state.archive_marker_y = marker_y;
                        cx.notify();
                    }
                    if ChatsArchiveRevealPolicy::should_hide(state.archive_reveal_phase, marker_y)
                        || (state.archive_reveal_phase == ChatsArchiveRevealPhase::Pulling
                            && marker_y <= 0.0
                            && !state.archive_gesture_active
                            && !state.archive_settling)
                    {
                        state.archive_reveal_phase = ChatsArchiveRevealPhase::Hidden;
                        cx.notify();
                    }
                    !metrics.is_tracking
                        && !metrics.is_dragging
                        && !metrics.is_decelerating
                        && state.archive_reveal_phase == ChatsArchiveRevealPhase::Hidden
                        && marker_y > ChatsArchiveRevealPolicy::TOP_TOLERANCE
                        && !state.archive_pull_eligible
                        && !state.archive_gesture_active
                        && !state.archive_settling
                });
                if normalize {
                    metrics_control.normal_top(window, cx);
                }
            })
            .on_visible(move |ids, _, cx| {
                visible_viewport.update(cx, |viewport, cx| viewport.visible(ids.to_vec(), cx));
                if !is_active_page {
                    return;
                }
                let Some(index) = ids.iter().filter_map(|id| visible_index_by_id.get(id).copied()).next_back() else {
                    return;
                };
                visible_view.update(cx, |view, cx| {
                    view.avatar_prefetch.update(&visible_page_id, index, &visible_chats, cx);
                    view.history_warmup.activate(&visible_page_id, cx);
                    view.history_warmup.update(&visible_page_id, index, &visible_chats, cx);
                });
            });

        let shows_skeleton =
            !first_rows_installed && (!args.restoration_ready || !frame.display_chats.is_empty());
        let skeleton = shows_skeleton.then(|| {
            div().absolute().top(px(floating_top_inset)).left_0().right_0().child(ChatListSkeleton::new())
        });
        div().relative().size_full().child(list).children(skeleton).into_any_element()
    }

    fn install_archive_changes(
        &self,
        page_id: &str,
        control: &Rc<RiseChatNativeListControl>,
        args: &ChatsListArgs,
        window: &mut Window,
        cx: &mut App,
    ) {
        let pins_archive = args.pins_archive;
        let (state, enabled_control) = (self.state.clone(), control.clone());
        on_change(
            SharedString::from(format!("chats-archive-enabled-{page_id}")),
            args.archive_interactions_enabled,
            false,
            window,
            cx,
            move |_, enabled, window, cx| {
                if pins_archive || *enabled {
                    return;
                }
                let normalize = state.update(cx, |state, cx| {
                    let normalize = ChatsArchiveScrollResetPolicy::should_normalize_to_normal_top(
                        state.archive_reveal_phase,
                        state.archive_marker_y,
                    );
                    state.archive_reveal_phase = ChatsArchiveRevealPhase::Hidden;
                    state.archive_pull_eligible = false;
                    cx.notify();
                    normalize
                });
                if normalize {
                    enabled_control.normal_top(window, cx);
                }
            },
        );

        let (state, pins_control) = (self.state.clone(), control.clone());
        on_change(
            SharedString::from(format!("chats-pins-archive-{page_id}")),
            pins_archive,
            false,
            window,
            cx,
            move |_, _, window, cx| {
                state.update(cx, |state, cx| {
                    state.archive_reveal_phase = ChatsArchiveRevealPhase::Hidden;
                    state.archive_pull_eligible = false;
                    cx.notify();
                });
                window.defer(cx, move |window, cx| {
                    let Some(list) = pins_control.list() else {
                        return;
                    };
                    let metrics = list.read(cx).scroll_metrics();
                    if metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating {
                        return;
                    }
                    pins_control.normal_top(window, cx);
                });
            },
        );
    }

    fn list_header(&self, frame: &ChatsListFrame, args: &ChatsListArgs, cx: &mut App) -> RiseChatNativeListRow {
        let display_is_empty = frame.display_chats.is_empty();
        let build = {
            let (state, actions) = (self.state.clone(), self.clone());
            let (pins_archive, archived_chats, archived_count, archived_unread) = (
                args.pins_archive,
                args.archived_chats.clone(),
                args.archived_count,
                args.archived_unread_chats_count,
            );
            Rc::new(move |_: &mut Window, _: &mut App| {
                let row_actions = actions.clone();
                let empty_state = display_is_empty.then(ChatsInlineEmptyState::new);
                vstack(0.0)
                    .w_full()
                    .child(ChatsArchiveRevealRow::new(
                        state.clone(),
                        pins_archive,
                        (*archived_chats).clone(),
                        archived_count,
                        archived_unread,
                        move |_, cx| row_actions.push(RootRoute::ArchivedChats, cx),
                    ))
                    .child(div().w_full().flex_none().h(px(1.0)))
                    .children(empty_state)
                    .into_any_element()
            })
        };
        let mut version = DefaultHasher::new();
        args.archived_count.hash(&mut version);
        args.archived_unread_chats_count.hash(&mut version);
        display_is_empty.hash(&mut version);
        args.pins_archive.hash(&mut version);
        for chat in args.archived_chats.iter().take(3) {
            chat.id.hash(&mut version);
            ChatListPreviewFactory::chat_title(chat, cx).hash(&mut version);
        }
        RiseChatNativeListRow {
            id: "chats-header".to_owned(),
            version: version.finish() as i64,
            content: build.clone(),
            measurement: build,
            geometry_version: Some(i64::from(display_is_empty)),
        }
    }

    fn row(
        actions: &Rc<ChatsActions>,
        frame: &Rc<ChatsListFrame>,
        index: usize,
        chat: &ChatInfo,
        draft: Option<ChatDraftPreview>,
        cx: &mut App,
    ) -> RiseChatNativeListRow {
        let is_selected = frame.selected_for_reorder.contains(&chat.id);
        let is_first = index == 0 && !frame.has_before;
        let is_open = frame.open_chat_id.as_deref() == Some(chat.id.as_str());

        let content = {
            let (actions, frame, draft) = (actions.clone(), frame.clone(), draft.clone());
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let chat = &frame.display_chats[index];
                let carries_anchor =
                    if frame.practice_listed { chat.id == frame.guide_chat_id } else { is_first };
                let (press_actions, press_chat, press_viewport) =
                    (actions.clone(), chat.clone(), frame.viewport.clone());
                let (hold_actions, hold_chat, hold_viewport, hold_source, hold_folder) = (
                    actions.clone(),
                    chat.clone(),
                    frame.viewport.clone(),
                    frame.source.clone(),
                    frame.folder_id.clone(),
                );
                let (select_actions, select_chat) = (actions.clone(), chat.clone());
                let (changed_reorder, changed_id, changed_page, changed_pinned) =
                    (actions.reorder.clone(), chat.id.clone(), frame.page_id.clone(), frame.pinned_ids.clone());
                let changed_control = actions.page_viewports.control(&frame.page_id);
                let ended_actions = actions.clone();
                let is_pinned = chat.is_pinned;

                let item = ChatItem::new(SharedString::from(format!("chats-row-{}-{}", frame.page_id, chat.id)), chat.clone())
                    .draft(draft.clone())
                    .guide_anchor(carries_anchor.then_some(RiseGuideAnchor::ChatsFirstChat))
                    .on_press(move |_, cx| {
                        press_viewport.update(cx, |viewport, cx| viewport.cache_detail(&press_chat, cx));
                        press_actions.on_row_press(&press_chat, cx);
                    })
                    .on_hold_action(move |action_id, _, cx| {
                        hold_viewport.update(cx, |viewport, cx| viewport.cache_detail(&hold_chat, cx));
                        if hold_source.iter().any(|candidate| candidate.id == hold_chat.id) {
                            ChatsScreenState::apply_hold_action(
                                &hold_actions.state,
                                &hold_chat.id,
                                action_id,
                                &hold_source,
                                hold_folder.as_deref(),
                                cx,
                            );
                        } else {
                            ChatsScreenState::apply_hold_action(
                                &hold_actions.state,
                                &hold_chat.id,
                                action_id,
                                std::slice::from_ref(&hold_chat),
                                hold_folder.as_deref(),
                                cx,
                            );
                        }
                    })
                    .custom_hold_actions(UserServicesStore::is_contact_join_virtual_chat_id(&chat.id).then(Vec::new))
                    .supports_selection(true)
                    .reorder_mode(frame.is_reorder_mode)
                    .pinned_count(frame.pinned_count)
                    .is_selected(is_selected)
                    .on_toggle_select(move |_, cx| select_actions.toggle_select(&select_chat, cx))
                    .on_reorder_changed(is_pinned.then(|| {
                        Rc::new(move |translation_y: f32, _: &mut Window, cx: &mut App| {
                            changed_reorder.update(cx, |reorder, cx| {
                                reorder.update(&changed_id, &changed_page, translation_y, &changed_pinned, &changed_control, cx);
                            });
                        }) as Rc<dyn Fn(f32, &mut Window, &mut App)>
                    }))
                    .on_reorder_ended(is_pinned.then(|| {
                        Rc::new(move |_: &mut Window, cx: &mut App| ended_actions.end_reorder_drag(cx))
                            as Rc<dyn Fn(&mut Window, &mut App)>
                    }));
                let moving = ChatsPinnedReorderMotion {
                    state: actions.reorder.clone(),
                    page_id: frame.page_id.clone(),
                    chat_id: chat.id.clone(),
                    measures_height: frame.is_reorder_mode && chat.is_pinned && frame.is_active_page,
                    content: div()
                        .relative()
                        .w_full()
                        .child(item)
                        .when(is_open, |row| {
                            row.child(div().absolute().inset_0().bg(frame.open_fill))
                        })
                        .into_any_element(),
                };
                let separator = EmbeddedForumRailFade.body(
                    div()
                        .w_full()
                        .flex_none()
                        .h(px(ChatListRowMetrics::SEPARATOR_HEIGHT))
                        .bg(frame.separator)
                        .opacity(if frame.is_reorder_mode { 0.0 } else { 1.0 })
                        .into_any_element(),
                    cx,
                );
                vstack(0.0).w_full().child(moving).child(separator).into_any_element()
            })
        };
        let measurement = {
            let (frame, draft) = (frame.clone(), draft.clone());
            Rc::new(move |_: &mut Window, _: &mut App| {
                let chat = &frame.display_chats[index];
                vstack(0.0)
                    .w_full()
                    .child(
                        ChatItem::new(
                            SharedString::from(format!("chats-row-measure-{}-{}", frame.page_id, chat.id)),
                            chat.clone(),
                        )
                        .draft(draft.clone())
                        .is_active(false)
                        .measurement_only(true)
                        .reorder_mode(frame.is_reorder_mode)
                        .is_selected(is_selected),
                    )
                    .child(div().w_full().flex_none().h(px(ChatListRowMetrics::SEPARATOR_HEIGHT)))
                    .into_any_element()
            })
        };

        let mut geometry = DefaultHasher::new();
        frame.viewport.read(cx).geometry_version(chat).hash(&mut geometry);
        if let Some(draft) = &draft {
            draft.text.hash(&mut geometry);
        }
        frame.is_reorder_mode.hash(&mut geometry);
        is_selected.hash(&mut geometry);
        if chat.r#type == ChatType::Direct {
            ChatListPreviewFactory::chat_title(chat, cx).hash(&mut geometry);
        }
        let geometry_version = geometry.finish() as i64;
        let mut version = DefaultHasher::new();
        geometry_version.hash(&mut version);
        frame.viewport.read(cx).content_version(chat).hash(&mut version);
        if let Some(draft) = &draft {
            draft.updated_at.to_bits().hash(&mut version);
        }
        (frame.is_reorder_mode && frame.is_active_page).hash(&mut version);
        frame.pinned_count.hash(&mut version);
        is_first.hash(&mut version);
        is_open.hash(&mut version);
        (frame.practice_listed && chat.id == frame.guide_chat_id).hash(&mut version);
        frame.pinned_ids.hash(&mut version);
        frame.open_fill.hash(&mut version);
        frame.separator.hash(&mut version);
        RiseChatNativeListRow {
            id: chat.id.clone(),
            version: version.finish() as i64,
            content,
            measurement,
            geometry_version: Some(geometry_version),
        }
    }

    fn on_row_press(&self, chat: &ChatInfo, cx: &mut App) {
        if UserServicesStore::is_contact_join_virtual_chat_id(&chat.id) {
            if self.state.read(cx).pending_chat_open_id.is_some() {
                return;
            }
            let chat_id = chat.id.clone();
            self.state.update(cx, |state, cx| {
                state.pending_chat_open_id = Some(chat_id.clone());
                cx.notify();
            });
            let (open_actions, open_id) = (self.clone(), chat_id.clone());
            let (error_actions, error_id) = (self.clone(), chat_id);
            UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.open_contact_join_virtual_chat(
                    chat,
                    move |real_chat_id, cx| {
                        if !open_actions.finish_pending_chat_open(&open_id, cx) {
                            return;
                        }
                        let is_chats_root = open_actions.nav_store.as_ref().is_some_and(|nav_store| {
                            let nav_store = nav_store.read(cx);
                            nav_store.current_tab == MainTab::Chats
                                && (nav_store.path.is_empty() || nav_store.chat_list_beside_stack())
                        });
                        if !real_chat_id.is_empty() && is_chats_root {
                            open_actions.open_chat(RootRoute::Chat { chat_id: Some(real_chat_id) }, cx);
                        }
                    },
                    move |error, cx| {
                        if !error_actions.finish_pending_chat_open(&error_id, cx) {
                            return;
                        }
                        Notifier::send(
                            tr("contact_join_chat_open_failed"),
                            NotifierSendOptions {
                                content: Some(error),
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Bottom,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    },
                    cx,
                );
            });
            return;
        }
        if chat.topics_enabled {
            self.leave_foreign_center_stack(&chat.id, cx);
            let already_open = self.view.read(cx).embedded_forum_chat_id.as_deref() == Some(chat.id.as_str())
                && self.embedded_forum_transition.read(cx).is_presented();
            if already_open {
                return;
            }
            self.open_embedded_forum(chat, false, cx);
            return;
        }
        if self.state.read(cx).pending_chat_open_id.is_some() {
            return;
        }
        let is_chats_tab =
            self.nav_store.as_ref().is_some_and(|nav_store| nav_store.read(cx).current_tab == MainTab::Chats);
        if is_chats_tab {
            self.open_chat(RootRoute::Chat { chat_id: Some(chat.id.clone()) }, cx);
            let beside_stack =
                self.nav_store.as_ref().is_some_and(|nav_store| nav_store.read(cx).chat_list_beside_stack());
            if beside_stack && self.view.read(cx).embedded_forum_chat_id.is_some() {
                self.close_embedded_forum(cx);
            }
        }
    }

    fn split_open_chat_id(&self, cx: &App) -> Option<String> {
        let nav_store = self.nav_store.as_ref()?.read(cx);
        if !nav_store.chat_list_beside_stack() || nav_store.current_tab != MainTab::Chats {
            return None;
        }
        if let Some(forum_id) = self.view.read(cx).embedded_forum_chat_id.clone()
            && self.embedded_forum_transition.read(cx).is_presented()
        {
            return Some(forum_id);
        }
        nav_store.path.iter().rev().find_map(|route| match route {
            RootRoute::Chat { chat_id } | RootRoute::ChatTopic { chat_id, .. } => chat_id.clone(),
            _ => None,
        })
    }

    fn finish_pending_chat_open(&self, chat_id: &str, cx: &mut App) -> bool {
        self.state.update(cx, |state, cx| {
            if state.pending_chat_open_id.as_deref() != Some(chat_id) {
                return false;
            }
            state.pending_chat_open_id = None;
            cx.notify();
            true
        })
    }

    fn archive_pull_events(&self, interactions_enabled: bool) -> ChatsArchivePullEvents {
        let (exposure_state, began_state, changed_state, ended_state, rested_state) =
            (self.state.clone(), self.state.clone(), self.state.clone(), self.state.clone(), self.state.clone());
        ChatsArchivePullEvents {
            allows_exposure: Rc::new(move |cx: &App| {
                exposure_state.read(cx).archive_reveal_phase != ChatsArchiveRevealPhase::Hidden
            }),
            on_began: Rc::new(move |marker_y: f32, cx: &mut App| {
                began_state.update(cx, |state, cx| {
                    state.cancel_archive_settle();
                    state.archive_gesture_active = true;
                    state.archive_gesture_eligibility_resolved = false;
                    state.archive_gesture_start_marker_y = marker_y;
                    state.archive_pull_eligible = false;
                    cx.notify();
                });
            }),
            on_changed: Rc::new(move |translation: Point<f32>, cx: &mut App| {
                changed_state.update(cx, |state, cx| {
                    if state.archive_gesture_eligibility_resolved
                        || translation.y.abs().max(translation.x.abs())
                            < ChatsArchiveRevealPolicy::GESTURE_MINIMUM_DISTANCE
                    {
                        return;
                    }
                    state.archive_gesture_eligibility_resolved = true;
                    cx.notify();
                    if translation.y <= 0.0
                        || translation.y.abs() <= translation.x.abs()
                        || !ChatsArchiveRevealPolicy::can_begin_pull(
                            state.archive_reveal_phase,
                            state.archive_gesture_start_marker_y,
                            interactions_enabled,
                        )
                    {
                        state.archive_pull_eligible = false;
                        return;
                    }
                    state.archive_pull_eligible = true;
                    state.archive_reveal_phase = ChatsArchiveRevealPhase::Pulling;
                });
            }),
            on_ended: Rc::new(move |marker_y: f32, velocity_y: f32, cx: &mut App| {
                ended_state.update(cx, |state, cx| {
                    let owns_settle =
                        state.archive_pull_eligible || state.archive_reveal_phase == ChatsArchiveRevealPhase::Revealed;
                    state.archive_gesture_active = false;
                    state.archive_gesture_eligibility_resolved = false;
                    state.archive_gesture_start_marker_y = 0.0;
                    state.archive_pull_eligible = false;
                    cx.notify();

                    if marker_y < -ChatsArchiveRevealPolicy::ROW_HEIGHT {
                        state.archive_reveal_phase = ChatsArchiveRevealPhase::Hidden;
                        return None;
                    }

                    let target = if owns_settle {
                        ChatsArchiveRevealPolicy::settle_target(marker_y, velocity_y)
                    } else {
                        ChatsArchiveSettleTarget::Hidden
                    };
                    let settles = ChatsArchiveRevealPolicy::needs_settle(marker_y, velocity_y);

                    if target == ChatsArchiveSettleTarget::Revealed {
                        if state.archive_reveal_phase != ChatsArchiveRevealPhase::Revealed {
                            let _ = selection_changed();
                            state.archive_reveal_phase = ChatsArchiveRevealPhase::Revealed;
                        }
                    } else if marker_y > 0.0 {
                        state.archive_reveal_phase = ChatsArchiveRevealPhase::Pulling;
                    } else {
                        state.archive_reveal_phase = ChatsArchiveRevealPhase::Hidden;
                    }

                    if !settles {
                        return None;
                    }
                    let final_phase = if target == ChatsArchiveSettleTarget::Revealed {
                        ChatsArchiveRevealPhase::Revealed
                    } else {
                        ChatsArchiveRevealPhase::Hidden
                    };
                    state.begin_archive_settle(final_phase, cx);
                    Some(ChatsArchiveRevealPolicy::settle_content_offset_delta(marker_y, target))
                })
            }),
            on_rested_over_row: Rc::new(move |marker_y: f32, cx: &mut App| {
                rested_state.update(cx, |state, cx| {
                    if state.archive_gesture_active || state.archive_settling {
                        return None;
                    }
                    let target = ChatsArchiveRevealPolicy::rest_settle_target(state.archive_reveal_phase, marker_y);
                    let (travel_phase, final_phase) = if target == ChatsArchiveSettleTarget::Revealed {
                        (ChatsArchiveRevealPhase::Revealed, ChatsArchiveRevealPhase::Revealed)
                    } else {
                        (ChatsArchiveRevealPhase::Pulling, ChatsArchiveRevealPhase::Hidden)
                    };
                    state.archive_reveal_phase = travel_phase;
                    cx.notify();
                    state.begin_archive_settle(final_phase, cx);
                    Some(ChatsArchiveRevealPolicy::settle_content_offset_delta(marker_y, target))
                })
            }),
        }
    }

    fn end_reorder_drag(&self, cx: &mut App) {
        self.reorder.update(cx, |reorder, cx| {
            reorder.finish(
                |ids, folder_id, cx: &mut App| {
                    ChatInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.reorder_pinned_chats(ids, folder_id.as_deref(), cx));
                },
                cx,
            );
        });
    }

    fn toggle_select(&self, chat: &ChatInfo, cx: &mut App) {
        if self.state.read(cx).selected_for_reorder.contains(&chat.id) {
            self.state.update(cx, |state, cx| {
                state.selected_for_reorder.remove(&chat.id);
                cx.notify();
            });
            return;
        }
        if !ChatsScreenState::is_selectable(chat) {
            return;
        }
        if self.state.read(cx).selected_for_reorder.len() as i64 >= ChatsSelectionChrome::selection_limit() {
            ChatsScreenState::announce_selection_limit(cx);
            return;
        }
        self.state.update(cx, |state, cx| {
            state.note_selected(chat);
            cx.notify();
        });
    }

    fn sync_selection_chrome(&self, is_tab_active: bool, cx: &mut App) {
        let chrome = ChatsSelectionChrome::shared(cx);
        let (is_reorder_mode, selected_count) = {
            let state = self.state.read(cx);
            (state.is_reorder_mode, state.selected_for_reorder.len() as i64)
        };
        if !is_reorder_mode || !is_tab_active {
            chrome.update(cx, |chrome, cx| chrome.deactivate(cx));
            return;
        }
        let (read_state, archive_state, delete_state) = (self.state.clone(), self.state.clone(), self.state.clone());
        let on_read: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let source = ChatServicesStore::shared(cx).read(cx).current_chat_infos(cx);
            ChatsScreenState::mark_selected_read(&read_state, &source, cx);
        });
        let on_archive: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let source = ChatServicesStore::shared(cx).read(cx).current_chat_infos(cx);
            ChatsScreenState::archive_selected(&archive_state, &source, cx);
        });
        let on_delete: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let source = ChatServicesStore::shared(cx).read(cx).current_chat_infos(cx);
            ChatsScreenState::delete_selected(&delete_state, &source, cx);
        });
        chrome.update(cx, |chrome, cx| chrome.activate(selected_count, on_read, on_archive, on_delete, cx));
    }

    fn dismiss_delete_dialog(&self, cx: &mut App) {
        let now = RiseClock::monotonic();
        self.pending_delete_presence.update(cx, |presence, cx| {
            presence.set_in(None, spring(0.22, 0.9), now, cx);
            cx.notify();
        });
        self.state.update(cx, |state, cx| {
            state.pending_delete_chat = None;
            cx.notify();
        });
    }

    fn delete_chat(&self, chat: &ChatInfo, for_everyone: bool, cx: &mut App) {
        self.dismiss_delete_dialog(cx);
        self.state.update(cx, |state, cx| {
            state.chat_row_local.remove(&chat.id);
            cx.notify();
        });
        let on_error: ChatInteractionsDone = Box::new(|cx: &mut App| {
            Notifier::send(
                tr("chat_delete_failed_title"),
                NotifierSendOptions {
                    content: Some(tr("chat_delete_failed_text")),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.delete_chat(&chat.id, for_everyone, false, None, Some(on_error), None, cx);
        });
    }

    fn clear_favourites_history(&self, chat: &ChatInfo, cx: &mut App) {
        self.dismiss_delete_dialog(cx);
        if chat.r#type != ChatType::Favourites {
            return;
        }
        let Some(user_id) = AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
        else {
            return;
        };
        MessageInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.delete_all_messages_from_user(&chat.id, &user_id, cx));
    }
}

#[derive(IntoElement)]
struct ChatsTabsHeader {
    tabs: Vec<TabHeaderConfig>,
    active_tab: Binding<i64>,
    live_drag_state: Entity<AnimatedTabsPagerLiveState>,
    on_tap_tab: AnimatedTabsTapTab,
}

impl RenderOnce for ChatsTabsHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (progress_state, base_state, release_state, suppress_state) = (
            self.live_drag_state.clone(),
            self.live_drag_state.clone(),
            self.live_drag_state.clone(),
            self.live_drag_state,
        );
        AnimatedTabsHeader::new(
            "chats-tabs-header",
            self.tabs,
            self.active_tab,
            Rc::new(move |cx: &App| progress_state.read(cx).drag_progress()),
            0.0,
            self.on_tap_tab,
        )
        .drag_base_index(Rc::new(move |cx: &App| base_state.read(cx).drag_base_index()))
        .drag_release_animation(Rc::new(move |cx: &App| release_state.read(cx).drag_release_animation()))
        .suppresses_tap(Rc::new(move |cx: &App| suppress_state.read(cx).suppresses_tab_taps()))
        .force_scrollable(true)
        .max_tab_width_fraction(Some(0.6))
    }
}

struct ChatsBelowChrome;

impl ChatsBelowChrome {
    fn body(self, content: impl IntoElement, cx: &mut App) -> Div {
        div().size_full().pt(px(list_shell_floating_top_inset(cx))).child(content)
    }
}

struct ChatsShellFrame {
    pages: Rc<Vec<ChatsTabPage>>,
    tab_configs: Vec<TabHeaderConfig>,
    tabs_count: i64,
    active_tab_binding: Binding<i64>,
    active_prefetch_page_id: Option<String>,
    frame: ChatsPageFrame,
    mount_anchor: i64,
    embedded_forum_chat: Option<ChatInfo>,
    embedded_forum_initial_snapshot: Option<ForumTopicListSnapshot>,
    embedded_forum_epoch: u64,
    embedded_forum_width: f32,
    requests_count: i64,
    embedded_forum_center_revision: String,
    embedded_forum_right_revision: String,
    embedded_forum_left_revision: String,
    search_overlay: bool,
}

struct ChatsChangesFrame {
    pages: Rc<Vec<ChatsTabPage>>,
    page_ids: Vec<String>,
    mount_anchor: i64,
    active_prefetch_page_id: Option<String>,
    active_page_index: i64,
    has_tabs: bool,
    embedded_forum_chat_id: Option<String>,
    allows_detached_snapshot: bool,
    is_reorder_mode: bool,
    selected_count: i64,
    is_chats_tab_active: bool,
    pending_delete_id: Option<String>,
    presence_partner_ids: Vec<String>,
    source_ids: Vec<String>,
    source: Rc<Vec<ChatInfo>>,
}

struct ChatsAllTabArgs {
    display_chats: Rc<Vec<ChatInfo>>,
    filtered_is_empty: bool,
    drafts: Rc<HashMap<String, ChatDraftPreview>>,
    source: Rc<Vec<ChatInfo>>,
    is_loading: bool,
    shows_archive_reveal: bool,
    folder_id: Option<String>,
    is_active_page: bool,
    rows_active: bool,
    avatar_prefetch_page_id: String,
    collapse_range: f32,
    has_embedded_forum: bool,
    restoration_ready: bool,
    has_before: bool,
}

/// what a page model was built from, the stores it also reads drop it on their notify
#[derive(Clone)]
struct ChatsPageModelKey {
    source: Rc<Vec<ChatInfo>>,
    archived: Option<Rc<Vec<ChatInfo>>>,
    folder: Option<ChatFolder>,
    snapshot: Option<Rc<RiseChatViewportSnapshot>>,
    has_before: bool,
}

/// archive merged into one shared source, folder pages filter it
struct ChatsArchiveMerge {
    archived: Rc<Vec<ChatInfo>>,
    source: Rc<Vec<ChatInfo>>,
    merged: Rc<Vec<ChatInfo>>,
}

impl ChatsPageModelKey {
    /// first input that differs, for the rebuild trace
    fn mismatch(&self, other: &Self) -> &'static str {
        let same = |lhs: &Option<Rc<_>>, rhs: &Option<Rc<_>>| match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => Rc::ptr_eq(lhs, rhs),
            (None, None) => true,
            _ => false,
        };
        if !Rc::ptr_eq(&self.source, &other.source) {
            "source"
        } else if self.folder != other.folder {
            "folder"
        } else if self.has_before != other.has_before {
            "has_before"
        } else if !same(&self.archived, &other.archived) {
            "archived"
        } else {
            "snapshot"
        }
    }

    fn matches(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.source, &other.source)
            && match (&self.archived, &other.archived) {
                (Some(lhs), Some(rhs)) => Rc::ptr_eq(lhs, rhs),
                (None, None) => true,
                _ => false,
            }
            && self.folder == other.folder
            && match (&self.snapshot, &other.snapshot) {
                (Some(lhs), Some(rhs)) => Rc::ptr_eq(lhs, rhs),
                (None, None) => true,
                _ => false,
            }
            && self.has_before == other.has_before
    }
}

/// list rows of one page and the id index its visibility reports read
struct ChatsRowsMemo {
    frame: ChatsListFrame,
    drafts: Rc<HashMap<String, ChatDraftPreview>>,
    viewport_revision: u64,
    rows: Rc<Vec<RiseChatNativeListRow>>,
    visible_index_by_id: Rc<HashMap<String, i64>>,
}

impl ChatsRowsMemo {
    /// every frame field a row closure or its version reads
    /// the viewport revision stands for the content versions the rows hash
    fn matches(&self, frame: &ChatsListFrame, drafts: &Rc<HashMap<String, ChatDraftPreview>>, viewport_revision: u64) -> bool {
        let kept = &self.frame;
        Rc::ptr_eq(&self.drafts, drafts)
            && self.viewport_revision == viewport_revision
            && Rc::ptr_eq(&kept.display_chats, &frame.display_chats)
            && Rc::ptr_eq(&kept.source, &frame.source)
            && kept.viewport == frame.viewport
            && kept.page_id == frame.page_id
            && kept.is_active_page == frame.is_active_page
            && kept.has_before == frame.has_before
            && kept.practice_listed == frame.practice_listed
            && kept.guide_chat_id == frame.guide_chat_id
            && kept.is_reorder_mode == frame.is_reorder_mode
            && kept.selected_for_reorder == frame.selected_for_reorder
            && kept.pinned_count == frame.pinned_count
            && kept.pinned_ids == frame.pinned_ids
            && kept.folder_id == frame.folder_id
            && kept.separator == frame.separator
            && kept.open_chat_id == frame.open_chat_id
            && kept.open_fill == frame.open_fill
    }
}

/// rows of one chats page as the all tab draws them
#[derive(Clone)]
struct ChatsPageModel {
    key: ChatsPageModelKey,
    page_source: Rc<Vec<ChatInfo>>,
    display_chats: Rc<Vec<ChatInfo>>,
    filtered_is_empty: bool,
    drafts: Rc<HashMap<String, ChatDraftPreview>>,
}

struct ChatsListArgs {
    drafts: Rc<HashMap<String, ChatDraftPreview>>,
    shows_archive_reveal: bool,
    pins_archive: bool,
    archived_chats: Rc<Vec<ChatInfo>>,
    archived_count: i64,
    archived_unread_chats_count: i64,
    archive_interactions_enabled: bool,
    restoration_ready: bool,
    collapse_range: f32,
    rows_active: bool,
    is_dragging: bool,
}

#[derive(IntoElement)]
pub struct ChatDeleteDialog {
    chat: ChatInfo,
    on_delete_for_me: ButtonAction,
    on_delete_for_everyone: ButtonAction,
    on_cancel: ButtonAction,
}

struct ChatDeleteDialogState {
    appeared: Animated<f32>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
}

impl ChatDeleteDialog {
    pub fn new(
        chat: ChatInfo,
        on_delete_for_me: impl Fn(&mut Window, &mut App) + 'static,
        on_delete_for_everyone: impl Fn(&mut Window, &mut App) + 'static,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            chat,
            on_delete_for_me: Rc::new(on_delete_for_me),
            on_delete_for_everyone: Rc::new(on_delete_for_everyone),
            on_cancel: Rc::new(on_cancel),
        }
    }

    fn peer_name(&self) -> String {
        ChatInteractionsStore::chat_delete_peer_name(&self.chat)
    }

    fn body_text(&self) -> String {
        if self.chat.r#type == ChatType::Favourites {
            return tr("chat_clear_favourites_dialog_body");
        }
        if self.chat.is_official_bot_chat() {
            return tr("chat_clear_history_dialog_body");
        }
        ChatInteractionsStore::chat_delete_dialog_body(&self.chat)
    }

    fn dialog_title(&self) -> String {
        if self.chat.r#type == ChatType::Favourites {
            return tr("chat_clear_favourites_dialog_title");
        }
        if self.chat.is_official_bot_chat() {
            return tr("chat_clear_history_dialog_title");
        }
        ChatInteractionsStore::chat_delete_dialog_title(&self.chat)
    }

    fn everyone_title(&self) -> String {
        if self.chat.r#type == ChatType::Direct {
            return tr("chat_delete_for_both").replace("{name}", &self.peer_name());
        }
        tr("chat_delete_for_everyone")
    }

    fn dialog_button(id: &'static str, title: String, destructive: bool, action: ButtonAction) -> AnyElement {
        let color: Hsla = if destructive { Rgba { r: 1.0, g: 0.34, b: 0.32, a: 1.0 }.into() } else { white() };
        let capsule = div()
            .w_full()
            .h(px(48.0))
            .px(px(10.0))
            .capsule()
            .bg(white().opacity(0.11))
            .flex()
            .items_center()
            .justify_center()
            .child(
                MainText::new(title)
                    .px(17.0)
                    .font_weight(if destructive { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                    .color(color)
                    .number_of_lines(1)
                    .minimum_scale_factor(0.78),
            )
            .liquid_press(id, LiquidPressShape::Capsule, false, false, false, Some(action));
        div().w_full().child(capsule).into_any_element()
    }
}

impl RenderOnce for ChatDeleteDialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state_id = SharedString::from(format!("chat-delete-dialog-{}", self.chat.id));
        let now = RiseClock::monotonic();
        let previous_focus = window.focused(cx);
        let state = window.use_keyed_state(state_id.clone(), cx, |_, cx| {
            let mut appeared = Animated::new(0.0);
            appeared.set_in(1.0, spring(0.28, 0.86), now, cx);
            ChatDeleteDialogState { appeared, focus: cx.focus_handle(), previous_focus }
        });
        let appeared = state.read(cx).appeared.frame_value(now, window);
        let focus = state.read(cx).focus.clone();

        let is_favourites = self.chat.r#type == ChatType::Favourites;
        let is_official_bot = self.chat.is_official_bot_chat();
        let first_button = if is_favourites {
            Some(("chat-delete-confirm", tr("chat_clear_favourites_confirm")))
        } else if is_official_bot {
            Some(("chat-delete-confirm", tr("chat_clear_history_confirm")))
        } else if self.chat.r#type == ChatType::Direct {
            Some(("chat-delete-for-me", tr("chat_delete_for_me")))
        } else {
            None
        };
        let first_button = first_button
            .map(|(id, title)| Self::dialog_button(id, title, true, self.on_delete_for_me.clone()));
        let everyone_button = (!is_favourites && !is_official_bot).then(|| {
            Self::dialog_button("chat-delete-for-everyone", self.everyone_title(), true, self.on_delete_for_everyone.clone())
        });
        let title = self.dialog_title();
        let body = self.body_text();

        let (backdrop_cancel, key_cancel) = (self.on_cancel.clone(), self.on_cancel.clone());
        let (appearing, leaving) = (focus.clone(), state.clone());
        let viewport = window.viewport_size();
        let layer = div()
            .id(state_id)
            .w(viewport.width)
            .h(viewport.height)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&focus)
            .layer_keys(LayerCancel::dismiss(move |window, cx| key_cancel(window, cx)))
            .child(
                div()
                    .id("chat-delete-dialog-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(0.64))
                    .on_click(move |_, window, cx| backdrop_cancel(window, cx)),
            )
            .child(
                div().w(px(332.0)).max_w(viewport.width).px(px(16.0)).child(
                    vstack(0.0)
                        .id("chat-delete-dialog-card")
                        .occlude()
                        .relative()
                        .w_full()
                        .max_w(px(300.0))
                        .rounded(px(28.0))
                        .overflow_hidden()
                        .opacity(appeared.clamp(0.0, 1.0))
                        .shadow(vec![BoxShadow {
                            color: black().opacity(0.4),
                            offset: point(px(0.0), px(14.0)),
                            blur_radius: px(28.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                        .child(div().absolute().inset_0().rounded(px(28.0)).bg(gpui::hsla(240.0 / 360.0, 0.04, 0.13, 1.0)))
                        .child(centred_stroke(white().opacity(0.09), 1.0, Shape::continuous(28.0)))
                        .child(
                            div().w_full().flex().justify_center().pt(px(20.0)).child(
                                ChatLogo::new("chat-delete-dialog-logo", self.chat.clone())
                                    .size(72.0)
                                    .icon_size(30.0),
                            ),
                        )
                        .child(
                            vstack(6.0)
                                .w_full()
                                .items_center()
                                .px(px(24.0))
                                .pt(px(24.0))
                                .child(
                                    MainText::new(title)
                                        .px(17.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(white())
                                        .tac(TextAlignT::Center),
                                )
                                .child(
                                    MainText::new(body)
                                        .px(17.0)
                                        .color(white().opacity(0.88))
                                        .line_height(22.0)
                                        .tac(TextAlignT::Center),
                                ),
                        )
                        .child(
                            vstack(8.0)
                                .w_full()
                                .px(px(16.0))
                                .pt(px(20.0))
                                .pb(px(16.0))
                                .children(first_button)
                                .children(everyone_button)
                                .child(Self::dialog_button(
                                    "chat-delete-cancel",
                                    tr("common_cancel"),
                                    false,
                                    self.on_cancel,
                                )),
                        )
                        .scale_effect(0.94 + 0.06 * appeared),
                ),
            )
            .on_appear("chat-delete-dialog-lifecycle", move |window, cx| window.focus(&appearing, cx))
            .and_on_disappear(move |window, cx| {
                let previous = leaving.read(cx).previous_focus.clone();
                if let (Some(previous), Some(window)) = (previous, window) {
                    window.focus(&previous, cx);
                }
            });
        deferred(anchored().position(point(px(0.0), px(0.0))).child(layer)).with_priority(1)
    }
}

/// status line of the header in its own state so a status change only reruns the line
struct ChatsNetworkStatusTitleState {
    status: ChatsHeaderStatus,
    title: Presence<ChatsHeaderStatus>,
    spinner: Presence<()>,
    badge: Presence<()>,
    badge_emoji: Option<SharedString>,
    text_width: Option<Animated<f32>>,
    spinner_width: Animated<f32>,
    badge_width: Animated<f32>,
    badge_content_width: Option<f32>,
    premium_badge_anchor: ViewAnchorProbe,
}

impl ChatsNetworkStatusTitleState {
    const MAX_WIDTH: f32 = 228.0;
    const SPACING: f32 = 4.0;
    const SPINNER_SIZE: f32 = 22.0;
    const BADGE_SIZE: f32 = 22.0;
    const STATUS_ANIMATION_SECONDS: f64 = 0.2;

    fn status_transition(reduce_motion: bool) -> Transition {
        if reduce_motion {
            return Transition::opacity();
        }
        Transition::asymmetric(
            Transition::opacity().combined(Transition::move_edge(Edge::Bottom)),
            Transition::opacity().combined(Transition::move_edge(Edge::Top)),
        )
    }

    fn accessory_transition() -> Transition {
        Transition::opacity().combined(Transition::scale(0.86))
    }

    fn status_text(status: ChatsHeaderStatus) -> gpui::Div {
        div().flex_none().max_w(px(Self::MAX_WIDTH)).child(
            MainText::new(status.text())
                .px(15.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::BOLD)
                .number_of_lines(1),
        )
    }

    fn text_width_measured(&mut self, width: f32, cx: &mut Context<Self>) {
        match &mut self.text_width {
            Some(animated) => {
                if (*animated.target() - width).abs() > 0.5 {
                    let spec = ease_in_out(Self::STATUS_ANIMATION_SECONDS);
                    animated.set_in(width, spec, RiseClock::monotonic(), cx);
                    cx.notify();
                }
            }
            None => {
                self.text_width = Some(Animated::new(width));
                cx.notify();
            }
        }
    }

    fn badge_slot_width(&self) -> f32 {
        self.badge_content_width.unwrap_or(Self::BADGE_SIZE) + Self::SPACING
    }

    fn badge_width_measured(&mut self, width: f32, cx: &mut Context<Self>) {
        if self.badge_content_width.is_some_and(|known| (known - width).abs() <= 0.5) {
            return;
        }
        self.badge_content_width = Some(width);
        if self.badge.is_present() {
            let slot = self.badge_slot_width();
            let spec = ease_in_out(Self::STATUS_ANIMATION_SECONDS);
            self.badge_width.set_in(slot, spec, RiseClock::monotonic(), cx);
        }
        cx.notify();
    }
}

#[derive(IntoElement, Default)]
struct ChatsNetworkStatusTitle {
    is_rise_request_pending: bool,
}

impl ChatsNetworkStatusTitle {
    fn new(is_rise_request_pending: bool) -> Self {
        Self { is_rise_request_pending }
    }
}

impl RenderOnce for ChatsNetworkStatusTitle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let services = ChatServicesStore::shared(cx);
        let cache_key = services.read(cx).chats_cache_key(cx);
        let is_legacy_request_pending =
            SaiWsPendingRegistry::shared(cx).read(cx).has_in_flight_request(cache_key.as_deref());
        let is_rise_request_pending = self.is_rise_request_pending;
        let maintenance = MaintenanceGate::shared(cx);
        let maintenance_snapshot = maintenance.read(cx).snapshot().clone();
        let status = ChatsHeaderStatus::resolve_shown(
            RiseTransportStatus::shown(cx),
            is_legacy_request_pending || is_rise_request_pending,
            maintenance_snapshot.is_closed,
        );

        let profile = AuthServicesStore::shared(cx).read(cx).profile().cloned();
        let badge_emoji = (status == ChatsHeaderStatus::Title)
            .then_some(profile)
            .flatten()
            .filter(|profile| profile.more.is_premium)
            .map(|profile| profile.more.premium_badge_emoji.clone().map(SharedString::from));
        let shows_badge = badge_emoji.is_some();
        let shows_spinner = status.shows_activity();
        let spinner_slot = ChatsNetworkStatusTitleState::SPINNER_SIZE + ChatsNetworkStatusTitleState::SPACING;
        let reduce_motion = rise_ui::swiftui::environment::reduce_motion(cx);

        let now = RiseClock::monotonic();
        let state = window.use_keyed_state("chats-network-status-title", cx, move |_, _| ChatsNetworkStatusTitleState {
            status,
            title: Presence::new(Some(status))
                .with_transition(ChatsNetworkStatusTitleState::status_transition(reduce_motion)),
            spinner: Presence::new(shows_spinner.then_some(()))
                .with_transition(ChatsNetworkStatusTitleState::accessory_transition()),
            badge: Presence::new(shows_badge.then_some(()))
                .with_transition(ChatsNetworkStatusTitleState::accessory_transition()),
            badge_emoji: None,
            text_width: None,
            spinner_width: Animated::new(if shows_spinner { spinner_slot } else { 0.0 }),
            badge_width: Animated::new(if shows_badge {
                ChatsNetworkStatusTitleState::BADGE_SIZE + ChatsNetworkStatusTitleState::SPACING
            } else {
                0.0
            }),
            badge_content_width: None,
            premium_badge_anchor: ViewAnchorProbe::new(),
        });
        let spec = ease_in_out(ChatsNetworkStatusTitleState::STATUS_ANIMATION_SECONDS);
        state.update(cx, |state, cx| {
            if state.status != status {
                tracing::warn!(target: "riseonly::X630", "chats header status {:?} -> {:?}", state.status, status);
                state.status = status;
                state.title.set_transition(ChatsNetworkStatusTitleState::status_transition(reduce_motion));
                state.title.set_keyed_in(Some(status), |status: &ChatsHeaderStatus| *status, spec, now, cx);
            }
            if state.spinner.is_present() != shows_spinner {
                state.spinner.set_in(shows_spinner.then_some(()), spec, now, cx);
                state.spinner_width.set_in(if shows_spinner { spinner_slot } else { 0.0 }, spec, now, cx);
            }
            if state.badge.is_present() != shows_badge {
                state.badge.set_in(shows_badge.then_some(()), spec, now, cx);
                let badge_slot = state.badge_slot_width();
                state.badge_width.set_in(if shows_badge { badge_slot } else { 0.0 }, spec, now, cx);
            }
            if let Some(emoji) = badge_emoji {
                state.badge_emoji = emoji;
            }
        });

        let measured_state = state.clone();
        let badge_measured_state = state.clone();
        let title_state = state.read(cx);
        let anchor = title_state.premium_badge_anchor.clone();
        let spinner_tint = theme.text.primary;
        let spinner = title_state.spinner.render_transitioned(now, window, |_| {
            div().child(LoaderUi::new().size(LoaderUiSize::Custom(ChatsNetworkStatusTitleState::SPINNER_SIZE)).color(Some(spinner_tint)))
        });
        let spinner_width = title_state.spinner_width.frame_value(now, window);
        let title = title_state.title.render_transitioned(now, window, |value| {
            let probe = measure(SharedString::new_static("chats-status-text-width"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                measured_state.update(cx, |state, cx| state.text_width_measured(width, cx));
            });
            div().flex().justify_center().child(ChatsNetworkStatusTitleState::status_text(*value).child(probe))
        });
        let departing_titles = title_state.title.render_departing_transitioned(now, window, |value| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_center()
                .child(ChatsNetworkStatusTitleState::status_text(*value))
        });
        let text_width = title_state
            .text_width
            .as_ref()
            .filter(|width| width.is_running(now))
            .map(|width| width.frame_value(now, window));
        let badge_width = title_state.badge_width.frame_value(now, window);
        let badge_width_running = title_state.badge_width.is_running(now);
        let badge_at_rest = title_state.badge.is_present() && !badge_width_running;
        let badge = title_state.badge.render_transitioned(now, window, |_| {
            let tap_anchor = anchor.clone();
            let badge_measured_state = badge_measured_state.clone();
            let badge_probe = measure(SharedString::new_static("chats-title-premium-badge-width"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                badge_measured_state.update(cx, |state, cx| state.badge_width_measured(width, cx));
            });
            div()
                .id("chats-title-premium-badge-tooltip")
                .relative()
                .flex_none()
                .tooltip_label(tr("settings_premium_badge_title"))
                .view_anchor_probe(&anchor)
                .child(
                    plain("chats-title-premium-badge")
                        .py(px(4.0))
                        .child(
                            PremiumBadge::new()
                                .emoji(title_state.badge_emoji.clone())
                                .size(ChatsNetworkStatusTitleState::BADGE_SIZE)
                                .shows_hint(false),
                        )
                        .on_press(move |window, cx| {
                            let frame = tap_anchor
                                .window_frame()
                                .unwrap_or_else(|| PremiumBadgePopoverLayout::detached_anchor(window));
                            PremiumBadgePickerPopover::shared(cx).update(cx, |popover, cx| popover.present(frame, cx));
                        }),
                )
                .child(badge_probe)
        });

        let line = hstack(0.0)
            .max_w(px(ChatsNetworkStatusTitleState::MAX_WIDTH))
            .min_h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .items_center()
            .justify_center()
            .child(div().flex_none().w(px(spinner_width)).flex().items_center().children(spinner))
            .child(
                div()
                    .relative()
                    .flex()
                    .justify_center()
                    .items_center()
                    .when_some(text_width, |slot, width| slot.w(px(width)))
                    .children(title)
                    .children(departing_titles),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .map(|slot| {
                        if badge_at_rest {
                            slot.pl(px(ChatsNetworkStatusTitleState::SPACING))
                        } else {
                            slot.w(px(badge_width))
                        }
                    })
                    .children(badge),
            )
            .into_any_element();
        ChatsMaintenanceMenu {
            is_active: status == ChatsHeaderStatus::Maintenance,
            snapshot: maintenance_snapshot,
            on_retry: Rc::new(move |_: &mut Window, cx: &mut App| {
                maintenance.update(cx, |maintenance, cx| maintenance.refresh_now(cx));
            }),
        }
        .body(line, window, cx)
    }
}

struct ChatsMaintenanceMenu {
    is_active: bool,
    snapshot: MaintenanceSnapshot,
    on_retry: ButtonAction,
}

impl ChatsMaintenanceMenu {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        if !self.is_active {
            return content;
        }
        let presented = window.use_keyed_state("chats-maintenance-menu-presented", cx, |_, _| false);
        let opener = presented.clone();
        let (reader, writer) = (presented.clone(), presented);
        let is_presented = Binding::new(
            move |cx: &App| *reader.read(cx),
            move |value: bool, _: &mut Window, cx: &mut App| {
                writer.update(cx, |presented, cx| {
                    *presented = value;
                    cx.notify();
                });
            },
        );
        let snapshot = self.snapshot;
        let on_retry = self.on_retry;
        plain("chats-maintenance-header")
            .on_press(move |_, cx| {
                opener.update(cx, |presented, cx| {
                    *presented = true;
                    cx.notify();
                });
            })
            .child(content)
            .tap_context_menu(
                "chats-maintenance-menu",
                is_presented,
                HoldContextMenuAlignment::Leading,
                false,
                false,
                false,
                None,
                Rc::new(move |_: &App| Self::menu_actions(&snapshot)),
                Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                    if id == "retry" {
                        on_retry(window, cx);
                    }
                }),
                Rc::new(|_: &mut Window, _: &mut App| div().into_any_element()),
            )
            .into_any_element()
    }

    fn menu_actions(snapshot: &MaintenanceSnapshot) -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new("title", snapshot.title.clone()).disabled(true),
            HoldContextMenuAction::new("reopening", snapshot.reopening_text()).disabled(true),
            HoldContextMenuAction::new("retry", tr("common_retry")).system_image("arrow.clockwise").section_break(true),
        ]
    }
}

#[derive(IntoElement)]
struct ChatsInlineEmptyState;

impl ChatsInlineEmptyState {
    fn new() -> Self {
        Self
    }
}

impl RenderOnce for ChatsInlineEmptyState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        vstack(12.0)
            .w_full()
            .min_h(px(260.0))
            .px(px(32.0))
            .items_center()
            .justify_center()
            .children(IconUi::sized(
                "bubble.left.and.bubble.right",
                px(34.0),
                theme.text.secondary.opacity(0.65),
                false,
            ))
            .child(
                MainText::new(tr("no_chats"))
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(3),
            )
    }
}

#[derive(Clone)]
enum ChatsTabPage {
    All,
    Folder(ChatFolder),
    CreatePrompt,
}

impl ChatsTabPage {
    fn tab_title(&self) -> String {
        match self {
            ChatsTabPage::All => tr("all_chats"),
            ChatsTabPage::Folder(folder) => {
                if folder.title.is_empty() {
                    "\u{1F4C1}".to_owned()
                } else {
                    folder.title.clone()
                }
            }
            ChatsTabPage::CreatePrompt => tr("folder_create"),
        }
    }

    fn tab_custom_emojis(&self) -> Vec<TabHeaderCustomEmoji> {
        match self {
            ChatsTabPage::Folder(folder) if !folder.title.is_empty() => {
                folder.title_custom_emojis().iter().map(TabHeaderCustomEmoji::from).collect()
            }
            _ => Vec::new(),
        }
    }

    fn stable_id(&self) -> String {
        match self {
            ChatsTabPage::All => "all".to_owned(),
            ChatsTabPage::Folder(folder) => format!("folder:{}", folder.id),
            ChatsTabPage::CreatePrompt => "create-prompt".to_owned(),
        }
    }
}

/// geometry of the chrome that folds away as the list scrolls
/// the search field and the gap before the folder tabs
pub enum ChatsChromeMetrics {}

impl ChatsChromeMetrics {
    pub const SEARCH_FIELD_HEIGHT: f32 = 42.0;
    pub const TABS_TOP_GAP: f32 = 14.0;

    pub fn collapse_range(has_tabs: bool) -> f32 {
        Self::SEARCH_FIELD_HEIGHT + if has_tabs { Self::TABS_TOP_GAP } else { 0.0 }
    }
}

/// chats selection mode as the tab containers see it
/// while active the tab bar slides away and the action bar takes its place
pub struct ChatsSelectionChrome {
    is_active: bool,
    selected_count: i64,
    on_read: Option<Rc<dyn Fn(&mut App)>>,
    on_archive: Option<Rc<dyn Fn(&mut App)>>,
    on_delete: Option<Rc<dyn Fn(&mut App)>>,
}

impl ChatsSelectionChrome {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_active: false, selected_count: 0, on_read: None, on_archive: None, on_delete: None }
    }

    /// at most this many chats per bulk action, every action is a single call per chat
    pub fn selection_limit() -> i64 {
        RiseLimits::client_value("chats_selection_max")
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn selected_count(&self) -> i64 {
        self.selected_count
    }

    pub fn activate(
        &mut self,
        selected_count: i64,
        on_read: Rc<dyn Fn(&mut App)>,
        on_archive: Rc<dyn Fn(&mut App)>,
        on_delete: Rc<dyn Fn(&mut App)>,
        cx: &mut Context<Self>,
    ) {
        self.on_read = Some(on_read);
        self.on_archive = Some(on_archive);
        self.on_delete = Some(on_delete);
        if self.selected_count != selected_count || !self.is_active {
            self.selected_count = selected_count;
            self.is_active = true;
            cx.notify();
        }
    }

    pub fn update_selected_count(&mut self, count: i64, cx: &mut Context<Self>) {
        if self.selected_count == count {
            return;
        }
        self.selected_count = count;
        cx.notify();
    }

    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.on_read = None;
        self.on_archive = None;
        self.on_delete = None;
        if self.is_active || self.selected_count != 0 {
            self.is_active = false;
            self.selected_count = 0;
            cx.notify();
        }
    }

    fn read(this: &Entity<Self>, cx: &mut App) {
        let action = this.read(cx).on_read.clone();
        if let Some(action) = action {
            action(cx);
        }
    }

    fn archive(this: &Entity<Self>, cx: &mut App) {
        let action = this.read(cx).on_archive.clone();
        if let Some(action) = action {
            action(cx);
        }
    }

    fn delete(this: &Entity<Self>, cx: &mut App) {
        let action = this.read(cx).on_delete.clone();
        if let Some(action) = action {
            action(cx);
        }
    }
}

/// bar that replaces the tab bar while chats are selected
/// buttons stay disabled until something is selected, every action ends selection mode
#[derive(IntoElement, Default)]
pub struct ChatsSelectionActionBar;

impl ChatsSelectionActionBar {
    pub fn new() -> Self {
        Self
    }

    fn button(
        id: &'static str,
        title: String,
        system_image: &'static str,
        tint: Hsla,
        enabled: bool,
        action: fn(&Entity<ChatsSelectionChrome>, &mut App),
        chrome: Entity<ChatsSelectionChrome>,
    ) -> AnyElement {
        plain(id)
            .flex_1()
            .h_full()
            .disabled(!enabled)
            .on_press(move |_, cx| action(&chrome, cx))
            .child(
                vstack(3.0)
                    .size_full()
                    .items_center()
                    .justify_center()
                    .opacity(if enabled { 1.0 } else { 0.4 })
                    .children(IconUi::sized(system_image, px(18.0), tint, false))
                    .child(
                        MainText::new(title)
                            .px(11.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(tint)
                            .number_of_lines(1)
                            .minimum_scale_factor(0.8),
                    ),
            )
            .into_any_element()
    }
}

impl RenderOnce for ChatsSelectionActionBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let chrome = ChatsSelectionChrome::shared(cx);
        let host = window.current_view();
        let observed = chrome.clone();
        let _chrome_observer = window.use_keyed_state("chats-selection-action-bar-observer", cx, move |_, cx| {
            App::observe(cx, &observed, move |_, cx: &mut App| cx.notify(host))
        });
        let enabled = chrome.read(cx).selected_count > 0;
        hstack(0.0)
            .w_full()
            .h(px(MAIN_TAB_BAR_HEIGHT))
            .child(Self::button(
                "chats-selection-read",
                tr("ctx_mark_read"),
                "checkmark.circle",
                theme.text.primary,
                enabled,
                ChatsSelectionChrome::read,
                chrome.clone(),
            ))
            .child(Self::button(
                "chats-selection-archive",
                tr("ctx_archive"),
                "archivebox",
                theme.text.primary,
                enabled,
                ChatsSelectionChrome::archive,
                chrome.clone(),
            ))
            .child(Self::button(
                "chats-selection-delete",
                tr("ctx_delete"),
                "trash",
                theme.semantic.error_100,
                enabled,
                ChatsSelectionChrome::delete,
                chrome,
            ))
            .capsule()
            .liquid_chrome_capsule_with(0.12, true, None, false)
    }
}

struct ChatWallpaperPrewarmDriver {
    key: String,
    task: Option<Task<()>>,
    _observations: [Subscription; 2],
}

impl ChatWallpaperPrewarmDriver {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let wallpapers = ChatWallpaperStore::shared(cx);
        let themes = theme_store(cx);
        let observations = [
            cx.observe_in(&wallpapers, window, |driver, _, window, cx| driver.sync(window, cx)),
            cx.observe_in(&themes, window, |driver, _, window, cx| driver.sync(window, cx)),
        ];
        let mut driver = Self { key: String::new(), task: None, _observations: observations };
        driver.sync(window, cx);
        driver
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = theme_store(cx).read(cx).resolved_appearance();
        let key = chat_wallpaper_prewarm_key(appearance, cx);
        if self.task.is_some() && key == self.key {
            return;
        }
        self.key = key;
        self.task = Some(cx.spawn_in(window, async move |_, cx| ChatWallpaperPrewarm::run(appearance, cx).await));
    }
}

fn chat_wallpaper_prewarm_key(appearance: RiseThemeAppearance, cx: &mut App) -> String {
    let wallpapers = ChatWallpaperStore::shared(cx);
    let wallpapers = wallpapers.read(cx);
    [
        appearance.raw_value().to_owned(),
        wallpapers.selected_preset_id(),
        wallpapers.pattern_intensity().to_string(),
        wallpapers.custom_pattern_colors().join(","),
        wallpapers.image_wallpaper_url().unwrap_or_default().to_owned(),
    ]
    .join("|")
}

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, Pixels, Point, ScrollHandle, SharedString, Subscription, Task,
    Window, bounds, canvas, fill, point, rgb, size,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAction, HoldContextMenuAlignment, HoldContextMenuLocksScrollKey,
};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::pressable_ui::PressableUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowUi};
use rise_ui::scroll_edge_blur_ui::ScrollEdgeBlurFalloff;
use rise_ui::skeleton_ui::{SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::layout_direction;
use rise_ui::swiftui::environment_values::accessibility_reduce_transparency;
use rise_ui::swiftui::linear_gradient::{GradientStop, LinearGradient};
use rise_ui::swiftui::transition::UnitPoint;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_fullscreen_pop::NavigationInteractivePopGuard;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::rise_app_background_ledger::RiseAppBackgroundLedger;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerItemHandler, MediaViewerPresentationHandlers, MediaViewerPresentationState,
    media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerGalleryItemKind, MediaViewerHeroSkin,
};
use crate::core::push_notifications::push_deep_link_store::PushDeepLinkStore;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::formatted_text_ui::FormattedText;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::HoldContextMenuTapPresentationModifiers;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::BotChatManagement;
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionResult;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotModel, BotTagRules, BotUpdateValues};
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::chat::chat_title_row::ChatTitleRow;
use crate::core::ui::premium_badge::BadgeInfoPrompt;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_media_grid_prefs::ChatProfileMediaGridPrefs;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::{ChatProfileTab, ChatProfileTabExt};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tabs::{
    ChatProfilePaneMountPolicy, ChatProfileTabs, ChatProfileViewportLayoutPolicy,
};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_contact_profile_policy::ChatContactProfilePolicy;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile_components::{
    ChatProfileActionRow, ChatProfileInfoCard,
};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile_peer_banner::ChatProfilePeerBanner;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::{ChatCountFormatter, ChatTileColor};
use crate::modules::chat::pages::chats::chat::chat_profile::contact_editor_sheet::ContactEditorSheet;
use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::ForumMemberGrouping;
use crate::modules::chat::pages::chats::chat::forum_home::topic_settings_sheet::TopicSettingsSheet;
use crate::modules::chat::shared::chat_presentation_policy::{
    ChatPresentationPolicy, ChatProfileNavigationPolicy, ChatProfileOpenChatDecision, ChatProfilePresentationMode,
};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_avatar_palette::AvatarPaletteChat;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatChannelKind, ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaKind};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_interactions::gift_appearance_interactions::GiftAppearanceInteractionsStore;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use crate::modules::story::stores::story::story_actions::story_actions::StoryActionDoneHandler;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileHorizontalPagerPolicy, ProfileMasterPinnedModifier, ProfileOuterScrollIntrospector, ProfileScrollCoordinator,
    ProfileScrollMetrics,
};
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::stores::user::user_actions::user_contact_types::UserContactPeer;
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::SavedContactDtoExt;
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    ProfileViewerPrivacy, UserGetProfileByTagGatewayDTOExt,
};
use crate::modules::user::components::profile_top::profile_top::BANNER_HEIGHT as PROFILE_BANNER_HEIGHT;
use crate::modules::user::components::user_block_scope_dialog::UserBlockAndReport;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const HEADER_TOP_PADDING: f32 = 8.0;
const HEADER_AVATAR_SIZE: f32 = 96.0;
const HEADER_AVATAR_SPACING: f32 = 22.0;
const HEADER_AVATAR_ICON_SIZE: f32 = 36.0;
const HEADER_AVATAR_RADIUS: f32 = 48.0;
const HEADER_LIFT_ANIMATION_DURATION: f64 = 0.25;
const OVERVIEW_SPACING: f32 = 16.0;
const VIEWPORT_MINIMUM_HEIGHT: f32 = 220.0;
const PLACEHOLDER_TITLE: &str = "\u{2014}";
const CURRENT_TAB_PARAM: &str = "currentTab";
const RETRY_CONTACT_COLOR: u32 = 0xF19A1A;
const ADD_CONTACT_COLOR: u32 = 0x0083E0;
const UNBLOCK_CONTACT_COLOR: u32 = 0x8E8E93;

fn child_id(id: &ElementId, name: &str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(name.to_owned()))
}

fn observe_store<T: 'static>(store: &Entity<T>, cx: &mut Context<ChatProfileState>) -> Subscription {
    cx.observe(store, |_, _, cx| cx.notify())
}

/// what the profile reads off the navigation stack, the path and the embedded forum
/// they decide if its conversation sits directly behind it
fn navigation_behind(nav: &NavigationStore) -> (Vec<RootRoute>, Option<String>) {
    (nav.path.clone(), nav.embedded_forum_chat_id().map(str::to_owned))
}

fn observe_navigation_behind(store: &Entity<NavigationStore>, cx: &mut Context<ChatProfileState>) -> Subscription {
    let mut last = navigation_behind(store.read(cx));
    cx.observe(store, move |_, store, cx| {
        let next = navigation_behind(store.read(cx));
        if next != last {
            last = next;
            cx.notify();
        }
    })
}

struct ChatProfileState {
    active_tab: i64,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    configured_scroll_scope: String,
    navigation_pop_paging_owner: Uuid,
    show_topic_settings: bool,
    show_contact_editor: bool,
    show_block_scope_dialog: bool,
    is_avatar_menu_presented: bool,
    tabs_pinned: bool,
    is_media_grid_menu_presented: bool,
    open_bot_settings_when_resolved: bool,
    viewport_height: f32,
    window_activations: u64,
    background_returns: u64,
    handled_background_generation: Option<u64>,
    gifts_refresh: Option<Task<()>>,
    avatar_lift: Option<String>,
    presence_key: Option<Rc<dyn Fn(&mut App) -> u64>>,
    shown_presence_key: Option<u64>,
    _subscriptions: Vec<Subscription>,
}

impl ChatProfileState {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll_coordinator = cx.new(ProfileScrollCoordinator::new);

        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let chat_services = ChatServicesStore::shared(cx);
        let chat_actions = ChatActionsStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        let story_services = StoryServicesStore::shared(cx);
        let gifts_services = GiftsServicesStore::shared(cx);
        let presence = PresenceServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let bot_services = BotServicesStore::shared(cx);
        let push_links = PushDeepLinkStore::shared(cx);
        let media_presenter = media_viewer_presentation(cx);
        let avatar_lift = Self::avatar_lift(media_presenter.read(cx));
        let nav_store = env::<NavigationStoreKey>(cx);

        let mut subscriptions = vec![
            observe_store(&hold_context_menu, cx),
            observe_store(&chat_services, cx),
            observe_store(&chat_actions, cx),
            observe_store(&user_services, cx),
            cx.observe(&story_services, |state: &mut Self, _, cx| {
                if state.is_avatar_menu_presented {
                    cx.notify();
                }
            }),
            observe_store(&gifts_services, cx),
            cx.observe(&presence, |state: &mut Self, _, cx| {
                if let Some(key) = state.presence_key.clone() {
                    if Some(key(cx)) == state.shown_presence_key {
                        return;
                    }
                }
                cx.notify();
            }),
            observe_store(&auth, cx),
            observe_store(&bot_services, cx),
            observe_store(&push_links, cx),
            cx.observe(&media_presenter, |state: &mut Self, viewer, cx| {
                let next = Self::avatar_lift(viewer.read(cx));
                if next != state.avatar_lift {
                    state.avatar_lift = next;
                    cx.notify();
                }
            }),
            cx.observe_window_activation(window, |state: &mut Self, window, cx| {
                if window.is_window_active() {
                    state.window_activations += 1;
                    if RiseAppBackgroundLedger::consume_return_from_background(
                        &mut state.handled_background_generation,
                        cx,
                    ) {
                        state.background_returns += 1;
                    }
                    cx.notify();
                }
            }),
            cx.observe_global::<RiseAppLifecycle>(|state: &mut Self, cx| {
                if RiseAppLifecycle::scene_phase(cx) == ScenePhase::Background {
                    return;
                }
                if RiseAppBackgroundLedger::consume_return_from_background(
                    &mut state.handled_background_generation,
                    cx,
                ) {
                    state.background_returns += 1;
                    cx.notify();
                }
            }),
        ];
        if let Some(nav_store) = nav_store.as_ref() {
            subscriptions.push(observe_navigation_behind(nav_store, cx));
        }

        Self {
            active_tab: 0,
            scroll_coordinator,
            configured_scroll_scope: String::new(),
            navigation_pop_paging_owner: Uuid::new_v4(),
            show_topic_settings: false,
            show_contact_editor: false,
            show_block_scope_dialog: false,
            is_avatar_menu_presented: false,
            tabs_pinned: false,
            is_media_grid_menu_presented: false,
            open_bot_settings_when_resolved: false,
            viewport_height: VIEWPORT_MINIMUM_HEIGHT,
            window_activations: 0,
            background_returns: 0,
            handled_background_generation: Some(RiseAppBackgroundLedger::background_generation(cx)),
            gifts_refresh: None,
            avatar_lift,
            presence_key: None,
            shown_presence_key: None,
            _subscriptions: subscriptions,
        }
    }

    /// the chat avatar hidden under the viewer, the only thing the profile reads of it
    /// a viewer read in render would redraw the panel on every viewer frame
    fn avatar_lift(viewer: &MediaViewerPresentationState) -> Option<String> {
        let prefix = MediaViewerFrameKeys::chat_avatar("");
        viewer
            .hero_lift_id()
            .filter(|id| viewer.bubble_cell_hidden() && id.starts_with(prefix.as_str()))
            .map(str::to_owned)
    }
}

#[derive(IntoElement)]
pub struct ChatProfile {
    id: ElementId,
    pub chat_id: Option<String>,
    pub topic_id: Option<String>,
    pub is_general: bool,
    pub presents_in_side_panel: bool,
}

impl ChatProfile {
    pub fn new(id: impl Into<ElementId>, chat_id: impl Into<Option<String>>) -> Self {
        Self { id: id.into(), chat_id: chat_id.into(), topic_id: None, is_general: false, presents_in_side_panel: false }
    }

    pub fn topic_id(mut self, topic_id: impl Into<Option<String>>) -> Self {
        self.topic_id = topic_id.into();
        self
    }

    pub fn is_general(mut self, is_general: bool) -> Self {
        self.is_general = is_general;
        self
    }

    /// desktop side panel beside the open chat is a root with its own close button
    /// so there is no back step
    pub fn presents_in_side_panel(mut self, presents_in_side_panel: bool) -> Self {
        self.presents_in_side_panel = presents_in_side_panel;
        self
    }
}

struct ChatProfileBody {
    id: ElementId,
    chat_id: Option<String>,
    topic_id: Option<String>,
    is_general: bool,
    presents_in_side_panel: bool,
    state: Entity<ChatProfileState>,
    nav_store: Option<Entity<NavigationStore>>,
    media_presenter: Entity<MediaViewerPresentationState>,
    live_chat: Option<ChatInfo>,
    resolved_chat_id: String,
    resolved_chat_type: ChatType,
    gifts_peer_user_id: Option<String>,
    members_first: bool,
    gifts_first: bool,
    shows_members_tab: bool,
    profile_content_restricted: bool,
    ordered_profile_tabs: Vec<ChatProfileTab>,
}

impl RenderOnce for ChatProfile {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, ChatProfileState::new);
        let body = Rc::new(ChatProfileBody::resolve(self, state, cx));
        body.body(window, cx)
    }
}

impl ChatProfileBody {
    fn resolve(profile: ChatProfile, state: Entity<ChatProfileState>, cx: &mut App) -> Self {
        let is_topic_profile = profile.topic_id.is_some() || profile.is_general;
        let services = ChatServicesStore::shared(cx);
        let live_chat = profile.chat_id.as_ref().and_then(|id| {
            let store = services.read(cx);
            store.chat_info(&store.canonical_chat_id(id), cx)
        });
        let resolved_chat_id = Self::resolve_chat_id(profile.chat_id.as_deref(), live_chat.as_ref(), &services, cx);
        let resolved_chat_type = match &live_chat {
            Some(chat) => chat.r#type,
            None if !resolved_chat_id.is_empty()
                && services.read(cx).favourites_chat_id(cx).as_deref() == Some(resolved_chat_id.as_str()) =>
            {
                ChatType::Favourites
            }
            None => ChatType::Direct,
        };
        let members_first = live_chat.as_ref().is_some_and(|chat| chat.r#type == ChatType::Group) && !is_topic_profile;
        let profile_content_restricted = live_chat.as_ref().is_some_and(|chat| {
            chat.r#type == ChatType::Direct
                && UserServicesStore::shared(cx).read(cx).peer_restricts_viewer(&chat.participant.id)
        });
        let gifts_peer_user_id = Self::resolve_gifts_peer(live_chat.as_ref(), is_topic_profile, cx);
        let gifts_first = gifts_peer_user_id.as_ref().is_some_and(|peer_id| {
            GiftsServicesStore::shared(cx).read(cx).has_gifts(peer_id) || ProfileViewerPrivacy::closes_gifts(peer_id, cx)
        });
        let shows_members_tab = resolved_chat_type != ChatType::Channel
            || live_chat.as_ref().is_some_and(ChatProfilePermissions::can_manage);
        let ordered_profile_tabs =
            ChatProfileTab::ordered(resolved_chat_type, members_first, gifts_first, shows_members_tab);

        Self {
            id: profile.id,
            chat_id: profile.chat_id,
            topic_id: profile.topic_id,
            is_general: profile.is_general,
            presents_in_side_panel: profile.presents_in_side_panel,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
            media_presenter: media_viewer_presentation(cx),
            live_chat,
            resolved_chat_id,
            resolved_chat_type,
            gifts_peer_user_id,
            members_first,
            gifts_first,
            shows_members_tab,
            profile_content_restricted,
            ordered_profile_tabs,
        }
    }

    fn resolve_chat_id(
        chat_id: Option<&str>,
        live_chat: Option<&ChatInfo>,
        services: &Entity<ChatServicesStore>,
        cx: &App,
    ) -> String {
        if let Some(chat) = live_chat.filter(|chat| !chat.id.is_empty()) {
            return chat.id.clone();
        }
        let trimmed = chat_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if trimmed.is_empty() {
            return String::new();
        }
        services.read(cx).canonical_chat_id(trimmed)
    }

    fn resolve_gifts_peer(live_chat: Option<&ChatInfo>, is_topic_profile: bool, cx: &mut App) -> Option<String> {
        let chat = live_chat?;
        if chat.r#type != ChatType::Direct || is_topic_profile || chat.is_bot_chat() {
            return None;
        }
        let peer_id = &chat.participant.id;
        if peer_id.is_empty() || AuthServicesStore::shared(cx).read(cx).is_current_user(peer_id) {
            return None;
        }
        Some(peer_id.clone())
    }

    fn is_topic_profile(&self) -> bool {
        self.topic_id.is_some() || self.is_general
    }

    /// general topic stands for the whole forum so it keeps the group rows
    /// invite links and members and settings
    /// which a member from inside a topic looks for
    fn shows_group_rows(&self) -> bool {
        self.topic_id.is_none()
    }

    fn profile_tab(&self, index: i64) -> Option<ChatProfileTab> {
        usize::try_from(index).ok().and_then(|index| self.ordered_profile_tabs.get(index).copied())
    }

    fn active_tab(&self, cx: &App) -> i64 {
        self.state.read(cx).active_tab
    }

    fn coordinator(&self, cx: &App) -> Entity<ProfileScrollCoordinator> {
        self.state.read(cx).scroll_coordinator.clone()
    }

    /// peer of a private chat, empty for a community since it has no common groups tab
    fn peer_user_id_for_common_groups(&self) -> String {
        self.live_chat.as_ref().map(|chat| chat.participant.id.clone()).unwrap_or_default()
    }

    fn current_user_id(cx: &mut App) -> String {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default()
    }

    fn topic_title(&self, cx: &mut App) -> Option<String> {
        if self.is_general {
            return Some(tr("chat_topics_general"));
        }
        let topic_id = self.topic_id.as_deref().filter(|topic_id| !topic_id.is_empty())?;
        ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.topics(&self.resolved_chat_id, cx))
            .into_iter()
            .find(|topic| topic.id == topic_id)
            .map(|topic| topic.title)
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.pop(cx));
        }
    }

    fn body(self: &Rc<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let chat = self.live_chat.clone().unwrap_or_else(|| {
            let id = if self.resolved_chat_id.is_empty() {
                self.chat_id.clone().unwrap_or_default()
            } else {
                self.resolved_chat_id.clone()
            };
            let mut chat = ChatInfo::new(id);
            chat.title = PLACEHOLDER_TITLE.to_owned();
            chat.r#type = self.resolved_chat_type;
            chat
        });
        let can_manage = ChatProfilePermissions::manages_chat(&chat, cx) || self.can_manage_owned_bot(&chat, cx);
        let can_edit = if self.is_topic_profile() {
            self.can_edit_topic(&chat, cx)
        } else {
            can_manage || ChatProfilePermissions::can_edit_profile(&chat, cx)
        };
        let profile_mode = ChatPresentationPolicy::profile_mode(chat.r#type);
        let visible_tabs =
            ChatProfileTab::ordered(chat.r#type, self.members_first, self.gifts_first, self.shows_members_tab);
        let scroll_scope = self.chat_profile_scroll_scope(&chat);

        self.install_changes(&chat, &visible_tabs, &scroll_scope, window, cx);
        let key_chat = chat.clone();
        let presence_key: Rc<dyn Fn(&mut App) -> u64> = Rc::new(move |cx: &mut App| Self::presence_view_key(&key_chat, cx));
        let shown_presence_key = presence_key(cx);
        self.state.update(cx, |state, _| {
            state.presence_key = Some(presence_key);
            state.shown_presence_key = Some(shown_presence_key);
        });

        let back = self.clone();
        let mut shell = ListShellUi::new()
            .use_safe_zone(true)
            .shows_back_button(!self.presents_in_side_panel)
            .on_back(move |_, cx| back.pop(cx))
            .header_chrome_shadow(None);
        let topic_title = if self.state.read(cx).tabs_pinned { self.topic_title(cx) } else { None };
        if self.state.read(cx).tabs_pinned && (topic_title.is_some() || !Self::chat_name_missing(&chat)) {
            let title = topic_title.unwrap_or_else(|| ChatTitleRow::plain_title(&chat, cx));
            let subtitle = Self::profile_subtitle(&chat, cx);
            let mut column = vstack(0.0).items_center().max_w(px(220.0)).min_w_0().child(
                MainText::new(title)
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1),
            );
            if !trimming_whitespaces_and_newlines(&subtitle).is_empty() {
                column = column.child(
                    MainText::new(subtitle)
                        .px(11.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(1),
                );
            }
            shell = shell.center_slot(column).center_slot_self_styled(true);
        }
        if let Some(right) = self.profile_right_view(&chat, can_edit, &theme, cx) {
            shell = shell.right_jsx(right);
        }

        let viewport_state = self.state.clone();
        let viewport_height = self.state.read(cx).viewport_height;
        let content = div()
            .relative()
            .size_full()
            .child(measure(child_id(&self.id, "viewport"), move |bounds, _, cx| {
                let height = f32::from(bounds.size.height).max(VIEWPORT_MINIMUM_HEIGHT);
                viewport_state.update(cx, |state, cx| {
                    if state.viewport_height != height {
                        state.viewport_height = height;
                        cx.notify();
                    }
                });
            }))
            .child(self.chat_profile_scroll_view(
                &chat,
                can_manage,
                profile_mode,
                visible_tabs.clone(),
                scroll_scope.clone(),
                viewport_height,
                &theme,
                window,
                cx,
            ));

        let appear = self.clone();
        let appear_chat = chat.clone();
        let appear_tabs = visible_tabs;
        let appear_scope = scroll_scope;
        let disappear = self.clone();

        div()
            .relative()
            .size_full()
            .child(shell.content(content))
            .children(self.topic_settings_sheet())
            .children(self.contact_editor_sheet(&chat, cx))
            .on_appear(child_id(&self.id, "appear"), move |_, cx| {
                appear.on_appear(&appear_chat, &appear_scope, &appear_tabs, cx);
            })
            .and_on_disappear(move |_, cx| disappear.on_disappear(cx))
            .into_any_element()
    }

    fn topic_settings_sheet(self: &Rc<Self>) -> Option<AnyElement> {
        let topic_id = self.topic_id.clone()?;
        let on_deleted = self.clone();
        Some(
            TopicSettingsSheet::new(
                child_id(&self.id, "topic-settings"),
                Binding::entity(
                    self.state.clone(),
                    |state: &ChatProfileState| state.show_topic_settings,
                    |state: &mut ChatProfileState, value, cx| {
                        state.show_topic_settings = value;
                        cx.notify();
                    },
                ),
                self.resolved_chat_id.clone(),
                topic_id,
            )
            .on_deleted(move |_, cx| on_deleted.pop(cx))
            .into_any_element(),
        )
    }

    fn contact_editor_sheet(self: &Rc<Self>, chat: &ChatInfo, cx: &mut App) -> Option<AnyElement> {
        if !self.is_ordinary_private_peer(chat, cx) {
            return None;
        }
        let existing = UserServicesStore::shared(cx)
            .update(cx, |store, cx| store.saved_contact(&chat.participant.id, cx));
        Some(
            ContactEditorSheet::new(
                child_id(&self.id, "contact-editor"),
                Binding::entity(
                    self.state.clone(),
                    |state: &ChatProfileState| state.show_contact_editor,
                    |state: &mut ChatProfileState, value, cx| {
                        state.show_contact_editor = value;
                        cx.notify();
                    },
                ),
                self.contact_peer(chat, cx),
                existing,
            )
            .into_any_element(),
        )
    }

    fn on_appear(&self, chat: &ChatInfo, scope: &str, visible_tabs: &[ChatProfileTab], cx: &mut App) {
        self.reset_chat_profile_scroll_if_needed(scope, visible_tabs, cx);
        let active_tab = self.active_tab(cx);
        self.update_navigation_pop_priority(active_tab, cx);
        self.publish_chat_profile_route_params(active_tab, cx);
        self.load_active_tab(active_tab, cx);
        let locked = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        self.coordinator(cx).update(cx, |coordinator, cx| coordinator.set_interaction_locked(locked, cx));

        let chat_id = self.resolved_chat_id.as_str();
        let interactions = ChatInteractionsStore::shared(cx);
        if self.is_topic_profile() && !chat_id.is_empty() {
            interactions.update(cx, |store, cx| {
                store.load_topics(chat_id, cx);
                store.load_roles(chat_id, cx);
                if let Some(topic_id) = self.topic_id.as_deref().filter(|id| !id.is_empty())
                    && !self.is_general
                {
                    store.load_effective_permissions(chat_id, Some(topic_id), false, cx);
                }
            });
        }
        if self.shows_group_rows() && chat.r#type.is_community() && !chat.is_community_preview() && !chat_id.is_empty() {
            interactions.update(cx, |store, cx| {
                store.load_effective_permissions(chat_id, None, false, cx);
                if chat.r#type == ChatType::Group {
                    store.load_chat_settings(chat_id, cx);
                }
            });
        }
        self.refresh_contact_state_if_needed(chat, cx);

        self.load_story_state_task(self.chat_story_user_id(chat), cx);
        self.gifts_task(self.gifts_peer_user_id.clone(), cx);
        self.chat_detail_task(self.chat_detail_task_id(chat), cx);
        self.load_bot_details(chat, cx);
    }

    fn on_disappear(&self, cx: &mut App) {
        if !self.presents_in_side_panel
            && let Some(nav_store) = &self.nav_store
        {
            nav_store.update(cx, |store, cx| store.clear_sai_active_screen_overlay_params(true, cx));
        }
        let (coordinator, owner) = {
            let state = self.state.read(cx);
            (state.scroll_coordinator.clone(), state.navigation_pop_paging_owner)
        };
        coordinator.update(cx, |coordinator, cx| coordinator.pause_interaction(cx));
        NavigationInteractivePopGuard::set_profile_tab_paging_back(false, owner, None, cx);
        self.state.update(cx, |state, _| state.gifts_refresh = None);
    }

    fn install_changes(
        self: &Rc<Self>,
        chat: &ChatInfo,
        visible_tabs: &[ChatProfileTab],
        scroll_scope: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (active_tab, activations, background_returns, show_dialog) = {
            let state = self.state.read(cx);
            (state.active_tab, state.window_activations, state.background_returns, state.show_block_scope_dialog)
        };

        let pending_link = PushDeepLinkStore::shared(cx).read(cx).pending_route().cloned();
        let link_state = self.state.clone();
        on_change(child_id(&self.id, "pending-link"), pending_link, false, window, cx, move |_, link, _, cx| {
            if link.is_some() {
                link_state.update(cx, |state, cx| {
                    state.show_contact_editor = false;
                    cx.notify();
                });
            }
        });

        let dialog = self.clone();
        let dialog_chat = chat.clone();
        on_change(child_id(&self.id, "block-scope-dialog"), show_dialog, false, window, cx, move |_, shown, _, cx| {
            if *shown {
                dialog.state.update(cx, |state, cx| {
                    state.show_block_scope_dialog = false;
                    cx.notify();
                });
                dialog.present_block_scope_dialog(&dialog_chat, cx);
            }
        });

        let activated = self.clone();
        on_change(child_id(&self.id, "window-activation"), activations, false, window, cx, move |_, _, _, cx| {
            activated.publish_chat_profile_route_params(activated.active_tab(cx), cx);
        });

        let returned = self.clone();
        let returned_chat_id = self.chat_detail_task_id(chat);
        on_change(child_id(&self.id, "background-return"), background_returns, false, window, cx, move |_, _, _, cx| {
            returned.chat_detail_task(returned_chat_id.clone(), cx);
            returned.load_active_tab(returned.active_tab(cx), cx);
        });

        let tab_change = self.clone();
        on_change(child_id(&self.id, "active-tab"), active_tab, false, window, cx, move |_, tab, _, cx| {
            tab_change.publish_chat_profile_route_params(*tab, cx);
            tab_change.load_active_tab(*tab, cx);
        });

        let restricted_change = self.clone();
        on_change(
            child_id(&self.id, "content-restricted"),
            self.profile_content_restricted,
            false,
            window,
            cx,
            move |_, restricted, _, cx| {
                if !*restricted {
                    restricted_change.load_active_tab(restricted_change.active_tab(cx), cx);
                }
            },
        );

        let scope_change = self.clone();
        let scope_chat = chat.clone();
        let scope_tabs = visible_tabs.to_vec();
        on_change(child_id(&self.id, "scroll-scope"), scroll_scope.to_owned(), false, window, cx, move |_, scope, _, cx| {
            let tab_before_reset = scope_change.active_tab(cx);
            scope_change.reset_chat_profile_scroll_if_needed(scope, &scope_tabs, cx);
            let active_tab = scope_change.active_tab(cx);
            if tab_before_reset == active_tab {
                scope_change.publish_chat_profile_route_params(active_tab, cx);
                scope_change.load_active_tab(active_tab, cx);
            }
            scope_change.refresh_contact_state_if_needed(&scope_chat, cx);
        });

        let locked = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let lock_state = self.clone();
        on_change(child_id(&self.id, "hold-lock"), locked, false, window, cx, move |_, locked, _, cx| {
            let locked = *locked;
            lock_state.coordinator(cx).update(cx, |coordinator, cx| coordinator.set_interaction_locked(locked, cx));
        });

        let last_message = Self::confirmed_last_message_key(chat)
            .map(|key| (key, chat.last_message.as_ref().map(Self::message_tabs_touched_by).unwrap_or_default()));
        let new_message = self.clone();
        on_change(child_id(&self.id, "last-message"), last_message, false, window, cx, move |_, message, _, cx| {
            if let Some((_, touched)) = message
                && !touched.is_empty()
            {
                new_message.refresh_message_tabs_window(new_message.active_tab(cx), touched, cx);
            }
        });

        let groups_change = self.clone();
        on_change(
            child_id(&self.id, "common-groups-peer"),
            self.peer_user_id_for_common_groups(),
            false,
            window,
            cx,
            move |_, peer_id, _, cx| {
                if peer_id.is_empty()
                    || groups_change.profile_tab(groups_change.active_tab(cx)) != Some(ChatProfileTab::Groups)
                {
                    return;
                }
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_common_chats(peer_id, false, cx));
            },
        );

        let managed_bot_id = self.managed_bot_id(chat, cx);
        let bot_change = self.clone();
        on_change(child_id(&self.id, "managed-bot"), managed_bot_id, true, window, cx, move |_, bot_id, _, cx| {
            let Some(bot_id) = bot_id.clone().filter(|bot_id| !bot_id.is_empty()) else {
                return;
            };
            if !bot_change.state.read(cx).open_bot_settings_when_resolved {
                return;
            }
            bot_change.state.update(cx, |state, _| state.open_bot_settings_when_resolved = false);
            bot_change.push(RootRoute::BotSettings { bot_id }, cx);
        });

        let story_task = self.clone();
        on_change(
            child_id(&self.id, "story-state-task"),
            self.chat_story_user_id(chat),
            false,
            window,
            cx,
            move |_, user_id, _, cx| story_task.load_story_state_task(user_id.clone(), cx),
        );

        let gifts_task = self.clone();
        on_change(
            child_id(&self.id, "gifts-task"),
            self.gifts_peer_user_id.clone(),
            false,
            window,
            cx,
            move |_, peer_id, _, cx| gifts_task.gifts_task(peer_id.clone(), cx),
        );

        let detail_task = self.clone();
        on_change(
            child_id(&self.id, "chat-detail-task"),
            self.chat_detail_task_id(chat),
            false,
            window,
            cx,
            move |_, chat_id, _, cx| detail_task.chat_detail_task(chat_id.clone(), cx),
        );

        let bot_task = self.clone();
        let bot_chat = chat.clone();
        let bot_task_id = if chat.is_bot_chat() { self.bot_tag(chat) } else { String::new() };
        on_change(child_id(&self.id, "bot-details-task"), bot_task_id, false, window, cx, move |_, _, _, cx| {
            bot_task.load_bot_details(&bot_chat, cx);
        });
    }

    fn load_story_state_task(&self, user_id: Option<String>, cx: &mut App) {
        let Some(user_id) = user_id else {
            return;
        };
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_stories_state(&[user_id], cx));
    }

    /// decides whether gifts leads the tabs
    /// this is the peer profile opening so look, gifts rule and first page are read fresh
    fn gifts_task(&self, peer_id: Option<String>, cx: &mut App) {
        let Some(peer_id) = peer_id else {
            self.state.update(cx, |state, _| state.gifts_refresh = None);
            return;
        };
        GiftAppearanceInteractionsStore::shared(cx).update(cx, |store, cx| store.profile_opened(Some(&peer_id), cx));
        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_foreign_profile_user_id(peer_id.clone(), cx));
        let task = cx.spawn(async move |cx| GiftsActionsStore::refresh_tab(&peer_id, cx).await);
        self.state.update(cx, |state, _| state.gifts_refresh = Some(task));
    }

    /// a chat this device hasnt loaded is fetched too, else the screen has only the id
    /// and would pass for private
    /// a private chat is read again so a peer photo hidden by privacy leaves the header and list
    fn chat_detail_task_id(&self, chat: &ChatInfo) -> String {
        if self.refreshes_chat_detail(chat) { chat.id.clone() } else { String::new() }
    }

    fn chat_detail_task(&self, chat_id: String, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_chat_by_id_action(&chat_id, true, cx));
    }

    fn refreshes_chat_detail(&self, chat: &ChatInfo) -> bool {
        if self.live_chat.is_none() {
            return true;
        }
        match chat.r#type {
            ChatType::Group | ChatType::Channel => true,
            ChatType::Direct => !chat.is_message_request,
            ChatType::Favourites => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn chat_profile_scroll_view(
        self: &Rc<Self>,
        chat: &ChatInfo,
        can_manage: bool,
        profile_mode: ChatProfilePresentationMode,
        visible_tabs: Vec<ChatProfileTab>,
        scroll_scope: String,
        viewport_height: f32,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let has_navigation_items = !self.nav_items(chat, can_manage, cx).is_empty();
        let hold_active = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let coordinator = self.coordinator(cx);
        let outer = coordinator.read(cx).outer_host_scroll_view();
        let handle = outer.handle();
        let page_color = theme.bg._100;
        let full_mode = profile_mode == ChatProfilePresentationMode::Full;

        let mut overview = vstack(OVERVIEW_SPACING).w_full().pt(px(HEADER_TOP_PADDING)).child(ChatProfileOverviewFadeModifier {
            is_enabled: !full_mode,
            handle: handle.clone(),
            color: page_color,
            content: self.header(chat, theme, window, cx),
        });
        if full_mode {
            if !chat.is_community_preview() {
                let open_chat = self.clone();
                let open_chat_target = chat.clone();
                overview = overview.child(
                    ChatProfileActionRow::new(child_id(&self.id, "action-row"), chat.clone())
                        .is_topic_profile(self.is_topic_profile())
                        .shows_open_chat(!self.presents_in_side_panel && !self.conversation_is_directly_behind(chat, cx))
                        .on_open_chat(move |_, cx| open_chat.open_conversation(&open_chat_target, cx)),
                );
            }
            let additional_items =
                if self.is_topic_profile() { Vec::new() } else { self.contact_action_items(chat, theme, cx) };
            overview = overview.child(ChatProfileOverviewFadeModifier {
                is_enabled: !self.shows_group_rows() || !has_navigation_items,
                handle: handle.clone(),
                color: page_color,
                content: ChatProfileInfoCard::new(child_id(&self.id, "info-card"), chat.clone())
                    .additional_items(additional_items)
                    .into_any_element(),
            });
            if self.shows_group_rows() && has_navigation_items {
                overview = overview.child(ChatProfileOverviewFadeModifier {
                    is_enabled: true,
                    handle: handle.clone(),
                    color: page_color,
                    content: GroupedBtns::new(child_id(&self.id, "nav-section"), self.nav_items(chat, can_manage, cx))
                        .into_any_element(),
                });
            }
        }

        let select = self.clone();
        let select_tabs = visible_tabs.clone();
        let tabs = ChatProfileTabs::new(
            SharedString::from(format!("chat-profile-tabs-{}", chat.id)),
            Binding::entity(
                self.state.clone(),
                |state: &ChatProfileState| state.active_tab,
                |state: &mut ChatProfileState, value, cx| {
                    state.active_tab = value;
                    cx.notify();
                },
            ),
            chat.id.clone(),
            chat.r#type,
            viewport_height,
            main_tab_bar_underlap(cx),
            scroll_scope,
            coordinator.clone(),
            move |index: i64, _: &mut Window, cx: &mut App| {
                select.select_chat_profile_tab(index, &select_tabs, true, cx);
            },
        )
        .topic_id(self.topic_id.clone())
        .group_by_role(self.is_topic_profile())
        .members_first(self.members_first)
        .gifts_first(self.gifts_first)
        .shows_members(self.shows_members_tab)
        .content_restricted(self.profile_content_restricted)
        .peer_user_id(chat.participant.id.clone());

        let swipe = self.clone();
        let swipe_tabs = visible_tabs.clone();
        let can_swipe_state = self.state.clone();
        let can_swipe_tabs = visible_tabs;
        let introspector = ProfileOuterScrollIntrospector::new(
            coordinator.clone(),
            Rc::new(move |direction: i64, cx: &mut App| {
                swipe.select_adjacent_chat_profile_tab(direction, &swipe_tabs, cx);
            }),
            Rc::new(move |direction: i64, cx: &App| {
                let active_tab = can_swipe_state.read(cx).active_tab;
                ProfileHorizontalPagerPolicy::target_index(active_tab, direction, can_swipe_tabs.len() as i64).is_some()
            }),
        );

        let stack = div()
            .relative()
            .w_full()
            .child(
                vstack(0.0)
                    .w_full()
                    .child(introspector)
                    .child(overview)
                    .child(div().h(px(ChatProfileViewportLayoutPolicy::OUTER_SPACING)))
                    .child(ChatProfileTabsPinnedProbe {
                        state: self.state.clone(),
                        handle: handle.clone(),
                        content: ChatProfileMasterPinnedModifier {
                            scroll_coordinator: coordinator,
                            content: tabs.into_any_element(),
                        }
                        .into_any_element(),
                    }),
            )
            .child(ChatProfileChromeBackdrop { handle, color: page_color });

        let refresh = self.clone();
        let scroller = scroll_view(child_id(&self.id, "outer-scroll"))
            .size_full()
            .scroll_handle(outer.handle())
            .restrict_scroll_to_axis(true)
            .shows_indicators(false)
            .scroll_disabled(hold_active || !outer.pan_enabled())
            .refreshable(move |_, cx| {
                refresh.refresh_active_tab(refresh.active_tab(cx), cx);
                Task::ready(())
            })
            .list_shell_scroll_under_chrome(cx)
            .child(env_scope::<HoldContextMenuLocksScrollKey>(hold_active).child(stack));

        let keys_refresh = self.clone();
        div()
            .size_full()
            .bg(page_color)
            .child(PageRefreshKeys::new(
                child_id(&self.id, "refresh-keys"),
                move |_, cx| keys_refresh.refresh_active_tab(keys_refresh.active_tab(cx), cx),
                scroller,
            ))
            .into_any_element()
    }

    fn conversation_is_directly_behind(&self, chat: &ChatInfo, cx: &App) -> bool {
        let Some(nav_store) = &self.nav_store else {
            return false;
        };
        let nav = nav_store.read(cx);
        ChatProfileNavigationPolicy::conversation_is_directly_behind(
            &nav.path,
            &self.conversation_identifiers(chat),
            nav.embedded_forum_chat_id(),
        )
    }

    fn conversation_identifiers(&self, chat: &ChatInfo) -> HashSet<String> {
        [chat.id.as_str(), self.resolved_chat_id.as_str(), self.chat_id.as_deref().unwrap_or_default()]
            .into_iter()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn select_chat_profile_tab(&self, index: i64, visible_tabs: &[ChatProfileTab], haptic: bool, cx: &mut App) {
        let Some(tab) = usize::try_from(index).ok().and_then(|index| visible_tabs.get(index).copied()) else {
            return;
        };
        if index == self.active_tab(cx) {
            return;
        }
        if haptic {
            let _ = selection_changed();
        }
        let coordinator = self.state.update(cx, |state, cx| {
            state.active_tab = index;
            cx.notify();
            state.scroll_coordinator.clone()
        });
        coordinator.update(cx, |coordinator, cx| coordinator.set_active_tab_key(tab.scroll_key(), cx));
        self.update_navigation_pop_priority(index, cx);
    }

    fn select_adjacent_chat_profile_tab(&self, direction: i64, visible_tabs: &[ChatProfileTab], cx: &mut App) {
        let Some(target) =
            ProfileHorizontalPagerPolicy::target_index(self.active_tab(cx), direction, visible_tabs.len() as i64)
        else {
            return;
        };
        self.select_chat_profile_tab(target, visible_tabs, true, cx);
    }

    fn reset_chat_profile_scroll_if_needed(&self, scope: &str, visible_tabs: &[ChatProfileTab], cx: &mut App) {
        let Some(first) = visible_tabs.first().copied() else {
            return;
        };
        if self.state.read(cx).configured_scroll_scope == scope {
            return;
        }
        let coordinator = self.state.update(cx, |state, cx| {
            state.configured_scroll_scope = scope.to_owned();
            state.active_tab = 0;
            cx.notify();
            state.scroll_coordinator.clone()
        });
        coordinator.update(cx, |coordinator, cx| {
            coordinator.set_visible_horizontal_tab_keys(visible_tabs.iter().map(|tab| tab.scroll_key()).collect(), cx);
            coordinator.set_scope_key(scope, first.scroll_key(), cx);
        });
        self.update_navigation_pop_priority(0, cx);
    }

    /// the right swipe is taken from screen dismissal only inside tab pages
    /// elsewhere like the header the profile still closes by swipe
    fn update_navigation_pop_priority(&self, active_index: i64, cx: &mut App) {
        let (coordinator, owner) = {
            let state = self.state.read(cx);
            (state.scroll_coordinator.clone(), state.navigation_pop_paging_owner)
        };
        let pager = coordinator.read(cx).horizontal_pager_host_scroll_view();
        NavigationInteractivePopGuard::set_profile_tab_paging_back(
            active_index > 0,
            owner,
            Some(Rc::new(move |point: Point<Pixels>| pager.is_in_window() && pager.frame().contains(&point))),
            cx,
        );
    }

    fn chat_profile_scroll_scope(&self, chat: &ChatInfo) -> String {
        let topic = if self.is_general { "general" } else { self.topic_id.as_deref().unwrap_or("root") };
        format!(
            "chat:{}|topic:{}|type:{:?}|members:{}|roster:{}|gifts:{}",
            chat.id, topic, chat.r#type, self.members_first, self.shows_members_tab, self.gifts_first
        )
    }

    fn profile_right_view(
        self: &Rc<Self>,
        chat: &ChatInfo,
        can_manage: bool,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let (tabs_pinned, active_tab) = {
            let state = self.state.read(cx);
            (state.tabs_pinned, state.active_tab)
        };
        if tabs_pinned && self.profile_tab(active_tab) == Some(ChatProfileTab::Media) && !self.profile_content_restricted {
            return Some(self.media_grid_menu_button(chat, cx));
        }
        if ChatPresentationPolicy::profile_mode(chat.r#type) != ChatProfilePresentationMode::Full {
            return None;
        }
        if self.is_ordinary_private_peer(chat, cx) {
            let peer_id = &chat.participant.id;
            let has_contact = UserServicesStore::shared(cx).update(cx, |store, cx| {
                store.is_saved_contact_loaded(peer_id, cx) && store.saved_contact(peer_id, cx).is_some()
            });
            return has_contact.then(|| self.contact_edit_button(theme));
        }
        if chat.is_bot_chat() {
            return (self.can_manage_owned_bot(chat, cx) && !self.is_general).then(|| self.edit_button(chat, theme));
        }
        (can_manage && !self.is_general).then(|| self.edit_button(chat, theme))
    }

    const MEDIA_ZOOM_IN_ACTION_ID: &'static str = "chat_profile_media_zoom_in";
    const MEDIA_ZOOM_OUT_ACTION_ID: &'static str = "chat_profile_media_zoom_out";
    const MEDIA_SHOW_PHOTOS_ACTION_ID: &'static str = "chat_profile_media_show_photos";
    const MEDIA_SHOW_VIDEOS_ACTION_ID: &'static str = "chat_profile_media_show_videos";

    fn media_grid_scope(topic_id: Option<&str>, chat_id: &str) -> String {
        MessageServicesStore::message_scope(chat_id, topic_id, None)
    }

    fn media_grid_menu_actions(prefs: &ChatProfileMediaGridPrefs, scope: &str) -> Vec<HoldContextMenuAction> {
        let photos = prefs.shows_photos(scope);
        let videos = prefs.shows_videos(scope);
        vec![
            HoldContextMenuAction::new(Self::MEDIA_ZOOM_IN_ACTION_ID, tr("chat_profile_media_zoom_in"))
                .system_image("plus.magnifyingglass")
                .disabled(!prefs.can_zoom_in()),
            HoldContextMenuAction::new(Self::MEDIA_ZOOM_OUT_ACTION_ID, tr("chat_profile_media_zoom_out"))
                .system_image("minus.magnifyingglass")
                .disabled(!prefs.can_zoom_out()),
            HoldContextMenuAction::new(Self::MEDIA_SHOW_PHOTOS_ACTION_ID, tr("chat_profile_media_show_photos"))
                .system_image("photo")
                .disabled(photos && !videos)
                .section_break(true)
                .checked(photos),
            HoldContextMenuAction::new(Self::MEDIA_SHOW_VIDEOS_ACTION_ID, tr("chat_profile_media_show_videos"))
                .system_image("video")
                .disabled(videos && !photos)
                .checked(videos),
        ]
    }

    fn media_grid_menu_button(self: &Rc<Self>, chat: &ChatInfo, cx: &mut App) -> AnyElement {
        let scope = Self::media_grid_scope(self.topic_id.as_deref(), &chat.id);
        let prefs = ChatProfileMediaGridPrefs::shared(cx);
        let actions_scope = scope.clone();
        let state = self.state.clone();
        let open_state = self.state.clone();
        let button = PageHeaderIconButton::new(child_id(&self.id, "media-grid-menu"), "ellipsis", move |_, cx| {
            open_state.update(cx, |state, cx| {
                state.is_media_grid_menu_presented = true;
                cx.notify();
            });
        })
        .font_size(17.0)
        .tap_context_menu(
            child_id(&self.id, "media-grid-menu-anchor"),
            Binding::entity(
                state,
                |state: &ChatProfileState| state.is_media_grid_menu_presented,
                |state: &mut ChatProfileState, value, cx| {
                    state.is_media_grid_menu_presented = value;
                    cx.notify();
                },
            ),
            HoldContextMenuAlignment::Trailing,
            false,
            false,
            false,
            None,
            Rc::new(move |cx: &App| Self::media_grid_menu_actions(prefs.read(cx), &actions_scope)),
            Rc::new(move |id: String, _: &mut Window, cx: &mut App| Self::handle_media_grid_menu_action(&id, &scope, cx)),
            Rc::new(|_: &mut Window, _: &mut App| {
                PageHeaderIconButton::new("chat-profile-media-grid-menu-source", "ellipsis", |_, _| {})
                    .font_size(17.0)
                    .into_any_element()
            }),
        );
        div()
            .id(child_id(&self.id, "media-grid-menu-label"))
            .aria_label(tr("share_action_more"))
            .child(button)
            .into_any_element()
    }

    fn handle_media_grid_menu_action(id: &str, scope: &str, cx: &mut App) {
        let prefs = ChatProfileMediaGridPrefs::shared(cx);
        prefs.update(cx, |prefs, cx| match id {
            Self::MEDIA_ZOOM_IN_ACTION_ID => prefs.zoom_in(cx),
            Self::MEDIA_ZOOM_OUT_ACTION_ID => prefs.zoom_out(cx),
            Self::MEDIA_SHOW_PHOTOS_ACTION_ID => prefs.toggle_photos(scope, cx),
            Self::MEDIA_SHOW_VIDEOS_ACTION_ID => prefs.toggle_videos(scope, cx),
            _ => {}
        });
    }

    fn can_edit_topic(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        let Some(topic_id) = self.topic_id.as_deref().filter(|id| !id.is_empty() && !self.is_general) else {
            return false;
        };
        if ChatProfilePermissions::is_owner(chat) {
            return true;
        }
        let chat_id = self.resolved_chat_id.as_str();
        ChatServicesStore::shared(cx).update(cx, |store, cx| {
            let has_permissions = store.effective_permissions(chat_id, Some(topic_id), cx).is_some()
                || store.effective_permissions(chat_id, None, cx).is_some();
            if !has_permissions {
                return ChatProfilePermissions::is_owner_or_admin(chat);
            }
            let kind = store
                .topic_from_any_cache(chat_id, topic_id, cx)
                .map_or(ChatChannelKind::Text, |topic| topic.channel_kind);
            store.can_manage_channel(chat_id, Some(topic_id), kind, cx)
        })
    }

    fn glass_edit_label(theme: &AppTheme) -> MainText {
        MainText::new(tr("common_edit")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary)
    }

    fn contact_edit_button(self: &Rc<Self>, theme: &AppTheme) -> AnyElement {
        let state = self.state.clone();
        LiquidGlassButton::new(child_id(&self.id, "contact-edit"), LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(10.0)
            .on_press(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.show_contact_editor = true;
                    cx.notify();
                });
            })
            .child(Self::glass_edit_label(theme))
            .into_any_element()
    }

    fn edit_button(self: &Rc<Self>, chat: &ChatInfo, theme: &AppTheme) -> AnyElement {
        let body = self.clone();
        let chat = chat.clone();
        LiquidGlassButton::new(child_id(&self.id, "edit"), LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(10.0)
            .on_press(move |_, cx| body.edit_pressed(&chat, cx))
            .child(Self::glass_edit_label(theme))
            .guide_anchor(RiseGuideAnchor::ChatProfileEdit)
            .into_any_element()
    }

    fn edit_pressed(&self, chat: &ChatInfo, cx: &mut App) {
        if self.topic_id.is_some() {
            self.state.update(cx, |state, cx| {
                state.show_topic_settings = true;
                cx.notify();
            });
        } else if chat.is_bot_chat() {
            if !self.can_manage_owned_bot(chat, cx) {
                return;
            }
            if let Some(bot_id) = self.managed_bot_id(chat, cx) {
                self.push(RootRoute::BotSettings { bot_id }, cx);
            } else {
                self.state.update(cx, |state, _| state.open_bot_settings_when_resolved = true);
                self.load_bot_details(chat, cx);
            }
        } else {
            self.push(RootRoute::ChatSettings { chat_id: Some(chat.id.clone()) }, cx);
        }
    }

    fn bot_tag(&self, chat: &ChatInfo) -> String {
        BotChatManagement::normalized_tag(chat)
    }

    fn managed_bot(&self, chat: &ChatInfo, cx: &mut App) -> Option<BotModel> {
        if !chat.is_bot_chat() {
            return None;
        }
        let tag = self.bot_tag(chat);
        if !BotTagRules::is_valid(&tag) {
            return None;
        }
        BotServicesStore::bot_by_tag(&tag, cx)
    }

    fn managed_bot_id(&self, chat: &ChatInfo, cx: &mut App) -> Option<String> {
        if !self.can_manage_owned_bot(chat, cx) {
            return None;
        }
        self.managed_bot(chat, cx).map(|bot| bot.id)
    }

    fn can_manage_owned_bot(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        let bot = self.managed_bot(chat, cx);
        BotChatManagement::can_manage(chat, &Self::current_user_id(cx), bot.as_ref())
    }

    fn load_bot_details(&self, chat: &ChatInfo, cx: &mut App) {
        if !chat.is_bot_chat() {
            return;
        }
        let tag = self.bot_tag(chat);
        if !BotTagRules::is_valid(&tag) {
            return;
        }
        BotInteractionsStore::shared(cx).update(cx, |store, cx| store.load_bot_by_tag(&tag, cx));
    }

    fn header_cover(&self, chat: &ChatInfo, peer_id: Option<&str>, cx: &mut App) -> Option<String> {
        if let Some(peer_id) = peer_id {
            return ChatProfilePeerBanner::cover(peer_id, cx);
        }
        if self.is_topic_profile() {
            return None;
        }
        let bot = self.managed_bot(chat, cx)?;
        ChatProfilePeerBanner::picture(Some(&bot.banner_url))
    }

    fn header(self: &Rc<Self>, chat: &ChatInfo, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let peer_id = self.gifts_peer_user_id.clone();
        let cover = self.header_cover(chat, peer_id.as_deref(), cx);

        let topic_title = self.topic_title(cx);
        let name_missing = topic_title.is_none() && Self::chat_name_missing(chat);
        let title_row = if let Some(topic_title) = topic_title {
            hstack(6.0)
                .max_w_full()
                .min_w_0()
                .child(MainText::new(topic_title).px(23.0).font_weight(FontWeight::BOLD).number_of_lines(1).min_w_0())
                .into_any_element()
        } else {
            let verified_side = px(17.0);
            hstack(6.0)
                .max_w_full()
                .min_w_0()
                .child(
                    ChatTitleRow::new(chat.clone())
                        .title_px(23.0)
                        .fav_px(23.0)
                        .icon_size(15.0)
                        .font_weight(FontWeight::BOLD)
                        .explains_badges(true),
                )
                .children(if chat.is_verified {
                    IconUi::sized("checkmark.seal.fill", verified_side, theme.primary._100, true).map(|seal| {
                        div()
                            .id("chat-profile-verified-seal")
                            .flex_none()
                            .cursor_pointer()
                            .tooltip_label(tr("desktop_badge_verified_hint"))
                            .on_click(|_: &gpui::ClickEvent, window, cx| BadgeInfoPrompt::official(window, cx))
                            .child(seal)
                    })
                } else {
                    None
                })
                .into_any_element()
        };

        let is_peer_online = chat.r#type == ChatType::Direct
            && PresenceServicesStore::shared(cx).read(cx).is_online(&chat.participant.id, chat.participant.is_online);
        let subtitle = MainText::new(Self::profile_subtitle(chat, cx))
            .px(14.0)
            .tac(TextAlignT::Center)
            .color(if is_peer_online { theme.semantic.success_100 } else { theme.text.secondary });

        let column = vstack(HEADER_AVATAR_SPACING)
                    .w_full()
                    .items_center()
                    .px(px(24.0))
                    .child(self.chat_profile_avatar(chat, peer_id.clone(), cx))
                    .child(if name_missing {
                        vstack(3.0)
                            .items_center()
                            .child(
                                div()
                                    .h(px(28.0))
                                    .flex()
                                    .items_center()
                                    .child(SkeletonLine::new(150.0, 18.0))
                                    .skeleton_shimmer(None),
                            )
                            .child(
                                div()
                                    .h(px(18.0))
                                    .flex()
                                    .items_center()
                                    .child(SkeletonLine::new(96.0, 11.0))
                                    .skeleton_shimmer(None),
                            )
                    } else {
                        vstack(3.0).w_full().min_w_0().items_center().child(title_row).child(subtitle)
                    });

        ChatProfileHeaderLift {
            id: child_id(&self.id, "header-lift"),
            banner_id: child_id(&self.id, "peer-banner"),
            peer_id,
            cover,
            column: column.into_any_element(),
        }
        .into_any_element()
    }

    fn chat_name_missing(chat: &ChatInfo) -> bool {
        if chat.has_no_direct_peer() {
            return trimming_whitespaces_and_newlines(&chat.title).is_empty();
        }
        let name = if chat.r#type == ChatType::Direct {
            if chat.participant.name.is_empty() { &chat.participant.tag } else { &chat.participant.name }
        } else {
            &chat.title
        };
        trimming_whitespaces_and_newlines(name).is_empty()
    }

    fn chat_profile_avatar(self: &Rc<Self>, chat: &ChatInfo, peer_id: Option<String>, cx: &mut App) -> AnyElement {
        let hero_hidden = self.avatar_hero_hidden(chat, cx);
        let tap = self.clone();
        let tap_chat = chat.clone();
        let frame_id = child_id(&self.id, "avatar-frame");
        let Some(user_id) = self.chat_story_user_id(chat) else {
            return div()
                .id(child_id(&self.id, "avatar"))
                .flex_none()
                .cursor_pointer()
                .child(Self::chat_profile_avatar_face(&self.id, chat, hero_hidden))
                .on_click(move |_, _, cx| tap.open_chat_profile_avatar(&tap_chat, cx))
                .gift_worn_frame(frame_id, peer_id, HEADER_AVATAR_SIZE)
                .into_any_element();
        };

        let press_state = self.state.clone();
        let stories = StoryServicesStore::shared(cx);
        let actions_user_id = user_id.clone();
        let action_view = self.clone();
        let action_tag = chat.participant.tag.clone();
        let source_view = self.clone();
        let source_chat = chat.clone();
        PressableUi::new(child_id(&self.id, "avatar"))
            .tap_scale(1.0)
            .long_press_scale(1.0)
            .long_press_duration(HoldContextMenuMetrics::CONFIRM_DURATION as f32)
            .on_press(move |_, cx| tap.open_chat_profile_avatar(&tap_chat, cx))
            .on_long_press(move |_, cx| {
                press_state.update(cx, |state, cx| {
                    state.is_avatar_menu_presented = true;
                    cx.notify();
                });
            })
            .child(Self::chat_profile_avatar_face(&self.id, chat, hero_hidden))
            .tap_context_menu(
                child_id(&self.id, "avatar-menu"),
                Binding::entity(
                    self.state.clone(),
                    |state: &ChatProfileState| state.is_avatar_menu_presented,
                    |state: &mut ChatProfileState, value, cx| {
                        state.is_avatar_menu_presented = value;
                        cx.notify();
                    },
                ),
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                Rc::new(move |cx: &App| Self::avatar_menu_actions(stories.read(cx), &actions_user_id, cx)),
                Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                    action_view.handle_avatar_menu_action(&id, &user_id, &action_tag, cx);
                }),
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    let hero_hidden = source_view.avatar_hero_hidden(&source_chat, cx);
                    Self::chat_profile_avatar_face(&source_view.id, &source_chat, hero_hidden).into_any_element()
                }),
            )
            .gift_worn_frame(frame_id, peer_id, HEADER_AVATAR_SIZE)
            .into_any_element()
    }

    fn chat_profile_avatar_face(id: &ElementId, chat: &ChatInfo, hero_hidden: bool) -> Div {
        let registry_id = MediaViewerFrameKeys::chat_avatar(&chat.id);
        div()
            .relative()
            .flex_none()
            .size(px(HEADER_AVATAR_SIZE))
            .rounded_full()
            .opacity(if hero_hidden { 0.0 } else { 1.0 })
            .child(
                WindowScreenFrameReader::new(registry_id)
                    .source_media_radii(Some(MessageBubbleRadii::uniform(px(HEADER_AVATAR_RADIUS)))),
            )
            .child(
                ChatLogo::new(child_id(id, "avatar-logo"), chat.clone())
                    .size(HEADER_AVATAR_SIZE)
                    .icon_size(HEADER_AVATAR_ICON_SIZE)
                    .show_online_indicator(false),
            )
    }

    fn avatar_hero_hidden(&self, chat: &ChatInfo, cx: &App) -> bool {
        let registry_id = MediaViewerFrameKeys::chat_avatar(&chat.id);
        self.state.read(cx).avatar_lift.as_deref() == Some(registry_id.as_str())
    }

    fn avatar_menu_actions(stories: &StoryServicesStore, user_id: &str, cx: &App) -> Vec<HoldContextMenuAction> {
        let mut actions =
            vec![HoldContextMenuAction::new("open_profile", tr("ctx_open_profile")).system_image("person.crop.circle")];
        if stories.has_active_stories(Some(user_id), cx) {
            actions.push(HoldContextMenuAction::new("view_story", tr("ctx_view_story")).system_image("play.circle"));
        }
        actions
    }

    fn handle_avatar_menu_action(self: &Rc<Self>, id: &str, user_id: &str, tag: &str, cx: &mut App) {
        match id {
            "open_profile" => self.push(
                RootRoute::UserPage { user_id: Some(user_id.to_owned()), tag: (!tag.is_empty()).then(|| tag.to_owned()) },
                cx,
            ),
            "view_story" => {
                StoryInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.open_stories_for_user_id(user_id, None, cx));
            }
            _ => {}
        }
    }

    fn open_chat_profile_avatar(self: &Rc<Self>, chat: &ChatInfo, cx: &mut App) {
        if let Some(user_id) = self.chat_story_user_id(chat) {
            let (has_active, has_known) = {
                let stories = StoryServicesStore::shared(cx);
                let stories = stories.read(cx);
                (stories.has_active_stories(Some(&user_id), cx), stories.has_known_stories_state(Some(&user_id), cx))
            };
            if has_active {
                let _ = selection_changed();
                StoryInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.open_stories_for_user_id(&user_id, None, cx));
                return;
            }
            if !has_known {
                let _ = selection_changed();
                let body = self.clone();
                let unavailable_chat = chat.clone();
                let on_unavailable: StoryActionDoneHandler =
                    Rc::new(move |cx: &mut App| body.open_avatar_media_viewer(&unavailable_chat, cx));
                StoryInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.open_stories_for_user_id(&user_id, Some(on_unavailable), cx));
                return;
            }
        }
        self.open_avatar_media_viewer(chat, cx);
    }

    fn open_avatar_media_viewer(&self, chat: &ChatInfo, cx: &mut App) {
        let Some(source) = self.chat_avatar_media_source(chat, cx) else {
            return;
        };
        let registry_id = MediaViewerFrameKeys::chat_avatar(&chat.id);
        let deletes_photo = self.can_delete_chat_photo(chat, cx);
        let owned_bot_id = if chat.is_bot_chat() { self.managed_bot_id(chat, cx) } else { None };
        let item = MediaViewerGalleryItem {
            id: registry_id.clone(),
            kind: MediaViewerGalleryItemKind::Image,
            url: source.url.clone(),
            thumbnail_url: None,
            width: 1.0,
            height: 1.0,
            duration: None,
            sender_id: chat.participant.id.clone(),
            sender_name: if chat.r#type == ChatType::Direct { chat.participant.name.clone() } else { chat.title.clone() },
            sender_logo: Some(source.url.clone()),
            created_at: 0.0,
            caption_title: String::new(),
            caption: String::new(),
            message_id: String::new(),
            index_in_message: 0,
            media_item_id: registry_id.clone(),
            uses_fit_letterbox: false,
            frame_registry_id: registry_id.clone(),
            is_own: false,
            can_delete: false,
            can_save: true,
            can_forward: false,
            can_reply: false,
            can_show_in_context: false,
            can_delete_for_me: false,
            can_delete_for_everyone: false,
            can_set_as_main: false,
            is_album_row: deletes_photo,
            display_source_override: Some(source.url.clone()),
            persist_media_to_disk: source.cache_policy == CleverImageCachePolicy::Standard,
            deletes_from_favourites: false,
            post_download: None,
            image_cache_policy: source.cache_policy,
        };
        let source_frame = MediaCellScreenFrameRegistry::frame(&registry_id).unwrap_or_default();
        let hero_skin = MediaViewerHeroSkin {
            display_source: source.url,
            uses_fit_foreground: false,
            media_tail: None,
            source_media_radii: Some(MessageBubbleRadii::uniform(px(HEADER_AVATAR_RADIUS))),
        };
        let chrome = if deletes_photo {
            MediaViewerChrome::OWN_PROFILE_ALBUM
        } else {
            MediaViewerChrome { shows_bottom_bar: false, can_report: false, ..MediaViewerChrome::default() }
        };
        let state = self.media_presenter.clone();
        let on_delete_photo = deletes_photo.then(|| {
            let presenter = state.clone();
            let chat_id = chat.id.clone();
            Rc::new(move |_: &MediaViewerGalleryItem, cx: &mut App| {
                if let Some(bot_id) = owned_bot_id.as_deref() {
                    let dismiss = presenter.clone();
                    let on_result: BotActionResult<BotModel> = Rc::new(move |_, cx: &mut App| {
                        dismiss.update(cx, |presenter, cx| presenter.request_animated_dismissal(cx));
                    });
                    let values = BotUpdateValues { avatar_url: Some(String::new()), ..BotUpdateValues::default() };
                    BotInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.update_bot(bot_id, values, Some(on_result), cx));
                    return;
                }
                let updated = ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.update_chat(&chat_id, None, None, Some(""), cx));
                if !updated {
                    return;
                }
                presenter.update(cx, |presenter, cx| presenter.request_animated_dismissal(cx));
            }) as MediaViewerItemHandler
        });
        let handlers = MediaViewerPresentationHandlers { on_delete_photo, ..MediaViewerPresentationHandlers::default() };
        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                state,
                vec![item],
                0,
                source_frame,
                registry_id,
                hero_skin,
                chrome,
                MediaViewerGalleryScope::Complete,
                handlers,
                cx,
            )
            .await;
        })
        .detach();
    }

    fn can_delete_chat_photo(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.is_bot_chat() {
            let Some(bot) = self.managed_bot(chat, cx) else {
                return false;
            };
            if self.managed_bot_id(chat, cx).is_none() {
                return false;
            }
            let avatar = trimming_whitespaces_and_newlines(&bot.avatar_url);
            return !avatar.is_empty() && !AvatarPalette::is_generated_avatar_url(Some(avatar));
        }
        if chat.r#type == ChatType::Direct || !ChatProfilePermissions::can_edit_profile(chat, cx) {
            return false;
        }
        let logo = chat.logo_url.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
        !logo.is_empty() && !AvatarPalette::is_generated_avatar_url(Some(logo))
    }

    fn chat_story_user_id(&self, chat: &ChatInfo) -> Option<String> {
        if chat.r#type != ChatType::Direct {
            return None;
        }
        let user_id = trimming_whitespaces_and_newlines(&chat.participant.id);
        (!user_id.is_empty()).then(|| user_id.to_owned())
    }

    fn chat_avatar_media_source(&self, chat: &ChatInfo, cx: &mut App) -> Option<ChatProfileAvatarMediaSource> {
        if chat.r#type == ChatType::Direct {
            let contact = UserServicesStore::shared(cx)
                .update(cx, |store, cx| store.saved_contact(&chat.participant.id, cx));
            if let Some(contact) = contact
                && let Some(url) = AvatarPalette::resolved_logo_url(Some(&contact.effective_avatar_url()))
            {
                let has_private_override = contact
                    .avatar_override_url
                    .as_deref()
                    .is_some_and(|url| !trimming_whitespaces_and_newlines(url).is_empty());
                return Some(ChatProfileAvatarMediaSource {
                    url,
                    cache_policy: if has_private_override {
                        CleverImageCachePolicy::PrivateEphemeral
                    } else {
                        CleverImageCachePolicy::Standard
                    },
                });
            }
        }
        let url = AvatarPalette::resolved_logo_url_for_chat(chat)?;
        Some(ChatProfileAvatarMediaSource { url, cache_policy: CleverImageCachePolicy::Standard })
    }

    fn presence_view_key(chat: &ChatInfo, cx: &mut App) -> u64 {
        let mut hasher = DefaultHasher::new();
        if chat.r#type == ChatType::Direct {
            PresenceServicesStore::shared(cx).read(cx).is_online(&chat.participant.id, chat.participant.is_online).hash(&mut hasher);
        }
        Self::profile_subtitle(chat, cx).hash(&mut hasher);
        hasher.finish()
    }

    fn profile_subtitle(chat: &ChatInfo, cx: &mut App) -> String {
        match chat.r#type {
            ChatType::Direct => {
                if chat.participant.is_deleted {
                    return tr("chat_status_long_ago");
                }
                if chat.has_no_direct_peer() {
                    return String::new();
                }
                PresenceServicesStore::shared(cx).update(cx, |store, cx| store.status_text_for_chat(chat, cx))
            }
            ChatType::Group => ForumMemberGrouping::members_line(
                &chat.id,
                chat.member_count,
                Some(ChatCountFormatter::members(chat.member_count)),
                cx,
            ),
            ChatType::Channel => ChatCountFormatter::subscribers(chat.member_count),
            ChatType::Favourites => tr("chat_status_favourites"),
        }
    }

    /// roles open in every group and channel and not only in a forum
    fn roles_item(self: &Rc<Self>, chat: &ChatInfo) -> GroupedBtn {
        let body = self.clone();
        let chat_id = chat.id.clone();
        GroupedBtn::new("roles", tr("roles_title"))
            .icon("person.3.sequence.fill")
            .icon_background(ChatTileColor::indigo())
            .callback(move |_, cx| body.push(RootRoute::ChatRoles { chat_id: Some(chat_id.clone()) }, cx))
    }

    fn members_item(self: &Rc<Self>, chat: &ChatInfo, id: &'static str, title_key: &'static str) -> GroupedBtn {
        let body = self.clone();
        let chat_id = chat.id.clone();
        GroupedBtn::new(id, tr(title_key))
            .icon("person.2.fill")
            .icon_background(ChatTileColor::blue())
            .trailing(ChatCountFormatter::compact(chat.member_count))
            .callback(move |_, cx| body.push(RootRoute::ChatSettingsMembers { chat_id: Some(chat_id.clone()) }, cx))
    }

    fn settings_item(self: &Rc<Self>, chat: &ChatInfo, title_key: &'static str) -> GroupedBtn {
        let body = self.clone();
        let chat_id = chat.id.clone();
        GroupedBtn::new("settings", tr(title_key))
            .icon("slider.horizontal.3")
            .icon_background(ChatTileColor::orange())
            .callback(move |_, cx| body.push(RootRoute::ChatSettings { chat_id: Some(chat_id.clone()) }, cx))
    }

    fn nav_items(self: &Rc<Self>, chat: &ChatInfo, can_manage: bool, cx: &mut App) -> Vec<GroupedBtn> {
        let mut items = Vec::new();
        let opens_settings = can_manage
            || ChatProfilePermissions::manages_reactions(chat, cx)
            || ChatProfilePermissions::manages_forum_structure(chat, cx)
            || ChatProfilePermissions::reads_moderation_lists(chat, cx);
        match chat.r#type {
            ChatType::Channel => {
                if can_manage {
                    items.push(self.members_item(chat, "subscribers", "chat_profile_subscribers"));
                }
                if can_manage || ChatProfilePermissions::manages_roles(chat, cx) {
                    items.push(self.roles_item(chat));
                }
                if opens_settings {
                    items.push(self.settings_item(chat, "chat_channel_settings_title"));
                }
            }
            ChatType::Group => {
                if can_manage {
                    items.push(self.members_item(chat, "members", "chat_profile_members"));
                }
                if can_manage || ChatProfilePermissions::manages_roles(chat, cx) {
                    items.push(self.roles_item(chat));
                }
                if opens_settings {
                    items.push(self.settings_item(chat, "chat_group_settings_title"));
                }
                if !can_manage && ChatProfilePermissions::distributes_invites(chat, cx) {
                    let body = self.clone();
                    let chat_id = chat.id.clone();
                    items.push(
                        GroupedBtn::new("links", tr("chat_settings_invation_links"))
                            .icon("link")
                            .icon_background(ChatTileColor::orange())
                            .callback(move |_, cx| {
                                body.push(RootRoute::ChatSettingsLinks { chat_id: Some(chat_id.clone()) }, cx);
                            }),
                    );
                }
            }
            ChatType::Direct | ChatType::Favourites => {}
        }
        items
    }

    fn show_block_scope_dialog_action(state: Entity<ChatProfileState>) -> impl Fn(&mut Window, &mut App) + 'static {
        move |_, cx| {
            state.update(cx, |state, cx| {
                state.show_block_scope_dialog = true;
                cx.notify();
            });
        }
    }

    fn contact_action_items(self: &Rc<Self>, chat: &ChatInfo, theme: &AppTheme, cx: &mut App) -> Vec<GroupedBtn> {
        if !self.is_blockable_peer(chat, cx) {
            return Vec::new();
        }

        let target_id = chat.participant.id.clone();
        let mut items = Vec::new();
        let user_store = UserServicesStore::shared(cx);

        let is_contact_peer = self.is_ordinary_private_peer(chat, cx);
        let is_loaded = user_store.update(cx, |store, cx| store.is_saved_contact_loaded(&target_id, cx));
        if is_contact_peer && !is_loaded {
            if user_store.read(cx).saved_contact_failed_targets.contains(&target_id) {
                let retry_target = target_id.clone();
                items.push(
                    GroupedBtn::new("retry_contact_state", tr("contact_state_retry"))
                        .icon("arrow.clockwise")
                        .icon_background(Hsla::from(rgb(RETRY_CONTACT_COLOR)))
                        .callback(move |_, cx| {
                            UserInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.refresh_saved_contact(&retry_target, cx));
                        }),
                );
            }
        } else if is_contact_peer
            && user_store.update(cx, |store, cx| store.saved_contact(&target_id, cx)).is_none()
        {
            let state = self.state.clone();
            items.push(
                GroupedBtn::new("add_contact", tr("contact_add_action"))
                    .icon("person.crop.circle.badge.plus")
                    .icon_background(Hsla::from(rgb(ADD_CONTACT_COLOR)))
                    .disabled(user_store.read(cx).saved_contact_pending_targets.contains(&target_id))
                    .callback(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.show_contact_editor = true;
                            cx.notify();
                        });
                    }),
            );
        }
        let block_scope = user_store.update(cx, |store, cx| store.viewer_block_scope(&target_id, cx));
        let block_pending = user_store.read(cx).contact_block_pending_targets.contains(&target_id);
        let Some(block_scope) = block_scope else {
            items.push(
                GroupedBtn::new("block_contact", tr("contact_block_action"))
                    .icon("hand.raised.fill")
                    .icon_background(theme.semantic.error_100)
                    .destructive(true)
                    .disabled(block_pending)
                    .guide_anchor(RiseGuideAnchor::ChatProfileBlock)
                    .show_chevron(false)
                    .callback(Self::show_block_scope_dialog_action(self.state.clone())),
            );
            return items;
        };

        let body = self.clone();
        let unblock_chat = chat.clone();
        items.push(
            GroupedBtn::new("block_contact", tr("contact_unblock_action"))
                .icon("hand.raised.slash.fill")
                .icon_background(Hsla::from(rgb(UNBLOCK_CONTACT_COLOR)))
                .subtitle(Self::block_scope_label(block_scope))
                .disabled(block_pending)
                .show_chevron(false)
                .callback(move |_, cx| body.update_block_scope(&unblock_chat, None, cx)),
        );
        items.push(
            GroupedBtn::new("block_scope_change", tr("contact_block_scope_change_action"))
                .icon("slider.horizontal.3")
                .icon_background(ChatTileColor::orange())
                .disabled(block_pending)
                .guide_anchor(RiseGuideAnchor::ChatProfileBlockScope)
                .show_chevron(false)
                .callback(Self::show_block_scope_dialog_action(self.state.clone())),
        );
        items
    }

    fn block_scope_label(scope: RiseBlockScope) -> String {
        match scope {
            RiseBlockScope::Standard => tr("contact_block_scope_standard"),
            RiseBlockScope::Total => tr("contact_block_scope_total"),
        }
    }

    fn present_block_scope_dialog(self: &Rc<Self>, chat: &ChatInfo, cx: &mut App) {
        let title_key = if UserServicesStore::shared(cx)
            .update(cx, |store, cx| store.viewer_has_blocked(&chat.participant.id, cx))
            == Some(true)
        {
            "contact_block_scope_title"
        } else {
            "contact_block_confirmation_title"
        };
        let message = format!(
            "{}: {}\n{}: {}",
            tr("contact_block_scope_standard"),
            tr("contact_block_scope_standard_desc"),
            tr("contact_block_scope_total"),
            tr("contact_block_scope_total_desc"),
        );
        let standard = self.clone();
        let standard_chat = chat.clone();
        let total = self.clone();
        let total_chat = chat.clone();
        let report_target = chat.participant.id.clone();
        let on_standard: NativeConfirmAlertAction = Box::new(move |cx: &mut App| {
            standard.update_block_scope(&standard_chat, Some(RiseBlockScope::Standard), cx)
        });
        let on_total: NativeConfirmAlertAction =
            Box::new(move |cx: &mut App| total.update_block_scope(&total_chat, Some(RiseBlockScope::Total), cx));
        let on_block_and_report: NativeConfirmAlertAction =
            Box::new(move |cx: &mut App| UserBlockAndReport::run(&report_target, None, None, cx));
        NativeConfirmAlert::present_destructive_choices(
            &tr(title_key),
            &message,
            vec![
                (tr("contact_block_scope_standard"), on_standard),
                (tr("contact_block_scope_total"), on_total),
                (tr("user_block_and_report"), on_block_and_report),
            ],
            None,
            cx,
        );
    }

    fn is_ordinary_private_peer(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        ChatContactProfilePolicy::is_eligible(chat, &Self::current_user_id(cx), self.is_topic_profile())
    }

    fn contact_peer(&self, chat: &ChatInfo, cx: &mut App) -> UserContactPeer {
        let profile = UserServicesStore::shared(cx)
            .update(cx, |store, cx| store.cached_foreign_profile_user_id(&chat.participant.id, None, cx))
            .and_then(|payload| payload.as_profile_model());
        UserContactPeer {
            id: chat.participant.id.clone(),
            name: chat.direct_peer_profile_name(),
            tag: chat.participant.tag.clone(),
            logo: chat.participant.more.logo.clone(),
            is_premium: chat.participant.more.is_premium,
            is_online: chat.participant.is_online,
            last_seen: chat.participant.last_seen,
            is_bot: chat.participant.is_bot,
            phone: profile.and_then(|profile| profile.more.phone),
        }
    }

    fn is_blockable_peer(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        ChatContactProfilePolicy::is_blockable(chat, &Self::current_user_id(cx), self.is_topic_profile())
    }

    fn refresh_contact_state_if_needed(&self, chat: &ChatInfo, cx: &mut App) {
        let interactions = UserInteractionsStore::shared(cx);
        if !self.is_ordinary_private_peer(chat, cx) {
            if self.is_blockable_peer(chat, cx) {
                interactions.update(cx, |store, cx| store.refresh_block_state(&chat.participant.id, cx));
            }
            return;
        }
        interactions.update(cx, |store, cx| {
            store.refresh_foreign_profile_user_id(chat.participant.id.clone(), cx);
            store.refresh_saved_contact(&chat.participant.id, cx);
        });
    }

    fn update_block_scope(&self, chat: &ChatInfo, scope: Option<RiseBlockScope>, cx: &mut App) {
        UserInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.set_user_block_scope(&chat.participant.id, scope, |_, _| {}, cx));
    }

    fn publish_chat_profile_route_params(&self, active_tab: i64, cx: &mut App) {
        let Some(tab) = self.profile_tab(active_tab) else {
            return;
        };
        if !self.owns_active_screen(cx) {
            return;
        }
        if let Some(nav_store) = &self.nav_store {
            let params = HashMap::from([(CURRENT_TAB_PARAM.to_owned(), tab.tab_key().to_owned())]);
            nav_store.update(cx, |store, cx| store.set_sai_active_screen_overlay_params(params, cx));
        }
    }

    fn owns_active_screen(&self, cx: &App) -> bool {
        if self.presents_in_side_panel {
            return false;
        }
        let Some(nav_store) = &self.nav_store else {
            return false;
        };
        match nav_store.read(cx).path.last() {
            Some(
                RootRoute::ChatProfile { chat_id }
                | RootRoute::GeneralProfile { chat_id }
                | RootRoute::TopicProfile { chat_id, .. },
            ) => *chat_id == self.chat_id,
            _ => false,
        }
    }

    fn open_conversation(&self, chat: &ChatInfo, cx: &mut App) {
        if self.is_topic_profile() {
            self.pop(cx);
            return;
        }
        let Some(nav_store) = self.nav_store.clone() else {
            return;
        };
        let decision = {
            let nav = nav_store.read(cx);
            ChatProfileNavigationPolicy::open_chat_decision(
                &nav.path,
                &self.conversation_identifiers(chat),
                nav.embedded_forum_chat_id(),
            )
        };
        match decision {
            ChatProfileOpenChatDecision::Unwind { depth } => {
                nav_store.update(cx, |store, cx| {
                    let path = store.path.iter().take(usize::try_from(depth).unwrap_or(0)).cloned().collect();
                    let tab = store.current_tab;
                    store.restore(path, tab, cx);
                });
            }
            ChatProfileOpenChatDecision::Push => {
                self.push(RootRoute::Chat { chat_id: Some(chat.id.clone()) }, cx);
            }
        }
    }

    fn load_active_tab(&self, tab_index: i64, cx: &mut App) {
        if self.resolved_chat_id.is_empty() {
            return;
        }
        let Some(selected_tab) = self.profile_tab(tab_index) else {
            return;
        };
        let loading_window = ChatProfilePaneMountPolicy::window(&self.ordered_profile_tabs, selected_tab);
        for tab in self.ordered_profile_tabs.iter().copied().filter(|tab| loading_window.contains(tab)) {
            self.load_profile_tab(tab, selected_tab, false, cx);
        }
    }

    fn load_profile_tab(&self, tab: ChatProfileTab, refresh_route_tab: ChatProfileTab, force_refresh: bool, cx: &mut App) {
        if self.profile_content_restricted {
            return;
        }
        let chat_id = self.resolved_chat_id.as_str();
        let topic_id = self.topic_id.as_deref();
        let route = Some(refresh_route_tab);
        let messages = MessageInteractionsStore::shared(cx);
        match tab {
            ChatProfileTab::Media => {
                messages.update(cx, |store, cx| store.load_initial_media_tab(chat_id, topic_id, route, force_refresh, cx));
            }
            ChatProfileTab::Fav => {
                messages.update(cx, |store, cx| store.load_initial_fav_tab(chat_id, route, force_refresh, cx));
            }
            ChatProfileTab::File => {
                messages.update(cx, |store, cx| store.load_initial_files_tab(chat_id, topic_id, route, force_refresh, cx));
            }
            ChatProfileTab::Link => {
                messages.update(cx, |store, cx| store.load_initial_link_tab(chat_id, topic_id, route, force_refresh, cx));
            }
            ChatProfileTab::Music => {
                messages.update(cx, |store, cx| store.load_initial_music_tab(chat_id, route, force_refresh, cx));
            }
            ChatProfileTab::Voice => {
                messages.update(cx, |store, cx| store.load_initial_audio_tab(chat_id, route, force_refresh, cx));
            }
            ChatProfileTab::Gif => {
                messages.update(cx, |store, cx| store.load_initial_gif_tab(chat_id, route, force_refresh, cx));
            }
            ChatProfileTab::Members => {
                let is_topic_profile = self.is_topic_profile();
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_members(chat_id, topic_id, Some(refresh_route_tab.tab_key()), force_refresh, cx);
                    if is_topic_profile {
                        store.load_roles(chat_id, cx);
                    }
                });
            }
            ChatProfileTab::Groups => {
                let peer_id = self.peer_user_id_for_common_groups();
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_common_chats(&peer_id, force_refresh, cx));
            }
            ChatProfileTab::Stories => {
                let peer_id = self.peer_user_id_for_common_groups();
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_profile_stories(Some(&peer_id), StoryProfileScope::Profile, force_refresh, cx);
                });
            }
            ChatProfileTab::Gifts => {
                let Some(peer_id) = self.gifts_peer_user_id.clone() else {
                    return;
                };
                cx.spawn(async move |cx| {
                    if force_refresh {
                        GiftsActionsStore::refresh_tab(&peer_id, cx).await;
                    } else {
                        GiftsActionsStore::load_tab(&peer_id, None, false, cx).await;
                    }
                })
                .detach();
            }
        }
    }

    fn confirmed_last_message_key(chat: &ChatInfo) -> Option<String> {
        let message = chat.last_message.as_ref()?;
        message
            .server_id
            .clone()
            .filter(|id| !id.is_empty())
            .or_else(|| (!message.id.is_empty() && !GetSaiServerId::is_temp_id(Some(message.id.as_str()))).then(|| message.id.clone()))
    }

    fn message_tabs_touched_by(message: &GetMessageMessage) -> Vec<ChatProfileTab> {
        let mut tabs = Vec::new();
        let mut add = |tab: ChatProfileTab| {
            if !tabs.contains(&tab) {
                tabs.push(tab);
            }
        };
        for item in &message.media_items {
            match item.kind {
                MessageMediaKind::Image | MessageMediaKind::Video => add(ChatProfileTab::Media),
                MessageMediaKind::Animation | MessageMediaKind::Gif => add(ChatProfileTab::Gif),
                MessageMediaKind::Document => add(ChatProfileTab::File),
                MessageMediaKind::Audio => add(ChatProfileTab::Music),
                MessageMediaKind::Voice | MessageMediaKind::VideoNote => add(ChatProfileTab::Voice),
                MessageMediaKind::Sticker => {}
            }
        }
        if message.link_preview.is_some() || FormattedText::contains_link(&message.text) {
            add(ChatProfileTab::Link);
        }
        tabs
    }

    fn refresh_message_tabs_window(&self, tab_index: i64, touched: &[ChatProfileTab], cx: &mut App) {
        if self.profile_content_restricted || self.resolved_chat_id.is_empty() {
            return;
        }
        let Some(selected_tab) = self.profile_tab(tab_index) else {
            return;
        };
        let loading_window = ChatProfilePaneMountPolicy::window(&self.ordered_profile_tabs, selected_tab);
        for tab in self.ordered_profile_tabs.iter().copied().filter(|tab| loading_window.contains(tab)) {
            if touched.contains(&tab) {
                self.load_profile_tab(tab, selected_tab, true, cx);
            }
        }
    }

    fn refresh_active_tab(&self, tab_index: i64, cx: &mut App) {
        if self.profile_content_restricted || self.resolved_chat_id.is_empty() {
            return;
        }
        let Some(tab) = self.profile_tab(tab_index) else {
            return;
        };
        match tab {
            ChatProfileTab::Members => {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.refresh_members(&self.resolved_chat_id, self.topic_id.as_deref(), cx);
                });
            }
            ChatProfileTab::Groups => {
                let peer_id = self.peer_user_id_for_common_groups();
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_common_chats(&peer_id, true, cx));
            }
            _ => self.load_profile_tab(tab, tab, true, cx),
        }
    }
}

struct ChatProfileAvatarMediaSource {
    url: String,
    cache_policy: CleverImageCachePolicy,
}

pub enum ChatProfileOverviewLayoutPolicy {}

impl ChatProfileOverviewLayoutPolicy {
    pub const FADE_DISTANCE: f32 = 88.0;

    /// the overview stays ordinary scroll content
    /// only its last visible strip fades through the background before the tab bar pins
    pub fn scroll_opacity(frame_max_y: f32, pinned_tabs_top: f32, outer_spacing: f32) -> f32 {
        let fully_hidden_max_y = pinned_tabs_top - outer_spacing;
        let progress = (frame_max_y - fully_hidden_max_y) / Self::FADE_DISTANCE.max(1.0);
        progress.clamp(0.0, 1.0)
    }
}

pub enum ChatProfileScrollCoordinatePolicy {}

impl ChatProfileScrollCoordinatePolicy {
    /// pinned tabs top counts the ios status bar and a desktop window has none
    /// so tabs pin at the bottom edge of the title bar
    pub fn local_pinned_tabs_top(floating_top_inset: f32) -> f32 {
        let inset = floating_top_inset.max(0.0);
        ProfileScrollMetrics::CHAT_PINNED_TABS_TOP.min(inset) - inset
    }
}

pub enum ChatProfileChromeBackdropPolicy {}

impl ChatProfileChromeBackdropPolicy {
    pub const FADE_IN_DISTANCE: f32 = 24.0;

    pub fn opacity(scrolled: f32) -> f32 {
        (scrolled / Self::FADE_IN_DISTANCE).clamp(0.0, 1.0)
    }
}

#[derive(IntoElement)]
struct ChatProfileHeaderLift {
    id: ElementId,
    banner_id: ElementId,
    peer_id: Option<String>,
    cover: Option<String>,
    column: AnyElement,
}

impl RenderOnce for ChatProfileHeaderLift {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let reach = list_shell_floating_top_inset(cx).max(0.0) + HEADER_TOP_PADDING;
        let lift = (PROFILE_BANNER_HEIGHT - reach - HEADER_AVATAR_SIZE / 2.0).max(ChatProfilePeerBanner::PICTURE_LIFT);
        let now = RiseClock::monotonic();
        let lift_state = window.use_keyed_state(self.id, cx, move |_, _| Animated::new(lift));
        let lifted = lift_state.update(cx, |animated, cx| {
            animated.set_in(lift, ease_out(HEADER_LIFT_ANIMATION_DURATION), now, cx);
            animated.frame_value(now, window)
        });
        let banner = div().absolute().top_0().left_0().right_0().child(ChatProfilePeerBanner::new(
            self.banner_id,
            self.peer_id,
            self.cover,
            HEADER_TOP_PADDING,
            lift + HEADER_AVATAR_SIZE / 2.0,
        ));
        div().relative().w_full().child(banner).child(div().w_full().pt(px(lifted)).child(self.column))
    }
}

#[derive(IntoElement)]
struct ChatProfileChromeBackdrop {
    handle: ScrollHandle,
    color: Hsla,
}

impl RenderOnce for ChatProfileChromeBackdrop {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let inset = list_shell_floating_top_inset(cx).max(0.0);
        let height = ChatProfileScrollCoordinatePolicy::local_pinned_tabs_top(inset) + inset;
        let handle = self.handle;
        let color = self.color;
        let blur = ScrollEdgeBlurFalloff::tint_stops(color, 0.0, accessibility_reduce_transparency(cx));
        let shadow = ScrollChromeShadowUi::new(ScrollChromeShadowStyle::list_shell_tall_header()).stops(cx);
        let direction = layout_direction(cx);
        canvas(
            |_, _, _| {},
            move |content_bounds, _, window, _| {
                let viewport_top = handle.bounds().top();
                let min_y = f32::from(content_bounds.top() - viewport_top) - inset;
                let opacity = ChatProfileChromeBackdropPolicy::opacity(-min_y);
                if opacity <= 0.0 {
                    return;
                }
                let frame = bounds(point(content_bounds.left(), viewport_top), size(content_bounds.size.width, px(height)));
                for layer in [&blur, &shadow] {
                    let stops = layer
                        .iter()
                        .map(|stop| GradientStop::new(stop.color.opacity(opacity), stop.location))
                        .collect();
                    LinearGradient::new(stops, UnitPoint::TOP, UnitPoint::BOTTOM).paint(frame, direction, window);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(height))
    }
}

/// gpui has no element opacity at paint time and the fade must not redraw on every scrolled pixel
/// so a veil of the page color is painted over the block
#[derive(IntoElement)]
struct ChatProfileOverviewFadeModifier {
    is_enabled: bool,
    handle: ScrollHandle,
    color: Hsla,
    content: AnyElement,
}

impl RenderOnce for ChatProfileOverviewFadeModifier {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let inset = list_shell_floating_top_inset(cx).max(0.0);
        let pinned_tabs_top = ChatProfileScrollCoordinatePolicy::local_pinned_tabs_top(inset);
        let handle = self.handle;
        let color = self.color;
        let is_enabled = self.is_enabled;
        div().relative().w_full().child(self.content).children(is_enabled.then(|| {
            canvas(
                |_, _, _| {},
                move |frame, _, window, _| {
                    let frame_max_y = f32::from(frame.bottom() - handle.bounds().top()) - inset;
                    let opacity = ChatProfileOverviewLayoutPolicy::scroll_opacity(
                        frame_max_y,
                        pinned_tabs_top,
                        ChatProfileViewportLayoutPolicy::OUTER_SPACING,
                    );
                    if opacity < 1.0 {
                        window.paint_quad(fill(frame, color.opacity(1.0 - opacity)));
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        }))
    }
}

#[derive(IntoElement)]
struct ChatProfileTabsPinnedProbe {
    state: Entity<ChatProfileState>,
    handle: ScrollHandle,
    content: AnyElement,
}

impl RenderOnce for ChatProfileTabsPinnedProbe {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let inset = list_shell_floating_top_inset(cx).max(0.0);
        let pinned_top = ChatProfileScrollCoordinatePolicy::local_pinned_tabs_top(inset);
        let is_pinned = self.state.read(cx).tabs_pinned;
        let handle = self.handle;
        let state = self.state;
        div().relative().w_full().child(self.content).child(
            canvas(
                move |frame, window, cx| {
                    let min_y = f32::from(frame.top() - handle.bounds().top()) - inset;
                    let pinned = min_y <= pinned_top + 1.0;
                    if pinned != is_pinned {
                        let state = state.clone();
                        window.defer(cx, move |_, cx| {
                            state.update(cx, |state, cx| {
                                if state.tabs_pinned != pinned {
                                    state.tabs_pinned = pinned;
                                    cx.notify();
                                }
                            });
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}

#[derive(IntoElement)]
struct ChatProfileMasterPinnedModifier {
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    content: AnyElement,
}

impl RenderOnce for ChatProfileMasterPinnedModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let inset = list_shell_floating_top_inset(cx).max(0.0);
        let pinned_tabs_top = ChatProfileScrollCoordinatePolicy::local_pinned_tabs_top(inset) + inset;
        ProfileMasterPinnedModifier::new(self.scroll_coordinator)
            .pinned_tabs_top(pinned_tabs_top)
            .body(self.content, window, cx)
    }
}

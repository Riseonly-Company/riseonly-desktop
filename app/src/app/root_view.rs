use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, ClickEvent, Context, Div, ElementId, Entity, ExternalPaths, FocusHandle, Focusable, Global, Hsla, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent, DragMoveEvent, NavigationDirection, Pixels, Render, SharedString, StyleRefinement, Subscription, Visibility, WeakEntity, Window,
    Bounds, canvas, deferred, div, fill, linear_color_stop, linear_gradient, point, prelude::*, px, relative, size,
};
use rise_canon::rise_key::guide_screen::camel as guide_screen_keys;
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::language_store::LanguageStore;
use rise_i18n::tr;
use rise_navigation::pane_layout::{PaneDemand, PaneLayout, PanePolicy};
use rise_platform::window_chrome::current_pane_corner_radius;
use rise_theme::{FONT_FAMILY, ThemeStore};
use rise_theme::theme::theme_environment::ThemeRootBoundary;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::NativeBottomSheetPresentationHost;
use rise_ui::keyboard_dismiss::{KeyboardDismissModifiers, ScreenFocus};
use rise_ui::reorder_grip::ReorderMotion;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::page_refresh::is_page_refresh_keystroke;
use rise_ui::swiftui::environment::{LayoutDirection, env_scope};
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::transition::UnitPoint;
use rise_ui::page_header_toolbar_button::{PageHeaderMetrics, PageHeaderToolbarSlot};
use rise_ui::page_header_ui::{PageBesideSplitListKey, PageCoveredByArrivingPageKey, PanelToggleHeaderReserveKey};
use rise_ui::IconUi;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::zoom_navigation_transition::{ZoomNavigationTransition, zoom_content_opacity};
use rise_ui::swiftui::open_url::{OpenURLAction, OpenURLActionResult, OpenURLKey};
use rise_widgets::overlay::blocking_loader_host::BlockingLoaderHost;
use rise_widgets::side_panel::panel_gestures;
use rise_widgets::side_panel::side_panel_preferences::SidePanelPreferences;
use rise_widgets::side_panel::side_panel_ui::{SidePanelEvent, SidePanelUi};
use rise_widgets::{BlockUi, glass_host, window_controls_band};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::push_notifications::socket_notification_source::SocketNotificationSource;
use crate::app::aside::aside_navigator::{self, AsideHeaderButtons, AsideNavigator, AsideReveal};
use crate::app::aside::aside_registry::{AsideScope, AsideSection, is_music_request_list};
use crate::app::rise_launch_splash::RiseLaunchSplashState;
use crate::riseonly_app::RiseonlyApp;
use crate::app::router::main_tabs::{MainTabs, MainTabsSideBar};
use crate::core::widgets::navigations::main_tab_bar::MainTabBarReveal;
use crate::core::widgets::wrappers::list_shell_ui::{PageColumnBackdrop, PageColumnBackdropKey};
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::app::router::navigation_fullscreen_pop::UINavigationFullscreenPopInstaller;
use crate::app::router::navigation_store::{
    NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush, SaiActiveScreenOverlay,
};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::app::router::root_route_destination::RootRouteDestination;
use crate::app::shell::command_palette::{CommandPalette, CommandPaletteEvent, CommandTarget, PaletteCommand, catalogue};
use crate::app::shell::shell_actions::{
    self, CloseWindow, GoBack, GoForward, KEY_CONTEXT, NewWindow, SelectSlot1, SelectSlot2, SelectSlot3,
    SelectSlot4, SelectSlot5, SelectSlot6, SelectSlot7, SelectSlot8, SelectSlot9, ToggleCommandPalette,
};
#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::app::shell::shell_actions::ToggleSaiDebugger;
use crate::app::shell::window_presenter::WindowPresenter;
use crate::app::shell::handover::frontmost_shell;
use crate::app::shell::root_process_lifecycle::RootProcessLifecycle;
use crate::app::shell::window_title_bar::{WindowTitleBar, draws_under_window_controls, window_frame};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::app_menu::OpenSettings;
use crate::core::desktop_platform::app_update_pill::AppUpdatePill;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase, WindowLifecycleTracking};
use crate::core::guides::guide_screen_resolver::{
    GuideNavigationBridge, GuideNavigationSnapshot, GuideScreenDestination, GuideScreenObserver,
};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiNavigationBridge;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_debug::SaiWsScreenRefreshDebug;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::maintenance::maintenance_modal::MaintenanceHost;
use crate::core::media_viewer::media_viewer_host::{MediaViewerHost, MediaViewerInlineHostView};
use crate::core::media_viewer::media_viewer_modal_host::MediaViewerModalHostBridge;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerPresentationKey, MediaViewerPresentationState, register_window_media_viewer_presentation,
};
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::push_notifications::push_notification_manager_hooks::PushNotificationManagerHooks;
use crate::core::push_notifications::desktop_link_routing::{
    ExternalUrlOutcome, is_canon_web_link, parse_external_link, resolve_external_url,
};
use crate::core::restriction::restriction_modal::RestrictionHost;
use crate::core::shared_element_transition::navigation::shared_element_navigation_delegate_proxy::SharedElementNavigationDelegateProxy;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementNavigationController, SharedElementNavigationControllerDelegate, SharedElementNavigationHost,
    SharedElementViewControllerTransitionCoordinator,
};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::{
    SharedElementStackHost, SharedElementStackHostKey,
};
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::{
    SharedElementStackDirectory, SharedElementTransitionCoordinator,
};
#[cfg(debug_assertions)]
use crate::core::ui::guide_anchor_ui::guide_anchor_inventory::GuideAnchorInventory;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_host::HoldContextMenuHost;
use rise_ui::guide_overlay_ui::guide_overlay_window::GuideOverlayWindow;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowLayer;
use crate::core::widgets::navigations::chat_launch_unread_bubble::ChatLaunchUnreadBubbleStore;
use crate::core::widgets::notifier::notifier_host::{NotifierHost, NotifierWindowPresenter};
use crate::core::widgets::overlay::achievement_earned_host::AchievementEarnedHost;
use crate::core::widgets::overlay::premium_gift_celebration_host::PremiumGiftCelebrationHost;
use crate::core::widgets::overlay::premium_promo_host::PremiumPromoHost;
use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;
use crate::core::widgets::overlay::premium_upsell_host::PremiumUpsellHost;
use crate::core::widgets::overlay::purchase_continue_host::PurchaseContinueHost;
use crate::core::widgets::overlay::subscription_success_host::SubscriptionSuccessHost;
use crate::core::widgets::sheets::people_picker_sheet::{PeoplePickerHost, PeoplePickerHostContext};
use crate::core::widgets::sheets::report_sheet::ReportSheetHost;
use crate::modules::chat::widgets::bottomsheets::chat_moderation_sheet::chat_moderation_sheet::ChatModerationSheetHost;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_members::member_rights_sheet::MemberRightsSheetPresentation;
use crate::modules::bot::pages::in_app_browser::{InAppBrowserHost, InAppBrowserStore};
use crate::core::widgets::sheets::share_sheet::ShareSheet;
use crate::modules::guides::engine::guide_runner::GuideRunner;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::components::web_login_approval::web_login_approval_sheet::WebLoginApprovalSheet;
use crate::modules::auth::components::web_login_approval::web_login_approval_store::WebLoginApprovalStore;
use crate::modules::auth::pages::blocked_account::blocked_account_page::BlockedAccountPage;
use crate::modules::auth::pages::sign::add_account_auth_flow::AddAccountAuthFlowView;
use crate::modules::auth::pages::sign::sign_in::sign_in::SignIn;
use crate::modules::auth::pages::sign::sign_up::sign_up::SignUp;
use crate::modules::auth::pages::sign::auth_step_flow::AuthStepFlowAction;
use crate::modules::auth::services::auth_entry_order::AuthEntryOrder;
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::pages::bot_mini_app_sheet::BotMiniAppPresentationHost;
use crate::modules::chat::components::chat::bubble::poll_voters_sheet::PollVotersSheetHost;
use crate::modules::chat::components::chat::input_picker::chat_gif_search_portal::ChatGifSearchPortal;
use crate::modules::chat::components::chat::input_picker::sticker_hold_overlay::StickerHoldOverlay;
use crate::modules::chat::components::chat::join_requests::join_chat_sheet::JoinChatSheetHost;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatViewLimitedPrivacyCoverOverlay;
use crate::modules::chat::components::chat::media_viewer::chat_media_viewer_gallery_bridge::ChatMediaViewerGalleryBridge;
use crate::modules::chat::components::chat::media_viewer::location_fullscreen::{
    LocationPresentationKey, LocationPresentationState,
};
use crate::modules::chat::components::chat::video_note::video_note_floating_pip::VideoNoteFloatingPipView;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::components::folders::chat_folder_edit_sheet::ChatFolderEditSheetLayer;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile::ChatProfile;
use crate::modules::chat::pages::chats::chat::forum_home::forum_topic_focus_presenter::ForumTopicFocusPresenter;
use crate::modules::chat::pages::chats::embedded_forum_transition::EmbeddedForumPresentationStore;
use crate::modules::chat::shared::chat_message_jump_presenter::ChatMessageJumpPresenter;
use crate::modules::chat::shared::chat_ws_cache_refresher_setup::ChatWsCacheRefresherSetup;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::CreateChatSheetState;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::comment::widgets::comment_photo_picker_host::CommentPhotoPickerHost;
use crate::modules::comment::widgets::comments_sheet::CommentsSheetHost;
use crate::core::ui::qr_code::rise_qr_code_modal::{RiseQRCodeModalHost, RiseQRCodePresentation};
use crate::modules::diagnostics::pages::bug_report::bug_report_sheet_host::{BugReportSheetHost, BugReportSurfaceProbe};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;
use crate::modules::gifts::components::gift_sheets_host::GiftSheetsHost;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::guides::stores::guides_interactions::guides_interactions::GuidesInteractionsStore;
use crate::modules::map::components::map_invite_sheet::MapInviteSheetHost;
use crate::modules::map::components::map_onboarding_sheet::MapOnboardingSheetHost;
use crate::modules::map::components::map_photo_card::MapPhotoPickerHost;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::music::pages::now_playing::music_listening_session_screen::MusicListeningSessionSheetLayer;
use crate::modules::music::pages::now_playing::music_player_auxiliary_sheets_host::MusicPlayerAuxiliarySheetsLayer;
use crate::modules::chat::components::chat::chat_music_player_sheet::ChatMusicPlayerSheetLayer;
use crate::core::ui::select_media_ui::SelectMediaSheetLayer;
use crate::modules::music::pages::now_playing::now_playing::{NowPlayingAside, NowPlayingHost};
use crate::modules::music::shared::utils::music::music_listening_interruption_gate::MusicListeningInterruptionAlertHost;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_global_player_bar::{
    MusicGlobalPlayerBar, MusicGlobalPlayerBarPresence, MusicGlobalPlayerListeners,
};
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicPlayerHiddenUnderFullPlayerKey;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportSheetHost;
use crate::modules::music::widgets::bottomsheets::playlists::music_track_download_sheet::MusicTrackDownloadSheet;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_invite_sheet::PlaylistInviteSheetHost;
use crate::modules::stickers::pages::pack_collaboration::pack_invite_sheet::{PackInviteSheetHost, PackInviteSheetStore};
use crate::modules::theme::components::theme_preview_sheet::theme_preview_sheet::ThemePreviewHost;
use crate::modules::theme::stores::theme_presets::theme_presets_actions::theme_presets_actions::ThemePresetsActionsStore;
use crate::modules::notify::components::announcement_modal::AnnouncementHost;
use crate::modules::notify::components::notifications_hub_sheet::NotificationsHubSheetHost;
use crate::modules::notify::components::release_notes_modal::ReleaseNotesHost;
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::post::components::collaborators::post_collaborators_presenter::PostCollaboratorsHost;
use crate::modules::user::components::profile_top::profile_highlight_sheets_host::ProfileHighlightSheetsHost;
use crate::modules::post::components::post::post_likes_sheet::PostLikesSheetHost;
use crate::modules::post::components::repost::repost_opinion_editor::RepostOpinionEditor;
use crate::modules::post::components::repost::repost_overlay::{
    RepostTransitionNamespace, RepostTransitionNamespaceKey,
};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::rtc::pages::rise_rtc_room_presentation_host::{
    RiseRTCCallWindowContentChanged, RiseRTCCallWindowMode, RiseRTCRoomPresentationHost,
};
use crate::modules::rtc::shared::rise_rtc_screen_safe_area::RiseRTCScreenTopInsetKey;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::DESKTOP_PANE_ORDER;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::share_extension::entry::share_item_loader::ShareExtensionItem;
use crate::modules::share_extension::send_handoff::share_send_handoff_host::ShareSendHandoffHost;
use crate::modules::share_extension::send_handoff::share_send_handoff_interactions::ShareSendHandoffInteractionsStore;
use crate::modules::share_extension::stores::share_targets::share_targets_interactions::ShareTargetsInteractionsStore;
use crate::modules::system_share::desktop_share_intake::{
    DesktopShareLayer, accept_files, is_shareable_link, items_for_text,
};
use crate::modules::shorts::components::shorts_more_sheet::ShortsMoreSheetHost;
use crate::modules::shorts::components::shorts_zoom_namespace::{ShortsZoomNamespaceId, ShortsZoomNamespaceKey, ShortsZoomSource};
use crate::modules::sparks::components::sparks_sheets::SparksSheetsHost;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::stickers::components::sticker_asset_composer_host::StickerAssetComposerHost;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::pages::post_stats::post_stats::PostStats;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet::StickerPackSheet;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;
use crate::modules::story::components::story_player::story_player_host::StoryPlayerHost;
use crate::modules::story::components::story_share_handoff_host::StoryShareHandoffHost;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::contact_experience_sheets::{ContactPhotoSuggestionSheet, ContactSyncOnboardingSheet};
use crate::modules::user::components::premium_badge_picker::premium_badge_picker_popover::PremiumBadgePickerPopoverHost;
use crate::modules::user::components::premium_gift::premium_gift_sheet::PremiumGiftHost;
use crate::modules::user::engine::rise_user_presentation::RiseUserPresentation;
use crate::modules::user::pages::profile::settings::moderation_settings::appeal_sheet::AppealSheetHost;
use crate::modules::user::shared::mention_resolver::{MentionTapDestination, MentionTapResolver};
use crate::modules::user::stores::subscription::subscription_interactions::subscription_interactions::SubscriptionInteractionsStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::SubscriptionServicesStore;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use crate::modules::user::stores::user_achievements::user_achievements_interactions::user_achievements_interactions::UserAchievementsInteractionsStore;
use crate::modules::onboarding::pages::onboarding::onboarding::Onboarding;

#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::core::lib::sai::modules::sai_debugger::sai_debugger_host::SaiDebuggerHost;

const DEEP_LINK_LOG_NAME: &str = "DeepLink";
const EMPTY_CHAT_COLUMN_KEY: &str = "desktop_chats_select_chat";
const RESUME_PENDING_ROUTE_DELAY: Duration = Duration::from_millis(400);
const SIDE_PANEL_CHAT_CONTEXT: &str = "chat";
const SIDE_PANEL_ASIDE_CONTEXT: &str = "aside";
const ASIDE_TOGGLE_SYMBOL: &str = "info.circle";
const ASIDE_TOGGLE_TOOLTIP_KEY: &str = "more";
const PAGE_HEADER_ROW_HEIGHT: f32 = PageHeaderMetrics::HEIGHT;
const RIFF_FRAME_ASPECT: f32 = 9.0 / 16.0;

#[derive(Clone, PartialEq, Eq, Hash)]
struct RootNavigationFlowIdentity {
    is_authenticated: bool,
    is_account_blocked: bool,
    account_id: Option<String>,
}

impl RootNavigationFlowIdentity {
    fn element_id(&self) -> ElementId {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        ElementId::from(("root-navigation-flow", hasher.finish()))
    }
}

/// closes screens of a topic, chat or playlist the account just lost access to
struct RootRevokedScreensCloser {
    nav_store: Entity<NavigationStore>,
    aside: Entity<AsideNavigator>,
    closed_removed_chats: HashSet<String>,
    known_draft_chats: HashMap<String, String>,
    known_draft_direct_chats: HashSet<String>,
    removed_topic_notice: Option<Uuid>,
    removed_from_chat_notice: Option<Uuid>,
    revoked_playlist_notice: Option<Uuid>,
}

impl RootRevokedScreensCloser {
    fn new(nav_store: Entity<NavigationStore>, aside: Entity<AsideNavigator>, cx: &mut App) -> Self {
        let chats = ChatServicesStore::shared(cx);
        let music = MusicServicesStore::shared(cx);
        let chats = chats.read(cx);
        let removed_topic_notice = chats.removed_topic_notice.as_ref().map(|notice| notice.id);
        let removed_from_chat_notice = chats.removed_from_chat_notice.as_ref().map(|notice| notice.id);
        let revoked_playlist_notice = music.read(cx).revoked_playlist_notice.as_ref().map(|notice| notice.id);
        let closed_removed_chats = chats.confirmed_removed_chat_ids.clone();
        let known_draft_chats = chats.temp_chat_id_to_real.clone();
        let known_draft_direct_chats = chats.draft_direct_chat_by_id.keys().cloned().collect();
        Self {
            nav_store,
            aside,
            closed_removed_chats,
            known_draft_chats,
            known_draft_direct_chats,
            removed_topic_notice,
            removed_from_chat_notice,
            revoked_playlist_notice,
        }
    }

    /// every notice closes its screens once
    fn body(&mut self, cx: &mut App) {
        let chats = ChatServicesStore::shared(cx);
        let music = MusicServicesStore::shared(cx);

        let topic_notice = chats.read(cx).removed_topic_notice.clone();
        let topic_notice_id = topic_notice.as_ref().map(|notice| notice.id);
        if topic_notice_id != self.removed_topic_notice {
            self.removed_topic_notice = topic_notice_id;
            if let Some(notice) = topic_notice {
                let mut closed = false;
                for topic_id in &notice.topic_ids {
                    let chat_id = notice.chat_id.clone();
                    let topic_id = topic_id.clone();
                    if self.nav_store.update(cx, |nav, cx| nav.close_topic(chat_id, topic_id, cx)) {
                        closed = true;
                    }
                }
                if closed {
                    notice.announce_once(cx);
                }
            }
        }

        let chat_notice = chats.read(cx).removed_from_chat_notice.clone();
        let chat_notice_id = chat_notice.as_ref().map(|notice| notice.id);
        if chat_notice_id != self.removed_from_chat_notice {
            self.removed_from_chat_notice = chat_notice_id;
            if let Some(notice) = chat_notice {
                self.nav_store.update(cx, |nav, cx| {
                    nav.close_chat(&notice.chat_id, cx);
                });
                self.aside.update(cx, |aside, cx| aside.close_chat(&notice.chat_id, cx));
            }
        }

        let removed_now: Vec<String> = {
            let removed = &chats.read(cx).confirmed_removed_chat_ids;
            self.closed_removed_chats.retain(|chat_id| removed.contains(chat_id));
            removed.iter().filter(|chat_id| !self.closed_removed_chats.contains(*chat_id)).cloned().collect()
        };
        for chat_id in removed_now {
            let drafts: Vec<String> = self
                .known_draft_chats
                .iter()
                .filter(|(_, real_id)| real_id.as_str() == chat_id)
                .map(|(draft_id, _)| draft_id.clone())
                .collect();
            for id in drafts.iter().chain(std::iter::once(&chat_id)) {
                self.nav_store.update(cx, |nav, cx| {
                    nav.close_chat(id, cx);
                });
                self.aside.update(cx, |aside, cx| aside.close_chat(id, cx));
            }
            self.closed_removed_chats.insert(chat_id);
        }
        let discarded_drafts: Vec<String> = {
            let services = chats.read(cx);
            let drafts = &services.draft_direct_chat_by_id;
            let discarded = self
                .known_draft_direct_chats
                .iter()
                .filter(|id| !drafts.contains_key(*id) && !services.temp_chat_id_to_real.contains_key(*id))
                .cloned()
                .collect();
            if drafts.len() != self.known_draft_direct_chats.len()
                || drafts.keys().any(|id| !self.known_draft_direct_chats.contains(id))
            {
                self.known_draft_direct_chats = drafts.keys().cloned().collect();
            }
            discarded
        };
        for id in &discarded_drafts {
            self.nav_store.update(cx, |nav, cx| {
                nav.close_chat(id, cx);
            });
            self.aside.update(cx, |aside, cx| aside.close_chat(id, cx));
        }
        let drafts_now = &chats.read(cx).temp_chat_id_to_real;
        if *drafts_now != self.known_draft_chats {
            self.known_draft_chats = drafts_now.clone();
        }

        let playlist_notice = music.read(cx).revoked_playlist_notice.clone();
        let playlist_notice_id = playlist_notice.as_ref().map(|notice| notice.id);
        if playlist_notice_id != self.revoked_playlist_notice {
            self.revoked_playlist_notice = playlist_notice_id;
            if let Some(notice) = playlist_notice {
                self.nav_store.update(cx, |nav, cx| {
                    nav.close_playlist(&notice.playlist_id, cx);
                });
            }
        }
    }
}

/// what the root compares between two turns to react to changes
#[derive(Clone, PartialEq)]
struct RootObservedState {
    is_authenticated: bool,
    is_account_blocked: bool,
    active_account_id: Option<String>,
    has_restored_persisted_session: bool,
    account_session_is_published: bool,
    path: Vec<RootRoute>,
    current_tab: MainTab,
    sai_active_screen_overlay: Option<SaiActiveScreenOverlay>,
    pending_route: Option<PushDeepLink>,
    pending_share_intent: Option<Uuid>,
    call_is_active: bool,
}

/// what the root draws of the account stores beyond the observed state
/// a change here notifies the root
#[derive(Clone, PartialEq)]
struct RootDrawnServices {
    lands_on_sign_in: bool,
    is_adding_account: bool,
    should_present_contact_sync_onboarding: bool,
}

struct RootNavigationStackController {
    nav_store: WeakEntity<NavigationStore>,
    top_host: Rc<Cell<SharedElementStackHost>>,
    delegate: RefCell<Option<Rc<dyn SharedElementNavigationControllerDelegate>>>,
    delegate_proxy: RefCell<Option<Rc<SharedElementNavigationDelegateProxy>>>,
}

impl SharedElementNavigationController for RootNavigationStackController {
    fn top_view_controller(&self, _cx: &App) -> Option<SharedElementNavigationHost> {
        Some(self.top_host.get())
    }

    fn pop_view_controller(&self, _animated: bool, cx: &mut App) {
        if let Some(nav) = self.nav_store.upgrade() {
            nav.update(cx, |nav, cx| nav.pop(cx));
        }
    }

    fn transition_coordinator(&self, _cx: &App) -> Option<Rc<dyn SharedElementViewControllerTransitionCoordinator>> {
        None
    }

    fn delegate(&self) -> Option<Rc<dyn SharedElementNavigationControllerDelegate>> {
        self.delegate.borrow().clone()
    }

    fn set_delegate(&self, delegate: Option<Rc<dyn SharedElementNavigationControllerDelegate>>) {
        *self.delegate.borrow_mut() = delegate;
    }

    fn associated_delegate_proxy(&self) -> Option<Rc<SharedElementNavigationDelegateProxy>> {
        self.delegate_proxy.borrow().clone()
    }

    fn set_associated_delegate_proxy(&self, proxy: Option<Rc<SharedElementNavigationDelegateProxy>>) {
        *self.delegate_proxy.borrow_mut() = proxy;
    }

    fn set_interactive_content_pop_enabled(&self, _enabled: bool, _cx: &mut App) {}

    fn preferred_transition_is_nil(&self, _controller: SharedElementNavigationHost, _cx: &App) -> bool {
        true
    }
}

/// sign up and sign in routes the root answers itself
/// pushed over the onboarding slides so the phone step pops back to them
enum RootAuthPage {
    SignUp(Entity<NavigationStore>),
    SignIn(Entity<NavigationStore>),
}

impl RootAuthPage {
    fn leave_action(nav_store: &Entity<NavigationStore>) -> AuthStepFlowAction {
        let nav_store = nav_store.clone();
        Rc::new(move |_: &mut Window, cx: &mut App| nav_store.update(cx, |nav, cx| nav.pop(cx)))
    }
}

impl Render for RootAuthPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self {
            Self::SignUp(nav_store) => {
                SignUp { on_leave: Some(Self::leave_action(nav_store)), ..SignUp::default() }.into_any_element()
            }
            Self::SignIn(nav_store) => {
                SignIn { on_leave: Some(Self::leave_action(nav_store)), ..SignIn::default() }.into_any_element()
            }
        }
    }
}

/// what the right panel answers for
/// a narrow window has no panel so each one keeps its phone form
/// call and roster live in the call own window
#[derive(Clone, PartialEq)]
enum RootSidePanelContext {
    StickerPack(Entity<StickerPackSheet>),
    Chat(String),
    PostStats(String),
    MusicQueue,
    Aside(Option<AsideSection>),
}

impl RootSidePanelContext {
    fn key(&self) -> &'static str {
        match self {
            Self::StickerPack(_) => "sticker_pack",
            Self::Chat(_) => SIDE_PANEL_CHAT_CONTEXT,
            Self::PostStats(_) => "post_stats",
            Self::MusicQueue => "music_queue",
            Self::Aside(section) => section.map_or(SIDE_PANEL_ASIDE_CONTEXT, AsideSection::context_key),
        }
    }

    fn draws_own_close(&self) -> bool {
        matches!(self, Self::StickerPack(_))
    }
}

struct RootSidePanelContent {
    context: Option<RootSidePanelContext>,
    nav_store: Entity<NavigationStore>,
    aside: Entity<AsideNavigator>,
    side_panel: Option<WeakEntity<SidePanelUi>>,
}

impl Render for RootSidePanelContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.context.clone() {
            Some(RootSidePanelContext::StickerPack(sheet)) => StickerPackSheet::side_panel_content(&sheet, cx),
            Some(RootSidePanelContext::Chat(chat_id)) => {
                ChatProfile::new("root-side-panel-chat-profile", Some(chat_id)).presents_in_side_panel(true).into_any_element()
            }
            Some(RootSidePanelContext::PostStats(post_id)) => {
                let side_panel = self.side_panel.clone();
                PostStats::new(post_id)
                    .on_back(move |cx| {
                        if let Some(side_panel) = &side_panel {
                            let _ = side_panel.update(cx, |side_panel, cx| side_panel.set_open(false, cx));
                        }
                    })
                    .into_any_element()
            }
            Some(RootSidePanelContext::MusicQueue) => {
                NowPlayingAside::new("root-side-panel-music-queue").into_any_element()
            }
            Some(RootSidePanelContext::Aside(_)) => self.aside.clone().into_any_element(),
            None => div().into_any_element(),
        };
        env_scope::<NavigationStoreKey>(Some(self.nav_store.clone()))
            .child(env_scope::<MusicPlayerHiddenUnderFullPlayerKey>(true).child(body))
    }
}

/// section rail as its own view so a notify in a page lays it out again only when the rail changed
/// also the list column grip being dragged, the shell owns the list width
#[derive(Clone, Copy, Debug)]
struct ListColumnDrag {
    start_x: Pixels,
    start_width: Pixels,
    rtl: bool,
}

struct ListColumnResizeGhost;

struct AsideColumnDrag;

impl Render for ListColumnResizeGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// a column of the framed window, the person can drag one onto another to swap their places
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum RootPane {
    List,
    Centre,
    Aside,
}

impl RootPane {
    const DEFAULT_ORDER: [Self; 3] = [Self::List, Self::Centre, Self::Aside];

    const fn raw_value(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Centre => "centre",
            Self::Aside => "aside",
        }
    }

    const fn group(self) -> &'static str {
        match self {
            Self::List => "root-pane-list",
            Self::Centre => "root-pane-centre",
            Self::Aside => "root-pane-aside",
        }
    }

    fn stored_order() -> [Self; 3] {
        let Some(stored) = UserDefaults::standard().string(DESKTOP_PANE_ORDER) else {
            return Self::DEFAULT_ORDER;
        };
        let parsed: Vec<Self> = stored
            .split(',')
            .filter_map(|raw| Self::DEFAULT_ORDER.into_iter().find(|pane| pane.raw_value() == raw))
            .collect();
        match parsed.as_slice() {
            [first, second, third] if first != second && second != third && first != third => [*first, *second, *third],
            _ => Self::DEFAULT_ORDER,
        }
    }

    fn store_order(order: [Self; 3]) {
        let raw: Vec<&str> = order.iter().map(|pane| pane.raw_value()).collect();
        UserDefaults::standard().set(DESKTOP_PANE_ORDER, raw.join(","));
    }

    fn position(order: &[Self; 3], pane: Self) -> usize {
        order.iter().position(|candidate| *candidate == pane).unwrap_or(0)
    }

    fn merged_order(order: [Self; 3], in_row: &[Self], shown: &[Self]) -> [Self; 3] {
        let mut merged = order;
        let mut next = shown.iter();
        for slot in &mut merged {
            if in_row.contains(slot)
                && let Some(pane) = next.next()
            {
                *slot = *pane;
            }
        }
        merged
    }
}

/// where a lifted column sits in the row, its edge distance from the row start
#[derive(Clone, Copy)]
struct PaneSlot {
    inset: Pixels,
    width: Pixels,
    gap: Pixels,
}

const PANE_LIFT_RISE: f32 = 4.0;
const PANE_GRIP_WIDTH: f32 = 64.0;

/// where a gap splitter sits, measured from the side whose columns all have a known width
#[derive(Clone, Copy)]
enum SplitterEdge {
    FromStart(Pixels),
    FromEnd(Pixels),
}

/// a column drag in flight, the column itself is in pane_press
#[derive(Clone, Copy)]
struct RootPaneDrag;

struct RootPaneDragGhost;

impl Render for RootPaneDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct RootSectionRail {
    nav_store: Entity<NavigationStore>,
    _subscriptions: Vec<Subscription>,
}

impl RootSectionRail {
    fn new(nav_store: Entity<NavigationStore>, cx: &mut Context<Self>) -> Self {
        let bubble = ChatLaunchUnreadBubbleStore::shared(cx);
        let presentation = RiseUserTabBarPresentation::shared(cx);
        let reveal = MainTabBarReveal::shared(cx);
        let subscriptions = vec![
            cx.observe(&bubble, |_, _, cx| cx.notify()),
            cx.observe(&presentation, |_, _, cx| cx.notify()),
            cx.observe(&reveal, |_, _, cx| cx.notify()),
        ];
        Self { nav_store, _subscriptions: subscriptions }
    }

    fn cached_style(theme: &rise_theme::AppTheme) -> StyleRefinement {
        StyleRefinement::default()
            .h_full()
            .flex_none()
            .w(theme.shell.rail_width + theme.shell.rail_outer_inset * 2.0)
    }
}

impl Render for RootSectionRail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App).clone();
        let corner_radius = RootView::pane_radius(&theme);
        let rail = div()
            .relative()
            .h_full()
            .child(MainTabsSideBar::new(RootView::tab_binding(&self.nav_store)).corner_radius(corner_radius));
        div()
            .h_full()
            .flex_shrink_0()
            .p(theme.shell.rail_outer_inset)
            .child(RootView::pane(rail, true, corner_radius, theme.shell.rail_outer_inset, &theme))
    }
}

/// directory of every window stack so transitions can find the page below a viewer
/// without it every exit falls back to a fade
#[derive(Default)]
struct RootStackDirectory {
    next_host_id: Cell<u64>,
    windows: RefCell<Vec<Weak<RootStackSnapshot>>>,
}

/// the hosts one window shows, tab root then pushed pages bottom first
struct RootStackSnapshot {
    root: SharedElementStackHost,
    stack: RefCell<Vec<SharedElementStackHost>>,
    parked: RefCell<Vec<SharedElementStackHost>>,
}

struct RootStackDirectoryGlobal(Rc<RootStackDirectory>);

impl Global for RootStackDirectoryGlobal {}

impl RootStackDirectory {
    fn shared(cx: &mut App) -> Rc<Self> {
        if let Some(global) = cx.try_global::<RootStackDirectoryGlobal>() {
            return global.0.clone();
        }
        let directory = Rc::new(Self::default());
        cx.set_global(RootStackDirectoryGlobal(directory.clone()));
        let installed: Rc<dyn SharedElementStackDirectory> = directory.clone();
        SharedElementTransitionCoordinator::shared(cx)
            .update(cx, |coordinator, _| coordinator.set_stack_directory(Some(installed)));
        directory
    }

    fn allocate_host(&self) -> SharedElementStackHost {
        let id = self.next_host_id.get() + 1;
        self.next_host_id.set(id);
        SharedElementStackHost { id }
    }

    fn attach_window(&self) -> Rc<RootStackSnapshot> {
        let snapshot = Rc::new(RootStackSnapshot {
            root: self.allocate_host(),
            stack: RefCell::new(Vec::new()),
            parked: RefCell::new(Vec::new()),
        });
        let mut windows = self.windows.borrow_mut();
        windows.retain(|window| window.strong_count() > 0);
        windows.push(Rc::downgrade(&snapshot));
        snapshot
    }

    fn snapshots(&self) -> Vec<Rc<RootStackSnapshot>> {
        self.windows.borrow().iter().filter_map(Weak::upgrade).collect()
    }
}

impl RootStackSnapshot {
    fn top(&self) -> SharedElementStackHost {
        self.stack.borrow().last().copied().unwrap_or(self.root)
    }
}

impl SharedElementStackDirectory for RootStackDirectory {
    fn stack_of(&self, host: SharedElementStackHost, _cx: &App) -> Option<Vec<SharedElementStackHost>> {
        for snapshot in self.snapshots() {
            let stack = snapshot.stack.borrow();
            if host == snapshot.root || stack.contains(&host) {
                return Some(std::iter::once(snapshot.root).chain(stack.iter().copied()).collect());
            }
            if snapshot.parked.borrow().contains(&host) {
                return Some(vec![host]);
            }
        }
        None
    }

    fn is_in_window(&self, host: SharedElementStackHost, _cx: &App) -> bool {
        self.snapshots().iter().any(|snapshot| snapshot.top() == host)
    }
}

struct RootStackPage {
    route: RootRoute,
    host: SharedElementStackHost,
    view: AnyView,
    backdrop: PageColumnBackdrop,
    owes_fresh: Cell<bool>,
    settled_width: Rc<Cell<Option<Pixels>>>,
}

/// how a page slot takes its width
#[derive(Clone, Copy, PartialEq, Eq)]
enum PageSlotWidth {
    Free,
    Record,
    Hold,
}

/// what the window answers wherever the pointer is, back button and a file nobody wanted
/// a pushed page slot occludes so the plate alone goes deaf there
#[derive(Clone)]
struct RootWindowInput {
    on_drop: Rc<dyn Fn(&ExternalPaths, &mut Window, &mut App)>,
    on_back: Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>,
}

impl RootWindowInput {
    fn attach<E: InteractiveElement>(&self, element: E) -> E {
        let on_drop = self.on_drop.clone();
        let on_back = self.on_back.clone();
        element
            .on_drop(move |paths: &ExternalPaths, window, cx| on_drop(paths, window, cx))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), move |event, window, cx| {
                on_back(event, window, cx)
            })
    }
}

/// hosts of the zstack that hold their own state, element hosts are built in render
struct RootHosts {
    now_playing_host: Entity<NowPlayingHost>,
    music_player_bar: Entity<MusicGlobalPlayerBar>,
    music_player_bar_presence: Entity<MusicGlobalPlayerBarPresence>,
    media_viewer_inline_host: Entity<MediaViewerInlineHostView>,
    media_viewer_modal_host_bridge: Entity<MediaViewerModalHostBridge>,
    _chat_media_viewer_gallery_bridge: Entity<ChatMediaViewerGalleryBridge>,
    music_listening_interruption_alert_host: Entity<MusicListeningInterruptionAlertHost>,
    map_photo_picker_host: Entity<MapPhotoPickerHost>,
    shorts_more_sheet_host: Entity<ShortsMoreSheetHost>,
    post_collaborators_host: Entity<PostCollaboratorsHost>,
    profile_highlight_sheets_host: Entity<ProfileHighlightSheetsHost>,
    comments_sheet_host: Entity<CommentsSheetHost>,
    post_likes_sheet_host: Entity<PostLikesSheetHost>,
    poll_voters_sheet_host: Entity<PollVotersSheetHost>,
    people_picker_host: Entity<PeoplePickerHost>,
    blocking_loader_host: Entity<BlockingLoaderHost>,
    premium_badge_picker_popover_host: Entity<PremiumBadgePickerPopoverHost>,
    notifier_host: Entity<NotifierHost>,
    restriction_host: Entity<RestrictionHost>,
    maintenance_host: Entity<MaintenanceHost>,
    subscription_success_host: Entity<SubscriptionSuccessHost>,
    premium_gift_celebration_host: Entity<PremiumGiftCelebrationHost>,
    achievement_earned_host: Entity<AchievementEarnedHost>,
    join_chat_sheet_host: Entity<JoinChatSheetHost>,
    map_invite_sheet_host: Entity<MapInviteSheetHost>,
    map_onboarding_sheet_host: Entity<MapOnboardingSheetHost>,
    playlist_invite_sheet_host: Entity<PlaylistInviteSheetHost>,
    pack_invite_sheet_host: Entity<PackInviteSheetHost>,
    sticker_pack_sheet: Entity<StickerPackSheet>,
    sticker_asset_composer_host: Entity<StickerAssetComposerHost>,
    web_login_approval_sheet: Entity<WebLoginApprovalSheet>,
    share_sheet: Entity<ShareSheet>,
    sparks_sheets_host: Entity<SparksSheetsHost>,
    gift_sheets_host: Entity<GiftSheetsHost>,
    music_track_download_sheet: Entity<MusicTrackDownloadSheet>,
    chat_moderation_sheet_host: Entity<ChatModerationSheetHost>,
    member_rights_sheet_host: Entity<MemberRightsSheetPresentation>,
    report_sheet_host: Entity<ReportSheetHost>,
    appeal_sheet_host: Entity<AppealSheetHost>,
    message_music_import_sheet_host: Entity<MessageMusicImportSheetHost>,
    sticker_hold_overlay: Entity<StickerHoldOverlay>,
    chat_gif_search_portal: Entity<ChatGifSearchPortal>,
    music_listening_session_sheet_layer: Entity<MusicListeningSessionSheetLayer>,
    music_player_auxiliary_sheets_layer: Entity<MusicPlayerAuxiliarySheetsLayer>,
    chat_music_player_sheet_layer: Entity<ChatMusicPlayerSheetLayer>,
    select_media_sheet_layer: Entity<SelectMediaSheetLayer>,
    chat_folder_edit_sheet_layer: Entity<ChatFolderEditSheetLayer>,
    story_player_host: Entity<StoryPlayerHost>,
    bot_mini_app_presentation_host: Entity<BotMiniAppPresentationHost>,
    in_app_browser_host: Entity<InAppBrowserHost>,
    share_send_handoff_host: Entity<ShareSendHandoffHost>,
    desktop_share_layer: Entity<DesktopShareLayer>,
    premium_upsell_host: Entity<PremiumUpsellHost>,
    purchase_continue_host: Entity<PurchaseContinueHost>,
    premium_gift_host: Entity<PremiumGiftHost>,
    premium_promo_host: Entity<PremiumPromoHost>,
    theme_preview_host: Entity<ThemePreviewHost>,
    rise_rtc_room_presentation_host: Entity<RiseRTCRoomPresentationHost>,
    bug_report_sheet_host: Entity<BugReportSheetHost>,
    hold_context_menu_host: Entity<HoldContextMenuHost>,
    app_update_pill: Entity<AppUpdatePill>,
    #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
    sai_debugger_host: Entity<SaiDebuggerHost>,
}

pub struct RootView {
    nav_store: Entity<NavigationStore>,
    auth_services: Entity<AuthServicesStore>,
    user_services: Entity<UserServicesStore>,
    theme_store: Entity<ThemeStore>,
    interruptions_unlocked_for_account_id: Option<String>,
    media_viewer_presentation: Entity<MediaViewerPresentationState>,
    location_presentation: Entity<LocationPresentationState>,
    repost_opinion_namespace: RepostTransitionNamespace,
    shorts_zoom_namespace: ShortsZoomNamespaceId,
    open_url_action: OpenURLAction,
    scene_phase: ScenePhase,
    window_handle: AnyWindowHandle,
    notification_permission_could_present: bool,
    observed: RootObservedState,
    revoked_screens_closer: RootRevokedScreensCloser,
    flow_identity: RootNavigationFlowIdentity,
    language_code: String,
    blocked_account_page: Option<Entity<BlockedAccountPage>>,
    onboarding: Option<Entity<Onboarding>>,
    stack: Vec<RootStackPage>,
    parked_pages: Vec<RootStackPage>,
    departing_page: Option<RootStackPage>,
    page_motion: Animated<f32>,
    page_motion_is_push: bool,
    page_motion_is_zoom: bool,
    stack_directory: Rc<RootStackDirectory>,
    stack_snapshot: Rc<RootStackSnapshot>,
    top_stack_host: Rc<Cell<SharedElementStackHost>>,
    _navigation_stack: Rc<dyn SharedElementNavigationController>,
    hosts: RootHosts,
    title_bar: Entity<WindowTitleBar>,
    section_rail: Entity<RootSectionRail>,
    columns_fresh: bool,
    header_reserve: gpui::Pixels,
    add_account_flow: Option<Entity<AddAccountAuthFlowView>>,
    motion_frame_pending: bool,
    drawn_services: RootDrawnServices,
    video_note_is_floating: bool,
    video_note_pip: Entity<VideoNoteFloatingPipView>,
    side_panel: Entity<SidePanelUi>,
    side_panel_content: Entity<RootSidePanelContent>,
    side_panel_context: Option<RootSidePanelContext>,
    aside: Entity<AsideNavigator>,
    aside_header: AnyView,
    aside_slide_over_open: bool,
    map_aside_was_open: bool,
    aside_shows_now_playing: bool,
    aside_fits: bool,
    pack_aside_fits: bool,
    sticker_pack_opened_on: Option<(MainTab, Vec<RootRoute>)>,
    music_queue_has_items: bool,
    in_app_call_key: Option<String>,
    in_app_call_nav: Option<(MainTab, Vec<RootRoute>)>,
    pane_layout: PaneLayout,
    list_width_scope: SharedString,
    list_width: Option<Pixels>,
    list_drag: Option<ListColumnDrag>,
    list_drag_ceiling: Pixels,
    pane_order: [RootPane; 3],
    pane_motion: ReorderMotion<RootPane>,
    pane_press: Option<(RootPane, Pixels)>,
    pane_strides: [(RootPane, f32); 3],
    panes_in_row: Vec<RootPane>,
    palette: Option<Entity<CommandPalette>>,
    _palette_subscription: Option<Subscription>,
    focus_handle: FocusHandle,
    _window_lifecycle: WindowLifecycleTracking,
    _subscriptions: Vec<Subscription>,
}

impl RootView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        RootProcessLifecycle::install_if_needed(cx);
        shell_actions::install_key_bindings(cx);

        let nav_store = cx.new(NavigationStore::new);
        let auth_services = AuthServicesStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let media_viewer_presentation = cx.new(|_| MediaViewerPresentationState::new());
        register_window_media_viewer_presentation(window, &media_viewer_presentation, cx);
        let location_presentation = cx.new(|_| LocationPresentationState::new());

        let stack_directory = RootStackDirectory::shared(cx);
        let stack_snapshot = stack_directory.attach_window();
        let top_stack_host = Rc::new(Cell::new(stack_snapshot.root));
        let navigation_stack: Rc<dyn SharedElementNavigationController> = Rc::new(RootNavigationStackController {
            nav_store: nav_store.downgrade(),
            top_host: top_stack_host.clone(),
            delegate: RefCell::new(None),
            delegate_proxy: RefCell::new(None),
        });
        nav_store.update(cx, |nav, _| nav.attach_navigation_window(&navigation_stack));

        let this = cx.weak_entity();
        let open_url_action = OpenURLAction::new(move |url, _window, cx| {
            this.update(cx, |view, cx| view.open_url_action(url, cx)).unwrap_or(OpenURLActionResult::SystemAction)
        });

        let hosts = Self::make_hosts(&nav_store, &media_viewer_presentation, window, cx);
        let title_bar = cx.new(|cx| {
            WindowTitleBar::new(window, cx).on_close(std::rc::Rc::new(
                |window: &mut Window, cx: &mut gpui::App| SocketNotificationSource::close_button(window, cx),
            ))
        });
        let aside = cx.new(|cx| AsideNavigator::new(&nav_store, cx));
        let section_rail = cx.new(|cx| RootSectionRail::new(nav_store.clone(), cx));
        let aside_header: AnyView = cx.new(|cx| AsideHeaderButtons::new(&aside, cx)).into();
        let side_panel_content = cx.new(|_: &mut Context<RootSidePanelContent>| RootSidePanelContent {
            context: None,
            nav_store: nav_store.clone(),
            aside: aside.clone(),
            side_panel: None,
        });
        let side_panel_view: AnyView = side_panel_content.clone().into();
        let side_panel = cx.new(|cx| {
            SidePanelUi::new(SIDE_PANEL_CHAT_CONTEXT, side_panel_view, SidePanelPreferences::standard(), cx)
        });
        side_panel_content.update(cx, |content, _| content.side_panel = Some(side_panel.downgrade()));
        let sticker_pack_store = StickerPackSheetStore::shared(cx);
        let music_player = MusicPlayerState::shared(cx);
        let music_queue_has_items = !music_player.read(cx).queue.is_empty();
        let call_host = hosts.rise_rtc_room_presentation_host.clone();

        let theme = rise_ui::theme(cx as &App);
        let pane_layout = PanePolicy::from_shell_metrics(theme.shell)
            .with_panel_metrics(theme.panel)
            .layout_for(window.viewport_size().width);

        let revoked_screens_closer = RootRevokedScreensCloser::new(nav_store.clone(), aside.clone(), cx);
        let chats = ChatServicesStore::shared(cx);
        let music = MusicServicesStore::shared(cx);
        let push_deep_link_store = PushDeepLinkStore::shared(cx);
        let achievements_interactions = UserAchievementsInteractionsStore::shared(cx);
        let user_presentation = RiseUserPresentation::shared(cx);
        let rtc_services = RTCServicesStore::shared(cx);
        let maintenance_gate = MaintenanceGate::shared(cx);
        let video_note_player = VideoNotePlayerStore::shared(cx);
        let post_presentation = RisePostContentPresentation::shared(cx);
        let qr_code_presentation = RiseQRCodePresentation::shared(cx);
        let post_services = PostServicesStore::shared(cx);

        let closing_observer = cx.weak_entity();
        let mut subscriptions = vec![
            cx.on_window_closed(move |cx, _| {
                let _ = closing_observer.update(cx, |view, cx| view.window_did_close(cx));
            }),
            cx.observe_window_activation(window, |view, window, cx| {
                if window.is_window_active() {
                    view.install_navigation_bridges(cx);
                }
            }),
            cx.observe(&auth_services, |view, _, cx| view.services_did_change(cx)),
            cx.observe(&user_services, |view, _, cx| view.services_did_change(cx)),
            cx.observe(&user_presentation, |view, _, cx| view.observed_state_did_change(cx)),
            cx.observe(&nav_store, |view, nav_store, cx| {
                if view.nav_change_moves_stack_only(&nav_store, cx) {
                    view.motion_frame_pending = true;
                }
                view.minimize_in_app_call_on_navigation(&nav_store, cx);
                cx.notify();
                view.state_did_change(cx);
            }),
            cx.subscribe(&call_host, |view, _, _: &RiseRTCCallWindowContentChanged, cx| view.in_app_call_did_change(cx)),
            cx.observe_global::<RiseRTCCallWindowMode>(|view, cx| view.in_app_call_did_change(cx)),
            cx.observe(&push_deep_link_store, |view, _, cx| view.observed_state_did_change(cx)),
            cx.observe(&achievements_interactions, |view, _, cx| view.observed_state_did_change(cx)),
            cx.observe(&rtc_services, |view, _, cx| view.observed_state_did_change(cx)),
            cx.observe_self(|view, cx| {
                if !std::mem::take(&mut view.motion_frame_pending) {
                    view.dirty_columns(cx);
                }
            }),
            cx.observe(&chats, |view, _, cx| view.revoked_screens_closer.body(cx)),
            cx.observe(&music, |view, _, cx| view.revoked_screens_closer.body(cx)),
            cx.observe(&maintenance_gate, |_, _, cx| cx.notify()),
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
            cx.observe(&video_note_player, |view, player, cx| {
                let is_floating = player.read(cx).is_floating();
                if is_floating != view.video_note_is_floating {
                    view.video_note_is_floating = is_floating;
                    cx.notify();
                }
            }),
            {
                let mut seen: Option<(bool, SharedString, Pixels, bool)> = None;
                cx.observe(&side_panel, move |_, panel, cx| {
                    let panel = panel.read(cx);
                    let now = (panel.is_open(), panel.context().clone(), panel.width(), panel.has_chosen_width());
                    if seen.as_ref() == Some(&now) {
                        return;
                    }
                    seen = Some(now);
                    GuideScreenObserver::visible_did_change(cx);
                    cx.notify();
                })
            },
            cx.observe(&qr_code_presentation, |_, _, cx| cx.notify()),
            cx.subscribe(&side_panel, |view, side_panel, event: &SidePanelEvent, cx| {
                let SidePanelEvent::Toggled { open } = event;
                if !*open && matches!(view.side_panel_context, Some(RootSidePanelContext::StickerPack(_))) {
                    StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.dismiss(cx));
                    side_panel.update(cx, |side_panel, cx| side_panel.set_open(true, cx));
                }
                if !*open {
                    view.aside_slide_over_open = false;
                }
            }),
            cx.observe(&aside, |view, aside, cx| {
                let now_playing = aside.read(cx).top_route() == Some(&RootRoute::MusicNowPlaying);
                if now_playing != view.aside_shows_now_playing {
                    view.aside_shows_now_playing = now_playing;
                    view.columns_fresh = true;
                    cx.notify();
                }
                view.side_panel_answer_did_change(cx);
            }),
            cx.subscribe(&aside, |view, _, event: &AsideReveal, cx| view.reveal_aside(event.toggles, cx)),
            cx.observe(&sticker_pack_store, |view, store, cx| {
                let is_presented = store.read(cx).is_presented();
                if !is_presented {
                    view.sticker_pack_opened_on = None;
                } else if view.sticker_pack_opened_on.is_none() {
                    let nav = view.nav_store.read(cx);
                    view.sticker_pack_opened_on = Some((nav.current_tab, nav.path.clone()));
                }
                view.side_panel_answer_did_change(cx);
            }),
            cx.observe(&post_presentation, |view, _, cx| view.side_panel_answer_did_change(cx)),
            cx.observe(&post_services, |view, _, cx| view.side_panel_answer_did_change(cx)),
            cx.observe(&music_player, |view, player, cx| {
                let has_items = !player.read(cx).queue.is_empty();
                if view.music_queue_has_items != has_items {
                    view.music_queue_has_items = has_items;
                    cx.notify();
                }
            }),
            cx.observe(&hosts.music_player_bar_presence, |_, _, cx| cx.notify()),
            cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
            cx.observe_global::<RiseAppLifecycle>(|view, cx| view.scene_phase_did_change(cx)),
            cx.observe_global::<LanguageStore>(|_, cx| cx.notify()),
        ];
        subscriptions.extend(ThemeRootBoundary::observe_window(window, cx));
        let language_code = LanguageStore::shared(cx).code().to_owned();
        let window_lifecycle = RiseAppLifecycle::track_window(window, cx);

        let observed = Self::read_observed_state(&nav_store, &auth_services, &user_services, cx);
        let flow_identity = Self::flow_identity_of(&observed);
        let drawn_services = Self::read_drawn_services(&auth_services, &user_services, cx);
        let video_note_is_floating = video_note_player.read(cx).is_floating();
        let video_note_pip = VideoNoteFloatingPipView::new(cx);

        let mut root = Self {
            nav_store,
            auth_services,
            user_services,
            theme_store,
            interruptions_unlocked_for_account_id: None,
            media_viewer_presentation,
            location_presentation,
            repost_opinion_namespace: RepostTransitionNamespace::default(),
            shorts_zoom_namespace: cx.entity_id().as_u64(),
            open_url_action,
            scene_phase: RiseAppLifecycle::scene_phase(cx),
            window_handle: window.window_handle(),
            notification_permission_could_present: false,
            observed,
            revoked_screens_closer,
            flow_identity,
            language_code,
            blocked_account_page: None,
            onboarding: None,
            stack: Vec::new(),
            parked_pages: Vec::new(),
            departing_page: None,
            page_motion: Animated::new(1.0),
            page_motion_is_push: true,
            page_motion_is_zoom: false,
            stack_directory,
            stack_snapshot,
            top_stack_host,
            _navigation_stack: navigation_stack,
            hosts,
            title_bar,
            section_rail,
            columns_fresh: true,
            header_reserve: gpui::px(0.0),
            add_account_flow: None,
            motion_frame_pending: false,
            drawn_services,
            video_note_is_floating,
            video_note_pip,
            side_panel,
            side_panel_content,
            side_panel_context: None,
            aside,
            aside_header,
            aside_slide_over_open: false,
            map_aside_was_open: false,
            aside_shows_now_playing: false,
            aside_fits: false,
            pack_aside_fits: false,
            sticker_pack_opened_on: None,
            music_queue_has_items,
            in_app_call_key: None,
            in_app_call_nav: None,
            pane_layout,
            list_width_scope: SharedString::default(),
            list_width: None,
            list_drag: None,
            list_drag_ceiling: gpui::px(0.0),
            pane_order: RootPane::stored_order(),
            pane_motion: ReorderMotion::new(),
            pane_press: None,
            pane_strides: RootPane::DEFAULT_ORDER.map(|pane| (pane, 0.0)),
            panes_in_row: Vec::new(),
            palette: None,
            _palette_subscription: None,
            focus_handle: cx.focus_handle(),
            _window_lifecycle: window_lifecycle,
            _subscriptions: subscriptions,
        };
        ScreenFocus::set_window_root(&root.focus_handle, window, cx);
        root.on_appear(cx);
        root
    }

    fn make_hosts(
        nav_store: &Entity<NavigationStore>,
        media_viewer_presentation: &Entity<MediaViewerPresentationState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> RootHosts {
        let celebration_nav = nav_store.clone();
        let join_nav = nav_store.clone();
        let map_nav = nav_store.clone();
        let playlist_nav = nav_store.clone();
        let profile_nav = nav_store.clone();
        let pack_invite_nav = nav_store.clone();
        let bot_nav = nav_store.clone();
        let viewer_for_bridge = media_viewer_presentation.clone();
        let viewer_for_gallery = media_viewer_presentation.clone();

        RootHosts {
            media_viewer_inline_host: cx.new(|_| MediaViewerInlineHostView::new(media_viewer_presentation.clone())),
            media_viewer_modal_host_bridge: cx.new(|cx| MediaViewerModalHostBridge::new(viewer_for_bridge, cx)),
            _chat_media_viewer_gallery_bridge: cx.new(|cx| ChatMediaViewerGalleryBridge::new(viewer_for_gallery, cx)),
            music_listening_interruption_alert_host: cx.new(MusicListeningInterruptionAlertHost::new),
            map_photo_picker_host: cx.new(|cx| MapPhotoPickerHost::new(window, cx)),
            shorts_more_sheet_host: cx.new(|cx| ShortsMoreSheetHost::new(window, cx)),
            post_collaborators_host: cx.new(PostCollaboratorsHost::new),
            profile_highlight_sheets_host: cx.new(ProfileHighlightSheetsHost::new),
            comments_sheet_host: cx.new(|_| CommentsSheetHost),
            post_likes_sheet_host: cx.new(|_| PostLikesSheetHost),
            poll_voters_sheet_host: cx.new(|_| PollVotersSheetHost),
            people_picker_host: cx.new(|cx| PeoplePickerHost::new(PeoplePickerHostContext::Root, window, cx)),
            blocking_loader_host: cx.new(|cx| BlockingLoaderHost::new(window, cx)),
            premium_badge_picker_popover_host: cx.new(PremiumBadgePickerPopoverHost::new),
            notifier_host: cx.new(|cx| NotifierHost::with_token(None, true, cx)),
            restriction_host: cx.new(RestrictionHost::new),
            maintenance_host: cx.new(|cx| MaintenanceHost::new(window, cx)),
            subscription_success_host: cx.new(|cx| SubscriptionSuccessHost::new(window, cx)),
            premium_gift_celebration_host: cx.new(|cx| {
                PremiumGiftCelebrationHost::new(
                    move |chat_id, _window, cx| {
                        celebration_nav.update(cx, |nav, cx| {
                            nav.switch_tab(MainTab::Chats, cx);
                            nav.push(RootRoute::Chat { chat_id: Some(chat_id) }, NavigationStoreSettlingPush::default(), cx);
                        });
                    },
                    window,
                    cx,
                )
            }),
            achievement_earned_host: cx.new(|cx| AchievementEarnedHost::new(window, cx)),
            join_chat_sheet_host: cx.new(|cx| {
                let call_nav = join_nav.clone();
                let topic_nav = join_nav.clone();
                JoinChatSheetHost::new(
                    move |chat_id, cx| Self::open_joined_chat(&join_nav, chat_id, cx),
                    move |chat_id, target, cx| {
                        Self::open_joined_chat(&call_nav, chat_id, cx);
                        RiseRTCNavigationGate::open(target, &call_nav, cx);
                    },
                    move |chat_id, topic_id, cx| Self::open_linked_topic(&topic_nav, chat_id, topic_id, cx),
                    cx,
                )
            }),
            map_invite_sheet_host: cx.new(|cx| {
                MapInviteSheetHost::new(move |cx| Self::open_world_map(&map_nav, cx), window, cx)
            }),
            map_onboarding_sheet_host: cx.new(|cx| MapOnboardingSheetHost::new(window, cx)),
            playlist_invite_sheet_host: cx.new(|cx| {
                PlaylistInviteSheetHost::new(
                    move |playlist_id, _window, cx| {
                        playlist_nav.update(cx, |nav, cx| {
                            nav.push(
                                RootRoute::MusicPlaylistDetail { collection_id: playlist_id },
                                NavigationStoreSettlingPush::default(),
                                cx,
                            );
                        });
                    },
                    move |user_id, tag, _window, cx| {
                        profile_nav.update(cx, |nav, cx| {
                            nav.push(
                                RootRoute::UserPage { user_id: Some(user_id), tag: Some(tag) },
                                NavigationStoreSettlingPush::default(),
                                cx,
                            );
                        });
                    },
                    window,
                    cx,
                )
            }),
            pack_invite_sheet_host: cx.new(|cx| {
                PackInviteSheetHost::new(
                    move |user_id, tag, _window, cx| {
                        pack_invite_nav.update(cx, |nav, cx| {
                            nav.push(
                                RootRoute::UserPage { user_id: Some(user_id), tag: Some(tag) },
                                NavigationStoreSettlingPush::default(),
                                cx,
                            );
                        });
                    },
                    window,
                    cx,
                )
            }),
            now_playing_host: cx.new(NowPlayingHost::new),
            music_player_bar: cx.new(MusicGlobalPlayerBar::new),
            music_player_bar_presence: cx.new(MusicGlobalPlayerBarPresence::new),
            sticker_pack_sheet: cx.new(StickerPackSheet::new),
            sticker_asset_composer_host: cx.new(StickerAssetComposerHost::new),
            web_login_approval_sheet: cx.new(WebLoginApprovalSheet::new),
            share_sheet: cx.new(|cx| ShareSheet::new(window, cx)),
            sparks_sheets_host: cx.new(|cx| SparksSheetsHost::new(window, cx)),
            gift_sheets_host: cx.new(|cx| GiftSheetsHost::new(window, cx)),
            music_track_download_sheet: cx.new(|cx| MusicTrackDownloadSheet::new(window, cx)),
            chat_moderation_sheet_host: cx.new(ChatModerationSheetHost::new),
            member_rights_sheet_host: MemberRightsSheetPresentation::shared(cx),
            report_sheet_host: cx.new(|cx| ReportSheetHost::new(window, cx)),
            appeal_sheet_host: cx.new(|cx| AppealSheetHost::new(window, cx)),
            message_music_import_sheet_host: cx.new(|cx| MessageMusicImportSheetHost::new(window, cx)),
            sticker_hold_overlay: cx.new(StickerHoldOverlay::new),
            chat_gif_search_portal: ChatGifSearchPortal::shared(cx),
            music_listening_session_sheet_layer: cx.new(|_| MusicListeningSessionSheetLayer),
            music_player_auxiliary_sheets_layer: cx.new(|_| MusicPlayerAuxiliarySheetsLayer),
            chat_music_player_sheet_layer: ChatMusicPlayerSheetLayer::install(cx),
            select_media_sheet_layer: SelectMediaSheetLayer::install(cx),
            chat_folder_edit_sheet_layer: cx.new(|_| ChatFolderEditSheetLayer),
            story_player_host: cx.new(StoryPlayerHost::new),
            bot_mini_app_presentation_host: cx.new(|cx| BotMiniAppPresentationHost::new(bot_nav, cx)),
            in_app_browser_host: cx.new(InAppBrowserHost::new),
            share_send_handoff_host: cx.new(ShareSendHandoffHost::new),
            desktop_share_layer: cx.new(DesktopShareLayer::new),
            premium_upsell_host: cx.new(|cx| PremiumUpsellHost::new(window, cx)),
            purchase_continue_host: cx.new(PurchaseContinueHost::new),
            premium_gift_host: cx.new(PremiumGiftHost::new),
            premium_promo_host: cx.new(|cx| PremiumPromoHost::new(window, cx)),
            theme_preview_host: cx.new(|cx| ThemePreviewHost::new(window, cx)),
            rise_rtc_room_presentation_host: cx.new(RiseRTCRoomPresentationHost::new),
            bug_report_sheet_host: cx.new(BugReportSheetHost::new),
            hold_context_menu_host: cx.new(|cx| HoldContextMenuHost::new(window, cx)),
            app_update_pill: cx.new(AppUpdatePill::new),
            #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
            sai_debugger_host: cx.new(SaiDebuggerHost::new),
        }
    }

    /// runs when the stack appears
    fn on_appear(&mut self, cx: &mut Context<Self>) {
        self.install_navigation_bridges(cx);

        let guides = GuidesInteractionsStore::shared(cx);
        guides.update(cx, |guides, cx| guides.start_watching_screens(cx));
        #[cfg(debug_assertions)]
        GuideAnchorInventory::install_screen_audit();
        ChatWsCacheRefresherSetup::install_if_needed(cx);
        let notify = NotifyInteractionsStore::shared(cx);
        notify.update(cx, |notify, cx| notify.start_notification_settings_outbox(cx));
        let active_account_id = self.observed.active_account_id.clone();
        self.theme_store.update(cx, |theme, cx| theme.activate(active_account_id, cx));
        self.bootstrap_authenticated_session_if_needed(cx);
        self.resume_shared_story_after_authentication_if_needed(cx);
        if !cx.has_global::<EngagementSessionStarted>() {
            cx.set_global(EngagementSessionStarted);
            let engagement = EngagementInteractionsStore::shared(cx);
            engagement.update(cx, |engagement, cx| engagement.start_session(cx));
        }
        let gate = MaintenanceGate::shared(cx);
        gate.update(cx, |gate, cx| gate.start(cx));
        self.latch_interruptions_if_ready(cx);

        self.path_probe_did_change(cx);
        if let Some(link) = self.observed.pending_route.clone() {
            self.schedule_pending_route(link, cx);
        }
    }

    /// bridges follow the front window, a running guide keeps its own
    /// weak refs so a closed window isnt kept alive
    fn install_navigation_bridges(&self, cx: &mut Context<Self>) {
        aside_navigator::install_frontmost(&self.aside, cx);
        let provider_nav = self.nav_store.downgrade();
        SaiNavigationBridge::install(
            Box::new(move |cx: &App| Some(provider_nav.upgrade()?.read(cx).sai_active_screen())),
            cx,
        );
        InAppLinkBrowser::install(move |url: Url, cx: &mut App| {
            InAppBrowserStore::shared(cx).update(cx, |store, cx| store.open(url, cx));
        });
        let visible_nav = self.nav_store.downgrade();
        let visible_chats = ChatServicesStore::shared(cx).downgrade();
        let visible_panel = self.side_panel.downgrade();
        let visible_panel_content = self.side_panel_content.downgrade();
        SaiNavigationBridge::install_visible(
            Box::new(move |cx: &App| {
                let mut screens = Vec::new();
                if let Some(nav) = visible_nav.upgrade() {
                    let nav = nav.read(cx);
                    screens.push(nav.sai_active_screen());
                    if nav.settings_list_beside_stack()
                        && let Some(list) = nav.settings_split_index().and_then(|index| nav.path.get(index))
                    {
                        let list = list.sai_active_screen();
                        if !screens.contains(&list) {
                            screens.push(list);
                        }
                    }
                }
                if let Some(aside) = aside_navigator::frontmost_screen(cx)
                    && !screens.contains(&aside)
                {
                    screens.push(aside);
                }
                if let Some(panel) = visible_panel.upgrade()
                    && panel.read(cx).is_open()
                    && let Some(content) = visible_panel_content.upgrade()
                    && content.read(cx).aside.read(cx).is_panel_shown()
                    && let Some(RootSidePanelContext::Chat(chat_id)) = content.read(cx).context.as_ref()
                {
                    let profile = RootRoute::ChatProfile { chat_id: Some(chat_id.clone()) }.sai_active_screen();
                    if !screens.contains(&profile) {
                        screens.push(profile);
                    }
                }
                if let Some(chats) = visible_chats.upgrade() {
                    let chats = chats.read(cx);
                    for screen in &mut screens {
                        if let Some(chat_id) = screen.params.get_mut(guide_screen_keys::CHAT_ID)
                            && let Some(real_id) = chats.temp_chat_id_to_real.get(chat_id.as_str())
                        {
                            *chat_id = real_id.clone();
                        }
                    }
                }
                screens
            }),
            cx,
        );
        SaiWsScreenRefreshDebug::log(|| "RootView: navigation bridge installed".to_owned(), cx);
        let nav_store = self.nav_store.downgrade();
        if GuideRunner::shared(cx).read(cx).running_guide_key().is_none() {
            Self::install_guide_navigation_bridge(nav_store);
        } else {
            GuideNavigationBridge::install_after_guide(move || Self::install_guide_navigation_bridge(nav_store));
        }
    }

    fn install_guide_navigation_bridge(nav_store: WeakEntity<NavigationStore>) {
        let applier_nav = nav_store.clone();
        GuideNavigationBridge::install_applier(move |destination, cx| {
            let Some(nav_store) = applier_nav.upgrade() else {
                return;
            };
            match destination {
                GuideScreenDestination::Tab(tab) => {
                    nav_store.update(cx, |nav, cx| {
                        nav.pop_to_root(cx);
                        nav.switch_tab(tab, cx);
                    });
                }
                GuideScreenDestination::Route(route) => {
                    let screen = route.sai_active_screen();
                    if nav_store.read(cx).sai_active_screen() == screen {
                        return;
                    }
                    if matches!(route, RootRoute::Settings) {
                        if SaiNavigationBridge::visible_screens(cx).contains(&screen) {
                            return;
                        }
                        nav_store.update(cx, |nav, cx| nav.open_rail_route(route, NavigationStoreSettlingPush::Wait, cx));
                        return;
                    }
                    nav_store.update(cx, |nav, cx| nav.push(route, NavigationStoreSettlingPush::Wait, cx));
                }
            }
        });

        let snapshot_nav = nav_store.clone();
        let restorer_nav = nav_store;
        GuideNavigationBridge::install_snapshotter(
            move |cx| {
                let nav_store = snapshot_nav.upgrade()?;
                let auth = AuthServicesStore::shared(cx);
                let creating_chat = CreateChatSheetState::shared(cx).read(cx).is_open();
                let auth = auth.read(cx);
                let nav = nav_store.read(cx);
                let forum_screen_id = RootRoute::ForumHome { chat_id: Some(String::new()) }.sai_active_screen().id;
                Some(GuideNavigationSnapshot {
                    account_id: auth.effective_user_id_for_wire(),
                    path: nav.path.clone(),
                    tab: nav.current_tab,
                    adding_account: auth.is_adding_account(),
                    creating_chat,
                    embedded_forum_chat_id: nav.sai_active_screen_overlay().and_then(|overlay| {
                        if overlay.screen.id == forum_screen_id {
                            overlay.screen.params.get(guide_screen_keys::CHAT_ID).cloned()
                        } else {
                            None
                        }
                    }),
                })
            },
            move |snapshot, cx| {
                let auth = AuthServicesStore::shared(cx);
                if snapshot.account_id != auth.read(cx).effective_user_id_for_wire() {
                    return;
                }
                let creating_chat = snapshot.creating_chat;
                let nav = restorer_nav.clone();
                let restore_navigation = move |cx: &mut App| {
                    let Some(nav) = nav.upgrade() else {
                        return;
                    };
                    let auth = AuthServicesStore::shared(cx);
                    if snapshot.account_id != auth.read(cx).effective_user_id_for_wire() {
                        return;
                    }
                    nav.update(cx, |nav, cx| nav.restore(snapshot.path, snapshot.tab, cx));
                    let forum_now = nav.read(cx).sai_active_screen_overlay().and_then(|overlay| {
                        overlay.screen.params.get(guide_screen_keys::CHAT_ID).cloned()
                    });
                    if forum_now != snapshot.embedded_forum_chat_id {
                        let chat_id = snapshot.embedded_forum_chat_id;
                        nav.update(cx, |nav, cx| nav.request_embedded_forum(chat_id, cx));
                    }
                    if !snapshot.adding_account && auth.read(cx).is_adding_account() {
                        auth.update(cx, |auth, cx| auth.cancel_adding_account(cx));
                    }
                };
                let create_chat = CreateChatSheetState::shared(cx);
                if !creating_chat && create_chat.read(cx).is_open() {
                    create_chat.update(cx, |sheet, cx| sheet.close_then(restore_navigation, cx));
                } else {
                    restore_navigation(cx);
                }
            },
        );
    }

    fn read_observed_state(
        nav_store: &Entity<NavigationStore>,
        auth_services: &Entity<AuthServicesStore>,
        user_services: &Entity<UserServicesStore>,
        cx: &mut App,
    ) -> RootObservedState {
        let pending_route = PushDeepLinkStore::shared(cx).read(cx).pending_route().cloned();
        let pending_share_intent = UserAchievementsInteractionsStore::shared(cx)
            .read(cx)
            .pending_share_intent()
            .map(|intent| intent.id);
        let rtc_services = RTCServicesStore::shared(cx);
        let call_is_active = rtc_services.read(cx).active_room(cx).is_some();
        let auth = auth_services.read(cx);
        let nav = nav_store.read(cx);
        RootObservedState {
            is_authenticated: auth.is_authenticated(),
            is_account_blocked: auth.is_account_blocked(),
            active_account_id: auth.active_account_id().map(str::to_owned),
            has_restored_persisted_session: auth.has_restored_persisted_session(),
            account_session_is_published: user_services.read(cx).account_session_is_published(cx),
            path: nav.path.clone(),
            current_tab: nav.current_tab,
            sai_active_screen_overlay: nav.sai_active_screen_overlay().cloned(),
            pending_route,
            pending_share_intent,
            call_is_active,
        }
    }

    fn flow_identity_of(state: &RootObservedState) -> RootNavigationFlowIdentity {
        RootNavigationFlowIdentity {
            is_authenticated: state.is_authenticated,
            is_account_blocked: state.is_account_blocked,
            account_id: state.active_account_id.clone(),
        }
    }

    /// runs every on_change of the root view in order, stack modifiers before the zstack ones
    fn observed_state_did_change(&mut self, cx: &mut Context<Self>) {
        let next = Self::read_observed_state(&self.nav_store, &self.auth_services, &self.user_services, cx);
        if next != self.observed {
            self.state_did_change(cx);
        }
    }

    fn read_drawn_services(
        auth_services: &Entity<AuthServicesStore>,
        user_services: &Entity<UserServicesStore>,
        cx: &App,
    ) -> RootDrawnServices {
        let auth = auth_services.read(cx);
        RootDrawnServices {
            lands_on_sign_in: auth.lands_on_sign_in(),
            is_adding_account: auth.is_adding_account(),
            should_present_contact_sync_onboarding: user_services.read(cx).should_present_contact_sync_onboarding(),
        }
    }

    fn services_did_change(&mut self, cx: &mut Context<Self>) {
        let drawn = Self::read_drawn_services(&self.auth_services, &self.user_services, cx);
        if drawn != self.drawn_services {
            self.drawn_services = drawn;
            cx.notify();
        }
        self.observed_state_did_change(cx);
    }

    fn state_did_change(&mut self, cx: &mut Context<Self>) {
        let next = Self::read_observed_state(&self.nav_store, &self.auth_services, &self.user_services, cx);
        if next == self.observed {
            return;
        }
        cx.notify();
        let previous = std::mem::replace(&mut self.observed, next.clone());
        if let Some((tab, path)) = &self.sticker_pack_opened_on
            && (*tab != next.current_tab || *path != next.path)
        {
            self.sticker_pack_opened_on = None;
            StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.dismiss(cx));
        }
        let owns_process_work = self.is_frontmost_shell(cx);

        if previous.account_session_is_published != next.account_session_is_published {
            self.latch_interruptions_if_ready(cx);
        }

        if previous.is_account_blocked != next.is_account_blocked {
            if next.is_account_blocked {
                self.nav_store.update(cx, |nav, cx| nav.pop_to_root(cx));
                self.dismiss_account_scoped_sheets(cx);
                if self.auth_services.read(cx).is_adding_account() {
                    self.auth_services.update(cx, |auth, cx| auth.cancel_adding_account(cx));
                }
                let comments = CommentInteractionsStore::shared(cx);
                if comments.read(cx).comments_sheet_visible {
                    comments.update(cx, |comments, cx| comments.close_comments_sheet(cx));
                }
                self.nav_store.update(cx, |nav, cx| nav.request_embedded_forum(None, cx));
            } else {
                self.bootstrap_authenticated_session_if_needed(cx);
                self.latch_interruptions_if_ready(cx);
            }
        }

        if previous.is_authenticated != next.is_authenticated {
            if next.is_authenticated {
                self.bootstrap_authenticated_session_if_needed(cx);
                self.latch_interruptions_if_ready(cx);
            } else {
                self.interruptions_unlocked_for_account_id = None;
                if owns_process_work {
                    RootProcessLifecycle::set_bootstrapped_account_id(None, cx);
                    RootProcessLifecycle::clear_deferred_call_end_refresh(cx);
                    let handoff = ShareSendHandoffInteractionsStore::shared(cx);
                    handoff.update(cx, |handoff, cx| handoff.account_did_change(cx));
                    let subscription = SubscriptionServicesStore::shared(cx);
                    subscription.update(cx, |subscription, cx| subscription.stop(cx));
                }
            }
        }

        if previous.call_is_active != next.call_is_active && owns_process_work {
            RootProcessLifecycle::note_call_activity_change(previous.call_is_active, next.call_is_active, cx);
        }

        if previous.path != next.path {
            self.nav_store.update(cx, |nav, cx| nav.validate_chat_first_frame_transition_path(cx));
            let count = next.path.len();
            let coordinator = SharedElementTransitionCoordinator::shared(cx);
            coordinator.update(cx, |coordinator, cx| coordinator.navigation_path_did_change(count, cx));
        }
        if previous.current_tab != next.current_tab {
            self.nav_store.update(cx, |nav, cx| nav.validate_chat_first_frame_transition_path(cx));
        }

        if previous.is_authenticated != next.is_authenticated {
            self.go_to_main_tabs_posts(cx);
            if next.is_authenticated {
                if owns_process_work {
                    PushNotificationManagerHooks::activate_for_authenticated_account(cx);
                    self.resume_shared_story_after_authentication_if_needed(cx);
                    let handoff = ShareSendHandoffInteractionsStore::shared(cx);
                    handoff.update(cx, |handoff, cx| handoff.resume_pending_if_possible(cx));
                }
                if let Some(link) = next.pending_route.clone() {
                    self.replay_pending_route_after_authentication(link, cx);
                }
            }
        }

        if !previous.has_restored_persisted_session && next.has_restored_persisted_session && owns_process_work {
            let account_id = next.active_account_id.clone();
            self.theme_store.update(cx, |theme, cx| theme.drop_launch_seed_if_signed_out(account_id.as_deref(), cx));
        }

        if previous.active_account_id != next.active_account_id {
            self.active_account_did_change(previous.active_account_id.clone(), next.active_account_id.clone(), cx);
        }

        if previous.path != next.path
            || previous.current_tab != next.current_tab
            || previous.sai_active_screen_overlay != next.sai_active_screen_overlay
        {
            self.path_probe_did_change(cx);
        }
        if previous.current_tab != next.current_tab {
            let bubble = ChatLaunchUnreadBubbleStore::shared(cx);
            let tab = next.current_tab;
            bubble.update(cx, |bubble, cx| bubble.note_tab_selected(tab, cx));
        }

        if previous.pending_route != next.pending_route
            && let Some(link) = next.pending_route.clone()
        {
            self.schedule_pending_route(link, cx);
        }

        if previous.pending_share_intent != next.pending_share_intent
            && next.pending_share_intent.is_some()
            && owns_process_work
        {
            self.pending_share_intent_did_arrive(cx);
        }

        self.flow_identity_did_change_if_needed(cx);
    }

    fn flow_identity_did_change_if_needed(&mut self, cx: &mut Context<Self>) {
        let identity = Self::flow_identity_of(&self.observed);
        if identity == self.flow_identity {
            return;
        }
        self.flow_identity = identity;
        self.stack.clear();
        self.parked_pages.clear();
        self.publish_stack_hosts();
        self.departing_page = None;
        self.page_motion.jump(1.0);
        self.blocked_account_page = None;
        self.onboarding = None;
        cx.notify();
    }

    fn path_probe_did_change(&self, cx: &mut Context<Self>) {
        let nav = self.nav_store.read(cx);
        BugReportSurfaceProbe::update(&nav.path, nav.current_tab);
        let screen = nav.sai_active_screen();
        GuideScreenObserver::screen_did_change(screen, cx);
    }

    fn pending_share_intent_did_arrive(&mut self, cx: &mut Context<Self>) {
        let achievements = UserAchievementsInteractionsStore::shared(cx);
        let Some(intent) = achievements.read(cx).pending_share_intent().cloned() else {
            return;
        };
        achievements.update(cx, |achievements, cx| achievements.consume_pending_share_intent(cx));
        if !self.auth_services.read(cx).is_authenticated() {
            return;
        }
        self.nav_store.update(cx, |nav, cx| nav.push(intent.route, NavigationStoreSettlingPush::default(), cx));
    }

    fn active_account_did_change(
        &mut self,
        old_value: Option<String>,
        new_value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let owns_process_work = self.is_frontmost_shell(cx);
        if owns_process_work {
            if old_value != new_value {
                RootProcessLifecycle::clear_deferred_call_end_refresh(cx);
            }
            if let Some(old_value) = old_value.as_deref() {
                self.theme_store.update(cx, |theme, cx| {
                    theme.deactivate(old_value, cx);
                });
            }
            let activated = new_value.clone();
            self.theme_store.update(cx, |theme, cx| theme.activate(activated, cx));
        }

        let Some(new_value) = new_value else {
            return;
        };
        if old_value.as_deref() == Some(new_value.as_str()) {
            return;
        }
        let Some(old_value) = old_value else {
            self.bootstrap_authenticated_session_if_needed(cx);
            return;
        };

        let guides = GuidesInteractionsStore::shared(cx);
        let return_to_own_profile =
            self.is_showing_own_profile(&old_value, cx) && !guides.read(cx).is_welcome_owed(&new_value);
        let own_profile_was_pushed = !self.nav_store.read(cx).path.is_empty();
        let profile_tab_was_open = self.nav_store.read(cx).current_tab == MainTab::Profile;
        if owns_process_work {
            let targets = ShareTargetsInteractionsStore::shared(cx);
            targets.update(cx, |targets, cx| targets.account_did_change(cx));
            let handoff = ShareSendHandoffInteractionsStore::shared(cx);
            handoff.update(cx, |handoff, cx| handoff.account_did_change(cx));
        }
        self.dismiss_account_scoped_sheets(cx);
        self.go_to_main_tabs_posts(cx);
        if return_to_own_profile {
            self.nav_store.update(cx, |nav, cx| {
                if own_profile_was_pushed && !profile_tab_was_open {
                    nav.push(RootRoute::MyProfile, NavigationStoreSettlingPush::default(), cx);
                } else {
                    nav.switch_tab(MainTab::Profile, cx);
                }
            });
        }
        self.interruptions_unlocked_for_account_id = None;
        if owns_process_work {
            RootProcessLifecycle::set_bootstrapped_account_id(None, cx);
            self.bootstrap_authenticated_session_if_needed(cx);
            let user = UserInteractionsStore::shared(cx);
            user.update(cx, |user, cx| user.refresh_my_profile(cx));
        }
    }

    fn scene_phase_did_change(&mut self, cx: &mut Context<Self>) {
        let phase = RiseAppLifecycle::scene_phase(cx);
        if phase == self.scene_phase {
            return;
        }
        self.scene_phase = phase;
        cx.notify();
        if phase != ScenePhase::Active && self.auth_services.read(cx).is_authenticated() {
            self.nav_store.update(cx, |nav, _| nav.cancel_chat_first_frame_transition(None));
        }
    }

    fn notification_permission_can_present(&self, cx: &App) -> bool {
        self.scene_phase == ScenePhase::Active
            && !RiseLaunchSplashState::is_visible()
            && self.auth_services.read(cx).has_restored_persisted_session()
    }

    fn sync_notification_permission(&mut self, cx: &mut Context<Self>) {
        let can_present = self.notification_permission_can_present(cx);
        let was_open = std::mem::replace(&mut self.notification_permission_could_present, can_present);
        if !can_present || was_open {
            return;
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |view, cx| {
                if view.notification_permission_can_present(cx) {
                    PushNotificationManagerHooks::request_permission_if_needed(cx);
                }
            });
        })
        .detach();
    }

    /// true when interruptions may show, one window at a time
    /// blocked page and maintenance hold everything back
    /// readiness is latched per account since live reads flap on reconnect
    fn interruptions_can_present(&self, cx: &App) -> bool {
        let auth = self.auth_services.read(cx);
        let unlocked = self.interruptions_unlocked_for_account_id.as_deref();
        !RiseLaunchSplashState::is_visible()
            && auth.is_authenticated()
            && !auth.is_account_blocked()
            && !MaintenanceGate::is_closed()
            && unlocked.is_some()
            && unlocked == auth.active_account_id()
            && !self.observed.call_is_active
    }

    fn dismiss_account_scoped_sheets(&mut self, cx: &mut Context<Self>) {
        self.media_viewer_presentation.update(cx, |viewer, cx| viewer.dismiss(cx));
        self.location_presentation.update(cx, |location, cx| location.dismiss(cx));
        let create_chat = CreateChatSheetState::shared(cx);
        if create_chat.read(cx).is_open() {
            create_chat.update(cx, |sheet, cx| sheet.close(cx));
        }
        let chats = ChatInteractionsStore::shared(cx);
        chats.update(cx, |chats, cx| chats.dismiss_join_link_sheet(cx));
        let gifts = GiftsInteractionsStore::shared(cx);
        gifts.update(cx, |gifts, cx| {
            gifts.close_shop(cx);
            gifts.close_wardrobe(cx);
            gifts.close_instance(cx);
            gifts.close_odds(cx);
            gifts.decline_equip_offer(cx);
            gifts.close_sparks_info(cx);
        });
        let sparks = SparksInteractionsStore::shared(cx);
        sparks.update(cx, |sparks, cx| {
            sparks.close_purchase_sheet(cx);
            sparks.close_send_sheet(cx);
            sparks.close_success_sheet(cx);
        });
    }

    fn go_to_main_tabs_posts(&mut self, cx: &mut Context<Self>) {
        self.nav_store.update(cx, |nav, cx| {
            nav.cancel_chat_first_frame_transition(None);
            nav.go_to_main_tabs_posts(cx);
        });
    }

    fn is_showing_own_profile(&self, account_id: &str, cx: &App) -> bool {
        let nav = self.nav_store.read(cx);
        let Some(route) = nav.path.last() else {
            return nav.current_tab == MainTab::Profile;
        };
        let (user_id, tag) = match route {
            RootRoute::MyProfile => return true,
            RootRoute::UserPage { user_id, tag } => (user_id, tag),
            _ => return false,
        };
        let tag = tag.as_deref().map(trimming_whitespaces_and_newlines).filter(|tag| !tag.is_empty());
        if let Some(tag) = tag {
            let auth = self.auth_services.read(cx);
            let Some(account) = auth.accounts().iter().find(|account| account.user_id == account_id) else {
                return false;
            };
            return compare_case_insensitive(&tag, &account.tag) == std::cmp::Ordering::Equal;
        }
        user_id.as_deref().map(trimming_whitespaces_and_newlines) == Some(account_id)
    }

    fn latch_interruptions_if_ready(&mut self, cx: &mut Context<Self>) {
        let auth = self.auth_services.read(cx);
        if !auth.is_authenticated() {
            return;
        }
        let Some(account_id) = auth.active_account_id().map(str::to_owned) else {
            return;
        };
        if self.interruptions_unlocked_for_account_id.as_deref() == Some(account_id.as_str())
            || !self.user_services.read(cx).account_session_is_published(cx)
        {
            return;
        }
        self.interruptions_unlocked_for_account_id = Some(account_id);
        cx.notify();
    }

    fn bootstrap_authenticated_session_if_needed(&mut self, cx: &mut Context<Self>) {
        let auth = self.auth_services.read(cx);
        if !auth.is_authenticated() || auth.is_account_blocked() {
            return;
        }
        let Some(account_id) = auth.active_account_id().map(str::to_owned) else {
            return;
        };
        if RootProcessLifecycle::bootstrapped_account_id(cx) == Some(account_id.as_str()) {
            return;
        }
        RootProcessLifecycle::set_bootstrapped_account_id(Some(account_id), cx);

        let current_tab = self.nav_store.read(cx).current_tab;
        let bubble = ChatLaunchUnreadBubbleStore::shared(cx);
        bubble.update(cx, |bubble, cx| bubble.arm_for_cold_launch(current_tab, cx));
        let user = UserInteractionsStore::shared(cx);
        user.update(cx, |user, cx| {
            user.load_theme_settings(false, cx);
            user.bootstrap_contacts_for_authenticated_account(cx);
        });
        let subscription = SubscriptionInteractionsStore::shared(cx);
        subscription.update(cx, |subscription, cx| subscription.ensure_iap_started(cx));
        user.update(cx, |user, cx| user.refresh_my_profile(cx));
        let targets = ShareTargetsInteractionsStore::shared(cx);
        targets.update(cx, |targets, cx| targets.bootstrap(cx));
        let handoff = ShareSendHandoffInteractionsStore::shared(cx);
        handoff.update(cx, |handoff, cx| handoff.bootstrap(cx));
    }

    fn handle_shared_send_url(&mut self, url: &Url, cx: &mut Context<Self>) -> bool {
        let handoff = ShareSendHandoffInteractionsStore::shared(cx);
        handoff.update(cx, |handoff, cx| handoff.handle_shared_send_url(url, cx))
    }

    fn handle_shared_story_url(&mut self, url: &Url, cx: &mut Context<Self>) -> bool {
        let interactions = StoryInteractionsStore::shared(cx);
        if !interactions.read(cx).is_shared_story_url(url) {
            return false;
        }
        if self.auth_services.read(cx).is_authenticated() {
            self.route_to_shared_story_profile(cx);
        }
        interactions.update(cx, |interactions, cx| interactions.handle_shared_story_url(url, cx))
    }

    pub(crate) fn resume_shared_story_after_authentication_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.auth_services.read(cx).is_authenticated() {
            return;
        }
        let interactions = StoryInteractionsStore::shared(cx);
        interactions.update(cx, |interactions, cx| interactions.discover_pending_shared_story_handoffs(cx));
        let service = StoryServicesStore::shared(cx);
        let service = service.read(cx);
        if service.story_share_handoff_phase.active_session_id().is_some()
            || !service.has_pending_shared_story_handoff()
        {
            return;
        }
        self.route_to_shared_story_profile(cx);
        interactions.update(cx, |interactions, cx| interactions.resume_pending_shared_story_handoff_if_possible(cx));
    }

    fn route_to_shared_story_profile(&mut self, cx: &mut Context<Self>) {
        self.nav_store.update(cx, |nav, cx| {
            nav.pop_to_root(cx);
            nav.switch_tab(MainTab::Profile, cx);
        });
    }

    /// handles a link tapped inside the app
    fn open_url_action(&mut self, url: &Url, cx: &mut Context<Self>) -> OpenURLActionResult {
        if self.handle_shared_send_url(url, cx) || self.handle_shared_story_url(url, cx) {
            return OpenURLActionResult::Handled;
        }
        let Some(link) = Self::parse_universal_link(url) else {
            return OpenURLActionResult::SystemAction;
        };
        if self.auth_services.read(cx).is_authenticated() {
            self.record_shared_link_open(&link, cx);
            self.handle_push_deep_link(link, cx);
        } else {
            let store = PushDeepLinkStore::shared(cx);
            store.update(cx, |store, cx| store.enqueue(link, cx));
        }
        OpenURLActionResult::Handled
    }

    /// handles a link from outside the app
    /// url scheme and first launch args or a second instance
    pub fn on_open_url(&mut self, url: Url, cx: &mut Context<Self>) {
        cx.defer(|cx| {
            if HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some() {
                HoldContextMenuPresenter::dismiss(cx);
            }
        });
        let outcome = resolve_external_url(&url, AppEnvironment::current(), |url| {
            self.handle_shared_send_url(url, cx) || self.handle_shared_story_url(url, cx)
        });
        tracing::warn!(target: "riseonly::X1458", url = %url.absolute_string(), ?outcome, "open url outcome");
        match outcome {
            ExternalUrlOutcome::Link(link) => {
                Sai::log(DEEP_LINK_LOG_NAME, || "universal link received", SaiLogKind::Network, cx);
                self.enqueue_external_link(link, cx);
            }
            ExternalUrlOutcome::Ignored if is_canon_web_link(&url) => {
                cx.open_url(url.absolute_string());
            }
            ExternalUrlOutcome::Ignored if is_shareable_link(&url) => {
                self.present_shared_items(items_for_text(url.absolute_string()), cx);
            }
            ExternalUrlOutcome::Shared | ExternalUrlOutcome::Ignored => {}
        }
    }

    /// shows the recipient layer over this window for items the share sheet handed over
    pub(crate) fn present_shared_items(&mut self, items: Vec<ShareExtensionItem>, cx: &mut Context<Self>) {
        self.hosts.desktop_share_layer.update(cx, |layer, cx| layer.enqueue(items, cx));
    }

    pub fn on_continue_user_activity(&mut self, url: &Url, cx: &mut Context<Self>) {
        let Some(link) = Self::parse_universal_link(url) else {
            return;
        };
        Sai::log(DEEP_LINK_LOG_NAME, || "user activity link received", SaiLogKind::Network, cx);
        self.enqueue_external_link(link, cx);
    }

    fn parse_universal_link(url: &Url) -> Option<PushDeepLink> {
        parse_external_link(url, AppEnvironment::current())
    }

    /// queues an outside link instead of pushing it
    /// on cold launch the session restore resets the stack first and only the queue consumer runs after
    fn enqueue_external_link(&mut self, link: PushDeepLink, cx: &mut Context<Self>) {
        if self.auth_services.read(cx).is_authenticated() {
            self.record_shared_link_open(&link, cx);
        }
        let store = PushDeepLinkStore::shared(cx);
        let queued = store.update(cx, |store, cx| store.enqueue(link, cx));
        tracing::warn!(target: "riseonly::X1458", queued, "external link enqueued");
    }

    /// a post opened by url counts as a link open
    /// written only on the url entries since pushes go through the same handler
    fn record_shared_link_open(&mut self, link: &PushDeepLink, cx: &mut Context<Self>) {
        match link {
            PushDeepLink::Post { post_id } | PushDeepLink::PostComment { post_id, .. } => {
                let posts = PostInteractionsStore::shared(cx);
                posts.update(cx, |posts, cx| posts.record_link_open(post_id, cx));
            }
            _ => {}
        }
    }

    fn pending_route_is_current(&self, link: &PushDeepLink, account_id: &str, account_epoch: &str, cx: &mut App) -> bool {
        let store = PushDeepLinkStore::shared(cx);
        let auth = self.auth_services.read(cx);
        let pending = store.read(cx).pending_route() == Some(link);
        let signed_in = auth.is_authenticated() && auth.active_account_id() == Some(account_id);
        let epoch = SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch));
        let frontmost = self.is_frontmost_shell(cx);
        tracing::warn!(target: "riseonly::X1458", ?link, pending, signed_in, epoch, frontmost, "pending route check");
        pending && signed_in && epoch && frontmost
    }

    /// the window the link was raised for is the one that opens it, whichever subscribed first
    pub(crate) fn is_frontmost_shell(&self, cx: &App) -> bool {
        frontmost_shell(cx).is_some_and(|shell| shell == self.window_handle)
    }

    fn window_did_close(&mut self, cx: &mut Context<Self>) {
        if self.is_frontmost_shell(cx) {
            self.install_navigation_bridges(cx);
        }
        let pending = PushDeepLinkStore::shared(cx).read(cx).pending_route().cloned();
        if let Some(link) = pending {
            self.schedule_pending_route(link, cx);
        }
    }

    /// consumer of the pending route, may already be queued on first mount
    /// runs next turn so the cold launch reset doesnt wipe the pushed screen
    fn schedule_pending_route(&mut self, link: PushDeepLink, cx: &mut Context<Self>) {
        let auth = self.auth_services.read(cx);
        tracing::warn!(target: "riseonly::X1458", ?link, signed_in = auth.is_authenticated(), epoch = SaiAccountCacheScope::active_epoch().is_some(), "pending route scheduled");
        if !auth.is_authenticated() {
            return;
        }
        let Some(account_id) = auth.active_account_id().map(str::to_owned) else {
            return;
        };
        let Some(account_epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |view, cx| {
                if !view.pending_route_is_current(&link, &account_id, &account_epoch, cx) {
                    return;
                }
                view.handle_push_deep_link(link, cx);
                let store = PushDeepLinkStore::shared(cx);
                store.update(cx, |store, cx| store.consume(cx));
            });
        })
        .detach();
    }

    fn replay_pending_route_after_authentication(&mut self, link: PushDeepLink, cx: &mut Context<Self>) {
        let auth = self.auth_services.read(cx);
        let Some(account_id) = auth.active_account_id().map(str::to_owned) else {
            return;
        };
        let Some(account_epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        let timer = cx.background_executor().timer(RESUME_PENDING_ROUTE_DELAY);
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |view, cx| {
                if !view.pending_route_is_current(&link, &account_id, &account_epoch, cx) {
                    return;
                }
                view.handle_push_deep_link(link.clone(), cx);
                let store = PushDeepLinkStore::shared(cx);
                if store.read(cx).pending_route() == Some(&link) {
                    store.update(cx, |store, cx| store.consume(cx));
                }
            });
        })
        .detach();
    }

    /// a group with topics opens as its topic list, same as the chat list does
    fn open_joined_chat(nav_store: &Entity<NavigationStore>, chat_id: String, cx: &mut App) {
        nav_store.update(cx, |nav, cx| nav.switch_tab(MainTab::Chats, cx));
        let services = ChatServicesStore::shared(cx);
        let topics_enabled = services.update(cx, |services, cx| services.topics_enabled(&chat_id, cx));
        let chat = if topics_enabled { services.read(cx).chat_info(&chat_id, cx) } else { None };
        let Some(chat) = chat else {
            nav_store.update(cx, |nav, cx| {
                let route = RootRoute::Chat { chat_id: Some(chat_id) };
                if !nav.pop_to(route.clone(), cx) {
                    nav.push(route, NavigationStoreSettlingPush::default(), cx);
                }
            });
            return;
        };
        let route = RootRoute::ForumHome { chat_id: Some(chat_id) };
        if nav_store.update(cx, |nav, cx| nav.pop_to(route.clone(), cx)) {
            return;
        }
        if nav_store.read(cx).path.is_empty() {
            let forum = EmbeddedForumPresentationStore::shared(cx);
            forum.update(cx, |forum, cx| forum.present(chat, cx));
        } else {
            nav_store.update(cx, |nav, cx| nav.push(route, NavigationStoreSettlingPush::default(), cx));
        }
    }

    fn open_linked_topic(nav_store: &Entity<NavigationStore>, chat_id: String, topic_id: String, cx: &mut App) {
        nav_store.update(cx, |nav, cx| nav.switch_tab(MainTab::Chats, cx));
        let forum_route = RootRoute::ForumHome { chat_id: Some(chat_id.clone()) };
        if !nav_store.update(cx, |nav, cx| nav.pop_to(forum_route.clone(), cx)) {
            let services = ChatServicesStore::shared(cx);
            let path_empty = nav_store.read(cx).path.is_empty();
            let topics_enabled =
                path_empty && services.update(cx, |services, cx| services.topics_enabled(&chat_id, cx));
            let chat = if topics_enabled { services.read(cx).chat_info(&chat_id, cx) } else { None };
            if let Some(chat) = chat {
                let forum = EmbeddedForumPresentationStore::shared(cx);
                forum.update(cx, |forum, cx| forum.present(chat, cx));
            } else {
                nav_store.update(cx, |nav, cx| nav.push(forum_route, NavigationStoreSettlingPush::Wait, cx));
            }
        }
        let nav = nav_store.clone();
        let route = RootRoute::ChatTopic { chat_id: Some(chat_id.clone()), topic_id: Some(topic_id.clone()) };
        ForumTopicFocusPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.enqueue(
                chat_id,
                topic_id,
                move |cx| {
                    nav.update(cx, |nav, cx| {
                        if !nav.pop_to(route.clone(), cx) {
                            nav.push(route, NavigationStoreSettlingPush::Wait, cx);
                        }
                    });
                },
                cx,
            );
        });
    }

    fn open_world_map(nav_store: &Entity<NavigationStore>, cx: &mut App) {
        let current = SaiNavigationBridge::current_screen(cx).map(|screen| screen.id);
        if current.as_deref() == Some(RootRoute::WorldMap.sai_active_screen().id.as_str()) {
            return;
        }
        nav_store.update(cx, |nav, cx| {
            nav.pop_to_root(cx);
            nav.switch_tab(MainTab::Maps, cx);
        });
    }

    fn handle_push_deep_link(&mut self, link: PushDeepLink, cx: &mut Context<Self>) {
        if self.auth_services.read(cx).is_account_blocked() {
            return;
        }
        let active_screen = self.nav_store.read(cx).sai_active_screen();
        if link.matches(Some(&active_screen)) {
            tracing::warn!(target: "riseonly::X1458", ?link, "deep link matches the open screen");
            return;
        }
        let comments = CommentInteractionsStore::shared(cx);
        tracing::warn!(target: "riseonly::X1458", ?link, sheet = comments.read(cx).comments_sheet_visible, "deep link routed");
        if !comments.read(cx).comments_sheet_visible {
            self.route_push_deep_link(link, cx);
            return;
        }
        let Some(account_id) = self.auth_services.read(cx).active_account_id().map(str::to_owned) else {
            return;
        };
        let Some(account_epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        comments.update(cx, |comments, cx| comments.close_comments_sheet(cx));
        let delay = Duration::from_secs_f64(BottomSheetMetrics::DISMISS_DURATION + 0.08);
        let timer = cx.background_executor().timer(delay);
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |view, cx| {
                let auth = view.auth_services.read(cx);
                let still_current = auth.is_authenticated()
                    && auth.active_account_id() == Some(account_id.as_str())
                    && SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch))
                    && !auth.is_account_blocked();
                let active_screen = view.nav_store.read(cx).sai_active_screen();
                if still_current && !link.matches(Some(&active_screen)) {
                    view.route_push_deep_link(link, cx);
                }
            });
        })
        .detach();
    }

    fn switch_tab_and_push(&self, tab: MainTab, route: RootRoute, cx: &mut App) {
        self.nav_store.update(cx, |nav, cx| {
            nav.switch_tab(tab, cx);
            nav.push(route, NavigationStoreSettlingPush::Wait, cx);
        });
    }

    fn push_waiting(&self, route: RootRoute, cx: &mut App) {
        self.nav_store.update(cx, |nav, cx| nav.push(route, NavigationStoreSettlingPush::Wait, cx));
    }

    fn switch_tab_and_pop_to_or_push(&self, tab: MainTab, route: RootRoute, cx: &mut App) {
        self.nav_store.update(cx, |nav, cx| {
            nav.switch_tab(tab, cx);
            if !nav.pop_to(route.clone(), cx) {
                nav.push(route, NavigationStoreSettlingPush::Wait, cx);
            }
        });
    }

    fn route_push_deep_link(&mut self, link: PushDeepLink, cx: &mut Context<Self>) {
        let player = MusicPlayerState::shared(cx);
        player.update(cx, |player, cx| {
            if player.show_queue_sheet {
                player.show_queue_sheet = false;
                cx.notify();
            }
            if player.show_now_playing() {
                player.set_show_now_playing(false, cx);
            }
        });
        ChatMusicPlayerSheetLayer::dismiss(cx);
        if PostServicesStore::shared(cx).read(cx).post_likes_sheet_visible {
            PostInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.close_post_likes(cx));
        }
        match link {
            PushDeepLink::Chat { chat_id } => {
                let topics_enabled = ChatServicesStore::shared(cx)
                    .update(cx, |services, cx| services.topics_enabled(&chat_id, cx));
                if topics_enabled {
                    Self::open_joined_chat(&self.nav_store, chat_id, cx);
                } else {
                    self.switch_tab_and_pop_to_or_push(MainTab::Chats, RootRoute::Chat { chat_id: Some(chat_id) }, cx);
                }
            }
            PushDeepLink::ChatTopic { chat_id, topic_id } => {
                let route = RootRoute::ChatTopic { chat_id: Some(chat_id), topic_id: Some(topic_id) };
                self.switch_tab_and_pop_to_or_push(MainTab::Chats, route, cx);
            }
            PushDeepLink::ChatMessage { chat_id, message_id, topic_id } => {
                self.nav_store.update(cx, |nav, cx| nav.switch_tab(MainTab::Chats, cx));
                let presenter = ChatMessageJumpPresenter::shared(cx);
                presenter.update(cx, |presenter, cx| {
                    presenter.enqueue(&chat_id, None, topic_id.as_deref(), &message_id, cx);
                });
                let is_open = match self.nav_store.read(cx).path.last() {
                    Some(
                        RootRoute::Chat { chat_id: open }
                        | RootRoute::ChatTopic { chat_id: open, .. }
                        | RootRoute::ForumAll { chat_id: open },
                    ) => open.as_deref() == Some(chat_id.as_str()),
                    _ => false,
                };
                if !is_open {
                    self.push_waiting(RootRoute::Chat { chat_id: Some(chat_id) }, cx);
                }
            }
            PushDeepLink::BotMiniApp { tag, slug, start_param } => {
                self.push_waiting(RootRoute::BotMiniAppLink { tag, slug, start_param }, cx);
            }
            PushDeepLink::BotChat { tag, start_param } => {
                self.push_waiting(RootRoute::BotStartLink { tag, start_param }, cx);
            }
            PushDeepLink::Post { post_id } => {
                let route = RootRoute::PostDetail { post_id: Some(post_id), origin: None };
                self.nav_store.update(cx, |nav, cx| {
                    nav.switch_tab_for_link(MainTab::Posts, cx);
                    nav.push_in_centre(route, NavigationStoreSettlingPush::Wait, cx);
                    tracing::warn!(target: "riseonly::X1458", tab = ?nav.current_tab, path = ?nav.path, "post link pushed");
                });
            }
            PushDeepLink::PostComment { post_id, comment_id } => {
                let comments = CommentInteractionsStore::shared(cx);
                comments.update(cx, |comments, cx| {
                    comments.arm_comment_focus(&post_id, comment_id.as_deref(), false, cx);
                });
                let route = RootRoute::PostDetail { post_id: Some(post_id), origin: None };
                self.nav_store.update(cx, |nav, cx| {
                    nav.switch_tab_for_link(MainTab::Posts, cx);
                    nav.push_in_centre(route, NavigationStoreSettlingPush::Wait, cx);
                });
            }
            PushDeepLink::Profile { user_id, tag } => {
                let route = RootRoute::UserPage { user_id, tag };
                self.nav_store.update(cx, |nav, cx| {
                    nav.switch_tab_for_link(MainTab::Posts, cx);
                    nav.push_in_centre(route, NavigationStoreSettlingPush::Wait, cx);
                });
            }
            PushDeepLink::PublicTag { tag } => {
                let nav_store = self.nav_store.clone();
                let resolver = MentionTapResolver::shared(cx);
                resolver.update(cx, |resolver, cx| {
                    resolver.resolve(
                        &tag,
                        move |destination, cx| {
                            nav_store.update(cx, |nav, cx| match destination {
                                MentionTapDestination::User { tag } => {
                                    nav.switch_tab(MainTab::Posts, cx);
                                    let route = RootRoute::UserPage { user_id: None, tag: Some(tag) };
                                    nav.push_in_centre(route, NavigationStoreSettlingPush::Wait, cx);
                                    tracing::warn!(target: "riseonly::X1458", tab = ?nav.current_tab, path = ?nav.path, "profile link pushed");
                                }
                                MentionTapDestination::Chat { id } => {
                                    nav.switch_tab(MainTab::Chats, cx);
                                    nav.push(RootRoute::Chat { chat_id: Some(id) }, NavigationStoreSettlingPush::Wait, cx);
                                }
                            });
                        },
                        cx,
                    );
                });
            }
            PushDeepLink::ContactJoinInbox => {
                let actions = UserActionsStore::shared(cx);
                actions.update(cx, |actions, cx| actions.list_contact_join_events_action(None, 0, cx));
                self.nav_store.update(cx, |nav, cx| {
                    nav.pop_to_root(cx);
                    nav.switch_tab(MainTab::Chats, cx);
                });
            }
            PushDeepLink::ContactPhotoSuggestions => {
                let actions = UserActionsStore::shared(cx);
                actions.update(cx, |actions, cx| actions.list_contact_photo_suggestions_action(None, 0, cx));
            }
            PushDeepLink::Story { story_id } => {
                if let Ok(id) = trimming_whitespaces_and_newlines(&story_id).parse::<i64>() {
                    let stories = StoryInteractionsStore::shared(cx);
                    stories.update(cx, |stories, cx| stories.open_story(id, cx));
                }
            }
            PushDeepLink::Highlight { highlight_id } => {
                if let Ok(id) = trimming_whitespaces_and_newlines(&highlight_id).parse::<i64>() {
                    let nav_store = self.nav_store.clone();
                    let stories = StoryInteractionsStore::shared(cx);
                    stories.update(cx, |stories, cx| {
                        stories.open_highlight_link(
                            id,
                            Rc::new(move |owner_id: String, cx: &mut App| {
                                nav_store.update(cx, |nav, cx| {
                                    nav.switch_tab(MainTab::Posts, cx);
                                    let route = RootRoute::UserPage { user_id: Some(owner_id), tag: None };
                                    nav.push_in_centre(route, NavigationStoreSettlingPush::Wait, cx);
                                });
                            }),
                            cx,
                        );
                    });
                }
            }
            PushDeepLink::Music => self.push_waiting(RootRoute::MusicTab, cx),
            PushDeepLink::MusicTrack { track_id } => {
                self.push_waiting(RootRoute::MusicTrackLink { track_id }, cx);
            }
            PushDeepLink::PlaylistTrack { playlist_id, track_id } => {
                self.push_waiting(RootRoute::MusicPlaylistTrackLink { playlist_id, track_id }, cx);
            }
            PushDeepLink::MusicReleaseTrack { track_id } => {
                self.push_waiting(RootRoute::AudioPage { track_id, source_selection: None }, cx);
            }
            PushDeepLink::MusicAlbum { album_id } => {
                self.push_waiting(RootRoute::MusicAlbumDetail { album_id }, cx);
            }
            PushDeepLink::JoinChat { token } => {
                let chats = ChatInteractionsStore::shared(cx);
                chats.update(cx, |chats, cx| chats.open_invite_link(&token, cx));
            }
            PushDeepLink::PlaylistInvite { token } => {
                let music = MusicInteractionsStore::shared(cx);
                music.update(cx, |music, cx| music.open_playlist_invite_link(&token, cx));
            }
            PushDeepLink::Playlist { playlist_id } => {
                self.push_waiting(RootRoute::MusicPlaylistLink { playlist_id }, cx);
            }
            PushDeepLink::PlaylistJoinRequests { playlist_id } => {
                self.push_waiting(RootRoute::MusicPlaylistJoinRequests { playlist_id }, cx);
            }
            PushDeepLink::PlaylistSession { playlist_id } => {
                self.push_waiting(RootRoute::MusicPlaylistDetail { collection_id: playlist_id }, cx);
            }
            PushDeepLink::StickerPackJoinRequests { pack_id } => {
                self.push_waiting(RootRoute::StickerPackJoinRequests { pack_id }, cx);
            }
            PushDeepLink::StickerPackDetail { pack_id, pack_kind } => {
                let kind = pack_kind
                    .and_then(|raw| PackKind::from_wire(&raw.to_lowercase()))
                    .unwrap_or(PackKind::Sticker);
                let sheet = StickerPackSheetStore::shared(cx);
                sheet.update(cx, |sheet, cx| sheet.present(&pack_id, kind, cx));
            }
            PushDeepLink::AddStickerPack { link } => {
                let sheet = StickerPackSheetStore::shared(cx);
                sheet.update(cx, |sheet, cx| sheet.present_link(&link, cx));
            }
            PushDeepLink::StickerPackInvite { token } => {
                let invite = PackInviteSheetStore::shared(cx);
                invite.update(cx, |invite, cx| invite.present(&token, cx));
            }
            PushDeepLink::AddTheme { theme_id } => {
                let nav_store = self.nav_store.clone();
                let actions = ThemePresetsActionsStore::shared(cx);
                actions.update(cx, |actions, cx| {
                    actions.open_link_action(
                        theme_id,
                        move |cx: &mut App| {
                            nav_store.update(cx, |nav, cx| nav.push(RootRoute::ThemesList, NavigationStoreSettlingPush::Wait, cx));
                        },
                        cx,
                    )
                });
            }
            PushDeepLink::WebLogin { login_token } => {
                let approval = WebLoginApprovalStore::shared(cx);
                approval.update(cx, |approval, cx| approval.present(&login_token, cx));
            }
            PushDeepLink::ManagedLogin { code } => {
                let auth = AuthInteractionsStore::shared(cx);
                auth.update(cx, |auth, cx| auth.enter_managed_account(&code, cx));
            }
            PushDeepLink::PremiumPromo { code } => {
                let promo = PremiumPromoStore::shared(cx);
                promo.update(cx, |promo, cx| promo.present(code, cx));
            }
            PushDeepLink::MapInvite { token } => {
                let map = MapInteractionsStore::shared(cx);
                map.update(cx, |map, cx| map.present_invite(&token, cx));
            }
            PushDeepLink::Map { user_id } => {
                Self::open_world_map(&self.nav_store, cx);
                if let Some(user_id) = user_id.filter(|user_id| !user_id.is_empty()) {
                    let map = MapInteractionsStore::shared(cx);
                    map.update(cx, |map, cx| map.focus_friend(&user_id, cx));
                }
            }
            PushDeepLink::Announcement { announcement_id } => {
                if let Ok(id) = trimming_whitespaces_and_newlines(&announcement_id).parse::<i64>() {
                    let notify = NotifyInteractionsStore::shared(cx);
                    notify.update(cx, |notify, cx| notify.open_announcement(id, cx));
                }
            }
            PushDeepLink::ModerationReport { .. } => {
                let moderation = ModerationInteractionsStore::shared(cx);
                moderation.update(cx, |moderation, cx| moderation.refresh_my_reports(cx));
                self.switch_tab_and_push(MainTab::Profile, RootRoute::MyReports, cx);
            }
            PushDeepLink::ModerationSanction { .. } | PushDeepLink::AppealVerdict { .. } => {
                let is_appeal_verdict = matches!(link, PushDeepLink::AppealVerdict { .. });
                let moderation = ModerationInteractionsStore::shared(cx);
                moderation.update(cx, |moderation, cx| {
                    moderation.refresh_blocks(cx);
                    moderation.refresh_my_appeals(cx);
                    if is_appeal_verdict {
                        moderation.open_my_blocks_on_appeals();
                    }
                });
                self.switch_tab_and_push(MainTab::Profile, RootRoute::MyBlocks, cx);
            }
            PushDeepLink::StudioTrack { track_id } => {
                let music = MusicInteractionsStore::shared(cx);
                music.update(cx, |music, cx| music.refresh_my_tracks(cx));
                self.nav_store.update(cx, |nav, cx| {
                    nav.switch_tab(MainTab::Profile, cx);
                    nav.push_in_centre(RootRoute::StudioTrackDetail { track_id }, NavigationStoreSettlingPush::Wait, cx);
                });
            }
            PushDeepLink::MyAlbumRequests => {
                self.switch_tab_and_push(MainTab::Profile, RootRoute::MyMusicAlbumRequests, cx);
            }
            PushDeepLink::MyArtistRequest => {
                self.switch_tab_and_push(MainTab::Profile, RootRoute::MyMusicArtistRequest, cx);
            }
            PushDeepLink::BugReport { .. } | PushDeepLink::Feedback { .. } => {
                let tab = if matches!(link, PushDeepLink::Feedback { .. }) { 1 } else { 0 };
                let reports = BugReportInteractionsStore::shared(cx);
                reports.update(cx, |reports, cx| {
                    reports.select_my_reports_tab(tab, cx);
                    reports.refresh_my_reports(cx);
                });
                self.switch_tab_and_push(MainTab::Profile, RootRoute::MyBugReports, cx);
            }
            PushDeepLink::MyReport { report_id } => {
                self.switch_tab_and_pop_to_or_push(MainTab::Profile, RootRoute::MyBugReports, cx);
                BugReportInteractionsStore::shared(cx)
                    .update(cx, |reports, cx| reports.open_linked_report(&report_id, cx));
            }
            PushDeepLink::SparksWallet => {
                self.switch_tab_and_push(MainTab::Profile, RootRoute::SparksWallet, cx);
            }
            PushDeepLink::Achievement { achievement_id } => {
                UserAchievementsInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.open_earned_achievement(achievement_id, cx));
                self.switch_tab_and_pop_to_or_push(MainTab::Profile, RootRoute::Achievements { user_id: None }, cx);
            }
            PushDeepLink::AdminConsole { url } => {
                rise_platform::open_url::open_url(&url, cx);
            }
        }
    }

    fn current_tab_binding(&self) -> Binding<MainTab> {
        Self::tab_binding(&self.nav_store)
    }

    fn tab_binding(nav_store: &Entity<NavigationStore>) -> Binding<MainTab> {
        Binding::entity(
            nav_store.clone(),
            |nav: &NavigationStore| nav.current_tab,
            |nav: &mut NavigationStore, tab, _cx| nav.current_tab = tab,
        )
    }

    fn shows_section_bar(&self, cx: &App) -> bool {
        let auth = self.auth_services.read(cx);
        auth.is_authenticated() && !auth.is_account_blocked()
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        shell_actions::go_back(window, cx);
    }

    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        shell_actions::go_forward(window, cx);
    }

    fn handle_escape_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        const ESCAPE_KEY: &str = "escape";
        let keystroke = &event.keystroke;
        if keystroke.key != ESCAPE_KEY || keystroke.modifiers.modified() {
            return;
        }
        cx.stop_propagation();
        if self.palette.is_some() {
            self.close_command_palette(window, cx);
            return;
        }
        if self.auth_services.read(cx).is_adding_account() {
            self.auth_services.update(cx, |auth, cx| auth.cancel_adding_account(cx));
            return;
        }
        if MediaViewerHost::redirect_orphan_escape(keystroke, false, window, cx) {
            return;
        }
        if let Some(sheet) = ScreenFocus::top_sheet(window, cx)
            && !sheet.contains_focused(window, cx)
        {
            window.focus(&sheet, cx);
            let escape = keystroke.clone();
            window.defer(cx, move |window, cx| {
                window.dispatch_keystroke(escape, cx);
            });
            return;
        }
        if self.aside_slide_over_open {
            self.aside_slide_over_open = false;
            cx.notify();
            return;
        }
        if matches!(self.side_panel_context, Some(RootSidePanelContext::Aside(_))) && self.aside.read(cx).can_step_back(cx) {
            tracing::info!(target: "riseonly::X1558", "escape steps the panel back");
            self.aside.update(cx, |aside, cx| aside.back(cx));
            return;
        }
        shell_actions::go_back(window, cx);
    }

    fn capture_escape_for_viewer(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        const ESCAPE_KEY: &str = "escape";
        let keystroke = &event.keystroke;
        if keystroke.key != ESCAPE_KEY || keystroke.modifiers.modified() || self.palette.is_some() {
            return;
        }
        if MediaViewerHost::redirect_orphan_escape(keystroke, true, window, cx) {
            cx.stop_propagation();
        }
    }

    fn handle_orphan_viewer_arrow(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            return;
        }
        if MediaViewerHost::redirect_orphan_arrow(&event.keystroke, window, cx) {
            cx.stop_propagation();
        }
    }

    fn handle_orphan_refresh_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !is_page_refresh_keystroke(&event.keystroke) {
            return;
        }
        let focused = window.focused(cx);
        let Some(screen) = ScreenFocus::last_screen(window, cx).filter(|screen| *screen != self.focus_handle) else {
            tracing::warn!(target: "riseonly::X778", focused = ?focused, "refresh key kept at root, no last screen");
            return;
        };
        let orphaned = ScreenFocus::is_idle(window, cx)
            || focused.as_ref().is_some_and(|focused| {
                *focused == self.focus_handle
                    || !focused.is_in_rendered_frame(window)
                    || (!ScreenFocus::text_field_is_focused(window, cx) && !screen.contains_focused(window, cx))
            });
        if !orphaned || ScreenFocus::top_sheet(window, cx).is_some() {
            tracing::warn!(target: "riseonly::X778", focused = ?focused, screen = ?screen, "refresh key kept at root, orphaned {orphaned}");
            return;
        }
        cx.stop_propagation();
        window.focus(&screen, cx);
        let keystroke = event.keystroke.clone();
        window.defer(cx, move |window, cx| {
            window.dispatch_keystroke(keystroke, cx);
        });
    }

    fn handle_orphan_typing_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let pastes = modifiers.secondary() && !modifiers.alt && keystroke.key == "v";
        let types = !(modifiers.control || modifiers.platform || modifiers.function)
            && keystroke
                .key_char
                .as_ref()
                .is_some_and(|typed| !typed.is_empty() && !typed.chars().any(char::is_control));
        if !pastes && !types {
            return;
        }
        let focused = window.focused(cx);
        let orphaned = ScreenFocus::is_idle(window, cx)
            || focused
                .as_ref()
                .is_some_and(|focused| *focused == self.focus_handle || !focused.is_in_rendered_frame(window));
        if !orphaned || ScreenFocus::top_sheet(window, cx).is_some() {
            return;
        }
        let Some(screen) = ScreenFocus::last_screen(window, cx).filter(|screen| *screen != self.focus_handle) else {
            return;
        };
        cx.stop_propagation();
        window.focus(&screen, cx);
        let keystroke = keystroke.clone();
        window.defer(cx, move |window, cx| {
            window.dispatch_keystroke(keystroke, cx);
        });
    }

    fn select_slot(&mut self, slot: u8, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shows_section_bar(cx) {
            return;
        }
        let current_tab = self.current_tab_binding();
        shell_actions::select_slot(slot, &current_tab, &self.nav_store, window, cx);
    }

    #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
    fn toggle_sai_debugger(&mut self, _: &ToggleSaiDebugger, _window: &mut Window, cx: &mut Context<Self>) {
        self.hosts.sai_debugger_host.update(cx, |host, cx| host.toggle_from_shortcut(cx));
    }

    fn toggle_command_palette(&mut self, _: &ToggleCommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_command_palette(window, cx);
            return;
        }
        if !self.shows_section_bar(cx) {
            return;
        }
        let commands = catalogue(cx);
        let palette = cx.new(|cx| CommandPalette::new(commands, cx));
        let query_focus = palette.read(cx).query_focus_handle(cx);
        window.focus(&query_focus, cx);
        self._palette_subscription = Some(cx.subscribe_in(&palette, window, |view, _, event: &CommandPaletteEvent, window, cx| match event {
            CommandPaletteEvent::Dismissed => view.close_command_palette(window, cx),
            CommandPaletteEvent::Activated(command) => {
                let command = command.clone();
                view.close_command_palette(window, cx);
                view.run_command(command, window, cx);
            }
        }));
        self.palette = Some(palette);
        cx.notify();
    }

    fn close_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self._palette_subscription = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn run_command(&mut self, command: PaletteCommand, window: &mut Window, cx: &mut Context<Self>) {
        match command.target {
            CommandTarget::Section(tab) => {
                let current_tab = self.current_tab_binding();
                shell_actions::select_main_tab(tab, &current_tab, &self.nav_store, window, cx);
            }
            CommandTarget::Route(route) => {
                self.nav_store.update(cx, |nav, cx| nav.open_rail_route(route, NavigationStoreSettlingPush::default(), cx));
            }
        }
    }

    fn open_settings(&mut self, _: &OpenSettings, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.shows_section_bar(cx) {
            return;
        }
        self.nav_store.update(cx, |nav, cx| nav.open_rail_route(RootRoute::Settings, NavigationStoreSettlingPush::default(), cx));
    }

    fn new_window(&mut self, _: &NewWindow, _window: &mut Window, cx: &mut Context<Self>) {
        WindowPresenter::open_shell(cx);
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    fn destination_view(&self, route: &RootRoute, cx: &mut Context<Self>) -> AnyView {
        match route {
            RootRoute::SignUp => {
                let nav_store = self.nav_store.clone();
                cx.new(|_| RootAuthPage::SignUp(nav_store)).into()
            }
            RootRoute::SignIn => {
                let nav_store = self.nav_store.clone();
                cx.new(|_| RootAuthPage::SignIn(nav_store)).into()
            }
            route => {
                if let Some(view) = aside_navigator::take_handed_page(route, cx) {
                    return view;
                }
                let route = route.clone();
                cx.new(|cx| RootRouteDestination::new(route, cx)).into()
            }
        }
    }

    fn language_did_change_if_needed(&mut self, cx: &mut Context<Self>) {
        let code = LanguageStore::shared(cx).code();
        if code == self.language_code {
            return;
        }
        self.language_code = code.to_owned();
        self.stack.clear();
        self.parked_pages.clear();
        self.publish_stack_hosts();
        self.departing_page = None;
        self.page_motion.jump(1.0);
    }

    fn publish_stack_hosts(&self) {
        *self.stack_snapshot.stack.borrow_mut() = self.stack.iter().map(|page| page.host).collect();
        *self.stack_snapshot.parked.borrow_mut() = self.parked_pages.iter().map(|page| page.host).collect();
    }

    fn sync_navigation_stack(&mut self, cx: &mut Context<Self>) {
        self.flow_identity_did_change_if_needed(cx);
        self.language_did_change_if_needed(cx);

        let path = self.nav_store.read(cx).path.clone();
        let old_len = self.stack.len();
        let shared = self.stack.iter().zip(path.iter()).take_while(|(page, route)| page.route == **route).count();
        if shared == old_len && path.len() == old_len {
            return;
        }

        let left: Vec<RootStackPage> = self.stack.drain(shared..).collect();
        self.parked_pages.extend(left);
        for route in path.into_iter().skip(shared) {
            if let Some(index) = self.parked_pages.iter().position(|page| page.route == route) {
                let page = self.parked_pages.remove(index);
                self.stack.push(page);
                continue;
            }
            let view = self.destination_view(&route, cx);
            let host = self.stack_directory.allocate_host();
            self.stack.push(RootStackPage {
                route,
                host,
                view,
                backdrop: PageColumnBackdrop::default(),
                owes_fresh: Cell::new(false),
                settled_width: Rc::new(Cell::new(None)),
            });
        }
        let parked_routes = self.nav_store.read(cx).parked_tab_routes();
        self.parked_pages.retain(|page| parked_routes.contains(&page.route));
        self.top_stack_host.set(self.stack.last().map_or(self.stack_snapshot.root, |page| page.host));
        self.publish_stack_hosts();

        self.departing_page = None;
        self.page_motion.jump(1.0);
    }

    fn window_input(cx: &mut Context<Self>) -> RootWindowInput {
        RootWindowInput {
            on_drop: Rc::new(cx.listener(|view, paths: &ExternalPaths, _, cx| {
                if let Some(items) = accept_files(paths.paths(), cx) {
                    view.present_shared_items(items, cx);
                }
            })),
            on_back: Rc::new(cx.listener(|_, _: &MouseDownEvent, window, cx| {
                shell_actions::go_back(window, cx);
            })),
        }
    }

    /// content of the navigation stack root
    fn root_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let auth = self.auth_services.read(cx);
        let (is_authenticated, is_blocked) = (auth.is_authenticated(), auth.is_account_blocked());
        let lands_on_sign_in = auth.lands_on_sign_in();
        let has_restored = auth.has_restored_persisted_session();

        if is_authenticated && is_blocked {
            let page = match &self.blocked_account_page {
                Some(page) => page.clone(),
                None => {
                    let page = cx.new(|cx| BlockedAccountPage::new(window, cx));
                    self.blocked_account_page = Some(page.clone());
                    page
                }
            };
            return page.into_any_element();
        }
        if is_authenticated {
            return env_scope::<SharedElementStackHostKey>(Some(self.stack_snapshot.root))
                .child(MainTabs::new(self.current_tab_binding()))
                .into_any_element();
        }
        if lands_on_sign_in {
            return SignIn::default().into_any_element();
        }
        if has_restored {
            let page = match &self.onboarding {
                Some(page) => page.clone(),
                None => {
                    let nav_store = self.nav_store.clone();
                    let page = cx.new(|cx| {
                        Onboarding::new(
                            move |_window, cx| {
                                let route = if AuthEntryOrder::opens_on_registration() {
                                    RootRoute::SignUp
                                } else {
                                    RootRoute::SignIn
                                };
                                nav_store.update(cx, |nav, cx| nav.push(route, NavigationStoreSettlingPush::default(), cx));
                            },
                            window,
                            cx,
                        )
                    });
                    self.onboarding = Some(page.clone());
                    page
                }
            };
            return page.into_any_element();
        }
        let theme = rise_ui::theme(cx as &App);
        div().size_full().bg(theme.bg._100).into_any_element()
    }

    /// one page in its slot, under the top it stays in the tree but unpainted
    /// so keyed state and scroll survive a push
    fn stack_page_element(
        page: &RootStackPage,
        is_drawn: bool,
        offset_fraction: f32,
        opacity: f32,
        window_input: &RootWindowInput,
        cached: bool,
    ) -> AnyElement {
        Self::stack_page_element_in(
            page,
            is_drawn,
            offset_fraction,
            opacity,
            window_input,
            None,
            cached,
            PageSlotWidth::Free,
        )
    }

    fn page_fills_wide_centre(route: &RootRoute) -> bool {
        matches!(
            route,
            RootRoute::MainTabs(_)
                | RootRoute::MusicTab
                | RootRoute::PostDetail { .. }
                | RootRoute::ShortViewer { .. }
                | RootRoute::PostsInteresting { .. }
                | RootRoute::Chat { .. }
                | RootRoute::ChatTopic { .. }
                | RootRoute::ForumAll { .. }
                | RootRoute::CaseContext { .. }
                | RootRoute::PinnedMessages { .. }
                | RootRoute::ChatThread { .. }
                | RootRoute::RtcRoom { .. }
                | RootRoute::ChatProfile { .. }
                | RootRoute::GeneralProfile { .. }
                | RootRoute::TopicProfile { .. }
                | RootRoute::UserPage { .. }
                | RootRoute::MyProfile
                | RootRoute::MusicArtistProfile { .. }
                | RootRoute::HashtagSearch { .. }
                | RootRoute::MusicSettings
                | RootRoute::MusicLibrary
                | RootRoute::MusicLocalTracks
                | RootRoute::MusicTrackRequestsAdmin
                | RootRoute::MusicArtistRequestsAdmin
                | RootRoute::MusicAlbumRequestsAdmin
                | RootRoute::MyMusicTrackRequests
                | RootRoute::MyMusicArtistRequest
                | RootRoute::MyMusicAlbumRequests
                | RootRoute::MyBugReports
                | RootRoute::UserSubscribers { .. }
                | RootRoute::UserSubs { .. }
                | RootRoute::UserFriends { .. }
                | RootRoute::ArtistStudio
                | RootRoute::MusicPlaylistDetail { .. }
                | RootRoute::MusicAlbumDetail { .. }
                | RootRoute::MusicPlaylistSettings { .. }
                | RootRoute::MusicPlaylistInviteLink { .. }
                | RootRoute::MusicPlaylistJoinRequests { .. }
                | RootRoute::MusicPlaylistMembers { .. }
                | RootRoute::MusicPlaylistMemberRights { .. }
                | RootRoute::CreatePostMedia { .. }
                | RootRoute::CreatePost { .. }
                | RootRoute::EditPost { .. }
                | RootRoute::CreatePostWithAchievement { .. }
                | RootRoute::BotMiniApp { .. }
                | RootRoute::BotMiniAppLink { .. }
                | RootRoute::PostStats { .. }
                | RootRoute::QrScanner
                | RootRoute::MusicNowPlaying
                | RootRoute::WorldMap
                | RootRoute::MapJourneyComposer
                | RootRoute::MapJourney { .. }
                | RootRoute::MapFriends
                | RootRoute::MapProfile
                | RootRoute::SignUp
                | RootRoute::SignIn
        )
    }

    /// how wide a centre page lays out its rows, none keeps the whole column
    fn page_column_width(route: &RootRoute, theme: &rise_theme::AppTheme) -> Option<Pixels> {
        if matches!(route, RootRoute::MyProfile | RootRoute::UserPage { .. }) {
            return Some(theme.shell.profile_max_width);
        }
        (!Self::page_fills_wide_centre(route)).then_some(theme.shell.list_max_width)
    }

    /// fills bounds with the colour a narrow page gave its column below the top inset
    /// repeats its header fade, nothing if the page has no colour
    fn paint_page_backdrop(backdrop: &PageColumnBackdrop, bounds: Bounds<Pixels>, window: &mut Window) {
        Self::paint_page_fill(backdrop, bounds, window);
        Self::paint_page_blurred_header(backdrop, bounds, window);
    }

    fn paint_page_fill(backdrop: &PageColumnBackdrop, bounds: Bounds<Pixels>, window: &mut Window) {
        let Some(color) = backdrop.color() else {
            return;
        };
        let top = px(backdrop.top_inset().clamp(0.0, f32::from(bounds.size.height)));
        let fill_bounds = Bounds::new(
            point(bounds.origin.x, bounds.origin.y + top),
            size(bounds.size.width, bounds.size.height - top),
        );
        window.paint_quad(fill(fill_bounds, color));
        if let Some(fade) = backdrop.header_fade() {
            let height = f32::from(bounds.size.height);
            let solid_end = fade.solid_end.clamp(0.0, height);
            let end = fade.end.clamp(solid_end, height);
            let mid = solid_end + (end - solid_end) * 0.45;
            let band = |from: f32, to: f32| {
                Bounds::new(
                    point(bounds.origin.x, bounds.origin.y + px(from)),
                    size(bounds.size.width, px(to - from)),
                )
            };
            let top = f32::from(top);
            if solid_end > top {
                window.paint_quad(fill(band(top, solid_end), fade.color));
            }
            if mid > solid_end {
                window.paint_quad(fill(
                    band(solid_end, mid),
                    linear_gradient(
                        180.0,
                        linear_color_stop(fade.color, 0.0),
                        linear_color_stop(fade.color.opacity(0.5), 1.0),
                    ),
                ));
            }
            if end > mid {
                window.paint_quad(fill(
                    band(mid, end),
                    linear_gradient(
                        180.0,
                        linear_color_stop(fade.color.opacity(0.5), 0.0),
                        linear_color_stop(fade.color.opacity(0.0), 1.0),
                    ),
                ));
            }
        }
    }

    fn paint_page_blurred_header(backdrop: &PageColumnBackdrop, bounds: Bounds<Pixels>, window: &mut Window) {
        let Some(header) = backdrop.blurred_header() else {
            return;
        };
        let height = header.height.clamp(0.0, f32::from(bounds.size.height));
        if height <= 0.0 {
            return;
        }
        let band = Bounds::new(bounds.origin, size(bounds.size.width, px(height)));
        window.paint_quad(fill(band, header.background));
        let direction = LayoutDirection::current();
        LinearGradient::new(header.tint, UnitPoint::TOP, UnitPoint::BOTTOM).paint(band, direction, window);
        LinearGradient::new(header.shadow, UnitPoint::TOP, UnitPoint::BOTTOM).paint(band, direction, window);
    }

    /// stack page element with rows held to column_width in the middle of a wider centre
    /// header and rows move together so back button and counter sit by the content
    #[allow(clippy::too_many_arguments)]
    fn stack_page_element_in(
        page: &RootStackPage,
        is_drawn: bool,
        offset_fraction: f32,
        opacity: f32,
        window_input: &RootWindowInput,
        column_width: Option<Pixels>,
        cached: bool,
        width: PageSlotWidth,
    ) -> AnyElement {
        let mut slot = window_input.attach(div().absolute().inset_0().occlude());
        if !is_drawn {
            slot.style().visibility = Some(Visibility::Hidden);
        }
        if offset_fraction > 0.0 {
            slot = slot.left(relative(offset_fraction)).right(relative(-offset_fraction));
        }
        let held_width = if width == PageSlotWidth::Hold { page.settled_width.get() } else { None };
        if let Some(held_width) = held_width {
            slot = if LayoutDirection::current().is_right_to_left() {
                slot.left(relative(1.0 + offset_fraction)).ml(held_width * -1.0).w(held_width)
            } else {
                slot.left(relative(offset_fraction)).w(held_width)
            };
        }
        if width == PageSlotWidth::Record {
            let settled_width = page.settled_width.clone();
            slot = slot.child(
                canvas(move |bounds, _, _| settled_width.set(Some(bounds.size.width)), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            );
        }
        if opacity < 1.0 {
            slot = slot.opacity(opacity);
        }
        let cached = if is_drawn {
            let owed = page.owes_fresh.replace(false);
            cached && !owed
        } else {
            if !cached {
                page.owes_fresh.set(true);
            }
            true
        };
        let view = if cached {
            page.view.clone().cached(StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
        } else {
            page.view.clone().into_any_element()
        };
        let content = env_scope::<SharedElementStackHostKey>(Some(page.host)).child(view);
        let Some(column_width) = column_width else {
            return slot.child(content).into_any_element();
        };
        let content = env_scope::<PageColumnBackdropKey>(Some(page.backdrop.clone())).child(content);
        let backdrop = page.backdrop.clone();
        let backdrop_fill = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| Self::paint_page_backdrop(&backdrop, bounds, window),
        )
        .absolute()
        .inset_0();
        slot.child(backdrop_fill).child(
            div()
                .size_full()
                .flex()
                .flex_row()
                .justify_center()
                .child(div().relative().h_full().w_full().max_w(column_width).child(content)),
        )
        .into_any_element()
    }

    /// a page in zoom flight over the dimming the transition puts on the source screen
    fn zoom_page_elements(
        &self,
        page: &RootStackPage,
        progress: f32,
        scrim: Hsla,
        window_input: &RootWindowInput,
        width: PageSlotWidth,
    ) -> Vec<AnyElement> {
        let cached = !self.columns_fresh;
        let Some(source_id) = ShortsZoomSource::source_id(&page.route) else {
            return vec![Self::stack_page_element_in(page, true, 0.0, 1.0, window_input, None, cached, width)];
        };
        let slot = Self::stack_page_element_in(
            page,
            true,
            0.0,
            zoom_content_opacity(progress),
            window_input,
            None,
            cached,
            width,
        );
        vec![
            div().absolute().inset_0().bg(scrim.opacity(progress)).into_any_element(),
            ZoomNavigationTransition::new(slot, self.shorts_zoom_namespace, source_id, progress).into_any_element(),
        ]
    }

    fn resolve_side_panel_context(&self, cx: &mut App) -> Option<RootSidePanelContext> {
        if self.pack_aside_fits
            && StickerPackSheetStore::shared(cx).read(cx).is_presented()
            && !CommentInteractionsStore::shared(cx).read(cx).comments_sheet_visible
        {
            return Some(RootSidePanelContext::StickerPack(self.hosts.sticker_pack_sheet.clone()));
        }
        let section = self.aside_section(cx);
        if self.aside.read(cx).can_go_back() {
            return Some(RootSidePanelContext::Aside(section));
        }
        if CommentInteractionsStore::shared(cx).read(cx).comments_sheet_visible && self.aside.read(cx).docks_comments(cx) {
            return Some(RootSidePanelContext::Aside(section));
        }
        let (chat_id, post_id, is_music_root, forum_home) = {
            let nav = self.nav_store.read(cx);
            let forum_home = nav.path.is_empty() && nav.embedded_forum_chat_id().is_some();
            let (chat_id, post_id, is_music_root) = match nav.path.last() {
                Some(
                    RootRoute::Chat { chat_id }
                    | RootRoute::ChatTopic { chat_id, .. }
                    | RootRoute::ForumAll { chat_id },
                ) => (chat_id.clone().filter(|chat_id| !chat_id.is_empty()), None, false),
                Some(RootRoute::PostDetail { post_id, .. }) => (None, post_id.clone(), false),
                Some(_) => (None, None, false),
                None => (None, None, nav.current_tab == MainTab::Music),
            };
            (chat_id, post_id, is_music_root, forum_home)
        };
        if let Some(chat_id) = chat_id {
            return Some(RootSidePanelContext::Chat(chat_id));
        }
        if forum_home && self.aside.read(cx).has_page_for(section) {
            return Some(RootSidePanelContext::Aside(section));
        }
        if let Some(post_id) = post_id {
            let post = PostServicesStore::shared(cx).update(cx, |store, cx| store.resolved_post(&post_id, cx));
            if let Some(post) = post
                && PostMediaCachePolicy::is_own_post(&post, cx)
            {
                return Some(RootSidePanelContext::PostStats(post.rise_view_target_post_id()));
            }
        }
        if is_music_root && self.music_queue_has_items {
            return Some(RootSidePanelContext::MusicQueue);
        }
        if self.aside.read(cx).has_page_for(section) {
            return Some(RootSidePanelContext::Aside(section));
        }
        None
    }

    fn chats_aside_default(&self, cx: &App) -> RootRoute {
        let nav = self.nav_store.read(cx);
        let open_chat = nav
            .path
            .iter()
            .rev()
            .find_map(|route| match route {
                RootRoute::Chat { chat_id: Some(chat_id) }
                | RootRoute::ChatTopic { chat_id: Some(chat_id), .. }
                | RootRoute::ForumAll { chat_id: Some(chat_id) }
                | RootRoute::ForumHome { chat_id: Some(chat_id) }
                    if !chat_id.is_empty() =>
                {
                    Some(chat_id.as_str())
                }
                _ => None,
            })
            .or_else(|| nav.embedded_forum_chat_id());
        match open_chat {
            Some(chat_id) => RootRoute::ChatProfile { chat_id: Some(chat_id.to_owned()) },
            None => RootRoute::MyProfile,
        }
    }

    fn profile_aside_default(&self, cx: &App) -> RootRoute {
        match self.nav_store.read(cx).path.last() {
            Some(RootRoute::UserPage { user_id, tag }) => {
                RootRoute::ProfileMore { user_id: user_id.clone(), tag: tag.clone() }
            }
            _ => RootRoute::ProfileMore { user_id: None, tag: None },
        }
    }

    fn aside_section(&self, cx: &App) -> Option<AsideSection> {
        let nav = self.nav_store.read(cx);
        if nav.settings_split_index().is_some() {
            return Some(AsideSection::Settings);
        }
        AsideScope::current(nav.current_tab, nav.path.last()).map(|scope| scope.section)
    }

    /// only reading columns keep the feed width and only beside a panel
    /// feed and post thread and chat beside the list
    /// every other page takes the whole centre
    fn caps_centre(&self, cx: &App) -> bool {
        let nav = self.nav_store.read(cx);
        let is_reading = |route: &RootRoute| {
            matches!(
                route,
                RootRoute::PostDetail { .. }
                    | RootRoute::Chat { .. }
                    | RootRoute::ChatTopic { .. }
                    | RootRoute::ForumAll { .. }
            )
        };
        if nav.current_tab == MainTab::Chats && nav.path.iter().any(is_reading) {
            return true;
        }
        match nav.path.last() {
            Some(RootRoute::PostDetail { .. }) => true,
            Some(_) => false,
            None => Self::tab_caps_centre(&nav.current_tab),
        }
    }

    fn tab_caps_centre(tab: &MainTab) -> bool {
        matches!(tab, MainTab::Posts | MainTab::Shorts)
    }

    fn toggle_aside_from_header(&mut self, aside_fits: bool, cx: &mut Context<Self>) {
        if aside_fits {
            self.side_panel.update(cx, |side_panel, cx| side_panel.set_open(true, cx));
            return;
        }
        self.aside_slide_over_open = !self.aside_slide_over_open;
        if self.aside_slide_over_open {
            self.side_panel.update(cx, |side_panel, cx| side_panel.set_open(true, cx));
        }
        cx.notify();
    }

    fn reveal_aside(&mut self, toggles: bool, cx: &mut Context<Self>) {
        self.sync_side_panel(cx);
        let fits = self.aside_fits || self.nav_store.read(cx).aside_takes_details();
        let is_open = self.side_panel.read(cx).is_open();
        if fits {
            let open = !(toggles && is_open);
            self.side_panel.update(cx, |side_panel, cx| side_panel.set_open(open, cx));
            return;
        }
        if toggles && is_open && self.aside_slide_over_open {
            self.aside_slide_over_open = false;
        } else {
            self.aside_slide_over_open = true;
            self.side_panel.update(cx, |side_panel, cx| side_panel.set_open(true, cx));
        }
        cx.notify();
    }

    fn pane_demand(&self, cx: &App) -> PaneDemand {
        let signed_in = self.shows_section_bar(cx);
        let nav = self.nav_store.read(cx);
        let splits = nav.current_tab == MainTab::Chats || nav.settings_split_index().is_some();
        PaneDemand {
            splits: signed_in && splits,
            caps_centre: signed_in && self.caps_centre(cx),
            has_aside: signed_in && self.side_panel_context.is_some(),
            aside_open: self.side_panel.read(cx).is_open(),
            centre_width: None,
        }
    }

    fn riff_centre_width(&self, window_height: Pixels, gap: Pixels, cx: &App) -> Option<Pixels> {
        (self.nav_store.read(cx).current_tab == MainTab::Shorts)
            .then(|| ((window_height - gap * 2.0) * RIFF_FRAME_ASPECT).max(gpui::px(0.0)))
    }

    fn side_panel_answer_did_change(&mut self, cx: &mut Context<Self>) {
        let previous = self.side_panel_context.clone();
        self.sync_side_panel(cx);
        if self.side_panel_context != previous {
            tracing::warn!(
                target: "riseonly::X1811",
                from = previous.as_ref().map(RootSidePanelContext::key),
                to = self.side_panel_context.as_ref().map(RootSidePanelContext::key),
                "panel answer changed"
            );
            self.motion_frame_pending = true;
            cx.notify();
        }
    }

    fn sync_side_panel(&mut self, cx: &mut Context<Self>) {
        let section = self.aside_section(cx);
        let chats_default = self.chats_aside_default(cx);
        let profile_default = self.profile_aside_default(cx);
        let search_filler = {
            let nav = self.nav_store.read(cx);
            !(nav.current_tab == MainTab::GlobalSearch && !nav.path.is_empty())
        };
        self.aside.update(cx, |aside, cx| {
            aside.set_default(AsideSection::Chats, chats_default, cx);
            aside.set_default(AsideSection::Profile, profile_default, cx);
            if search_filler {
                aside.set_default(AsideSection::Search, RootRoute::MyProfile, cx);
            } else {
                aside.clear_default(AsideSection::Search, cx);
            }
            aside.set_section(section, cx);
            aside.drop_centre_twin(cx);
        });
        self.drop_closed_map_pages(section, cx);
        let context = self.resolve_side_panel_context(cx);
        let header_overlays = matches!(context, Some(RootSidePanelContext::Aside(_)));
        let header_blurred = header_overlays && !self.aside.read(cx).shows_docked_comments(cx);
        self.side_panel.update(cx, |side_panel, cx| {
            side_panel.set_header_overlays(header_overlays, cx);
            side_panel.set_header_blurred(header_blurred, cx);
        });
        if self.side_panel_context == context {
            return;
        }
        self.side_panel_context = context.clone();
        GuideScreenObserver::visible_did_change(cx);
        let Some(context) = context else {
            return;
        };
        let enters_own_profile = matches!(context, RootSidePanelContext::Aside(Some(AsideSection::Profile))) && {
            let nav = self.nav_store.read(cx);
            nav.current_tab == MainTab::Profile && nav.path.is_empty()
        };
        let key = context.key();
        let shows_close = !context.draws_own_close();
        let header_leading = matches!(context, RootSidePanelContext::Aside(_)).then(|| self.aside_header.clone());
        self.side_panel_content.update(cx, |content, cx| {
            content.context = Some(context);
            cx.notify();
        });
        let is_new_key = {
            let current: &str = self.side_panel.read(cx).context();
            current != key
        };
        if is_new_key {
            let view: AnyView = self.side_panel_content.clone().into();
            self.side_panel.update(cx, |side_panel, cx| side_panel.set_context(key, view, cx));
        }
        if enters_own_profile {
            self.side_panel.update(cx, |side_panel, cx| side_panel.set_open(true, cx));
        }
        self.side_panel.update(cx, |side_panel, cx| {
            side_panel.set_shows_close(shows_close, cx);
            side_panel.set_header_leading(header_leading, cx);
        });
    }

    fn drop_closed_map_pages(&mut self, section: Option<AsideSection>, cx: &mut Context<Self>) {
        let is_open = section == Some(AsideSection::Maps) && self.side_panel.read(cx).is_open();
        let was_open = std::mem::replace(&mut self.map_aside_was_open, is_open);
        if was_open && !is_open && section == Some(AsideSection::Maps) && self.aside.read(cx).can_go_back() {
            self.aside.update(cx, |aside, cx| aside.reset_to_default(cx));
            cx.notify();
        }
    }

    fn in_app_call_did_change(&mut self, cx: &mut Context<Self>) {
        let key = self.hosts.rise_rtc_room_presentation_host.read(cx).in_app_call_key(cx);
        if key == self.in_app_call_key {
            return;
        }
        self.in_app_call_nav = key.as_ref().map(|_| {
            let nav = self.nav_store.read(cx);
            (nav.current_tab, nav.path.clone())
        });
        self.in_app_call_key = key;
        cx.notify();
    }

    fn minimize_in_app_call_on_navigation(&mut self, nav_store: &Entity<NavigationStore>, cx: &mut Context<Self>) {
        let Some((tab, path)) = self.in_app_call_nav.as_ref() else {
            return;
        };
        let nav = nav_store.read(cx);
        if nav.current_tab == *tab && nav.path == *path {
            return;
        }
        self.in_app_call_nav = None;
        self.hosts.rise_rtc_room_presentation_host.update(cx, |host, cx| host.minimize_in_app_call(cx));
    }

    fn in_app_call_pane(&self, framed: bool, gap: Pixels, theme: &rise_theme::AppTheme, window: &Window, cx: &App) -> Div {
        let top_inset = f32::from(self.screen_top_inset(window, cx));
        let host = self.hosts.rise_rtc_room_presentation_host.clone();
        let content = host.read(cx).in_app_call_content(host.downgrade(), cx);
        let body = div().relative().size_full().overflow_hidden().flex().justify_center().child(
            div().h_full().w_full().max_w(theme.shell.call_max_width).child(
                env_scope::<RiseRTCScreenTopInsetKey>(top_inset).child(div().size_full().child(content)),
            ),
        );
        let room = Self::pane(body, framed, Self::pane_radius(theme), gap, theme).h_full().w_full();
        div().flex_1().min_w_0().h_full().flex().child(room)
    }

    fn screen_top_inset(&self, window: &Window, cx: &App) -> Pixels {
        if self.nav_store.read(cx).path.last().is_some_and(draws_under_window_controls) {
            return gpui::px(0.0);
        }
        window_controls_band(window, cx)
    }

    fn nav_change_moves_stack_only(&self, nav_store: &Entity<NavigationStore>, cx: &App) -> bool {
        let nav = nav_store.read(cx);
        let old = &self.observed.path;
        let below_top_stays = nav.path.split_last().is_some_and(|(_, below)| old.starts_with(below));
        nav.current_tab == self.observed.current_tab
            && nav.path != *old
            && (nav.path.starts_with(old) || below_top_stays || old.starts_with(&nav.path))
    }

    fn request_motion_frame(&self, window: &Window, cx: &Context<Self>) {
        let root = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = root.update(cx, |view, cx| {
                view.motion_frame_pending = true;
                cx.notify();
            });
        });
    }

    fn navigation_stack(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = rise_ui::theme(cx as &App).clone();
        let width_scope: SharedString =
            self.aside_section(cx).map_or(SIDE_PANEL_ASIDE_CONTEXT, AsideSection::context_key).into();
        self.side_panel.update(cx, |panel, cx| panel.set_width_scope(width_scope.clone(), cx));
        self.sync_list_width_scope(width_scope, &theme);
        let (aside_width, aside_chosen) = {
            let panel = self.side_panel.read(cx);
            (panel.width(), panel.has_chosen_width())
        };
        let policy = PanePolicy::from_shell_metrics(theme.shell)
            .with_panel_metrics(theme.panel)
            .with_list_width(self.list_width)
            .with_chosen_aside(aside_chosen);
        let mut demand = self.pane_demand(cx);
        let framed = self.shows_section_bar(cx);
        let gap = if framed { theme.shell.rail_outer_inset } else { gpui::px(0.0) };
        let riff_width = self.riff_centre_width(window.viewport_size().height, gap, cx);
        if self.nav_store.read(cx).path.is_empty() {
            demand.centre_width = riff_width;
        }
        let radius = Self::pane_radius(&theme);
        let rail_width = if framed {
            theme.shell.rail_width + theme.shell.rail_outer_inset * 2.0
        } else {
            gpui::px(0.0)
        };
        let width = window.viewport_size().width - rail_width - gap;
        self.pane_layout = policy.layout_for_resize_demand(width, demand, self.pane_layout);
        let columns = policy.columns(width, self.pane_layout, demand, aside_width);
        if let Some(list) = columns.list {
            let panel_spare = columns.aside.map_or(gpui::px(0.0), |aside| {
                (aside - aside_width.clamp(policy.aside_min_width, policy.aside_max_width)).max(gpui::px(0.0))
            });
            self.list_drag_ceiling =
                list + (columns.centre - policy.split_detail_min_width).max(gpui::px(0.0)) + panel_spare;
        }
        let aside_fits = policy.aside_fits(width, demand);
        self.aside_fits = aside_fits;
        self.pack_aside_fits = framed && policy.aside_fits(width, PaneDemand { has_aside: true, ..demand });
        let previews_in_aside = framed
            && self.aside_section(cx) == Some(AsideSection::Search)
            && policy.aside_fits(width, PaneDemand { has_aside: true, ..demand });
        let comments_in_aside = framed
            && matches!(self.aside_section(cx), Some(AsideSection::Riffs | AsideSection::Posts))
            && policy.aside_fits(width, PaneDemand { has_aside: true, ..demand });
        let requests_in_aside = framed
            && self.nav_store.read(cx).path.last().is_some_and(is_music_request_list)
            && policy.aside_fits(width, PaneDemand { has_aside: true, ..demand });
        let map_settings_in_aside = framed
            && self.aside_section(cx) == Some(AsideSection::Maps)
            && policy.aside_fits(width, PaneDemand { has_aside: true, ..demand });
        let post_comments_in_aside = framed
            && matches!(self.nav_store.read(cx).path.last(), Some(RootRoute::PostDetail { .. }))
            && policy.aside_fits(width, PaneDemand { has_aside: true, ..demand });
        let profile_details_in_aside = framed
            && matches!(self.nav_store.read(cx).path.last(), Some(RootRoute::UserPage { .. } | RootRoute::MyProfile))
            && policy.aside_fits(width, PaneDemand { has_aside: true, ..demand });
        let slide_over = columns.slide_over.filter(|_| framed);
        let hosts_chat_info = framed && (columns.aside.is_some() || slide_over.is_some());
        if self.nav_store.read(cx).aside_hosts_chat_info() != hosts_chat_info {
            self.nav_store.update(cx, |nav, _| nav.set_aside_hosts_chat_info(hosts_chat_info));
            self.columns_fresh = true;
        }
        if slide_over.is_none() || aside_fits {
            self.aside_slide_over_open = false;
        }
        let slides_over = slide_over.filter(|_| self.aside_slide_over_open && self.side_panel.read(cx).is_open());
        let panel_shown = slides_over.is_some() || (columns.aside.is_some() && self.side_panel.read(cx).is_open());
        self.aside.update(cx, |aside, cx| aside.set_panel_shown(panel_shown, cx));
        let pack_in_panel = (columns.aside.is_some() || slides_over.is_some())
            && matches!(self.side_panel_context, Some(RootSidePanelContext::StickerPack(_)));
        self.hosts.sticker_pack_sheet.update(cx, |sheet, cx| sheet.set_presents_in_side_panel(pack_in_panel, cx));

        let now = RiseClock::monotonic();
        let fraction = self.page_motion.value(now);
        let is_moving = self.page_motion.is_running(now);
        if is_moving {
            self.request_motion_frame(window, cx);
        }
        if !is_moving {
            self.departing_page = None;
        }

        let top_index = self.stack.len().checked_sub(1);
        let is_split = columns.list.is_some();
        let settings_split = if is_split {
            self.nav_store.read(cx).settings_split_index().filter(|index| *index < self.stack.len())
        } else {
            None
        };
        let chat_split = is_split && settings_split.is_none();
        if self.nav_store.read(cx).chat_list_beside_stack() != chat_split {
            self.nav_store.update(cx, |nav, _| nav.set_chat_list_beside_stack(chat_split));
            self.columns_fresh = true;
        }
        if self.nav_store.read(cx).settings_list_beside_stack() != settings_split.is_some() {
            let beside = settings_split.is_some();
            self.nav_store.update(cx, |nav, _| nav.set_settings_list_beside_stack(beside));
            self.columns_fresh = true;
        }
        if let Some(index) = settings_split
            && index + 1 == self.stack.len()
            && self.nav_store.read(cx).path.len() == index + 1
        {
            let nav_store = self.nav_store.clone();
            cx.defer(move |cx| {
                nav_store.update(cx, |nav, cx| {
                    if nav.settings_list_beside_stack()
                        && nav.settings_split_index().is_some_and(|index| index + 1 == nav.path.len())
                    {
                        nav.open_first_settings_section(
                            crate::modules::user::pages::profile::settings::settings::Settings::first_section_route(),
                            cx,
                        );
                    }
                });
            });
        }
        let under_top_is_drawn = is_moving && self.page_motion_is_push;
        let root_is_drawn =
            chat_split || top_index.is_none() || (under_top_is_drawn && top_index == Some(0) && settings_split.is_none());

        let root_demand = PaneDemand {
            caps_centre: framed && Self::tab_caps_centre(&self.nav_store.read(cx).current_tab),
            centre_width: riff_width,
            ..demand
        };
        let mut root_slot = div().id("root-main-page").absolute().inset_0();
        if !chat_split && root_demand != demand {
            let root_width = policy.columns(width, self.pane_layout, root_demand, aside_width).centre_content;
            let leading = ((columns.centre_content - root_width) * 0.5).max(gpui::px(0.0));
            root_slot = div().id("root-main-page").absolute().top_0().bottom_0().left(leading).w(root_width);
        }
        if !root_is_drawn {
            root_slot.style().visibility = Some(Visibility::Hidden);
        }
        let root_slot = root_slot.child(self.root_page(window, cx));
        let (root_in_list, root_in_centre) = if chat_split { (Some(root_slot), None) } else { (None, Some(root_slot)) };
        if !chat_split {
            crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop::set_root_list_column(
                None, window,
            );
        }

        let shows_aside = columns.aside.is_some() && self.side_panel.read(cx).is_open();
        let takes_details = shows_aside
            || previews_in_aside
            || comments_in_aside
            || requests_in_aside
            || map_settings_in_aside
            || post_comments_in_aside
            || profile_details_in_aside;
        if self.nav_store.read(cx).aside_takes_details() != takes_details {
            tracing::info!(target: "riseonly::X1455", takes_details, shows_aside, profile_details_in_aside, "panel takes details changed");
            self.nav_store.update(cx, |nav, _| nav.set_aside_takes_details(takes_details));
            self.columns_fresh = true;
        }
        let header_opens_panel = matches!(
            self.nav_store.read(cx).path.last(),
            Some(RootRoute::Chat { .. } | RootRoute::ChatTopic { .. } | RootRoute::ForumAll { .. })
        );
        let page_runs_under_toggle = {
            let nav = self.nav_store.read(cx);
            match nav.path.last() {
                None => nav.current_tab == MainTab::Maps,
                Some(route) => matches!(
                    route,
                    RootRoute::UserPage { .. }
                        | RootRoute::MyProfile
                        | RootRoute::CaseContext { .. }
                        | RootRoute::PinnedMessages { .. }
                        | RootRoute::MainTabs(MainTab::Maps)
                        | RootRoute::WorldMap
                ),
            }
        };
        let toggle_side = theme.shell.rail_icon_size;
        let toggle_plate = gpui::px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE);
        let toggle_reserve = if slide_over.is_some() && !header_opens_panel {
            toggle_plate + theme.shell.rail_padding * 2.0
        } else {
            gpui::px(0.0)
        };
        let (layout_reserve, header_reserve) =
            if page_runs_under_toggle { (gpui::px(0.0), toggle_reserve) } else { (toggle_reserve, gpui::px(0.0)) };
        if self.header_reserve != header_reserve {
            self.header_reserve = header_reserve;
            self.columns_fresh = true;
        }
        let cached = !self.columns_fresh;
        let window_input = Self::window_input(cx);
        let mut pages = Vec::with_capacity(self.stack.len() + 1);
        let mut settings_list_page = None;
        for (index, page) in self.stack.iter().enumerate() {
            if let Some(settings_index) = settings_split {
                if index == settings_index {
                    settings_list_page = Some(Self::stack_page_element_in(
                        page,
                        true,
                        0.0,
                        1.0,
                        &window_input,
                        columns.list,
                        cached,
                        PageSlotWidth::Free,
                    ));
                    continue;
                }
                if index < settings_index {
                    pages.push(Self::stack_page_element(page, false, 0.0, 1.0, &window_input, cached));
                    continue;
                }
            }
            let is_top = Some(index) == top_index;
            let is_under_top = index + 2 == self.stack.len();
            let is_drawn = is_top || (is_under_top && under_top_is_drawn);
            let is_arriving = is_top && is_moving && self.page_motion_is_push;
            if is_arriving && self.page_motion_is_zoom {
                pages.extend(self.zoom_page_elements(
                    page,
                    fraction,
                    theme.material.scrim,
                    &window_input,
                    PageSlotWidth::Free,
                ));
                continue;
            }
            let offset = if is_arriving { 1.0 - fraction } else { 0.0 };
            let column_width = Self::page_column_width(&page.route, &theme);
            let slot_width = if is_top && !is_moving { PageSlotWidth::Record } else { PageSlotWidth::Hold };
            let element =
                Self::stack_page_element_in(page, is_drawn, offset, 1.0, &window_input, column_width, cached, slot_width);
            let covered = is_under_top && under_top_is_drawn;
            let element = env_scope::<PageCoveredByArrivingPageKey>(covered).child(element).into_any_element();
            if settings_split.is_some_and(|settings_index| index == settings_index + 1) {
                pages.push(env_scope::<PageBesideSplitListKey>(true).child(element).into_any_element());
            } else {
                pages.push(element);
            }
        }
        if let Some(departing) = self.departing_page.as_ref() {
            let leaving = if self.page_motion_is_zoom {
                self.zoom_page_elements(departing, fraction, theme.material.scrim, &window_input, PageSlotWidth::Hold)
            } else {
                let column_width = Self::page_column_width(&departing.route, &theme);
                vec![Self::stack_page_element_in(
                    departing,
                    true,
                    1.0 - fraction,
                    1.0,
                    &window_input,
                    column_width,
                    cached,
                    PageSlotWidth::Hold,
                )]
            };
            pages.extend(
                leaving
                    .into_iter()
                    .map(|element| env_scope::<PageCoveredByArrivingPageKey>(true).child(element).into_any_element()),
            );
        }

        let installer = UINavigationFullscreenPopInstaller::new(self.nav_store.clone());
        let top_inset = self.screen_top_inset(window, cx);
        let rtl = LayoutDirection::current().is_right_to_left();
        let player_beside = (shows_aside || slides_over.is_some())
            && matches!(self.side_panel_context, Some(RootSidePanelContext::Aside(_)))
            && self.aside.read(cx).top_route() == Some(&RootRoute::MusicNowPlaying);
        aside_navigator::set_player_beside(player_beside, cx);
        let pane_gaps = usize::from(columns.list.is_some()) + usize::from(shows_aside);
        let centre_width = (columns.centre - gap * pane_gaps as f32).max(gpui::px(0.0));
        let slides_over = slides_over.map(|panel_width| {
            let room = centre_width - policy.detail_min_width;
            if room >= policy.aside_min_width { panel_width.min(room) } else { panel_width.min(centre_width) }
        });
        let aside_column = columns.aside.or(slides_over);
        self.side_panel.update(cx, |side_panel, cx| side_panel.set_column_width(aside_column, cx));
        let empty_centre = chat_split && pages.is_empty();
        let backdrop_hidden = !self.stack.is_empty();
        let slide_reserve = slides_over
            .filter(|panel_width| centre_width - *panel_width >= policy.detail_min_width)
            .unwrap_or(gpui::px(0.0));
        let panel_button = slide_over.filter(|_| !header_opens_panel).map(|_| {
            let this = cx.weak_entity();
            let side = toggle_side;
            div()
                .id("root-aside-toggle")
                .absolute()
                .top(top_inset)
                .when(rtl, |toggle| toggle.left(slide_reserve))
                .when(!rtl, |toggle| toggle.right(slide_reserve))
                .h(gpui::px(PAGE_HEADER_ROW_HEIGHT))
                .pt(gpui::px(PageHeaderMetrics::TOP_INSET))
                .px(theme.shell.rail_padding)
                .flex()
                .items_center()
                .occlude()
                .tooltip_label(tr(ASIDE_TOGGLE_TOOLTIP_KEY))
                .child(
                    plain("root-aside-toggle-button")
                        .size(toggle_plate)
                        .rounded_full()
                        .child(
                            PageHeaderToolbarSlot::new(
                                div().flex().items_center().justify_center().children(IconUi::sized(
                                    ASIDE_TOGGLE_SYMBOL,
                                    side,
                                    theme.text.primary,
                                    false,
                                )),
                            )
                            .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE),
                        )
                        .on_press(move |_window, cx| {
                            let _ = this.update(cx, |view, cx| view.toggle_aside_from_header(aside_fits, cx));
                        }),
                )
        });
        let centre_room = (centre_width - slide_reserve - layout_reserve).max(gpui::px(0.0));
        let centre_content = div()
            .relative()
            .h_full()
            .w(columns.centre_content.min(centre_room))
            .overflow_hidden()
            .children(root_in_centre)
            .when(empty_centre, |content| {
                content.child(
                    div().size_full().flex().flex_col().justify_center().px(gpui::px(16.0)).child(
                        crate::modules::chat::components::chat::message_system_pill::MessageSystemPill::new(
                            tr(EMPTY_CHAT_COLUMN_KEY),
                        ),
                    ),
                )
            })
            .children(pages)
            .child(installer)
            .child(rise_ui::swiftui::measure::measure("root-centre-column-bounds", |bounds, _, cx| {
                NotifierWindowPresenter::set_centre_column(Some(bounds), cx)
            }));

        let slide_over_panel = slides_over.map(|panel_width| {
            div()
                .id("root-aside-slide-over")
                .absolute()
                .top(top_inset)
                .bottom_0()
                .when(rtl, |panel| panel.left_0())
                .when(!rtl, |panel| panel.right_0())
                .w(panel_width)
                .occlude()
                .shadow_lg()
                .child(self.side_panel_element(panel_width))
        });

        let centre_backdrop = top_index
            .filter(|index| settings_split != Some(*index))
            .and_then(|index| self.stack.get(index))
            .filter(|page| Self::page_column_width(&page.route, &theme).is_some())
            .map(|page| page.backdrop.clone());
        let centre_backdrop_fill = centre_backdrop.map(|backdrop| {
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let width = (bounds.size.width - slide_reserve).max(gpui::px(0.0));
                    let height = (bounds.size.height - top_inset).max(gpui::px(0.0));
                    let x = if rtl { bounds.origin.x + slide_reserve } else { bounds.origin.x };
                    let fill_bounds = Bounds::new(point(x, bounds.origin.y + top_inset), size(width, height));
                    Self::paint_page_backdrop(&backdrop, fill_bounds, window);
                },
            )
            .absolute()
            .inset_0()
        });

        let centre = div()
            .relative()
            .h_full()
            .w(centre_width)
            .flex_none()
            .pt(top_inset)
            .when(rtl, |centre| centre.pl(slide_reserve + layout_reserve))
            .when(!rtl, |centre| centre.pr(slide_reserve + layout_reserve))
            .flex()
            .flex_row()
            .justify_center()
            .when(chat_split, |centre| {
                let mut layer = div().absolute().inset_0();
                if backdrop_hidden {
                    layer.style().visibility = Some(Visibility::Hidden);
                }
                centre.child(
                    layer.child(
                        crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatBackdropLayers::new(
                            crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatBackdropLayout::FullBleed,
                        ),
                    ),
                )
            })
            .children(centre_backdrop_fill)
            .child(
                env_scope::<PanelToggleHeaderReserveKey>(header_reserve)
                    .child(env_scope::<MusicPlayerHiddenUnderFullPlayerKey>(true).child(centre_content)),
            )
            .children(panel_button)
            .children(slide_over_panel);

        let row = div().flex();
        let mut row = if rtl { row.flex_row_reverse() } else { row.flex_row() }.flex_1().min_w_0().h_full();
        if framed {
            row = row.pt(gap).pb(gap);
            row = if rtl { row.pl(gap) } else { row.pr(gap) };
        }
        let order = if framed { self.displayed_pane_order() } else { RootPane::DEFAULT_ORDER };
        let mut panes: Vec<(RootPane, AnyElement, Option<Pixels>)> = Vec::new();
        let mut floating: Option<AnyElement> = None;
        let aside_leads = RootPane::position(&order, RootPane::Aside) < RootPane::position(&order, RootPane::Centre);
        let aside_in_row = columns.aside.is_some() && shows_aside && self.in_app_call_key.is_none();
        let docks = framed && self.in_app_call_key.is_none();
        let pane_width = |pane: RootPane| match pane {
            RootPane::List => columns.list.unwrap_or_default(),
            RootPane::Centre => centre_width,
            RootPane::Aside => columns.aside.filter(|_| aside_in_row).unwrap_or_default(),
        };
        let in_row = |pane: RootPane| match pane {
            RootPane::List => columns.list.is_some(),
            RootPane::Centre => true,
            RootPane::Aside => aside_in_row,
        };
        self.pane_strides = RootPane::DEFAULT_ORDER.map(|pane| (pane, f32::from(pane_width(pane) + gap)));
        self.panes_in_row = self.pane_order.iter().copied().filter(|pane| in_row(*pane)).collect();
        let slot = |pane: RootPane| PaneSlot {
            inset: order
                .iter()
                .take_while(|candidate| **candidate != pane)
                .filter(|candidate| in_row(**candidate))
                .map(|candidate| pane_width(*candidate) + gap)
                .fold(gpui::px(0.0), |sum, stride| sum + stride),
            width: pane_width(pane),
            gap,
        };
        self.side_panel.update(cx, |panel, cx| {
            panel.set_grip_trailing(aside_leads != rtl, cx);
            panel.set_grip_outside(aside_in_row, cx);
        });
        if let Some(list_width) = columns.list {
            let has_settings_list = settings_list_page.is_some();
            let list = div()
                .relative()
                .h_full()
                .flex_none()
                .w(list_width)
                .pt(top_inset)
                .when(root_in_list.is_some(), |list| {
                    list.child(rise_ui::swiftui::measure::measure("root-list-column-bounds", |bounds, window, _| {
                        crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop::set_root_list_column(
                            Some(bounds),
                            window,
                        )
                    }))
                })
                .children(root_in_list)
                .children(settings_list_page)
                .when(has_settings_list, |list| list.child(Self::x1418_list_probe()))
                .on_drag_move(cx.listener(|view, event: &DragMoveEvent<ListColumnDrag>, _window, cx| {
                    view.resize_list_to(event.event.position.x, cx);
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|view, _: &MouseUpEvent, _window, cx| view.end_list_resize(cx)))
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|view, _: &MouseUpEvent, _window, cx| view.end_list_resize(cx)),
                );
            let (list, lifted) = self.dockable(Self::pane(list, framed, radius, gap, &theme), RootPane::List, docks, slot(RootPane::List), &theme, window, cx);
            floating = floating.or(lifted);
            panes.push((RootPane::List, list, Some(list_width)));
            if !framed {
                panes.push((RootPane::List, BlockUi::divider(&theme).into_any_element(), Some(theme.shell.column_divider)));
            }
        }
        if self.in_app_call_key.is_some() {
            panes.push((RootPane::Centre, self.in_app_call_pane(framed, gap, &theme, window, cx).into_any_element(), None));
        } else {
            let (centre, lifted) = self.dockable(Self::pane(centre, framed, radius, gap, &theme), RootPane::Centre, docks, slot(RootPane::Centre), &theme, window, cx);
            floating = floating.or(lifted);
            panes.push((RootPane::Centre, centre, Some(centre_width)));
        }
        if let Some(aside_width) = columns.aside.filter(|_| aside_in_row) {
            let aside = div()
                .relative()
                .h_full()
                .flex_none()
                .w(aside_width)
                .pt(top_inset)
                .child(self.side_panel_element(aside_width));
            let (aside, lifted) = self.dockable(Self::pane(aside, framed, radius, gap, &theme), RootPane::Aside, docks, slot(RootPane::Aside), &theme, window, cx);
            floating = floating.or(lifted);
            panes.push((RootPane::Aside, aside, Some(aside_width)));
        }
        panes.sort_by_key(|(pane, _, _)| RootPane::position(&order, *pane));
        let kinds: Vec<RootPane> = panes.iter().map(|(pane, _, _)| *pane).collect();
        let widths: Vec<Option<Pixels>> = panes.iter().map(|(_, _, width)| *width).collect();
        let centre_at = kinds.iter().position(|pane| *pane == RootPane::Centre).unwrap_or(0);
        let gap_before = |index: usize| index > 0 && kinds.get(index - 1) != kinds.get(index);
        let end_pad = if framed { gap } else { gpui::px(0.0) };
        let panes_move = self.pane_motion.is_running(RiseClock::monotonic());
        let mut splitters: Vec<AnyElement> = Vec::new();
        for (index, (pane, element, _)) in panes.into_iter().enumerate() {
            if gap_before(index) {
                row = row.child(div().flex_none().h_full().w(gap));
                if !panes_move {
                    let owner_leads = centre_at >= index;
                    let owner = if owner_leads { kinds[index - 1] } else { pane };
                    let edge = if owner_leads {
                        let span = widths[..index].iter().map(|width| width.unwrap_or_default()).fold(gpui::px(0.0), |sum, width| sum + width);
                        let gaps = (1..index).filter(|earlier| gap_before(*earlier)).count() as f32;
                        SplitterEdge::FromStart(span + gap * gaps)
                    } else {
                        let span = widths[index..].iter().map(|width| width.unwrap_or_default()).fold(gpui::px(0.0), |sum, width| sum + width);
                        let gaps = (index + 1..kinds.len()).filter(|later| gap_before(*later)).count() as f32;
                        SplitterEdge::FromEnd(end_pad + span + gap * gaps)
                    };
                    splitters.push(self.pane_splitter(owner, owner_leads == rtl, edge, columns.list, gap, &theme, cx).into_any_element());
                }
            }
            row = row.child(element);
        }
        let holes = if docks {
            let viewport = window.viewport_size().width;
            (0..kinds.len())
                .filter_map(|index| {
                    let width = widths[index]?;
                    let span = widths[..index].iter().map(|width| width.unwrap_or_default()).fold(gpui::px(0.0), |sum, width| sum + width);
                    let gaps = (1..=index).filter(|earlier| gap_before(*earlier)).count() as f32;
                    let middle = rail_width + span + gap * gaps + width * 0.5;
                    let middle = if rtl { viewport - middle } else { middle };
                    Some((middle - gpui::px(PANE_GRIP_WIDTH * 0.5), middle + gpui::px(PANE_GRIP_WIDTH * 0.5)))
                })
                .collect()
        } else {
            Vec::new()
        };
        self.title_bar.update(cx, |bar, cx| bar.set_drag_holes(holes, cx));
        row.relative()
            .children(splitters)
            .children(floating)
            .when(self.pane_motion.dragging().is_some(), |row| row.cursor_grabbing())
            .on_drag_move(cx.listener(|view, event: &DragMoveEvent<AsideColumnDrag>, _window, cx| {
                let pointer_x = event.event.position.x;
                view.side_panel.update(cx, |panel, cx| panel.resize_to_pointer(pointer_x, cx));
            }))
            .on_drag_move(cx.listener(|view, event: &DragMoveEvent<RootPaneDrag>, _window, cx| {
                view.drag_pane(event.event.position.x, cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|view, _: &MouseUpEvent, _window, cx| view.end_column_gestures(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|view, _: &MouseUpEvent, _window, cx| view.end_column_gestures(cx)))
    }

    fn end_column_gestures(&mut self, cx: &mut Context<Self>) {
        self.side_panel.update(cx, |panel, cx| panel.end_resize(cx));
        self.end_pane_drag(cx);
    }

    fn displayed_pane_order(&self) -> [RootPane; 3] {
        if self.pane_motion.dragging().is_none() {
            return self.pane_order;
        }
        RootPane::merged_order(self.pane_order, &self.panes_in_row, &self.pane_motion.shown_order(&self.panes_in_row))
    }

    fn drag_pane(&mut self, pointer_x: Pixels, cx: &mut Context<Self>) {
        let Some((pane, start_x)) = self.pane_press else {
            return;
        };
        let direction = if LayoutDirection::current().is_right_to_left() { -1.0 } else { 1.0 };
        let translation = f32::from(pointer_x - start_x) * direction;
        let strides = self.pane_strides;
        let stride = move |candidate: &RootPane| {
            strides.iter().find(|(pane, _)| pane == candidate).map_or(0.0, |(_, stride)| *stride)
        };
        let reduce_motion = rise_ui::swiftui::environment::reduce_motion(cx);
        let base = self.panes_in_row.clone();
        self.pane_motion.drag(&pane, translation, &base, stride, RiseClock::monotonic(), reduce_motion);
        cx.notify();
    }

    fn end_pane_drag(&mut self, cx: &mut Context<Self>) {
        self.pane_press = None;
        let reduce_motion = rise_ui::swiftui::environment::reduce_motion(cx);
        let Some(shown) = self.pane_motion.end(RiseClock::monotonic(), reduce_motion) else {
            return;
        };
        let order = RootPane::merged_order(self.pane_order, &self.panes_in_row, &shown);
        if order != self.pane_order {
            self.pane_order = order;
            RootPane::store_order(order);
        }
        cx.notify();
    }

    /// a framed column lifts out by the grip at its top and follows the pointer, the others slide aside
    /// a double click on the grip puts every column back
    #[allow(clippy::too_many_arguments)]
    fn dockable(
        &self,
        pane_element: Div,
        pane: RootPane,
        docks: bool,
        slot: PaneSlot,
        theme: &rise_theme::AppTheme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        if !docks {
            return (pane_element.into_any_element(), None);
        }
        let dragged = self.pane_motion.dragging() == Some(&pane);
        let others_move = self.pane_motion.dragging().is_some_and(|moving| *moving != pane);
        let lift = self.pane_motion.lift(&pane, window);
        let offset = self.pane_motion.offset(&pane, window);
        let scale = window.scale_factor();
        let snap = move |value: f32| gpui::px((value * scale).round() / scale);
        let bar = theme.text.secondary;
        let grip = div()
            .absolute()
            .top(gpui::px(2.0))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id(SharedString::new_static(pane.group()))
                    .w(gpui::px(PANE_GRIP_WIDTH))
                    .h(gpui::px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .occlude()
                    .when(dragged, |grip| grip.cursor_grabbing())
                    .when(!dragged, |grip| grip.cursor_grab())
                    .tooltip_label(tr("desktop_pane_drag_hint"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, event: &MouseDownEvent, _, _| view.pane_press = Some((pane, event.position.x))),
                    )
                    .on_click(cx.listener(|view, event: &ClickEvent, _, cx| {
                        if event.click_count() == 2 {
                            view.reset_pane_order(cx);
                        }
                    }))
                    .on_drag(RootPaneDrag, |_, _, _, cx| cx.new(|_| RootPaneDragGhost))
                    .child(
                        div()
                            .w(gpui::px(40.0))
                            .h(gpui::px(4.0))
                            .rounded_full()
                            .bg(bar.opacity(if dragged { 0.45 } else { 0.0 }))
                            .group_hover(pane.group(), move |style| style.bg(bar.opacity(0.45))),
                    ),
            );
        let pane_element = pane_element.group(pane.group()).when(!others_move, |element| element.child(grip));
        let rtl = LayoutDirection::current().is_right_to_left();
        if !dragged && lift <= 0.0 {
            let shifted = pane_element
                .when(rtl, |element| element.right(snap(offset)))
                .when(!rtl, |element| element.left(snap(offset)));
            return (shifted.into_any_element(), None);
        }
        let rise = snap(PANE_LIFT_RISE * lift);
        let inset = slot.inset + snap(offset);
        let lifted = div()
            .absolute()
            .top(slot.gap - rise)
            .bottom(slot.gap + rise)
            .w(slot.width)
            .when(rtl, |float| float.right(inset))
            .when(!rtl, |float| float.left(inset))
            .child(pane_element.shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.08 + 0.22 * lift),
                offset: gpui::point(gpui::px(0.0), gpui::px(1.0 + 13.0 * lift)),
                blur_radius: gpui::px(6.0 + 30.0 * lift),
                spread_radius: gpui::px(0.0),
                inset: false,
            }]));
        let hole = div().flex_none().h_full().w(slot.width);
        (hole.into_any_element(), Some(lifted.into_any_element()))
    }

    fn reset_pane_order(&mut self, cx: &mut Context<Self>) {
        self.pane_press = None;
        self.pane_motion.reset();
        if self.pane_order == RootPane::DEFAULT_ORDER {
            cx.notify();
            return;
        }
        self.pane_order = RootPane::DEFAULT_ORDER;
        RootPane::store_order(self.pane_order);
        self.columns_fresh = true;
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn pane_splitter(
        &self,
        owner: RootPane,
        flipped: bool,
        edge: SplitterEdge,
        list_width: Option<Pixels>,
        gap: Pixels,
        theme: &rise_theme::AppTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let reach = theme.panel.grip_width;
        let leading = LayoutDirection::current().is_right_to_left();
        let hitbox = div()
            .id(("root-pane-splitter", owner as usize))
            .absolute()
            .top_0()
            .bottom_0()
            .w(gap + reach)
            .map(|hitbox| match edge {
                SplitterEdge::FromStart(inset) if leading => hitbox.right(inset - reach * 0.5),
                SplitterEdge::FromStart(inset) => hitbox.left(inset - reach * 0.5),
                SplitterEdge::FromEnd(inset) if leading => hitbox.left(inset - reach * 0.5),
                SplitterEdge::FromEnd(inset) => hitbox.right(inset - reach * 0.5),
            })
            .occlude()
            .cursor_col_resize();
        let hitbox = match (owner, list_width) {
            (RootPane::List, Some(list_width)) => hitbox
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, event: &MouseDownEvent, _window, cx| {
                        view.begin_list_resize(event, list_width, flipped, cx);
                    }),
                )
                .on_drag(ListColumnDrag { start_x: gpui::px(0.0), start_width: list_width, rtl: flipped }, |_, _, _, cx| {
                    cx.new(|_| ListColumnResizeGhost)
                }),
            _ => hitbox
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, event: &MouseDownEvent, _window, cx| {
                        view.side_panel.update(cx, |panel, cx| panel.begin_resize(event, cx));
                    }),
                )
                .on_drag(AsideColumnDrag, |_, _, _, cx| cx.new(|_| ListColumnResizeGhost)),
        };
        hitbox
    }

    fn begin_list_resize(&mut self, event: &MouseDownEvent, list_width: Pixels, rtl: bool, cx: &mut Context<Self>) {
        if event.click_count >= 2 {
            self.list_drag = None;
            self.list_width = None;
            SidePanelPreferences::standard().forget_list_width(&self.list_width_scope);
            cx.notify();
            return;
        }
        self.list_drag = Some(ListColumnDrag {
            start_x: event.position.x,
            start_width: list_width,
            rtl,
        });
        cx.notify();
    }

    fn resize_list_to(&mut self, pointer_x: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.list_drag else {
            return;
        };
        let metrics = rise_ui::theme(cx as &App).panel;
        let delta = pointer_x - drag.start_x;
        let delta = if drag.rtl { -delta } else { delta };
        let width = panel_gestures::list_width_during_drag(&metrics, drag.start_width, delta, self.list_drag_ceiling);
        if self.list_width != Some(width) {
            self.list_width = Some(width);
            cx.notify();
        }
    }

    fn end_list_resize(&mut self, cx: &mut Context<Self>) {
        if self.list_drag.take().is_none() {
            return;
        }
        if let Some(width) = self.list_width {
            SidePanelPreferences::standard().remember_list_width(&self.list_width_scope, width);
        }
        cx.notify();
    }

    fn sync_list_width_scope(&mut self, scope: SharedString, theme: &rise_theme::AppTheme) {
        if self.list_width_scope == scope || self.list_drag.is_some() {
            return;
        }
        self.list_width = SidePanelPreferences::standard().list_width(&scope, &theme.panel);
        self.list_width_scope = scope;
    }

    fn x1418_list_probe() -> impl IntoElement {
        canvas(
            |bounds, window, _| (bounds, window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)),
            move |_, (bounds, hitbox): (Bounds<Pixels>, gpui::Hitbox), window, _| {
                let wheel_hitbox = hitbox.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
                    if phase != gpui::DispatchPhase::Capture {
                        return;
                    }
                    let inside = bounds.contains(&event.position);
                    let reachable = hitbox.is_hovered(window);
                    tracing::warn!(
                        target: "riseonly::X1418",
                        position = ?event.position,
                        inside,
                        reachable,
                        "settings list down"
                    );
                    if inside && !reachable {
                        Self::x1556_log_blocker(event.position, "down", window);
                    }
                });
                window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, window, _| {
                    if phase != gpui::DispatchPhase::Capture || !bounds.contains(&event.position) {
                        return;
                    }
                    let reachable = wheel_hitbox.should_handle_scroll(window);
                    tracing::warn!(
                        target: "riseonly::X1465",
                        position = ?event.position,
                        reachable,
                        column_height = ?bounds.size.height,
                        "settings list wheel"
                    );
                    if !reachable {
                        Self::x1556_log_blocker(event.position, "wheel", window);
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }

    /// debug log for the list column going deaf after a long session, no idea why this shit happens
    /// prints size, place and layer of the hitbox that eats the pointer or none
    fn x1556_log_blocker(position: gpui::Point<Pixels>, input: &'static str, window: &Window) {
        match window.mouse_blocker_at(position) {
            Some((bounds, behavior, layer, in_front)) => tracing::warn!(
                target: "riseonly::X1556",
                input,
                ?position,
                ?bounds,
                ?behavior,
                layer,
                in_front,
                "settings list blocked by"
            ),
            None => tracing::warn!(target: "riseonly::X1556", input, ?position, "settings list unblocked, probe not hit"),
        }
    }

    /// one corner radius for every pane, the window own arc
    /// zero gap keeps the seam from shrinking the macos arc
    fn pane_radius(theme: &rise_theme::AppTheme) -> Pixels {
        gpui::px(current_pane_corner_radius(0.0, f32::from(theme.shell.block_radius)))
    }

    /// backdrop between panes, a shade off the panes so each reads as its own box
    fn pane_backdrop(theme: &rise_theme::AppTheme) -> gpui::Hsla {
        theme.bg._200
    }

    /// a column as its own rounded box over the backdrop
    /// gpui clips children to rectangles only so a ring in the backdrop colour covers the square corners
    fn pane(column: Div, framed: bool, radius: Pixels, gap: Pixels, theme: &rise_theme::AppTheme) -> Div {
        if !framed {
            return column;
        }
        let ring = gap.max(radius);
        column
            .overflow_hidden()
            .rounded(radius)
            .bg(theme.bg._100)
            .shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.08),
                offset: gpui::point(gpui::px(0.0), gpui::px(1.0)),
                blur_radius: gpui::px(6.0),
                spread_radius: gpui::px(0.0),
                inset: false,
            }])
            .child(
                div()
                    .absolute()
                    .top(-ring)
                    .left(-ring)
                    .right(-ring)
                    .bottom(-ring)
                    .rounded(radius + ring)
                    .border(ring)
                    .border_color(Self::pane_backdrop(theme)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(radius)
                    .border_1()
                    .border_color(theme.border._100),
            )
    }

    fn music_player_row(&self, framed: bool, theme: &rise_theme::AppTheme, cx: &App) -> Option<AnyElement> {
        if !framed || !self.hosts.music_player_bar_presence.read(cx).is_present() {
            return None;
        }
        let gap = theme.shell.rail_outer_inset;
        let bar = AnyView::from(self.hosts.music_player_bar.clone()).cached(StyleRefinement::default().size_full());
        let pane = Self::pane(div().relative().size_full().child(bar), true, Self::pane_radius(theme), gap, theme);
        let band = gpui::px(MusicGlobalPlayerListeners::AVATAR_BAND);
        let listeners = AnyView::from(self.hosts.music_player_bar.read(cx).listeners())
            .cached(StyleRefinement::default().size_full());
        let listeners = div()
            .absolute()
            .left(gap)
            .right(gap)
            .top(-band)
            .h(gpui::px(MusicGlobalPlayerBar::HEIGHT) + band)
            .child(listeners);
        Some(
            div()
                .relative()
                .flex_none()
                .w_full()
                .h(gpui::px(MusicGlobalPlayerBar::HEIGHT) + gap)
                .px(gap)
                .pb(gap)
                .child(pane)
                .child(listeners)
                .into_any_element(),
        )
    }

    fn app_update_overlay(&self, framed: bool, rtl: bool, theme: &rise_theme::AppTheme, cx: &App) -> Div {
        let gap = theme.shell.rail_outer_inset;
        let side = if framed { theme.shell.rail_width + gap * 3.0 + theme.spacing._600 } else { theme.spacing._600 };
        let player = if framed && self.hosts.music_player_bar_presence.read(cx).is_present() {
            gpui::px(MusicGlobalPlayerBar::HEIGHT) + gap
        } else {
            gpui::px(0.0)
        };
        let overlay = div().absolute().bottom(player + gap + theme.spacing._600).child(self.hosts.app_update_pill.clone());
        if rtl { overlay.right(side) } else { overlay.left(side) }
    }

    fn section_bar(&self, cx: &App) -> Option<AnyElement> {
        if !self.shows_section_bar(cx) {
            return None;
        }
        if self.columns_fresh {
            return Some(self.section_rail.clone().into_any_element());
        }
        let style = RootSectionRail::cached_style(rise_ui::theme(cx));
        Some(self.section_rail.clone().cached(style).into_any_element())
    }

    fn dirty_columns(&self, cx: &mut Context<Self>) {
        App::notify(cx, self.section_rail.entity_id());
        App::notify(cx, self.side_panel.entity_id());
        for page in self.stack.iter().chain(self.departing_page.iter()) {
            App::notify(cx, page.view.entity_id());
        }
    }

    fn side_panel_element(&self, width: Pixels) -> AnyElement {
        if self.columns_fresh {
            return self.side_panel.clone().into_any_element();
        }
        let style = StyleRefinement::default().h_full().flex_none().w(width);
        self.side_panel.clone().cached(style).into_any_element()
    }

    fn palette_overlay(&self, theme: &rise_theme::AppTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let palette = self.palette.clone()?;
        Some(
            div()
                .id("root-command-palette")
                .absolute()
                .inset_0()
                .bg(theme.material.scrim)
                .flex()
                .flex_col()
                .items_center()
                .pt(theme.shell.palette_top_inset)
                .on_click(cx.listener(|view, _: &ClickEvent, window, cx| view.close_command_palette(window, cx)))
                .child(palette)
                .into_any_element(),
        )
    }

    fn z_stack_layers(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        fn layer(child: impl IntoElement) -> AnyElement {
            div().absolute().inset_0().child(child).into_any_element()
        }
        fn host<T: Render>(entity: &Entity<T>) -> AnyElement {
            layer(entity.clone().cached(StyleRefinement::default().size_full()))
        }
        let hosts = &self.hosts;
        let is_floating = self.video_note_is_floating;
        let interruptions_can_present = self.interruptions_can_present(cx);

        let mut layers = vec![host(&hosts.chat_gif_search_portal), host(&hosts.now_playing_host)];
        if is_floating {
            layers.push(layer(self.video_note_pip.clone().cached(StyleRefinement::default().size_full())));
        }
        layers.push(host(&self.location_presentation));
        layers.push(host(&hosts.premium_badge_picker_popover_host));
        layers.push(host(&hosts.blocking_loader_host));
        layers.push(host(&hosts.subscription_success_host));
        if interruptions_can_present {
            layers.push(host(&hosts.premium_gift_celebration_host));
            layers.push(host(&hosts.achievement_earned_host));
        }
        layers.extend([
            host(&hosts.join_chat_sheet_host),
            host(&hosts.map_invite_sheet_host),
            host(&hosts.map_onboarding_sheet_host),
            host(&hosts.playlist_invite_sheet_host),
            host(&hosts.pack_invite_sheet_host),
            host(&hosts.music_listening_session_sheet_layer),
            host(&hosts.music_player_auxiliary_sheets_layer),
            host(&hosts.chat_music_player_sheet_layer),
            host(&hosts.select_media_sheet_layer),
            host(&hosts.chat_folder_edit_sheet_layer),
            host(&hosts.web_login_approval_sheet),
            host(&hosts.comments_sheet_host),
            layer(RiseQRCodeModalHost::new()),
            host(&hosts.post_likes_sheet_host),
            host(&hosts.poll_voters_sheet_host),
            layer(CommentPhotoPickerHost::new()),
            host(&hosts.map_photo_picker_host),
            host(&hosts.shorts_more_sheet_host),
            host(&hosts.sticker_pack_sheet),
            host(&hosts.post_collaborators_host),
            host(&hosts.profile_highlight_sheets_host),
            host(&hosts.sticker_asset_composer_host),
            host(&hosts.people_picker_host),
            host(&hosts.message_music_import_sheet_host),
            host(&hosts.sticker_hold_overlay),
            host(&hosts.story_player_host),
            host(&hosts.share_sheet),
            host(&hosts.gift_sheets_host),
            host(&hosts.sparks_sheets_host),
            host(&hosts.music_track_download_sheet),
            host(&hosts.chat_moderation_sheet_host),
            host(&hosts.member_rights_sheet_host),
            host(&hosts.report_sheet_host),
            host(&hosts.appeal_sheet_host),
            layer(NotificationsHubSheetHost::new()),
            layer(RepostOpinionEditor::new("root-repost-opinion-editor")),
            host(&hosts.bot_mini_app_presentation_host),
            host(&hosts.in_app_browser_host),
            layer(StoryShareHandoffHost::new()),
            host(&hosts.share_send_handoff_host),
            host(&hosts.premium_gift_host),
            host(&hosts.premium_promo_host),
            host(&hosts.theme_preview_host),
            host(&hosts.premium_upsell_host),
            // why: the purchase modal renders with the root every frame, a cached replay of its deferred layer drew it without text
            layer(hosts.purchase_continue_host.clone()),
            host(&hosts.bug_report_sheet_host),
            host(&hosts.media_viewer_inline_host),
            host(&hosts.media_viewer_modal_host_bridge),
            host(&hosts.music_listening_interruption_alert_host),
        ]);
        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        layers.push(host(&hosts.sai_debugger_host));
        if interruptions_can_present {
            layers.push(layer(AnnouncementHost::new()));
            layers.push(layer(ReleaseNotesHost::new()));
        }
        layers.push(host(&hosts.maintenance_host));
        layers.push(host(&hosts.rise_rtc_room_presentation_host));
        layers.push(deferred(host(&hosts.notifier_host)).with_priority(NotifierHost::LAYER_PRIORITY).into_any_element());
        layers.push(host(&hosts.restriction_host));
        layers
    }

    fn contact_sync_onboarding_sheet(&self) -> NativeBottomSheetPresentationHost {
        let reader = self.user_services.clone();
        let writer = self.user_services.clone();
        NativeBottomSheetPresentationHost::new(
            "root-contact-sync-onboarding",
            Binding::new(
                move |cx: &App| !RiseLaunchSplashState::is_visible() && reader.read(cx).should_present_contact_sync_onboarding(),
                move |value, _window, cx| {
                    writer.update(cx, |user, cx| {
                        user.should_present_contact_sync_onboarding = value;
                        cx.notify();
                    });
                },
            ),
            |_window, _cx| ContactSyncOnboardingSheet::new(),
        )
        .interactive_dismiss_disabled(true)
    }
}

/// set by the first window on_appear
struct EngagementSessionStarted;

impl Global for EngagementSessionStarted {}

impl Focusable for RootView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_navigation_stack(cx);
        self.sync_side_panel(cx);
        self.sync_notification_permission(cx);

        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }

        let theme = rise_ui::theme(cx as &App).clone();
        let add_account_flow = if self.auth_services.read(cx).is_adding_account() {
            Some(self.add_account_flow.get_or_insert_with(|| cx.new(|_| AddAccountAuthFlowView)).clone())
        } else {
            self.add_account_flow = None;
            None
        };
        let stack = self.navigation_stack(window, cx);
        let section_bar = self.section_bar(cx);
        self.columns_fresh = false;
        let layers = self.z_stack_layers(cx);
        let contact_sync = self.contact_sync_onboarding_sheet();
        let palette_overlay = self.palette_overlay(&theme, cx);

        let framed = self.shows_section_bar(cx);
        let rtl = LayoutDirection::current().is_right_to_left();
        self.title_bar.update(cx, |bar, cx| bar.dock_controls_in_rail(framed, cx));
        let music_bar = self.music_player_row(framed, &theme, cx);
        let columns = div()
            .id(self.flow_identity.element_id())
            .size_full()
            .flex()
            .when(rtl, |flow| flow.flex_row_reverse())
            .when(!rtl, |flow| flow.flex_row())
            .children(section_bar)
            .child(stack)
            .install_keyboard_dismiss_on_tap();
        let columns = div().w_full().flex_1().min_h_0().child(columns);
        let flow = div()
            .size_full()
            .when(framed, |flow| flow.bg(Self::pane_backdrop(&theme)))
            .flex()
            .flex_col()
            .child(columns)
            .children(music_bar);

        let content = div()
            .id(ElementId::Name(self.language_code.clone().into()))
            .relative()
            .size_full()
            .child(flow)
            .children(layers)
            .child(self.app_update_overlay(framed, rtl, &theme, cx))
            .children(add_account_flow.map(|flow| {
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .child(AnyView::from(flow).cached(StyleRefinement::default().size_full()))
            }))
            .child(contact_sync)
            .child(div().absolute().inset_0().child(ContactPhotoSuggestionSheet::new()))
            .child(
                AnyView::from(self.hosts.desktop_share_layer.clone())
                    .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
            )
            .children(palette_overlay);

        let environment = env_scope::<NavigationStoreKey>(Some(self.nav_store.clone())).child(
            env_scope::<MediaViewerPresentationKey>(Some(self.media_viewer_presentation.clone())).child(
                env_scope::<LocationPresentationKey>(Some(self.location_presentation.clone())).child(
                    env_scope::<RepostTransitionNamespaceKey>(Some(self.repost_opinion_namespace.clone()))
                        .child(
                            env_scope::<ShortsZoomNamespaceKey>(Some(self.shorts_zoom_namespace))
                                .child(env_scope::<OpenURLKey>(self.open_url_action.clone()).child(content)),
                        ),
                ),
            ),
        );

        let plate = BlockUi::window(&theme)
            .font_family(FONT_FAMILY)
            .relative()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(|view, _: &SelectSlot1, window, cx| view.select_slot(1, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot2, window, cx| view.select_slot(2, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot3, window, cx| view.select_slot(3, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot4, window, cx| view.select_slot(4, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot5, window, cx| view.select_slot(5, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot6, window, cx| view.select_slot(6, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot7, window, cx| view.select_slot(7, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot8, window, cx| view.select_slot(8, window, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot9, window, cx| view.select_slot(9, window, cx)))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::close_window))
            .on_action(SidePanelUi::toggle_listener(self.side_panel.clone()))
            .capture_key_down(cx.listener(Self::capture_escape_for_viewer))
            .on_key_down(cx.listener(Self::handle_escape_key))
            .on_key_down(cx.listener(Self::handle_orphan_refresh_key))
            .on_key_down(cx.listener(Self::handle_orphan_typing_key))
            .on_key_down(cx.listener(Self::handle_orphan_viewer_arrow))
            .child(environment)
            .child(
                AnyView::from(self.title_bar.clone())
                    .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
            )
            .child(glass_host())
            .child(MediaEditorWindowLayer)
            .child(
                deferred(
                    AnyView::from(self.hosts.hold_context_menu_host.clone())
                        .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
                )
                .with_priority(HoldContextMenuHost::LAYER_PRIORITY),
            )
            .child(ChatViewLimitedPrivacyCoverOverlay::new())
            .child(GuideOverlayWindow::new());

        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        let plate = plate.on_action(cx.listener(Self::toggle_sai_debugger));

        let plate = Self::window_input(cx).attach(plate);
        let framed = window_frame(plate, window, cx);
        RiseonlyApp::body(framed, cx)
    }
}

#[cfg(test)]
mod navigation_bridge_tests {
    use gpui::{TestAppContext, WindowHandle};
    use rise_theme::AppTheme;

    use super::*;
    use crate::app::shell::window_presenter::WindowPresenter;

    fn open_shell(cx: &mut TestAppContext) -> WindowHandle<RootView> {
        cx.update(|cx| WindowPresenter::open_handle(cx, RootView::new)).expect("the shell window opens")
    }

    fn activate(shell: WindowHandle<RootView>, cx: &mut TestAppContext) {
        cx.update(|cx| shell.update(cx, |_, window, _| window.activate_window())).expect("the window is open");
        cx.run_until_parked();
    }

    fn screen_of(shell: WindowHandle<RootView>, cx: &mut TestAppContext) -> String {
        cx.update(|cx| shell.update(cx, |view, _, cx| view.nav_store.read(cx).sai_active_screen().id))
            .expect("the window is open")
    }

    fn bridge_screen(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| SaiNavigationBridge::current_screen(cx).map(|screen| screen.id))
    }

    #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
    #[gpui::test]
    fn the_debugger_shortcut_brings_the_orb_back_and_toggles_the_sheet(cx: &mut TestAppContext) {
        use crate::core::lib::sai::modules::sai_debugger::sai_debugger_overlay_state::SaiDebuggerOverlayState;

        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
            shell_actions::install_key_bindings(cx);
        });
        let shell = open_shell(cx);
        let overlay = cx.update(SaiDebuggerOverlayState::shared);
        assert!(cx.update(|cx| overlay.read(cx).is_orb_visible()));
        cx.update(|cx| overlay.update(cx, |state, cx| state.dismiss_orb(cx)));
        cx.run_until_parked();

        let press = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                shell.update(cx, |view, window, cx| {
                    window.focus(&view.focus_handle, cx);
                    window.dispatch_action(Box::new(ToggleSaiDebugger), cx);
                })
            })
            .expect("the window is open");
            cx.run_until_parked();
        };
        let sheet_is_open = |cx: &mut TestAppContext| {
            cx.update(|cx| shell.update(cx, |view, _, cx| view.hosts.sai_debugger_host.read(cx).is_sheet_open()))
                .expect("the window is open")
        };

        press(cx);
        assert!(sheet_is_open(cx));
        assert!(cx.update(|cx| overlay.read(cx).is_orb_visible()));

        press(cx);
        assert!(!sheet_is_open(cx));
        assert!(cx.update(|cx| overlay.read(cx).is_orb_visible()));
    }

    #[gpui::test]
    fn the_bridge_follows_the_front_window_and_leaves_a_closed_one(cx: &mut TestAppContext) {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });
        let first = open_shell(cx);
        let second = open_shell(cx);
        cx.update(|cx| {
            first.update(cx, |view, _, cx| view.nav_store.update(cx, |nav, cx| nav.switch_tab(MainTab::Chats, cx)))
        })
        .expect("the window is open");
        cx.run_until_parked();
        assert_ne!(screen_of(first, cx), screen_of(second, cx));

        activate(first, cx);
        assert_eq!(bridge_screen(cx), Some(screen_of(first, cx)));
        activate(second, cx);
        assert_eq!(bridge_screen(cx), Some(screen_of(second, cx)));

        cx.update(|cx| second.update(cx, |_, window, _| window.remove_window())).expect("the window is open");
        cx.run_until_parked();
        assert_eq!(bridge_screen(cx), Some(screen_of(first, cx)));
    }

    fn tab_of(shell: WindowHandle<RootView>, cx: &mut TestAppContext) -> MainTab {
        cx.update(|cx| shell.update(cx, |view, _, cx| view.nav_store.read(cx).current_tab)).expect("the window is open")
    }

    fn hold_guide(cx: &mut TestAppContext) {
        cx.update(|cx| {
            GuideRunner::shared(cx).update(cx, |runner, _| runner.hold_guide_for_bridge_test("bridge-test"));
        });
    }

    /// ends the guide and reads its bridge tab in one update
    /// so a replacing guide snapshots before any later turn
    fn end_guide_and_read_its_bridge_tab(cx: &mut TestAppContext) -> Option<MainTab> {
        cx.update(|cx| {
            GuideRunner::shared(cx).update(cx, |runner, cx| runner.end_held_guide_for_bridge_test(cx));
            GuideNavigationBridge::snapshot(cx).map(|snapshot| snapshot.tab)
        })
    }

    #[gpui::test]
    fn the_guide_bridge_keeps_the_guides_window_and_moves_to_the_front_one_when_it_ends(cx: &mut TestAppContext) {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });
        let first = open_shell(cx);
        let second = open_shell(cx);
        cx.update(|cx| {
            first.update(cx, |view, _, cx| view.nav_store.update(cx, |nav, cx| nav.switch_tab(MainTab::Chats, cx)))
        })
        .expect("the window is open");
        cx.run_until_parked();
        let first_tab = tab_of(first, cx);
        let second_tab = tab_of(second, cx);
        assert_ne!(first_tab, second_tab);

        activate(first, cx);
        hold_guide(cx);
        activate(second, cx);
        let running = cx.update(|cx| GuideNavigationBridge::snapshot(cx).map(|snapshot| snapshot.tab));
        assert_eq!(running, Some(first_tab));
        assert_eq!(end_guide_and_read_its_bridge_tab(cx), Some(second_tab));

        activate(first, cx);
        hold_guide(cx);
        cx.update(|cx| first.update(cx, |_, window, _| window.remove_window())).expect("the window is open");
        cx.run_until_parked();
        assert_eq!(end_guide_and_read_its_bridge_tab(cx), Some(second_tab));
    }

    fn palette_is_open(shell: WindowHandle<RootView>, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| shell.update(cx, |view, _, _| view.palette.is_some())).expect("the window is open")
    }

    #[gpui::test]
    fn escape_on_the_plate_closes_the_palette_and_a_modified_one_does_not(cx: &mut TestAppContext) {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });
        let shell = open_shell(cx);
        cx.update(|cx| {
            shell.update(cx, |view, window, cx| {
                let commands = catalogue(cx);
                view.palette = Some(cx.new(|cx| CommandPalette::new(commands, cx)));
                window.focus(&view.focus_handle, cx);
                cx.notify();
            })
        })
        .expect("the window is open");
        cx.run_until_parked();

        cx.simulate_keystrokes(shell.into(), "shift-escape");
        cx.run_until_parked();
        assert!(palette_is_open(shell, cx));

        cx.simulate_keystrokes(shell.into(), "escape");
        cx.run_until_parked();
        assert!(!palette_is_open(shell, cx));
    }
}

#[cfg(test)]
mod window_input_tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use gpui::{FileDropEvent, Modifiers, TestAppContext, WindowHandle, point, px};
    use rise_theme::AppTheme;

    use super::*;
    use crate::app::shell::window_presenter::WindowPresenter;

    struct BlankPage;

    impl Render for BlankPage {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    struct PushedPagePlate {
        page: RootStackPage,
        window_input: RootWindowInput,
    }

    impl Render for PushedPagePlate {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.window_input
                .attach(div().size_full())
                .child(RootView::stack_page_element(&self.page, true, 0.0, 1.0, &self.window_input, false))
        }
    }

    #[gpui::test]
    fn a_pushed_page_keeps_the_back_button_and_the_file_drop_once_each(cx: &mut TestAppContext) {
        let backs = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        let window_input = RootWindowInput {
            on_drop: {
                let drops = drops.clone();
                Rc::new(move |_: &ExternalPaths, _: &mut Window, _: &mut App| drops.set(drops.get() + 1))
            },
            on_back: {
                let backs = backs.clone();
                Rc::new(move |_: &MouseDownEvent, _: &mut Window, _: &mut App| backs.set(backs.get() + 1))
            },
        };
        let (_plate, cx) = cx.add_window_view(|_, cx| PushedPagePlate {
            page: RootStackPage {
                route: RootRoute::MyProfile,
                host: SharedElementStackHost { id: 1 },
                view: cx.new(|_| BlankPage).into(),
                backdrop: PageColumnBackdrop::default(),
                owes_fresh: Cell::new(false),
                settled_width: Rc::new(Cell::new(None)),
            },
            window_input,
        });
        cx.run_until_parked();
        let position = point(px(10.0), px(10.0));

        cx.simulate_mouse_down(position, MouseButton::Navigate(NavigationDirection::Back), Modifiers::none());
        assert_eq!(backs.get(), 1);

        cx.simulate_event(FileDropEvent::Entered {
            position,
            paths: ExternalPaths([PathBuf::from("dropped.txt")].into_iter().collect()),
        });
        cx.simulate_event(FileDropEvent::Submit { position });
        assert_eq!(drops.get(), 1);
    }

    #[gpui::test]
    fn a_new_flow_identity_shows_its_path_without_the_push_slide(cx: &mut TestAppContext) {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });
        let shell: WindowHandle<RootView> =
            cx.update(|cx| WindowPresenter::open_handle(cx, RootView::new)).expect("the shell window opens");
        cx.update(|cx| {
            shell.update(cx, |view, _, cx| {
                view.flow_identity.is_authenticated = !view.flow_identity.is_authenticated;
                view.flow_identity_did_change_if_needed(cx);
                view.page_motion_is_push = false;
                view.nav_store.update(cx, |nav, cx| {
                    nav.push(RootRoute::MyProfile, NavigationStoreSettlingPush::default(), cx);
                });
            })
        })
        .expect("the window is open");
        cx.update(|cx| cx.update_window(shell.into(), |_, window, cx| window.draw(cx).clear(cx)))
            .expect("the window is open");
        cx.update(|cx| {
            shell.update(cx, |view, _, _| {
                assert_eq!(view.stack.len(), 1);
                assert!(!view.page_motion_is_push);
                assert!(view.departing_page.is_none());
            })
        })
        .expect("the window is open");
    }
}

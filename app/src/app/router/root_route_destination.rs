use gpui::{
    AnyElement, AnyView, App, AppContext, Context, FontWeight, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, StyleRefinement, Styled, Window, div, px,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::swiftui::environment::env;
use rise_ui::bg_wrapper_ui::BgWrapperUi;
use rise_ui::{MainText, SecondaryText, TextAlignT};

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::app::router::navigation_types::{CreatePostIntent, RootRoute};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::bot::shared::bot_ui_validation::{BotMiniAppInitData, BotMiniAppRouteSource};
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::ChatSettingsRightsGuardModifiers;
use crate::modules::user::pages::profile::settings::achievements::achievements::Achievements;
use crate::modules::user::pages::profile_more::profile_more::ProfileMore;
use crate::modules::moderation::pages::all_moderators::AllModerators;
use crate::modules::moderation::pages::appeal_review::AppealReview;
use crate::modules::moderation::pages::appeals_review::AppealsReview;
use crate::modules::chat::pages::chats::archived_chats::ArchivedChatsScreen;
use crate::modules::music::pages::artist_profile::artist_profile::ArtistProfile;
use crate::modules::music::pages::now_playing::now_playing::NowPlayingAside;
use crate::modules::music::pages::settings::artist_studio::ArtistStudio;
use crate::modules::music::pages::audio_page::audio_page::AudioPage;
use crate::modules::music::pages::settings::become_artist_settings::BecomeArtistSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::bg_color_settings::bg_color_settings::BgColorSettings;
use crate::modules::user::pages::profile::settings::privacy_settings::blocked_users_screen::BlockedUsersScreen;
use crate::modules::user::pages::profile::settings::customization_settings::settings::border_color_settings::border_color_settings::BorderColorSettings;
use crate::modules::bot::pages::bot_commands::BotCommands;
use crate::modules::bot::pages::bot_mini_app_editor::BotMiniAppEditor;
use crate::modules::bot::pages::bot_mini_app_screen::BotMiniAppLinkResolverScreen;
use crate::modules::bot::pages::bot_mini_app_screen::BotMiniAppScreen;
use crate::modules::bot::pages::bot_mini_apps::BotMiniApps;
use crate::modules::bot::pages::bot_settings::BotSettings;
use crate::modules::bot::pages::bot_start_link_screen::BotStartLinkScreen;
use crate::modules::bot::pages::bot_webhook::BotWebhook;
use crate::modules::user::pages::profile::settings::customization_settings::settings::btn_color_settings::btn_color_settings::BtnColorSettings;
use crate::modules::user::pages::profile::settings::memory_settings::pages::cached_chats::cached_chats::CachedChats;
use crate::modules::user::pages::profile::settings::memory_settings::pages::cached_datas::cached_datas::CachedDatas;
use crate::modules::user::pages::profile::settings::memory_settings::pages::cached_media::cached_media::CachedMedia;
use crate::modules::moderation::pages::case_context_page::CaseContextPage;
use crate::modules::moderation::pages::case_notes_page::CaseNotesPage;
use crate::modules::chat::components::chat_item::chat_context_preview::ChatContextPreviewDestination;
use crate::modules::chat::pages::chat_folders::chat_folders_settings::ChatFoldersSettings;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile::ChatProfile;
use crate::modules::chat::pages::chats::chat::chat_roles::chat_role_edit_screen::ChatRoleEditScreen;
use crate::modules::chat::pages::chats::chat::chat_roles::chat_roles_screen::ChatRolesScreen;
use crate::modules::chat::pages::chats::chat::chat_screen::chat_screen::ChatScreen;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings::ChatSettings;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_admins::chat_settings_admins::ChatSettingsAdmins;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_allowed_reactions::chat_settings_allowed_reactions::ChatSettingsAllowedReactions;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_blacklist::chat_settings_blacklist::ChatSettingsBlacklist;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_linked_chat::chat_settings_linked_chat::ChatSettingsLinkedChat;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_links::chat_settings_links::ChatSettingsLinks;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_members::chat_settings_members::ChatSettingsMembers;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_permissions::chat_settings_permissions::ChatSettingsPermissions;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_recent_actions::chat_settings_recent_actions::ChatSettingsRecentActions;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_type::chat_settings_type::ChatSettingsType;
use crate::modules::chat::pages::chats::chat::chat_statistics::chat_statistics::ChatStatistics;
use crate::modules::chat::pages::chats::chat::chat_topics::chat_topics_screen::ChatTopicsScreen;
use crate::modules::user::pages::profile::settings::customization_settings::settings::chat_wallpapers_settings::chat_wallpapers_settings::ChatWallpapersSettings;
use crate::modules::story::pages::close_friends::close_friends_screen::CloseFriendsScreen;
use crate::modules::user::pages::profile::create_post::create_post::CreatePost;
use crate::modules::user::pages::profile::create_post::create_post_media_page::CreatePostMediaPage;
use crate::modules::vacancy::pages::create_resume::create_resume::CreateResume;
use crate::modules::vacancy::pages::create_vacancy::create_vacancy::CreateVacancy;
use crate::modules::user::pages::profile::settings::customization_settings::customization_settings::CustomizationSettings;
use crate::modules::vacancy::pages::create_vacancy::edit_vacancy_route::EditVacancyRoute;
use crate::modules::user::pages::profile::settings::customization_settings::settings::energy_saving_settings::energy_saving_settings::EnergySavingSettings;
use crate::modules::chat::pages::chats::chat::forum_home::forum_home_screen::ForumHomeScreen;
use crate::modules::search::pages::global_search::global_search::GlobalSearch;
use crate::modules::user::pages::goals::goal_detail::GoalDetail;
use crate::modules::guides::pages::guides::guides_catalogue::GuidesCatalogue;
use crate::modules::guides::pages::guides::guides_catalogue::GuidesCategoryPage;
use crate::modules::user::pages::profile::settings::language_settings::language_settings::LanguageSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::launch_animation_settings::launch_animation_settings::LaunchAnimationSettings;
use crate::modules::music::pages::settings::local_music_tracks::LocalMusicTracks;
use crate::modules::map::pages::map_friends::map_friends_page::MapFriendsPage;
use crate::modules::map::pages::map_friends::map_profile_page::MapProfilePage;
use crate::modules::map::pages::world_map::map_journey_page::MapJourneyComposerPage;
use crate::modules::map::pages::world_map::map_journey_page::MapJourneyPage;
use crate::modules::map::pages::map_settings::map_settings_page::MapSettingsPage;
use crate::modules::map::pages::map_settings::map_settings_page::MapSettingsSectionPage;
use crate::modules::user::pages::profile::settings::memory_settings::memory_settings::MemorySettings;
use crate::modules::user::pages::profile::settings::about_project::about_project::{AboutProject, AboutProjectFAQ};
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionCompletion;
use crate::modules::chat::stores::message::message_interactions::MessageInteractionsStore;
use crate::modules::chat::pages::chats::message_requests::MessageRequestsScreen;
use crate::modules::user::pages::profile::settings::moderation_settings::moderation_settings::ModerationSettings;
use crate::modules::moderation::pages::moderator_detail::ModeratorDetail;
use crate::modules::music::pages::settings::moderation::music_album_request_detail::MusicAlbumRequestDetail;
use crate::modules::music::pages::settings::moderation::music_album_requests_list::MusicAlbumRequestsList;
use crate::modules::music::pages::settings::moderation::music_artist_request_detail::MusicArtistRequestDetail;
use crate::modules::music::pages::settings::moderation::music_artist_requests_list::MusicArtistRequestsList;
use crate::modules::music::pages::music_library::music_library::MusicLibraryScreen;
use crate::modules::user::pages::profile::settings::notification_settings::music_notification_settings::MusicNotificationSettings;
use crate::modules::music::pages::settings::music_settings::MusicSettings;
use crate::modules::music::pages::music_tab::music_tab::MusicTab;
use crate::modules::music::pages::settings::moderation::music_track_request_detail::MusicTrackRequestDetail;
use crate::modules::music::pages::settings::moderation::music_track_requests_list::MusicTrackRequestsList;
use crate::modules::user::pages::profile::settings::moderation_settings::my_blocks::MyBlocks;
use crate::modules::bot::pages::my_bots::MyBots;
use crate::modules::diagnostics::pages::my_bug_reports::my_bug_reports::MyBugReports;
use crate::modules::music::pages::settings::moderation::my_music_album_requests::MyMusicAlbumRequests;
use crate::modules::music::pages::settings::moderation::my_music_artist_request::MyMusicArtistRequest;
use crate::modules::music::pages::settings::moderation::my_music_track_requests::MyMusicTrackRequests;
use crate::modules::user::pages::profile::settings::moderation_settings::my_reports::MyReports;
use crate::modules::vacancy::pages::my_resume::my_resume::MyResume;
use crate::modules::sparks::pages::my_sparks::my_sparks_page::MySparksPage;
use crate::modules::user::pages::profile::settings::sticker_settings::my_sticker_packs::MyStickerPacks;
use crate::modules::vacancy::pages::my_vacancies::my_vacancies::MyVacancies;
use crate::modules::user::pages::profile::settings::region_settings::notification_language_settings::NotificationLanguageSettings;
use crate::modules::user::pages::profile::settings::notification_settings::notification_settings::NotificationSettings;
use crate::modules::notify::pages::notifications::notifications::Notifications;
use crate::modules::chat::pages::chats::pinned_messages::pinned_messages::PinnedMessages;
use crate::modules::user::pages::plans::plan_detail::PlanDetail;
use crate::modules::music::pages::playlist_detail::playlist_deep_link_screen::PlaylistDeepLinkScreen;
use crate::modules::music::pages::playlist_detail::playlist_detail::PlaylistDetail;
use crate::modules::music::pages::playlist_collaboration::playlist_invite_link_screen::PlaylistInviteLinkScreen;
use crate::modules::music::pages::playlist_collaboration::playlist_join_requests_screen::PlaylistJoinRequestsScreen;
use crate::modules::music::pages::playlist_collaboration::playlist_member_rights_screen::PlaylistMemberRightsScreen;
use crate::modules::music::pages::playlist_collaboration::playlist_members_screen::PlaylistMembersScreen;
use crate::modules::music::pages::playlist_collaboration::playlist_settings_screen::PlaylistSettingsScreen;
use crate::modules::music::pages::playlist_detail::playlist_track_deep_link_screen::PlaylistTrackDeepLinkScreen;
use crate::modules::post::pages::posts::post_detail::post_detail::PostDetail;
use crate::modules::post::pages::post_stats::post_stats::PostStats;
use crate::modules::post::pages::post_view_history::post_view_history::PostViewHistory;
use crate::modules::post::pages::posts::posts_interesting::posts_interesting::PostsInteresting;
use crate::modules::user::pages::profile::settings::premium_badge_settings::premium_badge_settings::PremiumBadgeSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::primary_color_settings::primary_color_settings::PrimaryColorSettings;
use crate::modules::user::pages::profile::settings::privacy_settings::privacy_exceptions_screen::PrivacyExceptionsScreen;
use crate::modules::user::pages::profile::settings::privacy_settings::privacy_setting::PrivacySettingScreen;
use crate::modules::user::pages::profile::settings::privacy_settings::privacy_settings::PrivacySettings;
use crate::modules::user::pages::profile::profile::Profile;
use crate::modules::user::pages::profile::settings::profile_settings::profile_settings::ProfileSettings;
use crate::modules::user::pages::profile::settings::quick_reaction_settings::quick_reaction_settings::QuickReactionSettings;
use crate::modules::user::pages::profile::settings::region_settings::region_settings::RegionSettings;
use crate::modules::moderation::pages::report_detail::ReportDetail;
use crate::modules::moderation::pages::reports_queue::ReportsQueue;
use crate::core::ui::qr_code::rise_qr_code_page::RiseQRCodePage;
use crate::core::ui::qr_code::rise_qr_scanner_page::RiseQRScannerPage;
use crate::modules::rtc::media::rise_rtc_audio_settings::{RiseRTCAudioSettingsScreen, RiseRTCStreamSettingsScreen};
use crate::modules::rtc::pages::rise_rtc_group_voice_settings_screen::RiseRTCGroupVoiceSettingsScreen;
use crate::modules::rtc::pages::rise_rtc_room_screen::RiseRTCRoomScreen;
use crate::modules::user::pages::profile::settings::customization_settings::settings::secondary_text_color_settings::secondary_text_color_settings::SecondaryTextColorSettings;
use crate::modules::user::pages::profile::settings::sessions_settings::sessions_settings::SessionsSettings;
use crate::modules::user::pages::profile::settings::settings::Settings;
use crate::modules::shorts::pages::short_viewer::short_viewer_page::ShortViewerPage;
use crate::modules::user::pages::profile::settings::social_links::social_links_editor::SocialLinksEditor;
use crate::modules::user::pages::profile::settings::sticker_settings::sticker_settings::StickerSettings;
use crate::modules::story::pages::stories_archive::stories_archive::StoriesArchive;
use crate::modules::story::pages::story_edit::story_edit_page::StoryEditPage;
use crate::modules::music::pages::settings::studio_album_detail::StudioAlbumDetail;
use crate::modules::music::pages::settings::studio_track_detail::StudioTrackDetail;
use crate::modules::user::pages::profile::settings::subscription_settings::subscription_settings::SubscriptionSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::tab_bar_settings::tab_bar_settings::TabBarSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::text_color_settings::text_color_settings::TextColorSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::text_size_settings::text_size_settings::TextSizeSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::theme_settings::theme_settings::ThemeSettings;
use crate::modules::user::pages::profile::settings::customization_settings::themes_list_settings::ThemesListSettings;
use crate::modules::user::pages::profile::settings::customization_settings::settings::block_color_settings::block_color_settings::BlockColorSettings;
use crate::modules::stickers::pages::pack_collaboration::pack_coauthor_rights_screen::PackCoauthorRightsScreen;
use crate::modules::stickers::pages::pack_collaboration::pack_coauthors_screen::PackCoauthorsScreen;
use crate::modules::stickers::pages::pack_collaboration::pack_invite_link_joiners_screen::PackInviteLinkJoinersScreen;
use crate::modules::stickers::pages::pack_collaboration::pack_invite_links_screen::PackInviteLinksScreen;
use crate::modules::stickers::pages::pack_collaboration::pack_join_requests_screen::PackJoinRequestsScreen;
use crate::core::lib::rise_feature_gate::RiseFeatureGate;
use crate::core::widgets::wrappers::list_shell_ui::ListShellUi;
use crate::modules::music::pages::playlist_detail::track_deep_link_screen::TrackDeepLinkScreen;
use crate::modules::user::pages::profile::info::friends::user_friends::UserFriends;
use crate::modules::user::pages::profile::info::subs::user_subs::UserSubs;
use crate::modules::user::pages::profile::info::subscribers::user_subscribers::UserSubscribers;
use crate::modules::vacancy::pages::vacancy_detail::vacancy_detail::VacancyDetail;
use crate::modules::vacancy::pages::vacancy_list::vacancy_list::VacancyList;
use crate::modules::vacancy::pages::settings::vacancy_settings::VacancySettings;
use crate::modules::map::pages::world_map::world_map_page::WorldMapPage;
use crate::modules::user::pages::profile::settings::customization_settings::settings::chat_wallpapers_settings::chat_wallpapers_settings::ChatWallpapersScope;

/// screen of one route, what the navigation stack shows for each path entry
/// root view keeps it as long as the route stays in a stack
pub struct RootRouteDestination {
    route: RootRoute,
    screen: Option<AnyView>,
}

impl RootRouteDestination {
    pub fn new(route: RootRoute, _cx: &mut Context<Self>) -> Self {
        Self::build(route)
    }

    pub fn in_aside(route: RootRoute, _cx: &mut Context<Self>) -> Self {
        Self::build(route)
    }

    fn build(route: RootRoute) -> Self {
        Self { route, screen: None }
    }
}

impl Render for RootRouteDestination {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let navigation_store = env::<NavigationStoreKey>(cx);
        let navigation_epoch = navigation_store
            .as_ref()
            .and_then(|store| store.read(cx).chat_presentation_epoch_for(&self.route));
        let content = route_content(&self.route, &mut self.screen, navigation_epoch, window, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(div().flex().flex_col().flex_1().min_h_0().w_full().child(content))
    }
}

fn held<V: Render>(
    screen: &mut Option<AnyView>,
    cx: &mut Context<RootRouteDestination>,
    build: impl FnOnce(&mut Context<V>) -> V,
) -> AnyElement {
    screen.get_or_insert_with(|| cx.new(build).into()).clone().into_any_element()
}

/// held view for a page that reads nothing from the route, its frame is reused until it notifies
fn held_cached<V: Render>(
    screen: &mut Option<AnyView>,
    cx: &mut Context<RootRouteDestination>,
    build: impl FnOnce(&mut Context<V>) -> V,
) -> AnyElement {
    screen
        .get_or_insert_with(|| cx.new(build).into())
        .clone()
        .cached(StyleRefinement::default().size_full())
        .replay_when_moved()
        .into_any_element()
}

struct StatelessRouteScreen {
    build: Box<dyn Fn() -> AnyElement>,
}

impl Render for StatelessRouteScreen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        (self.build)()
    }
}

fn held_stateless(
    screen: &mut Option<AnyView>,
    cx: &mut Context<RootRouteDestination>,
    build: impl Fn() -> AnyElement + 'static,
) -> AnyElement {
    held_cached(screen, cx, move |_| StatelessRouteScreen { build: Box::new(build) })
}

fn rights_guarded(content: impl IntoElement, route: &RootRoute, chat_id: &Option<String>) -> AnyElement {
    content.chat_settings_rights_guard(route.clone(), chat_id.clone()).into_any_element()
}

fn route_content(
    route: &RootRoute,
    screen: &mut Option<AnyView>,
    navigation_epoch: Option<Uuid>,
    window: &mut Window,
    cx: &mut Context<RootRouteDestination>,
) -> AnyElement {
    match route {
        RootRoute::PostDetail { post_id, origin } => {
            held(screen, cx, |cx| PostDetail::new(post_id.clone(), origin.clone(), cx))
        }
        RootRoute::ShortViewer { post, origin, sound_track_id } => {
            ShortViewerPage::new(post.clone(), origin.clone(), sound_track_id.clone()).into_any_element()
        }
        RootRoute::PostsInteresting { selected_post } => {
            PostsInteresting::new(selected_post.clone()).into_any_element()
        }
        RootRoute::Chat { chat_id } => {
            ChatContextPreviewDestination::new(chat_id.clone(), false, navigation_epoch).into_any_element()
        }
        RootRoute::MessageRequests => MessageRequestsScreen::new().into_any_element(),
        RootRoute::ArchivedChats => ArchivedChatsScreen::new().into_any_element(),
        RootRoute::ChatTopic { chat_id, topic_id } => held_cached(screen, cx, |cx| {
            ChatScreen::new(chat_id.clone(), topic_id.clone(), false, None, navigation_epoch, None, cx)
        }),
        RootRoute::RtcRoom { target } => RiseRTCRoomScreen::new(target.clone(), false).into_any_element(),
        RootRoute::ChatThread { chat_id, root_id } => held_cached(screen, cx, |cx| {
            ChatScreen::new(chat_id.clone(), None, false, root_id.clone(), navigation_epoch, None, cx)
        }),
        RootRoute::ChatProfile { chat_id } => ChatProfile::new("chat-profile", chat_id.clone()).into_any_element(),
        RootRoute::ChatSettings { chat_id } => {
            let content = held(screen, cx, |cx| ChatSettings::new(chat_id.clone(), cx));
            rights_guarded(content, route, chat_id)
        }
        RootRoute::ChatSettingsType { chat_id } => {
            let content = held(screen, cx, |cx| ChatSettingsType::new(chat_id.clone(), window, cx));
            rights_guarded(content, route, chat_id)
        }
        RootRoute::ChatSettingsLinks { chat_id } => {
            rights_guarded(ChatSettingsLinks::new(chat_id.clone()), route, chat_id)
        }
        RootRoute::ChatSettingsLinkedChat { chat_id } => {
            let content = held(screen, cx, |cx| ChatSettingsLinkedChat::new(chat_id.clone(), cx));
            rights_guarded(content, route, chat_id)
        }
        RootRoute::ChatStatistics { chat_id } => {
            let content = held(screen, cx, |cx| ChatStatistics::new(chat_id.clone(), cx));
            rights_guarded(content, route, chat_id)
        }
        RootRoute::ChatSettingsAllowedReactions { chat_id } => {
            let content = held(screen, cx, |cx| ChatSettingsAllowedReactions::new(chat_id.clone(), cx));
            rights_guarded(content, route, chat_id)
        }
        RootRoute::ChatSettingsMembers { chat_id } => {
            let content = held(screen, cx, |cx| ChatSettingsMembers::new(chat_id.clone(), cx));
            rights_guarded(content, route, chat_id)
        }
        RootRoute::ChatSettingsPermissions { chat_id } => {
            rights_guarded(ChatSettingsPermissions::new(chat_id.clone()), route, chat_id)
        }
        RootRoute::ChatSettingsAdmins { chat_id } => {
            rights_guarded(ChatSettingsAdmins::new(chat_id.clone()), route, chat_id)
        }
        RootRoute::ChatSettingsBlacklist { chat_id } => {
            rights_guarded(ChatSettingsBlacklist::new(chat_id.clone()), route, chat_id)
        }
        RootRoute::ChatSettingsRecentActions { chat_id } => {
            rights_guarded(ChatSettingsRecentActions::new(chat_id.clone()), route, chat_id)
        }
        RootRoute::ChatTopics { chat_id } => held(screen, cx, |cx| ChatTopicsScreen::new(chat_id.clone(), cx)),
        RootRoute::GroupVoiceSettings { chat_id } => {
            held(screen, cx, |cx| RiseRTCGroupVoiceSettingsScreen::new(chat_id.clone(), window, cx))
        }
        RootRoute::ForumHome { chat_id } => held(screen, cx, |cx| ForumHomeScreen::new(chat_id.clone(), cx)),
        RootRoute::ForumAll { chat_id } => {
            ChatContextPreviewDestination::new(chat_id.clone(), true, navigation_epoch).into_any_element()
        }
        RootRoute::GeneralProfile { chat_id } => {
            ChatProfile::new("chat-profile", chat_id.clone()).is_general(true).into_any_element()
        }
        RootRoute::TopicProfile { chat_id, topic_id } => {
            ChatProfile::new("chat-profile", chat_id.clone()).topic_id(topic_id.clone()).into_any_element()
        }
        RootRoute::ChatRoles { chat_id } => ChatRolesScreen::new(chat_id.clone()).into_any_element(),
        RootRoute::ChatRoleEdit { chat_id, role_id } => {
            ChatRoleEditScreen::new(chat_id.clone(), role_id.clone()).into_any_element()
        }
        RootRoute::PinnedMessages { chat_id, topic_id } => {
            PinnedMessages::new(chat_id.clone(), topic_id.clone()).into_any_element()
        }
        RootRoute::Notifications => Notifications::new().shows_back_button(true).into_any_element(),
        RootRoute::UserPage { user_id, tag } => {
            Profile::new("profile").is_user(true).user_id(user_id.clone()).tag(tag.clone()).into_any_element()
        }
        RootRoute::UserSubs { user_id } => UserSubs::new(user_id.clone()).into_any_element(),
        RootRoute::UserSubscribers { user_id } => UserSubscribers::new(user_id.clone()).into_any_element(),
        RootRoute::UserFriends { user_id } => UserFriends::new(user_id.clone()).into_any_element(),
        RootRoute::ProfileMore { user_id, tag } => {
            ProfileMore::new("profile-more", user_id.clone(), tag.clone()).into_any_element()
        }
        RootRoute::CreatePostMedia { intent, track_seed, journey_id } => {
            held(screen, cx, |_| CreatePostMediaPage::new(*intent, track_seed.clone(), journey_id.clone()))
        }
        RootRoute::CreatePost { intent, journey_id } => {
            held(screen, cx, |cx| CreatePost::new(*intent, None, None, journey_id.clone(), window, cx))
        }
        RootRoute::EditPost { post } => held(screen, cx, |cx| {
            CreatePost::new(CreatePostIntent::Publication, Some(post.clone()), None, None, window, cx)
        }),
        RootRoute::EditStory { story_id } => StoryEditPage::new(*story_id).into_any_element(),
        RootRoute::StoriesArchive => StoriesArchive::new().into_any_element(),
        RootRoute::CloseFriends => CloseFriendsScreen::new().into_any_element(),
        RootRoute::Settings => Settings::new().into_any_element(),
        RootRoute::MyBots => MyBots::new().into_any_element(),
        RootRoute::BotSettings { bot_id } => held(screen, cx, |cx| BotSettings::new(bot_id.clone(), cx)),
        RootRoute::BotCommands { bot_id } => BotCommands::new(bot_id.clone()).into_any_element(),
        RootRoute::BotWebhook { bot_id } => BotWebhook::new(bot_id.clone()).into_any_element(),
        RootRoute::BotMiniApps { bot_id } => held(screen, cx, |cx| BotMiniApps::new(bot_id.clone(), cx)),
        RootRoute::BotMiniAppEditor { bot_id, mini_app_id } => {
            BotMiniAppEditor::new(bot_id.clone(), mini_app_id.clone()).into_any_element()
        }
        RootRoute::BotMiniApp { url, title, allows_development_http, chat_id, .. } => {
            let launch_url = url.clone();
            let chat_id = chat_id.clone();
            BotMiniAppScreen::new(BotMiniAppRouteSource::LaunchUrl(url.clone()), title.clone(), *allows_development_http)
                .on_send_data(move |data, _window, cx| {
                    send_mini_app_data(chat_id.as_deref(), &data, &BotMiniAppInitData::extract(&launch_url), cx);
                })
                .into_any_element()
        }
        RootRoute::BotMiniAppLink { tag, slug, start_param } => {
            BotMiniAppLinkResolverScreen::new(tag.clone(), slug.clone(), start_param.clone(), None).into_any_element()
        }
        RootRoute::BotStartLink { tag, start_param } => {
            held(screen, cx, |cx| BotStartLinkScreen::new(tag.clone(), start_param.clone(), cx))
        }
        RootRoute::ProfileSettings => ProfileSettings::new().into_any_element(),
        RootRoute::Achievements { user_id } => Achievements::new(user_id.clone()).into_any_element(),
        RootRoute::RtcAudioSettings => held(screen, cx, RiseRTCAudioSettingsScreen::new),
        RootRoute::RtcStreamSettings => held(screen, cx, RiseRTCStreamSettingsScreen::new),
        RootRoute::QuickReactionSettings => QuickReactionSettings::new().into_any_element(),
        RootRoute::PremiumBadgeSettings => PremiumBadgeSettings::new().into_any_element(),
        RootRoute::SocialLinksEditor => SocialLinksEditor::new().into_any_element(),
        RootRoute::PrivacySettings => held_stateless(screen, cx, || PrivacySettings::new().into_any_element()),
        RootRoute::PrivacySetting { r#type } => PrivacySettingScreen::new(r#type.clone()).into_any_element(),
        RootRoute::PrivacyExceptions { r#type, action } => {
            PrivacyExceptionsScreen::new(r#type.clone(), action.clone()).into_any_element()
        }
        RootRoute::BlockedUsers => BlockedUsersScreen::new().into_any_element(),
        RootRoute::SessionsSettings => SessionsSettings::new().into_any_element(),
        RootRoute::ModerationSettings => held_stateless(screen, cx, || ModerationSettings::new().into_any_element()),
        RootRoute::MyReports => MyReports::new().into_any_element(),
        RootRoute::MyBugReports => MyBugReports::new().into_any_element(),
        RootRoute::MyBlocks => held_stateless(screen, cx, || MyBlocks::new().into_any_element()),
        RootRoute::ReportsQueue => held_stateless(screen, cx, || ReportsQueue::new().into_any_element()),
        RootRoute::ReportDetail { report_id } => ReportDetail::new(report_id.clone()).into_any_element(),
        RootRoute::CaseContext { report_id } => CaseContextPage::new(report_id.clone()).into_any_element(),
        RootRoute::CaseNotes { report_id } => CaseNotesPage::new(report_id.clone()).into_any_element(),
        RootRoute::AllModerators => held_stateless(screen, cx, || AllModerators::new().into_any_element()),
        RootRoute::ModeratorDetail { user_id } => ModeratorDetail::new(user_id.clone()).into_any_element(),
        RootRoute::AppealsReview => AppealsReview::new().into_any_element(),
        RootRoute::AppealReview { appeal_id } => AppealReview::new(appeal_id.clone()).into_any_element(),
        RootRoute::LanguageSettings => LanguageSettings::new().into_any_element(),
        RootRoute::RegionSettings => RegionSettings::new().into_any_element(),
        RootRoute::NotificationLanguageSettings => NotificationLanguageSettings::new().into_any_element(),
        RootRoute::MemorySettings => MemorySettings::new().into_any_element(),
        RootRoute::AboutProject => AboutProject::new().into_any_element(),
        RootRoute::AboutProjectFAQ => AboutProjectFAQ::new().into_any_element(),
        RootRoute::PostViewHistory => PostViewHistory::new().into_any_element(),
        RootRoute::CachedChats => CachedChats::new().into_any_element(),
        RootRoute::CachedMedia => CachedMedia::new().into_any_element(),
        RootRoute::CachedDatas => CachedDatas::new().into_any_element(),
        RootRoute::CustomizationSettings => CustomizationSettings::new().into_any_element(),
        RootRoute::GuidesCatalogue => held(screen, cx, GuidesCatalogue::new),
        RootRoute::GuidesCategory { category } => {
            held(screen, cx, |cx| GuidesCategoryPage::new(category.clone(), cx))
        }
        RootRoute::ThemeSettings => ThemeSettings::new().into_any_element(),
        RootRoute::ThemesList => ThemesListSettings::new().into_any_element(),
        RootRoute::TabBarSettings => TabBarSettings::new().into_any_element(),
        RootRoute::ChatWallpapersSettings => {
            ChatWallpapersSettings::new(ChatWallpapersScope::Account).into_any_element()
        }
        RootRoute::ChatWallpaperPicker { chat_id, topic_id } => ChatWallpapersSettings::new(ChatWallpapersScope::Chat {
            chat_id: chat_id.clone().unwrap_or_default(),
            topic_id: topic_id.clone(),
        })
        .into_any_element(),
        RootRoute::BgColorSettings => BgColorSettings::new().into_any_element(),
        RootRoute::BlockColorSettings => BlockColorSettings::new().into_any_element(),
        RootRoute::BtnColorSettings => BtnColorSettings::new().into_any_element(),
        RootRoute::PrimaryColorSettings => PrimaryColorSettings::new().into_any_element(),
        RootRoute::TextColorSettings => TextColorSettings::new().into_any_element(),
        RootRoute::SecondaryTextColorSettings => SecondaryTextColorSettings::new().into_any_element(),
        RootRoute::BorderColorSettings => BorderColorSettings::new().into_any_element(),
        RootRoute::TextSizeSettings => TextSizeSettings::new().into_any_element(),
        RootRoute::EnergySavingSettings => EnergySavingSettings::new().into_any_element(),
        RootRoute::LaunchAnimationSettings => LaunchAnimationSettings::new().into_any_element(),
        RootRoute::StickerSettings { kind } => StickerSettings::new(*kind).into_any_element(),
        RootRoute::MyStickerPacks { kind } => MyStickerPacks::new(*kind).into_any_element(),
        RootRoute::StickerPackCoauthors { pack_id } => PackCoauthorsScreen::new(pack_id.clone()).into_any_element(),
        RootRoute::StickerPackCoauthorRights { pack_id, user_id } => {
            PackCoauthorRightsScreen::new(pack_id.clone(), user_id.clone()).into_any_element()
        }
        RootRoute::StickerPackInviteLinks { pack_id } => PackInviteLinksScreen::new(pack_id.clone()).into_any_element(),
        RootRoute::StickerPackInviteLinkJoiners { pack_id, link_id } => {
            PackInviteLinkJoinersScreen::new(pack_id.clone(), link_id.clone()).into_any_element()
        }
        RootRoute::StickerPackJoinRequests { pack_id } => {
            PackJoinRequestsScreen::new(pack_id.clone()).into_any_element()
        }
        RootRoute::ChatFoldersSettings => ChatFoldersSettings::new().into_any_element(),
        RootRoute::SubscriptionSettings => held_cached(screen, cx, SubscriptionSettings::new),
        RootRoute::SparksWallet => held(screen, cx, MySparksPage::new),
        RootRoute::NotificationSettings => NotificationSettings::new().into_any_element(),
        RootRoute::MusicNotificationSettings => MusicNotificationSettings::new().into_any_element(),
        RootRoute::GoalDetail { goal_id } => GoalDetail::new(goal_id.clone()).into_any_element(),
        RootRoute::PlanDetail { plan_id } => PlanDetail::new(plan_id.clone()).into_any_element(),
        RootRoute::MusicPlaylistDetail { collection_id } => {
            let collection_id = collection_id.clone();
            held_stateless(screen, cx, move || PlaylistDetail::new(collection_id.clone()).into_any_element())
        }
        RootRoute::MusicTrackLink { track_id } => TrackDeepLinkScreen::new(track_id.clone()).into_any_element(),
        RootRoute::MusicPlaylistLink { playlist_id } => {
            PlaylistDeepLinkScreen::new(playlist_id.clone()).into_any_element()
        }
        RootRoute::MusicPlaylistTrackLink { playlist_id, track_id } => {
            PlaylistTrackDeepLinkScreen::new(playlist_id.clone(), track_id.clone()).into_any_element()
        }
        RootRoute::MusicPlaylistSettings { playlist_id } => {
            PlaylistSettingsScreen::new(playlist_id.clone()).into_any_element()
        }
        RootRoute::MusicPlaylistInviteLink { playlist_id } => {
            PlaylistInviteLinkScreen::new(playlist_id.clone()).into_any_element()
        }
        RootRoute::MusicPlaylistJoinRequests { playlist_id } => {
            PlaylistJoinRequestsScreen::new(playlist_id.clone()).into_any_element()
        }
        RootRoute::MusicPlaylistMembers { playlist_id } => {
            PlaylistMembersScreen::new(playlist_id.clone()).into_any_element()
        }
        RootRoute::MusicPlaylistMemberRights { playlist_id, member_user_id } => {
            PlaylistMemberRightsScreen::new(playlist_id.clone(), member_user_id.clone()).into_any_element()
        }
        RootRoute::MusicAlbumDetail { album_id } => {
            PlaylistDetail::new(album_id.clone()).is_album(true).into_any_element()
        }
        RootRoute::MusicArtistProfile { artist_id } => ArtistProfile::new(artist_id.clone()).into_any_element(),
        RootRoute::MusicSettings => MusicSettings::new().into_any_element(),
        RootRoute::MusicLibrary => MusicLibraryScreen::new().into_any_element(),
        RootRoute::MusicLocalTracks => LocalMusicTracks::new().into_any_element(),
        RootRoute::MusicNowPlaying => NowPlayingAside::new("aside-now-playing").into_any_element(),
        RootRoute::BecomeArtistSettings => BecomeArtistSettings::new().into_any_element(),
        RootRoute::ArtistStudio => ArtistStudio::new().into_any_element(),
        RootRoute::MusicArtistRequestsAdmin => MusicArtistRequestsList::new().into_any_element(),
        RootRoute::MusicArtistRequestDetail { request_id } => {
            MusicArtistRequestDetail::new(request_id.clone()).into_any_element()
        }
        RootRoute::MusicTrackRequestsAdmin => MusicTrackRequestsList::new().into_any_element(),
        RootRoute::MusicTrackRequestDetail { track_id } => {
            MusicTrackRequestDetail::new(track_id.clone()).into_any_element()
        }
        RootRoute::MyMusicArtistRequest => MyMusicArtistRequest::new().into_any_element(),
        RootRoute::MyMusicTrackRequests => MyMusicTrackRequests::new().into_any_element(),
        RootRoute::MyMusicAlbumRequests => MyMusicAlbumRequests::new().into_any_element(),
        RootRoute::MusicAlbumRequestsAdmin => MusicAlbumRequestsList::new().into_any_element(),
        RootRoute::MusicAlbumRequestDetail { request_id } => {
            MusicAlbumRequestDetail::new(request_id.clone()).into_any_element()
        }
        RootRoute::StudioAlbumDetail { album_id } => StudioAlbumDetail::new(album_id.clone()).into_any_element(),
        RootRoute::VacancyDetail { .. }
        | RootRoute::CreateVacancy
        | RootRoute::EditVacancy { .. }
        | RootRoute::CreateResume
        | RootRoute::MyResume
        | RootRoute::MyVacancies
        | RootRoute::VacancySettings
        | RootRoute::VacancyList
            if !RiseFeatureGate::VACANCIES =>
        {
            SectionUnavailableScreen.into_any_element()
        }
        RootRoute::VacancyDetail { vacancy_id } => held(screen, cx, |cx| VacancyDetail::new(*vacancy_id, cx)),
        RootRoute::CreateVacancy => CreateVacancy::new(None).into_any_element(),
        RootRoute::EditVacancy { vacancy_id } => EditVacancyRoute::new(*vacancy_id).into_any_element(),
        RootRoute::CreateResume => CreateResume::new().into_any_element(),
        RootRoute::MyResume => MyResume::new().into_any_element(),
        RootRoute::MyVacancies => held(screen, cx, MyVacancies::new),
        RootRoute::VacancySettings => held(screen, cx, VacancySettings::new),
        RootRoute::StudioTrackDetail { track_id } => StudioTrackDetail::new(track_id.clone()).into_any_element(),
        RootRoute::MyProfile => Profile::new("my-profile").into_any_element(),
        RootRoute::VacancyList => held(screen, cx, |cx| VacancyList::new(true, cx)),
        RootRoute::MusicTab => MusicTab::new("music-tab").shows_back_button(true).into_any_element(),
        RootRoute::HashtagSearch { tag, kind } => {
            held(screen, cx, |cx| GlobalSearch::new(format!("#{tag}"), *kind, cx))
        }
        RootRoute::AudioPage { track_id, source_selection } => AudioPage::new("audio-page", track_id.clone())
            .source_selection(source_selection.clone())
            .into_any_element(),
        RootRoute::CreatePostWithAchievement { achievement_id } => held(screen, cx, |cx| {
            CreatePost::new(CreatePostIntent::Publication, None, Some(*achievement_id), None, window, cx)
        }),
        RootRoute::PostStats { post_id } => PostStats::new(post_id.clone()).into_any_element(),
        RootRoute::QrCode { subject } => held(screen, cx, |cx| RiseQRCodePage::new(subject.clone(), window, cx)),
        RootRoute::QrScanner => held(screen, cx, |cx| RiseQRScannerPage::new(window, cx)),
        RootRoute::MapJourneyComposer => held(screen, cx, |cx| MapJourneyComposerPage::new(window, cx)),
        RootRoute::MapJourney { post_id } => held(screen, cx, |cx| MapJourneyPage::new(*post_id, window, cx)),
        RootRoute::WorldMap => held(screen, cx, |cx| WorldMapPage::new(None, true, window, cx)),
        RootRoute::MapFriends => held(screen, cx, MapFriendsPage::new),
        RootRoute::MapProfile => held(screen, cx, MapProfilePage::new),
        RootRoute::MapSettings => held(screen, cx, MapSettingsPage::new),
        RootRoute::MapSettingsSection(section) => {
            held(screen, cx, |cx| MapSettingsSectionPage::new(*section, cx))
        }
        RootRoute::Onboarding | RootRoute::SignIn | RootRoute::SignUp | RootRoute::MainTabs(_) => {
            PlaceholderRouteView::new("\u{2014}").into_any_element()
        }
    }
}

fn send_mini_app_data(chat_id: Option<&str>, data: &str, init_data: &str, cx: &mut App) {
    let Some(chat_id) = chat_id.filter(|chat_id| !chat_id.is_empty() && !init_data.is_empty()) else {
        Notifier::send(
            tr("bot_callback_failed"),
            NotifierSendOptions {
                content: Some(tr("bot_button_not_supported")),
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
        return;
    };
    let completion: MessageActionCompletion = Box::new(|result, cx| {
        if let Err(error) = result {
            Notifier::send(
                tr("bot_callback_failed"),
                NotifierSendOptions {
                    content: Some(error.to_string()),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
    });
    let store = MessageInteractionsStore::shared(cx);
    let temp_id =
        store.update(cx, |store, cx| store.send_web_app_data(chat_id, data, init_data, None, Some(completion), cx));
    if temp_id.is_none() {
        Notifier::send(
            tr("bot_callback_failed"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}

#[derive(IntoElement)]
pub struct PlaceholderRouteView {
    title: SharedString,
    subtitle: Option<SharedString>,
}

impl PlaceholderRouteView {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), subtitle: None }
    }

    pub fn subtitle(mut self, subtitle: impl Into<Option<SharedString>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }
}

impl RenderOnce for PlaceholderRouteView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let subtitle = self.subtitle.filter(|subtitle| !subtitle.is_empty());
        BgWrapperUi::new().child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.0))
                .pt(px(60.0))
                .size_full()
                .child(MainText::new(self.title).px(22.0).font_weight(FontWeight::BOLD))
                .children(subtitle.map(|subtitle| SecondaryText::new(subtitle).px(12.0))),
        )
    }
}

#[derive(IntoElement)]
struct SectionUnavailableScreen;

impl RenderOnce for SectionUnavailableScreen {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let navigation_store = env::<NavigationStoreKey>(cx);
        let secondary = rise_ui::theme(cx).text.secondary;
        ListShellUi::new()
            .on_back(move |_window, cx| {
                if let Some(navigation_store) = &navigation_store {
                    navigation_store.update(cx, |store, cx| store.pop(cx));
                }
            })
            .content(
                div().flex().flex_col().items_center().justify_center().size_full().child(
                    div().px(px(32.0)).child(
                        MainText::new(tr("server_error_unavailable"))
                            .px(16.0)
                            .color(secondary)
                            .number_of_lines(2)
                            .tac(TextAlignT::Center),
                    ),
                ),
            )
    }
}

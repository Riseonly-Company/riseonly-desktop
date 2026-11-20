use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::mem;

use rise_canon::rise_key::guide_screen::camel as guide_screen_keys;
use rise_core::foundation::foundation_url::Url;

use crate::app::router::map_settings_section_hooks::MapSettingsSection;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiActiveScreen;
use crate::core::ui::qr_code::rise_qr_code_card::RiseQRCodeSubject;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackSelection;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPageScope, RisePostProfileKind, RisePostSearchKind,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;

/// raw values are the screen names guides and refresh read
/// music and vacancies and map tabs use what their routes use
/// profile is ones own profile
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MainTab {
    Posts,
    GlobalSearch,
    Chats,
    Notifications,
    Shorts,
    Profile,
    Music,
    Vacancies,
    Maps,
}

impl MainTab {
    pub const ALL_CASES: [MainTab; 9] = [
        MainTab::Posts,
        MainTab::GlobalSearch,
        MainTab::Chats,
        MainTab::Notifications,
        MainTab::Shorts,
        MainTab::Profile,
        MainTab::Music,
        MainTab::Vacancies,
        MainTab::Maps,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MainTab::Posts => "Posts",
            MainTab::GlobalSearch => "GlobalSearch",
            MainTab::Chats => "Chats",
            MainTab::Notifications => "Notifications",
            MainTab::Shorts => "Shorts",
            MainTab::Profile => "Profile",
            MainTab::Music => "MusicTab",
            MainTab::Vacancies => "VacancyList",
            MainTab::Maps => "WorldMap",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|tab| tab.raw_value() == raw)
    }

    pub const fn sai_screen_id(self) -> &'static str {
        self.raw_value()
    }
}

/// how the post composer opens, not the wire kinds
/// the wire has two post kinds and a reel is a publication with a short video
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CreatePostIntent {
    Publication,
    Reel,
    Thread,
}

impl CreatePostIntent {
    pub const ALL_CASES: [CreatePostIntent; 3] =
        [CreatePostIntent::Publication, CreatePostIntent::Reel, CreatePostIntent::Thread];

    pub const fn raw_value(self) -> &'static str {
        match self {
            CreatePostIntent::Publication => "publication",
            CreatePostIntent::Reel => "reel",
            CreatePostIntent::Thread => "thread",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|intent| intent.raw_value() == raw)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RiseRTCRoomTarget {
    Topic { chat_id: String, topic_id: String },
    Group { chat_id: String },
    DirectCall { call_id: String },
}

#[derive(Clone, PartialEq, Debug)]
pub enum RootRoute {
    Onboarding,
    SignIn,
    SignUp,
    MainTabs(MainTab),
    PostDetail { post_id: Option<String>, origin: Option<RisePostPageScope> },
    ShortViewer {
        post: GetPostFeedResponse,
        origin: Option<RisePostPageScope>,
        sound_track_id: Option<String>,
    },
    PostsInteresting { selected_post: Option<GetPostFeedResponse> },
    Chat { chat_id: Option<String> },
    MessageRequests,
    ArchivedChats,
    ChatTopic { chat_id: Option<String>, topic_id: Option<String> },
    RtcRoom { target: RiseRTCRoomTarget },
    ChatThread { chat_id: Option<String>, root_id: Option<String> },
    ChatProfile { chat_id: Option<String> },
    ChatSettings { chat_id: Option<String> },
    ChatSettingsType { chat_id: Option<String> },
    ChatSettingsLinks { chat_id: Option<String> },
    ChatSettingsLinkedChat { chat_id: Option<String> },
    ChatStatistics { chat_id: Option<String> },
    ChatSettingsAllowedReactions { chat_id: Option<String> },
    ChatWallpaperPicker { chat_id: Option<String>, topic_id: Option<String> },
    ChatSettingsMembers { chat_id: Option<String> },
    ChatSettingsPermissions { chat_id: Option<String> },
    ChatSettingsAdmins { chat_id: Option<String> },
    ChatSettingsBlacklist { chat_id: Option<String> },
    ChatSettingsRecentActions { chat_id: Option<String> },
    ChatTopics { chat_id: Option<String> },
    GroupVoiceSettings { chat_id: String },
    ForumHome { chat_id: Option<String> },
    ForumAll { chat_id: Option<String> },
    GeneralProfile { chat_id: Option<String> },
    TopicProfile { chat_id: Option<String>, topic_id: Option<String> },
    ChatRoles { chat_id: Option<String> },
    ChatRoleEdit { chat_id: Option<String>, role_id: Option<String> },
    PinnedMessages { chat_id: Option<String>, topic_id: Option<String> },
    Notifications,
    UserPage { user_id: Option<String>, tag: Option<String> },
    UserSubs { user_id: Option<String> },
    UserSubscribers { user_id: Option<String> },
    UserFriends { user_id: Option<String> },
    ProfileMore { user_id: Option<String>, tag: Option<String> },
    CreatePostMedia {
        intent: CreatePostIntent,
        track_seed: Option<MediaPickerTrackSelection>,
        journey_id: Option<String>,
    },
    CreatePost { intent: CreatePostIntent, journey_id: Option<String> },
    EditPost { post: GetPostFeedResponse },
    EditStory { story_id: i64 },
    StoriesArchive,
    CloseFriends,
    Settings,
    MyBots,
    BotSettings { bot_id: String },
    BotCommands { bot_id: String },
    BotWebhook { bot_id: String },
    BotMiniApps { bot_id: String },
    BotMiniAppEditor { bot_id: String, mini_app_id: Option<String> },
    BotMiniApp {
        url: Url,
        title: String,
        allows_development_http: bool,
        chat_id: Option<String>,
        bot_id: Option<String>,
        is_web_link: bool,
    },
    BotMiniAppLink { tag: String, slug: String, start_param: Option<String> },
    BotStartLink { tag: String, start_param: Option<String> },
    ProfileSettings,
    Achievements { user_id: Option<String> },
    RtcAudioSettings,
    RtcStreamSettings,
    QuickReactionSettings,
    PremiumBadgeSettings,
    SocialLinksEditor,
    PrivacySettings,
    PrivacySetting { r#type: Option<String> },
    PrivacyExceptions { r#type: String, action: String },
    BlockedUsers,
    SessionsSettings,
    ModerationSettings,
    MyReports,
    MyBugReports,
    MyBlocks,
    ReportsQueue,
    ReportDetail { report_id: String },
    CaseContext { report_id: String },
    CaseNotes { report_id: String },
    AllModerators,
    ModeratorDetail { user_id: String },
    AppealsReview,
    AppealReview { appeal_id: String },
    LanguageSettings,
    RegionSettings,
    NotificationLanguageSettings,
    MemorySettings,
    AboutProject,
    AboutProjectFAQ,
    PostViewHistory,
    CachedChats,
    CachedMedia,
    CachedDatas,
    CustomizationSettings,
    GuidesCatalogue,
    GuidesCategory { category: String },
    ThemeSettings,
    ThemesList,
    TabBarSettings,
    ChatWallpapersSettings,
    BgColorSettings,
    BlockColorSettings,
    BtnColorSettings,
    PrimaryColorSettings,
    TextColorSettings,
    SecondaryTextColorSettings,
    BorderColorSettings,
    TextSizeSettings,
    EnergySavingSettings,
    LaunchAnimationSettings,
    StickerSettings { kind: PackKind },
    MyStickerPacks { kind: PackKind },
    StickerPackCoauthors { pack_id: String },
    StickerPackCoauthorRights { pack_id: String, user_id: String },
    StickerPackInviteLinks { pack_id: String },
    StickerPackInviteLinkJoiners { pack_id: String, link_id: String },
    StickerPackJoinRequests { pack_id: String },
    ChatFoldersSettings,
    SubscriptionSettings,
    SparksWallet,
    NotificationSettings,
    MusicNotificationSettings,
    GoalDetail { goal_id: String },
    PlanDetail { plan_id: String },
    MusicPlaylistDetail { collection_id: String },
    MusicTrackLink { track_id: String },
    MusicPlaylistLink { playlist_id: String },
    MusicPlaylistTrackLink { playlist_id: String, track_id: String },
    MusicPlaylistSettings { playlist_id: String },
    MusicPlaylistInviteLink { playlist_id: String },
    MusicPlaylistJoinRequests { playlist_id: String },
    MusicPlaylistMembers { playlist_id: String },
    MusicPlaylistMemberRights { playlist_id: String, member_user_id: String },
    MusicAlbumDetail { album_id: String },
    MusicArtistProfile { artist_id: String },
    MusicSettings,
    MusicLibrary,
    MusicLocalTracks,
    MusicNowPlaying,
    BecomeArtistSettings,
    ArtistStudio,
    MusicArtistRequestsAdmin,
    MusicArtistRequestDetail { request_id: String },
    MusicTrackRequestsAdmin,
    MusicTrackRequestDetail { track_id: String },
    MyMusicArtistRequest,
    MyMusicTrackRequests,
    MyMusicAlbumRequests,
    MusicAlbumRequestsAdmin,
    MusicAlbumRequestDetail { request_id: String },
    StudioAlbumDetail { album_id: String },
    VacancyDetail { vacancy_id: i64 },
    CreateVacancy,
    EditVacancy { vacancy_id: i64 },
    CreateResume,
    MyResume,
    MyVacancies,
    VacancySettings,
    StudioTrackDetail { track_id: String },
    MyProfile,
    VacancyList,
    MusicTab,
    HashtagSearch { tag: String, kind: RisePostSearchKind },
    AudioPage { track_id: String, source_selection: Option<MediaPickerTrackSelection> },
    CreatePostWithAchievement { achievement_id: i64 },
    PostStats { post_id: String },
    QrCode { subject: RiseQRCodeSubject },
    QrScanner,
    WorldMap,
    MapJourneyComposer,
    MapJourney { post_id: i64 },
    MapFriends,
    MapProfile,
    MapSettings,
    MapSettingsSection(MapSettingsSection),
}

impl Eq for RootRoute {}

impl Hash for RootRoute {
    fn hash<H: Hasher>(&self, state: &mut H) {
        mem::discriminant(self).hash(state);
    }
}

impl RootRoute {
    /// one post, one placement per list
    /// profile publications open a post and the riffs tab opens the riff viewer
    /// other lists send riff only posts to the viewer and the rest to the post page
    pub fn post_content(post: &GetPostFeedResponse, origin: Option<RisePostPageScope>) -> RootRoute {
        let profile_kind = match &origin {
            Some(RisePostPageScope::Profile { kind, .. }) => Some(*kind),
            _ => None,
        };
        if let Some(kind) = profile_kind {
            match kind {
                RisePostProfileKind::Publications => {
                    return RootRoute::PostDetail { post_id: Some(post.id.clone()), origin };
                }
                RisePostProfileKind::Riffs => {
                    return RootRoute::ShortViewer { post: post.clone(), origin, sound_track_id: None };
                }
                RisePostProfileKind::Reposts
                | RisePostProfileKind::Mentions
                | RisePostProfileKind::Favorites
                | RisePostProfileKind::Liked => {
                    return if post.is_riff {
                        RootRoute::ShortViewer { post: post.clone(), origin, sound_track_id: None }
                    } else {
                        RootRoute::PostDetail { post_id: Some(post.id.clone()), origin }
                    };
                }
                _ => {}
            }
        }
        if post.is_riff_only() {
            return RootRoute::ShortViewer { post: post.clone(), origin, sound_track_id: None };
        }
        RootRoute::PostDetail { post_id: Some(post.id.clone()), origin }
    }

    pub fn sai_active_screen(&self) -> SaiActiveScreen {
        match self {
            RootRoute::Chat { chat_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                SaiActiveScreen::new("Chat".to_owned(), params)
            }
            RootRoute::ChatTopic { chat_id, topic_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                insert_present(&mut params, guide_screen_keys::TOPIC_ID, topic_id);
                SaiActiveScreen::new("Chat".to_owned(), params)
            }
            RootRoute::RtcRoom { target } => match target {
                RiseRTCRoomTarget::Topic { chat_id, topic_id } => SaiActiveScreen::new(
                    "RTCRoom".to_owned(),
                    HashMap::from([
                        (guide_screen_keys::CHAT_ID.to_owned(), chat_id.clone()),
                        (guide_screen_keys::TOPIC_ID.to_owned(), topic_id.clone()),
                    ]),
                ),
                RiseRTCRoomTarget::Group { chat_id } => SaiActiveScreen::new(
                    "RTCRoom".to_owned(),
                    HashMap::from([(guide_screen_keys::CHAT_ID.to_owned(), chat_id.clone())]),
                ),
                RiseRTCRoomTarget::DirectCall { call_id } => SaiActiveScreen::new(
                    "RTCCall".to_owned(),
                    HashMap::from([("callId".to_owned(), call_id.clone())]),
                ),
            },
            RootRoute::ForumAll { chat_id } => {
                let mut params = HashMap::from([("topicMode".to_owned(), "all".to_owned())]);
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                SaiActiveScreen::new("Chat".to_owned(), params)
            }
            RootRoute::ChatThread { chat_id, root_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                insert_present(&mut params, guide_screen_keys::THREAD_ROOT_ID, root_id);
                SaiActiveScreen::new("Chat".to_owned(), params)
            }
            RootRoute::ChatProfile { chat_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                SaiActiveScreen::new("ChatProfile".to_owned(), params)
            }
            RootRoute::ChatWallpaperPicker { chat_id, topic_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                insert_present(&mut params, guide_screen_keys::TOPIC_ID, topic_id);
                SaiActiveScreen::new("ChatWallpaperPicker".to_owned(), params)
            }
            RootRoute::ChatTopics { chat_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                SaiActiveScreen::new("ChatTopics".to_owned(), params)
            }
            RootRoute::ForumHome { chat_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                SaiActiveScreen::new("ForumHome".to_owned(), params)
            }
            RootRoute::PinnedMessages { chat_id, topic_id } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::CHAT_ID, chat_id);
                insert_present(&mut params, guide_screen_keys::TOPIC_ID, topic_id);
                SaiActiveScreen::new("PinnedMessages".to_owned(), params)
            }
            RootRoute::MusicPlaylistDetail { collection_id } => SaiActiveScreen::new(
                "MusicPlaylistDetail".to_owned(),
                HashMap::from([(guide_screen_keys::PLAYLIST_ID.to_owned(), collection_id.clone())]),
            ),
            RootRoute::MusicTrackLink { track_id } => SaiActiveScreen::new(
                "MusicTrackLink".to_owned(),
                HashMap::from([(guide_screen_keys::TRACK_ID.to_owned(), track_id.clone())]),
            ),
            RootRoute::MusicPlaylistLink { playlist_id } => SaiActiveScreen::new(
                "MusicPlaylistLink".to_owned(),
                HashMap::from([(guide_screen_keys::PLAYLIST_ID.to_owned(), playlist_id.clone())]),
            ),
            RootRoute::MusicPlaylistTrackLink { playlist_id, track_id } => SaiActiveScreen::new(
                "MusicPlaylistTrackLink".to_owned(),
                HashMap::from([
                    (guide_screen_keys::PLAYLIST_ID.to_owned(), playlist_id.clone()),
                    (guide_screen_keys::TRACK_ID.to_owned(), track_id.clone()),
                ]),
            ),
            RootRoute::MusicPlaylistSettings { playlist_id }
            | RootRoute::MusicPlaylistInviteLink { playlist_id }
            | RootRoute::MusicPlaylistJoinRequests { playlist_id } => {
                SaiActiveScreen::new(
                    self.sai_screen_id(),
                    HashMap::from([(guide_screen_keys::PLAYLIST_ID.to_owned(), playlist_id.clone())]),
                )
            }
            RootRoute::MainTabs(tab) => SaiActiveScreen::new(tab.sai_screen_id().to_owned(), HashMap::new()),
            RootRoute::AudioPage { track_id, .. } => {
                SaiActiveScreen::new(
                    self.sai_screen_id(),
                    HashMap::from([(guide_screen_keys::TRACK_ID.to_owned(), track_id.clone())]),
                )
            }
            RootRoute::PostDetail { post_id, .. } => {
                let mut params = HashMap::new();
                insert_present(&mut params, guide_screen_keys::POST_ID, post_id);
                SaiActiveScreen::new(self.sai_screen_id(), params)
            }
            _ => SaiActiveScreen::new(self.sai_screen_id(), HashMap::new()),
        }
    }

    fn sai_screen_id(&self) -> String {
        let id = match self {
            RootRoute::Onboarding => "Onboarding",
            RootRoute::SignIn => "SignIn",
            RootRoute::SignUp => "SignUp",
            RootRoute::MainTabs(tab) => tab.sai_screen_id(),
            RootRoute::PostDetail { .. } => "PostDetail",
            RootRoute::ShortViewer { .. } => "ShortViewer",
            RootRoute::PostsInteresting { .. } => "PostsInteresting",
            RootRoute::Chat { .. } => "Chat",
            RootRoute::MessageRequests => "MessageRequests",
            RootRoute::ArchivedChats => "ArchivedChats",
            RootRoute::ChatTopic { .. } => "Chat",
            RootRoute::RtcRoom { .. } => "RTCRoom",
            RootRoute::ChatThread { .. } => "Chat",
            RootRoute::ChatProfile { .. } => "ChatProfile",
            RootRoute::ChatSettings { .. } => "ChatSettings",
            RootRoute::ChatSettingsType { .. } => "ChatSettingsType",
            RootRoute::ChatSettingsLinks { .. } => "ChatSettingsLinks",
            RootRoute::ChatSettingsLinkedChat { .. } => "ChatSettingsLinkedChat",
            RootRoute::ChatStatistics { .. } => "ChatStatistics",
            RootRoute::ChatSettingsAllowedReactions { .. } => "ChatSettingsAllowedReactions",
            RootRoute::ChatWallpaperPicker { .. } => "ChatWallpaperPicker",
            RootRoute::ChatSettingsMembers { .. } => "ChatSettingsMembers",
            RootRoute::ChatSettingsPermissions { .. } => "ChatSettingsPermissions",
            RootRoute::ChatSettingsAdmins { .. } => "ChatSettingsAdmins",
            RootRoute::ChatSettingsBlacklist { .. } => "ChatSettingsBlacklist",
            RootRoute::ChatSettingsRecentActions { .. } => "ChatSettingsRecentActions",
            RootRoute::ChatTopics { .. } => "ChatTopics",
            RootRoute::GroupVoiceSettings { .. } => "GroupVoiceSettings",
            RootRoute::ForumHome { .. } => "ForumHome",
            RootRoute::ForumAll { .. } => "Chat",
            RootRoute::GeneralProfile { .. } => "ChatProfile",
            RootRoute::TopicProfile { .. } => "ChatProfile",
            RootRoute::ChatRoles { .. } => "ChatRoles",
            RootRoute::ChatRoleEdit { .. } => "ChatRoleEdit",
            RootRoute::PinnedMessages { .. } => "PinnedMessages",
            RootRoute::Notifications => "Notifications",
            RootRoute::UserPage { .. } => "UserPage",
            RootRoute::UserSubs { .. } => "UserSubs",
            RootRoute::UserSubscribers { .. } => "UserSubscribers",
            RootRoute::UserFriends { .. } => "UserFriends",
            RootRoute::ProfileMore { .. } => "ProfileMore",
            RootRoute::CreatePostMedia { .. } => "CreatePostMedia",
            RootRoute::CreatePost { .. } => "CreatePost",
            RootRoute::EditPost { .. } => "EditPost",
            RootRoute::EditStory { .. } => "EditStory",
            RootRoute::StoriesArchive => "StoriesArchive",
            RootRoute::CloseFriends => "CloseFriends",
            RootRoute::Settings => "Settings",
            RootRoute::MyBots => "MyBots",
            RootRoute::BotSettings { .. } => "BotSettings",
            RootRoute::BotCommands { .. } => "BotCommands",
            RootRoute::BotWebhook { .. } => "BotWebhook",
            RootRoute::BotMiniApps { .. } => "BotMiniApps",
            RootRoute::BotMiniAppEditor { .. } => "BotMiniAppEditor",
            RootRoute::BotMiniApp { .. } | RootRoute::BotMiniAppLink { .. } => "BotMiniApp",
            RootRoute::BotStartLink { .. } => "BotStartLink",
            RootRoute::ProfileSettings => "ProfileSettings",
            RootRoute::Achievements { .. } => "Achievements",
            RootRoute::RtcAudioSettings => "RTCAudioSettings",
            RootRoute::RtcStreamSettings => "RTCStreamSettings",
            RootRoute::QuickReactionSettings => "QuickReactionSettings",
            RootRoute::PremiumBadgeSettings => "PremiumBadgeSettings",
            RootRoute::SocialLinksEditor => "SocialLinksEditor",
            RootRoute::PrivacySettings => "PrivacySettings",
            RootRoute::PrivacySetting { .. } => "PrivacySetting",
            RootRoute::PrivacyExceptions { .. } => "PrivacyExceptions",
            RootRoute::BlockedUsers => "BlockedUsers",
            RootRoute::SessionsSettings => "SessionsSettings",
            RootRoute::ModerationSettings => "ModerationSettings",
            RootRoute::MyReports => "MyReports",
            RootRoute::MyBugReports => "MyBugReports",
            RootRoute::MyBlocks => "MyBlocks",
            RootRoute::ReportsQueue => "ReportsQueue",
            RootRoute::ReportDetail { .. } => "ReportDetail",
            RootRoute::CaseContext { .. } => "CaseContext",
            RootRoute::CaseNotes { .. } => "CaseNotes",
            RootRoute::AllModerators => "AllModerators",
            RootRoute::ModeratorDetail { .. } => "ModeratorDetail",
            RootRoute::AppealsReview => "AppealsReview",
            RootRoute::AppealReview { .. } => "AppealReview",
            RootRoute::LanguageSettings => "LanguageSettings",
            RootRoute::RegionSettings => "RegionSettings",
            RootRoute::NotificationLanguageSettings => "NotificationLanguageSettings",
            RootRoute::MemorySettings => "MemorySettings",
            RootRoute::AboutProject => "AboutProject",
            RootRoute::AboutProjectFAQ => "AboutProjectFAQ",
            RootRoute::PostViewHistory => "PostViewHistory",
            RootRoute::CachedChats => "CachedChats",
            RootRoute::CachedMedia => "CachedMedia",
            RootRoute::CachedDatas => "CachedDatas",
            RootRoute::CustomizationSettings => "CustomizationSettings",
            RootRoute::GuidesCatalogue => "GuidesCatalogue",
            RootRoute::GuidesCategory { .. } => "GuidesCategory",
            RootRoute::ThemeSettings => "ThemeSettings",
            RootRoute::ThemesList => "ThemesList",
            RootRoute::TabBarSettings => "TabBarSettings",
            RootRoute::ChatWallpapersSettings => "ChatWallpapersSettings",
            RootRoute::BgColorSettings => "BgColorSettings",
            RootRoute::BlockColorSettings => "BlockColorSettings",
            RootRoute::BtnColorSettings => "BtnColorSettings",
            RootRoute::PrimaryColorSettings => "PrimaryColorSettings",
            RootRoute::TextColorSettings => "TextColorSettings",
            RootRoute::SecondaryTextColorSettings => "SecondaryTextColorSettings",
            RootRoute::BorderColorSettings => "BorderColorSettings",
            RootRoute::TextSizeSettings => "TextSizeSettings",
            RootRoute::EnergySavingSettings => "EnergySavingSettings",
            RootRoute::LaunchAnimationSettings => "LaunchAnimationSettings",
            RootRoute::StickerSettings { .. } => "StickerSettings",
            RootRoute::MyStickerPacks { .. } => "MyStickerPacks",
            RootRoute::StickerPackCoauthors { .. } => "StickerPackCoauthors",
            RootRoute::StickerPackCoauthorRights { .. } => "StickerPackCoauthorRights",
            RootRoute::StickerPackInviteLinks { .. } => "StickerPackInviteLinks",
            RootRoute::StickerPackInviteLinkJoiners { .. } => "StickerPackInviteLinkJoiners",
            RootRoute::StickerPackJoinRequests { .. } => "StickerPackJoinRequests",
            RootRoute::ChatFoldersSettings => "ChatFoldersSettings",
            RootRoute::SubscriptionSettings => "SubscriptionSettings",
            RootRoute::SparksWallet => "SparksWallet",
            RootRoute::NotificationSettings => "NotificationSettings",
            RootRoute::MusicNotificationSettings => "MusicNotificationSettings",
            RootRoute::GoalDetail { .. } => "GoalDetail",
            RootRoute::PlanDetail { .. } => "PlanDetail",
            RootRoute::MusicPlaylistDetail { .. } => "MusicPlaylistDetail",
            RootRoute::MusicTrackLink { .. } => "MusicTrackLink",
            RootRoute::MusicPlaylistLink { .. } => "MusicPlaylistLink",
            RootRoute::MusicPlaylistTrackLink { .. } => "MusicPlaylistTrackLink",
            RootRoute::MusicPlaylistSettings { .. } => "MusicPlaylistSettings",
            RootRoute::MusicPlaylistInviteLink { .. } => "MusicPlaylistInviteLink",
            RootRoute::MusicPlaylistJoinRequests { .. } => "MusicPlaylistJoinRequests",
            RootRoute::MusicPlaylistMembers { .. } => "MusicPlaylistMembers",
            RootRoute::MusicPlaylistMemberRights { .. } => "MusicPlaylistMemberRights",
            RootRoute::MusicAlbumDetail { .. } => "MusicAlbumDetail",
            RootRoute::MusicArtistProfile { .. } => "MusicArtistProfile",
            RootRoute::MusicSettings => "MusicSettings",
            RootRoute::MusicLibrary => "MusicLibrary",
            RootRoute::MusicLocalTracks => "MusicLocalTracks",
            RootRoute::MusicNowPlaying => "MusicNowPlaying",
            RootRoute::BecomeArtistSettings => "BecomeArtistSettings",
            RootRoute::ArtistStudio => "ArtistStudio",
            RootRoute::MusicArtistRequestsAdmin => "MusicArtistRequestsList",
            RootRoute::MusicArtistRequestDetail { .. } => "MusicArtistRequestDetail",
            RootRoute::MusicTrackRequestsAdmin => "MusicTrackRequestsList",
            RootRoute::MusicTrackRequestDetail { .. } => "MusicTrackRequestDetail",
            RootRoute::MyMusicArtistRequest => "MyMusicArtistRequest",
            RootRoute::MyMusicTrackRequests => "MyMusicTrackRequests",
            RootRoute::MyMusicAlbumRequests => "MyMusicAlbumRequests",
            RootRoute::MusicAlbumRequestsAdmin => "MusicAlbumRequestsList",
            RootRoute::MusicAlbumRequestDetail { .. } => "MusicAlbumRequestDetail",
            RootRoute::StudioAlbumDetail { .. } => "StudioAlbumDetail",
            RootRoute::VacancyDetail { .. } => "VacancyDetail",
            RootRoute::CreateVacancy => "CreateVacancy",
            RootRoute::EditVacancy { .. } => "EditVacancy",
            RootRoute::CreateResume => "CreateResume",
            RootRoute::MyResume => "MyResume",
            RootRoute::MyVacancies => "MyVacancies",
            RootRoute::VacancySettings => "VacancySettings",
            RootRoute::StudioTrackDetail { .. } => "StudioTrackDetail",
            RootRoute::MyProfile => "MyProfile",
            RootRoute::VacancyList => "VacancyList",
            RootRoute::MusicTab => "MusicTab",
            RootRoute::HashtagSearch { .. } => "HashtagSearch",
            RootRoute::AudioPage { .. } => "AudioPage",
            RootRoute::CreatePostWithAchievement { .. } => "CreatePost",
            RootRoute::PostStats { .. } => "PostStats",
            RootRoute::QrCode { .. } => "QRCode",
            RootRoute::QrScanner => "QRScanner",
            RootRoute::WorldMap => "WorldMap",
            RootRoute::MapJourneyComposer => "MapJourneyComposer",
            RootRoute::MapJourney { .. } => "MapJourney",
            RootRoute::MapFriends => "MapFriends",
            RootRoute::MapProfile => "MapProfile",
            RootRoute::MapSettings => "MapSettings",
            RootRoute::MapSettingsSection(section) => return format!("MapSettings.{}", section.raw_value()),
        };
        id.to_owned()
    }
}

impl RootRoute {
    pub fn shows_music_mini_player(&self) -> bool {
        !matches!(
            self,
            RootRoute::Onboarding
                | RootRoute::SignIn
                | RootRoute::SignUp
                | RootRoute::MainTabs(_)
                | RootRoute::Chat { .. }
                | RootRoute::ChatTopic { .. }
                | RootRoute::ChatThread { .. }
                | RootRoute::ForumAll { .. }
                | RootRoute::CaseNotes { .. }
                | RootRoute::RtcRoom { .. }
                | RootRoute::ShortViewer { .. }
                | RootRoute::QrScanner
                | RootRoute::WorldMap
                | RootRoute::MapJourney { .. }
                | RootRoute::BotMiniApp { .. }
                | RootRoute::BotMiniAppLink { .. }
                | RootRoute::BotStartLink { .. }
                | RootRoute::CreatePostMedia { .. }
                | RootRoute::CreatePost { .. }
                | RootRoute::EditPost { .. }
                | RootRoute::CreatePostWithAchievement { .. }
                | RootRoute::EditStory { .. }
                | RootRoute::MapJourneyComposer
        )
    }
}

fn insert_present(params: &mut HashMap<String, String>, key: &str, value: &Option<String>) {
    if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
        params.insert(key.to_owned(), value.to_owned());
    }
}

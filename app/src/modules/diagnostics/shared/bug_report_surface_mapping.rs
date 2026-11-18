use rise_canon::RiseBugReportSurface;

use crate::app::router::navigation_types::{MainTab, RootRoute};

impl RootRoute {
    pub fn bug_report_surface(&self) -> RiseBugReportSurface {
        match self {
            RootRoute::Onboarding => RiseBugReportSurface::Onboarding,
            RootRoute::SignIn | RootRoute::SignUp => RiseBugReportSurface::Auth,

            RootRoute::MainTabs(tab) => tab.bug_report_surface(),

            RootRoute::PostDetail { .. }
            | RootRoute::PostsInteresting { .. }
            | RootRoute::CreatePostMedia { .. }
            | RootRoute::CreatePost { .. }
            | RootRoute::EditPost { .. }
            | RootRoute::CreatePostWithAchievement { .. }
            | RootRoute::PostStats { .. }
            | RootRoute::PostViewHistory => RiseBugReportSurface::Post,
            RootRoute::ShortViewer { .. } => RiseBugReportSurface::Shorts,
            RootRoute::HashtagSearch { .. } => RiseBugReportSurface::Search,

            RootRoute::Chat { .. }
            | RootRoute::MessageRequests
            | RootRoute::ArchivedChats
            | RootRoute::ChatTopic { .. }
            | RootRoute::ChatThread { .. }
            | RootRoute::PinnedMessages { .. }
            | RootRoute::CachedChats
            | RootRoute::ChatFoldersSettings => RiseBugReportSurface::Chat,
            RootRoute::ChatProfile { .. }
            | RootRoute::ChatSettings { .. }
            | RootRoute::ChatSettingsType { .. }
            | RootRoute::ChatSettingsLinks { .. }
            | RootRoute::ChatSettingsLinkedChat { .. }
            | RootRoute::ChatStatistics { .. }
            | RootRoute::ChatSettingsAllowedReactions { .. }
            | RootRoute::ChatSettingsMembers { .. }
            | RootRoute::ChatSettingsPermissions { .. }
            | RootRoute::ChatSettingsAdmins { .. }
            | RootRoute::ChatSettingsBlacklist { .. }
            | RootRoute::ChatSettingsRecentActions { .. }
            | RootRoute::ChatRoles { .. }
            | RootRoute::ChatRoleEdit { .. }
            | RootRoute::GeneralProfile { .. }
            | RootRoute::TopicProfile { .. }
            | RootRoute::ChatWallpaperPicker { .. } => RiseBugReportSurface::ChatProfile,
            RootRoute::QrCode { .. } | RootRoute::QrScanner => RiseBugReportSurface::ChatProfile,
            RootRoute::MapJourney { .. }
            | RootRoute::MapJourneyComposer
            | RootRoute::WorldMap
            | RootRoute::MapFriends
            | RootRoute::MapProfile
            | RootRoute::MapSettings
            | RootRoute::MapSettingsSection(_) => RiseBugReportSurface::Profile,
            RootRoute::ChatTopics { .. } | RootRoute::ForumHome { .. } | RootRoute::ForumAll { .. } => {
                RiseBugReportSurface::Forum
            }
            RootRoute::GroupVoiceSettings { .. } => RiseBugReportSurface::VoiceChannel,
            RootRoute::RtcRoom { .. } | RootRoute::RtcAudioSettings => RiseBugReportSurface::Call,

            RootRoute::Notifications
            | RootRoute::NotificationSettings
            | RootRoute::MusicNotificationSettings
            | RootRoute::NotificationLanguageSettings => RiseBugReportSurface::Notifications,

            RootRoute::UserPage { .. }
            | RootRoute::UserSubs { .. }
            | RootRoute::UserSubscribers { .. }
            | RootRoute::UserFriends { .. }
            | RootRoute::ProfileMore { .. }
            | RootRoute::MyProfile
            | RootRoute::Achievements { .. }
            | RootRoute::GoalDetail { .. }
            | RootRoute::PlanDetail { .. }
            | RootRoute::StoriesArchive
            | RootRoute::EditStory { .. }
            | RootRoute::CloseFriends => RiseBugReportSurface::Profile,
            RootRoute::ProfileSettings | RootRoute::SocialLinksEditor => RiseBugReportSurface::ProfileEdit,

            RootRoute::Settings
            | RootRoute::LanguageSettings
            | RootRoute::RegionSettings
            | RootRoute::MemorySettings
            | RootRoute::AboutProject
            | RootRoute::AboutProjectFAQ
            | RootRoute::CachedMedia
            | RootRoute::CachedDatas
            | RootRoute::PrivacySettings
            | RootRoute::PrivacySetting { .. }
            | RootRoute::PrivacyExceptions { .. }
            | RootRoute::BlockedUsers
            | RootRoute::SessionsSettings
            | RootRoute::CustomizationSettings
            | RootRoute::GuidesCatalogue
            | RootRoute::GuidesCategory { .. }
            | RootRoute::ThemeSettings
            | RootRoute::ThemesList
            | RootRoute::TabBarSettings
            | RootRoute::LaunchAnimationSettings
            | RootRoute::ChatWallpapersSettings
            | RootRoute::BgColorSettings
            | RootRoute::BlockColorSettings
            | RootRoute::BtnColorSettings
            | RootRoute::PrimaryColorSettings
            | RootRoute::TextColorSettings
            | RootRoute::SecondaryTextColorSettings
            | RootRoute::BorderColorSettings
            | RootRoute::TextSizeSettings
            | RootRoute::EnergySavingSettings
            | RootRoute::QuickReactionSettings
            | RootRoute::MyBugReports => RiseBugReportSurface::Settings,

            RootRoute::PremiumBadgeSettings | RootRoute::SubscriptionSettings | RootRoute::SparksWallet => {
                RiseBugReportSurface::Premium
            }

            RootRoute::StickerSettings { .. }
            | RootRoute::MyStickerPacks { .. }
            | RootRoute::StickerPackCoauthors { .. }
            | RootRoute::StickerPackCoauthorRights { .. }
            | RootRoute::StickerPackInviteLinks { .. }
            | RootRoute::StickerPackInviteLinkJoiners { .. }
            | RootRoute::StickerPackJoinRequests { .. } => RiseBugReportSurface::Stickers,

            RootRoute::MyBots
            | RootRoute::BotSettings { .. }
            | RootRoute::BotCommands { .. }
            | RootRoute::BotWebhook { .. }
            | RootRoute::BotMiniApps { .. }
            | RootRoute::BotMiniAppEditor { .. }
            | RootRoute::BotMiniApp { .. }
            | RootRoute::BotMiniAppLink { .. }
            | RootRoute::BotStartLink { .. } => RiseBugReportSurface::Bot,

            RootRoute::ModerationSettings
            | RootRoute::MyReports
            | RootRoute::MyBlocks
            | RootRoute::ReportsQueue
            | RootRoute::ReportDetail { .. }
            | RootRoute::CaseContext { .. }
            | RootRoute::CaseNotes { .. }
            | RootRoute::AllModerators
            | RootRoute::ModeratorDetail { .. }
            | RootRoute::AppealsReview
            | RootRoute::AppealReview { .. } => RiseBugReportSurface::Moderation,

            RootRoute::MusicTab
            | RootRoute::MusicPlaylistDetail { .. }
            | RootRoute::MusicTrackLink { .. }
            | RootRoute::MusicPlaylistLink { .. }
            | RootRoute::MusicPlaylistTrackLink { .. }
            | RootRoute::MusicPlaylistSettings { .. }
            | RootRoute::MusicPlaylistInviteLink { .. }
            | RootRoute::MusicPlaylistJoinRequests { .. }
            | RootRoute::MusicPlaylistMembers { .. }
            | RootRoute::MusicPlaylistMemberRights { .. }
            | RootRoute::MusicAlbumDetail { .. }
            | RootRoute::MusicArtistProfile { .. }
            | RootRoute::MusicSettings
            | RootRoute::MusicLibrary
            | RootRoute::MusicLocalTracks
            | RootRoute::MusicNowPlaying
            | RootRoute::BecomeArtistSettings
            | RootRoute::ArtistStudio
            | RootRoute::MusicArtistRequestsAdmin
            | RootRoute::MusicArtistRequestDetail { .. }
            | RootRoute::MusicTrackRequestsAdmin
            | RootRoute::MusicTrackRequestDetail { .. }
            | RootRoute::MyMusicArtistRequest
            | RootRoute::MyMusicTrackRequests
            | RootRoute::MyMusicAlbumRequests
            | RootRoute::MusicAlbumRequestsAdmin
            | RootRoute::MusicAlbumRequestDetail { .. }
            | RootRoute::StudioAlbumDetail { .. }
            | RootRoute::StudioTrackDetail { .. }
            | RootRoute::AudioPage { .. } => RiseBugReportSurface::Music,

            RootRoute::VacancyDetail { .. }
            | RootRoute::VacancyList
            | RootRoute::CreateVacancy
            | RootRoute::EditVacancy { .. }
            | RootRoute::CreateResume
            | RootRoute::MyResume
            | RootRoute::MyVacancies
            | RootRoute::VacancySettings => RiseBugReportSurface::Vacancy,
        }
    }
}

impl MainTab {
    pub fn bug_report_surface(self) -> RiseBugReportSurface {
        match self {
            MainTab::Posts => RiseBugReportSurface::Feed,
            MainTab::GlobalSearch => RiseBugReportSurface::Search,
            MainTab::Chats => RiseBugReportSurface::ChatList,
            MainTab::Notifications => RiseBugReportSurface::Notifications,
            MainTab::Shorts => RiseBugReportSurface::Shorts,
            MainTab::Vacancies => RiseBugReportSurface::Vacancy,
            MainTab::Profile => RiseBugReportSurface::Profile,
            MainTab::Music => RiseBugReportSurface::Music,
            MainTab::Maps => RiseBugReportSurface::Profile,
        }
    }
}

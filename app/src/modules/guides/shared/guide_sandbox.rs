use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, Global, Task, WeakEntity};
use parking_lot::Mutex;
use rise_canon::{ChatPerm, RiseGuideSignal, RiseIdentityCanon, RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_energy::rise_energy_saving::RiseEnergySavingSettings;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use rise_engine::{RiseEngine, RiseJSONValue, RiseWireMethodDescriptor};
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use rise_theme::theme::theme_store::GuideSnapshot as ThemeStoreGuideSnapshot;
use serde_json::Value;

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingHeartbeatExt};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::sai::Sai;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::engine::core::rise_conversation_entity_records::RiseMessageLocalPhase;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationEntityStamp, RiseConversationPresentation, RiseMessagePresentationContentStamp,
    RiseMessageScopePresentationSnapshot, RiseMessageWireSnapshot,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatInfo, ChatSection, ChatTopic, ChatType, User, VoiceChannelConfig,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTO;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;
use crate::modules::guides::engine::rise_guides_models::RiseGuide;
use crate::modules::guides::shared::guide_sandbox_audio::{
    GuideSandboxAudio, GuideSandboxAudioPrepared, GuideSandboxAudioTrack,
};
use crate::modules::guides::shared::guide_sandbox_bots::GuideSandboxBotsScene;
use crate::modules::guides::shared::guide_sandbox_call::{
    GuideSandboxCallLesson, GuideSandboxCallMedia, GuideSandboxCallScene,
};
use crate::modules::guides::shared::guide_sandbox_discussion::GuideSandboxDiscussionScene;
use crate::modules::guides::shared::guide_sandbox_folders::GuideSandboxFoldersScene;
use crate::modules::guides::shared::guide_sandbox_jobs::GuideSandboxJobsScene;
use crate::modules::guides::shared::guide_sandbox_map::GuideSandboxMapScene;
use crate::modules::guides::shared::guide_sandbox_mute::GuideSandboxMuteScene;
use crate::modules::guides::shared::guide_sandbox_photo::GuideSandboxPhotoScene;
use crate::modules::guides::shared::guide_sandbox_posts::GuideSandboxPostsScene;
use crate::modules::guides::shared::guide_sandbox_roles::GuideSandboxRolesScene;
use crate::modules::guides::shared::guide_sandbox_settings::{
    GuideSandboxAchievementsScene, GuideSandboxAppWallpaperScene, GuideSandboxProfileLookScene,
    GuideSandboxSessionsScene,
};
use crate::modules::guides::shared::guide_sandbox_stickers::GuideSandboxStickersScene;
use crate::modules::guides::shared::guide_sandbox_stories::GuideSandboxStoriesScene;
use crate::modules::guides::shared::guide_sandbox_wallpaper::GuideSandboxWallpaperScene;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::MusicAudioEffectSettings;
use crate::modules::music::shared::utils::music::music_effect_file_player::MusicEffectPlaybackLimits;
use crate::modules::music::shared::utils::music::music_models::{MusicRepeatMode, TrackModel};
use crate::modules::music::shared::utils::music::music_player_state::{
    MusicListeningPermissionRequestNotice, MusicPlayerState,
};
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningAudioEffect, MusicListeningParticipant, MusicListeningParticipantPermissions,
    MusicPlaylistInviteLink, MusicPlaylistJoinRequest, MusicPlaylistListener, MusicPlaylistListeningSession,
    MusicPlaylistListeningSnapshot, MusicPlaylistMember, MusicPlaylistMemberPermissions, MusicPlaylistSettings,
    MusicPlaylistSummary, MusicUserReference,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    RiseStoredMusicPlaylist, RiseStoredMusicTrack, WireArtistDTO, WireLocalPlaylistDestinationDTO,
};
use crate::modules::music::widgets::bottomsheets::playlists::playlist_visibility::PlaylistVisibility;
use crate::modules::notify::stores::notify_actions::notify_profile_settings_wire_types::ProfileNotificationSettingsModel;
use crate::modules::rtc::engine::core::rise_rtc_store_layout::{RiseRTCRoomMeta, RiseRTCRoomRepositorySnapshot};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCConnectionState, RiseRTCModerationAction, RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRaisedHand,
    RiseRTCRoomConfig, RiseRTCRoomMode, RiseRTCStateError, RiseRTCUserProfile,
};
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCPresentation;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::stores::user::user_actions::user_types::{GoalItem, PlanItem};

const LOG_NAME: &str = "Guides";

impl GuideSandbox {
    pub fn chat_id() -> String {
        format!("{}guide_chat", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn playlist_id() -> String {
        format!("{}guide_playlist", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn message_id() -> String {
        format!("{}guide_message", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn media_id() -> String {
        format!("{}guide_audio", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn track_id() -> String {
        format!("{}guide_track", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the mp3 the reader picks through the plus of the guide playlist, it never leaves the computer
    pub fn reader_track_id() -> String {
        format!("{}guide_reader_track", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// panda own local playlist where the reader is a member, named by the join lesson
    pub fn shared_playlist_id() -> String {
        format!("{}guide_shared_playlist", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn session_id() -> String {
        format!("{}guide_session", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn panda_user_id() -> String {
        format!("{}guide_panda", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn join_request_id() -> String {
        format!("{}guide_join_request", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the reader own community with topics plus its two rooms, named by the voice lessons
    pub fn forum_chat_id() -> String {
        format!("{}guide_forum", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn forum_section_id() -> String {
        format!("{}guide_forum_section", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn stage_topic_id() -> String {
        format!("{}guide_stage", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn voice_topic_id() -> String {
        format!("{}guide_voice", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn stage_room_id() -> String {
        format!("{}guide_stage_room", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn voice_room_id_value() -> String {
        format!("{}guide_voice_room", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the map scene, panda as a friend on the map with her request and the reader invented
    /// place and home
    /// the map screen has no id so a step names the scene in sandbox
    pub fn map_scene_id() -> String {
        format!("{}guide_map", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn map_place_id() -> String {
        format!("{}guide_place", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn map_home_id() -> String {
        format!("{}guide_home", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn map_request_id() -> String {
        format!("{}guide_map_request", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the roles scene, the community gets roles and a member list and a text channel
    /// named in sandbox on a route into the community
    pub fn roles_scene_id() -> String {
        format!("{}guide_roles", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn team_topic_id() -> String {
        format!("{}guide_team", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn everyone_role_id() -> String {
        format!("{}guide_role_everyone", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn moderators_role_id() -> String {
        format!("{}guide_role_moderators", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn friends_role_id() -> String {
        format!("{}guide_role_friends", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the peer scene, the invented conversation is a private chat with panda so her profile
    /// carries the contact rows
    pub fn peer_scene_id() -> String {
        format!("{}guide_peer", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the wallpaper scene, the invented conversation gets a catalogue of pictures to pin
    pub fn wallpaper_scene_id() -> String {
        format!("{}guide_wallpaper", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the discussion scene, the reader own channel and a group to take its comments
    pub fn discussion_scene_id() -> String {
        format!("{}guide_discussion", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn channel_id() -> String {
        format!("{}guide_channel", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn discussion_group_id() -> String {
        format!("{}guide_group", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the folders scene, a folder the reader creates is kept on the computer
    pub fn folders_scene_id() -> String {
        format!("{}guide_folders", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn folder_id() -> String {
        format!("{}guide_folder", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the photo scene, the invented conversation holds one photo the reader sent
    pub fn photo_scene_id() -> String {
        format!("{}guide_photo", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn photo_message_id() -> String {
        format!("{}guide_photo_message", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn photo_media_id() -> String {
        format!("{}guide_photo_media", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the stories scene, the reader profile gets practice stories and highlights
    pub fn stories_scene_id() -> String {
        format!("{}guide_stories", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the close friends scene, panda among the suggestions and the list kept on the computer
    pub fn close_friends_scene_id() -> String {
        format!("{}guide_close_friends", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the posts scene, the reader post with statistics and panda post with an invitation
    pub fn posts_scene_id() -> String {
        format!("{}guide_posts", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn own_post_id() -> String {
        format!("{}guide_post", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn coauthor_post_id() -> String {
        format!("{}guide_coauthor_post", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the sound scene, panda original sound on its sound page
    pub fn sound_scene_id() -> String {
        format!("{}guide_sound", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn sound_track_id() -> String {
        format!("{}guide_sound_track", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the composer scene, every option of a new post published nowhere
    pub fn composer_scene_id() -> String {
        format!("{}guide_composer", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// settings lessons, each scene notes the reader settings when the lesson starts and puts
    /// them back when it ends
    /// anything that would reach the server is answered on the computer
    pub fn energy_scene_id() -> String {
        format!("{}guide_energy", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn theme_scene_id() -> String {
        format!("{}guide_theme", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn app_wallpaper_scene_id() -> String {
        format!("{}guide_app_wallpaper", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn profile_look_scene_id() -> String {
        format!("{}guide_profile_look", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn sessions_scene_id() -> String {
        format!("{}guide_sessions", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn goals_scene_id() -> String {
        format!("{}guide_goals", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn bug_report_scene_id() -> String {
        format!("{}guide_bug_report", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn achievements_scene_id() -> String {
        format!("{}guide_achievements", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn stickers_scene_id() -> String {
        format!("{}guide_stickers", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn sticker_pack_id() -> String {
        format!("{}guide_sticker_pack", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the playlist members scene, panda is already a member of the practice playlist and her
    /// rights change on the computer
    pub fn playlist_members_scene_id() -> String {
        format!("{}guide_playlist_members", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the profile tracks scene, the reader pinned tracks are noted and put back
    pub fn profile_tracks_scene_id() -> String {
        format!("{}guide_profile_tracks", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the jobs lessons, the resume filled in on the computer panda vacancy and the reader own
    pub fn resume_scene_id() -> String {
        format!("{}guide_resume", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    pub fn jobs_scene_id() -> String {
        format!("{}guide_jobs", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the bot lessons, the practice bot with its commands and webhook and mini apps
    pub fn bots_scene_id() -> String {
        format!("{}guide_bots", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// the call lessons, a practice call with panda from her profile in the invented chat
    pub fn call_scene_id() -> String {
        format!("{}guide_call", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }

    /// mute all and its exceptions answered on the computer with panda chat in the list
    pub fn mute_scene_id() -> String {
        format!("{}guide_mute", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX)
    }
}

const PRACTICE_CALL_ID: &str = "5e1f0c4a-6b1d-4c7e-9a5d-00000c0ffee1";

pub const SCENE_PARAM: &str = "sandbox";

const PANDA_REACTION_MS: u64 = 1_500;

static SENT_AT: LazyLock<Date> = LazyLock::new(RiseClock::now);

static FIXTURE_IDS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    [
        GuideSandbox::chat_id(),
        GuideSandbox::playlist_id(),
        GuideSandbox::message_id(),
        GuideSandbox::media_id(),
        GuideSandbox::track_id(),
        GuideSandbox::reader_track_id(),
        GuideSandbox::shared_playlist_id(),
        GuideSandbox::session_id(),
        GuideSandbox::panda_user_id(),
        GuideSandbox::join_request_id(),
        GuideSandbox::invite_token(),
        GuideSandbox::forum_chat_id(),
        GuideSandbox::forum_section_id(),
        GuideSandbox::stage_topic_id(),
        GuideSandbox::voice_topic_id(),
        GuideSandbox::stage_room_id(),
        GuideSandbox::voice_room_id_value(),
        GuideSandbox::map_scene_id(),
        GuideSandbox::map_place_id(),
        GuideSandbox::map_home_id(),
        GuideSandbox::map_request_id(),
        GuideSandbox::roles_scene_id(),
        GuideSandbox::team_topic_id(),
        GuideSandbox::everyone_role_id(),
        GuideSandbox::moderators_role_id(),
        GuideSandbox::friends_role_id(),
        GuideSandbox::peer_scene_id(),
        GuideSandbox::wallpaper_scene_id(),
        GuideSandbox::discussion_scene_id(),
        GuideSandbox::channel_id(),
        GuideSandbox::discussion_group_id(),
        GuideSandbox::folders_scene_id(),
        GuideSandbox::folder_id(),
        GuideSandbox::photo_scene_id(),
        GuideSandbox::photo_message_id(),
        GuideSandbox::photo_media_id(),
        GuideSandbox::stories_scene_id(),
        GuideSandbox::close_friends_scene_id(),
        GuideSandbox::posts_scene_id(),
        GuideSandbox::own_post_id(),
        GuideSandbox::coauthor_post_id(),
        GuideSandbox::sound_scene_id(),
        GuideSandbox::sound_track_id(),
        GuideSandbox::composer_scene_id(),
        GuideSandbox::energy_scene_id(),
        GuideSandbox::theme_scene_id(),
        GuideSandbox::app_wallpaper_scene_id(),
        GuideSandbox::profile_look_scene_id(),
        GuideSandbox::sessions_scene_id(),
        GuideSandbox::goals_scene_id(),
        GuideSandbox::bug_report_scene_id(),
        GuideSandbox::achievements_scene_id(),
        GuideSandbox::stickers_scene_id(),
        GuideSandbox::sticker_pack_id(),
        GuideSandbox::playlist_members_scene_id(),
        GuideSandbox::profile_tracks_scene_id(),
        GuideSandbox::resume_scene_id(),
        GuideSandbox::jobs_scene_id(),
        GuideSandbox::bots_scene_id(),
        GuideSandbox::practice_bot_id(),
        GuideSandbox::practice_grid_id(),
        GuideSandbox::practice_journey_id(),
        GuideSandbox::call_scene_id(),
        GuideSandbox::practice_call_id(),
        GuideSandbox::practice_call_room_id(),
        GuideSandbox::mute_scene_id(),
    ]
    .into_iter()
    .collect()
});

static SETTINGS_SCENE_IDS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    [
        GuideSandbox::energy_scene_id(),
        GuideSandbox::theme_scene_id(),
        GuideSandbox::app_wallpaper_scene_id(),
        GuideSandbox::profile_look_scene_id(),
        GuideSandbox::sessions_scene_id(),
        GuideSandbox::goals_scene_id(),
        GuideSandbox::bug_report_scene_id(),
        GuideSandbox::achievements_scene_id(),
        GuideSandbox::stickers_scene_id(),
        GuideSandbox::profile_tracks_scene_id(),
        GuideSandbox::resume_scene_id(),
        GuideSandbox::jobs_scene_id(),
        GuideSandbox::bots_scene_id(),
        GuideSandbox::call_scene_id(),
        GuideSandbox::mute_scene_id(),
    ]
    .into_iter()
    .collect()
});

static ARMED_FLAG: AtomicBool = AtomicBool::new(false);

static WALL_INSTALLED: AtomicBool = AtomicBool::new(false);

static REFUSED_METHODS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// what a guide is allowed to invent and where the invention stops
/// guide walks the real app, only data under it is substituted
/// armed by temp ids in its route, requests naming them are refused
pub struct GuideSandbox {
    pub(crate) is_armed: bool,
    playlist_title: Option<String>,
    holds_track: bool,
    reader_track: Option<RiseStoredMusicTrack>,
    reader_track_files: Vec<PathBuf>,

    effect_settings_before_guide: Option<MusicAudioEffectSettings>,

    uses_shared_playlist: bool,
    panda_joins_on_play: bool,
    panda_requested_to_join: bool,
    panda_requested_at_ms: i64,
    panda_accepted: bool,
    panda_permissions: MusicPlaylistMemberPermissions,
    panda_member_version: i64,
    session: Option<MusicPlaylistListeningSession>,
    session_revision: i64,
    panda_listening_since_ms: i64,
    pub(crate) scene_tasks: Vec<Task<()>>,

    uses_forum: bool,
    joined_room_id: Option<String>,
    room_seats: Vec<GuideSandboxVoiceSeat>,
    room_revision: i64,
    room_opened_at_ms: i64,

    pub(crate) map: Option<GuideSandboxMapScene>,
    pub(crate) map_tasks: Vec<Task<()>>,
    pub(crate) roles: Option<GuideSandboxRolesScene>,
    pub(crate) scenes: HashSet<String>,
    pub(crate) panda_block_scope: Option<RiseBlockScope>,
    pub(crate) wallpaper: Option<GuideSandboxWallpaperScene>,
    pub(crate) discussion: Option<GuideSandboxDiscussionScene>,
    pub(crate) folder_scene: Option<GuideSandboxFoldersScene>,
    pub(crate) photo: Option<GuideSandboxPhotoScene>,
    pub(crate) stories_scene: Option<GuideSandboxStoriesScene>,
    pub(crate) posts_scene: Option<GuideSandboxPostsScene>,
    pub(crate) energy_before: Option<RiseEnergySavingSettings>,
    pub(crate) sessions_scene: Option<GuideSandboxSessionsScene>,
    pub(crate) goals_before: Option<(Vec<GoalItem>, Vec<PlanItem>)>,
    pub(crate) bug_report_scene: bool,
    pub(crate) panda_notifications: Option<ProfileNotificationSettingsModel>,
    pub(crate) achievements_scene: Option<GuideSandboxAchievementsScene>,
    pub(crate) theme_before: Option<ThemeStoreGuideSnapshot>,
    pub(crate) app_wallpaper_scene: Option<GuideSandboxAppWallpaperScene>,
    pub(crate) profile_look_scene: Option<GuideSandboxProfileLookScene>,
    pub(crate) stickers_scene: Option<GuideSandboxStickersScene>,
    pub(crate) featured_tracks_before: Option<(String, Vec<TrackModel>)>,
    pub(crate) jobs_scene: Option<GuideSandboxJobsScene>,
    pub(crate) bots_scene: Option<GuideSandboxBotsScene>,
    pub(crate) call_scene: Option<GuideSandboxCallScene>,
    pub(crate) mute_scene: Option<GuideSandboxMuteScene>,
    pub(crate) call_lesson: Option<GuideSandboxCallLesson>,
    pub(crate) call_media: GuideSandboxCallMedia,

    scene_track: GuideSandboxAudioTrack,
    pub(crate) audio: Option<GuideSandboxAudioPrepared>,
    audio_settled: bool,
    audio_task: Option<Task<()>>,
}

struct GuideSandboxExisting(WeakEntity<GuideSandbox>);

impl Global for GuideSandboxExisting {}

impl GuideSandbox {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    /// the instance if one was built, for slot readers that hold only the app
    pub fn existing(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GuideSandboxExisting>().and_then(|existing| existing.0.upgrade())
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let weak = cx.weak_entity();
        cx.set_global(GuideSandboxExisting(weak));
        Self {
            is_armed: false,
            playlist_title: None,
            holds_track: false,
            reader_track: None,
            reader_track_files: Vec::new(),
            effect_settings_before_guide: None,
            uses_shared_playlist: false,
            panda_joins_on_play: false,
            panda_requested_to_join: false,
            panda_requested_at_ms: 0,
            panda_accepted: false,
            panda_permissions: MusicPlaylistMemberPermissions::NONE,
            panda_member_version: 1,
            session: None,
            session_revision: 1,
            panda_listening_since_ms: 0,
            scene_tasks: Vec::new(),
            uses_forum: false,
            joined_room_id: None,
            room_seats: Vec::new(),
            room_revision: 0,
            room_opened_at_ms: 0,
            map: None,
            map_tasks: Vec::new(),
            roles: None,
            scenes: HashSet::new(),
            panda_block_scope: None,
            wallpaper: None,
            discussion: None,
            folder_scene: None,
            photo: None,
            stories_scene: None,
            posts_scene: None,
            energy_before: None,
            sessions_scene: None,
            goals_before: None,
            bug_report_scene: false,
            panda_notifications: None,
            achievements_scene: None,
            theme_before: None,
            app_wallpaper_scene: None,
            profile_look_scene: None,
            stickers_scene: None,
            featured_tracks_before: None,
            jobs_scene: None,
            bots_scene: None,
            call_scene: None,
            mute_scene: None,
            call_lesson: None,
            call_media: GuideSandboxCallMedia::new(),
            scene_track: GuideSandboxAudioTrack::RiseUp,
            audio: None,
            audio_settled: false,
            audio_task: None,
        }
    }

    /// invite token in the canon form uuid.version.64hex
    /// a temp_ token isnt a playlist reference so the link builder returned nothing and the
    /// lesson showed an empty field
    pub fn invite_token() -> String {
        format!("5e1f0c4a-6b1d-4c7e-9a5d-0000006a1de5.1.{}67756964", "0".repeat(56))
    }

    pub fn practice_call_id() -> String {
        PRACTICE_CALL_ID.to_owned()
    }

    pub fn practice_call_room_id() -> String {
        format!("rtc:call:{PRACTICE_CALL_ID}")
    }

    /// every id the sandbox invents, the wall matches request bodies against this set
    pub fn fixture_ids() -> &'static HashSet<String> {
        &FIXTURE_IDS
    }

    pub fn settings_scene_ids() -> &'static HashSet<String> {
        &SETTINGS_SCENE_IDS
    }

    /// track length as the asset reports it so the bubble and the playlist row cant disagree
    pub fn track_duration_ms(&self) -> i64 {
        self.audio.as_ref().map_or(0, |audio| audio.duration_ms)
    }

    /// the same stamp for rows other files invent so every row of a scene reads as one moment
    pub fn scene_stamp() -> f64 {
        SENT_AT.time_interval_since_1970()
    }

    fn sent_at_stamp_ms() -> i64 {
        SENT_AT.ms()
    }

    pub fn is_armed(&self) -> bool {
        self.is_armed
    }

    pub fn playlist_title(&self) -> Option<&str> {
        self.playlist_title.as_deref()
    }

    pub fn holds_track(&self) -> bool {
        self.holds_track
    }

    pub fn reader_track(&self) -> Option<&RiseStoredMusicTrack> {
        self.reader_track.as_ref()
    }

    pub fn session(&self) -> Option<&MusicPlaylistListeningSession> {
        self.session.as_ref()
    }

    pub fn scenes(&self) -> &HashSet<String> {
        &self.scenes
    }

    pub fn scene_track(&self) -> GuideSandboxAudioTrack {
        self.scene_track
    }

    pub fn audio(&self) -> Option<&GuideSandboxAudioPrepared> {
        self.audio.as_ref()
    }

    pub fn has_playlist(&self) -> bool {
        self.playlist_title.is_some()
    }

    pub fn is_armed_for_transport() -> bool {
        ARMED_FLAG.load(Ordering::Acquire)
    }

    pub fn refused_methods() -> &'static Mutex<HashSet<String>> {
        &REFUSED_METHODS
    }

    /// adds a scene methods to the refused set, disarm clears the whole set
    pub fn refuse_methods(methods: &HashSet<String>) {
        Self::refused_methods().lock().extend(methods.iter().cloned());
    }

    pub fn is_wanted(guide: &RiseGuide) -> bool {
        let wanted = [
            Self::chat_id(),
            Self::playlist_id(),
            Self::shared_playlist_id(),
            Self::forum_chat_id(),
            Self::map_scene_id(),
            Self::roles_scene_id(),
            Self::peer_scene_id(),
            Self::wallpaper_scene_id(),
            Self::channel_id(),
            Self::folders_scene_id(),
            Self::photo_scene_id(),
            Self::stories_scene_id(),
            Self::close_friends_scene_id(),
            Self::posts_scene_id(),
            Self::sound_scene_id(),
            Self::composer_scene_id(),
        ];
        guide.steps.iter().any(|step| {
            step.route_params
                .values()
                .any(|value| wanted.contains(value) || Self::settings_scene_ids().contains(value))
        })
    }

    fn names_route_value(guide: &RiseGuide, id: &str) -> bool {
        guide.steps.iter().any(|step| step.route_params.values().any(|value| value == id))
    }

    /// arms the sandbox the way this guide needs it
    /// playlist stays empty until the reader creates it only when the guide teaches creation
    /// effect chain is cleared for the guide and restored by disarm
    pub fn arm(&mut self, guide: &RiseGuide, cx: &mut Context<Self>) {
        self.reset(cx);
        self.is_armed = true;
        ARMED_FLAG.store(true, Ordering::Release);
        Self::install_transport_wall_once();
        let teaches_playlist_creation =
            guide.steps.iter().any(|step| step.advance_signal == Some(RiseGuideSignal::PlaylistCreated));
        if !teaches_playlist_creation {
            self.playlist_title = Some(tr("music_default_local_playlist_title"));
            self.holds_track = true;
        }
        self.uses_shared_playlist = Self::names_route_value(guide, &Self::shared_playlist_id());
        self.panda_joins_on_play = !self.uses_shared_playlist
            && guide.steps.iter().any(|step| step.advance_signal == Some(RiseGuideSignal::ListeningSessionActive));
        self.scene_track = if self.uses_shared_playlist || self.panda_joins_on_play {
            GuideSandboxAudioTrack::Chillhop
        } else {
            GuideSandboxAudioTrack::RiseUp
        };
        self.panda_listening_since_ms = self.now_ms();
        self.scenes = guide.steps.iter().filter_map(|step| step.route_params.get(SCENE_PARAM).cloned()).collect();
        if self.scenes.contains(&Self::call_scene_id()) {
            self.scenes.insert(Self::peer_scene_id());
            self.call_lesson = Some(GuideSandboxCallLesson::new(guide));
        }
        if self.scenes.contains(&Self::playlist_members_scene_id()) {
            self.panda_accepted = true;
        }
        self.uses_forum = Self::names_route_value(guide, &Self::forum_chat_id());
        if self.uses_forum && self.scenes.contains(&Self::roles_scene_id()) {
            self.arm_roles(cx);
        }
        if self.scenes.contains(&Self::wallpaper_scene_id()) {
            self.arm_wallpaper(cx);
        }
        if self.scenes.contains(&Self::discussion_scene_id()) {
            self.discussion = Some(GuideSandboxDiscussionScene::default());
            self.publish_discussion(cx);
        }
        if self.scenes.contains(&Self::folders_scene_id()) {
            self.folder_scene = Some(GuideSandboxFoldersScene::default());
        }
        if self.scenes.contains(&Self::photo_scene_id()) {
            self.arm_photo(cx);
        }
        self.arm_gallery_photo(guide, cx);
        if self.scenes.contains(&Self::stories_scene_id()) {
            self.arm_stories(cx);
        }
        if self.scenes.contains(&Self::posts_scene_id()) {
            self.posts_scene = Some(GuideSandboxPostsScene::default());
        }
        self.arm_settings_scenes(cx);
        self.arm_bots(guide, cx);
        if self.uses_forum {
            self.publish_forum(cx);
        }
        if Self::wants_map(guide) {
            self.arm_map(guide, cx);
        }
        let player = MusicPlayerState::shared(cx);
        let effects = player.read(cx).audio_effect_settings().clone();
        if effects.is_any_enabled() {
            self.effect_settings_before_guide = Some(effects);
            player.update(cx, |player, cx| player.restore_audio_effect_settings(MusicAudioEffectSettings::off(), cx));
        }
        self.prepare_audio(cx);
        cx.notify();
    }

    pub fn disarm(&mut self, cx: &mut Context<Self>) {
        self.is_armed = false;
        ARMED_FLAG.store(false, Ordering::Release);
        self.audio_task = None;
        self.withdraw_conversation(cx);
        self.withdraw_playback(cx);
        self.withdraw_voice_room(cx);
        self.withdraw_forum(cx);
        self.withdraw_team_channel(cx);
        self.withdraw_wallpaper(cx);
        self.withdraw_discussion(cx);
        self.withdraw_photo();
        Self::withdraw_gallery_photo();
        self.withdraw_stories(cx);
        self.withdraw_settings_scenes(cx);
        self.withdraw_map(cx);
        self.reset(cx);
        cx.notify();
    }

    fn withdraw_playback(&mut self, cx: &mut Context<Self>) {
        let player = MusicPlayerState::shared(cx);
        let session_id = Self::session_id();
        let track_id = Self::track_id();
        let reader_track_id = Self::reader_track_id();
        let before = self.effect_settings_before_guide.take();
        self.session = None;
        player.update(cx, |player, cx| {
            if player.listening_session.as_ref().is_some_and(|session| session.session_id == session_id) {
                player.detach_listening_session_locally(cx);
            }
            let current = player.current_track.as_ref().map(|track| track.id.clone());
            if current.as_deref() == Some(track_id.as_str()) || current.as_deref() == Some(reader_track_id.as_str()) {
                player.stop_and_clear(cx);
                player.set_show_audio_effects_sheet(false, cx);
                player.set_show_now_playing(false, cx);
            }
            if let Some(before) = before {
                player.restore_audio_effect_settings(before, cx);
            }
        });
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.scene_tasks.clear();
        self.uses_shared_playlist = false;
        self.panda_joins_on_play = false;
        self.panda_requested_to_join = false;
        self.panda_requested_at_ms = 0;
        self.panda_accepted = false;
        self.panda_permissions = MusicPlaylistMemberPermissions::NONE;
        self.panda_member_version = 1;
        self.session = None;
        self.session_revision = 1;
        self.uses_forum = false;
        self.joined_room_id = None;
        self.room_seats.clear();
        self.room_revision = 0;
        self.room_opened_at_ms = 0;
        self.playlist_title = None;
        self.holds_track = false;
        self.reader_track = None;
        let files = std::mem::take(&mut self.reader_track_files);
        if !files.is_empty() {
            cx.background_spawn(async move {
                for file in files {
                    FileManager.remove_item(&file).ok();
                }
            })
            .detach();
        }
        self.scene_track = GuideSandboxAudioTrack::RiseUp;
        self.audio = None;
        self.audio_settled = false;
        self.map_tasks.clear();
        self.map = None;
        self.roles = None;
        self.scenes.clear();
        self.panda_block_scope = None;
        self.wallpaper = None;
        self.discussion = None;
        self.folder_scene = None;
        self.photo = None;
        self.stories_scene = None;
        self.posts_scene = None;
        self.energy_before = None;
        self.sessions_scene = None;
        self.goals_before = None;
        self.bug_report_scene = false;
        self.panda_notifications = None;
        self.achievements_scene = None;
        self.theme_before = None;
        self.app_wallpaper_scene = None;
        self.profile_look_scene = None;
        self.stickers_scene = None;
        self.featured_tracks_before = None;
        self.jobs_scene = None;
        self.bots_scene = None;
        self.call_scene = None;
        self.call_lesson = None;
        self.mute_scene = None;
        Self::refused_methods().lock().clear();
        GuideFacts::set(RiseGuideSignal::HoldMenuOpened, false, cx);
    }

    /// finds the track off the main thread then publishes the conversation that carries it
    fn prepare_audio(&mut self, cx: &mut Context<Self>) {
        let track = self.scene_track;
        let prepare = GuideSandboxAudio::prepare(track, cx);
        self.audio_task = Some(cx.spawn(async move |this, cx| {
            let prepared = prepare.await;
            this.update(cx, |sandbox, cx| {
                if !sandbox.is_armed || sandbox.scene_track != track {
                    return;
                }
                sandbox.audio = prepared;
                sandbox.audio_settled = true;
                sandbox.publish_conversation(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn install_transport_wall_once() {
        if WALL_INSTALLED.swap(true, Ordering::AcqRel) {
            return;
        }
        let next = RiseEngine::outbound_guard();
        RiseEngine::set_outbound_guard(Some(Arc::new(
            move |descriptor: &RiseWireMethodDescriptor, request: &RiseJSONValue| -> Option<anyhow::Error> {
            if !(Self::refuses_json(request) || Self::refuses_service_method(&descriptor.service, &descriptor.method)) {
                return next.as_ref().and_then(|next| next(descriptor, request));
            }
            let route = format!("{}.{}", descriptor.service, descriptor.method);
            if let Some(hop) = RiseEngineMainHop::installed() {
                hop.post(move |cx| {
                    Sai::log(LOG_NAME, || format!("sandbox refused {route}"), SaiLogKind::Custom("guide".to_owned()), cx);
                });
            }
            Some(anyhow::Error::new(SaiWsSendError::GuideSandbox))
            },
        )));
    }

    pub fn refuses_service_method(service: &str, method: &str) -> bool {
        Self::is_armed_for_transport() && Self::refused_methods().lock().contains(&format!("{service}.{method}"))
    }

    /// whether an outbound body names an invented id while the sandbox is armed
    pub fn refuses_json_object(json_object: &Value) -> bool {
        Self::is_armed_for_transport() && Self::names_fixture_object(json_object)
    }

    pub fn refuses_json(value: &RiseJSONValue) -> bool {
        Self::is_armed_for_transport() && Self::names_fixture_json(value)
    }

    fn names_fixture_object(value: &Value) -> bool {
        match value {
            Value::String(text) => Self::fixture_ids().contains(text),
            Value::Array(list) => list.iter().any(Self::names_fixture_object),
            Value::Object(dictionary) => dictionary.values().any(Self::names_fixture_object),
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }

    fn names_fixture_json(value: &RiseJSONValue) -> bool {
        match value {
            RiseJSONValue::String(text) => Self::fixture_ids().contains(text),
            RiseJSONValue::Array(list) => list.iter().any(Self::names_fixture_json),
            RiseJSONValue::Object(dictionary) => dictionary.values().any(Self::names_fixture_json),
            RiseJSONValue::Null
            | RiseJSONValue::Bool(_)
            | RiseJSONValue::Integer(_)
            | RiseJSONValue::UnsignedInteger(_)
            | RiseJSONValue::Number(_) => false,
        }
    }

    pub fn owns_playlist_id(&self, id: &str) -> bool {
        self.is_armed && (id == Self::playlist_id() || id == Self::shared_playlist_id())
    }

    pub fn owns_session_id(&self, id: &str) -> bool {
        self.is_armed && id == Self::session_id()
    }

    pub fn owns_chat_id(&self, id: Option<&str>) -> bool {
        self.is_armed && id == Some(Self::chat_id().as_str())
    }

    pub fn owns_track_id(&self, id: &str) -> bool {
        self.is_armed && (id == Self::track_id() || id == Self::reader_track_id())
    }

    /// the invented message is a real message to the hold menu and gets the same rows
    /// every row that would reach the server is stopped by the wall not hidden
    pub fn owns_message_id(&self, id: &str) -> bool {
        self.is_armed && (id == Self::message_id() || (self.photo.is_some() && id == Self::photo_message_id()))
    }

    /// the reader submitted the real create sheet and nothing goes out
    /// still emits playlist_created since a local playlist now exists on this screen
    pub fn create_playlist(&mut self, title: &str, cx: &mut Context<Self>) {
        if !self.is_armed {
            return;
        }
        let trimmed = trimming_whitespaces_and_newlines(title);
        self.playlist_title =
            Some(if trimmed.is_empty() { tr("music_default_local_playlist_title") } else { trimmed.to_owned() });
        GuideSignalBus::emit(RiseGuideSignal::PlaylistCreated);
        cx.notify();
    }

    pub fn add_track_to_playlist(&mut self, cx: &mut Context<Self>) {
        if !self.is_armed {
            return;
        }
        if !self.has_playlist() {
            self.create_playlist("", cx);
        }
        self.holds_track = true;
        GuideSignalBus::emit(RiseGuideSignal::TrackAddedToPlaylist);
        cx.notify();
    }

    /// the playlist as the wire would have described it
    /// visibility must be local and owner_user_id the reader own, the detail page reads that
    /// pair to show the plus button
    pub fn playlist(&self, id: &str, cx: &mut App) -> Option<RiseStoredMusicPlaylist> {
        if !self.is_armed {
            return None;
        }
        if id == Self::shared_playlist_id() {
            if !self.uses_shared_playlist {
                return None;
            }
            let tracks = vec![self.track_dto(cx)];
            return Some(Self::playlist_dto(
                id,
                &Self::panda_user_id(),
                &tr("guide_scene_shared_playlist_title"),
                tracks,
            ));
        }
        let title = self.playlist_title.as_deref().filter(|_| id == Self::playlist_id())?;
        let mut tracks = if self.holds_track { vec![self.track_dto(cx)] } else { Vec::new() };
        tracks.extend(self.reader_track.iter().cloned());
        Some(Self::playlist_dto(id, &self.viewer_user_id(cx), title, tracks))
    }

    /// the reader saved their own mp3 into the guide playlist
    /// it stays a file on the computer, played from there and goes away with the guide
    pub fn add_reader_track(
        &mut self,
        title: &str,
        artist_name: &str,
        duration_ms: i64,
        audio_file: Url,
        cover_file: Option<Url>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_armed {
            return;
        }
        if !self.has_playlist() {
            self.create_playlist("", cx);
        }
        let scratch = rise_platform::paths::temporary_directory();
        let scratch_files = [Some(&audio_file), cover_file.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|url| url.standardized().to_file_path())
            .filter(|path| path.starts_with(&scratch));
        self.reader_track_files.extend(scratch_files);
        self.reader_track = Some(RiseStoredMusicTrack {
            id: Some(Self::reader_track_id()),
            title: Some(title.to_owned()),
            description: None,
            duration_ms: Some(duration_ms),
            cover_url: cover_file.as_ref().map(|url| url.absolute_string().to_owned()),
            audio_url: Some(audio_file.absolute_string().to_owned()),
            play_count: Some(0),
            like_count: Some(0),
            is_liked: Some(false),
            album: None,
            artist: Some(WireArtistDTO { stage_name: artist_name.to_owned(), ..WireArtistDTO::default() }),
            created_at_ms: Some(self.now_ms()),
            status: Some(LOCAL_LIBRARY_STATUS.to_owned()),
            genre: None,
            rejection_reason: None,
            scheduled_at_ms: None,
            is_exclusive: Some(false),
            library_owner_user_id: AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire(),
            ..RiseStoredMusicTrack::default()
        });
        GuideSignalBus::emit(RiseGuideSignal::TrackAddedToPlaylist);
        cx.notify();
    }

    fn playlist_dto(id: &str, owner_user_id: &str, title: &str, tracks: Vec<RiseStoredMusicTrack>) -> RiseStoredMusicPlaylist {
        let now = Self::sent_at_stamp_ms();
        RiseStoredMusicPlaylist {
            id: Some(id.to_owned()),
            owner_user_id: Some(owner_user_id.to_owned()),
            title: Some(title.to_owned()),
            description: Some(String::new()),
            cover_url: None,
            visibility: Some(PlaylistVisibility::Local.raw_value().to_owned()),
            tracks: Some(tracks),
            created_at_ms: Some(now),
            updated_at_ms: Some(now),
            revision: Some(1),
            is_default_local: Some(false),
            ..RiseStoredMusicPlaylist::default()
        }
    }

    /// the row the real mp3 import sheet lists as a destination
    pub fn playlist_destination(&self, cx: &mut App) -> Option<WireLocalPlaylistDestinationDTO> {
        let title = self.playlist_title.as_deref().filter(|_| self.is_armed)?;
        Some(WireLocalPlaylistDestinationDTO {
            can_add_tracks: true,
            id: Self::playlist_id(),
            owner_user_id: self.viewer_user_id(cx),
            revision: 1,
            title: title.to_owned(),
            updated_at_ms: Self::sent_at_stamp_ms(),
            viewer_role: OWNER_ROLE.to_owned(),
            ..WireLocalPlaylistDestinationDTO::default()
        })
    }

    /// the track, settled
    /// transfer_phase stays none, merge pipeline strips rows that look like uploads
    /// audio_url is the bundled file or empty, never a made up path
    pub fn track_dto(&self, cx: &mut App) -> RiseStoredMusicTrack {
        let (title, artist) = self.track_naming();
        RiseStoredMusicTrack {
            id: Some(Self::track_id()),
            title: Some(title),
            description: None,
            duration_ms: Some(self.track_duration_ms()),
            cover_url: self.cover_url_string(),
            audio_url: Some(self.audio_url_string().unwrap_or_default()),
            play_count: Some(0),
            like_count: Some(0),
            is_liked: Some(false),
            album: None,
            artist: Some(WireArtistDTO { stage_name: artist, ..WireArtistDTO::default() }),
            created_at_ms: Some(Self::sent_at_stamp_ms()),
            status: Some(LOCAL_LIBRARY_STATUS.to_owned()),
            genre: None,
            rejection_reason: None,
            scheduled_at_ms: None,
            is_exclusive: Some(false),
            library_owner_user_id: AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire(),
            ..RiseStoredMusicTrack::default()
        }
    }

    fn audio_url_string(&self) -> Option<String> {
        let audio = self.audio.as_ref()?;
        Url::from_file_path(&audio.url).map(|url| url.absolute_string().to_owned())
    }

    fn cover_url_string(&self) -> Option<String> {
        let cover = self.audio.as_ref()?.cover_url.as_ref()?;
        Url::from_file_path(cover).map(|url| url.absolute_string().to_owned())
    }

    /// the attachment the invented message carries
    /// the mime says mp3 since that makes the real hold menu offer add to local playlist
    /// remote url stays empty, no file on any server
    pub fn audio_item(&self) -> MessageMediaItem {
        MessageMediaItem {
            thumbnail_url: self.cover_url_string(),
            duration: Some(self.track_duration_ms() as f64 / 1_000.0),
            file_name: Some(tr(self.scene_track.file_name_key())),
            file_size: Some(self.audio.as_ref().map_or(0, |audio| audio.byte_count)),
            mime_type: Some(AUDIO_MIME_TYPE.to_owned()),
            local_preview_url: self.audio_url_string(),
            ..MessageMediaItem::new(Self::media_id(), MessageMediaKind::Audio, String::new())
        }
    }

    pub fn message(&self) -> GetMessageMessage {
        GetMessageMessage {
            chat_id: Self::chat_id(),
            sender_id: Self::chat_id(),
            sender_name: tr("guide_scene_sender"),
            content_type: MessageContentType::Audio,
            text: tr("guide_scene_message_caption"),
            created_at: SENT_AT.time_interval_since_1970(),
            media_items: vec![self.audio_item()],
            ..GetMessageMessage::new(Self::message_id())
        }
    }

    /// panda is the other side of the chat only in lessons about her as a contact
    /// a participant makes the chat screen ask for her presence and profile which older lessons
    /// never needed
    pub fn chat(&self) -> ChatInfo {
        let mut chat = ChatInfo { title: tr("guide_scene_sender"), r#type: ChatType::Direct, ..ChatInfo::new(Self::chat_id()) };
        chat.participant.name = tr("guide_scene_sender");
        chat.participant.tag = tr("guide_scene_sender_tag");
        if self.scenes.contains(&Self::peer_scene_id()) {
            chat.participant = User {
                id: Self::panda_user_id(),
                name: tr("guide_scene_sender"),
                tag: tr("guide_scene_sender_tag"),
                is_online: true,
                ..User::default()
            };
        }
        chat.wallpaper = self.wallpaper.as_ref().and_then(|wallpaper| wallpaper.placement.clone());
        chat
    }

    pub fn track_naming(&self) -> (String, String) {
        ChatAudioTrackAdapter::title_and_artist(&self.audio_item(), "")
    }
}

const LOCAL_LIBRARY_STATUS: &str = "local_library";
const AUDIO_MIME_TYPE: &str = "audio/mpeg";
const OWNER_ROLE: &str = "owner";
const MEMBER_ROLE: &str = "member";
const ACTIVE_STATUS: &str = "active";
const PENDING_STATUS: &str = "pending";

impl GuideSandbox {
    pub fn publish_scene(&mut self, cx: &mut Context<Self>) {
        self.publish_conversation(cx);
        self.publish_forum(cx);
        self.publish_team_channel(cx);
        self.publish_discussion(cx);
    }

    /// sandbox chat id starts with temp_ which gates both data tasks and the send path and
    /// typing and pagination cursors so opening it costs no request
    pub fn publish_conversation(&mut self, cx: &mut Context<Self>) {
        if !self.is_armed || !self.audio_settled {
            return;
        }
        if self.photo.as_ref().is_some_and(|photo| photo.file_url.is_none()) {
            return;
        }
        let presentation = RiseConversationPresentation::shared(cx);
        let Some(account_id) =
            presentation.read(cx).account_id().filter(|id| !id.is_empty()).map(str::to_owned)
        else {
            return;
        };
        let chat = self.chat().to_wire_dto(None);
        let shown = if self.photo.is_none() { self.message() } else { self.photo_message() };
        let edits = self.photo.as_ref().map_or(0, |photo| u64::try_from(photo.edits).unwrap_or(0));
        let scope = RiseMessageScope::new(Self::chat_id(), None, None);
        let stamp = RiseConversationEntityStamp {
            version: i64::try_from(edits).unwrap_or(i64::MAX).saturating_add(1),
            updated_at_milliseconds: (shown.created_at * 1_000.0) as i64,
        };
        let source = RiseMessageWireSnapshot {
            account_id: account_id.clone(),
            scope,
            revision: 1,
            rows: Vec::new(),
            local_ids: Vec::new(),
            entity_stamps: Vec::new(),
            anchor_local_id: Some(shown.id.clone()),
            has_cached_before: false,
            has_cached_after: false,
            before_cursor: None,
            after_cursor: None,
            has_more_before: false,
            has_more_after: false,
            load_state: RiseEntityViewLoadState::Loaded,
            error_code: None,
        };
        let shown_id = shown.id.clone();
        let snapshot = RiseMessageScopePresentationSnapshot {
            revision: u64::from(u32::MAX).saturating_add(edits),
            source,
            messages: vec![shown],
            wires: vec![MessageWireDTO::default()],
            visible_local_ids: vec![shown_id.clone()],
            content_stamps_by_local_id: HashMap::from([(
                shown_id,
                RiseMessagePresentationContentStamp {
                    entity: stamp,
                    local_revision: edits.saturating_add(1),
                    local_phase: RiseMessageLocalPhase::Remote,
                },
            )]),
            anchor_offset: 0,
            pinned_messages: Vec::new(),
            latest_reply_keyboard_message: None,
        };
        presentation.update(cx, |presentation, cx| {
            presentation.publish_chat_detail(chat, &account_id, Some(false), cx);
            presentation.publish_message_scope(snapshot, cx);
        });
    }

    pub fn withdraw_conversation(&mut self, cx: &mut Context<Self>) {
        let scope = RiseMessageScope::new(Self::chat_id(), None, None);
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.remove_message_scope(&scope, cx);
            presentation.remove_chat_detail(&Self::chat_id(), cx);
        });
        ChatScrollPositionStore::clear(&Self::chat_id(), cx);
    }

    pub fn sent_at_ms(&self) -> i64 {
        (self.message().created_at * 1_000.0) as i64
    }
}

impl GuideSandbox {
    fn viewer_user_id(&self, cx: &mut App) -> String {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default()
    }

    pub fn now_ms(&self) -> i64 {
        RiseClock::now().ms()
    }

    /// a lease horizon the ui never prints, the session heartbeat period is the nearest canon
    /// number for how long a room lives unattended
    pub fn lease_ms(&self) -> i64 {
        let nanoseconds = RiseTimingCanon::heartbeat("music_listening_session").client_interval_nanoseconds();
        i64::try_from(nanoseconds / 1_000_000).unwrap_or(i64::MAX)
    }

    pub fn panda_user(&self) -> MusicUserReference {
        MusicUserReference {
            id: Self::panda_user_id(),
            name: tr("guide_scene_sender"),
            tag: tr("guide_scene_sender_tag"),
            logo: String::new(),
            is_premium: false,
            premium_badge_emoji: None,
            name_color: None,
        }
    }

    pub fn viewer_user(&self, cx: &mut App) -> MusicUserReference {
        let id = self.viewer_user_id(cx);
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile();
        MusicUserReference {
            id,
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            logo: profile.map(|profile| profile.more.logo.clone()).unwrap_or_default(),
            is_premium: profile.is_some_and(|profile| profile.more.is_premium),
            premium_badge_emoji: profile.and_then(|profile| profile.more.premium_badge_emoji.clone()),
            name_color: profile.and_then(|profile| profile.more.name_color.clone()),
        }
    }

    /// settings screen of either invented playlist, the reader owns one and is a plain member of
    /// the other
    /// numbers follow panda progress
    pub fn playlist_settings(&self, id: &str, cx: &mut App) -> Option<MusicPlaylistSettings> {
        let dto = self.playlist(id, cx)?;
        let is_own = id == Self::playlist_id();
        let members: i64 = if is_own { if self.panda_accepted { 2 } else { 1 } } else { 2 };
        let pending: i64 = if is_own && self.panda_requested_to_join && !self.panda_accepted { 1 } else { 0 };
        let limit = RiseLimits::service_value("free_participant_limit");
        let stamp = Self::sent_at_stamp_ms();
        let owner_is_premium = is_own && self.viewer_user(cx).is_premium;
        Some(MusicPlaylistSettings {
            playlist: MusicPlaylistSummary {
                id: dto.id.clone().unwrap_or_else(|| id.to_owned()),
                owner_user_id: dto.owner_user_id.clone().unwrap_or_default(),
                title: dto.title.clone().unwrap_or_default(),
                description: String::new(),
                cover_url: String::new(),
                visibility: PlaylistVisibility::Local.raw_value().to_owned(),
                tracks: dto.tracks.unwrap_or_default().iter().filter_map(RiseStoredMusicTrack::as_track_model).collect(),
                created_at_ms: stamp,
                updated_at_ms: stamp,
                revision: 1,
                new_tracks_on_top: false,
            },
            viewer_role: if is_own { OWNER_ROLE } else { MEMBER_ROLE }.to_owned(),
            viewer_permissions: if is_own {
                MusicPlaylistMemberPermissions::OWNER
            } else {
                MusicPlaylistMemberPermissions::NONE
            },
            total_participant_count: members,
            active_participant_count: members,
            suspended_member_count: 0,
            pending_request_count: pending,
            effective_participant_limit: limit.max(members),
            owner_is_premium,
            can_invite: is_own,
            can_review_requests: is_own,
            access_state: ACTIVE_STATUS.to_owned(),
            access_epoch: 1,
        })
    }

    /// built the way the real link is from a token the wall knows so copying it gives a well
    /// formed but dead address and nothing else anywhere
    pub fn invite_link(&self, id: &str) -> Option<MusicPlaylistInviteLink> {
        if !self.is_armed || id != Self::playlist_id() {
            return None;
        }
        let url = RiseonlyPublicLink::Playlist { reference: Self::invite_token() }
            .web_url(AppEnvironment::current())
            .map(|url| url.absolute_string().to_owned())
            .unwrap_or_default();
        Some(MusicPlaylistInviteLink { playlist_id: id.to_owned(), url, token: Self::invite_token(), version: 1 })
    }

    /// none for a playlist that isnt the sandbox one, empty when it is and panda hasnt asked or
    /// was already answered
    pub fn join_requests(&self, id: &str) -> Option<Vec<MusicPlaylistJoinRequest>> {
        if !self.is_armed || id != Self::playlist_id() {
            return None;
        }
        if !self.panda_requested_to_join || self.panda_accepted {
            return Some(Vec::new());
        }
        Some(vec![MusicPlaylistJoinRequest {
            id: Self::join_request_id(),
            playlist_id: id.to_owned(),
            user_id: Self::panda_user_id(),
            user: self.panda_user(),
            status: PENDING_STATUS.to_owned(),
            requested_at_ms: self.panda_requested_at_ms,
            processed_at_ms: None,
            request_version: 1,
        }])
    }

    pub fn members(&self, id: &str, cx: &mut App) -> Option<Vec<MusicPlaylistMember>> {
        if !self.is_armed || (id != Self::playlist_id() && id != Self::shared_playlist_id()) {
            return None;
        }
        if id == Self::shared_playlist_id() {
            if !self.uses_shared_playlist {
                return None;
            }
            return Some(vec![
                self.member(self.panda_user(), id, OWNER_ROLE, MusicPlaylistMemberPermissions::OWNER, 1),
                self.member(self.viewer_user(cx), id, MEMBER_ROLE, MusicPlaylistMemberPermissions::NONE, 1),
            ]);
        }
        let mut list =
            vec![self.member(self.viewer_user(cx), id, OWNER_ROLE, MusicPlaylistMemberPermissions::OWNER, 1)];
        if self.panda_accepted {
            list.push(self.member(
                self.panda_user(),
                id,
                MEMBER_ROLE,
                self.panda_permissions,
                self.panda_member_version,
            ));
        }
        Some(list)
    }

    /// the reader saved panda rights on the real rights page
    pub fn update_panda_permissions(
        &mut self,
        permissions: MusicPlaylistMemberPermissions,
        cx: &mut Context<Self>,
    ) -> Option<MusicPlaylistMember> {
        if !self.is_armed || !self.panda_accepted {
            return None;
        }
        self.panda_permissions = permissions;
        self.panda_member_version += 1;
        GuideSignalBus::emit(RiseGuideSignal::PlaylistMemberRightsSaved);
        cx.notify();
        Some(self.member(
            self.panda_user(),
            &Self::playlist_id(),
            MEMBER_ROLE,
            permissions,
            self.panda_member_version,
        ))
    }

    /// the reader removed panda from the practice playlist
    pub fn remove_panda_from_playlist(&mut self, cx: &mut Context<Self>) {
        if !self.is_armed || !self.panda_accepted {
            return;
        }
        self.panda_accepted = false;
        GuideSignalBus::emit(RiseGuideSignal::PlaylistMemberRemoved);
        cx.notify();
    }

    fn member(
        &self,
        user: MusicUserReference,
        playlist_id: &str,
        role: &str,
        permissions: MusicPlaylistMemberPermissions,
        version: i64,
    ) -> MusicPlaylistMember {
        MusicPlaylistMember {
            playlist_id: playlist_id.to_owned(),
            user_id: user.id.clone(),
            user,
            role: role.to_owned(),
            status: ACTIVE_STATUS.to_owned(),
            permissions,
            accepted_at_ms: Self::sent_at_stamp_ms(),
            version,
        }
    }

    /// reader copied the invite link, panda opens it a bit later and a request shows up
    /// in settings. nothing is emitted
    pub fn note_invite_link_copied(&mut self, cx: &mut Context<Self>) {
        if !self.is_armed || self.panda_requested_to_join || self.panda_accepted {
            return;
        }
        self.schedule(PANDA_REACTION_MS, cx, |sandbox, cx| {
            if !sandbox.is_armed || sandbox.panda_accepted {
                return;
            }
            sandbox.panda_requested_at_ms = sandbox.now_ms();
            sandbox.panda_requested_to_join = true;
            cx.notify();
        });
    }

    pub fn review_join_request(&mut self, approve: bool, cx: &mut Context<Self>) -> Option<MusicPlaylistMember> {
        if !self.is_armed {
            return None;
        }
        self.panda_requested_to_join = false;
        cx.notify();
        if !approve {
            return None;
        }
        self.panda_accepted = true;
        GuideSignalBus::emit(RiseGuideSignal::JoinRequestApproved);
        Some(self.member(
            self.panda_user(),
            &Self::playlist_id(),
            MEMBER_ROLE,
            MusicPlaylistMemberPermissions::NONE,
            1,
        ))
    }

    /// who is listening to which fake playlist right now
    pub fn listening_snapshot(&self, id: &str, cx: &mut App) -> Option<MusicPlaylistListeningSnapshot> {
        if !self.is_armed || (id != Self::playlist_id() && id != Self::shared_playlist_id()) {
            return None;
        }
        let session_id = self.session.as_ref().map(|session| session.session_id.clone());
        let mut listeners = Vec::new();
        if id == Self::shared_playlist_id() {
            if !self.uses_shared_playlist {
                return None;
            }
            listeners.push(self.listener(
                self.panda_user(),
                id,
                &Self::track_id(),
                0,
                true,
                self.panda_listening_since_ms,
                session_id.clone(),
            ));
            if self.session.is_some() {
                listeners.push(self.viewer_listener(id, cx));
            }
        } else if let Some(session) = self.session.as_ref().filter(|session| session.playlist_id == id) {
            listeners.push(self.viewer_listener(id, cx));
            listeners.push(self.listener(
                self.panda_user(),
                id,
                &session.track_id,
                session.anchor_position_ms,
                session.is_playing,
                session.anchor_server_time_ms,
                Some(session.session_id.clone()),
            ));
        }
        let active = self.session.clone().filter(|session| session.playlist_id == id);
        let now = self.now_ms();
        Some(MusicPlaylistListeningSnapshot {
            total_listener_count: i64::try_from(listeners.len()).unwrap_or(i64::MAX),
            listeners,
            presence_sequence: self.session_revision,
            server_time_ms: now,
            active_session: active,
            watcher_lease_until_ms: now + self.lease_ms(),
            unchanged: false,
        })
    }

    fn viewer_listener(&self, playlist_id: &str, cx: &mut App) -> MusicPlaylistListener {
        let player = MusicPlayerState::shared(cx);
        let (track_id, current_time, is_playing) = {
            let player = player.read(cx);
            (
                player.current_track.as_ref().map_or_else(Self::track_id, |track| track.id.clone()),
                player.current_time,
                player.is_playing,
            )
        };
        let session_id = self.session.as_ref().map(|session| session.session_id.clone());
        self.listener(
            self.viewer_user(cx),
            playlist_id,
            &track_id,
            (current_time.max(0.0) * 1_000.0) as i64,
            is_playing,
            self.now_ms(),
            session_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn listener(
        &self,
        user: MusicUserReference,
        playlist_id: &str,
        track_id: &str,
        anchor_position_ms: i64,
        is_playing: bool,
        anchor_server_time_ms: i64,
        session_id: Option<String>,
    ) -> MusicPlaylistListener {
        MusicPlaylistListener {
            playlist_id: playlist_id.to_owned(),
            user_id: user.id.clone(),
            user,
            track_id: track_id.to_owned(),
            anchor_position_ms,
            is_playing,
            anchor_server_time_ms,
            client_sequence: self.session_revision,
            session_id,
            audio_effects: Vec::new(),
        }
    }

    /// reader started the fake track
    /// in the host lesson panda joins a bit later and asks for the one right
    pub fn track_playback_started(&mut self, cx: &mut Context<Self>) {
        if !self.is_armed || !self.panda_joins_on_play || self.session.is_some() {
            return;
        }
        self.schedule(PANDA_REACTION_MS, cx, |sandbox, cx| {
            if !sandbox.is_armed || sandbox.session.is_some() {
                return;
            }
            let player = MusicPlayerState::shared(cx);
            let (playing_track, current_time, is_playing) = {
                let player = player.read(cx);
                (player.current_track.as_ref().map(|track| track.id.clone()), player.current_time, player.is_playing)
            };
            if playing_track.as_deref() != Some(Self::track_id().as_str()) {
                return;
            }
            let viewer_user_id = sandbox.viewer_user_id(cx);
            let viewer_user = sandbox.viewer_user(cx);
            let participants = vec![sandbox.participant(sandbox.panda_user(), MusicListeningParticipantPermissions::NONE)];
            let hosted = sandbox.make_session(
                &Self::playlist_id(),
                &viewer_user_id,
                viewer_user,
                participants,
                MusicListeningParticipantPermissions::new_with_apply_effects(true, true, true, true),
                (current_time.max(0.0) * 1_000.0) as i64,
                is_playing,
            );
            sandbox.session = Some(hosted.clone());
            cx.notify();
            let server_time_ms = sandbox.now_ms();
            player.update(cx, |player, cx| {
                player.attach_playlist_listening_session(hosted, server_time_ms, Some(viewer_user_id), cx);
            });
            sandbox.schedule(PANDA_REACTION_MS, cx, |sandbox, cx| {
                let Some(current) = sandbox.session.as_ref().filter(|_| sandbox.is_armed) else {
                    return;
                };
                let notice = MusicListeningPermissionRequestNotice {
                    session_id: current.session_id.clone(),
                    participant_user_id: Self::panda_user_id(),
                    permissions: MusicListeningParticipantPermissions::new_with_apply_effects(false, false, false, true),
                };
                MusicPlayerState::shared(cx).update(cx, |player, cx| {
                    player.listening_permission_request_notice = Some(notice);
                    cx.notify();
                });
            });
        });
    }

    /// reader clicked panda in the listeners sheet of her playlist
    pub fn join_listening(&mut self, id: &str, cx: &mut Context<Self>) -> Option<MusicPlaylistListeningSession> {
        if !self.is_armed || id != Self::shared_playlist_id() || !self.uses_shared_playlist {
            return None;
        }
        if let Some(session) = self.session.clone() {
            return Some(session);
        }
        let participants = vec![self.participant(self.viewer_user(cx), MusicListeningParticipantPermissions::NONE)];
        let joined = self.make_session(
            id,
            &Self::panda_user_id(),
            self.panda_user(),
            participants,
            MusicListeningParticipantPermissions::NONE,
            self.track_duration_ms() / 4,
            true,
        );
        self.session = Some(joined.clone());
        cx.notify();
        Some(joined)
    }

    /// state command the reader's player would have sent to the room
    #[allow(clippy::too_many_arguments)]
    pub fn update_session_state(
        &mut self,
        track_id: &str,
        position_ms: i64,
        is_playing: bool,
        shuffle_enabled: bool,
        shuffle_seed: i64,
        repeat_mode: &str,
        audio_effect_enabled: bool,
        audio_effect_rate_milli: i64,
        audio_effects: Vec<MusicListeningAudioEffect>,
        cx: &mut Context<Self>,
    ) -> Option<MusicPlaylistListeningSession> {
        if !self.is_armed {
            return None;
        }
        let current = self.session.clone()?;
        self.session_revision += 1;
        let now = self.now_ms();
        let next = MusicPlaylistListeningSession {
            track_id: track_id.to_owned(),
            anchor_position_ms: position_ms,
            is_playing,
            anchor_server_time_ms: now,
            revision: self.session_revision,
            shuffle_enabled,
            shuffle_seed,
            repeat_mode: repeat_mode.to_owned(),
            lease_until_ms: now + self.lease_ms(),
            audio_effect_enabled,
            audio_effect_rate_milli,
            audio_effects,
            ..current
        };
        self.session = Some(next.clone());
        cx.notify();
        Some(next)
    }

    /// host changed a participant's rights
    pub fn update_participant_permissions(
        &mut self,
        user_id: &str,
        permissions: MusicListeningParticipantPermissions,
        cx: &mut Context<Self>,
    ) -> Option<MusicPlaylistListeningSession> {
        if !self.is_armed {
            return None;
        }
        let current = self.session.clone()?;
        self.session_revision += 1;
        let participants = current
            .participants
            .iter()
            .map(|existing| {
                if existing.user_id == user_id {
                    MusicListeningParticipant { permissions, ..existing.clone() }
                } else {
                    existing.clone()
                }
            })
            .collect();
        let next = MusicPlaylistListeningSession {
            revision: self.session_revision,
            participants,
            lease_until_ms: self.now_ms() + self.lease_ms(),
            ..current
        };
        self.session = Some(next.clone());
        cx.notify();
        Some(next)
    }

    /// reader left or closed the room
    pub fn session_ended(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        cx.notify();
    }

    fn participant(
        &self,
        user: MusicUserReference,
        permissions: MusicListeningParticipantPermissions,
    ) -> MusicListeningParticipant {
        MusicListeningParticipant { user_id: user.id.clone(), user, permissions, joined_at_ms: self.now_ms() }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_session(
        &mut self,
        playlist_id: &str,
        host_user_id: &str,
        host_user: MusicUserReference,
        participants: Vec<MusicListeningParticipant>,
        viewer_permissions: MusicListeningParticipantPermissions,
        position_ms: i64,
        is_playing: bool,
    ) -> MusicPlaylistListeningSession {
        self.session_revision += 1;
        let now = self.now_ms();
        MusicPlaylistListeningSession {
            session_id: Self::session_id(),
            playlist_id: playlist_id.to_owned(),
            host_user_id: host_user_id.to_owned(),
            host_user,
            track_id: Self::track_id(),
            anchor_position_ms: position_ms,
            is_playing,
            anchor_server_time_ms: now,
            revision: self.session_revision,
            generation: 1,
            shuffle_enabled: false,
            shuffle_seed: 0,
            repeat_mode: MusicRepeatMode::Off.wire().to_owned(),
            participants,
            viewer_permissions,
            lease_until_ms: now + self.lease_ms(),
            audio_effect_enabled: false,
            audio_effect_rate_milli: MusicEffectPlaybackLimits::rate_milli(1.0),
            audio_effects: Vec::new(),
        }
    }

    /// runs body on the sandbox after after_ms, dropped if the sandbox drops the task first
    /// every reset and room change does that
    pub fn schedule(
        &mut self,
        after_ms: u64,
        cx: &mut Context<Self>,
        body: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(after_ms)).await;
            this.update(cx, body).ok();
        });
        self.scene_tasks.push(task);
    }
}

/// one chair in the fake room
/// mutable half the sandbox edits when the reader moderates, the participant is derived on each publish
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxVoiceSeat {
    pub profile: RiseRTCUserProfile,
    pub role: RiseRTCParticipantRole,
    pub microphone_enabled: bool,
    pub deafened: bool,
    pub server_muted: bool,
    pub server_deafened: bool,
    pub hand_raised: bool,
    pub hand_raised_at_ms: i64,
    pub joined_at_ms: i64,
    pub order_sequence: i64,
}

impl GuideSandboxVoiceSeat {
    pub fn new(
        profile: RiseRTCUserProfile,
        role: RiseRTCParticipantRole,
        microphone_enabled: bool,
        joined_at_ms: i64,
        order_sequence: i64,
    ) -> Self {
        Self {
            profile,
            role,
            microphone_enabled,
            deafened: false,
            server_muted: false,
            server_deafened: false,
            hand_raised: false,
            hand_raised_at_ms: 0,
            joined_at_ms,
            order_sequence,
        }
    }
}

impl GuideSandbox {
    pub fn owns_forum_chat_id(&self, id: Option<&str>) -> bool {
        self.is_armed && self.uses_forum && id == Some(Self::forum_chat_id().as_str())
    }

    pub fn owns_voice_room_id(&self, id: Option<&str>) -> bool {
        self.is_armed
            && self.uses_forum
            && id.is_some_and(|id| id == Self::stage_room_id() || id == Self::voice_room_id_value())
    }

    /// panda is a user reference not an account, clicking her profile goes nowhere
    pub fn owns_user_id(&self, id: &str) -> bool {
        self.is_armed && id == Self::panda_user_id()
    }

    /// room a sandbox channel opens, none for a topic the sandbox never made
    /// read off the main thread
    pub fn voice_room_id(topic_id: &str) -> Option<String> {
        if topic_id == Self::stage_topic_id() {
            Some(Self::stage_room_id())
        } else if topic_id == Self::voice_topic_id() {
            Some(Self::voice_room_id_value())
        } else {
            None
        }
    }

    /// the group as the chat list would describe it, reader's own with topics on
    /// published as a standalone detail so no list shows it
    pub fn forum_chat(&self, cx: &mut App) -> ChatInfo {
        ChatInfo {
            title: tr("guide_scene_forum_title"),
            r#type: ChatType::Group,
            member_count: 2,
            role: ChatPerm::OWNER_ROLES.first().map(|role| role.to_lowercase()),
            participant: User { id: self.viewer_user_id(cx), ..User::default() },
            topics_enabled: true,
            created_at: Self::scene_stamp(),
            updated_at: Self::scene_stamp(),
            ..ChatInfo::new(Self::forum_chat_id())
        }
    }

    pub fn forum_sections(&self) -> Vec<ChatSection> {
        vec![ChatSection {
            id: Self::forum_section_id(),
            name: tr("guide_scene_forum_section"),
            position: 0,
            name_entities: None,
        }]
    }

    /// stage first, the guide anchors on the first voice or stage row
    pub fn forum_topics(&self) -> Vec<ChatTopic> {
        let rooms = [
            Self::channel_topic(
                Self::stage_topic_id(),
                tr("guide_scene_stage_channel"),
                "\u{1F3A4}",
                ChatChannelKind::Stage,
                0,
            ),
            Self::channel_topic(
                Self::voice_topic_id(),
                tr("guide_scene_voice_channel"),
                "\u{1F3A7}",
                ChatChannelKind::Voice,
                1,
            ),
        ];
        self.team_topic().into_iter().chain(rooms).collect()
    }

    /// owner rights, administrator short circuits every check like the real owner
    pub fn forum_permissions(&self) -> EffectivePermissionsDTO {
        EffectivePermissionsDTO {
            permissions: i64::try_from(ChatPerm::ADMINISTRATOR).unwrap_or_default(),
            ..EffectivePermissionsDTO::default()
        }
    }

    pub fn publish_forum(&mut self, cx: &mut Context<Self>) {
        if !self.is_armed || !self.uses_forum {
            return;
        }
        let chat = self.forum_chat(cx).to_wire_dto(None);
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            let Some(account_id) = presentation.account_id().filter(|id| !id.is_empty()).map(str::to_owned) else {
                return;
            };
            presentation.publish_chat_detail(chat, &account_id, Some(false), cx);
        });
    }

    fn withdraw_forum(&mut self, cx: &mut Context<Self>) {
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.remove_chat_detail(&Self::forum_chat_id(), cx);
        });
    }

    fn channel_topic(id: String, title: String, icon: &str, kind: ChatChannelKind, position: i64) -> ChatTopic {
        ChatTopic {
            section_id: Self::forum_section_id(),
            updated_at: Self::scene_stamp(),
            channel_kind: kind,
            voice_config: Self::channel_config(kind),
            ..ChatTopic::new(id, title, icon.to_owned(), position, false, false)
        }
    }

    /// channel defaults with video and screen share off
    /// nothing here can render a track so a camera button would just fail
    fn channel_config(kind: ChatChannelKind) -> Option<VoiceChannelConfig> {
        VoiceChannelConfig::defaults(kind).map(|mut config| {
            config.video_enabled = false;
            config.screen_share_enabled = false;
            config
        })
    }

    /// reader opened a sandbox channel, the room shows at once because there is no server to wait for
    pub fn join_voice_room(&mut self, room_id: &str, cx: &mut Context<Self>) -> Result<(), RiseRTCStateError> {
        if !self.owns_voice_room_id(Some(room_id)) {
            return Err(RiseRTCStateError::MissingActiveRoom);
        }
        let presentation = RiseRTCPresentation::shared(cx);
        if presentation.read(cx).current_account_id().is_none_or(str::is_empty) {
            return Err(RiseRTCStateError::StaleAccount);
        }
        if self.joined_room_id.as_deref() == Some(room_id) {
            self.publish_voice_room(cx);
            return Ok(());
        }
        self.scene_tasks.clear();
        self.joined_room_id = Some(room_id.to_owned());
        self.room_opened_at_ms = self.now_ms();
        self.room_seats =
            vec![GuideSandboxVoiceSeat::new(self.viewer_profile(cx), RiseRTCParticipantRole::Moderator, true, self.room_opened_at_ms, 1)];
        let selected = room_id.to_owned();
        presentation.update(cx, |presentation, cx| presentation.select_room(Some(selected), cx));
        self.publish_voice_room(cx);
        presentation
            .update(cx, |presentation, cx| presentation.publish_connection_state(RiseRTCConnectionState::Connected, cx));

        let is_stage = room_id == Self::stage_room_id();
        let joined = room_id.to_owned();
        self.schedule(PANDA_REACTION_MS, cx, move |sandbox, cx| {
            if sandbox.joined_room_id.as_deref() != Some(joined.as_str()) {
                return;
            }
            let role = if is_stage { RiseRTCParticipantRole::Listener } else { RiseRTCParticipantRole::Speaker };
            let seat = GuideSandboxVoiceSeat::new(sandbox.panda_profile(), role, !is_stage, sandbox.now_ms(), 2);
            sandbox.room_seats.push(seat);
            sandbox.publish_voice_room(cx);
            if !is_stage {
                return;
            }
            sandbox.schedule(PANDA_REACTION_MS, cx, move |sandbox, cx| {
                if sandbox.joined_room_id.as_deref() != Some(joined.as_str()) {
                    return;
                }
                let Some(index) = sandbox.seat_index(&Self::panda_user_id()) else {
                    return;
                };
                sandbox.room_seats[index].hand_raised = true;
                let now = sandbox.now_ms();
                sandbox.room_seats[index].hand_raised_at_ms = now;
                sandbox.publish_voice_room(cx);
            });
        });
        Ok(())
    }

    pub fn refresh_voice_room(&mut self, cx: &mut Context<Self>) {
        self.publish_voice_room(cx);
    }

    /// reader left, projection goes back to idle and panda leaves with the room
    pub fn leave_voice_room(&mut self, cx: &mut Context<Self>) {
        if self.joined_room_id.is_none() {
            return;
        }
        self.scene_tasks.clear();
        self.joined_room_id = None;
        self.room_seats.clear();
        RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.select_room(None, cx);
            presentation.publish_connection_state(RiseRTCConnectionState::Idle, cx);
        });
        cx.notify();
    }

    pub fn promote_to_speaker(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.seat_index(user_id) else {
            return;
        };
        let seat = &mut self.room_seats[index];
        seat.role = RiseRTCParticipantRole::Speaker;
        seat.hand_raised = false;
        seat.microphone_enabled = true;
        self.publish_voice_room(cx);
    }

    pub fn move_to_audience(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.seat_index(user_id) else {
            return;
        };
        let seat = &mut self.room_seats[index];
        seat.role = RiseRTCParticipantRole::Listener;
        seat.microphone_enabled = false;
        self.publish_voice_room(cx);
    }

    pub fn moderate(&mut self, user_id: &str, action: RiseRTCModerationAction, cx: &mut Context<Self>) {
        let Some(index) = self.seat_index(user_id) else {
            return;
        };
        match action {
            RiseRTCModerationAction::ForceMute => self.room_seats[index].server_muted = true,
            RiseRTCModerationAction::AllowUnmute => self.room_seats[index].server_muted = false,
            RiseRTCModerationAction::ForceDeafen => self.room_seats[index].server_deafened = true,
            RiseRTCModerationAction::AllowUndeafen => self.room_seats[index].server_deafened = false,
            RiseRTCModerationAction::Kick => {
                self.room_seats.remove(index);
            }
            RiseRTCModerationAction::BlockCamera
            | RiseRTCModerationAction::AllowCamera
            | RiseRTCModerationAction::BlockScreenShare
            | RiseRTCModerationAction::AllowScreenShare
            | RiseRTCModerationAction::Unspecified => return,
        }
        self.publish_voice_room(cx);
    }

    pub fn set_self_microphone(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let viewer_user_id = self.viewer_user_id(cx);
        let Some(index) = self.seat_index(&viewer_user_id) else {
            return;
        };
        let seat = &mut self.room_seats[index];
        seat.microphone_enabled = enabled;
        if enabled {
            seat.deafened = false;
        }
        self.publish_voice_room(cx);
    }

    pub fn set_self_deafened(&mut self, deafened: bool, cx: &mut Context<Self>) {
        let viewer_user_id = self.viewer_user_id(cx);
        let Some(index) = self.seat_index(&viewer_user_id) else {
            return;
        };
        let seat = &mut self.room_seats[index];
        seat.deafened = deafened;
        if deafened {
            seat.microphone_enabled = false;
        }
        self.publish_voice_room(cx);
    }

    fn withdraw_voice_room(&mut self, cx: &mut Context<Self>) {
        let target = RTCServicesStore::shared(cx).read(cx).room_presentation_target().cloned();
        if let Some(target) = target
            && matches!(&target, RiseRTCRoomTarget::Topic { chat_id, .. } if *chat_id == Self::forum_chat_id())
        {
            RTCInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.dismiss_room_presentation(cx));
            RiseRTCNavigationGate::mark_self_in_voice_channel(&target, false, cx);
        }
        self.leave_voice_room(cx);
    }

    fn seat_index(&self, user_id: &str) -> Option<usize> {
        self.room_seats.iter().position(|seat| seat.profile.id == user_id)
    }

    /// the room as the repository would describe it, built from the seats
    /// server mute or deafen reports the mic off
    fn publish_voice_room(&mut self, cx: &mut Context<Self>) {
        let Some(room_id) = self.joined_room_id.clone() else {
            return;
        };
        let presentation = RiseRTCPresentation::shared(cx);
        let Some(account_id) =
            presentation.read(cx).current_account_id().filter(|id| !id.is_empty()).map(str::to_owned)
        else {
            return;
        };
        self.room_revision += 1;
        let now = self.now_ms();
        let is_stage = room_id == Self::stage_room_id();
        let kind = if is_stage { ChatChannelKind::Stage } else { ChatChannelKind::Voice };
        let defaults = Self::channel_config(kind);

        let participants: Vec<RiseRTCParticipant> = self
            .room_seats
            .iter()
            .map(|seat| RiseRTCParticipant {
                user_id: seat.profile.id.clone(),
                role: seat.role,
                connected: true,
                microphone_enabled: seat.microphone_enabled && !seat.server_muted && !seat.server_deafened,
                camera_enabled: false,
                screen_share_enabled: false,
                server_muted: seat.server_muted,
                server_deafened: seat.server_deafened,
                server_camera_blocked: false,
                server_screen_share_blocked: false,
                deafened: seat.deafened,
                hand_raised: seat.hand_raised,
                joined_at_ms: seat.joined_at_ms,
                updated_at_ms: now,
                lease_expires_at_ms: now + self.lease_ms(),
                order_sequence: seat.order_sequence,
                watching_user_id: String::new(),
                profile: Some(seat.profile.clone()),
            })
            .collect();
        let hands: Vec<RiseRTCRaisedHand> = self
            .room_seats
            .iter()
            .filter(|seat| seat.hand_raised)
            .map(|seat| RiseRTCRaisedHand {
                user_id: seat.profile.id.clone(),
                requested_at_ms: seat.hand_raised_at_ms,
                expires_at_ms: None,
                note: None,
                profile: Some(seat.profile.clone()),
            })
            .collect();
        let config = RiseRTCRoomConfig {
            room_id: room_id.clone(),
            livekit_room_name: String::new(),
            chat_id: Self::forum_chat_id(),
            channel_id: Some(if is_stage { Self::stage_topic_id() } else { Self::voice_topic_id() }),
            mode: if is_stage { RiseRTCRoomMode::Stage } else { RiseRTCRoomMode::Voice },
            max_participants: defaults.as_ref().map_or(0, |config| config.user_limit),
            max_speakers: defaults.as_ref().map_or(0, |config| config.speaker_limit),
            video_enabled: defaults.as_ref().is_some_and(|config| config.video_enabled),
            screen_share_enabled: defaults.as_ref().is_some_and(|config| config.screen_share_enabled),
            audio_bitrate_kbps: defaults.as_ref().map_or(0, |config| config.audio_bitrate_kbps),
            region: defaults.as_ref().map(|config| config.region.clone()).unwrap_or_default(),
            revision: self.room_revision,
            created_at_ms: self.room_opened_at_ms,
            updated_at_ms: now,
        };
        let speakers = participants
            .iter()
            .filter(|participant| {
                participant.role == RiseRTCParticipantRole::Speaker
                    || participant.role == RiseRTCParticipantRole::Moderator
            })
            .count();
        let meta = RiseRTCRoomMeta {
            participant_count: i64::try_from(participants.len()).unwrap_or(i64::MAX),
            speaker_count: i64::try_from(speakers).unwrap_or(i64::MAX),
            generated_at_ms: now,
            profiles_complete: true,
            profile_error: None,
            has_more_participants: false,
            next_participant_cursor: None,
            has_more_raised_hands: false,
            next_raised_hand_cursor: None,
        };
        let snapshot = RiseRTCRoomRepositorySnapshot {
            account_id,
            revision: self.room_revision,
            room: Some(config),
            participants,
            raised_hands: hands,
            meta: Some(meta),
        };
        presentation.update(cx, |presentation, cx| presentation.publish_room(snapshot, cx));
        cx.notify();
    }

    pub fn viewer_profile(&self, cx: &mut App) -> RiseRTCUserProfile {
        let id = self.viewer_user_id(cx);
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile();
        RiseRTCUserProfile {
            id,
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            avatar_url: profile.map(|profile| profile.more.logo.clone()).unwrap_or_default(),
            is_premium: profile.is_some_and(|profile| profile.more.is_premium),
            is_official: profile.is_some_and(|profile| profile.more.is_official),
            is_online: true,
            last_seen_ms: 0,
        }
    }

    pub fn panda_profile(&self) -> RiseRTCUserProfile {
        RiseRTCUserProfile {
            id: Self::panda_user_id(),
            tag: tr("guide_scene_sender_tag"),
            name: tr("guide_scene_sender"),
            avatar_url: String::new(),
            is_premium: false,
            is_official: false,
            is_online: true,
            last_seen_ms: 0,
        }
    }
}

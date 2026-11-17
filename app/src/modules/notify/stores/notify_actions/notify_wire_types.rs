use rise_canon::rise_key::notify;
use rise_canon::rise_request::notify::UpdateNotificationSettings;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::app_info::AppInfo;
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::modules::notify::stores::notify_actions::notify_settings_types::NotificationSettingsModel;
use crate::modules::notify::stores::notify_actions::rise_rtc_system_bridges_hooks::rise_rtc_system_bridges_hooks;

pub enum RiseAPNSRegistrationTarget {}

impl RiseAPNSRegistrationTarget {
    /// bundle identifier this build ships under
    /// info.plist on macos, desktop app id on linux
    /// aUMID on windows
    pub fn bundle_id() -> String {
        AppInfo::current(AppEnvironment::current().as_str())
            .map(|info| trimming_whitespaces_and_newlines(&info.bundle_identifier).to_owned())
            .unwrap_or_default()
    }

    /// voip push environment read through the rtc slot
    /// an empty slot answers sandbox
    pub fn environment() -> String {
        match rise_rtc_system_bridges_hooks() {
            Some(hooks) => (hooks.current)(),
            None => RISE_RTC_SYSTEM_VOIP_APNS_ENVIRONMENT_FALLBACK.to_owned(),
        }
    }
}

const RISE_RTC_SYSTEM_VOIP_APNS_ENVIRONMENT_FALLBACK: &str = "sandbox";

pub type NotificationSettingsGatewayDTO = rise_canon::rise_response::notify::GetNotificationSettings;

pub trait NotificationSettingsGatewayModel {
    fn to_model(&self) -> NotificationSettingsModel;
}

impl NotificationSettingsGatewayModel for NotificationSettingsGatewayDTO {
    fn to_model(&self) -> NotificationSettingsModel {
        NotificationSettingsModel {
            all_enabled: self.all_enabled,
            messages_enabled: self.messages_enabled,
            mentions_enabled: self.mentions_enabled,
            replies_enabled: self.replies_enabled,
            comments_enabled: self.comments_enabled,
            post_likes_enabled: self.post_likes_enabled,
            comment_likes_enabled: self.comment_likes_enabled,
            new_followers_enabled: self.new_followers_enabled,
            new_posts_enabled: self.new_posts_enabled,
            stories_enabled: self.stories_enabled,
            coauthors_enabled: self.coauthors_enabled,
            tags_enabled: self.tags_enabled,
            map_reactions_enabled: self.map_reactions_enabled,
            sparks_enabled: self.sparks_enabled,
            music_enabled: self.music_enabled,
            music_session_joined_enabled: self.music_session_joined_enabled,
            music_session_left_enabled: self.music_session_left_enabled,
            music_session_ended_enabled: self.music_session_ended_enabled,
            music_session_permission_requests_enabled: self.music_session_permission_requests_enabled,
            music_session_permission_updates_enabled: self.music_session_permission_updates_enabled,
            music_playlist_track_added_enabled: self.music_playlist_track_added_enabled,
            music_playlist_track_removed_enabled: self.music_playlist_track_removed_enabled,
            music_artist_releases_enabled: self.music_artist_releases_enabled,
            music_playlist_access_enabled: self.music_playlist_access_enabled,
            music_playlist_listening_started_enabled: self.music_playlist_listening_started_enabled,
            announcements_enabled: self.announcements_enabled,
            mute_all_profiles: self.mute_all_profiles,
        }
    }
}

/// patch is keyed by wire names, same keys the optimistic cache patch uses
fn update_notification_settings_field<'a>(
    body: &'a mut UpdateNotificationSettings,
    wire: &str,
) -> Option<&'a mut Option<bool>> {
    Some(match wire {
        notify::ALL_ENABLED => &mut body.all_enabled,
        notify::MESSAGES_ENABLED => &mut body.messages_enabled,
        notify::MENTIONS_ENABLED => &mut body.mentions_enabled,
        notify::REPLIES_ENABLED => &mut body.replies_enabled,
        notify::COMMENTS_ENABLED => &mut body.comments_enabled,
        notify::POST_LIKES_ENABLED => &mut body.post_likes_enabled,
        notify::COMMENT_LIKES_ENABLED => &mut body.comment_likes_enabled,
        notify::NEW_FOLLOWERS_ENABLED => &mut body.new_followers_enabled,
        notify::NEW_POSTS_ENABLED => &mut body.new_posts_enabled,
        notify::STORIES_ENABLED => &mut body.stories_enabled,
        notify::COAUTHORS_ENABLED => &mut body.coauthors_enabled,
        notify::TAGS_ENABLED => &mut body.tags_enabled,
        notify::MAP_REACTIONS_ENABLED => &mut body.map_reactions_enabled,
        notify::SPARKS_ENABLED => &mut body.sparks_enabled,
        notify::MUSIC_ENABLED => &mut body.music_enabled,
        notify::MUSIC_SESSION_JOINED_ENABLED => &mut body.music_session_joined_enabled,
        notify::MUSIC_SESSION_LEFT_ENABLED => &mut body.music_session_left_enabled,
        notify::MUSIC_SESSION_ENDED_ENABLED => &mut body.music_session_ended_enabled,
        notify::MUSIC_SESSION_PERMISSION_REQUESTS_ENABLED => &mut body.music_session_permission_requests_enabled,
        notify::MUSIC_SESSION_PERMISSION_UPDATES_ENABLED => &mut body.music_session_permission_updates_enabled,
        notify::MUSIC_PLAYLIST_TRACK_ADDED_ENABLED => &mut body.music_playlist_track_added_enabled,
        notify::MUSIC_PLAYLIST_TRACK_REMOVED_ENABLED => &mut body.music_playlist_track_removed_enabled,
        notify::MUSIC_ARTIST_RELEASES_ENABLED => &mut body.music_artist_releases_enabled,
        notify::MUSIC_PLAYLIST_ACCESS_ENABLED => &mut body.music_playlist_access_enabled,
        notify::MUSIC_PLAYLIST_LISTENING_STARTED_ENABLED => &mut body.music_playlist_listening_started_enabled,
        notify::ANNOUNCEMENTS_ENABLED => &mut body.announcements_enabled,
        notify::MUTE_ALL_PROFILES => &mut body.mute_all_profiles,
        _ => return None,
    })
}

pub trait UpdateNotificationSettingsPatch: Sized {
    fn from_patch(patch: &Map<String, Value>) -> Self;
}

impl UpdateNotificationSettingsPatch for UpdateNotificationSettings {
    fn from_patch(patch: &Map<String, Value>) -> Self {
        let mut body = Self::default();
        for (key, value) in patch {
            let enabled = value.as_bool();
            let known = enabled.is_some() && update_notification_settings_field(&mut body, key).is_some();
            debug_assert!(known, "notification settings patch: unknown key {key}");
            if let (Some(enabled), Some(field)) = (enabled, update_notification_settings_field(&mut body, key)) {
                *field = Some(enabled);
            }
        }
        body
    }
}

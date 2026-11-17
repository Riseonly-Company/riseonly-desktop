use gpui::App;
use rise_canon::RiseGuideSignal;
use rise_i18n::tr;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::stores::post_actions::post_types::PostCollaboratorUser;

impl GuideSandbox {
    pub fn owns_composer_scene(&self) -> bool {
        self.is_armed && self.scenes.contains(&Self::composer_scene_id())
    }

    /// panda at the top of the people pickers the composer opens
    pub fn composer_picker_suggestions(&self, _cx: &App) -> Vec<PostCollaboratorUser> {
        if !self.owns_composer_scene() {
            return Vec::new();
        }
        vec![PostCollaboratorUser {
            id: Self::panda_user_id(),
            is_friend: true,
            is_official: false,
            is_premium: false,
            logo: String::new(),
            name: tr("guide_scene_sender"),
            name_color: None,
            premium_badge_emoji: None,
            since_ms: 0,
            tag: tr("guide_scene_sender_tag"),
        }]
    }

    /// the bundled track among the recent sounds once its on disk
    pub fn composer_suggested_tracks(&self, cx: &mut App) -> Vec<RiseStoredMusicTrack> {
        if !self.owns_composer_scene() || self.audio.is_none() {
            return Vec::new();
        }
        vec![self.track_dto(cx)]
    }

    /// true for lessons that end in the composer
    /// a real post from them would hit the server with media that never existed
    pub fn intercepts_publish(&self) -> bool {
        self.is_armed
            && (self.scenes.contains(&Self::composer_scene_id())
                || self.scenes.contains(&Self::sound_scene_id())
                || self.scenes.contains(&Self::achievements_scene_id())
                || self.scenes.contains(&Self::map_scene_id()))
    }

    /// reader pressed publish, post is faked with no upload and no request
    pub fn publish_composed_post(&self) {
        if !self.intercepts_publish() {
            return;
        }
        GuideSignalBus::emit(RiseGuideSignal::PostCreated);
    }
}

use futures::future::{BoxFuture, LocalBoxFuture};
use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCParticipant;

/// one page of an rtc channel listing with its has more flag
pub type ForumVoiceRosterPage = anyhow::Result<(Vec<RiseRTCParticipant>, bool)>;

/// rtc domain of the active account as the forum roster load sees it
pub struct ForumVoiceRosterSource {
    pub account_id: String,
    pub list_channel_participants:
        Box<dyn Fn(String, Option<String>, i64) -> BoxFuture<'static, ForumVoiceRosterPage> + Send + Sync>,
}

pub struct ForumVoiceRosterHooks {
    pub require_source: fn(&mut App) -> LocalBoxFuture<'static, Option<ForumVoiceRosterSource>>,
}

pub(crate) static FORUM_VOICE_ROSTER_HOOKS: FeatureHookSlot<ForumVoiceRosterHooks> =
    FeatureHookSlot::new("FORUM_VOICE_ROSTER_HOOKS");

pub fn set_forum_voice_roster_hooks(hooks: ForumVoiceRosterHooks) {
    let _ = FORUM_VOICE_ROSTER_HOOKS.set(hooks);
}

pub fn forum_voice_roster_hooks() -> Option<&'static ForumVoiceRosterHooks> {
    FORUM_VOICE_ROSTER_HOOKS.get()
}

use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetMusicHooks, set_account_state_reset_music_hooks,
};
use crate::modules::music::shared::utils::music::featured_profile_track_store::FeaturedProfileTrackStore;
use crate::modules::music::shared::utils::music::music_audio_effect_store::MusicAudioEffectStore;
use crate::modules::music::shared::utils::music::music_effect_thermal_guard::MusicEffectThermalGuard;
use crate::modules::music::shared::utils::music::music_local_state::MusicLocalState;
use crate::modules::music::shared::utils::music::music_local_track_draft_store::MusicLocalTrackDraftStore;
use crate::modules::music::shared::utils::music::music_playback_preferences::MusicPlaybackPreferences;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::shared::utils::music::music_track_download_store::MusicTrackDownloadStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;

pub fn install_account_state_reset_music_hooks() {
    set_account_state_reset_music_hooks(AccountStateResetMusicHooks {
        activate_account_state: |cx: &mut App| {
            MusicLocalState::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
            FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
            MusicAudioEffectStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
            MusicPlaybackPreferences::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
        },
        music_local_track_draft_store_interrupt_for_account_change: |cx: &mut App| {
            MusicLocalTrackDraftStore::shared(cx).update(cx, |store, _| store.interrupt_for_account_change())
        },
        reset_account_state: |cx: &mut App| {
            MessageMusicImportPresenter::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicPlayerState::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicServicesStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicLocalState::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicAudioEffectStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicPlaybackPreferences::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicEffectThermalGuard::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicTrackDownloadStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MusicLocalTrackDraftStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
        remove_persisted_account: |user_id: &str, cx: &mut App| {
            MusicLocalState::shared(cx).read(cx).remove_persisted_account(user_id);
            MusicLocalTrackDraftStore::shared(cx).read(cx).remove_persisted_account(user_id);
            FeaturedProfileTrackStore::shared(cx).read(cx).remove_persisted_account(user_id);
            MusicAudioEffectStore::shared(cx).read(cx).remove_persisted_account(user_id);
            MusicPlaybackPreferences::shared(cx).read(cx).remove_persisted_account(user_id);
        },
    });
}

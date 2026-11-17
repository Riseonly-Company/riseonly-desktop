use gpui::App;

use crate::modules::music::shared::utils::music::music_player_rtc_hooks::{
    MusicPlayerRtcHooks, set_music_player_rtc_hooks,
};
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;

pub(crate) fn install_music_player_rtc_hooks() {
    let _ = set_music_player_rtc_hooks(MusicPlayerRtcHooks {
        request_music_playback: |retry: Box<dyn FnOnce(&mut App)>, cx: &mut App| {
            RTCInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.request_music_playback(retry, cx))
        },
    });
}

use crate::modules::music::shared::utils::music::music_player_state_hooks::{
    MusicPlayerStateActivityHooks, set_music_player_state_activity_hooks,
};
use crate::modules::user::engine::rise_meaningful_activity::RiseMeaningfulActivity;

pub fn install_music_player_state_activity_hooks() {
    set_music_player_state_activity_hooks(MusicPlayerStateActivityHooks {
        set_source: |id, active, is_active, cx| {
            RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| {
                activity.set_source(id, active, is_active, cx);
            });
        },
    });
}

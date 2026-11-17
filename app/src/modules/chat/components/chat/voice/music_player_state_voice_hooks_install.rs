use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::music::shared::utils::music::music_player_state_hooks::{
    MusicPlayerStateVoiceHooks, set_music_player_state_voice_hooks,
};

pub fn install_music_player_state_voice_hooks() {
    set_music_player_state_voice_hooks(MusicPlayerStateVoiceHooks {
        is_active: |cx| ChatVoiceMessagePlayer::shared(cx).read(cx).is_active(),
        stop: |cx| {
            ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| player.stop(cx));
        },
    });
}

use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::music::shared::utils::music::music_player_state_hooks::{
    MusicPlayerStateVideoNoteHooks, set_music_player_state_video_note_hooks,
};

pub fn install_music_player_state_video_note_hooks() {
    set_music_player_state_video_note_hooks(MusicPlayerStateVideoNoteHooks {
        is_active: |cx| VideoNotePlayerStore::shared(cx).read(cx).is_active(),
        stop: |cx| {
            VideoNotePlayerStore::shared(cx).update(cx, |player, cx| player.stop(cx));
        },
    });
}

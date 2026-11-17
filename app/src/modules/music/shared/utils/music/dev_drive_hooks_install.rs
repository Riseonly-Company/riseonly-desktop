use crate::app::dev_drive::dev_drive_hooks::{DevDriveHooks, set_dev_drive_hooks};
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

pub fn install_dev_drive_hooks() {
    set_dev_drive_hooks(DevDriveHooks {
        music_playing: |cx| MusicPlayerState::try_read(cx).is_some_and(|player| player.is_playing),
    });
}

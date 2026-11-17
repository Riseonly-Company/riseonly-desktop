use gpui::{App, Global};

struct PlayerBeside(bool);

impl Global for PlayerBeside {}

pub fn set_player_beside(shown: bool, cx: &mut App) {
    if player_beside(cx) != shown {
        cx.set_global(PlayerBeside(shown));
        cx.refresh_windows();
    }
}

pub fn player_beside(cx: &App) -> bool {
    cx.try_global::<PlayerBeside>().is_some_and(|beside| beside.0)
}

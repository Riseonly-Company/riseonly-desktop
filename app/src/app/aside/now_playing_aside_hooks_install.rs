use crate::app::aside::aside_navigator;
use crate::modules::music::pages::now_playing::now_playing_aside_hooks::{
    NowPlayingAsideHooks, set_now_playing_aside_hooks,
};

pub fn install_now_playing_aside_hooks() {
    set_now_playing_aside_hooks(NowPlayingAsideHooks {
        observe_frontmost: aside_navigator::observe_frontmost,
        has_frontmost: aside_navigator::has_frontmost,
        set_default: aside_navigator::set_default,
        clear_default: aside_navigator::clear_default,
    });
}

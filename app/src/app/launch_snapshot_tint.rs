use gpui::{App, Hsla};
use rise_theme::theme::theme_store::ThemeStore;

pub enum LaunchSnapshotTint {}

impl LaunchSnapshotTint {
    pub fn install(cx: &mut App) {
        let _ = ThemeStore::shared(cx);
    }

    pub fn launch_background(cx: &mut App) -> Hsla {
        ThemeStore::shared(cx).read(cx).current_theme().bg._100
    }
}

use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

/// what the dev drive reads from the music player
pub struct DevDriveHooks {
    pub music_playing: fn(&App) -> bool,
}

pub(crate) static DEV_DRIVE_HOOKS: FeatureHookSlot<DevDriveHooks> = FeatureHookSlot::new("DEV_DRIVE_HOOKS");

pub fn set_dev_drive_hooks(hooks: DevDriveHooks) {
    let _ = DEV_DRIVE_HOOKS.set(hooks);
}

pub fn dev_drive_hooks() -> Option<&'static DevDriveHooks> {
    DEV_DRIVE_HOOKS.get()
}

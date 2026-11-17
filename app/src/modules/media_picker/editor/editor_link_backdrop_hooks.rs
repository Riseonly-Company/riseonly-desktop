use gpui::AnyElement;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct EditorLinkBackdropHooks {
    pub default_wallpaper_preview: fn() -> AnyElement,
}

pub(crate) static EDITOR_LINK_BACKDROP_HOOKS: FeatureHookSlot<EditorLinkBackdropHooks> =
    FeatureHookSlot::new("EDITOR_LINK_BACKDROP_HOOKS");

pub fn set_editor_link_backdrop_hooks(hooks: EditorLinkBackdropHooks) {
    let _ = EDITOR_LINK_BACKDROP_HOOKS.set(hooks);
}

pub fn editor_link_backdrop_hooks() -> Option<&'static EditorLinkBackdropHooks> {
    EDITOR_LINK_BACKDROP_HOOKS.get()
}

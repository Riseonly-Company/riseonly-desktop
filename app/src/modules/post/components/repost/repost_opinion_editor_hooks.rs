use gpui::{AnyElement, App, ElementId, Window};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

pub struct RepostOpinionEditorHooks {
    pub emoji_tab: fn(
        ElementId,
        Box<dyn Fn(String, &mut Window, &mut App)>,
        Box<dyn Fn(StickerItem, &mut Window, &mut App)>,
        Box<dyn Fn(&mut Window, &mut App)>,
        bool,
        bool,
    ) -> AnyElement,
}

pub(crate) static REPOST_OPINION_EDITOR_HOOKS: FeatureHookSlot<RepostOpinionEditorHooks> =
    FeatureHookSlot::new("REPOST_OPINION_EDITOR_HOOKS");

pub fn set_repost_opinion_editor_hooks(hooks: RepostOpinionEditorHooks) {
    let _ = REPOST_OPINION_EDITOR_HOOKS.set(hooks);
}

pub fn repost_opinion_editor_hooks() -> Option<&'static RepostOpinionEditorHooks> {
    REPOST_OPINION_EDITOR_HOOKS.get()
}

/// emoji tab of the opinion editor, none until the hooks are installed
pub fn repost_opinion_editor_emoji_tab(
    id: impl Into<ElementId>,
    on_select_emoji: impl Fn(String, &mut Window, &mut App) + 'static,
    on_select_custom_emoji: impl Fn(StickerItem, &mut Window, &mut App) + 'static,
    on_backspace: impl Fn(&mut Window, &mut App) + 'static,
    is_active: bool,
    uses_flat_chrome: bool,
) -> Option<AnyElement> {
    repost_opinion_editor_hooks().map(|hooks| {
        (hooks.emoji_tab)(
            id.into(),
            Box::new(on_select_emoji),
            Box::new(on_select_custom_emoji),
            Box::new(on_backspace),
            is_active,
            uses_flat_chrome,
        )
    })
}

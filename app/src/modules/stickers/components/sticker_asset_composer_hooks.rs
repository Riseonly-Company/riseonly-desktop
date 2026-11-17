use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, Entity, IntoElement, Window, div};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;

pub(crate) type StickerAssetPickerComplete = Rc<dyn Fn(MediaPickerResult, &mut Window, &mut App)>;
/// called when the media picker is dismissed
pub(crate) type StickerAssetPickerDismissed = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct StickerAssetComposerHooks {
    pub media_picker_sheet: fn(
        id: ElementId,
        is_presented: Binding<bool>,
        kind: PackKind,
        theme_store: Entity<ThemeStore>,
        on_complete: StickerAssetPickerComplete,
        on_dismissed: StickerAssetPickerDismissed,
    ) -> AnyElement,
}

pub(crate) static STICKER_ASSET_COMPOSER_HOOKS: FeatureHookSlot<StickerAssetComposerHooks> = FeatureHookSlot::new("STICKER_ASSET_COMPOSER_HOOKS");

/// installs the hooks once, a second call keeps the first hooks
pub(crate) fn set_sticker_asset_composer_hooks(hooks: StickerAssetComposerHooks) {
    let _ = STICKER_ASSET_COMPOSER_HOOKS.set(hooks);
}

impl StickerAssetComposerHooks {
    pub(crate) fn media_picker_sheet(
        id: ElementId,
        is_presented: Binding<bool>,
        kind: PackKind,
        theme_store: Entity<ThemeStore>,
        on_complete: StickerAssetPickerComplete,
        on_dismissed: StickerAssetPickerDismissed,
    ) -> AnyElement {
        match STICKER_ASSET_COMPOSER_HOOKS.get() {
            Some(hooks) => (hooks.media_picker_sheet)(id, is_presented, kind, theme_store, on_complete, on_dismissed),
            None => div()
                .on_appear(id, move |window, cx| {
                    is_presented.set(false, window, cx);
                    on_dismissed(window, cx);
                })
                .into_any_element(),
        }
    }
}

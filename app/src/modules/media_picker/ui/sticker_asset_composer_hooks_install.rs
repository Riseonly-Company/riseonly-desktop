use gpui::{AnyElement, App, ElementId, Entity, IntoElement, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::stickers::components::sticker_asset_composer_hooks::{
    StickerAssetComposerHooks, StickerAssetPickerComplete, StickerAssetPickerDismissed,
    set_sticker_asset_composer_hooks,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;

fn media_picker_sheet(
    id: ElementId,
    is_presented: Binding<bool>,
    kind: PackKind,
    theme_store: Entity<ThemeStore>,
    on_complete: StickerAssetPickerComplete,
    on_dismissed: StickerAssetPickerDismissed,
) -> AnyElement {
    MediaPickerSheet::new(
        id,
        is_presented,
        MediaPickerConfiguration::sticker_asset(kind),
        theme_store,
        move |result, window, cx| on_complete(result, window, cx),
    )
    .experimental_bottom_sheet(true)
    .on_dismissed(move |window: &mut Window, cx: &mut App| on_dismissed(window, cx))
    .into_any_element()
}

pub fn install_sticker_asset_composer_hooks() {
    set_sticker_asset_composer_hooks(StickerAssetComposerHooks { media_picker_sheet });
}


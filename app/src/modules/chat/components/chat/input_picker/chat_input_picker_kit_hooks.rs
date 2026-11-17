use gpui::{App, Subscription};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerUploadPresentation;

pub struct ChatInputPickerKitHooks {
    pub sticker_upload: fn(&str, &App) -> Option<RiseStickerUploadPresentation>,
    pub cancel_upload: fn(&RiseStickerUploadPresentation, &mut App),
    pub observe: fn(Box<dyn Fn(&mut App)>, &mut App) -> Subscription,
}

pub(crate) static CHAT_INPUT_PICKER_KIT_HOOKS: FeatureHookSlot<ChatInputPickerKitHooks> = FeatureHookSlot::new("CHAT_INPUT_PICKER_KIT_HOOKS");

impl ChatInputPickerKitHooks {
    pub fn install(hooks: ChatInputPickerKitHooks) {
        let _ = CHAT_INPUT_PICKER_KIT_HOOKS.set(hooks);
    }

    pub fn shared() -> Option<&'static ChatInputPickerKitHooks> {
        CHAT_INPUT_PICKER_KIT_HOOKS.get()
    }

    pub fn resolved_sticker_upload(local_sticker_id: &str, cx: &App) -> Option<RiseStickerUploadPresentation> {
        Self::shared().and_then(|hooks| (hooks.sticker_upload)(local_sticker_id, cx))
    }

    pub fn resolved_cancel_upload(upload: &RiseStickerUploadPresentation, cx: &mut App) {
        if let Some(hooks) = Self::shared() {
            (hooks.cancel_upload)(upload, cx);
        }
    }

    pub fn resolved_observe(on_change: Box<dyn Fn(&mut App)>, cx: &mut App) -> Option<Subscription> {
        Self::shared().map(|hooks| (hooks.observe)(on_change, cx))
    }
}

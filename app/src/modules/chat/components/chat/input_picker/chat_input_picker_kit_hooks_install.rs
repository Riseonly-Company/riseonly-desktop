use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit_hooks::ChatInputPickerKitHooks;
use crate::modules::stickers::components::sticker_pack_add_cell::StickerAssetComposerStore;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

pub(crate) fn install_chat_input_picker_kit_hooks() {
    ChatInputPickerKitHooks::install(ChatInputPickerKitHooks {
        sticker_upload: |local_sticker_id, cx| {
            rise_ui::shared::try_read::<StickersServicesStore>(cx)
                .and_then(|store| store.sticker_upload(local_sticker_id))
        },
        cancel_upload: |upload, cx| {
            let owned_by_composer = StickerAssetComposerStore::shared(cx)
                .update(cx, |composer, cx| composer.cancel_upload(&upload.request_id, &upload.pack_id, cx));
            if !owned_by_composer {
                StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.cancel_sticker_upload(&upload.pack_id, &upload.request_id, cx);
                });
            }
        },
        observe: |on_change, cx| {
            let store = StickersServicesStore::shared(cx);
            cx.observe(&store, move |_, cx| on_change(cx))
        },
    });
}

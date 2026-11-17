use gpui::App;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::message_sticker_view_hooks::{
    MessageStickerViewHooks, set_message_sticker_view_hooks,
};
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::StickerMediaDisplayURL;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

fn services(cx: &App) -> Option<&StickersServicesStore> {
    rise_ui::shared::try_read::<StickersServicesStore>(cx)
}

pub fn install_message_sticker_view_hooks() {
    set_message_sticker_view_hooks(MessageStickerViewHooks {
        effective_user_id_for_wire: |cx| {
            rise_ui::shared::try_read::<AuthServicesStore>(cx).and_then(|store| store.effective_user_id_for_wire())
        },
        protects_content: |message, chat| MessagePermissions::new(message, chat, false).protects_content(),
        pack_stickers: |pack_id, cx| services(cx).map(|store| store.pack_stickers(pack_id)).unwrap_or_default(),
        is_favourite: |user_id, sticker_id, cx| {
            services(cx).is_some_and(|store| store.is_favourite(user_id, sticker_id))
        },
        is_gif_favourite: |user_id, gif, cx| services(cx).is_some_and(|store| store.is_gif_favourite(user_id, gif)),
        toggle_favourite: |user_id, sticker, cx| {
            StickersInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.toggle_favourite(user_id, sticker, cx));
        },
        toggle_gif_favourite: |user_id, gif, cx| {
            StickersInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.toggle_gif_favourite(user_id, gif, cx));
        },
        resolve: StickerMediaDisplayURL::resolve,
    });
}

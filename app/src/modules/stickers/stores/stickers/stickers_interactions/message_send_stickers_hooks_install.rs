use crate::modules::chat::stores::message::message_interactions::message_send_stickers_hooks::{
    MessageSendStickersHooks, set_message_send_stickers_hooks,
};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;

pub fn install_message_send_stickers_hooks() {
    set_message_send_stickers_hooks(MessageSendStickersHooks {
        write_recent: |user_id, sticker_id, kind, cx| {
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.write_recent(user_id, sticker_id, kind, cx);
            });
        },
        add_gif_favourite: |user_id, gif, cx| {
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.add_gif_favourite(user_id, gif, cx);
            });
        },
    });
}

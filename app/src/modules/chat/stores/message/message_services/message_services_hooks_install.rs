use gpui::App;

use crate::core::lib::sai::sai_messages_cache_hooks::{SaiMessagesCacheHooks, set_sai_messages_cache_hooks};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

pub fn install_message_services_hooks() {
    set_sai_messages_cache_hooks(SaiMessagesCacheHooks {
        notify_messages_cache_did_change: Box::new(|cache_key: &str, cx: &mut App| {
            let cache_key = cache_key.to_owned();
            cx.defer(move |cx| {
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.notify_messages_cache_did_change(&cache_key, cx));
            });
        }),
    });
}

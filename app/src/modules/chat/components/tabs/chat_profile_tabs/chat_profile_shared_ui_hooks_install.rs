use gpui::App;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui_hooks::{
    ChatProfileSharedUiHooks, set_chat_profile_shared_ui_hooks,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn message_tab_render_status(cache_id: &str, chat_id: &str, has_entries: bool, cx: &mut App) -> SaiWsSliceStatus {
    MessageServicesStore::shared(cx)
        .update(cx, |services, cx| services.message_tab_render_status(cache_id, chat_id, has_entries, cx))
}

fn message_tab_error(cache_id: &str, chat_id: &str, cx: &mut App) -> Option<String> {
    MessageServicesStore::shared(cx).update(cx, |services, cx| services.message_tab_error(cache_id, chat_id, cx))
}

pub fn install_chat_profile_shared_ui_hooks() {
    set_chat_profile_shared_ui_hooks(ChatProfileSharedUiHooks { message_tab_render_status, message_tab_error });
}

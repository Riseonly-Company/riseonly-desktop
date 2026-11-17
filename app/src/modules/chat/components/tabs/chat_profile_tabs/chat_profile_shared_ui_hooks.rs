use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;

pub(crate) struct ChatProfileSharedUiHooks {
    pub message_tab_render_status: fn(cache_id: &str, chat_id: &str, has_entries: bool, cx: &mut App) -> SaiWsSliceStatus,
    pub message_tab_error: fn(cache_id: &str, chat_id: &str, cx: &mut App) -> Option<String>,
}

pub(crate) static CHAT_PROFILE_SHARED_UI_HOOKS: FeatureHookSlot<ChatProfileSharedUiHooks> = FeatureHookSlot::new("CHAT_PROFILE_SHARED_UI_HOOKS");

/// installs the hooks once, a second call keeps the first ones
pub(crate) fn set_chat_profile_shared_ui_hooks(hooks: ChatProfileSharedUiHooks) {
    let _ = CHAT_PROFILE_SHARED_UI_HOOKS.set(hooks);
}

pub(crate) fn chat_profile_shared_ui_hooks() -> Option<&'static ChatProfileSharedUiHooks> {
    CHAT_PROFILE_SHARED_UI_HOOKS.get()
}

impl ChatProfileSharedUiHooks {
    pub(crate) fn message_tab_render_status_or_default(
        cache_id: &str,
        chat_id: &str,
        has_entries: bool,
        cx: &mut App,
    ) -> SaiWsSliceStatus {
        chat_profile_shared_ui_hooks().map_or(SaiWsSliceStatus::Idle, |hooks| {
            (hooks.message_tab_render_status)(cache_id, chat_id, has_entries, cx)
        })
    }

    pub(crate) fn message_tab_error_or_default(cache_id: &str, chat_id: &str, cx: &mut App) -> Option<String> {
        chat_profile_shared_ui_hooks().and_then(|hooks| (hooks.message_tab_error)(cache_id, chat_id, cx))
    }
}

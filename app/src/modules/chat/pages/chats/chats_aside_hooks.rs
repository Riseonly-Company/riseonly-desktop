use gpui::App;

use crate::app::router::navigation_types::RootRoute;
use crate::core::feature_hooks::FeatureHookSlot;

/// what the chat pages ask the right panel of the front window
pub struct ChatsAsideHooks {
    pub follow_topic: fn(&str, Option<&RootRoute>, &mut App),
}

pub(crate) static CHATS_ASIDE_HOOKS: FeatureHookSlot<ChatsAsideHooks> = FeatureHookSlot::new("CHATS_ASIDE_HOOKS");

pub fn set_chats_aside_hooks(hooks: ChatsAsideHooks) {
    let _ = CHATS_ASIDE_HOOKS.set(hooks);
}

pub fn chats_aside_hooks() -> Option<&'static ChatsAsideHooks> {
    CHATS_ASIDE_HOOKS.get()
}

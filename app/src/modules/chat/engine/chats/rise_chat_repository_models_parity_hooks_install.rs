use crate::core::rise_engine_bridge::rise_canon_events_parity_hooks::RiseCanonEventsParityChatModelsHooks;
use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatRealtimeMessageKind;

pub fn install_rise_chat_repository_models_parity_hooks() {
    if !(cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")) {
        return;
    }
    RiseCanonEventsParityChatModelsHooks::install(RiseCanonEventsParityChatModelsHooks {
        all_raw_values: Box::new(|| {
            RiseChatRealtimeMessageKind::all_cases().iter().map(|kind| kind.raw_value().to_owned()).collect()
        }),
    });
}

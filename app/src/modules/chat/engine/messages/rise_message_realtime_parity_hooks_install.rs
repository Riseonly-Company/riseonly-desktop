use crate::core::rise_engine_bridge::rise_canon_events_parity_hooks::RiseCanonEventsParityMessageRealtimeHooks;
use crate::modules::chat::engine::messages::rise_message_realtime::RiseMessageRepositoryRealtimeEvent;

pub(crate) fn install_rise_message_realtime_parity_hooks() {
    if !(cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")) {
        return;
    }
    RiseCanonEventsParityMessageRealtimeHooks::install(RiseCanonEventsParityMessageRealtimeHooks {
        all_raw_values: Box::new(|| {
            RiseMessageRepositoryRealtimeEvent::all_cases().iter().map(|event| event.raw_value().to_owned()).collect()
        }),
    });
}

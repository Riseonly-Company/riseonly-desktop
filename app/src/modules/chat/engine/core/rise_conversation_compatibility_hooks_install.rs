use crate::core::lib::sai::sai_conversation_hooks::{SaiConversationHooks, set_sai_conversation_hooks};
use crate::modules::chat::engine::core::rise_conversation_compatibility_bridge::RiseConversationCompatibilityBridge;

pub(crate) fn install_rise_conversation_compatibility_hooks() {
    set_sai_conversation_hooks(SaiConversationHooks { handle: Box::new(RiseConversationCompatibilityBridge::handle) });
}

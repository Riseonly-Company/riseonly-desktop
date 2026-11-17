use futures::future::LocalBoxFuture;
use gpui::App;
use serde_json::Value;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWsOnSuccess;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsRequestOptions;

pub(crate) type SaiConversationHandle = Box<
    dyn Fn(
            Value,
            SaiWsRequestOptions,
            Option<SaiWsOnSuccess>,
            &mut App,
        ) -> LocalBoxFuture<'static, anyhow::Result<Option<u64>>>
        + Send
        + Sync,
>;

pub struct SaiConversationHooks {
    pub handle: SaiConversationHandle,
}

pub(crate) static SAI_CONVERSATION_HOOKS: FeatureHookSlot<SaiConversationHooks> = FeatureHookSlot::new("SAI_CONVERSATION_HOOKS");

pub fn set_sai_conversation_hooks(hooks: SaiConversationHooks) {
    let _ = SAI_CONVERSATION_HOOKS.set(hooks);
}

pub(crate) fn sai_conversation_hooks() -> Option<&'static SaiConversationHooks> {
    SAI_CONVERSATION_HOOKS.get()
}

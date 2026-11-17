use std::sync::LazyLock;
use std::time::Duration;

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use gpui::App;
use rise_canon::{RiseTimingCanon, RiseTimingDebounce};

use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingDebounceExt};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageLinkMetadataWireDTO;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

static LINK_PREVIEW_DEBOUNCE: LazyLock<RiseTimingDebounce> =
    LazyLock::new(|| RiseTimingCanon::debounce("chat_link_preview"));

impl MessageInteractionsStore {
    pub fn load_link_preview(
        url: String,
        cx: &mut App,
    ) -> LocalBoxFuture<'static, anyhow::Result<MessageLinkMetadataWireDTO>> {
        let actions = Self::actions(cx);
        let executor = cx.background_executor().clone();
        let async_cx = cx.to_async();
        async move {
            executor.timer(Duration::from_nanos(LINK_PREVIEW_DEBOUNCE.client_debounce_nanoseconds())).await;
            let load = async_cx.update(|cx| actions.update(cx, |actions, cx| actions.load_link_preview_action(&url, cx)));
            load.await
        }
        .boxed_local()
    }
}

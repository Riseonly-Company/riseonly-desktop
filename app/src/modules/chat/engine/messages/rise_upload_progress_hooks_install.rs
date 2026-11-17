use futures::FutureExt;
use futures::StreamExt;
use gpui::App;
use rise_engine::rise_upload::rise_upload_progress::RiseUploadKey;

use crate::modules::chat::components::chat::bubble::rise_exact_upload_progress_ring_hooks::{
    RiseExactUploadProgressRingHooks, set_rise_exact_upload_progress_ring_hooks,
};
use crate::modules::chat::components::chat::bubble::rise_native_message_image_grid_hooks::{
    RiseNativeMessageImageGridHooks, RiseUploadProgressUpdates, set_rise_native_message_image_grid_hooks,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;

pub(crate) fn install_rise_upload_progress_hooks() {
    set_rise_exact_upload_progress_ring_hooks(RiseExactUploadProgressRingHooks {
        upload_progress_updates: |cx: &App, key: RiseUploadKey| {
            let domain = rise_ui::shared::try_read::<RiseConversationDomainRegistry>(cx)?.active()?;
            let messages = domain.messages.clone();
            Some(async move { messages.upload_progress_updates(key).await }.boxed_local())
        },
    });
    set_rise_native_message_image_grid_hooks(RiseNativeMessageImageGridHooks {
        upload_progress_updates: grid_upload_progress_updates,
    });
}

fn grid_upload_progress_updates(key: RiseUploadKey, cx: &mut App) -> Option<RiseUploadProgressUpdates> {
    let domain = RiseConversationDomainRegistry::shared(cx).read(cx).active()?;
    let messages = domain.messages.clone();
    Some(
        async move {
            let updates = messages.upload_progress_updates(key).await;
            futures::stream::unfold(updates, |mut updates| async move {
                let element = updates.next().await?;
                Some((element, updates))
            })
            .boxed_local()
        }
        .boxed_local(),
    )
}

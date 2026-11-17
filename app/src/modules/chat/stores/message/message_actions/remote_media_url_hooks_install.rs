use crate::core::media::rise_media_transfer_hub_hooks::RiseMediaTransferHubHooks;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

pub(crate) fn install_remote_media_url_hooks() {
    RiseMediaTransferHubHooks::install(RiseMediaTransferHubHooks {
        optional_normalized: RemoteMediaURL::optional_normalized,
    });
}

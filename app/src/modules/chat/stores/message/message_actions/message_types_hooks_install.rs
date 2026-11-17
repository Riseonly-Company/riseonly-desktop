use crate::core::lib::sai::sai_remote_media_url_hooks::{SaiRemoteMediaUrlHooks, set_sai_remote_media_url_hooks};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

pub(crate) fn install_message_types_hooks() {
    set_sai_remote_media_url_hooks(SaiRemoteMediaUrlHooks {
        optional_normalized: Box::new(RemoteMediaURL::optional_normalized),
    });
}

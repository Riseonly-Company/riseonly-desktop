use std::sync::Arc;

use rise_ui::swiftui::environment::EnvKey;

use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// shared and not owned, the list hands one snapshot to every row
/// a per row copy of the transcript would make render quadratic
pub struct ChatGalleryMessagesKey;

impl EnvKey for ChatGalleryMessagesKey {
    type Value = Option<Arc<Vec<Arc<GetMessageMessage>>>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// whether the gallery may page beyond the messages it was handed
/// off only for filtered sets like pinned, moderation case or saved tab
/// paging there would leave the set
pub struct ChatGalleryPaginatesKey;

impl EnvKey for ChatGalleryPaginatesKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        true
    }
}

use rise_canon::rise_key::upload;
use rise_canon::rise_wire_canon::key as wire_key;
use serde_json::{Map, Value};

use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

/// stable cover url out of a finished upload
/// lives with the upload helpers so music, chat
/// contact, bot and vacancy stores can use it without a ui dependency
pub enum MusicCoverUpload {}

impl MusicCoverUpload {
    pub fn extract_remote_url(done: &Map<String, Value>) -> Option<String> {
        if let Some(url) = done.get(wire_key::DATA).and_then(Value::as_object).and_then(Self::extract_remote_url) {
            return Some(url);
        }
        RemoteMediaURL::optional_normalized(done.get(upload::FILE_URL).and_then(Value::as_str))
            .filter(|normalized| !normalized.is_empty())
    }
}

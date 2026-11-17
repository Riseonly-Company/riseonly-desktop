use rise_canon::RiseEvent;
use serde_json::Value;

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;

/// decodes a socket event payload into the canon event type
pub trait RiseEventPayload: Sized {
    /// reads the data object of a socket handler as the named event
    /// none if its not an object or doesnt decode
    fn decode_payload(name: &str, payload: Option<&Value>) -> Option<Self>;
}

impl RiseEventPayload for RiseEvent {
    fn decode_payload(name: &str, payload: Option<&Value>) -> Option<Self> {
        let object = AnyDictionaryBridge::dictionary(payload)?;
        let data = serde_json::to_vec(&Value::Object(object)).ok()?;
        RiseEvent::decode(name, &data).ok()
    }
}

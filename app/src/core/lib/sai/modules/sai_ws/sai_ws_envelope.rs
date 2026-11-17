use rise_canon::RiseWireRequestMetadata;
use rise_canon::rise_wire_canon::{OUTBOUND_FRAME_TYPE, key};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_engine::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;

pub enum SaiWsEnvelopeBuilder {}

impl SaiWsEnvelopeBuilder {
    pub fn make_outbound_id() -> u64 {
        RiseRequestIDAllocator::shared().next(RiseClock::now()).raw_value()
    }

    /// seconds from a request id, ids below the canon minimum give zero instead of crashing
    pub fn make_timestamp_seconds(id_millis: u64) -> i64 {
        RiseRequestID::new(id_millis).map_or(0, RiseRequestIDAllocator::timestamp_seconds)
    }

    pub fn json_string(
        service: &str,
        method: &str,
        data_value: &Value,
        id_millis: u64,
        timestamp_seconds: i64,
    ) -> anyhow::Result<String> {
        let metadata = Self::json_object_from_encodable(&RiseWireRequestMetadata::default())?;

        let mut root = Map::new();
        root.insert(key::ID.to_owned(), Value::from(id_millis));
        root.insert(key::FRAME_TYPE.to_owned(), Value::from(OUTBOUND_FRAME_TYPE));
        root.insert(key::SERVICE.to_owned(), Value::from(service));
        root.insert(key::METHOD.to_owned(), Value::from(method));
        root.insert(key::DATA.to_owned(), data_value.clone());
        root.insert(key::TIMESTAMP.to_owned(), Value::from(timestamp_seconds));
        root.insert(key::METADATA.to_owned(), metadata);

        let options = JSONWritingOptions {
            without_escaping_slashes: true,
            ..JSONWritingOptions::default()
        };
        let Some(data) = JSONSerialization::data_with_json_object(&Value::Object(root), options) else {
            return Err(SaiWsSendError::EncodingFailed("JSON data from frame".to_owned()).into());
        };
        String::from_utf8(data)
            .map_err(|_| SaiWsSendError::EncodingFailed("UTF-8 string from JSON data".to_owned()).into())
    }

    pub fn json_object_from_encodable<T: Serialize + ?Sized>(value: &T) -> anyhow::Result<Value> {
        Ok(serde_json::to_value(value)?)
    }
}

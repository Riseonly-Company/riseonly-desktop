use rise_engine::{CancellationError, RiseJSONValue};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub enum RiseConversationCodec {}

impl RiseConversationCodec {
    /// request bodies already use wire names so encode stays literal
    pub async fn encode<Value: Serialize + Send + 'static>(value: Value) -> anyhow::Result<Vec<u8>> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> { Ok(RiseJSONValue::encode(&value)?.data(true)?) })
            .await
            .map_err(|_| CancellationError)?
    }

    /// responses arrive in snake case wire shape, canon decoders take that directly
    pub async fn decode_wire<Value: DeserializeOwned + Send + 'static>(data: Vec<u8>) -> anyhow::Result<Value> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Value> { Ok(serde_json::from_slice(&data)?) })
            .await
            .map_err(|_| CancellationError)?
    }

    pub fn encode_cursor(value: Option<&str>) -> Option<Vec<u8>> {
        value.map(|value| value.as_bytes().to_vec())
    }

    pub fn decode_cursor(value: Option<&[u8]>) -> Option<String> {
        value.and_then(|value| String::from_utf8(value.to_vec()).ok())
    }
}

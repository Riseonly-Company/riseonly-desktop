use std::collections::HashMap;

use rise_canon::rise_http_path;
use rise_canon::rise_key::attestation;
use rise_platform::HostOs;
use serde::ser::{Error as _, SerializeMap};
use serde::{Serialize, Serializer};

use crate::modules::auth::shared::device_identity::DeviceIdentity;

/// app attest proof for entry calls
/// desktop has no verifiable key so calls carry no attest headers
/// server counts them in shadow mode
pub enum AppAttestClient {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppAttestClientEnvelope {
    pub platform: String,
    pub challenge: String,
    pub key_id: String,
    pub attestation: Option<String>,
    pub assertion: Option<String>,
}

impl AppAttestClientEnvelope {
    pub fn new(
        challenge: String,
        key_id: String,
        attestation: Option<String>,
        assertion: Option<String>,
    ) -> Self {
        Self {
            platform: DeviceIdentity::platform(HostOs::current()).to_owned(),
            challenge,
            key_id,
            attestation,
            assertion,
        }
    }
}

impl Serialize for AppAttestClientEnvelope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(attestation::PLATFORM, &self.platform)?;
        container.serialize_entry(attestation::CHALLENGE, &self.challenge)?;
        container.serialize_entry(attestation::KEY_ID, &self.key_id)?;
        if let Some(value) = &self.attestation {
            container.serialize_entry(attestation::ATTESTATION, value)?;
        }
        if let Some(value) = &self.assertion {
            container.serialize_entry(attestation::ASSERTION, value)?;
        }
        container.end()
    }
}

impl AppAttestClient {
    pub const CHALLENGE_PATH: &'static str = rise_http_path::auth::ATTESTATION_CHALLENGE;

    pub fn is_supported() -> bool {
        false
    }

    /// envelope for a register or sign in or appeal call
    /// none means send nothing, not an error to show on desktop always none
    pub async fn envelope() -> Option<AppAttestClientEnvelope> {
        None
    }

    /// headers binding one sensitive call to this device
    /// empty when theres no attested key, the call goes out anyway and the gateway only counts it
    pub async fn request_headers(
        _service: &str,
        _method: &str,
        _data: &[u8],
    ) -> HashMap<String, String> {
        HashMap::new()
    }

    pub fn forget_key() {}
}

/// canon request body with the device app attest envelope beside its fields
/// the gateway reads attestation from the raw body so its a policy input of the entry flow
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseAttestedBody<Body> {
    pub body: Body,
    pub attestation: Option<AppAttestClientEnvelope>,
}

impl<Body: Serialize> Serialize for RiseAttestedBody<Body> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let serde_json::Value::Object(mut container) =
            serde_json::to_value(&self.body).map_err(S::Error::custom)?
        else {
            return Err(S::Error::custom("a canon request body encodes as an object"));
        };
        if let Some(envelope) = &self.attestation {
            let value = serde_json::to_value(envelope).map_err(S::Error::custom)?;
            container.insert(attestation::ATTESTATION.to_owned(), value);
        }
        container.serialize(serializer)
    }
}

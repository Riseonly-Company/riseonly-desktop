use std::time::Duration;

use serde::{Deserialize, Serialize};

use rise_core::RequestId;

/// The message a gateway error field actually carries, or `None` when it carries
/// nothing.
///
/// A proto `string` has no absent form, so `error` is present on SUCCESSFUL
/// replies too, holding `""`. Test for a message, never for the field.
pub fn remote_message(field: Option<&str>) -> Option<&str> {
    field.map(str::trim).filter(|message| !message.is_empty())
}

/// Whether a request may be sent again after an inconclusive failure. Set it from
/// the backend's guarantee, never from a client guess.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReplayPolicy {
    Never,
    ReadOnly,
    RequestId,
    IdempotencyKey { field: &'static str },
}

impl ReplayPolicy {
    pub fn permits_replay(&self) -> bool {
        !matches!(self, Self::Never)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MethodDescriptor {
    pub service: &'static str,
    pub method: &'static str,
    pub timeout: Duration,
    pub replay: ReplayPolicy,
}

impl MethodDescriptor {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

    pub const fn read(service: &'static str, method: &'static str) -> Self {
        Self {
            service,
            method,
            timeout: Self::DEFAULT_TIMEOUT,
            replay: ReplayPolicy::ReadOnly,
        }
    }

    pub const fn mutation(service: &'static str, method: &'static str) -> Self {
        Self {
            service,
            method,
            timeout: Self::DEFAULT_TIMEOUT,
            replay: ReplayPolicy::Never,
        }
    }

    pub const fn request_id_mutation(service: &'static str, method: &'static str) -> Self {
        Self {
            service,
            method,
            timeout: Self::DEFAULT_TIMEOUT,
            replay: ReplayPolicy::RequestId,
        }
    }

    pub const fn idempotent_mutation(
        service: &'static str,
        method: &'static str,
        field: &'static str,
    ) -> Self {
        Self {
            service,
            method,
            timeout: Self::DEFAULT_TIMEOUT,
            replay: ReplayPolicy::IdempotencyKey { field },
        }
    }

    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.service, self.method)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RequestEnvelope<T> {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub service: &'static str,
    pub method: &'static str,
    pub data: T,
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<RequestMetadata>,
}

impl<T> RequestEnvelope<T> {
    pub const KIND: &'static str = "service_call";

    pub fn new(id: RequestId, descriptor: &MethodDescriptor, data: T, timestamp: u64) -> Self {
        Self {
            id: id.get(),
            kind: Self::KIND,
            service: descriptor.service,
            method: descriptor.method,
            data,
            timestamp,
            metadata: None,
        }
    }

    pub fn with_metadata(mut self, metadata: RequestMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RequestMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ResponseStatus {
    Success,
    Error,
}

#[derive(Clone, PartialEq, Debug)]
pub struct RemoteError {
    pub code: Option<String>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_proto_string_is_not_a_message() {
        assert_eq!(remote_message(Some("")), None);
        assert_eq!(remote_message(Some("   ")), None);
        assert_eq!(remote_message(Some("\n\t")), None);
        assert_eq!(remote_message(None), None);
    }

    #[test]
    fn a_real_message_survives_trimmed() {
        assert_eq!(
            remote_message(Some("  Tag already exists  ")),
            Some("Tag already exists")
        );
    }

    #[test]
    fn only_never_forbids_replay() {
        assert!(!ReplayPolicy::Never.permits_replay());
        assert!(ReplayPolicy::ReadOnly.permits_replay());
        assert!(ReplayPolicy::RequestId.permits_replay());
        assert!(ReplayPolicy::IdempotencyKey { field: "key" }.permits_replay());
    }

    #[test]
    fn a_plain_mutation_defaults_to_no_replay() {
        let descriptor = MethodDescriptor::mutation("chat", "delete_message");
        assert_eq!(descriptor.replay, ReplayPolicy::Never);
        assert!(!descriptor.replay.permits_replay());
    }

    #[test]
    fn descriptors_carry_the_default_timeout_until_overridden() {
        let default = MethodDescriptor::read("user", "get_profile");
        assert_eq!(default.timeout, MethodDescriptor::DEFAULT_TIMEOUT);

        let slow =
            MethodDescriptor::read("user", "get_profile").with_timeout(Duration::from_secs(60));
        assert_eq!(slow.timeout, Duration::from_secs(60));
    }

    #[test]
    fn the_envelope_serialises_to_the_gateway_shape() {
        let descriptor = MethodDescriptor::read("auth", "sign_in");
        let id = RequestId::new(4242).unwrap();
        let envelope = RequestEnvelope::new(id, &descriptor, serde_json::json!({"a": 1}), 99);

        let encoded = serde_json::to_value(&envelope).unwrap();
        assert_eq!(encoded["id"], 4242);
        assert_eq!(encoded["type"], "service_call");
        assert_eq!(encoded["service"], "auth");
        assert_eq!(encoded["method"], "sign_in");
        assert_eq!(encoded["data"]["a"], 1);
        assert_eq!(encoded["timestamp"], 99);
        assert!(
            encoded.get("metadata").is_none(),
            "absent metadata must not be emitted"
        );
    }

    #[test]
    fn qualified_name_is_service_dot_method() {
        assert_eq!(
            MethodDescriptor::read("post", "get_feed").qualified_name(),
            "post.get_feed"
        );
    }
}

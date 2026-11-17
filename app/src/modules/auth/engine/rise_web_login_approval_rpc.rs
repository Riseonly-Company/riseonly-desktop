use std::fmt;
use std::sync::Arc;

use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};

/// reasons a web login approval can be refused
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseWebLoginApprovalRefusal {
    Expired,
    Refused,
}

impl fmt::Display for RiseWebLoginApprovalRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expired => f.write_str("RiseWebLoginApprovalRefusal.expired"),
            Self::Refused => f.write_str("RiseWebLoginApprovalRefusal.refused"),
        }
    }
}

impl std::error::Error for RiseWebLoginApprovalRefusal {}

#[derive(Clone)]
pub struct RiseWebLoginApprovalRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseWebLoginApprovalRPC {
    pub async fn send(&self, login_token: &str, approve: bool) -> anyhow::Result<()> {
        let endpoint = RiseCanonEndpoints::required(rise_wire_methods::auth::SERVICE, rise_wire_methods::auth::APPROVE_WEB_LOGIN);
        let descriptor = RiseWireMethodDescriptor::new(endpoint.service, endpoint.method)
            .with_timeout(endpoint.client_timeout())
            .with_replay_policy(RiseWireReplayPolicy::Never);
        let request = rise_request::auth::ApproveWebLogin {
            approve: Some(approve),
            login_token: Some(login_token.to_owned()),
        };
        let response: rise_response::auth::ApproveWebLogin = self
            .engine
            .call_descriptor_response(&descriptor, &request, None, RiseWireRequestMetadata::default())
            .await?;
        let reason = response.error;
        if reason.is_empty() {
            return Ok(());
        }
        Err(if reason.contains("expired") {
            RiseWebLoginApprovalRefusal::Expired
        } else {
            RiseWebLoginApprovalRefusal::Refused
        }
        .into())
    }
}

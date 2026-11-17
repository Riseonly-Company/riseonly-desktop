use std::fmt;
use std::sync::Arc;

use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::modules::vacancy::stores::vacancy_actions::vacancy_wire_types::WireResumeDTO;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseVacancyResumeCannotParseResponse;

impl fmt::Display for RiseVacancyResumeCannotParseResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("URLError(cannotParseResponse)")
    }
}

impl std::error::Error for RiseVacancyResumeCannotParseResponse {}

#[derive(Clone)]
pub struct RiseVacancyResumeRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseVacancyResumeRPC {
    pub async fn update(&self, request: &rise_request::vacancy::UpdateResume) -> anyhow::Result<WireResumeDTO> {
        let endpoint =
            RiseCanonEndpoints::required(rise_wire_methods::vacancy::SERVICE, rise_wire_methods::vacancy::UPDATE_RESUME);
        let descriptor = RiseWireMethodDescriptor::new(endpoint.service, endpoint.method)
            .with_timeout(endpoint.client_timeout())
            .with_replay_policy(RiseWireReplayPolicy::Never);
        let response: rise_response::vacancy::UpdateResume = self
            .engine
            .call_descriptor_response(&descriptor, request, None, RiseWireRequestMetadata::default())
            .await?;
        match response.resume {
            Some(resume) if Some(resume.id) == request.resume_id => Ok(resume),
            _ => Err(RiseVacancyResumeCannotParseResponse.into()),
        }
    }
}

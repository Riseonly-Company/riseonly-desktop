use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};
use crate::modules::guides::engine::rise_guides_wire_models::{
    RiseAckGuideResponseDTO, RiseGuideResponseDTO, RiseGuidesResponseDTO,
    RiseSetGuideAutoStartResponseDTO,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseGuidesEndpointReplay {
    ReadOnly,
    Never,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseGuidesEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseGuidesEndpointReplay,
}

impl RiseGuidesEndpoint {
    const SERVICE: &'static str = rise_wire_methods::notify::SERVICE;

    /// deadline and replay policy come from the canon catalogue, expecting only asserts the
    /// declaration matches
    #[track_caller]
    pub fn canonical(method: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(Self::SERVICE, method);
        debug_assert!(
            entry.kind == expecting,
            "{}.{method}: canon declares {}, not {}",
            Self::SERVICE,
            entry.kind.name(),
            expecting.name()
        );
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            replay: if entry.kind == RiseEndpointKind::Read {
                RiseGuidesEndpointReplay::ReadOnly
            } else {
                RiseGuidesEndpointReplay::Never
            },
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(Self::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(match self.replay {
                RiseGuidesEndpointReplay::ReadOnly => RiseWireReplayPolicy::ReadOnly,
                RiseGuidesEndpointReplay::Never => RiseWireReplayPolicy::Never,
            })
    }
}

static GET_PENDING_GUIDES: LazyLock<RiseGuidesEndpoint> = LazyLock::new(|| {
    RiseGuidesEndpoint::canonical(rise_wire_methods::notify::GET_PENDING_GUIDES, RiseEndpointKind::Read)
});

static GET_GUIDE: LazyLock<RiseGuidesEndpoint> = LazyLock::new(|| {
    RiseGuidesEndpoint::canonical(rise_wire_methods::notify::GET_GUIDE, RiseEndpointKind::Read)
});

static ACK_GUIDE: LazyLock<RiseGuidesEndpoint> = LazyLock::new(|| {
    RiseGuidesEndpoint::canonical(rise_wire_methods::notify::ACK_GUIDE, RiseEndpointKind::Mutation)
});

static SET_GUIDE_AUTO_START: LazyLock<RiseGuidesEndpoint> = LazyLock::new(|| {
    RiseGuidesEndpoint::canonical(rise_wire_methods::notify::SET_GUIDE_AUTO_START, RiseEndpointKind::Mutation)
});

pub enum RiseGuidesEndpoints {}

impl RiseGuidesEndpoints {
    pub fn get_pending_guides() -> &'static RiseGuidesEndpoint {
        &GET_PENDING_GUIDES
    }

    pub fn get_guide() -> &'static RiseGuidesEndpoint {
        &GET_GUIDE
    }

    pub fn ack_guide() -> &'static RiseGuidesEndpoint {
        &ACK_GUIDE
    }

    pub fn set_guide_auto_start() -> &'static RiseGuidesEndpoint {
        &SET_GUIDE_AUTO_START
    }

    pub fn all() -> [&'static RiseGuidesEndpoint; 4] {
        [
            Self::get_pending_guides(),
            Self::get_guide(),
            Self::ack_guide(),
            Self::set_guide_auto_start(),
        ]
    }

    pub fn endpoint(method: &str) -> Option<&'static RiseGuidesEndpoint> {
        let method = trimming_whitespaces_and_newlines(method);
        Self::all().into_iter().find(|endpoint| endpoint.method == method)
    }
}

#[derive(Clone)]
pub struct RiseGuidesRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseGuidesRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn get_pending_guides(
        &self,
        request: rise_request::notify::GetPendingGuides,
    ) -> anyhow::Result<RiseGuidesResponseDTO> {
        self.call(RiseGuidesEndpoints::get_pending_guides(), &request).await
    }

    pub async fn get_guide(
        &self,
        request: rise_request::notify::GetGuide,
    ) -> anyhow::Result<RiseGuideResponseDTO> {
        self.call(RiseGuidesEndpoints::get_guide(), &request).await
    }

    pub async fn ack_guide(
        &self,
        request: rise_request::notify::AckGuide,
    ) -> anyhow::Result<RiseAckGuideResponseDTO> {
        self.call(RiseGuidesEndpoints::ack_guide(), &request).await
    }

    pub async fn set_guide_auto_start(
        &self,
        request: rise_request::notify::SetGuideAutoStart,
    ) -> anyhow::Result<RiseSetGuideAutoStartResponseDTO> {
        self.call(RiseGuidesEndpoints::set_guide_auto_start(), &request).await
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseGuidesEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: serde::Serialize + Sync,
        Response: serde::de::DeserializeOwned,
    {
        self.engine
            .call_descriptor_response(
                &endpoint.descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }
}

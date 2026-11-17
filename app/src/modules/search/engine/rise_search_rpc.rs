use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_wire_methods};
use rise_core::foundation::foundation_date::Date;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::modules::search::engine::rise_search_debug_trace::RiseSearchDebugTrace;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseSearchEndpoint {
    pub service: String,
    pub method: String,
    pub timeout: Duration,
}

impl RiseSearchEndpoint {
    /// descriptor and deadline come from the canon catalog, search is a read so one retry policy
    #[track_caller]
    pub fn new(service: &str, method: &str) -> Self {
        let entry = RiseCanonEndpoints::required(service, method);
        debug_assert!(
            entry.kind == RiseEndpointKind::Read,
            "{service}.{method}: canon declares {}, not read",
            entry.kind.name()
        );
        Self { service: entry.service.to_owned(), method: entry.method.to_owned(), timeout: entry.client_timeout() }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(self.service.clone(), self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(RiseWireReplayPolicy::ReadOnly)
    }
}

static GLOBAL_SEARCH: LazyLock<RiseSearchEndpoint> =
    LazyLock::new(|| RiseSearchEndpoint::new(rise_wire_methods::search::SERVICE, rise_wire_methods::search::GLOBAL_SEARCH));

pub enum RiseSearchEndpoints {}

impl RiseSearchEndpoints {
    pub fn global_search() -> &'static RiseSearchEndpoint {
        &GLOBAL_SEARCH
    }

    pub fn all() -> Vec<&'static RiseSearchEndpoint> {
        vec![Self::global_search()]
    }
}

#[derive(Clone)]
pub struct RiseSearchRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseSearchRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn call<Request, Response>(
        &self,
        endpoint: &RiseSearchEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        let started_at = Date::now();
        RiseSearchDebugTrace::network_started(endpoint);
        let result = self
            .engine
            .call_descriptor_response::<Request, Response>(
                &endpoint.descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await;
        RiseSearchDebugTrace::network_finished(endpoint, started_at, result.as_ref().err());
        result
    }
}

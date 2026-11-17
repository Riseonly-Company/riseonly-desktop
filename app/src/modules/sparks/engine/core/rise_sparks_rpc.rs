use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::rise_key;
use rise_canon::rise_request::ledger as ledger_request;
use rise_canon::rise_request::subscription as subscription_request;
use rise_canon::{RiseWireRequestMetadata, rise_wire_methods};
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseJSONValue, RiseWireError, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use rise_canon::rise_ledger::HistoryFilter as RiseLedgerHistoryFilter;

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};
use crate::modules::auth::services::app_attest_client::AppAttestClient;
use crate::modules::sparks::engine::rise_sparks_engine_models::{
    RiseSparksBalanceResponseDTO, RiseSparksEarningsResponseDTO, RiseSparksHistoryResponseDTO,
    RiseSparksPurchaseResponseDTO, RiseSparksTransferResponseDTO,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseSparksEndpoint {
    pub service: String,
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseWireReplayPolicy,
}

impl RiseSparksEndpoint {
    /// builds the descriptor for a canon method and expected kind
    #[track_caller]
    pub fn new(canonical: &str, method: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(canonical, method);
        debug_assert!(
            entry.kind == expecting,
            "{canonical}.{method}: canon declares {}, not {}",
            entry.kind.name(),
            expecting.name()
        );
        let replay = match entry.kind {
            RiseEndpointKind::Read => RiseWireReplayPolicy::ReadOnly,
            RiseEndpointKind::IdempotentMutation => RiseWireReplayPolicy::IdempotencyKey {
                field: entry
                    .idempotency_field
                    .unwrap_or(rise_key::ledger::IDEMPOTENCY_KEY)
                    .to_owned(),
            },
            RiseEndpointKind::RequestIdMutation => RiseWireReplayPolicy::RequestID,
            RiseEndpointKind::Mutation => RiseWireReplayPolicy::Never,
        };
        Self {
            service: entry.service.to_owned(),
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            replay,
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(self.service.clone(), self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(self.replay.clone())
    }
}

static GET_BALANCE: LazyLock<RiseSparksEndpoint> = LazyLock::new(|| {
    RiseSparksEndpoint::new(
        rise_wire_methods::ledger::SERVICE,
        rise_wire_methods::ledger::GET_SPARKS_BALANCE,
        RiseEndpointKind::Read,
    )
});

static LIST_HISTORY: LazyLock<RiseSparksEndpoint> = LazyLock::new(|| {
    RiseSparksEndpoint::new(
        rise_wire_methods::ledger::SERVICE,
        rise_wire_methods::ledger::LIST_SPARKS_HISTORY,
        RiseEndpointKind::Read,
    )
});

static GET_EARNINGS: LazyLock<RiseSparksEndpoint> = LazyLock::new(|| {
    RiseSparksEndpoint::new(
        rise_wire_methods::ledger::SERVICE,
        rise_wire_methods::ledger::GET_SPARKS_EARNINGS,
        RiseEndpointKind::Read,
    )
});

static TRANSFER_EARNED: LazyLock<RiseSparksEndpoint> = LazyLock::new(|| {
    RiseSparksEndpoint::new(
        rise_wire_methods::ledger::SERVICE,
        rise_wire_methods::ledger::TRANSFER_EARNED_SPARKS,
        RiseEndpointKind::Mutation,
    )
});

static VERIFY_PURCHASE: LazyLock<RiseSparksEndpoint> = LazyLock::new(|| {
    RiseSparksEndpoint::new(
        rise_wire_methods::subscription::SERVICE,
        rise_wire_methods::subscription::VERIFY_SPARKS_PURCHASE,
        RiseEndpointKind::IdempotentMutation,
    )
});

pub enum RiseSparksEndpoints {}

impl RiseSparksEndpoints {
    pub fn get_balance() -> &'static RiseSparksEndpoint {
        &GET_BALANCE
    }

    pub fn list_history() -> &'static RiseSparksEndpoint {
        &LIST_HISTORY
    }

    pub fn get_earnings() -> &'static RiseSparksEndpoint {
        &GET_EARNINGS
    }

    pub fn transfer_earned() -> &'static RiseSparksEndpoint {
        &TRANSFER_EARNED
    }

    pub fn verify_purchase() -> &'static RiseSparksEndpoint {
        &VERIFY_PURCHASE
    }
}

#[derive(Clone)]
pub struct RiseSparksRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseSparksRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn balance(&self) -> anyhow::Result<RiseSparksBalanceResponseDTO> {
        self.call(RiseSparksEndpoints::get_balance(), &ledger_request::GetSparksBalance {}).await
    }

    pub async fn earnings(&self) -> anyhow::Result<RiseSparksEarningsResponseDTO> {
        self.call(RiseSparksEndpoints::get_earnings(), &ledger_request::GetSparksEarnings {}).await
    }

    pub async fn transfer_earned(&self) -> anyhow::Result<RiseSparksTransferResponseDTO> {
        self.call(RiseSparksEndpoints::transfer_earned(), &ledger_request::TransferEarnedSparks {}).await
    }

    pub async fn history(
        &self,
        filter: RiseLedgerHistoryFilter,
        relative_id: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<RiseSparksHistoryResponseDTO> {
        self.call(
            RiseSparksEndpoints::list_history(),
            &ledger_request::ListSparksHistory {
                filter: Some(filter.slug().to_owned()),
                limit: Some(limit),
                relative_id: Some(relative_id.unwrap_or_default().to_owned()),
            },
        )
        .await
    }

    pub async fn verify_purchase(
        &self,
        signed_transaction: &str,
        consumption_consent: bool,
    ) -> anyhow::Result<RiseSparksPurchaseResponseDTO> {
        let endpoint = RiseSparksEndpoints::verify_purchase();
        let request = subscription_request::VerifySparksPurchase {
            consumption_consent: Some(consumption_consent),
            signed_transaction: Some(signed_transaction.to_owned()),
        };
        let body = serde_json::to_vec(&request)?;
        let headers = AppAttestClient::request_headers(&endpoint.service, &endpoint.method, &body).await;
        if headers.is_empty() {
            return self.call(endpoint, &request).await;
        }
        let payload = self
            .engine
            .call_descriptor(
                &endpoint.descriptor(),
                &RiseJSONValue::String(String::from_utf8_lossy(&body).into_owned()),
                None,
                RiseWireRequestMetadata {
                    headers: headers.into_iter().collect::<BTreeMap<_, _>>(),
                    ..RiseWireRequestMetadata::default()
                },
            )
            .await?;
        payload
            .decode_as::<RiseSparksPurchaseResponseDTO>()
            .map_err(|_| RiseWireError::ResponseDecodingFailed.into())
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseSparksEndpoint,
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

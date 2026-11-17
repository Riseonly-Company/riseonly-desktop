use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::rise_key;
use rise_canon::rise_request::market as market_request;
use rise_canon::{RiseWireRequestMetadata, rise_wire_methods};
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseJSONValue, RiseWireError, RiseWireMethodDescriptor, RiseWireReplayPolicy};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};
use crate::modules::auth::services::app_attest_client::AppAttestClient;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAppearanceDTO, RiseGiftAppearancesResponseDTO, RiseGiftCatalogItemResponseDTO,
    RiseGiftCatalogResponseDTO, RiseGiftEquipResponseDTO, RiseGiftInstanceResponseDTO,
    RiseGiftItemKind, RiseGiftSendResponseDTO, RiseGiftStateResponseDTO, RiseGiftTabResponseDTO,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseGiftsEndpoint {
    pub service: String,
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseWireReplayPolicy,
}

impl RiseGiftsEndpoint {
    /// endpoint with a method name and the expected kind
    #[track_caller]
    pub fn new(method: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(rise_wire_methods::market::SERVICE, method);
        debug_assert!(
            entry.kind == expecting,
            "{}.{method}: canon declares {}, not {}",
            rise_wire_methods::market::SERVICE,
            entry.kind.name(),
            expecting.name()
        );
        let replay = match entry.kind {
            RiseEndpointKind::Read => RiseWireReplayPolicy::ReadOnly,
            RiseEndpointKind::IdempotentMutation => RiseWireReplayPolicy::IdempotencyKey {
                field: entry
                    .idempotency_field
                    .unwrap_or(rise_key::market::IDEMPOTENCY_KEY)
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

static CATALOG: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::GET_CATALOG, RiseEndpointKind::Read)
});

static CATALOG_ITEM: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::GET_CATALOG_ITEM, RiseEndpointKind::Read)
});

static SEND: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::SEND_GIFT, RiseEndpointKind::IdempotentMutation)
});

static USER_GIFTS: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::LIST_USER_GIFTS, RiseEndpointKind::Read)
});

static INSTANCE: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::GET_GIFT_INSTANCE, RiseEndpointKind::Read)
});

static SET_DISPLAYED: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(
        rise_wire_methods::market::SET_GIFT_DISPLAYED,
        RiseEndpointKind::IdempotentMutation,
    )
});

static SET_PINNED: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::SET_GIFT_PINNED, RiseEndpointKind::IdempotentMutation)
});

static SET_EQUIPPED: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::SET_EQUIPPED, RiseEndpointKind::IdempotentMutation)
});

static APPEARANCES: LazyLock<RiseGiftsEndpoint> = LazyLock::new(|| {
    RiseGiftsEndpoint::new(rise_wire_methods::market::GET_APPEARANCES, RiseEndpointKind::Read)
});

pub enum RiseGiftsEndpoints {}

impl RiseGiftsEndpoints {
    pub fn catalog() -> &'static RiseGiftsEndpoint {
        &CATALOG
    }

    pub fn catalog_item() -> &'static RiseGiftsEndpoint {
        &CATALOG_ITEM
    }

    /// market service dedupes by the buyer key so a retry returns the first order and never
    /// charges twice
    pub fn send() -> &'static RiseGiftsEndpoint {
        &SEND
    }

    pub fn user_gifts() -> &'static RiseGiftsEndpoint {
        &USER_GIFTS
    }

    pub fn instance() -> &'static RiseGiftsEndpoint {
        &INSTANCE
    }

    pub fn set_displayed() -> &'static RiseGiftsEndpoint {
        &SET_DISPLAYED
    }

    pub fn set_pinned() -> &'static RiseGiftsEndpoint {
        &SET_PINNED
    }

    /// keyed by the instance, putting on what is worn or taking off what isnt changes nothing
    pub fn set_equipped() -> &'static RiseGiftsEndpoint {
        &SET_EQUIPPED
    }

    pub fn appearances() -> &'static RiseGiftsEndpoint {
        &APPEARANCES
    }
}

#[derive(Clone)]
pub struct RiseGiftsRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseGiftsRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    /// catalog page for a kind, kind defaults to item and after to none
    pub async fn catalog(
        &self,
        locale: &str,
        kind: RiseGiftItemKind,
        after: Option<i64>,
    ) -> anyhow::Result<RiseGiftCatalogResponseDTO> {
        let item_kind = if kind == RiseGiftItemKind::Item {
            String::new()
        } else {
            kind.slug().to_owned()
        };
        self.call(
            RiseGiftsEndpoints::catalog(),
            &market_request::GetCatalog {
                item_kind: Some(item_kind),
                limit: Some(0),
                locale: Some(locale.to_owned()),
                relative_id: Some(after.map(|after| after.to_string()).unwrap_or_default()),
            },
        )
        .await
    }

    pub async fn catalog_item(
        &self,
        gift_id: i64,
        locale: &str,
    ) -> anyhow::Result<RiseGiftCatalogItemResponseDTO> {
        self.call(
            RiseGiftsEndpoints::catalog_item(),
            &market_request::GetCatalogItem {
                gift_id: Some(gift_id),
                locale: Some(locale.to_owned()),
            },
        )
        .await
    }

    /// spends sparks so the gateway binds it to this device, body travels as a json string
    /// signed by the attest key
    /// without an attested key its a plain call
    pub async fn send(
        &self,
        request: market_request::SendGift,
    ) -> anyhow::Result<RiseGiftSendResponseDTO> {
        let endpoint = RiseGiftsEndpoints::send();
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
            .decode_as::<RiseGiftSendResponseDTO>()
            .map_err(|_| RiseWireError::ResponseDecodingFailed.into())
    }

    /// gifts of a user, kind defaults to none
    pub async fn user_gifts(
        &self,
        owner_id: &str,
        cursor: Option<&str>,
        limit: i64,
        locale: &str,
        kind: Option<RiseGiftItemKind>,
    ) -> anyhow::Result<RiseGiftTabResponseDTO> {
        self.call(
            RiseGiftsEndpoints::user_gifts(),
            &market_request::ListUserGifts {
                item_kind: Some(kind.map(|kind| kind.slug().to_owned()).unwrap_or_default()),
                limit: Some(limit),
                locale: Some(locale.to_owned()),
                owner_id: Some(owner_id.to_owned()),
                relative_id: Some(cursor.unwrap_or_default().to_owned()),
            },
        )
        .await
    }

    pub async fn instance(&self, id: &str, locale: &str) -> anyhow::Result<RiseGiftInstanceResponseDTO> {
        self.call(
            RiseGiftsEndpoints::instance(),
            &market_request::GetGiftInstance {
                instance_id: Some(id.to_owned()),
                locale: Some(locale.to_owned()),
            },
        )
        .await
    }

    pub async fn set_displayed(&self, id: &str, displayed: bool) -> anyhow::Result<RiseGiftStateResponseDTO> {
        self.call(
            RiseGiftsEndpoints::set_displayed(),
            &market_request::SetGiftDisplayed {
                displayed: Some(displayed),
                instance_id: Some(id.to_owned()),
            },
        )
        .await
    }

    pub async fn set_pinned(&self, id: &str, pinned: bool) -> anyhow::Result<RiseGiftStateResponseDTO> {
        self.call(
            RiseGiftsEndpoints::set_pinned(),
            &market_request::SetGiftPinned {
                instance_id: Some(id.to_owned()),
                pinned: Some(pinned),
            },
        )
        .await
    }

    pub async fn set_equipped(&self, id: &str, equipped: bool) -> anyhow::Result<RiseGiftEquipResponseDTO> {
        self.call(
            RiseGiftsEndpoints::set_equipped(),
            &market_request::SetEquipped {
                equipped: Some(equipped),
                instance_id: Some(id.to_owned()),
            },
        )
        .await
    }

    /// what these people wear, asks in canon sized batches and joins the answers
    pub async fn appearances(&self, user_ids: &[String]) -> anyhow::Result<Vec<RiseGiftAppearanceDTO>> {
        let mut seen = HashSet::new();
        let ids: Vec<String> = user_ids
            .iter()
            .filter(|id| !id.is_empty() && seen.insert(id.as_str()))
            .cloned()
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let batch = Self::appearance_batch_max().max(1);
        let mut out = Vec::new();
        for slice in ids.chunks(batch) {
            let response: RiseGiftAppearancesResponseDTO = self
                .call(
                    RiseGiftsEndpoints::appearances(),
                    &market_request::GetAppearances {
                        user_ids: Some(slice.to_vec()),
                    },
                )
                .await?;
            out.extend(response.items);
        }
        Ok(out)
    }

    pub fn appearance_batch_max() -> usize {
        usize::try_from(rise_limits::service_limit::GIFT_APPEARANCE_BATCH_MAX).unwrap_or(usize::MAX)
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseGiftsEndpoint,
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

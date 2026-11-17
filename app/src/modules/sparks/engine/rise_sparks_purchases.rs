use std::sync::Arc;

use futures::future::BoxFuture;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::{CancellationError, RiseEngine};
use rise_canon::rise_ledger::{SPARKS_PACKS, SparksPack, StoreEnvironment};

use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use crate::modules::notify::stores::notify_actions::notify_wire_types::RiseAPNSRegistrationTarget;
use crate::modules::sparks::engine::core::rise_sparks_rpc::RiseSparksRPC;
use crate::modules::sparks::engine::rise_sparks_engine_models::RiseSparksPurchaseResponseDTO;

/// one pack as the sheet shows it, price follows the storefront currency and app language
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseSparksPackOffer {
    pub product_id: String,
    pub amount: i64,
    pub display_price: String,
}

impl RiseSparksPackOffer {
    pub fn id(&self) -> String {
        self.product_id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseSparksPurchaseOutcome {
    Credited { amount: i64, spendable_after: Option<i64> },
    Pending,
    Cancelled,
    Failed { message: String, code: Option<String> },
    ContinueOnPhone { product_id: String },
}

pub type RiseSparksPurchasesOnCredited =
    Arc<dyn Fn(RiseSparksPurchaseResponseDTO) -> BoxFuture<'static, ()> + Send + Sync>;

struct RiseSparksPurchasesState {
    is_shutdown: bool,
    on_credited: Option<RiseSparksPurchasesOnCredited>,
}

pub struct RiseSparksPurchases {
    pub rpc: RiseSparksRPC,
    pub account_token: Option<Uuid>,
    state: RiseIsolated<RiseSparksPurchasesState>,
}

impl RiseSparksPurchases {
    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        Self {
            rpc: RiseSparksRPC::new(engine),
            account_token: Uuid::from_uuid_string(account_id),
            state: RiseIsolated::new(RiseSparksPurchasesState { is_shutdown: false, on_credited: None }),
        }
    }

    pub fn environment() -> StoreEnvironment {
        rise_canon::rise_ledger::store_environment_for_bundle_id(&RiseAPNSRegistrationTarget::bundle_id())
            .unwrap_or(StoreEnvironment::Production)
    }

    pub fn product_ids() -> Vec<String> {
        SPARKS_PACKS.iter().map(|pack| pack.product_id(Self::environment())).collect()
    }

    pub async fn start(&self, on_credited: RiseSparksPurchasesOnCredited) {
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            state.on_credited = Some(on_credited);
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            state.is_shutdown = true;
            state.on_credited = None;
        });
    }

    /// canon packs cheapest first with no price, the sheet sends the buyer to the phone
    pub async fn offers(&self) -> anyhow::Result<Vec<RiseSparksPackOffer>> {
        if self.state.with(|state| state.is_shutdown) {
            return Err(CancellationError.into());
        }
        let environment = Self::environment();
        let mut offers: Vec<RiseSparksPackOffer> = SPARKS_PACKS
            .iter()
            .map(|pack: &SparksPack| RiseSparksPackOffer {
                product_id: pack.product_id(environment),
                amount: i64::from(pack.amount),
                display_price: String::new(),
            })
            .collect();
        offers.sort_by_key(|offer| offer.amount);
        Ok(offers)
    }

    pub async fn settle_unfinished(&self) {}

    /// the sheet shows only the plain line, what went wrong goes to the bug report trail
    pub fn failure_text(text: String, cause: String) -> String {
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, move || format!("sparks: {cause}"));
        text
    }

    pub async fn purchase(&self, product_id: &str) -> RiseSparksPurchaseOutcome {
        if self.state.with(|state| state.is_shutdown) {
            return RiseSparksPurchaseOutcome::Cancelled;
        }
        RiseSparksPurchaseOutcome::ContinueOnPhone { product_id: product_id.to_owned() }
    }
}

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Context, Entity};
use rise_canon::RiseEvent;
use rise_canon::rise_key::subscription as key;
use rise_canon::rise_ledger::{PremiumProductKind, StoreEnvironment, premium_period_token};
use rise_core::foundation::foundation_url::{UrlComponents, UrlQueryItem};
use rise_errors::ErrorCode;
use rise_events::name;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::HostOs;
use rise_platform::open_url::open_url;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticUpdate;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::store_price::{StorePrice, StorePriceBook};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::overlay::premium_gift_celebration_store::PremiumGiftCelebrationStore;
use crate::core::widgets::overlay::subscription_success_store::SubscriptionSuccessStore;
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::auth::shared::device_identity::DeviceIdentity;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::stores::subscription::premium_gift_store::PremiumGiftStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_types::PlanVerifyOrigin;
use crate::modules::user::stores::subscription::subscription_actions::subscription_wire_types::VerifyPurchaseGatewayScalars;
use crate::modules::user::stores::subscription::subscription_services::subscription_services_hooks::SubscriptionServicesHooks;
use crate::modules::user::stores::user::user_actions::user_types::SubscriptionStatus;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProductSubscriptionPeriodUnit {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProductSubscriptionPeriod {
    pub unit: ProductSubscriptionPeriodUnit,
    pub value: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProductIntroductoryOffer {
    pub period: ProductSubscriptionPeriod,
    pub is_free_trial: bool,
}

/// paywall product, desktop sells nothing so catalogue is plan ids with their period
/// price and offer stay empty
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Product {
    pub id: String,
    pub display_name: String,
    pub price: Option<Decimal>,
    pub currency_code: Option<String>,
    pub subscription_period: Option<ProductSubscriptionPeriod>,
    pub introductory_offer: Option<ProductIntroductoryOffer>,
}

impl Product {
    pub fn from_catalogue_id(id: String) -> Self {
        let subscription_period = Self::period_named_by(&id);
        Self {
            display_name: id.clone(),
            id,
            price: None,
            currency_code: None,
            subscription_period,
            introductory_offer: None,
        }
    }

    /// takes the price the phones store gave this product, none while no phone reported one
    pub fn adopt_store_price(&mut self, book: &StorePriceBook) {
        let price = book.price(&self.id);
        self.price = price.map(|price| price.amount);
        self.currency_code = price.map(|price| price.currency_code.clone());
    }

    fn period_named_by(id: &str) -> Option<ProductSubscriptionPeriod> {
        let last = premium_period_token(id)?;
        if let Some(count) = last.strip_suffix('m') {
            let value = count.parse::<i64>().ok()?;
            return Some(ProductSubscriptionPeriod { unit: ProductSubscriptionPeriodUnit::Month, value });
        }
        let value = last.strip_suffix('y')?.parse::<i64>().ok()?;
        Some(ProductSubscriptionPeriod { unit: ProductSubscriptionPeriodUnit::Year, value })
    }
}

/// where the desktop sends a buyer, the same phone app screen by deep link or qr
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SubscriptionPhoneHandoff {
    Purchase { product_id: String },
    Restore,
    Manage,
}

pub struct SubscriptionServicesStore {
    products: Vec<Product>,
    is_loading_products: bool,
    is_purchasing: bool,
    load_error: Option<String>,
    last_error: Option<String>,
    selected_product_id: Option<String>,
    intro_offer_eligibility: HashMap<String, bool>,
    did_start: bool,
    store_diagnostics: Option<String>,
    continue_on_phone: Option<SubscriptionPhoneHandoff>,
    pending_product_id: Option<String>,
    celebrate_on_next_verify: bool,
}

impl SubscriptionServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let book = StorePriceBook::shared(cx);
        cx.observe(&book, |store, book, cx| {
            let book = book.read(cx);
            for product in &mut store.products {
                product.adopt_store_price(book);
            }
            cx.notify();
        })
        .detach();
        Self {
            products: Vec::new(),
            is_loading_products: false,
            is_purchasing: false,
            load_error: None,
            last_error: None,
            selected_product_id: None,
            intro_offer_eligibility: HashMap::new(),
            did_start: false,
            store_diagnostics: None,
            continue_on_phone: None,
            pending_product_id: None,
            celebrate_on_next_verify: false,
        }
    }

    pub fn base_product_ids() -> Vec<&'static str> {
        PremiumProductKind::Plan.products().map(|product| product.product).collect()
    }

    pub(crate) fn store_environment() -> StoreEnvironment {
        match AppEnvironment::current() {
            AppEnvironment::Prod => StoreEnvironment::Production,
            AppEnvironment::Staging => StoreEnvironment::Staging,
            AppEnvironment::Dev => StoreEnvironment::Dev,
        }
    }

    pub fn product_ids() -> Vec<String> {
        PremiumProductKind::Plan.product_ids(Self::store_environment())
    }

    pub fn products(&self) -> &[Product] {
        &self.products
    }

    pub fn is_loading_products(&self) -> bool {
        self.is_loading_products
    }

    pub fn is_purchasing(&self) -> bool {
        self.is_purchasing
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn selected_product_id(&self) -> Option<&str> {
        self.selected_product_id.as_deref()
    }

    pub fn intro_offer_eligibility(&self) -> &HashMap<String, bool> {
        &self.intro_offer_eligibility
    }

    pub fn did_start(&self) -> bool {
        self.did_start
    }

    pub fn store_diagnostics(&self) -> Option<&str> {
        self.store_diagnostics.as_deref()
    }

    pub fn continue_on_phone(&self) -> Option<&SubscriptionPhoneHandoff> {
        self.continue_on_phone.as_ref()
    }

    pub fn phone_link(&self) -> Option<RiseonlyPublicLink> {
        let SubscriptionPhoneHandoff::Purchase { product_id } = self.continue_on_phone()? else {
            return None;
        };
        Some(RiseonlyPublicLink::PurchasePremium { period: premium_period_token(product_id)? })
    }

    pub fn dismiss_continue_on_phone(&mut self, cx: &mut Context<Self>) {
        self.continue_on_phone = None;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.is_purchasing = false;
        self.last_error = None;
        self.pending_product_id = None;
        self.celebrate_on_next_verify = false;
        self.continue_on_phone = None;
        cx.notify();
    }

    pub fn set_last_error(&mut self, message: Option<String>, cx: &mut Context<Self>) {
        self.last_error = message;
        cx.notify();
    }

    pub fn clear_last_error(&mut self, cx: &mut Context<Self>) {
        self.last_error = None;
        cx.notify();
    }

    pub fn finish_pending_transaction_if_needed(&mut self, cx: &mut Context<Self>) {
        self.clear_pending_transaction(cx);
    }

    pub fn clear_pending_transaction(&mut self, cx: &mut Context<Self>) {
        self.pending_product_id = None;
        cx.notify();
    }

    pub fn start_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.did_start {
            return;
        }

        self.did_start = true;
        cx.notify();
        self.load_products(cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.did_start = false;
        cx.notify();
    }

    pub fn load_products(&mut self, cx: &mut Context<Self>) {
        self.is_loading_products = true;
        self.load_error = None;

        let requested = Self::product_ids();
        let mut fetched: Vec<Product> = requested.iter().cloned().map(Product::from_catalogue_id).collect();
        fetched.sort_by_key(|product| Self::months(product).unwrap_or(0));
        let book = StorePriceBook::shared(cx);
        for product in &mut fetched {
            product.adopt_store_price(book.read(cx));
        }
        StorePriceBook::refresh(cx);
        self.products = fetched;
        if self.selected_product_id.as_ref().is_none_or(|selected| {
            !self.products.iter().any(|product| &product.id == selected)
        }) {
            self.selected_product_id = self
                .products
                .iter()
                .find(|product| Self::is_yearly(product))
                .or_else(|| self.products.last())
                .map(|product| product.id.clone());
        }
        let mut eligibility = HashMap::new();
        for product in &self.products {
            if product.introductory_offer.is_some() {
                eligibility.insert(product.id.clone(), false);
            }
        }
        self.intro_offer_eligibility = eligibility;
        let returned: Vec<String> = self.products.iter().map(|product| product.id.clone()).collect();
        self.update_store_diagnostics(&requested, &returned, cx);
        self.is_loading_products = false;
        cx.notify();
    }

    fn update_store_diagnostics(&mut self, requested: &[String], returned: &[String], cx: &mut Context<Self>) {
        let env = AppEnvironment::current().as_str();
        let missing: Vec<&str> =
            requested.iter().filter(|id| !returned.contains(*id)).map(String::as_str).collect();
        let line = format!("IAP loaded {}/{} env={env} missing={}", returned.len(), requested.len(), missing.join(","));
        Sai::log("Subscription", move || line, SaiLogKind::Network, cx);

        if !returned.is_empty() || AppEnvironment::current() == AppEnvironment::Prod {
            self.store_diagnostics = None;
            return;
        }
        self.store_diagnostics = Some(format!("env={env} · platform={}", DeviceIdentity::platform(HostOs::current())));
    }

    pub fn select_product(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_product_id = Some(id);
        cx.notify();
    }

    pub fn purchase_selected(&mut self, cx: &mut Context<Self>) {
        let Some(product) = self
            .selected_product_id
            .as_ref()
            .and_then(|id| self.products.iter().find(|product| &product.id == id))
            .cloned()
        else {
            return;
        };

        self.purchase(&product, cx);
    }

    /// plan is paid in the phone app, the sheet says so and opens the same screen there
    pub fn purchase(&mut self, product: &Product, cx: &mut Context<Self>) {
        self.last_error = None;
        self.continue_on_phone = Some(SubscriptionPhoneHandoff::Purchase { product_id: product.id.clone() });
        cx.notify();
    }

    /// true if the plan is bound to another riseonly account
    pub fn is_held_by_another_account(error: &anyhow::Error) -> bool {
        error
            .downcast_ref::<SaiWsResponseError>()
            .and_then(|error| error.code.as_deref())
            .is_some_and(|code| code == ErrorCode::UserSubscriptionOtherAccount.wire())
    }

    pub fn restore_purchases(&mut self, cx: &mut Context<Self>) {
        self.last_error = None;
        self.continue_on_phone = Some(SubscriptionPhoneHandoff::Restore);
        cx.notify();
        SubscriptionActionsStore::shared(cx).update(cx, |actions, cx| actions.get_subscription_status_action(true, cx));
    }

    pub fn display_title(&self, product: &Product) -> String {
        let fallback = || Self::plan_title_key(&product.id).map(tr).unwrap_or_else(|| product.display_name.clone());
        let Some(period) = product.subscription_period else {
            return fallback();
        };

        match (period.unit, period.value) {
            (ProductSubscriptionPeriodUnit::Year, 1) | (ProductSubscriptionPeriodUnit::Month, 12) => {
                tr("subscription_plan_yearly")
            }
            (ProductSubscriptionPeriodUnit::Month, 6) => tr("subscription_plan_6month"),
            (ProductSubscriptionPeriodUnit::Month, 3) => tr("subscription_plan_3month"),
            (ProductSubscriptionPeriodUnit::Month, 1) => tr("subscription_plan_monthly"),
            _ => fallback(),
        }
    }

    /// human name for a plan reported by product id,
    /// the status has no plan name so plan is the raw catalogue id
    pub fn plan_title(&self, id: Option<&str>) -> String {
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return tr("subscription_plan_active");
        };

        if let Some(product) = self.products.iter().find(|product| product.id.to_lowercase() == id.to_lowercase()) {
            return self.display_title(product);
        }
        Self::plan_title_key(id).map(tr).unwrap_or_else(|| tr("subscription_plan_active"))
    }

    /// exact match on the last _ part, never a suffix test
    /// 2m would swallow 12m
    /// env suffix like _dev or _staging is stripped first
    pub fn plan_title_key(id: &str) -> Option<&'static str> {
        Self::plan_title_key_for_period_token(&premium_period_token(id)?)
    }

    fn plan_title_key_for_period_token(token: &str) -> Option<&'static str> {
        match token {
            "1m" => Some("subscription_plan_monthly"),
            "3m" => Some("subscription_plan_3month"),
            "6m" => Some("subscription_plan_6month"),
            "12m" | "1y" => Some("subscription_plan_yearly"),
            _ => None,
        }
    }

    pub fn renewal_note(&self, product: &Product) -> String {
        let Some(period) = product.subscription_period else {
            return tr("subscription_info_text");
        };

        match (period.unit, period.value) {
            (ProductSubscriptionPeriodUnit::Year, 1) | (ProductSubscriptionPeriodUnit::Month, 12) => {
                tr("subscription_renews_yearly")
            }
            (ProductSubscriptionPeriodUnit::Month, 6) => tr("subscription_renews_6month"),
            (ProductSubscriptionPeriodUnit::Month, 3) => tr("subscription_renews_3month"),
            (ProductSubscriptionPeriodUnit::Month, 1) => tr("subscription_renews_monthly"),
            _ => tr("subscription_info_text"),
        }
    }

    /// empty without a store price, only the phones store knows it
    pub fn display_price(&self, product: &Product) -> String {
        match (&product.price, &product.currency_code) {
            (Some(price), Some(currency_code)) => StorePrice::text(price, currency_code),
            _ => String::new(),
        }
    }

    /// intro offer this account can still take as a free period like 7 days free,
    /// none if used or its a discount not a trial
    pub fn intro_offer_label(&self, product: &Product) -> Option<String> {
        if self.intro_offer_eligibility.get(&product.id) != Some(&true) {
            return None;
        }
        let offer = product.introductory_offer.filter(|offer| offer.is_free_trial)?;
        let count = offer.period.value;
        match offer.period.unit {
            ProductSubscriptionPeriodUnit::Day => Some(tr_args("subscription_trial_days", &[FormatArg::from(count)])),
            ProductSubscriptionPeriodUnit::Week => {
                Some(tr_args("subscription_trial_days", &[FormatArg::from(count * 7)]))
            }
            ProductSubscriptionPeriodUnit::Month => {
                Some(tr_args("subscription_trial_months", &[FormatArg::from(count)]))
            }
            ProductSubscriptionPeriodUnit::Year => {
                Some(tr_args("subscription_trial_months", &[FormatArg::from(count * 12)]))
            }
        }
    }

    pub fn savings_percent(&self, product: &Product) -> Option<i64> {
        let months = Self::months(product).filter(|months| *months > 1)?;
        let monthly = self.products.iter().find(|candidate| Self::months(candidate) == Some(1))?;
        let monthly_price = monthly.price.filter(|price| *price > Decimal::ZERO)?;
        let price = product.price?;
        let reference = monthly_price * Decimal::from(months);
        if reference <= price {
            return None;
        }
        let saving = (reference - price) / reference * Decimal::from(100);
        let percent = saving.trunc().to_i64()?;
        (percent > 0).then_some(percent)
    }

    pub fn is_recommended(&self, product: &Product) -> bool {
        Self::is_yearly(product)
    }

    fn months(product: &Product) -> Option<i64> {
        let period = product.subscription_period?;
        match period.unit {
            ProductSubscriptionPeriodUnit::Month => Some(period.value),
            ProductSubscriptionPeriodUnit::Year => Some(period.value * 12),
            ProductSubscriptionPeriodUnit::Week | ProductSubscriptionPeriodUnit::Day => None,
        }
    }

    fn is_yearly(product: &Product) -> bool {
        let Some(period) = product.subscription_period else {
            return false;
        };
        (period.unit == ProductSubscriptionPeriodUnit::Year && period.value == 1)
            || (period.unit == ProductSubscriptionPeriodUnit::Month && period.value == 12)
    }

    pub fn subscription_cancel_optimistic(user_id: &str) -> SaiWsOptimisticUpdate {
        SaiWsOptimisticUpdate {
            enabled: true,
            target_cache_key: Some(SaiCache::new(["subscriptionStatus", user_id]).key),
            patch_root_in_place: true,
            patch_paths: vec![key::STATUS.to_owned(), key::camel::IS_ACTIVE.to_owned()],
            create_patch: Some(Arc::new(|_: &Value, _current: &Value, _: &mut App| -> anyhow::Result<Map<String, Value>> {
                let mut patch = Map::new();
                patch.insert(key::STATUS.to_owned(), Value::from(SubscriptionStatus::Cancelled.raw_value()));
                patch.insert(key::camel::IS_ACTIVE.to_owned(), Value::Bool(false));
                Ok(patch)
            })),
            create_rollback_snapshot: Some(Arc::new(|_: &Value, current: &Value, _: &mut App| -> anyhow::Result<Map<String, Value>> {
                let Some(d) = AnyDictionaryBridge::dictionary(Some(current)) else {
                    return Ok(Map::new());
                };
                let mut snapshot = Map::new();
                snapshot.insert(
                    key::STATUS.to_owned(),
                    d.get(key::STATUS).cloned().unwrap_or_else(|| Value::from(SubscriptionStatus::Active.raw_value())),
                );
                snapshot.insert(
                    key::camel::IS_ACTIVE.to_owned(),
                    d.get(key::camel::IS_ACTIVE).cloned().unwrap_or(Value::Bool(true)),
                );
                Ok(snapshot)
            })),
            ..SaiWsOptimisticUpdate::default()
        }
    }

    pub fn handle_verify_purchase_success(&mut self, cx: &mut Context<Self>) {
        let data = SubscriptionActionsStore::shared(cx).read(cx).verify_purchase_sai.data();
        if data.as_ref().and_then(|data| data.active_flag()) != Some(true) {
            return;
        }

        let should_celebrate = self.celebrate_on_next_verify;
        self.celebrate_on_next_verify = false;

        let subscription_id = data.as_ref().and_then(|data| data.subscription_id_text());
        let expires_at = data.as_ref().and_then(|data| data.expires_at_value());
        let is_active = data.as_ref().and_then(|data| data.active_flag()).unwrap_or(true);
        self.apply_subscription_premium_patch(subscription_id.clone(), expires_at, cx);
        let product_id = self.pending_product_id.clone();
        if !self.apply_subscription_status_patch(subscription_id, expires_at, is_active, product_id, cx) {
            SubscriptionActionsStore::shared(cx)
                .update(cx, |actions, cx| actions.get_subscription_status_action(true, cx));
        }
        self.clear_last_error(cx);

        if should_celebrate {
            SubscriptionSuccessStore::shared(cx).update(cx, |success, cx| success.show(cx));
        }

        self.finish_pending_transaction_if_needed(cx);
    }

    /// manage subscription, hands off to the phone app since the desktop has no store account
    pub fn open_manage_subscriptions(&mut self, cx: &mut Context<Self>) {
        self.last_error = None;
        self.continue_on_phone = Some(SubscriptionPhoneHandoff::Manage);
        cx.notify();
    }

    pub fn open_app_store_subscriptions(&mut self, cx: &mut Context<Self>) {
        self.open_manage_subscriptions(cx);
    }

    pub fn open_terms_of_use(&mut self, cx: &mut Context<Self>) {
        self.open_public_document("terms-of-service", cx);
    }

    pub fn open_privacy_policy(&mut self, cx: &mut Context<Self>) {
        self.open_public_document("privacy-policy", cx);
    }

    fn open_public_document(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(public_site_url) = SubscriptionServicesHooks::public_site_url() else {
            return;
        };
        let Some(mut components) = UrlComponents::from_url(&public_site_url, false) else {
            return;
        };
        components.set_path(&format!("/{path}"));
        components.set_query_items(Some(vec![UrlQueryItem::new(
            "source",
            Some(DeviceIdentity::platform(HostOs::current()).to_owned()),
        )]));
        let Some(url) = components.url() else {
            return;
        };

        open_url(&url, cx);
    }

    pub fn handle_verify_purchase_error(
        &mut self,
        error: &anyhow::Error,
        origin: PlanVerifyOrigin,
        cx: &mut Context<Self>,
    ) {
        self.celebrate_on_next_verify = false;
        self.clear_pending_transaction(cx);
        let message = format!("verify_purchase error: {error}");
        Sai::log("Subscription", move || message, SaiLogKind::Network, cx);
        if Self::is_held_by_another_account(error) {
            if origin == PlanVerifyOrigin::StoreUpdate {
                return;
            }
            self.clear_last_error(cx);
            PremiumGiftStore::shared(cx).update(cx, |gift, cx| gift.present_for_self(cx));
            return;
        }
        self.set_last_error(Some(error.to_string()), cx);
    }

    pub fn handle_restore_purchases_success(&mut self, cx: &mut Context<Self>) {
        let data = SubscriptionActionsStore::shared(cx).read(cx).restore_purchases_sai.data();
        let Some(active) =
            data.and_then(|data| data.restored_purchases.into_iter().find(|purchase| purchase.is_active))
        else {
            self.set_last_error(Some(tr("subscription_restore_empty")), cx);
            return;
        };

        self.apply_subscription_premium_patch(active.subscription_id.clone(), active.expires_at, cx);
        if !self.apply_subscription_status_patch(
            active.subscription_id.clone(),
            active.expires_at,
            active.is_active,
            Some(active.product_id.clone()),
            cx,
        ) {
            SubscriptionActionsStore::shared(cx)
                .update(cx, |actions, cx| actions.get_subscription_status_action(true, cx));
        }
        self.clear_last_error(cx);
        Notifier::send(
            tr("subscription_restored_success"),
            NotifierSendOptions {
                r#type: NotifierType::Success,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn handle_restore_purchases_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let message = format!("restore_purchases error: {error}");
        Sai::log("Subscription", move || message, SaiLogKind::Network, cx);
        if Self::is_held_by_another_account(error) {
            self.clear_last_error(cx);
            PremiumGiftStore::shared(cx).update(cx, |gift, cx| gift.present_for_self(cx));
            return;
        }
        self.set_last_error(Some(error.to_string()), cx);
    }

    /// is_premium is a token claim and uploads size their allowance from it,
    /// so the token is refreshed now not at expiry
    pub fn subscription_changed_event_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::SubscriptionChanged(event)) = RiseEvent::decode_payload(name::SUBSCRIPTION_CHANGED, any)
        else {
            return;
        };
        let auth = AuthServicesStore::shared(cx);
        let Some(mut profile) = auth.read(cx).profile().cloned() else {
            return;
        };
        if profile.id != event.user_id {
            return;
        }
        let my_id = profile.id.clone();
        profile.more.is_premium = event.is_premium;
        if let Some(parsed) = SubscriptionStatus::from_raw(&event.status) {
            profile.subscription_status = parsed;
        }
        auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
        SaiWsResponseCache::remove(
            &SaiCache::new(["getMyProfile", my_id.as_str()]).key,
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        SaiWsResponseCache::remove(
            &UserServicesStore::profile_by_id_cache_key(&my_id, &my_id),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        let tag = auth.read(cx).profile().map(|profile| profile.tag.clone()).filter(|tag| !tag.is_empty());
        if let Some(tag) = tag {
            SaiWsResponseCache::remove(
                &UserServicesStore::profile_by_tag_cache_key(&tag),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_my_profile(cx));
        SubscriptionActionsStore::shared(cx).update(cx, |actions, cx| actions.get_subscription_status_action(true, cx));
        AuthTokenRefresh::refresh_access_token(None, cx).detach();
        if event.is_premium {
            PremiumGiftCelebrationStore::shared(cx).update(cx, |celebration, cx| celebration.schedule_check(cx));
        }
    }

    pub fn apply_subscription_premium_patch(
        &mut self,
        subscription_id: Option<String>,
        expires_at_seconds: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let auth = AuthServicesStore::shared(cx);
        let Some(mut profile) = auth.read(cx).profile().cloned() else {
            return;
        };

        profile.more.is_premium = true;
        profile.subscription_status = SubscriptionStatus::Active;
        auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
        let _ = subscription_id;
        let _ = expires_at_seconds;
    }

    fn apply_subscription_status_patch(
        &mut self,
        subscription_id: Option<String>,
        expires_at: Option<i64>,
        is_active: bool,
        product_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(user_id) = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|user_id| !user_id.is_empty())
        else {
            return false;
        };
        let cache_key = SaiCache::new(["subscriptionStatus", user_id.as_str()]).key;
        let existing = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );

        let Some(mut root) = AnyDictionaryBridge::dictionary(existing.as_ref()) else {
            return false;
        };

        let status = if is_active { SubscriptionStatus::Active } else { SubscriptionStatus::Inactive };
        root.insert(key::SUCCESS.to_owned(), Value::Bool(true));
        root.insert(key::STATUS.to_owned(), Value::from(status.raw_value()));
        root.insert(key::camel::IS_ACTIVE.to_owned(), Value::Bool(is_active));
        root.insert(key::camel::IS_PREMIUM.to_owned(), Value::Bool(is_active));
        root.remove(key::IS_ACTIVE);
        root.remove(key::IS_PREMIUM);
        if let Some(subscription_id) = subscription_id {
            root.insert(key::camel::SUBSCRIPTION_ID.to_owned(), Value::from(subscription_id));
            root.remove(key::SUBSCRIPTION_ID);
        }
        if let Some(expires_at) = expires_at {
            root.insert(key::camel::EXPIRES_AT.to_owned(), Value::from(expires_at));
            root.remove(key::EXPIRES_AT);
        }
        if let Some(product_id) = product_id {
            root.insert(key::camel::PRODUCT_ID.to_owned(), Value::from(product_id));
            root.remove(key::PRODUCT_ID);
        }
        let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
        true
    }
}

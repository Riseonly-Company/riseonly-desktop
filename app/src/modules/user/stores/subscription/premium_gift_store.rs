use gpui::{App, Context, Entity};
use rise_canon::rise_event_payload::SubscriptionChangedData;
use rise_canon::rise_ledger::{self, PremiumProductKind};
use rise_i18n::{FormatArg, tr, tr_args};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::store_price::StorePriceBook;
use crate::core::widgets::notifier::notifier_store::{
    NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType,
};
use crate::core::widgets::overlay::subscription_success_store::SubscriptionSuccessStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_wire_types::VerifyPurchaseGatewayScalars;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::{
    Product, SubscriptionServicesStore,
};
use crate::modules::user::stores::user::user_actions::user_types::SubscriptionStatus;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PremiumGiftStoreRecipient {
    pub id: String,
    pub tag: String,
    pub name: String,
    pub logo: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PremiumGiftStoreMode {
    Gift,
    ForSelf,
}

pub struct PremiumGiftStore {
    recipient: Option<PremiumGiftStoreRecipient>,
    mode: PremiumGiftStoreMode,
    gift_products: Vec<Product>,
    pass_products: Vec<Product>,
    is_loading_products: bool,
    load_error: Option<String>,
    selected_product_id: Option<String>,
    is_purchasing: bool,
    last_error: Option<String>,
    is_presented: bool,
    delivered_count: i64,
    continue_on_phone: Option<String>,
    celebrates_on_dismiss: bool,
}

impl PremiumGiftStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let book = StorePriceBook::shared(cx);
        cx.observe(&book, |store, book, cx| {
            let book = book.read(cx);
            for product in store.gift_products.iter_mut().chain(store.pass_products.iter_mut()) {
                product.adopt_store_price(book);
            }
            cx.notify();
        })
        .detach();
        Self {
            recipient: None,
            mode: PremiumGiftStoreMode::Gift,
            gift_products: Vec::new(),
            pass_products: Vec::new(),
            is_loading_products: false,
            load_error: None,
            selected_product_id: None,
            is_purchasing: false,
            last_error: None,
            is_presented: false,
            delivered_count: 0,
            continue_on_phone: None,
            celebrates_on_dismiss: false,
        }
    }

    pub fn product_ids() -> Vec<String> {
        Self::for_environment(PremiumProductKind::Gift)
    }

    pub fn pass_product_ids() -> Vec<String> {
        Self::for_environment(PremiumProductKind::Pass)
    }

    pub fn base_product_ids() -> Vec<&'static str> {
        PremiumProductKind::Gift.products().map(|product| product.product).collect()
    }

    pub fn base_pass_product_ids() -> Vec<&'static str> {
        PremiumProductKind::Pass.products().map(|product| product.product).collect()
    }

    fn for_environment(kind: PremiumProductKind) -> Vec<String> {
        kind.product_ids(SubscriptionServicesStore::store_environment())
    }

    pub fn is_gift_product(product_id: &str) -> bool {
        rise_ledger::is_gift_product(product_id)
    }

    pub fn is_pass_product(product_id: &str) -> bool {
        rise_ledger::is_pass_product(product_id)
    }

    pub fn recipient(&self) -> Option<&PremiumGiftStoreRecipient> {
        self.recipient.as_ref()
    }

    pub fn mode(&self) -> PremiumGiftStoreMode {
        self.mode
    }

    pub fn gift_products(&self) -> &[Product] {
        &self.gift_products
    }

    pub fn pass_products(&self) -> &[Product] {
        &self.pass_products
    }

    pub fn is_loading_products(&self) -> bool {
        self.is_loading_products
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn selected_product_id(&self) -> Option<&str> {
        self.selected_product_id.as_deref()
    }

    pub fn is_purchasing(&self) -> bool {
        self.is_purchasing
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn delivered_count(&self) -> i64 {
        self.delivered_count
    }

    pub fn continue_on_phone(&self) -> Option<&str> {
        self.continue_on_phone.as_deref()
    }

    pub fn phone_link(&self) -> Option<RiseonlyPublicLink> {
        let period = rise_ledger::premium_period_token(self.continue_on_phone()?)?;
        match self.mode {
            PremiumGiftStoreMode::ForSelf => Some(RiseonlyPublicLink::PurchasePremium { period }),
            PremiumGiftStoreMode::Gift => {
                Some(RiseonlyPublicLink::PurchaseGift { user_id: self.recipient.as_ref()?.id.clone(), period })
            }
        }
    }

    pub fn dismiss_continue_on_phone(&mut self, cx: &mut Context<Self>) {
        self.continue_on_phone = None;
        cx.notify();
    }

    /// catalogue of the current mode, gifts or passes for this account
    pub fn products(&self) -> &[Product] {
        if self.mode == PremiumGiftStoreMode::ForSelf { &self.pass_products } else { &self.gift_products }
    }

    pub fn present(&mut self, recipient: PremiumGiftStoreRecipient, cx: &mut Context<Self>) {
        self.switch_mode(PremiumGiftStoreMode::Gift);
        self.recipient = Some(recipient);
        self.last_error = None;
        self.is_presented = true;
        cx.notify();
        self.load_products_if_needed(cx);
    }

    /// opens the sheet for this account itself after the server said the store plan belongs to
    /// another account
    pub fn present_for_self(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = AuthServicesStore::shared(cx).read(cx).profile().cloned() else {
            return;
        };
        if profile.id.is_empty() {
            return;
        }
        self.switch_mode(PremiumGiftStoreMode::ForSelf);
        self.recipient = Some(PremiumGiftStoreRecipient {
            id: profile.id,
            tag: profile.tag,
            name: profile.name,
            logo: profile.avatar_url.unwrap_or(profile.more.logo),
        });
        self.last_error = None;
        self.is_presented = true;
        cx.notify();
        self.load_products_if_needed(cx);
    }

    pub fn prepare_inline(&mut self, recipient: PremiumGiftStoreRecipient, cx: &mut Context<Self>) {
        if self.recipient.as_ref() != Some(&recipient) || self.mode != PremiumGiftStoreMode::Gift {
            self.last_error = None;
        }
        self.switch_mode(PremiumGiftStoreMode::Gift);
        self.recipient = Some(recipient);
        cx.notify();
        self.load_products_if_needed(cx);
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.is_presented = false;
        cx.notify();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.celebrates_on_dismiss {
            return;
        }
        self.celebrates_on_dismiss = false;
        SubscriptionSuccessStore::shared(cx).update(cx, |success, cx| success.show(cx));
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.celebrates_on_dismiss = false;
        self.is_presented = false;
        self.mode = PremiumGiftStoreMode::Gift;
        self.recipient = None;
        self.gift_products = Vec::new();
        self.pass_products = Vec::new();
        self.selected_product_id = None;
        self.last_error = None;
        self.continue_on_phone = None;
        cx.notify();
    }

    fn switch_mode(&mut self, next: PremiumGiftStoreMode) {
        self.mode = next;
        self.adopt_selection();
    }

    fn adopt_selection(&mut self) {
        let keeps_selection = self
            .selected_product_id
            .as_ref()
            .is_some_and(|selected| self.products().iter().any(|product| &product.id == selected));
        if !keeps_selection {
            self.selected_product_id = self.products().first().map(|product| product.id.clone());
        }
    }

    pub fn load_products_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.products().is_empty() || self.is_loading_products {
            return;
        }
        self.load_products(cx);
    }

    /// catalogue is the canon period list of the mode, the price comes from the phone store
    pub fn load_products(&mut self, cx: &mut Context<Self>) {
        let for_self = self.mode == PremiumGiftStoreMode::ForSelf;
        self.is_loading_products = true;
        self.load_error = None;
        let ids = if for_self { Self::pass_product_ids() } else { Self::product_ids() };
        let mut sorted: Vec<Product> = ids.into_iter().map(Product::from_catalogue_id).collect();
        sorted.sort_by_key(|product| Self::months_of(&product.id));
        let book = StorePriceBook::shared(cx);
        for product in &mut sorted {
            product.adopt_store_price(book.read(cx));
        }
        StorePriceBook::refresh(cx);
        let is_empty = sorted.is_empty();
        if for_self {
            self.pass_products = sorted;
        } else {
            self.gift_products = sorted;
        }
        self.adopt_selection();
        if is_empty {
            self.load_error = Some(tr(if for_self {
                "subscription_products_unavailable"
            } else {
                "premium_gift_products_unavailable"
            }));
        }
        self.is_loading_products = false;
        cx.notify();
    }

    pub fn select_product(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_product_id = Some(id);
        cx.notify();
    }

    /// the three longest periods,
    /// shortest first,
    /// the row the gift shop shows
    pub fn shelf_products(&self) -> Vec<Product> {
        let products = self.products();
        products[products.len().saturating_sub(3)..].to_vec()
    }

    pub fn months_for(&self, product: &Product) -> i64 {
        Self::months_of(&product.id)
    }

    pub fn savings_percent(&self, product: &Product) -> Option<i64> {
        let months = Self::months_of(&product.id);
        let base = self
            .products()
            .iter()
            .filter(|candidate| {
                Self::months_of(&candidate.id) > 0 && candidate.price.is_some_and(|price| price > Decimal::ZERO)
            })
            .min_by_key(|candidate| Self::months_of(&candidate.id))?;
        let base_months = Self::months_of(&base.id);
        if months <= base_months {
            return None;
        }
        let base_price = base.price?;
        let price = product.price?;
        let reference = base_price / Decimal::from(base_months) * Decimal::from(months);
        if reference <= price {
            return None;
        }
        let saving = (reference - price) / reference * Decimal::from(100);
        let percent = saving.trunc().to_i64()?;
        (percent > 0).then_some(percent)
    }

    pub fn period_title(&self, product: &Product) -> String {
        let key = Self::period_title_key_for_token(&Self::period_token(&product.id)).unwrap_or("premium_gift_period_1m");
        tr(key)
    }

    fn period_title_key_for_token(token: &str) -> Option<&'static str> {
        match token {
            "1m" => Some("premium_gift_period_1m"),
            "3m" => Some("premium_gift_period_3m"),
            "6m" => Some("premium_gift_period_6m"),
            "12m" => Some("premium_gift_period_12m"),
            _ => None,
        }
    }

    fn period_token(product_id: &str) -> String {
        rise_ledger::premium_period_token(product_id).unwrap_or_default()
    }

    fn months_of(product_id: &str) -> i64 {
        rise_ledger::premium_period_for_product_id(product_id).map_or(0, |period| i64::from(period.months))
    }

    pub fn purchase_selected(&mut self, cx: &mut Context<Self>) {
        if self.recipient.is_none() {
            return;
        }
        let Some(product_id) = self
            .selected_product_id
            .as_ref()
            .filter(|id| self.products().iter().any(|product| &product.id == *id))
            .cloned()
        else {
            return;
        };
        self.last_error = None;
        self.continue_on_phone = Some(product_id);
        cx.notify();
    }

    pub fn handle_verify_success(&mut self, cx: &mut Context<Self>) {
        let granted = SubscriptionActionsStore::shared(cx)
            .read(cx)
            .verify_gift_purchase_sai
            .data()
            .and_then(|data| data.active_flag())
            == Some(true);
        if !granted {
            self.last_error = Some(tr("subscription_purchase_error"));
            cx.notify();
            Sai::log("PremiumGift", || "gift verified without a grant", SaiLogKind::Network, cx);
            return;
        }
        self.last_error = None;
        let sheet_was_open = self.is_presented;
        self.is_presented = false;
        cx.notify();
        if self.mode == PremiumGiftStoreMode::ForSelf {
            Self::own_premium_moved(cx);
            if sheet_was_open {
                self.celebrates_on_dismiss = true;
            } else {
                SubscriptionSuccessStore::shared(cx).update(cx, |success, cx| success.show(cx));
            }
            return;
        }
        let name = self.recipient.as_ref().map(|recipient| recipient.name.clone()).unwrap_or_default();
        self.delivered_count += 1;
        NotifierStore::shared(cx).update(cx, |notifier, cx| {
            notifier.send(
                tr_args("premium_gift_success", &[FormatArg::from(name)]),
                NotifierSendOptions {
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        if let Some(recipient) = self.recipient.clone() {
            UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.refresh_foreign_profile_user_id(recipient.id, cx);
                interactions.refresh_foreign_profile_tag(recipient.tag, cx);
            });
        }
    }

    pub fn handle_verify_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.last_error = Some(error.to_string());
        cx.notify();
        let message = format!("gift verify failed: {error}");
        Sai::log("PremiumGift", move || message, SaiLogKind::Network, cx);
    }

    pub fn settle_unfinished_passes(&mut self, _cx: &mut Context<Self>) {}

    fn own_premium_moved(cx: &mut App) {
        let Some(my_id) = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()) else {
            return;
        };
        let event = SubscriptionChangedData {
            user_id: my_id,
            is_premium: true,
            status: SubscriptionStatus::Active.raw_value().to_owned(),
            ..SubscriptionChangedData::default()
        };
        let Ok(payload) = serde_json::to_value(&event) else {
            return;
        };
        SubscriptionServicesStore::shared(cx)
            .update(cx, |services, cx| services.subscription_changed_event_handler(Some(&payload), cx));
    }
}

use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_proto::subscription::StorePriceSnapshot;
use rise_canon::{rise_request, rise_response, rise_wire_methods};
use rise_i18n::language_store::LanguageStore;
use rise_i18n::number_format::decimal_format_style_currency::{
    CurrencyFormatStyleConfigurationPresentation, DecimalFormatStyleCurrency,
};
use rust_decimal::Decimal;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsRequestOptions, SaiWsRequestOptionsInit};
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

pub enum StorePrice {}

impl StorePrice {
    pub fn text(price: &Decimal, currency_code: &str) -> String {
        DecimalFormatStyleCurrency::currency(currency_code)
            .presentation(CurrencyFormatStyleConfigurationPresentation::Narrow)
            .locale(LanguageStore::current_locale().identifier())
            .format(price)
    }
}

/// one product with the price a phone store gave for it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneStorePrice {
    pub amount: Decimal,
    pub currency_code: String,
}

/// prices the phone store showed for this account
/// no store on desktop so a phone reports prices and the server keeps the last per platform
/// none for a product no phone priced
pub struct StorePriceBook {
    prices: HashMap<String, PhoneStorePrice>,
    account_id: Option<String>,
    is_loading: bool,
    slice: Rc<SaiTypedSlice<rise_response::subscription::GetStorePrices>>,
}

impl StorePriceBook {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self { prices: HashMap::new(), account_id: None, is_loading: false, slice: SaiTypedSlice::new(cx) }
    }

    pub fn price(&self, product_id: &str) -> Option<&PhoneStorePrice> {
        self.prices.get(product_id)
    }

    /// price text as purchase screens show it, empty until a phone reports it
    pub fn text(&self, product_id: &str) -> String {
        self.price(product_id)
            .map(|price| StorePrice::text(&price.amount, &price.currency_code))
            .unwrap_or_default()
    }

    pub fn prices_of(snapshots: &[StorePriceSnapshot]) -> HashMap<String, PhoneStorePrice> {
        let mut prices = HashMap::new();
        for snapshot in snapshots {
            if snapshot.currency_code.is_empty() {
                continue;
            }
            for price in &snapshot.prices {
                prices.entry(price.product_id.clone()).or_insert_with(|| PhoneStorePrice {
                    amount: Decimal::new(price.amount_micros, 6).normalize(),
                    currency_code: snapshot.currency_code.clone(),
                });
            }
        }
        prices
    }

    /// asks the server again after the current update ends
    /// so a catalogue loaded inside another store update never reads the leased account store
    pub fn refresh(cx: &mut App) {
        cx.defer(|cx| Self::shared(cx).update(cx, |book, cx| book.load(cx)));
    }

    /// asks the server again, stores showing a catalogue pick up the prices when the answer lands
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let account_id = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|user_id| !user_id.is_empty());
        if account_id != self.account_id {
            self.account_id = account_id.clone();
            self.prices.clear();
            cx.notify();
        }
        if account_id.is_none() || self.is_loading {
            return;
        }
        self.is_loading = true;

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::GET_STORE_PRICES.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });
        let book = cx.entity();
        let slice = Rc::clone(&self.slice);
        let loaded = book.clone();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let snapshots = slice.data().map(|reply| reply.snapshots).unwrap_or_default();
            loaded.update(cx, |book, cx| {
                book.is_loading = false;
                if book.account_id != account_id {
                    return;
                }
                let prices = Self::prices_of(&snapshots);
                if prices != book.prices {
                    book.prices = prices;
                    cx.notify();
                }
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            book.update(cx, |book, _| book.is_loading = false);
            let message = format!("get_store_prices error: {error}");
            Sai::log("Subscription", move || message, SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(
            &self.slice,
            rise_request::subscription::GetStorePrices {},
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }
}

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_request::subscription::restore_purchases::ReceiptsElement;
use rise_canon::{rise_request, rise_response, rise_wire_methods};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::stores::subscription::premium_gift_store::PremiumGiftStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_types::PlanVerifyOrigin;
use crate::modules::user::stores::subscription::subscription_actions::subscription_wire_types::{
    IAPPurchaseReceiptBody, PremiumGiftNoticeDTO, PremiumPromoLinkDTO, PremiumPromoRedeemDTO,
    RestorePurchasesGatewayDTO, SubscriptionStatusGatewayDTO, VerifyPurchaseGatewayDTO,
};
use crate::modules::user::stores::subscription::subscription_services::subscription_services::SubscriptionServicesStore;

pub struct SubscriptionActionsStore {
    pub subscription_status_sai: Rc<SaiTypedSlice<SubscriptionStatusGatewayDTO>>,
    pub verify_purchase_sai: Rc<SaiTypedSlice<VerifyPurchaseGatewayDTO>>,
    pub verify_gift_purchase_sai: Rc<SaiTypedSlice<VerifyPurchaseGatewayDTO>>,
    pub settle_pass_sai: Rc<SaiTypedSlice<VerifyPurchaseGatewayDTO>>,
    pub gift_notice_sai: Rc<SaiTypedSlice<PremiumGiftNoticeDTO>>,
    pub ack_gift_notices_sai: Rc<SaiTypedSlice<rise_response::subscription::AckGiftNotices>>,
    pub restore_purchases_sai: Rc<SaiTypedSlice<RestorePurchasesGatewayDTO>>,
    pub cancel_subscription_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_promo_link_sai: Rc<SaiTypedSlice<PremiumPromoLinkDTO>>,
    pub redeem_promo_link_sai: Rc<SaiTypedSlice<PremiumPromoRedeemDTO>>,
}

impl SubscriptionActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            subscription_status_sai: SaiTypedSlice::new(cx),
            verify_purchase_sai: SaiTypedSlice::new(cx),
            verify_gift_purchase_sai: SaiTypedSlice::new(cx),
            settle_pass_sai: SaiTypedSlice::new(cx),
            gift_notice_sai: SaiTypedSlice::new(cx),
            ack_gift_notices_sai: SaiTypedSlice::new(cx),
            restore_purchases_sai: SaiTypedSlice::new(cx),
            cancel_subscription_sai: SaiTypedSlice::new(cx),
            get_promo_link_sai: SaiTypedSlice::new(cx),
            redeem_promo_link_sai: SaiTypedSlice::new(cx),
        }
    }

    fn signed_in_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|user_id| !user_id.is_empty())
    }

    fn log_only_on_error(label: &'static str) -> SaiWsOnError {
        Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let message = format!("{label} error: {error}");
            Sai::log("Subscription", move || message, SaiLogKind::Network, cx);
        })
    }

    pub fn get_subscription_status_action(&mut self, force_network: bool, cx: &mut Context<Self>) {
        let Some(user_id) = Self::signed_in_user_id(cx) else {
            return;
        };

        let body = rise_request::subscription::GetStatus {};
        let cache_key = SaiCache::new(["subscriptionStatus", user_id.as_str()]).key;
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::GET_STATUS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: force_network,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: self.subscription_status_sai.data().is_none(),
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: force_network,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            screen_refresh: if force_network {
                None
            } else {
                Some(SaiWsScreenRefresh::new(true, vec!["SubscriptionSettings".to_owned()], None))
            },
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(
            &self.subscription_status_sai,
            body,
            options,
            None,
            Some(Self::log_only_on_error("subscription.get_status")),
            cx,
        );
    }

    pub fn verify_purchase_action(
        &mut self,
        receipt: IAPPurchaseReceiptBody,
        origin: PlanVerifyOrigin,
        cx: &mut Context<Self>,
    ) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let body = receipt.verify_request();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::VERIFY_PURCHASE.to_owned(),
            reflect_pending_in_ui: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, cx: &mut App| {
            SubscriptionServicesStore::shared(cx).update(cx, |services, cx| services.handle_verify_purchase_success(cx));
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            SubscriptionServicesStore::shared(cx)
                .update(cx, |services, cx| services.handle_verify_purchase_error(error, origin, cx));
        });
        SaiWs::request_slice(&self.verify_purchase_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn verify_gift_purchase_action(&mut self, receipt: IAPPurchaseReceiptBody, cx: &mut Context<Self>) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let body = receipt.verify_request();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::VERIFY_PURCHASE.to_owned(),
            reflect_pending_in_ui: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, cx: &mut App| {
            PremiumGiftStore::shared(cx).update(cx, |gift, cx| gift.handle_verify_success(cx));
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            PremiumGiftStore::shared(cx).update(cx, |gift, cx| gift.handle_verify_error(error, cx));
        });
        SaiWs::request_slice(&self.verify_gift_purchase_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn settle_pass_action(
        &mut self,
        receipt: IAPPurchaseReceiptBody,
        on_settled: impl Fn(&mut App) + 'static,
        on_failed: impl Fn(&anyhow::Error, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let body = receipt.verify_request();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::VERIFY_PURCHASE.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| on_settled(cx));
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| on_failed(error, cx));
        SaiWs::request_slice(&self.settle_pass_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_gift_notice_action(
        &mut self,
        on_loaded: impl FnOnce(Option<PremiumGiftNoticeDTO>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::GET_GIFT_NOTICE.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.gift_notice_sai);
        let on_loaded = RefCell::new(Some(on_loaded));
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(on_loaded) = on_loaded.borrow_mut().take() else {
                return;
            };
            on_loaded(slice.data(), cx);
        });
        SaiWs::request_slice(
            &self.gift_notice_sai,
            rise_request::subscription::GetGiftNotice {},
            options,
            Some(success),
            Some(Self::log_only_on_error("get_gift_notice")),
            cx,
        );
    }

    pub fn ack_gift_notices_action(&mut self, cx: &mut Context<Self>) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::ACK_GIFT_NOTICES.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(
            &self.ack_gift_notices_sai,
            rise_request::subscription::AckGiftNotices {},
            options,
            None,
            Some(Self::log_only_on_error("ack_gift_notices")),
            cx,
        );
    }

    pub fn restore_purchases_action(&mut self, receipts: Vec<IAPPurchaseReceiptBody>, cx: &mut Context<Self>) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let body = rise_request::subscription::RestorePurchases {
            receipts: Some(
                receipts
                    .into_iter()
                    .map(|receipt| ReceiptsElement {
                        platform: Some(receipt.platform),
                        product_id: Some(receipt.product_id),
                        receipt: Some(receipt.receipt),
                        transaction_id: Some(receipt.transaction_id),
                        ..ReceiptsElement::default()
                    })
                    .collect(),
            ),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::RESTORE_PURCHASES.to_owned(),
            reflect_pending_in_ui: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, cx: &mut App| {
            SubscriptionServicesStore::shared(cx)
                .update(cx, |services, cx| services.handle_restore_purchases_success(cx));
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            SubscriptionServicesStore::shared(cx)
                .update(cx, |services, cx| services.handle_restore_purchases_error(error, cx));
        });
        SaiWs::request_slice(&self.restore_purchases_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn cancel_subscription_action(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::signed_in_user_id(cx) else {
            return;
        };

        let cache_key = SaiCache::new(["subscriptionStatus", user_id.as_str()]).key;
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::CANCEL.to_owned(),
            cache_key: Some(cache_key),
            reflect_pending_in_ui: false,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            optimistic_update: Some(SubscriptionServicesStore::subscription_cancel_optimistic(&user_id)),
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(
            &self.cancel_subscription_sai,
            rise_request::subscription::Cancel {},
            options,
            None,
            Some(Self::log_only_on_error("subscription.cancel")),
            cx,
        );
    }

    pub fn get_promo_link_action(&mut self, code: String, cx: &mut Context<Self>) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let body = rise_request::subscription::GetPromoLink { code };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::GET_PROMO_LINK.to_owned(),
            reflect_pending_in_ui: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, cx: &mut App| {
            PremiumPromoStore::shared(cx).update(cx, |promo, cx| promo.handle_preview_success(cx));
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            PremiumPromoStore::shared(cx).update(cx, |promo, cx| promo.handle_preview_error(error, cx));
        });
        SaiWs::request_slice(&self.get_promo_link_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn redeem_promo_link_action(&mut self, code: String, cx: &mut Context<Self>) {
        if Self::signed_in_user_id(cx).is_none() {
            return;
        }

        let body = rise_request::subscription::RedeemPromoLink { code };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::subscription::SERVICE.to_owned(),
            method: rise_wire_methods::subscription::REDEEM_PROMO_LINK.to_owned(),
            reflect_pending_in_ui: true,
            allows_deferred_send: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, cx: &mut App| {
            PremiumPromoStore::shared(cx).update(cx, |promo, cx| promo.handle_redeem_success(cx));
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            PremiumPromoStore::shared(cx).update(cx, |promo, cx| promo.handle_redeem_error(error, cx));
        });
        SaiWs::request_slice(&self.redeem_promo_link_sai, body, options, Some(success), Some(failure), cx);
    }
}

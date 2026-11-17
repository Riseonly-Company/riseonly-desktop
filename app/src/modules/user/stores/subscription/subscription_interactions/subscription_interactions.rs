use gpui::{App, Context, Entity};

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::modules::user::stores::subscription::premium_gift_store::PremiumGiftStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::SubscriptionServicesStore;

pub struct SubscriptionInteractionsStore {}

impl SubscriptionInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn actions(cx: &mut App) -> Entity<SubscriptionActionsStore> {
        SubscriptionActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<SubscriptionServicesStore> {
        SubscriptionServicesStore::shared(cx)
    }

    pub fn ensure_subscription_status_loaded(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).subscription_status_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_subscription_status_action(false, cx));
    }

    pub fn refresh_subscription_status(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_subscription_status_action(true, cx));
    }

    pub fn cancel_subscription(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.cancel_subscription_action(cx));
    }

    pub fn buy_subscription(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.purchase_selected(cx));
    }

    pub fn restore_purchases(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.restore_purchases(cx));
    }

    pub fn ensure_iap_started(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.start_if_needed(cx));
        PremiumGiftStore::shared(cx).update(cx, |gift, cx| gift.settle_unfinished_passes(cx));
    }

    pub fn open_manage_subscriptions(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.open_manage_subscriptions(cx));
    }

    pub fn open_app_store_subscriptions(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.open_app_store_subscriptions(cx));
    }

    pub fn open_terms_of_use(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.open_terms_of_use(cx));
    }

    pub fn open_privacy_policy(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.open_privacy_policy(cx));
    }
}

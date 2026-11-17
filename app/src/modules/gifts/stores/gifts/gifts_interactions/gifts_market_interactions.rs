use gpui::{App, AsyncApp, Context, WeakEntity};
use rise_i18n::tr;

use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftItemKind,
};
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;

pub type GiftsEquipDone = Box<dyn FnOnce(&mut App) + 'static>;

impl GiftsInteractionsStore {
    pub fn select_shop_kind(&mut self, kind: RiseGiftItemKind, cx: &mut Context<Self>) {
        if self.shop_kind == kind {
            return;
        }
        self.shop_kind = kind;
        cx.notify();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_catalog(kind, false, cx).await;
        })
        .detach();
    }

    pub fn open_shop_for_self(&mut self, kind: RiseGiftItemKind, cx: &mut Context<Self>) {
        let Some(me) = GiftPeople::me(cx) else {
            return;
        };
        self.open_shop(me, kind, cx);
    }

    pub fn toggle_equipped(&mut self, instance: RiseGiftInstanceDTO, cx: &mut Context<Self>) {
        let equipped = !instance.is_equipped();
        self.set_equipped(instance, equipped, None, cx);
    }

    pub fn set_equipped(
        &mut self,
        instance: RiseGiftInstanceDTO,
        equipped: bool,
        on_done: Option<GiftsEquipDone>,
        cx: &mut Context<Self>,
    ) {
        if !instance.is_cosmetic() || self.is_equipping {
            return;
        }
        let is_own = AuthServicesStore::shared(cx).read(cx).is_current_user(&instance.owner_user_id);
        if !is_own {
            return;
        }
        self.is_equipping = true;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            match GiftsActionsStore::set_equipped(&instance, equipped, cx).await {
                Ok(changed) => {
                    let _ = this.update(cx, |store, cx| {
                        if store.opened_instance.as_ref().is_some_and(|opened| opened.id == changed.id) {
                            store.opened_instance = Some(changed.clone());
                        }
                        if store.sent_instance.as_ref().is_some_and(|sent| sent.id == changed.id) {
                            store.sent_instance = Some(changed.clone());
                        }
                        cx.notify();
                    });
                    if let Some(on_done) = on_done {
                        cx.update(on_done);
                    }
                }
                Err(error) => cx.update(|cx| Self::notify_error(&error, cx)),
            }
            let _ = this.update(cx, |store, cx| {
                store.is_equipping = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn offer_to_wear(&mut self, instance: &RiseGiftInstanceDTO, cx: &mut Context<Self>) -> bool {
        if !instance.is_cosmetic() || instance.is_equipped() || !instance.is_gifted_by_other() {
            return false;
        }
        let is_own = AuthServicesStore::shared(cx).read(cx).is_current_user(&instance.owner_user_id);
        if !is_own {
            return false;
        }
        if !self.opened_instance.as_ref().is_some_and(|opened| opened.id == instance.id) {
            self.equip_offer = Some(instance.clone());
            cx.notify();
        }
        true
    }

    pub fn accept_equip_offer(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self.equip_offer.clone() else {
            return;
        };
        let worn = offer.clone();
        let on_done: GiftsEquipDone = Box::new(move |cx: &mut App| {
            GiftsInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.equip_offer = None;
                cx.notify();
            });
            let store = GiftsServicesStore::shared(cx);
            let content = store.read(cx).localized_name(worn.gift_id, cx).unwrap_or_else(|| worn.name.clone());
            Notifier::send(
                tr("gift_equip_worn"),
                NotifierSendOptions {
                    content: Some(content),
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        self.set_equipped(offer, true, Some(on_done), cx);
    }

    pub fn decline_equip_offer(&mut self, cx: &mut Context<Self>) {
        self.equip_offer = None;
        cx.notify();
    }

    pub fn open_wardrobe(&mut self, kind: RiseGiftItemKind, cx: &mut Context<Self>) {
        if !kind.equippable() {
            return;
        }
        self.wardrobe_kind = Some(kind);
        cx.notify();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_wardrobe(kind, true, cx).await;
            GiftsActionsStore::load_own_appearance(false, cx).await;
        })
        .detach();
    }

    pub fn close_wardrobe(&mut self, cx: &mut Context<Self>) {
        self.wardrobe_kind = None;
        cx.notify();
    }

    pub fn load_more_wardrobe(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.wardrobe_kind else {
            return;
        };
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_more_wardrobe(kind, cx).await;
        })
        .detach();
    }

    pub fn buy_from_wardrobe(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.wardrobe_kind else {
            return;
        };
        self.pending_self_shop_kind = Some(kind);
        self.wardrobe_kind = None;
        cx.notify();
    }

    pub fn wardrobe_did_dismiss(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.pending_self_shop_kind.take() else {
            return;
        };
        self.open_shop_for_self(kind, cx);
    }

    pub fn refresh_appearance(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_own_appearance(true, cx).await;
        })
        .detach();
    }

    pub fn drop_account_prompts(&mut self, cx: &mut Context<Self>) {
        self.equip_offer = None;
        self.wardrobe_kind = None;
        self.pending_self_shop_kind = None;
        self.is_sparks_info_presented = false;
        cx.notify();
    }
}

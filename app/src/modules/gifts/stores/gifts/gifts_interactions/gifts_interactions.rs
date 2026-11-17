use std::collections::HashSet;

use gpui::{App, AsyncApp, Context, Entity, WeakEntity};
use rise_canon::RiseEvent;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseJSONValue;
use rise_errors::{ErrorCode, ErrorParam};
use rise_i18n::{FormatArg, format_printf, tr};
use rise_limits::{SERVICE_LIMITS, service_limit};
use rise_theme::theme::theme_store::ThemeStore;

use crate::core::push_notifications::push_notification_manager::PushNotificationManager;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::notifier::notifier_host::EDGE_PADDING as NOTIFIER_EDGE_PADDING;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeKind, RiseGiftCatalogItemDTO, RiseGiftCatalogItemExt, RiseGiftInstanceDTO,
    RiseGiftInstanceExt, RiseGiftItemKind, RiseGiftUserDTO,
};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gift_shop_footer_hooks::gift_shop_footer_window_lift;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;
use crate::modules::sparks::engine::rise_sparks_repository::RiseSparksFailure;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_actions::SparksActionsStore;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;
use crate::modules::user::stores::subscription::premium_gift_store::{
    PremiumGiftStore, PremiumGiftStoreRecipient,
};

pub const GIFT_SHOP_SHEET_FOOTER_VERTICAL_PADDING: f32 = 10.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GiftRecipient {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub avatar: String,
    pub cover: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GiftShopFilter {
    #[default]
    All,
    Collectible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GiftOddsRequest {
    pub gift_id: i64,
    pub kind: RiseGiftAttributeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GiftsInstanceList {
    pub owner_id: String,
    pub kind: Option<RiseGiftItemKind>,
}

pub struct GiftsInteractionsStore {
    pub shop_recipient: Option<GiftRecipient>,
    pub shop_filter: GiftShopFilter,
    pub shop_kind: RiseGiftItemKind,
    pub selected_gift: Option<RiseGiftCatalogItemDTO>,
    pub caption: String,
    pub anonymous: bool,
    pub is_sending: bool,
    pub sent_instance: Option<RiseGiftInstanceDTO>,
    pub opened_instance: Option<RiseGiftInstanceDTO>,
    pub opened_list: Option<GiftsInstanceList>,
    pub is_changing_instance: bool,
    pub odds_request: Option<GiftOddsRequest>,
    pub confirming_hide: bool,
    pub equip_offer: Option<RiseGiftInstanceDTO>,
    pub wardrobe_kind: Option<RiseGiftItemKind>,
    pub is_equipping: bool,
    pub is_sparks_info_presented: bool,
    send_key: String,
    pub pending_self_shop_kind: Option<RiseGiftItemKind>,
    pending_gift_back: Option<GiftRecipient>,
    card_names_loading: HashSet<i64>,
}

impl GiftsInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            shop_recipient: None,
            shop_filter: GiftShopFilter::All,
            shop_kind: RiseGiftItemKind::Item,
            selected_gift: None,
            caption: String::new(),
            anonymous: false,
            is_sending: false,
            sent_instance: None,
            opened_instance: None,
            opened_list: None,
            is_changing_instance: false,
            odds_request: None,
            confirming_hide: false,
            equip_offer: None,
            wardrobe_kind: None,
            is_equipping: false,
            is_sparks_info_presented: false,
            send_key: Self::new_send_key(),
            pending_self_shop_kind: None,
            pending_gift_back: None,
            card_names_loading: HashSet::new(),
        }
    }

    fn new_send_key() -> String {
        Uuid::new_v4().uuid_string().to_lowercase()
    }

    pub fn actions(cx: &mut App) -> Entity<GiftsActionsStore> {
        GiftsActionsStore::shared(cx)
    }

    fn is_opened(&self, id: &str) -> bool {
        self.opened_instance.as_ref().is_some_and(|opened| opened.id == id)
    }

    pub(crate) fn notify_error(error: &anyhow::Error, cx: &mut App) {
        Notifier::send(
            RiseSparksFailure::message(error),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn is_shop_presented(&self) -> bool {
        self.shop_recipient.is_some()
    }

    pub fn is_shop_for_self(&self, cx: &mut App) -> bool {
        let Some(recipient) = &self.shop_recipient else {
            return false;
        };
        AuthServicesStore::shared(cx).read(cx).is_current_user(&recipient.id)
    }

    pub fn open_shop(&mut self, mut recipient: GiftRecipient, kind: RiseGiftItemKind, cx: &mut Context<Self>) {
        if recipient.cover.is_none() {
            recipient.cover = GiftPeople::person(&recipient.id, cx).and_then(|person| person.cover);
        }
        let remembered = recipient.clone();
        GiftsServicesStore::shared(cx).update(cx, |services, cx| services.remember(remembered, cx));
        self.selected_gift = None;
        self.sent_instance = None;
        self.caption = String::new();
        self.anonymous = false;
        self.shop_filter = GiftShopFilter::All;
        self.shop_kind = kind;
        self.shop_recipient = Some(recipient.clone());
        let is_own = AuthServicesStore::shared(cx).read(cx).is_current_user(&recipient.id);
        if !is_own {
            let inline = PremiumGiftStoreRecipient {
                id: recipient.id.clone(),
                tag: recipient.tag.clone(),
                name: recipient.name.clone(),
                logo: recipient.avatar.clone(),
            };
            PremiumGiftStore::shared(cx).update(cx, |premium, cx| premium.prepare_inline(inline, cx));
        }
        cx.notify();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_catalog(kind, false, cx).await;
            cx.update(|cx| {
                SparksActionsStore::shared(cx).update(cx, |sparks, cx| sparks.refresh_balance_action(false, cx));
            });
            for other in RiseGiftItemKind::SHOP_ORDER {
                if other != kind {
                    GiftsActionsStore::load_catalog(other, false, cx).await;
                }
            }
        })
        .detach();
    }

    pub fn close_shop(&mut self, cx: &mut Context<Self>) {
        self.shop_recipient = None;
        self.selected_gift = None;
        self.sent_instance = None;
        self.is_sending = false;
        self.is_sparks_info_presented = false;
        cx.notify();
    }

    pub fn open_sparks_info(&mut self, cx: &mut Context<Self>) {
        self.is_sparks_info_presented = true;
        cx.notify();
    }

    pub fn close_sparks_info(&mut self, cx: &mut Context<Self>) {
        self.is_sparks_info_presented = false;
        cx.notify();
    }

    pub fn select(&mut self, gift: RiseGiftCatalogItemDTO, cx: &mut Context<Self>) {
        if gift.is_sold_out() {
            return;
        }
        let gift_id = gift.id;
        self.selected_gift = Some(gift);
        self.caption = String::new();
        self.anonymous = false;
        self.send_key = Self::new_send_key();
        cx.notify();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_catalog_item(gift_id, false, cx).await;
        })
        .detach();
    }

    pub fn back_to_catalog(&mut self, cx: &mut Context<Self>) {
        self.selected_gift = None;
        self.sent_instance = None;
        cx.notify();
    }

    pub fn shortfall(&self, gift: &RiseGiftCatalogItemDTO, cx: &mut App) -> Option<i64> {
        let store = SparksServicesStore::shared(cx);
        let sparks = store.read(cx);
        if !sparks.has_balance() {
            return None;
        }
        let missing = gift.price_sparks - sparks.spendable_now();
        if missing > 0 { Some(missing) } else { None }
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        let (Some(gift), Some(recipient)) = (self.selected_gift.clone(), self.shop_recipient.clone()) else {
            return;
        };
        if self.is_sending {
            return;
        }
        if let Some(missing) = self.shortfall(&gift, cx) {
            SparksInteractionsStore::shared(cx)
                .update(cx, |sparks, cx| sparks.open_purchase_sheet(Some(missing), cx));
            return;
        }
        self.is_sending = true;
        let key = self.send_key.clone();
        let for_self = self.is_shop_for_self(cx);
        let text = if for_self {
            String::new()
        } else {
            character_prefix(
                trimming_whitespaces_and_newlines(&self.caption),
                RiseGiftsLimits::caption_max_chars(),
            )
            .to_owned()
        };
        let hidden = if for_self { false } else { self.anonymous };
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = GiftsActionsStore::send(&gift, &recipient.id, &text, hidden, &key, cx).await;
            match result {
                Ok(response) => {
                    let _ = this.update(cx, |store, cx| {
                        store.send_key = Self::new_send_key();
                        store.sent_instance = response.instance;
                        cx.notify();
                    });
                    cx.update(|cx| {
                        SparksActionsStore::shared(cx).update(cx, |sparks, cx| sparks.refresh_balance_action(true, cx));
                    });
                }
                Err(error) => Self::handle_send_failure(&this, error, &gift, cx).await,
            }
            let _ = this.update(cx, |store, cx| {
                store.is_sending = false;
                cx.notify();
            });
        })
        .detach();
    }

    async fn handle_send_failure(
        this: &WeakEntity<Self>,
        error: anyhow::Error,
        gift: &RiseGiftCatalogItemDTO,
        cx: &mut AsyncApp,
    ) {
        let remote_code = RiseSparksFailure::code(&error);
        let code = remote_code.as_deref().and_then(ErrorCode::from_wire);
        match code {
            Some(ErrorCode::SparksInsufficientBalance) => {
                let _ = this.update(cx, |store, _| store.send_key = Self::new_send_key());
                let needed = RiseSparksFailure::integer_detail(ErrorParam::Shortfall.wire(), &error);
                cx.update(|cx| {
                    SparksInteractionsStore::shared(cx).update(cx, |sparks, cx| sparks.open_purchase_sheet(needed, cx));
                });
                return;
            }
            Some(ErrorCode::GiftPriceChanged | ErrorCode::GiftSoldOut) => {
                let _ = this.update(cx, |store, _| store.send_key = Self::new_send_key());
                if let Some(fresh) = GiftsActionsStore::load_catalog_item(gift.id, true, cx)
                    .await
                    .and_then(|response| response.gift)
                {
                    let _ = this.update(cx, |store, cx| {
                        store.selected_gift = if fresh.is_sold_out() { None } else { Some(fresh) };
                        cx.notify();
                    });
                }
                GiftsActionsStore::load_catalog(gift.kind(), true, cx).await;
            }
            Some(ErrorCode::CommonUpstreamUnavailable) => {}
            _ if remote_code.is_none() => {}
            _ => {
                let _ = this.update(cx, |store, _| store.send_key = Self::new_send_key());
            }
        }
        let title = RiseSparksFailure::message(&error);
        cx.update(|cx| {
            let button_height = f32::from(ThemeStore::shared(cx).read(cx).current_theme().button.height_300);
            let footer_lift = button_height + GIFT_SHOP_SHEET_FOOTER_VERTICAL_PADDING;
            let footer_lift = gift_shop_footer_window_lift()
                .map_or(footer_lift, |lift| footer_lift.max(lift - NOTIFIER_EDGE_PADDING));
            Notifier::send(
                title,
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    offset: footer_lift,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
    }

    pub async fn show_tab(owner_id: String, kind: Option<RiseGiftItemKind>, cx: &mut AsyncApp) {
        GiftsActionsStore::load_tab(&owner_id, None, false, cx).await;
        let Some(kind) = kind else {
            return;
        };
        let has_more = cx.update(|cx| GiftsServicesStore::shared(cx).read(cx).tab(&owner_id).has_more);
        if !has_more {
            return;
        }
        GiftsActionsStore::load_tab(&owner_id, Some(kind), false, cx).await;
    }

    pub async fn revalidate_tab(owner_id: String, cx: &mut AsyncApp) {
        let stale = cx.update(|cx| {
            if AuthServicesStore::shared(cx).read(cx).is_current_user(&owner_id) {
                return false;
            }
            GiftsServicesStore::shared(cx).read(cx).tab(&owner_id).loaded
        });
        if !stale {
            return;
        }
        GiftsActionsStore::refresh_tab(&owner_id, cx).await;
    }

    pub fn load_more_tab(&mut self, owner_id: String, kind: Option<RiseGiftItemKind>, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_more(&owner_id, kind, cx).await;
        })
        .detach();
    }

    pub fn open_instance(
        &mut self,
        instance: RiseGiftInstanceDTO,
        list: Option<GiftsInstanceList>,
        cx: &mut Context<Self>,
    ) {
        let id = instance.id.clone();
        self.opened_list = list;
        self.confirming_hide = false;
        self.opened_instance = Some(instance);
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Some(fresh) = GiftsActionsStore::instance(&id, cx).await else {
                return;
            };
            let _ = this.update(cx, |store, cx| {
                if store.is_opened(&fresh.id) {
                    store.opened_instance = Some(fresh);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn open_instance_id(&mut self, id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Some(instance) = GiftsActionsStore::instance(&id, cx).await else {
                cx.update(|cx| {
                    Notifier::send(
                        tr("server_error_gone"),
                        NotifierSendOptions {
                            r#type: NotifierType::Warning,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                });
                return;
            };
            let _ = this.update(cx, |store, cx| {
                store.opened_list = None;
                store.confirming_hide = false;
                store.opened_instance = Some(instance);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn close_instance(&mut self, cx: &mut Context<Self>) {
        self.opened_instance = None;
        self.opened_list = None;
        self.confirming_hide = false;
        cx.notify();
    }

    pub fn instance_page_ids(&self, cx: &mut App) -> Vec<String> {
        let Some(opened) = &self.opened_instance else {
            return Vec::new();
        };
        let Some(list) = &self.opened_list else {
            return vec![opened.id.clone()];
        };
        let services = GiftsServicesStore::shared(cx);
        let ids: Vec<String> = services
            .read(cx)
            .tab_kind(&list.owner_id, list.kind)
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect();
        if ids.contains(&opened.id) { ids } else { vec![opened.id.clone()] }
    }

    pub fn page_instance(&self, id: &str, cx: &mut App) -> Option<RiseGiftInstanceDTO> {
        if let Some(opened) = &self.opened_instance
            && opened.id == id
        {
            return Some(opened.clone());
        }
        let list = self.opened_list.as_ref()?;
        let services = GiftsServicesStore::shared(cx);
        services
            .read(cx)
            .tab_kind(&list.owner_id, list.kind)
            .items
            .iter()
            .find(|item| item.id == id)
            .cloned()
    }

    pub fn show_instance(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_opened(id) {
            return;
        }
        let Some(instance) = self.page_instance(id, cx) else {
            return;
        };
        self.confirming_hide = false;
        self.opened_instance = Some(instance);
        cx.notify();
    }

    pub fn load_more_opened_list(&mut self, cx: &mut Context<Self>) {
        let Some(list) = self.opened_list.clone() else {
            return;
        };
        self.load_more_tab(list.owner_id, list.kind, cx);
    }

    pub fn request_visibility_change_instance(&mut self, instance: RiseGiftInstanceDTO, cx: &mut Context<Self>) {
        if !self.is_opened(&instance.id) {
            self.opened_instance = Some(instance);
            cx.notify();
        }
        self.request_visibility_change(cx);
    }

    pub fn request_visibility_change(&mut self, cx: &mut Context<Self>) {
        let Some(instance) = self.opened_instance.clone() else {
            return;
        };
        if instance.is_displayed() {
            self.confirming_hide = true;
            cx.notify();
        } else {
            self.toggle_displayed_instance(instance, cx);
        }
    }

    pub fn toggle_displayed(&mut self, cx: &mut Context<Self>) {
        let Some(instance) = self.opened_instance.clone() else {
            return;
        };
        self.toggle_displayed_instance(instance, cx);
    }

    pub fn toggle_pinned(&mut self, cx: &mut Context<Self>) {
        let Some(instance) = self.opened_instance.clone() else {
            return;
        };
        self.toggle_pinned_instance(instance, cx);
    }

    pub fn toggle_displayed_instance(&mut self, instance: RiseGiftInstanceDTO, cx: &mut Context<Self>) {
        if self.is_changing_instance {
            return;
        }
        self.is_changing_instance = true;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            match GiftsActionsStore::set_displayed(&instance, !instance.is_displayed(), cx).await {
                Ok(changed) => {
                    let _ = this.update(cx, |store, cx| {
                        if store.is_opened(&changed.id) {
                            store.opened_instance = Some(changed);
                            cx.notify();
                        }
                    });
                }
                Err(error) => cx.update(|cx| Self::notify_error(&error, cx)),
            }
            let _ = this.update(cx, |store, cx| {
                store.is_changing_instance = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn toggle_pinned_instance(&mut self, instance: RiseGiftInstanceDTO, cx: &mut Context<Self>) {
        if self.is_changing_instance {
            return;
        }
        self.is_changing_instance = true;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            match GiftsActionsStore::set_pinned(&instance, !instance.is_pinned(), cx).await {
                Ok(changed) => {
                    let _ = this.update(cx, |store, cx| {
                        if store.is_opened(&changed.id) {
                            store.opened_instance = Some(changed);
                            cx.notify();
                        }
                    });
                }
                Err(error) => cx.update(|cx| Self::notify_error(&error, cx)),
            }
            let _ = this.update(cx, |store, cx| {
                store.is_changing_instance = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn gift_back(&mut self, giver: &RiseGiftUserDTO, cx: &mut Context<Self>) {
        self.pending_gift_back = Some(GiftRecipient {
            id: giver.id.clone(),
            name: giver.name.clone(),
            tag: giver.tag.clone(),
            avatar: giver.avatar.clone(),
            cover: None,
        });
        self.close_instance(cx);
    }

    pub fn instance_did_dismiss(&mut self, cx: &mut Context<Self>) {
        let Some(recipient) = self.pending_gift_back.take() else {
            return;
        };
        self.open_shop(recipient, RiseGiftItemKind::Item, cx);
    }

    pub fn open_odds(&mut self, gift_id: i64, kind: RiseGiftAttributeKind, cx: &mut Context<Self>) {
        self.odds_request = Some(GiftOddsRequest { gift_id, kind });
        cx.notify();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            GiftsActionsStore::load_catalog_item(gift_id, false, cx).await;
        })
        .detach();
    }

    pub fn close_odds(&mut self, cx: &mut Context<Self>) {
        self.odds_request = None;
        cx.notify();
    }

    pub fn localize_chat_card_name(&mut self, gift_id: i64, cx: &mut Context<Self>) {
        if gift_id <= 0 {
            return;
        }
        let store = GiftsServicesStore::shared(cx);
        let known = {
            let services = store.read(cx);
            services.catalog_details(cx).contains_key(&gift_id)
                || services.catalog_details_failed(cx).contains(&gift_id)
        };
        if known || !self.card_names_loading.insert(gift_id) {
            return;
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            GiftsActionsStore::load_catalog_item(gift_id, false, cx).await;
            let _ = this.update(cx, |store, _| {
                store.card_names_loading.remove(&gift_id);
            });
        })
        .detach();
    }

    fn decode_gift_received(payload: &RiseJSONValue) -> Result<RiseGiftInstanceDTO, String> {
        let data = payload.data(false).map_err(|error| format!("payload: {error}"))?;
        let event = RiseEvent::decode(rise_events::name::GIFT_RECEIVED, &data)
            .map_err(|error| format!("event: {error:?}"))?;
        let RiseEvent::GiftReceived(event) = event else {
            return Err("event: other kind".to_owned());
        };
        let wire = event.instance.ok_or_else(|| "instance: missing".to_owned())?;
        serde_json::from_value::<RiseGiftInstanceDTO>(wire).map_err(|error| format!("instance: {error}"))
    }

    pub fn handle_gift_received(&mut self, payload: RiseJSONValue, cx: &mut Context<Self>) {
        let instance = match Self::decode_gift_received(&payload) {
            Ok(instance) => instance,
            Err(reason) => {
                tracing::warn!(target: "riseonly::X1260", %reason, "gift received undecoded");
                GiftsActionsStore::shared(cx).update(cx, |actions, cx| actions.refresh_own_tab_if_loaded(cx));
                return;
            }
        };
        GiftsServicesStore::shared(cx).update(cx, |services, cx| services.insert(&instance, cx));
        if self.offer_to_wear(&instance, cx) {
            tracing::warn!(target: "riseonly::X1260", id = %instance.id, "gift received offered to wear");
            return;
        }
        if !instance.is_gifted_by_other() {
            tracing::warn!(target: "riseonly::X1260", id = %instance.id, grant = instance.is_grant(), anonymous = instance.anonymous, has_sender = instance.sender.is_some(), "gift received not from other");
            return;
        }
        tracing::warn!(target: "riseonly::X1260", id = %instance.id, "gift received toast");
        let sender = instance.sender.as_ref().map(|sender| sender.name.clone()).unwrap_or_default();
        let sent_by_owner =
            instance.sender.as_ref().map(|sender| sender.id.as_str()) == Some(instance.owner_user_id.as_str());
        let title = if instance.is_grant() {
            tr("gift_received_from_riseonly")
        } else if sender.is_empty() || sent_by_owner {
            tr("gift_received_toast")
        } else {
            format_printf(&tr("msg_sys_gift_sent_to_you"), &[FormatArg::from(sender.as_str())])
        };
        let instance_id = instance.id.clone();
        PushNotificationManager::shared(cx).update(cx, |manager, _| manager.note_gift_toast(instance_id));
        Notifier::send(
            title,
            NotifierSendOptions {
                content: Some(instance.name.clone()),
                r#type: NotifierType::Success,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}

pub enum RiseGiftsLimits {}

impl RiseGiftsLimits {
    pub fn caption_max_chars() -> usize {
        Self::service_limit("gift_caption_max_chars", service_limit::GIFT_CAPTION_MAX_CHARS)
    }

    pub fn pinned_max() -> usize {
        Self::service_limit("gift_pinned_max", service_limit::GIFT_PINNED_MAX)
    }

    fn service_limit(name: &str, fallback: u64) -> usize {
        let spec = SERVICE_LIMITS.iter().find(|spec| spec.name == name);
        debug_assert!(spec.is_some(), "market-service limit {name} is missing from canon");
        usize::try_from(spec.map_or(fallback, |spec| spec.value)).unwrap_or(usize::MAX)
    }
}

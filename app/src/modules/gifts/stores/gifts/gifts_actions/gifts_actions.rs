use std::collections::HashSet;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::rise_request::market;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::{CancellationError, RiseJSONValue};
use rise_i18n::LanguageStore;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::GIFT_SHOP_ITEM_SHELF_HAS_FILTERS;

use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::engine::core::rise_gifts_rpc::RiseGiftsRPC;
use crate::modules::gifts::engine::rise_gifts_domain::{RiseGiftsDomain, RiseGiftsDomainRegistry};
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftCatalogItemDTO, RiseGiftCatalogItemExt, RiseGiftCatalogItemResponseDTO, RiseGiftInstanceDTO,
    RiseGiftInstanceExt, RiseGiftItemKind, RiseGiftSendResponseDTO,
};
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_market_types::GiftsCatalogState;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::{GiftsServicesStore, GiftsTabState};

pub struct GiftsActionsStore {}

impl GiftsActionsStore {
    pub const PAGE_SIZE: i64 = 30;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn services(cx: &mut App) -> Entity<GiftsServicesStore> {
        GiftsServicesStore::shared(cx)
    }

    pub fn locale(cx: &mut App) -> String {
        LanguageStore::shared(cx).code().to_owned()
    }

    pub(crate) fn with_services<R>(
        cx: &mut AsyncApp,
        update: impl FnOnce(&mut GiftsServicesStore, &mut Context<GiftsServicesStore>) -> R,
    ) -> R {
        cx.update(|cx| Self::services(cx).update(cx, update))
    }

    async fn require_domain(cx: &mut AsyncApp) -> anyhow::Result<Arc<RiseGiftsDomain>> {
        let domain = cx.update(|cx| {
            RiseGiftsDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
        });
        domain.await
    }

    pub(crate) fn is_active(domain: &Arc<RiseGiftsDomain>, cx: &mut AsyncApp) -> bool {
        cx.update(|cx| {
            RiseGiftsDomainRegistry::shared(cx)
                .read(cx)
                .active()
                .is_some_and(|active| Arc::ptr_eq(active, domain))
        })
    }

    pub async fn rpc(cx: &mut AsyncApp) -> anyhow::Result<RiseGiftsRPC> {
        Ok(Self::require_domain(cx).await?.rpc.clone())
    }

    pub fn own_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    pub async fn load_catalog(kind: RiseGiftItemKind, force: bool, cx: &mut AsyncApp) {
        let requested = cx.update(Self::locale);
        let started = cx.update(|cx| {
            let services = Self::services(cx);
            services.update(cx, |store, cx| store.align_catalog_locale(&requested, cx));
            let mut state = services.read(cx).catalog(kind).clone();
            if state.loading || (!force && state.loaded) {
                return None;
            }
            state.loading = true;
            if kind == RiseGiftItemKind::Item {
                let has_filters = UserDefaults::standard()
                    .object(GIFT_SHOP_ITEM_SHELF_HAS_FILTERS)
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
                services.update(cx, |store, cx| store.set_item_shelf_has_filters(has_filters, cx));
            }
            if !state.loaded && RiseTransportStatus::is_offline(cx) {
                state.failed = true;
            }
            let was_loaded = state.loaded;
            services.update(cx, |store, cx| store.set_catalog(kind, state, cx));
            if was_loaded {
                return Some(None);
            }
            Some(Some(RiseTransportStatus::on_next_disconnect(
                move |cx| {
                    let waiting_store = Self::services(cx);
                    let mut waiting = waiting_store.read(cx).catalog(kind).clone();
                    if !waiting.loading || waiting.loaded || waiting.failed {
                        return;
                    }
                    waiting.failed = true;
                    waiting_store.update(cx, |store, cx| store.set_catalog(kind, waiting, cx));
                },
                cx,
            )))
        });
        let Some(watch) = started else {
            return;
        };
        let result: anyhow::Result<Vec<RiseGiftCatalogItemDTO>> = async {
            let rpc = Self::rpc(cx).await?;
            let mut items = Vec::new();
            let mut seen = HashSet::new();
            let mut after: Option<i64> = None;
            loop {
                let page = rpc.catalog(&requested, kind, after).await?;
                let fresh: Vec<RiseGiftCatalogItemDTO> =
                    page.items.into_iter().filter(|item| seen.insert(item.id)).collect();
                let last = fresh.last().map(|item| item.id);
                items.extend(fresh);
                match (page.has_more, last) {
                    (true, Some(last)) => after = Some(last),
                    _ => break,
                }
            }
            Ok(items)
        }
        .await;
        cx.update(|cx| {
            let services = Self::services(cx);
            if services.read(cx).catalog_locale() != requested.as_str() {
                return;
            }
            match result {
                Ok(items) => {
                    if kind == RiseGiftItemKind::Item {
                        let has_filters = items.iter().any(|item| item.is_limited())
                            && items.iter().any(|item| !item.is_limited());
                        UserDefaults::standard().set(GIFT_SHOP_ITEM_SHELF_HAS_FILTERS, has_filters);
                        services.update(cx, |store, cx| store.set_item_shelf_has_filters(has_filters, cx));
                    }
                    let state = GiftsCatalogState { items, loaded: true, loading: false, failed: false };
                    services.update(cx, |store, cx| store.set_catalog(kind, state, cx));
                }
                Err(_) => {
                    let mut failed = services.read(cx).catalog(kind).clone();
                    failed.loading = false;
                    failed.failed = !failed.loaded;
                    services.update(cx, |store, cx| store.set_catalog(kind, failed, cx));
                }
            }
        });
        if let Some(drop_watch) = watch {
            drop_watch.stop();
        }
    }

    pub async fn load_catalog_item(
        gift_id: i64,
        force: bool,
        cx: &mut AsyncApp,
    ) -> Option<RiseGiftCatalogItemResponseDTO> {
        let requested = cx.update(Self::locale);
        let cached = Self::with_services(cx, |store, cx| {
            store.align_catalog_locale(&requested, cx);
            if !force && let Some(cached) = store.catalog_details(cx).get(&gift_id) {
                return Some(cached.clone());
            }
            store.catalog_details_failed_mut(cx).remove(&gift_id);
            None
        });
        if cached.is_some() {
            return cached;
        }
        let response = match Self::rpc(cx).await {
            Ok(rpc) => rpc.catalog_item(gift_id, &requested).await.ok(),
            Err(_) => None,
        };
        Self::with_services(cx, |store, cx| {
            if store.catalog_locale() != requested.as_str() {
                return store.catalog_details(cx).get(&gift_id).cloned();
            }
            let Some(response) = response else {
                if !store.catalog_details(cx).contains_key(&gift_id) {
                    store.catalog_details_failed_mut(cx).insert(gift_id);
                }
                return store.catalog_details(cx).get(&gift_id).cloned();
            };
            store.catalog_details_failed_mut(cx).remove(&gift_id);
            store.catalog_details_mut(cx).insert(gift_id, response.clone());
            if let Some(gift) = &response.gift {
                store.replace_gift(gift, cx);
            }
            Some(response)
        })
    }

    pub async fn send(
        gift: &RiseGiftCatalogItemDTO,
        recipient_id: &str,
        caption: &str,
        anonymous: bool,
        idempotency_key: &str,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<RiseGiftSendResponseDTO> {
        let rpc = Self::rpc(cx).await?;
        let locale = cx.update(Self::locale);
        let response = rpc
            .send(market::SendGift {
                anonymous: Some(anonymous),
                caption: Some(caption.to_owned()),
                expected_price_sparks: Some(gift.price_sparks),
                gift_id: Some(gift.id),
                idempotency_key: Some(idempotency_key.to_owned()),
                locale: Some(locale),
                recipient_user_id: Some(recipient_id.to_owned()),
            })
            .await?;
        if let Some(instance) = &response.instance {
            Self::with_services(cx, |store, cx| store.insert(instance, cx));
        }
        let gift_id = gift.id;
        cx.spawn(async move |cx: &mut AsyncApp| {
            Self::load_catalog_item(gift_id, true, cx).await;
        })
        .detach();
        Ok(response)
    }

    pub async fn load_tab(owner_id: &str, kind: Option<RiseGiftItemKind>, force: bool, cx: &mut AsyncApp) {
        let owner = trimming_whitespaces_and_newlines(owner_id).to_owned();
        if owner.is_empty() {
            return;
        }
        let started = Self::with_services(cx, |store, cx| {
            let mut state = store.fetched_tab(&owner, kind).clone();
            if state.loading || (!force && state.loaded) {
                return None;
            }
            let depth = if state.loaded { state.items.len() } else { 0 };
            state.loading = true;
            store.set_tab_kind(&owner, kind, state, cx);
            Some(depth)
        });
        let Some(depth) = started else {
            return;
        };
        let domain = Self::require_domain(cx).await.ok();
        let result: anyhow::Result<Option<GiftsTabState>> = async {
            let Some(active) = &domain else {
                return Err(CancellationError.into());
            };
            let mut items: Vec<RiseGiftInstanceDTO> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            let mut cursor: Option<String> = None;
            loop {
                let locale = cx.update(Self::locale);
                let response = active
                    .rpc
                    .user_gifts(&owner, cursor.as_deref(), Self::PAGE_SIZE, &locale, kind)
                    .await?;
                if !Self::is_active(active, cx) {
                    return Ok(None);
                }
                items.extend(response.items.iter().filter(|item| seen.insert(item.id.clone())).cloned());
                cursor = if response.next_cursor.is_empty() { None } else { Some(response.next_cursor.clone()) };
                if items.len() < depth && response.has_more && cursor.is_some() {
                    continue;
                }
                return Ok(Some(GiftsTabState {
                    total: response.total.unwrap_or(items.len() as i64),
                    has_more: response.has_more,
                    next_cursor: cursor,
                    items,
                    loaded: true,
                    loading: false,
                    failed: false,
                }));
            }
        }
        .await;
        match result {
            Ok(Some(state)) => Self::with_services(cx, |store, cx| store.set_tab_kind(&owner, kind, state, cx)),
            Ok(None) => {}
            Err(_) => {
                if let Some(current) = &domain
                    && !Self::is_active(current, cx)
                {
                    return;
                }
                Self::with_services(cx, |store, cx| {
                    let mut failed = store.fetched_tab(&owner, kind).clone();
                    failed.loading = false;
                    failed.failed = !failed.loaded;
                    failed.loaded = true;
                    store.set_tab_kind(&owner, kind, failed, cx);
                });
            }
        }
    }

    pub async fn load_more(owner_id: &str, kind: Option<RiseGiftItemKind>, cx: &mut AsyncApp) {
        let started = Self::with_services(cx, |store, cx| {
            let mut state = store.fetched_tab(owner_id, kind).clone();
            if !state.loaded || !state.has_more || state.loading {
                return None;
            }
            let cursor = state.next_cursor.clone()?;
            state.loading = true;
            store.set_tab_kind(owner_id, kind, state, cx);
            Some(cursor)
        });
        let Some(cursor) = started else {
            return;
        };
        let domain = Self::require_domain(cx).await.ok();
        let result: anyhow::Result<Option<_>> = async {
            let Some(active) = &domain else {
                return Err(CancellationError.into());
            };
            let locale = cx.update(Self::locale);
            let response = active
                .rpc
                .user_gifts(owner_id, Some(cursor.as_str()), Self::PAGE_SIZE, &locale, kind)
                .await?;
            if !Self::is_active(active, cx) {
                return Ok(None);
            }
            Ok(Some(response))
        }
        .await;
        match result {
            Ok(Some(response)) => Self::with_services(cx, |store, cx| {
                let mut next = store.fetched_tab(owner_id, kind).clone();
                let known: HashSet<String> = next.items.iter().map(|item| item.id.clone()).collect();
                next.items
                    .extend(response.items.into_iter().filter(|item| !known.contains(&item.id)));
                next.total = response.total.unwrap_or(next.total);
                next.has_more = response.has_more;
                next.next_cursor =
                    if response.next_cursor.is_empty() { None } else { Some(response.next_cursor) };
                next.loading = false;
                store.set_tab_kind(owner_id, kind, next, cx);
            }),
            Ok(None) => {}
            Err(_) => {
                if let Some(current) = &domain
                    && !Self::is_active(current, cx)
                {
                    return;
                }
                Self::with_services(cx, |store, cx| {
                    let mut next = store.fetched_tab(owner_id, kind).clone();
                    next.loading = false;
                    store.set_tab_kind(owner_id, kind, next, cx);
                });
            }
        }
    }

    pub async fn refresh_tab(owner_id: &str, cx: &mut AsyncApp) {
        Self::load_tab(owner_id, None, true, cx).await;
        let kinds = Self::with_services(cx, |store, _| store.fetched_kinds(owner_id));
        for kind in kinds {
            Self::load_tab(owner_id, Some(kind), true, cx).await;
        }
    }

    pub fn refresh_own_tab_if_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(own) = Self::own_user_id(cx) else {
            return;
        };
        let store = Self::services(cx);
        let is_loaded = {
            let services = store.read(cx);
            services.tab(&own).loaded || !services.fetched_kinds(&own).is_empty()
        };
        if !is_loaded {
            return;
        }
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            Self::refresh_tab(&own, cx).await;
        })
        .detach();
    }

    pub fn handle_gifts_changed(&mut self, payload: RiseJSONValue, cx: &mut Context<Self>) {
        self.reload_own_look_if_changed(&payload, cx);
        self.refresh_own_tab_if_loaded(cx);
        self.refresh_own_cosmetics_if_loaded(cx);
    }

    pub async fn instance(id: &str, cx: &mut AsyncApp) -> Option<RiseGiftInstanceDTO> {
        let locale = cx.update(Self::locale);
        let rpc = Self::rpc(cx).await.ok()?;
        let response = rpc.instance(id, &locale).await.ok()?;
        if let Some(instance) = &response.instance {
            Self::with_services(cx, |store, cx| store.replace(instance, cx));
        }
        response.instance
    }

    pub async fn set_displayed(
        instance: &RiseGiftInstanceDTO,
        displayed: bool,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<RiseGiftInstanceDTO> {
        let state = Self::rpc(cx).await?.set_displayed(&instance.id, displayed).await?;
        let updated = instance.with_displayed(state.displayed, state.pinned, state.pinned_at_ms);
        Self::with_services(cx, |store, cx| store.replace(&updated, cx));
        Ok(updated)
    }

    pub async fn set_pinned(
        instance: &RiseGiftInstanceDTO,
        pinned: bool,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<RiseGiftInstanceDTO> {
        let state = Self::rpc(cx).await?.set_pinned(&instance.id, pinned).await?;
        let updated = instance.with_displayed(state.displayed, state.pinned, state.pinned_at_ms);
        Self::with_services(cx, |store, cx| store.replace(&updated, cx));
        Ok(updated)
    }
}

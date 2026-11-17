use std::collections::HashSet;

use gpui::{AsyncApp, Context};
use rise_canon::RiseEvent;
use rise_engine::RiseJSONValue;
use rise_gifts::GiftsChangedReason;

use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAppearanceDTO, RiseGiftAppearanceExt, RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftItemKind,
    RiseGiftTabResponseDTO,
};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_actions::gift_appearance_actions::GiftAppearanceActionsStore;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_market_types::GiftsAppearanceState;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsTabState;

impl GiftsActionsStore {
    const LOOK_CHANGE_REASONS: [GiftsChangedReason; 2] =
        [GiftsChangedReason::Equipped, GiftsChangedReason::Unequipped];

    pub async fn load_wardrobe(kind: RiseGiftItemKind, force: bool, cx: &mut AsyncApp) {
        Self::load_wardrobe_keeping_shown(kind, force, false, cx).await;
    }

    async fn load_wardrobe_keeping_shown(kind: RiseGiftItemKind, force: bool, keeps_shown: bool, cx: &mut AsyncApp) {
        if !kind.equippable() {
            return;
        }
        let Some(owner) = cx.update(Self::own_user_id) else {
            return;
        };
        let started = Self::with_services(cx, |store, cx| {
            let mut state = store.wardrobe(kind).clone();
            if state.loading || (!force && state.loaded) {
                return false;
            }
            state.loading = true;
            store.set_wardrobe(kind, state, cx);
            true
        });
        if !started {
            return;
        }
        let locale = cx.update(Self::locale);
        let result: anyhow::Result<RiseGiftTabResponseDTO> = async {
            Self::rpc(cx)
                .await?
                .user_gifts(&owner, None, Self::PAGE_SIZE, &locale, Some(kind))
                .await
        }
        .await;
        match result {
            Ok(response) => {
                let total = response.total.unwrap_or(response.items.len() as i64);
                let next_cursor = if response.next_cursor.is_empty() { None } else { Some(response.next_cursor) };
                let (mut items, rest): (Vec<RiseGiftInstanceDTO>, Vec<RiseGiftInstanceDTO>) =
                    response.items.into_iter().partition(|item| item.is_equipped());
                items.extend(rest);
                let fresh = GiftsTabState {
                    items,
                    total,
                    has_more: response.has_more,
                    next_cursor,
                    loaded: true,
                    loading: false,
                    failed: false,
                };
                Self::with_services(cx, |store, cx| {
                    let state =
                        if keeps_shown { Self::refreshed_wardrobe(store.wardrobe(kind), fresh) } else { fresh };
                    store.set_wardrobe(kind, state, cx);
                });
            }
            Err(_) => Self::with_services(cx, |store, cx| {
                let mut failed = store.wardrobe(kind).clone();
                failed.loading = false;
                failed.failed = !failed.loaded;
                failed.loaded = true;
                store.set_wardrobe(kind, failed, cx);
            }),
        }
    }

    fn refreshed_wardrobe(shown: &GiftsTabState, fresh: GiftsTabState) -> GiftsTabState {
        if !shown.loaded || shown.failed || shown.items.is_empty() {
            return fresh;
        }
        let shown_ids: HashSet<&str> = shown.items.iter().map(|item| item.id.as_str()).collect();
        let added: Vec<RiseGiftInstanceDTO> =
            fresh.items.iter().filter(|item| !shown_ids.contains(item.id.as_str())).cloned().collect();
        let head_span = fresh.items.len().saturating_sub(added.len());
        let mut items = added;
        for (index, item) in shown.items.iter().enumerate() {
            match fresh.items.iter().find(|candidate| candidate.id == item.id) {
                Some(updated) => items.push(updated.clone()),
                None if index >= head_span => items.push(item.clone()),
                None => {}
            }
        }
        let keeps_tail = shown.items.len() > fresh.items.len();
        GiftsTabState {
            items,
            total: fresh.total,
            has_more: if keeps_tail { shown.has_more } else { fresh.has_more },
            next_cursor: if keeps_tail { shown.next_cursor.clone() } else { fresh.next_cursor },
            loaded: true,
            loading: false,
            failed: false,
        }
    }

    pub async fn load_more_wardrobe(kind: RiseGiftItemKind, cx: &mut AsyncApp) {
        let Some(owner) = cx.update(Self::own_user_id) else {
            return;
        };
        let started = Self::with_services(cx, |store, cx| {
            let mut state = store.wardrobe(kind).clone();
            if !state.loaded || !state.has_more || state.loading {
                return None;
            }
            let cursor = state.next_cursor.clone()?;
            state.loading = true;
            store.set_wardrobe(kind, state, cx);
            Some(cursor)
        });
        let Some(cursor) = started else {
            return;
        };
        let locale = cx.update(Self::locale);
        let result: anyhow::Result<RiseGiftTabResponseDTO> = async {
            Self::rpc(cx)
                .await?
                .user_gifts(&owner, Some(cursor.as_str()), Self::PAGE_SIZE, &locale, Some(kind))
                .await
        }
        .await;
        match result {
            Ok(response) => Self::with_services(cx, |store, cx| {
                let mut next = store.wardrobe(kind).clone();
                let known: HashSet<String> = next.items.iter().map(|item| item.id.clone()).collect();
                next.items
                    .extend(response.items.into_iter().filter(|item| !known.contains(&item.id)));
                next.total = response.total.unwrap_or(next.total);
                next.has_more = response.has_more;
                next.next_cursor =
                    if response.next_cursor.is_empty() { None } else { Some(response.next_cursor) };
                next.loading = false;
                store.set_wardrobe(kind, next, cx);
            }),
            Err(_) => Self::with_services(cx, |store, cx| {
                let mut next = store.wardrobe(kind).clone();
                next.loading = false;
                store.set_wardrobe(kind, next, cx);
            }),
        }
    }

    pub async fn load_own_appearance(force: bool, cx: &mut AsyncApp) {
        let Some(owner) = cx.update(Self::own_user_id) else {
            return;
        };
        let resolving = cx.update(|cx| {
            GiftAppearanceActionsStore::shared(cx)
                .update(cx, |appearances, cx| appearances.resolve_appearance_action(owner, force, cx))
        });
        let Some(appearance) = resolving.await else {
            return;
        };
        Self::resolve_worn_instances(&appearance, cx).await;
    }

    async fn resolve_worn_instances(appearance: &RiseGiftAppearanceDTO, cx: &mut AsyncApp) {
        let gift_ids: HashSet<i64> = RiseGiftItemKind::cosmetics()
            .into_iter()
            .filter_map(|kind| appearance.worn(kind).map(|worn| worn.gift_id))
            .collect();
        for gift_id in gift_ids {
            if gift_id > 0 {
                Self::load_catalog_item(gift_id, false, cx).await;
            }
        }
        let requested = cx.update(Self::locale);
        let relocalized = Self::with_services(cx, |store, cx| {
            let current = store.appearance().locale.clone();
            let relocalized = !current.is_empty() && current != requested;
            if current != requested {
                let mut next = if relocalized { GiftsAppearanceState::empty() } else { store.appearance().clone() };
                next.locale = requested.clone();
                store.set_appearance(next, cx);
            }
            relocalized
        });
        let worn: Vec<String> = RiseGiftItemKind::cosmetics()
            .into_iter()
            .filter_map(|kind| appearance.worn(kind).map(|worn| worn.instance_id.clone()))
            .collect();
        for id in worn {
            let looked_up = cx.update(|cx| {
                let own = Self::own_user_id(cx).unwrap_or_default();
                let store = Self::services(cx);
                let services = store.read(cx);
                if services.appearance().instances.contains_key(&id) {
                    return None;
                }
                if relocalized {
                    return Some(None);
                }
                let known = services
                    .tab(&own)
                    .items
                    .iter()
                    .find(|item| item.id == id)
                    .or_else(|| {
                        RiseGiftItemKind::cosmetics()
                            .into_iter()
                            .find_map(|kind| services.wardrobe(kind).items.iter().find(|item| item.id == id))
                    })
                    .cloned();
                Some(known)
            });
            let Some(cached) = looked_up else {
                continue;
            };
            let instance = match cached {
                Some(hit) => Some(hit),
                None => Self::instance(&id, cx).await,
            };
            let Some(instance) = instance else {
                continue;
            };
            Self::with_services(cx, |store, cx| {
                if store.appearance().locale != requested {
                    return;
                }
                let mut next = store.appearance().clone();
                next.instances.insert(id.clone(), instance);
                store.set_appearance(next, cx);
            });
        }
    }

    pub async fn set_equipped(
        instance: &RiseGiftInstanceDTO,
        equipped: bool,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<RiseGiftInstanceDTO> {
        let state = Self::rpc(cx).await?.set_equipped(&instance.id, equipped).await?;
        let now = state.equipped;
        let changed = instance.with_equipped(now);
        Self::with_services(cx, |store, cx| store.apply_equipped(instance, now, cx));
        cx.update(|cx| {
            GiftAppearanceActionsStore::shared(cx)
                .update(cx, |appearances, cx| appearances.apply_own_equipped_action(&changed, now, cx));
        });
        Ok(changed)
    }

    pub fn reload_own_look_if_changed(&mut self, payload: &RiseJSONValue, cx: &mut Context<Self>) {
        let Ok(data) = payload.data(false) else {
            return;
        };
        let Ok(RiseEvent::UserGiftsChanged(event)) = RiseEvent::decode(rise_events::name::USER_GIFTS_CHANGED, &data)
        else {
            return;
        };
        if !Self::LOOK_CHANGE_REASONS.iter().any(|reason| reason.slug() == event.reason) {
            return;
        }
        let Some(owner) = Self::own_user_id(cx) else {
            return;
        };
        GiftAppearanceActionsStore::shared(cx)
            .update(cx, |appearances, cx| appearances.reload_changed_appearance_action(owner, cx));
    }

    pub fn refresh_own_cosmetics_if_loaded(&mut self, cx: &mut Context<Self>) {
        let store = Self::services(cx);
        let (kinds, names_read) = {
            let services = store.read(cx);
            let kinds: Vec<RiseGiftItemKind> = RiseGiftItemKind::cosmetics()
                .into_iter()
                .filter(|kind| services.wardrobe(*kind).loaded)
                .collect();
            (kinds, !services.appearance().instances.is_empty())
        };
        if kinds.is_empty() && !names_read {
            return;
        }
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            for kind in kinds {
                Self::load_wardrobe_keeping_shown(kind, true, true, cx).await;
            }
            if names_read {
                Self::load_own_appearance(false, cx).await;
            }
        })
        .detach();
    }
}

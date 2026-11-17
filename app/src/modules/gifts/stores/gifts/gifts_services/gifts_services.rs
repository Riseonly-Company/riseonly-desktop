use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::LanguageStore;

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftCatalogItemDTO, RiseGiftCatalogItemResponseDTO, RiseGiftInstanceDTO, RiseGiftInstanceExt,
    RiseGiftItemKind,
};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_market_types::{
    GiftsAppearanceState, GiftsCatalogState,
};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftRecipient;
use crate::modules::user::stores::profile_inner_content_events::PROFILE_INNER_CONTENT_WILL_REORDER;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GiftsTabState {
    pub items: Vec<RiseGiftInstanceDTO>,
    pub total: i64,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub loaded: bool,
    pub loading: bool,
    pub failed: bool,
}

impl GiftsTabState {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_complete(&self) -> bool {
        self.loaded && !self.has_more
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GiftsKindTabKey {
    pub owner_id: String,
    pub kind: RiseGiftItemKind,
}

static EMPTY_CATALOG: LazyLock<GiftsCatalogState> = LazyLock::new(GiftsCatalogState::empty);
static EMPTY_TAB: LazyLock<GiftsTabState> = LazyLock::new(GiftsTabState::empty);
static EMPTY_CATALOG_DETAILS: LazyLock<HashMap<i64, RiseGiftCatalogItemResponseDTO>> =
    LazyLock::new(HashMap::new);
static EMPTY_CATALOG_DETAILS_FAILED: LazyLock<HashSet<i64>> = LazyLock::new(HashSet::new);

pub struct GiftsServicesStore {
    catalogs: HashMap<RiseGiftItemKind, GiftsCatalogState>,
    item_shelf_has_filters: bool,
    stored_catalog_details: HashMap<i64, RiseGiftCatalogItemResponseDTO>,
    stored_catalog_details_failed: HashSet<i64>,
    catalog_locale: String,
    tabs: HashMap<String, GiftsTabState>,
    kind_tabs: HashMap<GiftsKindTabKey, GiftsTabState>,
    kind_cuts: HashMap<GiftsKindTabKey, GiftsTabState>,
    wardrobes: HashMap<RiseGiftItemKind, GiftsTabState>,
    appearance: GiftsAppearanceState,
    tab_owners: Vec<String>,
    people: HashMap<String, GiftRecipient>,
    people_order: Vec<String>,
}

impl GiftsServicesStore {
    const PEOPLE_LIMIT: usize = 64;
    const TAB_OWNERS_LIMIT: usize = 32;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            catalogs: HashMap::new(),
            item_shelf_has_filters: true,
            stored_catalog_details: HashMap::new(),
            stored_catalog_details_failed: HashSet::new(),
            catalog_locale: String::new(),
            tabs: HashMap::new(),
            kind_tabs: HashMap::new(),
            kind_cuts: HashMap::new(),
            wardrobes: HashMap::new(),
            appearance: GiftsAppearanceState::empty(),
            tab_owners: Vec::new(),
            people: HashMap::new(),
            people_order: Vec::new(),
        }
    }

    pub fn catalogs(&self) -> &HashMap<RiseGiftItemKind, GiftsCatalogState> {
        &self.catalogs
    }

    pub fn item_shelf_has_filters(&self) -> bool {
        self.item_shelf_has_filters
    }

    fn is_catalog_locale_current(&self, cx: &App) -> bool {
        LanguageStore::active(cx).is_some_and(|language| language.code() == self.catalog_locale)
    }

    pub fn catalog_details(&self, cx: &App) -> &HashMap<i64, RiseGiftCatalogItemResponseDTO> {
        if self.is_catalog_locale_current(cx) {
            &self.stored_catalog_details
        } else {
            &EMPTY_CATALOG_DETAILS
        }
    }

    pub fn catalog_details_mut(
        &mut self,
        cx: &mut Context<Self>,
    ) -> &mut HashMap<i64, RiseGiftCatalogItemResponseDTO> {
        if !self.is_catalog_locale_current(cx) {
            self.stored_catalog_details.clear();
        }
        cx.notify();
        &mut self.stored_catalog_details
    }

    pub fn set_catalog_details(
        &mut self,
        value: HashMap<i64, RiseGiftCatalogItemResponseDTO>,
        cx: &mut Context<Self>,
    ) {
        self.stored_catalog_details = value;
        cx.notify();
    }

    pub fn catalog_details_failed(&self, cx: &App) -> &HashSet<i64> {
        if self.is_catalog_locale_current(cx) {
            &self.stored_catalog_details_failed
        } else {
            &EMPTY_CATALOG_DETAILS_FAILED
        }
    }

    pub fn catalog_details_failed_mut(&mut self, cx: &mut Context<Self>) -> &mut HashSet<i64> {
        if !self.is_catalog_locale_current(cx) {
            self.stored_catalog_details_failed.clear();
        }
        cx.notify();
        &mut self.stored_catalog_details_failed
    }

    pub fn set_catalog_details_failed(&mut self, value: HashSet<i64>, cx: &mut Context<Self>) {
        self.stored_catalog_details_failed = value;
        cx.notify();
    }

    pub fn catalog_locale(&self) -> &str {
        &self.catalog_locale
    }

    pub fn tabs(&self) -> &HashMap<String, GiftsTabState> {
        &self.tabs
    }

    pub fn kind_tabs(&self) -> &HashMap<GiftsKindTabKey, GiftsTabState> {
        &self.kind_tabs
    }

    pub fn kind_cuts(&self) -> &HashMap<GiftsKindTabKey, GiftsTabState> {
        &self.kind_cuts
    }

    pub fn wardrobes(&self) -> &HashMap<RiseGiftItemKind, GiftsTabState> {
        &self.wardrobes
    }

    pub fn appearance(&self) -> &GiftsAppearanceState {
        &self.appearance
    }

    pub fn catalog(&self, kind: RiseGiftItemKind) -> &GiftsCatalogState {
        self.catalogs.get(&kind).unwrap_or(&EMPTY_CATALOG)
    }

    pub fn localized_name(&self, gift_id: i64, cx: &App) -> Option<String> {
        if gift_id <= 0 {
            return None;
        }
        let name = &self.catalog_details(cx).get(&gift_id)?.gift.as_ref()?.name;
        let trimmed = trimming_whitespaces_and_newlines(name);
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    pub fn align_catalog_locale(&mut self, locale: &str, cx: &mut Context<Self>) {
        if self.catalog_locale == locale {
            return;
        }
        self.catalog_locale = locale.to_owned();
        self.catalogs.clear();
        self.stored_catalog_details.clear();
        self.stored_catalog_details_failed.clear();
        cx.notify();
    }

    pub fn set_catalog(&mut self, kind: RiseGiftItemKind, state: GiftsCatalogState, cx: &mut Context<Self>) {
        self.catalogs.insert(kind, state);
        cx.notify();
    }

    pub fn set_item_shelf_has_filters(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.item_shelf_has_filters == value {
            return;
        }
        self.item_shelf_has_filters = value;
        cx.notify();
    }

    pub fn replace_gift(&mut self, gift: &RiseGiftCatalogItemDTO, cx: &mut Context<Self>) {
        let mut changed = false;
        for state in self.catalogs.values_mut() {
            let Some(index) = state.items.iter().position(|item| item.id == gift.id) else {
                continue;
            };
            state.items[index] = gift.clone();
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    pub fn tab(&self, owner_id: &str) -> &GiftsTabState {
        self.tabs.get(owner_id).unwrap_or(&EMPTY_TAB)
    }

    pub fn tab_kind(&self, owner_id: &str, kind: Option<RiseGiftItemKind>) -> &GiftsTabState {
        let Some(kind) = kind else {
            return self.tab(owner_id);
        };
        let key = GiftsKindTabKey { owner_id: owner_id.to_owned(), kind };
        self.kind_cuts
            .get(&key)
            .or_else(|| self.kind_tabs.get(&key))
            .unwrap_or(&EMPTY_TAB)
    }

    pub fn fetched_tab(&self, owner_id: &str, kind: Option<RiseGiftItemKind>) -> &GiftsTabState {
        let Some(kind) = kind else {
            return self.tab(owner_id);
        };
        self.kind_tabs
            .get(&GiftsKindTabKey { owner_id: owner_id.to_owned(), kind })
            .unwrap_or(&EMPTY_TAB)
    }

    pub fn set_tab(&mut self, owner_id: &str, state: GiftsTabState, cx: &mut Context<Self>) {
        self.set_tab_kind(owner_id, None, state, cx);
    }

    pub fn set_tab_kind(
        &mut self,
        owner_id: &str,
        kind: Option<RiseGiftItemKind>,
        state: GiftsTabState,
        cx: &mut Context<Self>,
    ) {
        self.remember_tab_owner(owner_id);
        Self::announce_reorder(&self.fetched_tab(owner_id, kind).items, &state.items);
        match kind {
            Some(kind) => {
                self.kind_tabs.insert(GiftsKindTabKey { owner_id: owner_id.to_owned(), kind }, state);
            }
            None => self.commit_tab(owner_id, state),
        }
        cx.notify();
    }

    pub fn has_gifts(&self, owner_id: &str) -> bool {
        self.tab(owner_id).total > 0
    }

    pub fn fetched_kinds(&self, owner_id: &str) -> Vec<RiseGiftItemKind> {
        self.kind_tabs
            .iter()
            .filter(|(key, state)| key.owner_id == owner_id && state.loaded)
            .map(|(key, _)| key.kind)
            .collect()
    }

    pub fn replace(&mut self, instance: &RiseGiftInstanceDTO, cx: &mut Context<Self>) {
        let owners: Vec<String> = self
            .tabs
            .iter()
            .filter(|(_, state)| state.items.iter().any(|item| item.id == instance.id))
            .map(|(owner, _)| owner.clone())
            .collect();
        for owner in owners {
            let Some(state) = self.tabs.get(&owner) else {
                continue;
            };
            let Some(index) = state.items.iter().position(|item| item.id == instance.id) else {
                continue;
            };
            let mut next = state.clone();
            next.items[index] = instance.clone();
            next.items = Self::tab_order(next.items);
            Self::announce_reorder(&state.items, &next.items);
            self.commit_tab(&owner, next);
        }
        for state in self.kind_tabs.values_mut() {
            let Some(index) = state.items.iter().position(|item| item.id == instance.id) else {
                continue;
            };
            let old = state.items.clone();
            state.items[index] = instance.clone();
            state.items = Self::tab_order(std::mem::take(&mut state.items));
            Self::announce_reorder(&old, &state.items);
        }
        for state in self.wardrobes.values_mut() {
            let Some(index) = state.items.iter().position(|item| item.id == instance.id) else {
                continue;
            };
            state.items[index] = instance.clone();
        }
        if let Some(known) = self.appearance.instances.get_mut(&instance.id) {
            *known = instance.clone();
        }
        cx.notify();
    }

    pub fn insert(&mut self, instance: &RiseGiftInstanceDTO, cx: &mut Context<Self>) {
        if let Some(state) = self.tabs.get(&instance.owner_user_id)
            && state.loaded
            && !state.items.iter().any(|item| item.id == instance.id)
        {
            let mut next = state.clone();
            let mut items = vec![instance.clone()];
            items.extend(next.items.iter().cloned());
            next.items = Self::tab_order(items);
            next.total += 1;
            Self::announce_reorder(&state.items, &next.items);
            self.commit_tab(&instance.owner_user_id, next);
        }
        let key = GiftsKindTabKey { owner_id: instance.owner_user_id.clone(), kind: instance.kind() };
        if let Some(state) = self.kind_tabs.get_mut(&key)
            && state.loaded
            && !state.items.iter().any(|item| item.id == instance.id)
        {
            let old = state.items.clone();
            let mut items = vec![instance.clone()];
            items.append(&mut state.items);
            state.items = Self::tab_order(items);
            state.total += 1;
            Self::announce_reorder(&old, &state.items);
        }
        let is_own = AuthServicesStore::shared(cx).read(cx).is_current_user(&instance.owner_user_id);
        if instance.is_cosmetic()
            && is_own
            && let Some(shelf) = self.wardrobes.get_mut(&instance.kind())
            && shelf.loaded
            && !shelf.items.iter().any(|item| item.id == instance.id)
        {
            shelf.items.insert(0, instance.clone());
            shelf.total += 1;
        }
        cx.notify();
    }

    pub fn tab_order(mut items: Vec<RiseGiftInstanceDTO>) -> Vec<RiseGiftInstanceDTO> {
        items.sort_by(|lhs, rhs| match (lhs.is_pinned(), rhs.is_pinned()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => rhs.pinned_at_ms.cmp(&lhs.pinned_at_ms),
            (false, false) => rhs.created_at_ms.cmp(&lhs.created_at_ms),
        });
        items
    }

    fn announce_reorder(old: &[RiseGiftInstanceDTO], new: &[RiseGiftInstanceDTO]) {
        let keeps_order =
            new.len() >= old.len() && new.iter().zip(old.iter()).all(|(next, previous)| next.id == previous.id);
        if old.is_empty() || keeps_order {
            return;
        }
        RiseAppEvents::default_center().post(PROFILE_INNER_CONTENT_WILL_REORDER);
    }

    fn commit_tab(&mut self, owner_id: &str, state: GiftsTabState) {
        for kind in RiseGiftItemKind::SHOP_ORDER {
            let key = GiftsKindTabKey { owner_id: owner_id.to_owned(), kind };
            if !state.is_complete() {
                self.kind_cuts.remove(&key);
                continue;
            }
            let items: Vec<RiseGiftInstanceDTO> =
                state.items.iter().filter(|item| item.kind() == kind).cloned().collect();
            let cut = GiftsTabState {
                total: items.len() as i64,
                items,
                loaded: true,
                ..GiftsTabState::default()
            };
            if self.kind_cuts.get(&key) != Some(&cut) {
                self.kind_cuts.insert(key, cut);
            }
        }
        self.tabs.insert(owner_id.to_owned(), state);
    }

    fn remember_tab_owner(&mut self, owner_id: &str) {
        if let Some(index) = self.tab_owners.iter().position(|owner| owner == owner_id) {
            self.tab_owners.remove(index);
        }
        self.tab_owners.push(owner_id.to_owned());
        while self.tab_owners.len() > Self::TAB_OWNERS_LIMIT {
            let stale = self.tab_owners.remove(0);
            self.tabs.remove(&stale);
            for kind in RiseGiftItemKind::SHOP_ORDER {
                let key = GiftsKindTabKey { owner_id: stale.clone(), kind };
                self.kind_tabs.remove(&key);
                self.kind_cuts.remove(&key);
            }
        }
    }

    pub fn wardrobe(&self, kind: RiseGiftItemKind) -> &GiftsTabState {
        self.wardrobes.get(&kind).unwrap_or(&EMPTY_TAB)
    }

    pub fn set_wardrobe(&mut self, kind: RiseGiftItemKind, state: GiftsTabState, cx: &mut Context<Self>) {
        self.wardrobes.insert(kind, state);
        cx.notify();
    }

    pub fn set_appearance(&mut self, state: GiftsAppearanceState, cx: &mut Context<Self>) {
        self.appearance = state;
        cx.notify();
    }

    pub fn apply_equipped(&mut self, instance: &RiseGiftInstanceDTO, equipped: bool, cx: &mut Context<Self>) {
        let kind = instance.kind();
        let changed = instance.with_equipped(equipped);
        let adjusted = |item: &RiseGiftInstanceDTO| -> RiseGiftInstanceDTO {
            if item.id == changed.id {
                return changed.clone();
            }
            if equipped && item.kind() == kind && item.is_equipped() && item.owner_user_id == changed.owner_user_id {
                return item.with_equipped(false);
            }
            item.clone()
        };
        if let Some(state) = self.tabs.get(&changed.owner_user_id) {
            let mut next = state.clone();
            next.items = state.items.iter().map(&adjusted).collect();
            if &next != state {
                self.commit_tab(&changed.owner_user_id, next);
            }
        }
        let key = GiftsKindTabKey { owner_id: changed.owner_user_id.clone(), kind };
        if let Some(state) = self.kind_tabs.get_mut(&key) {
            let items: Vec<RiseGiftInstanceDTO> = state.items.iter().map(&adjusted).collect();
            if items != state.items {
                state.items = items;
            }
        }
        if let Some(shelf) = self.wardrobes.get_mut(&kind) {
            shelf.items = shelf.items.iter().map(&adjusted).collect();
        }
        if equipped || self.appearance.instances.contains_key(&changed.id) {
            self.appearance.instances.insert(changed.id.clone(), changed.clone());
        }
        cx.notify();
    }

    pub fn remember(&mut self, person: GiftRecipient, cx: &mut Context<Self>) {
        if !self.people.contains_key(&person.id) {
            self.people_order.push(person.id.clone());
        }
        self.people.insert(person.id.clone(), person);
        while self.people_order.len() > Self::PEOPLE_LIMIT {
            let stale = self.people_order.remove(0);
            self.people.remove(&stale);
        }
        cx.notify();
    }

    pub fn person(&self, id: &str) -> Option<&GiftRecipient> {
        self.people.get(id)
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.catalogs.clear();
        self.stored_catalog_details.clear();
        self.stored_catalog_details_failed.clear();
        self.tabs.clear();
        self.kind_tabs.clear();
        self.kind_cuts.clear();
        self.tab_owners.clear();
        self.wardrobes.clear();
        self.appearance = GiftsAppearanceState::empty();
        self.people.clear();
        self.people_order.clear();
        cx.notify();
    }
}

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, Context, Entity, Subscription};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::stickers::engine::rise_sticker_models::{
    RiseStickerLoadState, RiseStickerMutationID, RiseStickerMutationState, RiseStickerPerformancePolicy,
    RiseStickerPresentationSnapshot, RiseStickerResourceID, RiseStickerUploadPresentation,
};
use crate::modules::stickers::engine::rise_pack_collaboration_presentation::{
    RisePackCollaborationPresentation, RisePackCollaborationSnapshot,
};
use crate::modules::stickers::engine::rise_sticker_presentation::RiseStickerPresentation;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_actions::StickersActionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, PackKind, StickerItem, StickerPack,
};
use crate::modules::stickers::stores::stickers::stickers_guide_sandbox_hooks::stickers_guide_sandbox_hooks;

pub struct StickersServicesStore {
    presentation: Entity<RiseStickerPresentation>,
    snapshot: Arc<RiseStickerPresentationSnapshot>,
    sticker_uploads: Arc<HashMap<String, RiseStickerUploadPresentation>>,
    pub(super) collaboration: Arc<RisePackCollaborationSnapshot>,
    effective_user_id_for_wire: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl StickersServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseStickerPresentation::shared(cx);
        let collaboration = RisePackCollaborationPresentation::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let effective_user_id_for_wire = auth.read(cx).effective_user_id_for_wire();
        let weak = cx.weak_entity();
        let mut subscriptions = vec![
            cx.observe(&presentation, |store, _, cx| store.refresh_snapshot(cx)),
            cx.observe(&collaboration, |store, collaboration, cx| {
                store.collaboration = Arc::clone(collaboration.read(cx).snapshot());
                cx.notify();
            }),
            cx.observe(&auth, |store, auth, cx| {
                let user_id = auth.read(cx).effective_user_id_for_wire();
                if store.effective_user_id_for_wire != user_id {
                    store.effective_user_id_for_wire = user_id;
                    cx.notify();
                }
            }),
        ];
        let sandbox_subscription = (stickers_guide_sandbox_hooks().observe_sandbox)(
            Box::new(move |cx| {
                if let Some(store) = weak.upgrade() {
                    store.update(cx, |store, cx| store.refresh_snapshot(cx));
                }
            }),
            cx,
        );
        subscriptions.extend(sandbox_subscription);
        let (snapshot, sticker_uploads) = Self::projected(&presentation, cx);
        let collaboration = Arc::clone(collaboration.read(cx).snapshot());
        Self {
            presentation,
            snapshot,
            sticker_uploads,
            collaboration,
            effective_user_id_for_wire,
            _subscriptions: subscriptions,
        }
    }

    #[allow(clippy::type_complexity)]
    fn projected(
        presentation: &Entity<RiseStickerPresentation>,
        cx: &App,
    ) -> (Arc<RiseStickerPresentationSnapshot>, Arc<HashMap<String, RiseStickerUploadPresentation>>) {
        let current = presentation.read(cx);
        let real = Arc::clone(current.snapshot());
        let uploads = Arc::clone(current.sticker_uploads());
        let sandbox = stickers_guide_sandbox_hooks();
        if (sandbox.owns_stickers_scene)(cx) {
            (Arc::new((sandbox.stickers_snapshot)((*real).clone(), cx)), uploads)
        } else {
            (real, uploads)
        }
    }

    fn refresh_snapshot(&mut self, cx: &mut Context<Self>) {
        let (snapshot, sticker_uploads) = Self::projected(&self.presentation, cx);
        if Arc::ptr_eq(&self.snapshot, &snapshot) && Arc::ptr_eq(&self.sticker_uploads, &sticker_uploads) {
            return;
        }
        self.snapshot = snapshot;
        self.sticker_uploads = sticker_uploads;
        cx.notify();
    }

    pub fn pending_favourite_ids(&self) -> &HashSet<String> {
        &self.snapshot.pending_favourite_ids
    }

    pub fn pending_gif_ids(&self) -> &HashSet<String> {
        &self.snapshot.pending_gif_ids
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        StickersActionsStore::shared(cx).update(cx, |actions, cx| actions.cancel_all_sticker_uploads_action(cx));
    }

    pub fn default_packs(&self, kind: PackKind) -> Vec<StickerPack> {
        self.snapshot.default_packs.get(&kind).map(|page| page.items.clone()).unwrap_or_default()
    }

    pub fn saved_packs(&self, user_id: &str, kind: PackKind) -> Vec<StickerPack> {
        if !self.owns(user_id) {
            return Vec::new();
        }
        self.snapshot
            .saved_packs
            .get(&kind)
            .map(|page| page.items.iter().take(Self::picker_packs_limit()).cloned().collect())
            .unwrap_or_default()
    }

    pub fn saved_pack_summaries(&self, user_id: &str, kind: PackKind) -> Vec<StickerPack> {
        if !self.owns(user_id) {
            return Vec::new();
        }
        self.snapshot.saved_packs.get(&kind).map(|page| page.items.clone()).unwrap_or_default()
    }

    pub fn my_packs(&self, user_id: &str, kind: PackKind) -> Vec<StickerPack> {
        if !self.owns(user_id) {
            return Vec::new();
        }
        self.snapshot.my_packs.get(&kind).map(|page| page.items.clone()).unwrap_or_default()
    }

    pub fn my_packs_slice(&self, user_id: &str, kind: PackKind) -> &[StickerPack] {
        if !self.owns(user_id) {
            return &[];
        }
        self.snapshot.my_packs.get(&kind).map_or(&[], |page| page.items.as_slice())
    }

    pub fn pack_first_sticker(&self, pack_id: &str) -> Option<&StickerItem> {
        self.snapshot.packs.get(pack_id).and_then(|content| content.stickers.first())
    }

    pub fn default_packs_next_relative_id(&self, kind: PackKind) -> Option<String> {
        self.snapshot.default_packs.get(&kind).and_then(|page| page.next_relative_id.clone())
    }

    pub fn default_packs_has_more(&self, kind: PackKind) -> bool {
        self.snapshot.default_packs.get(&kind).is_some_and(|page| page.has_more)
    }

    pub fn default_packs_resolved(&self, kind: PackKind) -> bool {
        self.snapshot.default_packs.contains_key(&kind)
    }

    pub fn saved_packs_next_relative_id(&self, user_id: &str, kind: PackKind) -> Option<String> {
        if !self.owns(user_id) {
            return None;
        }
        self.snapshot.saved_packs.get(&kind).and_then(|page| page.next_relative_id.clone())
    }

    pub fn saved_packs_resolved(&self, user_id: &str, kind: PackKind) -> bool {
        self.owns(user_id) && self.snapshot.saved_packs.contains_key(&kind)
    }

    pub fn saved_packs_failed(&self, user_id: &str, kind: PackKind) -> bool {
        self.owns(user_id)
            && !self.snapshot.saved_packs.contains_key(&kind)
            && self.snapshot.load_states.get(&RiseStickerResourceID::SavedPacks(kind)) == Some(&RiseStickerLoadState::Failed)
    }

    pub fn saved_packs_has_more(&self, user_id: &str, kind: PackKind) -> bool {
        self.owns(user_id) && self.snapshot.saved_packs.get(&kind).is_some_and(|page| page.has_more)
    }

    pub fn my_packs_next_relative_id(&self, user_id: &str, kind: PackKind) -> Option<String> {
        if !self.owns(user_id) {
            return None;
        }
        self.snapshot.my_packs.get(&kind).and_then(|page| page.next_relative_id.clone())
    }

    pub fn my_packs_has_more(&self, user_id: &str, kind: PackKind) -> bool {
        self.owns(user_id) && self.snapshot.my_packs.get(&kind).is_some_and(|page| page.has_more)
    }

    pub fn my_packs_total(&self, user_id: &str, kind: PackKind) -> i64 {
        if !self.owns(user_id) {
            return 0;
        }
        let page = self.snapshot.my_packs.get(&kind);
        let total = page.and_then(|page| page.total).unwrap_or(0);
        let count = page.map_or(0, |page| i64::try_from(page.items.len()).unwrap_or(i64::MAX));
        total.max(count)
    }

    pub fn is_creating_pack(&self, kind: PackKind) -> bool {
        Self::is_in_flight(self.snapshot.mutation_states.get(&RiseStickerMutationID::CreatePack(kind)))
    }

    pub fn create_pack_error(&self, kind: PackKind) -> Option<String> {
        self.snapshot.mutation_errors.get(&RiseStickerMutationID::CreatePack(kind)).cloned()
    }

    pub fn upload_progress(&self, pack_id: &str) -> Option<f64> {
        self.snapshot.mutation_progress.get(&RiseStickerMutationID::AddSticker { pack_id: pack_id.to_owned() }).copied()
    }

    pub fn sticker_upload_error(&self, pack_id: &str) -> Option<String> {
        self.snapshot.mutation_errors.get(&RiseStickerMutationID::AddSticker { pack_id: pack_id.to_owned() }).cloned()
    }

    pub fn sticker_edit_error(&self, sticker_id: &str) -> Option<String> {
        self.snapshot
            .mutation_errors
            .get(&RiseStickerMutationID::EditSticker { sticker_id: sticker_id.to_owned() })
            .cloned()
    }

    pub fn is_uploading_sticker(&self, pack_id: &str) -> bool {
        Self::is_in_flight(
            self.snapshot.mutation_states.get(&RiseStickerMutationID::AddSticker { pack_id: pack_id.to_owned() }),
        )
    }

    pub fn sticker_upload(&self, local_sticker_id: &str) -> Option<RiseStickerUploadPresentation> {
        self.sticker_uploads.get(local_sticker_id).cloned()
    }

    pub fn mutation_state(&self, mutation: &RiseStickerMutationID) -> Option<RiseStickerMutationState> {
        self.snapshot.mutation_states.get(mutation).cloned()
    }

    pub fn pack_stickers(&self, pack_id: &str) -> Vec<StickerItem> {
        self.snapshot.packs.get(pack_id).map(|content| content.stickers.clone()).unwrap_or_default()
    }

    pub fn has_resolved_pack(&self, pack_id: &str) -> bool {
        self.snapshot.packs.contains_key(pack_id)
    }

    pub fn is_pack_unavailable(&self, pack_id: &str) -> bool {
        !self.snapshot.packs.contains_key(pack_id)
            && self.snapshot.load_states.get(&RiseStickerResourceID::Pack(pack_id.to_owned()))
                == Some(&RiseStickerLoadState::Loaded)
    }

    pub fn pack(&self, pack_id: &str) -> Option<StickerPack> {
        self.snapshot.packs.get(pack_id).and_then(|content| content.pack.clone())
    }

    pub fn pack_summary(&self, pack_id: &str) -> Option<StickerPack> {
        if let Some(pack) = self.pack(pack_id) {
            return Some(pack);
        }
        for kind in [PackKind::Sticker, PackKind::Emoji] {
            for pages in [&self.snapshot.my_packs, &self.snapshot.saved_packs, &self.snapshot.default_packs] {
                if let Some(pack) = pages.get(&kind).and_then(|page| page.items.iter().find(|pack| pack.id == pack_id)) {
                    return Some(pack.clone());
                }
            }
        }
        None
    }

    pub fn pack_by_link(&self, link: &str) -> Option<StickerPack> {
        self.snapshot.linked_packs.get(Self::normalized(link)).and_then(|content| content.pack.clone())
    }

    pub fn stickers_by_link(&self, link: &str) -> Vec<StickerItem> {
        self.snapshot.linked_packs.get(Self::normalized(link)).map(|content| content.stickers.clone()).unwrap_or_default()
    }

    pub fn favourites(&self, user_id: &str) -> Vec<StickerItem> {
        if self.owns(user_id) { self.snapshot.favourites.clone() } else { Vec::new() }
    }

    pub fn recents(&self, user_id: &str, kind: Option<PackKind>) -> Vec<StickerItem> {
        if !self.owns(user_id) {
            return Vec::new();
        }
        if let Some(kind) = kind {
            return self.snapshot.recents.get(&kind).cloned().unwrap_or_default();
        }
        Self::deduplicated(self.snapshot.recents.values().flatten().cloned().collect())
    }

    pub fn gif_favourites(&self, user_id: &str) -> Vec<GifItem> {
        if self.owns(user_id) { self.snapshot.gif_favourites.items.clone() } else { Vec::new() }
    }

    pub fn gif_favourites_next_relative_id(&self, user_id: &str) -> Option<String> {
        if self.owns(user_id) { self.snapshot.gif_favourites.next_relative_id.clone() } else { None }
    }

    pub fn gif_favourites_has_more(&self, user_id: &str) -> bool {
        self.owns(user_id) && self.snapshot.gif_favourites.has_more
    }

    pub fn gif_search_results(&self, query: &str) -> Vec<GifItem> {
        self.snapshot.gif_searches.get(Self::normalized(query)).map(|page| page.items.clone()).unwrap_or_default()
    }

    pub fn gif_search_next_relative_id(&self, query: &str) -> Option<String> {
        self.snapshot.gif_searches.get(Self::normalized(query)).and_then(|page| page.next_relative_id.clone())
    }

    pub fn gif_search_has_more(&self, query: &str) -> bool {
        self.snapshot.gif_searches.get(Self::normalized(query)).is_some_and(|page| page.has_more)
    }

    pub fn is_loading(&self, resource: RiseStickerResourceID) -> bool {
        self.snapshot.load_states.get(&resource) == Some(&RiseStickerLoadState::Loading)
    }

    pub fn is_failed(&self, resource: RiseStickerResourceID) -> bool {
        self.snapshot.load_states.get(&resource) == Some(&RiseStickerLoadState::Failed)
    }

    pub fn is_favourite(&self, user_id: &str, sticker_id: &str) -> bool {
        self.owns(user_id) && self.snapshot.favourites.iter().any(|sticker| sticker.id == sticker_id)
    }

    pub fn gif_favourite(&self, user_id: &str, gif: &GifItem) -> Option<GifItem> {
        if !self.owns(user_id) {
            return None;
        }
        self.snapshot.gif_favourites.items.iter().find(|saved| Self::is_same_gif(saved, gif)).cloned()
    }

    pub fn is_gif_favourite(&self, user_id: &str, gif: &GifItem) -> bool {
        self.gif_favourite(user_id, gif).is_some()
    }

    pub fn custom_emoji_item(&self, id: &str, cx: &App) -> Option<StickerItem> {
        let user_id = self.effective_user_id_for_wire.as_deref()?;
        if !self.owns(user_id) {
            return None;
        }
        self.presentation.read(cx).custom_emoji(id, |snapshot| {
            if let Some(hit) =
                snapshot.recents.get(&PackKind::Emoji).and_then(|recents| recents.iter().find(|sticker| sticker.id == id))
            {
                return Some(hit.clone());
            }
            let defaults = snapshot.default_packs.get(&PackKind::Emoji).map(|page| page.items.as_slice()).unwrap_or_default();
            let saved = snapshot.saved_packs.get(&PackKind::Emoji).map(|page| page.items.as_slice()).unwrap_or_default();
            for pack in defaults.iter().chain(saved.iter().take(Self::picker_packs_limit())) {
                if let Some(hit) =
                    snapshot.packs.get(&pack.id).and_then(|content| content.stickers.iter().find(|sticker| sticker.id == id))
                {
                    return Some(hit.clone());
                }
            }
            snapshot.custom_emoji.get(id).cloned()
        })
    }

    fn owns(&self, user_id: &str) -> bool {
        !user_id.is_empty() && self.snapshot.account_id == user_id
    }

    fn normalized(value: &str) -> &str {
        trimming_whitespaces_and_newlines(value)
    }

    fn picker_packs_limit() -> usize {
        usize::try_from(RiseStickerPerformancePolicy::picker_packs_limit()).unwrap_or(0)
    }

    fn is_in_flight(state: Option<&RiseStickerMutationState>) -> bool {
        match state {
            Some(
                RiseStickerMutationState::Preparing
                | RiseStickerMutationState::Submitting
                | RiseStickerMutationState::Uploading,
            ) => true,
            Some(RiseStickerMutationState::Failed) | None => false,
        }
    }

    fn is_same_gif(saved: &GifItem, gif: &GifItem) -> bool {
        if saved.id == gif.id {
            return true;
        }
        if !gif.file_url.is_empty() && saved.file_url == gif.file_url {
            return true;
        }
        !gif.source_id.is_empty() && saved.source == gif.source && saved.source_id == gif.source_id
    }

    fn deduplicated(values: Vec<StickerItem>) -> Vec<StickerItem> {
        let mut seen = HashSet::new();
        values.into_iter().filter(|sticker| !sticker.id.is_empty() && seen.insert(sticker.id.clone())).collect()
    }
}

use gpui::{App, AsyncApp, Context, Entity, Task};

use crate::modules::gifts::engine::rise_gifts_domain::RiseGiftsDomainRegistry;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAppearanceDTO, RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftWornDTO, RiseGiftWornExt,
};

pub struct GiftAppearanceActionsStore {}

impl GiftAppearanceActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn load_appearances_action(&mut self, user_ids: Vec<String>, force: bool, cx: &mut Context<Self>) {
        if user_ids.is_empty() {
            return;
        }

        let domain = RiseGiftsDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx));
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.appearances.load(&user_ids, force).await;
        })
        .detach();
    }

    pub fn resolve_appearance_action(
        &mut self,
        user_id: String,
        force: bool,
        cx: &mut Context<Self>,
    ) -> Task<Option<RiseGiftAppearanceDTO>> {
        let domain = RiseGiftsDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx));
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return None;
            };

            domain.appearances.resolve(&user_id, force).await
        })
    }

    pub fn apply_own_equipped_action(
        &mut self,
        instance: &RiseGiftInstanceDTO,
        equipped: bool,
        cx: &mut Context<Self>,
    ) {
        if !instance.is_cosmetic() {
            return;
        }
        let Some(domain) = RiseGiftsDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return;
        };

        let owner = instance.owner_user_id.clone();
        let worn = RiseGiftWornDTO::new(instance);
        let kind = instance.kind();
        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.appearances.apply_equipped(&owner, worn, kind, equipped).await;
            domain.appearances.load(&[owner], true).await;
        })
        .detach();
    }

    pub fn reload_changed_appearance_action(&mut self, user_id: String, cx: &mut Context<Self>) {
        let Some(domain) = RiseGiftsDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return;
        };

        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.appearances.invalidate(&user_id).await;
            domain.appearances.load(&[user_id], true).await;
        })
        .detach();
    }
}

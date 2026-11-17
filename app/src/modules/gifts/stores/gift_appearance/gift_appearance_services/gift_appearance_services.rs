use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::engine::rise_gift_appearance_presentation::RiseGiftAppearancePresentation;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAppearanceDTO, RiseGiftAppearanceExt, RiseGiftItemKind, RiseGiftWornDTO,
};

pub struct GiftAppearanceServicesStore {}

impl GiftAppearanceServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn presentation(cx: &mut App) -> Entity<RiseGiftAppearancePresentation> {
        RiseGiftAppearancePresentation::shared(cx)
    }

    pub fn normalized(raw: Option<&str>) -> Option<String> {
        let raw = raw?;
        let id = trimming_whitespaces_and_newlines(raw);
        if id.is_empty() { None } else { Some(id.to_owned()) }
    }

    pub fn appearance(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) -> Option<RiseGiftAppearanceDTO> {
        let id = Self::normalized(user_id)?;
        Self::presentation(cx).update(cx, |presentation, cx| presentation.appearance(&id, cx))
    }

    pub fn worn(
        &mut self,
        kind: RiseGiftItemKind,
        of: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<RiseGiftWornDTO> {
        self.appearance(of, cx)?.worn(kind).cloned()
    }

    pub fn own_appearance(&mut self, cx: &mut Context<Self>) -> Option<RiseGiftAppearanceDTO> {
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        self.appearance(user_id.as_deref(), cx)
    }
}

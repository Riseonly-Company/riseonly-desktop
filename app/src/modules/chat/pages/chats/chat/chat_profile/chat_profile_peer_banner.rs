use std::sync::Arc;

use gpui::{App, ElementId, SharedString, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::swiftui::prelude::*;

use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::{
    ChatBackdropLayers, ChatBackdropLayout,
};
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornProfileEffect;
use crate::modules::user::stores::user::user_actions::user_wire_types::UserGetProfileByTagGatewayDTOExt;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

fn named_child(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// drawn from the top of the screen down to the middle of the avatar
/// a worn profile effect plays once over it, above the picture below controls
/// bot shows only its picture
#[derive(IntoElement)]
pub struct ChatProfilePeerBanner {
    id: ElementId,
    user_id: Option<String>,
    cover: Option<String>,
    top_padding: f32,
    avatar_center: f32,
    effect_replay: i64,
}

impl ChatProfilePeerBanner {
    pub const CORNER_RADIUS: f32 = 26.0;
    pub const PICTURE_LIFT: f32 = 50.0;

    pub fn new(
        id: impl Into<ElementId>,
        user_id: Option<String>,
        cover: Option<String>,
        top_padding: f32,
        avatar_center: f32,
    ) -> Self {
        Self { id: id.into(), user_id, cover, top_padding, avatar_center, effect_replay: 0 }
    }

    pub fn effect_replay(mut self, effect_replay: i64) -> Self {
        self.effect_replay = effect_replay;
        self
    }

    /// banner the person uploaded, the foreign profile is already read for this screen
    pub fn cover(user_id: &str, cx: &mut App) -> Option<String> {
        let payload =
            UserServicesStore::shared(cx).update(cx, |services, cx| services.foreign_profile_payload(user_id, cx));
        Self::picture(payload?.as_profile_model()?.cover_image_url.as_deref())
    }

    /// a stored banner address the backdrop can draw, empty means no picture
    pub fn picture(raw: Option<&str>) -> Option<String> {
        let source = trimming_whitespaces_and_newlines(raw?);
        source.to_lowercase().starts_with("http").then(|| source.to_owned())
    }
}

impl RenderOnce for ChatProfilePeerBanner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let reach = list_shell_floating_top_inset(cx).max(0.0) + self.top_padding;

        let interactions = UserInteractionsStore::shared(cx);
        let task_user_id = self.user_id.clone();
        use_task(named_child(&self.id, "load"), self.user_id.clone(), window, cx, async move |cx| {
            let Some(user_id) = task_user_id else {
                return;
            };
            interactions.update(cx, |interactions, cx| interactions.load_foreign_profile_user_id(user_id, cx));
        });

        let effect_id = ElementId::NamedChild(
            Arc::new(named_child(&self.id, "effect")),
            SharedString::from(self.effect_replay.to_string()),
        );
        div()
            .relative()
            .top(px(-reach))
            .w_full()
            .h(px(reach + self.avatar_center))
            .rounded_bl(px(Self::CORNER_RADIUS))
            .rounded_br(px(Self::CORNER_RADIUS))
            .overflow_hidden()
            .when_some(self.cover, |banner, cover| {
                banner.child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(ChatBackdropLayers::new(ChatBackdropLayout::ProfileBanner).banner_url(Some(cover))),
                )
            })
            .when(self.user_id.is_some(), |banner| {
                banner.child(div().absolute().inset_0().child(GiftWornProfileEffect::new(effect_id, self.user_id)))
            })
    }
}

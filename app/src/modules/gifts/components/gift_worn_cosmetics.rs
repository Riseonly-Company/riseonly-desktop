use std::sync::Arc;

use gpui::{AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, Styled, Window, div};
use rise_theme::theme::app_theme::alpha;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::linear_gradient::{GradientStop, LinearGradient};
use rise_ui::swiftui::shapes::pass_through;
use rise_ui::swiftui::transition::UnitPoint;

use crate::modules::gifts::components::gift_cosmetic_playback::{GiftMotionStyle, GiftMotionStyleKey};
use crate::modules::gifts::components::gift_frame_view::GiftFrameModifier;
use crate::modules::gifts::components::gift_nameplate_view::GiftNameplateView;
use crate::modules::gifts::components::gift_profile_effect_view::GiftProfileEffectView;
use crate::modules::gifts::engine::rise_gifts_engine_models::{RiseGiftItemKind, RiseGiftWornDTO};
use crate::modules::gifts::stores::gift_appearance::gift_appearance_interactions::gift_appearance_interactions::GiftAppearanceInteractionsStore;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_services::gift_appearance_services::GiftAppearanceServicesStore;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn worn(kind: RiseGiftItemKind, user_id: Option<&str>, cx: &mut App) -> Option<RiseGiftWornDTO> {
    GiftAppearanceServicesStore::shared(cx).update(cx, |store, cx| store.worn(kind, user_id, cx))
}

/// asks the store to want this user appearance when the view appears
fn want_on_appear(id: ElementId, user_id: Option<String>, window: &mut Window, cx: &mut App) {
    let wanted = user_id.clone();
    use_task(id, user_id, window, cx, async move |cx| {
        cx.update(|_, cx| {
            GiftAppearanceInteractionsStore::shared(cx).update(cx, |store, cx| store.want(wanted.as_deref(), cx));
        })
        .ok();
    });
}

pub trait GiftWornCosmeticsModifiers: IntoElement + Sized {
    /// the frame user_id wears around this avatar, plays once per appearance or loops while on
    /// screen when loops is set like a profile header
    /// avatar keeps its size and touches
    fn gift_worn_frame(self, id: impl Into<ElementId>, user_id: Option<String>, avatar_size: f32) -> GiftWornFrameModifier {
        GiftWornFrameModifier {
            id: id.into(),
            content: self.into_any_element(),
            user_id,
            avatar_size,
            animated: true,
            loops: false,
        }
    }

    /// the nameplate user_id wears behind this member row with its rolled effect behind the plate
    /// row keeps its height and hit areas
    fn gift_worn_nameplate(self, id: impl Into<ElementId>, user_id: Option<String>) -> GiftWornNameplateModifier {
        GiftWornNameplateModifier { id: id.into(), content: self.into_any_element(), user_id }
    }
}

impl<T: IntoElement> GiftWornCosmeticsModifiers for T {}

#[derive(IntoElement)]
pub struct GiftWornFrameModifier {
    id: ElementId,
    content: AnyElement,
    user_id: Option<String>,
    avatar_size: f32,
    animated: bool,
    loops: bool,
}

impl GiftWornFrameModifier {
    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    pub fn loops(mut self, loops: bool) -> Self {
        self.loops = loops;
        self
    }
}

impl RenderOnce for GiftWornFrameModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let worn = worn(RiseGiftItemKind::AvatarFrame, self.user_id.as_deref(), cx);
        want_on_appear(child_id(&self.id, "want"), self.user_id.clone(), window, cx);
        let motion_style = if self.loops { GiftMotionStyle::Continuous } else { env::<GiftMotionStyleKey>(cx) };
        let frame = GiftFrameModifier::new(
            child_id(&self.id, "frame"),
            worn.as_ref().and_then(|worn| worn.model.as_ref()).and_then(|model| model.media.clone()),
            self.avatar_size,
            self.content,
        )
        .effect(worn.as_ref().and_then(|worn| worn.effect.as_ref()).and_then(|effect| effect.media.clone()))
        .animated(self.animated);
        env_scope::<GiftMotionStyleKey>(motion_style).child(frame)
    }
}

#[derive(IntoElement)]
pub struct GiftWornNameplateModifier {
    id: ElementId,
    content: AnyElement,
    user_id: Option<String>,
}

impl RenderOnce for GiftWornNameplateModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        want_on_appear(child_id(&self.id, "want"), self.user_id.clone(), window, cx);
        div()
            .relative()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(GiftWornNameplateLayer { id: child_id(&self.id, "plate"), user_id: self.user_id }),
            )
            .child(self.content)
    }
}

#[derive(IntoElement)]
struct GiftWornNameplateLayer {
    id: ElementId,
    user_id: Option<String>,
}

impl RenderOnce for GiftWornNameplateLayer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let worn = worn(RiseGiftItemKind::Nameplate, self.user_id.as_deref(), cx);
        let model = worn.as_ref().and_then(|worn| worn.model.as_ref()).and_then(|model| model.media.clone());
        let effect = worn.as_ref().and_then(|worn| worn.effect.as_ref()).and_then(|effect| effect.media.clone());
        if model.is_none() && effect.is_none() {
            return div().into_any_element();
        }
        let page = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let layer = pass_through()
            .relative()
            .size_full()
            .child(
                GiftNameplateView::new(child_id(&self.id, "view"), model)
                    .effect(effect)
                    .animated(true),
            )
            .child(
                div().absolute().inset_0().child(
                    LinearGradient::new(
                        vec![
                            GradientStop::new(alpha(page, 0.55), 0.0),
                            GradientStop::new(alpha(page, 0.3), 0.4),
                            GradientStop::new(alpha(page, 0.0), 0.72),
                        ],
                        UnitPoint::LEADING,
                        UnitPoint::TRAILING,
                    )
                    .view(),
                ),
            );
        env_scope::<GiftMotionStyleKey>(GiftMotionStyle::Continuous).child(layer).into_any_element()
    }
}

/// profile effect user_id wears over a banner, one canon length play on open
/// not again on scroll or click or return from a pushed screen
/// only a new page or replay bump replays it, never with reduce motion
#[derive(IntoElement)]
pub struct GiftWornProfileEffect {
    id: ElementId,
    user_id: Option<String>,
    replay: i64,
}

impl GiftWornProfileEffect {
    pub fn new(id: impl Into<ElementId>, user_id: Option<String>) -> Self {
        Self { id: id.into(), user_id, replay: 0 }
    }

    pub fn replay(mut self, replay: i64) -> Self {
        self.replay = replay;
        self
    }
}

impl RenderOnce for GiftWornProfileEffect {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let worn = worn(RiseGiftItemKind::ProfileEffect, self.user_id.as_deref(), cx);
        GiftProfileEffectView::new(
            child_id(&self.id, "effect"),
            worn.as_ref().and_then(|worn| worn.model.as_ref()).and_then(|model| model.media.clone()),
        )
        .once(true)
        .rests_on_poster(false)
        .replay(self.replay)
    }
}

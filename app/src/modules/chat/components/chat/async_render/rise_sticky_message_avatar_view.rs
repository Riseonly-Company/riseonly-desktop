use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AccessibleAction, App, Context, ElementId, RenderOnce, Role, SharedString, Window, div};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListBottomStickyHeaderInteractive, RiseManualListReusableView,
};
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;

pub type RiseStickyMessageAvatarTap = Rc<dyn Fn(&mut Window, &mut App)>;

/// theme is a gpui global here so theme store environment has nothing to carry
#[derive(Clone, Default, IntoElement)]
struct RiseStickyMessageAvatarContent {
    id: Option<ElementId>,
    source: Option<String>,
    name: Option<String>,
    user_id: Option<String>,
    color_scope_id: Option<String>,
    is_deleted: bool,
    size: f32,
}

impl RenderOnce for RiseStickyMessageAvatarContent {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (Some(id), true) = (self.id, self.size > 0.0) else {
            return div().into_any_element();
        };
        UserLogoUi::new(id.clone())
            .source(self.source.map(SharedString::from))
            .name(self.name.map(SharedString::from))
            .user_id(self.user_id.clone().map(SharedString::from))
            .color_scope_id(self.color_scope_id.map(SharedString::from))
            .is_deleted(self.is_deleted)
            .size(self.size)
            .gift_worn_frame((id, "worn-frame"), self.user_id, self.size)
            .into_any_element()
    }
}

pub struct RiseStickyMessageAvatarView {
    hosted_view: RiseStickyMessageAvatarContent,
    on_tap: Option<RiseStickyMessageAvatarTap>,
    accessibility_label: Option<String>,
}

impl RiseStickyMessageAvatarView {
    pub const REUSE_IDENTIFIER: &'static str = "RiseStickyMessageAvatarView";

    pub fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_bottom_sticky_header_interactive::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        Self { hosted_view: RiseStickyMessageAvatarContent::default(), on_tap: None, accessibility_label: None }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        source: Option<String>,
        name: Option<String>,
        user_id: Option<String>,
        color_scope_id: Option<String>,
        is_deleted: bool,
        size: f32,
        on_tap: RiseStickyMessageAvatarTap,
        cx: &mut Context<Self>,
    ) {
        self.on_tap = Some(on_tap);
        self.accessibility_label = name.clone();
        self.hosted_view = RiseStickyMessageAvatarContent {
            id: Some(ElementId::View(cx.entity_id())),
            source,
            name,
            user_id,
            color_scope_id,
            is_deleted,
            size,
        };
        cx.notify();
    }

    pub fn accessibility_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(on_tap) = self.on_tap.clone() else {
            return false;
        };
        window.defer(cx, move |window, cx| on_tap(window, cx));
        true
    }
}

impl RiseManualListReusableView for RiseStickyMessageAvatarView {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        self.on_tap = None;
        self.accessibility_label = None;
        self.hosted_view = RiseStickyMessageAvatarContent::default();
        cx.notify();
    }
}

impl RiseManualListBottomStickyHeaderInteractive for RiseStickyMessageAvatarView {
    fn activate_bottom_sticky_header(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.accessibility_activate(window, cx);
    }
}

impl Render for RiseStickyMessageAvatarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let root = div()
            .id(("rise-sticky-message-avatar", cx.entity_id().as_u64()))
            .size_full()
            .role(Role::Button)
            .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                let _ = this.update(cx, |view, cx| {
                    view.accessibility_activate(window, cx);
                });
            })
            .child(self.hosted_view.clone());
        match self.accessibility_label.clone() {
            Some(label) => root.aria_label(label.clone()).tooltip_label(label).into_any_element(),
            None => root.into_any_element(),
        }
    }
}

use gpui::{
    AnyView, App, Bounds, Context, ElementId, Entity, EntityId, IntoElement, ParentElement, Pixels, Point,
    Render, RenderOnce, StyleRefinement, Styled, Window, WindowId, div,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::measure::measure;

use crate::modules::chat::components::chat::input_picker::chat_gif_search_overlay::GifSearchHero;

struct PortalSlot {
    owner: EntityId,
    window: WindowId,
    view: AnyView,
    bounds: Option<Bounds<Pixels>>,
    dismissed: bool,
}

/// root layer and the slot an anchor fills, one sheet at a time since only one chat shows it
pub struct ChatGifSearchPortal {
    slot: Option<PortalSlot>,
    presence: Animated<f32>,
    origin: Point<Pixels>,
}

impl ChatGifSearchPortal {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { slot: None, presence: Animated::new(0.0), origin: Point::default() })
    }

    fn present(&mut self, owner: EntityId, window: WindowId, view: &AnyView, cx: &mut Context<Self>) {
        if let Some(slot) = self.slot.as_ref()
            && slot.owner == owner
            && slot.view.entity_id() == view.entity_id()
            && !slot.dismissed
        {
            return;
        }
        let bounds = self.slot.as_ref().filter(|slot| slot.owner == owner).and_then(|slot| slot.bounds);
        self.slot = Some(PortalSlot { owner, window, view: view.clone(), bounds, dismissed: false });
        self.presence.set_in(1.0, GifSearchHero::animation(), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn set_bounds(&mut self, owner: EntityId, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if let Some(slot) = self.slot.as_mut().filter(|slot| slot.owner == owner)
            && slot.bounds != Some(bounds)
        {
            slot.bounds = Some(bounds);
            cx.notify();
        }
    }

    /// anchor left the chat (binding flipped or chat closed), the sheet fades out
    fn dismiss(&mut self, owner: EntityId, cx: &mut Context<Self>) {
        if let Some(slot) = self.slot.as_mut().filter(|slot| slot.owner == owner && !slot.dismissed) {
            slot.dismissed = true;
            self.presence.set_in(0.0, GifSearchHero::animation(), RiseClock::monotonic(), cx);
            cx.notify();
        }
    }
}

impl Render for ChatGifSearchPortal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let presence = self.presence.frame_value(now, window);
        let running = self.presence.is_running(now);
        if running {
            window.request_animation_frame();
        } else if self.slot.as_ref().is_some_and(|slot| slot.dismissed) {
            self.slot = None;
        }
        let this = cx.weak_entity();
        let layer = div().size_full().relative().child(measure("chat-gif-search-portal-origin", move |bounds, _, cx| {
            let _ = this.update(cx, |portal, cx| {
                if portal.origin != bounds.origin {
                    portal.origin = bounds.origin;
                    cx.notify();
                }
            });
        }));
        let Some(slot) = self.slot.as_ref() else {
            return layer;
        };
        let Some(bounds) = slot.bounds.filter(|_| slot.window == window.window_handle().window_id()) else {
            return layer;
        };
        let content = if running {
            slot.view.clone().into_any_element()
        } else {
            slot.view.clone().cached(StyleRefinement::default().size_full()).into_any_element()
        };
        layer.child(
            div()
                .absolute()
                .left(bounds.origin.x - self.origin.x)
                .top(bounds.origin.y - self.origin.y)
                .w(bounds.size.width)
                .h(bounds.size.height)
                .opacity(presence.clamp(0.0, 1.0))
                .child(content),
        )
    }
}

struct PortalOwner;

/// placed where the chat drew the sheet, reports its bounds to the root layer
#[derive(IntoElement)]
pub struct ChatGifSearchPortalAnchor {
    id: ElementId,
    view: AnyView,
}

impl ChatGifSearchPortalAnchor {
    pub fn new(id: impl Into<ElementId>, view: AnyView) -> Self {
        Self { id: id.into(), view }
    }
}

impl RenderOnce for ChatGifSearchPortalAnchor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let owner = window
            .use_keyed_state(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "owner".into()), cx, |_, cx| {
                let owner = cx.entity_id();
                cx.on_release(move |_, cx| {
                    ChatGifSearchPortal::shared(cx).update(cx, |portal, cx| portal.dismiss(owner, cx));
                })
                .detach();
                PortalOwner
            })
            .entity_id();
        let window_id = window.window_handle().window_id();
        let portal = ChatGifSearchPortal::shared(cx);
        portal.update(cx, |portal, cx| portal.present(owner, window_id, &self.view, cx));
        div().absolute().inset_0().child(measure(
            ElementId::NamedChild(std::sync::Arc::new(self.id), "bounds".into()),
            move |bounds, _, cx| {
                portal.update(cx, |portal, cx| portal.set_bounds(owner, bounds, cx));
            },
        ))
    }
}

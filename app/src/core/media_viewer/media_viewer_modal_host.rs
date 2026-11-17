use gpui::{AnyElement, App, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, anchored, deferred, div, px};
use rise_ui::swiftui::prelude::env_scope;
use rise_ui::window_cover::WindowCoverFrame;

use crate::core::media_viewer::media_viewer_host::MediaViewerHost;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerPresentationRoute, MediaViewerPresentationState,
};
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerFloorKey;

/// presents the media viewer host as the top window layer for a modal route
/// the comments sheet is a modal too and the root overlay stays under it whatever the z index
pub struct MediaViewerModalHostBridge {
    state: Entity<MediaViewerPresentationState>,
    last_is_presented: bool,
    last_items_empty: bool,
    last_route: MediaViewerPresentationRoute,
    _state_observation: Subscription,
    _presenter_observation: Subscription,
}

impl MediaViewerModalHostBridge {
    pub fn new(state: Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) -> Self {
        let snapshot = state.read(cx);
        let last_is_presented = snapshot.is_presented();
        let last_items_empty = snapshot.items().is_empty();
        let last_route = snapshot.presentation_route();
        let observation = cx.observe(&state, |this: &mut Self, state, cx| {
            let snapshot = state.read(cx);
            let is_presented = snapshot.is_presented();
            let items_empty = snapshot.items().is_empty();
            let route = snapshot.presentation_route();
            if is_presented == this.last_is_presented
                && items_empty == this.last_items_empty
                && route == this.last_route
            {
                return;
            }
            this.last_is_presented = is_presented;
            this.last_items_empty = items_empty;
            this.last_route = route;
            this.sync(cx);
        });
        let presenter = MediaViewerModalPresenter::shared(cx);
        let presenter_observation = cx.observe(&presenter, |_, _, cx| cx.notify());
        let bridge = Self {
            state,
            last_is_presented,
            last_items_empty,
            last_route,
            _state_observation: observation,
            _presenter_observation: presenter_observation,
        };
        bridge.sync(cx);
        bridge
    }

    fn sync(&self, cx: &mut App) {
        let state = self.state.clone();
        MediaViewerModalPresenter::shared(cx).update(cx, |presenter, cx| presenter.sync(state, cx));
    }
}

impl MediaViewerModalHostBridge {
    pub const LAYER_PRIORITY: usize = 1400;
}

impl Render for MediaViewerModalHostBridge {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = MediaViewerModalPresenter::shared(cx).read(cx).host() else {
            return div().into_any_element();
        };
        let cover = WindowCoverFrame::resolve(window, cx);
        let layer: AnyElement = anchored()
            .position(cover.origin)
            .child(
                env_scope::<SheetLayerFloorKey>(Self::LAYER_PRIORITY)
                    .child(div().relative().w(cover.size.width).h(cover.size.height).child(host)),
            )
            .into_any_element();
        div().size(px(0.0)).child(deferred(layer).with_priority(Self::LAYER_PRIORITY)).into_any_element()
    }
}

/// the window top layer, root view draws host above every sheet and observes this entity
pub struct MediaViewerModalPresenter {
    hosting_state: Option<Entity<MediaViewerPresentationState>>,
}

impl MediaViewerModalPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { hosting_state: None })
    }

    pub fn sync(&mut self, state: Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) {
        let wants_host = {
            let snapshot = state.read(cx);
            snapshot.presentation_route() == MediaViewerPresentationRoute::Modal && !snapshot.items().is_empty()
        };
        if wants_host {
            self.present_if_needed(state, cx);
        } else {
            self.dismiss_host(cx);
        }
    }

    pub fn is_hosting(&self) -> bool {
        self.hosting_state.is_some()
    }

    /// layer the root view draws over everything while a modal presentation is up
    pub fn host(&self) -> Option<MediaViewerHost> {
        self.hosting_state
            .clone()
            .map(|state| MediaViewerHost::new(state, MediaViewerPresentationRoute::Modal))
    }

    fn present_if_needed(&mut self, state: Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) {
        if self.hosting_state.is_some() {
            return;
        }
        self.hosting_state = Some(state);
        cx.notify();
    }

    fn dismiss_host(&mut self, cx: &mut Context<Self>) {
        if self.hosting_state.take().is_some() {
            cx.notify();
        }
    }
}

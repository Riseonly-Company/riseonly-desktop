use gpui::{
    BoxShadow, Context, Entity, FocusHandle, IntoElement, Render, Subscription, Window, black, div,
    point, prelude::*, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::overlay::blocking_loader_store::BlockingLoaderStore;

pub struct BlockingLoaderHost {
    store: Entity<BlockingLoaderStore>,
    dimming: Presence<()>,
    loader: Presence<()>,
    focus_handle: FocusHandle,
    focus_before: Option<FocusHandle>,
    _store_observation: Subscription,
}

impl BlockingLoaderHost {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = BlockingLoaderStore::shared(cx);
        let shown = store.read(cx).is_visible().then_some(());
        let observation = cx.observe_in(&store, window, |host, store, window, cx| {
            let is_visible = store.read(cx).is_visible();
            host.follow_store(is_visible, window, cx);
            cx.notify();
        });
        Self {
            store,
            dimming: Presence::new(shown).with_transition(Transition::opacity()),
            loader: Presence::new(shown)
                .with_transition(Transition::opacity().combined(Transition::scale(0.9))),
            focus_handle: cx.focus_handle(),
            focus_before: None,
            _store_observation: observation,
        }
    }

    fn follow_store(&mut self, is_visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if is_visible {
            if !self.focus_handle.is_focused(window) {
                self.focus_before = window.focused(cx);
                window.focus(&self.focus_handle, cx);
            }
        } else if let Some(previous) = self.focus_before.take() {
            if self.focus_handle.is_focused(window) {
                window.focus(&previous, cx);
            }
        }
    }
}

impl Render for BlockingLoaderHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_visible = self.store.read(cx).is_visible();
        let now = RiseClock::monotonic();
        let animation = spring(0.34, 0.86);
        self.dimming.set_in(is_visible.then_some(()), animation, now, cx);
        self.loader.set_in(is_visible.then_some(()), animation, now, cx);

        let store = self.store.clone();
        let dimming = self.dimming.render_transitioned(now, window, |_| {
            div()
                .id("blocking-loader-dimming")
                .absolute()
                .inset_0()
                .bg(black().opacity(0.32))
                .when(is_visible, |dimming| {
                    dimming.occlude().on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| store.cancel(cx));
                    })
                })
        });

        let loader = self.loader.render_transitioned(now, window, |_| {
            let card = div()
                .size(px(78.))
                .flex()
                .items_center()
                .justify_center()
                .child(LoaderUi::new().size(LoaderUiSize::Large))
                .liquid_chrome_rounded_rectangle(22.);
            div()
                .id("blocking-loader-card")
                .size(px(78.))
                .rounded(px(22.))
                .shadow(vec![BoxShadow {
                    color: black().opacity(0.3),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(22.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .when(is_visible, |card| card.occlude())
                .child(card)
        });

        div()
            .size_full()
            .relative()
            .when(is_visible, |host| {
                host.track_focus(&self.focus_handle)
                    .on_key_down(|_, _, cx| cx.stop_propagation())
            })
            .children(dimming)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(loader),
            )
    }
}

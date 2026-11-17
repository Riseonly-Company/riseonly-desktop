use gpui::{
    AnyView, App, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
};

use super::theme_store::ThemeStore;

pub enum ThemeStoreKey {}

impl ThemeStoreKey {
    pub fn default_value(cx: &mut App) -> Entity<ThemeStore> {
        ThemeStore::shared(cx)
    }
}

pub fn theme_store(cx: &mut App) -> Entity<ThemeStore> {
    ThemeStoreKey::default_value(cx)
}

/// root of a window tree
/// follows the window system appearance and its activation, redraws the tree when the theme commits
pub struct ThemeRootBoundary {
    content: AnyView,
    _subscriptions: Vec<Subscription>,
}

impl ThemeRootBoundary {
    pub fn new(content: AnyView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self { content, _subscriptions: Self::observe_window(window, cx) }
    }

    /// what the boundary does for a window, for a window root that cant sit under another view
    pub fn observe_window<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Vec<Subscription> {
        let store = theme_store(cx);
        let appearance = window.appearance();
        store.update(cx, |store, cx| {
            store.prepare_palette_seed(cx);
            let follows_system = store.preferred_color_scheme().is_none();
            store.release_stale_override(follows_system, cx);
            store.system_color_scheme_did_change(appearance, cx);
        });

        let store_for_appearance = store.clone();
        let store_for_activation = store.clone();
        vec![
            cx.observe_in(&store, window, |_, store, _window, cx| {
                let appearance = cx.window_appearance();
                store.update(cx, |store, cx| {
                    let follows_system = store.preferred_color_scheme().is_none();
                    store.release_stale_override(follows_system, cx);
                    store.system_interface_style_did_change(appearance, cx);
                });
                cx.notify();
            }),
            cx.observe_window_appearance(window, move |_, _window, cx| {
                let appearance = cx.window_appearance();
                store_for_appearance.update(cx, |store, cx| store.system_color_scheme_did_change(appearance, cx));
            }),
            cx.observe_window_activation(window, move |_, window, cx| {
                if !window.is_window_active() {
                    return;
                }
                store_for_activation.update(cx, |store, cx| store.refresh_system_appearance(cx));
            }),
        ]
    }
}

impl Render for ThemeRootBoundary {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.content.clone())
    }
}

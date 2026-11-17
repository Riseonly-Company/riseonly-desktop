use std::time::Duration;

use gpui::{AnyWindowHandle, App, ElementId, Global, WeakEntity, Window, WindowId};
use rise_core::foundation::foundation_date::RiseClock;

use crate::native_confirm_alert::NativeConfirmAlert;
use crate::swiftui::environment::EnvKey;

pub enum TopPresenter {}

impl TopPresenter {
    /// active window, else the first open one
    /// none when no window is open
    pub fn current(cx: &App) -> Option<AnyWindowHandle> {
        cx.active_window().or_else(|| cx.windows().into_iter().next())
    }

    pub fn has_presented(cx: &App) -> bool {
        let Some(key_window) = cx.active_window() else {
            return false;
        };
        if NativeConfirmAlert::is_presenting(cx) {
            return true;
        }
        let window_id = key_window.window_id();
        cx.try_global::<PresentedLayers>().is_some_and(|layers| {
            layers
                .leases
                .iter()
                .any(|(owner, lease)| *owner == window_id && lease.upgrade().is_some())
        })
    }

    pub fn has_presented_in(window_id: WindowId, cx: &App) -> bool {
        cx.try_global::<PresentedLayers>().is_some_and(|layers| {
            layers
                .leases
                .iter()
                .any(|(owner, lease)| *owner == window_id && lease.upgrade().is_some())
        })
    }

    /// marks a modal layer as presented in this window while the caller keeps drawing it
    /// a sheet or cover host calls it every frame under an id unique to that host
    pub fn note_presented(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) {
        let window_id = window.window_handle().window_id();
        window.use_keyed_state(id, cx, move |_, cx| {
            let lease = cx.weak_entity();
            let layers = cx.default_global::<PresentedLayers>();
            layers.leases.retain(|(_, held)| held.upgrade().is_some());
            layers.leases.push((window_id, lease));
            PresentationLease { presented_at: RiseClock::monotonic() }
        });
    }

    /// when the latest modal layer still drawn in this window was presented, none with none up
    pub fn top_presented_at(window: &Window, cx: &App) -> Option<Duration> {
        let window_id = window.window_handle().window_id();
        cx.try_global::<PresentedLayers>()?
            .leases
            .iter()
            .filter(|(owner, _)| *owner == window_id)
            .filter_map(|(_, lease)| lease.upgrade())
            .map(|lease| lease.read(cx).presented_at)
            .max()
    }
}

/// true inside the content of a modal layer, drawn by a presented layer and not under one
pub struct InsidePresentedLayerKey;

impl EnvKey for InsidePresentedLayerKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// element state of one presented layer, alive while its host draws it
struct PresentationLease {
    presented_at: Duration,
}

#[derive(Default)]
struct PresentedLayers {
    leases: Vec<(WindowId, WeakEntity<PresentationLease>)>,
}

impl Global for PresentedLayers {}

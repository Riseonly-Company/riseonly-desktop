use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Global};
use rise_energy::rise_energy_saving::RiseEnergySavingStore;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, UIApplication};

pub struct RiseEnergySavingActivation;

impl Global for RiseEnergySavingActivation {}

impl RiseEnergySavingActivation {
    pub fn install(cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        cx.set_global(Self);

        let (signals, mut inbox) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = signals.unbounded_send(());
            })
            .detach();

        cx.spawn(async move |cx| {
            while inbox.next().await.is_some() {
                cx.update(|cx| {
                    let store = RiseEnergySavingStore::shared(cx);
                    store.update(cx, |store, cx| store.refresh_device_state(cx));
                });
            }
        })
        .detach();
    }
}

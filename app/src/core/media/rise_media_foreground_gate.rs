use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Task};

use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication,
};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};

pub const RISE_MEDIA_FOREGROUND_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("rise.media.foreground.did.change");

/// is the app on screen right now, for playback that must stay silent in background
/// music and calls keep sounding with every window hidden and dont ask this gate
/// everything else asks it
pub struct RiseMediaForegroundGate {
    is_foreground: bool,
    _observers: Vec<RiseSubscription>,
    _signals: Task<()>,
}

impl RiseMediaForegroundGate {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let is_foreground = RiseAppLifecycle::scene_phase(cx) != ScenePhase::Background;
        tracing::warn!(target: "riseonly::X1291", is_foreground, phase = ?RiseAppLifecycle::scene_phase(cx), "foreground gate new");
        let (sender, mut inbox) = mpsc::unbounded::<bool>();
        let center = RiseAppEvents::default_center();
        let observe = |name: RiseNotificationName, is_foreground: bool| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(is_foreground);
            })
        };
        let observers = vec![
            observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, false),
            observe(UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION, true),
        ];
        let signals = cx.spawn(async move |this, cx| {
            while let Some(is_foreground) = inbox.next().await {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    this.apply(is_foreground, cx);
                });
                if updated.is_err() {
                    return;
                }
            }
        });
        Self {
            is_foreground,
            _observers: observers,
            _signals: signals,
        }
    }

    pub fn is_foreground(&self) -> bool {
        self.is_foreground
    }

    fn apply(&mut self, is_foreground: bool, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X1291", was = self.is_foreground, is_foreground, "foreground gate signal");
        if self.is_foreground == is_foreground {
            return;
        }
        self.is_foreground = is_foreground;
        cx.notify();
        RiseAppEvents::default_center().post(RISE_MEDIA_FOREGROUND_DID_CHANGE);
    }
}

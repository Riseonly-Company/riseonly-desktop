use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Global};
use rise_media::audio_route::{AudioRouteChange, AudioRouteMonitor};

use crate::core::desktop_platform::av_audio_session::AVAudioSession;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName};

pub use rise_media::audio_route::AudioRouteChangeReason as AVAudioSessionRouteChangeReason;

impl AVAudioSession {
    pub const ROUTE_CHANGE_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("AVAudioSessionRouteChangeNotification");
}

/// payload of the route change notification, carries the reason key
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AVAudioSessionRouteChange {
    pub reason: AVAudioSessionRouteChangeReason,
}

impl AVAudioSessionRouteChange {
    pub const fn reason_raw_value(&self) -> u64 {
        self.reason.raw_value()
    }
}

/// owns the route monitor for the life of the process, installed once by the shell at startup
pub struct AVAudioSessionRouteBridge {
    _monitor: Option<AudioRouteMonitor>,
}

impl Global for AVAudioSessionRouteBridge {}

impl AVAudioSessionRouteBridge {
    pub fn install(cx: &mut App) {
        Self::install_with(RiseAppEvents::default_center().clone(), cx);
    }

    pub fn install_with(events: RiseAppEvents, cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        let (changes, mut inbox) = mpsc::unbounded::<AudioRouteChange>();
        let monitor = match AudioRouteMonitor::start(move |change| {
            let _ = changes.unbounded_send(change);
        }) {
            Ok(monitor) => Some(monitor),
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "audio route changes are not watched");
                None
            }
        };
        cx.set_global(Self { _monitor: monitor });

        cx.spawn(async move |cx| {
            while let Some(change) = inbox.next().await {
                let events = events.clone();
                cx.update(move |_| {
                    events.post_with(
                        AVAudioSession::ROUTE_CHANGE_NOTIFICATION,
                        AVAudioSessionRouteChange {
                            reason: change.reason,
                        },
                    );
                });
            }
        })
        .detach();
    }
}

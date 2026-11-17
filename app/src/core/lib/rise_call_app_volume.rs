use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Entity, Global, Task, WeakEntity};
use rise_media::video::av_player::AVPlayer;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::CALL_APP_VOLUME;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription};
use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;

/// the apps own sound level while a call or voice chat owns the audio
/// voice messages and circles and music and videos are multiplied by gain and the call isnt
/// outside a call its 1
pub enum RiseCallAppVolume {}

/// video players that follow the gain, held weakly
struct RiseCallAppVolumeFollowers {
    players: Vec<WeakEntity<AVPlayer>>,
    _subscription: RiseSubscription,
    _delivery: Task<()>,
}

impl Global for RiseCallAppVolumeFollowers {}

impl RiseCallAppVolume {
    pub const DID_CHANGE_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("riseonly.callAudio.appVolumeDidChange");

    pub fn level() -> f32 {
        let Some(stored) = UserDefaults::standard().object(CALL_APP_VOLUME).and_then(|value| value.as_f64()) else {
            return 1.0;
        };
        (stored as f32).clamp(0.0, 1.0)
    }

    pub fn gain() -> f32 {
        if RiseCallAudioOwnership::is_call_active() { Self::level() } else { 1.0 }
    }

    pub fn set_level(value: f32) {
        let clamped = value.clamp(0.0, 1.0);
        if (clamped - Self::level()).abs() <= 0.001 {
            return;
        }
        UserDefaults::standard().set(CALL_APP_VOLUME, clamped);
        Self::post_change();
    }

    /// video players have no fades of their own so their volume is the gain
    /// kept in step with the slider and call start or end
    pub fn follow(player: &Entity<AVPlayer>, cx: &mut App) {
        if !cx.has_global::<RiseCallAppVolumeFollowers>() {
            let followers = Self::followers(cx);
            cx.set_global(followers);
        }
        let followers = cx.global_mut::<RiseCallAppVolumeFollowers>();
        followers.players.retain(|weak| weak.upgrade().is_some());
        followers.players.push(player.downgrade());
        let gain = Self::gain();
        player.update(cx, |player, cx| player.set_volume(gain, cx));
    }

    /// any thread, followers take the new gain on the main thread
    pub fn post_change() {
        RiseAppEvents::default_center().post(Self::DID_CHANGE_NOTIFICATION);
    }

    fn followers(cx: &mut App) -> RiseCallAppVolumeFollowers {
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let subscription = RiseAppEvents::default_center().observe(Self::DID_CHANGE_NOTIFICATION, move |_| {
            let _ = sender.unbounded_send(());
        });
        let delivery = cx.spawn(async move |cx| {
            while receiver.next().await.is_some() {
                cx.update(|cx| {
                    let gain = Self::gain();
                    let players: Vec<WeakEntity<AVPlayer>> = cx
                        .try_global::<RiseCallAppVolumeFollowers>()
                        .map(|followers| followers.players.clone())
                        .unwrap_or_default();
                    for player in players {
                        let _ = player.update(cx, |player, cx| player.set_volume(gain, cx));
                    }
                });
            }
        });
        RiseCallAppVolumeFollowers { players: Vec::new(), _subscription: subscription, _delivery: delivery }
    }
}

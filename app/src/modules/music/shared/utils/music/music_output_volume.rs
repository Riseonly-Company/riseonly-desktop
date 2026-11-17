use gpui::{App, Global};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::DESKTOP_MUSIC_VOLUME;

use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

pub enum MusicOutputVolume {}

/// volume before a mute, restored on the next speaker press
struct MusicOutputVolumeBeforeMute(f32);

impl Global for MusicOutputVolumeBeforeMute {}

impl MusicOutputVolume {
    const MUTED_BELOW: f32 = 0.005;

    pub fn level() -> f32 {
        let Some(stored) = UserDefaults::standard().object(DESKTOP_MUSIC_VOLUME).and_then(|value| value.as_f64()) else {
            return 1.0;
        };
        (stored as f32).clamp(0.0, 1.0)
    }

    /// level the music engines play at, bar level times the call share
    pub fn gain() -> f32 {
        RiseCallAppVolume::gain() * Self::level()
    }

    pub fn is_muted() -> bool {
        Self::level() < Self::MUTED_BELOW
    }

    pub fn set_level(value: f32, cx: &mut App) {
        let clamped = value.clamp(0.0, 1.0);
        if (clamped - Self::level()).abs() <= 0.001 {
            return;
        }
        UserDefaults::standard().set(DESKTOP_MUSIC_VOLUME, clamped);
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.apply_call_app_volume(cx));
    }

    pub fn toggle_mute(cx: &mut App) {
        if Self::is_muted() {
            let restored = cx.try_global::<MusicOutputVolumeBeforeMute>().map_or(1.0, |before| before.0);
            Self::set_level(restored.max(Self::MUTED_BELOW * 2.0), cx);
        } else {
            cx.set_global(MusicOutputVolumeBeforeMute(Self::level()));
            Self::set_level(0.0, cx);
        }
    }
}

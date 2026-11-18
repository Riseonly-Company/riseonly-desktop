use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, Global};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::DESKTOP_MUSIC_VOLUME;

use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

pub enum MusicOutputVolume {}

/// volume before a mute, restored on the next speaker press
struct MusicOutputVolumeBeforeMute(f32);

impl Global for MusicOutputVolumeBeforeMute {}

/// bumped on every level change so each slider bound to the level redraws
#[derive(Default)]
pub struct MusicOutputVolumeRevision(pub u64);

impl Global for MusicOutputVolumeRevision {}

/// sounds that play over the music right now, a reel or a story with its sound on
#[derive(Default)]
struct MusicOutputVolumeDuckers(HashSet<String>);

impl Global for MusicOutputVolumeDuckers {}

static DUCKED: AtomicBool = AtomicBool::new(false);

impl MusicOutputVolume {
    const MUTED_BELOW: f32 = 0.005;
    const DUCKED_SHARE: f32 = 0.25;

    pub fn level() -> f32 {
        let Some(stored) = UserDefaults::standard().object(DESKTOP_MUSIC_VOLUME).and_then(|value| value.as_f64()) else {
            return 1.0;
        };
        (stored as f32).clamp(0.0, 1.0)
    }

    /// level the music engines play at, only the music slider moves it
    /// cubic taper so the low end of the slider is really quiet
    pub fn gain() -> f32 {
        let gain = Self::level().powi(3);
        if DUCKED.load(Ordering::Relaxed) { gain * Self::DUCKED_SHARE } else { gain }
    }

    /// a reel or story with sound plays over the music, the music steps back but keeps playing
    /// so a shared listening session goes on and nothing has to ask first
    pub fn duck(owner: &str, active: bool, cx: &mut App) {
        let mut duckers = cx.try_global::<MusicOutputVolumeDuckers>().map(|duckers| duckers.0.clone()).unwrap_or_default();
        let changed = if active { duckers.insert(owner.to_owned()) } else { duckers.remove(owner) };
        if !changed {
            return;
        }
        DUCKED.store(!duckers.is_empty(), Ordering::Relaxed);
        cx.set_global(MusicOutputVolumeDuckers(duckers));
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.apply_call_app_volume(cx));
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
        let revision = cx.try_global::<MusicOutputVolumeRevision>().map_or(0, |revision| revision.0);
        cx.set_global(MusicOutputVolumeRevision(revision.wrapping_add(1)));
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

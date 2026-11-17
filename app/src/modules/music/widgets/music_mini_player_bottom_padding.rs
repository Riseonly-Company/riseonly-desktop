use gpui::{AnyElement, App, Context, ElementId, Entity, IntoElement, Subscription, Window, div, px};
use rise_ui::swiftui::modifiers::{ModifiedContent, ModifierExt, ViewModifier};
use rise_ui::swiftui::prelude::*;

use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

struct MusicPlayerObservation {
    is_covered: bool,
    _subscription: Subscription,
}

/// true over content the full player covers when open, like the right panel
/// skips player publications meanwhile
/// redraws once on close
pub struct MusicPlayerHiddenUnderFullPlayerKey;

impl EnvKey for MusicPlayerHiddenUnderFullPlayerKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// whether content here is hidden by the open full player right now
pub(crate) fn is_hidden_under_full_player(player: &MusicPlayerState, hides_under_full_player: bool) -> bool {
    hides_under_full_player && player.show_now_playing()
}

/// observe the player for a centre page
/// full player covers the window so the page skips publications
/// redraws once when it closes
pub(crate) fn observe_player_unless_covered<T: 'static>(
    player: &Entity<MusicPlayerState>,
    cx: &mut Context<T>,
) -> Subscription {
    let mut was_covered = player.read(cx).show_now_playing();
    cx.observe(player, move |_, player, cx| {
        let is_covered = player.read(cx).show_now_playing();
        if is_covered && was_covered {
            return;
        }
        was_covered = is_covered;
        cx.notify();
    })
}

/// a render under the open full player that reads it but hears it through a filtered observation
/// so a publication doesnt redraw its cached view
pub(crate) fn hear_player_through_observation_while_covered(player: &Entity<MusicPlayerState>, cx: &App) {
    let hides_under_full_player = env::<MusicPlayerHiddenUnderFullPlayerKey>(cx);
    if is_hidden_under_full_player(player.read(cx), hides_under_full_player) {
        gpui::hear_entity_through_events_only(player.entity_id());
    }
}

/// shared player read by a stateless view
/// observation lives in element state and redraws the view that draws id
pub(crate) fn observed_music_player(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<MusicPlayerState> {
    let player = MusicPlayerState::shared(cx);
    let observed = player.clone();
    let hides_under_full_player = env::<MusicPlayerHiddenUnderFullPlayerKey>(cx);
    let is_covered = is_hidden_under_full_player(player.read(cx), hides_under_full_player);
    if is_covered {
        gpui::hear_entity_through_events_only(player.entity_id());
    }
    let _observation = window.use_keyed_state(id, cx, move |_, cx| MusicPlayerObservation {
        is_covered,
        _subscription: cx.observe(&observed, move |observation: &mut MusicPlayerObservation, player, cx| {
            let is_covered = is_hidden_under_full_player(player.read(cx), hides_under_full_player);
            if is_covered && observation.is_covered {
                return;
            }
            observation.is_covered = is_covered;
            cx.notify();
        }),
    });
    player
}

pub struct MusicMiniPlayerBottomPaddingModifier {
    base: f32,
    tab_underlap: f32,
}

impl ViewModifier for MusicMiniPlayerBottomPaddingModifier {
    fn body(self, content: AnyElement, _window: &mut Window, _cx: &mut App) -> AnyElement {
        div().w_full().pb(px(self.base + self.tab_underlap)).child(content).into_any_element()
    }
}

pub trait MusicMiniPlayerBottomPaddingModifiers: IntoElement + Sized {
    /// bottom padding for the mini player, tab_underlap is 0 outside a root tab
    fn music_mini_player_bottom_padding(
        self,
        base: f32,
        tab_underlap: f32,
    ) -> ModifiedContent<MusicMiniPlayerBottomPaddingModifier> {
        self.modifier(MusicMiniPlayerBottomPaddingModifier { base, tab_underlap })
    }
}

impl<E: IntoElement> MusicMiniPlayerBottomPaddingModifiers for E {}

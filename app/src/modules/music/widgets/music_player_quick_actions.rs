use gpui::{
    App, Div, ElementId, FontWeight, Hsla, IntoElement, RenderOnce, SharedString, Stateful, Window, div, px, white,
};
use rise_i18n::tr;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use super::music_mini_player_bottom_padding::observed_music_player;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

#[derive(IntoElement)]
pub struct MusicPlayerQuickActions {
    id: ElementId,
    on_dark_background: bool,
}

impl MusicPlayerQuickActions {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), on_dark_background: false }
    }

    pub fn on_dark_background(mut self, on_dark_background: bool) -> Self {
        self.on_dark_background = on_dark_background;
        self
    }

    #[allow(clippy::too_many_arguments)]
    fn action(
        &self,
        id: &'static str,
        icon: &'static str,
        title: String,
        accessibility_title: String,
        active: bool,
        on_press: impl Fn(&mut Window, &mut App) + 'static,
        cx: &App,
    ) -> Stateful<Div> {
        let theme = rise_ui::theme(cx);
        let foreground: Hsla = if active {
            theme.primary._100
        } else if self.on_dark_background {
            white().opacity(0.7)
        } else {
            theme.text.secondary
        };
        let background = if active { theme.primary._100.opacity(0.12) } else { foreground.opacity(0.08) };
        let hint = SharedString::from(accessibility_title);
        let tooltip_label = hint.clone();

        div()
            .id(id)
            .aria_label(hint)
            .aria_selected(active)
            .tooltip_label(tooltip_label)
            .child(
                ButtonUi::new(SharedString::from(format!("{id}-button")))
                    .background_color(background)
                    .height(44.0)
                    .padding_horizontal(10.0)
                    .on_press(on_press)
                    .child(
                        hstack(7.0)
                            .children(IconUi::sized(icon, px(16.0), foreground, false))
                            .child(
                                MainText::new(title)
                                    .px(12.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(foreground)
                                    .line_limit(1)
                                    .minimum_scale_factor(0.85),
                            ),
                    ),
            )
    }
}

impl RenderOnce for MusicPlayerQuickActions {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = observed_music_player(self.id.clone(), window, cx);
        if !player.read(cx).is_music_playback() {
            return div().into_any_element();
        }
        let has_active_audio_effect = player.read(cx).has_active_audio_effect();
        let is_listening_together = player.read(cx).is_listening_together();
        let can_show_playlist_listeners = player.update(cx, |player, cx| player.can_show_playlist_listeners(cx));

        hstack(8.0)
            .child(self.action(
                "music-quick-action-effects",
                if has_active_audio_effect { "waveform.path.ecg" } else { "waveform" },
                tr("music_effects_short"),
                tr("music_effects_title"),
                has_active_audio_effect,
                |_, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.open_audio_effects(cx));
                },
                cx,
            ))
            .when(can_show_playlist_listeners, |row| {
                row.child(self.action(
                    "music-quick-action-listeners",
                    "shareplay",
                    tr("music_live_listeners_short"),
                    tr("music_live_listeners_title"),
                    is_listening_together,
                    |_, cx| {
                        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.open_playlist_listeners(cx));
                    },
                    cx,
                ))
            })
            .into_any_element()
    }
}

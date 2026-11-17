use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, Entity, SharedString, Window, white};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::modifiers::ModifierExt;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::modules::music::pages::now_playing::now_playing::MusicListeningControlAlert;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::MusicAudioEffectKind;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_audio_effect_section::MusicAudioEffectSection;
use crate::modules::music::widgets::music_mini_player_bottom_padding::observed_music_player;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

#[derive(IntoElement)]
pub struct MusicAudioEffectsSheet {
    id: ElementId,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl MusicAudioEffectsSheet {
    pub fn new(id: impl Into<ElementId>, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), on_close: Rc::new(on_close) }
    }

    /// effects are session owned, everyone hears the same chain and only granted members change it
    /// ad-hoc mp3 over the session is the exception, its effects are local
    fn is_follower_without_control(player: &MusicPlayerState) -> bool {
        let Some(session) = player.listening_session() else {
            return false;
        };
        if !player.is_listening_session_follower() || player.is_listening_playback_displaced_by_ad_hoc() {
            return false;
        }
        !session.viewer_permissions.can_apply_effects
    }

    /// one footnote under the first row instead of twelve identical ones
    fn footnote(kind: MusicAudioEffectKind, player: &MusicPlayerState) -> Option<String> {
        if kind != MusicAudioEffectKind::ALL[0] {
            return None;
        }
        if Self::is_follower_without_control(player) {
            return Some(tr("music_effects_permission_hint"));
        }
        player.listening_session().map(|_| tr("music_effects_session_hint"))
    }

    fn section(
        kind: MusicAudioEffectKind,
        section_player: &Entity<MusicPlayerState>,
        cx: &App,
    ) -> MusicAudioEffectSection {
        let player = section_player.read(cx);
        let settings = player.audio_effect_settings();
        let guide_anchor = (kind == MusicAudioEffectKind::ALL[0]).then_some(RiseGuideAnchor::MusicEffectsFirstEffect);
        MusicAudioEffectSection::new(
            kind,
            settings.is_enabled(kind),
            settings.value(kind),
            Self::is_follower_without_control(player),
            move |enabled, _, cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.set_audio_effect(kind, enabled, cx));
            },
            move |value, _, cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.set_audio_effect_value(kind, value, cx));
            },
        )
        .footnote(Self::footnote(kind, player))
        .guide_anchor(guide_anchor)
        .on_locked_tap(|_, cx| {
            MusicInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.report_blocked_audio_effect_control(cx));
        })
    }
}

impl RenderOnce for MusicAudioEffectsSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = observed_music_player(child_id(&self.id, "player"), window, cx);
        let theme = rise_ui::theme(cx).clone();
        let (can_restore, shows_comparison) = {
            let player = player.read(cx);
            let is_any_enabled = player.audio_effect_settings().is_any_enabled();
            let can_restore = !is_any_enabled && player.audio_effect_settings_before_reset().is_some();
            (can_restore, !Self::is_follower_without_control(player) && (is_any_enabled || can_restore))
        };

        let on_left = self.on_close.clone();
        let on_right = self.on_close.clone();
        let sections = MusicAudioEffectKind::ALL.into_iter().map(|kind| Self::section(kind, &player, cx)).collect::<Vec<_>>();

        let comparison = shows_comparison.then(|| {
            let tint = if can_restore { theme.primary._100 } else { theme.button.bg_300.opacity(0.35) };
            let text_color = if can_restore { white() } else { theme.text.primary };
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .bg(theme.bg._100)
                .pt(px(12.))
                .px(px(16.))
                .pb(px(16.))
                .child(
                LiquidGlassButton::new("musicEffectsComparison", LiquidGlassButtonShape::Capsule)
                    .horizontal_padding(0.)
                    .vertical_padding(0.)
                    .tint(Some(tint))
                    .on_press(|_, cx| {
                        MusicInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.reset_or_restore_audio_effects(cx));
                    })
                    .child(
                        div().w_full().h(px(44.)).flex().items_center().justify_center().child(
                            MainText::new(tr(if can_restore { "music_effects_restore" } else { "music_effects_reset" }))
                                .px(16.)
                                .color(text_color),
                        ),
                    ),
            )
        });

        vstack(0.)
            .size_full()
            .bg(theme.bg._100)
            .child(
                SheetHeaderBar::new(child_id(&self.id, "header"), tr("music_effects_title"), move |window, cx| {
                    on_left(window, cx);
                })
                .left_system_image("xmark")
                .on_right(move |window, cx| on_right(window, cx))
                .left_guide_anchor(Some(RiseGuideAnchor::MusicEffectsClose)),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(
                        scroll_view(child_id(&self.id, "scroll")).size_full().shows_indicators(false).child(
                            vstack(16.).w_full().pt(px(8.)).children(sections).child(spacer_min_h(88.)),
                        ),
                    )
                    .children(comparison),
            )
            .modifier(MusicListeningControlAlert::new(child_id(&self.id, "control-alert")))
    }
}

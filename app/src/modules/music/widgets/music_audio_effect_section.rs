use std::rc::Rc;

use gpui::{App, IntoElement, RenderOnce, SharedString, Window, div, px};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_music::{PERCENT_SPAN, SEMITONE_SPAN, ValueLabel};
use rise_theme::color;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt, MusicEffectCanon,
};

pub trait MusicAudioEffectKindPresentation: Sized {
    fn title(self) -> String;
    fn hint(self) -> String;
    fn system_image(self) -> &'static str;
    fn accent_hex(self) -> &'static str;
    fn slider_title(self) -> String;
    fn minimum_caption(self) -> String;
    fn maximum_caption(self) -> String;
    fn slider_breakpoints(self) -> Vec<f64>;
    fn value_label(self, value: f64) -> String;
}

impl MusicAudioEffectKindPresentation for MusicAudioEffectKind {
    fn title(self) -> String {
        tr(self.canon_spec().i18n_key)
    }

    fn hint(self) -> String {
        tr(self.canon_spec().hint_i18n_key)
    }

    fn system_image(self) -> &'static str {
        match self {
            Self::Slowed => "waveform.path.ecg",
            Self::Nightcore => "hare",
            Self::Pitch => "tuningfork",
            Self::BassBoost => "speaker.wave.3",
            Self::Distance => "door.left.hand.closed",
            Self::Underwater => "water.waves",
            Self::Concert => "building.columns",
            Self::Echo => "wave.3.right",
            Self::NightMode => "moon.stars",
            Self::Spin => "arrow.triangle.2.circlepath",
            Self::Vinyl => "record.circle",
        }
    }

    fn accent_hex(self) -> &'static str {
        self.canon_spec().accent_hex
    }

    fn slider_title(self) -> String {
        tr(self.canon_spec().slider_i18n_key)
    }

    fn minimum_caption(self) -> String {
        tr(self.canon_spec().min_caption_i18n_key)
    }

    fn maximum_caption(self) -> String {
        tr(self.canon_spec().max_caption_i18n_key)
    }

    fn slider_breakpoints(self) -> Vec<f64> {
        self.canon_scale()
            .breakpoints_milli
            .iter()
            .map(|milli| MusicEffectCanon::unit(*milli))
            .collect()
    }

    fn value_label(self, value: f64) -> String {
        match self.canon_spec().value_label {
            ValueLabel::Multiplier => format!("{value:.2}x"),
            ValueLabel::Semitones => {
                let semitones = ((value - self.neutral_value()) * SEMITONE_SPAN as f64).round() as i64;
                if semitones > 0 { format!("+{semitones}") } else { semitones.to_string() }
            }
            ValueLabel::Percent => format!("{}%", (value * PERCENT_SPAN as f64).round() as i64),
        }
    }
}

pub type MusicAudioEffectToggle = Rc<dyn Fn(bool, &mut Window, &mut App)>;
pub type MusicAudioEffectValue = Rc<dyn Fn(f64, &mut Window, &mut App)>;
pub type MusicAudioEffectLockedTap = Rc<dyn Fn(&mut Window, &mut App)>;

/// one effect in the list
/// a switch row plus its strength slider only while the effect is on
#[derive(IntoElement)]
pub struct MusicAudioEffectSection {
    kind: MusicAudioEffectKind,
    is_enabled: bool,
    value: f64,
    is_locked: bool,
    footnote: Option<String>,
    guide_anchor: Option<RiseGuideAnchor>,
    on_toggle: MusicAudioEffectToggle,
    on_value: MusicAudioEffectValue,
    on_locked_tap: Option<MusicAudioEffectLockedTap>,
}

impl MusicAudioEffectSection {
    pub fn new(
        kind: MusicAudioEffectKind,
        is_enabled: bool,
        value: f64,
        is_locked: bool,
        on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
        on_value: impl Fn(f64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            kind,
            is_enabled,
            value,
            is_locked,
            footnote: None,
            guide_anchor: None,
            on_toggle: Rc::new(on_toggle),
            on_value: Rc::new(on_value),
            on_locked_tap: None,
        }
    }

    pub fn footnote(mut self, footnote: impl Into<Option<String>>) -> Self {
        self.footnote = footnote.into();
        self
    }

    pub fn guide_anchor(mut self, guide_anchor: impl Into<Option<RiseGuideAnchor>>) -> Self {
        self.guide_anchor = guide_anchor.into();
        self
    }

    pub fn on_locked_tap(mut self, on_locked_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_locked_tap = Some(Rc::new(on_locked_tap));
        self
    }

    fn items(&self, cx: &App) -> Vec<GroupedBtn> {
        let kind = self.kind;
        let raw_value = kind.raw_value();
        let is_enabled = self.is_enabled;
        let is_locked = self.is_locked;
        let secondary = rise_ui::theme(cx).text.secondary;

        let binding_toggle = self.on_toggle.clone();
        let binding_locked_tap = self.on_locked_tap.clone();
        let value = Binding::new(
            move |_| is_enabled,
            move |next, window, cx| {
                if is_locked {
                    if let Some(on_locked_tap) = &binding_locked_tap {
                        on_locked_tap(window, cx);
                    }
                } else {
                    binding_toggle(next, window, cx);
                }
            },
        );

        let row_toggle = self.on_toggle.clone();
        let row_locked_tap = self.on_locked_tap.clone();
        let row = GroupedBtn::new(SharedString::from(format!("{raw_value}_toggle")), kind.title())
            .icon(kind.system_image())
            .icon_background(color(kind.accent_hex()))
            .subtitle(kind.hint())
            .guide_anchor(self.guide_anchor)
            .trailing_slot(
                hstack(8.0)
                    .when(is_locked, |slot| {
                        slot.children(IconUi::sized("lock.fill", px(13.0), secondary, true))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("{raw_value}_trailing")))
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .opacity(if is_locked { 0.45 } else { 1.0 })
                            .child(SwitchUi::new(SharedString::from(format!("{raw_value}_switch")), value)),
                    ),
            )
            .callback(move |window, cx| {
                if is_locked {
                    if let Some(on_locked_tap) = &row_locked_tap {
                        on_locked_tap(window, cx);
                    }
                    return;
                }
                row_toggle(!is_enabled, window, cx);
            });

        let mut rows = vec![row];
        if is_enabled {
            rows.push(
                GroupedBtn::new(SharedString::from(format!("{raw_value}_amount")), kind.slider_title())
                    .content_slot(self.slider_row(cx)),
            );
        }
        rows
    }

    fn slider_row(&self, cx: &App) -> impl IntoElement + use<> {
        let kind = self.kind;
        let theme = rise_ui::theme(cx);
        let value_color = if self.is_locked { theme.text.secondary } else { theme.primary._100 };
        let on_change = self.on_value.clone();
        let on_commit = self.on_value.clone();

        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                hstack(8.0)
                    .w_full()
                    .child(MainText::new(kind.slider_title()).px(15.0))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new(kind.value_label(self.value))
                            .px(14.0)
                            .color(value_color)
                            .monospaced_digits(true),
                    ),
            )
            .child(
                SteppedSliderUi::new(
                    SharedString::from(format!("{}_slider", kind.raw_value())),
                    self.value,
                    kind.value_range(),
                    kind.value_step(),
                    move |value, window, cx| on_change(value, window, cx),
                    move |value, window, cx| on_commit(value, window, cx),
                )
                .breakpoints(kind.slider_breakpoints())
                .is_enabled(!self.is_locked),
            )
            .child(
                hstack(None)
                    .w_full()
                    .child(SecondaryText::new(kind.minimum_caption()).px(11.0))
                    .child(spacer_min_w(8.0))
                    .child(SecondaryText::new(kind.maximum_caption()).px(11.0)),
            )
    }
}

impl RenderOnce for MusicAudioEffectSection {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("music-audio-effect-{}", self.kind.raw_value()));
        let items = self.items(cx);
        let mut group = GroupedBtns::new(id, items);
        if let Some(footnote) = self.footnote {
            group = group.end_group_title(footnote);
        }
        group
    }
}

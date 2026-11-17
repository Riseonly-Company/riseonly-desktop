use gpui::{App, FontWeight, SharedString, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::stacks::{hstack, spacer, vstack};
use rise_ui::swiftui::stepper::Stepper;
use rise_ui::switch_ui::SwitchUi;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatChannelKind, VoiceChannelConfig};

const UNLIMITED_VALUE: &str = "\u{221E}";
const AUDIO_BITRATE_STEP: i64 = 8;
const TILE_CORNER_RADIUS: f32 = 16.0;
const EDITOR_CORNER_RADIUS: f32 = 20.0;
const UNAVAILABLE_TILE_OPACITY: f32 = 0.4;

pub trait ChatChannelKindPresentation {
    fn localized_title(self) -> String;

    /// single source for channel kind glyphs, topic list and picker and voice badge
    /// voice keeps the speaker cone, a bare waveform reads as a voice message
    /// stage is the figure with waves, a different room
    fn glyph_name(self) -> &'static str;
}

impl ChatChannelKindPresentation for ChatChannelKind {
    fn localized_title(self) -> String {
        match self {
            ChatChannelKind::Text => tr("rtc_channel_text"),
            ChatChannelKind::Voice => tr("rtc_channel_voice"),
            ChatChannelKind::Stage => tr("rtc_channel_stage"),
        }
    }

    fn glyph_name(self) -> &'static str {
        match self {
            ChatChannelKind::Text => "number",
            ChatChannelKind::Voice => "speaker.wave.2.fill",
            ChatChannelKind::Stage => "person.wave.2.fill",
        }
    }
}

/// voice channel config in a shape steppers and toggles can bind to
/// normalization and bounds still belong to VoiceChannelConfig
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceChannelConfigDraft {
    pub user_limit: i64,
    pub speaker_limit: i64,
    pub audio_bitrate_kbps: i64,
    pub video_enabled: bool,
    pub screen_share_enabled: bool,
    pub default_muted: bool,
}

impl VoiceChannelConfigDraft {
    pub fn new(config: Option<VoiceChannelConfig>) -> Self {
        let resolved = config.or_else(|| VoiceChannelConfig::defaults(ChatChannelKind::Voice));
        Self {
            user_limit: resolved.as_ref().map_or(0, |config| config.user_limit),
            speaker_limit: resolved.as_ref().map_or(0, |config| config.speaker_limit),
            audio_bitrate_kbps: resolved
                .as_ref()
                .map_or_else(VoiceChannelConfig::minimum_audio_bitrate_kbps, |config| config.audio_bitrate_kbps),
            video_enabled: resolved.as_ref().is_none_or(|config| config.video_enabled),
            screen_share_enabled: resolved.as_ref().is_none_or(|config| config.screen_share_enabled),
            default_muted: resolved.as_ref().is_some_and(|config| config.default_muted),
        }
    }

    pub fn normalized(&self, kind: ChatChannelKind) -> Option<VoiceChannelConfig> {
        if !kind.is_rtc() {
            return None;
        }
        VoiceChannelConfig {
            user_limit: self.user_limit,
            speaker_limit: self.speaker_limit,
            audio_bitrate_kbps: self.audio_bitrate_kbps,
            video_enabled: self.video_enabled,
            screen_share_enabled: self.screen_share_enabled,
            default_muted: self.default_muted,
            region: String::new(),
        }
        .normalized(kind)
        .ok()
    }
}

/// tiles instead of a segmented picker, a single segment cant be disabled
/// so an unavailable kind had to be cut from the list
#[derive(IntoElement)]
pub struct ChannelKindPicker {
    selection: Binding<ChatChannelKind>,
    allowed: Vec<ChatChannelKind>,
}

impl ChannelKindPicker {
    pub fn new(selection: Binding<ChatChannelKind>, allowed: Vec<ChatChannelKind>) -> Self {
        Self { selection, allowed }
    }

    fn tile(&self, kind: ChatChannelKind, theme: &AppTheme, cx: &App) -> AnyElement {
        let is_selected = self.selection.get(cx) == kind;
        let is_enabled = self.allowed.contains(&kind);
        let tint = if is_selected { theme.primary._100 } else { theme.text.secondary };
        let selection = self.selection.clone();

        plain(SharedString::from(format!("channel-kind-tile-{}", kind.wire())))
            .disabled(!is_enabled)
            .on_press(move |window: &mut Window, cx: &mut App| {
                if !is_enabled || is_selected {
                    return;
                }
                let _ = selection_changed();
                selection.set(kind, window, cx);
            })
            .relative()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.0))
            .py(px(12.0))
            .continuous_corners(TILE_CORNER_RADIUS)
            .bg(if is_selected { theme.primary._100.opacity(0.14) } else { theme.bg._200 })
            .when(!is_enabled, |tile| tile.opacity(UNAVAILABLE_TILE_OPACITY))
            .children(IconUi::sized(kind.glyph_name(), px(17.0), tint, false))
            .child(
                MainText::new(kind.localized_title())
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .font_weight(if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                    .color(if is_selected { theme.text.primary } else { theme.text.secondary })
                    .number_of_lines(1)
                    .minimum_scale_factor(0.8),
            )
            .when(is_selected, |tile| {
                tile.child(centred_stroke(
                    theme.primary._100.opacity(0.55),
                    1.0,
                    Shape::continuous(TILE_CORNER_RADIUS),
                ))
            })
            .guide_anchor((kind == ChatChannelKind::Voice).then_some(RiseGuideAnchor::TopicEditorVoiceKind))
            .into_any_element()
    }
}

impl RenderOnce for ChannelKindPicker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let tiles: Vec<AnyElement> =
            ChatChannelKind::ALL.into_iter().map(|kind| self.tile(kind, &theme, cx)).collect();

        vstack(9.0)
            .w_full()
            .items_stretch()
            .px(px(16.0))
            .child(
                div().px(px(4.0)).child(
                    MainText::new(tr("topic_editor_kind_title")).px(13.0).color(theme.text.secondary),
                ),
            )
            .child(hstack(8.0).w_full().children(tiles))
    }
}

#[derive(IntoElement)]
pub struct VoiceChannelConfigEditor {
    draft: Binding<VoiceChannelConfigDraft>,
    kind: ChatChannelKind,
}

impl VoiceChannelConfigEditor {
    pub fn new(draft: Binding<VoiceChannelConfigDraft>, kind: ChatChannelKind) -> Self {
        Self { draft, kind }
    }

    fn field<T: Clone + 'static>(
        &self,
        get: impl Fn(&VoiceChannelConfigDraft) -> T + 'static,
        set: impl Fn(&mut VoiceChannelConfigDraft, T) + 'static,
    ) -> Binding<T> {
        let reader = self.draft.clone();
        let writer = self.draft.clone();
        Binding::new(
            move |cx: &App| get(&reader.get(cx)),
            move |value: T, window: &mut Window, cx: &mut App| {
                let mut next = writer.get(cx);
                set(&mut next, value);
                writer.set(next, window, cx);
            },
        )
    }

    /// same type as the stepper rows
    /// a string title took the system size and the system toggle kept it on one line
    /// so default muted in russian got cut mid word
    fn toggle(id: &'static str, title: String, is_on: Binding<bool>) -> Div {
        hstack(8.0)
            .w_full()
            .child(div().flex_1().min_w_0().child(MainText::new(title).px(15.0).number_of_lines(0)))
            .child(SwitchUi::new(id, is_on))
    }

    fn label(title: String, value: String, theme: &AppTheme) -> Div {
        hstack(None::<f32>)
            .flex_1()
            .min_w_0()
            .child(MainText::new(title).px(15.0))
            .child(spacer())
            .child(MainText::new(value).px(14.0).color(theme.text.secondary))
    }

    fn limit_value(limit: i64) -> String {
        if limit == 0 { UNLIMITED_VALUE.to_owned() } else { limit.to_string() }
    }

    fn stepper_row(label: Div, stepper: Stepper) -> Div {
        hstack(8.0).w_full().child(label).child(stepper)
    }
}

impl RenderOnce for VoiceChannelConfigEditor {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let draft = self.draft.get(cx);

        let user_limit = self.field(
            |draft| draft.user_limit,
            |draft, value: i64| {
                if value == 1 {
                    draft.user_limit = if draft.user_limit == 0 { 2 } else { 0 };
                } else {
                    draft.user_limit = value;
                }
                if draft.user_limit > 0 && draft.speaker_limit > draft.user_limit {
                    draft.speaker_limit = draft.user_limit;
                }
            },
        );
        let is_premium = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| profile.more.is_premium);
        let bitrate_ceiling = VoiceChannelConfig::audio_bitrate_ceiling_kbps(is_premium);
        let bitrate = draft.audio_bitrate_kbps.min(bitrate_ceiling);
        let audio_bitrate = self.field(
            move |draft| draft.audio_bitrate_kbps.min(bitrate_ceiling),
            |draft, value: i64| draft.audio_bitrate_kbps = value,
        );

        let maximum_speaker_limit = VoiceChannelConfig::maximum_speaker_limit();
        let speaker_ceiling = maximum_speaker_limit.min(if draft.user_limit == 0 {
            maximum_speaker_limit
        } else {
            draft.user_limit
        });
        let speaker_row = (self.kind == ChatChannelKind::Stage).then(|| {
            Self::stepper_row(
                Self::label(tr("rtc_speaker_limit"), Self::limit_value(draft.speaker_limit), &theme),
                Stepper::new(
                    "voice-channel-speaker-limit",
                    self.field(|draft| draft.speaker_limit, |draft, value: i64| draft.speaker_limit = value),
                    0..=speaker_ceiling,
                ),
            )
        });

        let video_enabled = self.field(|draft| draft.video_enabled, |draft, value: bool| draft.video_enabled = value);
        let screen_share_enabled = self.field(
            |draft| draft.screen_share_enabled,
            |draft, value: bool| draft.screen_share_enabled = value,
        );
        let default_muted = self.field(|draft| draft.default_muted, |draft, value: bool| draft.default_muted = value);

        div().w_full().px(px(12.0)).child(
            vstack(12.0)
                .w_full()
                .items_stretch()
                .px(px(16.0))
                .py(px(14.0))
                .bg(theme.bg._200)
                .continuous_corners(EDITOR_CORNER_RADIUS)
                .child(Self::stepper_row(
                    Self::label(tr("rtc_user_limit"), Self::limit_value(draft.user_limit), &theme),
                    Stepper::new(
                        "voice-channel-user-limit",
                        user_limit,
                        0..=VoiceChannelConfig::maximum_user_limit(),
                    ),
                ))
                .children(speaker_row)
                .child(Self::stepper_row(
                    Self::label(
                        tr("rtc_audio_bitrate"),
                        tr_args("rtc_audio_bitrate_kbps", &[FormatArg::Int(bitrate)]),
                        &theme,
                    ),
                    Stepper::new(
                        "voice-channel-audio-bitrate",
                        audio_bitrate,
                        VoiceChannelConfig::minimum_audio_bitrate_kbps()..=bitrate_ceiling,
                    )
                    .step(AUDIO_BITRATE_STEP),
                ))
                .child(Self::toggle("voice-channel-video", tr("rtc_video_enabled"), video_enabled))
                .child(Self::toggle(
                    "voice-channel-screen-share",
                    tr("rtc_screen_share_enabled"),
                    screen_share_enabled,
                ))
                .child(Self::toggle("voice-channel-default-muted", tr("rtc_default_muted"), default_muted)),
        )
    }
}

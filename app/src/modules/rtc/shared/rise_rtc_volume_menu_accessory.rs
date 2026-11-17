use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ElementId, Empty, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Subscription, Window, div, px,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAccessory;
use rise_ui::shared::try_read;
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};
use rise_ui::{IconUi, MainText};

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCParticipant;
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::RiseRTCListenerVolume;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;

/// volume panel geometry
/// summary_height feeds both the hold menu measurement and the placement
/// maths, every child is pinned to one of these numbers so the sum is exact
pub enum RiseRTCVolumeMenuMetrics {}

impl RiseRTCVolumeMenuMetrics {
    pub const HORIZONTAL_PADDING: f32 = 16.0;
    pub const VERTICAL_PADDING: f32 = 10.0;
    pub const ROW_SPACING: f32 = 6.0;
    pub const TITLE_HEIGHT: f32 = 22.0;
    pub const SLIDER_HEIGHT: f32 = 32.0;
    pub const HINT_HEIGHT: f32 = 32.0;

    pub fn summary_height(shows_hint: bool) -> f32 {
        let base = Self::VERTICAL_PADDING * 2.0 + Self::TITLE_HEIGHT + Self::ROW_SPACING + Self::SLIDER_HEIGHT;
        if shows_hint { base + Self::ROW_SPACING + Self::HINT_HEIGHT } else { base }
    }
}

/// per listener volume rail as the hold menu accessory panel
/// accessory not action row, a button click fights the slider drag
/// only way to put arbitrary content in this menu
pub enum RiseRTCVolumeMenuAccessory {}

impl RiseRTCVolumeMenuAccessory {
    /// silence with no explicit choice behind it can only come from a total block
    /// computed from values so a row carries the answer as a render input and doesnt read a store
    pub fn shows_blocked_hint(percent: f64, has_explicit_override: bool) -> bool {
        RiseRTCListenerVolume::is_muted(percent) && !has_explicit_override
    }

    /// shows_blocked_hint is frozen by the caller, re deciding
    /// it live would shrink the panel under the moving pointer
    /// the space stays and the text fades once the rail leaves silence
    pub fn configuration(
        participant: &RiseRTCParticipant,
        shows_blocked_hint: bool,
        cx: &App,
    ) -> Option<HoldContextMenuAccessory> {
        let user_id = trimming_whitespaces_and_newlines(&participant.user_id).to_owned();
        if user_id.is_empty() || try_read::<GuideSandbox>(cx).is_some_and(|sandbox| sandbox.owns_user_id(&user_id)) {
            return None;
        }

        Some(HoldContextMenuAccessory::new(
            RiseRTCVolumeMenuMetrics::summary_height(shows_blocked_hint),
            Rc::new(move |_: Rc<dyn Fn(&mut App)>| {
                RiseRTCVolumeMenuPanel { user_id: user_id.clone(), shows_blocked_hint }.into_any_element()
            }),
            Rc::new(|_: Rc<dyn Fn(&mut App)>, _: Rc<dyn Fn(String, &mut App)>| Empty.into_any_element()),
        ))
    }
}

/// holds the store observation of a view drawn inside the hold menu or a row, a value resolved
/// by the source row would be frozen when the menu captures that rows content closure
struct RiseRTCListenerVolumeObservation {
    volume: f64,
    _subscription: Subscription,
}

fn observed_listener_volume(id: ElementId, user_id: &str, window: &mut Window, cx: &mut App) -> f64 {
    let rtc = RTCServicesStore::shared(cx);
    let observed = rtc.clone();
    let observed_user = user_id.to_string();
    let _observation = window.use_keyed_state(id, cx, move |_, cx| RiseRTCListenerVolumeObservation {
        volume: observed.read(cx).listener_volume_percent(&observed_user, cx),
        _subscription: cx.observe(&observed, move |this: &mut RiseRTCListenerVolumeObservation, rtc, cx| {
            let volume = rtc.read(cx).listener_volume_percent(&observed_user, cx);
            if volume != this.volume {
                this.volume = volume;
                cx.notify();
            }
        }),
    });
    rtc.read(cx).listener_volume_percent(user_id, cx)
}

type RiseRTCListenerVolumeContentBuilder = Box<dyn FnOnce(f64, &mut Window, &mut App) -> AnyElement>;

/// keep the observation inside the copied view, a value resolved by the
/// source row freezes when the hold menu captures that rows content closure
#[derive(IntoElement)]
pub struct RiseRTCListenerVolumeContent {
    user_id: String,
    content: RiseRTCListenerVolumeContentBuilder,
}

impl RiseRTCListenerVolumeContent {
    pub fn new(
        user_id: impl Into<String>,
        content: impl FnOnce(f64, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self { user_id: user_id.into(), content: Box::new(content) }
    }
}

impl RenderOnce for RiseRTCListenerVolumeContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("rtc-listener-volume-{}", self.user_id));
        let percent = observed_listener_volume(id.into(), &self.user_id, window, cx);
        (self.content)(percent, window, cx)
    }
}

#[derive(IntoElement)]
struct RiseRTCVolumeMenuPanel {
    user_id: String,
    shows_blocked_hint: bool,
}

struct RiseRTCVolumeMenuPanelState {
    local_percent: Option<f64>,
}

impl RenderOnce for RiseRTCVolumeMenuPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let observation_id = SharedString::from(format!("rtc-volume-panel-observation-{}", self.user_id));
        let stored_percent = observed_listener_volume(observation_id.into(), &self.user_id, window, cx);
        let state = window.use_keyed_state(
            SharedString::from(format!("rtc-volume-panel-{}", self.user_id)),
            cx,
            |_, _| RiseRTCVolumeMenuPanelState { local_percent: None },
        );
        let percent = state.read(cx).local_percent.unwrap_or(stored_percent);
        let is_muted = RiseRTCListenerVolume::is_muted(percent);

        let change_state = state.clone();
        let change_user = self.user_id.clone();
        let commit_user = self.user_id.clone();
        let slider = SteppedSliderUi::new(
            SharedString::from(format!("rtc-volume-slider-{}", self.user_id)),
            percent,
            RiseRTCListenerVolume::MINIMUM_PERCENT..=RiseRTCListenerVolume::MAXIMUM_PERCENT,
            RiseRTCListenerVolume::STEP_PERCENT,
            move |value, _: &mut Window, cx: &mut App| {
                change_state.update(cx, |state, cx| {
                    state.local_percent = Some(value);
                    cx.notify();
                });
                RTCInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.preview_listener_volume(&change_user, value, cx));
            },
            move |value, _: &mut Window, cx: &mut App| {
                state.update(cx, |state, cx| {
                    state.local_percent = Some(value);
                    cx.notify();
                });
                RTCInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.commit_listener_volume(&commit_user, value, cx));
            },
        )
        .breakpoints(RiseRTCListenerVolume::BREAKPOINTS.to_vec());

        vstack(RiseRTCVolumeMenuMetrics::ROW_SPACING)
            .items_start()
            .child(
                hstack(8.0)
                    .w_full()
                    .h(px(RiseRTCVolumeMenuMetrics::TITLE_HEIGHT))
                    .items_center()
                    .children(IconUi::sized(
                        if is_muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" },
                        px(13.0),
                        theme.text.primary,
                        true,
                    ))
                    .child(MainText::new(tr("rtc_volume_title")).px(15.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new(RiseRTCListenerVolume::percent_label(percent))
                            .px(13.0)
                            .secondary(true)
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    ),
            )
            .child(div().w_full().h(px(RiseRTCVolumeMenuMetrics::SLIDER_HEIGHT)).child(slider))
            .when(self.shows_blocked_hint, |panel| {
                panel.child(
                    div()
                        .w_full()
                        .h(px(RiseRTCVolumeMenuMetrics::HINT_HEIGHT))
                        .opacity(if is_muted { 1.0 } else { 0.0 })
                        .child(MainText::new(tr("rtc_volume_blocked_hint")).px(12.0).secondary(true).number_of_lines(2)),
                )
            })
            .px(px(RiseRTCVolumeMenuMetrics::HORIZONTAL_PADDING))
            .py(px(RiseRTCVolumeMenuMetrics::VERTICAL_PADDING))
            .min_w(px(HoldContextMenuMetrics::MENU_MIN_WIDTH))
            .max_w(px(HoldContextMenuMetrics::MENU_MAX_WIDTH))
            .min_h(px(RiseRTCVolumeMenuMetrics::summary_height(self.shows_blocked_hint)))
    }
}

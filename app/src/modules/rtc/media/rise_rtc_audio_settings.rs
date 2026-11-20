use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, BoxShadow, Context, ElementId, Entity, FocusHandle, FontWeight, Global, Hsla,
    IntoElement, KeyDownEvent, RenderOnce, SharedString, Subscription, Task, WeakEntity, Window, black, container_query,
    point, white,
};
use rise_canon::{RiseLimits, RisePrivacyAudience, RisePrivacyRule};
use rise_canon::rise_response::user::GetCallAudioSettings;
use rise_engine::CancellationError;
use rise_i18n::tr;
use rise_media::rtc_media::{
    self, AudioDevice, AudioProcessingOptions, AudioProcessingSettings, MediaIntent, MicrophoneTestState, RtcUnavailable,
    INPUT_GAIN_DEFAULT, SENSITIVITY_EPSILON, gate_engaged, input_meter_level, is_locally_speaking, normalize_input_gain,
    perform,
};
use rise_platform::HostOs;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    DESKTOP_RTC_AUDIO_INPUT_DEVICE, DESKTOP_RTC_AUDIO_OUTPUT_DEVICE, DESKTOP_RTC_STREAMER_MODE, RTC_AUDIO_INPUT_GAIN,
    RTC_AUDIO_INPUT_SENSITIVITY, RTC_AUDIO_SOFTWARE_NOISE_SUPPRESSION,
};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_backdrop::backdrop_blur_layer;
use rise_ui::button_ui::ButtonUi;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::liquid_chrome::LiquidChrome;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::slider::slider;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;
use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::modules::music::shared::utils::music::music_output_volume::MusicOutputVolume;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_safe_area_insets,
};
use crate::core::widgets::navigations::main_tab_bar_settings::RiseTabBarPlatform;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::rtc::media::rise_rtc_live_kit_audio_coordinator::RiseRTCLiveKitAudioCoordinator;
use crate::modules::rtc::media::rise_rtc_live_kit_media_controller::RiseRTCLiveKitMediaController;
use crate::modules::rtc::media::rise_rtc_media_presentation::RiseRTCMediaPresentation;
use crate::modules::user::shared::privacy_mock::PrivacySettingsWireDTORules;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::core::rise_user_rpc::RiseUserRPC;
use crate::modules::user::engine::rise_user_domain::RiseUserActionExecutor;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RiseRTCAudioProcessingSettings {
    pub software_noise_suppression: bool,
    pub input_sensitivity: f64,
    pub input_gain: f64,
}

impl RiseRTCAudioProcessingSettings {
    pub fn capture_options(&self) -> AudioProcessingOptions {
        AudioProcessingSettings::from(*self).options_for(HostOs::current())
    }

    pub fn processing_options(&self) -> AudioProcessingOptions {
        AudioProcessingSettings::from(*self).options_for(HostOs::current())
    }
}

impl From<RiseRTCAudioProcessingSettings> for AudioProcessingSettings {
    fn from(settings: RiseRTCAudioProcessingSettings) -> Self {
        Self {
            software_noise_suppression: settings.software_noise_suppression,
            input_sensitivity: settings.input_sensitivity,
            input_gain: settings.input_gain,
        }
    }
}

pub struct RiseRTCAudioSettingsStore {
    software_noise_suppression: bool,
    input_sensitivity: f64,
    input_gain: f64,
    input_device: Option<String>,
    output_device: Option<String>,
    input_devices: Vec<AudioDevice>,
    output_devices: Vec<AudioDevice>,
    devices_generation: u64,
    devices_task: Option<Task<()>>,
    streamer_mode: bool,
    defaults: UserDefaults,
}

impl RiseRTCAudioSettingsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self::with_defaults(UserDefaults::standard())
    }

    fn with_defaults(defaults: UserDefaults) -> Self {
        let software_noise_suppression = defaults.bool(RTC_AUDIO_SOFTWARE_NOISE_SUPPRESSION);
        let input_sensitivity = if defaults.object(RTC_AUDIO_INPUT_SENSITIVITY).is_none() {
            0.0
        } else {
            defaults.double(RTC_AUDIO_INPUT_SENSITIVITY).max(0.0).min(1.0)
        };
        let input_gain = Self::stored_input_gain(&defaults);
        let input_device = defaults.string(DESKTOP_RTC_AUDIO_INPUT_DEVICE).filter(|id| !id.is_empty());
        let output_device = defaults.string(DESKTOP_RTC_AUDIO_OUTPUT_DEVICE).filter(|id| !id.is_empty());
        let streamer_mode = defaults.bool(DESKTOP_RTC_STREAMER_MODE);
        Self {
            software_noise_suppression,
            input_sensitivity,
            input_gain,
            input_device,
            output_device,
            input_devices: Vec::new(),
            output_devices: Vec::new(),
            devices_generation: 0,
            devices_task: None,
            streamer_mode,
            defaults,
        }
    }

    pub fn streamer_mode(&self) -> bool {
        self.streamer_mode
    }

    pub fn set_streamer_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.streamer_mode == enabled {
            return;
        }
        self.streamer_mode = enabled;
        self.defaults.set(DESKTOP_RTC_STREAMER_MODE, enabled);
        if enabled {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_privacy_loaded(cx));
        }
        cx.notify();
    }

    pub fn software_noise_suppression(&self) -> bool {
        self.software_noise_suppression
    }

    pub fn input_sensitivity(&self) -> f64 {
        self.input_sensitivity
    }

    pub fn input_gain(&self) -> f64 {
        self.input_gain
    }

    fn stored_input_gain(defaults: &UserDefaults) -> f64 {
        if defaults.object(RTC_AUDIO_INPUT_GAIN).is_none() {
            return INPUT_GAIN_DEFAULT;
        }
        normalize_input_gain(defaults.double(RTC_AUDIO_INPUT_GAIN))
    }

    /// mic volume, 1 is the voice as captured and 2 doubles it
    pub fn set_input_gain(&mut self, value: f64, cx: &mut Context<Self>) {
        let normalized = normalize_input_gain(value);
        if (self.input_gain - normalized).abs() <= 0.005 {
            return;
        }
        self.input_gain = normalized;
        self.defaults.set(RTC_AUDIO_INPUT_GAIN, normalized);
        cx.notify();
        if !RiseCallAudioOwnership::is_call_active() && !rtc_media::microphone_test_state().is_active() {
            return;
        }
        let _ = self.apply_to_audio_engine();
    }

    pub fn input_device(&self) -> Option<&str> {
        self.input_device.as_deref()
    }

    pub fn output_device(&self) -> Option<&str> {
        self.output_device.as_deref()
    }

    /// microphones to pick from, the systems own choice is left to the automatic row
    pub fn input_devices(&self) -> &[AudioDevice] {
        &self.input_devices
    }

    pub fn output_devices(&self) -> &[AudioDevice] {
        &self.output_devices
    }

    pub fn snapshot(&self) -> RiseRTCAudioProcessingSettings {
        RiseRTCAudioProcessingSettings {
            software_noise_suppression: self.software_noise_suppression,
            input_sensitivity: self.input_sensitivity,
            input_gain: self.input_gain,
        }
    }

    pub fn set_software_noise_suppression(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.software_noise_suppression == enabled {
            return;
        }
        self.software_noise_suppression = enabled;
        self.defaults.set(RTC_AUDIO_SOFTWARE_NOISE_SUPPRESSION, enabled);
        cx.notify();
        self.apply_to_active_session(cx);
    }

    /// slider only moves the gate threshold, the processing chain doesnt depend on it
    pub fn set_input_sensitivity(&mut self, value: f64, cx: &mut Context<Self>) {
        let normalized = value.max(0.0).min(1.0);
        if (self.input_sensitivity - normalized).abs() <= SENSITIVITY_EPSILON {
            return;
        }
        self.input_sensitivity = normalized;
        self.defaults.set(RTC_AUDIO_INPUT_SENSITIVITY, normalized);
        cx.notify();
        if !RiseCallAudioOwnership::is_call_active() && !rtc_media::microphone_test_state().is_active() {
            return;
        }
        let _ = self.apply_to_audio_engine();
    }

    pub fn commit_input_sensitivity(&mut self, cx: &mut Context<Self>) {
        let value = self.input_sensitivity;
        RiseRTCAudioSettingsSync::shared(cx).update(cx, |sync, cx| sync.push(value, cx));
    }

    pub fn adopt_server_input_sensitivity(&mut self, value: f64, cx: &mut Context<Self>) {
        self.set_input_sensitivity(value, cx);
    }

    /// keeps the stored settings for the next call
    /// during a call it retunes the gate and the mic processing stages
    pub fn apply_to_audio_engine(&self) -> Result<(), RtcUnavailable> {
        perform(&MediaIntent::ApplyAudioProcessing(self.snapshot().into()))
    }

    fn apply_to_active_session(&mut self, cx: &mut Context<Self>) {
        let in_call = RiseCallAudioOwnership::is_call_active();
        if !in_call && !rtc_media::microphone_test_state().is_active() {
            return;
        }
        if let Err(error) = self.apply_to_audio_engine() {
            let error_value = format!("{error:?}");
            Sai::log(
                "RTC",
                move || {
                    format!(
                        "audio processing configuration skipped error_type={} error_value={error_value}",
                        std::any::type_name::<RtcUnavailable>()
                    )
                },
                SaiLogKind::Network,
                cx,
            );
        }
        if !in_call {
            return;
        }
        let settings = self.snapshot();
        cx.spawn(async move |_, cx| {
            let _ = RiseRTCLiveKitMediaController::shared().apply_audio_processing(settings, cx).await;
        })
        .detach();
    }

    pub fn set_input_device(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let id = id.filter(|id| !id.is_empty());
        if self.input_device == id {
            return;
        }
        Self::store_device(&self.defaults, DESKTOP_RTC_AUDIO_INPUT_DEVICE, id.as_deref());
        self.input_device = id;
        cx.notify();
        self.devices_did_change(cx);
    }

    pub fn set_output_device(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let id = id.filter(|id| !id.is_empty());
        if self.output_device == id {
            return;
        }
        Self::store_device(&self.defaults, DESKTOP_RTC_AUDIO_OUTPUT_DEVICE, id.as_deref());
        self.output_device = id;
        cx.notify();
        self.devices_did_change(cx);
    }

    fn store_device(defaults: &UserDefaults, key: &str, id: Option<&str>) {
        match id {
            Some(id) => defaults.set(key, id),
            None => defaults.remove_object(key),
        }
    }

    /// a pick here holds for every call and moves the live one at once
    /// a pick from the call own route sheet stays with that call
    fn devices_did_change(&mut self, cx: &mut Context<Self>) {
        self.apply_devices_to_audio_engine(cx);
        let input = self.input_device.clone();
        RiseRTCLiveKitAudioCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.adopt_settings_devices(input, cx));
        if rtc_media::microphone_test_state().is_active() {
            Self::log_test_start(rtc_media::start_microphone_test(), cx);
        }
    }

    /// hands the picked devices to the media layer which keeps them for every call
    pub fn apply_devices_to_audio_engine(&self, cx: &mut App) {
        if let Err(error) = rtc_media::set_preferred_audio_devices(self.input_device.clone(), self.output_device.clone()) {
            let error_value = format!("{error:?}");
            Sai::log(
                "RTC",
                move || format!("audio device preference skipped error_value={error_value}"),
                SaiLogKind::Network,
                cx,
            );
        }
    }

    /// lists devices off the main thread and the main thread only commits the newest answer
    pub fn refresh_devices(&mut self, cx: &mut Context<Self>) {
        self.devices_generation = self.devices_generation.wrapping_add(1);
        let generation = self.devices_generation;
        let listing = cx.background_spawn(async move { (rtc_media::input_devices(), rtc_media::output_devices()) });
        self.devices_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let (inputs, outputs) = listing.await;
            this.update(cx, |store, cx| {
                if store.devices_generation != generation {
                    return;
                }
                store.devices_task = None;
                let inputs = inputs.unwrap_or_default();
                let outputs = outputs.unwrap_or_default();
                let input_devices: Vec<AudioDevice> = rtc_media::pickable_devices(&inputs).into_iter().cloned().collect();
                let output_devices: Vec<AudioDevice> = rtc_media::pickable_devices(&outputs).into_iter().cloned().collect();
                if store.input_devices != input_devices || store.output_devices != output_devices {
                    store.input_devices = input_devices;
                    store.output_devices = output_devices;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub fn start_microphone_test(&mut self, cx: &mut Context<Self>) {
        if !RiseCallAudioOwnership::is_call_active() {
            self.apply_devices_to_audio_engine(cx);
            if let Err(error) = self.apply_to_audio_engine() {
                let error_value = format!("{error:?}");
                Sai::log(
                    "RTC",
                    move || format!("microphone test chain skipped error_value={error_value}"),
                    SaiLogKind::Network,
                    cx,
                );
            }
        }
        Self::log_test_start(rtc_media::start_microphone_test(), cx);
        RiseMicrophoneTestHush::follow(cx);
        cx.notify();
    }

    pub fn stop_microphone_test(&mut self, cx: &mut Context<Self>) {
        rtc_media::stop_microphone_test();
        RiseMicrophoneTestHush::follow(cx);
        cx.notify();
    }

    fn log_test_start(started: Result<(), RtcUnavailable>, cx: &mut App) {
        if let Err(error) = started {
            let error_value = format!("{error:?}");
            Sai::log("RTC", move || format!("microphone test refused error_value={error_value}"), SaiLogKind::Network, cx);
        }
    }
}

/// noise gate threshold kept on the server per platform
/// read at sign in and when settings open, written when the slider is let go
/// a platform the server never saw (revision 0) gets this devices value once
const MICROPHONE_TEST_HUSH_WATCH_INTERVAL: Duration = Duration::from_millis(150);

/// call mute and deafen buttons read off while a microphone test holds the mic
#[derive(Default)]
pub struct RiseMicrophoneTestHush {
    active: bool,
    watching: bool,
}

impl Global for RiseMicrophoneTestHush {}

impl RiseMicrophoneTestHush {
    /// true while the test runs, the call neither hears us nor plays to us
    pub fn is_active(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|hush| hush.active)
    }

    fn follow(cx: &mut App) {
        let active = rtc_media::microphone_test_state().is_active();
        let (shown, watching) = cx.try_global::<Self>().map_or((false, false), |hush| (hush.active, hush.watching));
        let watch = active && !watching;
        if shown != active || watch {
            let hush = cx.default_global::<Self>();
            hush.active = active;
            hush.watching = watching || watch;
        }
        if !watch {
            return;
        }
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(MICROPHONE_TEST_HUSH_WATCH_INTERVAL).await;
                if !cx.update(Self::tick) {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick(cx: &mut App) -> bool {
        let active = rtc_media::microphone_test_state().is_active();
        if !active {
            let hush = cx.default_global::<Self>();
            hush.active = false;
            hush.watching = false;
            return false;
        }
        if !Self::is_active(cx) {
            cx.default_global::<Self>().active = true;
        }
        true
    }
}

pub struct RiseRTCAudioSettingsSync {
    platform: RiseTabBarPlatform,
    permille_max: i64,
    load_task: Option<Task<()>>,
    write_sequence: u64,
}

impl RiseRTCAudioSettingsSync {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            platform: RiseTabBarPlatform::CURRENT,
            permille_max: RiseLimits::service_value("call_audio_input_sensitivity_permille_max"),
            load_task: None,
            write_sequence: 0,
        }
    }

    fn active_account_id(cx: &mut App) -> Option<String> {
        let auth = AuthServicesStore::shared(cx);
        auth.read(cx).active_account_id().map(str::to_owned)
    }

    fn signed_in_account_id(cx: &mut App) -> Option<String> {
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        if !auth.is_authenticated() {
            return None;
        }
        auth.active_account_id().map(str::to_owned)
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = Self::signed_in_account_id(cx) else {
            return;
        };
        if self.load_task.is_some() {
            return;
        }
        let started_at = self.write_sequence;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let domain = cx.update(RiseUserActionExecutor::require_domain).await;
            let reply = match domain {
                Ok(domain) => RiseUserRPC::new(domain.engine.clone()).get_call_audio_settings().await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |sync, cx| {
                sync.load_task = None;
                sync.adopt(reply, &account_id, started_at, cx);
            });
        }));
    }

    fn adopt(
        &mut self,
        reply: anyhow::Result<GetCallAudioSettings>,
        account_id: &str,
        started_at: u64,
        cx: &mut Context<Self>,
    ) {
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                if error.downcast_ref::<CancellationError>().is_none() {
                    let label = RiseUserDebugTrace::error_label(&error);
                    Sai::log("RTC", move || format!("call audio settings load failed: {label}"), SaiLogKind::Network, cx);
                }
                return;
            }
        };
        if Self::active_account_id(cx).as_deref() != Some(account_id) {
            return;
        }
        if !reply.success {
            let error = reply.error;
            Sai::log("RTC", move || format!("call audio settings load refused: {error}"), SaiLogKind::Network, cx);
            return;
        }
        if started_at != self.write_sequence {
            return;
        }
        let own = reply
            .settings
            .and_then(|settings| match self.platform {
                RiseTabBarPlatform::Ios => settings.ios,
                RiseTabBarPlatform::Android => settings.android,
            })
            .filter(|own| own.revision != 0);
        let Some(own) = own else {
            let value = RiseRTCAudioSettingsStore::shared(cx).read(cx).input_sensitivity();
            self.push(value, cx);
            return;
        };
        let permille = own.input_sensitivity_permille.max(0).min(self.permille_max);
        let value = permille as f64 / self.permille_max.max(1) as f64;
        RiseRTCAudioSettingsStore::shared(cx).update(cx, |store, cx| store.adopt_server_input_sensitivity(value, cx));
    }

    pub fn push(&mut self, value: f64, cx: &mut Context<Self>) {
        let Some(account_id) = Self::signed_in_account_id(cx) else {
            return;
        };
        self.write_sequence += 1;
        let permille_max = self.permille_max;
        let permille = ((value.clamp(0.0, 1.0) * permille_max as f64).round() as i64).max(0).min(permille_max);
        let platform = self.platform;
        cx.spawn(async move |_, cx| {
            let domain = cx.update(RiseUserActionExecutor::require_domain).await;
            let reply = match domain {
                Ok(domain) => {
                    if cx.update(Self::active_account_id).as_deref() != Some(account_id.as_str()) {
                        return;
                    }
                    RiseUserRPC::new(domain.engine.clone()).update_call_audio_settings(platform, permille).await
                }
                Err(error) => Err(error),
            };
            cx.update(|cx| match reply {
                Ok(reply) if !reply.success => {
                    let error = reply.error;
                    Sai::log("RTC", move || format!("call audio settings update refused: {error}"), SaiLogKind::Network, cx);
                }
                Ok(_) => {}
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {}
                Err(error) => {
                    let label = RiseUserDebugTrace::error_label(&error);
                    Sai::log("RTC", move || format!("call audio settings update failed: {label}"), SaiLogKind::Network, cx);
                }
            });
        })
        .detach();
    }
}

/// level gate on the published mic signal after capture processing
/// desktop seam runs the gate on each block before the sdk audio source
pub enum RiseRTCInputNoiseGate {}

impl RiseRTCInputNoiseGate {
    pub fn is_engaged(sensitivity: f64) -> bool {
        gate_engaged(sensitivity)
    }

    pub fn meter_level() -> f64 {
        input_meter_level()
    }

    pub fn is_locally_speaking() -> bool {
        is_locally_speaking()
    }
}

/// categories shown again during the current share, a new share hides them again
#[derive(Default)]
struct RiseRTCStreamerReveals(HashSet<RisePrivacyRule>);

impl Global for RiseRTCStreamerReveals {}

/// streamer mode, own profile data the privacy settings keep from others stays covered while the screen is shared
pub enum RiseRTCStreamerMode {}

impl RiseRTCStreamerMode {
    const VEIL_BLUR: f32 = 18.0;
    const VEIL_MIN_HEIGHT: f32 = 280.0;

    pub fn is_live(cx: &mut App) -> bool {
        if !RiseRTCAudioSettingsStore::shared(cx).read(cx).streamer_mode() {
            return false;
        }
        let Some(user_id) =
            AuthServicesStore::shared(cx).read(cx).active_account_id().filter(|id| !id.is_empty()).map(str::to_owned)
        else {
            return false;
        };
        RiseRTCMediaPresentation::shared(cx).read(cx).screen_share_track(&user_id).is_some()
    }

    /// own data under this rule is covered now, an unknown audience counts as private until the settings load
    pub fn conceals(rule: RisePrivacyRule, cx: &mut App) -> bool {
        if !Self::is_live(cx) || cx.try_global::<RiseRTCStreamerReveals>().is_some_and(|shown| shown.0.contains(&rule)) {
            return false;
        }
        let optimistic = UserServicesStore::shared(cx).read(cx).optimistic_privacy().cloned();
        let privacy = optimistic.or_else(|| UserActionsStore::shared(cx).read(cx).get_privacy_sai.data());
        privacy.is_none_or(|privacy| privacy.audience(rule) != RisePrivacyAudience::All)
    }

    pub fn reveal(rule: RisePrivacyRule, cx: &mut App) {
        cx.default_global::<RiseRTCStreamerReveals>().0.insert(rule);
        cx.refresh_windows();
    }

    pub(crate) fn share_started(cx: &mut App) {
        if let Some(shown) = cx.try_global::<RiseRTCStreamerReveals>()
            && !shown.0.is_empty()
        {
            cx.set_global(RiseRTCStreamerReveals::default());
        }
        if RiseRTCAudioSettingsStore::shared(cx).read(cx).streamer_mode() {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_privacy_loaded(cx));
            cx.refresh_windows();
        }
    }

    pub(crate) fn share_ended(cx: &mut App) {
        if RiseRTCAudioSettingsStore::shared(cx).read(cx).streamer_mode() {
            cx.refresh_windows();
        }
    }

    /// small show action for a covered row
    pub fn reveal_button(id: impl Into<ElementId>, rule: RisePrivacyRule, theme: &AppTheme) -> AnyElement {
        div()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                ButtonUi::new(id)
                    .background_color(theme.primary._100)
                    .height(30.0)
                    .fit_content(true)
                    .padding_horizontal(14.0)
                    .on_press(move |_, cx| Self::reveal(rule, cx))
                    .child(MainText::new(tr("desktop_streamer_mode_show")).px(13.0).font_weight(FontWeight::SEMIBOLD)),
            )
            .into_any_element()
    }

    pub fn veil(id: impl Into<ElementId>, rule: RisePrivacyRule, content: AnyElement, cx: &mut App) -> AnyElement {
        if !Self::conceals(rule, cx) {
            return content;
        }
        let id = id.into();
        let theme = rise_ui::theme(cx).clone();
        let blur = backdrop_blur_layer(Self::VEIL_BLUR, cx);
        let dim = if blur.is_some() { 0.45 } else { 0.96 };
        div()
            .relative()
            .size_full()
            .min_h(px(Self::VEIL_MIN_HEIGHT))
            .child(content)
            .child(
                div()
                    .id(id.clone())
                    .absolute()
                    .inset_0()
                    .block_mouse_except_scroll()
                    .children(blur)
                    .child(div().absolute().inset_0().bg(theme.bg._100.opacity(dim)))
                    .child(
                        vstack(12.0)
                            .absolute()
                            .top(px(48.0))
                            .left_0()
                            .right_0()
                            .px(px(24.0))
                            .items_center()
                            .children(IconUi::sized("eye.slash", px(28.0), theme.text.secondary, false))
                            .child(
                                MainText::new(tr("desktop_streamer_mode_hidden"))
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .tac(TextAlignT::Center),
                            )
                            .child(Self::reveal_button(ElementId::NamedChild(std::sync::Arc::new(id), "show".into()), rule, &theme)),
                    ),
            )
            .into_any_element()
    }
}

pub struct RiseRTCStreamSettingsScreen {
    _settings: Subscription,
}

impl RiseRTCStreamSettingsScreen {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = RiseRTCAudioSettingsStore::shared(cx);
        let settings = cx.observe(&store, |_, _, cx| cx.notify());
        Self { _settings: settings }
    }
}

impl Render for RiseRTCStreamSettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let navigation_store = env::<NavigationStoreKey>(cx);
        let theme = rise_ui::theme(cx).clone();
        let shell_insets = list_shell_safe_area_insets(cx);
        let store = RiseRTCAudioSettingsStore::shared(cx);
        let enabled = store.read(cx).streamer_mode();
        let toggle = RiseRTCAudioSettingsContent::settings_toggle(
            "rtc-streamer-mode",
            tr("desktop_streamer_mode_title"),
            tr("desktop_streamer_mode_description"),
            enabled,
            &theme,
            move |enabled, _, cx| store.update(cx, |store, cx| store.set_streamer_mode(enabled, cx)),
        );
        let body = scroll_view("rtc-stream-settings-scroll")
            .shows_indicators(false)
            .list_shell_scroll_under_chrome(cx)
            .bg(theme.bg._100)
            .child(
                div().w_full().pt(px(14.0)).pb(px(shell_insets.bottom.max(8.0) + 12.0)).child(
                    div()
                        .w_full()
                        .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
                        .child(GroupedCardMetrics::SHAPE.round(vstack(0.0).w_full().bg(theme.bg._200).child(toggle)))
                        .frame_max_width(Alignment::Leading),
                ),
            );
        SettingsWrapperUi::new(tr("desktop_settings_streams_title"), move |_, cx| {
            if let Some(navigation_store) = navigation_store.as_ref() {
                navigation_store.update(cx, |navigation_store, cx| navigation_store.pop(cx));
            }
        })
        .content(body)
    }
}

pub struct RiseRTCAudioSettingsScreen {
    _settings: Subscription,
}

impl RiseRTCAudioSettingsScreen {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = RiseRTCAudioSettingsStore::shared(cx);
        let settings = cx.observe(&store, |_, _, cx| cx.notify());
        Self { _settings: settings }
    }
}

impl Render for RiseRTCAudioSettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let navigation_store = env::<NavigationStoreKey>(cx);
        SettingsWrapperUi::new(tr("rtc_voice_settings"), move |_, cx| {
            if let Some(navigation_store) = navigation_store.as_ref() {
                navigation_store.update(cx, |navigation_store, cx| navigation_store.pop(cx));
            }
        })
        .content(RiseRTCAudioSettingsScrollBody)
    }
}

#[derive(IntoElement)]
struct RiseRTCAudioSettingsScrollBody;

impl RenderOnce for RiseRTCAudioSettingsScrollBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let shell_insets = list_shell_safe_area_insets(cx);
        let theme = rise_ui::theme(cx).clone();

        scroll_view("rtc-audio-settings-scroll")
            .shows_indicators(false)
            .list_shell_scroll_under_chrome(cx)
            .bg(theme.bg._100)
            .child(
                div()
                    .w_full()
                    .pt(px(14.0))
                    .pb(px(shell_insets.bottom.max(8.0) + 12.0))
                    .child(
                        vstack(14.0)
                            .align(HorizontalAlignment::Leading)
                            .w_full()
                            .min_w_0()
                            .child(RiseRTCAudioDevicesSection { theme: theme.clone() })
                            .child(RiseRTCAudioSettingsContent),
                    ),
            )
    }
}

#[derive(IntoElement)]
pub struct RiseRTCAudioSettingsContent;

impl RenderOnce for RiseRTCAudioSettingsContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use_task("rtc-audio-settings-sync", (), window, cx, async |cx| {
            let _ = cx.update(|_, cx| RiseRTCAudioSettingsSync::shared(cx).update(cx, |sync, cx| sync.load(cx)));
        });
        let theme = rise_ui::theme(cx).clone();
        let settings = RiseRTCAudioSettingsStore::shared(cx);
        let snapshot = settings.read(cx).snapshot();
        let toggle_store = settings.clone();
        let sensitivity_store = settings.clone();
        let commit_store = settings;

        vstack(14.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .min_w_0()
            .child(
                div().px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(
                    MainText::new(tr("rtc_voice_processing"))
                        .px(13.0)
                        .font_weight(FontWeight::BOLD)
                        .color(theme.text.secondary),
                ),
            )
            .child(
                div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                    GroupedCardMetrics::SHAPE.round(
                        vstack(0.0).w_full().bg(theme.bg._200).child(Self::settings_toggle(
                            "rtc-software-noise-suppression",
                            tr("rtc_software_noise_suppression"),
                            tr("rtc_software_noise_suppression_description"),
                            snapshot.software_noise_suppression,
                            &theme,
                            move |enabled, _, cx| {
                                toggle_store.update(cx, |store, cx| store.set_software_noise_suppression(enabled, cx));
                            },
                        )),
                    ),
                ),
            )
            .child(
                div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                    GroupedCardMetrics::SHAPE.round(
                        vstack(10.0)
                            .align(HorizontalAlignment::Leading)
                            .w_full()
                            .p(px(16.0))
                            .bg(theme.bg._200)
                            .child(
                                hstack(None)
                                    .w_full()
                                    .child(
                                        vstack(3.0)
                                            .align(HorizontalAlignment::Leading)
                                            .min_w_0()
                                            .flex_shrink_1()
                                            .child(
                                                MainText::new(tr("rtc_input_sensitivity"))
                                                    .px(15.0)
                                                    .font_weight(FontWeight::SEMIBOLD),
                                            )
                                            .child(
                                                MainText::new(tr("rtc_input_sensitivity_description"))
                                                    .px(12.0)
                                                    .color(theme.text.secondary),
                                            ),
                                    )
                                    .child(spacer_min_w(8.0))
                                    .child(
                                        div().flex_none().child(
                                            MainText::new(Self::sensitivity_label(snapshot.input_sensitivity))
                                                .px(12.0)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .color(theme.primary._100),
                                        ),
                                    ),
                            )
                            .child(RiseRTCInputThresholdSlider {
                                value: snapshot.input_sensitivity,
                                theme: theme.clone(),
                                on_change: Rc::new(move |value, cx| {
                                    sensitivity_store.update(cx, |store, cx| store.set_input_sensitivity(value, cx));
                                }),
                                on_commit: Rc::new(move |cx| {
                                    commit_store.update(cx, |store, cx| store.commit_input_sensitivity(cx));
                                }),
                            }),
                    ),
                ),
            )
            .child(RiseRTCAppVolumeCard {
                id: "rtc-input-gain",
                title: tr("rtc_input_gain"),
                description: tr("rtc_input_gain_description"),
                read: || RiseRTCAudioSettingsStore::stored_input_gain(&UserDefaults::standard()) as f32,
                write: |value, cx| {
                    RiseRTCAudioSettingsStore::shared(cx).update(cx, |store, cx| store.set_input_gain(f64::from(value), cx));
                },
                max: rtc_media::INPUT_GAIN_MAX as f32,
                theme: theme.clone(),
            })
            .child(RiseRTCAppVolumeCard {
                id: "rtc-app-volume",
                title: tr("rtc_app_volume_title"),
                description: tr("desktop_rtc_app_volume_description"),
                read: RiseCallAppVolume::level,
                write: |value, _| RiseCallAppVolume::set_level(value),
                max: 1.0,
                theme: theme.clone(),
            })
            .child(RiseRTCAppVolumeCard {
                id: "rtc-music-volume",
                title: tr("editor_volume_music"),
                description: tr("desktop_rtc_music_volume_description"),
                read: MusicOutputVolume::level,
                write: MusicOutputVolume::set_level,
                max: 1.0,
                theme: theme.clone(),
            })
            .child(
                div().w_full().min_w_0().px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(
                    MainText::new(tr("rtc_voice_processing_default_note"))
                        .px(12.0)
                        .color(theme.text.secondary),
                ),
            )
            .frame_max_width(Alignment::Leading)
    }
}

impl RiseRTCAudioSettingsContent {
    fn sensitivity_label(input_sensitivity: f64) -> SharedString {
        if !RiseRTCInputNoiseGate::is_engaged(input_sensitivity) {
            return tr("rtc_input_open").into();
        }
        format!("{}%", (input_sensitivity * 100.0).round() as i64).into()
    }

    fn settings_toggle(
        id: &'static str,
        title: String,
        detail: String,
        is_on: bool,
        theme: &AppTheme,
        action: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let secondary: Hsla = theme.text.secondary;
        hstack(None)
            .w_full()
            .p(px(16.0))
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .min_w_0()
                    .flex_shrink_1()
                    .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD))
                    .child(MainText::new(detail).px(12.0).color(secondary)),
            )
            .child(spacer_min_w(8.0))
            .child(div().flex_none().child(SwitchUi::new(id, Binding::new(move |_| is_on, action))))
    }
}

#[derive(IntoElement)]
struct RiseRTCAudioDevicesSection {
    theme: AppTheme,
}

impl RenderOnce for RiseRTCAudioDevicesSection {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        vstack(14.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .min_w_0()
            .child(
                div().px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(
                    MainText::new(tr("rtc_audio_route"))
                        .px(13.0)
                        .font_weight(FontWeight::BOLD)
                        .color(theme.text.secondary),
                ),
            )
            .child(RiseRTCAudioDevicesCard { theme: theme.clone() })
            .frame_max_width(Alignment::Leading)
    }
}

#[derive(IntoElement)]
struct RiseRTCAudioDevicesCard {
    theme: AppTheme,
}

impl RenderOnce for RiseRTCAudioDevicesCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use_task("rtc-audio-devices-refresh", (), window, cx, async |cx| {
            let _ = cx.update(|_, cx| RiseRTCAudioSettingsStore::shared(cx).update(cx, |store, cx| store.refresh_devices(cx)));
        });
        let theme = self.theme;
        let store = RiseRTCAudioSettingsStore::shared(cx);
        let (inputs, input, outputs, output) = {
            let store = store.read(cx);
            (
                store.input_devices().to_vec(),
                store.input_device().map(str::to_owned),
                store.output_devices().to_vec(),
                store.output_device().map(str::to_owned),
            )
        };
        let refresh_store = store.clone();
        let refresh: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
            refresh_store.update(cx, |store, cx| store.refresh_devices(cx));
        });
        let input_store = store.clone();
        let output_store = store;

        div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
            GroupedCardMetrics::SHAPE.round(
                vstack(0.0)
                    .w_full()
                    .bg(theme.bg._200)
                    .child(Self::device_row(
                        "rtc-audio-input-device",
                        tr("rtc_audio_input"),
                        inputs,
                        input,
                        &theme,
                        Rc::new(move |id, cx| input_store.update(cx, |store, cx| store.set_input_device(id, cx))),
                        refresh.clone(),
                    ))
                    .child(div().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100)))
                    .child(Self::device_row(
                        "rtc-audio-output-device",
                        tr("rtc_audio_output"),
                        outputs,
                        output,
                        &theme,
                        Rc::new(move |id, cx| output_store.update(cx, |store, cx| store.set_output_device(id, cx))),
                        refresh,
                    )),
            ),
        )
    }
}

impl RiseRTCAudioDevicesCard {
    const MENU_LABEL_MAX_WIDTH: f32 = 280.0;

    fn device_row(
        id: &'static str,
        title: String,
        devices: Vec<AudioDevice>,
        selected: Option<String>,
        theme: &AppTheme,
        on_pick: Rc<dyn Fn(Option<String>, &mut App)>,
        refresh: Rc<dyn Fn(&mut App)>,
    ) -> impl IntoElement {
        let automatic = tr("rtc_audio_automatic");
        let current = match selected.as_deref() {
            None => automatic.clone(),
            Some(selected) => devices
                .iter()
                .find(|device| device.id == selected)
                .map_or_else(|| selected.to_owned(), |device| device.name.clone()),
        };
        let mut items = Vec::with_capacity(devices.len() + 1);
        let pick_automatic = on_pick.clone();
        items.push(ContextMenuItem::new(0, automatic, None, String::new(), move |_, cx| pick_automatic(None, cx)));
        for (index, device) in devices.into_iter().enumerate() {
            let pick = on_pick.clone();
            let device_id = device.id.clone();
            items.push(ContextMenuItem::new(index as i64 + 1, device.name, None, device.id, move |_, cx| {
                pick(Some(device_id.clone()), cx);
            }));
        }
        let label = hstack(6.0)
            .max_w(px(Self::MENU_LABEL_MAX_WIDTH))
            .min_w_0()
            .child(
                div()
                    .min_w_0()
                    .flex_shrink_1()
                    .child(MainText::new(current).px(15.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .children(IconUi::sized("chevron.down", px(12.0), theme.text.secondary, false));

        hstack(None)
            .w_full()
            .p(px(16.0))
            .child(div().flex_none().child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD)))
            .child(spacer_min_w(8.0))
            .child(
                div().min_w_0().flex_shrink_1().child(
                    ContextMenuUi::new(id, items, label)
                        .selected(selected.unwrap_or_default())
                        .on_presented_change(move |is_open, _, cx| {
                            if is_open {
                                refresh(cx);
                            }
                        }),
                ),
            )
    }
}

#[derive(IntoElement)]
struct RiseRTCAppVolumeCard {
    id: &'static str,
    title: String,
    description: String,
    read: fn() -> f32,
    write: fn(f32, &mut App),
    max: f32,
    theme: AppTheme,
}

impl RenderOnce for RiseRTCAppVolumeCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let (read, write) = (self.read, self.write);
        let level = window.use_keyed_state(SharedString::from(format!("{}-level", self.id)), cx, move |_, _| f64::from(read()));
        let live = f64::from(read());
        if (*level.read(cx) - live).abs() > 0.001 {
            level.update(cx, |level, _| *level = live);
        }
        let current = live;

        div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
            GroupedCardMetrics::SHAPE.round(
                vstack(10.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .p(px(16.0))
                    .bg(theme.bg._200)
                    .child(
                        hstack(None)
                            .w_full()
                            .child(
                                vstack(3.0)
                                    .align(HorizontalAlignment::Leading)
                                    .min_w_0()
                                    .flex_shrink_1()
                                    .child(
                                        MainText::new(self.title)
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD),
                                    )
                                    .child(
                                        MainText::new(self.description)
                                            .px(12.0)
                                            .color(theme.text.secondary),
                                    ),
                            )
                            .child(spacer_min_w(8.0))
                            .child(
                                div().flex_none().child(
                                    MainText::new(format!("{}%", (current * 100.0).round() as i64))
                                        .px(12.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.primary._100),
                                ),
                            ),
                    )
                    .child(
                        slider(
                            self.id,
                            Binding::entity(
                                level,
                                |level: &f64| *level,
                                move |level: &mut f64, value: f64, cx| {
                                    *level = value;
                                    write(value as f32, cx);
                                    cx.notify();
                                },
                            ),
                            0.0..=f64::from(self.max),
                        )
                        .tint(theme.primary._100),
                    ),
            ),
        )
    }
}

const THRESHOLD_SLIDER_HEIGHT: f32 = 28.0;
const THRESHOLD_TRACK_HEIGHT: f32 = 6.0;
const THRESHOLD_THUMB_DIAMETER: f32 = 24.0;
const THRESHOLD_METER_INTERVAL: Duration = Duration::from_millis(50);
const THRESHOLD_ACCESSIBILITY_STEP: f64 = 0.05;
const THRESHOLD_BELOW_OPACITY: f32 = 0.4;

struct RiseRTCInputThresholdSliderState {
    width: Rc<Cell<f32>>,
    is_dragging: bool,
    focus: FocusHandle,
    meter: Option<Task<()>>,
    owns_test: bool,
}

impl Drop for RiseRTCInputThresholdSliderState {
    fn drop(&mut self) {
        if self.owns_test {
            rtc_media::stop_microphone_test();
        }
    }
}

#[derive(IntoElement)]
struct RiseRTCInputThresholdSlider {
    value: f64,
    theme: AppTheme,
    on_change: Rc<dyn Fn(f64, &mut App)>,
    on_commit: Rc<dyn Fn(&mut App)>,
}

impl RenderOnce for RiseRTCInputThresholdSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.current_view();
        let state = window.use_keyed_state("rtc-input-threshold-slider", cx, |_, cx| RiseRTCInputThresholdSliderState {
            width: Rc::new(Cell::new(0.0)),
            is_dragging: false,
            focus: cx.focus_handle().tab_stop(true),
            meter: None,
            owns_test: false,
        });
        let test_state = rtc_media::microphone_test_state();
        let meter_live = RiseCallAudioOwnership::is_call_active() || test_state.is_active();
        state.update(cx, |state, cx| {
            if !meter_live {
                state.meter = None;
                return;
            }
            if state.meter.is_none() {
                state.meter = Some(cx.spawn(async move |state, cx| {
                    loop {
                        cx.background_executor().timer(THRESHOLD_METER_INTERVAL).await;
                        if state
                            .update(cx, |_, cx| {
                                cx.notify();
                                App::notify(cx, host);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            }
        });

        let value = self.value;
        let theme = self.theme;
        let (width, focus) = {
            let state = state.read(cx);
            (state.width.clone(), state.focus.clone())
        };
        let level = if meter_live { RiseRTCInputNoiseGate::meter_level() } else { 0.0 };
        let shows_failure = test_state == MicrophoneTestState::Failed && state.read(cx).owns_test;
        let test_button = microphone_test_button(test_state, state.clone(), host, &theme);
        let passes = !RiseRTCInputNoiseGate::is_engaged(value) || level >= value;
        let fill = theme.semantic.success_100.opacity(if passes { 1.0 } else { THRESHOLD_BELOW_OPACITY });
        let border = theme.text.secondary.opacity(0.25);

        let laid_out = width.clone();
        let changed_state = state.clone();
        let ended_state = state;
        let on_drag_change = self.on_change.clone();
        let on_drag_commit = self.on_commit.clone();
        let on_key_change = self.on_change;
        let on_key_commit = self.on_commit;

        let slider = div()
            .id("rtc-input-threshold-slider")
            .relative()
            .w_full()
            .h(px(THRESHOLD_SLIDER_HEIGHT))
            .role(gpui::Role::Slider)
            .aria_label(SharedString::from(tr("rtc_input_sensitivity")))
            .aria_value(SharedString::from(RiseFormat::percent(value)))
            .track_focus(&focus)
            .cursor_pointer()
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let delta = match event.keystroke.key.as_str() {
                    "left" | "down" => -THRESHOLD_ACCESSIBILITY_STEP,
                    "right" | "up" => THRESHOLD_ACCESSIBILITY_STEP,
                    _ => return,
                };
                cx.stop_propagation();
                on_key_change((value + delta).clamp(0.0, 1.0), cx);
                on_key_commit(cx);
            })
            .child(container_query(move |size, _, _| {
                let width = f32::from(size.width);
                laid_out.set(width);
                threshold_slider_body(width, value, level, fill, border)
            }))
            .gesture(
                "rtc-input-threshold-slider-drag",
                DragGesture::new(0.0)
                    .on_changed(move |drag, _, cx| {
                        changed_state.update(cx, |state, _| state.is_dragging = true);
                        let span = (width.get() - THRESHOLD_THUMB_DIAMETER).max(1.0);
                        let x = f32::from(drag.location.x) - THRESHOLD_THUMB_DIAMETER / 2.0;
                        on_drag_change(f64::from(x / span).clamp(0.0, 1.0), cx);
                    })
                    .on_ended(move |_, _, cx| {
                        let was_dragging = ended_state.update(cx, |state, _| std::mem::replace(&mut state.is_dragging, false));
                        if was_dragging {
                            on_drag_commit(cx);
                        }
                    }),
            );
        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(div().w_full().child(slider))
            .child(test_button)
            .when(shows_failure, |column| {
                column.child(MainText::new(tr("rtc_media_microphone_failed")).px(12.0).color(theme.semantic.error_100))
            })
    }
}

fn microphone_test_button(
    test_state: MicrophoneTestState,
    slider_state: Entity<RiseRTCInputThresholdSliderState>,
    host: gpui::EntityId,
    theme: &AppTheme,
) -> impl IntoElement {
    let is_active = test_state.is_active();
    let title = if is_active { tr("rtc_microphone_test_stop") } else { tr("rtc_microphone_test_start") };
    ButtonUi::new("rtc-microphone-test")
        .background_color(LiquidChrome::surface_fill(theme, 1))
        .padding_horizontal(16.0)
        .on_press(move |_, cx| {
            let is_active = rtc_media::microphone_test_state().is_active();
            slider_state.update(cx, |state, _| state.owns_test = !is_active);
            RiseRTCAudioSettingsStore::shared(cx).update(cx, |store, cx| {
                if is_active {
                    store.stop_microphone_test(cx);
                } else {
                    store.start_microphone_test(cx);
                }
            });
            App::notify(cx, host);
        })
        .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
}

fn threshold_slider_body(width: f32, value: f64, level: f64, fill: Hsla, border: Hsla) -> gpui::Div {
    let track_width = (width - THRESHOLD_THUMB_DIAMETER).max(0.0);
    let thumb_x = THRESHOLD_THUMB_DIAMETER / 2.0 + track_width * value as f32;
    div()
        .relative()
        .size_full()
        .child(
            div()
                .absolute()
                .left(px(THRESHOLD_THUMB_DIAMETER / 2.0))
                .top(px((THRESHOLD_SLIDER_HEIGHT - THRESHOLD_TRACK_HEIGHT) / 2.0))
                .w(px(track_width))
                .h(px(THRESHOLD_TRACK_HEIGHT))
                .child(div().absolute().inset_0().rounded_full().bg(white()).border(px(0.5)).border_color(border))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(px(track_width * level as f32))
                        .rounded_full()
                        .bg(fill),
                ),
        )
        .child(
            div()
                .absolute()
                .left(px(thumb_x - THRESHOLD_THUMB_DIAMETER / 2.0))
                .top(px((THRESHOLD_SLIDER_HEIGHT - THRESHOLD_THUMB_DIAMETER) / 2.0))
                .size(px(THRESHOLD_THUMB_DIAMETER))
                .rounded_full()
                .bg(white())
                .shadow(vec![BoxShadow {
                    color: black().opacity(0.25),
                    offset: point(px(0.0), px(1.0)),
                    blur_radius: px(3.0),
                    spread_radius: px(0.0),
                    inset: false,
                }]),
        )
}

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, ScrollHandle,
    SharedString, Subscription, Task, WeakEntity, Window, div, px,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::{NSRange, utf16_to_byte_range};
use rise_errors::ErrorCode;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_theme::AppTheme;
use rise_theme::ui_color_system::UIColor;
use rise_ui::bg_wrapper_ui::BgWrapperUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::error_text_ui::ErrorTextUi;
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::phone_country_catalog::{PhoneCountryCatalog, PhoneInputCountry};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::date_picker::{DatePicker, DatePickerComponents};
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::open_url::{OpenURLAction, OpenURLActionResult, OpenURLKey};
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};
use rise_ui::swiftui::repeating_timer::RepeatingTimer;
use rise_ui::swiftui::rich_text::{AttributeContainer, AttributedString, RichFont, RichText};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};
use rise_validate::{PHONE, TextSchema, VERIFICATION_CODE, ViolationKind};
use rise_widgets::window_controls_band;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::{RiseLimitsCanonRules, ValidationSchemaCanonRules};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::input_ui::InputUi;
use crate::core::ui::phone_input_ui::PhoneInputUi;
use crate::modules::auth::services::registration_policy_probe::{
    RegistrationPolicyProbe, RegistrationPolicyProbeMode,
};
use crate::modules::auth::stores::auth_actions::auth_types::{AuthCodeChannel, AuthCodeDelivery};
use crate::modules::auth::stores::auth_actions::auth_wire_types::{
    AuthCheckTagResponse, AuthLoginResponse, AuthRegisterResponse, AuthResetPasswordResponse, AuthSendCodeResponse,
    AuthSendCodeResponseExt,
};
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::{AuthFlowChanged, AuthServicesStore};
use crate::modules::auth::widgets::modals::password_reset_intro_sheet::PasswordResetIntroSheet;
use crate::modules::auth::widgets::modals::telegram_redirect_sheet::{
    TelegramRedirectSheet, TelegramRedirectSheetPurpose,
};
use crate::modules::user::shared::rise_legal_links::{RiseLegalLinks, RiseLegalLinksDocument};

const ESCAPE_KEY: &str = "escape";
const ENTER_KEY: &str = "enter";
const BACKSPACE_KEY: &str = "backspace";
const TAB_KEY: &str = "tab";

const COMPACT_HEIGHT: f32 = 500.0;

/// auth field rules addressed by wire pointer
/// only the four fields whose contract profile matches the rule the screen enforces
/// phone and code left off, contract describes the wire number not typed
pub enum AuthFieldRules {}

impl AuthFieldRules {
    pub fn login_password() -> RiseFieldRule {
        RiseFieldRule::new("auth_login", "/password")
    }

    pub fn registration_password() -> RiseFieldRule {
        RiseFieldRule::new("auth_register", "/password")
    }

    pub fn name() -> RiseFieldRule {
        RiseFieldRule::new("auth_register", "/name")
    }

    pub fn tag() -> RiseFieldRule {
        RiseFieldRule::new("auth_register", "/tag")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStepFlowModelPurpose {
    Login,
    Registration,
    PasswordReset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthStepFlowModelStep {
    Phone,
    LoginPassword,
    RegistrationName,
    RegistrationBirthDate,
    RegistrationTag,
    RegistrationPassword,
    RegistrationPasswordConfirmation,
    VerificationCode,
    ResetCode,
    ResetPassword,
    ResetPasswordConfirmation,
    RegistrationInvite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthStepFlowModelTagAvailability {
    Idle,
    Checking,
    Available,
    Unavailable,
    Invalid(String),
    Failed,
}

pub struct AuthStepFlowModel {
    pub purpose: AuthStepFlowModelPurpose,
    pub step: AuthStepFlowModelStep,
    pub selected_country: PhoneInputCountry,
    pub phone_number: String,
    pub login_password: String,
    pub name: String,
    pub tag: String,
    pub registration_password: String,
    pub repeated_password: String,
    pub new_password: String,
    pub repeated_new_password: String,
    pub code: String,
    pub birth_date: Date,
    pub tag_availability: AuthStepFlowModelTagAvailability,
    pub is_submitting: bool,
    pub is_resending: bool,
    pub code_length: i64,
    pub registration_mode: RegistrationPolicyProbeMode,
    pub invite_code: String,
    moves_forward: bool,

    registration_started_from_login: bool,
    is_opening_registration: bool,
    password_reset_started_from_step: AuthStepFlowModelStep,
    can_reuse_verification_code: bool,
    field_errors: HashMap<AuthStepFlowModelStep, String>,
    server_error_texts: HashMap<AuthStepFlowModelStep, (String, String)>,
    tag_check_generation: u64,
    registration_policy_task: Option<Task<()>>,
}

impl AuthStepFlowModel {
    pub fn new(purpose: AuthStepFlowModelPurpose) -> Self {
        let now = RiseClock::now();
        Self {
            purpose,
            step: AuthStepFlowModelStep::Phone,
            selected_country: PhoneCountryCatalog::default_country(),
            phone_number: String::new(),
            login_password: String::new(),
            name: String::new(),
            tag: String::new(),
            registration_password: String::new(),
            repeated_password: String::new(),
            new_password: String::new(),
            repeated_new_password: String::new(),
            code: String::new(),
            birth_date: Calendar::current().date_by_adding(CalendarComponent::Year, -18, &now).unwrap_or(now),
            tag_availability: AuthStepFlowModelTagAvailability::Idle,
            is_submitting: false,
            is_resending: false,
            code_length: VERIFICATION_CODE.maximum_value() as i64,
            registration_mode: RegistrationPolicyProbeMode::Open,
            invite_code: String::new(),
            moves_forward: true,
            registration_started_from_login: false,
            is_opening_registration: false,
            password_reset_started_from_step: AuthStepFlowModelStep::LoginPassword,
            can_reuse_verification_code: false,
            field_errors: HashMap::new(),
            server_error_texts: HashMap::new(),
            tag_check_generation: 0,
            registration_policy_task: None,
        }
    }

    pub fn moves_forward(&self) -> bool {
        self.moves_forward
    }

    pub fn calling_code(&self) -> &str {
        &self.selected_country.calling_code
    }

    pub fn composed_phone_number(&self) -> String {
        AuthInteractionsStore::compose_phone_number(self.calling_code(), &self.phone_number)
    }

    /// canon length rule plus the country own pattern, eg +998 90123 passes 7 to 15 digits but is
    /// short for uzbekistan
    /// no pattern means no extra check
    pub fn phone_is_complete(&self) -> bool {
        if !Self::is_valid_phone(&self.composed_phone_number()) {
            return false;
        }
        let required = PhoneCountryCatalog::national_digit_count(&self.selected_country);
        self.phone_number.chars().filter(|character| character.is_numeric()).count() >= required
    }

    pub fn requires_invite(&self) -> bool {
        self.registration_mode == RegistrationPolicyProbeMode::InviteOnly
    }

    pub fn total_steps(&self) -> i64 {
        match self.purpose {
            AuthStepFlowModelPurpose::Login => 2,
            AuthStepFlowModelPurpose::PasswordReset => 5,
            AuthStepFlowModelPurpose::Registration => {
                if self.requires_invite() {
                    8
                } else {
                    7
                }
            }
        }
    }

    pub fn progress_index(&self) -> i64 {
        let shift = if self.requires_invite() { 1 } else { 0 };
        match self.step {
            AuthStepFlowModelStep::Phone => 0,
            AuthStepFlowModelStep::LoginPassword => 1,
            AuthStepFlowModelStep::RegistrationInvite => 1,
            AuthStepFlowModelStep::RegistrationName => 1 + shift,
            AuthStepFlowModelStep::RegistrationBirthDate => 2 + shift,
            AuthStepFlowModelStep::RegistrationTag => 3 + shift,
            AuthStepFlowModelStep::RegistrationPassword => 4 + shift,
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => 5 + shift,
            AuthStepFlowModelStep::VerificationCode => 6 + shift,
            AuthStepFlowModelStep::ResetCode => 2,
            AuthStepFlowModelStep::ResetPassword => 3,
            AuthStepFlowModelStep::ResetPasswordConfirmation => 4,
        }
    }

    pub fn shows_back_button(&self) -> bool {
        self.step != AuthStepFlowModelStep::Phone
    }

    pub fn animation_name(&self) -> &'static str {
        self.animation_name_for_step(self.step)
    }

    fn animation_name_for_step(&self, step: AuthStepFlowModelStep) -> &'static str {
        match step {
            AuthStepFlowModelStep::Phone => "hello",
            AuthStepFlowModelStep::LoginPassword => "quite",
            AuthStepFlowModelStep::RegistrationName => "greetings",
            AuthStepFlowModelStep::RegistrationInvite => "greetings",
            AuthStepFlowModelStep::RegistrationBirthDate => "greetings",
            AuthStepFlowModelStep::RegistrationTag => match self.tag_availability {
                AuthStepFlowModelTagAvailability::Available => "like",
                AuthStepFlowModelTagAvailability::Unavailable | AuthStepFlowModelTagAvailability::Invalid(_) => "no",
                _ => "what",
            },
            AuthStepFlowModelStep::RegistrationPassword => "work",
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => "help",
            AuthStepFlowModelStep::VerificationCode => "gift",
            AuthStepFlowModelStep::ResetCode => "gift",
            AuthStepFlowModelStep::ResetPassword => "work",
            AuthStepFlowModelStep::ResetPasswordConfirmation => "help",
        }
    }

    pub fn title_key(step: AuthStepFlowModelStep) -> &'static str {
        match step {
            AuthStepFlowModelStep::Phone => "auth_phone_step_title",
            AuthStepFlowModelStep::LoginPassword => "auth_login_password_step_title",
            AuthStepFlowModelStep::RegistrationName => "auth_name_step_title",
            AuthStepFlowModelStep::RegistrationInvite => "auth_invite_step_title",
            AuthStepFlowModelStep::RegistrationBirthDate => "auth_birth_date_step_title",
            AuthStepFlowModelStep::RegistrationTag => "auth_tag_step_title",
            AuthStepFlowModelStep::RegistrationPassword => "auth_password_step_title",
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => "auth_repeat_password_step_title",
            AuthStepFlowModelStep::VerificationCode => "send_code",
            AuthStepFlowModelStep::ResetCode => "send_code",
            AuthStepFlowModelStep::ResetPassword => "auth_new_password_step_title",
            AuthStepFlowModelStep::ResetPasswordConfirmation => "auth_repeat_password_step_title",
        }
    }

    pub fn subtitle_key(step: AuthStepFlowModelStep) -> &'static str {
        match step {
            AuthStepFlowModelStep::Phone => "auth_phone_step_subtitle",
            AuthStepFlowModelStep::LoginPassword => "auth_login_password_step_subtitle",
            AuthStepFlowModelStep::RegistrationName => "auth_name_step_subtitle",
            AuthStepFlowModelStep::RegistrationInvite => "auth_invite_step_subtitle",
            AuthStepFlowModelStep::RegistrationBirthDate => "auth_birth_date_step_subtitle",
            AuthStepFlowModelStep::RegistrationTag => "auth_tag_step_subtitle",
            AuthStepFlowModelStep::RegistrationPassword => "auth_password_step_subtitle",
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => "auth_repeat_password_step_subtitle",
            AuthStepFlowModelStep::VerificationCode => "send_code_subtitle",
            AuthStepFlowModelStep::ResetCode => "send_code_subtitle",
            AuthStepFlowModelStep::ResetPassword => "auth_new_password_step_subtitle",
            AuthStepFlowModelStep::ResetPasswordConfirmation => "auth_repeat_password_step_subtitle",
        }
    }

    pub fn button_title_key(&self) -> &'static str {
        match self.step {
            AuthStepFlowModelStep::LoginPassword => "signin",
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => "auth_get_code",
            AuthStepFlowModelStep::VerificationCode => "signup",
            AuthStepFlowModelStep::ResetPasswordConfirmation => "auth_reset_password_submit",
            _ => "next",
        }
    }

    pub fn can_continue(&self) -> bool {
        if self.is_submitting {
            return false;
        }
        match self.step {
            AuthStepFlowModelStep::Phone => {
                if self.purpose == AuthStepFlowModelPurpose::Registration
                    && self.registration_mode == RegistrationPolicyProbeMode::Closed
                {
                    return false;
                }
                self.phone_is_complete()
            }
            AuthStepFlowModelStep::RegistrationInvite => {
                !rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines(&self.invite_code).is_empty()
            }
            AuthStepFlowModelStep::LoginPassword => !self.login_password.is_empty(),
            AuthStepFlowModelStep::RegistrationName => Self::name_validation_key(&self.name).is_none(),
            AuthStepFlowModelStep::RegistrationBirthDate => Self::birth_date_validation_key(self.birth_date).is_none(),
            AuthStepFlowModelStep::RegistrationTag => {
                self.tag_availability == AuthStepFlowModelTagAvailability::Available
            }
            AuthStepFlowModelStep::RegistrationPassword => {
                Self::password_validation_key(&self.registration_password).is_none()
            }
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => {
                !self.repeated_password.is_empty() && self.repeated_password == self.registration_password
            }
            AuthStepFlowModelStep::VerificationCode | AuthStepFlowModelStep::ResetCode => {
                self.code_is_complete(&self.code)
            }
            AuthStepFlowModelStep::ResetPassword => Self::password_validation_key(&self.new_password).is_none(),
            AuthStepFlowModelStep::ResetPasswordConfirmation => {
                !self.repeated_new_password.is_empty() && self.repeated_new_password == self.new_password
            }
        }
    }

    /// canon rule while the length is the canon one
    /// a length the server announced otherwise is checked as that many ascii digits
    fn code_follows_canon_rule(code_length: i64) -> bool {
        code_length == VERIFICATION_CODE.maximum_value() as i64
    }

    pub fn code_is_complete(&self, value: &str) -> bool {
        Self::code_is_complete_for_length(value, self.code_length)
    }

    fn code_is_complete_for_length(value: &str, code_length: i64) -> bool {
        if Self::code_follows_canon_rule(code_length) {
            return VERIFICATION_CODE.is_valid(value);
        }
        value.chars().count() as i64 == code_length && value.chars().all(|character| character.is_ascii_digit())
    }

    pub fn code_input(&self, raw: &str) -> String {
        if Self::code_follows_canon_rule(self.code_length) {
            return VERIFICATION_CODE.input(raw);
        }
        raw.chars()
            .filter(char::is_ascii_digit)
            .take(self.code_length.max(0) as usize)
            .collect()
    }

    pub fn error_key(&self, step: AuthStepFlowModelStep) -> Option<&str> {
        self.field_errors.get(&step).map(String::as_str)
    }

    /// ready text under the field of the current step
    /// a greyed out continue must say why so the live check speaks before any tap
    /// only too short on the name waits for continue
    pub fn inline_error_text(&self) -> Option<String> {
        let stored = self.field_errors.get(&self.step);
        match self.step {
            AuthStepFlowModelStep::RegistrationName => {
                if !self.name.is_empty() {
                    let text = Self::name_limit_text(&self.name)
                        .or_else(|| Self::live_schema_text(&AuthFieldRules::name(), &self.name));
                    if text.is_some() {
                        return text;
                    }
                }
                if stored.is_some() {
                    let text = AuthFieldRules::name().schema.as_ref().and_then(|schema| schema.error_text(&self.name));
                    if text.is_some() {
                        return text;
                    }
                }
            }
            AuthStepFlowModelStep::RegistrationTag => {
                if !self.tag.is_empty() {
                    let text = AuthFieldRules::tag()
                        .schema
                        .as_ref()
                        .and_then(|schema| schema.error_text_for_raw_value(&self.tag));
                    if text.is_some() {
                        return text;
                    }
                }
            }
            AuthStepFlowModelStep::RegistrationPassword => {
                if let Some(text) = Self::live_or_stored_password_text(&self.registration_password, stored.is_some()) {
                    return Some(text);
                }
            }
            AuthStepFlowModelStep::ResetPassword => {
                if let Some(text) = Self::live_or_stored_password_text(&self.new_password, stored.is_some()) {
                    return Some(text);
                }
            }
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => {
                if Self::confirmation_differs(&self.repeated_password, &self.registration_password) {
                    return Some(tr("auth_password_mismatch"));
                }
            }
            AuthStepFlowModelStep::ResetPasswordConfirmation => {
                if Self::confirmation_differs(&self.repeated_new_password, &self.new_password) {
                    return Some(tr("auth_password_mismatch"));
                }
            }
            _ => {}
        }
        if let Some((key, text)) = self.server_error_texts.get(&self.step) {
            if Some(key) == stored {
                return Some(text.clone());
            }
        }
        stored.map(|key| tr(key))
    }

    fn live_or_stored_password_text(value: &str, has_stored_error: bool) -> Option<String> {
        let rule = AuthFieldRules::registration_password();
        if !value.is_empty() {
            let text = Self::live_schema_text(&rule, value);
            if text.is_some() {
                return text;
            }
        }
        if has_stored_error {
            return rule.schema.as_ref().and_then(|schema| schema.error_text(value));
        }
        None
    }

    fn live_schema_text(rule: &RiseFieldRule, value: &str) -> Option<String> {
        let violation = rule.schema.as_ref()?.check(value).err()?;
        if matches!(violation.kind, ViolationKind::TooShort { .. }) {
            return None;
        }
        <TextSchema as ValidationSchemaCanonRules>::text(&violation)
    }

    fn name_limit_text(value: &str) -> Option<String> {
        let rule = AuthFieldRules::name();
        let violation = rule.schema.as_ref()?.check(value).err()?;
        let ViolationKind::TooLong { maximum, .. } = violation.kind else {
            return None;
        };
        Some(tr_args("form_max_length", &[FormatArg::Int(maximum as i64)]))
    }

    /// mismatch only once the repeat is as long as the original, before that its still being typed
    fn confirmation_differs(repeated: &str, original: &str) -> bool {
        !repeated.is_empty() && repeated.chars().count() >= original.chars().count() && repeated != original
    }

    pub fn update_phone(&mut self, value: &str, cx: &mut Context<Self>) {
        self.phone_number = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::Phone);
        cx.notify();
    }

    pub fn update_country(&mut self, country: &PhoneInputCountry, cx: &mut Context<Self>) {
        self.selected_country = country.clone();
        self.phone_number = PhoneInputUi::national_digits(&self.phone_number, country);
        self.field_errors.remove(&AuthStepFlowModelStep::Phone);
        cx.notify();
    }

    pub fn update_login_password(&mut self, value: &str, cx: &mut Context<Self>) {
        self.login_password = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::LoginPassword);
        cx.notify();
    }

    pub fn update_name(&mut self, value: &str, cx: &mut Context<Self>) {
        self.name = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::RegistrationName);
        cx.notify();
    }

    pub fn update_invite_code(&mut self, value: &str, cx: &mut Context<Self>) {
        self.invite_code = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::RegistrationInvite);
        cx.notify();
    }

    pub fn load_registration_policy(&mut self, cx: &mut Context<Self>) {
        if self.purpose != AuthStepFlowModelPurpose::Registration {
            return;
        }
        self.registration_policy_task = Some(cx.spawn(async move |this, cx| {
            let Some(mode) = RegistrationPolicyProbe::read().await else {
                return;
            };
            let _ = this.update(cx, |model, cx| {
                model.registration_mode = mode;
                if mode == RegistrationPolicyProbeMode::Closed {
                    let text = tr("registration_closed");
                    AuthInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.present_localized_auth_flow_error(&text, cx));
                }
                cx.notify();
            });
        }));
    }

    pub fn handle_invite_rejected(&mut self, error_key: &str, cx: &mut Context<Self>) {
        self.registration_mode = RegistrationPolicyProbeMode::InviteOnly;
        self.can_reuse_verification_code = self.code_is_complete(&self.code);
        self.field_errors.insert(AuthStepFlowModelStep::RegistrationInvite, error_key.to_owned());
        self.move_to(AuthStepFlowModelStep::RegistrationInvite, false, cx);
    }

    pub fn invite_rejection_key(code: Option<&str>) -> Option<&'static str> {
        let value = code?;
        if value == ErrorCode::RegistrationInviteRequired.wire() || value == ErrorCode::RegistrationInviteInvalid.wire()
        {
            return CanonErrors::entry(code)?.i18n_key;
        }
        None
    }

    pub fn update_registration_password(&mut self, value: &str, cx: &mut Context<Self>) {
        self.registration_password = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::RegistrationPassword);
        if !self.repeated_password.is_empty() {
            self.field_errors.remove(&AuthStepFlowModelStep::RegistrationPasswordConfirmation);
        }
        cx.notify();
    }

    pub fn update_repeated_password(&mut self, value: &str, cx: &mut Context<Self>) {
        self.repeated_password = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::RegistrationPasswordConfirmation);
        cx.notify();
    }

    pub fn update_code(&mut self, value: &str, cx: &mut Context<Self>) {
        self.code = self.code_input(value);
        self.field_errors.remove(&AuthStepFlowModelStep::VerificationCode);
        self.field_errors.remove(&AuthStepFlowModelStep::ResetCode);
        self.server_error_texts.remove(&AuthStepFlowModelStep::VerificationCode);
        self.server_error_texts.remove(&AuthStepFlowModelStep::ResetCode);
        cx.notify();
    }

    pub fn update_new_password(&mut self, value: &str, cx: &mut Context<Self>) {
        self.new_password = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::ResetPassword);
        if !self.repeated_new_password.is_empty() {
            self.field_errors.remove(&AuthStepFlowModelStep::ResetPasswordConfirmation);
        }
        cx.notify();
    }

    pub fn update_repeated_new_password(&mut self, value: &str, cx: &mut Context<Self>) {
        self.repeated_new_password = value.to_owned();
        self.field_errors.remove(&AuthStepFlowModelStep::ResetPasswordConfirmation);
        cx.notify();
    }

    pub fn update_tag(&mut self, raw_value: &str, cx: &mut Context<Self>) {
        self.tag = AuthFieldRules::tag()
            .schema
            .as_ref()
            .map_or_else(|| raw_value.to_owned(), |schema| schema.input(raw_value));
        self.tag_check_generation += 1;
        let generation = self.tag_check_generation;
        AuthInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.cancel_registration_tag_check(cx));
        cx.notify();

        if self.tag.is_empty() {
            self.tag_availability = AuthStepFlowModelTagAvailability::Idle;
            self.field_errors.remove(&AuthStepFlowModelStep::RegistrationTag);
            return;
        }

        if let Some(validation_key) = Self::tag_validation_key(&self.tag) {
            self.tag_availability = AuthStepFlowModelTagAvailability::Invalid(validation_key.to_owned());
            self.field_errors.insert(AuthStepFlowModelStep::RegistrationTag, validation_key.to_owned());
            return;
        }

        self.field_errors.remove(&AuthStepFlowModelStep::RegistrationTag);
        self.tag_availability = AuthStepFlowModelTagAvailability::Checking;
        let this = cx.weak_entity();
        let tag = self.tag.clone();
        let on_finish = Rc::new(move |result: Result<AuthCheckTagResponse, &anyhow::Error>, cx: &mut App| {
            let _ = this.update(cx, |model, cx| {
                if generation == model.tag_check_generation {
                    model.apply_tag_check(result, cx);
                }
            });
        });
        AuthInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.schedule_registration_tag_check(&tag, on_finish, cx));
    }

    fn apply_tag_check(&mut self, result: Result<AuthCheckTagResponse, &anyhow::Error>, cx: &mut Context<Self>) {
        match result {
            Ok(response) => {
                let response_tag =
                    response.normalized_tag.as_deref().map_or_else(|| self.tag.clone(), str::to_lowercase);
                if response_tag != self.tag {
                    return;
                }
                let is_available = response.available.or(response.exists.map(|exists| !exists));
                if response.success == Some(false) {
                    self.field_errors.remove(&AuthStepFlowModelStep::RegistrationTag);
                    self.tag_availability = AuthStepFlowModelTagAvailability::Failed;
                } else if is_available == Some(true) {
                    self.field_errors.remove(&AuthStepFlowModelStep::RegistrationTag);
                    self.tag_availability = AuthStepFlowModelTagAvailability::Available;
                } else if is_available == Some(false) {
                    self.tag_availability = AuthStepFlowModelTagAvailability::Unavailable;
                    self.field_errors.insert(AuthStepFlowModelStep::RegistrationTag, "tag_already_exists".to_owned());
                } else {
                    self.field_errors.remove(&AuthStepFlowModelStep::RegistrationTag);
                    self.tag_availability = AuthStepFlowModelTagAvailability::Failed;
                }
            }
            Err(_) => {
                self.field_errors.remove(&AuthStepFlowModelStep::RegistrationTag);
                self.tag_availability = AuthStepFlowModelTagAvailability::Failed;
            }
        }
        cx.notify();
    }

    pub fn validate_current_step(&mut self, cx: &mut Context<Self>) -> bool {
        let error_key: Option<&'static str> = match self.step {
            AuthStepFlowModelStep::Phone => {
                if self.phone_is_complete()
                    && PhoneCountryCatalog::is_phone_number_valid(&self.composed_phone_number())
                {
                    None
                } else {
                    Some("auth_phone_invalid")
                }
            }
            AuthStepFlowModelStep::LoginPassword => {
                AuthFieldRules::login_password().schema.as_ref().and_then(|schema| schema.error_key(&self.login_password))
            }
            AuthStepFlowModelStep::RegistrationName => Self::name_validation_key(&self.name),
            AuthStepFlowModelStep::RegistrationInvite => {
                if rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines(&self.invite_code).is_empty()
                {
                    Some("auth_invite_required")
                } else {
                    None
                }
            }
            AuthStepFlowModelStep::RegistrationBirthDate => Self::birth_date_validation_key(self.birth_date),
            AuthStepFlowModelStep::RegistrationTag => Self::tag_validation_key(&self.tag).or(
                if self.tag_availability == AuthStepFlowModelTagAvailability::Available {
                    None
                } else {
                    Some("auth_tag_not_verified")
                },
            ),
            AuthStepFlowModelStep::RegistrationPassword => Self::password_validation_key(&self.registration_password),
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => {
                Self::confirmation_error_key(&self.repeated_password, &self.registration_password)
            }
            AuthStepFlowModelStep::VerificationCode | AuthStepFlowModelStep::ResetCode => {
                if self.code_is_complete(&self.code) {
                    None
                } else {
                    VERIFICATION_CODE.error_key(&self.code).or_else(|| VERIFICATION_CODE.error_key(""))
                }
            }
            AuthStepFlowModelStep::ResetPassword => Self::password_validation_key(&self.new_password),
            AuthStepFlowModelStep::ResetPasswordConfirmation => {
                Self::confirmation_error_key(&self.repeated_new_password, &self.new_password)
            }
        };
        match error_key {
            Some(key) => {
                self.field_errors.insert(self.step, key.to_owned());
            }
            None => {
                self.field_errors.remove(&self.step);
            }
        }
        self.server_error_texts.remove(&self.step);
        cx.notify();
        error_key.is_none()
    }

    fn confirmation_error_key(repeated: &str, original: &str) -> Option<&'static str> {
        if repeated.is_empty() {
            return Some("auth_password_required");
        }
        if repeated == original { None } else { Some("auth_password_mismatch") }
    }

    pub fn advance_after_local_validation(&mut self, cx: &mut Context<Self>) {
        if !self.validate_current_step(cx) {
            return;
        }
        match self.step {
            AuthStepFlowModelStep::Phone => {
                if self.purpose == AuthStepFlowModelPurpose::Login {
                    self.move_to(AuthStepFlowModelStep::LoginPassword, true, cx);
                } else if self.requires_invite() {
                    self.move_to(AuthStepFlowModelStep::RegistrationInvite, true, cx);
                } else {
                    self.move_to(AuthStepFlowModelStep::RegistrationName, true, cx);
                }
            }
            AuthStepFlowModelStep::RegistrationInvite => self.move_to(AuthStepFlowModelStep::RegistrationName, true, cx),
            AuthStepFlowModelStep::RegistrationName => {
                self.move_to(AuthStepFlowModelStep::RegistrationBirthDate, true, cx)
            }
            AuthStepFlowModelStep::RegistrationBirthDate => {
                self.move_to(AuthStepFlowModelStep::RegistrationTag, true, cx)
            }
            AuthStepFlowModelStep::RegistrationTag => self.move_to(AuthStepFlowModelStep::RegistrationPassword, true, cx),
            AuthStepFlowModelStep::RegistrationPassword => {
                self.move_to(AuthStepFlowModelStep::RegistrationPasswordConfirmation, true, cx)
            }
            AuthStepFlowModelStep::ResetCode => self.move_to(AuthStepFlowModelStep::ResetPassword, true, cx),
            AuthStepFlowModelStep::ResetPassword => {
                self.move_to(AuthStepFlowModelStep::ResetPasswordConfirmation, true, cx)
            }
            _ => {}
        }
    }

    pub fn begin_registration_from_login(
        &mut self,
        on_closed: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Option<Task<()>> {
        self.begin_registration_from_login_read_policy(RegistrationPolicyProbe::read(), on_closed, cx)
    }

    pub fn begin_registration_from_login_read_policy(
        &mut self,
        read_policy: impl Future<Output = Option<RegistrationPolicyProbeMode>> + 'static,
        on_closed: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Option<Task<()>> {
        if self.is_opening_registration {
            return None;
        }
        self.is_opening_registration = true;
        Some(cx.spawn(async move |this, cx| {
            let mode = read_policy.await;
            let _ = this.update(cx, move |model, cx| {
                model.is_opening_registration = false;
                if model.step != AuthStepFlowModelStep::LoginPassword {
                    return;
                }
                if let Some(mode) = mode {
                    model.registration_mode = mode;
                }
                if model.registration_mode == RegistrationPolicyProbeMode::Closed {
                    on_closed(cx);
                    let text = tr("registration_closed");
                    AuthInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.present_localized_auth_flow_error(&text, cx));
                    cx.notify();
                    return;
                }
                model.purpose = AuthStepFlowModelPurpose::Registration;
                model.registration_started_from_login = true;
                model.field_errors.remove(&AuthStepFlowModelStep::LoginPassword);
                let next = if model.requires_invite() {
                    AuthStepFlowModelStep::RegistrationInvite
                } else {
                    AuthStepFlowModelStep::RegistrationName
                };
                model.move_to(next, true, cx);
            });
        }))
    }

    pub fn begin_password_reset(&mut self, cx: &mut Context<Self>) {
        self.purpose = AuthStepFlowModelPurpose::PasswordReset;
        self.password_reset_started_from_step = self.step;
        self.new_password.clear();
        self.repeated_new_password.clear();
        self.code.clear();
        self.field_errors.remove(&AuthStepFlowModelStep::ResetCode);
        self.field_errors.remove(&AuthStepFlowModelStep::ResetPassword);
        self.field_errors.remove(&AuthStepFlowModelStep::ResetPasswordConfirmation);
        cx.notify();
    }

    pub fn cancel_password_reset(&mut self, cx: &mut Context<Self>) {
        if self.purpose != AuthStepFlowModelPurpose::PasswordReset {
            return;
        }
        self.purpose = AuthStepFlowModelPurpose::Login;
        self.code.clear();
        self.new_password.clear();
        self.repeated_new_password.clear();
        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.cancel_password_reset(cx));
        self.move_to(self.password_reset_started_from_step, false, cx);
    }

    pub fn finish_password_reset(&mut self, signed_in: bool, cx: &mut Context<Self>) {
        self.purpose = AuthStepFlowModelPurpose::Login;
        self.code.clear();
        self.new_password.clear();
        self.repeated_new_password.clear();
        self.login_password.clear();
        self.field_errors.remove(&AuthStepFlowModelStep::LoginPassword);
        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.cancel_password_reset(cx));
        cx.notify();
        if signed_in {
            return;
        }
        self.move_to(AuthStepFlowModelStep::LoginPassword, false, cx);
    }

    pub fn move_to_reset_code(&mut self, cx: &mut Context<Self>) {
        self.code.clear();
        self.move_to(AuthStepFlowModelStep::ResetCode, true, cx);
    }

    pub fn handle_reset_code_rejected(&mut self, error_key: &str, text: Option<String>, cx: &mut Context<Self>) {
        self.store_code_rejection(AuthStepFlowModelStep::ResetCode, error_key, text);
        self.move_to(AuthStepFlowModelStep::ResetCode, false, cx);
    }

    pub fn handle_verification_code_rejected(&mut self, error_key: &str, text: Option<String>, cx: &mut Context<Self>) {
        self.store_code_rejection(AuthStepFlowModelStep::VerificationCode, error_key, text);
        cx.notify();
    }

    fn store_code_rejection(&mut self, step: AuthStepFlowModelStep, error_key: &str, text: Option<String>) {
        self.code.clear();
        self.field_errors.insert(step, error_key.to_owned());
        match text {
            Some(text) => {
                self.server_error_texts.insert(step, (error_key.to_owned(), text));
            }
            None => {
                self.server_error_texts.remove(&step);
            }
        }
    }

    /// the number itself was refused (country without codes, a number its country cant have)
    /// everything typed stays, the phone step shows why and takes another number
    pub fn handle_phone_rejected(&mut self, error_key: &str, cx: &mut Context<Self>) {
        if self.purpose == AuthStepFlowModelPurpose::PasswordReset {
            self.purpose = AuthStepFlowModelPurpose::Login;
            self.code.clear();
            self.new_password.clear();
            self.repeated_new_password.clear();
            AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.cancel_password_reset(cx));
        }
        self.registration_started_from_login = false;
        self.field_errors.insert(AuthStepFlowModelStep::Phone, error_key.to_owned());
        self.move_to(AuthStepFlowModelStep::Phone, false, cx);
    }

    pub fn restart_code_entry(&mut self, code_length: i64, cx: &mut Context<Self>) {
        self.code_length = code_length;
        self.code.clear();
        self.field_errors.remove(&AuthStepFlowModelStep::VerificationCode);
        self.field_errors.remove(&AuthStepFlowModelStep::ResetCode);
        cx.notify();
    }

    pub fn use_login_from_phone(&mut self, cx: &mut Context<Self>) {
        self.purpose = AuthStepFlowModelPurpose::Login;
        self.registration_started_from_login = false;
        self.field_errors.remove(&AuthStepFlowModelStep::Phone);
        cx.notify();
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        match self.step {
            AuthStepFlowModelStep::LoginPassword => self.move_to(AuthStepFlowModelStep::Phone, false, cx),
            AuthStepFlowModelStep::RegistrationInvite => self.leave_registration_start(cx),
            AuthStepFlowModelStep::RegistrationName => {
                if self.requires_invite() {
                    self.move_to(AuthStepFlowModelStep::RegistrationInvite, false, cx);
                } else {
                    self.leave_registration_start(cx);
                }
            }
            AuthStepFlowModelStep::RegistrationBirthDate => {
                self.move_to(AuthStepFlowModelStep::RegistrationName, false, cx)
            }
            AuthStepFlowModelStep::RegistrationTag => {
                AuthInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.cancel_registration_tag_check(cx));
                self.move_to(AuthStepFlowModelStep::RegistrationBirthDate, false, cx);
            }
            AuthStepFlowModelStep::RegistrationPassword => self.move_to(AuthStepFlowModelStep::RegistrationTag, false, cx),
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => {
                self.move_to(AuthStepFlowModelStep::RegistrationPassword, false, cx)
            }
            AuthStepFlowModelStep::VerificationCode => {
                self.move_to(AuthStepFlowModelStep::RegistrationPasswordConfirmation, false, cx)
            }
            AuthStepFlowModelStep::ResetCode => self.cancel_password_reset(cx),
            AuthStepFlowModelStep::ResetPassword => self.move_to(AuthStepFlowModelStep::ResetCode, false, cx),
            AuthStepFlowModelStep::ResetPasswordConfirmation => {
                self.move_to(AuthStepFlowModelStep::ResetPassword, false, cx)
            }
            AuthStepFlowModelStep::Phone => {}
        }
    }

    fn leave_registration_start(&mut self, cx: &mut Context<Self>) {
        if self.registration_started_from_login {
            self.purpose = AuthStepFlowModelPurpose::Login;
            self.registration_started_from_login = false;
            self.move_to(AuthStepFlowModelStep::LoginPassword, false, cx);
        } else {
            self.move_to(AuthStepFlowModelStep::Phone, false, cx);
        }
    }

    pub fn move_to_verification_code(&mut self, cx: &mut Context<Self>) {
        self.can_reuse_verification_code = false;
        self.code.clear();
        self.move_to(AuthStepFlowModelStep::VerificationCode, true, cx);
    }

    pub fn resume_reusable_verification_code(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_reuse_verification_code || !self.code_is_complete(&self.code) {
            return false;
        }
        self.can_reuse_verification_code = false;
        self.move_to(AuthStepFlowModelStep::VerificationCode, true, cx);
        true
    }

    pub fn handle_registration_tag_conflict(&mut self, cx: &mut Context<Self>) {
        self.tag_check_generation += 1;
        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.cancel_registration_tag_check(cx));
        self.can_reuse_verification_code = self.code_is_complete(&self.code);
        self.tag_availability = AuthStepFlowModelTagAvailability::Unavailable;
        self.field_errors.insert(AuthStepFlowModelStep::RegistrationTag, "tag_already_exists".to_owned());
        self.move_to(AuthStepFlowModelStep::RegistrationTag, false, cx);
    }

    pub fn is_valid_phone(value: &str) -> bool {
        PHONE.is_valid(value)
    }

    pub fn name_validation_key(value: &str) -> Option<&'static str> {
        AuthFieldRules::name().schema.as_ref().and_then(|schema| schema.error_key(value))
    }

    pub fn selectable_birth_dates() -> RangeInclusive<Date> {
        let today = RiseClock::now();
        let earliest = Calendar::current().date_from_components(&DateComponents::ymd(1901, 1, 1)).unwrap_or(today);
        earliest..=today
    }

    pub fn update_birth_date(&mut self, value: Date, cx: &mut Context<Self>) {
        self.birth_date = value;
        self.field_errors.remove(&AuthStepFlowModelStep::RegistrationBirthDate);
        cx.notify();
    }

    pub fn birth_date_validation_key(value: Date) -> Option<&'static str> {
        let today = RiseClock::now();
        if value > today {
            return Some("limit_error_too_young");
        }
        let years = Calendar::current()
            .date_components_between(&[CalendarComponent::Year], &value, &today)
            .year
            .unwrap_or(0);
        if years >= RiseLimits::service_value("minimum_registration_age_years") {
            None
        } else {
            Some("limit_error_too_young")
        }
    }

    pub fn wire_birth_date(value: Date) -> String {
        let calendar = Calendar::current();
        format!(
            "{:04}-{:02}-{:02}",
            calendar.component(CalendarComponent::Year, &value),
            calendar.component(CalendarComponent::Month, &value),
            calendar.component(CalendarComponent::Day, &value),
        )
    }

    pub fn password_validation_key(value: &str) -> Option<&'static str> {
        AuthFieldRules::registration_password().schema.as_ref().and_then(|schema| schema.error_key(value))
    }

    pub fn tag_validation_key(value: &str) -> Option<&'static str> {
        AuthFieldRules::tag().schema.as_ref().and_then(|schema| schema.error_key_for_raw_value(value))
    }

    fn move_to(&mut self, new_step: AuthStepFlowModelStep, forward: bool, cx: &mut Context<Self>) {
        self.moves_forward = forward;
        self.step = new_step;
        cx.notify();
    }
}

/// on_auth and on_close callbacks
pub type AuthStepFlowAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// one sheet slot instead of two presented flags
/// recovery hands the reset explainer straight to the telegram sheet, two flags cant say close
/// one then open the other
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthStepFlowSheetRoute {
    PasswordResetIntro,
    Telegram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthStepFlowFieldTail {
    tag: AuthStepFlowModelTagAvailability,
    error: Option<String>,
}

pub struct AuthStepFlow {
    on_auth: AuthStepFlowAction,
    on_close: Option<AuthStepFlowAction>,
    on_leave: Option<AuthStepFlowAction>,
    model: Entity<AuthStepFlowModel>,
    services: Entity<AuthServicesStore>,
    sheet_route: Option<AuthStepFlowSheetRoute>,
    displayed_sheet_route: AuthStepFlowSheetRoute,
    pending_sheet_route: Option<AuthStepFlowSheetRoute>,
    field_focus_request: i64,
    reset_sign_in_id: Rc<RefCell<Option<Uuid>>>,
    alert_purpose: Option<AuthStepFlowModelPurpose>,
    is_alert_presented: bool,
    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    presented_step: AuthStepFlowModelStep,
    page: Presence<AuthStepFlowModelStep>,
    departing_pages: Vec<Presence<AuthStepFlowModelStep>>,
    this: WeakEntity<Self>,
    _subscriptions: Vec<Subscription>,
}

impl AuthStepFlow {
    const STEP_COLUMN_WIDTH: f32 = 500.0;
    const STEP_COLUMN_PADDING: f32 = 22.0;

    pub fn new(
        purpose: AuthStepFlowModelPurpose,
        on_auth: AuthStepFlowAction,
        on_close: Option<AuthStepFlowAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = cx.new(|cx| {
            let mut model = AuthStepFlowModel::new(purpose);
            model.load_registration_policy(cx);
            model
        });
        let services = AuthServicesStore::shared(cx);
        let step = model.read(cx).step;
        let subscriptions = vec![
            cx.observe_in(&model, window, |flow, _, window, cx| {
                flow.sync_step_transition(window, cx);
                cx.notify();
            }),
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.subscribe(&services, |_, _, _: &AuthFlowChanged, cx| cx.notify()),
        ];
        Self {
            on_auth,
            on_close,
            on_leave: None,
            model,
            services,
            sheet_route: None,
            displayed_sheet_route: AuthStepFlowSheetRoute::PasswordResetIntro,
            pending_sheet_route: None,
            field_focus_request: 0,
            reset_sign_in_id: Rc::new(RefCell::new(None)),
            alert_purpose: None,
            is_alert_presented: false,
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            presented_step: step,
            page: Presence::new(Some(step)),
            departing_pages: Vec::new(),
            this: cx.weak_entity(),
            _subscriptions: subscriptions,
        }
    }

    /// back button and esc on the first step leave the pushed flow
    pub fn with_on_leave(mut self, on_leave: Option<AuthStepFlowAction>) -> Self {
        self.on_leave = on_leave;
        self
    }

    /// back button shows on every step but the first, and on the first when the flow was pushed
    fn can_go_back(&self, cx: &App) -> bool {
        self.model.read(cx).shows_back_button() || self.on_leave.is_some()
    }

    pub fn model(&self) -> &Entity<AuthStepFlowModel> {
        &self.model
    }

    fn sync_step_transition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (step, moves_forward) = {
            let model = self.model.read(cx);
            (model.step, model.moves_forward())
        };
        if step == self.presented_step {
            return;
        }
        self.presented_step = step;

        let (entering_edge, leaving_edge) =
            if moves_forward { (Edge::Trailing, Edge::Leading) } else { (Edge::Leading, Edge::Trailing) };
        let transition = Transition::asymmetric(
            Transition::move_edge(entering_edge).combined(Transition::opacity()),
            Transition::move_edge(leaving_edge).combined(Transition::opacity()),
        );
        let now = RiseClock::monotonic();
        let mut leaving = std::mem::replace(&mut self.page, Presence::new(None).with_transition(transition.clone()));
        leaving.set_transition(transition);
        leaving.set_in(None, ease_in_out(0.28), now, cx);
        self.departing_pages.push(leaving);
        self.page.set_in(Some(step), ease_in_out(0.28), now, cx);

        if step == AuthStepFlowModelStep::RegistrationBirthDate {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn step_back_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Self::dismiss_keyboard(&self.focus_handle, window, cx);
        if !self.model.read(cx).shows_back_button() {
            if let Some(on_leave) = self.on_leave.clone() {
                on_leave(window, cx);
            }
            return;
        }
        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.cancel_verification_code_entry(cx));
        self.model.update(cx, |model, cx| model.go_back(cx));
    }

    /// esc goes back through the steps then out of the flow if it can be closed
    fn escape_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sheet_route.is_some() || self.model.read(cx).is_submitting {
            return;
        }
        if self.can_go_back(cx) {
            self.step_back_requested(window, cx);
        } else if let Some(on_close) = self.on_close.clone() {
            on_close(window, cx);
        }
    }

    fn back_action(&self) -> impl Fn(&mut Window, &mut App) + 'static {
        let this = self.this.clone();
        move |window, cx| {
            let _ = this.update(cx, |flow, cx| flow.step_back_requested(window, cx));
        }
    }

    fn escape_action(&self) -> impl Fn(&mut Window, &mut App) + 'static {
        let this = self.this.clone();
        move |window, cx| {
            let _ = this.update(cx, |flow, cx| flow.escape_requested(window, cx));
        }
    }

    fn continue_action(&self) -> impl Fn(&mut Window, &mut App) + 'static {
        let this = self.this.clone();
        move |window, cx| {
            let _ = this.update(cx, |flow, cx| flow.continue_tapped(window, cx));
        }
    }

    fn edge_swipe_back(&self) -> impl IntoElement {
        let this = self.this.clone();
        let model = self.model.clone();
        div().absolute().left_0().top_0().bottom_0().w(px(20.0)).gesture(
            "auth-step-edge-swipe",
            DragGesture::new(10.0).on_ended(move |value, window, cx| {
                let dx = f32::from(value.translation.width);
                let dy = f32::from(value.translation.height);
                let predicted = f32::from(value.predicted_end_translation.width);
                if model.read(cx).is_submitting || dx <= dy.abs() || !(dx > 60.0 || predicted > 140.0) {
                    return;
                }
                let _ = this.update(cx, |flow, cx| flow.step_back_requested(window, cx));
            }),
        )
    }

    fn progress_header(&self, theme: &AppTheme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (total_steps, progress_index, shows_back_button, is_submitting) = {
            let model = self.model.read(cx);
            (model.total_steps(), model.progress_index(), self.can_go_back(cx), model.is_submitting)
        };
        let now = RiseClock::monotonic();
        let idle = theme.border._200.opacity(0.7);
        let segments = (0..total_steps)
            .map(|index| {
                let target = if index <= progress_index { 1.0_f32 } else { 0.0 };
                let id = gpui::ElementId::NamedInteger(SharedString::new_static("auth-progress-segment"), index as u64);
                let progress = window.use_keyed_state(id, cx, |_, _| Animated::new(target));
                if *progress.read(cx).target() != target {
                    progress.update(cx, |progress, cx| progress.set_in(target, ease_in_out(0.25), now, cx));
                }
                let value = progress.read(cx).frame_value(now, window);
                div().flex_1().h(px(4.0)).rounded_full().bg(idle.blend(theme.primary._100.opacity(value)))
            })
            .collect::<Vec<_>>();

        div()
            .relative()
            .w_full()
            .h(px(54.0))
            .px(px(18.0))
            .pt(px(10.0))
            .pb(px(2.0))
            .flex()
            .items_center()
            .justify_center()
            .child(div().flex().flex_row().gap(px(5.0)).w_full().max_w(px(190.0)).children(segments))
            .when(shows_back_button, |header| {
                header.child(
                    div().absolute().left(px(18.0)).top(px(10.0)).child(
                        LiquidGlassButton::new("auth-step-back", LiquidGlassButtonShape::Circle)
                            .size(40.0)
                            .disabled(is_submitting)
                            .accessibility_label(Some(tr("back")))
                            .on_press(self.back_action())
                            .children(IconUi::sized("chevron.backward", px(15.0), theme.text.primary, false)),
                    ),
                )
            })
            .when_some(self.on_close.clone(), |header, on_close| {
                header.child(
                    div().absolute().right(px(18.0)).top(px(10.0)).child(
                        LiquidGlassButton::new("auth-step-close", LiquidGlassButtonShape::Circle)
                            .size(40.0)
                            .disabled(is_submitting)
                            .accessibility_label(Some(tr("close")))
                            .on_press(move |window, cx| on_close(window, cx))
                            .children(IconUi::sized("xmark", px(14.0), theme.text.primary, false)),
                    ),
                )
            })
            .into_any_element()
    }

    fn step_page(&self, step: AuthStepFlowModelStep, theme: &AppTheme, is_compact: bool, cx: &App) -> Div {
        let animation_name = self.model.read(cx).animation_name_for_step(step);
        div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(Self::STEP_COLUMN_WIDTH))
            .px(px(Self::STEP_COLUMN_PADDING))
            .pb(px(22.0))
            .child(
                div()
                    .id(gpui::ElementId::NamedInteger(
                        SharedString::from(format!("auth-step-lottie-{animation_name}")),
                        step as u64,
                    ))
                    .w_full()
                    .flex()
                    .justify_center()
                    .pt(px(if is_compact { 0.0 } else { 8.0 }))
                    .child(LottieView::new(animation_name).size(if is_compact { 112.0 } else { 168.0 }).speed(0.85)),
            )
            .child(
                MainText::new(tr(AuthStepFlowModel::title_key(step)))
                    .px(27.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .w_full()
                    .mt(px(4.0)),
            )
            .child(
                SecondaryText::new(self.subtitle_text(step, cx))
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .line_height(20.0)
                    .mt(8.0)
                    .w_full()
                    .px_2(),
            )
            .child(div().w_full().mt(px(26.0)).mb(px(14.0)).child(self.step_field(step, theme, cx)))
    }

    #[allow(clippy::too_many_arguments)]
    fn step_input(
        &self,
        step: AuthStepFlowModelStep,
        name: &'static str,
        value: &str,
        placeholder: String,
        is_secure: bool,
        on_value: fn(&mut AuthStepFlowModel, &str, &mut Context<AuthStepFlowModel>),
        cx: &App,
    ) -> InputUi {
        let model = self.model.clone();
        let focus_request = (step == self.model.read(cx).step).then_some(self.field_focus_request);
        InputUi::new(SharedString::from(format!("auth-step-input-{name}")))
            .values(HashMap::from([(name.to_owned(), value.to_owned())]))
            .errors(self.field_errors(name, step, cx))
            .name(name)
            .placeholder(placeholder)
            .is_secure(is_secure)
            .height(Some(56.0))
            .auto_focus(true)
            .focus_request(focus_request)
            .shows_secure_toggle(is_secure)
            .on_submit(self.continue_action())
            .on_cancel(self.escape_action())
            .set_value(move |_, value, _, cx| {
                model.update(cx, |model, cx| on_value(model, value, cx));
            })
    }

    fn step_field(&self, step: AuthStepFlowModelStep, theme: &AppTheme, cx: &App) -> AnyElement {
        let model = self.model.read(cx);
        match step {
            AuthStepFlowModelStep::Phone => {
                let on_value = self.model.clone();
                let on_country = self.model.clone();
                PhoneInputUi::new("auth-step-input-phone")
                    .values(HashMap::from([("number".to_owned(), model.phone_number.clone())]))
                    .errors(self.field_errors("number", step, cx))
                    .name("number")
                    .placeholder(tr("phone_number_placeholder"))
                    .padding_top(0.0)
                    .selected_country(Some(model.selected_country.clone()))
                    .auto_focus(true)
                    .on_submit(self.continue_action())
                    .on_cancel(self.escape_action())
                    .set_value(move |_, value, _, cx| {
                        on_value.update(cx, |model, cx| model.update_phone(value, cx));
                    })
                    .set_country(move |country, _, cx| {
                        on_country.update(cx, |model, cx| model.update_country(country, cx));
                    })
                    .into_any_element()
            }
            AuthStepFlowModelStep::LoginPassword => {
                let this = self.this.clone();
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(9.0))
                    .w_full()
                    .child(self.step_input(
                        step,
                        "loginPassword",
                        &model.login_password,
                        tr("password_placeholder"),
                        true,
                        AuthStepFlowModel::update_login_password,
                        cx,
                    ))
                    .children(self.field_error_line(step, cx))
                    .child(
                        plain("auth-reset-password-link")
                            .disabled(model.is_submitting)
                            .opacity(if model.is_submitting { 0.5 } else { 1.0 })
                            .mr(px(5.0))
                            .on_press(move |window, cx| {
                                let _ = this.update(cx, |flow, cx| {
                                    Self::dismiss_keyboard(&flow.focus_handle, window, cx);
                                    flow.model.update(cx, |model, cx| model.begin_password_reset(cx));
                                    flow.swap_sheet(AuthStepFlowSheetRoute::PasswordResetIntro, cx);
                                });
                            })
                            .child(
                                MainText::new(tr("auth_reset_password_link"))
                                    .px(13.0)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(theme.primary._100),
                            ),
                    )
                    .into_any_element()
            }
            AuthStepFlowModelStep::RegistrationName => self.labelled_input(
                step,
                self.step_input(
                    step,
                    "name",
                    &model.name,
                    tr("name_placeholder"),
                    false,
                    AuthStepFlowModel::update_name,
                    cx,
                ),
                false,
                cx,
            ),
            AuthStepFlowModelStep::RegistrationInvite => self.labelled_input(
                step,
                self.step_input(
                    step,
                    "inviteCode",
                    &model.invite_code,
                    tr("auth_invite_placeholder"),
                    false,
                    AuthStepFlowModel::update_invite_code,
                    cx,
                ),
                false,
                cx,
            ),
            AuthStepFlowModelStep::RegistrationBirthDate => {
                let reader = self.model.clone();
                let writer = self.model.clone();
                let error_key = model
                    .error_key(step)
                    .map(str::to_owned)
                    .or_else(|| AuthStepFlowModel::birth_date_validation_key(model.birth_date).map(str::to_owned));
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.0))
                    .w_full()
                    .child(
                        DatePicker::new(
                            "auth-birth-date-picker",
                            Binding::new(
                                move |cx: &App| reader.read(cx).birth_date,
                                move |value, _window, cx| {
                                    writer.update(cx, |model, cx| model.update_birth_date(value, cx));
                                },
                            ),
                        )
                        .range(AuthStepFlowModel::selectable_birth_dates())
                        .displayed_components(DatePickerComponents::Date),
                    )
                    .when_some(error_key, |column, key| {
                        column.child(
                            SecondaryText::new(tr(&key)).px(13.0).tac(TextAlignT::Center).w_full(),
                        )
                    })
                    .into_any_element()
            }
            AuthStepFlowModelStep::RegistrationTag => {
                let is_current = step == model.step;
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .w_full()
                    .child(
                        self.step_input(
                            step,
                            "tag",
                            &model.tag,
                            tr("auth_tag_placeholder"),
                            false,
                            AuthStepFlowModel::update_tag,
                            cx,
                        )
                        .rule(AuthFieldRules::tag()),
                    )
                    .children(self.field_error_line(step, cx))
                    .when(is_current, |column| column.children(self.tag_status(theme, cx)))
                    .into_any_element()
            }
            AuthStepFlowModelStep::RegistrationPassword | AuthStepFlowModelStep::ResetPassword => {
                let input = if step == AuthStepFlowModelStep::RegistrationPassword {
                    self.step_input(
                        step,
                        "registrationPassword",
                        &model.registration_password,
                        tr("password_placeholder"),
                        true,
                        AuthStepFlowModel::update_registration_password,
                        cx,
                    )
                } else {
                    self.step_input(
                        step,
                        "newPassword",
                        &model.new_password,
                        tr("password_placeholder"),
                        true,
                        AuthStepFlowModel::update_new_password,
                        cx,
                    )
                };
                self.labelled_input(step, input, true, cx)
            }
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => self.labelled_input(
                step,
                self.step_input(
                    step,
                    "repeatedPassword",
                    &model.repeated_password,
                    tr("repeat_password_placeholder"),
                    true,
                    AuthStepFlowModel::update_repeated_password,
                    cx,
                ),
                false,
                cx,
            ),
            AuthStepFlowModelStep::ResetPasswordConfirmation => self.labelled_input(
                step,
                self.step_input(
                    step,
                    "repeatedNewPassword",
                    &model.repeated_new_password,
                    tr("repeat_password_placeholder"),
                    true,
                    AuthStepFlowModel::update_repeated_new_password,
                    cx,
                ),
                false,
                cx,
            ),
            AuthStepFlowModelStep::VerificationCode | AuthStepFlowModelStep::ResetCode => {
                self.code_step_field(step, cx)
            }
        }
    }

    fn labelled_input(&self, step: AuthStepFlowModelStep, input: InputUi, shows_hint: bool, cx: &App) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(if shows_hint { 9.0 } else { 6.0 }))
            .w_full()
            .child(input)
            .children(self.field_error_line(step, cx))
            .when(shows_hint, |column| {
                column.child(SecondaryText::new(tr("auth_password_hint")).px(12.0).ml(5.0))
            })
            .into_any_element()
    }

    fn code_step_field(&self, step: AuthStepFlowModelStep, cx: &App) -> AnyElement {
        let model = self.model.read(cx);
        let delivery = self.services.read(cx).code_delivery().cloned();
        let is_sending = model.is_resending || model.is_submitting;
        let code_length = model.code_length;
        let on_edit = self.model.clone();
        let this_complete = self.this.clone();
        let this_resend = self.this.clone();
        let this_telegram = self.this.clone();
        let error_text = model.error_key(step).map(tr);

        div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .w_full()
            .child(AuthVerificationCodeField {
                code: model.code.clone(),
                digit_count: code_length,
                is_complete: Rc::new(move |value: &str| AuthStepFlowModel::code_is_complete_for_length(value, code_length)),
                error_text,
                external_focus_request: self.field_focus_request,
                on_edit: Rc::new(move |value: &str, _: &mut Window, cx: &mut App| {
                    on_edit.update(cx, |model, cx| model.update_code(value, cx));
                }),
                on_complete: Rc::new(move |_: &str, window: &mut Window, cx: &mut App| {
                    let _ = this_complete.update(cx, |flow, cx| flow.continue_tapped(window, cx));
                }),
                on_submit: Rc::new(self.continue_action()),
            })
            .when_some(delivery.clone().filter(AuthCodeDelivery::offers_resend), |column, delivery| {
                column.child(AuthCodeResendRow {
                    delivery,
                    is_sending,
                    on_resend: Rc::new(move |channel: Option<AuthCodeChannel>, _: &mut Window, cx: &mut App| {
                        let _ = this_resend.update(cx, |flow, cx| flow.resend_code(channel, cx));
                    }),
                    on_open_telegram: Rc::new(|_: &mut Window, cx: &mut App| {
                        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.reopen_telegram_bot(cx));
                    }),
                })
            })
            .when(delivery.is_some_and(|delivery| delivery.offers_telegram), |column| {
                column.child(AuthTelegramFallbackRow {
                    is_sending,
                    on_use: Rc::new(move |_: &mut Window, cx: &mut App| {
                        let _ = this_telegram.update(cx, |flow, cx| flow.use_telegram(cx));
                    }),
                })
            })
            .into_any_element()
    }

    fn tag_status(&self, theme: &AppTheme, cx: &App) -> Option<AnyElement> {
        let green = UIColor::system_green().resolved_color(theme.appearance);
        match self.model.read(cx).tag_availability {
            AuthStepFlowModelTagAvailability::Checking => Some(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .pl(px(5.0))
                    .child(progress_view().control_size(ControlSize::Small))
                    .child(SecondaryText::new(tr("auth_tag_checking")).px(12.0))
                    .into_any_element(),
            ),
            AuthStepFlowModelTagAvailability::Available => Some(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(7.0))
                    .pl(px(5.0))
                    .children(IconUi::sized("checkmark.circle.fill", px(17.0), green, true))
                    .child(MainText::new(tr("auth_tag_available")).px(12.0).color(green))
                    .into_any_element(),
            ),
            AuthStepFlowModelTagAvailability::Failed => {
                let model = self.model.clone();
                Some(
                    plain("auth-tag-check-retry")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(7.0))
                        .ml(px(5.0))
                        .on_press(move |_, cx| {
                            model.update(cx, |model, cx| {
                                let tag = model.tag.clone();
                                model.update_tag(&tag, cx);
                            });
                        })
                        .children(IconUi::sized("arrow.clockwise", px(12.0), theme.primary._100, false))
                        .child(
                            MainText::new(tr("auth_tag_check_failed"))
                                .px(12.0)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.primary._100),
                        )
                        .into_any_element(),
                )
            }
            _ => None,
        }
    }

    fn action_footer(&self, theme: &AppTheme, cx: &App) -> AnyElement {
        let model = self.model.read(cx);
        let alternate: Option<AnyElement> = if model.step == AuthStepFlowModelStep::LoginPassword {
            let this = self.this.clone();
            Some(self.alternate_action(
                tr("noaccount"),
                tr("signup"),
                move |window, cx| {
                    let this = this.clone();
                    let Some(flow) = this.upgrade() else {
                        return;
                    };
                    let focus_handle = flow.read(cx).focus_handle.clone();
                    Self::dismiss_keyboard(&focus_handle, window, cx);
                    let on_closed = {
                        let this = this.clone();
                        move |cx: &mut App| {
                            let _ = this.update(cx, |flow, cx| {
                                flow.alert_purpose = Some(AuthStepFlowModelPurpose::Registration);
                                cx.notify();
                            });
                        }
                    };
                    let model = flow.read(cx).model.clone();
                    model.update(cx, |model, cx| {
                        if let Some(task) = model.begin_registration_from_login(on_closed, cx) {
                            task.detach();
                        }
                    });
                },
                theme,
                cx,
            ))
        } else if model.step == AuthStepFlowModelStep::Phone && model.purpose == AuthStepFlowModelPurpose::Registration {
            let flow_model = self.model.clone();
            Some(self.alternate_action(
                tr("haveaccount"),
                tr("signin"),
                move |_, cx| flow_model.update(cx, |model, cx| model.use_login_from_phone(cx)),
                theme,
                cx,
            ))
        } else {
            None
        };

        div()
            .w_full()
            .flex()
            .justify_center()
            .pt(px(10.0))
            .pb(px(8.0))
            .px(px(18.0))
            .bg(theme.bg._100.opacity(0.97))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(11.0))
                    .w_full()
                    .max_w(px(Self::STEP_COLUMN_WIDTH - 2.0 * Self::STEP_COLUMN_PADDING))
                    .children(alternate)
                    .child(
                        ButtonUi::new("auth-step-continue")
                            .disabled(!model.can_continue())
                            .is_pending(model.is_submitting)
                            .height(54.0)
                            .on_press(self.continue_action())
                            .child(MainText::new(tr(model.button_title_key())).font_weight(FontWeight::SEMIBOLD)),
                    )
                    .when(model.step == AuthStepFlowModelStep::Phone, |column| column.child(AuthTermsConsentText)),
            )
            .into_any_element()
    }

    fn alternate_action(
        &self,
        prefix: String,
        action_title: String,
        action: impl Fn(&mut Window, &mut App) + 'static,
        theme: &AppTheme,
        cx: &App,
    ) -> AnyElement {
        let is_submitting = self.model.read(cx).is_submitting;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.0))
            .child(SecondaryText::new(prefix).px(14.0))
            .child(
                plain("auth-alternate-action")
                    .disabled(is_submitting)
                    .opacity(if is_submitting { 0.5 } else { 1.0 })
                    .on_press(action)
                    .child(
                        MainText::new(action_title)
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
            .into_any_element()
    }

    fn field_errors(&self, name: &str, step: AuthStepFlowModelStep, cx: &App) -> HashMap<String, String> {
        let model = self.model.read(cx);
        if step != model.step {
            return HashMap::new();
        }
        match model.inline_error_text() {
            Some(text) => HashMap::from([(format!("{name}Err"), text)]),
            None => HashMap::new(),
        }
    }

    /// the field error icon only opens on a click so the reason is spelled out under the field
    fn field_error_line(&self, step: AuthStepFlowModelStep, cx: &App) -> Option<AnyElement> {
        let model = self.model.read(cx);
        if step != model.step {
            return None;
        }
        let text = model.inline_error_text()?;
        Some(div().w_full().child(ErrorTextUi::new(text).px(12.0)).into_any_element())
    }

    fn auth_flow_error(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).auth_flow_error().map(str::to_owned)
    }

    fn clear_auth_flow_error(cx: &mut App) {
        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.clear_auth_flow_error(cx));
    }

    fn set_is_submitting(model: &Entity<AuthStepFlowModel>, is_submitting: bool, cx: &mut App) {
        model.update(cx, |model, cx| {
            model.is_submitting = is_submitting;
            cx.notify();
        });
    }

    fn set_is_resending(model: &Entity<AuthStepFlowModel>, is_resending: bool, cx: &mut App) {
        model.update(cx, |model, cx| {
            model.is_resending = is_resending;
            cx.notify();
        });
    }

    fn continue_tapped(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model_entity = self.model.clone();
        let (step, is_submitting, phone_number, calling_code) = {
            let model = model_entity.read(cx);
            (model.step, model.is_submitting, model.phone_number.clone(), model.calling_code().to_owned())
        };
        if is_submitting {
            return;
        }

        match step {
            AuthStepFlowModelStep::Phone
            | AuthStepFlowModelStep::RegistrationInvite
            | AuthStepFlowModelStep::RegistrationName
            | AuthStepFlowModelStep::RegistrationBirthDate
            | AuthStepFlowModelStep::RegistrationTag
            | AuthStepFlowModelStep::RegistrationPassword
            | AuthStepFlowModelStep::ResetCode
            | AuthStepFlowModelStep::ResetPassword => {
                model_entity.update(cx, |model, cx| model.advance_after_local_validation(cx));
            }
            AuthStepFlowModelStep::LoginPassword => {
                if !model_entity.update(cx, |model, cx| model.validate_current_step(cx)) {
                    return;
                }
                Self::set_is_submitting(&model_entity, true, cx);
                let password = model_entity.read(cx).login_password.clone();
                let finished = model_entity.clone();
                AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.submit_sign_in(
                        &phone_number,
                        &password,
                        &calling_code,
                        Rc::new(|| true),
                        Some(Rc::new(move |_: Result<AuthLoginResponse, &anyhow::Error>, cx: &mut App| {
                            Self::set_is_submitting(&finished, false, cx);
                        })),
                        cx,
                    );
                });
            }
            AuthStepFlowModelStep::RegistrationPasswordConfirmation => {
                if !model_entity.update(cx, |model, cx| model.validate_current_step(cx)) {
                    return;
                }
                if model_entity.update(cx, |model, cx| model.resume_reusable_verification_code(cx)) {
                    return;
                }
                Self::set_is_submitting(&model_entity, true, cx);
                let finished = model_entity.clone();
                AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.submit_registration_code(
                        &phone_number,
                        &calling_code,
                        Some(Rc::new(move |result: Result<AuthSendCodeResponse, &anyhow::Error>, cx: &mut App| {
                            Self::set_is_submitting(&finished, false, cx);
                            if let Some(key) = Self::phone_rejection_key(cx) {
                                Self::clear_auth_flow_error(cx);
                                finished.update(cx, |model, cx| model.handle_phone_rejected(key, cx));
                                return;
                            }
                            if let Ok(response) = result {
                                if !response.code_sent() {
                                    let message =
                                        if response.error.is_empty() { tr("send_code_error") } else { response.error };
                                    AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                        interactions.present_auth_flow_error(&message, cx);
                                    });
                                }
                            }
                        })),
                        cx,
                    );
                });
            }
            AuthStepFlowModelStep::ResetPasswordConfirmation => self.submit_password_reset(window, cx),
            AuthStepFlowModelStep::VerificationCode => {
                if !model_entity.update(cx, |model, cx| model.validate_current_step(cx)) {
                    return;
                }
                Self::dismiss_keyboard(&self.focus_handle, window, cx);
                Self::set_is_submitting(&model_entity, true, cx);
                let finished = model_entity.clone();
                let (name, tag, password, code, birth_date, invite_code) = {
                    let model = model_entity.read(cx);
                    (
                        model.name.clone(),
                        model.tag.clone(),
                        model.registration_password.clone(),
                        model.code.clone(),
                        AuthStepFlowModel::wire_birth_date(model.birth_date),
                        model.requires_invite().then(|| model.invite_code.clone()),
                    )
                };
                AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.submit_register(
                        &name,
                        &tag,
                        &phone_number,
                        &password,
                        &code,
                        &calling_code,
                        Some(birth_date.as_str()),
                        invite_code.as_deref(),
                        Some(Rc::new(move |_: Result<AuthRegisterResponse, &anyhow::Error>, cx: &mut App| {
                            Self::set_is_submitting(&finished, false, cx);
                            if let Some(key) = Self::code_rejection_key(cx) {
                                let text = Self::auth_flow_error(cx);
                                Self::clear_auth_flow_error(cx);
                                finished.update(cx, |model, cx| model.handle_verification_code_rejected(key, text, cx));
                                return;
                            }
                            let code = AuthServicesStore::shared(cx).read(cx).auth_flow_error_code().map(str::to_owned);
                            if let Some(key) = AuthStepFlowModel::invite_rejection_key(code.as_deref()) {
                                Self::clear_auth_flow_error(cx);
                                finished.update(cx, |model, cx| model.handle_invite_rejected(key, cx));
                                return;
                            }
                            if Self::auth_flow_error(cx).as_deref() == Some(tr("tag_already_exists").as_str()) {
                                Self::clear_auth_flow_error(cx);
                                finished.update(cx, |model, cx| model.handle_registration_tag_conflict(cx));
                            }
                        })),
                        cx,
                    );
                });
            }
        }
    }

    fn submit_password_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model_entity = self.model.clone();
        if !model_entity.update(cx, |model, cx| model.validate_current_step(cx)) {
            return;
        }
        Self::dismiss_keyboard(&self.focus_handle, window, cx);
        Self::set_is_submitting(&model_entity, true, cx);
        let submission_id = Uuid::new_v4();
        *self.reset_sign_in_id.borrow_mut() = Some(submission_id);
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let reset_sign_in_id = self.reset_sign_in_id.clone();
        let is_current_submission: Rc<dyn Fn() -> bool> = Rc::new(move || {
            *reset_sign_in_id.borrow() == Some(submission_id)
                && SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
        });
        let (phone_number, calling_code, code, password) = {
            let model = model_entity.read(cx);
            (
                model.phone_number.clone(),
                model.calling_code().to_owned(),
                model.code.clone(),
                model.new_password.clone(),
            )
        };

        let reset_sign_in_id = self.reset_sign_in_id.clone();
        let is_current = is_current_submission.clone();
        let on_finish = Rc::new(move |result: Result<AuthResetPasswordResponse, &anyhow::Error>, cx: &mut App| {
            if !is_current() {
                return;
            }
            let flow_error = Self::auth_flow_error(cx);
            let rejection = Self::code_rejection_key(cx).or_else(|| Self::rejected_reset_code_key(flow_error.as_deref()));
            if let Some(error_key) = rejection {
                Self::set_is_submitting(&model_entity, false, cx);
                *reset_sign_in_id.borrow_mut() = None;
                Self::clear_auth_flow_error(cx);
                model_entity.update(cx, |model, cx| model.handle_reset_code_rejected(error_key, flow_error, cx));
                return;
            }
            let was_answered = result.is_ok();
            let succeeded = result.is_ok_and(|response| response.success) && flow_error.is_none();
            if !succeeded {
                Self::set_is_submitting(&model_entity, false, cx);
                *reset_sign_in_id.borrow_mut() = None;
                if was_answered && flow_error.is_none() {
                    let message = tr("auth_reset_password_error");
                    AuthInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.present_auth_flow_error(&message, cx));
                }
                return;
            }

            let (phone_number, calling_code, password) = {
                let model = model_entity.read(cx);
                (model.phone_number.clone(), model.calling_code().to_owned(), model.new_password.clone())
            };
            let finished = model_entity.clone();
            let sign_in_id = reset_sign_in_id.clone();
            let is_current = is_current.clone();
            AuthInteractionsStore::shared(cx).update(cx, move |interactions, cx| {
                interactions.announce_password_reset_succeeded(cx);
                interactions.submit_sign_in(
                    &phone_number,
                    &password,
                    &calling_code,
                    is_current,
                    Some(Rc::new(move |sign_in_result: Result<AuthLoginResponse, &anyhow::Error>, cx: &mut App| {
                        if *sign_in_id.borrow() != Some(submission_id) {
                            return;
                        }
                        *sign_in_id.borrow_mut() = None;
                        let signed_in = sign_in_result.is_ok() && Self::auth_flow_error(cx).is_none();
                        finished.update(cx, |model, cx| model.finish_password_reset(signed_in, cx));
                        Self::set_is_submitting(&finished, false, cx);
                    })),
                    cx,
                );
            });
        });

        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.submit_password_reset(
                &phone_number,
                &calling_code,
                &code,
                &password,
                is_current_submission,
                Some(on_finish),
                cx,
            );
        });
    }

    /// masked because this sheet is open exactly when someone is stuck and may screenshot or share
    /// the screen
    fn masked_phone_number(&self, cx: &App) -> String {
        RiseFormat::masked_phone(&self.model.read(cx).composed_phone_number())
    }

    fn auth_error_title_key(&self, cx: &App) -> &'static str {
        match self.alert_purpose.unwrap_or(self.model.read(cx).purpose) {
            AuthStepFlowModelPurpose::Login => "auth_signin_error_title",
            AuthStepFlowModelPurpose::PasswordReset => "auth_reset_password_error_title",
            AuthStepFlowModelPurpose::Registration => "register_error_title",
        }
    }

    /// presents route and closes whatever is up first
    /// the dismissal must land before the next presentation, sheet_dismissed finishes it
    fn swap_sheet(&mut self, route: AuthStepFlowSheetRoute, cx: &mut Context<Self>) {
        match self.sheet_route {
            None => {
                self.sheet_route = Some(route);
                self.displayed_sheet_route = route;
            }
            Some(current) if current == route => return,
            Some(_) => {
                self.pending_sheet_route = Some(route);
                self.sheet_route = None;
            }
        }
        cx.notify();
    }

    fn sheet_dismissed(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.pending_sheet_route.take() {
            let this = self.this.clone();
            cx.defer(move |cx| {
                let _ = this.update(cx, |flow, cx| {
                    flow.sheet_route = Some(next);
                    flow.displayed_sheet_route = next;
                    cx.notify();
                });
            });
            return;
        }

        if self.services.read(cx).telegram_redirect_active() {
            AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.complete_telegram_redirect(cx));
            self.advance_to_code_entry(cx);
        }

        let (step, purpose, is_submitting) = {
            let model = self.model.read(cx);
            (model.step, model.purpose, model.is_submitting)
        };
        let is_code_step = step == AuthStepFlowModelStep::VerificationCode || step == AuthStepFlowModelStep::ResetCode;
        if is_code_step || !is_submitting {
            self.field_focus_request += 1;
            cx.notify();
        }
        if is_code_step {
            return;
        }

        if purpose == AuthStepFlowModelPurpose::PasswordReset
            && step == AuthStepFlowModelStep::LoginPassword
            && !is_submitting
        {
            self.model.update(cx, |model, cx| model.cancel_password_reset(cx));
        }
    }

    fn password_reset_requested(&mut self, cx: &mut Context<Self>) {
        let model_entity = self.model.clone();
        if model_entity.read(cx).is_submitting {
            return;
        }
        Self::set_is_submitting(&model_entity, true, cx);
        let (phone_number, calling_code) = {
            let model = model_entity.read(cx);
            (model.phone_number.clone(), model.calling_code().to_owned())
        };
        let this = self.this.clone();
        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.submit_password_reset_code(
                &phone_number,
                &calling_code,
                Some(Rc::new(move |result: Result<AuthSendCodeResponse, &anyhow::Error>, cx: &mut App| {
                    Self::set_is_submitting(&model_entity, false, cx);
                    let close_sheet = |cx: &mut App| {
                        let _ = this.update(cx, |flow, cx| {
                            flow.sheet_route = None;
                            cx.notify();
                        });
                    };
                    if AuthServicesStore::shared(cx).read(cx).code_entry_active() {
                        close_sheet(cx);
                        return;
                    }
                    if result.as_ref().is_ok_and(|response| response.code_sent()) {
                        return;
                    }

                    close_sheet(cx);
                    if let Some(key) = Self::phone_rejection_key(cx) {
                        Self::clear_auth_flow_error(cx);
                        model_entity.update(cx, |model, cx| model.handle_phone_rejected(key, cx));
                        return;
                    }
                    let refusal = result.ok().map(|response| {
                        if response.error.is_empty() { tr("send_code_error") } else { response.error }
                    });
                    if refusal.is_some() || Self::auth_flow_error(cx).is_some() {
                        let _ = this.update(cx, |flow, cx| {
                            flow.alert_purpose = Some(AuthStepFlowModelPurpose::PasswordReset);
                            cx.notify();
                        });
                    }
                    model_entity.update(cx, |model, cx| model.cancel_password_reset(cx));
                    if let Some(refusal) = refusal {
                        AuthInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.present_auth_flow_error(&refusal, cx));
                    }
                })),
                cx,
            );
        });
    }

    fn advance_to_code_entry(&mut self, cx: &mut Context<Self>) {
        let code_length = self
            .services
            .read(cx)
            .code_delivery()
            .map_or(VERIFICATION_CODE.maximum_value() as i64, |delivery| delivery.code_length);
        self.model.update(cx, |model, cx| {
            model.code_length = code_length;
            if model.purpose == AuthStepFlowModelPurpose::PasswordReset {
                model.move_to_reset_code(cx);
            } else {
                model.move_to_verification_code(cx);
            }
        });
    }

    /// answer shared by resend and the telegram switch
    /// a refused number goes back to the phone step, a sent code restarts the cells
    fn code_resent_callback(
        model: Entity<AuthStepFlowModel>,
    ) -> Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)> {
        Rc::new(move |result: Result<AuthSendCodeResponse, &anyhow::Error>, cx: &mut App| {
            Self::set_is_resending(&model, false, cx);
            if let Some(key) = Self::phone_rejection_key(cx) {
                Self::clear_auth_flow_error(cx);
                model.update(cx, |model, cx| model.handle_phone_rejected(key, cx));
                return;
            }
            if result.is_ok_and(|response| response.code_sent()) {
                let code_length = AuthServicesStore::shared(cx).read(cx).code_delivery().map(|delivery| delivery.code_length);
                if let Some(code_length) = code_length {
                    model.update(cx, |model, cx| model.restart_code_entry(code_length, cx));
                }
            }
        })
    }

    fn use_telegram(&mut self, cx: &mut Context<Self>) {
        let model_entity = self.model.clone();
        let (is_busy, phone_number, calling_code) = {
            let model = model_entity.read(cx);
            (model.is_resending || model.is_submitting, model.phone_number.clone(), model.calling_code().to_owned())
        };
        if is_busy {
            return;
        }
        Self::set_is_resending(&model_entity, true, cx);
        let on_finish = Self::code_resent_callback(model_entity.clone());
        let started = AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.switch_to_telegram(&phone_number, &calling_code, Some(on_finish), cx)
        });
        if !started {
            Self::set_is_resending(&model_entity, false, cx);
        }
    }

    fn resend_code(&mut self, channel: Option<AuthCodeChannel>, cx: &mut Context<Self>) {
        let model_entity = self.model.clone();
        let (is_busy, phone_number, calling_code) = {
            let model = model_entity.read(cx);
            (model.is_resending || model.is_submitting, model.phone_number.clone(), model.calling_code().to_owned())
        };
        if is_busy {
            return;
        }
        Self::set_is_resending(&model_entity, true, cx);
        let on_finish = Self::code_resent_callback(model_entity.clone());
        let started = AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.resend_code(&phone_number, &calling_code, channel, Some(on_finish), cx)
        });
        if !started {
            Self::set_is_resending(&model_entity, false, cx);
        }
    }

    /// canon names the field a refusal belongs to
    /// number refusals go back to the phone step and not an alert over a step that cant fix them
    fn phone_rejection_key(cx: &mut App) -> Option<&'static str> {
        Self::field_rejection_key(PHONE.field(), cx)
    }

    fn code_rejection_key(cx: &mut App) -> Option<&'static str> {
        Self::field_rejection_key(VERIFICATION_CODE.field(), cx)
    }

    fn field_rejection_key(field: &str, cx: &mut App) -> Option<&'static str> {
        let code = AuthServicesStore::shared(cx).read(cx).auth_flow_error_code()?.to_owned();
        if CanonErrors::field(Some(&code), &HashMap::new()).as_deref() != Some(field) {
            return None;
        }
        CanonErrors::entry(Some(&code))?.i18n_key
    }

    fn subtitle_text(&self, step: AuthStepFlowModelStep, cx: &App) -> String {
        let fallback = || tr(AuthStepFlowModel::subtitle_key(step));
        if step != AuthStepFlowModelStep::VerificationCode && step != AuthStepFlowModelStep::ResetCode {
            return fallback();
        }
        let Some(delivery) = self.services.read(cx).code_delivery() else {
            return fallback();
        };
        if delivery.channel == Some(AuthCodeChannel::TelegramBot) && !delivery.offers_resend() {
            return fallback();
        }
        match delivery.channel {
            Some(AuthCodeChannel::TelegramBot) => return tr("otp_code_sent_telegram"),
            Some(AuthCodeChannel::FlashCall) => return tr("otp_code_flash_call"),
            _ => {}
        }
        let destination = if delivery.destination_masked.is_empty() {
            self.masked_phone_number(cx)
        } else {
            delivery.destination_masked.clone()
        };
        let key =
            if delivery.channel == Some(AuthCodeChannel::Viber) { "otp_code_sent_viber" } else { "otp_code_sent_sms" };
        tr_args(key, &[FormatArg::Text(destination.into())])
    }

    fn rejected_reset_code_key(message: Option<&str>) -> Option<&'static str> {
        let message = message?;
        ["send_code_invalidcode_error", "auth_code_expired", "verify_too_many_attempts"]
            .into_iter()
            .find(|key| message == tr(key))
    }

    /// drops keyboard focus
    /// keys only travel from the focus to the root, an empty focus goes to the root view off the flow
    /// so the flow keeps focus and esc and enter still work
    fn dismiss_keyboard(focus_handle: &FocusHandle, window: &mut Window, cx: &mut App) {
        window.focus(focus_handle, cx);
    }

    fn sheet_host(&self, theme: &AppTheme) -> NativeBottomSheetPresentationHost {
        let reader = self.this.clone();
        let writer = self.this.clone();
        let content = self.this.clone();
        let dismissed = self.this.clone();
        let detent_height = match self.displayed_sheet_route {
            AuthStepFlowSheetRoute::PasswordResetIntro => PasswordResetIntroSheet::PRESENTATION_HEIGHT,
            AuthStepFlowSheetRoute::Telegram => TelegramRedirectSheet::PRESENTATION_HEIGHT,
        };
        NativeBottomSheetPresentationHost::new(
            "auth-step-flow-sheet",
            Binding::new(
                move |cx: &App| reader.upgrade().is_some_and(|flow| flow.read(cx).sheet_route.is_some()),
                move |is_presented, _window, cx| {
                    if !is_presented {
                        let _ = writer.update(cx, |flow, cx| {
                            flow.sheet_route = None;
                            cx.notify();
                        });
                    }
                },
            ),
            move |_window, cx| Self::sheet_content(&content, cx),
        )
        .presentation_detents([PresentationDetent::Height(detent_height)])
        .presentation_drag_indicator(true)
        .presentation_background(theme.bg._100)
        .on_dismiss(move |_window, cx| {
            let _ = dismissed.update(cx, |flow, cx| flow.sheet_dismissed(cx));
        })
    }

    fn sheet_content(this: &WeakEntity<Self>, cx: &mut App) -> AnyElement {
        let Some(flow) = this.upgrade() else {
            return div().into_any_element();
        };
        let flow = flow.read(cx);
        let services = flow.services.read(cx);
        match flow.displayed_sheet_route {
            AuthStepFlowSheetRoute::PasswordResetIntro => {
                let this = this.clone();
                PasswordResetIntroSheet::new(flow.masked_phone_number(cx))
                    .is_submitting(flow.model.read(cx).is_submitting)
                    .on_continue(move |_window, cx| {
                        let _ = this.update(cx, |flow, cx| flow.password_reset_requested(cx));
                    })
                    .into_any_element()
            }
            AuthStepFlowSheetRoute::Telegram => {
                let this = this.clone();
                let purpose = if services.password_reset_active() {
                    TelegramRedirectSheetPurpose::PasswordReset
                } else {
                    TelegramRedirectSheetPurpose::Registration
                };
                TelegramRedirectSheet::new(services.registration_bot_username(), services.registration_phone_payload())
                    .purpose(purpose)
                    .on_enter_code(move |_window, cx| {
                        AuthInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.complete_telegram_redirect(cx));
                        let _ = this.update(cx, |flow, cx| {
                            flow.advance_to_code_entry(cx);
                            flow.sheet_route = None;
                            cx.notify();
                        });
                    })
                    .into_any_element()
            }
        }
    }

    fn observe_changes(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (telegram_active, code_entry_active, is_authenticated, has_error) = {
            let services = self.services.read(cx);
            (
                services.telegram_redirect_active(),
                services.code_entry_active(),
                services.is_authenticated(),
                services.auth_flow_error().is_some(),
            )
        };
        let (tail, step) = {
            let model = self.model.read(cx);
            (
                AuthStepFlowFieldTail { tag: model.tag_availability.clone(), error: model.inline_error_text() },
                model.step,
            )
        };

        let this = self.this.clone();
        on_change("auth-telegram-redirect", telegram_active, false, window, cx, move |_, active, _, cx| {
            if *active {
                let _ = this.update(cx, |flow, cx| flow.swap_sheet(AuthStepFlowSheetRoute::Telegram, cx));
            }
        });

        let this = self.this.clone();
        on_change("auth-code-entry", code_entry_active, false, window, cx, move |_, active, _, cx| {
            if *active {
                let _ = this.update(cx, |flow, cx| {
                    let step = flow.model.read(cx).step;
                    if step != AuthStepFlowModelStep::VerificationCode && step != AuthStepFlowModelStep::ResetCode {
                        flow.advance_to_code_entry(cx);
                    }
                });
            }
        });

        let on_auth = self.on_auth.clone();
        on_change("auth-authenticated", is_authenticated, false, window, cx, move |was, is, window, cx| {
            if !*was && *is {
                on_auth(window, cx);
            }
        });

        let this = self.this.clone();
        on_change("auth-flow-error", has_error, true, window, cx, move |_, has_error, _, cx| {
            let _ = this.update(cx, |flow, cx| {
                if *has_error {
                    flow.present_auth_flow_alert(cx);
                } else {
                    flow.alert_purpose = None;
                }
            });
        });

        let handle = self.scroll_handle.clone();
        on_change("auth-field-tail", tail, false, window, cx, move |_, _, window, _| {
            handle.scroll_to_bottom();
            window.refresh();
        });

        let handle = self.scroll_handle.clone();
        let this = self.this.clone();
        use_task("auth-step-settle", step, window, cx, async move |cx| {
            cx.background_executor().timer(Duration::from_millis(320)).await;
            handle.scroll_to_bottom();
            let _ = this.update(cx, |_, cx| cx.notify());
        });
    }

    fn present_auth_flow_alert(&mut self, cx: &mut Context<Self>) {
        if self.is_alert_presented {
            return;
        }
        let Some(message) = self.services.read(cx).auth_flow_error().map(str::to_owned) else {
            return;
        };
        self.is_alert_presented = true;
        let title = tr(self.auth_error_title_key(cx));
        let this = self.this.clone();
        NativeConfirmAlert::present_notice(
            &title,
            &message,
            None,
            None,
            move |cx| {
                let _ = this.update(cx, |flow, _| flow.is_alert_presented = false);
                Self::clear_auth_flow_error(cx);
            },
            cx,
        );
    }
}

impl Render for AuthStepFlow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let now = RiseClock::monotonic();
        self.observe_changes(window, cx);
        self.departing_pages.retain(|page| page.is_running(now));

        let is_compact = f32::from(window.viewport_size().height) < COMPACT_HEIGHT;
        let band = window_controls_band(window, cx);
        let header = self.progress_header(&theme, window, cx);
        let footer = self.action_footer(&theme, cx);
        let shows_back_button = self.can_go_back(cx);

        let (entering, departing) = {
            let app: &App = cx;
            let entering =
                self.page.render_transitioned(now, window, |step| self.step_page(*step, &theme, is_compact, app));
            let departing = self
                .departing_pages
                .iter()
                .filter_map(|page| {
                    page.render_transitioned(now, window, |step| self.step_page(*step, &theme, is_compact, app))
                })
                .map(|page| div().absolute().top_0().left_0().w_full().flex().justify_center().child(page))
                .collect::<Vec<_>>();
            (entering, departing)
        };

        let on_key = self.this.clone();
        let key_focus = self.focus_handle.clone();

        div()
            .id("auth-step-flow")
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let modifiers = &event.keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform {
                    return;
                }
                if event.keystroke.key == TAB_KEY {
                    cx.stop_propagation();
                    if modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    return;
                }
                if modifiers.shift {
                    return;
                }
                match event.keystroke.key.as_str() {
                    ESCAPE_KEY => {
                        cx.stop_propagation();
                        let _ = on_key.update(cx, |flow, cx| flow.escape_requested(window, cx));
                    }
                    ENTER_KEY if key_focus.is_focused(window) => {
                        cx.stop_propagation();
                        let _ = on_key.update(cx, |flow, cx| flow.continue_tapped(window, cx));
                    }
                    _ => {}
                }
            })
            .child(
                BgWrapperUi::new().child(
                    div()
                        .flex()
                        .flex_col()
                        .size_full()
                        .pt(band)
                        .child(header)
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(
                                    scroll_view("auth-step-scroll").scroll_handle(self.scroll_handle.clone()).child(
                                        div()
                                            .relative()
                                            .w_full()
                                            .flex()
                                            .justify_center()
                                            .children(entering)
                                            .children(departing),
                                    ),
                                )
                                .when(shows_back_button, |region| region.child(self.edge_swipe_back())),
                        )
                        .child(footer),
                ),
            )
            .child(self.sheet_host(&theme))
            .on_disappear("auth-step-flow-lifecycle", {
                let this = self.this.clone();
                move |_, cx| {
                    let _ = this.update(cx, |flow, cx| {
                        *flow.reset_sign_in_id.borrow_mut() = None;
                        flow.sheet_route = None;
                        flow.pending_sheet_route = None;
                        cx.notify();
                    });
                    AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.cancel_registration_tag_check(cx);
                        interactions.reset_auth_flow_presentation(cx);
                    });
                }
            })
    }
}

/// what the code field holds after an edit
/// a pasted whole code takes over with its first digits, shorter edits happen in place
/// only ascii digits stay
pub enum AuthCodeEntry {}

impl AuthCodeEntry {
    pub fn applying(replacement: &str, range: NSRange, current: &str, digit_count: i64) -> String {
        let inserted = Self::digits(replacement);
        if digit_count > 0 && inserted.chars().count() as i64 >= digit_count {
            return inserted.chars().take(digit_count as usize).collect();
        }
        let edited = match utf16_to_byte_range(current, range) {
            Some(byte_range) => {
                let mut edited = current.to_owned();
                edited.replace_range(byte_range, &inserted);
                edited
            }
            None => format!("{current}{inserted}"),
        };
        Self::normalized(&edited, digit_count)
    }

    pub fn normalized(raw: &str, digit_count: i64) -> String {
        Self::digits(raw).chars().take(digit_count.max(0) as usize).collect()
    }

    fn digits(raw: &str) -> String {
        raw.chars().filter(char::is_ascii_digit).collect()
    }
}

struct AuthOneTimeCodeInput {
    focus_handle: FocusHandle,
    pending_activation: Option<Subscription>,
}

impl AuthOneTimeCodeInput {
    fn new(cx: &mut Context<Self>) -> Self {
        Self { focus_handle: cx.focus_handle().tab_stop(true), pending_activation: None }
    }

    /// moves focus at once, active window or not
    /// keys reach cells by focus alone, waiting for activation never landed
    /// an inactive window asks once more on activation
    fn raise_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        if window.is_window_active() || self.pending_activation.is_some() {
            return;
        }
        self.pending_activation = Some(cx.observe_window_activation(window, |input, window, cx| {
            if !window.is_window_active() {
                return;
            }
            input.pending_activation = None;
            window.focus(&input.focus_handle, cx);
        }));
    }
}

#[derive(IntoElement)]
struct AuthVerificationCodeField {
    code: String,
    digit_count: i64,
    is_complete: Rc<dyn Fn(&str) -> bool>,
    error_text: Option<String>,
    external_focus_request: i64,
    on_edit: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    on_complete: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    on_submit: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl AuthVerificationCodeField {
    fn code_cell(index: i64, code: &str, digit_count: i64, theme: &AppTheme, window: &mut Window, cx: &mut App) -> Div {
        let count = code.chars().count() as i64;
        let character = code.chars().nth(index.max(0) as usize);
        let is_current = index == count && count < digit_count;

        let now = RiseClock::monotonic();
        let target = if is_current { 1.0_f32 } else { 0.0 };
        let id = gpui::ElementId::NamedInteger(SharedString::new_static("auth-code-cell"), index as u64);
        let progress = window.use_keyed_state(id, cx, |_, _| Animated::new(target));
        if *progress.read(cx).target() != target {
            progress.update(cx, |progress, cx| progress.set_in(target, ease_in_out(0.12), now, cx));
        }
        let value = progress.read(cx).frame_value(now, window);

        div()
            .flex_1()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(62.0))
                    .h(px(58.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(16.0))
                    .bg(theme.bg._200)
                    .border(px(if is_current { 1.5 } else { 1.0 }))
                    .border_color(theme.input.border_300.blend(theme.primary._100.opacity(value)))
                    .children(character.map(|character| {
                        MainText::new(character.to_string()).px(23.0).font_weight(FontWeight::SEMIBOLD)
                    })),
            )
    }
}

impl RenderOnce for AuthVerificationCodeField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let input = window.use_keyed_state("auth-one-time-code-input", cx, |_, cx| AuthOneTimeCodeInput::new(cx));
        let focus_handle = input.read(cx).focus_handle.clone();

        let is_complete = self.is_complete.clone();
        let on_complete = self.on_complete.clone();
        on_change("auth-code-value", self.code.clone(), false, window, cx, move |_, new_value, window, cx| {
            if is_complete(new_value) {
                on_complete(new_value, window, cx);
            }
        });
        let error_input = input.clone();
        on_change("auth-code-error", self.error_text.clone(), false, window, cx, move |_, new_value, window, cx| {
            if new_value.as_deref().is_some_and(|text| !text.is_empty()) {
                error_input.update(cx, |input, cx| input.raise_keyboard(window, cx));
            }
        });
        let request_input = input.clone();
        on_change("auth-code-focus-request", self.external_focus_request, false, window, cx, move |_, _, window, cx| {
            request_input.update(cx, |input, cx| input.raise_keyboard(window, cx));
        });

        let cells = (0..self.digit_count)
            .map(|index| Self::code_cell(index, &self.code, self.digit_count, &theme, window, cx))
            .collect::<Vec<_>>();

        let code = self.code;
        let digit_count = self.digit_count;
        let on_edit = self.on_edit;
        let on_submit = self.on_submit;
        let click_input = input.clone();
        let appear_input = input;

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0))
            .w_full()
            .child(
                div()
                    .id("auth-code-cells")
                    .track_focus(&focus_handle)
                    .flex()
                    .flex_row()
                    .gap(px(10.0))
                    .w_full()
                    .h(px(58.0))
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, window, cx| {
                        click_input.update(cx, |input, cx| input.raise_keyboard(window, cx));
                    })
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let keystroke = &event.keystroke;
                        let modifiers = &keystroke.modifiers;
                        let current_length = code.len();
                        let edited = if modifiers.secondary() && keystroke.key == "v" {
                            cx.read_from_clipboard().and_then(|item| item.text()).map(|text| {
                                AuthCodeEntry::applying(
                                    &text,
                                    NSRange { location: current_length, length: 0 },
                                    &code,
                                    digit_count,
                                )
                            })
                        } else if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
                            return;
                        } else if keystroke.key == BACKSPACE_KEY {
                            (current_length > 0).then(|| {
                                AuthCodeEntry::applying(
                                    "",
                                    NSRange { location: current_length - 1, length: 1 },
                                    &code,
                                    digit_count,
                                )
                            })
                        } else if keystroke.key == ENTER_KEY {
                            cx.stop_propagation();
                            on_submit(window, cx);
                            return;
                        } else {
                            keystroke.key_char.as_deref().map(|typed| {
                                AuthCodeEntry::applying(
                                    typed,
                                    NSRange { location: current_length, length: 0 },
                                    &code,
                                    digit_count,
                                )
                            })
                        };
                        if let Some(next) = edited.filter(|next| *next != code) {
                            cx.stop_propagation();
                            on_edit(&next, window, cx);
                        }
                    })
                    .children(cells),
            )
            .when_some(self.error_text.filter(|text| !text.is_empty()), |column, text| {
                column.child(ErrorTextUi::new(text).px(12.0))
            })
            .on_appear("auth-code-appear", move |window, cx| {
                appear_input.update(cx, |input, cx| input.raise_keyboard(window, cx));
            })
    }
}

#[derive(IntoElement)]
struct AuthTermsConsentText;

impl AuthTermsConsentText {
    /// canon text carries terms and privacy placeholders in the order its language needs
    /// each is replaced by its own clickable name wherever it stands
    fn attributed(text_color: Hsla, link_color: Hsla) -> AttributedString {
        let links = [
            ("{terms}", tr("auth_terms_consent_terms"), RiseLegalLinks::url(RiseLegalLinksDocument::TermsOfService)),
            ("{privacy}", tr("auth_terms_consent_privacy"), RiseLegalLinks::url(RiseLegalLinksDocument::PrivacyPolicy)),
        ];
        let plain_attributes = || AttributeContainer {
            font: Some(RichFont::system(12.0)),
            foreground_color: Some(text_color),
            ..AttributeContainer::default()
        };
        let mut result = AttributedString::new("");
        let template = tr("auth_terms_consent");
        let mut rest = template.as_str();
        loop {
            let next = links
                .iter()
                .filter_map(|link| rest.find(link.0).map(|start| (link, start)))
                .min_by_key(|(_, start)| *start);
            let Some(((token, title, url), start)) = next else {
                result.append(AttributedString::with_attributes(rest, plain_attributes()));
                return result;
            };
            result.append(AttributedString::with_attributes(&rest[..start], plain_attributes()));
            result.append(AttributedString::with_attributes(
                title.clone(),
                AttributeContainer {
                    font: Some(RichFont::system_weight(12.0, FontWeight::SEMIBOLD)),
                    foreground_color: Some(link_color),
                    link: url.clone(),
                    ..AttributeContainer::default()
                },
            ));
            rest = &rest[start + token.len()..];
        }
    }
}

impl RenderOnce for AuthTermsConsentText {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let text = Self::attributed(theme.text.secondary, theme.primary._100);
        let action = OpenURLAction::new(|url: &Url, _window, cx| {
            RiseLegalLinks::open_url(Some(url.clone()), cx);
            OpenURLActionResult::Handled
        });
        div()
            .w_full()
            .flex()
            .justify_center()
            .text_center()
            .text_size(px(12.0))
            .child(env_scope::<OpenURLKey>(action).child(RichText::new("auth-terms-consent-text", text)))
    }
}

#[derive(IntoElement)]
struct AuthTelegramFallbackRow {
    is_sending: bool,
    on_use: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl AuthTelegramFallbackRow {
    const BRAND: &'static str = "Telegram";
}

impl RenderOnce for AuthTelegramFallbackRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let secondary = rise_ui::theme(cx).text.secondary;
        let phrase = tr("otp_no_code_telegram");
        let mut parts = phrase.split("%@");
        let lead = parts.next().unwrap_or_default().to_owned();
        let tail = parts.collect::<Vec<_>>().join(Self::BRAND);

        let plain_attributes = || AttributeContainer {
            font: Some(RichFont::system(14.0)),
            foreground_color: Some(secondary),
            ..AttributeContainer::default()
        };
        let mut text = AttributedString::with_attributes(lead, plain_attributes());
        text.append(AttributedString::with_attributes(
            Self::BRAND,
            AttributeContainer {
                font: Some(RichFont::system_weight(14.0, FontWeight::SEMIBOLD)),
                foreground_color: Some(TelegramRedirectSheet::telegram_blue()),
                ..AttributeContainer::default()
            },
        ));
        text.append(AttributedString::with_attributes(tail, plain_attributes()));

        let on_use = self.on_use;
        plain("auth-telegram-fallback")
            .w_full()
            .flex()
            .justify_center()
            .text_center()
            .disabled(self.is_sending)
            .opacity(if self.is_sending { 0.5 } else { 1.0 })
            .on_press(move |window, cx| on_use(window, cx))
            .child(RichText::new("auth-telegram-fallback-text", text))
    }
}

struct AuthCodeResendTimeline {
    delivery: AuthCodeDelivery,
    is_sending: bool,
    on_resend: Rc<dyn Fn(Option<AuthCodeChannel>, &mut Window, &mut App)>,
    on_open_telegram: Rc<dyn Fn(&mut Window, &mut App)>,
    timer: Option<RepeatingTimer>,
}

impl AuthCodeResendTimeline {
    fn remaining(delivery: &AuthCodeDelivery) -> i64 {
        delivery.resend_available_at.time_interval_since(&RiseClock::now()).ceil() as i64
    }

    fn start_timer(&mut self, cx: &mut Context<Self>) {
        self.timer = Some(RepeatingTimer::scheduled_timer(Duration::from_secs(1), true, cx, |timeline, cx| {
            if Self::remaining(&timeline.delivery) <= 0 {
                if let Some(timer) = timeline.timer.as_mut() {
                    timer.invalidate();
                }
            }
            cx.notify();
        }));
    }

    fn action(
        id: SharedString,
        title: String,
        is_sending: bool,
        color: Hsla,
        perform: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        plain(id)
            .disabled(is_sending)
            .opacity(if is_sending { 0.5 } else { 1.0 })
            .on_press(perform)
            .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).color(color))
    }
}

impl Render for AuthCodeResendTimeline {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let primary = rise_ui::theme(cx).primary._100;
        let remaining = Self::remaining(&self.delivery);
        let is_sending = self.is_sending;

        let mut column = div().flex().flex_col().items_center().gap(px(12.0)).w_full();
        if self.delivery.channel == Some(AuthCodeChannel::TelegramBot) {
            let on_open_telegram = self.on_open_telegram.clone();
            column = column.child(Self::action(
                SharedString::new_static("auth-open-telegram"),
                tr("otp_open_telegram"),
                is_sending,
                primary,
                move |window, cx| on_open_telegram(window, cx),
            ));
        }
        if remaining > 0 {
            return column.child(
                SecondaryText::new(tr_args(
                    "otp_resend_in",
                    &[FormatArg::Text(RiseFormat::duration(remaining).into())],
                ))
                .px(14.0)
                .tac(TextAlignT::Center),
            );
        }
        let on_resend = self.on_resend.clone();
        column = column.child(Self::action(
            SharedString::new_static("auth-resend-code"),
            tr("otp_resend_code"),
            is_sending,
            primary,
            move |window, cx| on_resend(None, window, cx),
        ));
        for channel in &self.delivery.alternatives {
            if let Some(key) = channel.switch_title_key() {
                let channel = *channel;
                let on_resend = self.on_resend.clone();
                column = column.child(Self::action(
                    SharedString::from(format!("auth-resend-{}", channel.raw_value())),
                    tr(key),
                    is_sending,
                    primary,
                    move |window, cx| on_resend(Some(channel), window, cx),
                ));
            }
        }
        column
    }
}

#[derive(IntoElement)]
struct AuthCodeResendRow {
    delivery: AuthCodeDelivery,
    is_sending: bool,
    on_resend: Rc<dyn Fn(Option<AuthCodeChannel>, &mut Window, &mut App)>,
    on_open_telegram: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for AuthCodeResendRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let seed = self.delivery.clone();
        let on_resend = self.on_resend.clone();
        let on_open_telegram = self.on_open_telegram.clone();
        let is_sending = self.is_sending;
        let timeline = window.use_keyed_state("auth-code-resend-timeline", cx, move |_, cx| {
            let mut timeline =
                AuthCodeResendTimeline { delivery: seed, is_sending, on_resend, on_open_telegram, timer: None };
            timeline.start_timer(cx);
            timeline
        });
        timeline.update(cx, |timeline, cx| {
            if timeline.delivery != self.delivery {
                timeline.delivery = self.delivery;
                timeline.start_timer(cx);
            }
            timeline.is_sending = self.is_sending;
            timeline.on_resend = self.on_resend;
            timeline.on_open_telegram = self.on_open_telegram;
        });
        timeline
    }
}

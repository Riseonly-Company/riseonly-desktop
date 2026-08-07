use std::collections::HashMap;

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement,
    Render, Subscription, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_media::lottie::lottie_view::LottieView;
use rise_theme::AppTheme;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState};
use rise_ui::{BoxUi, ButtonTone, ButtonUi, IconSize, IconUi, MainText, TextTone};

use crate::core::animations;
use crate::modules::auth::engine::rise_auth_presentation::TagAvailability;
use crate::modules::auth::stores::auth_actions::auth_types::{
    AuthStep, AuthStepForm, TagMood, validate,
};
use crate::modules::auth::stores::auth_interactions::{AuthInteractionsStore, StepOutcome};
use crate::modules::auth::stores::auth_services::AuthServicesStore;

/// Which path the user started on.
///
/// One screen, two entry points, exactly as the reference. It is not fixed: the
/// footer lets somebody with no account cross over from sign-in, and somebody
/// who has one cross back, without leaving the screen or retyping their number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthMode {
    SignIn,
    SignUp,
}

pub struct AuthStepFlow {
    registering: bool,
    step: AuthStep,
    form: AuthStepForm,
    field: Entity<InputUiState>,
    interactions: AuthInteractionsStore,
    services: Entity<AuthServicesStore>,
    /// One player per pose, built once. The mascot changes on every step and on
    /// every tag verdict; rebuilding would re-rasterise a whole timeline each
    /// time, which on the tag step is once per keystroke.
    animations: HashMap<&'static str, Entity<LottieView>>,
    animation_side: gpui::Pixels,
    scale_factor: f32,
    /// The last locally-detected problem. Server refusals live on the snapshot
    /// instead, so a refusal survives a re-render and a keystroke clears only
    /// what the keystroke is about.
    local_error: Option<&'static str>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// The default calling code.
///
/// `+7` is the reference's own default and covers the two largest markets the
/// product ships in. A country picker is a component of its own and belongs to
/// `user`, which owns the country catalogue.
const DEFAULT_CALLING_CODE: &str = "7";

impl AuthStepFlow {
    pub fn new(
        mode: AuthMode,
        interactions: AuthInteractionsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = interactions.services().clone();
        let field = cx.new(|cx| InputUiState::new(InputMode::SingleLine, cx));

        let subscriptions = vec![
            cx.subscribe(&field, |flow, _, event, cx| match event {
                InputUiEvent::Changed => flow.field_changed(cx),
                InputUiEvent::Submitted => flow.submit(cx),
                InputUiEvent::Cancelled => flow.back(cx),
            }),
            // The flow watches the snapshot rather than polling it: the Telegram
            // hop finishes outside the app, and only the repository knows when
            // the code step may open.
            cx.observe(&services, |flow, _, cx| flow.snapshot_changed(cx)),
        ];

        let mut flow = Self {
            registering: matches!(mode, AuthMode::SignUp),
            step: AuthStep::Phone,
            form: AuthStepForm {
                calling_code: DEFAULT_CALLING_CODE.to_owned(),
                ..AuthStepForm::default()
            },
            field,
            interactions,
            services,
            animations: HashMap::new(),
            animation_side: rise_ui::theme(cx as &App).auth.step_art_size,
            scale_factor: window.scale_factor(),
            local_error: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };

        flow.ensure_animation(flow.step.animation(TagMood::Unknown), cx);
        flow.sync_field(cx);
        flow
    }

    pub fn step(&self) -> AuthStep {
        self.step
    }

    #[cfg(test)]
    pub fn form(&self) -> &AuthStepForm {
        &self.form
    }

    #[cfg(test)]
    pub fn is_registering(&self) -> bool {
        self.registering
    }

    /// Loads a pose the first time it is asked for and keeps it.
    fn ensure_animation(&mut self, name: &'static str, cx: &mut Context<Self>) {
        if self.animations.contains_key(name) {
            return;
        }
        let view = animations::lottie(name, self.animation_side, self.scale_factor, cx);
        self.animations.insert(name, view);
    }

    fn tag_mood(&self, cx: &App) -> TagMood {
        match self.services.read(cx).tag_availability() {
            TagAvailability::Available => TagMood::Available,
            TagAvailability::Taken | TagAvailability::Invalid => TagMood::Rejected,
            _ => TagMood::Unknown,
        }
    }

    fn sync_field(&mut self, cx: &mut Context<Self>) {
        let value = self.form.value_for(self.step).to_owned();
        let secure = self.step.is_secure();
        let placeholder = tr(self.step.placeholder_key());

        let height = rise_ui::theme(cx as &App).auth.field_height;
        self.field.update(cx, |field, cx| {
            field.set_height(Some(height), cx);
            field.set_secure(secure, cx);
            field.set_placeholder(placeholder, cx);
            field.reset(value, cx);
        });
    }

    fn field_changed(&mut self, cx: &mut Context<Self>) {
        let value = self.field.read(cx).text().to_owned();
        self.form.set(self.step, value.clone());
        self.local_error = None;

        if self.step == AuthStep::RegistrationTag {
            self.interactions.check_tag(&value);
        }

        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        match self
            .interactions
            .submit_step(self.step, &self.form, self.registering, cx)
        {
            StepOutcome::Advance(next) => self.move_to(next, cx),
            StepOutcome::Invalid(key) => {
                self.local_error = Some(key);
                cx.notify();
            }
            StepOutcome::Submitted | StepOutcome::Ignored => cx.notify(),
        }
    }

    fn move_to(&mut self, step: AuthStep, cx: &mut Context<Self>) {
        self.step = step;
        self.local_error = None;

        let mood = self.tag_mood(cx as &App);
        self.ensure_animation(step.animation(mood), cx);
        self.sync_field(cx);
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.step.previous() else {
            return;
        };
        self.interactions.cancel_verification_code_entry();
        self.interactions.clear_error();
        self.move_to(previous, cx);
    }

    /// Crosses between signing in and registering without losing the number.
    ///
    /// This is the reference's `beginRegistrationFromLogin` / `useLoginFromPhone`
    /// pair, and it is the answer to "there is no way to register when you have
    /// no account": the footer offers it on exactly the two steps where the user
    /// has just found out which one they are on.
    fn switch_purpose(&mut self, registering: bool, cx: &mut Context<Self>) {
        if self.registering == registering {
            return;
        }
        self.registering = registering;
        self.interactions.clear_error();

        let next = if registering {
            AuthStep::RegistrationName
        } else {
            AuthStep::LoginPassword
        };
        self.move_to(next, cx);
    }

    fn snapshot_changed(&mut self, cx: &mut Context<Self>) {
        let flow = self.services.read(cx).flow().clone();

        if flow.code_entry_active && self.step != AuthStep::VerificationCode {
            self.move_to(AuthStep::VerificationCode, cx);
            return;
        }

        // The tag verdict changes the mascot, so its pose has to be loaded
        // before the render that wants it.
        if self.step == AuthStep::RegistrationTag {
            let mood = self.tag_mood(cx as &App);
            let name = self.step.animation(mood);
            self.ensure_animation(name, cx);
        }

        cx.notify();
    }

    fn open_telegram(&mut self, cx: &mut Context<Self>) {
        if let Some(link) = self.services.read(cx).telegram_link() {
            rise_platform::gpui_shim::open_url(cx, &link);
        }
    }

    fn message_key(&self, cx: &App) -> Option<&'static str> {
        self.local_error
            .or_else(|| self.services.read(cx).error_key())
    }

    fn can_continue(&self, cx: &App) -> bool {
        !self.services.read(cx).is_busy() && validate(self.step, &self.form).is_valid()
    }

    // ── pieces of the screen ────────────────────────────────────────────────

    /// The bar across the top: a back button and one segment per step.
    fn header(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let metrics = theme.auth;
        let (index, total) = self.step.progress(self.registering);

        let segments = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.progress_gap)
            .w(metrics.progress_width)
            .children((0..total).map(|position| {
                div()
                    .flex_1()
                    .h(metrics.progress_height)
                    .rounded(theme.radius._600)
                    .bg(if position <= index {
                        theme.primary._100
                    } else {
                        theme.border._200
                    })
            }));

        let mut header = div()
            .relative()
            .w_full()
            .h(metrics.header_height)
            .flex()
            .items_center()
            .justify_center()
            .child(segments);

        if self.step.previous().is_some() {
            let mut button = div()
                .id("auth.back")
                .absolute()
                .left_0()
                .size(metrics.header_button_size)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme.bg._200)
                .cursor_pointer()
                .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| flow.back(cx)));

            if let Some(icon) =
                IconUi::render(theme, "chevron.left", IconSize::Small, theme.text.primary)
            {
                button = button.child(icon);
            }

            header = header.child(button);
        }

        header.into_any_element()
    }

    /// What sits under the step's subtitle: the field, or the Telegram hop.
    fn body(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        if self.services.read(cx as &App).telegram_link().is_some() {
            return self.telegram_step(theme, cx);
        }

        let mut body = div()
            .flex()
            .flex_col()
            .gap(theme.spacing._300)
            .w_full()
            .child(self.field.clone());

        match self.step {
            AuthStep::RegistrationTag => {
                if let Some(status) = self.tag_status(theme, cx) {
                    body = body.child(status);
                }
            }
            AuthStep::RegistrationPassword => {
                body = body.child(
                    MainText::body(theme, TextTone::Secondary)
                        .text_size(theme.typography.caption().size)
                        .child(tr("auth_password_hint")),
                );
            }
            _ => {}
        }

        body.into_any_element()
    }

    fn tag_status(&self, theme: &AppTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (key, color) = match self.services.read(cx as &App).tag_availability() {
            TagAvailability::Checking => ("auth_tag_checking", theme.text.secondary),
            TagAvailability::Available => ("auth_tag_available", theme.semantic.success_200),
            TagAvailability::Taken => ("tag_already_exists", theme.semantic.error_200),
            TagAvailability::Unknown => ("auth_tag_check_failed", theme.primary._100),
            TagAvailability::Idle | TagAvailability::Invalid => return None,
        };

        Some(
            MainText::body(theme, TextTone::Secondary)
                .text_size(theme.typography.caption().size)
                .text_color(color)
                .child(tr(key))
                .into_any_element(),
        )
    }

    /// The step the code actually comes from.
    ///
    /// The bot is not decoration: send-code succeeds by handing back a bot
    /// username, the user leaves for Telegram, shares their number, and comes
    /// back with a code. Without this the flow asks for a code nobody ever sent.
    fn telegram_step(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let metrics = theme.auth;
        let link = self
            .services
            .read(cx as &App)
            .telegram_link()
            .unwrap_or_default();

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(theme.spacing._400)
            .w_full()
            .child(
                MainText::body(theme, TextTone::Secondary)
                    .text_center()
                    .child(tr("tg_redirect_subtitle")),
            )
            .child(
                MainText::body(theme, TextTone::Secondary)
                    .text_size(theme.typography.caption().size)
                    .text_color(theme.primary._100)
                    .child(link),
            )
            .child(
                div()
                    .id("auth.telegram.open")
                    .w_full()
                    .cursor_pointer()
                    .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| flow.open_telegram(cx)))
                    .child(
                        ButtonUi::sized(theme, ButtonTone::Primary, metrics.field_height)
                            .w_full()
                            .child(tr("tg_redirect_open_btn")),
                    ),
            )
            .child(
                div()
                    .id("auth.telegram.done")
                    .w_full()
                    .cursor_pointer()
                    .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| {
                        flow.interactions.complete_telegram_redirect();
                        flow.move_to(AuthStep::VerificationCode, cx);
                    }))
                    .child(
                        ButtonUi::sized(theme, ButtonTone::Neutral, metrics.field_height)
                            .w_full()
                            .child(tr("tg_redirect_got_code_btn")),
                    ),
            )
            .into_any_element()
    }

    /// The primary action, and the way across to the other purpose.
    fn footer(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let metrics = theme.auth;
        let mut footer = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(theme.spacing._400)
            .w_full();

        // "No account? Sign up" on the password step, "Have an account? Sign in"
        // on the phone step of a registration — the reference's own two.
        let crossover = match (self.step, self.registering) {
            (AuthStep::LoginPassword, _) => Some(("noaccount", "signup", true)),
            (AuthStep::Phone, true) => Some(("haveaccount", "signin", false)),
            _ => None,
        };

        if let Some((prefix, action, to_registration)) = crossover {
            footer = footer.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing._200)
                    .child(MainText::body(theme, TextTone::Secondary).child(tr(prefix)))
                    .child(
                        div()
                            .id("auth.crossover")
                            .cursor_pointer()
                            .on_click(cx.listener(move |flow, _: &ClickEvent, _, cx| {
                                flow.switch_purpose(to_registration, cx)
                            }))
                            .child(
                                MainText::body(theme, TextTone::Primary)
                                    .text_color(theme.primary._100)
                                    .child(tr(action)),
                            ),
                    ),
            );
        }

        let enabled = self.can_continue(cx as &App);
        let label = self.step.action_key().map(tr).unwrap_or_else(|| tr("next"));

        footer
            .child(
                div()
                    .id("auth.submit")
                    .w_full()
                    .when(enabled, |row| row.cursor_pointer())
                    .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| flow.submit(cx)))
                    .child(
                        // One token for the field and the button, so the primary
                        // action is never a different size from the control it
                        // submits.
                        ButtonUi::sized(theme, ButtonTone::Primary, metrics.field_height)
                            .w_full()
                            .opacity(if enabled { 1.0 } else { 0.5 })
                            .child(label),
                    ),
            )
            .into_any_element()
    }
}

impl Focusable for AuthStepFlow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AuthStepFlow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme: AppTheme = rise_ui::theme(cx as &App).clone();
        let metrics = theme.auth;
        let on_telegram = self.services.read(cx).telegram_link().is_some();
        let mood = self.tag_mood(cx as &App);
        let message = self.message_key(cx as &App);

        // The caret belongs in the field the step is about, so typing works the
        // moment the screen appears rather than after a click. Not while the
        // Telegram step is up: there is no field to type into.
        if !on_telegram && window.focused(cx).is_none() {
            let handle = self.field.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }

        let header = self.header(&theme, cx);
        let body = self.body(&theme, cx);
        let footer = self.footer(&theme, cx);

        let mut page = div()
            .flex()
            .flex_col()
            .items_center()
            .w(metrics.content_width)
            .max_w_full();

        if let Some(animation) = self.animations.get(self.step.animation(mood)) {
            page = page.child(animation.clone());
        }

        page = page
            .child(
                div()
                    .mt(metrics.step_art_gap)
                    .text_center()
                    .text_size(theme.typography.display_small().size)
                    .line_height(theme.typography.display_small().line_height)
                    .font(theme.typography.display_small().font)
                    .text_color(theme.text.primary)
                    .child(tr(self.step.title_key())),
            )
            .child(
                div().mt(theme.spacing._400).text_center().child(
                    MainText::body(&theme, TextTone::Secondary).child(tr(self.step.subtitle_key())),
                ),
            )
            .child(div().mt(metrics.step_field_gap).w_full().child(body));

        if let Some(key) = message {
            page = page.child(
                div().mt(theme.spacing._300).child(
                    MainText::body(&theme, TextTone::Secondary)
                        .text_size(theme.typography.caption().size)
                        .text_color(theme.semantic.error_200)
                        .child(tr(key)),
                ),
            );
        }

        BoxUi::screen(&theme)
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .items_center()
            // The window has no titlebar, so the header sits below the band the
            // traffic lights occupy rather than under them.
            .pt(theme.shell.window_drag_height)
            .px(metrics.content_padding)
            .pb(metrics.content_padding)
            .child(div().w(metrics.content_width).max_w_full().child(header))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(page),
            )
            .child(div().w(metrics.content_width).max_w_full().child(footer))
    }
}

#[cfg(test)]
#[path = "auth_step_flow_tests.rs"]
mod tests;

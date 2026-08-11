use std::collections::HashMap;

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement,
    Render, Subscription, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_media::lottie::lottie_view::LottieView;
use rise_theme::AppTheme;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState};
use rise_ui::phone_input::{PhoneCountry, countries};
use rise_ui::{BoxUi, ButtonTone, ButtonUi, CodeFieldUi, IconSize, IconUi, MainText, TextTone};
use rise_widgets::{CountryMenu, CountryMenuEvent, ModalAction, ModalUi, ModalWidth};

use crate::core::animations;
use crate::modules::auth::engine::rise_auth_presentation::TagAvailability;
use crate::modules::auth::shared::auth_validation;
use crate::modules::auth::stores::auth_actions::auth_types::{
    AuthStep, AuthStepForm, TagMood, validate,
};
use crate::modules::auth::stores::auth_interactions::{AuthInteractionsStore, StepOutcome};
use crate::modules::auth::stores::auth_services::AuthServicesStore;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthMode {
    SignIn,
    SignUp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthFlowEvent {
    Dismissed,
}

pub struct AuthStepFlow {
    registering: bool,
    can_dismiss: bool,
    step: AuthStep,
    form: AuthStepForm,
    field: Entity<InputUiState>,
    interactions: AuthInteractionsStore,
    services: Entity<AuthServicesStore>,
    animations: HashMap<&'static str, Entity<LottieView>>,
    animation_side: gpui::Pixels,
    scale_factor: f32,
    local_error: Option<&'static str>,
    can_reuse_code: bool,
    // `clear_error` round-trips through the actor: hides the stale key until the snapshot agrees.
    suppressed_error: Option<&'static str>,
    registration_started_from_login: bool,
    country_menu: Option<Entity<CountryMenu>>,
    country_subscription: Option<Subscription>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl AuthStepFlow {
    pub fn new(
        mode: AuthMode,
        can_dismiss: bool,
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
            cx.observe(&services, |flow, _, cx| flow.snapshot_changed(cx)),
        ];

        let mut flow = Self {
            registering: matches!(mode, AuthMode::SignUp),
            can_dismiss,
            step: AuthStep::Phone,
            form: AuthStepForm::detected(
                countries(),
                rise_platform::device_locale::current().region.as_deref(),
            ),
            field,
            interactions,
            services,
            animations: HashMap::new(),
            animation_side: rise_ui::theme(cx as &App).auth.step_art_size,
            scale_factor: window.scale_factor(),
            local_error: None,
            can_reuse_code: false,
            suppressed_error: None,
            registration_started_from_login: false,
            country_menu: None,
            country_subscription: None,
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
        let typed = self.field.read(cx).text().to_owned();

        let filtered = match self.step {
            AuthStep::RegistrationTag => auth_validation::tag_input(&typed),
            AuthStep::VerificationCode => auth_validation::code_input(&typed),
            _ => typed.clone(),
        };

        self.form.set(self.step, filtered.clone());
        self.local_error = None;

        let shown = if self.step == AuthStep::Phone {
            self.form.phone.national().to_owned()
        } else {
            filtered.clone()
        };
        if shown != typed {
            self.field.update(cx, |field, cx| field.reset(shown, cx));
        }

        if self.step == AuthStep::RegistrationTag {
            self.interactions.check_tag(&filtered);
        }

        if self.step == AuthStep::VerificationCode && CodeFieldUi::is_complete(&filtered) {
            self.submit(cx);
            return;
        }

        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        match self.interactions.submit_step(
            self.step,
            &self.form,
            self.registering,
            self.can_reuse_code,
            cx,
        ) {
            StepOutcome::Advance(next) => {
                if next == AuthStep::VerificationCode {
                    self.can_reuse_code = false;
                }
                self.move_to(next, cx)
            }
            StepOutcome::Invalid(key) => {
                self.local_error = Some(key);
                cx.notify();
            }
            StepOutcome::Submitted | StepOutcome::Ignored => cx.notify(),
        }
    }

    fn recover_from_tag_conflict(&mut self, cx: &mut Context<Self>) {
        self.can_reuse_code = self.form.code.chars().count() == auth_validation::CODE_LENGTH;
        self.dismiss_refusal(cx);
        self.interactions.mark_tag_taken();
        self.move_to(AuthStep::RegistrationTag, cx);
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
        if self.services.read(cx as &App).telegram_link().is_some() {
            self.finish_telegram_redirect(cx);
            return;
        }

        if self.step == AuthStep::RegistrationName && self.registration_started_from_login {
            self.registering = false;
            self.registration_started_from_login = false;
            self.dismiss_refusal(cx);
            self.move_to(AuthStep::LoginPassword, cx);
            return;
        }

        let Some(previous) = self.step.previous() else {
            if self.can_dismiss {
                cx.emit(AuthFlowEvent::Dismissed);
            }
            return;
        };
        self.interactions.cancel_verification_code_entry();
        self.dismiss_refusal(cx);
        self.move_to(previous, cx);
    }

    fn begin_registration_from_login(&mut self, cx: &mut Context<Self>) {
        if self.registering {
            return;
        }
        self.registering = true;
        self.registration_started_from_login = true;
        self.dismiss_refusal(cx);
        self.move_to(AuthStep::RegistrationName, cx);
    }

    fn use_login_from_phone(&mut self, cx: &mut Context<Self>) {
        if !self.registering {
            return;
        }
        self.registering = false;
        self.registration_started_from_login = false;
        self.local_error = None;
        self.dismiss_refusal(cx);
    }

    fn snapshot_changed(&mut self, cx: &mut Context<Self>) {
        let flow = self.services.read(cx).flow().clone();

        if flow.error_key != self.suppressed_error {
            self.suppressed_error = None;
        }

        if self.step == AuthStep::VerificationCode
            && self.server_refusal(cx as &App) == Some("tag_already_exists")
        {
            self.recover_from_tag_conflict(cx);
            return;
        }

        if flow.code_entry_active && self.step != AuthStep::VerificationCode {
            self.move_to(AuthStep::VerificationCode, cx);
            return;
        }

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

    fn message_key(&self) -> Option<&'static str> {
        self.local_error
    }

    fn server_refusal(&self, cx: &App) -> Option<&'static str> {
        self.services
            .read(cx)
            .error_key()
            .filter(|key| Some(*key) != self.suppressed_error)
    }

    fn dismiss_refusal(&mut self, cx: &mut Context<Self>) {
        self.suppressed_error = self.services.read(cx as &App).error_key();
        self.interactions.clear_error();
        cx.notify();
    }

    fn can_continue(&self, cx: &App) -> bool {
        let services = self.services.read(cx);
        if services.is_busy() || !validate(self.step, &self.form).is_valid() {
            return false;
        }
        if self.step == AuthStep::RegistrationTag {
            return services.tag_is_confirmed_free();
        }
        true
    }

    fn header(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let metrics = theme.auth;
        let (index, total) = self.step.progress(self.registering);
        let busy = self.services.read(cx as &App).is_busy();

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

        if self.step.previous().is_some() || self.can_dismiss {
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
                .when(!busy, |button| {
                    button
                        .cursor_pointer()
                        .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| flow.back(cx)))
                })
                .when(busy, |button| button.opacity(0.5));

            if let Some(icon) =
                IconUi::render(theme, "chevron.left", IconSize::Small, theme.text.primary)
            {
                button = button.child(icon);
            }

            header = header.child(button);
        }

        header.into_any_element()
    }

    fn country_button(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let country = self.form.phone.country().clone();
        let metrics = theme.auth;

        let open = self.country_menu.is_some();

        let mut button =
            div()
                .id("auth.country")
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing._200)
                .h(metrics.field_height)
                .px(theme.input.padding_x)
                .rounded(theme.input.radius_300)
                .bg(theme.input.bg_200)
                .border_1()
                .border_color(if open {
                    theme.primary._100
                } else {
                    theme.input.border_300
                })
                .cursor_pointer()
                .on_click(cx.listener(|flow, _: &ClickEvent, window, cx| {
                    flow.toggle_country_menu(window, cx)
                }));

        button = button.child(rise_ui::FlagUi::render(
            theme,
            &country.region,
            metrics.flag_width,
        ));

        button =
            button.child(MainText::body(theme, TextTone::Primary).child(country.display_code()));

        let chevron = if self.country_menu.is_some() {
            "chevron.up"
        } else {
            "chevron.down"
        };
        if let Some(icon) = IconUi::render(theme, chevron, IconSize::Small, theme.text.secondary) {
            button = button.child(icon);
        }

        button.into_any_element()
    }

    fn toggle_country_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.country_menu.is_some() {
            self.close_country_menu(window, cx);
        } else {
            self.open_country_menu(window, cx);
        }
    }

    fn open_country_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.form.phone.country().region.clone();
        let catalogue = countries().all().to_vec();

        let menu = cx.new(|cx| CountryMenu::new(catalogue, &selected, cx));

        self.country_subscription =
            Some(
                cx.subscribe_in(&menu, window, |flow, _, event, window, cx| match event {
                    CountryMenuEvent::Chose(country) => {
                        flow.choose_country(country.clone(), window, cx)
                    }
                    CountryMenuEvent::Dismissed => flow.close_country_menu(window, cx),
                }),
            );

        let handle = menu.read(cx).query_focus_handle(cx as &App);
        window.focus(&handle, cx);
        menu.read(cx).reveal_selection();

        self.country_menu = Some(menu);
        cx.notify();
    }

    fn close_country_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.country_menu.take().is_none() {
            return;
        }
        self.country_subscription = None;

        let field = self.field.read(cx).focus_handle(cx);
        window.focus(&field, cx);
        cx.notify();
    }

    fn choose_country(
        &mut self,
        country: PhoneCountry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.form.phone.set_country(country);
        self.close_country_menu(window, cx);
        self.sync_field(cx);
        cx.notify();
    }

    fn body(&self, theme: &AppTheme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div().flex().flex_col().gap(theme.spacing._300).w_full();

        if self.step == AuthStep::Phone {
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.spacing._300)
                    .w_full()
                    .child(self.country_button(theme, cx))
                    .child(div().flex_1().child(self.field.clone())),
            );
        } else if self.step == AuthStep::VerificationCode {
            let focused = self
                .field
                .read(cx as &App)
                .focus_handle(cx as &App)
                .is_focused(window);
            body = body.child(CodeFieldUi::render(
                theme,
                "auth.code",
                &self.form.code,
                self.field.clone(),
                focused,
                cx.listener(|flow, _: &ClickEvent, window, cx| {
                    let handle = flow.field.read(cx as &App).focus_handle(cx as &App);
                    window.focus(&handle, cx);
                }),
            ));
        } else {
            body = body.child(self.field.clone());
        }

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
        let services = self.services.read(cx as &App);
        let (key, color) = match services.tag_availability() {
            TagAvailability::Checking => ("auth_tag_checking", theme.text.secondary),
            TagAvailability::Available => ("auth_tag_available", theme.semantic.success_200),
            TagAvailability::Taken => ("tag_already_exists", theme.semantic.error_200),
            TagAvailability::Unknown => ("auth_tag_check_failed", theme.primary._100),
            TagAvailability::Invalid => (
                services
                    .tag_problem_key()
                    .unwrap_or("tag_invalid_characters"),
                theme.semantic.error_200,
            ),
            TagAvailability::Idle => return None,
        };

        Some(
            MainText::body(theme, TextTone::Secondary)
                .text_size(theme.typography.caption().size)
                .text_color(color)
                .child(tr(key))
                .into_any_element(),
        )
    }

    fn telegram_modal(
        &self,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let link = self.services.read(cx as &App).telegram_link()?;

        let flow = cx.entity();
        let finish = move |_window: &mut Window, cx: &mut App| {
            flow.update(cx, |flow, cx| flow.finish_telegram_redirect(cx));
        };
        let open = {
            let flow = cx.entity();
            move |_window: &mut Window, cx: &mut App| {
                flow.update(cx, |flow, cx| flow.open_telegram(cx));
            }
        };

        let modal = ModalUi::new("auth.telegram")
            .title(tr("tg_redirect_title"))
            .subtitle(tr("tg_redirect_subtitle"))
            .width(ModalWidth::Medium)
            .without_scroll()
            .track_focus(&self.focus_handle)
            .on_dismiss(finish.clone())
            .child(
                div()
                    .w_full()
                    .text_center()
                    .child(
                        MainText::body(theme, TextTone::Secondary)
                            .text_size(theme.typography.caption().size)
                            .text_color(theme.primary._100)
                            .child(link),
                    )
                    .into_any_element(),
            )
            .action(
                ModalAction::primary("auth.telegram.open", tr("tg_redirect_open_btn"))
                    .on_click(open),
            )
            .action(
                ModalAction::neutral("auth.telegram.done", tr("tg_redirect_got_code_btn"))
                    .on_click(finish),
            );

        Some(modal.render(theme, window, cx))
    }

    fn finish_telegram_redirect(&mut self, cx: &mut Context<Self>) {
        self.interactions.complete_telegram_redirect();
        self.move_to(AuthStep::VerificationCode, cx);
    }

    fn footer(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let metrics = theme.auth;
        let mut footer = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(theme.spacing._400)
            .w_full();

        let busy = self.services.read(cx as &App).is_busy();

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
                            .when(!busy, |link| {
                                link.cursor_pointer().on_click(cx.listener(
                                    move |flow, _: &ClickEvent, _, cx| {
                                        if to_registration {
                                            flow.begin_registration_from_login(cx);
                                        } else {
                                            flow.use_login_from_phone(cx);
                                        }
                                    },
                                ))
                            })
                            .when(busy, |link| link.opacity(0.5))
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
                    .when(enabled, |row| {
                        row.cursor_pointer()
                            .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| flow.submit(cx)))
                    })
                    .child(
                        ButtonUi::sized(theme, ButtonTone::Primary, metrics.field_height)
                            .w_full()
                            .opacity(if enabled { 1.0 } else { 0.5 })
                            .child(label),
                    ),
            )
            .into_any_element()
    }
}

impl gpui::EventEmitter<AuthFlowEvent> for AuthStepFlow {}

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
        let message = self.message_key();

        if !on_telegram && window.focused(cx).is_none() {
            let handle = self.field.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }

        let header = self.header(&theme, cx);
        let body = self.body(&theme, window, cx);
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
            .child(div().mt(theme.spacing._400).text_center().child(
                MainText::body(&theme, TextTone::Secondary).child(tr(self.step.subtitle_key())),
            ))
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

        let mut screen = BoxUi::screen(&theme)
            .track_focus(&self.focus_handle)
            .relative()
            .flex()
            .flex_col()
            .items_center()
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
            .child(div().w(metrics.content_width).max_w_full().child(footer));

        if let Some(key) = self.server_refusal(cx as &App) {
            let title = if self.registering {
                "register_error_title"
            } else {
                "auth_signin_error_title"
            };
            let heading = theme.typography.headline();

            screen = screen.child(
                div()
                    .id("auth.refusal.scrim")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.material.scrim)
                    .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| flow.dismiss_refusal(cx)))
                    .child(
                        div()
                            .occlude()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(theme.spacing._400)
                            .w(metrics.content_width)
                            .max_w_full()
                            .p(theme.spacing._600)
                            .bg(theme.bg._100)
                            .border_1()
                            .border_color(theme.border._200)
                            .rounded(theme.radius._400)
                            .shadow_lg()
                            .child(
                                div()
                                    .text_center()
                                    .text_size(heading.size)
                                    .line_height(heading.line_height)
                                    .font(heading.font)
                                    .text_color(theme.text.primary)
                                    .child(tr(title)),
                            )
                            .child(
                                div().text_center().child(
                                    MainText::body(&theme, TextTone::Secondary).child(tr(key)),
                                ),
                            )
                            .child(
                                div()
                                    .id("auth.refusal.ok")
                                    .w_full()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|flow, _: &ClickEvent, _, cx| {
                                        flow.dismiss_refusal(cx)
                                    }))
                                    .child(
                                        ButtonUi::sized(
                                            &theme,
                                            ButtonTone::Primary,
                                            metrics.field_height,
                                        )
                                        .w_full()
                                        .child(tr("ok")),
                                    ),
                            ),
                    ),
            );
        }

        if let Some(modal) = self.telegram_modal(&theme, window, cx) {
            screen = screen.child(modal);
        }

        if let Some(menu) = self.country_menu.clone() {
            screen = screen.child(
                div()
                    .id("auth.country.scrim")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.material.scrim)
                    .on_click(cx.listener(|flow, _: &ClickEvent, window, cx| {
                        flow.close_country_menu(window, cx)
                    }))
                    .child(menu),
            );
        }

        screen
    }
}

#[cfg(test)]
#[path = "auth_step_flow_tests.rs"]
mod tests;

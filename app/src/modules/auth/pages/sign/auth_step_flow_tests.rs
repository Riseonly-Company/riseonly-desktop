use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::TestAppContext;
use rise_engine::{HttpDescriptor, MethodDescriptor, WireError};
use rise_platform::InMemorySecureStore;
use serde_json::{Value, json};

use crate::core::engine_bridge::SocketCredential;
use crate::modules::auth::engine::core::rise_auth_credential_store::AuthCredentialStore;
use crate::modules::auth::engine::core::rise_auth_transport::AuthTransport;
use crate::modules::auth::engine::rise_auth_domain::RiseAuthDomain;
use crate::modules::auth::stores::auth_actions::AuthActionsStore;

use super::*;

struct SilentTransport;

impl AuthTransport for SilentTransport {
    fn call(
        &self,
        _descriptor: &'static MethodDescriptor,
        _body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        Box::pin(async { Ok(json!({})) })
    }

    fn http(
        &self,
        _descriptor: &'static HttpDescriptor,
        _body: Value,
        _authorization: Option<String>,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        Box::pin(async { Ok(json!({})) })
    }

    fn authenticate(&self, _credential: SocketCredential) {}
}

struct Harness {
    _runtime: tokio::runtime::Runtime,
    directory: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

fn open(
    mode: AuthMode,
    name: &str,
    cx: &mut TestAppContext,
) -> (gpui::WindowHandle<AuthStepFlow>, Harness) {
    let directory = std::env::temp_dir().join(format!("rise-auth-flow-{name}"));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let domain = Arc::new(RiseAuthDomain::for_test(
        runtime.handle(),
        Arc::new(SilentTransport),
        Arc::new(AuthCredentialStore::new(&directory)),
        Arc::new(InMemorySecureStore::new()),
    ));

    cx.update(|cx| {
        if !cx.has_global::<rise_theme::AppTheme>() {
            rise_ui::install_theme(rise_theme::AppTheme::dark(), cx);
        }
    });

    let services = cx.update(|cx| {
        cx.new(|cx| {
            crate::modules::auth::stores::auth_services::AuthServicesStore::new(
                Arc::clone(&domain),
                cx,
            )
        })
    });
    let interactions = AuthInteractionsStore::new(AuthActionsStore::new(domain), services);

    let window =
        cx.add_window(|window, cx| AuthStepFlow::new(mode, true, interactions, window, cx));
    (
        window,
        Harness {
            _runtime: runtime,
            directory,
        },
    )
}

fn type_into(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext, text: &str) {
    window
        .update(cx, |flow, _, cx| {
            let field = flow.field.clone();
            field.update(cx, |field, cx| field.set_text(text, cx));
            flow.field_changed(cx);
        })
        .expect("window is open");
}

fn submit(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) {
    window
        .update(cx, |flow, _, cx| flow.submit(cx))
        .expect("window is open");
}

fn confirm_tag(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) {
    window
        .update(cx, |flow, _, cx| {
            flow.services.update(cx, |services, cx| {
                services.force_tag_availability(TagAvailability::Available, cx);
            });
        })
        .expect("window is open");
}

fn step(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) -> AuthStep {
    window.update(cx, |flow, _, _| flow.step()).unwrap()
}

fn message(
    window: &gpui::WindowHandle<AuthStepFlow>,
    cx: &mut TestAppContext,
) -> Option<&'static str> {
    window.update(cx, |flow, _, _| flow.message_key()).unwrap()
}

fn refusal(
    window: &gpui::WindowHandle<AuthStepFlow>,
    cx: &mut TestAppContext,
) -> Option<&'static str> {
    window
        .update(cx, |flow, _, cx| flow.server_refusal(cx as &App))
        .unwrap()
}

#[gpui::test]
fn signing_in_goes_from_the_phone_to_the_password(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "signin-steps", cx);
    assert_eq!(step(&window, cx), AuthStep::Phone);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);

    assert_eq!(step(&window, cx), AuthStep::LoginPassword);
}

#[gpui::test]
fn signing_up_goes_from_the_same_phone_step_to_the_profile(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "signup-steps", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);

    assert_eq!(
        step(&window, cx),
        AuthStep::RegistrationName,
        "the phone step is shared; only the mode decides what follows it"
    );
}

#[gpui::test]
fn a_short_phone_number_never_leaves_the_step(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "short-phone", cx);

    type_into(&window, cx, "123");
    submit(&window, cx);

    assert_eq!(step(&window, cx), AuthStep::Phone);
    assert_eq!(message(&window, cx), Some("auth_phone_invalid"));
}

#[gpui::test]
fn a_keystroke_clears_the_problem_the_last_press_reported(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "clears", cx);

    type_into(&window, cx, "123");
    submit(&window, cx);
    assert!(message(&window, cx).is_some());

    type_into(&window, cx, "1234567");
    assert_eq!(
        message(&window, cx),
        None,
        "an error the user is already fixing must not stay under the field"
    );
}

#[gpui::test]
fn going_back_keeps_what_the_earlier_steps_collected(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "back-keeps", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    type_into(&window, cx, "Name");
    submit(&window, cx);
    assert_eq!(step(&window, cx), AuthStep::RegistrationTag);

    window.update(cx, |flow, _, cx| flow.back(cx)).unwrap();
    assert_eq!(step(&window, cx), AuthStep::RegistrationName);
    assert_eq!(
        window
            .update(cx, |flow, _, _| flow.form().name.clone())
            .unwrap(),
        "Name",
        "walking back must not throw away a step the user already completed"
    );

    window.update(cx, |flow, _, cx| flow.back(cx)).unwrap();
    assert_eq!(step(&window, cx), AuthStep::Phone);
    assert_eq!(
        window
            .update(cx, |flow, _, _| flow.form().phone.national().to_owned())
            .unwrap(),
        "9991234567"
    );
}

#[gpui::test]
fn back_from_the_first_step_does_nothing(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "back-root", cx);
    window.update(cx, |flow, _, cx| flow.back(cx)).unwrap();
    assert_eq!(step(&window, cx), AuthStep::Phone);
}

#[gpui::test]
fn the_field_becomes_secure_exactly_on_the_password_steps(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "secure", cx);

    let is_secure = |cx: &mut TestAppContext| {
        window
            .update(cx, |flow, _, cx| flow.field.read(cx).is_secure())
            .unwrap()
    };

    assert!(!is_secure(cx));
    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    assert!(is_secure(cx), "a password must not be typed in the clear");
}

#[gpui::test]
fn the_field_shows_the_value_the_step_it_moved_to_already_had(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "field-sync", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    type_into(&window, cx, "Name");
    window.update(cx, |flow, _, cx| flow.back(cx)).unwrap();

    assert_eq!(
        window
            .update(cx, |flow, _, cx| flow.field.read(cx).text().to_owned())
            .unwrap(),
        "9991234567",
        "the field is reused across steps, so it has to be rewritten on every move"
    );
}

#[gpui::test]
fn a_password_that_does_not_match_its_confirmation_is_named_as_a_mismatch(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "mismatch", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    type_into(&window, cx, "Name");
    submit(&window, cx);
    type_into(&window, cx, "riseonly");
    confirm_tag(&window, cx);
    submit(&window, cx);
    type_into(&window, cx, "password12");
    submit(&window, cx);
    assert_eq!(
        step(&window, cx),
        AuthStep::RegistrationPasswordConfirmation
    );

    type_into(&window, cx, "different");
    submit(&window, cx);

    assert_eq!(
        step(&window, cx),
        AuthStep::RegistrationPasswordConfirmation
    );
    assert_eq!(message(&window, cx), Some("auth_password_mismatch"));
}

fn is_registering(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) -> bool {
    window
        .update(cx, |flow, _, _| flow.is_registering())
        .unwrap()
}

fn cross_over_to_sign_in(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) {
    window
        .update(cx, |flow, _, cx| flow.use_login_from_phone(cx))
        .expect("window is open");
}

fn cross_over_to_registration(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) {
    window
        .update(cx, |flow, _, cx| flow.begin_registration_from_login(cx))
        .expect("window is open");
}

fn back(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) {
    window
        .update(cx, |flow, _, cx| flow.back(cx))
        .expect("window is open");
}

#[gpui::test]
fn crossing_over_to_sign_in_stays_on_the_phone_step(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "crossover-stays", cx);

    type_into(&window, cx, "9991234567");
    assert!(is_registering(&window, cx));

    cross_over_to_sign_in(&window, cx);

    assert_eq!(
        step(&window, cx),
        AuthStep::Phone,
        "the reference does not move here, and neither may this"
    );
    assert!(!is_registering(&window, cx));
    assert_eq!(
        window
            .update(cx, |flow, _, cx| flow.field.read(cx).text().to_owned())
            .unwrap(),
        "9991234567",
        "the number is the same number either way"
    );
}

#[gpui::test]
fn continuing_after_the_crossover_goes_to_the_login_password(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "crossover-continue", cx);

    type_into(&window, cx, "9991234567");
    cross_over_to_sign_in(&window, cx);
    submit(&window, cx);

    assert_eq!(step(&window, cx), AuthStep::LoginPassword);
}

#[gpui::test]
fn crossing_over_to_registration_moves_to_the_name_step(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "crossover-register", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    assert_eq!(step(&window, cx), AuthStep::LoginPassword);

    cross_over_to_registration(&window, cx);

    assert_eq!(step(&window, cx), AuthStep::RegistrationName);
    assert!(is_registering(&window, cx));
}

#[gpui::test]
fn going_back_returns_to_the_step_the_detour_began_on(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "crossover-back", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    cross_over_to_registration(&window, cx);

    back(&window, cx);

    assert_eq!(step(&window, cx), AuthStep::LoginPassword);
    assert!(
        !is_registering(&window, cx),
        "coming back out of the detour is coming back to signing in"
    );
}

#[gpui::test]
fn a_registration_begun_at_the_phone_step_goes_back_to_it(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "back-plain", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    assert_eq!(step(&window, cx), AuthStep::RegistrationName);

    back(&window, cx);

    assert_eq!(step(&window, cx), AuthStep::Phone);
    assert!(is_registering(&window, cx));
}

#[gpui::test]
fn a_server_refusal_never_becomes_the_fields_own_message(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "refusal-channel", cx);

    window
        .update(cx, |flow, _, cx| {
            flow.services.update(cx, |services, cx| {
                services.force_error(Some("auth_signin_error"), cx);
            });
        })
        .unwrap();

    assert_eq!(
        message(&window, cx),
        None,
        "the caption belongs to local validation alone"
    );
    assert_eq!(refusal(&window, cx), Some("auth_signin_error"));

    window
        .update(cx, |flow, _, cx| flow.dismiss_refusal(cx))
        .unwrap();

    assert_eq!(
        refusal(&window, cx),
        None,
        "dismissing must stop showing it now, not when the actor answers"
    );
}

#[gpui::test]
fn a_refusal_does_not_follow_the_user_across_the_crossover(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignIn, "refusal-crossover", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    window
        .update(cx, |flow, _, cx| {
            flow.services.update(cx, |services, cx| {
                services.force_error(Some("auth_signin_error"), cx);
            });
        })
        .unwrap();
    assert_eq!(refusal(&window, cx), Some("auth_signin_error"));

    cross_over_to_registration(&window, cx);

    assert_eq!(step(&window, cx), AuthStep::RegistrationName);
    assert_eq!(
        refusal(&window, cx),
        None,
        "the sign-in refusal has nothing to say about a name"
    );
}

fn force_error(
    window: &gpui::WindowHandle<AuthStepFlow>,
    cx: &mut TestAppContext,
    key: Option<&'static str>,
) {
    window
        .update(cx, |flow, _, cx| {
            flow.services
                .update(cx, |services, cx| services.force_error(key, cx));
        })
        .unwrap();
}

#[gpui::test]
fn a_tag_taken_mid_registration_goes_back_to_the_tag_step(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "tag-conflict", cx);

    type_into(&window, cx, "9991234567");
    submit(&window, cx);
    type_into(&window, cx, "Name");
    submit(&window, cx);
    type_into(&window, cx, "riseonly");
    confirm_tag(&window, cx);
    submit(&window, cx);
    type_into(&window, cx, "password12");
    submit(&window, cx);
    type_into(&window, cx, "password12");
    submit(&window, cx);

    window
        .update(cx, |flow, _, cx| {
            flow.move_to(AuthStep::VerificationCode, cx)
        })
        .unwrap();
    type_into(&window, cx, "1234");

    force_error(&window, cx, Some("tag_already_exists"));
    window
        .update(cx, |flow, _, cx| flow.snapshot_changed(cx))
        .unwrap();

    assert_eq!(
        step(&window, cx),
        AuthStep::RegistrationTag,
        "a recoverable refusal must not be a dead end"
    );
    assert_eq!(
        refusal(&window, cx),
        None,
        "the alert is answered by moving, not by telling them off"
    );
    assert_eq!(
        window
            .update(cx, |flow, _, _| flow.form().code.clone())
            .unwrap(),
        "1234",
        "the code is still valid on the server; asking for a new one costs a message"
    );
}

#[gpui::test]
fn a_recovered_registration_reuses_the_code_it_already_has(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "tag-conflict-resume", cx);

    window
        .update(cx, |flow, _, cx| {
            flow.form.code = "1234".into();
            flow.form.registration_password = "password12".into();
            flow.form.repeated_password = "password12".into();
            flow.move_to(AuthStep::VerificationCode, cx);
        })
        .unwrap();
    force_error(&window, cx, Some("tag_already_exists"));
    window
        .update(cx, |flow, _, cx| flow.snapshot_changed(cx))
        .unwrap();
    force_error(&window, cx, None);

    type_into(&window, cx, "another");
    confirm_tag(&window, cx);
    submit(&window, cx);
    assert_eq!(step(&window, cx), AuthStep::RegistrationPassword);
    submit(&window, cx);
    assert_eq!(
        step(&window, cx),
        AuthStep::RegistrationPasswordConfirmation
    );

    submit(&window, cx);

    assert_eq!(
        step(&window, cx),
        AuthStep::VerificationCode,
        "the code is still live, so send_code must be skipped entirely"
    );
}

#[gpui::test]
fn the_fourth_digit_submits_the_code(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "code-autosubmit", cx);

    window
        .update(cx, |flow, _, cx| {
            flow.move_to(AuthStep::VerificationCode, cx)
        })
        .unwrap();

    type_into(&window, cx, "123");
    assert_eq!(step(&window, cx), AuthStep::VerificationCode);
    assert_eq!(message(&window, cx), None, "three digits is not yet wrong");

    type_into(&window, cx, "1234");
    assert_eq!(message(&window, cx), None);
}

#[gpui::test]
fn the_code_field_keeps_only_four_digits(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "code-filter", cx);

    window
        .update(cx, |flow, _, cx| {
            flow.move_to(AuthStep::VerificationCode, cx)
        })
        .unwrap();

    type_into(&window, cx, "12a3 b4567");

    assert_eq!(
        window
            .update(cx, |flow, _, _| flow.form().code.clone())
            .unwrap(),
        "1234"
    );
}

#[gpui::test]
fn dismissing_the_telegram_hand_off_advances_to_the_code(cx: &mut TestAppContext) {
    let (window, _harness) = open(AuthMode::SignUp, "telegram-modal", cx);

    window
        .update(cx, |flow, _, cx| {
            flow.move_to(AuthStep::RegistrationPasswordConfirmation, cx);
            flow.services.update(cx, |services, cx| {
                services.force_telegram_redirect("riseonly_bot", cx);
            });
        })
        .unwrap();

    assert_eq!(
        step(&window, cx),
        AuthStep::RegistrationPasswordConfirmation,
        "the modal floats over the step; it does not replace it"
    );

    back(&window, cx);

    assert_eq!(
        step(&window, cx),
        AuthStep::VerificationCode,
        "closing means the user has been to the bot, not that they cancelled"
    );
}

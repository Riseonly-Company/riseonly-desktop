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

/// A transport that answers nothing.
///
/// These tests are about the SCREEN — which step is on it, what a keystroke
/// does, what an invalid field says — and the repository's own rules are tested
/// against a scripted server in `rise_auth_repository_tests.rs`. Answering here
/// would only make the screen tests depend on the wire.
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

    let window = cx.add_window(|window, cx| AuthStepFlow::new(mode, interactions, window, cx));
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

fn step(window: &gpui::WindowHandle<AuthStepFlow>, cx: &mut TestAppContext) -> AuthStep {
    window.update(cx, |flow, _, _| flow.step()).unwrap()
}

fn message(
    window: &gpui::WindowHandle<AuthStepFlow>,
    cx: &mut TestAppContext,
) -> Option<&'static str> {
    window
        .update(cx, |flow, _, cx| flow.message_key(cx as &App))
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
            .update(cx, |flow, _, _| flow.form().phone.clone())
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
    submit(&window, cx);
    type_into(&window, cx, "password");
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

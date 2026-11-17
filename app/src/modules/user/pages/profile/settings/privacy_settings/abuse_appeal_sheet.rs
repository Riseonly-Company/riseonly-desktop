use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, Render, Subscription, Window};
use rise_canon::rise_blob::abuse_appeal;
use rise_canon::rise_http_path;
use rise_canon::rise_key;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_theme::ui_color_system::UIColor;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::NativeBottomSheetPresentationHost;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::core::lib::sai::modules::sai_fetch::sai_fetch::SaiFetch;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_error::SaiFetchHttpError;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_options::SaiFetchOptions;
use crate::core::stores::http::http_service::HttpService;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::auth::services::app_attest_client::{AppAttestClient, AppAttestClientEnvelope};
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::auth::stores::auth_actions::auth_wire_types::{AuthSendCodeResponse, AuthSendCodeResponseExt};
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

const CODE_MINIMUM_LENGTH: usize = 4;
const REASON_BULLET: &str = "\u{2022}";
const SHEET_HEADER_HEIGHT: f32 = 44.0;

/// appeals against the abuse ladder. score clears only via app attest or sms code,
/// else a human reviews and the sheet says so
#[derive(IntoElement, Default)]
pub struct AbuseAppealSheet;

impl AbuseAppealSheet {
    pub fn new() -> Self {
        Self
    }

    fn outcome_view(outcome: &AbuseAppealModelOutcome, theme: &AppTheme) -> impl IntoElement {
        vstack(10.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .p(px(14.0))
            .bg(theme.bg._200)
            .rounded(px(14.0))
            .child(
                MainText::new(outcome.headline.clone())
                    .w_full()
                    .px(16.0)
                    .tac(TextAlignT::Left)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary),
            )
            .when(!outcome.reasons.is_empty(), |stack| {
                stack
                    .child(
                        MainText::new(tr("abuse_appeal_reasons_title"))
                            .w_full()
                            .px(13.0)
                            .tac(TextAlignT::Left)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.secondary),
                    )
                    .children(outcome.reasons.iter().map(|reason| {
                        hstack(8.0)
                            .w_full()
                            .align(VerticalAlignment::Top)
                            .child(MainText::new(REASON_BULLET).px(14.0).color(theme.text.primary))
                            .child(
                                MainText::new(reason.clone())
                                    .flex_1()
                                    .min_w_0()
                                    .px(14.0)
                                    .tac(TextAlignT::Left)
                                    .color(theme.text.primary),
                            )
                    }))
            })
    }

    fn action_button(
        id: &'static str,
        title: String,
        style: BorderedStyle,
        disabled: bool,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div().w_full().child(
            system_button(id, style).tint(theme.primary._100).disabled(disabled).on_press(action).child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .py(px(12.0))
                    .child(MainText::new(title).px(17.0)),
            ),
        )
    }

    fn proof_section(model: &Entity<AbuseAppealModel>, theme: &AppTheme, red: Hsla, cx: &App) -> impl IntoElement {
        let snapshot = model.read(cx);
        let is_busy = snapshot.is_busy;
        let device_supported = snapshot.device_supported();
        let phone_confirmed = snapshot.phone_confirmed;
        let code_sent = snapshot.code_sent;
        let code_too_short = snapshot.code.chars().count() < CODE_MINIMUM_LENGTH;
        let can_send_code = snapshot.can_send_code(cx);

        let verify = model.clone();
        let stack = vstack(16.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(Self::action_button(
                "abuse-appeal-verify-device",
                tr("abuse_appeal_verify_device"),
                BorderedStyle::Prominent,
                is_busy || !device_supported,
                theme,
                move |_, cx| verify.update(cx, |model, cx| model.verify_device(cx)),
            ))
            .when(!device_supported, |stack| {
                stack.child(MainText::new(tr("attestation_unavailable")).w_full().px(14.0).tac(TextAlignT::Left).color(red))
            });
        if phone_confirmed {
            return stack;
        }
        if code_sent {
            let code = Binding::entity(
                model.clone(),
                |model: &AbuseAppealModel| model.code.clone(),
                |model: &mut AbuseAppealModel, value, cx| {
                    model.code = value;
                    cx.notify();
                },
            );
            let confirm = model.clone();
            let cancel = dismiss(cx);
            return stack
                .child(
                    div().w_full().p(px(12.0)).bg(theme.bg._200).rounded(px(12.0)).child(
                        ShapedTextField::new("abuse-appeal-code", tr("abuse_appeal_enter_code"), code)
                            .on_cancel(move |window, cx| cancel.call_as_function(window, cx)),
                    ),
                )
                .child(Self::action_button(
                    "abuse-appeal-submit-code",
                    tr("abuse_appeal_submit_code"),
                    BorderedStyle::Bordered,
                    is_busy || code_too_short,
                    theme,
                    move |_, cx| confirm.update(cx, |model, cx| model.confirm_with_code(cx)),
                ));
        }
        let send = model.clone();
        stack.child(Self::action_button(
            "abuse-appeal-send-code",
            tr("abuse_appeal_send_code"),
            BorderedStyle::Bordered,
            is_busy || !can_send_code,
            theme,
            move |_, cx| send.update(cx, |model, cx| model.send_code(cx)),
        ))
    }
}

impl RenderOnce for AbuseAppealSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let red = UIColor::system_red().resolved_color(theme.appearance);
        let model = window.use_keyed_state("abuse-appeal-model", cx, |_, cx| AbuseAppealModel::new(cx));

        let task_model = model.clone();
        use_task("abuse-appeal-check", (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| task_model.update(cx, |model, cx| model.check(cx)));
        });

        let outcome = model.read(cx).outcome.clone();
        let is_busy = model.read(cx).is_busy;
        let error = model.read(cx).error.clone();
        let is_cleared = outcome.as_ref().is_some_and(|outcome| outcome.cleared);
        let close = dismiss(cx);
        vstack(0.0)
            .w_full()
            .bg(theme.bg._100)
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w_full()
                    .h(px(SHEET_HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        MainText::new(tr("abuse_appeal_title"))
                            .px(17.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .child(
                        div().absolute().right(px(16.0)).top_0().bottom_0().flex().items_center().child(
                            plain("abuse-appeal-close")
                                .on_press(move |window, cx| close.call_as_function(window, cx))
                                .child(MainText::new(tr("close")).px(17.0).color(theme.primary._100)),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .child(
                    vstack(16.0)
                        .relative()
                        .w_full()
                        .align(HorizontalAlignment::Leading)
                        .p(px(20.0))
                        .child(
                            MainText::new(tr("abuse_appeal_subtitle"))
                                .w_full()
                                .px(15.0)
                                .tac(TextAlignT::Left)
                                .color(theme.text.secondary),
                        )
                        .when_some(outcome.as_ref(), |stack, outcome| stack.child(Self::outcome_view(outcome, &theme)))
                        .when(is_busy, |stack| {
                            stack.child(div().w_full().flex().justify_center().py(px(8.0)).child(progress_view()))
                        })
                        .when(!is_cleared, |stack| stack.child(Self::proof_section(&model, &theme, red, cx)))
                        .when_some(error, |stack, error| {
                            stack.child(MainText::new(error).w_full().px(14.0).tac(TextAlignT::Left).color(red))
                        }),
                ),
            )
    }
}

/// sheet of the page that owns the flag, modal host draws surface and backdrop
/// content mounts only while presented so each open runs a fresh check
pub struct AbuseAppealSheetPresentation {
    is_presented: bool,
}

impl AbuseAppealSheetPresentation {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_presented: false }
    }

    pub fn present(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.set_presented(true, cx);
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.set_presented(false, cx);
    }

    fn set_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if self.is_presented == presented {
            return;
        }
        self.is_presented = presented;
        cx.notify();
    }
}

impl Render for AbuseAppealSheetPresentation {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let read = cx.entity();
        let write = cx.entity();
        let is_presented = Binding::new(
            move |cx: &App| read.read(cx).is_presented,
            move |value: bool, _: &mut Window, cx: &mut App| {
                write.update(cx, |presentation, cx| presentation.set_presented(value, cx));
            },
        );
        NativeBottomSheetPresentationHost::new("abuse-appeal-sheet", is_presented, |_, _| AbuseAppealSheet::new())
            .presentation_drag_indicator(false)
            .presentation_background(background)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbuseAppealModelOutcome {
    pub cleared: bool,
    pub escalated: bool,
    pub headline: String,
    pub reasons: Vec<String>,
}

type AbuseAppealModelResponse = abuse_appeal::Reply;

/// asking without proof only reads the state,
/// server clears a borderline score itself when attestation isnt offered
#[derive(Clone)]
struct AbuseAppealModelBody {
    attestation: Option<AppAttestClientEnvelope>,
    code: Option<String>,
}

impl Serialize for AbuseAppealModelBody {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(attestation) = &self.attestation {
            container.serialize_entry(rise_key::attestation::ATTESTATION, attestation)?;
        }
        if let Some(code) = &self.code {
            container.serialize_entry(rise_key::abuse_appeal::CODE, code)?;
        }
        container.end()
    }
}

pub struct AbuseAppealModel {
    pub outcome: Option<AbuseAppealModelOutcome>,
    pub error: Option<String>,
    pub is_busy: bool,
    pub code_sent: bool,
    pub phone_confirmed: bool,
    pub code: String,
    _auth_observation: Subscription,
}

impl AbuseAppealModel {
    fn new(cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        Self {
            outcome: None,
            error: None,
            is_busy: false,
            code_sent: false,
            phone_confirmed: false,
            code: String::new(),
            _auth_observation: cx.observe(&auth, |_, _, cx| cx.notify()),
        }
    }

    pub fn device_supported(&self) -> bool {
        AppAttestClient::is_supported()
    }

    /// server resolves the number by account id, the client only needs one to call send_code
    pub fn can_send_code(&self, cx: &App) -> bool {
        AuthServicesStore::try_read(cx)
            .and_then(|auth| auth.profile())
            .and_then(|profile| profile.more.phone.as_deref())
            .is_some_and(|phone| !phone.is_empty())
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        self.submit(AbuseAppealModelBody { attestation: None, code: None }, false, cx);
    }

    pub fn verify_device(&mut self, cx: &mut Context<Self>) {
        self.is_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let envelope = AppAttestClient::envelope().await;
            let _ = this.update(cx, |model, cx| {
                model.is_busy = false;
                let Some(envelope) = envelope else {
                    model.error = Some(tr("attestation_unavailable"));
                    cx.notify();
                    return;
                };
                model.submit(AbuseAppealModelBody { attestation: Some(envelope), code: None }, false, cx);
            });
        })
        .detach();
    }

    pub fn send_code(&mut self, cx: &mut Context<Self>) {
        let phone = AuthServicesStore::shared(cx).read(cx).profile().and_then(|profile| profile.more.phone.clone());
        let Some(phone) = phone else {
            return;
        };
        let digits: String = phone.chars().filter(|character| character.is_numeric()).collect();
        if digits.is_empty() {
            return;
        }
        self.error = None;
        self.is_busy = true;
        cx.notify();
        let this = cx.weak_entity();
        let on_finish: Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)> =
            Rc::new(move |result: Result<AuthSendCodeResponse, &anyhow::Error>, cx: &mut App| {
                let code_sent = matches!(&result, Ok(response) if response.code_sent());
                let _ = this.update(cx, |model, cx| {
                    if code_sent {
                        model.code_sent = true;
                    } else {
                        model.error = Some(tr("send_code_error"));
                    }
                    model.is_busy = false;
                    cx.notify();
                });
            });
        AuthInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.submit_send_code(&digits, "", None, None, false, Some(on_finish), cx);
        });
    }

    pub fn confirm_with_code(&mut self, cx: &mut Context<Self>) {
        self.submit(AbuseAppealModelBody { attestation: None, code: Some(self.code.clone()) }, true, cx);
    }

    fn submit(&mut self, body: AbuseAppealModelBody, with_code: bool, cx: &mut Context<Self>) {
        self.submit_attempt(body, with_code, false, cx);
    }

    fn submit_attempt(&mut self, body: AbuseAppealModelBody, with_code: bool, renewed: bool, cx: &mut Context<Self>) {
        self.error = None;
        self.is_busy = true;
        cx.notify();
        let retry_body = body.clone();
        let requested = SaiFetch::request_encodable_decoding::<AbuseAppealModelResponse, AbuseAppealModelBody>(
            &HttpService::api_route(rise_http_path::abuse::APPEAL),
            &body,
            None,
            SaiFetchOptions::default(),
            cx,
        );
        let handle = match requested {
            Ok(handle) => handle,
            Err(error) => {
                x440_mark(|| format!("x440 appeal request err: {error:#}"));
                self.submit_failed(cx);
                return;
            }
        };
        let success = cx.weak_entity();
        let failure = cx.weak_entity();
        SaiFetch::handler(
            &handle,
            move |response: AbuseAppealModelResponse, cx: &mut App| {
                let _ = success.update(cx, |model, cx| model.apply_response(&response, with_code, cx));
            },
            move |error: &anyhow::Error, cx: &mut App| {
                x440_mark(|| format!("x440 appeal failed: {error:#}"));
                if !renewed && is_refused_token(error) {
                    let refresh = AuthTokenRefresh::refresh_access_token(None, cx);
                    let retry = failure.clone();
                    cx.spawn(async move |cx| {
                        let refreshed = refresh.await;
                        let _ = retry.update(cx, |model, cx| match refreshed {
                            Ok(()) => model.submit_attempt(retry_body, with_code, true, cx),
                            Err(refresh_error) => {
                                x440_mark(|| format!("x440 refresh failed: {refresh_error:#}"));
                                model.submit_failed(cx);
                            }
                        });
                    })
                    .detach();
                    return;
                }
                let _ = failure.update(cx, |model, cx| model.submit_failed(cx));
            },
            cx,
        );
    }

    fn submit_failed(&mut self, cx: &mut Context<Self>) {
        self.is_busy = false;
        self.error = Some(tr("abuse_appeal_error"));
        cx.notify();
    }

    fn apply_response(&mut self, response: &AbuseAppealModelResponse, with_code: bool, cx: &mut Context<Self>) {
        self.is_busy = false;
        cx.notify();
        let cleared = response.cleared.unwrap_or(false);
        if with_code && !cleared && response.phone_verified != Some(true) {
            self.code = String::new();
            self.error = Some(tr("send_code_invalidcode_error"));
            return;
        }
        let mut seen: HashSet<String> = HashSet::new();
        let reasons: Vec<String> = response
            .reasons
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|reason| {
                let Some(rule) = reason.rule.as_deref().filter(|rule| !rule.is_empty()) else {
                    return reason.meaning.clone();
                };
                let key = format!("abuse_rule_{}", rule.replace('.', "_"));
                let text = tr(&key);
                Some(if text == key { reason.meaning.clone().unwrap_or_else(|| rule.to_owned()) } else { text })
            })
            .filter(|reason| seen.insert(reason.clone()))
            .collect();
        let escalated = response.escalated.unwrap_or(false);
        let headline = if cleared {
            if reasons.is_empty() { tr("abuse_appeal_nothing") } else { tr("abuse_appeal_cleared") }
        } else if with_code {
            self.phone_confirmed = true;
            tr("abuse_appeal_code_accepted")
        } else if escalated {
            tr("abuse_appeal_escalated")
        } else {
            response.message.clone().unwrap_or_else(|| tr("abuse_appeal_subtitle"))
        };
        self.outcome = Some(AbuseAppealModelOutcome { cleared, escalated, headline, reasons });
    }
}

fn is_refused_token(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(cause.downcast_ref::<SaiFetchHttpError>(), Some(SaiFetchHttpError::BadStatus { code: 401, .. }))
    })
}

fn x440_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X440", "{}", stage());
}

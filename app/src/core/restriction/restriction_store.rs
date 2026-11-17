use std::collections::HashMap;

use gpui::{App, Context, Entity};
use rise_catalog::RestrictionScope;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_errors::{ErrorCategory, ErrorCode, ErrorParam};
use rise_i18n::tr;
use rise_ui::keyboard_dismiss::KeyboardDismiss;

use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::canon_failure_relay::RiseServerRefusalRelay;
use crate::core::lib::restriction_until_format::RestrictionUntilFormat;
use crate::core::widgets::notifier::notifier_store::NotifierStore;

/// a refusal the restriction modal explains, a moderation one or a feature with no room left
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestrictionRefusal {
    Restriction { scope: String, until_ms: i64 },
    Rejection { violations_left: Option<i64> },
    Overload { text: Option<String> },
}

impl RestrictionRefusal {
    /// restricted_scope detail alone makes a restriction whatever the code
    /// so a new backend restriction reaches users without an app release
    pub fn new(failure: &RiseFailure) -> Option<Self> {
        if let Some(scope) = failure.details.get(ErrorParam::RestrictedScope.wire()) {
            let until = failure
                .details
                .get(ErrorParam::RestrictedUntilMs.wire())
                .and_then(|value| int_from_string(value))
                .unwrap_or(0);
            return Some(Self::Restriction { scope: scope.clone(), until_ms: until });
        }
        let entry = CanonErrors::entry(failure.code.as_deref());
        if entry.map(|entry| entry.category) == Some(ErrorCategory::Overload) {
            return Some(Self::Overload { text: CanonErrors::text_for(failure) });
        }
        if entry.map(|entry| entry.code) != Some(ErrorCode::ModerationContentRejected) {
            return None;
        }
        let left = failure
            .details
            .get(ErrorParam::ViolationsLeft.wire())
            .and_then(|value| int_from_string(value));
        Some(Self::Rejection { violations_left: left })
    }
}

/// what the restriction modal draws, built when the refusal lands so the modal only draws
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestrictionPrompt {
    pub id: Uuid,
    pub animation: String,
    pub title: String,
    pub subtitle: String,
}

impl RestrictionPrompt {
    pub fn new(refusal: &RestrictionRefusal, now: &Date) -> Self {
        let id = Uuid::new_v4();
        match refusal {
            RestrictionRefusal::Restriction { scope, until_ms } => {
                let scope_key = RestrictionScope::from_wire(scope)
                    .map_or("restriction_scope_generic", RestrictionScope::i18n_key);
                Self {
                    id,
                    animation: "no".to_owned(),
                    title: tr("restriction_modal_title"),
                    subtitle: [tr(scope_key), RestrictionUntilFormat::text(*until_ms, now)].join("\n"),
                }
            }
            RestrictionRefusal::Rejection { violations_left } => {
                let mut lines = vec![tr("moderation_error_content_rejected")];
                if let Some(violations_left) = violations_left {
                    lines.push(if *violations_left > 0 {
                        tr("moderation_rejected_violations_left")
                            .replace("{violations_left}", &violations_left.to_string())
                    } else {
                        tr("moderation_rejected_next_mutes")
                    });
                }
                Self {
                    id,
                    animation: "sorry".to_owned(),
                    title: tr("moderation_rejected_title"),
                    subtitle: lines.join("\n"),
                }
            }
            RestrictionRefusal::Overload { text } => Self {
                id,
                animation: "work".to_owned(),
                title: tr("overload_modal_title"),
                subtitle: text.clone().unwrap_or_else(|| tr("error_gw_capacity_exhausted")),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Signature {
    code: Option<String>,
    details: HashMap<String, String>,
}

/// the only road from a moderation or an overload refusal to the restriction modal
pub struct RestrictionStore {
    prompt: Option<RestrictionPrompt>,
    last_signature: Option<Signature>,
    last_presented_at: Date,
}

impl RestrictionStore {
    const REPEAT_WINDOW_SECONDS: f64 = 2.0;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { prompt: None, last_signature: None, last_presented_at: Date::distant_past() }
    }

    pub fn prompt(&self) -> Option<&RestrictionPrompt> {
        self.prompt.as_ref()
    }

    pub fn is_presented(&self) -> bool {
        self.prompt.is_some()
    }

    pub fn install(cx: &mut App) {
        RiseServerRefusalRelay::subscribe(
            |failure, cx| {
                let now = Date::now();
                RestrictionStore::shared(cx).update(cx, |store, cx| store.present(failure, &now, cx));
            },
            cx,
        );
    }

    /// for a reporter about to toast a failure, true when the modal says it instead
    /// the relay may land after the reporter so the modal is raised here too
    pub fn explains(&mut self, failure: Option<&RiseFailure>, cx: &mut Context<Self>) -> bool {
        let Some(failure) = failure else {
            return false;
        };
        if RestrictionRefusal::new(failure).is_none() {
            return false;
        }
        self.present(failure, &Date::now(), cx);
        true
    }

    pub fn explains_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) -> bool {
        let failure = CanonErrors::failure(error);
        self.explains(failure.as_ref(), cx)
    }

    pub fn present(&mut self, failure: &RiseFailure, now: &Date, cx: &mut Context<Self>) {
        let Some(refusal) = RestrictionRefusal::new(failure) else {
            return;
        };
        let signature = Signature { code: failure.code.clone(), details: failure.details.clone() };
        if self.last_signature.as_ref() == Some(&signature)
            && now.time_interval_since(&self.last_presented_at) < Self::REPEAT_WINDOW_SECONDS
        {
            return;
        }
        self.last_signature = Some(signature);
        self.last_presented_at = *now;
        self.prompt = Some(RestrictionPrompt::new(&refusal, now));
        cx.notify();
        KeyboardDismiss::perform(cx);
        NotifierStore::shared(cx).update(cx, |notifier, cx| notifier.dismiss_errors(cx));
        let code = failure.code.clone().unwrap_or_default();
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("restriction modal: {code}"));
    }

    /// id keeps a fade out of one prompt from closing the next one that replaced it
    pub fn dismiss(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.as_ref() else {
            return;
        };
        if id.is_some_and(|id| prompt.id != id) {
            return;
        }
        self.prompt = None;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.prompt = None;
        self.last_signature = None;
        self.last_presented_at = Date::distant_past();
        cx.notify();
    }
}

impl RiseFailure {
    /// true if it carries restricted_scope, the relay passes it even under a code this build lacks
    pub fn is_restriction(&self) -> bool {
        self.details.contains_key(ErrorParam::RestrictedScope.wire())
    }
}

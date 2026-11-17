use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_canon::RiseLimits;
use rise_canon::rise_quotas::{
    auth_service, event_collector_service, file_service, message_service, ml2_service, music_service, notify_service,
    post_service, stickers_service, stories_service, subscription_service,
};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_errors::{ErrorCode, ErrorParam};
use rise_i18n::{FormatArg, Locale, format_printf, tr};
use rise_validate::CollectionLimit;

use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::canon_failure_relay::RiseServerRefusalRelay;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::premium_auth_hooks::PremiumAuthHooks;

#[derive(Clone, Debug, PartialEq)]
pub struct PremiumUpsellPrompt {
    pub id: Uuid,
    pub reason: String,
    pub is_download_limit: bool,
    pub can_upgrade: bool,
    pub recovery_text: Option<String>,
    pub is_feature: bool,
}

impl PremiumUpsellPrompt {
    pub fn new(id: Uuid, reason: String) -> Self {
        Self {
            id,
            reason,
            is_download_limit: false,
            can_upgrade: true,
            recovery_text: None,
            is_feature: false,
        }
    }
}

static REPEAT_COOLDOWN_SECONDS: LazyLock<f64> =
    LazyLock::new(|| RiseLimits::client_value("premium_upsell_repeat_cooldown_ms") as f64 / 1_000.0);

const AUTOMATIC_QUOTAS: [&str; 19] = [
    notify_service::NOTIFY_VOIP_TOKEN.wire_name,
    notify_service::NOTIFY_DEVICE_TOKEN.wire_name,
    auth_service::AUTH_DEVICE_TOKEN.wire_name,
    subscription_service::SUBSCRIPTION_RECEIPT_VERIFY.wire_name,
    message_service::MESSAGE_TYPING.wire_name,
    message_service::MESSAGE_PLAYED.wire_name,
    post_service::POST_VIEW_RECEIPT.wire_name,
    post_service::POST_SHARE_RECEIPT.wire_name,
    post_service::POST_LINK_OPEN_RECEIPT.wire_name,
    post_service::POST_FEED_SESSION.wire_name,
    stories_service::STORY_VIEW.wire_name,
    music_service::MUSIC_PLAY_RECORD.wire_name,
    stickers_service::STICKERS_RECENT_WRITE.wire_name,
    event_collector_service::EVENT_TRACK.wire_name,
    ml2_service::ML2_EVENT_INGEST.wire_name,
    ml2_service::ML2_FEED_COMPUTE.wire_name,
    notify_service::NOTIFY_RELEASE_ACK.wire_name,
    notify_service::NOTIFY_GUIDE_ACK.wire_name,
    notify_service::NOTIFY_ANNOUNCEMENT_ACK.wire_name,
];

/// only road from a quota refusal to the paywall
/// offers only if server put upgrade_tier in the details
/// client side ceilings like pinned chats use it too
pub struct PremiumUpsellStore {
    prompt: Option<PremiumUpsellPrompt>,
    paywall_request_count: u64,
    suppressed_until: Date,
}

impl PremiumUpsellStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            prompt: None,
            paywall_request_count: 0,
            suppressed_until: Date::distant_past(),
        }
    }

    pub fn prompt(&self) -> Option<&PremiumUpsellPrompt> {
        self.prompt.as_ref()
    }

    pub fn paywall_request_count(&self) -> u64 {
        self.paywall_request_count
    }

    pub fn install(cx: &mut App) {
        RiseServerRefusalRelay::subscribe(
            |failure, cx| {
                let failure = failure.clone();
                Self::shared(cx).update(cx, |store, cx| store.offer(&failure, cx));
            },
            cx,
        );
    }

    pub fn offer(&mut self, failure: &RiseFailure, cx: &mut Context<Self>) {
        let quota = failure.details.get(ErrorParam::Quota.wire());
        if let Some(quota) = quota
            && AUTOMATIC_QUOTAS.contains(&quota.as_str())
        {
            return;
        }
        if failure.code.as_deref() == Some(ErrorCode::CommonQuotaExceeded.wire())
            && quota.map(String::as_str) == Some(file_service::FILE_EGRESS_BYTES.wire_name)
        {
            if self.prompt.is_some() || RiseClock::now() < self.suppressed_until {
                return;
            }
            let recovery = CanonErrors::retry_after_seconds(&failure.details).and_then(|seconds| {
                if seconds <= 0 {
                    return None;
                }
                let date = RiseClock::now().adding_time_interval(seconds as f64);
                let label = RiseFormat::date_time_label(
                    &date,
                    &RiseClock::now(),
                    &Calendar::current(),
                    Locale::app_locale().identifier(),
                );
                Some(format_printf(&tr("download_limit_retry"), &[FormatArg::Text(label.into())]))
            });
            self.prompt = Some(PremiumUpsellPrompt {
                is_download_limit: true,
                can_upgrade: failure.subscription_lifts_limit(),
                recovery_text: recovery,
                ..PremiumUpsellPrompt::new(Uuid::new_v4(), tr("download_limit_message"))
            });
            cx.notify();
            return;
        }
        if !failure.subscription_lifts_limit() {
            return;
        }
        let is_feature = failure.code.as_deref() == Some(ErrorCode::CommonPremiumRequired.wire());
        let reason = if is_feature {
            String::new()
        } else {
            CanonErrors::text_for(failure).unwrap_or_else(|| tr("premium_upsell_reason_generic"))
        };
        let source = quota.cloned().or_else(|| failure.code.clone()).unwrap_or_default();
        self.show(reason, &source, is_feature, cx);
    }

    /// for a reporter about to toast a refusal, the relay may land after it so the offer is made here first
    /// true when the sheet is up and already says it
    pub fn offer_covers(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) -> bool {
        let Some(failure) = CanonErrors::failure(error) else {
            return false;
        };
        if !failure.subscription_lifts_limit() {
            return false;
        }
        self.offer(&failure, cx);
        self.prompt.is_some()
    }

    pub fn offer_client_limit(
        &mut self,
        limit: &CollectionLimit,
        reason: String,
        is_premium: bool,
        cx: &mut Context<Self>,
    ) {
        self.offer_client_ceiling(
            limit.name,
            limit.maximum as i64,
            limit.premium_maximum.map(|maximum| maximum as i64),
            reason,
            is_premium,
            cx,
        );
    }

    /// same for a ceiling canon declares outside the validation tables (post media count)
    pub fn offer_client_ceiling(
        &mut self,
        source: &str,
        maximum: i64,
        premium_maximum: Option<i64>,
        reason: String,
        is_premium: bool,
        cx: &mut Context<Self>,
    ) {
        if is_premium {
            return;
        }
        let Some(premium_maximum) = premium_maximum else {
            return;
        };
        if premium_maximum <= maximum {
            return;
        }
        self.show(reason, source, false, cx);
    }

    /// content canon gives to premium only, like premium sticker pack or name color
    /// server would say upgrade_tier anyway but the client knows and skips the doomed request
    pub fn offer_premium_feature(&mut self, reason: String, source: &str, is_premium: bool, cx: &mut Context<Self>) {
        if is_premium {
            return;
        }
        self.show(reason, source, true, cx);
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.prompt.is_none() {
            return;
        }
        self.prompt = None;
        self.suppressed_until = RiseClock::now().adding_time_interval(*REPEAT_COOLDOWN_SECONDS);
        cx.notify();
    }

    pub fn proceed_to_paywall(&mut self, cx: &mut Context<Self>) {
        self.dismiss(cx);
        self.paywall_request_count = self.paywall_request_count.wrapping_add(1);
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.prompt = None;
        self.suppressed_until = Date::distant_past();
        cx.notify();
    }

    /// a feature offer answers a tap so the quota cooldown doesnt mute it
    fn show(&mut self, reason: String, source: &str, is_feature: bool, cx: &mut Context<Self>) {
        if self.prompt.is_some() || !(is_feature || RiseClock::now() >= self.suppressed_until) {
            return;
        }
        self.prompt = Some(PremiumUpsellPrompt {
            is_feature,
            ..PremiumUpsellPrompt::new(Uuid::new_v4(), reason)
        });
        let source = source.to_owned();
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, move || {
            format!("premium upsell offered: {source}")
        });
        cx.notify();
    }
}

pub struct PremiumUpsellInteractionsStore;

impl PremiumUpsellInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self)
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        PremiumUpsellStore::shared(cx).update(cx, |store, cx| store.dismiss(cx));
    }

    pub fn proceed_to_paywall(&mut self, cx: &mut Context<Self>) {
        PremiumUpsellStore::shared(cx).update(cx, |store, cx| store.proceed_to_paywall(cx));
    }

    pub fn offer_client_ceiling(
        &mut self,
        source: &str,
        maximum: i64,
        premium_maximum: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let is_premium = PremiumAuthHooks::profile(cx).is_some_and(|profile| profile.more.is_premium);
        PremiumUpsellStore::shared(cx).update(cx, |store, cx| {
            store.offer_client_ceiling(
                source,
                maximum,
                premium_maximum,
                tr("premium_upsell_reason_generic"),
                is_premium,
                cx,
            );
        });
    }
}

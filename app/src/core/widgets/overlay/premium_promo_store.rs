use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_catalog::{PremiumPromoGrantKind, PremiumPromoRefusal};
use rise_canon::rise_event_payload::SubscriptionChangedData;
use rise_i18n::{FormatArg, tr, tr_args, tr_plural_with};
use rise_platform::haptics::{NotificationKind, notification, prepare};
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;

use crate::core::lib::canon_errors::CanonErrors;

use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType, NotifierUndoOptions,
};
use crate::core::widgets::overlay::subscription_success_store::SubscriptionSuccessStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_wire_types::PremiumPromoLinkDTO;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::SubscriptionServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::SubscriptionStatus;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub struct PremiumPromoStore {
    code: Option<String>,
    is_presented: bool,
    preview: Option<PremiumPromoLinkDTO>,
    is_loading: bool,
    is_redeeming: bool,
    error_text: Option<String>,
    celebrates_on_dismiss: bool,
    reconnect_watch: Option<RiseTransportDropWatch>,
    connection_deadline: Option<Task<()>>,
    retried_while_connected: bool,
    progress_delay: Option<Task<()>>,
    owns_loader: bool,
    loads_in_background: bool,
}

impl PremiumPromoStore {
    const PROGRESS_DELAY_MILLISECONDS: u64 = 400;

    const CONNECTION_WAIT_SECONDS: u64 = 20;

    const WIRE_REFUSAL_PREFIX: &'static str = "PREMIUM_PROMO_";
    const DAYS_IN_YEAR: i64 = 365;
    const MONTHS_IN_YEAR: i64 = 12;
    const DAYS_IN_MONTH: i64 = 30;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            code: None,
            is_presented: false,
            preview: None,
            is_loading: false,
            is_redeeming: false,
            error_text: None,
            celebrates_on_dismiss: false,
            reconnect_watch: None,
            connection_deadline: None,
            retried_while_connected: false,
            progress_delay: None,
            owns_loader: false,
            loads_in_background: false,
        }
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn preview(&self) -> Option<&PremiumPromoLinkDTO> {
        self.preview.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn is_redeeming(&self) -> bool {
        self.is_redeeming
    }

    pub fn error_text(&self) -> Option<&str> {
        self.error_text.as_deref()
    }

    /// loads the preview, the sheet opens from handle_preview_success once the link is claimable
    pub fn present(&mut self, code: String, cx: &mut Context<Self>) {
        self.code = Some(code.clone());
        self.preview = None;
        self.error_text = None;
        self.is_redeeming = false;
        self.is_loading = true;
        cx.notify();
        self.stop_connection_wait();
        self.retried_while_connected = false;
        self.begin_progress(code.clone(), cx);
        SubscriptionActionsStore::shared(cx).update(cx, |actions, cx| actions.get_promo_link_action(code, cx));
        RiseTransportStatus::wake_if_offline(cx);
    }

    fn begin_progress(&mut self, code: String, cx: &mut Context<Self>) {
        self.finish_progress(cx);
        self.loads_in_background = false;
        self.progress_delay = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::PROGRESS_DELAY_MILLISECONDS))
                .await;
            let _ = this.update(cx, |store, cx| {
                if !store.is_loading || store.code.as_deref() != Some(code.as_str()) {
                    return;
                }
                store.progress_delay = None;
                store.owns_loader = true;
                let weak = cx.weak_entity();
                BlockingLoaderStore::shared(cx).update(cx, |loader, cx| {
                    loader.show(
                        move |cx| {
                            let _ = weak.update(cx, |store, _| {
                                store.owns_loader = false;
                                store.loads_in_background = true;
                            });
                        },
                        cx,
                    );
                });
            });
        }));
    }

    fn finish_progress(&mut self, cx: &mut Context<Self>) {
        self.progress_delay = None;
        if self.owns_loader {
            self.owns_loader = false;
            BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
        }
    }

    /// claimable link opens its sheet at once unless the person sent the wait to background
    /// then a ready toast offers to open it
    fn open_sheet(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.code.clone().filter(|_| self.loads_in_background) else {
            self.is_presented = true;
            cx.notify();
            return;
        };
        self.loads_in_background = false;
        let weak = cx.weak_entity();
        Notifier::send_undo(
            tr("done"),
            tr("ctx_open"),
            NotifierUndoOptions {
                content: Some(self.title_text()),
                r#type: NotifierType::Success,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System(
                    if self.is_verified_grant() { "checkmark.seal" } else { "sparkle" }.to_owned(),
                ),
                shows_countdown: false,
                ..NotifierUndoOptions::default()
            },
            |_| {},
            move |cx| {
                let _ = weak.update(cx, |store, cx| {
                    if store.code.as_deref() != Some(code.as_str())
                        || !store.preview.as_ref().is_some_and(|preview| preview.redeemable)
                    {
                        return;
                    }
                    store.is_presented = true;
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn wait_for_connection(&mut self, code: String, cx: &mut Context<Self>) {
        if self.connection_deadline.is_none() {
            let deadline_code = code.clone();
            self.connection_deadline = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(Self::CONNECTION_WAIT_SECONDS))
                    .await;
                let _ = this.update(cx, |store, cx| store.give_up_waiting(&deadline_code, cx));
            }));
        }
        self.stop_reconnect_watch();
        let this = cx.weak_entity();
        let watch_code = code.clone();
        self.reconnect_watch = Some(RiseTransportStatus::on_next_connect(
            move |cx| {
                let _ = this.update(cx, |store, cx| {
                    if !store.is_loading || store.code.as_deref() != Some(watch_code.as_str()) {
                        return;
                    }
                    store.reconnect_watch = None;
                    SubscriptionActionsStore::shared(cx)
                        .update(cx, |actions, cx| actions.get_promo_link_action(watch_code.clone(), cx));
                });
            },
            cx,
        ));
        RiseTransportStatus::wake_if_offline(cx);
        if RiseTransportStatus::is_connected(cx) && !self.retried_while_connected {
            self.retried_while_connected = true;
            self.stop_reconnect_watch();
            SubscriptionActionsStore::shared(cx).update(cx, |actions, cx| actions.get_promo_link_action(code, cx));
        }
    }

    fn give_up_waiting(&mut self, code: &str, cx: &mut Context<Self>) {
        self.connection_deadline = None;
        if !self.is_loading || self.code.as_deref() != Some(code) {
            return;
        }
        self.is_loading = false;
        cx.notify();
        self.stop_connection_wait();
        self.finish_progress(cx);
        self.loads_in_background = false;
        Notifier::send(
            tr("error_connection_lost"),
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("wifi.slash".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn stop_reconnect_watch(&mut self) {
        if let Some(watch) = self.reconnect_watch.take() {
            watch.stop();
        }
    }

    fn stop_connection_wait(&mut self) {
        self.stop_reconnect_watch();
        self.connection_deadline = None;
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.is_presented = false;
        cx.notify();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.celebrates_on_dismiss {
            return;
        }
        self.celebrates_on_dismiss = false;
        SubscriptionSuccessStore::shared(cx).update(cx, |success, cx| success.show(cx));
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.celebrates_on_dismiss = false;
        self.stop_connection_wait();
        self.finish_progress(cx);
        self.loads_in_background = false;
        self.code = None;
        self.preview = None;
        self.error_text = None;
        self.is_loading = false;
        self.is_redeeming = false;
        self.is_presented = false;
        cx.notify();
    }

    pub fn redeem(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.code.clone() else {
            return;
        };
        if self.is_redeeming || self.is_loading || !self.preview.as_ref().is_some_and(|preview| preview.redeemable) {
            return;
        }
        self.is_redeeming = true;
        self.error_text = None;
        cx.notify();
        SubscriptionActionsStore::shared(cx).update(cx, |actions, cx| actions.redeem_promo_link_action(code, cx));
    }

    pub fn can_redeem(&self) -> bool {
        !self.is_loading && self.preview.as_ref().is_some_and(|preview| preview.redeemable)
    }

    pub fn is_lifetime(&self) -> bool {
        self.preview.as_ref().is_some_and(|preview| preview.lifetime)
    }

    /// true when the link hands out the verified checkmark instead of premium
    pub fn is_verified_grant(&self) -> bool {
        self.preview.as_ref().is_some_and(|preview| Self::is_verified_kind(&preview.grant_kind))
    }

    fn is_verified_kind(grant_kind: &str) -> bool {
        grant_kind == PremiumPromoGrantKind::Verified.wire()
    }

    pub fn title_text(&self) -> String {
        tr(if self.is_verified_grant() {
            "verified_promo_title"
        } else {
            "premium_promo_title"
        })
    }

    pub fn subtitle_text(&self) -> String {
        tr(if self.is_verified_grant() {
            "verified_promo_subtitle"
        } else {
            "premium_promo_subtitle"
        })
    }

    pub fn cta_text(&self) -> String {
        tr(if self.is_verified_grant() {
            "verified_promo_cta"
        } else {
            "premium_promo_cta"
        })
    }

    /// eg 3 months or 14 days or forever or the checkmark, from the grant the server described
    pub fn period_text(&self) -> String {
        if self.is_verified_grant() {
            return tr("verified_promo_badge");
        }
        Self::period_text_for(
            self.preview.as_ref().map_or(0, |preview| preview.grant_days),
            self.is_lifetime(),
        )
    }

    pub fn period_text_for(grant_days: i64, lifetime: bool) -> String {
        if lifetime || grant_days <= 0 {
            return tr("premium_promo_period_lifetime");
        }
        if grant_days == Self::DAYS_IN_YEAR {
            return tr_plural_with(
                "premium_gift_months",
                Self::MONTHS_IN_YEAR,
                FormatArg::from(Self::MONTHS_IN_YEAR),
            );
        }
        if grant_days % Self::DAYS_IN_MONTH == 0 {
            let months = grant_days / Self::DAYS_IN_MONTH;
            return tr_plural_with("premium_gift_months", months, FormatArg::from(months));
        }
        tr_plural_with("premium_promo_period_days", grant_days, FormatArg::from(grant_days))
    }

    /// toast for a link that cant be claimed, by service vocabulary and by what the link hands out
    pub fn refusal_text(refusal_code: Option<&str>, verified: bool) -> String {
        let refusal = refusal_code.and_then(|code| {
            PremiumPromoRefusal::ALL
                .into_iter()
                .find(|refusal| refusal.wire() == code)
        });
        match refusal {
            Some(PremiumPromoRefusal::Expired) => tr(if verified {
                "promo_link_expired_verified"
            } else {
                "promo_link_expired_premium"
            }),
            Some(PremiumPromoRefusal::Exhausted | PremiumPromoRefusal::Inactive) => tr(if verified {
                "promo_link_exhausted_verified"
            } else {
                "promo_link_exhausted_premium"
            }),
            Some(PremiumPromoRefusal::AlreadyRedeemed) => tr(if verified {
                "verified_promo_already_redeemed"
            } else {
                "premium_promo_already_redeemed"
            }),
            Some(PremiumPromoRefusal::NotFound) | None => tr("premium_promo_invalid"),
        }
    }

    fn refuse(&mut self, refusal_code: Option<&str>, verified: bool, cx: &mut Context<Self>) {
        self.is_presented = false;
        cx.notify();
        Notifier::send(
            Self::refusal_text(refusal_code, verified),
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System(if verified { "checkmark.seal" } else { "sparkle" }.to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn handle_preview_success(&mut self, cx: &mut Context<Self>) {
        self.is_loading = false;
        self.stop_connection_wait();
        self.finish_progress(cx);
        let data = SubscriptionActionsStore::shared(cx).read(cx).get_promo_link_sai.data();
        self.preview = data.clone();
        cx.notify();
        if !data.as_ref().is_some_and(|data| data.redeemable) {
            self.loads_in_background = false;
            let refusal = data
                .as_ref()
                .map(|data| data.refusal_code.clone())
                .filter(|code| !code.is_empty())
                .unwrap_or_else(|| PremiumPromoRefusal::NotFound.wire().to_owned());
            let verified = data.as_ref().is_some_and(|data| Self::is_verified_kind(&data.grant_kind));
            self.refuse(Some(&refusal), verified, cx);
            return;
        }
        self.open_sheet(cx);
    }

    pub fn handle_preview_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let refusal = Self::refusal(error);
        if refusal.is_none()
            && CanonErrors::is_connection_loss(error)
            && let Some(code) = self.code.clone()
        {
            self.wait_for_connection(code, cx);
            return;
        }
        self.is_loading = false;
        self.stop_connection_wait();
        self.finish_progress(cx);
        self.loads_in_background = false;
        self.refuse(refusal.as_deref(), false, cx);
    }

    pub fn handle_redeem_success(&mut self, cx: &mut Context<Self>) {
        self.is_redeeming = false;
        cx.notify();
        let data = SubscriptionActionsStore::shared(cx).read(cx).redeem_promo_link_sai.data();
        let data_kind = data.as_ref().map(|data| data.grant_kind.as_str());
        if data_kind.is_some_and(Self::is_verified_kind) || (data_kind.is_none() && self.is_verified_grant()) {
            self.handle_verified_grant(cx);
            return;
        }
        let period = Self::period_text_for(
            data.as_ref()
                .map(|data| data.grant_days)
                .or_else(|| self.preview.as_ref().map(|preview| preview.grant_days))
                .unwrap_or(0),
            data.as_ref().map_or_else(|| self.is_lifetime(), |data| data.lifetime),
        );
        let sheet_was_open = self.is_presented;
        self.is_presented = false;

        let my_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        if let Some(my_id) = my_id {
            let event = SubscriptionChangedData {
                user_id: my_id,
                is_premium: true,
                status: SubscriptionStatus::Active.raw_value().to_owned(),
                ..SubscriptionChangedData::default()
            };
            if let Ok(payload) = serde_json::to_value(&event) {
                SubscriptionServicesStore::shared(cx).update(cx, |services, cx| {
                    services.subscription_changed_event_handler(Some(&payload), cx);
                });
            }
        }
        NotifierStore::shared(cx).update(cx, |notifier, cx| {
            notifier.send(
                tr_args("premium_promo_success", &[FormatArg::from(period)]),
                NotifierSendOptions {
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        if sheet_was_open {
            self.celebrates_on_dismiss = true;
        } else {
            SubscriptionSuccessStore::shared(cx).update(cx, |success, cx| success.show(cx));
        }
    }

    fn handle_verified_grant(&mut self, cx: &mut Context<Self>) {
        self.is_presented = false;
        cx.notify();
        prepare();
        notification(NotificationKind::Success);

        let profile = AuthServicesStore::shared(cx).read(cx).profile().cloned();
        if let Some(mut profile) = profile {
            profile.more.is_official = true;
            let my_id = profile.id.clone();
            let tag = profile.tag.clone();
            AuthServicesStore::shared(cx).update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
            SaiWsResponseCache::remove(
                &SaiCache::new(["getMyProfile", my_id.as_str()]).key,
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
            SaiWsResponseCache::remove(
                &UserServicesStore::profile_by_id_cache_key(&my_id, &my_id),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
            if !tag.is_empty() {
                SaiWsResponseCache::remove(
                    &UserServicesStore::profile_by_tag_cache_key(&tag),
                    SaiCacheCleanLayer::Everywhere,
                    cx,
                );
            }
        }
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_my_profile(cx));
        Notifier::send(
            tr("verified_promo_success"),
            NotifierSendOptions {
                r#type: NotifierType::Success,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("checkmark.seal.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn handle_redeem_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.is_redeeming = false;
        if let Some(refusal) = Self::refusal(error) {
            let verified = self.is_verified_grant();
            self.refuse(Some(&refusal), verified, cx);
        } else if matches!(error.downcast_ref::<SaiWsSendError>(), Some(SaiWsSendError::NotConnected)) {
            self.error_text = Some(tr("error_connection_lost"));
        } else {
            self.error_text = Some(tr("retry_later_error"));
        }
        cx.notify();
    }

    /// wire code PREMIUM_PROMO_ALREADY_REDEEMED becomes already_redeemed here
    pub fn refusal(error: &anyhow::Error) -> Option<String> {
        let code = error.downcast_ref::<SaiWsResponseError>()?.code.as_deref()?;
        let refusal = code.strip_prefix(Self::WIRE_REFUSAL_PREFIX)?;
        Some(refusal.to_lowercase())
    }
}

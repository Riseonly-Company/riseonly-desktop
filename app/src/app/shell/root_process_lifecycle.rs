use std::rc::Rc;

use gpui::{App, Global, Subscription};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::app::root_view::RootView;
use crate::app::shell::handover::frontmost_shell;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::rise_app_background_ledger::RiseAppBackgroundLedger;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::notify::engine::rise_notify_domain::RiseNotifyDomainRegistry;
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::presence::stores::presence::presence_actions::presence_actions::PresenceActionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::share_extension::send_handoff::share_send_handoff_interactions::ShareSendHandoffInteractionsStore;
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

const PRESENCE_ONLINE: &str = "online";
const PRESENCE_AWAY: &str = "away";

struct RootDeferredCallEndRefresh {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
}

/// process wide lifecycle state, not tied to one window
/// cmd+n opens more root views and each would report every scene phase
/// closing the last window is the trip to background
pub(crate) struct RootProcessLifecycle {
    scene_phase: ScenePhase,
    handled_background_generation: Option<u64>,
    bootstrapped_account_id: Option<String>,
    deferred_call_end_refresh: Option<RootDeferredCallEndRefresh>,
    _scene_phase_observer: Subscription,
}

impl Global for RootProcessLifecycle {}

impl RootProcessLifecycle {
    pub(crate) fn install_if_needed(cx: &mut App) {
        Self::install_with(Rc::new(Self::scene_phase_work), cx);
    }

    fn install_with(work: Rc<dyn Fn(ScenePhase, &mut App)>, cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        let observer = cx.observe_global::<RiseAppLifecycle>(move |cx| {
            let phase = RiseAppLifecycle::scene_phase(cx);
            let this = cx.global_mut::<Self>();
            if phase == this.scene_phase {
                return;
            }
            this.scene_phase = phase;
            work(phase, cx);
        });
        let scene_phase = RiseAppLifecycle::scene_phase(cx);
        cx.set_global(Self {
            scene_phase,
            handled_background_generation: None,
            bootstrapped_account_id: None,
            deferred_call_end_refresh: None,
            _scene_phase_observer: observer,
        });
    }

    pub(crate) fn bootstrapped_account_id(cx: &App) -> Option<&str> {
        cx.try_global::<Self>().and_then(|this| this.bootstrapped_account_id.as_deref())
    }

    pub(crate) fn set_bootstrapped_account_id(account_id: Option<String>, cx: &mut App) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().bootstrapped_account_id = account_id;
        }
    }

    pub(crate) fn clear_deferred_call_end_refresh(cx: &mut App) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().deferred_call_end_refresh = None;
        }
    }

    fn scene_phase_work(phase: ScenePhase, cx: &mut App) {
        if phase == ScenePhase::Active {
            let gate = MaintenanceGate::shared(cx);
            gate.update(cx, |gate, cx| gate.application_did_become_active(cx));
            Self::refresh_deferred_call_end_announcements(cx);
        }
        let auth_services = AuthServicesStore::shared(cx);
        if !auth_services.read(cx).is_authenticated() {
            return;
        }
        match phase {
            ScenePhase::Active => {
                auth_services.update(cx, |auth, cx| {
                    auth.adopt_share_extension_credential_if_needed(cx);
                });
                RiseShareAppBridge::synchronize_credential(false);
                let handoff = ShareSendHandoffInteractionsStore::shared(cx);
                handoff.update(cx, |handoff, cx| handoff.resume_pending_if_possible(cx));
                if let Some(shell) = frontmost_shell(cx).and_then(|shell| shell.downcast::<RootView>()) {
                    let _ = shell.update(cx, |view, _, cx| view.resume_shared_story_after_authentication_if_needed(cx));
                }
                if !Self::consume_return_from_background(cx) {
                    return;
                }
                Self::set_application_active(true, cx);
                Self::set_away_action(PRESENCE_ONLINE, cx);
                let user = UserInteractionsStore::shared(cx);
                user.update(cx, |user, cx| user.application_did_become_active_for_contacts(cx));
            }
            ScenePhase::Background => {
                RiseAppBackgroundLedger::note_entered_background(Date::now(), cx);
                Notifier::commit_pending_undo(cx);
                let messages = MessageInteractionsStore::shared(cx);
                messages.update(cx, |messages, cx| messages.checkpoint_pending_chat_caches(cx));
                Self::set_away_action(PRESENCE_AWAY, cx);
                Self::set_application_active(false, cx);
            }
            ScenePhase::Inactive => {
                if !Self::consume_return_from_background(cx) {
                    return;
                }
                tracing::warn!(target: "riseonly::X1525", "visible again without focus, transport live");
                Self::set_application_active(true, cx);
                Self::set_away_action(PRESENCE_ONLINE, cx);
                let user = UserInteractionsStore::shared(cx);
                user.update(cx, |user, cx| user.application_did_become_active_for_contacts(cx));
            }
        }
    }

    fn consume_return_from_background(cx: &mut App) -> bool {
        let mut handled = cx.global::<Self>().handled_background_generation;
        let returned = RiseAppBackgroundLedger::consume_return_from_background(&mut handled, cx);
        cx.global_mut::<Self>().handled_background_generation = handled;
        returned
    }

    fn set_application_active(active: bool, cx: &mut App) {
        let auth_services = AuthServicesStore::shared(cx);
        let access_token = auth_services.read(cx).tokens_and_other_data().access_token.clone();
        SaiWs::set_application_active(active, Some(access_token), cx);
    }

    fn set_away_action(status: &str, cx: &mut App) {
        let presence = PresenceActionsStore::shared(cx);
        presence.update(cx, |presence, cx| presence.set_away_action(status.to_owned(), cx));
    }

    pub(crate) fn note_call_activity_change(was_active: bool, is_active: bool, cx: &mut App) {
        if !was_active || is_active || !cx.has_global::<Self>() {
            return;
        }
        let registry = RiseNotifyDomainRegistry::shared(cx);
        let auth_services = AuthServicesStore::shared(cx);
        let auth = auth_services.read(cx);
        let pending = (|| {
            if !auth.is_authenticated() {
                return None;
            }
            let account_id = auth.active_account_id()?.to_owned();
            let account_epoch = SaiAccountCacheScope::active_epoch()?;
            let domain = registry.read(cx).active()?;
            if domain.account_id != account_id || domain.account_epoch != account_epoch {
                return None;
            }
            Some(RootDeferredCallEndRefresh { account_id, account_epoch, session_id: domain.session_id })
        })();
        let has_pending = pending.is_some();
        cx.global_mut::<Self>().deferred_call_end_refresh = pending;
        if has_pending {
            Self::refresh_deferred_call_end_announcements(cx);
        }
    }

    fn refresh_deferred_call_end_announcements(cx: &mut App) {
        if RiseAppLifecycle::scene_phase(cx) != ScenePhase::Active {
            return;
        }
        let Some(pending) = cx.global_mut::<Self>().deferred_call_end_refresh.take() else {
            return;
        };
        let auth_services = AuthServicesStore::shared(cx);
        let auth = auth_services.read(cx);
        if !auth.is_authenticated() || auth.active_account_id() != Some(pending.account_id.as_str()) {
            return;
        }
        if !SaiAccountCacheScope::is_current_account_epoch(Some(&pending.account_id), Some(&pending.account_epoch)) {
            return;
        }
        let registry = RiseNotifyDomainRegistry::shared(cx);
        if !registry.read(cx).matches(&pending.account_id, &pending.account_epoch, pending.session_id) {
            return;
        }
        let rtc_services = RTCServicesStore::shared(cx);
        if rtc_services.read(cx).active_room(cx).is_some() {
            return;
        }
        let notify = NotifyInteractionsStore::shared(cx);
        notify.update(cx, |notify, cx| notify.call_activity_did_change(true, false, cx));
    }
}

#[cfg(test)]
#[path = "root_process_lifecycle_tests.rs"]
mod tests;

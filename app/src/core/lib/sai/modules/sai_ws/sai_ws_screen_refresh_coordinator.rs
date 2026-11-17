use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Global, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, UIApplication};
use crate::core::lib::rise_app_background_ledger::RiseAppBackgroundLedger;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{
    SAI_WS_DID_CONNECT, SAI_WS_DID_DISCONNECT, SaiWebSocketManager,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{
    SaiNavigationBridge, SaiWsScreenRefresh,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_debug::SaiWsScreenRefreshDebug;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_route_matcher::SaiWsScreenRouteMatcher;
use crate::core::lib::sai::sai::Sai;

const MAXIMUM_REGISTRY_ENTRIES: usize = 128;
const OFFLINE_RETRY_DELAY: Duration = Duration::from_millis(400);
const TARGETED_REVALIDATION_DEBOUNCE: Duration = Duration::from_millis(80);

pub type SaiWsScreenRefreshResend = Box<dyn Fn(&mut App)>;

struct RegisteredEntry {
    cache_key: String,
    refresh: SaiWsScreenRefresh,
    resend: Rc<dyn Fn(&mut App)>,
    sequence: u64,
}

#[derive(Clone, Copy)]
enum LifecycleSignal {
    DidBecomeActive,
    DidDisconnect,
    DidConnect,
}

#[derive(Default)]
struct SaiWsScreenRefreshCoordinatorState {
    revalidated_cache_keys: HashSet<String>,
    revalidated_epoch_by_key: HashMap<String, String>,
    registry: HashMap<String, RegisteredEntry>,
    lifecycle_installed: bool,
    has_connected_transport: bool,
    reconnect_catch_up_pending: bool,
    next_registry_sequence: u64,
    targeted_revalidation_task: Option<Task<()>>,
    targeted_revalidation_keys: HashSet<String>,
    handled_background_generation: Option<u64>,
}

impl Global for SaiWsScreenRefreshCoordinatorState {}

pub enum SaiWsScreenRefreshCoordinator {}

impl SaiWsScreenRefreshCoordinator {
    fn state(cx: &mut App) -> &mut SaiWsScreenRefreshCoordinatorState {
        cx.default_global::<SaiWsScreenRefreshCoordinatorState>()
    }

    pub fn install_lifecycle_observers_if_needed(cx: &mut App) {
        let is_connected = SaiWebSocketManager::shared(cx).read(cx).is_connected();
        let state = Self::state(cx);
        if is_connected {
            state.has_connected_transport = true;
        }
        if state.lifecycle_installed {
            return;
        }
        state.lifecycle_installed = true;

        let (signals, mut inbox) = mpsc::unbounded::<LifecycleSignal>();
        let center = RiseAppEvents::default_center();
        for (name, signal) in [
            (UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, LifecycleSignal::DidBecomeActive),
            (SAI_WS_DID_DISCONNECT, LifecycleSignal::DidDisconnect),
            (SAI_WS_DID_CONNECT, LifecycleSignal::DidConnect),
        ] {
            let signals = signals.clone();
            center
                .observe(name, move |_| {
                    let _ = signals.unbounded_send(signal);
                })
                .detach();
        }
        cx.spawn(async move |cx| {
            while let Some(signal) = inbox.next().await {
                match signal {
                    LifecycleSignal::DidBecomeActive => cx.update(Self::on_app_became_active),
                    LifecycleSignal::DidDisconnect => cx.update(Self::on_transport_disconnected),
                    LifecycleSignal::DidConnect => {
                        let catch_up = cx.update(Self::on_transport_connected);
                        catch_up.await;
                    }
                }
            }
        })
        .detach();
    }

    pub fn on_app_became_active(cx: &mut App) {
        let mut handled = Self::state(cx).handled_background_generation;
        let returned = RiseAppBackgroundLedger::consume_return_from_background(&mut handled, cx);
        Self::state(cx).handled_background_generation = handled;
        if !returned {
            return;
        }
        let registry_count = Self::state(cx).registry.len();
        SaiWsScreenRefreshDebug::log(
            move || format!("onAppBecameActive — cleared revalidated keys, registry={registry_count}"),
            cx,
        );
        Self::reset_revalidation_cycle(cx);
        Self::resend_registered_after_foreground(cx);
    }

    pub fn on_transport_disconnected(cx: &mut App) {
        let state = Self::state(cx);
        if !state.has_connected_transport {
            return;
        }
        state.has_connected_transport = false;
        state.reconnect_catch_up_pending = true;
    }

    pub fn on_transport_connected(cx: &mut App) -> Task<()> {
        let state = Self::state(cx);
        let should_catch_up = state.reconnect_catch_up_pending;
        state.reconnect_catch_up_pending = false;
        state.has_connected_transport = true;
        if !should_catch_up {
            return Task::ready(());
        }
        Self::reset_revalidation_cycle(cx);
        Self::resend_matching_active_screen("reconnect", false, cx)
    }

    pub fn should_background_revalidate(
        cache_key: &str,
        refresh: &SaiWsScreenRefresh,
        cx: &mut App,
    ) -> bool {
        if !refresh.enabled {
            let key = cache_key.to_owned();
            SaiWsScreenRefreshDebug::log(move || format!("shouldRevalidate({key}) → false (disabled)"), cx);
            return false;
        }
        if !Self::matches_active_screen(refresh, cx) {
            let key = cache_key.to_owned();
            SaiWsScreenRefreshDebug::log(move || format!("shouldRevalidate({key}) → false (screen)"), cx);
            return false;
        }
        let key = cache_key.to_owned();
        if Self::state(cx).revalidated_cache_keys.contains(cache_key) {
            SaiWsScreenRefreshDebug::log(
                move || format!("shouldRevalidate({key}) → false (already revalidated this cycle)"),
                cx,
            );
            return false;
        }
        SaiWsScreenRefreshDebug::log(move || format!("shouldRevalidate({key}) → true"), cx);
        true
    }

    pub fn mark_revalidated(cache_key: &str, cx: &mut App) {
        let epoch = SaiAccountCacheScope::active_epoch().unwrap_or_default();
        let state = Self::state(cx);
        state.revalidated_cache_keys.insert(cache_key.to_owned());
        state.revalidated_epoch_by_key.insert(cache_key.to_owned(), epoch);
        let count = state.revalidated_cache_keys.len();
        let key = cache_key.to_owned();
        SaiWsScreenRefreshDebug::log(move || format!("markRevalidated({key}) keys={count}"), cx);
    }

    pub fn needs_epoch_revalidation(cache_key: &str, cx: &mut App) -> bool {
        let epoch = SaiAccountCacheScope::active_epoch().unwrap_or_default();
        Self::state(cx).revalidated_epoch_by_key.get(cache_key) != Some(&epoch)
    }

    pub fn register(
        cache_key: &str,
        refresh: impl Borrow<SaiWsScreenRefresh>,
        resend: SaiWsScreenRefreshResend,
        cx: &mut App,
    ) {
        let refresh: &SaiWsScreenRefresh = refresh.borrow();
        let ck = trimming_whitespaces_and_newlines(cache_key).to_owned();
        if ck.is_empty() || !refresh.enabled {
            return;
        }
        let state = Self::state(cx);
        state.next_registry_sequence = state.next_registry_sequence.wrapping_add(1);
        if !state.registry.contains_key(&ck)
            && state.registry.len() >= MAXIMUM_REGISTRY_ENTRIES
            && let Some(oldest) = state
                .registry
                .iter()
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(key, _)| key.clone())
        {
            state.registry.remove(&oldest);
        }
        let sequence = state.next_registry_sequence;
        state.registry.insert(
            ck.clone(),
            RegisteredEntry {
                cache_key: ck.clone(),
                refresh: refresh.clone(),
                resend: Rc::from(resend),
                sequence,
            },
        );
        let count = state.registry.len();
        SaiWsScreenRefreshDebug::log(move || format!("register({ck}) registry={count}"), cx);
    }

    pub fn unregister(cache_key: &str, cx: &mut App) {
        let ck = trimming_whitespaces_and_newlines(cache_key);
        Self::state(cx).registry.remove(ck);
    }

    fn resend_registered_after_foreground(cx: &mut App) {
        if Self::state(cx).registry.is_empty() {
            SaiWsScreenRefreshDebug::log(
                || {
                    "foreground resend: registry empty — no prior screenRefresh requests registered"
                        .to_owned()
                },
                cx,
            );
            return;
        }
        Self::resend_matching_active_screen("foreground", true, cx).detach();
    }

    pub fn resend_matching_active_screen(
        trigger: &'static str,
        allow_retry_when_offline: bool,
        cx: &mut App,
    ) -> Task<()> {
        let Some(screen_id) = Self::active_screen_label(cx) else {
            SaiWsScreenRefreshDebug::log(
                move || format!("{trigger} resend: no active screen (bridge nil?)"),
                cx,
            );
            return Task::ready(());
        };
        let sent = Self::resend_matching_active_screen_once(cx);
        if sent > 0 {
            Sai::log(
                "WS",
                move || format!("screen refresh [{trigger}] screen={screen_id} resent {sent} request(s)"),
                SaiLogKind::Cache,
                cx,
            );
            return Task::ready(());
        }
        let registry_count = Self::state(cx).registry.len();
        let logged_screen = screen_id.clone();
        SaiWsScreenRefreshDebug::log(
            move || {
                format!("{trigger} resend: 0 sent screen={logged_screen} registry={registry_count}")
            },
            cx,
        );
        if !allow_retry_when_offline || SaiWebSocketManager::shared(cx).read(cx).is_connected() {
            return Task::ready(());
        }
        SaiWsScreenRefreshDebug::log(move || format!("{trigger} resend: offline — retry in 400ms"), cx);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(OFFLINE_RETRY_DELAY).await;
            cx.update(|cx| {
                let retry_sent = Self::resend_matching_active_screen_once(cx);
                if retry_sent > 0 {
                    Sai::log(
                        "WS",
                        move || {
                            format!(
                                "screen refresh [{trigger}] retry screen={screen_id} resent {retry_sent} request(s)"
                            )
                        },
                        SaiLogKind::Cache,
                        cx,
                    );
                }
            });
        })
    }

    fn resend_matching_active_screen_once(cx: &mut App) -> usize {
        let entries: Vec<(String, SaiWsScreenRefresh, Rc<dyn Fn(&mut App)>)> = Self::state(cx)
            .registry
            .values()
            .map(|entry| (entry.cache_key.clone(), entry.refresh.clone(), entry.resend.clone()))
            .collect();
        let mut sent = 0;
        for (ck, refresh, resend) in entries {
            if !Self::matches_active_screen(&refresh, cx) {
                continue;
            }
            if Self::state(cx).revalidated_cache_keys.contains(&ck) {
                SaiWsScreenRefreshDebug::log(
                    move || format!("resend skip {ck} (already revalidated this cycle)"),
                    cx,
                );
                continue;
            }
            SaiWsScreenRefreshDebug::log(move || format!("resend → {ck}"), cx);
            resend(cx);
            sent += 1;
        }
        sent
    }

    pub fn reset_revalidation_cycle(cx: &mut App) {
        Self::state(cx).revalidated_cache_keys.clear();
    }

    pub fn reset_account_state(cx: &mut App) {
        let state = Self::state(cx);
        state.targeted_revalidation_task = None;
        state.targeted_revalidation_keys = HashSet::new();
        state.revalidated_cache_keys.clear();
        state.revalidated_epoch_by_key.clear();
        state.registry.clear();
        state.next_registry_sequence = 0;
        state.has_connected_transport = false;
        state.reconnect_catch_up_pending = false;
    }

    pub fn schedule_targeted_revalidation(
        cache_keys: &HashSet<String>,
        account_id: &str,
        account_epoch: &str,
        cx: &mut App,
    ) {
        let keys: HashSet<String> = cache_keys
            .iter()
            .map(|raw| trimming_whitespaces_and_newlines(raw))
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect();
        if keys.is_empty()
            || !SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch))
        {
            return;
        }
        for key in &keys {
            Self::forget_revalidated(key, cx);
        }
        Self::state(cx).targeted_revalidation_keys.extend(keys);
        let account_id = account_id.to_owned();
        let account_epoch = account_epoch.to_owned();
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(TARGETED_REVALIDATION_DEBOUNCE).await;
            cx.update(|cx| {
                if !SaiAccountCacheScope::is_current_account_epoch(
                    Some(&account_id),
                    Some(&account_epoch),
                ) {
                    return;
                }
                let state = Self::state(cx);
                let pending = std::mem::take(&mut state.targeted_revalidation_keys);
                if let Some(current) = state.targeted_revalidation_task.take() {
                    current.detach();
                }
                Self::resend_registered(&pending, "rise-auxiliary-invalidation", cx);
            });
        });
        Self::state(cx).targeted_revalidation_task = Some(task);
    }

    pub fn forget_revalidated(cache_key: &str, cx: &mut App) {
        let ck = trimming_whitespaces_and_newlines(cache_key);
        if ck.is_empty() {
            return;
        }
        let state = Self::state(cx);
        state.revalidated_cache_keys.remove(ck);
        state.revalidated_epoch_by_key.remove(ck);
    }

    fn resend_registered(cache_keys: &HashSet<String>, trigger: &'static str, cx: &mut App) {
        if cache_keys.is_empty() {
            return;
        }
        let Some(screen_id) = Self::active_screen_label(cx) else {
            return;
        };
        let mut sorted: Vec<&String> = cache_keys.iter().collect();
        sorted.sort();
        let mut sent = 0;
        for cache_key in sorted {
            let entry = Self::state(cx)
                .registry
                .get(cache_key)
                .map(|entry| (entry.refresh.clone(), entry.resend.clone()));
            let Some((refresh, resend)) = entry else {
                continue;
            };
            if !Self::matches_active_screen(&refresh, cx) {
                continue;
            }
            resend(cx);
            sent += 1;
        }
        if sent == 0 {
            return;
        }
        Sai::log(
            "WS",
            move || format!("screen refresh [{trigger}] screen={screen_id} resent {sent} request(s)"),
            SaiLogKind::Cache,
            cx,
        );
    }

    /// label for the log, desktop joins every visible lease
    fn active_screen_label(cx: &mut App) -> Option<String> {
        let screens = SaiNavigationBridge::visible_screens(cx);
        if screens.is_empty() {
            return None;
        }
        Some(screens.iter().map(|screen| screen.id.as_str()).collect::<Vec<_>>().join(","))
    }

    pub fn matches_active_screen(refresh: &SaiWsScreenRefresh, cx: &mut App) -> bool {
        if !refresh.enabled {
            return false;
        }
        let screens = SaiNavigationBridge::visible_screens(cx);
        if screens.is_empty() {
            SaiWsScreenRefreshDebug::log(
                || {
                    "matchesActiveScreen → false (SaiNavigationBridge nil — RootView.onAppear install?)"
                        .to_owned()
                },
                cx,
            );
            return false;
        }
        let ok = screens.iter().any(|current| {
            SaiWsScreenRouteMatcher::matches(
                &refresh.when_on_screen,
                refresh.route_params.as_ref(),
                current,
            )
        });
        if !ok && SaiWsScreenRefreshDebug::enabled() {
            let described: Vec<String> = screens
                .iter()
                .map(|screen| format!("id={} params={:?}", screen.id, screen.params))
                .collect();
            SaiWsScreenRefreshDebug::log(
                move || format!("matchesActiveScreen → false {}", described.join("; ")),
                cx,
            );
        }
        ok
    }
}

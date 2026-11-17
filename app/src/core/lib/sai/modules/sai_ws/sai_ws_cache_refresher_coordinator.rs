use std::collections::HashSet;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::lib::rise_app_background_ledger::RiseAppBackgroundLedger;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::{SaiCacheCleanLayer, SaiCacheCleaner};
use crate::core::lib::sai::modules::sai_ws::sai_ws_cache_refresher::{
    SAI_WS_CACHE_REFRESHER_DID_INVALIDATE, SaiWsCacheRefresherDidInvalidateUserInfo,
    SaiWsCacheRefresherRule,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{
    SaiActiveScreen, SaiNavigationBridge,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_coordinator::SaiWsScreenRefreshCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_route_matcher::SaiWsScreenRouteMatcher;
use crate::core::lib::sai::sai::Sai;

const LOG_NAME: &str = "WS";
const TRIGGER_NAVIGATION: &str = "navigation";
const TRIGGER_FOREGROUND: &str = "foreground";

/// one app wide entity for the cache refresher
/// rule closures run with the entity released so a rule can reach the coordinator again
pub struct SaiWsCacheRefresherCoordinator {
    rules: Vec<SaiWsCacheRefresherRule>,
    lifecycle_installed: bool,
    refreshed_keys_this_cycle: HashSet<String>,
    last_applied_screen: Option<SaiActiveScreen>,
    handled_background_generation: Option<u64>,
    _did_become_active_observer: Option<RiseSubscription>,
    _did_become_active_loop: Option<Task<()>>,
}

impl SaiWsCacheRefresherCoordinator {
    fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            rules: Vec::new(),
            lifecycle_installed: false,
            refreshed_keys_this_cycle: HashSet::new(),
            last_applied_screen: None,
            handled_background_generation: None,
            _did_become_active_observer: None,
            _did_become_active_loop: None,
        }
    }

    pub fn configure(new_rules: Vec<SaiWsCacheRefresherRule>, cx: &mut App) {
        let count = Self::shared(cx).update(cx, |coordinator, cx| {
            coordinator.rules = new_rules;
            cx.notify();
            coordinator.rules.len()
        });
        Self::install_lifecycle_observers_if_needed(cx);
        Sai::log(
            LOG_NAME,
            move || format!("configureCacheRefresher: {count} rule(s)"),
            SaiLogKind::Cache,
            cx,
        );
    }

    pub fn notify_active_screen_changed(cx: &mut App) {
        Self::apply_if_needed(TRIGGER_NAVIGATION, cx);
    }

    pub fn reset_account_state(cx: &mut App) {
        Self::shared(cx).update(cx, |coordinator, cx| {
            coordinator.refreshed_keys_this_cycle.clear();
            coordinator.last_applied_screen = None;
            cx.notify();
        });
    }

    /// window activation comes on the shell thread and hops to main through a channel
    pub fn install_lifecycle_observers_if_needed(cx: &mut App) {
        Self::shared(cx).update(cx, |coordinator, cx| {
            if coordinator.lifecycle_installed {
                return;
            }
            coordinator.lifecycle_installed = true;
            let (activations, mut inbox) = mpsc::unbounded::<()>();
            coordinator._did_become_active_observer = Some(RiseAppEvents::default_center().observe(
                UIApplication::DID_BECOME_ACTIVE_NOTIFICATION,
                move |_| {
                    let _ = activations.unbounded_send(());
                },
            ));
            coordinator._did_become_active_loop = Some(cx.spawn(async move |_, cx| {
                while inbox.next().await.is_some() {
                    cx.update(Self::on_app_became_active);
                }
            }));
            cx.notify();
        });
    }

    fn on_app_became_active(cx: &mut App) {
        let returned = Self::shared(cx).update(cx, |coordinator, cx| {
            if !RiseAppBackgroundLedger::consume_return_from_background(
                &mut coordinator.handled_background_generation,
                cx,
            ) {
                return false;
            }
            coordinator.refreshed_keys_this_cycle.clear();
            coordinator.last_applied_screen = None;
            cx.notify();
            true
        });
        if !returned {
            return;
        }
        SaiWsScreenRefreshCoordinator::reset_revalidation_cycle(cx);
        Self::apply_if_needed(TRIGGER_FOREGROUND, cx);
    }

    fn apply_if_needed(trigger: &str, cx: &mut App) {
        let rules = Self::shared(cx).read(cx).rules.clone();
        if rules.is_empty() {
            return;
        }
        let Some(screen) = SaiNavigationBridge::current_screen(cx) else {
            return;
        };

        let unchanged = Self::shared(cx).update(cx, |coordinator, cx| {
            if trigger == TRIGGER_NAVIGATION
                && coordinator.last_applied_screen.as_ref() == Some(&screen)
            {
                return true;
            }
            coordinator.last_applied_screen = Some(screen.clone());
            cx.notify();
            false
        });
        if unchanged {
            return;
        }

        let mut all_keys: Vec<String> = Vec::new();
        for rule in &rules {
            if !SaiWsScreenRouteMatcher::matches(
                &rule.when_on_screen,
                rule.route_params.as_ref(),
                &screen,
            ) {
                continue;
            }

            let keys: Vec<String> = (rule.refresh_cache_keys)(&screen, cx)
                .iter()
                .map(|key| trimming_whitespaces_and_newlines(key).to_owned())
                .filter(|key| !key.is_empty())
                .collect();
            if keys.is_empty() {
                continue;
            }

            all_keys.extend(keys.iter().cloned());
            Self::invalidate_caches(&keys, &rule.refresh_cache_layers, trigger, cx);
            if let Some(on_refresh) = &rule.on_refresh {
                on_refresh(&screen, &keys, cx);
            }
        }

        if !all_keys.is_empty() {
            let screen_id = screen.id.clone();
            let trigger = trigger.to_owned();
            Sai::log(
                LOG_NAME,
                move || {
                    format!(
                        "cacheRefresher [{trigger}] screen={screen_id} keys={}",
                        all_keys.join(", ")
                    )
                },
                SaiLogKind::Cache,
                cx,
            );
        }
    }

    fn invalidate_caches(
        keys: &[String],
        layers: &[SaiCacheCleanLayer],
        trigger: &str,
        cx: &mut App,
    ) {
        let mut dropped: Vec<String> = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            let first_this_cycle = Self::shared(cx).update(cx, |coordinator, cx| {
                let inserted = coordinator.refreshed_keys_this_cycle.insert(key.clone());
                if inserted {
                    cx.notify();
                }
                inserted
            });
            if !first_this_cycle {
                continue;
            }
            let layer = layers
                .get(index)
                .or_else(|| layers.last())
                .copied()
                .unwrap_or(SaiCacheCleanLayer::Everywhere);
            SaiCacheCleaner::clean(std::slice::from_ref(key), layer, cx);
            SaiWsScreenRefreshCoordinator::forget_revalidated(key, cx);
            dropped.push(key.clone());
        }
        if dropped.is_empty() {
            return;
        }
        RiseAppEvents::default_center().post_with(
            SAI_WS_CACHE_REFRESHER_DID_INVALIDATE,
            SaiWsCacheRefresherDidInvalidateUserInfo {
                cache_keys: dropped,
                trigger: trigger.to_owned(),
            },
        );
    }
}

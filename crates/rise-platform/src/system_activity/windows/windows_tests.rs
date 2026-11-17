use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use ::windows::Win32::Networking::WinSock::{
    NetworkConnectivityLevelHintConstrainedInternetAccess, NetworkConnectivityLevelHintHidden,
    NetworkConnectivityLevelHintInternetAccess, NetworkConnectivityLevelHintLocalAccess,
    NetworkConnectivityLevelHintNone, NetworkConnectivityLevelHintUnknown,
};

use crate::system_activity::*;
use super::{NativeSystemActivity, level_is_satisfied};

#[test]
fn every_usable_interface_is_a_satisfied_path() {
    assert!(!level_is_satisfied(NetworkConnectivityLevelHintNone));
    assert!(!level_is_satisfied(NetworkConnectivityLevelHintHidden));
    for level in [
        NetworkConnectivityLevelHintUnknown,
        NetworkConnectivityLevelHintLocalAccess,
        NetworkConnectivityLevelHintConstrainedInternetAccess,
        NetworkConnectivityLevelHintInternetAccess,
    ] {
        assert!(level_is_satisfied(level), "{level:?}");
    }
}

#[test]
fn the_suspend_resume_registration_is_made_and_undone() {
    let watch = NativeSystemActivity
        .watch_sleep(Arc::new(|_event| {}))
        .expect("every supported Windows accepts a callback suspend and resume registration");
    drop(watch);
}

#[test]
fn the_connectivity_hint_reports_the_current_level_on_start() {
    let (sender, receiver) = mpsc::channel();
    let watch = NativeSystemActivity
        .watch_network_path(Arc::new(move |event| {
            let _ = sender.send(event);
        }))
        .expect("Windows 10 2004 and later export NotifyNetworkConnectivityHintChange");
    let first = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the initial notification carries the current level");
    assert!(matches!(first, SystemActivityEvent::NetworkPath { .. }));
    drop(watch);
}

#[test]
fn a_watch_dropped_inside_its_own_callback_does_not_deadlock() {
    let (sender, receiver) = mpsc::channel::<()>();
    let slot: Arc<parking_lot::Mutex<Option<ActivityWatch>>> = Arc::default();
    let inner = Arc::clone(&slot);
    let watch = NativeSystemActivity
        .watch_network_path(Arc::new(move |_event| {
            drop(inner.lock().take());
            let _ = sender.send(());
        }))
        .expect("connectivity hint registration");
    *slot.lock() = Some(watch);
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the callback returns while the cancel runs on another thread");
    let left = slot.lock().take();
    drop(left);
}

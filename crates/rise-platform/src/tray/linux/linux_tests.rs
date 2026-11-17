use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::{take_announce, Announce, LinuxTray, Shared};
use crate::gpui_shim::PlatformSupport;
use crate::tray::*;

fn shared() -> Shared {
    Shared::new(&TrayConfig::new("Riseonly", "Riseonly"))
}

#[test]
fn nothing_changed_means_nothing_to_announce() {
    let shared = shared();
    let mut state = shared.state.lock();
    assert_eq!(take_announce(&mut state), Some(None));
}

#[test]
fn a_badge_is_announced_as_a_new_tooltip_with_the_count_folded_in() {
    let shared = shared();
    shared.update(|state| {
        state.count = 4;
        state.tooltip_dirty = true;
    });
    let mut state = shared.state.lock();
    assert_eq!(
        take_announce(&mut state),
        Some(Some(Announce {
            tooltip: true,
            status: None,
            layout: None
        }))
    );
    assert_eq!(state.tooltip(), "Riseonly (4)");
    assert_eq!(take_announce(&mut state), Some(None), "announced once");
}

#[test]
fn hiding_is_announced_as_the_passive_status() {
    let shared = shared();
    shared.update(|state| {
        state.visible = false;
        state.status_dirty = true;
    });
    let mut state = shared.state.lock();
    assert_eq!(
        take_announce(&mut state),
        Some(Some(Announce {
            tooltip: false,
            status: Some("Passive"),
            layout: None
        }))
    );
}

#[test]
fn a_dropped_tray_ends_the_service() {
    let shared = shared();
    shared.update(|state| {
        state.tooltip_dirty = true;
        state.closed = true;
    });
    let mut state = shared.state.lock();
    assert_eq!(take_announce(&mut state), None);
}

#[test]
fn opening_hands_over_the_service_exactly_once() {
    let mut tray = open(TrayConfig::new("Riseonly", "Riseonly")).expect("the tray builds");
    assert!(tray.take_service().is_some());
    assert!(tray.take_service().is_none());
    assert_eq!(tray.availability(), TrayAvailability::Available);
    assert!(tray.set_badge(3).is_supported());
}

#[test]
fn a_tray_without_a_host_keeps_changes_for_the_next_watcher() {
    let shared = Arc::new(shared());
    shared.no_host.store(true, Ordering::Release);
    let mut tray = LinuxTray {
        shared: Arc::clone(&shared),
        service: None,
        handler: Default::default(),
    };
    assert_eq!(tray.set_badge(3), PlatformSupport::Unsupported);
    assert_eq!(tray.availability(), TrayAvailability::NoHost);
    assert_eq!(shared.state.lock().count, 3);
    shared.no_host.store(false, Ordering::Release);
    assert_eq!(tray.availability(), TrayAvailability::Available);
    assert_eq!(tray.set_visible(false), PlatformSupport::Performed);
}

#[test]
fn a_new_menu_is_announced_with_its_revision_once() {
    let shared = shared();
    let first = shared.state.lock().revision;
    shared.update(|state| {
        state.menu = vec![TrayMenuItem::plain(1, "Open")];
        state.revision += 1;
        state.layout_dirty = true;
    });
    let mut state = shared.state.lock();
    assert_eq!(
        take_announce(&mut state),
        Some(Some(Announce {
            tooltip: false,
            status: None,
            layout: Some(first + 1)
        }))
    );
    assert_eq!(take_announce(&mut state), Some(None), "announced once");
}

#[test]
fn a_click_waits_in_the_queue_for_the_main_thread() {
    let shared = shared();
    shared.send_event(TrayEvent::Activate);
    shared.send_event(TrayEvent::Menu(5));
    let mut events = shared.pending_events.lock().take().expect("not taken yet");
    assert_eq!(events.try_recv().ok(), Some(TrayEvent::Activate));
    assert_eq!(events.try_recv().ok(), Some(TrayEvent::Menu(5)));
}

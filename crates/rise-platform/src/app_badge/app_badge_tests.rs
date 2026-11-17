use std::cell::RefCell;
use std::rc::Rc;

use crate::app_badge::*;
use crate::tray::{InMemoryTray, Tray, dock_badge_label, menu_bar_title};
use crate::host_os::HostOs;

fn reset() {
    clear_tray_listener();
    let _ = clear();
}

fn listening_tray() -> Rc<RefCell<InMemoryTray>> {
    let tray = Rc::new(RefCell::new(InMemoryTray::new()));
    let held = Rc::clone(&tray);
    set_tray_listener(move |count| {
        let _ = held.borrow_mut().set_badge(count);
    });
    tray
}

#[test]
fn zero_draws_no_badge() {
    assert_eq!(badge_text(0), None);
}

#[test]
fn one_shows_its_own_digit() {
    assert_eq!(badge_text(1), Some("1".to_owned()));
}

#[test]
fn the_cap_is_still_a_number() {
    assert_eq!(BADGE_CAP, 999);
    assert_eq!(badge_text(999), Some("999".to_owned()));
}

#[test]
fn one_past_the_cap_collapses_as_on_ios() {
    assert_eq!(badge_text(1_000), Some("999+".to_owned()));
    assert_eq!(badge_text(u32::MAX), Some("999+".to_owned()));
}

#[test]
fn set_count_remembers_the_number() {
    reset();
    let _ = set_count(7);
    assert_eq!(current_count(), 7);
    reset();
}

#[test]
fn clear_takes_the_number_to_zero_and_the_tray_with_it() {
    reset();
    let tray = listening_tray();
    let _ = set_count(5);
    assert_eq!(tray.borrow().badge(), Some("5"));

    let _ = clear();
    assert_eq!(current_count(), 0);
    assert_eq!(tray.borrow().badge(), None);
    reset();
}

#[test]
fn the_tray_and_the_badge_read_the_same_at_every_edge() {
    reset();
    let tray = listening_tray();
    for count in [0, 1, 999, 1_000, u32::MAX] {
        let _ = set_count(count);
        assert_eq!(
            tray.borrow().badge().map(str::to_owned),
            badge_text(count),
            "the tray drifted from the badge at {count}"
        );
        assert_eq!(dock_badge_label(HostOs::MacOs, count), badge_text(count));
    }
    assert_eq!(menu_bar_title("R", 1_000), "R 999+");
    reset();
}

#[test]
fn a_tray_opened_after_the_total_was_known_catches_up_at_once() {
    reset();
    let _ = set_count(12);
    let tray = listening_tray();
    assert_eq!(tray.borrow().badge(), Some("12"));
    reset();
}

#[test]
fn a_tray_that_went_away_hears_nothing_more() {
    reset();
    let tray = listening_tray();
    let _ = set_count(3);
    clear_tray_listener();
    let _ = set_count(4);
    assert_eq!(tray.borrow().badge(), Some("3"));
    reset();
}

#[test]
fn a_listener_may_ask_for_the_current_count_while_it_is_told() {
    reset();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = Rc::clone(&seen);
    set_tray_listener(move |_| held.borrow_mut().push(current_count()));
    let _ = set_count(9);
    assert_eq!(*seen.borrow(), vec![0, 9]);
    reset();
}

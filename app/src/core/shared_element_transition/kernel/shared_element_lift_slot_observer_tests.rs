use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{Entity, TestAppContext};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementKey, SharedElementPart, SharedElementSurface,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::{
    SharedElementLiftBoard, SharedElementLiftObservation, SharedElementLiftSlot,
};
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;

type LiftRecorder = Rc<RefCell<Vec<bool>>>;

fn key(surface: &SharedElementSurface, item_id: &str) -> SharedElementKey {
    SharedElementKey::new(surface.clone(), item_id, SharedElementPart::Media)
}

fn fresh_surface() -> SharedElementSurface {
    SharedElementSurface::PostList { session: SharedElementTransitionCoordinator::next_session() }
}

fn board(cx: &mut TestAppContext) -> Entity<SharedElementLiftBoard> {
    cx.update(SharedElementLiftBoard::shared)
}

fn slot(cx: &mut TestAppContext, key: &SharedElementKey) -> Entity<SharedElementLiftSlot> {
    let board = board(cx);
    cx.update(|cx| board.update(cx, |board, cx| board.slot(key, cx)))
}

fn observe(cx: &mut TestAppContext, slot: &Entity<SharedElementLiftSlot>, recorder: &LiftRecorder) -> SharedElementLiftObservation {
    let recorder = recorder.clone();
    cx.update(|cx| {
        slot.update(cx, |slot, cx| {
            slot.add_observer(move |is_hidden, _| recorder.borrow_mut().push(is_hidden), cx)
        })
    })
}

fn lift(cx: &mut TestAppContext, keys: &[&SharedElementKey], epoch: u64) {
    let board = board(cx);
    let keys: HashSet<SharedElementKey> = keys.iter().map(|key| (*key).clone()).collect();
    cx.update(|cx| board.update(cx, |board, cx| board.lift(keys, epoch, cx)));
}

fn unlift(cx: &mut TestAppContext, key: &SharedElementKey, epoch: u64) {
    let board = board(cx);
    cx.update(|cx| board.update(cx, |board, cx| board.unlift(key, epoch, cx)));
}

fn land(cx: &mut TestAppContext, epoch: u64) {
    let board = board(cx);
    cx.update(|cx| board.update(cx, |board, cx| board.land(epoch, cx)));
}

fn clear_all(cx: &mut TestAppContext) {
    let board = board(cx);
    cx.update(|cx| board.update(cx, |board, cx| board.clear_all(cx)));
}

fn is_hidden(cx: &mut TestAppContext, slot: &Entity<SharedElementLiftSlot>) -> bool {
    cx.update(|cx| slot.read(cx).is_hidden())
}

fn cancel(cx: &mut TestAppContext, token: &mut SharedElementLiftObservation) {
    cx.update(|cx| token.cancel(cx));
}

#[gpui::test]
fn lift_notifies_once_and_only_on_real_change(cx: &mut TestAppContext) {
    let surface = fresh_surface();
    let key = key(&surface, "a");
    let slot = slot(cx, &key);
    let recorder = LiftRecorder::default();
    let mut token = observe(cx, &slot, &recorder);

    lift(cx, &[&key], 9_001);
    lift(cx, &[&key], 9_001);
    lift(cx, &[&key], 9_002);

    assert_eq!(*recorder.borrow(), vec![true]);
    assert!(is_hidden(cx, &slot));

    land(cx, 9_002);
    assert_eq!(*recorder.borrow(), vec![true, false]);
    cancel(cx, &mut token);
}

#[gpui::test]
fn unlift_notifies_only_for_the_lifting_epoch(cx: &mut TestAppContext) {
    let surface = fresh_surface();
    let key = key(&surface, "a");
    let slot = slot(cx, &key);
    let recorder = LiftRecorder::default();
    let mut token = observe(cx, &slot, &recorder);

    unlift(cx, &key, 9_101);
    assert!(recorder.borrow().is_empty());

    lift(cx, &[&key], 9_101);
    unlift(cx, &key, 9_102);
    assert_eq!(*recorder.borrow(), vec![true]);
    assert!(is_hidden(cx, &slot));

    unlift(cx, &key, 9_101);
    unlift(cx, &key, 9_101);
    assert_eq!(*recorder.borrow(), vec![true, false]);
    assert!(!is_hidden(cx, &slot));
    cancel(cx, &mut token);
}

#[gpui::test]
fn land_notifies_only_its_own_epoch(cx: &mut TestAppContext) {
    let surface = fresh_surface();
    let first = key(&surface, "a");
    let second = key(&surface, "b");
    let first_slot = slot(cx, &first);
    let second_slot = slot(cx, &second);
    let first_recorder = LiftRecorder::default();
    let second_recorder = LiftRecorder::default();
    let mut first_token = observe(cx, &first_slot, &first_recorder);
    let mut second_token = observe(cx, &second_slot, &second_recorder);

    lift(cx, &[&first], 9_201);
    lift(cx, &[&second], 9_202);

    land(cx, 9_201);
    assert_eq!(*first_recorder.borrow(), vec![true, false]);
    assert_eq!(*second_recorder.borrow(), vec![true]);
    assert!(is_hidden(cx, &second_slot));

    land(cx, 9_201);
    assert_eq!(*first_recorder.borrow(), vec![true, false]);
    assert_eq!(*second_recorder.borrow(), vec![true]);

    land(cx, 9_202);
    assert_eq!(*second_recorder.borrow(), vec![true, false]);
    cancel(cx, &mut first_token);
    cancel(cx, &mut second_token);
}

#[gpui::test]
fn clear_all_notifies_each_lifted_slot_once(cx: &mut TestAppContext) {
    let surface = fresh_surface();
    let first = key(&surface, "a");
    let second = key(&surface, "b");
    let idle = key(&surface, "c");
    let first_slot = slot(cx, &first);
    let second_slot = slot(cx, &second);
    let idle_slot = slot(cx, &idle);
    let first_recorder = LiftRecorder::default();
    let second_recorder = LiftRecorder::default();
    let idle_recorder = LiftRecorder::default();
    let mut tokens = vec![
        observe(cx, &first_slot, &first_recorder),
        observe(cx, &second_slot, &second_recorder),
        observe(cx, &idle_slot, &idle_recorder),
    ];

    lift(cx, &[&first, &second], 9_301);
    clear_all(cx);
    clear_all(cx);

    assert_eq!(*first_recorder.borrow(), vec![true, false]);
    assert_eq!(*second_recorder.borrow(), vec![true, false]);
    assert!(idle_recorder.borrow().is_empty());
    assert!(!is_hidden(cx, &first_slot));
    assert!(!is_hidden(cx, &second_slot));
    for token in &mut tokens {
        cancel(cx, token);
    }
}

#[gpui::test]
fn cancelled_and_released_observers_are_not_called(cx: &mut TestAppContext) {
    let surface = fresh_surface();
    let key = key(&surface, "a");
    let slot = slot(cx, &key);
    let cancelled_recorder = LiftRecorder::default();
    let released_recorder = LiftRecorder::default();
    let live_recorder = LiftRecorder::default();

    let mut cancelled = observe(cx, &slot, &cancelled_recorder);
    {
        let released = observe(cx, &slot, &released_recorder);
        drop(released);
    }
    let mut live = observe(cx, &slot, &live_recorder);
    cancel(cx, &mut cancelled);

    lift(cx, &[&key], 9_401);
    land(cx, 9_401);

    assert!(cancelled_recorder.borrow().is_empty());
    assert!(released_recorder.borrow().is_empty());
    assert_eq!(*live_recorder.borrow(), vec![true, false]);
    cancel(cx, &mut live);
}

#[gpui::test]
fn nested_write_from_an_observer_delivers_only_the_latest_value_to_later_observers(cx: &mut TestAppContext) {
    let surface = fresh_surface();
    let key = key(&surface, "a");
    let slot = slot(cx, &key);
    let first_recorder = LiftRecorder::default();
    let later_recorder = LiftRecorder::default();
    let mut first = {
        let recorder = first_recorder.clone();
        let key = key.clone();
        cx.update(|cx| {
            slot.update(cx, |slot, cx| {
                slot.add_observer(
                    move |is_hidden, cx| {
                        recorder.borrow_mut().push(is_hidden);
                        if is_hidden {
                            SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.unlift(&key, 9_501, cx));
                        }
                    },
                    cx,
                )
            })
        })
    };
    let mut later = observe(cx, &slot, &later_recorder);

    lift(cx, &[&key], 9_501);

    assert_eq!(*first_recorder.borrow(), vec![true, false]);
    assert_eq!(*later_recorder.borrow(), vec![false]);
    assert!(!is_hidden(cx, &slot));
    land(cx, 9_501);
    assert_eq!(*later_recorder.borrow(), vec![false]);
    cancel(cx, &mut first);
    cancel(cx, &mut later);
}

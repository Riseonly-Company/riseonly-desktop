use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, ElementId, FocusHandle, HitboxBehavior, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, RenderOnce, Styled, Task, Window,
    canvas, div, point,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::context_click::is_context_click_here;
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthrough;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::rise_strict_tap_gesture_recognizer::{
    RiseStrictTapGestureRecognizer, RiseTapPolicy,
};
use rise_ui::swiftui::context_menu::is_context_menu_keystroke;
use rise_ui::swiftui::text_selection::{mark_gesture_plate_press, text_glyph_took_press};
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeReader};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_touch_tracker::HoldContextMenuTouchTracker;

const LOG_NAME: &str = "HoldMenu";
const DOUBLE_TAP_FAILURE_DELAY: f64 = 0.35;

pub type HoldGesturePlateAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// receives the plate, its probe says where it is drawn
pub type HoldGesturePlateViewAction = Rc<dyn Fn(&ViewAnchorProbe, &mut Window, &mut App)>;
pub type HoldGesturePlateConfirm = Rc<dyn Fn(&ViewAnchorProbe, Point<Pixels>, &mut Window, &mut App)>;
pub type HoldGesturePlatePointAction = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
pub type HoldGesturePlateRelease = Rc<dyn Fn(Option<Point<Pixels>>, &mut Window, &mut App)>;
pub type HoldGesturePlatePredicate = Rc<dyn Fn(&App) -> bool>;

/// defaults are tap tolerance of MOVEMENT_SLOP, every callback a no op
/// can_double_tap true and no double tap
/// builders set the rest, every point is in window coordinates
#[derive(IntoElement)]
pub struct HoldGesturePlate {
    id: ElementId,
    callbacks: Coordinator,
    tap_movement_tolerance: f32,
    context_click_across_row: bool,
}

#[derive(Clone)]
struct Coordinator {
    on_touch_down: HoldGesturePlateAction,
    on_anticipate: HoldGesturePlateViewAction,
    on_confirm: HoldGesturePlateConfirm,
    on_update: HoldGesturePlatePointAction,
    on_release: HoldGesturePlateRelease,
    on_cancel: HoldGesturePlateAction,
    on_tap: HoldGesturePlatePointAction,
    can_double_tap: HoldGesturePlatePredicate,
    on_double_tap: Option<HoldGesturePlatePointAction>,
}

impl HoldGesturePlate {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            callbacks: Coordinator {
                on_touch_down: Rc::new(|_, _| {}),
                on_anticipate: Rc::new(|_, _, _| {}),
                on_confirm: Rc::new(|_, _, _, _| {}),
                on_update: Rc::new(|_, _, _| {}),
                on_release: Rc::new(|_, _, _| {}),
                on_cancel: Rc::new(|_, _| {}),
                on_tap: Rc::new(|_, _, _| {}),
                can_double_tap: Rc::new(|_| true),
                on_double_tap: None,
            },
            tap_movement_tolerance: RiseTapPolicy::MOVEMENT_SLOP,
            context_click_across_row: false,
        }
    }

    /// desktop, a context click anywhere in the row the plate sits in opens this plate menu at the pointer
    /// beside the bubble or over the avatar column, same as telegram desktop
    /// row is the clip the plate is drawn in
    pub fn context_click_across_row(mut self, context_click_across_row: bool) -> Self {
        self.context_click_across_row = context_click_across_row;
        self
    }

    pub fn tap_movement_tolerance(mut self, tap_movement_tolerance: f32) -> Self {
        self.tap_movement_tolerance = tap_movement_tolerance;
        self
    }

    pub fn on_touch_down(mut self, on_touch_down: HoldGesturePlateAction) -> Self {
        self.callbacks.on_touch_down = on_touch_down;
        self
    }

    pub fn on_anticipate(mut self, on_anticipate: HoldGesturePlateViewAction) -> Self {
        self.callbacks.on_anticipate = on_anticipate;
        self
    }

    pub fn on_confirm(mut self, on_confirm: HoldGesturePlateConfirm) -> Self {
        self.callbacks.on_confirm = on_confirm;
        self
    }

    pub fn on_update(mut self, on_update: HoldGesturePlatePointAction) -> Self {
        self.callbacks.on_update = on_update;
        self
    }

    pub fn on_release(mut self, on_release: HoldGesturePlateRelease) -> Self {
        self.callbacks.on_release = on_release;
        self
    }

    pub fn on_cancel(mut self, on_cancel: HoldGesturePlateAction) -> Self {
        self.callbacks.on_cancel = on_cancel;
        self
    }

    pub fn on_tap(mut self, on_tap: HoldGesturePlatePointAction) -> Self {
        self.callbacks.on_tap = on_tap;
        self
    }

    pub fn can_double_tap(mut self, can_double_tap: HoldGesturePlatePredicate) -> Self {
        self.callbacks.can_double_tap = can_double_tap;
        self
    }

    pub fn on_double_tap(mut self, on_double_tap: Option<HoldGesturePlatePointAction>) -> Self {
        self.callbacks.on_double_tap = on_double_tap;
        self
    }
}

/// progress of a long press recognizer for the current press
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LongPressState {
    Possible,
    Began,
    Failed,
}

/// plate with the per press flags of its coordinator
/// state is keyed element state so a plate that leaves the tree mid press is dropped
/// with its start point and timers and pending tap, a remounted plate starts fresh
struct Plate {
    coordinator: Coordinator,
    probe: ViewAnchorProbe,
    focus_handle: FocusHandle,
    tap_movement_tolerance: f32,
    confirmed: bool,
    anticipated: bool,
    block_tap_for_current_touch: bool,
    is_pressed: bool,
    touch_moved_beyond_tap_slop: bool,
    touch_start_location: Option<Point<Pixels>>,
    touch_start_time: Duration,
    anticipation: LongPressState,
    confirm: LongPressState,
    tap_recognizer: RiseStrictTapGestureRecognizer,
    double_tap_recognizer: RiseStrictTapGestureRecognizer,
    hold_timers: Option<Task<()>>,
    pending_tap: Option<Task<()>>,
}

impl Plate {
    fn new(coordinator: Coordinator, focus_handle: FocusHandle) -> Self {
        Self {
            coordinator,
            probe: ViewAnchorProbe::new(),
            focus_handle,
            tap_movement_tolerance: RiseTapPolicy::MOVEMENT_SLOP,
            confirmed: false,
            anticipated: false,
            block_tap_for_current_touch: false,
            is_pressed: false,
            touch_moved_beyond_tap_slop: false,
            touch_start_location: None,
            touch_start_time: Duration::ZERO,
            anticipation: LongPressState::Possible,
            confirm: LongPressState::Possible,
            tap_recognizer: RiseStrictTapGestureRecognizer::new(),
            double_tap_recognizer: RiseStrictTapGestureRecognizer::new(),
            hold_timers: None,
            pending_tap: None,
        }
    }

    fn routes_tap_through_recognizers(&self) -> bool {
        self.coordinator.on_double_tap.is_some()
    }

    /// fresh callbacks and tolerance every render
    fn sync(&mut self, coordinator: Coordinator, tap_movement_tolerance: f32) {
        self.coordinator = coordinator;
        self.tap_movement_tolerance = tap_movement_tolerance;
        self.tap_recognizer.movement_slop = tap_movement_tolerance;
        self.double_tap_recognizer.movement_slop = tap_movement_tolerance;
    }

    fn reset_touch_cycle(&mut self) {
        self.confirmed = false;
        self.anticipated = false;
        self.block_tap_for_current_touch = false;
    }

    fn mark_hold_engaged(&mut self) {
        self.block_tap_for_current_touch = true;
    }

    fn exceeds(point: Point<Pixels>, origin: Point<Pixels>, tolerance: f32) -> bool {
        let dx = f32::from(point.x - origin.x);
        let dy = f32::from(point.y - origin.y);
        dx * dx + dy * dy > tolerance * tolerance
    }
}

impl RenderOnce for HoldGesturePlate {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        HoldContextMenuTouchTracker::install(window);
        let callbacks = self.callbacks.clone();
        let plate =
            window.use_keyed_state(self.id.clone(), cx, move |_, cx| Plate::new(callbacks, cx.focus_handle().tab_stop(true)));
        let (probe, focus_handle) = plate.update(cx, |plate, _| {
            plate.sync(self.callbacks, self.tap_movement_tolerance);
            (plate.probe.clone(), plate.focus_handle.clone())
        });
        let focus_ring = rise_ui::theme(cx).primary._100;

        let primary = plate.clone();
        let secondary = plate.clone();
        let released = plate.clone();
        let released_secondary = plate.clone();
        let follower = plate.clone();
        let keyed = plate.clone();
        let row_band = self.context_click_across_row.then(|| row_context_band(plate.clone()));
        div()
            .id(self.id)
            .absolute()
            .inset_0()
            .track_focus(&focus_handle)
            .focus_visible(move |style| style.border_2().border_color(focus_ring))
            .capture_any_mouse_down(|_: &MouseDownEvent, window: &mut Window, _: &mut App| window.prevent_default())
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if is_context_menu_keystroke(&event.keystroke) {
                    cx.stop_propagation();
                    Plate::confirm_from_keyboard(&keyed, window, cx);
                }
            })
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                mark_gesture_plate_press(event.position, cx);
                let secondary = is_context_click_here(event.button, &event.modifiers);
                Plate::touches_began(&primary, event.position, secondary, event.click_count, window, cx);
            })
            .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, window, cx| {
                Plate::touches_began(&secondary, event.position, true, 1, window, cx);
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, window, cx| {
                Plate::touches_ended(&released, event, window, cx);
            })
            .on_mouse_up(MouseButton::Right, move |event: &MouseUpEvent, window, cx| {
                Plate::touches_ended(&released_secondary, event, window, cx);
            })
            .child(ViewAnchorProbeReader::new(probe))
            .children(row_band)
            .with_press_follower(follower)
    }
}

fn row_context_band(plate: gpui::Entity<Plate>) -> impl IntoElement {
    canvas(
        |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
            let mask = window.content_mask().bounds;
            let top = mask.top().max(bounds.top());
            let bottom = mask.bottom().min(bounds.bottom()).max(top);
            window.insert_hitbox(
                Bounds::from_corners(point(mask.left(), top), point(mask.right(), bottom)),
                HitboxBehavior::Normal,
            )
        },
        move |bounds: Bounds<Pixels>, band, window: &mut Window, _: &mut App| {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Bubble || bounds.contains(&event.position) || !band.is_hovered(window) {
                    return;
                }
                let secondary =
                    event.button == MouseButton::Right || is_context_click_here(event.button, &event.modifiers);
                if !secondary || HoldContextMenuPresenter::has_active_request(cx) {
                    return;
                }
                cx.stop_propagation();
                Plate::touches_began(&plate, event.position, true, 1, window, cx);
            });
        },
    )
    .absolute()
    .size_full()
}

trait PressFollower: ParentElement + Sized {
    fn with_press_follower(self, plate: gpui::Entity<Plate>) -> Self {
        let follower = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moved = plate.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble || !moved.read(cx).is_pressed {
                        return;
                    }
                    if event.pressed_button.is_none() {
                        Plate::touches_cancelled(&moved, window, cx);
                    } else {
                        Plate::touches_moved(&moved, event.position, window, cx);
                    }
                });
                let ended = plate.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Bubble && ended.read(cx).is_pressed {
                        Plate::touches_ended(&ended, event, window, cx);
                    }
                });
            },
        )
        .absolute()
        .size_0();
        let mut this = self;
        this.extend(std::iter::once(follower.into_any_element()));
        this
    }
}

impl<T: ParentElement> PressFollower for T {}

type PlateEntity = gpui::Entity<Plate>;

/// what the strict tap recognizers made of a release
#[derive(Clone, Copy)]
struct TapRelease {
    location: Point<Pixels>,
    click_count: usize,
    tap: bool,
    double_tap: bool,
}

impl Plate {
    fn touches_began(
        entity: &PlateEntity,
        location: Point<Pixels>,
        secondary: bool,
        click_count: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let menu_is_open = HoldContextMenuPresenter::has_active_request(cx);
        if !menu_is_open && !secondary && BubbleScrollPassthrough::with_shared(|shared| shared.contains(location)) {
            return;
        }
        if entity.read(cx).is_pressed {
            Self::touches_cancelled(entity, window, cx);
        }
        let on_touch_down = entity.update(cx, |plate, _| {
            if plate.is_pressed {
                return None;
            }
            plate.is_pressed = true;
            plate.touch_moved_beyond_tap_slop = false;
            plate.touch_start_location = Some(location);
            plate.touch_start_time = RiseClock::monotonic();
            plate.anticipation = LongPressState::Possible;
            plate.confirm = LongPressState::Possible;
            plate.tap_recognizer.touches_began(location);
            plate.double_tap_recognizer.touches_began(location);
            plate.reset_touch_cycle();
            Some(plate.coordinator.on_touch_down.clone())
        });
        let Some(on_touch_down) = on_touch_down else { return };
        on_touch_down(window, cx);

        if secondary {
            Self::handle_confirm_began(entity, location, window, cx);
            return;
        }
        if click_count >= 2 {
            return;
        }
        let weak = entity.downgrade();
        let window_handle = window.window_handle();
        let timers = cx.spawn(async move |cx| {
            let anticipation = Duration::from_secs_f64(HoldContextMenuMetrics::ANTICIPATION_DURATION);
            let confirm = Duration::from_secs_f64(HoldContextMenuMetrics::CONFIRM_DURATION);
            cx.background_executor().timer(anticipation).await;
            let Some(plate) = weak.upgrade() else { return };
            let _ = window_handle.update(cx, |_, window, cx| Self::handle_anticipation_began(&plate, window, cx));
            cx.background_executor().timer(confirm.saturating_sub(anticipation)).await;
            let _ = window_handle.update(cx, |_, window, cx| {
                let location = window.mouse_position();
                Self::handle_confirm_began(&plate, location, window, cx);
            });
        });
        entity.update(cx, |plate, _| plate.hold_timers = Some(timers));
    }

    fn handle_anticipation_began(entity: &PlateEntity, window: &mut Window, cx: &mut App) {
        let confirmed = entity.read(cx).confirmed;
        let state = entity.read(cx).anticipation;
        if state != LongPressState::Possible || !entity.read(cx).is_pressed {
            return;
        }
        Sai::log(LOG_NAME, || format!("anticipate began confirmed={confirmed}"), SaiLogKind::Ui, cx);
        let on_anticipate = entity.update(cx, |plate, _| {
            plate.anticipation = LongPressState::Began;
            if plate.confirmed {
                return None;
            }
            plate.anticipated = true;
            plate.mark_hold_engaged();
            Some((plate.coordinator.on_anticipate.clone(), plate.probe.clone()))
        });
        if let Some((on_anticipate, probe)) = on_anticipate {
            on_anticipate(&probe, window, cx);
        }
    }

    fn handle_confirm_began(entity: &PlateEntity, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let on_confirm = entity.update(cx, |plate, _| {
            if plate.confirm != LongPressState::Possible || !plate.is_pressed {
                return None;
            }
            plate.confirm = LongPressState::Began;
            plate.confirmed = true;
            plate.mark_hold_engaged();
            Some((plate.coordinator.on_confirm.clone(), plate.probe.clone()))
        });
        if let Some((on_confirm, probe)) = on_confirm {
            on_confirm(&probe, location, window, cx);
        }
    }

    fn confirm_from_keyboard(entity: &PlateEntity, window: &mut Window, cx: &mut App) {
        let (on_confirm, probe) = {
            let plate = entity.read(cx);
            (plate.coordinator.on_confirm.clone(), plate.probe.clone())
        };
        let Some(frame) = probe.window_frame() else { return };
        on_confirm(&probe, frame.center(), window, cx);
    }

    fn touches_moved(entity: &PlateEntity, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let on_update = entity.update(cx, |plate, _| {
            let start = plate.touch_start_location?;
            if Self::exceeds(location, start, plate.tap_movement_tolerance) {
                plate.touch_moved_beyond_tap_slop = true;
            }
            plate.tap_recognizer.touches_moved(location);
            plate.double_tap_recognizer.touches_moved(location);
            if plate.anticipation == LongPressState::Possible
                && Self::exceeds(location, start, HoldContextMenuMetrics::CANCEL_MOVEMENT * 0.6)
            {
                plate.anticipation = LongPressState::Failed;
            }
            if plate.confirm == LongPressState::Possible
                && Self::exceeds(location, start, HoldContextMenuMetrics::CANCEL_MOVEMENT)
            {
                plate.confirm = LongPressState::Failed;
            }
            if plate.anticipation == LongPressState::Failed && plate.confirm == LongPressState::Failed {
                plate.hold_timers = None;
            }
            (plate.confirm == LongPressState::Began && plate.confirmed).then(|| plate.coordinator.on_update.clone())
        });
        if let Some(on_update) = on_update {
            on_update(location, window, cx);
        }
    }

    fn touches_ended(entity: &PlateEntity, event: &MouseUpEvent, window: &mut Window, cx: &mut App) {
        let location = event.position;
        let menu_is_open = HoldContextMenuPresenter::has_active_request(cx);
        let pressed_text = entity.read(cx).touch_start_location.is_some_and(|start| text_glyph_took_press(start, cx));
        let snapshot = entity.update(cx, |plate, _| {
            if !plate.is_pressed {
                return None;
            }
            plate.is_pressed = false;
            plate.hold_timers = None;
            if let Some(start) = plate.touch_start_location
                && Self::exceeds(location, start, plate.tap_movement_tolerance)
            {
                plate.touch_moved_beyond_tap_slop = true;
            }
            let held = RiseClock::monotonic().saturating_sub(plate.touch_start_time).as_secs_f64();
            let tap = plate.tap_recognizer.touches_ended(location);
            let double_tap = plate.double_tap_recognizer.touches_ended(location) && !pressed_text;
            plate.touch_start_location = None;
            Some((plate.confirmed, plate.anticipation, plate.confirm, held, tap, double_tap))
        });
        let Some((confirmed, anticipation, confirm, held, tap, double_tap)) = snapshot else { return };

        if !confirmed && held >= HoldContextMenuMetrics::CONFIRM_DURATION {
            let moved = entity.read(cx).touch_moved_beyond_tap_slop;
            Sai::log(
                LOG_NAME,
                || format!("hold unconfirmed cancelled=false held={held:.2} moved={moved}"),
                SaiLogKind::Ui,
                cx,
            );
        }
        let coordinator = entity.read(cx).coordinator.clone();
        if !confirmed && !menu_is_open {
            (coordinator.on_cancel)(window, cx);
        }
        let routes = entity.read(cx).routes_tap_through_recognizers();
        if !routes {
            let blocked = entity.read(cx).block_tap_for_current_touch;
            let moved = entity.read(cx).touch_moved_beyond_tap_slop;
            let menu_active = HoldContextMenuPresenter::has_active_request(cx);
            if !blocked && !moved && !menu_active {
                (coordinator.on_tap)(location, window, cx);
            } else if !moved && !confirmed {
                tracing::warn!(target: "riseonly::X1161", blocked, menu_active, held, "plate tap dropped");
            }
        }

        if confirm == LongPressState::Began && confirmed {
            (coordinator.on_release)(Some(location), window, cx);
            entity.update(cx, |plate, _| {
                plate.confirmed = false;
                plate.anticipated = false;
            });
        }
        if anticipation == LongPressState::Began {
            let anticipated = entity.read(cx).anticipated;
            let confirmed_now = entity.read(cx).confirmed;
            if !confirmed_now && anticipated {
                Sai::log(LOG_NAME, || "anticipate end state=ended inWindow=true".to_owned(), SaiLogKind::Ui, cx);
                entity.update(cx, |plate, _| plate.anticipated = false);
                if !HoldContextMenuPresenter::has_active_request(cx) {
                    (coordinator.on_cancel)(window, cx);
                }
            }
        }
        if routes {
            let release = TapRelease { location, click_count: event.click_count, tap, double_tap };
            Self::route_taps(entity, &coordinator, release, window, cx);
        }
    }

    fn touches_cancelled(entity: &PlateEntity, window: &mut Window, cx: &mut App) {
        let snapshot = entity.update(cx, |plate, _| {
            if !plate.is_pressed {
                return None;
            }
            plate.is_pressed = false;
            plate.hold_timers = None;
            let held = RiseClock::monotonic().saturating_sub(plate.touch_start_time).as_secs_f64();
            plate.tap_recognizer.reset();
            plate.double_tap_recognizer.reset();
            plate.touch_start_location = None;
            Some((plate.confirmed, plate.anticipation, plate.confirm, held))
        });
        let Some((confirmed, anticipation, confirm, held)) = snapshot else { return };

        if !confirmed && held >= HoldContextMenuMetrics::ANTICIPATION_DURATION {
            let moved = entity.read(cx).touch_moved_beyond_tap_slop;
            Sai::log(
                LOG_NAME,
                || format!("hold unconfirmed cancelled=true held={held:.2} moved={moved}"),
                SaiLogKind::Ui,
                cx,
            );
        }
        let coordinator = entity.read(cx).coordinator.clone();
        if !confirmed && !HoldContextMenuPresenter::has_active_request(cx) {
            (coordinator.on_cancel)(window, cx);
        }

        if confirm == LongPressState::Began && confirmed {
            (coordinator.on_release)(None, window, cx);
            entity.update(cx, |plate, _| {
                plate.confirmed = false;
                plate.anticipated = false;
            });
        }
        if anticipation == LongPressState::Began {
            let anticipated = entity.read(cx).anticipated;
            let confirmed_now = entity.read(cx).confirmed;
            if !confirmed_now && anticipated {
                Sai::log(LOG_NAME, || "anticipate end state=cancelled inWindow=true".to_owned(), SaiLogKind::Ui, cx);
                entity.update(cx, |plate, _| plate.anticipated = false);
                if !HoldContextMenuPresenter::has_active_request(cx) {
                    (coordinator.on_cancel)(window, cx);
                }
            }
        }
    }

    fn route_taps(
        entity: &PlateEntity,
        coordinator: &Coordinator,
        release: TapRelease,
        window: &mut Window,
        cx: &mut App,
    ) {
        let TapRelease { location, click_count, tap, double_tap } = release;
        if entity.read(cx).block_tap_for_current_touch {
            return;
        }
        let can_double_tap = (coordinator.can_double_tap)(cx);
        if click_count >= 2 && click_count % 2 == 0 && double_tap && can_double_tap {
            entity.update(cx, |plate, _| plate.pending_tap = None);
            if let Some(on_double_tap) = &coordinator.on_double_tap
                && !HoldContextMenuPresenter::has_active_request(cx)
            {
                on_double_tap(location, window, cx);
            }
            return;
        }
        if !tap || click_count >= 2 {
            return;
        }
        if !can_double_tap {
            if !HoldContextMenuPresenter::has_active_request(cx) {
                (coordinator.on_tap)(location, window, cx);
            }
            return;
        }
        let on_tap = coordinator.on_tap.clone();
        let window_handle = window.window_handle();
        let pending = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(DOUBLE_TAP_FAILURE_DELAY)).await;
            let _ = window_handle.update(cx, |_, window, cx| {
                if !HoldContextMenuPresenter::has_active_request(cx) {
                    on_tap(location, window, cx);
                }
            });
        });
        entity.update(cx, |plate, _| plate.pending_tap = Some(pending));
    }
}


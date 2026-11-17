use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    App, Bounds, BoxShadow, Canvas, Div, ElementId, Entity, EntityId, FocusHandle, Global, Hsla, IntoElement, KeyDownEvent, MouseMoveEvent,
    Pixels, PlatformInput, Size, Styled, WeakEntity, Window, WindowId, black, point,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::AppTheme;

use crate::swiftui::measure::measure;
use crate::swiftui::prelude::*;
use crate::swiftui::transition::{TransitionEffect, TransitionModifiers, Transitioned};
use crate::window_cover::WindowCoverFrame;

const TAB_KEY: &str = "tab";

const MAXIMUM_TAB_WALK: usize = 512;

/// every number of the modal presentation
pub enum ModalSheetMetrics {}

impl ModalSheetMetrics {
    pub const MAX_HEIGHT_FRACTION: f32 = 0.85;
    pub const FILL_FLOOR_HEIGHT: f32 = 480.0;
    pub const CORNER_RADIUS: f32 = 16.0;
    pub const CONTENT_INSET: f32 = 5.0;
    pub const APPEAR_SCALE: f32 = 0.96;
    pub const PRESENT_DURATION: f64 = 0.18;
    pub const DISMISS_DURATION: f64 = 0.14;
    pub const SHADOW_RADIUS: f32 = 32.0;
    pub const SHADOW_OPACITY: f32 = 0.3;
    pub const SHADOW_OFFSET_Y: f32 = 10.0;
    pub const OUTLINE_OPACITY: f32 = 0.6;
    pub const OUTLINE_WIDTH: f32 = 1.0;
}

/// where a modal sheet fits in its container
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ModalSheetFrame {
    pub width: f32,
    pub max_height: f32,
}

impl ModalSheetFrame {
    pub fn for_window(window: &Window, cx: &App) -> Self {
        Self::resolve(WindowCoverFrame::resolve(window, cx).size, crate::theme(cx))
    }

    pub fn resolve(container: Size<Pixels>, theme: &AppTheme) -> Self {
        Self::laid_out(
            f32::from(container.width),
            f32::from(container.height),
            f32::from(theme.modal.width_medium),
            f32::from(theme.shell.overlay_margin),
        )
    }

    pub fn for_window_wide(window: &Window, cx: &App) -> Self {
        let container = WindowCoverFrame::resolve(window, cx).size;
        let theme = crate::theme(cx);
        Self::laid_out(
            f32::from(container.width),
            f32::from(container.height),
            f32::from(theme.modal.width_large),
            f32::from(theme.shell.overlay_margin),
        )
    }

    pub fn for_window_small(window: &Window, cx: &App) -> Self {
        let container = WindowCoverFrame::resolve(window, cx).size;
        let theme = crate::theme(cx);
        Self::laid_out(
            f32::from(container.width),
            f32::from(container.height),
            f32::from(theme.modal.width_small),
            f32::from(theme.shell.overlay_margin),
        )
    }

    pub(crate) fn laid_out(container_width: f32, container_height: f32, asked_width: f32, margin: f32) -> Self {
        let width = asked_width.min(container_width - margin * 2.0).max(1.0);
        let max_height = (container_height * ModalSheetMetrics::MAX_HEIGHT_FRACTION)
            .min(container_height - margin * 2.0)
            .max(1.0);
        Self { width, max_height }
    }

    /// least height of a sheet whose content fills the surface
    pub fn fill_floor(&self) -> f32 {
        ModalSheetMetrics::FILL_FLOOR_HEIGHT.min(self.max_height)
    }

    /// fixed height for a sheet that cant be sized by its content
    pub fn filled_height(&self, asked: f32) -> f32 {
        asked.max(self.fill_floor()).min(self.max_height)
    }

    /// ceiling of what goes inside the surface
    pub fn content_max_height(&self) -> f32 {
        (self.max_height - ModalSheetMetrics::CONTENT_INSET * 2.0).max(1.0)
    }
}

/// modals stacked in one window
/// two centered modals of one width differ only in height
/// a shorter one on top would show the one under it, so a surface keeps the height of those before it
pub enum ModalSheetStack {}

impl ModalSheetStack {
    pub fn enter(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> ModalSheetStackEntry {
        let window_id = window.window_handle().window_id();
        let lease = window.use_keyed_state(id, cx, move |_, cx| {
            let lease = cx.weak_entity();
            let surfaces = cx.default_global::<ModalSheetSurfaces>();
            surfaces.leases.retain(|(_, held)| held.upgrade().is_some());
            surfaces.leases.push((window_id, lease));
            let frame = Rc::new(Cell::new(Bounds::default()));
            LIVE_SURFACE_FRAMES.with(|live| {
                let mut live = live.borrow_mut();
                live.retain(|held| held.strong_count() > 0);
                live.push(Rc::downgrade(&frame));
            });
            ModalSheetLease {
                presented_at: RiseClock::monotonic(),
                height: 0.0,
                frame,
            }
        });
        let (presented_at, lease_id) = (lease.read(cx).presented_at, lease.entity_id());
        let floor = cx.try_global::<ModalSheetSurfaces>().and_then(|surfaces| {
            surfaces
                .leases
                .iter()
                .filter(|(owner, _)| *owner == window_id)
                .filter_map(|(_, held)| held.upgrade())
                .filter(|held| held.entity_id() != lease_id)
                .map(|held| {
                    let held = held.read(cx);
                    (held.presented_at, held.height)
                })
                .filter(|(held_at, height)| *held_at < presented_at && *height > 0.0)
                .map(|(_, height)| height)
                .reduce(f32::max)
        });
        ModalSheetStackEntry { lease, floor }
    }
}

pub struct ModalSheetStackEntry {
    lease: Entity<ModalSheetLease>,
    pub floor: Option<f32>,
}

impl ModalSheetStackEntry {
    /// layer for the surface children that reports its height to the surfaces above
    pub fn measurer(&self, id: impl Into<ElementId>) -> Canvas<()> {
        let lease = self.lease.downgrade();
        measure(id, move |bounds, _window, cx| {
            let height = f32::from(bounds.size.height) + ModalSheetMetrics::OUTLINE_WIDTH * 2.0;
            let _ = lease.update(cx, |lease, cx| {
                lease.frame.set(bounds.dilate(px(ModalSheetMetrics::OUTLINE_WIDTH)));
                if (lease.height - height).abs() > 0.5 {
                    lease.height = height;
                    cx.notify();
                }
            });
        })
    }

    /// floor raised to the stack floor, within the frame ceiling
    pub fn raised(&self, floor: f32, frame: &ModalSheetFrame) -> f32 {
        self.floor.map_or(floor, |stacked| floor.max(stacked.min(frame.max_height)))
    }
}

/// element state of one drawn surface
struct ModalSheetLease {
    presented_at: Duration,
    height: f32,
    frame: Rc<Cell<Bounds<Pixels>>>,
}

thread_local! {
    static LIVE_SURFACE_FRAMES: RefCell<Vec<Weak<Cell<Bounds<Pixels>>>>> = const { RefCell::new(Vec::new()) };
}

impl ModalSheetStack {
    /// drawn surface rect lies on, the last presented one if several hold it
    /// window coordinates, none when rect is not inside a modal sheet
    pub fn surface_containing(rect: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let centre = rect.center();
        LIVE_SURFACE_FRAMES.with(|live| {
            live.borrow()
                .iter()
                .rev()
                .filter_map(Weak::upgrade)
                .map(|frame| frame.get())
                .find(|frame| frame.size.width > px(0.0) && frame.size.height > px(0.0) && frame.contains(&centre))
        })
    }
}

#[derive(Default)]
struct ModalSheetSurfaces {
    leases: Vec<(WindowId, WeakEntity<ModalSheetLease>)>,
}

impl Global for ModalSheetSurfaces {}

/// layer that centers the surface
/// no listeners, so a click beside the surface reaches the backdrop
pub fn modal_sheet_centering() -> Div {
    div().absolute().inset_0().flex().items_center().justify_center()
}

/// color of the hairline around the dialog
pub fn modal_sheet_outline(theme: &AppTheme) -> Hsla {
    theme.border._200.opacity(ModalSheetMetrics::OUTLINE_OPACITY)
}

/// the dialog outline and appearance, appearance runs from 0 (gone) to 1 (at rest)
pub fn modal_sheet_chrome<E: Styled + IntoElement>(
    surface: E,
    background: Hsla,
    outline: Hsla,
    appearance: f32,
) -> Transitioned {
    let appearance = appearance.clamp(0.0, 1.0);
    surface
        .overflow_hidden()
        .rounded(px(ModalSheetMetrics::CORNER_RADIUS))
        .p(px(ModalSheetMetrics::CONTENT_INSET))
        .bg(background)
        .border_1()
        .border_color(outline)
        .shadow(vec![BoxShadow {
            color: black().opacity(ModalSheetMetrics::SHADOW_OPACITY),
            offset: point(px(0.0), px(ModalSheetMetrics::SHADOW_OFFSET_Y)),
            blur_radius: px(ModalSheetMetrics::SHADOW_RADIUS),
            spread_radius: px(0.0),
            inset: false,
        }])
        .transition_effect(TransitionEffect {
            opacity: appearance,
            scale: ModalSheetMetrics::APPEAR_SCALE + (1.0 - ModalSheetMetrics::APPEAR_SCALE) * appearance,
            ..TransitionEffect::IDENTITY
        })
        .keys_cached_paint()
}

/// dialog frame, clip and appearance without fill
/// hairline and shadow are left out too
/// modal keeps its place while a layer above stands in for it, eg the media editor
pub fn modal_sheet_chrome_bare<E: Styled + IntoElement>(surface: E, appearance: f32) -> Transitioned {
    let appearance = appearance.clamp(0.0, 1.0);
    surface
        .overflow_hidden()
        .rounded(px(ModalSheetMetrics::CORNER_RADIUS))
        .p(px(ModalSheetMetrics::CONTENT_INSET))
        .transition_effect(TransitionEffect {
            opacity: appearance,
            scale: ModalSheetMetrics::APPEAR_SCALE + (1.0 - ModalSheetMetrics::APPEAR_SCALE) * appearance,
            ..TransitionEffect::IDENTITY
        })
        .keys_cached_paint()
}

/// hover of the page under a sheet that just opened
/// a tooltip scheduled before the backdrop painted shows anyway and gpui keeps it
/// a pointer move in place hits the backdrop and the page loses hover
pub fn modal_sheet_settle_pointer(redraw: EntityId, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        cx.notify(redraw);
        window.on_next_frame(|window, cx| {
            if window.last_input_was_keyboard() {
                return;
            }
            let event = MouseMoveEvent {
                position: window.mouse_position(),
                pressed_button: None,
                modifiers: window.modifiers(),
            };
            let _ = window.dispatch_event(PlatformInput::MouseMove(event), cx);
        });
    });
}

/// gives Some(backwards) for the keys that move focus, a shortcut with Tab in it is not one
fn focus_cycle_direction(event: &KeyDownEvent) -> Option<bool> {
    let keystroke = &event.keystroke;
    let modifiers = &keystroke.modifiers;
    if keystroke.key != TAB_KEY || modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
        return None;
    }
    Some(modifiers.shift)
}

/// moves along the window tab stops to the next one inside the sheet, wrapping around
/// with no stop inside the focus rests on the sheet itself so the page under never gets keys
fn walk_into_sheet(backwards: bool, sheet_focus: &FocusHandle, window: &mut Window, cx: &mut App) {
    let mut first_stop: Option<FocusHandle> = None;
    for _ in 0..MAXIMUM_TAB_WALK {
        if backwards {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        let Some(focused) = window.focused(cx) else {
            break;
        };
        if focused != *sheet_focus && sheet_focus.contains_focused(window, cx) {
            return;
        }
        match &first_stop {
            Some(first) if *first == focused => break,
            Some(_) => {}
            None => first_stop = Some(focused),
        }
    }
    window.focus(sheet_focus, cx);
}

/// goes on the sheet root as capture_key_down
/// a text field moves focus by its own binding and a screen may walk its cells
/// so neither is taken over, only brings back a focus that left the sheet
pub fn modal_sheet_keep_focus_inside(event: &KeyDownEvent, sheet_focus: &FocusHandle, window: &mut Window, cx: &mut App) {
    let Some(backwards) = focus_cycle_direction(event) else {
        return;
    };
    let sheet_focus = sheet_focus.clone();
    window.defer(cx, move |window, cx| {
        if !sheet_focus.contains_focused(window, cx) {
            walk_into_sheet(backwards, &sheet_focus, window, cx);
        }
    });
}

/// goes on the sheet root as on_key_down, for Tab nothing inside handled
/// says whether it took the key
pub fn modal_sheet_cycle_focus(event: &KeyDownEvent, sheet_focus: &FocusHandle, window: &mut Window, cx: &mut App) -> bool {
    let Some(backwards) = focus_cycle_direction(event) else {
        return false;
    };
    walk_into_sheet(backwards, sheet_focus, window, cx);
    true
}

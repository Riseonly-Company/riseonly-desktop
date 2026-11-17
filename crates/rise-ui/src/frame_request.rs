use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::{App, AsyncWindowContext, Bounds, EntityId, IntoElement, Pixels, Styled, Window, WindowId, canvas};

/// true when the bounds meet the content mask of every clipping ancestor and the window
pub fn is_painted_on_screen(bounds: Bounds<Pixels>, window: &Window) -> bool {
    let clip = window.content_mask().bounds;
    if !bounds.intersects(&clip) {
        return false;
    }
    let visible = bounds.intersect(&clip);
    visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO
}

/// asks for the next frame when bounds are on screen
/// call it from a prepaint closure, render cant know if the element is drawn at all
#[track_caller]
pub fn request_frame_if_on_screen(bounds: Bounds<Pixels>, window: &Window) {
    if is_shown_on_screen(bounds, window) {
        window.request_animation_frame();
    }
}

/// bounds are on screen and not under an opaque cover
/// for an animation that advances by itself instead of asking for frames
pub fn is_shown_on_screen(bounds: Bounds<Pixels>, window: &Window) -> bool {
    is_painted_on_screen(bounds, window) && !held_under_opaque_cover(bounds, window)
}

pub const CAPPED_TICK_INTERVAL: Duration = Duration::from_millis(33);

thread_local! {
    static PENDING_TICKS: RefCell<HashMap<WindowId, HashSet<EntityId>>> = RefCell::new(HashMap::new());
}

pub fn request_tick_if_on_screen(bounds: Bounds<Pixels>, interval: Duration, window: &Window, cx: &App) {
    if !is_painted_on_screen(bounds, window) || held_under_opaque_cover(bounds, window) {
        return;
    }
    let view = window.current_view();
    let id = window.window_handle().window_id();
    let armed = PENDING_TICKS.with(|pending| {
        let mut pending = pending.borrow_mut();
        let armed = pending.contains_key(&id);
        pending.entry(id).or_default().insert(view);
        armed
    });
    if armed {
        return;
    }
    window
        .spawn(cx, async move |cx: &mut AsyncWindowContext| {
            cx.background_executor().timer(interval).await;
            let views = PENDING_TICKS.with(|pending| pending.borrow_mut().remove(&id)).unwrap_or_default();
            let _ = cx.update(|_, cx| {
                for view in views {
                    cx.notify(view);
                }
            });
        })
        .detach();
}

pub fn frame_request_layer() -> impl IntoElement {
    canvas(
        |bounds, window: &mut Window, _: &mut App| request_frame_if_on_screen(bounds, window),
        |_, (), _: &mut Window, _: &mut App| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// last place and frame an opaque cover was prepainted in, per window
#[derive(Clone, Copy)]
struct OpaqueCover {
    bounds: Bounds<Pixels>,
    prepainted_frame: u64,
}

thread_local! {
    static OPAQUE_COVERS: RefCell<HashMap<WindowId, OpaqueCover>> = RefCell::new(HashMap::new());
    static HELD_UNDER_COVER: RefCell<HashMap<WindowId, HashSet<EntityId>>> = RefCell::new(HashMap::new());
}

/// first child of an opaque full window layer like the media editor
/// what the window draws under it stops asking for frames
/// a spinner or looping lottie there redrew every frame, call release_opaque_cover when it goes
pub fn opaque_cover_layer() -> impl IntoElement {
    canvas(
        |bounds, window: &mut Window, _: &mut App| {
            let cover = OpaqueCover { bounds, prepainted_frame: window.draw_count() };
            OPAQUE_COVERS.with(|covers| covers.borrow_mut().insert(window.window_handle().window_id(), cover));
        },
        |_, (), _: &mut Window, _: &mut App| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// ends the cover of this window and wakes every view that held its frame requests under it
pub fn release_opaque_cover(window: &Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    if OPAQUE_COVERS.with(|covers| covers.borrow_mut().remove(&id)).is_none() {
        return;
    }
    let held = HELD_UNDER_COVER.with(|held| held.borrow_mut().remove(&id)).unwrap_or_default();
    if held.is_empty() {
        return;
    }
    window.defer(cx, move |_, cx| {
        for view in held {
            cx.notify(view);
        }
    });
}

/// element lies inside the cover and was prepainted before it this frame so its drawn under it
/// its view is noted to wake when the cover goes
fn held_under_opaque_cover(bounds: Bounds<Pixels>, window: &Window) -> bool {
    let id = window.window_handle().window_id();
    let Some(cover) = OPAQUE_COVERS.with(|covers| covers.borrow().get(&id).copied()) else {
        return false;
    };
    if cover.prepainted_frame == window.draw_count() {
        return false;
    }
    if cover.prepainted_frame + 1 < window.draw_count() {
        return false;
    }
    let visible = bounds.intersect(&window.content_mask().bounds);
    let (inner, outer) = (visible.bottom_right(), cover.bounds.bottom_right());
    let contained = visible.origin.x >= cover.bounds.origin.x
        && visible.origin.y >= cover.bounds.origin.y
        && inner.x <= outer.x
        && inner.y <= outer.y;
    if !contained {
        return false;
    }
    let view = window.current_view();
    HELD_UNDER_COVER.with(|held| held.borrow_mut().entry(id).or_default().insert(view));
    true
}

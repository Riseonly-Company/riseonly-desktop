use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{
    App, Bounds, DispatchPhase, ElementId, Entity, HitboxBehavior, IntoElement, Pixels, Point, RenderOnce,
    ScrollDelta, ScrollHandle, ScrollWheelEvent, Styled, TouchPhase, Window, canvas, point, px,
};
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthrough;
use rise_ui::pager_back_swipe_navigation_hooks::PagerBackSwipeNavigationHooks;

/// mount it in the background of the first page only
/// plain paging scroller rests at its leading edge on page one so the edge comes from scroll_handle
/// tab view pager keeps its strip centered on every page so the caller says it via is_leading_page
#[derive(IntoElement)]
pub struct PagerBackSwipeYield {
    id: ElementId,
    is_leading_page: Option<bool>,
    scroll_handle: Option<ScrollHandle>,
}

impl PagerBackSwipeYield {
    /// id keys the swipe session across frames
    pub fn new(id: impl Into<ElementId>, is_leading_page: Option<bool>) -> Self {
        Self {
            id: id.into(),
            is_leading_page,
            scroll_handle: None,
        }
    }

    /// scroller of the pager, gpui has no view walk so the pager passes its handle
    /// without it the yield covers its own bounds and with no is_leading_page never claims a swipe
    pub fn scroll_handle(mut self, scroll_handle: ScrollHandle) -> Self {
        self.scroll_handle = Some(scroll_handle);
        self
    }
}

impl PagerBackSwipeYield {
    pub fn rests_at_leading_page(scroll_view: &ScrollHandle) -> Option<bool> {
        let region = scroll_view.bounds();
        ATTACHED.with(|attached| {
            let mut attached = attached.borrow_mut();
            attached.retain(|entry| entry.strong_count() > 0);
            attached
                .iter()
                .filter_map(Weak::upgrade)
                .find(|attachment| attachment.region.get() == region)
                .and_then(|attachment| attachment.is_leading_page.get())
        })
    }
}

/// a horizontal pager that knows if its first page is on screen
/// the tab pager takes a back drag that starts on it there, the one drag it cant page
pub trait LeadingPageReporting {
    fn rests_at_leading_page(&self) -> bool;
}

/// pager a mounted probe sits in
/// found by the scroller laid out bounds since gpui scroll handles have no identity to key on
#[derive(Default)]
struct PagerBackSwipeYieldAttachment {
    region: Cell<Bounds<Pixels>>,
    is_leading_page: Cell<Option<bool>>,
}

thread_local! {
    static ATTACHED: RefCell<Vec<Weak<PagerBackSwipeYieldAttachment>>> = const { RefCell::new(Vec::new()) };
}

fn attach(attachment: &Rc<PagerBackSwipeYieldAttachment>) {
    ATTACHED.with(|attached| {
        let mut attached = attached.borrow_mut();
        let weak = Rc::downgrade(attachment);
        if !attached.iter().any(|entry| entry.ptr_eq(&weak)) {
            attached.push(weak);
        }
    });
}

#[derive(Default)]
struct PagerBackSwipeYieldSession {
    attachment: Rc<PagerBackSwipeYieldAttachment>,
    started: Option<ScrollWheelEvent>,
    translation: Point<Pixels>,
    decided: bool,
    claimed: bool,
}

impl PagerBackSwipeYieldSession {
    fn reset(&mut self) {
        self.started = None;
        self.translation = Point::default();
        self.decided = false;
        self.claimed = false;
    }
}

impl RenderOnce for PagerBackSwipeYield {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let session: Entity<PagerBackSwipeYieldSession> =
            window.use_keyed_state(self.id.clone(), cx, |_, _| PagerBackSwipeYieldSession::default());
        let is_leading_page = self.is_leading_page;
        let scroll_handle = self.scroll_handle;
        let attachment = session.read(cx).attachment.clone();
        canvas(
            {
                let scroll_handle = scroll_handle.clone();
                move |bounds, window, _cx| {
                    let attached = scroll_handle
                        .as_ref()
                        .map(ScrollHandle::bounds)
                        .filter(|region| region.size.width > Pixels::ZERO);
                    if let Some(region) = attached {
                        attachment.region.set(region);
                        attachment.is_leading_page.set(is_leading_page);
                        attach(&attachment);
                    }
                    let region = attached.unwrap_or(bounds);
                    (region, window.insert_hitbox(region, HitboxBehavior::Normal))
                }
            },
            move |_, (region, hitbox), window, _cx| {
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let ScrollDelta::Pixels(delta) = event.delta else {
                        return;
                    };
                    match event.touch_phase {
                        TouchPhase::Started => {
                            if !region.contains(&event.position) || !hitbox.should_handle_scroll(window) {
                                return;
                            }
                            session.update(cx, |session, _| {
                                session.reset();
                                session.translation = delta;
                                session.started = Some(event.clone());
                            });
                        }
                        TouchPhase::Moved => {
                            let (claimed, decide) = session.update(cx, |session, _| {
                                if session.started.is_none() {
                                    return (false, false);
                                }
                                if session.claimed {
                                    return (true, false);
                                }
                                if session.decided {
                                    return (false, false);
                                }
                                session.translation = point(
                                    session.translation.x + delta.x,
                                    session.translation.y + delta.y,
                                );
                                (false, session.translation != Point::default())
                            });
                            if claimed {
                                cx.stop_propagation();
                                drive_pop(event, window, cx);
                                return;
                            }
                            if !decide {
                                return;
                            }
                            let (start, translation) = session.update(cx, |session, _| {
                                session.decided = true;
                                (session.started.clone(), session.translation)
                            });
                            let Some(start) = start else {
                                return;
                            };
                            if !pop_takes_drag(
                                start.position,
                                event.position,
                                translation,
                                is_leading_page,
                                scroll_handle.as_ref(),
                                window,
                                cx,
                            ) {
                                return;
                            }
                            session.update(cx, |session, _| session.claimed = true);
                            cx.stop_propagation();
                            drive_pop(&start, window, cx);
                            drive_pop(event, window, cx);
                        }
                        TouchPhase::Ended | TouchPhase::Cancelled => {
                            let claimed = session.update(cx, |session, _| {
                                let claimed = session.claimed;
                                session.reset();
                                claimed
                            });
                            if claimed {
                                cx.stop_propagation();
                                drive_pop(event, window, cx);
                            }
                        }
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

fn drive_pop(event: &ScrollWheelEvent, window: &mut Window, cx: &mut App) {
    let Some(drive_pop) = PagerBackSwipeNavigationHooks::installed(cx).map(|hooks| hooks.drive_pop.clone()) else {
        return;
    };
    drive_pop(event, window, cx);
}

fn pop_takes_drag(
    start: Point<Pixels>,
    location: Point<Pixels>,
    translation: Point<Pixels>,
    is_leading_page: Option<bool>,
    scroll_handle: Option<&ScrollHandle>,
    window: &Window,
    cx: &App,
) -> bool {
    let Some(hooks) = PagerBackSwipeNavigationHooks::installed(cx) else {
        return false;
    };
    if (hooks.is_media_editor_editing)(cx) || !(hooks.can_pop)(window, cx) {
        return false;
    }

    let is_rtl = (hooks.is_right_to_left)(cx);
    let reading_x = if is_rtl { -translation.x } else { translation.x };
    if reading_x <= Pixels::ZERO || translation.x.abs() <= translation.y.abs() {
        return false;
    }

    if BubbleScrollPassthrough::with_shared(|passthrough| passthrough.contains(location))
        || (hooks.blocks_interactive_pop)(start, window, cx)
    {
        return false;
    }

    if let Some(is_leading_page) = is_leading_page {
        return is_leading_page;
    }

    let Some(scroll_handle) = scroll_handle else {
        return false;
    };
    let local = translation.x;
    let offset = scroll_handle.offset().x;
    let min_offset = -scroll_handle.max_offset().x.max(Pixels::ZERO);
    if local > Pixels::ZERO {
        return offset >= -px(0.5);
    }
    if local < Pixels::ZERO {
        return offset <= min_offset + px(0.5);
    }
    false
}


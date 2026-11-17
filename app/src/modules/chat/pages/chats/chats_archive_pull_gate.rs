use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, DispatchPhase, Entity, Hitbox, HitboxBehavior, IntoElement, Point,
    RenderOnce, ScrollDelta, ScrollWheelEvent, Styled, Task, TouchPhase, WeakEntity, Window, canvas, point, px,
};

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::chat::pages::chats::chats_archive_reveal::ChatsArchiveRevealPolicy;

const WHEEL_LINE_STEP: f32 = 40.0;

/// pan lifecycle of the chat list from the list that owns the vertical axis
/// a trackpad touch always ends or cancels, a notched wheel has no release
/// so the pause after its last notch ends the pan
#[derive(Clone)]
pub struct ChatsArchivePullEvents {
    pub allows_exposure: Rc<dyn Fn(&App) -> bool>,
    pub on_began: Rc<dyn Fn(f32, &mut App)>,
    pub on_changed: Rc<dyn Fn(Point<f32>, &mut App)>,
    pub on_ended: Rc<dyn Fn(f32, f32, &mut App) -> Option<f32>>,
    pub on_rested_over_row: Rc<dyn Fn(f32, &mut App) -> Option<f32>>,
}

pub struct ChatsArchivePullGateCoordinator {
    manual_list: Option<WeakEntity<RiseManualListView>>,
    events: ChatsArchivePullEvents,
    is_enabled: bool,
    is_tracking: bool,
    rest_check_generation: u64,
    settle_generation: u64,
    pan_start_marker_y: Option<f32>,
    translation_y: f32,
    has_seen_touch_lifecycle: bool,
    is_wheel_pan: bool,
    rest_check_task: Option<Task<()>>,
    wheel_pause_task: Option<Task<()>>,
}

impl ChatsArchivePullGateCoordinator {
    pub fn new(events: ChatsArchivePullEvents, is_enabled: bool) -> Self {
        Self {
            manual_list: None,
            events,
            is_enabled,
            is_tracking: false,
            rest_check_generation: 0,
            settle_generation: 0,
            pan_start_marker_y: None,
            translation_y: 0.0,
            has_seen_touch_lifecycle: false,
            is_wheel_pan: false,
            rest_check_task: None,
            wheel_pause_task: None,
        }
    }

    pub fn update(&mut self, events: ChatsArchivePullEvents, is_enabled: bool, cx: &mut Context<Self>) {
        let became_enabled = is_enabled && !self.is_enabled;
        self.events = events;
        self.is_enabled = is_enabled;
        if !is_enabled {
            self.pan_start_marker_y = None;
            self.settle_generation = self.settle_generation.wrapping_add(1);
        }
        if became_enabled {
            self.scroll_state_did_change(cx);
        }
    }

    pub fn attach(&mut self, list: WeakEntity<RiseManualListView>, cx: &mut Context<Self>) {
        if self.manual_list.as_ref() == Some(&list) {
            return;
        }
        self.detach();
        self.manual_list = Some(list);
        self.schedule_rest_check(cx);
    }

    /// offset observation, the list reports every metrics change here
    /// one turn later since it is being updated while it reports
    pub fn scroll_state_did_change(&mut self, cx: &mut Context<Self>) {
        if self.manual_list.is_some() {
            self.schedule_rest_check(cx);
        }
    }

    fn list(&self) -> Option<Entity<RiseManualListView>> {
        self.manual_list.as_ref().and_then(WeakEntity::upgrade)
    }

    fn marker_y(&self, cx: &App) -> Option<f32> {
        let list = self.list()?;
        let list = list.read(cx);
        Some(
            ChatsArchiveRevealPolicy::ROW_HEIGHT + f32::from(list.content_top_offset_y())
                - f32::from(list.integration_scroll_view().content_offset.y),
        )
    }

    fn schedule_rest_check(&mut self, cx: &mut Context<Self>) {
        self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
        self.rest_check_task = None;
        let Some(marker_y) = self.marker_y(cx) else {
            return;
        };
        let Some(list) = self.list() else {
            return;
        };
        let scroll_view = list.read(cx).integration_scroll_view();
        if !scroll_view.is_tracking && !scroll_view.is_dragging {
            self.pan_start_marker_y = Some(marker_y);
        }
        if !self.is_enabled
            || (marker_y.abs() > ChatsArchiveRevealPolicy::TOP_TOLERANCE
                && !ChatsArchiveRevealPolicy::rests_over_row(marker_y))
        {
            return;
        }
        let generation = self.rest_check_generation;
        self.rest_check_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(ChatsArchiveRevealPolicy::REST_CHECK_DELAY))
                .await;
            let _ = this.update(cx, |coordinator, cx| {
                if coordinator.rest_check_generation == generation {
                    coordinator.settle_if_resting_over_row(cx);
                }
            });
        }));
    }

    fn settle_if_resting_over_row(&mut self, cx: &mut Context<Self>) {
        if !self.is_enabled {
            return;
        }
        let Some(list) = self.list() else {
            return;
        };
        let is_moving = {
            let list = list.read(cx);
            let scroll_view = list.integration_scroll_view();
            scroll_view.is_tracking
                || scroll_view.is_dragging
                || scroll_view.is_decelerating
                || list.is_scroll_in_progress()
        };
        if is_moving {
            self.schedule_rest_check(cx);
            return;
        }
        let Some(marker_y) = self.marker_y(cx) else {
            return;
        };
        if !ChatsArchiveRevealPolicy::rests_over_row(marker_y) {
            return;
        }
        let on_rested_over_row = self.events.on_rested_over_row.clone();
        let Some(delta) = on_rested_over_row(marker_y, cx) else {
            self.schedule_rest_check(cx);
            return;
        };
        self.settle(delta, cx);
    }

    pub fn detach(&mut self) {
        self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
        self.settle_generation = self.settle_generation.wrapping_add(1);
        self.pan_start_marker_y = None;
        self.manual_list = None;
        self.is_tracking = false;
        self.is_wheel_pan = false;
        self.rest_check_task = None;
        self.wheel_pause_task = None;
    }

    pub fn handle_pan(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        if self.manual_list.is_none() {
            return;
        }
        let delta_y = f32::from(event.delta.pixel_delta(px(WHEEL_LINE_STEP)).y);
        match event.touch_phase {
            TouchPhase::Started => {
                self.has_seen_touch_lifecycle = true;
                self.is_wheel_pan = false;
                self.wheel_pause_task = None;
                self.began(cx);
                self.changed(delta_y, cx);
            }
            TouchPhase::Moved => {
                if self.is_tracking && !self.is_wheel_pan {
                    self.changed(delta_y, cx);
                    return;
                }
                let is_notched = matches!(event.delta, ScrollDelta::Lines(_));
                if !is_notched && self.has_seen_touch_lifecycle {
                    return;
                }
                if !self.is_wheel_pan {
                    self.is_wheel_pan = true;
                    self.began(cx);
                }
                self.changed(delta_y, cx);
                self.schedule_wheel_pause_end(cx);
            }
            TouchPhase::Ended => {
                let velocity_y = self.release_velocity_y(cx);
                self.ended(velocity_y, cx);
            }
            TouchPhase::Cancelled => self.ended(0.0, cx),
        }
    }

    fn release_velocity_y(&self, cx: &App) -> f32 {
        self.list().map_or(0.0, |list| -f32::from(list.read(cx).scroll_metrics().velocity_y))
    }

    fn began(&mut self, cx: &mut Context<Self>) {
        self.settle_generation = self.settle_generation.wrapping_add(1);
        self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
        self.rest_check_task = None;
        let start_marker = self.pan_start_marker_y.take().or_else(|| self.marker_y(cx));
        self.translation_y = 0.0;
        self.is_tracking = self.is_enabled;
        if !self.is_tracking {
            return;
        }
        let Some(start_marker) = start_marker else {
            return;
        };
        let on_began = self.events.on_began.clone();
        on_began(start_marker, cx);
    }

    fn changed(&mut self, delta_y: f32, cx: &mut Context<Self>) {
        if !self.is_tracking {
            return;
        }
        self.translation_y += delta_y;
        let on_changed = self.events.on_changed.clone();
        on_changed(point(0.0, self.translation_y), cx);
    }

    fn schedule_wheel_pause_end(&mut self, cx: &mut Context<Self>) {
        self.wheel_pause_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(ChatsArchiveRevealPolicy::REST_CHECK_DELAY))
                .await;
            let _ = this.update(cx, |coordinator, cx| {
                coordinator.is_wheel_pan = false;
                coordinator.wheel_pause_task = None;
                coordinator.ended(0.0, cx);
            });
        }));
    }

    fn ended(&mut self, velocity_y: f32, cx: &mut Context<Self>) {
        if !self.is_tracking {
            return;
        }
        self.is_tracking = false;
        if let Some(marker_y) = self.marker_y(cx) {
            let on_ended = self.events.on_ended.clone();
            if let Some(delta) = on_ended(marker_y, velocity_y, cx) {
                self.settle(delta, cx);
            }
        }
        self.schedule_rest_check(cx);
    }

    fn settle(&mut self, delta: f32, cx: &mut Context<Self>) {
        if delta.abs() <= ChatsArchiveRevealPolicy::MARKER_EPSILON {
            return;
        }
        let Some(list) = self.list() else {
            return;
        };
        let target = {
            let list = list.read(cx);
            f32::from(list.integration_scroll_view().content_offset.y) - f32::from(list.content_top_offset_y()) + delta
        };
        self.settle_generation = self.settle_generation.wrapping_add(1);
        let generation = self.settle_generation;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |coordinator, cx| {
                if !coordinator.is_enabled || coordinator.settle_generation != generation {
                    return;
                }
                let Some(list) = coordinator.list() else {
                    return;
                };
                let window_handle = list.read(cx).window_handle();
                let _ = window_handle.update(cx, |_, window, cx| {
                    list.update(cx, |list, cx| list.settle_content_offset_from_top(px(target), window, cx));
                });
            });
        })
        .detach();
    }
}

/// mounted behind the list, hands the wheel to the coordinator
/// the scroller is known directly so no probe walks the view tree
#[derive(IntoElement)]
pub struct ChatsArchivePullGate {
    coordinator: Entity<ChatsArchivePullGateCoordinator>,
}

impl ChatsArchivePullGate {
    pub fn new(coordinator: Entity<ChatsArchivePullGateCoordinator>) -> Self {
        Self { coordinator }
    }
}

impl RenderOnce for ChatsArchivePullGate {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let coordinator = self.coordinator;
        canvas(
            |bounds, window, _cx| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox: Hitbox, window, _cx| {
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture
                        || !bounds.contains(&event.position)
                        || !hitbox.should_handle_scroll(window)
                    {
                        return;
                    }
                    coordinator.update(cx, |coordinator, cx| coordinator.handle_pan(event, cx));
                });
            },
        )
        .absolute()
        .inset_0()
    }
}

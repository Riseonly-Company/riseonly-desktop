use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, Entity, IntoElement, Render, RenderOnce, Subscription, Task, Window, div,
    prelude::*, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue, GestureMask};
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{Spring, spring};

use crate::app::router::navigation_types::CreatePostIntent;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::camera::capture::rise_camera_presenter::RiseCameraPresenter;
use crate::modules::camera::components::feed_media_picker_page_hooks::feed_media_picker_page_hooks;

const SETTLE_RESPONSE: f64 = 0.34;
const SETTLE_DAMPING_FRACTION: f64 = 0.92;
const CLOSE_DRAG_MINIMUM_DISTANCE: f32 = 14.0;
const CLOSE_DRAG_AXIS_RATIO: f32 = 1.1;
const RELEASE_VELOCITY_FACTOR: f32 = 4.0;

/// media picker the feed swipe pulls in after the finger, grid with the live camera as first tile
/// only the host reads progress so a drag frame doesnt rerender the grid
pub struct FeedMediaPickerReveal {
    progress: f32,
    presented_progress: Animated<f32>,
    is_dragging: bool,
    is_mounted: bool,
    settle_generation: u64,
    settle_task: Option<Task<()>>,
}

impl FeedMediaPickerReveal {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            progress: 0.0,
            presented_progress: Animated::new(0.0),
            is_dragging: false,
            is_mounted: false,
            settle_generation: 0,
            settle_task: None,
        }
    }

    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// value on screen this frame, frames are asked for only while the settle spring runs
    pub fn presented_progress(&self, window: &Window) -> f32 {
        self.presented_progress.frame_value(RiseClock::monotonic(), window)
    }

    pub fn is_dragging(&self) -> bool {
        self.is_dragging
    }

    pub fn is_mounted(&self) -> bool {
        self.is_mounted
    }

    pub fn is_open(&self) -> bool {
        !self.is_dragging && self.progress >= 0.999
    }

    pub fn feed_drag_changed(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if !self.is_dragging {
            self.is_dragging = true;
            self.is_mounted = true;
            self.settle_generation = self.settle_generation.wrapping_add(1);
        }
        self.set_progress_now(fraction.clamp(0.0, 1.0));
        cx.notify();
    }

    pub fn feed_drag_ended(&mut self, fraction: f32, velocity: f32, cx: &mut Context<Self>) {
        if !self.is_dragging {
            return;
        }
        self.is_dragging = false;
        let opens = (fraction > RiseCameraPresenter::OPEN_THRESHOLD && velocity > -RiseCameraPresenter::FLING_VELOCITY)
            || (velocity > RiseCameraPresenter::FLING_VELOCITY && fraction > 0.0);
        self.settle(opens, cx);
    }

    pub fn feed_drag_cancelled(&mut self, cx: &mut Context<Self>) {
        if !self.is_dragging {
            return;
        }
        self.is_dragging = false;
        self.settle(false, cx);
    }

    pub fn close_drag_changed(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if !self.is_mounted {
            return;
        }
        if !self.is_dragging {
            self.is_dragging = true;
            self.settle_generation = self.settle_generation.wrapping_add(1);
        }
        self.set_progress_now(1.0 - fraction.clamp(0.0, 1.0));
        cx.notify();
    }

    pub fn close_drag_ended(&mut self, fraction: f32, velocity: f32, cx: &mut Context<Self>) {
        if !self.is_dragging {
            return;
        }
        self.is_dragging = false;
        let closes = (fraction > RiseCameraPresenter::OPEN_THRESHOLD && velocity < RiseCameraPresenter::FLING_VELOCITY)
            || velocity < -RiseCameraPresenter::FLING_VELOCITY;
        self.settle(!closes, cx);
    }

    /// animated is false when a pushed screen already covers the page
    pub fn close(&mut self, animated: bool, cx: &mut Context<Self>) {
        if !self.is_mounted {
            return;
        }
        self.is_dragging = false;
        if !animated {
            self.settle_generation = self.settle_generation.wrapping_add(1);
            self.settle_task = None;
            self.set_progress_now(0.0);
            self.is_mounted = false;
            cx.notify();
            return;
        }
        self.settle(false, cx);
    }

    fn set_progress_now(&mut self, progress: f32) {
        self.progress = progress;
        self.presented_progress.jump(progress);
    }

    fn settle(&mut self, open: bool, cx: &mut Context<Self>) {
        self.settle_generation = self.settle_generation.wrapping_add(1);
        let generation = self.settle_generation;
        self.progress = if open { 1.0 } else { 0.0 };
        self.presented_progress.set_in(
            self.progress,
            spring(SETTLE_RESPONSE, SETTLE_DAMPING_FRACTION),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
        let settles_in = Spring::new(SETTLE_RESPONSE, SETTLE_DAMPING_FRACTION).settling_duration();
        self.settle_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(settles_in).await;
            let _ = this.update(cx, |this, cx| {
                if open || this.settle_generation != generation || this.is_dragging {
                    return;
                }
                this.is_mounted = false;
                cx.notify();
            });
        }));
    }
}

pub struct FeedMediaPickerHost {
    reveal: Entity<FeedMediaPickerReveal>,
    page: Option<AnyView>,
    close_drag_axis: Rc<Cell<Option<bool>>>,
    width: Rc<Cell<f32>>,
    _reveal_observation: Subscription,
}

impl FeedMediaPickerHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let reveal = FeedMediaPickerReveal::shared(cx);
        let reveal_observation = cx.observe(&reveal, |_, _, cx| cx.notify());
        Self {
            reveal,
            page: None,
            close_drag_axis: Rc::new(Cell::new(None)),
            width: Rc::new(Cell::new(1.0)),
            _reveal_observation: reveal_observation,
        }
    }

    fn close_drag(&self) -> DragGesture {
        let axis = self.close_drag_axis.clone();
        let ended_axis = self.close_drag_axis.clone();
        let width = self.width.clone();
        let ended_width = self.width.clone();
        DragGesture::new(CLOSE_DRAG_MINIMUM_DISTANCE)
            .on_changed(move |value: &DragGestureValue, _, cx| {
                let reveal = FeedMediaPickerReveal::shared(cx);
                let (dx, dy) = (f32::from(value.translation.width), f32::from(value.translation.height));
                if axis.get().is_none() {
                    axis.set(Some(reveal.read(cx).is_open() && dx.abs() > dy.abs() * CLOSE_DRAG_AXIS_RATIO && dx < 0.0));
                }
                if axis.get() != Some(true) {
                    return;
                }
                let fraction = -dx / width.get();
                reveal.update(cx, |reveal, cx| reveal.close_drag_changed(fraction, cx));
            })
            .on_ended(move |value: &DragGestureValue, _, cx| {
                let took = ended_axis.take() == Some(true);
                if !took {
                    return;
                }
                let dx = f32::from(value.translation.width);
                let velocity = (f32::from(value.predicted_end_translation.width) - dx) * RELEASE_VELOCITY_FACTOR;
                let fraction = -dx / ended_width.get();
                FeedMediaPickerReveal::shared(cx)
                    .update(cx, |reveal, cx| reveal.close_drag_ended(fraction, velocity, cx));
            })
    }
}

impl Render for FeedMediaPickerHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width_cell = self.width.clone();
        let host = div().absolute().inset_0().child(measure("feed-media-picker-host-size", move |bounds, window, _| {
            let width = f32::from(bounds.size.width).max(1.0);
            if width_cell.get() != width {
                width_cell.set(width);
                window.refresh();
            }
        }));
        let (is_mounted, progress, takes_clicks) = {
            let reveal = self.reveal.read(cx);
            (reveal.is_mounted(), reveal.presented_progress(window), reveal.is_open() || reveal.is_dragging())
        };
        if !is_mounted {
            self.page = None;
            return host.into_any_element();
        }
        if self.page.is_none() {
            self.page = feed_media_picker_page_hooks()
                .map(|hooks| (hooks.create_post_media_page)(CreatePostIntent::Publication, None, None, cx));
        }
        let Some(page) = self.page.clone() else {
            return host.into_any_element();
        };
        let actions = CreatePostMediaHostActions {
            on_dismiss: Rc::new(|_, cx| {
                FeedMediaPickerReveal::shared(cx).update(cx, |reveal, cx| reveal.close(true, cx));
            }),
            on_details_pushed: Rc::new(|_, cx| {
                FeedMediaPickerReveal::shared(cx).update(cx, |reveal, cx| reveal.close(false, cx));
            }),
        };
        let offset = (progress - 1.0) * self.width.get();
        let page_layer = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(offset))
            .w(px(self.width.get()))
            .child(env_scope::<CreatePostMediaHostKey>(Some(actions)).child(page))
            .children((!takes_clicks).then(|| {
                div().id("feed-media-picker-settling").absolute().inset_0().occlude()
            }))
            .occludes_bottom_chrome();
        host.child(page_layer)
            .simultaneous_gesture("feed-media-picker-close-drag", self.close_drag(), GestureMask::All)
            .into_any_element()
    }
}

/// moves the app screen with the feed picker, they stand side by side and travel together
/// only this wrapper reads progress so a drag frame moves one layer
#[derive(IntoElement)]
pub struct FeedMediaPickerRevealShift {
    content: AnyElement,
}

impl RenderOnce for FeedMediaPickerRevealShift {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = window
            .use_keyed_state("feed-media-picker-reveal-shift-width", cx, |_, _| Rc::new(Cell::new(0.0_f32)))
            .read(cx)
            .clone();
        let reveal = FeedMediaPickerReveal::shared(cx);
        let reveal = reveal.read(cx);
        let progress = if reveal.is_mounted() { reveal.presented_progress(window) } else { 0.0 };
        let measured = width.clone();
        div()
            .relative()
            .size_full()
            .left(px(progress * width.get()))
            .child(self.content)
            .child(measure("feed-media-picker-reveal-shift-size", move |bounds, window, _| {
                let width = f32::from(bounds.size.width);
                if measured.get() != width {
                    measured.set(width);
                    window.refresh();
                }
            }))
    }
}

pub trait FeedMediaPickerRevealShiftModifiers: IntoElement + Sized {
    fn feed_media_picker_reveal_shift(self) -> FeedMediaPickerRevealShift {
        FeedMediaPickerRevealShift { content: self.into_any_element() }
    }
}

impl<E: IntoElement> FeedMediaPickerRevealShiftModifiers for E {}

/// picker page mounted outside the navigation stack
/// cancel and the details push go to its host instead of popping the stack
#[derive(Clone)]
pub struct CreatePostMediaHostActions {
    pub on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_details_pushed: Rc<dyn Fn(&mut Window, &mut App)>,
}

pub struct CreatePostMediaHostKey;

impl EnvKey for CreatePostMediaHostKey {
    type Value = Option<CreatePostMediaHostActions>;

    fn default_value() -> Self::Value {
        None
    }
}

/// create post media host from the environment, read only in render
pub fn create_post_media_host(cx: &App) -> Option<CreatePostMediaHostActions> {
    env::<CreatePostMediaHostKey>(cx)
}

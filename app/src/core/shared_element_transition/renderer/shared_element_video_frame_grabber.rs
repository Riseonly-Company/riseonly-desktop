use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{
    App, AppContext as _, Bounds, ElementId, Entity, IntoElement, Pixels, RenderImage, Styled, Task, WeakEntity,
    canvas, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_media::video::av_player::{AVLayerVideoGravity, AVPlayer, AVPlayerItemStatus, AVPlayerPixelBuffer};

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementStopFrameRequest, SharedElementStopFrameResult,
};

/// polled on every step of a grab, false settles the request as missed
pub type SharedElementVideoFrameEpochGuard = Rc<dyn Fn(&App) -> bool>;

struct SharedElementPlayerLayerViewInner {
    player: WeakEntity<AVPlayer>,
    video_gravity: AVLayerVideoGravity,
    frame: Cell<Option<Bounds<Pixels>>>,
    is_hidden: Cell<bool>,
}

/// desktop stand in for a view whose layer is a player layer
#[derive(Clone)]
pub struct SharedElementPlayerLayerView {
    inner: Rc<SharedElementPlayerLayerViewInner>,
}

impl SharedElementPlayerLayerView {
    /// true only for the same published layer, an equal one doesnt count
    pub fn same_layer(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn new(player: &Entity<AVPlayer>, video_gravity: AVLayerVideoGravity) -> Self {
        Self {
            inner: Rc::new(SharedElementPlayerLayerViewInner {
                player: player.downgrade(),
                video_gravity,
                frame: Cell::new(None),
                is_hidden: Cell::new(false),
            }),
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        self.inner.is_hidden.set(hidden);
    }

    pub fn is_hidden(&self) -> bool {
        self.inner.is_hidden.get()
    }

    /// layer frame in window space, none while its not in a window
    pub fn frame(&self) -> Option<Bounds<Pixels>> {
        self.inner.frame.get()
    }

    pub fn player(&self) -> Option<Entity<AVPlayer>> {
        self.inner.player.upgrade()
    }

    /// zero hit test element that fills the player layer frame, draw it inside that frame
    pub fn probe(&self, id: impl Into<ElementId>) -> impl IntoElement {
        let id: ElementId = id.into();
        let view = self.clone();
        canvas(
            move |bounds, window, _| {
                window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<SharedElementPlayerLayerAttachment, _>(global_id, |attachment, window| {
                        let attachment = match attachment {
                            Some(attachment) if Rc::ptr_eq(&attachment.view, &view.inner) => attachment,
                            _ => SharedElementPlayerLayerAttachment {
                                view: view.inner.clone(),
                            },
                        };
                        view.inner.frame.set(Some(bounds));
                        let moved = view.inner.clone();
                        window.on_replayed_move(move |offset, _, _, _| moved.frame.set(Some(bounds + offset)));
                        ((), attachment)
                    })
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// where the picture sits in the layer frame, none while the presentation size is unknown
    fn video_rect(&self, frame: Bounds<Pixels>, cx: &App) -> Option<Bounds<Pixels>> {
        let presentation = self.player()?.read(cx).presentation_size();
        if presentation.width <= 0.0 || presentation.height <= 0.0 {
            return None;
        }
        let width = f32::from(frame.size.width);
        let height = f32::from(frame.size.height);
        let scale = match self.inner.video_gravity {
            AVLayerVideoGravity::ResizeAspect => (width / presentation.width).min(height / presentation.height),
            AVLayerVideoGravity::ResizeAspectFill => (width / presentation.width).max(height / presentation.height),
            AVLayerVideoGravity::Resize => return Some(frame),
        };
        let video_width = presentation.width * scale;
        let video_height = presentation.height * scale;
        Some(Bounds::new(
            point(
                frame.origin.x + px((width - video_width) / 2.0),
                frame.origin.y + px((height - video_height) / 2.0),
            ),
            size(px(video_width), px(video_height)),
        ))
    }
}

/// element state of a drawn probe, gpui drops it after the first frame that skips the probe
/// meaning the layer left the window
struct SharedElementPlayerLayerAttachment {
    view: Rc<SharedElementPlayerLayerViewInner>,
}

impl Drop for SharedElementPlayerLayerAttachment {
    fn drop(&mut self) {
        self.view.frame.set(None);
    }
}

thread_local! {
    static ACTIVE: RefCell<HashMap<usize, Rc<SharedElementVideoFrameGrabber>>> = RefCell::new(HashMap::new());
}

/// grabs the frame a playing inline reel or carousel video shows at exit
/// player is never paused, resumed or muted
/// if the player stops, is swapped or leaves the window first the request settles as missed
pub struct SharedElementVideoFrameGrabber {
    pub request: Rc<SharedElementStopFrameRequest>,
    item_url: Url,
    player: WeakEntity<AVPlayer>,
    player_layer_view: Weak<SharedElementPlayerLayerViewInner>,
    content_rect: Bounds<Pixels>,
    epoch_guard: SharedElementVideoFrameEpochGuard,
    conversion_task: RefCell<Option<Task<()>>>,
    is_finished: Cell<bool>,
}

impl SharedElementVideoFrameGrabber {
    const MINIMUM_COVERAGE: f32 = 0.5;
    const TICK: Duration = Duration::from_micros(8_333);

    /// first view in root that is not hidden, is in a window and covers at least half of screen_rect
    pub fn player_layer_view(
        root: &[SharedElementPlayerLayerView],
        covering: Bounds<Pixels>,
    ) -> Option<SharedElementPlayerLayerView> {
        let target_area = Self::area(covering);
        if target_area <= 0.0 {
            return None;
        }
        root.iter()
            .find(|view| {
                if view.is_hidden() {
                    return false;
                }
                let Some(frame) = view.frame() else {
                    return false;
                };
                if !frame.intersects(&covering) {
                    return false;
                }
                Self::area(frame.intersect(&covering)) >= target_area * Self::MINIMUM_COVERAGE
            })
            .cloned()
    }

    /// true when playing and showing a decoded frame
    /// a layer not ready for display is still under the poster cover
    pub fn is_playing(player_layer_view: &SharedElementPlayerLayerView, cx: &App) -> bool {
        let Some(player) = player_layer_view.player() else {
            return false;
        };
        let player = player.read(cx);
        player.current_item().is_some()
            && player.rate() > 0.0
            && player.current_item_status() == AVPlayerItemStatus::ReadyToPlay
            && player.is_ready_for_display()
    }

    pub fn make_stop_frame_request(
        player_layer_view: &SharedElementPlayerLayerView,
        epoch_guard: SharedElementVideoFrameEpochGuard,
        cx: &mut App,
    ) -> Option<Rc<SharedElementStopFrameRequest>> {
        if !epoch_guard(cx) || !Self::is_playing(player_layer_view, cx) {
            return None;
        }
        let frame = player_layer_view.frame()?;
        let player = player_layer_view.player()?;
        let item_url = player.read(cx).current_item()?.url().clone();
        let content_rect = player_layer_view
            .video_rect(frame, cx)
            .filter(|rect| rect.size.width > px(0.0) && rect.size.height > px(0.0))
            .unwrap_or(frame);
        let grabber = Rc::new(Self {
            request: Rc::new(SharedElementStopFrameRequest::new(RiseClock::monotonic(), None)),
            item_url,
            player: player.downgrade(),
            player_layer_view: Rc::downgrade(&player_layer_view.inner),
            content_rect,
            epoch_guard,
            conversion_task: RefCell::new(None),
            is_finished: Cell::new(false),
        });
        Self::start(&grabber, cx);
        Some(grabber.request.clone())
    }

    fn start(this: &Rc<Self>, cx: &mut App) {
        ACTIVE.with(|active| active.borrow_mut().insert(Self::identifier(this), this.clone()));
        let weak = Rc::downgrade(this);
        this.request.set_cancellation_handler(Some(Box::new(move || {
            if let Some(grabber) = weak.upgrade() {
                grabber.finish(None);
            }
        })));

        let weak = Rc::downgrade(this);
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Self::TICK).await;
                let Some(grabber) = weak.upgrade() else {
                    return;
                };
                let keeps_polling = cx.update(|cx| grabber.display_link_did_fire(cx));
                if !keeps_polling {
                    return;
                }
            }
        })
        .detach();

        let weak = Rc::downgrade(this);
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(SharedElementMotion::STOP_FRAME_BUDGET))
                .await;
            if let Some(grabber) = weak.upgrade() {
                grabber.finish(None);
            }
        })
        .detach();
    }

    fn display_link_did_fire(self: &Rc<Self>, cx: &mut App) -> bool {
        if self.is_finished.get() {
            return false;
        }
        if !(self.epoch_guard)(cx)
            || self
                .request
                .is_expired(RiseClock::monotonic(), SharedElementMotion::STOP_FRAME_BUDGET)
        {
            self.finish(None);
            return false;
        }
        if let Some(player) = self.player.upgrade()
            && let Some(buffer) = player.update(cx, |player, _| player.copy_pixel_buffer_for_current_time())
        {
            self.start_conversion(buffer, cx);
            return false;
        }
        if !self.is_source_live(cx) {
            self.finish(None);
            return false;
        }
        true
    }

    fn is_source_live(&self, cx: &App) -> bool {
        let Some(view) = self.player_layer_view.upgrade() else {
            return false;
        };
        if view.frame.get().is_none() || view.player.entity_id() != self.player.entity_id() {
            return false;
        }
        let Some(player) = self.player.upgrade() else {
            return false;
        };
        let player = player.read(cx);
        let same_item = player
            .current_item()
            .is_some_and(|item| *item.url() == self.item_url);
        same_item && player.rate() > 0.0
    }

    fn start_conversion(self: &Rc<Self>, buffer: AVPlayerPixelBuffer, cx: &mut App) {
        let (sender, receiver) = oneshot::channel::<Option<RenderImage>>();
        let conversion = cx.background_spawn(async move {
            let _ = sender.send(buffer.oriented_image());
        });
        *self.conversion_task.borrow_mut() = Some(conversion);
        let weak = Rc::downgrade(self);
        cx.spawn(async move |cx| {
            let image = receiver.await.ok().flatten();
            let Some(grabber) = weak.upgrade() else {
                return;
            };
            cx.update(|cx| grabber.conversion_did_finish(image, cx));
        })
        .detach();
    }

    fn conversion_did_finish(&self, image: Option<RenderImage>, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        let Some(image) = image else {
            self.finish(None);
            return;
        };
        let pixel_size = image.size(0);
        if pixel_size.width.0 <= 0 || pixel_size.height.0 <= 0 || !(self.epoch_guard)(cx) {
            self.finish(None);
            return;
        }
        let aspect = pixel_size.width.0 as f32 / pixel_size.height.0 as f32;
        self.finish(Some(SharedElementStopFrameResult {
            image: Arc::new(image),
            aspect,
            content_rect: self.content_rect,
        }));
    }

    fn finish(&self, result: Option<SharedElementStopFrameResult>) {
        if self.is_finished.get() {
            return;
        }
        self.is_finished.set(true);
        let conversion = self.conversion_task.borrow_mut().take();
        drop(conversion);
        match result {
            Some(result) => self.request.resolve(result),
            None => self.request.miss(),
        }
        let identifier = self as *const Self as usize;
        let removed = ACTIVE.with(|active| active.borrow_mut().remove(&identifier));
        drop(removed);
    }

    fn identifier(this: &Rc<Self>) -> usize {
        Rc::as_ptr(this) as usize
    }

    fn area(bounds: Bounds<Pixels>) -> f32 {
        f32::from(bounds.size.width) * f32::from(bounds.size.height)
    }
}

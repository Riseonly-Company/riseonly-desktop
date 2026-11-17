use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AsyncApp, BorrowAppContext, Context, Entity, EntityId, Global, RenderImage, Task};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;

use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::{RiseLottieFrameSequence, RiseLottieRasterPolicy};
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottiePreparationCancellation;

pub trait RiseLottiePlaybackTarget: Sized + 'static {
    fn playback_sequence(&self) -> Option<Arc<RiseLottieFrameSequence>>;
    fn present_frame(&mut self, image: Arc<RenderImage>, index: i64, cx: &mut Context<Self>);
    fn playback_did_finish_one_shot(&mut self, cx: &mut Context<Self>);
}

struct RiseLottieDisplayDriverPendingFrame {
    index: i64,
    lock: Mutex<(bool, Option<Arc<RenderImage>>)>,
}

impl RiseLottieDisplayDriverPendingFrame {
    fn new(index: i64) -> Self {
        Self {
            index,
            lock: Mutex::new((false, None)),
        }
    }

    fn resolve(&self, image: Option<Arc<RenderImage>>) {
        *self.lock.lock() = (true, image);
    }

    fn result(&self) -> (bool, Option<Arc<RenderImage>>) {
        let state = self.lock.lock();
        (state.0, state.1.clone())
    }
}

struct RiseLottieDisplayDriverRegistration {
    playback_sequence: Box<dyn Fn(&App) -> Option<Option<Arc<RiseLottieFrameSequence>>>>,
    present_frame: Rc<dyn Fn(Arc<RenderImage>, i64, &mut App)>,
    playback_did_finish_one_shot: Rc<dyn Fn(&mut App)>,
    loops: bool,
    cancellation: Arc<RiseLottiePreparationCancellation>,
    started_at: f64,
    last_presented_index: Option<i64>,
    pending_frame: Option<Arc<RiseLottieDisplayDriverPendingFrame>>,
}

impl RiseLottieDisplayDriverRegistration {
    fn new<T: RiseLottiePlaybackTarget>(target: &Entity<T>, loops: bool, started_at: f64, initial_index: i64) -> Self {
        let sequence_target = target.downgrade();
        let present_target = target.downgrade();
        let finish_target = target.downgrade();
        Self {
            playback_sequence: Box::new(move |cx: &App| {
                sequence_target
                    .upgrade()
                    .map(|target| target.read(cx).playback_sequence())
            }),
            present_frame: Rc::new(move |image: Arc<RenderImage>, index: i64, cx: &mut App| {
                if let Some(target) = present_target.upgrade() {
                    target.update(cx, |target, cx| target.present_frame(image, index, cx));
                }
            }),
            playback_did_finish_one_shot: Rc::new(move |cx: &mut App| {
                if let Some(target) = finish_target.upgrade() {
                    target.update(cx, |target, cx| target.playback_did_finish_one_shot(cx));
                }
            }),
            loops,
            cancellation: Arc::new(RiseLottiePreparationCancellation::new()),
            started_at,
            last_presented_index: Some(initial_index),
            pending_frame: None,
        }
    }
}

impl Drop for RiseLottieDisplayDriverRegistration {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn current_media_time() -> f64 {
    RiseClock::monotonic().as_secs_f64()
}

/// one frame clock for every animated sticker on screen
/// a loop per player costs a main thread hop per frame per player
/// one clock at the animation cadence replaces it, each tick is a bounded pass over rasterized frames
#[derive(Default)]
pub struct RiseLottieDisplayDriver {
    registrations: HashMap<EntityId, RiseLottieDisplayDriverRegistration>,
    display_link: Option<Task<()>>,
    installed_cadence: f64,
}

impl Global for RiseLottieDisplayDriver {}

impl RiseLottieDisplayDriver {
    /// the one driver of the app, created on first use
    pub fn shared<R>(cx: &mut App, body: impl FnOnce(&mut Self, &mut App) -> R) -> R {
        cx.update_default_global::<Self, R>(body)
    }

    pub fn add<T: RiseLottiePlaybackTarget>(
        &mut self,
        target: &Entity<T>,
        loops: bool,
        starting_at: i64,
        cx: &mut App,
    ) {
        let id = target.entity_id();
        let elapsed_at_index = target
            .read(cx)
            .playback_sequence()
            .map_or(0.0, |sequence| starting_at as f64 / sequence.frame_rate.max(1.0));
        if let Some(previous) = self.registrations.get(&id) {
            previous.cancellation.cancel();
        }
        self.registrations.insert(
            id,
            RiseLottieDisplayDriverRegistration::new(target, loops, current_media_time() - elapsed_at_index, starting_at),
        );
        self.start_if_needed(cx);
    }

    pub fn remove<T: RiseLottiePlaybackTarget>(&mut self, target: &Entity<T>) {
        if let Some(registration) = self.registrations.remove(&target.entity_id()) {
            registration.cancellation.cancel();
        }
        self.stop_if_idle();
    }

    fn presentation_stride(of: &RiseLottieFrameSequence) -> i64 {
        RiseLottieRasterPolicy::presentation_stride(of.frame_rate, of.bytes_per_frame() / 4)
    }

    fn cadence(of: &RiseLottieFrameSequence) -> f64 {
        RiseLottieRasterPolicy::presentation_frame_rate(of.frame_rate, of.bytes_per_frame() / 4)
    }

    fn apply_cadence(&mut self, cadence: f64) {
        if self.display_link.is_none() || cadence == self.installed_cadence {
            return;
        }
        self.installed_cadence = cadence;
    }

    fn start_if_needed(&mut self, cx: &mut App) {
        if self.registrations.is_empty() || self.display_link.is_some() {
            return;
        }
        self.display_link = Some(cx.spawn(async move |cx: &mut AsyncApp| RiseLottieDisplayLinkProxy::run(cx).await));
        self.installed_cadence = 0.0;
        self.apply_cadence(RiseLottieRasterPolicy::maximum_frame_rate());
    }

    fn stop_if_idle(&mut self) {
        if !self.registrations.is_empty() {
            return;
        }
        self.display_link = None;
    }

    /// one pass over the registrations
    /// returns the view updates to run once the driver isnt borrowed
    /// a view may add or remove itself inside them
    fn tick(&mut self, cx: &App) -> Vec<Box<dyn FnOnce(&mut App)>> {
        let now = current_media_time();
        let mut presents: Vec<Box<dyn FnOnce(&mut App)>> = Vec::new();
        let mut finished: Vec<EntityId> = Vec::new();
        let mut dead: Vec<EntityId> = Vec::new();
        let mut required: f64 = 0.0;

        for (id, registration) in &mut self.registrations {
            let Some(sequence) = (registration.playback_sequence)(cx) else {
                dead.push(*id);
                continue;
            };
            let Some(sequence) = sequence.filter(|sequence| sequence.frame_count > 0) else {
                continue;
            };

            required = required.max(Self::cadence(&sequence));
            let stride = Self::presentation_stride(&sequence);
            let elapsed = (now - registration.started_at).max(0.0);
            let raw_index = (elapsed * sequence.frame_rate.max(1.0)) as i64;

            let index: i64;
            let image: Arc<RenderImage>;
            if let Some(pending) = registration.pending_frame.clone() {
                let (is_complete, ready) = pending.result();
                if !is_complete {
                    continue;
                }
                registration.pending_frame = None;
                let Some(ready) = ready else {
                    continue;
                };
                index = pending.index;
                image = ready;
                registration.started_at = now - index as f64 / sequence.frame_rate;
            } else {
                index = if registration.loops {
                    ((raw_index % sequence.frame_count) / stride) * stride
                } else if raw_index >= sequence.frame_count - 1 {
                    sequence.frame_count - 1
                } else {
                    (raw_index / stride) * stride
                };
                if registration.last_presented_index == Some(index) {
                    if !registration.loops && index == sequence.frame_count - 1 {
                        finished.push(*id);
                    }
                    continue;
                }
                if let Some(cached) = sequence.cached_frame(index) {
                    image = cached;
                } else {
                    let pending = Arc::new(RiseLottieDisplayDriverPendingFrame::new(index));
                    let cancellation = Arc::clone(&registration.cancellation);
                    registration.pending_frame = Some(Arc::clone(&pending));
                    sequence.request_frame(
                        index,
                        Arc::new(move || !cancellation.is_cancelled()),
                        Some(Box::new(move |image: Option<Arc<RenderImage>>| pending.resolve(image))),
                    );
                    continue;
                }
            }

            registration.last_presented_index = Some(index);
            let present_frame = Rc::clone(&registration.present_frame);
            presents.push(Box::new(move |cx: &mut App| present_frame(image, index, cx)));
            if !registration.loops && index == sequence.frame_count - 1 {
                finished.push(*id);
            } else {
                Self::prefetch(&sequence, index, stride, registration);
            }
        }

        if required > 0.0 {
            self.apply_cadence(required);
        }

        for id in dead {
            if let Some(registration) = self.registrations.remove(&id) {
                registration.cancellation.cancel();
            }
        }
        for id in finished {
            if let Some(registration) = self.registrations.remove(&id) {
                registration.cancellation.cancel();
                let playback_did_finish_one_shot = Rc::clone(&registration.playback_did_finish_one_shot);
                presents.push(Box::new(move |cx: &mut App| playback_did_finish_one_shot(cx)));
            }
        }
        self.stop_if_idle();
        presents
    }

    /// keeps the raster pool a few presentations ahead so the first loop fills in without blocking a tick
    /// steps by the presentation stride since any other index is never shown
    fn prefetch(sequence: &Arc<RiseLottieFrameSequence>, after: i64, stride: i64, registration: &RiseLottieDisplayDriverRegistration) {
        for step in 1..=3 {
            let upcoming = after + step * stride;
            let next = if registration.loops {
                ((upcoming % sequence.frame_count) / stride) * stride
            } else {
                upcoming.min(sequence.frame_count - 1)
            };
            if sequence.cached_frame(next).is_none() {
                let cancellation = Arc::clone(&registration.cancellation);
                sequence.request_frame(next, Arc::new(move || !cancellation.is_cancelled()), None);
            }
            if !registration.loops && next == sequence.frame_count - 1 {
                break;
            }
        }
    }
}

enum RiseLottieDisplayLinkProxy {}

impl RiseLottieDisplayLinkProxy {
    async fn run(cx: &mut AsyncApp) {
        loop {
            let cadence = cx.update(|cx| {
                cx.try_global::<RiseLottieDisplayDriver>()
                    .filter(|driver| driver.display_link.is_some())
                    .map(|driver| driver.installed_cadence)
            });
            let Some(cadence) = cadence.filter(|cadence| *cadence > 0.0) else {
                return;
            };
            cx.background_executor()
                .timer(Duration::from_secs_f64(1.0 / cadence))
                .await;
            cx.update(Self::tick);
        }
    }

    fn tick(cx: &mut App) {
        if !cx.has_global::<RiseLottieDisplayDriver>() {
            return;
        }
        let updates = cx.update_global::<RiseLottieDisplayDriver, _>(|driver, cx| driver.tick(cx));
        for update in updates {
            update(cx);
        }
    }
}

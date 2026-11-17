use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{AnyView, App, AppContext, Entity, SharedString, Task};
use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::LatLng;
use rise_ui::swiftui::motion::ease_in_out;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::map::components::pins::map_friend_pin_view::{MapPinPalette, MapSelfPinView};
use crate::modules::map::components::pins::map_pin_layer::{map_glide_frames, MapAnnotationGlide};
use crate::modules::map::components::rise_map_host::{MapAnnotation, MapAnnotationView, RiseMapHost, WeakRiseMapHost};
use crate::modules::map::recorder::map_self_location_feed::{
    MapSelfHeading, MapSelfLocation, MapSelfLocationFeed, MapSelfLocationFeedObserver,
};
use crate::modules::map::shared::map_friend_status::{MapFriendStatus, MapPinTile, MapPinTrail, MapSpeedFormat};

pub struct MapSelfAnnotation {
    pub glide: RefCell<MapAnnotationGlide>,
}

impl MapAnnotation for MapSelfAnnotation {
    fn coordinate(&self) -> LatLng {
        self.glide.borrow().presented(RiseClock::monotonic())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// profile rendered on the reader's own pin
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapSelfProfile {
    pub id: String,
    pub name: String,
    pub logo: String,
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapSelfPinState {
    pub tile: Option<MapPinTile>,
    pub trail: Option<MapPinTrail>,
}

static MOTION_TIMEOUT_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("map_friend_motion"));

/// the reader's pin that replaces the system blue dot
/// position and compass come straight from the device feed
/// no fix means no pin, last known spot shows at once
pub struct MapSelfPinLayer {
    this: Weak<RefCell<MapSelfPinLayer>>,
    map_view: Option<WeakRiseMapHost>,
    profile: Option<MapSelfProfile>,
    palette: Option<MapPinPalette>,
    annotation: Option<Rc<MapSelfAnnotation>>,
    location: Option<MapSelfLocation>,
    heading: Option<MapSelfHeading>,
    observer_id: Option<Uuid>,
    retired_observer_id: Option<Uuid>,
    observer_epoch: u64,
    still_timer: Option<Task<()>>,
    glide_frames: Option<Task<()>>,
}

impl MapSelfPinLayer {
    const MOVE_ANIMATION: f64 = 0.5;

    /// shared with the coordinator, the feed and the timer reach the layer weakly
    pub fn new() -> Rc<RefCell<Self>> {
        Rc::new_cyclic(|this| {
            RefCell::new(Self {
                this: this.clone(),
                map_view: None,
                profile: None,
                palette: None,
                annotation: None,
                location: None,
                heading: None,
                observer_id: None,
                retired_observer_id: None,
                observer_epoch: 0,
                still_timer: None,
                glide_frames: None,
            })
        })
    }

    /// a walker gives a fix every few steps while the map is open
    /// no fix for longer than this means standing still whatever the last speed said
    fn motion_timeout_ms() -> i64 {
        *MOTION_TIMEOUT_MS
    }

    pub fn update(&mut self, profile: Option<&MapSelfProfile>, palette: &MapPinPalette, map_view: &RiseMapHost, cx: &mut App) {
        self.map_view = Some(map_view.downgrade());
        let profile_changed = profile != self.profile.as_ref();
        let palette_changed = Some(palette) != self.palette.as_ref();
        self.profile = profile.cloned();
        self.palette = Some(palette.clone());
        if let Some(retired) = self.retired_observer_id.take() {
            MapSelfLocationFeed::shared(cx).update(cx, |feed, _| feed.cancel(retired));
        }

        if self.profile.is_none() {
            self.stop(cx);
            return;
        }
        if self.observer_id.is_none() {
            let feed = MapSelfLocationFeed::shared(cx);
            self.location = feed.read(cx).location().cloned();
            self.heading = feed.read(cx).heading().cloned();
            let epoch = self.observer_epoch;
            let on_location = self.this.clone();
            let on_heading = self.this.clone();
            let observer = MapSelfLocationFeedObserver {
                on_location: Box::new(move |next, cx| {
                    let Some(this) = on_location.upgrade() else {
                        return;
                    };
                    let Ok(mut layer) = this.try_borrow_mut() else {
                        return;
                    };
                    if layer.observer_epoch == epoch {
                        layer.location_changed(next, cx);
                    }
                }),
                on_heading: Box::new(move |next, cx| {
                    let Some(this) = on_heading.upgrade() else {
                        return;
                    };
                    let Ok(mut layer) = this.try_borrow_mut() else {
                        return;
                    };
                    if layer.observer_epoch == epoch {
                        layer.heading_changed(next, cx);
                    }
                }),
            };
            self.observer_id = Some(feed.update(cx, |feed, _| feed.observe(observer)));
            self.place(false, cx);
        } else if profile_changed || palette_changed {
            self.refresh_view(false, cx);
        }
    }

    pub fn view(&self, annotation: &Rc<dyn MapAnnotation>, map_view: &RiseMapHost, cx: &mut App) -> Option<MapAnnotationView> {
        annotation.as_any().downcast_ref::<MapSelfAnnotation>()?;
        let profile = self.profile.clone()?;
        let palette = self.palette.clone()?;
        let state = self.state();
        let heading = self.heading.clone();
        let zoom = map_view.zoom_level();
        let direction = map_view.direction();
        let view = map_view
            .dequeue_reusable_annotation_view(MapSelfPinView::REUSE_ID)
            .and_then(|view| view.view.downcast::<MapSelfPinView>().ok())
            .unwrap_or_else(|| cx.new(|cx| MapSelfPinView::new(Some(SharedString::from(MapSelfPinView::REUSE_ID)), cx)));
        let side = view.update(cx, |view, cx| {
            view.apply_camera(zoom, direction, false, cx);
            view.configure(&profile, &palette, cx);
            view.apply(&state, &palette, false, cx);
            view.apply_heading(heading.as_ref(), false, cx);
            f64::from(view.ring_side())
        });
        let mut annotation_view = MapAnnotationView::new(AnyView::from(view), MapSelfPinView::REUSE_ID, side, side);
        annotation_view.z_position = f64::from(MapSelfPinView::Z_POSITION);
        Some(annotation_view)
    }

    pub fn camera_changed(&mut self, map_view: &RiseMapHost, animated: bool, cx: &mut App) {
        let zoom = map_view.zoom_level();
        let direction = map_view.direction();
        if let Some(view) = self.pin_view() {
            view.update(cx, |view, cx| view.apply_camera(zoom, direction, animated, cx));
        }
    }

    pub fn refresh_motion(&mut self, cx: &mut App) {
        if let Some(view) = self.pin_view() {
            view.update(cx, |view, cx| view.refresh_motion(cx));
        }
    }

    pub fn teardown(&mut self, map_view: &RiseMapHost) {
        self.stop_without_feed(Some(map_view));
        if let Some(observer_id) = self.observer_id.take() {
            self.retired_observer_id = Some(observer_id);
        }
    }

    fn stop(&mut self, cx: &mut App) {
        if let Some(observer_id) = self.observer_id.take() {
            MapSelfLocationFeed::shared(cx).update(cx, |feed, _| feed.cancel(observer_id));
        }
        let map_view = self.map_view.as_ref().and_then(WeakRiseMapHost::upgrade);
        self.stop_without_feed(map_view.as_ref());
        if let Some(map_view) = map_view {
            map_view.set_needs_display(cx);
        }
    }

    fn stop_without_feed(&mut self, map_view: Option<&RiseMapHost>) {
        self.observer_epoch = self.observer_epoch.wrapping_add(1);
        self.still_timer = None;
        self.glide_frames = None;
        self.location = None;
        self.heading = None;
        if let (Some(annotation), Some(map_view)) = (self.annotation.take(), map_view) {
            let annotation: Rc<dyn MapAnnotation> = annotation;
            map_view.remove_annotation(&annotation);
        }
    }

    fn location_changed(&mut self, next: Option<MapSelfLocation>, cx: &mut App) {
        self.location = next;
        self.place(true, cx);
    }

    fn heading_changed(&mut self, next: Option<MapSelfHeading>, cx: &mut App) {
        self.heading = next.clone();
        if let Some(view) = self.pin_view() {
            view.update(cx, |view, cx| view.apply_heading(next.as_ref(), true, cx));
        }
    }

    fn place(&mut self, animated: bool, cx: &mut App) {
        let Some(map_view) = self.map_view.as_ref().and_then(WeakRiseMapHost::upgrade) else {
            return;
        };
        let Some(coordinate) = self.location.as_ref().map(MapSelfLocation::coordinate) else {
            self.still_timer = None;
            if let Some(annotation) = self.annotation.take() {
                let annotation: Rc<dyn MapAnnotation> = annotation;
                map_view.remove_annotation(&annotation);
                map_view.set_needs_display(cx);
            }
            return;
        };

        match self.annotation.as_ref() {
            Some(annotation) => {
                if animated {
                    annotation.glide.borrow_mut().glide(coordinate, ease_in_out(Self::MOVE_ANIMATION));
                    self.drive_glide(&map_view, cx);
                } else {
                    annotation.glide.borrow_mut().jump(coordinate);
                }
            }
            None => {
                let created = Rc::new(MapSelfAnnotation { glide: RefCell::new(MapAnnotationGlide::new(coordinate)) });
                self.annotation = Some(created.clone());
                map_view.add_annotation(created);
            }
        }
        map_view.set_needs_display(cx);
        self.refresh_view(animated, cx);
        self.arm_still_timer(cx);
    }

    fn drive_glide(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        if self.glide_frames.is_some() {
            return;
        }
        let this = self.this.clone();
        let is_running = move || {
            let Some(this) = this.upgrade() else {
                return false;
            };
            let Ok(mut layer) = this.try_borrow_mut() else {
                return true;
            };
            let now = RiseClock::monotonic();
            let running = layer.annotation.as_ref().is_some_and(|annotation| annotation.glide.borrow().is_running(now));
            if !running {
                if let Some(frames) = layer.glide_frames.take() {
                    frames.detach();
                }
            }
            running
        };
        self.glide_frames = Some(map_glide_frames(map_view, is_running, cx));
    }

    fn refresh_view(&mut self, animated: bool, cx: &mut App) {
        let (Some(view), Some(profile), Some(palette)) = (self.pin_view(), self.profile.clone(), self.palette.clone()) else {
            return;
        };
        let state = self.state();
        view.update(cx, |view, cx| {
            view.configure(&profile, &palette, cx);
            view.apply(&state, &palette, animated, cx);
        });
    }

    fn state(&self) -> MapSelfPinState {
        let Some(location) = self.location.as_ref() else {
            return MapSelfPinState { tile: None, trail: None };
        };
        let now_ms = RiseClock::now().ms();
        if let Some(since) = location.unavailable_since_ms {
            return MapSelfPinState { tile: MapFriendStatus::inactive(since, now_ms).tile, trail: None };
        }
        if !location.is_at_home && location.stay_since_ms.is_none() && now_ms - location.time_ms <= Self::motion_timeout_ms() {
            if let Some(speed) = MapSpeedFormat::parts(location.speed_meters_per_second) {
                return MapSelfPinState {
                    tile: MapPinTile::new(None, None, Some(speed)),
                    trail: MapPinTrail::new(location.course_degrees, location.speed_meters_per_second),
                };
            }
        }
        let stayed_ms = location.stay_since_ms.map(|since| (now_ms - since).max(0)).unwrap_or(0);
        let emoji = if location.is_at_home {
            Some(MapFriendStatus::HOME_EMOJI.to_string())
        } else {
            location.place_kind.map(|kind| kind.emoji().to_string())
        };
        let text = (stayed_ms >= 60_000).then(|| MapFriendStatus::duration_parts(stayed_ms));
        MapSelfPinState { tile: MapPinTile::new(emoji, None, text), trail: None }
    }

    /// standing still gives no fix to say so so a timer reports it
    fn arm_still_timer(&mut self, cx: &mut App) {
        self.still_timer = None;
        let Some(location) = self.location.as_ref() else {
            return;
        };
        let now_ms = RiseClock::now().ms();
        let delay_ms = match location.unavailable_since_ms.or(location.stay_since_ms) {
            Some(since) if since > 0 => 60_000 - (now_ms - since).max(0) % 60_000,
            _ => {
                let remaining = location.time_ms + Self::motion_timeout_ms() - now_ms;
                if remaining < 0 {
                    return;
                }
                remaining + 200
            }
        };
        let delay = Duration::from_millis(u64::try_from(delay_ms).unwrap_or(0));
        let this = self.this.clone();
        self.still_timer = Some(cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            let Some(this) = this.upgrade() else {
                return;
            };
            cx.update(|cx| {
                let Ok(mut layer) = this.try_borrow_mut() else {
                    return;
                };
                if let Some(firing) = layer.still_timer.take() {
                    firing.detach();
                }
                layer.refresh_view(true, cx);
                layer.arm_still_timer(cx);
            });
        }));
    }

    pub fn current_view(&self) -> Option<Entity<MapSelfPinView>> {
        self.pin_view()
    }

    fn pin_view(&self) -> Option<Entity<MapSelfPinView>> {
        let annotation: Rc<dyn MapAnnotation> = self.annotation.clone()?;
        let map_view = self.map_view.as_ref()?.upgrade()?;
        map_view.view_for(&annotation)?.downcast::<MapSelfPinView>().ok()
    }
}

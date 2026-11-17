use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{AnyView, App, AppContext, Entity, SharedString, Task, Window};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::{LatLng, ScreenPoint};
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::{ease_in_out, linear, MotionSpec};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::components::pins::map_friend_pin_view::{
    MapFriendPinView, MapFriendStackPinView, MapPinPalette,
};
use crate::modules::map::components::pins::map_pin_decor::{MapPinAngle, MapPinZoom};
use crate::modules::map::components::rise_map_host::{MapAnnotation, MapAnnotationView, RiseMapHost};
use crate::modules::map::shared::map_friend_status::MapPinModel;
use crate::modules::map::stores::map_actions::map_types::MapLiveFriend;

/// presentation coordinate of an annotation
/// the model value is the target and the map samples the glide when it draws
#[derive(Clone, Debug)]
pub struct MapAnnotationGlide {
    latitude: Animated<f64>,
    longitude: Animated<f64>,
}

impl MapAnnotationGlide {
    pub fn new(coordinate: LatLng) -> Self {
        Self { latitude: Animated::new(coordinate.latitude), longitude: Animated::new(coordinate.longitude) }
    }

    pub fn target(&self) -> LatLng {
        LatLng::new(*self.latitude.target(), *self.longitude.target())
    }

    pub fn jump(&mut self, coordinate: LatLng) {
        self.latitude.jump(coordinate.latitude);
        self.longitude.jump(coordinate.longitude);
    }

    /// glide starts where the pin is drawn now
    /// annotation moves ignore reduce motion so this does too
    pub fn glide(&mut self, coordinate: LatLng, spec: MotionSpec) {
        let now = RiseClock::monotonic();
        self.latitude.set(coordinate.latitude, spec, now, false);
        self.longitude.set(coordinate.longitude, spec, now, false);
    }

    pub fn presented(&self, now: MotionTime) -> LatLng {
        LatLng::new(self.latitude.value(now), self.longitude.value(now))
    }

    pub fn is_running(&self, now: MotionTime) -> bool {
        self.latitude.is_running(now) || self.longitude.is_running(now)
    }
}

/// redraws the map each display frame while is_running holds
/// so a glide on an annotation moves on screen
/// why: desktop map draws annotations at the presented coordinate only on redraw
pub fn map_glide_frames(map: &RiseMapHost, is_running: impl Fn() -> bool + 'static, cx: &mut App) -> Task<()> {
    const FRAME: Duration = Duration::from_millis(16);
    let map = map.downgrade();
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(FRAME).await;
        let Some(map) = map.upgrade() else {
            return;
        };
        cx.update(|cx| map.set_needs_display(cx));
        if !is_running() {
            return;
        }
    })
}

pub struct MapPinAnnotation {
    pub key: String,
    pub members: RefCell<Vec<MapLiveFriend>>,
    pub glide: RefCell<MapAnnotationGlide>,
}

impl MapPinAnnotation {
    pub fn new(key: String, members: Vec<MapLiveFriend>, coordinate: LatLng) -> Self {
        Self { key, members: RefCell::new(members), glide: RefCell::new(MapAnnotationGlide::new(coordinate)) }
    }

    pub fn is_stack(&self) -> bool {
        self.members.borrow().len() > 1
    }
}

impl MapAnnotation for MapPinAnnotation {
    fn coordinate(&self) -> LatLng {
        self.glide.borrow().presented(RiseClock::monotonic())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

static MAXIMUM_GLIDE: LazyLock<f64> =
    LazyLock::new(|| RiseLimits::service_value("map_watched_update_interval_ms") as f64 / 1_000.0);

/// friends as annotations, diffed by friend id so a move updates one coordinate
/// friends closer than a finger's width fold into a stack
/// regrouped when the camera settles
pub struct MapPinLayer {
    pub on_select: Rc<dyn Fn(Option<String>, &mut Window, &mut App)>,

    this: Weak<RefCell<MapPinLayer>>,
    friends: Vec<MapLiveFriend>,
    revision: u64,
    selected_id: Option<String>,
    palette: Option<MapPinPalette>,
    annotations: HashMap<String, Rc<MapPinAnnotation>>,
    last_grouping_zoom: f64,
    pending_regroup_id: Option<Uuid>,
    is_regrouping: bool,
    last_camera_zoom: f64,
    last_camera_direction: f64,
    clock: Option<Task<()>>,
    glide_frames: Option<Task<()>>,
    map_view: Option<RiseMapHost>,
}

impl MapPinLayer {
    const GROUPING_DISTANCE: f64 = 30.0;
    const REGROUP_ZOOM_STEP: f64 = 0.35;
    const STACK_ZOOM_STEP: f64 = 2.5;
    const FOCUS_ZOOM_LEVEL: f64 = 14.5;
    const MOVE_ANIMATION: f64 = 0.6;
    const CLOCK_INTERVAL: f64 = 30.0;

    /// coordinator keeps the layer shared, timers and deferred regroups reach it weakly
    pub fn new() -> Rc<RefCell<Self>> {
        Rc::new_cyclic(|this| {
            RefCell::new(Self {
                on_select: Rc::new(|_, _, _| {}),
                this: this.clone(),
                friends: Vec::new(),
                revision: u64::MAX,
                selected_id: None,
                palette: None,
                annotations: HashMap::new(),
                last_grouping_zoom: -1.0,
                pending_regroup_id: None,
                is_regrouping: false,
                last_camera_zoom: -1.0,
                last_camera_direction: -1.0,
                clock: None,
                glide_frames: None,
                map_view: None,
            })
        })
    }

    fn maximum_glide() -> f64 {
        *MAXIMUM_GLIDE
    }

    pub fn update(
        &mut self,
        friends: &[MapLiveFriend],
        revision: u64,
        selected_id: Option<&str>,
        palette: &MapPinPalette,
        map_view: &RiseMapHost,
        cx: &mut App,
    ) {
        self.map_view = Some(map_view.clone());
        let friends_changed = revision != self.revision;
        let selection_changed = selected_id != self.selected_id.as_deref();
        let palette_changed = Some(palette) != self.palette.as_ref();
        if !(friends_changed || selection_changed || palette_changed) {
            return;
        }

        let previous_selection = self.selected_id.clone();
        self.friends = friends.to_vec();
        self.revision = revision;
        self.selected_id = selected_id.map(str::to_string);
        self.palette = Some(palette.clone());

        self.regroup(map_view, friends_changed && !selection_changed, cx);
        if palette_changed {
            self.refresh_visible(map_view, cx);
        }
        if selection_changed {
            self.highlight(previous_selection.as_deref(), false, map_view, cx);
            self.highlight(selected_id, true, map_view, cx);
        }
        self.start_clock(map_view, cx);
    }

    pub fn region_is_changing(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        self.apply_camera(map_view, false, false, cx);
        if (map_view.zoom_level() - self.last_grouping_zoom).abs() < Self::REGROUP_ZOOM_STEP {
            return;
        }
        self.schedule_regroup(map_view, cx);
    }

    pub fn region_did_change(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        self.schedule_regroup(map_view, cx);
        self.apply_camera(map_view, true, true, cx);
    }

    fn schedule_regroup(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        if self.is_regrouping || self.pending_regroup_id.is_some() {
            return;
        }
        let request_id = Uuid::new_v4();
        self.pending_regroup_id = Some(request_id);
        let this = self.this.clone();
        let map_view = map_view.downgrade();
        cx.defer(move |cx| {
            let Some(this) = this.upgrade() else {
                return;
            };
            let mut layer = this.borrow_mut();
            if layer.pending_regroup_id != Some(request_id) {
                return;
            }
            layer.pending_regroup_id = None;
            let Some(map_view) = map_view.upgrade() else {
                return;
            };
            layer.regroup(&map_view, false, cx);
        });
    }

    pub fn refresh_motion(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        for annotation in map_view.visible_annotations() {
            if let Some(view) = map_view.view_for(&annotation).and_then(|view| view.downcast::<MapFriendPinView>().ok()) {
                view.update(cx, |view, cx| view.refresh_motion(cx));
            }
        }
    }

    /// size follows zoom and wake follows map rotation
    /// only on screen pins have views and only a camera that really moved gets here
    fn apply_camera(&mut self, map_view: &RiseMapHost, animated: bool, force: bool, cx: &mut App) {
        let zoom = map_view.zoom_level();
        let direction = map_view.direction();
        if !(force
            || (zoom - self.last_camera_zoom).abs() > 0.02
            || MapPinAngle::shortest(direction - self.last_camera_direction).abs() > 0.5)
        {
            return;
        }
        self.last_camera_zoom = zoom;
        self.last_camera_direction = direction;
        for annotation in map_view.visible_annotations() {
            if annotation.as_any().downcast_ref::<MapPinAnnotation>().is_none() {
                continue;
            }
            let Some(view) = map_view.view_for(&annotation) else {
                continue;
            };
            match view.downcast::<MapFriendPinView>() {
                Ok(pin) => pin.update(cx, |pin, cx| pin.apply_camera(zoom, direction, animated, cx)),
                Err(view) => {
                    if let Ok(stack) = view.downcast::<MapFriendStackPinView>() {
                        stack.update(cx, |stack, cx| stack.apply_zoom(zoom, cx));
                    }
                }
            }
        }
    }

    pub fn view(&self, annotation: &Rc<dyn MapAnnotation>, map_view: &RiseMapHost, cx: &mut App) -> Option<MapAnnotationView> {
        let pin = annotation.as_any().downcast_ref::<MapPinAnnotation>()?;
        let palette = self.palette.clone()?;
        let members = pin.members.borrow().clone();
        let zoom = map_view.zoom_level();
        if pin.is_stack() {
            let view = map_view
                .dequeue_reusable_annotation_view(MapFriendStackPinView::REUSE_ID)
                .and_then(|view| view.view.downcast::<MapFriendStackPinView>().ok())
                .unwrap_or_else(|| {
                    cx.new(|cx| MapFriendStackPinView::new(Some(SharedString::from(MapFriendStackPinView::REUSE_ID)), cx))
                });
            view.update(cx, |view, cx| {
                view.apply_zoom(zoom, cx);
                view.configure(&members, &palette, cx);
            });
            let (width, height) = MapFriendStackPinView::frame_size();
            return Some(MapAnnotationView::new(
                AnyView::from(view),
                MapFriendStackPinView::REUSE_ID,
                f64::from(width),
                f64::from(height),
            ));
        }

        let friend = members.first()?;
        let model = MapPinModel::new(friend, Self::now_milliseconds())?;
        let highlighted = self.selected_id.as_deref() == Some(friend.id());
        let direction = map_view.direction();
        let view = map_view
            .dequeue_reusable_annotation_view(MapFriendPinView::REUSE_ID)
            .and_then(|view| view.view.downcast::<MapFriendPinView>().ok())
            .unwrap_or_else(|| cx.new(|cx| MapFriendPinView::new(Some(SharedString::from(MapFriendPinView::REUSE_ID)), cx)));
        let side = view.update(cx, |view, cx| {
            view.apply_camera(zoom, direction, false, cx);
            view.configure(&model, &palette, cx);
            view.set_highlighted(highlighted, false, cx);
            f64::from(view.ring_side())
        });
        Some(MapAnnotationView::new(AnyView::from(view), MapFriendPinView::REUSE_ID, side, side))
    }

    pub fn did_select(&mut self, annotation: &Rc<dyn MapAnnotation>, map_view: &RiseMapHost, window: &mut Window, cx: &mut App) {
        map_view.deselect_annotation(annotation);
        let Some(pin) = annotation.as_any().downcast_ref::<MapPinAnnotation>() else {
            return;
        };
        if pin.is_stack() {
            let zoom = map_view.maximum_zoom_level().min(map_view.zoom_level() + Self::STACK_ZOOM_STEP);
            let centre = pin.glide.borrow().target();
            map_view.set_center(centre, Some(zoom), None, true, cx);
        } else {
            let friend_id = pin.members.borrow().first().map(|friend| friend.id().to_string());
            if let Some(friend_id) = friend_id {
                let on_select = self.on_select.clone();
                on_select(Some(friend_id), window, cx);
            }
        }
    }

    /// the friend's own pin while on screen, none if folded into a stack
    pub fn pin_view(&self, user_id: &str) -> Option<Entity<MapFriendPinView>> {
        let annotation = self.annotations.get(user_id)?;
        if annotation.is_stack() {
            return None;
        }
        let annotation: Rc<dyn MapAnnotation> = annotation.clone();
        self.map_view.as_ref()?.view_for(&annotation)?.downcast::<MapFriendPinView>().ok()
    }

    pub fn coordinate(&self, user_id: &str) -> Option<LatLng> {
        self.friends.iter().find(|friend| friend.id() == user_id)?.position.as_ref().map(|position| position.fix.coordinate())
    }

    /// true once the friend is on the map and the camera is on its way
    pub fn focus(&self, user_id: &str, map_view: &RiseMapHost, cx: &mut App) -> bool {
        let Some(fix) = self
            .friends
            .iter()
            .find(|friend| friend.id() == user_id)
            .and_then(|friend| friend.position.as_ref())
            .map(|position| &position.fix)
        else {
            return false;
        };
        let zoom = map_view.zoom_level().max(Self::FOCUS_ZOOM_LEVEL);
        map_view.set_center(fix.coordinate(), Some(zoom), None, true, cx);
        true
    }

    pub fn teardown(&mut self, _map_view: &RiseMapHost) {
        self.pending_regroup_id = None;
        self.clock = None;
        self.glide_frames = None;
    }

    fn regroup(&mut self, map_view: &RiseMapHost, animated: bool, cx: &mut App) {
        if self.is_regrouping || map_view.bounds_width() <= 0.0 {
            return;
        }
        self.is_regrouping = true;
        self.last_grouping_zoom = map_view.zoom_level();

        let grouping_distance = Self::GROUPING_DISTANCE * f64::from(MapPinZoom::scale(map_view.zoom_level()));
        let mut groups: Vec<(Option<ScreenPoint>, Vec<MapLiveFriend>)> = Vec::new();
        for friend in &self.friends {
            let Some(fix) = friend.position.as_ref().map(|position| &position.fix) else {
                continue;
            };
            if self.selected_id.as_deref() == Some(friend.id()) {
                groups.push((None, vec![friend.clone()]));
                continue;
            }
            let point = map_view.convert_coordinate_to_point(fix.coordinate());
            let index = point.and_then(|point| {
                groups.iter().position(|(seed, _)| {
                    seed.is_some_and(|seed| (seed.x - point.x).hypot(seed.y - point.y) < grouping_distance)
                })
            });
            match index {
                Some(index) => groups[index].1.push(friend.clone()),
                None => groups.push((point, vec![friend.clone()])),
            }
        }

        let mut desired: HashMap<String, (LatLng, Vec<MapLiveFriend>)> = HashMap::new();
        for (_, members) in groups {
            let Some(first) = members.first() else {
                continue;
            };
            let key = if members.len() == 1 { first.id().to_string() } else { format!("stack|{}", first.id()) };
            desired.insert(key, (Self::centre(&members), members));
        }

        let removed: Vec<String> = self.annotations.keys().filter(|key| !desired.contains_key(*key)).cloned().collect();
        if !removed.is_empty() {
            let gone: Vec<Rc<dyn MapAnnotation>> = removed
                .iter()
                .filter_map(|key| self.annotations.remove(key))
                .map(|annotation| annotation as Rc<dyn MapAnnotation>)
                .collect();
            map_view.remove_annotations(&gone);
        }
        let mut placement_changed = !removed.is_empty();

        let mut added: Vec<Rc<dyn MapAnnotation>> = Vec::new();
        for (key, (coordinate, members)) in desired {
            if let Some(existing) = self.annotations.get(&key).cloned() {
                let duration = Self::move_duration(&existing.members.borrow(), &members);
                *existing.members.borrow_mut() = members;
                let glides = animated && !existing.is_stack();
                placement_changed |= Self::r#move(&existing, coordinate, glides, duration);
                self.reconfigure(&existing, map_view, cx);
            } else {
                let annotation = Rc::new(MapPinAnnotation::new(key.clone(), members, coordinate));
                self.annotations.insert(key, annotation.clone());
                added.push(annotation);
            }
        }
        if !added.is_empty() {
            map_view.add_annotations(added);
            placement_changed = true;
        }
        self.drive_glides(map_view, cx);
        if placement_changed {
            map_view.set_needs_display(cx);
        }
        self.is_regrouping = false;
    }

    /// true when the pin got a new target
    fn r#move(annotation: &MapPinAnnotation, coordinate: LatLng, animated: bool, duration: f64) -> bool {
        let mut glide = annotation.glide.borrow_mut();
        let current = glide.target();
        if current.latitude == coordinate.latitude && current.longitude == coordinate.longitude {
            return false;
        }
        if !animated {
            glide.jump(coordinate);
            return true;
        }
        let spec = if duration > Self::MOVE_ANIMATION { linear(duration) } else { ease_in_out(duration) };
        glide.glide(coordinate, spec);
        true
    }

    fn drive_glides(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        if self.glide_frames.is_some() || !self.is_gliding() {
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
            let running = layer.is_gliding();
            if !running {
                if let Some(frames) = layer.glide_frames.take() {
                    frames.detach();
                }
            }
            running
        };
        self.glide_frames = Some(map_glide_frames(map_view, is_running, cx));
    }

    fn is_gliding(&self) -> bool {
        let now = RiseClock::monotonic();
        self.annotations.values().any(|annotation| annotation.glide.borrow().is_running(now))
    }

    fn move_duration(previous: &[MapLiveFriend], next: &[MapLiveFriend]) -> f64 {
        if previous.len() != 1 || next.len() != 1 {
            return Self::MOVE_ANIMATION;
        }
        let before = previous.first().and_then(|friend| friend.position.as_ref()).map(|position| &position.fix);
        let after = next.first().and_then(|friend| friend.position.as_ref()).map(|position| &position.fix);
        let (Some(before), Some(after)) = (before, after) else {
            return Self::MOVE_ANIMATION;
        };
        if !(after.motion.is_moving() && after.stay_since == 0 && after.recorded_at > before.recorded_at) {
            return Self::MOVE_ANIMATION;
        }
        let seconds = (after.recorded_at - before.recorded_at) as f64 / 1_000.0;
        seconds.max(Self::MOVE_ANIMATION).min(Self::maximum_glide())
    }

    /// only answers for on screen annotations which is the set worth it
    fn reconfigure(&self, annotation: &Rc<MapPinAnnotation>, map_view: &RiseMapHost, cx: &mut App) {
        let Some(palette) = self.palette.clone() else {
            return;
        };
        let handle: Rc<dyn MapAnnotation> = annotation.clone();
        let Some(view) = map_view.view_for(&handle) else {
            return;
        };
        let members = annotation.members.borrow().clone();
        match view.downcast::<MapFriendStackPinView>() {
            Ok(stack) => stack.update(cx, |stack, cx| stack.configure(&members, &palette, cx)),
            Err(view) => {
                let (Ok(pin), Some(friend)) = (view.downcast::<MapFriendPinView>(), members.first()) else {
                    return;
                };
                if let Some(model) = MapPinModel::new(friend, Self::now_milliseconds()) {
                    pin.update(cx, |pin, cx| pin.configure(&model, &palette, cx));
                }
            }
        }
    }

    fn refresh_visible(&self, map_view: &RiseMapHost, cx: &mut App) {
        for annotation in map_view.visible_annotations() {
            let Some(pin) = annotation.as_any().downcast_ref::<MapPinAnnotation>() else {
                continue;
            };
            if let Some(owned) = self.annotations.get(&pin.key).cloned() {
                self.reconfigure(&owned, map_view, cx);
            }
        }
    }

    fn highlight(&self, user_id: Option<&str>, highlighted: bool, map_view: &RiseMapHost, cx: &mut App) {
        let Some(annotation) = user_id.and_then(|user_id| self.annotations.get(user_id)).cloned() else {
            return;
        };
        let annotation: Rc<dyn MapAnnotation> = annotation;
        let Some(view) = map_view.view_for(&annotation).and_then(|view| view.downcast::<MapFriendPinView>().ok()) else {
            return;
        };
        view.update(cx, |view, cx| view.set_highlighted(highlighted, true, cx));
    }

    fn start_clock(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        if self.clock.is_some() {
            return;
        }
        let this = self.this.clone();
        let map_view = map_view.downgrade();
        let interval = Duration::from_secs_f64(Self::CLOCK_INTERVAL);
        self.clock = Some(cx.spawn(async move |cx| loop {
            cx.background_executor().timer(interval).await;
            let (Some(this), Some(map_view)) = (this.upgrade(), map_view.upgrade()) else {
                return;
            };
            if map_view.projection().is_none() {
                continue;
            }
            cx.update(|cx| {
                if let Ok(layer) = this.try_borrow() {
                    layer.refresh_visible(&map_view, cx);
                }
            });
        }));
    }

    fn centre(members: &[MapLiveFriend]) -> LatLng {
        let fixes: Vec<_> = members.iter().filter_map(|friend| friend.position.as_ref().map(|position| &position.fix)).collect();
        let Some(first) = fixes.first() else {
            return LatLng::default();
        };
        if fixes.len() <= 1 {
            return first.coordinate();
        }
        let mut latitude = 0.0;
        let mut longitude_offset = 0.0;
        for fix in &fixes {
            latitude += fix.latitude;
            let mut delta = fix.longitude - first.longitude;
            if delta > 180.0 {
                delta -= 360.0;
            }
            if delta < -180.0 {
                delta += 360.0;
            }
            longitude_offset += delta;
        }
        let count = fixes.len() as f64;
        let mut longitude = first.longitude + longitude_offset / count;
        if longitude > 180.0 {
            longitude -= 360.0;
        }
        if longitude < -180.0 {
            longitude += 360.0;
        }
        LatLng::new(latitude / count, longitude)
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}

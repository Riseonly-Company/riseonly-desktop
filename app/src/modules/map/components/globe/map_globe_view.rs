use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{
    App, BoxShadow, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, PinchEvent, Pixels, Point, Render, RenderImage,
    ScrollDelta, ScrollWheelEvent, StatefulInteractiveElement, Styled, StyledImage, Task, TouchPhase, Window, canvas,
    div, img, point, px, rgb, rgba,
};
use gpui::prelude::FluentBuilder;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, Frame, RgbaImage};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::map_view::{Camera, LatLng, SnapshotRequest, snapshot};
use rise_platform::AppPaths;
use rise_platform::accessibility::reduce_motion;
use smallvec::SmallVec;

use crate::core::config::app_environment::AppEnvironment;
use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::components::globe::map_globe_layer::{MapGlobeCameraHandler, MapGlobePersonHandler};
use crate::modules::map::components::pins::map_friend_pin_view::{MapAvatarView, MapPinPalette};
use rise_media::map_view::{WHEEL_ZOOM_CAP, WHEEL_ZOOM_PER_LINE, WHEEL_ZOOM_POINTS_PER_LEVEL};

/// somebody on the globe, the reader or a friend
#[derive(Clone, PartialEq, Debug)]
pub struct MapGlobePerson {
    pub id: String,
    pub name: String,
    pub logo: String,
    pub coordinate: LatLng,
    pub is_self: bool,
}

/// ball coasting after a fling
/// keeps the last frame time so each step moves by real elapsed time
struct MapGlobeSpin {
    last_frame: Duration,
}

/// drag in progress, last pointer spot and how fast it moved
struct MapGlobePan {
    last_position: Point<Pixels>,
    last_time: Duration,
    velocity: Point<f64>,
}

/// pinch in progress, the scale and its rate per second
struct MapGlobePinch {
    scale: f64,
    last_time: Duration,
    velocity: f64,
}

/// the world as a ball for heights where a flat map just repeats
/// maplibre has no globe so its a sphere textured with a map snapshot
/// people are flat avatars fading at the edge
pub struct MapGlobeView {
    pub on_dive_in: MapGlobeCameraHandler,
    pub on_settle: MapGlobeCameraHandler,
    pub on_select_person: MapGlobePersonHandler,

    pins: HashMap<String, MapGlobePinView>,
    people: Vec<MapGlobePerson>,
    palette: Option<MapPinPalette>,
    centre: LatLng,
    distance: f32,
    spin: Option<MapGlobeSpin>,
    velocity: Point<f64>,
    pinch_start_distance: f32,
    has_dived_in: bool,
    texture_key: Option<String>,
    is_following_map: bool,

    bounds: Bounds<Pixels>,
    scale_factor: f32,
    texture: Option<Arc<RgbaImage>>,
    texture_task: Option<Task<()>>,
    ball: Option<Arc<RenderImage>>,
    ball_task: Option<Task<()>>,
    ball_is_stale: bool,
    pan: Option<MapGlobePan>,
    pinch: Option<MapGlobePinch>,
    focus_handle: FocusHandle,
}

impl MapGlobeView {
    const FIELD_OF_VIEW: f64 = 36.0;
    const FARTHEST: f32 = 5.6;
    const NEAREST: f32 = 2.2;
    const RESTING_DISTANCE: f32 = 3.9;
    const LATITUDE_LIMIT: f64 = 80.0;
    const PIN_SIDE: f32 = 22.0;
    const BALL_COLOUR: [u8; 3] = [23, 33, 51];
    const FLING_WINDOW: Duration = Duration::from_millis(60);

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            on_dive_in: Rc::new(|_, _, _, _| {}),
            on_settle: Rc::new(|_, _, _, _| {}),
            on_select_person: Rc::new(|_, _, _| {}),
            pins: HashMap::new(),
            people: Vec::new(),
            palette: None,
            centre: LatLng::new(30.0, 60.0),
            distance: Self::RESTING_DISTANCE,
            spin: None,
            velocity: Point::default(),
            pinch_start_distance: Self::RESTING_DISTANCE,
            has_dived_in: false,
            texture_key: None,
            is_following_map: false,
            bounds: Bounds::default(),
            scale_factor: 1.0,
            texture: None,
            texture_task: None,
            ball: None,
            ball_task: None,
            ball_is_stale: false,
            pan: None,
            pinch: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn width(&self) -> f64 {
        f64::from(f32::from(self.bounds.size.width))
    }

    fn height(&self) -> f64 {
        f64::from(f32::from(self.bounds.size.height))
    }

    pub fn enter(&mut self, coordinate: LatLng, matching_zoom: Option<f64>, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_spin(window, cx);
        self.has_dived_in = false;
        self.is_following_map = true;
        self.centre = coordinate;
        let matched =
            matching_zoom.and_then(|zoom| self.camera_distance(zoom, coordinate.latitude)).map(|wanted| self.clamped(wanted));
        self.distance = matched.unwrap_or(Self::RESTING_DISTANCE);
        self.apply_camera(cx);
    }

    /// driven from the flat map camera frame by frame while its gesture lasts
    pub fn follow(&mut self, next: LatLng, zoom_level: f64, cx: &mut Context<Self>) {
        if !self.is_following_map || self.has_dived_in {
            return;
        }
        let Some(matched) = self.camera_distance(zoom_level, next.latitude) else {
            return;
        };
        self.centre = LatLng::new(next.latitude.clamp(-Self::LATITUDE_LIMIT, Self::LATITUDE_LIMIT), next.longitude);
        self.distance = self.clamped(matched);
        self.apply_camera(cx);
    }

    fn clamped(&self, wanted: f32) -> f32 {
        Self::FARTHEST.min((Self::NEAREST * 0.9).max(wanted))
    }

    /// flat map zoom that shows the screen center at the ball's scale
    /// mercator stretches with latitude and the ball doesnt, hence the cosine
    fn zoom(&self, distance: f32, latitude: f64) -> f64 {
        let per_point = self.degrees_per_point_at(distance);
        let cosine = 0.2_f64.max((latitude * std::f64::consts::PI / 180.0).cos());
        (360.0 * cosine / (512.0 * per_point)).log2()
    }

    fn camera_distance(&self, zoom: f64, latitude: f64) -> Option<f32> {
        if self.width() <= 0.0 {
            return None;
        }
        let cosine = 0.2_f64.max((latitude * std::f64::consts::PI / 180.0).cos());
        let per_point = 360.0 * cosine / (512.0 * 2_f64.powf(zoom));
        let visible = per_point * self.width() * std::f64::consts::PI / 180.0;
        Some((1.0 + visible / (2.0 * (Self::FIELD_OF_VIEW * std::f64::consts::PI / 360.0).tan())) as f32)
    }

    /// parks the flat map under the ball at the spot and zoom the way back opens with
    /// tiles are loaded before the handover
    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let centre = self.centre;
        let zoom = self.zoom(Self::NEAREST * 0.9, centre.latitude);
        let on_settle = self.on_settle.clone();
        cx.defer_in(window, move |_, window, cx| on_settle(centre, zoom, window, cx));
    }

    pub fn update(&mut self, next: Vec<MapGlobePerson>, palette: MapPinPalette, cx: &mut Context<Self>) {
        let changed = next != self.people || Some(&palette) != self.palette.as_ref();
        self.people = next;
        self.palette = Some(palette.clone());
        if !changed {
            return;
        }
        let wanted: Vec<&str> = self.people.iter().map(|person| person.id.as_str()).collect();
        self.pins.retain(|id, _| wanted.contains(&id.as_str()));
        for person in &self.people {
            let pin = self.pins.entry(person.id.clone()).or_insert_with(|| MapGlobePinView::new(Self::PIN_SIDE));
            pin.configure(person, &palette, cx);
        }
        self.layout_pins();
        cx.notify();
    }

    pub fn apply_texture(&mut self, style_json: &str, key: &str, cx: &mut Context<Self>) {
        if self.texture_key.as_deref() == Some(key) {
            return;
        }
        self.texture_key = Some(key.to_string());
        let request = MapGlobeArt::texture(style_json.to_string(), key.to_string(), cx);
        let key = key.to_string();
        self.texture_task = Some(cx.spawn(async move |this, cx| {
            let image = request.await;
            let _ = this.update(cx, |globe, cx| {
                if globe.texture_key.as_deref() != Some(key.as_str()) {
                    return;
                }
                let Some(image) = image else {
                    globe.texture_key = None;
                    return;
                };
                globe.texture = Some(image);
                globe.request_ball(cx);
            });
        }));
    }

    fn handle_pan_began(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_spin(window, cx);
        self.is_following_map = false;
        self.pan = Some(MapGlobePan {
            last_position: position,
            last_time: RiseClock::monotonic(),
            velocity: Point::default(),
        });
    }

    fn handle_pan_changed(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(pan) = self.pan.as_mut() else {
            return;
        };
        let now = RiseClock::monotonic();
        let moved = point(
            f64::from(f32::from(position.x - pan.last_position.x)),
            f64::from(f32::from(position.y - pan.last_position.y)),
        );
        let seconds = now.saturating_sub(pan.last_time).as_secs_f64();
        if seconds > 0.0 {
            pan.velocity = point(moved.x / seconds, moved.y / seconds);
        }
        pan.last_position = position;
        pan.last_time = now;
        self.turn(moved, cx);
    }

    fn handle_pan_ended(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pan) = self.pan.take() else {
            return;
        };
        let held = RiseClock::monotonic().saturating_sub(pan.last_time) > Self::FLING_WINDOW;
        self.velocity = if held { Point::default() } else { pan.velocity };
        self.start_spin(window, cx);
        if self.spin.is_none() {
            self.settle(window, cx);
        }
    }

    fn handle_pinch(&mut self, phase: TouchPhase, delta: f64, window: &mut Window, cx: &mut Context<Self>) {
        match phase {
            TouchPhase::Started => {
                self.stop_spin(window, cx);
                self.is_following_map = false;
                self.pinch_start_distance = self.distance;
                self.pinch = Some(MapGlobePinch {
                    scale: 1.0,
                    last_time: RiseClock::monotonic(),
                    velocity: 0.0,
                });
            }
            TouchPhase::Moved => {
                let Some(pinch) = self.pinch.as_mut() else {
                    return;
                };
                let now = RiseClock::monotonic();
                let next = pinch.scale * (1.0 + delta);
                let seconds = now.saturating_sub(pinch.last_time).as_secs_f64();
                if seconds > 0.0 {
                    pinch.velocity = (next - pinch.scale) / seconds;
                }
                pinch.scale = next;
                pinch.last_time = now;
                let scale = pinch.scale;
                self.pinch_changed(scale, window, cx);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some(pinch) = self.pinch.take() else {
                    return;
                };
                self.pinch_ended(pinch.scale, pinch.velocity, window, cx);
            }
        }
    }

    fn pinch_changed(&mut self, scale: f64, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.pinch_start_distance / (scale.max(0.2) as f32);
        if wanted < Self::NEAREST * 0.9 && !self.has_dived_in {
            self.has_dived_in = true;
            self.distance = Self::NEAREST * 0.9;
            self.apply_camera(cx);
            let centre = self.centre;
            let zoom = self.zoom(self.distance, centre.latitude);
            let on_dive_in = self.on_dive_in.clone();
            cx.defer_in(window, move |_, window, cx| on_dive_in(centre, zoom, window, cx));
        }
        if self.has_dived_in {
            self.distance = (Self::NEAREST * 0.45).max(wanted);
            self.apply_camera(cx);
            let centre = self.centre;
            let zoom = self.zoom(self.distance, centre.latitude);
            cx.defer(move |cx| {
                MapGlobeCameraLink::shared(cx).update(cx, |link, cx| link.globe_camera_changed(centre, zoom, cx));
            });
            return;
        }
        self.distance = self.clamped(wanted);
        self.apply_camera(cx);
    }

    fn pinch_ended(&mut self, scale: f64, velocity: f64, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_dived_in {
            let scale = scale.max(0.2);
            let zoom_velocity = velocity / scale / std::f64::consts::LN_2;
            cx.defer(move |cx| {
                MapGlobeCameraLink::shared(cx).update(cx, |link, cx| link.coast_map(zoom_velocity, cx));
            });
        } else {
            self.settle(window, cx);
        }
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let zooms = event.modifiers.secondary() || event.modifiers.control;
        match event.delta {
            ScrollDelta::Lines(lines) => {
                self.zoom_by(f64::from(lines.y) * WHEEL_ZOOM_PER_LINE, window, cx);
            }
            ScrollDelta::Pixels(pixels) if zooms => {
                self.pan = None;
                let levels = (f64::from(f32::from(pixels.y)) / WHEEL_ZOOM_POINTS_PER_LEVEL).clamp(-WHEEL_ZOOM_CAP, WHEEL_ZOOM_CAP);
                self.zoom_by(levels, window, cx);
            }
            ScrollDelta::Pixels(pixels) => match event.touch_phase {
                TouchPhase::Started => self.handle_pan_began(event.position, window, cx),
                TouchPhase::Moved => {
                    if self.pan.is_none() {
                        self.handle_pan_began(event.position, window, cx);
                    }
                    let position = self.pan.as_ref().map_or(event.position, |pan| pan.last_position);
                    self.handle_pan_changed(position + pixels, cx);
                }
                TouchPhase::Ended => self.handle_pan_ended(window, cx),
                TouchPhase::Cancelled => self.pan = None,
            },
        }
    }

    /// one discrete zoom step run as a short pinch so diving into the flat map stays one path
    pub fn zoom_by(&mut self, levels: f64, window: &mut Window, cx: &mut Context<Self>) {
        if levels == 0.0 {
            return;
        }
        let target = self.zoom(self.distance, self.centre.latitude) + levels;
        self.handle_pinch(TouchPhase::Started, 0.0, window, cx);
        self.handle_pinch(TouchPhase::Moved, 2_f64.powf(levels) - 1.0, window, cx);
        if let Some(pinch) = self.pinch.as_mut() {
            pinch.velocity = 0.0;
        }
        self.handle_pinch(TouchPhase::Ended, 0.0, window, cx);
        if self.has_dived_in && levels > 0.0 {
            cx.defer(move |cx| {
                let Some(map) = MapGlobeCameraLink::shared(cx).read(cx).map_view() else {
                    return;
                };
                let target = map.maximum_zoom_level().min(target.max(map.zoom_level()));
                map.set_zoom_level(target, true, cx);
            });
        }
    }

    fn handle_double_click(&mut self, position: Point<Pixels>, zoom_out: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.pan = None;
        self.stop_spin(window, cx);
        self.is_following_map = false;
        if !zoom_out {
            let centre = self.window_centre();
            let offset = point(
                f64::from(f32::from(centre.x - position.x)) / 2.0,
                f64::from(f32::from(centre.y - position.y)) / 2.0,
            );
            self.turn(offset, cx);
        }
        self.zoom_by(if zoom_out { -1.0 } else { 1.0 }, window, cx);
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let levels = match event.keystroke.key.as_str() {
            "+" | "=" | "add" => 1.0,
            "-" | "subtract" => -1.0,
            _ => return,
        };
        cx.stop_propagation();
        self.zoom_by(levels, window, cx);
    }

    fn handle_pin_tap(&mut self, person_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.pins.get(person_id).and_then(|pin| pin.person_id.clone()) else {
            return;
        };
        let on_select_person = self.on_select_person.clone();
        cx.defer_in(window, move |_, window, cx| on_select_person(id, window, cx));
    }

    fn degrees_per_point(&self) -> f64 {
        self.degrees_per_point_at(self.distance)
    }

    fn degrees_per_point_at(&self, distance: f32) -> f64 {
        if self.width() <= 0.0 {
            return 0.2;
        }
        let visible = 2.0 * f64::from(distance - 1.0) * (Self::FIELD_OF_VIEW * std::f64::consts::PI / 360.0).tan();
        visible / self.width() * 180.0 / std::f64::consts::PI
    }

    fn turn(&mut self, moved: Point<f64>, cx: &mut Context<Self>) {
        let scale = self.degrees_per_point();
        let mut longitude = self.centre.longitude - moved.x * scale;
        if longitude > 180.0 {
            longitude -= 360.0;
        }
        if longitude < -180.0 {
            longitude += 360.0;
        }
        let latitude = (self.centre.latitude + moved.y * scale).clamp(-Self::LATITUDE_LIMIT, Self::LATITUDE_LIMIT);
        self.centre = LatLng::new(latitude, longitude);
        self.apply_camera(cx);
    }

    fn start_spin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.velocity.x.hypot(self.velocity.y) <= 40.0 || reduce_motion() {
            return;
        }
        self.spin = Some(MapGlobeSpin {
            last_frame: RiseClock::monotonic(),
        });
        Self::schedule_spin_frame(window, cx);
    }

    fn schedule_spin_frame(window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = this.update(cx, |globe, cx| globe.step_spin(window, cx));
        });
        cx.notify();
    }

    fn step_spin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(spin) = self.spin.as_mut() else {
            return;
        };
        let now = RiseClock::monotonic();
        let seconds = now.saturating_sub(spin.last_frame).as_secs_f64();
        spin.last_frame = now;
        self.turn(point(self.velocity.x * seconds, self.velocity.y * seconds), cx);
        let decay = 0.04_f64.powf(seconds);
        self.velocity = point(self.velocity.x * decay, self.velocity.y * decay);
        if self.velocity.x.hypot(self.velocity.y) < 8.0 {
            self.stop_spin(window, cx);
        } else {
            Self::schedule_spin_frame(window, cx);
        }
    }

    fn stop_spin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_coasting = self.spin.is_some();
        self.spin = None;
        self.velocity = Point::default();
        if was_coasting {
            self.settle(window, cx);
        }
    }

    fn set_bounds(&mut self, bounds: Bounds<Pixels>, scale_factor: f32, cx: &mut Context<Self>) {
        if self.bounds == bounds && self.scale_factor == scale_factor {
            return;
        }
        let resized = self.bounds.size != bounds.size || self.scale_factor != scale_factor;
        self.bounds = bounds;
        self.scale_factor = scale_factor;
        if resized {
            self.apply_camera(cx);
        }
    }

    fn apply_camera(&mut self, cx: &mut Context<Self>) {
        self.layout_pins();
        self.request_ball(cx);
        cx.notify();
    }

    /// draws the ball and halo for the current camera off the main thread
    /// one drawing at a time, a camera change meanwhile redraws with the newest camera
    fn request_ball(&mut self, cx: &mut Context<Self>) {
        if self.width() <= 0.0 {
            return;
        }
        if self.ball_task.is_some() {
            self.ball_is_stale = true;
            return;
        }
        let job = MapGlobeBallJob {
            texture: self.texture.clone(),
            centre: self.centre,
            distance: f64::from(self.distance),
            width: self.width(),
            scale_factor: f64::from(self.scale_factor.max(1.0)),
        };
        let drawing = cx.background_executor().spawn(async move { MapGlobeArt::sphere(&job) });
        self.ball_task = Some(cx.spawn(async move |this, cx| {
            let image = drawing.await;
            let _ = this.update(cx, |globe, cx| {
                globe.ball_task = None;
                if let Some(previous) = std::mem::replace(&mut globe.ball, image) {
                    cx.drop_image(previous, None);
                }
                cx.notify();
                if std::mem::take(&mut globe.ball_is_stale) {
                    globe.request_ball(cx);
                }
            });
        }));
    }

    fn glow_side(&self) -> f64 {
        MapGlobeArt::glow_side(f64::from(self.distance), self.width())
    }

    /// avatars fade towards the edge and vanish once their spot is behind the ball
    fn layout_pins(&mut self) {
        if self.width() <= 0.0 {
            return;
        }
        let eye = [0.0, 0.0, f64::from(self.distance)];
        let (width, height, centre, distance) = (self.width(), self.height(), self.centre, f64::from(self.distance));
        for person in &self.people {
            let Some(pin) = self.pins.get_mut(&person.id) else {
                continue;
            };
            let world = MapGlobeArt::world_position(MapGlobeArt::position(person.coordinate), centre);
            let to_eye = [eye[0] - world[0], eye[1] - world[1], eye[2] - world[2]];
            let length = (to_eye[0] * to_eye[0] + to_eye[1] * to_eye[1] + to_eye[2] * to_eye[2]).sqrt().max(0.0001);
            let facing = (world[0] * to_eye[0] + world[1] * to_eye[1] + world[2] * to_eye[2]) / length;
            let alpha = ((facing - 0.04) / 0.22).clamp(0.0, 1.0) as f32;
            pin.alpha = alpha;
            pin.is_user_interaction_enabled = alpha > 0.6;
            if alpha <= 0.0 {
                continue;
            }
            if let Some(projected) = MapGlobeArt::project(world, distance, width, height) {
                pin.center = projected;
            }
        }
    }

    /// pin center in window coords, none while its hidden behind the ball
    pub fn reaction_anchor(&self, user_id: &str) -> Option<Point<Pixels>> {
        let pin = self.pins.get(user_id)?;
        if pin.alpha <= 0.01 {
            return None;
        }
        Some(self.bounds.origin + point(px(pin.center.x as f32), px(pin.center.y as f32)))
    }

    /// globe's own center in window coords
    pub fn window_centre(&self) -> Point<Pixels> {
        self.bounds.center()
    }

    fn pin_elements(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let Some(palette) = self.palette.clone() else {
            return Vec::new();
        };
        let mut ordered: Vec<&MapGlobePerson> = self.people.iter().filter(|person| !person.is_self).collect();
        ordered.extend(self.people.iter().filter(|person| person.is_self));
        let mut elements = Vec::with_capacity(ordered.len());
        for person in ordered {
            let Some(pin) = self.pins.get(&person.id) else {
                continue;
            };
            if pin.alpha <= 0.0 {
                continue;
            }
            elements.push(pin.render(person, &palette, cx));
        }
        elements
    }
}

impl Render for MapGlobeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let side = self.glow_side() as f32;
        let (width, height) = (f32::from(self.bounds.size.width), f32::from(self.bounds.size.height));
        let measure = canvas(
            move |bounds, window, cx| {
                let scale_factor = window.scale_factor();
                let Some(globe) = this.upgrade() else {
                    return;
                };
                let link = MapGlobeCameraLink::shared(cx);
                if link.read(cx).globe().is_none_or(|current| current != globe) {
                    link.update(cx, |link, _| link.globe = Some(globe.downgrade()));
                }
                globe.update(cx, |globe, cx| globe.set_bounds(bounds, scale_factor, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let mut root = div()
            .id("map-globe-view")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x080a17))
            .child(img(STAR_FIELD.clone()).absolute().size_full().object_fit(ObjectFit::Cover))
            .child(measure);
        if let Some(ball) = self.ball.clone() {
            root = root.child(
                img(ball)
                    .absolute()
                    .left(px((width - side) / 2.0))
                    .top(px((height - side) / 2.0))
                    .size(px(side)),
            );
        }
        root.children(self.pin_elements(cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|globe, event: &MouseDownEvent, window, cx| {
                    window.focus(&globe.focus_handle, cx);
                    if event.click_count == 2 {
                        globe.handle_double_click(event.position, event.modifiers.shift, window, cx);
                    } else {
                        globe.handle_pan_began(event.position, window, cx);
                    }
                }),
            )
            .on_mouse_move(cx.listener(|globe, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    globe.handle_pan_changed(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|globe, _: &MouseUpEvent, window, cx| globe.handle_pan_ended(window, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|globe, _: &MouseUpEvent, window, cx| globe.handle_pan_ended(window, cx)),
            )
            .on_pinch(cx.listener(|globe, event: &PinchEvent, window, cx| {
                globe.handle_pinch(event.phase, f64::from(event.delta), window, cx);
            }))
            .on_scroll_wheel(cx.listener(|globe, event: &ScrollWheelEvent, window, cx| {
                globe.handle_scroll_wheel(event, window, cx);
            }))
            .on_key_down(cx.listener(|globe, event: &KeyDownEvent, window, cx| {
                globe.handle_key_down(event, window, cx);
            }))
    }
}

/// avatar on the globe, ring and face only since theres no room for tiles
pub struct MapGlobePinView {
    side: f32,
    ring_width: f32,
    person_id: Option<String>,
    is_self_pin: bool,
    alpha: f32,
    is_user_interaction_enabled: bool,
    center: Point<f64>,
    avatar: MapAvatarView,
}

impl MapGlobePinView {
    pub fn new(side: f32) -> Self {
        Self {
            side: side + 4.0,
            ring_width: 2.0,
            person_id: None,
            is_self_pin: false,
            alpha: 0.0,
            is_user_interaction_enabled: false,
            center: Point::default(),
            avatar: MapAvatarView::new(),
        }
    }

    pub fn person_id(&self) -> Option<&str> {
        self.person_id.as_deref()
    }

    pub fn is_self_pin(&self) -> bool {
        self.is_self_pin
    }

    pub fn configure(&mut self, person: &MapGlobePerson, palette: &MapPinPalette, cx: &mut Context<MapGlobeView>) {
        self.person_id = Some(person.id.clone());
        self.is_self_pin = person.is_self;
        let diameter = self.side - 2.0 * self.ring_width;
        self.avatar.configure(&person.logo, &person.name, &person.id, diameter, palette, cx);
    }

    fn render(&self, person: &MapGlobePerson, palette: &MapPinPalette, cx: &mut Context<MapGlobeView>) -> gpui::AnyElement {
        let side = self.side;
        let diameter = side - 2.0 * self.ring_width;
        let ring = if person.is_self { palette.primary } else { palette.ring };
        let id = person.id.clone();
        let interactive = self.is_user_interaction_enabled;
        div()
            .id(gpui::ElementId::Name(format!("map-globe-pin-{}", person.id).into()))
            .absolute()
            .left(px(self.center.x as f32 - side / 2.0))
            .top(px(self.center.y as f32 - side / 2.0))
            .size(px(side))
            .rounded_full()
            .bg(ring)
            .opacity(self.alpha)
            .shadow(vec![BoxShadow {
                color: rgba(0x0000_0059).into(),
                offset: point(px(0.), px(1.)),
                blur_radius: px(3.),
                spread_radius: px(0.),
                inset: false,
            }])
            .p(px(self.ring_width))
            .child(self.avatar.element(diameter, 1.0))
            .when(interactive, move |pin| {
                pin.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(
                    move |globe, _, window, cx| globe.handle_pin_tap(&id, window, cx),
                ))
            })
            .into_any_element()
    }
}

/// one drawing of the ball, the camera and the texture it samples
struct MapGlobeBallJob {
    texture: Option<Arc<RgbaImage>>,
    centre: LatLng,
    distance: f64,
    width: f64,
    scale_factor: f64,
}

/// generated parts of the globe, not loaded
/// ball, stars and world texture
pub enum MapGlobeArt {}

static MEMORY: LazyLock<Mutex<HashMap<String, Arc<RgbaImage>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

static STAR_FIELD: LazyLock<Arc<RenderImage>> = LazyLock::new(MapGlobeArt::star_field);

impl MapGlobeArt {
    const MAX_MERCATOR_LATITUDE: f64 = 85.051_128_78;
    const TEXTURE_ZOOM: f64 = 2.0;
    const TEXTURE_POINTS: u32 = 2_048;
    const MAX_BALL_PIXELS: f64 = 1_600.0;
    const GLOW_LOCATIONS: [f64; 5] = [0.0, 0.8, 0.835, 0.88, 1.0];
    const GLOW_COLOURS: [[f64; 4]; 5] = [
        [1.0, 1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0, 0.85],
        [0.7, 0.82, 1.0, 0.28],
        [1.0, 1.0, 1.0, 0.0],
    ];

    /// point on the unit sphere, longitude zero faces the camera and north is up
    pub fn position(coordinate: LatLng) -> [f64; 3] {
        let latitude = coordinate.latitude.to_radians();
        let longitude = coordinate.longitude.to_radians();
        [latitude.cos() * longitude.sin(), latitude.sin(), latitude.cos() * longitude.cos()]
    }

    fn world_position(local: [f64; 3], centre: LatLng) -> [f64; 3] {
        let (sin_lon, cos_lon) = (-centre.longitude.to_radians()).sin_cos();
        let x = local[0] * cos_lon + local[2] * sin_lon;
        let z = -local[0] * sin_lon + local[2] * cos_lon;
        let y = local[1];
        let (sin_lat, cos_lat) = centre.latitude.to_radians().sin_cos();
        [x, y * cos_lat - z * sin_lat, y * sin_lat + z * cos_lat]
    }

    fn local_position(world: [f64; 3], centre: LatLng) -> [f64; 3] {
        let (sin_lat, cos_lat) = (-centre.latitude.to_radians()).sin_cos();
        let y = world[1] * cos_lat - world[2] * sin_lat;
        let z = world[1] * sin_lat + world[2] * cos_lat;
        let x = world[0];
        let (sin_lon, cos_lon) = centre.longitude.to_radians().sin_cos();
        [x * cos_lon + z * sin_lon, y, -x * sin_lon + z * cos_lon]
    }

    /// focal length in points of a camera whose horizontal view spans width
    fn focal_length(width: f64) -> f64 {
        (width / 2.0) / (MapGlobeView::FIELD_OF_VIEW.to_radians() / 2.0).tan()
    }

    /// projects a world point to view points with the camera on the z axis
    fn project(world: [f64; 3], distance: f64, width: f64, height: f64) -> Option<Point<f64>> {
        let depth = distance - world[2];
        if depth <= 0.0 {
            return None;
        }
        let focal = Self::focal_length(width);
        Some(point(width / 2.0 + focal * world[0] / depth, height / 2.0 - focal * world[1] / depth))
    }

    fn ball_radius(distance: f64, width: f64) -> f64 {
        let half = MapGlobeView::FIELD_OF_VIEW.to_radians() / 2.0;
        let angular = (1.0 / distance.max(1.0001)).asin();
        angular.tan() / half.tan() * width / 2.0
    }

    fn glow_side(distance: f64, width: f64) -> f64 {
        Self::ball_radius(distance, width) / 0.835 * 2.0
    }

    fn mercator_v(latitude: f64) -> f64 {
        let clamped = latitude.clamp(-Self::MAX_MERCATOR_LATITUDE, Self::MAX_MERCATOR_LATITUDE).to_radians();
        (0.5 - (std::f64::consts::FRAC_PI_4 + clamped / 2.0).tan().ln() / (2.0 * std::f64::consts::PI)).clamp(0.0, 1.0)
    }

    fn sample(texture: &RgbaImage, u: f64, v: f64) -> [f64; 3] {
        let (width, height) = texture.dimensions();
        let x = u * f64::from(width) - 0.5;
        let y = (v * f64::from(height) - 0.5).clamp(0.0, f64::from(height - 1));
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let column = |offset: f64| ((x0 + offset).rem_euclid(f64::from(width))) as u32;
        let row = |offset: f64| ((y0 + offset).min(f64::from(height - 1))) as u32;
        let mut colour = [0.0; 3];
        for (dx, dy, weight) in [(0.0, 0.0, (1.0 - fx) * (1.0 - fy)), (1.0, 0.0, fx * (1.0 - fy)), (0.0, 1.0, (1.0 - fx) * fy), (1.0, 1.0, fx * fy)] {
            let pixel = texture.get_pixel(column(dx), row(dy)).0;
            for channel in 0..3 {
                colour[channel] += f64::from(pixel[channel]) * weight;
            }
        }
        colour
    }

    fn glow(t: f64) -> [f64; 4] {
        let locations = Self::GLOW_LOCATIONS;
        if t >= 1.0 {
            return Self::GLOW_COLOURS[4];
        }
        for index in 1..locations.len() {
            if t <= locations[index] {
                let span = (locations[index] - locations[index - 1]).max(f64::EPSILON);
                let f = ((t - locations[index - 1]) / span).clamp(0.0, 1.0);
                let (a, b) = (Self::GLOW_COLOURS[index - 1], Self::GLOW_COLOURS[index]);
                return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f];
            }
        }
        Self::GLOW_COLOURS[4]
    }

    fn sphere(job: &MapGlobeBallJob) -> Option<Arc<RenderImage>> {
        let side = Self::glow_side(job.distance, job.width);
        if side <= 0.0 || !side.is_finite() {
            return None;
        }
        let pixels = (side * job.scale_factor).min(Self::MAX_BALL_PIXELS).ceil().max(1.0) as u32;
        let points_per_pixel = side / f64::from(pixels);
        let radius = Self::ball_radius(job.distance, job.width);
        let focal = Self::focal_length(job.width);
        let half = side / 2.0;
        let mut buffer = RgbaImage::new(pixels, pixels);
        for (column, row, pixel) in buffer.enumerate_pixels_mut() {
            let x = (f64::from(column) + 0.5) * points_per_pixel - half;
            let y = (f64::from(row) + 0.5) * points_per_pixel - half;
            let from_centre = x.hypot(y);
            let glow = Self::glow(from_centre / half);
            let coverage = ((radius - from_centre) / points_per_pixel + 0.5).clamp(0.0, 1.0);
            let mut colour = [glow[0] * 255.0, glow[1] * 255.0, glow[2] * 255.0];
            let mut alpha = glow[3];
            if coverage > 0.0 {
                let ball = Self::ball_colour(job, x, y, focal);
                let blended = coverage + alpha * (1.0 - coverage);
                for channel in 0..3 {
                    let premultiplied = ball[channel] * coverage + colour[channel] * alpha * (1.0 - coverage);
                    colour[channel] = (premultiplied / blended.max(f64::EPSILON)).min(255.0);
                }
                alpha = blended;
            }
            *pixel = image::Rgba([
                colour[2].round() as u8,
                colour[1].round() as u8,
                colour[0].round() as u8,
                (alpha * 255.0).round() as u8,
            ]);
        }
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
        Some(Arc::new(RenderImage::new(frames)))
    }

    fn ball_colour(job: &MapGlobeBallJob, x: f64, y: f64, focal: f64) -> [f64; 3] {
        let Some(texture) = job.texture.as_deref() else {
            return MapGlobeView::BALL_COLOUR.map(f64::from);
        };
        let length = (x * x + y * y + focal * focal).sqrt();
        let direction = [x / length, -y / length, -focal / length];
        let eye = [0.0, 0.0, job.distance];
        let b = eye[2] * direction[2];
        let discriminant = (b * b - (job.distance * job.distance - 1.0)).max(0.0);
        let t = -b - discriminant.sqrt();
        let world = [direction[0] * t, direction[1] * t, eye[2] + direction[2] * t];
        let local = Self::local_position(world, job.centre);
        let latitude = local[1].clamp(-1.0, 1.0).asin().to_degrees();
        let longitude = local[0].atan2(local[2]).to_degrees();
        Self::sample(texture, (longitude + 180.0) / 360.0, Self::mercator_v(latitude))
    }

    fn star_field() -> Arc<RenderImage> {
        let (width, height) = (512_u32, 1_024_u32);
        let mut buffer = RgbaImage::new(width, height);
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f64 / 10_000.0
        };
        for _ in 0..420 {
            let radius = 0.4 + next() * 0.9;
            let x = next() * f64::from(width);
            let y = next() * f64::from(height);
            let alpha = 0.25 + next() * 0.6;
            let coverage = (radius * radius * std::f64::consts::FRAC_PI_4).min(1.0);
            let (column, row) = ((x + radius / 2.0) as u32, (y + radius / 2.0) as u32);
            if column < width && row < height {
                let value = (alpha * coverage * 255.0).round() as u8;
                buffer.put_pixel(column, row, image::Rgba([255, 255, 255, value]));
            }
        }
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
        Arc::new(RenderImage::new(frames))
    }

    fn folder() -> Option<PathBuf> {
        AppPaths::resolve(AppEnvironment::compiled().canon())
            .ok()
            .map(|paths| paths.cache_dir().join("map-globe"))
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }

    /// gets the texture from memory
    /// then the caches folder
    /// then one snapshot of the whole world zoomed to fill a square exactly
    pub fn texture(style_json: String, key: String, cx: &mut App) -> Task<Option<Arc<RgbaImage>>> {
        if let Some(hit) = MEMORY.lock().get(&key).cloned() {
            return Task::ready(Some(hit));
        }
        cx.background_executor().spawn(async move {
            let folder = Self::folder()?;
            let file = folder.join(format!("{key}-z{}.jpg", Self::TEXTURE_ZOOM));
            if let Some(stored) = std::fs::read(&file).ok().and_then(|data| image::load_from_memory(&data).ok()) {
                let stored = Arc::new(stored.into_rgba8());
                MEMORY.lock().insert(key, stored.clone());
                return Some(stored);
            }
            std::fs::create_dir_all(&folder).ok()?;
            let paths = AppPaths::resolve(AppEnvironment::compiled().canon()).ok()?;
            let request = SnapshotRequest {
                style_json,
                width: Self::TEXTURE_POINTS,
                height: Self::TEXTURE_POINTS,
                scale_factor: 2.0,
                camera: Camera::new(LatLng::new(0.0, 0.0), Self::TEXTURE_ZOOM),
            };
            let frame = snapshot(&request, &paths).ok()?;
            let mut rgba = frame.bgra;
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            let image = Arc::new(RgbaImage::from_raw(frame.width, frame.height, rgba)?);
            MEMORY.lock().insert(key, image.clone());
            let rgb = DynamicImage::ImageRgba8((*image).clone()).into_rgb8();
            let mut data = Vec::new();
            if JpegEncoder::new_with_quality(&mut data, 90).encode_image(&rgb).is_ok() {
                let _ = Self::write_atomically(&file, &data);
            }
            Some(image)
        })
    }
}

use std::f64::consts::PI;
use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use rise_platform::{AppPaths, HostOs, PlatformSupport};
use thiserror::Error;

use crate::texture::frame::{FrameFormat, FrameGeometry, GeometryError};

pub const TILE_SIZE: f64 = 512.0;
pub const MAX_LATITUDE: f64 = 85.051_128_779_806_59;
pub const MIN_ZOOM: f64 = 0.0;
pub const MAX_ZOOM: f64 = 20.0;
pub const MAX_PITCH: f64 = 60.0;
pub const FIELD_OF_VIEW: f64 = 0.643_501_108_793_284_4;
pub const DEFAULT_ANIMATION_SECS: f64 = 0.3;
pub const KEY_PAN_POINTS: f64 = 150.0;
pub const KEY_ROTATE_DEGREES: f64 = 25.0;
pub const DRAG_BEARING_PER_POINT: f64 = 0.8;
pub const DRAG_PITCH_PER_POINT: f64 = -0.5;
pub const WHEEL_ZOOM_PER_LINE: f64 = 0.5;
pub const WHEEL_ZOOM_CAP: f64 = 1.0;
pub const WHEEL_ZOOM_POINTS_PER_LEVEL: f64 = 120.0;
pub const FLY_CURVE: f64 = 1.42;
pub const FLY_SPEED: f64 = 1.2;

/// geographic coordinate in degrees
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct LatLng {
    pub latitude: f64,
    pub longitude: f64,
}

impl LatLng {
    pub const fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }
}

/// point in logical pixels from the top left of the map element
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

impl ScreenPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// map element size in logical pixels plus the window scale factor
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
    pub scale_factor: f32,
}

impl Viewport {
    pub fn center(&self) -> ScreenPoint {
        ScreenPoint::new(self.width / 2.0, self.height / 2.0)
    }

    /// device pixel size the renderer draws into
    pub fn pixel_size(&self) -> (u32, u32) {
        let scale = f64::from(self.scale_factor.max(1.0));
        (
            (self.width * scale).round().max(1.0) as u32,
            (self.height * scale).round().max(1.0) as u32,
        )
    }
}

/// map camera, center and zoom and direction and pitch
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera {
    pub center: LatLng,
    pub zoom: f64,
    pub bearing: f64,
    pub pitch: f64,
}

impl Camera {
    pub const fn new(center: LatLng, zoom: f64) -> Self {
        Self {
            center,
            zoom,
            bearing: 0.0,
            pitch: 0.0,
        }
    }

    /// camera the map would really hold for this one
    /// zoom and pitch inside limits and latitude inside the mercator square
    /// longitude and bearing wrapped
    pub fn clamped(self) -> Self {
        Self {
            center: LatLng::new(
                self.center.latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE),
                wrap_longitude(self.center.longitude),
            ),
            zoom: self.zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            bearing: normalize_bearing(self.bearing),
            pitch: self.pitch.clamp(0.0, MAX_PITCH),
        }
    }

    /// width of the whole world in logical pixels at this zoom
    pub fn world_size(&self) -> f64 {
        TILE_SIZE * 2f64.powf(self.zoom)
    }
}

/// wraps a longitude into -180..180
pub fn wrap_longitude(longitude: f64) -> f64 {
    let wrapped = (longitude + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 && longitude > 0.0 {
        180.0
    } else {
        wrapped
    }
}

/// wraps a bearing into 0..360
pub fn normalize_bearing(bearing: f64) -> f64 {
    let wrapped = bearing.rem_euclid(360.0);
    if wrapped >= 360.0 { 0.0 } else { wrapped }
}

/// signed shortest turn from one bearing to another, in -180..180
pub fn shortest_turn(from: f64, to: f64) -> f64 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

/// web mercator in the unit square, x grows east and y grows south
pub fn mercator(coordinate: LatLng) -> (f64, f64) {
    let latitude = coordinate.latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE).to_radians();
    let x = (coordinate.longitude + 180.0) / 360.0;
    let y = (1.0 - (PI / 4.0 + latitude / 2.0).tan().ln() / PI) / 2.0;
    (x, y)
}

/// inverse of mercator, longitude is wrapped into -180..180
pub fn inverse_mercator(x: f64, y: f64) -> LatLng {
    let longitude = x * 360.0 - 180.0;
    let latitude = (2.0 * (PI * (1.0 - 2.0 * y)).exp().atan() - PI / 2.0).to_degrees();
    LatLng::new(latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE), wrap_longitude(longitude))
}

/// converts between screen points and coordinates for one camera and size
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Projection {
    pub camera: Camera,
    pub viewport: Viewport,
}

impl Projection {
    pub fn new(camera: Camera, viewport: Viewport) -> Self {
        Self {
            camera: camera.clamped(),
            viewport,
        }
    }

    /// distance from the eye to the map center in logical pixels, so the center draws at scale 1
    fn eye_distance(&self) -> f64 {
        0.5 / (FIELD_OF_VIEW / 2.0).tan() * self.viewport.height
    }

    fn center_world(&self) -> (f64, f64) {
        let (x, y) = mercator(self.camera.center);
        let size = self.camera.world_size();
        (x * size, y * size)
    }

    /// offset from the map center of the ground point drawn at this screen point
    /// in world pixels east and south, none above the horizon of a pitched map
    pub fn ground_offset(&self, point: ScreenPoint) -> Option<(f64, f64)> {
        let center = self.viewport.center();
        let (sx, sy) = (point.x - center.x, point.y - center.y);
        let pitch = self.camera.pitch.to_radians();
        let distance = self.eye_distance();
        let denominator = distance * pitch.cos() + sy * pitch.sin();
        if denominator <= f64::EPSILON * distance {
            return None;
        }
        let down = sy * distance / denominator;
        let depth = distance - down * pitch.sin();
        let across = sx * depth / distance;
        let bearing = self.camera.bearing.to_radians();
        Some((
            across * bearing.cos() - down * bearing.sin(),
            across * bearing.sin() + down * bearing.cos(),
        ))
    }

    pub fn screen_of_offset(&self, offset: (f64, f64)) -> Option<ScreenPoint> {
        let (dx, dy) = offset;
        let bearing = self.camera.bearing.to_radians();
        let across = dx * bearing.cos() + dy * bearing.sin();
        let down = -dx * bearing.sin() + dy * bearing.cos();
        let pitch = self.camera.pitch.to_radians();
        let distance = self.eye_distance();
        let depth = distance - down * pitch.sin();
        if depth <= f64::EPSILON * distance {
            return None;
        }
        let center = self.viewport.center();
        Some(ScreenPoint::new(
            center.x + across * distance / depth,
            center.y + down * pitch.cos() * distance / depth,
        ))
    }

    /// screen point of a coordinate, uses the world copy nearest the center
    pub fn project(&self, coordinate: LatLng) -> Option<ScreenPoint> {
        let size = self.camera.world_size();
        let (cx, cy) = self.center_world();
        let (x, y) = mercator(coordinate);
        let mut dx = x * size - cx;
        dx -= (dx / size).round() * size;
        self.screen_of_offset((dx, y * size - cy))
    }

    /// coordinate drawn at a screen point, none above the horizon
    pub fn unproject(&self, point: ScreenPoint) -> Option<LatLng> {
        let (dx, dy) = self.ground_offset(point)?;
        let size = self.camera.world_size();
        let (cx, cy) = self.center_world();
        Some(inverse_mercator((cx + dx) / size, ((cy + dy) / size).clamp(0.0, 1.0)))
    }

    /// camera that keeps all but the center and draws the coordinate at the point
    pub fn place_under(&self, coordinate: LatLng, point: ScreenPoint) -> Option<Camera> {
        let (dx, dy) = self.ground_offset(point)?;
        let size = self.camera.world_size();
        let (x, y) = mercator(coordinate);
        let center = inverse_mercator(x - dx / size, (y - dy / size).clamp(0.0, 1.0));
        Some(Camera {
            center,
            ..self.camera
        })
    }
}

/// why the camera moved, a bit set with the same bits as maplibre ios
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct CameraChangeReason(pub u16);

impl CameraChangeReason {
    pub const NONE: Self = Self(0);
    pub const PROGRAMMATIC: Self = Self(1 << 0);
    pub const RESET_NORTH: Self = Self(1 << 1);
    pub const GESTURE_PAN: Self = Self(1 << 2);
    pub const GESTURE_PINCH: Self = Self(1 << 3);
    pub const GESTURE_ROTATE: Self = Self(1 << 4);
    pub const GESTURE_ZOOM_IN: Self = Self(1 << 5);
    pub const GESTURE_ZOOM_OUT: Self = Self(1 << 6);
    pub const GESTURE_ONE_FINGER_ZOOM: Self = Self(1 << 7);
    pub const GESTURE_TILT: Self = Self(1 << 8);
    pub const TRANSITION_CANCELLED: Self = Self(1 << 9);
    pub const ZOOM_GESTURES: Self = Self(
        Self::GESTURE_PINCH.0
            | Self::GESTURE_ZOOM_IN.0
            | Self::GESTURE_ZOOM_OUT.0
            | Self::GESTURE_ONE_FINGER_ZOOM.0,
    );

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_gesture(self) -> bool {
        self.0 & !(Self::PROGRAMMATIC.0 | Self::RESET_NORTH.0 | Self::TRANSITION_CANCELLED.0) != 0
    }
}

/// interpolation of an animated camera change
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AnimationCurve {
    Ease,
    Fly(FlyPath),
}

/// precomputed path of a fly animation, in units of the start screen
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FlyPath {
    start_width: f64,
    travel: f64,
    r0: f64,
    length: f64,
    zoom_only: Option<f64>,
}

impl FlyPath {
    fn new(from: &Camera, to: &Camera, viewport: &Viewport) -> Self {
        let rho = FLY_CURVE;
        let rho2 = rho * rho;
        let start_width = viewport.width.max(viewport.height).max(1.0);
        let end_width = start_width / 2f64.powf(to.zoom - from.zoom);
        let travel = world_distance(from, to, from.world_size());
        let r = |width: f64, sign: f64| {
            let b = (end_width * end_width - start_width * start_width
                + sign * rho2 * rho2 * travel * travel)
                / (2.0 * width * rho2 * travel);
            (-b + (b * b + 1.0).sqrt()).ln()
        };
        let r0 = r(start_width, 1.0);
        let r1 = r(end_width, -1.0);
        let length = (r1 - r0) / rho;
        if travel.abs() < 1e-6 || !length.is_finite() {
            let zoom_only = if end_width < start_width { 1.0 } else { -1.0 };
            return Self {
                start_width,
                travel: 0.0,
                r0: 0.0,
                length: (end_width / start_width).ln().abs() / rho,
                zoom_only: Some(zoom_only),
            };
        }
        Self {
            start_width,
            travel,
            r0,
            length,
            zoom_only: None,
        }
    }

    /// screenfuls the path covers, animation lasts length / FLY_SPEED seconds
    pub fn length(&self) -> f64 {
        self.length
    }

    /// visible width and travelled fraction of the way at path position s
    fn at(&self, s: f64) -> (f64, f64) {
        let rho = FLY_CURVE;
        if let Some(sign) = self.zoom_only {
            return (self.start_width * (-sign * rho * s).exp(), 0.0);
        }
        let width = self.start_width * self.r0.cosh() / (self.r0 + rho * s).cosh();
        let travelled = self.start_width
            * (self.r0.cosh() * (self.r0 + rho * s).tanh() - self.r0.sinh())
            / (rho * rho);
        (width, travelled / self.travel)
    }
}

fn world_distance(from: &Camera, to: &Camera, size: f64) -> f64 {
    let (ax, ay) = mercator(from.center);
    let (bx, by) = mercator(to.center);
    let mut dx = bx - ax;
    dx -= dx.round();
    ((dx * size).powi(2) + ((by - ay) * size).powi(2)).sqrt()
}

/// one animated camera change, the map element samples it every frame it draws
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraAnimation {
    pub from: Camera,
    pub to: Camera,
    pub duration_secs: f64,
    pub curve: AnimationCurve,
}

impl CameraAnimation {
    pub fn ease(from: Camera, to: Camera, duration_secs: f64) -> Self {
        Self {
            from: from.clamped(),
            to: to.clamped(),
            duration_secs: duration_secs.max(0.0),
            curve: AnimationCurve::Ease,
        }
    }

    pub fn fly(from: Camera, to: Camera, viewport: &Viewport) -> Self {
        let (from, to) = (from.clamped(), to.clamped());
        let path = FlyPath::new(&from, &to, viewport);
        Self {
            from,
            to,
            duration_secs: path.length() / FLY_SPEED,
            curve: AnimationCurve::Fly(path),
        }
    }

    pub fn is_finished(&self, elapsed_secs: f64) -> bool {
        elapsed_secs >= self.duration_secs
    }

    /// camera at elapsed_secs into the animation, the target once it is over
    pub fn sample(&self, elapsed_secs: f64) -> Camera {
        if self.is_finished(elapsed_secs) || self.duration_secs <= 0.0 {
            return self.to;
        }
        let t = (elapsed_secs / self.duration_secs).clamp(0.0, 1.0);
        match self.curve {
            AnimationCurve::Ease => {
                let k = UnitBezier::new(0.0, 0.0, 0.25, 1.0).solve(t);
                let zoom = lerp(self.from.zoom, self.to.zoom, k);
                self.blend(k, zoom, k)
            }
            AnimationCurve::Fly(path) => {
                let k = UnitBezier::new(0.25, 0.1, 0.25, 1.0).solve(t);
                let (width, travelled) = path.at(k * path.length());
                let zoom = self.from.zoom + (path.start_width / width).log2();
                let fraction = if path.zoom_only.is_some() {
                    k
                } else {
                    travelled
                };
                self.blend(fraction, zoom, k)
            }
        }
    }

    fn blend(&self, fraction: f64, zoom: f64, k: f64) -> Camera {
        let (ax, ay) = mercator(self.from.center);
        let (bx, by) = mercator(self.to.center);
        let mut dx = bx - ax;
        dx -= dx.round();
        Camera {
            center: inverse_mercator(ax + dx * fraction, lerp(ay, by, fraction)),
            zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            bearing: normalize_bearing(
                self.from.bearing + shortest_turn(self.from.bearing, self.to.bearing) * k,
            ),
            pitch: lerp(self.from.pitch, self.to.pitch, k),
        }
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// css cubic bezier timing curve from the origin to the corner
/// solved for x like webkit unitbezier
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UnitBezier {
    cx: f64,
    bx: f64,
    ax: f64,
    cy: f64,
    by: f64,
    ay: f64,
}

impl UnitBezier {
    pub fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        Self {
            cx,
            bx,
            ax: 1.0 - cx - bx,
            cy,
            by,
            ay: 1.0 - cy - by,
        }
    }

    fn curve_x(&self, t: f64) -> f64 {
        ((self.ax * t + self.bx) * t + self.cx) * t
    }

    fn curve_y(&self, t: f64) -> f64 {
        ((self.ay * t + self.by) * t + self.cy) * t
    }

    fn slope_x(&self, t: f64) -> f64 {
        (3.0 * self.ax * t + 2.0 * self.bx) * t + self.cx
    }

    /// curve y at horizontal position x in 0..1
    pub fn solve(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let epsilon = 1e-7;
        let mut t = x;
        for _ in 0..8 {
            let error = self.curve_x(t) - x;
            if error.abs() < epsilon {
                return self.curve_y(t);
            }
            let slope = self.slope_x(t);
            if slope.abs() < 1e-6 {
                break;
            }
            t -= error / slope;
        }
        let (mut low, mut high) = (0.0, 1.0);
        t = x;
        while low < high {
            let value = self.curve_x(t);
            if (value - x).abs() < epsilon {
                break;
            }
            if x > value {
                low = t;
            } else {
                high = t;
            }
            let next = (high - low) / 2.0 + low;
            if (next - t).abs() < f64::EPSILON {
                break;
            }
            t = next;
        }
        self.curve_y(t)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragButton {
    Primary,
    Secondary,
}

/// modifier keys held during an input event
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InputModifiers {
    pub secondary: bool,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

/// how far a wheel or trackpad scrolled
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum WheelDelta {
    Pixels { x: f64, y: f64 },
    Lines { x: f64, y: f64 },
}

/// keys the map answers when it has focus
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapKey {
    ZoomIn,
    ZoomOut,
    Up,
    Down,
    Left,
    Right,
}

/// pointer and keyboard input in map element coordinates
/// the gpui component turns its mouse and scroll and pinch and key events into these
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MapInput {
    DragStart {
        point: ScreenPoint,
        button: DragButton,
        modifiers: InputModifiers,
    },
    DragMove {
        point: ScreenPoint,
    },
    DragEnd,
    Wheel {
        point: ScreenPoint,
        delta: WheelDelta,
        modifiers: InputModifiers,
    },
    Pinch {
        point: ScreenPoint,
        delta: f64,
    },
    DoubleClick {
        point: ScreenPoint,
        modifiers: InputModifiers,
    },
    Key {
        key: MapKey,
        modifiers: InputModifiers,
    },
}

/// what the map element does with the camera after an input
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CameraCommand {
    Jump {
        camera: Camera,
        reason: CameraChangeReason,
    },
    Animate {
        animation: CameraAnimation,
        reason: CameraChangeReason,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    Pan { grabbed: LatLng },
    Rotate { last: ScreenPoint },
}

/// turns input into camera commands, remembers the drag in progress between events
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct MapInputController {
    drag: Option<Drag>,
}

impl MapInputController {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// command for one input, none when the map leaves the event to someone else
    /// any key but plus and minus with the secondary modifier, so cmd and ctrl shortcuts reach the app
    pub fn handle(
        &mut self,
        input: MapInput,
        camera: &Camera,
        viewport: &Viewport,
    ) -> Option<CameraCommand> {
        let projection = Projection::new(*camera, *viewport);
        match input {
            MapInput::DragStart {
                point,
                button,
                modifiers,
            } => {
                self.drag = if button == DragButton::Secondary || modifiers.control {
                    Some(Drag::Rotate { last: point })
                } else {
                    projection
                        .unproject(point)
                        .map(|grabbed| Drag::Pan { grabbed })
                };
                None
            }
            MapInput::DragMove { point } => match self.drag? {
                Drag::Pan { grabbed } => {
                    let camera = projection.place_under(grabbed, point)?;
                    Some(jump(camera, CameraChangeReason::GESTURE_PAN))
                }
                Drag::Rotate { last } => {
                    self.drag = Some(Drag::Rotate { last: point });
                    let camera = Camera {
                        bearing: projection.camera.bearing
                            + (point.x - last.x) * DRAG_BEARING_PER_POINT,
                        pitch: projection.camera.pitch + (point.y - last.y) * DRAG_PITCH_PER_POINT,
                        ..projection.camera
                    };
                    Some(jump(
                        camera,
                        CameraChangeReason::GESTURE_ROTATE.union(CameraChangeReason::GESTURE_TILT),
                    ))
                }
            },
            MapInput::DragEnd => {
                self.drag = None;
                None
            }
            MapInput::Wheel {
                point,
                delta,
                modifiers,
            } => {
                let zooms = modifiers.secondary || modifiers.control;
                match delta {
                    WheelDelta::Pixels { y, .. } if zooms => {
                        let levels = (y / WHEEL_ZOOM_POINTS_PER_LEVEL).clamp(-WHEEL_ZOOM_CAP, WHEEL_ZOOM_CAP);
                        let camera = zoom_around(&projection, levels, point)?;
                        Some(jump(camera, CameraChangeReason::GESTURE_PINCH))
                    }
                    WheelDelta::Pixels { x, y } => {
                        let grabbed = projection.unproject(viewport.center())?;
                        let target = ScreenPoint::new(viewport.center().x + x, viewport.center().y + y);
                        let camera = projection.place_under(grabbed, target)?;
                        Some(jump(camera, CameraChangeReason::GESTURE_PAN))
                    }
                    WheelDelta::Lines { y, .. } => {
                        let levels = (y * WHEEL_ZOOM_PER_LINE).clamp(-WHEEL_ZOOM_CAP, WHEEL_ZOOM_CAP);
                        let camera = zoom_around(&projection, levels, point)?;
                        Some(jump(camera, CameraChangeReason::GESTURE_PINCH))
                    }
                }
            }
            MapInput::Pinch { point, delta } => {
                if delta <= -1.0 {
                    return None;
                }
                let camera = zoom_around(&projection, (1.0 + delta).log2(), point)?;
                Some(jump(camera, CameraChangeReason::GESTURE_PINCH))
            }
            MapInput::DoubleClick { point, modifiers } => {
                let (levels, reason) = if modifiers.alt {
                    (-1.0, CameraChangeReason::GESTURE_ZOOM_OUT)
                } else {
                    (1.0, CameraChangeReason::GESTURE_ZOOM_IN)
                };
                let target = zoom_around(&projection, levels, point)?;
                Some(animate(projection.camera, target, reason))
            }
            MapInput::Key { key, modifiers } => {
                if modifiers.secondary && !matches!(key, MapKey::ZoomIn | MapKey::ZoomOut) {
                    return None;
                }
                key_command(&projection, key, modifiers)
            }
        }
    }
}

fn jump(camera: Camera, reason: CameraChangeReason) -> CameraCommand {
    CameraCommand::Jump {
        camera: camera.clamped(),
        reason,
    }
}

fn animate(from: Camera, to: Camera, reason: CameraChangeReason) -> CameraCommand {
    CameraCommand::Animate {
        animation: CameraAnimation::ease(from, to, DEFAULT_ANIMATION_SECS),
        reason,
    }
}

/// camera zoomed by levels that keeps the coordinate under the anchor where it is
pub fn zoom_around(projection: &Projection, levels: f64, anchor: ScreenPoint) -> Option<Camera> {
    let grabbed = projection.unproject(anchor)?;
    let zoomed = Projection::new(
        Camera {
            zoom: (projection.camera.zoom + levels).clamp(MIN_ZOOM, MAX_ZOOM),
            ..projection.camera
        },
        projection.viewport,
    );
    zoomed.place_under(grabbed, anchor)
}

fn key_command(projection: &Projection, key: MapKey, modifiers: InputModifiers) -> Option<CameraCommand> {
    let camera = projection.camera;
    let center = projection.viewport.center();
    let pan = |dx: f64, dy: f64| -> Option<CameraCommand> {
        let grabbed = projection.unproject(center)?;
        let target = projection.place_under(grabbed, ScreenPoint::new(center.x + dx, center.y + dy))?;
        Some(animate(camera, target, CameraChangeReason::GESTURE_PAN))
    };
    let rotate = |degrees: f64| -> Option<CameraCommand> {
        let target = Camera {
            bearing: camera.bearing + degrees,
            ..camera
        };
        Some(animate(camera, target, CameraChangeReason::GESTURE_ROTATE))
    };
    match key {
        MapKey::ZoomIn => Some(animate(
            camera,
            zoom_around(projection, 1.0, center)?,
            CameraChangeReason::GESTURE_ZOOM_IN,
        )),
        MapKey::ZoomOut => Some(animate(
            camera,
            zoom_around(projection, -1.0, center)?,
            CameraChangeReason::GESTURE_ZOOM_OUT,
        )),
        MapKey::Left if modifiers.alt => rotate(-KEY_ROTATE_DEGREES),
        MapKey::Right if modifiers.alt => rotate(KEY_ROTATE_DEGREES),
        MapKey::Up => pan(0.0, KEY_PAN_POINTS),
        MapKey::Down => pan(0.0, -KEY_PAN_POINTS),
        MapKey::Left => pan(KEY_PAN_POINTS, 0.0),
        MapKey::Right => pan(-KEY_PAN_POINTS, 0.0),
    }
}

/// pin drawn by a gpui component over the map, with its view size
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Annotation {
    pub id: u64,
    pub coordinate: LatLng,
    pub width: f64,
    pub height: f64,
    pub anchor: (f64, f64),
}

/// rectangle in logical pixels of the map element
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScreenRect {
    pub origin: ScreenPoint,
    pub width: f64,
    pub height: f64,
}

impl ScreenRect {
    pub fn contains(&self, point: ScreenPoint) -> bool {
        point.x >= self.origin.x
            && point.x <= self.origin.x + self.width
            && point.y >= self.origin.y
            && point.y <= self.origin.y + self.height
    }
}

impl Projection {
    /// where an annotation view draws, none when its coordinate is off screen
    pub fn annotation_frame(&self, annotation: &Annotation) -> Option<ScreenRect> {
        let at = self.project(annotation.coordinate)?;
        Some(ScreenRect {
            origin: ScreenPoint::new(
                at.x - annotation.width * annotation.anchor.0,
                at.y - annotation.height * annotation.anchor.1,
            ),
            width: annotation.width,
            height: annotation.height,
        })
    }

    pub fn hit_test(&self, annotations: &[Annotation], point: ScreenPoint) -> Option<u64> {
        annotations
            .iter()
            .rev()
            .find(|annotation| {
                self.annotation_frame(annotation)
                    .is_some_and(|frame| frame.contains(point))
            })
            .map(|annotation| annotation.id)
    }
}

/// options for a still image of a style, used for the globe texture
#[derive(Clone, PartialEq, Debug)]
pub struct SnapshotRequest {
    pub style_json: String,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub camera: Camera,
}

pub const MAX_SNAPSHOT_SIDE: u32 = 8192;

#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum SnapshotError {
    #[error("a snapshot needs a non-empty size, got {width}x{height}")]
    EmptySize { width: u32, height: u32 },
    #[error("a snapshot side of {0} device pixels is over the {MAX_SNAPSHOT_SIDE} limit")]
    TooLarge(u32),
    #[error("a snapshot needs a style")]
    NoStyle,
    #[error("the snapshot style did not load: {0}")]
    Style(String),
    #[error("the snapshot did not render: {0}")]
    Renderer(String),
    #[error(transparent)]
    Unavailable(#[from] MapUnavailable),
}

impl SnapshotRequest {
    /// device pixel size to render, checked before the renderer is asked
    pub fn pixel_size(&self) -> Result<(u32, u32), SnapshotError> {
        if self.width == 0 || self.height == 0 {
            return Err(SnapshotError::EmptySize {
                width: self.width,
                height: self.height,
            });
        }
        if self.style_json.chars().all(char::is_whitespace) {
            return Err(SnapshotError::NoStyle);
        }
        let scale = f64::from(self.scale_factor.max(1.0));
        let width = (f64::from(self.width) * scale).round() as u32;
        let height = (f64::from(self.height) * scale).round() as u32;
        let side = width.max(height);
        if side > MAX_SNAPSHOT_SIDE {
            return Err(SnapshotError::TooLarge(side));
        }
        Ok((width, height))
    }
}

/// tile cache path that maplibre keeps, in the app cache dir so the os may purge it
pub fn tile_cache_path(paths: &AppPaths) -> PathBuf {
    paths.cache_dir().join("maps").join("tile-cache.db")
}

/// gpu api the maplibre renderer draws with on a host
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GpuBackend {
    Metal,
    Vulkan,
}

/// renderer backend per host, the crate default feature on each os
/// maplibre_native draws with metal on macos and vulkan on linux and windows
/// no direct3d backend so windows uses vulkan too
pub const fn backend_for(host: HostOs) -> GpuBackend {
    match host {
        HostOs::MacOs => GpuBackend::Metal,
        HostOs::Linux | HostOs::Windows => GpuBackend::Vulkan,
    }
}

/// why no map draws, the feature shows its empty map state with this text in the trace
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum MapUnavailable {
    #[error("{host:?} cannot draw a map with {backend:?}: {reason}")]
    NoNativeCore {
        host: HostOs,
        backend: GpuBackend,
        reason: &'static str,
    },
    #[error("the map render thread did not start: {0}")]
    RenderThread(String),
}

impl MapUnavailable {
    /// true when the graphics card or its driver is what is missing
    pub fn is_missing_gpu(&self) -> bool {
        matches!(self, Self::NoNativeCore { backend: GpuBackend::Vulkan, .. })
    }
}

/// how a style failed to load
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StyleFailure {
    Parse,
    Load,
    NotFound,
    Unknown,
}

/// one device pixel frame of the map in bgra8
/// unpremultiplied with packed rows, the format the texture seam takes from a cpu producer
#[derive(Clone, PartialEq, Debug)]
pub struct MapFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub camera: Camera,
}

impl MapFrame {
    /// frame from the renderer rgba rows, none if the buffer isnt width * height pixels
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8], camera: Camera) -> Option<Self> {
        let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        if width == 0 || height == 0 || rgba.len() != expected {
            return None;
        }
        let mut bgra = Vec::with_capacity(expected);
        for pixel in rgba.chunks_exact(4) {
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        Some(Self {
            width,
            height,
            bgra,
            camera,
        })
    }

    /// layout the texture seam allocates once per map and writes each frame into
    pub fn geometry(&self) -> Result<FrameGeometry, GeometryError> {
        FrameGeometry::packed(FrameFormat::Bgra8, self.width, self.height)
    }
}

/// what a map element asks of its renderer, the style is json
/// a new style drops runtime sources, layers and images
/// so a component adds them again on StyleLoaded
#[derive(Clone, PartialEq, Debug)]
pub enum MapRequest {
    SetStyle(String),
    Resize(Viewport),
    SetCamera(Camera),
    SetGeoJson { source_id: String, geojson: String },
    RemoveSource(String),
    AddLayer {
        layer_json: String,
        before: Option<String>,
    },
    RemoveLayer(String),
    AddImage {
        image_id: String,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        pixel_ratio: f32,
        signed_distance_field: bool,
    },
    RemoveImage(String),
}

impl MapRequest {
    /// name of the request in a RequestFailed event
    pub const fn name(&self) -> &'static str {
        match self {
            Self::SetStyle(_) => "set_style",
            Self::Resize(_) => "resize",
            Self::SetCamera(_) => "set_camera",
            Self::SetGeoJson { .. } => "set_geojson",
            Self::RemoveSource(_) => "remove_source",
            Self::AddLayer { .. } => "add_layer",
            Self::RemoveLayer(_) => "remove_layer",
            Self::AddImage { .. } => "add_image",
            Self::RemoveImage(_) => "remove_image",
        }
    }
}

/// what the renderer reports back, loading callbacks and frames
#[derive(Clone, PartialEq, Debug)]
pub enum MapEvent {
    WillStartLoading,
    StyleLoaded,
    StyleFailed {
        failure: StyleFailure,
        message: String,
    },
    Frame(MapFrame),
    Idle,
    RequestFailed {
        request: &'static str,
        message: String,
    },
}

/// everything a map needs to start, style and element size and first camera
#[derive(Clone)]
pub struct MapOptions {
    pub style_json: String,
    pub viewport: Viewport,
    pub camera: Camera,
    pub tile_cache: PathBuf,
    pub notify: Arc<dyn Fn() + Send + Sync>,
}

impl MapOptions {
    pub fn new(
        style_json: String,
        viewport: Viewport,
        camera: Camera,
        paths: &AppPaths,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            style_json,
            viewport,
            camera: camera.clamped(),
            tile_cache: tile_cache_path(paths),
            notify,
        }
    }
}

impl fmt::Debug for MapOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MapOptions")
            .field("style_bytes", &self.style_json.len())
            .field("viewport", &self.viewport)
            .field("camera", &self.camera)
            .field("tile_cache", &self.tile_cache)
            .finish_non_exhaustive()
    }
}

/// renderer thread is gone, the element shows its empty map state
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
#[error("the map renderer has stopped")]
pub struct MapClosed;

/// element side of one live map, dropping it stops the render thread
#[derive(Debug)]
pub struct MapHandle {
    requests: Sender<MapRequest>,
    events: Receiver<MapEvent>,
}

impl MapHandle {
    pub fn new(requests: Sender<MapRequest>, events: Receiver<MapEvent>) -> Self {
        Self { requests, events }
    }

    pub fn send(&self, request: MapRequest) -> Result<(), MapClosed> {
        self.requests.send(request).map_err(|_| MapClosed)
    }

    /// events queued since the last call
    /// frames older than the newest are dropped since only the newest is drawn
    pub fn drain(&self) -> Vec<MapEvent> {
        newest_frame_only(self.events.try_iter().collect())
    }
}

/// keeps every event but only the last Frame
pub fn newest_frame_only(events: Vec<MapEvent>) -> Vec<MapEvent> {
    let last_frame = events
        .iter()
        .rposition(|event| matches!(event, MapEvent::Frame(_)));
    events
        .into_iter()
        .enumerate()
        .filter(|(index, event)| !matches!(event, MapEvent::Frame(_)) || Some(*index) == last_frame)
        .map(|(_, event)| event)
        .collect()
}

/// region changing and region changed events of the map
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RegionEvent {
    IsChanging {
        camera: Camera,
        reason: CameraChangeReason,
    },
    DidChange {
        camera: Camera,
        reason: CameraChangeReason,
        animated: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Running {
    animation: CameraAnimation,
    started_secs: f64,
    reason: CameraChangeReason,
}

/// element camera between frames, applies commands and samples animations by the frame clock
/// also says when the region is changing and when it settled
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraDriver {
    camera: Camera,
    running: Option<Running>,
    moving: Option<CameraChangeReason>,
}

impl CameraDriver {
    pub fn new(camera: Camera) -> Self {
        Self {
            camera: camera.clamped(),
            running: None,
            moving: None,
        }
    }

    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn is_animating(&self) -> bool {
        self.running.is_some()
    }

    /// applies a command at frame time now_secs
    /// a jump during a drag reports region changing
    /// a new command cuts a running animation short with TRANSITION_CANCELLED
    pub fn apply(&mut self, command: CameraCommand, now_secs: f64) -> Vec<RegionEvent> {
        let mut events = self.cancel();
        match command {
            CameraCommand::Jump { camera, reason } => {
                self.camera = camera.clamped();
                self.moving = Some(self.moving.map_or(reason, |open| open.union(reason)));
                events.push(RegionEvent::IsChanging {
                    camera: self.camera,
                    reason,
                });
            }
            CameraCommand::Animate { animation, reason } => {
                events.extend(self.settle());
                self.running = Some(Running {
                    animation,
                    started_secs: now_secs,
                    reason,
                });
            }
        }
        events
    }

    /// sets the camera without animation, region changes and settles at once
    pub fn jump_to(&mut self, camera: Camera, reason: CameraChangeReason) -> Vec<RegionEvent> {
        let mut events = self.cancel();
        events.extend(self.settle());
        self.camera = camera.clamped();
        events.push(RegionEvent::IsChanging {
            camera: self.camera,
            reason,
        });
        events.push(RegionEvent::DidChange {
            camera: self.camera,
            reason,
            animated: false,
        });
        events
    }

    /// drag ended, reports region did change for the jumps it made
    pub fn settle(&mut self) -> Vec<RegionEvent> {
        self.moving
            .take()
            .map(|reason| RegionEvent::DidChange {
                camera: self.camera,
                reason,
                animated: false,
            })
            .into_iter()
            .collect()
    }

    /// samples the running animation at frame time now_secs
    pub fn tick(&mut self, now_secs: f64) -> Vec<RegionEvent> {
        let Some(running) = self.running else {
            return Vec::new();
        };
        let elapsed = (now_secs - running.started_secs).max(0.0);
        self.camera = running.animation.sample(elapsed);
        let mut events = vec![RegionEvent::IsChanging {
            camera: self.camera,
            reason: running.reason,
        }];
        if running.animation.is_finished(elapsed) {
            self.running = None;
            events.push(RegionEvent::DidChange {
                camera: self.camera,
                reason: running.reason,
                animated: true,
            });
        }
        events
    }

    fn cancel(&mut self) -> Vec<RegionEvent> {
        self.running
            .take()
            .map(|running| RegionEvent::DidChange {
                camera: self.camera,
                reason: running.reason.union(CameraChangeReason::TRANSITION_CANCELLED),
                animated: true,
            })
            .into_iter()
            .collect()
    }
}

/// one tile of the world grid at a whole zoom level
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileId {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    /// tile corners, north west first
    pub fn bounds(&self) -> (LatLng, LatLng) {
        let count = f64::from(1u32 << self.zoom);
        let corner = |x: f64, y: f64| LatLng::new(inverse_mercator(0.0, y / count).latitude, x / count * 360.0 - 180.0);
        (
            corner(f64::from(self.x), f64::from(self.y)),
            corner(f64::from(self.x + 1), f64::from(self.y + 1)),
        )
    }
}

/// computed shape source for desktop
/// the component computes features for tiles in view and sends their union as one SetGeoJson
/// since the renderer only takes geojson sources, zoom follows the source min and max
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ComputedTiles {
    pub min_zoom: u8,
    pub max_zoom: u8,
}

impl ComputedTiles {
    /// tiles the viewport covers at the camera zoom clamped to the source range, from the 4 screen corners
    /// a pitched view whose top corners miss the ground uses the center
    pub fn tiles_in_view(&self, projection: &Projection) -> Vec<TileId> {
        let zoom = (projection.camera.zoom.floor() as i64)
            .clamp(i64::from(self.min_zoom), i64::from(self.max_zoom)) as u8;
        let viewport = projection.viewport;
        let corners = [
            ScreenPoint::new(0.0, 0.0),
            ScreenPoint::new(viewport.width, 0.0),
            ScreenPoint::new(0.0, viewport.height),
            ScreenPoint::new(viewport.width, viewport.height),
        ];
        let mut points: Vec<(f64, f64)> = corners
            .iter()
            .filter_map(|corner| projection.unproject(*corner))
            .map(mercator)
            .collect();
        if points.is_empty() {
            points.push(mercator(projection.camera.center));
        }
        let count = 1u32 << zoom;
        let cell = |value: f64| ((value * f64::from(count)).floor() as i64).clamp(0, i64::from(count) - 1) as u32;
        let (min_x, max_x) = points.iter().fold((u32::MAX, 0), |(lo, hi), (x, _)| (lo.min(cell(*x)), hi.max(cell(*x))));
        let (min_y, max_y) = points.iter().fold((u32::MAX, 0), |(lo, hi), (_, y)| (lo.min(cell(*y)), hi.max(cell(*y))));
        let mut tiles = Vec::new();
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                tiles.push(TileId { zoom, x, y });
            }
        }
        tiles
    }
}

pub fn feature_collection(features: Vec<serde_json::Value>) -> String {
    let features: Vec<serde_json::Value> = features
        .into_iter()
        .filter(|feature| {
            let is_feature = feature.get("type").and_then(serde_json::Value::as_str) == Some("Feature");
            if !is_feature {
                tracing::debug!("computed shape source skipped a value that is not a GeoJSON Feature");
            }
            is_feature
        })
        .collect();
    serde_json::json!({ "type": "FeatureCollection", "features": features }).to_string()
}

pub trait MapEngine {
    fn available(&self, host: HostOs) -> Result<GpuBackend, MapUnavailable>;
    /// starts a map on one render thread that owns the renderer, its run loop is thread bound
    fn open(&self, host: HostOs, options: MapOptions) -> Result<MapHandle, MapUnavailable>;
    /// one still frame, blocks until drawn so keep it off the main thread
    fn snapshot(&self, host: HostOs, request: &SnapshotRequest, tile_cache: PathBuf) -> Result<MapFrame, SnapshotError>;
}

/// whether this host can draw a map and with which backend
pub fn availability() -> Result<GpuBackend, MapUnavailable> {
    super::os::NativeMapEngine.available(HostOs::current())
}

/// starts a map for an element, frames and loading events come through the handle
pub fn open_map(options: MapOptions) -> Result<MapHandle, MapUnavailable> {
    super::os::NativeMapEngine.open(HostOs::current(), options)
}

/// still image of a style for the globe texture
/// blocking, run it on a background executor
pub fn snapshot(request: &SnapshotRequest, paths: &AppPaths) -> Result<MapFrame, SnapshotError> {
    request.pixel_size()?;
    super::os::NativeMapEngine.snapshot(HostOs::current(), request, tile_cache_path(paths))
}

pub fn support() -> PlatformSupport {
    match availability() {
        Ok(_) => PlatformSupport::Performed,
        Err(reason) => {
            tracing::debug!(%reason, "map view unavailable");
            PlatformSupport::Unsupported
        }
    }
}

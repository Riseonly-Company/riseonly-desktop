use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    Animation, AnimationExt, App, Bounds, Context, ElementId, Entity, FocusHandle, FontWeight, IntoElement,
    KeyDownEvent, Modifiers, PinchEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels,
    Render, RenderImage, RenderOnce, ScrollDelta, ScrollWheelEvent, SharedString, Task, Window, canvas, img, point,
    white,
};
use image::{Frame, RgbaImage};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_url::{UrlComponents, UrlQueryItem};
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_i18n::{I18n, tr};
use rise_media::map_view::{
    Camera, CameraCommand, CameraDriver, DragButton, InputModifiers, LatLng, MapEvent, MapFrame, MapHandle, MapInput,
    MapInputController, MapKey, MapOptions, MapRequest, Projection, ScreenPoint, TILE_SIZE, Viewport, WheelDelta,
    mercator, open_map,
};
use rise_platform::AppPaths;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::IconUi;
use rise_ui::animated_media_playback_scope::is_animated_media_playback_active;
use rise_ui::liquid_chrome::{LiquidChromeModifiers, LiquidGlassContrast};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::{EnvKey, ScenePhase, env, reduce_motion, scene_phase};
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec, Spring};
use rise_ui::swiftui::open_url::open_url;
use rise_ui::swiftui::prelude::*;
use smallvec::SmallVec;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::stores::message::message_actions::message_types::MessageLocationInfo;
use crate::modules::map::shared::rise_map_style::RiseMapStyle;

pub const LOCATION_SPAN_DEGREES: f64 = 0.012;

static APP_PATHS: LazyLock<Option<AppPaths>> =
    LazyLock::new(|| AppPaths::resolve(AppEnvironment::compiled().canon()).ok());

pub fn location_app_paths() -> Option<&'static AppPaths> {
    APP_PATHS.as_ref()
}

/// zoom at which the map shows a lat lng region whole in a width by height view
/// like mapkit fits a region to the view aspect
pub fn region_zoom(center: LatLng, latitude_delta: f64, longitude_delta: f64, width: f64, height: f64) -> f64 {
    let half = latitude_delta / 2.0;
    let north = mercator(LatLng::new(center.latitude + half, center.longitude)).1;
    let south = mercator(LatLng::new(center.latitude - half, center.longitude)).1;
    let span_y = (south - north).abs().max(f64::EPSILON);
    let span_x = (longitude_delta / 360.0).max(f64::EPSILON);
    let zoom_y = (height / (TILE_SIZE * span_y)).log2();
    let zoom_x = (width / (TILE_SIZE * span_x)).log2();
    zoom_x.min(zoom_y)
}

/// style the desktop map draws in the current theme and app language
pub fn location_map_style(cx: &mut App) -> String {
    let appearance = theme_store(cx).read(cx).resolved_appearance();
    RiseMapStyle::json(appearance, I18n::shared().code())
}

/// bgra picture of a map frame as one gpui image
pub fn location_map_image(frame: MapFrame) -> Option<Arc<RenderImage>> {
    let buffer = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
    Some(Arc::new(RenderImage::new(frames)))
}

pub struct LocationPresentationState {
    location: Option<MessageLocationInfo>,
    is_presented: bool,
    opacity: Animated<f32>,
    pending_clear: Option<Task<()>>,
}

impl LocationPresentationState {
    pub fn new() -> Self {
        Self {
            location: None,
            is_presented: false,
            opacity: Animated::new(0.0),
            pending_clear: None,
        }
    }

    pub fn location(&self) -> Option<&MessageLocationInfo> {
        self.location.as_ref()
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn present(&mut self, loc: MessageLocationInfo, cx: &mut Context<Self>) {
        self.pending_clear = None;
        if self.location.is_none() {
            self.opacity.jump(0.0);
        }
        self.location = Some(loc);
        self.is_presented = true;
        self.opacity
            .set_in(1.0, MotionSpec::spring(Spring::new(0.4, 0.86)), RiseClock::monotonic(), cx);
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.is_presented = false;
        self.opacity
            .set_in(0.0, MotionSpec::spring(Spring::new(0.32, 0.88)), RiseClock::monotonic(), cx);
        cx.notify();
        self.pending_clear = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(360)).await;
            let _ = this.update(cx, |this, cx| {
                if !this.is_presented {
                    this.location = None;
                    cx.notify();
                }
            });
        }));
    }
}

impl Default for LocationPresentationState {
    fn default() -> Self {
        Self::new()
    }
}

/// presenter the bubbles of one chat share, without a host above them there is none
pub struct LocationPresentationKey;

impl EnvKey for LocationPresentationKey {
    type Value = Option<Entity<LocationPresentationState>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// reads the location presentation from the environment
pub fn location_presentation(cx: &App) -> Option<Entity<LocationPresentationState>> {
    env::<LocationPresentationKey>(cx)
}

#[derive(IntoElement)]
pub struct LocationFullscreenHost {
    id: ElementId,
    location: Option<MessageLocationInfo>,
    opacity: f32,
    pub state: Entity<LocationPresentationState>,
}

impl RenderOnce for LocationFullscreenHost {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Some(loc) = self.location else {
            return div().into_any_element();
        };
        let state = self.state;
        div()
            .relative()
            .size_full()
            .opacity(self.opacity)
            .child(
                LocationFullscreenView::new(self.id, loc, move |_, cx| state.update(cx, |state, cx| state.dismiss(cx)))
                    .occludes_bottom_chrome(),
            )
            .into_any_element()
    }
}

impl Render for LocationPresentationState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        LocationFullscreenHost {
            id: ElementId::Name("root-location-fullscreen".into()),
            location: self.location.clone(),
            opacity: self.opacity.frame_value(RiseClock::monotonic(), window),
            state: cx.entity(),
        }
    }
}

/// live map of one presented point
/// renderer handle, camera and newest frame
struct LocationFullscreenMap {
    location: MessageLocationInfo,
    handle: Option<MapHandle>,
    driver: CameraDriver,
    input: MapInputController,
    viewport: Option<Viewport>,
    origin: gpui::Point<Pixels>,
    image: Option<Arc<RenderImage>>,
    focus_handle: FocusHandle,
    _pump: Option<Task<()>>,
}

impl LocationFullscreenMap {
    fn new(location: MessageLocationInfo, cx: &mut Context<Self>) -> Self {
        let center = LatLng::new(location.latitude, location.longitude);
        Self {
            location,
            handle: None,
            driver: CameraDriver::new(Camera::new(center, 0.0)),
            input: MapInputController::default(),
            viewport: None,
            origin: point(px(0.0), px(0.0)),
            image: None,
            focus_handle: cx.focus_handle(),
            _pump: None,
        }
    }

    fn layout(&mut self, bounds: Bounds<Pixels>, scale_factor: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.origin = bounds.origin;
        let viewport = Viewport {
            width: f64::from(f32::from(bounds.size.width)),
            height: f64::from(f32::from(bounds.size.height)),
            scale_factor,
        };
        if self.viewport == Some(viewport) || viewport.width < 1.0 || viewport.height < 1.0 {
            return;
        }
        let first = self.viewport.is_none();
        self.viewport = Some(viewport);
        if first {
            let center = LatLng::new(self.location.latitude, self.location.longitude);
            let zoom = region_zoom(center, LOCATION_SPAN_DEGREES, LOCATION_SPAN_DEGREES, viewport.width, viewport.height);
            self.driver = CameraDriver::new(Camera::new(center, zoom));
            self.open(viewport, window, cx);
        } else if let Some(handle) = &self.handle {
            let _ = handle.send(MapRequest::Resize(viewport));
        }
    }

    fn open(&mut self, viewport: Viewport, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paths) = location_app_paths() else {
            tracing::debug!("location map has no app paths");
            return;
        };
        let (wake, mut woken) = mpsc::unbounded::<()>();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake.unbounded_send(());
        });
        window.focus(&self.focus_handle, cx);
        let options = MapOptions::new(location_map_style(cx), viewport, self.driver.camera(), paths, notify);
        match open_map(options) {
            Ok(handle) => self.handle = Some(handle),
            Err(reason) => {
                tracing::debug!(%reason, "location map unavailable");
                return;
            }
        }
        self._pump = Some(cx.spawn_in(window, async move |this, cx| {
            while woken.next().await.is_some() {
                if this.update_in(cx, |this, window, cx| this.drain(window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(handle) = &self.handle else {
            return;
        };
        for event in handle.drain() {
            if let MapEvent::Frame(frame) = event {
                if let Some(image) = location_map_image(frame) {
                    if let Some(old) = self.image.replace(image) {
                        let _ = window.drop_image(old);
                    }
                    cx.notify();
                }
            }
        }
    }

    fn apply(&mut self, command: Option<CameraCommand>, cx: &mut Context<Self>) {
        let Some(command) = command else {
            return;
        };
        let now = RiseClock::monotonic().as_secs_f64();
        let _ = self.driver.apply(command, now);
        self.send_camera();
        cx.notify();
    }

    fn send_camera(&self) {
        if let Some(handle) = &self.handle {
            let _ = handle.send(MapRequest::SetCamera(self.driver.camera()));
        }
    }

    fn handle_input(&mut self, input: MapInput, cx: &mut Context<Self>) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let camera = self.driver.camera();
        let command = self.input.handle(input, &camera, &viewport);
        self.apply(command, cx);
    }

    fn local(&self, position: gpui::Point<Pixels>) -> ScreenPoint {
        ScreenPoint::new(
            f64::from(f32::from(position.x - self.origin.x)),
            f64::from(f32::from(position.y - self.origin.y)),
        )
    }

    fn modifiers(modifiers: &Modifiers) -> InputModifiers {
        InputModifiers {
            secondary: modifiers.secondary(),
            control: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
        }
    }

    fn map_key(key: &str) -> Option<MapKey> {
        match key {
            "+" | "=" => Some(MapKey::ZoomIn),
            "-" => Some(MapKey::ZoomOut),
            "up" => Some(MapKey::Up),
            "down" => Some(MapKey::Down),
            "left" => Some(MapKey::Left),
            "right" => Some(MapKey::Right),
            _ => None,
        }
    }

    fn render_map(
        entity: &Entity<Self>,
        location: &MessageLocationInfo,
        on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
        id: ElementId,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let now = RiseClock::monotonic().as_secs_f64();
        let (image, pin_at, focus_handle) = entity.update(cx, |map, _| {
            if map.driver.is_animating() {
                let _ = map.driver.tick(now);
                map.send_camera();
            }
            let pin_at = map.viewport.and_then(|viewport| {
                Projection::new(map.driver.camera(), viewport)
                    .project(LatLng::new(location.latitude, location.longitude))
            });
            (map.image.clone(), pin_at, map.focus_handle.clone())
        });
        if entity.read(cx).driver.is_animating() {
            window.request_animation_frame();
        }

        let layout_entity = entity.clone();
        let probe = canvas(
            move |bounds, window, cx| {
                let scale_factor = window.scale_factor();
                let entity = layout_entity.clone();
                window.defer(cx, move |window, cx| {
                    entity.update(cx, |map, cx| map.layout(bounds, scale_factor, window, cx));
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let pin_side = 64.0_f32;
        let pin = pin_at.map(|at| {
            div()
                .absolute()
                .left(px(at.x as f32 - pin_side / 2.0))
                .top(px(at.y as f32 - pin_side / 2.0))
                .child(
                    zstack(Alignment::Center)
                        .frame_size(pin_side, pin_side)
                        .child(ChatLocationPin::new(ElementId::NamedChild(Arc::new(id.clone()), "pin".into()), location.live_remaining(Date::now()).is_some())),
                )
        });

        let down = entity.clone();
        let right_down = entity.clone();
        let moved = entity.clone();
        let up = entity.clone();
        let right_up = entity.clone();
        let wheel = entity.clone();
        let key = entity.clone();
        let pinch = entity.clone();
        div()
            .id(ElementId::NamedChild(Arc::new(id), "map".into()))
            .track_focus(&focus_handle)
            .absolute()
            .inset_0()
            .overflow_hidden()
            .children(image.map(|image| img(image).size_full().object_fit(ObjectFit::Cover)))
            .child(probe)
            .children(pin)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                down.update(cx, |map, cx| {
                    let at = map.local(event.position);
                    let modifiers = Self::modifiers(&event.modifiers);
                    let input = if event.click_count == 2 {
                        MapInput::DoubleClick { point: at, modifiers }
                    } else {
                        MapInput::DragStart { point: at, button: DragButton::Primary, modifiers }
                    };
                    map.handle_input(input, cx);
                });
            })
            .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                right_down.update(cx, |map, cx| {
                    let at = map.local(event.position);
                    let modifiers = Self::modifiers(&event.modifiers);
                    map.handle_input(MapInput::DragStart { point: at, button: DragButton::Secondary, modifiers }, cx);
                });
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                moved.update(cx, |map, cx| {
                    if map.input.is_dragging() {
                        let at = map.local(event.position);
                        map.handle_input(MapInput::DragMove { point: at }, cx);
                    }
                });
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                up.update(cx, |map, cx| map.handle_input(MapInput::DragEnd, cx));
            })
            .on_mouse_up(MouseButton::Right, move |_: &MouseUpEvent, _, cx| {
                right_up.update(cx, |map, cx| map.handle_input(MapInput::DragEnd, cx));
            })
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                wheel.update(cx, |map, cx| {
                    let at = map.local(event.position);
                    let delta = match event.delta {
                        ScrollDelta::Pixels(delta) => WheelDelta::Pixels {
                            x: f64::from(f32::from(delta.x)),
                            y: f64::from(f32::from(delta.y)),
                        },
                        ScrollDelta::Lines(delta) => WheelDelta::Lines {
                            x: f64::from(delta.x),
                            y: f64::from(delta.y),
                        },
                    };
                    let modifiers = Self::modifiers(&event.modifiers);
                    map.handle_input(MapInput::Wheel { point: at, delta, modifiers }, cx);
                });
            })
            .on_pinch(move |event: &PinchEvent, _, cx| {
                pinch.update(cx, |map, cx| {
                    let at = map.local(event.position);
                    map.handle_input(MapInput::Pinch { point: at, delta: f64::from(event.delta) }, cx);
                });
            })
            .layer_keys(LayerCancel::Dismiss(on_dismiss))
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                let Some(map_key) = Self::map_key(&keystroke.key) else {
                    return;
                };
                let modifiers = Self::modifiers(&keystroke.modifiers);
                key.update(cx, |map, cx| map.handle_input(MapInput::Key { key: map_key, modifiers }, cx));
            })
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct LocationFullscreenView {
    id: ElementId,
    location: MessageLocationInfo,
    on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl LocationFullscreenView {
    fn new(id: ElementId, location: MessageLocationInfo, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            id,
            location,
            on_dismiss: Rc::new(on_dismiss),
        }
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    fn title(location: &MessageLocationInfo) -> String {
        location.title.clone().unwrap_or_else(|| {
            if location.live_remaining(Date::now()).is_some() {
                tr("chat_msg_live_location")
            } else {
                tr("chat_msg_location")
            }
        })
    }

    fn chrome(&self) -> AnyElement {
        let on_dismiss = self.on_dismiss.clone();
        let close = LiquidGlassButton::new(self.child_id("close"), LiquidGlassButtonShape::Circle)
            .on_press(move |window, cx| on_dismiss(window, cx))
            .children(IconUi::sized("xmark", px(15.0), white(), true));
        let location = self.location.clone();
        let open = LiquidGlassButton::new(self.child_id("open-in-maps"), LiquidGlassButtonShape::Circle)
            .on_press(move |window, cx| Self::open_in_maps(&location, window, cx))
            .children(IconUi::sized("arrow.up.right.square", px(15.0), white(), true));
        hstack(None)
            .w_full()
            .px(px(14.0))
            .pt(px(56.0))
            .child(close)
            .child(spacer_min_w(0.0))
            .child(open)
            .liquid_glass_contrast(LiquidGlassContrast::Media)
    }

    fn details_card(&self) -> AnyElement {
        let location = self.location.clone();
        let mut card = vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .child(
                div()
                    .swiftui_font(Font::system_weight(17.0, FontWeight::BOLD))
                    .text_color(white())
                    .line_clamp(2)
                    .child(SharedString::from(Self::title(&self.location))),
            );
        if let Some(addr) = self.location.address.clone() {
            card = card.child(
                div()
                    .swiftui_font(Font::system_size(13.0))
                    .text_color(white().opacity(0.78))
                    .line_clamp(2)
                    .child(SharedString::from(addr)),
            );
        }
        let button = plain(self.child_id("open-in-maps-card"))
            .on_press(move |window, cx| Self::open_in_maps(&location, window, cx))
            .child(
                hstack(6.0)
                    .px(px(14.0))
                    .py(px(8.0))
                    .capsule()
                    .bg(white().opacity(0.18))
                    .text_color(white())
                    .children(IconUi::sized("map.fill", px(13.0), white(), true))
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(13.0, FontWeight::SEMIBOLD))
                            .child(SharedString::from(tr("map_open_in_maps"))),
                    ),
            );
        let plate = card
            .child(div().pt(px(6.0)).child(button))
            .p(px(16.0))
            .w_full()
            .continuous_corners(18.0)
            .bg(gpui::black().opacity(0.55));
        div().w_full().px(px(14.0)).pb(px(36.0)).child(plate).into_any_element()
    }

    fn open_in_maps(location: &MessageLocationInfo, window: &mut Window, cx: &mut App) {
        let mut components = UrlComponents::new();
        components.set_scheme(Some("https"));
        components.set_host(Some("maps.apple.com"));
        components.set_path("/");
        components.set_query_items(Some(vec![
            UrlQueryItem::new("ll", Some(format!("{},{}", location.latitude, location.longitude))),
            UrlQueryItem::new("q", Some(Self::title(location))),
        ]));
        let Some(url) = components.url() else {
            return;
        };
        open_url(cx).call_as_function(&url, window, cx);
    }
}

impl RenderOnce for LocationFullscreenView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let location = self.location.clone();
        let map_key = ElementId::NamedChild(
            Arc::new(self.id.clone()),
            SharedString::from(format!("map-{}-{}", location.latitude, location.longitude)),
        );
        let map = window.use_keyed_state(map_key, cx, move |_, cx| LocationFullscreenMap::new(location, cx));
        let map_view =
            LocationFullscreenMap::render_map(&map, &self.location, self.on_dismiss.clone(), self.id.clone(), window, cx);
        zstack(Alignment::Top)
            .size_full()
            .child(map_view)
            .child(self.chrome())
            .child(vstack(0.0).size_full().child(spacer()).child(self.details_card()))
    }
}

#[derive(IntoElement)]
pub struct ChatLocationPin {
    id: ElementId,
    pub live: bool,
    pub pulse_diameter: f32,
    pub resting_pulse_diameter: f32,
}

impl ChatLocationPin {
    pub fn new(id: impl Into<ElementId>, live: bool) -> Self {
        Self {
            id: id.into(),
            live,
            pulse_diameter: 64.0,
            resting_pulse_diameter: 24.0,
        }
    }

    pub fn pulse_diameter(mut self, pulse_diameter: f32) -> Self {
        self.pulse_diameter = pulse_diameter;
        self
    }

    pub fn resting_pulse_diameter(mut self, resting_pulse_diameter: f32) -> Self {
        self.resting_pulse_diameter = resting_pulse_diameter;
        self
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    fn should_animate(&self, cx: &mut App) -> bool {
        let energy_saving = RiseEnergySavingStore::shared(cx);
        let energy_saving = energy_saving.read(cx);
        self.live
            && !is_message_layout_measurement(cx)
            && is_animated_media_playback_active(cx)
            && scene_phase(cx) == ScenePhase::Active
            && !reduce_motion(cx)
            && !energy_saving.system_low_power_mode()
            && !energy_saving.reduces(RiseEnergySavingFeature::InterfaceEffects)
    }
}

impl RenderOnce for ChatLocationPin {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme().clone();
        let should_animate = self.should_animate(cx);

        let mut pin = zstack(Alignment::Center);
        if should_animate {
            let pulse = self.pulse_diameter;
            let resting = self.resting_pulse_diameter;
            let fill = theme.semantic.error_100.opacity(0.32);
            let easing = CubicBezier::EASE_OUT.easing();
            pin = pin.child(
                zstack(Alignment::Center).frame_size(pulse, pulse).child(
                    div().circle().bg(fill).with_animation(
                        self.child_id("pulse"),
                        Animation::new(Duration::from_millis(1600)).repeat().with_easing(easing),
                        move |circle, delta| {
                            let diameter = resting + (pulse - resting) * delta;
                            circle.frame_size(diameter, diameter).opacity(0.9 * (1.0 - delta))
                        },
                    ),
                ),
            );
        }
        let dot = if self.live { theme.semantic.error_100 } else { theme.primary._100 };
        pin.child(
            div()
                .circle()
                .bg(dot)
                .frame_size(22.0, 22.0)
                .border(px(3.0))
                .border_color(white())
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.25),
                    offset: point(px(0.0), px(2.0)),
                    blur_radius: px(4.0),
                    spread_radius: px(0.0),
                    inset: false,
                }]),
        )
    }
}

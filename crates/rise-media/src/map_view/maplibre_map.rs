use std::cell::Cell;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use maplibre_native as mln;

use super::map_view::*;

const FRAME_INTERVAL: Duration = Duration::from_millis(16);

const TILE_CACHE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone)]
struct Emitter {
    events: Sender<MapEvent>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Emitter {
    /// queues an event and wakes the element, false once the element dropped its handle
    fn emit(&self, event: MapEvent) -> bool {
        let sent = self.events.send(event).is_ok();
        if sent {
            (self.notify)();
        }
        sent
    }

    fn refused(&self, request: &'static str, message: String) {
        tracing::debug!(request, %message, "map style request refused");
        self.emit(MapEvent::RequestFailed { request, message });
    }
}

/// starts the render thread of one map
pub(crate) fn open(options: MapOptions) -> Result<MapHandle, MapUnavailable> {
    let (request_tx, request_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("rise-map".to_owned())
        .spawn(move || run(options, request_rx, event_tx))
        .map_err(|error| MapUnavailable::RenderThread(error.to_string()))?;
    Ok(MapHandle::new(request_tx, event_rx))
}

/// draws one still frame on its own thread and waits for it
pub(crate) fn snapshot(request: &SnapshotRequest, tile_cache: PathBuf) -> Result<MapFrame, SnapshotError> {
    let request = request.clone();
    let (answer_tx, answer_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("rise-map-snapshot".to_owned())
        .spawn(move || {
            if answer_tx.send(render_snapshot(&request, tile_cache)).is_err() {
                tracing::debug!("map snapshot finished after its caller stopped waiting");
            }
        })
        .map_err(|error| SnapshotError::Renderer(error.to_string()))?;
    answer_rx
        .recv()
        .map_err(|_| SnapshotError::Renderer("the snapshot thread ended without a frame".to_owned()))?
}

fn resources(tile_cache: PathBuf) -> mln::ResourceOptions {
    if let Some(parent) = tile_cache.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            tracing::debug!(%error, path = %parent.display(), "map tile cache directory not created; tiles load uncached");
        }
    }
    mln::ResourceOptions::default()
        .with_cache_path(tile_cache)
        .with_maximum_cache_size(TILE_CACHE_BYTES)
}

/// renderer takes the element logical size and draws size * pixel_ratio device pixels
fn logical_size(viewport: &Viewport) -> (NonZeroU32, NonZeroU32) {
    let side = |value: f64| NonZeroU32::new(value.round().max(1.0) as u32).unwrap_or(NonZeroU32::MIN);
    (side(viewport.width), side(viewport.height))
}

fn camera_update(camera: &Camera) -> mln::CameraUpdate {
    mln::CameraUpdate::new()
        .center(mln::LatLng {
            lat: camera.center.latitude,
            lng: camera.center.longitude,
        })
        .zoom(camera.zoom)
        .bearing(camera.bearing)
        .pitch(camera.pitch)
}

fn style_failure(kind: mln::MapLoadErrorKind) -> StyleFailure {
    match kind {
        mln::MapLoadErrorKind::StyleParse => StyleFailure::Parse,
        mln::MapLoadErrorKind::StyleLoad => StyleFailure::Load,
        mln::MapLoadErrorKind::NotFound => StyleFailure::NotFound,
        _ => StyleFailure::Unknown,
    }
}

fn build_renderer(viewport: &Viewport, tile_cache: PathBuf) -> mln::ImageRenderer<mln::Continuous> {
    let (width, height) = logical_size(viewport);
    mln::ImageRendererBuilder::new()
        .with_size(width, height)
        .with_pixel_ratio(viewport.scale_factor.max(1.0))
        .with_resource_options(resources(tile_cache))
        .build_continuous_renderer()
}

/// wires renderer callbacks to the element, loading events and the draw a frame flag
fn observe(renderer: &mut mln::ImageRenderer<mln::Continuous>, emitter: &Emitter, dirty: &Rc<Cell<bool>>) {
    renderer.set_render_requested_callback({
        let dirty = Rc::clone(dirty);
        move || dirty.set(true)
    });
    let observer = renderer.map_observer();
    observer.set_will_start_loading_map_callback({
        let emitter = emitter.clone();
        move || {
            emitter.emit(MapEvent::WillStartLoading);
        }
    });
    observer.set_did_finish_loading_style_callback({
        let emitter = emitter.clone();
        let dirty = Rc::clone(dirty);
        move || {
            dirty.set(true);
            emitter.emit(MapEvent::StyleLoaded);
        }
    });
    observer.set_did_fail_loading_map_callback({
        let emitter = emitter.clone();
        move |error| {
            tracing::debug!(%error, "map style failed to load");
            emitter.emit(MapEvent::StyleFailed {
                failure: style_failure(error.kind),
                message: error.message,
            });
        }
    });
    observer.set_did_become_idle_callback({
        let emitter = emitter.clone();
        move || {
            emitter.emit(MapEvent::Idle);
        }
    });
}

struct LiveMap {
    renderer: mln::ImageRenderer<mln::Continuous>,
    emitter: Emitter,
    dirty: Rc<Cell<bool>>,
    style_json: String,
    viewport: Viewport,
    camera: Camera,
    tile_cache: PathBuf,
}

impl LiveMap {
    fn new(options: MapOptions, emitter: Emitter) -> Self {
        let mut renderer = build_renderer(&options.viewport, options.tile_cache.clone());
        let dirty = Rc::new(Cell::new(true));
        observe(&mut renderer, &emitter, &dirty);
        let mut map = Self {
            renderer,
            emitter,
            dirty,
            style_json: options.style_json,
            viewport: options.viewport,
            camera: options.camera.clamped(),
            tile_cache: options.tile_cache,
        };
        map.load_style();
        map.renderer.update_camera(&camera_update(&map.camera));
        map
    }

    fn load_style(&mut self) {
        drop(self.renderer.load_style_from_json_str(&self.style_json));
    }

    fn apply(&mut self, request: MapRequest) {
        let name = request.name();
        match request {
            MapRequest::SetStyle(json) => {
                self.style_json = json;
                self.load_style();
            }
            MapRequest::Resize(viewport) => self.resize(viewport),
            MapRequest::SetCamera(camera) => {
                self.camera = camera.clamped();
                self.renderer.update_camera(&camera_update(&self.camera));
            }
            MapRequest::SetGeoJson { source_id, geojson } => {
                if let Err(message) = self.set_geojson(&source_id, &geojson) {
                    self.emitter.refused(name, message);
                }
            }
            MapRequest::RemoveSource(source_id) => self.renderer.style().remove_source(source_id),
            MapRequest::AddLayer { layer_json, before } => {
                let added = mln::AnyLayer::from_json_str(&layer_json).and_then(|layer| {
                    let mut style = self.renderer.style();
                    match before {
                        Some(before) => style.add_layer_before(layer, before),
                        None => style.add_layer(layer),
                    }
                });
                if let Err(error) = added {
                    self.emitter.refused(name, error.to_string());
                }
            }
            MapRequest::RemoveLayer(layer_id) => {
                if self.renderer.style().remove_layer(&layer_id).is_none() {
                    self.emitter.refused(name, format!("no layer {layer_id} in the style"));
                }
            }
            MapRequest::AddImage {
                image_id,
                rgba,
                width,
                height,
                pixel_ratio,
                signed_distance_field,
            } => {
                let Some(pixels) = image::RgbaImage::from_raw(width, height, rgba) else {
                    self.emitter
                        .refused(name, format!("image {image_id} is not {width}x{height} RGBA"));
                    return;
                };
                let image = image::DynamicImage::ImageRgba8(pixels);
                let added = self
                    .renderer
                    .style()
                    .add_image(&image_id, &image, pixel_ratio, signed_distance_field);
                if let Err(error) = added {
                    self.emitter.refused(name, error.to_string());
                }
            }
            MapRequest::RemoveImage(image_id) => self.renderer.style().remove_image(image_id),
        }
        self.dirty.set(true);
    }

    /// replaces the shape of an existing geojson source or adds the source
    fn set_geojson(&mut self, source_id: &str, geojson: &str) -> Result<(), String> {
        let shape = mln::GeoJson::from_json_str(geojson).map_err(|error| error.to_string())?;
        let mut style = self.renderer.style();
        let replaced = match style.source_mut(source_id) {
            Some(mln::SourceRefMut::GeoJson(mut source)) => {
                source.set_geojson(&shape);
                true
            }
            Some(source) => {
                return Err(format!("source {source_id} is {:?}, not GeoJSON", source.source_type()));
            }
            None => false,
        };
        if !replaced {
            let mut source = mln::GeoJsonSource::new(source_id);
            source.set_geojson(&shape);
            style.add_source(source).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// a new logical size just resizes the map
    /// a new scale factor needs a new renderer which reloads the style
    /// so components add runtime layers again on the next StyleLoaded
    fn resize(&mut self, viewport: Viewport) {
        let rescaled = viewport.scale_factor.max(1.0) != self.viewport.scale_factor.max(1.0);
        self.viewport = viewport;
        if rescaled {
            self.renderer = build_renderer(&viewport, self.tile_cache.clone());
            observe(&mut self.renderer, &self.emitter, &self.dirty);
            self.load_style();
            self.renderer.update_camera(&camera_update(&self.camera));
            return;
        }
        let (width, height) = logical_size(&viewport);
        self.renderer.set_map_size(mln::Size {
            width: width.get(),
            height: height.get(),
        });
    }

    fn draw_if_needed(&mut self) -> bool {
        if !self.dirty.replace(false) {
            return true;
        }
        self.renderer.render_once();
        let image = self.renderer.read_still_image();
        let size = image.size();
        match MapFrame::from_rgba(size.width, size.height, image.buffer(), self.camera) {
            Some(frame) => self.emitter.emit(MapEvent::Frame(frame)),
            None => {
                tracing::debug!(width = size.width, height = size.height, "map frame had no pixels; skipped");
                true
            }
        }
    }
}

fn run(options: MapOptions, requests: Receiver<MapRequest>, events: Sender<MapEvent>) {
    let emitter = Emitter {
        events,
        notify: Arc::clone(&options.notify),
    };
    let mut map = LiveMap::new(options, emitter);
    let run_loop = mln::RunLoopHandle::current();
    loop {
        let request = match requests.recv_timeout(FRAME_INTERVAL) {
            Ok(request) => Some(request),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let alive = super::os::frame_pool(|| {
            if let Some(request) = request {
                map.apply(request);
                for request in requests.try_iter() {
                    map.apply(request);
                }
            }
            run_loop.tick();
            map.draw_if_needed()
        });
        if !alive {
            return;
        }
    }
}

fn render_snapshot(request: &SnapshotRequest, tile_cache: PathBuf) -> Result<MapFrame, SnapshotError> {
    let viewport = Viewport {
        width: f64::from(request.width),
        height: f64::from(request.height),
        scale_factor: request.scale_factor,
    };
    let (width, height) = logical_size(&viewport);
    let mut renderer = mln::ImageRendererBuilder::new()
        .with_size(width, height)
        .with_pixel_ratio(request.scale_factor.max(1.0))
        .with_resource_options(resources(tile_cache))
        .build_static_renderer();
    renderer
        .load_style_from_json_str(&request.style_json)
        .wait()
        .map_err(|error| SnapshotError::Style(error.to_string()))?;
    let image = renderer
        .render_static(&camera_update(&request.camera.clamped()))
        .map_err(|error| SnapshotError::Renderer(error.to_string()))?;
    let pixels = image.as_image();
    MapFrame::from_rgba(pixels.width(), pixels.height(), pixels.as_raw(), request.camera.clamped())
        .ok_or_else(|| SnapshotError::Renderer("the snapshot frame had no pixels".to_owned()))
}

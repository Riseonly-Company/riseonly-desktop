use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    div, hsla, img, point, px, white, AnyView, App, AppContext, BoxShadow, Context, FontWeight, Hsla, ImageSource,
    IntoElement, ObjectFit, ParentElement, Render, RenderImage, SharedString, Styled, StyledImage, Task, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::{LatLng, ScreenPoint};
use rise_theme::ui_color_system::UIColor;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::IconUi;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;
use crate::modules::map::components::pins::map_pin_decor::MapPinZoom;
use crate::modules::map::components::rise_map_host::{MapAnnotation, MapAnnotationView, RiseMapHost};
use crate::modules::map::engine::core::rise_map_photo_upload::MapPhotoURL;
use crate::modules::map::shared::map_gallery_export::MapGalleryThumbnail;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;

pub struct MapPhotoAnnotation {
    pub key: String,
    pub members: RefCell<Vec<MapPhotoGrid>>,
    pub coordinate: RefCell<LatLng>,
}

impl MapPhotoAnnotation {
    pub fn is_stack(&self) -> bool {
        self.members.borrow().len() > 1
    }
}

impl MapAnnotation for MapPhotoAnnotation {
    fn coordinate(&self) -> LatLng {
        *self.coordinate.borrow()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// what the print shows, a decoded bitmap or a local file read by the image element
#[derive(Clone)]
enum MapPhotoPinImage {
    Bitmap(Arc<RenderImage>),
    File(PathBuf),
}

/// a photo on the route, small rounded print with a white edge
/// corner count when several were taken close together
pub struct MapPhotoPinView {
    selection_state: Option<bool>,
    selection_color: Option<Hsla>,
    image_key: Option<String>,
    image: Option<MapPhotoPinImage>,
    image_background: Option<Hsla>,
    plate_alpha: f32,
    badge: Option<(String, Hsla)>,
    load_task: Option<Task<()>>,
    load_generation: u64,
    zoom_scale: f64,
    is_highlighted_pin: bool,
    transform: Animated<f32>,
}

impl MapPhotoPinView {
    pub const REUSE_ID: &'static str = "rise.map.photo";

    pub const SIDE: f64 = 31.0;
    const EDGE: f64 = 2.0;
    const CORNER: f64 = 9.0;
    const BADGE_HEIGHT: f64 = 15.0;
    const BADGE_CAP: i64 = 99;

    pub fn new(_reuse_identifier: Option<SharedString>, _cx: &mut Context<Self>) -> Self {
        Self {
            selection_state: None,
            selection_color: None,
            image_key: None,
            image: None,
            image_background: None,
            plate_alpha: 1.0,
            badge: None,
            load_task: None,
            load_generation: 0,
            zoom_scale: 1.0,
            is_highlighted_pin: false,
            transform: Animated::new(1.0),
        }
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.cancel_load();
        self.image_key = None;
        self.image = None;
        self.selection_state = None;
        self.selection_color = None;
        self.is_highlighted_pin = false;
        self.apply_transform(false, cx);
    }

    /// far out a print settles at the same smaller size as the avatars and stays
    pub fn apply_zoom(&mut self, zoom: f64, cx: &mut Context<Self>) {
        let scale = f64::from(MapPinZoom::scale(zoom));
        if (scale - self.zoom_scale).abs() <= 0.004 {
            return;
        }
        self.zoom_scale = scale;
        self.apply_transform(false, cx);
    }

    fn apply_transform(&mut self, animated: bool, cx: &mut Context<Self>) {
        let scale = (self.zoom_scale * if self.is_highlighted_pin { 1.22 } else { 1.0 }) as f32;
        if animated {
            self.transform.set(scale, spring(0.3, 0.7), RiseClock::monotonic(), false);
        } else {
            self.transform.jump(scale);
        }
        cx.notify();
    }

    pub fn configure(&mut self, members: &[MapPhotoGrid], palette: &MapPinPalette, cx: &mut Context<Self>) {
        let Some(cover) = members.first() else {
            return;
        };
        self.image_background = Some(Hsla { a: 0.35, ..palette.stale });
        self.plate_alpha = if cover.is_pending {
            0.7
        } else if cover.is_draft {
            0.82
        } else {
            1.0
        };

        let held: i64 = members.iter().map(|member| member.photo_count).sum();
        self.badge = (held > 1).then(|| {
            let text = if held > Self::BADGE_CAP {
                format!("{}+", Self::BADGE_CAP)
            } else {
                held.to_string()
            };
            let fill = if cover.is_draft { palette.stale } else { palette.primary };
            (text, fill)
        });
        self.load(cover, cx);
        cx.notify();
    }

    pub fn set_highlighted(&mut self, highlighted: bool, animated: bool, cx: &mut Context<Self>) {
        if highlighted == self.is_highlighted_pin {
            return;
        }
        self.is_highlighted_pin = highlighted;
        self.apply_transform(animated, cx);
    }

    pub fn set_selection_indicator(&mut self, selected: Option<bool>, color: Hsla, cx: &mut Context<Self>) {
        if selected == self.selection_state && Some(color) == self.selection_color {
            return;
        }
        self.selection_state = selected;
        self.selection_color = Some(color);
        cx.notify();
    }

    /// small decode of the same protected file the card opens
    /// memory only, a private picture must not reach the disk cache
    fn load(&mut self, grid: &MapPhotoGrid, cx: &mut Context<Self>) {
        if let Some(local_identifier) = grid.cover_url.strip_prefix("ph://") {
            self.load_gallery_cover(local_identifier.to_string(), cx);
            return;
        }
        let Some(url) = MapPhotoURL::absolute(&grid.cover_url) else {
            self.cancel_load();
            self.image_key = None;
            self.image = None;
            return;
        };
        if url.starts_with("file://") {
            if let Some(file) = Url::from_string(&url).and_then(|file| file.to_file_path()) {
                self.cancel_load();
                self.image_key = Some(url);
                self.image = Some(MapPhotoPinImage::File(file));
                return;
            }
        }

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let pixels = (Self::SIDE * 3.0).ceil() as i64;
        let key = format!(
            "{url}|{}|{}",
            account_user_id.as_deref().unwrap_or("anonymous"),
            account_epoch.as_deref().unwrap_or("none")
        );
        if Some(&key) == self.image_key.as_ref() {
            return;
        }
        self.cancel_load();
        self.image_key = Some(key);
        self.image = None;

        if let Some(hit) = CleverImageBitmapLoader::memory_image(
            &url,
            false,
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            Some(pixels),
            CleverImageCachePolicy::MemoryOnly,
            None,
            false,
        ) {
            self.image = Some(MapPhotoPinImage::Bitmap(hit));
            return;
        }

        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let image = CleverImageBitmapLoader::load(
                &url,
                false,
                true,
                account_user_id,
                account_epoch,
                Some(pixels),
                CleverImageCachePolicy::MemoryOnly,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await;
            let _ = this.update(cx, |view, cx| {
                let Some(image) = image else {
                    return;
                };
                if view.load_generation != generation {
                    return;
                }
                view.image = Some(MapPhotoPinImage::Bitmap(image));
                cx.notify();
            });
        }));
    }

    fn load_gallery_cover(&mut self, local_identifier: String, cx: &mut Context<Self>) {
        let key = format!("ph|{local_identifier}");
        if Some(&key) == self.image_key.as_ref() {
            return;
        }
        self.cancel_load();
        self.image_key = Some(key);
        self.image = None;
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let image = MapGalleryThumbnail::load_local_identifier(&local_identifier, Self::SIDE as f32).await;
            let _ = this.update(cx, |view, cx| {
                let Some(image) = image else {
                    return;
                };
                if view.load_generation != generation {
                    return;
                }
                view.image = Some(MapPhotoPinImage::Bitmap(image));
                cx.notify();
            });
        }));
    }

    fn cancel_load(&mut self) {
        self.load_task = None;
        self.load_generation = self.load_generation.wrapping_add(1);
    }
}

impl Render for MapPhotoPinView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = f64::from(self.transform.frame_value(RiseClock::monotonic(), window));
        let side = Self::SIDE * scale;
        let edge = Self::EDGE * scale;
        let corner = Self::CORNER * scale;
        let badge_height = Self::BADGE_HEIGHT * scale;
        let secondary_label = UIColor::secondary_label().resolved_color(rise_ui::theme(cx).appearance);

        let photo = self.image.clone().map(|image| {
            let source = match image {
                MapPhotoPinImage::Bitmap(bitmap) => ImageSource::Render(bitmap),
                MapPhotoPinImage::File(path) => ImageSource::from(path),
            };
            img(source).size_full().object_fit(ObjectFit::Cover)
        });

        let selection = self.selection_state.map(|selected| {
            let (symbol, tint) = if selected {
                ("checkmark.circle.fill", self.selection_color.unwrap_or(secondary_label))
            } else {
                ("circle", secondary_label)
            };
            div()
                .absolute()
                .left(px((-6.0 * scale) as f32))
                .top(px((-6.0 * scale) as f32))
                .size(px((18.0 * scale) as f32))
                .rounded(px((9.0 * scale) as f32))
                .bg(white())
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized(symbol, px((18.0 * scale) as f32), tint, selected))
        });

        let badge = self.badge.clone().map(|(text, fill)| {
            div()
                .absolute()
                .right(px((-5.0 * scale) as f32))
                .top(px((-5.0 * scale) as f32))
                .h(px(badge_height as f32))
                .min_w(px(badge_height as f32))
                .px(px((5.0 * scale) as f32))
                .rounded(px((badge_height / 2.0) as f32))
                .overflow_hidden()
                .bg(fill)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px((9.5 * scale) as f32))
                .font_weight(FontWeight::BOLD)
                .text_color(white())
                .child(text)
        });

        div()
            .relative()
            .size(px(side as f32))
            .rounded(px(corner as f32))
            .shadow(vec![BoxShadow {
                color: hsla(0.0, 0.0, 0.0, 0.28),
                offset: point(px(0.0), px((2.0 * scale) as f32)),
                blur_radius: px((10.0 * scale) as f32),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(corner as f32))
                    .bg(white())
                    .opacity(self.plate_alpha),
            )
            .child(
                div()
                    .absolute()
                    .left(px(edge as f32))
                    .top(px(edge as f32))
                    .size(px((side - 2.0 * edge) as f32))
                    .rounded(px((corner - edge) as f32))
                    .overflow_hidden()
                    .when_some(self.image_background, |this, background| this.bg(background))
                    .children(photo),
            )
            .children(badge)
            .children(selection)
    }
}

/// photos as annotations by the same rules as friends
/// diffed by id and folded into a stack on overlap
/// regrouped when the camera settles, views count only in viewport
pub struct MapPhotoPinLayer {
    pub on_select: Rc<dyn Fn(Option<String>, &mut Window, &mut App)>,

    this: Weak<RefCell<MapPhotoPinLayer>>,
    photos: Vec<MapPhotoGrid>,
    revision: u64,
    is_visible: bool,
    selected_id: Option<String>,
    palette: Option<MapPinPalette>,
    annotations: HashMap<String, Rc<MapPhotoAnnotation>>,
    last_grouping_zoom: f64,
    pending_regroup_id: Option<Uuid>,
    is_regrouping: bool,
}

impl MapPhotoPinLayer {
    const GROUPING_DISTANCE: f64 = 34.0;
    const REGROUP_ZOOM_STEP: f64 = 0.5;
    const STACK_ZOOM_STEP: f64 = 2.5;
    const FOCUS_ZOOM_LEVEL: f64 = 12.5;

    /// shared with the coordinator, the deferred regroup reaches the layer weakly
    pub fn new() -> Rc<RefCell<Self>> {
        Rc::new_cyclic(|this| {
            RefCell::new(Self {
                on_select: Rc::new(|_, _, _| {}),
                this: this.clone(),
                photos: Vec::new(),
                revision: u64::MAX,
                is_visible: false,
                selected_id: None,
                palette: None,
                annotations: HashMap::new(),
                last_grouping_zoom: -1.0,
                pending_regroup_id: None,
                is_regrouping: false,
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        photos: &[MapPhotoGrid],
        revision: u64,
        is_visible: bool,
        selected_id: Option<&str>,
        palette: &MapPinPalette,
        map_view: &RiseMapHost,
        cx: &mut App,
    ) {
        let photos_changed = revision != self.revision || is_visible != self.is_visible;
        let selection_changed = selected_id != self.selected_id.as_deref();
        let palette_changed = Some(palette) != self.palette.as_ref();
        if !(photos_changed || selection_changed || palette_changed) {
            return;
        }

        let previous_selection = self.selected_id.clone();
        self.photos = if is_visible { photos.to_vec() } else { Vec::new() };
        self.revision = revision;
        self.is_visible = is_visible;
        self.selected_id = selected_id.map(str::to_string);
        self.palette = Some(palette.clone());

        self.regroup(map_view, cx);
        if selection_changed {
            self.highlight(previous_selection.as_deref(), false, map_view, cx);
            self.highlight(selected_id, true, map_view, cx);
            if let Some(selected_id) = selected_id {
                self.centre(selected_id, map_view, cx);
            }
        }
    }

    pub fn region_is_changing(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        if self.photos.is_empty() {
            return;
        }
        let zoom = map_view.zoom_level();
        for annotation in map_view.visible_annotations() {
            if annotation.as_any().downcast_ref::<MapPhotoAnnotation>().is_none() {
                continue;
            }
            if let Some(view) = map_view.view_for(&annotation).and_then(|view| view.downcast::<MapPhotoPinView>().ok()) {
                view.update(cx, |view, cx| view.apply_zoom(zoom, cx));
            }
        }
        if (zoom - self.last_grouping_zoom).abs() < Self::REGROUP_ZOOM_STEP {
            return;
        }
        self.schedule_regroup(map_view, cx);
    }

    pub fn region_did_change(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        if self.photos.is_empty() {
            return;
        }
        self.schedule_regroup(map_view, cx);
    }

    pub fn teardown(&mut self, _map_view: &RiseMapHost) {
        self.pending_regroup_id = None;
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
            layer.regroup(&map_view, cx);
        });
    }

    pub fn view(&self, annotation: &Rc<dyn MapAnnotation>, map_view: &RiseMapHost, cx: &mut App) -> Option<MapAnnotationView> {
        let pin = annotation.as_any().downcast_ref::<MapPhotoAnnotation>()?;
        let palette = self.palette.clone()?;
        let members = pin.members.borrow().clone();
        let highlighted =
            !pin.is_stack() && members.first().map(|photo| photo.id.as_str()) == self.selected_id.as_deref();
        let zoom = map_view.zoom_level();
        let view = map_view
            .dequeue_reusable_annotation_view(MapPhotoPinView::REUSE_ID)
            .and_then(|view| view.view.downcast::<MapPhotoPinView>().ok())
            .unwrap_or_else(|| cx.new(|cx| MapPhotoPinView::new(Some(SharedString::from(MapPhotoPinView::REUSE_ID)), cx)));
        view.update(cx, |view, cx| {
            view.prepare_for_reuse(cx);
            view.apply_zoom(zoom, cx);
            view.configure(&members, &palette, cx);
            view.set_highlighted(highlighted, false, cx);
        });
        Some(MapAnnotationView::new(AnyView::from(view), MapPhotoPinView::REUSE_ID, MapPhotoPinView::SIDE, MapPhotoPinView::SIDE))
    }

    pub fn did_select(&mut self, annotation: &Rc<dyn MapAnnotation>, map_view: &RiseMapHost, window: &mut Window, cx: &mut App) -> bool {
        let Some(pin) = annotation.as_any().downcast_ref::<MapPhotoAnnotation>() else {
            return false;
        };
        map_view.deselect_annotation(annotation);
        let first_id = pin.members.borrow().first().map(|photo| photo.id.clone());
        let on_select = self.on_select.clone();
        if pin.is_stack() {
            let zoom = map_view.maximum_zoom_level().min(map_view.zoom_level() + Self::STACK_ZOOM_STEP);
            match first_id {
                Some(first_id) if zoom - map_view.zoom_level() < 0.2 => on_select(Some(first_id), window, cx),
                _ => {
                    let centre = *pin.coordinate.borrow();
                    map_view.set_center(centre, Some(zoom), None, true, cx);
                }
            }
        } else if let Some(photo_id) = first_id {
            on_select(Some(photo_id), window, cx);
        }
        true
    }

    fn centre(&self, photo_id: &str, map_view: &RiseMapHost, cx: &mut App) {
        let Some(photo) = self.photos.iter().find(|photo| photo.id == photo_id) else {
            return;
        };
        let zoom = map_view.zoom_level().max(Self::FOCUS_ZOOM_LEVEL);
        map_view.set_center(photo.coordinate(), Some(zoom), None, true, cx);
    }

    fn regroup(&mut self, map_view: &RiseMapHost, cx: &mut App) {
        if self.is_regrouping || map_view.bounds_width() <= 0.0 {
            return;
        }
        self.is_regrouping = true;
        self.last_grouping_zoom = map_view.zoom_level();

        let mut groups: Vec<(Option<ScreenPoint>, Vec<MapPhotoGrid>)> = Vec::new();
        for photo in &self.photos {
            if self.selected_id.as_deref() == Some(photo.id.as_str()) {
                groups.push((None, vec![photo.clone()]));
                continue;
            }
            let point = map_view.convert_coordinate_to_point(photo.coordinate());
            let index = point.and_then(|point| {
                groups.iter().position(|(seed, _)| {
                    seed.is_some_and(|seed| (seed.x - point.x).hypot(seed.y - point.y) < Self::GROUPING_DISTANCE)
                })
            });
            match index {
                Some(index) => groups[index].1.push(photo.clone()),
                None => groups.push((point, vec![photo.clone()])),
            }
        }

        let mut desired: HashMap<String, Vec<MapPhotoGrid>> = HashMap::new();
        for (_, members) in groups {
            let Some(first) = members.first() else {
                continue;
            };
            let key = if members.len() == 1 { first.id.clone() } else { format!("stack|{}", first.id) };
            desired.insert(key, members);
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
        for (key, members) in desired {
            let Some(cover) = members.first() else {
                continue;
            };
            let cover_coordinate = cover.coordinate();
            if let Some(existing) = self.annotations.get(&key).cloned() {
                let changed = *existing.members.borrow() != members;
                *existing.members.borrow_mut() = members.clone();
                if *existing.coordinate.borrow() != cover_coordinate {
                    *existing.coordinate.borrow_mut() = cover_coordinate;
                    placement_changed = true;
                }
                let handle: Rc<dyn MapAnnotation> = existing;
                let view = map_view.view_for(&handle).and_then(|view| view.downcast::<MapPhotoPinView>().ok());
                if let (true, Some(palette), Some(view)) = (changed, self.palette.clone(), view) {
                    view.update(cx, |view, cx| view.configure(&members, &palette, cx));
                }
            } else {
                let annotation = Rc::new(MapPhotoAnnotation {
                    key: key.clone(),
                    members: RefCell::new(members),
                    coordinate: RefCell::new(cover_coordinate),
                });
                self.annotations.insert(key, annotation.clone());
                added.push(annotation);
            }
        }
        if !added.is_empty() {
            map_view.add_annotations(added);
            placement_changed = true;
        }
        if placement_changed {
            map_view.set_needs_display(cx);
        }
        self.is_regrouping = false;
    }

    fn highlight(&self, photo_id: Option<&str>, highlighted: bool, map_view: &RiseMapHost, cx: &mut App) {
        let Some(annotation) = photo_id.and_then(|photo_id| self.annotations.get(photo_id)).cloned() else {
            return;
        };
        let annotation: Rc<dyn MapAnnotation> = annotation;
        let Some(view) = map_view.view_for(&annotation).and_then(|view| view.downcast::<MapPhotoPinView>().ok()) else {
            return;
        };
        view.update(cx, |view, cx| view.set_highlighted(highlighted, true, cx));
    }
}

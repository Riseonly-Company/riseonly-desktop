use std::cell::RefCell;
use std::collections::HashSet;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Corners, DispatchPhase, Entity, FillOptions, FocusHandle, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Path, PathBuilder, PathStyle, PinchEvent, Pixels, Point, Render, RenderImage, ScrollWheelEvent, Size,
    Styled, Subscription, Task, TouchPhase, WeakEntity, Window, canvas, div, point, px, white,
};
use image::{Frame, RgbaImage};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::RiseAppEvents;
use rise_ui::swiftui::measure::measure;

use crate::modules::media_picker::editor::editor_canvas_photo_entity_view::EditorCanvasPhotoEntityView;
use crate::modules::media_picker::editor::editor_entity_snapping::{
    EditorEntitySideHandle, EditorEntitySnap, EditorEntitySnapFeedback,
};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_link_entity::{EDITOR_LINK_CUSTOMIZE, EditorLinkEntity, EditorLinkEntityView};
use crate::modules::media_picker::editor::editor_music_plate::EditorMusicPlateEntityView;
use crate::modules::media_picker::editor::editor_person_tag_entity_view::EditorPersonTagEntityView;
use crate::modules::media_picker::editor::editor_state::{
    EditorDrawStroke, EditorDrawStrokeTool, EditorImageState, EditorMusicPlateEntity, EditorPersonTagEntity,
    EditorTextEntity,
};
use crate::modules::media_picker::editor::editor_sticker_entity::{EditorStickerEntity, EditorStickerEntityView};
use crate::modules::media_picker::editor::editor_text_entity_view::EditorTextEntityView;
use crate::modules::media_picker::editor::editor_two_finger_transform::{
    EditorPassThroughView, EditorTwoFingerTransformable,
};
use crate::modules::media_picker::editor::story_canvas_model::EditorCanvasPhotoEntity;

const TAP_SLOP: f32 = 10.0;
const NUDGE: f32 = 1.0;
const SHIFT_NUDGE: f32 = 10.0;
const WHEEL_LINE_HEIGHT: f32 = 20.0;
const WHEEL_SCALE_PER_PIXEL: f32 = 0.005;
const WHEEL_DEGREES_PER_PIXEL: f32 = 0.25;
const DISC_TURN_COSINE: f32 = 0.978;
const MIN_POINT_DISTANCE: f32 = 0.5;
const NEON_GLOW_RINGS: [(f32, f32); 4] = [(8.0, 0.1), (6.0, 0.13), (4.0, 0.17), (2.0, 0.22)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorOverlayCanvasInteractionMode {
    Draw,
    TextPlacement,
    TagPlacement,
    EntityEdit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorOverlayCanvasImageOrientation {
    pub rotation_steps: i64,
    pub mirrored: bool,
    pub free_rotation_degrees: f32,
}

/// one child of the entities container in z order
#[derive(Clone)]
enum EditorOverlayEntityView {
    Text(Entity<EditorTextEntityView>),
    MusicPlate(Entity<EditorMusicPlateEntityView>),
    PersonTag(Entity<EditorPersonTagEntityView>),
    Sticker(Entity<EditorStickerEntityView>),
    Link(Entity<EditorLinkEntityView>),
}

/// thing the two finger gesture can hit
/// a text, sticker or link entity or a canvas photo
#[derive(Clone)]
enum EditorOverlayTwoFingerTarget {
    Text(Entity<EditorTextEntityView>),
    Sticker(Entity<EditorStickerEntityView>),
    Link(Entity<EditorLinkEntityView>),
    Photo(Entity<EditorCanvasPhotoEntityView>),
}

impl EditorOverlayTwoFingerTarget {
    fn entity_id(&self, cx: &App) -> Uuid {
        match self {
            Self::Text(view) => view.read(cx).entity_id(),
            Self::Sticker(view) => view.read(cx).entity_id(),
            Self::Link(view) => view.read(cx).entity_id(),
            Self::Photo(view) => view.read(cx).entity_id(),
        }
    }

    fn begin(&self, cx: &mut App) {
        match self {
            Self::Text(view) => view.update(cx, |view, cx| view.begin_two_finger_transform(cx)),
            Self::Sticker(view) => view.update(cx, |view, cx| view.begin_two_finger_transform(cx)),
            Self::Link(view) => view.update(cx, |view, cx| view.begin_two_finger_transform(cx)),
            Self::Photo(view) => view.update(cx, |view, cx| view.begin_two_finger_transform(cx)),
        }
    }

    fn apply(&self, scale: f32, rotation: f32, cx: &mut App) {
        match self {
            Self::Text(view) => view.update(cx, |view, cx| view.apply_two_finger_transform(scale, rotation, cx)),
            Self::Sticker(view) => view.update(cx, |view, cx| view.apply_two_finger_transform(scale, rotation, cx)),
            Self::Link(view) => view.update(cx, |view, cx| view.apply_two_finger_transform(scale, rotation, cx)),
            Self::Photo(view) => view.update(cx, |view, cx| view.apply_two_finger_transform(scale, rotation, cx)),
        }
    }

    fn end(&self, cx: &mut App) {
        match self {
            Self::Text(view) => view.update(cx, |view, cx| view.end_two_finger_transform(cx)),
            Self::Sticker(view) => view.update(cx, |view, cx| view.end_two_finger_transform(cx)),
            Self::Link(view) => view.update(cx, |view, cx| view.end_two_finger_transform(cx)),
            Self::Photo(view) => view.update(cx, |view, cx| view.end_two_finger_transform(cx)),
        }
    }
}

/// what the held left button turned out to be
/// a draw pan starts at once, an empty tap waits for release
#[derive(Clone, Copy)]
enum EditorOverlayPress {
    Draw,
    Tap(Point<f32>),
}

/// blur brush shows the blurred photo through its stroke
/// gpui has no layer mask so a run of blur strokes is the blurred photo with alpha cut to them
#[derive(Clone)]
struct EditorOverlayBlurLayer {
    first_index: usize,
    last_index: usize,
    strokes: Vec<EditorDrawStroke>,
    image: Arc<RenderImage>,
}

struct EditorOverlayBlurRun {
    first_index: usize,
    last_index: usize,
    strokes: Vec<EditorDrawStroke>,
}

type EditorOverlayStrokePaths = Vec<(Path<Pixels>, Hsla)>;

/// tessellated strokes cache so an unrelated repaint like a dragged text doesnt redo them
#[derive(Default)]
struct EditorOverlayStrokeCache {
    key: Option<(u64, Bounds<Pixels>)>,
    layers: Vec<EditorOverlayStrokePaths>,
    preview_key: Option<(u64, Bounds<Pixels>)>,
    preview: EditorOverlayStrokePaths,
}

/// everything the paint pass needs, taken out of the view at render
struct EditorOverlayStrokesFrame {
    generation: u64,
    display_zoom: f32,
    strokes: Rc<Vec<EditorDrawStroke>>,
    preview_generation: u64,
    preview: Option<Rc<EditorDrawStroke>>,
    hidden: Rc<HashSet<usize>>,
    blur_layers: Rc<Vec<EditorOverlayBlurLayer>>,
    cache: Rc<RefCell<EditorOverlayStrokeCache>>,
}

pub struct EditorOverlayCanvas {
    brush_tool: EditorDrawStrokeTool,
    pub brush_color: Hsla,
    pub brush_width_fraction: f32,
    interaction_mode: EditorOverlayCanvasInteractionMode,
    pub image_orientation: EditorOverlayCanvasImageOrientation,

    pub on_state_changed: Option<Rc<dyn Fn(EditorImageState, &mut App)>>,
    pub on_request_edit_text: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_request_create_text: Option<Rc<dyn Fn(Point<f32>, &mut App)>>,
    pub on_request_create_tag: Option<Rc<dyn Fn(Point<f32>, &mut App)>>,
    pub on_text_focus_changed: Option<Rc<dyn Fn(Option<Uuid>, &mut App)>>,

    photo_views: Vec<Entity<EditorCanvasPhotoEntityView>>,
    entity_views: Vec<EditorOverlayEntityView>,
    plate_view: Option<Entity<EditorMusicPlateEntityView>>,
    snap_guide_vertical: bool,
    snap_guide_horizontal: bool,
    current_stroke_points: Vec<Point<f32>>,
    preview_layer: Option<Rc<EditorDrawStroke>>,
    preview_generation: u64,
    painted_strokes: Rc<Vec<EditorDrawStroke>>,
    strokes_generation: u64,
    hidden_strokes: Rc<HashSet<usize>>,
    stroke_cache: Rc<RefCell<EditorOverlayStrokeCache>>,

    blur_source_image: Option<Arc<RenderImage>>,
    blurred_image: Option<Arc<RenderImage>>,
    blur_generation: u64,
    is_preparing_blur: bool,
    prepare_blur_task: Option<Task<()>>,
    blur_layers: Rc<Vec<EditorOverlayBlurLayer>>,
    blur_compose_task: Option<Task<()>>,
    blur_compose_dirty: bool,

    is_hidden: bool,
    state: EditorImageState,
    selected_entity_id: Option<Uuid>,
    editing_text_entity_id: Option<Uuid>,
    two_finger_target: Option<EditorOverlayTwoFingerTarget>,
    two_finger_scale: f32,

    canvas_bounds: Bounds<Pixels>,
    display_zoom: f32,
    press: Option<EditorOverlayPress>,
    focus_handle: FocusHandle,
    focus_on_render: bool,
    snap_feedback: EditorEntitySnapFeedback,
    wheel_rotation: Option<(Uuid, f32)>,
    _release_images: Subscription,
}

impl EditorOverlayCanvas {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let release_images = cx.on_release(|this: &mut Self, cx: &mut App| {
            let mut images: Vec<Arc<RenderImage>> = this.blur_layers.iter().map(|layer| layer.image.clone()).collect();
            images.extend(this.blurred_image.take());
            Self::drop_images(images, cx);
        });
        Self {
            brush_tool: EditorDrawStrokeTool::Pen,
            brush_color: white(),
            brush_width_fraction: 0.04,
            interaction_mode: EditorOverlayCanvasInteractionMode::EntityEdit,
            image_orientation: EditorOverlayCanvasImageOrientation {
                rotation_steps: 0,
                mirrored: false,
                free_rotation_degrees: 0.0,
            },
            on_state_changed: None,
            on_request_edit_text: None,
            on_request_create_text: None,
            on_request_create_tag: None,
            on_text_focus_changed: None,
            photo_views: Vec::new(),
            entity_views: Vec::new(),
            plate_view: None,
            snap_guide_vertical: false,
            snap_guide_horizontal: false,
            current_stroke_points: Vec::new(),
            preview_layer: None,
            preview_generation: 0,
            painted_strokes: Rc::new(Vec::new()),
            strokes_generation: 0,
            hidden_strokes: Rc::new(HashSet::new()),
            stroke_cache: Rc::new(RefCell::new(EditorOverlayStrokeCache::default())),
            blur_source_image: None,
            blurred_image: None,
            blur_generation: 0,
            is_preparing_blur: false,
            prepare_blur_task: None,
            blur_layers: Rc::new(Vec::new()),
            blur_compose_task: None,
            blur_compose_dirty: false,
            is_hidden: false,
            state: EditorImageState::empty(None, Size { width: 0.0, height: 0.0 }),
            selected_entity_id: None,
            editing_text_entity_id: None,
            two_finger_target: None,
            two_finger_scale: 1.0,
            canvas_bounds: Bounds::default(),
            display_zoom: 1.0,
            press: None,
            focus_handle: cx.focus_handle(),
            focus_on_render: false,
            snap_feedback: EditorEntitySnapFeedback::new(),
            wheel_rotation: None,
            _release_images: release_images,
        }
    }

    pub fn brush_tool(&self) -> EditorDrawStrokeTool {
        self.brush_tool
    }

    pub fn set_brush_tool(&mut self, brush_tool: EditorDrawStrokeTool, cx: &mut Context<Self>) {
        self.brush_tool = brush_tool;
        self.prepare_brush_blur_if_needed(cx);
    }

    pub fn interaction_mode(&self) -> EditorOverlayCanvasInteractionMode {
        self.interaction_mode
    }

    pub fn set_interaction_mode(&mut self, interaction_mode: EditorOverlayCanvasInteractionMode, cx: &mut Context<Self>) {
        let old_value = self.interaction_mode;
        self.interaction_mode = interaction_mode;
        if interaction_mode == EditorOverlayCanvasInteractionMode::Draw
            && old_value != EditorOverlayCanvasInteractionMode::Draw
            && self.selected_entity_id.is_some()
        {
            self.select_entity(None, cx);
            return;
        }
        self.update_gesture_enable(cx);
    }

    pub fn editing_text_entity_id(&self) -> Option<Uuid> {
        self.editing_text_entity_id
    }

    pub fn set_editing_text_entity_id(&mut self, editing_text_entity_id: Option<Uuid>, cx: &mut Context<Self>) {
        if self.editing_text_entity_id == editing_text_entity_id {
            return;
        }
        self.editing_text_entity_id = editing_text_entity_id;
        cx.notify();
    }

    pub fn set_blur_source_image(&mut self, image: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        let unchanged = match (&self.blur_source_image, &image) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        self.blur_source_image = image;
        if unchanged {
            return;
        }
        self.blur_generation = self.blur_generation.wrapping_add(1);
        let mut released: Vec<Arc<RenderImage>> = self.blur_layers.iter().map(|layer| layer.image.clone()).collect();
        released.extend(self.blurred_image.take());
        Self::drop_images(released, cx);
        self.blur_layers = Rc::new(Vec::new());
        self.is_preparing_blur = false;
        self.prepare_blur_task = None;
        self.blur_compose_task = None;
        self.blur_compose_dirty = false;
        self.prepare_brush_blur_if_needed(cx);
        cx.notify();
    }

    pub fn is_hidden(&self) -> bool {
        self.is_hidden
    }

    pub fn set_is_hidden(&mut self, is_hidden: bool, cx: &mut Context<Self>) {
        let changed = self.is_hidden != is_hidden;
        self.is_hidden = is_hidden;
        if !is_hidden {
            self.prepare_brush_blur_if_needed(cx);
        }
        if changed {
            cx.notify();
        }
    }

    pub fn selected_entity_id(&self) -> Option<Uuid> {
        self.selected_entity_id
    }

    pub fn set_display_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if self.display_zoom == zoom {
            return;
        }
        self.display_zoom = zoom;
        for view in self.entity_views.clone() {
            match view {
                EditorOverlayEntityView::PersonTag(view) => {
                    view.update(cx, |view, cx| view.set_display_zoom(zoom, cx));
                }
                EditorOverlayEntityView::MusicPlate(view) => {
                    view.update(cx, |view, cx| view.set_display_zoom(zoom, cx));
                }
                EditorOverlayEntityView::Text(view) => {
                    view.update(cx, |view, cx| view.set_display_zoom(zoom, cx));
                }
                EditorOverlayEntityView::Sticker(_) | EditorOverlayEntityView::Link(_) => {}
            }
        }
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.redraw_strokes(cx);
    }

    fn bounds_size(&self) -> Size<f32> {
        Size {
            width: f32::from(self.canvas_bounds.size.width),
            height: f32::from(self.canvas_bounds.size.height),
        }
    }

    fn local(&self, position: Point<Pixels>) -> Point<f32> {
        Point {
            x: f32::from(position.x - self.canvas_bounds.origin.x),
            y: f32::from(position.y - self.canvas_bounds.origin.y),
        }
    }

    fn layout_subviews(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let size_changed = self.canvas_bounds.size != bounds.size;
        self.canvas_bounds = bounds;
        if size_changed {
            self.redraw_strokes(cx);
            self.layout_entities(cx);
        }
        cx.notify();
    }

    fn set_snap_guides(&mut self, vertical: bool, horizontal: bool, cx: &mut Context<Self>) {
        self.snap_guide_vertical = vertical;
        self.snap_guide_horizontal = horizontal;
        cx.notify();
    }

    pub fn apply(&mut self, incoming: EditorImageState, cx: &mut Context<Self>) {
        let state = self.keeping_stacking_order(incoming);
        let strokes_changed = state.strokes != self.state.strokes;
        let texts_changed = state.texts != self.state.texts;
        let plate_identity_changed =
            state.music_plate.as_ref().map(|plate| plate.id) != self.state.music_plate.as_ref().map(|plate| plate.id);
        let tag_identities_changed =
            Self::tag_identities(state.person_tags.as_deref()) != Self::tag_identities(self.state.person_tags.as_deref());
        let old_sticker_ids: Vec<Uuid> = self.state.stickers.iter().flatten().map(|sticker| sticker.id).collect();
        let new_sticker_ids: Vec<Uuid> = state.stickers.iter().flatten().map(|sticker| sticker.id).collect();
        let sticker_identities_changed = new_sticker_ids != old_sticker_ids;
        let added_sticker = new_sticker_ids.last().copied().filter(|id| !old_sticker_ids.contains(id));
        let old_photo_ids: Vec<Uuid> =
            self.state.story_canvas.iter().flat_map(|canvas| canvas.photos.iter()).map(|photo| photo.id).collect();
        let new_photo_ids: Vec<Uuid> =
            state.story_canvas.iter().flat_map(|canvas| canvas.photos.iter()).map(|photo| photo.id).collect();
        let photo_identities_changed = new_photo_ids != old_photo_ids;
        let added_photo = if self.state.story_canvas.is_none() {
            None
        } else {
            new_photo_ids.iter().rev().find(|id| !old_photo_ids.contains(id)).copied()
        };
        let old_link_ids: Vec<Uuid> = self.state.links.iter().flatten().map(|link| link.id).collect();
        let new_link_ids: Vec<Uuid> = state.links.iter().flatten().map(|link| link.id).collect();
        let link_identities_changed = new_link_ids != old_link_ids;
        let added_link = new_link_ids.last().copied().filter(|id| !old_link_ids.contains(id));
        self.state = state;
        if photo_identities_changed {
            self.rebuild_photos(cx);
            if added_photo.is_some() {
                self.select_entity(added_photo, cx);
            }
        }
        if link_identities_changed {
            self.rebuild_links(cx);
            if added_link.is_some() {
                self.select_entity(added_link, cx);
            }
        }
        if strokes_changed {
            self.redraw_strokes(cx);
            self.prepare_brush_blur_if_needed(cx);
        }
        if plate_identity_changed {
            self.rebuild_plate(self.state.music_plate.is_some(), cx);
        }
        if tag_identities_changed {
            self.rebuild_person_tags(cx);
        }
        if sticker_identities_changed {
            self.rebuild_stickers(cx);
            if added_sticker.is_some() {
                self.select_entity(added_sticker, cx);
            }
        }
        if texts_changed {
            self.rebuild_entities(cx);
        } else {
            self.layout_entities(cx);
        }
        self.apply_stacking_order(cx);
    }

    fn keeping_stacking_order(&self, incoming: EditorImageState) -> EditorImageState {
        let mut result = incoming;
        let entity_ids: HashSet<Uuid> = result
            .texts
            .iter()
            .map(|text| text.id)
            .chain(result.stickers.iter().flatten().map(|sticker| sticker.id))
            .chain(result.links.iter().flatten().map(|link| link.id))
            .collect();
        if let Some(own) = self.state.entity_order.as_ref() {
            let mut order: Vec<Uuid> = own.iter().copied().filter(|id| entity_ids.contains(id)).collect();
            for id in result.entity_order.iter().flatten() {
                if entity_ids.contains(id) && !order.contains(id) {
                    order.push(*id);
                }
            }
            result.entity_order = if order.is_empty() { None } else { Some(order) };
        }
        let own_photos = self.state.story_canvas.as_ref().map(|canvas| canvas.photos.as_slice()).unwrap_or_default();
        if !own_photos.is_empty()
            && let Some(canvas) = result.story_canvas.as_mut()
        {
            let rank = |id: Uuid, offset: usize| {
                own_photos.iter().position(|photo| photo.id == id).unwrap_or(own_photos.len() + offset)
            };
            let mut ranked: Vec<(usize, EditorCanvasPhotoEntity)> = std::mem::take(&mut canvas.photos)
                .into_iter()
                .enumerate()
                .map(|(offset, photo)| (rank(photo.id, offset), photo))
                .collect();
            ranked.sort_by_key(|(rank, _)| *rank);
            canvas.photos = ranked.into_iter().map(|(_, photo)| photo).collect();
        }
        result
    }

    fn view_entity_id(view: &EditorOverlayEntityView, cx: &App) -> Uuid {
        match view {
            EditorOverlayEntityView::Text(view) => view.read(cx).entity_id,
            EditorOverlayEntityView::MusicPlate(view) => view.read(cx).entity_id,
            EditorOverlayEntityView::PersonTag(view) => view.read(cx).entity_id,
            EditorOverlayEntityView::Sticker(view) => view.read(cx).entity_id,
            EditorOverlayEntityView::Link(view) => view.read(cx).entity_id,
        }
    }

    fn is_hidden_view(&self, view: &EditorOverlayEntityView, cx: &App) -> bool {
        matches!(view, EditorOverlayEntityView::Text(text) if Some(text.read(cx).entity_id) == self.editing_text_entity_id)
    }

    fn two_finger_target_of(view: &EditorOverlayEntityView) -> Option<EditorOverlayTwoFingerTarget> {
        match view {
            EditorOverlayEntityView::Text(view) => Some(EditorOverlayTwoFingerTarget::Text(view.clone())),
            EditorOverlayEntityView::Sticker(view) => Some(EditorOverlayTwoFingerTarget::Sticker(view.clone())),
            EditorOverlayEntityView::Link(view) => Some(EditorOverlayTwoFingerTarget::Link(view.clone())),
            EditorOverlayEntityView::MusicPlate(_) | EditorOverlayEntityView::PersonTag(_) => None,
        }
    }

    /// finds the view for an entity id
    /// include_hidden is the last fallback of the two finger lookup and skips the hidden check
    fn transformable_view(&self, id: Uuid, include_hidden: bool, cx: &App) -> Option<EditorOverlayTwoFingerTarget> {
        let entity = self
            .entity_views
            .iter()
            .filter(|view| include_hidden || !self.is_hidden_view(view, cx))
            .filter_map(Self::two_finger_target_of)
            .find(|target| target.entity_id(cx) == id);
        entity.or_else(|| {
            self.photo_views
                .iter()
                .find(|view| view.read(cx).entity_id == id)
                .map(|view| EditorOverlayTwoFingerTarget::Photo(view.clone()))
        })
    }

    /// element focused last stacks above the others, in editor and export
    /// photos stay one layer under the drawing
    fn raise(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let mut changed = false;
        let photo_index =
            self.state.story_canvas.as_ref().and_then(|canvas| canvas.photos.iter().position(|photo| photo.id == id));
        if let (Some(index), Some(canvas)) = (photo_index, self.state.story_canvas.as_mut()) {
            if index == 0 {
                return;
            }
            if index != canvas.photos.len() - 1 {
                let photo = canvas.photos.remove(index);
                canvas.photos.push(photo);
                changed = true;
            }
            if let Some(position) = self.photo_views.iter().position(|view| view.read(cx).entity_id == id) {
                let view = self.photo_views.remove(position);
                self.photo_views.push(view);
                cx.notify();
            }
        } else if self.state.texts.iter().any(|text| text.id == id)
            || self.state.stickers.iter().flatten().any(|sticker| sticker.id == id)
            || self.state.links.iter().flatten().any(|link| link.id == id)
        {
            let mut order = self.state.entity_order.clone().unwrap_or_default();
            if order.last() != Some(&id) {
                order.retain(|entry| *entry != id);
                order.push(id);
                self.state.entity_order = Some(order);
                changed = true;
            }
            self.apply_stacking_order(cx);
        }
        if !changed {
            return;
        }
        self.state.was_edited = true;
        self.fire_state_changed(cx);
    }

    fn apply_stacking_order(&mut self, cx: &mut Context<Self>) {
        for id in self.state.entity_order.clone().unwrap_or_default() {
            if let Some(position) = self.entity_views.iter().position(|view| Self::view_entity_id(view, cx) == id) {
                let view = self.entity_views.remove(position);
                self.entity_views.push(view);
            }
        }
        cx.notify();
    }

    fn tag_identities(tags: Option<&[EditorPersonTagEntity]>) -> Vec<Uuid> {
        tags.unwrap_or_default().iter().map(|tag| tag.id).collect()
    }

    /// fires the state changed callback
    /// owner calls apply on this view in response so it runs after this update is over
    fn fire_state_changed(&self, cx: &mut Context<Self>) {
        if let Some(on_state_changed) = self.on_state_changed.clone() {
            let state = self.state.clone();
            App::defer(cx, move |cx| on_state_changed(state, cx));
        }
    }

    fn drop_images(images: Vec<Arc<RenderImage>>, cx: &mut App) {
        if images.is_empty() {
            return;
        }
        cx.defer(move |cx| {
            for image in images {
                cx.drop_image(image, None);
            }
        });
    }

    fn redraw_strokes(&mut self, cx: &mut Context<Self>) {
        self.painted_strokes = Rc::new(self.state.strokes.clone());
        self.strokes_generation = self.strokes_generation.wrapping_add(1);
        self.hidden_strokes = Rc::new(HashSet::new());
        self.preview_layer = None;
        if self.brush_tool == EditorDrawStrokeTool::Eraser {
            let points = self.current_stroke_points.clone();
            self.preview_erasure(&points);
        }
        self.schedule_blur_compose(cx);
        cx.notify();
    }

    fn line_width(stroke: &EditorDrawStroke, size: Size<f32>, zoom: f32) -> f32 {
        (stroke.width * size.width.min(size.height)).max(2.0 * zoom)
    }

    /// builds the stroke paths and the colors they fill with, no layer
    fn make_stroke_layer(
        stroke: &EditorDrawStroke,
        size: Size<f32>,
        origin: Point<Pixels>,
        zoom: f32,
    ) -> EditorOverlayStrokePaths {
        let line_width = Self::line_width(stroke, size, zoom);
        let sub_paths = EditorImageBake::stroke_path(stroke, size, line_width);
        let mut layer = Vec::new();
        let mut push = |width: f32, color: Hsla| {
            if let Some(path) = Self::round_stroke_path(&sub_paths, width, origin) {
                layer.push((path, color));
            }
        };
        match stroke.tool {
            EditorDrawStrokeTool::Pen | EditorDrawStrokeTool::Pencil | EditorDrawStrokeTool::Arrow => {
                push(line_width, stroke.color);
            }
            EditorDrawStrokeTool::Marker => push(line_width, stroke.color.opacity(0.45)),
            EditorDrawStrokeTool::Neon => {
                for (extra, opacity) in NEON_GLOW_RINGS {
                    push(line_width + extra * 2.0 * zoom, stroke.color.opacity(opacity));
                }
                push(line_width, stroke.color);
            }
            EditorDrawStrokeTool::Blur => push(line_width, white().opacity(0.3)),
            EditorDrawStrokeTool::Eraser => {}
        }
        layer
    }

    /// outline of a stroke with round caps and joins as one non-zero fill
    /// so a translucent stroke doesnt darken where it overlaps itself
    pub(crate) fn round_stroke_path(sub_paths: &[Vec<Point<f32>>], line_width: f32, origin: Point<Pixels>) -> Option<Path<Pixels>> {
        let radius = line_width / 2.0;
        let at = |p: Point<f32>| point(origin.x + px(p.x), origin.y + px(p.y));
        let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::non_zero()));
        let mut is_empty = true;
        for sub_path in sub_paths {
            if sub_path.len() < 2 {
                continue;
            }
            is_empty = false;
            let mut kept: Vec<Point<f32>> = Vec::with_capacity(sub_path.len());
            for &p in sub_path {
                let far_enough = kept.last().is_none_or(|last: &Point<f32>| {
                    (p.x - last.x).hypot(p.y - last.y) >= MIN_POINT_DISTANCE
                });
                if far_enough {
                    kept.push(p);
                }
            }
            for pair in kept.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let length = (b.x - a.x).hypot(b.y - a.y);
                let nx = -(b.y - a.y) / length * radius;
                let ny = (b.x - a.x) / length * radius;
                builder.add_polygon(
                    &[
                        at(Point { x: a.x + nx, y: a.y + ny }),
                        at(Point { x: b.x + nx, y: b.y + ny }),
                        at(Point { x: b.x - nx, y: b.y - ny }),
                        at(Point { x: a.x - nx, y: a.y - ny }),
                    ],
                    true,
                );
            }
            for window in kept.windows(3) {
                Self::add_join(&mut builder, window[0], window[1], window[2], radius, &at);
            }
            for (index, &p) in kept.iter().enumerate() {
                let is_end = index == 0 || index + 1 == kept.len();
                if is_end || Self::turn_cosine(kept[index - 1], p, kept[index + 1]) < DISC_TURN_COSINE {
                    Self::add_disc(&mut builder, p, radius, &at);
                }
            }
        }
        if is_empty {
            return None;
        }
        if let Ok(path) = builder.build() {
            return Some(path);
        }
        let mut fallback = PathBuilder::stroke(px(line_width));
        for sub_path in sub_paths {
            let mut points = sub_path.iter().copied();
            if let Some(first) = points.next() {
                fallback.move_to(at(first));
                for p in points {
                    fallback.line_to(at(p));
                }
            }
        }
        fallback.build().ok()
    }

    fn turn_cosine(previous: Point<f32>, p: Point<f32>, next: Point<f32>) -> f32 {
        let (ax, ay) = (p.x - previous.x, p.y - previous.y);
        let (bx, by) = (next.x - p.x, next.y - p.y);
        (ax * bx + ay * by) / (ax.hypot(ay) * bx.hypot(by))
    }

    fn add_join(
        builder: &mut PathBuilder,
        previous: Point<f32>,
        p: Point<f32>,
        next: Point<f32>,
        radius: f32,
        at: &impl Fn(Point<f32>) -> Point<Pixels>,
    ) {
        let normal = |a: Point<f32>, b: Point<f32>| {
            let length = (b.x - a.x).hypot(b.y - a.y);
            (length > f32::EPSILON)
                .then(|| Point { x: -(b.y - a.y) / length * radius, y: (b.x - a.x) / length * radius })
        };
        let (Some(incoming), Some(outgoing)) = (normal(previous, p), normal(p, next)) else {
            return;
        };
        let bisector_x = incoming.x + outgoing.x;
        let bisector_y = incoming.y + outgoing.y;
        let bisector_length = bisector_x.hypot(bisector_y);
        if bisector_length <= f32::EPSILON {
            return;
        }
        for side in [1.0_f32, -1.0] {
            let mut fan = [
                p,
                Point { x: p.x + incoming.x * side, y: p.y + incoming.y * side },
                Point {
                    x: p.x + bisector_x / bisector_length * radius * side,
                    y: p.y + bisector_y / bisector_length * radius * side,
                },
                Point { x: p.x + outgoing.x * side, y: p.y + outgoing.y * side },
            ];
            let doubled_area: f32 = (0..fan.len())
                .map(|index| {
                    let (a, b) = (fan[index], fan[(index + 1) % fan.len()]);
                    a.x * b.y - b.x * a.y
                })
                .sum();
            if doubled_area.abs() <= 1e-3 {
                continue;
            }
            if doubled_area > 0.0 {
                fan.reverse();
            }
            builder.add_polygon(&fan.map(at), true);
        }
    }

    fn add_disc(builder: &mut PathBuilder, center: Point<f32>, radius: f32, at: &impl Fn(Point<f32>) -> Point<Pixels>) {
        let sides = (radius.ceil() as usize).clamp(8, 32);
        let points: Vec<Point<Pixels>> = (0..sides)
            .map(|side| {
                let angle = -2.0 * PI * side as f32 / sides as f32;
                at(Point { x: center.x + angle.cos() * radius, y: center.y + angle.sin() * radius })
            })
            .collect();
        builder.add_polygon(&points, true);
    }

    fn paint_strokes(frame: &EditorOverlayStrokesFrame, bounds: Bounds<Pixels>, window: &mut Window) {
        let size = Size { width: f32::from(bounds.size.width), height: f32::from(bounds.size.height) };
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let mut cache = frame.cache.borrow_mut();
        let key = Some((frame.generation, bounds));
        if cache.key != key {
            cache.layers =
                frame.strokes.iter().map(|stroke| Self::make_stroke_layer(stroke, size, bounds.origin, frame.display_zoom)).collect();
            cache.key = key;
        }
        let preview_key = Some((frame.preview_generation, bounds));
        if cache.preview_key != preview_key {
            cache.preview = frame
                .preview
                .as_ref()
                .map(|stroke| Self::make_stroke_layer(stroke, size, bounds.origin, frame.display_zoom))
                .unwrap_or_default();
            cache.preview_key = preview_key;
        }
        let cache = &*cache;

        let paint_at = |index: usize, tool: EditorDrawStrokeTool, paths: &EditorOverlayStrokePaths, window: &mut Window| {
            let blur_layer = (tool == EditorDrawStrokeTool::Blur)
                .then(|| frame.blur_layers.iter().find(|layer| layer.first_index <= index && index <= layer.last_index))
                .flatten();
            if let Some(layer) = blur_layer {
                if layer.first_index == index {
                    let _ = window.paint_image(bounds, bounds, Corners::default(), layer.image.clone(), 0, false);
                }
                return;
            }
            for (path, color) in paths {
                window.paint_path(path.clone(), *color);
            }
        };
        for (index, (stroke, paths)) in frame.strokes.iter().zip(&cache.layers).enumerate() {
            if !frame.hidden.contains(&index) {
                paint_at(index, stroke.tool, paths, window);
            }
        }
        if let Some(preview) = frame.preview.as_ref() {
            paint_at(frame.strokes.len(), preview.tool, &cache.preview, window);
        }
    }

    fn handle_draw_cancelled(&mut self, cx: &mut Context<Self>) {
        self.preview_layer = None;
        self.current_stroke_points.clear();
        self.redraw_strokes(cx);
    }

    fn handle_draw_changed(&mut self, location: Point<f32>, began: bool, cx: &mut Context<Self>) {
        if self.interaction_mode != EditorOverlayCanvasInteractionMode::Draw {
            return;
        }
        let Some(normalized) = self.normalize(location) else {
            return;
        };
        if began {
            self.current_stroke_points = vec![normalized];
        } else {
            self.current_stroke_points.push(normalized);
        }
        self.preview_stroke(cx);
    }

    fn prepare_brush_blur_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.blurred_image.is_some() || self.is_preparing_blur || self.is_hidden {
            return;
        }
        let Some(source) = self.blur_source_image.clone() else {
            return;
        };
        if self.brush_tool != EditorDrawStrokeTool::Blur
            && !self.state.strokes.iter().any(|stroke| stroke.tool == EditorDrawStrokeTool::Blur)
        {
            return;
        }
        self.is_preparing_blur = true;
        let generation = self.blur_generation;
        self.prepare_blur_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let blurred = cx
                .background_spawn(async move {
                    EditorImageBake::brush_blur_image(&source, EditorImageBake::BRUSH_BLUR_PREVIEW_MAX_PIXEL_SIDE)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.blur_generation != generation {
                    return;
                }
                this.is_preparing_blur = false;
                let is_ready = blurred.is_some();
                this.blurred_image = blurred;
                if is_ready && this.state.strokes.iter().any(|stroke| stroke.tool == EditorDrawStrokeTool::Blur) {
                    this.redraw_strokes(cx);
                } else if is_ready {
                    this.schedule_blur_compose(cx);
                }
            });
        }));
    }

    fn blur_runs(&self) -> Vec<EditorOverlayBlurRun> {
        let mut runs: Vec<EditorOverlayBlurRun> = Vec::new();
        let mut is_open = false;
        let preview = self.preview_layer.as_deref().map(|stroke| (self.painted_strokes.len(), stroke));
        let visible = self
            .painted_strokes
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.hidden_strokes.contains(index))
            .chain(preview);
        for (index, stroke) in visible {
            if stroke.tool != EditorDrawStrokeTool::Blur {
                is_open = false;
                continue;
            }
            match runs.last_mut().filter(|_| is_open) {
                Some(run) => {
                    run.last_index = index;
                    run.strokes.push(stroke.clone());
                }
                None => runs.push(EditorOverlayBlurRun {
                    first_index: index,
                    last_index: index,
                    strokes: vec![stroke.clone()],
                }),
            }
            is_open = true;
        }
        runs
    }

    /// one job at a time, pointer moves arriving meanwhile only ask for one more pass after
    fn schedule_blur_compose(&mut self, cx: &mut Context<Self>) {
        if self.blur_compose_task.is_some() {
            self.blur_compose_dirty = true;
            return;
        }
        let Some(blurred) = self.blurred_image.clone() else {
            return;
        };
        let size = self.bounds_size();
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let runs = self.blur_runs();
        let old_layers = self.blur_layers.clone();
        if runs.is_empty() {
            Self::drop_images(old_layers.iter().map(|layer| layer.image.clone()).collect(), cx);
            self.blur_layers = Rc::new(Vec::new());
            return;
        }
        let generation = self.blur_generation;
        let zoom = self.display_zoom;
        let reused: Vec<Option<Arc<RenderImage>>> = runs
            .iter()
            .map(|run| {
                old_layers.iter().find(|layer| layer.strokes == run.strokes).map(|layer| layer.image.clone())
            })
            .collect();
        self.blur_compose_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let layers = cx
                .background_spawn(async move {
                    runs.into_iter()
                        .zip(reused)
                        .filter_map(|(run, image)| {
                            let image = image.or_else(|| Self::compose_blur_layer(&blurred, size, zoom, &run.strokes))?;
                            Some(EditorOverlayBlurLayer {
                                first_index: run.first_index,
                                last_index: run.last_index,
                                strokes: run.strokes,
                                image,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.blur_compose_task = None;
                if this.blur_generation != generation {
                    return;
                }
                let released: Vec<Arc<RenderImage>> = this
                    .blur_layers
                    .iter()
                    .filter(|old| !layers.iter().any(|layer| Arc::ptr_eq(&layer.image, &old.image)))
                    .map(|old| old.image.clone())
                    .collect();
                Self::drop_images(released, cx);
                this.blur_layers = Rc::new(layers);
                cx.notify();
                if this.blur_compose_dirty {
                    this.blur_compose_dirty = false;
                    this.schedule_blur_compose(cx);
                }
            });
        }));
    }

    fn compose_blur_layer(
        blurred: &RenderImage,
        canvas: Size<f32>,
        zoom: f32,
        strokes: &[EditorDrawStroke],
    ) -> Option<Arc<RenderImage>> {
        let image_size = blurred.size(0);
        let width = u32::try_from(image_size.width.0).ok()?;
        let height = u32::try_from(image_size.height.0).ok()?;
        let mut bytes = blurred.as_bytes(0)?.to_vec();
        let pixels = Size { width: width as f32, height: height as f32 };
        let scale = pixels.width / canvas.width;
        let mut coverage = vec![0_u8; width as usize * height as usize];
        for stroke in strokes {
            let line_width = Self::line_width(stroke, canvas, zoom) * scale;
            for sub_path in EditorImageBake::stroke_path(stroke, pixels, line_width) {
                let mut points = sub_path.iter().copied();
                let Some(mut previous) = points.next() else {
                    continue;
                };
                for p in points {
                    Self::cover_segment(&mut coverage, width, height, previous, p, line_width / 2.0);
                    previous = p;
                }
            }
        }
        for (pixel, cover) in bytes.chunks_exact_mut(4).zip(&coverage) {
            pixel[3] = (u32::from(pixel[3]) * u32::from(*cover) / 255) as u8;
        }
        let bitmap = RgbaImage::from_raw(width, height, bytes)?;
        Some(Arc::new(RenderImage::new([Frame::new(bitmap)])))
    }

    fn cover_segment(coverage: &mut [u8], width: u32, height: u32, a: Point<f32>, b: Point<f32>, radius: f32) {
        let reach = radius + 1.0;
        let left = (a.x.min(b.x) - reach).floor().max(0.0) as u32;
        let top = (a.y.min(b.y) - reach).floor().max(0.0) as u32;
        let right = ((a.x.max(b.x) + reach).ceil().max(0.0) as u32).min(width);
        let bottom = ((a.y.max(b.y) + reach).ceil().max(0.0) as u32).min(height);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length_squared = dx * dx + dy * dy;
        for y in top..bottom {
            let row = y as usize * width as usize;
            for x in left..right {
                let (px_x, px_y) = (x as f32 + 0.5 - a.x, y as f32 + 0.5 - a.y);
                let along = if length_squared > 0.0 {
                    ((px_x * dx + px_y * dy) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance = (px_x - along * dx).hypot(px_y - along * dy);
                let cover = ((radius + 0.5 - distance).clamp(0.0, 1.0) * 255.0) as u8;
                let cell = &mut coverage[row + x as usize];
                *cell = (*cell).max(cover);
            }
        }
    }

    fn normalize(&self, location: Point<f32>) -> Option<Point<f32>> {
        let size = self.bounds_size();
        if size.width <= 0.0 || size.height <= 0.0 {
            return None;
        }
        Some(Point { x: location.x / size.width, y: location.y / size.height })
    }

    fn preview_stroke(&mut self, cx: &mut Context<Self>) {
        self.preview_layer = None;
        let Some(last_point) = self.current_stroke_points.last().copied() else {
            return;
        };
        if self.brush_tool == EditorDrawStrokeTool::Eraser {
            self.preview_erasure(&[last_point]);
            self.schedule_blur_compose(cx);
            cx.notify();
            return;
        }
        self.preview_layer = Some(Rc::new(EditorDrawStroke {
            tool: self.brush_tool,
            color: self.brush_color,
            width: self.brush_width_fraction,
            points: self.current_stroke_points.clone(),
        }));
        self.preview_generation = self.preview_generation.wrapping_add(1);
        if self.brush_tool == EditorDrawStrokeTool::Blur {
            self.schedule_blur_compose(cx);
        }
        cx.notify();
    }

    fn preview_erasure(&mut self, points: &[Point<f32>]) {
        if points.is_empty() {
            return;
        }
        let radius = self.brush_width_fraction.max(0.02);
        let hidden = Rc::make_mut(&mut self.hidden_strokes);
        for (index, stroke) in self.state.strokes.iter().enumerate() {
            if !hidden.contains(&index) && Self::stroke_intersects(&stroke.points, points, radius) {
                hidden.insert(index);
            }
        }
    }

    fn commit_stroke(&mut self, cx: &mut Context<Self>) {
        let mut points = std::mem::take(&mut self.current_stroke_points);
        self.preview_layer = None;
        if points.is_empty() {
            cx.notify();
            return;
        }
        if points.len() == 1 {
            let p = points[0];
            points = vec![p, Point { x: p.x + 0.0001, y: p.y + 0.0001 }];
        }

        if self.brush_tool == EditorDrawStrokeTool::Eraser {
            let eraser_radius = self.brush_width_fraction.max(0.02);
            self.state.strokes.retain(|stroke| !Self::stroke_intersects(&stroke.points, &points, eraser_radius));
            self.state.was_edited = true;
            self.redraw_strokes(cx);
            self.fire_state_changed(cx);
            return;
        }

        self.state.strokes.push(EditorDrawStroke {
            tool: self.brush_tool,
            color: self.brush_color,
            width: self.brush_width_fraction,
            points,
        });
        self.state.was_edited = true;
        self.redraw_strokes(cx);
        self.fire_state_changed(cx);
    }

    fn stroke_intersects(stroke: &[Point<f32>], eraser_path: &[Point<f32>], radius: f32) -> bool {
        let r2 = radius * radius;
        stroke.iter().any(|a| {
            eraser_path.iter().any(|b| {
                let dx = a.x - b.x;
                let dy = a.y - b.y;
                dx * dx + dy * dy < r2
            })
        })
    }

    fn draw_pan_is_enabled(&self) -> bool {
        self.interaction_mode == EditorOverlayCanvasInteractionMode::Draw && self.selected_entity_id.is_none()
    }

    fn pointer_pressed(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let location = self.local(event.position);
        if self.contains_entity(location, cx) {
            return;
        }
        if self.draw_pan_is_enabled() {
            self.press = Some(EditorOverlayPress::Draw);
            self.handle_draw_changed(location, true, cx);
        } else {
            self.press = Some(EditorOverlayPress::Tap(location));
        }
        cx.notify();
    }

    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.pointer_released(event.position, cx);
            return;
        }
        if let Some(EditorOverlayPress::Draw) = self.press {
            let location = self.local(event.position);
            self.handle_draw_changed(location, false, cx);
        }
    }

    fn pointer_released(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let location = self.local(position);
        match self.press.take() {
            Some(EditorOverlayPress::Draw) => {
                if self.interaction_mode == EditorOverlayCanvasInteractionMode::Draw {
                    self.commit_stroke(cx);
                }
            }
            Some(EditorOverlayPress::Tap(start)) => {
                if (location.x - start.x).hypot(location.y - start.y) <= TAP_SLOP {
                    self.handle_empty_tap(location, cx);
                }
            }
            None => {}
        }
    }

    fn handle_empty_tap(&mut self, location: Point<f32>, cx: &mut Context<Self>) {
        match self.interaction_mode {
            EditorOverlayCanvasInteractionMode::TextPlacement => {
                if let (Some(normalized), Some(on_request_create_text)) =
                    (self.normalize(location), self.on_request_create_text.clone())
                {
                    App::defer(cx, move |cx| on_request_create_text(normalized, cx));
                }
            }
            EditorOverlayCanvasInteractionMode::TagPlacement => {
                if let (Some(normalized), Some(on_request_create_tag)) =
                    (self.normalize(location), self.on_request_create_tag.clone())
                {
                    App::defer(cx, move |cx| on_request_create_tag(normalized, cx));
                }
            }
            EditorOverlayCanvasInteractionMode::Draw | EditorOverlayCanvasInteractionMode::EntityEdit => {
                self.select_entity(None, cx);
            }
        }
    }

    /// keeps the stage taps and pans off texts, plates and other entities
    /// and off canvas photos while arranging
    pub fn contains_entity(&self, location: Point<f32>, cx: &App) -> bool {
        if self.entity_frame_contains(location, cx) {
            return true;
        }
        if self.interaction_mode != EditorOverlayCanvasInteractionMode::EntityEdit {
            return false;
        }
        self.photo_views.iter().any(|view| view.read(cx).point(location))
    }

    fn entity_frame_contains(&self, location: Point<f32>, cx: &App) -> bool {
        self.entity_views.iter().filter(|view| !self.is_hidden_view(view, cx)).any(|view| match view {
            EditorOverlayEntityView::Text(view) => view.read(cx).contains_canvas_point(location),
            EditorOverlayEntityView::MusicPlate(view) => view.read(cx).contains_canvas_point(location),
            EditorOverlayEntityView::PersonTag(view) => view.read(cx).contains_canvas_point(location),
            EditorOverlayEntityView::Sticker(view) => view.read(cx).contains_canvas_point(location),
            EditorOverlayEntityView::Link(view) => view.read(cx).point(location),
        })
    }

    fn update_gesture_enable(&mut self, cx: &mut Context<Self>) {
        if matches!(self.press, Some(EditorOverlayPress::Draw)) && !self.draw_pan_is_enabled() {
            self.press = None;
            self.handle_draw_cancelled(cx);
        }
        if self.interaction_mode != EditorOverlayCanvasInteractionMode::EntityEdit {
            self.end_two_finger(cx);
        }
        self.apply_entity_interaction_states(cx);
    }

    fn apply_entity_interaction_states(&mut self, cx: &mut Context<Self>) {
        let focused = self.selected_entity_id.is_some();
        let selected_id = self.selected_entity_id;
        let mode = self.interaction_mode;
        let entity_state = |entity_id: Uuid| -> (bool, bool) {
            let selected = Some(entity_id) == selected_id;
            match mode {
                EditorOverlayCanvasInteractionMode::Draw => (selected && focused, selected && focused),
                EditorOverlayCanvasInteractionMode::EntityEdit => (selected, true),
                EditorOverlayCanvasInteractionMode::TextPlacement | EditorOverlayCanvasInteractionMode::TagPlacement => {
                    (false, false)
                }
            }
        };
        let photos_allow_body_drag = mode == EditorOverlayCanvasInteractionMode::EntityEdit;
        for view in self.photo_views.clone() {
            view.update(cx, |view, cx| view.set_interaction_state(photos_allow_body_drag, cx));
        }
        for view in self.entity_views.clone() {
            match view {
                EditorOverlayEntityView::Sticker(view) => {
                    view.update(cx, |view, cx| {
                        let (handles_visible, allows_body_drag) = entity_state(view.entity_id);
                        view.set_interaction_state(handles_visible, allows_body_drag, cx);
                    });
                }
                EditorOverlayEntityView::Link(view) => {
                    view.update(cx, |view, cx| {
                        let (handles_visible, allows_body_drag) = entity_state(view.entity_id);
                        view.set_interaction_state(handles_visible, allows_body_drag, cx);
                    });
                }
                EditorOverlayEntityView::PersonTag(view) => {
                    view.update(cx, |view, _| {
                        let allows_body_drag = match mode {
                            EditorOverlayCanvasInteractionMode::Draw => Some(view.entity_id) == selected_id && focused,
                            EditorOverlayCanvasInteractionMode::EntityEdit
                            | EditorOverlayCanvasInteractionMode::TagPlacement => true,
                            EditorOverlayCanvasInteractionMode::TextPlacement => false,
                        };
                        view.set_interaction_state(allows_body_drag);
                    });
                }
                EditorOverlayEntityView::Text(view) => {
                    view.update(cx, |view, cx| {
                        let (handles_visible, allows_body_drag) = entity_state(view.entity_id);
                        view.set_interaction_state(handles_visible, allows_body_drag, cx);
                    });
                }
                EditorOverlayEntityView::MusicPlate(view) => {
                    view.update(cx, |view, cx| {
                        let (handles_visible, allows_body_drag) = entity_state(view.entity_id);
                        view.set_interaction_state(handles_visible, allows_body_drag, cx);
                    });
                }
            }
        }
    }

    fn rebuild_entities(&mut self, cx: &mut Context<Self>) {
        self.entity_views.retain(|view| !matches!(view, EditorOverlayEntityView::Text(_)));
        for entity in self.state.texts.clone() {
            let view = self.make_entity_view(&entity, cx);
            self.entity_views.push(EditorOverlayEntityView::Text(view));
        }
        self.layout_entities(cx);
    }

    fn layout_entities(&mut self, cx: &mut Context<Self>) {
        let size = self.bounds_size();
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let selected_id = self.selected_entity_id;
        let tags = self.state.person_tags.clone().unwrap_or_default();
        for view in self.entity_views.clone() {
            match view {
                EditorOverlayEntityView::Text(view) => {
                    let id = view.read(cx).entity_id;
                    if let Some(entity) = self.state.texts.iter().find(|entity| entity.id == id) {
                        view.update(cx, |view, cx| view.apply(entity, size, Some(id) == selected_id, cx));
                    }
                }
                EditorOverlayEntityView::MusicPlate(view) => {
                    let id = view.read(cx).entity_id;
                    if let Some(plate) = self.state.music_plate.as_ref() {
                        view.update(cx, |view, cx| view.apply(plate, size, Some(id) == selected_id, cx));
                    }
                }
                EditorOverlayEntityView::PersonTag(view) => {
                    let id = view.read(cx).entity_id;
                    if let Some(entity) = tags.iter().find(|entity| entity.id == id) {
                        view.update(cx, |view, cx| view.apply(entity, size, Some(id) == selected_id, cx));
                    }
                }
                EditorOverlayEntityView::Sticker(view) => {
                    let id = view.read(cx).entity_id;
                    if let Some(entity) = self.state.stickers.iter().flatten().find(|entity| entity.id == id) {
                        view.update(cx, |view, cx| view.apply(entity, size, Some(id) == selected_id, cx));
                    }
                }
                EditorOverlayEntityView::Link(view) => {
                    let id = view.read(cx).entity_id;
                    if let Some(entity) = self.state.links.iter().flatten().find(|entity| entity.id == id) {
                        view.update(cx, |view, cx| view.apply(entity, size, Some(id) == selected_id, cx));
                    }
                }
            }
        }
        for view in self.photo_views.clone() {
            let id = view.read(cx).entity_id;
            let photos = self.state.story_canvas.as_ref().map(|canvas| canvas.photos.as_slice()).unwrap_or_default();
            if let Some(entity) = photos.iter().find(|entity| entity.id == id) {
                view.update(cx, |view, cx| view.apply(entity, size, Some(id) == selected_id, cx));
            }
        }
        cx.notify();
    }

    fn rebuild_stickers(&mut self, cx: &mut Context<Self>) {
        self.entity_views.retain(|view| !matches!(view, EditorOverlayEntityView::Sticker(_)));
        for entity in self.state.stickers.clone().unwrap_or_default() {
            let view = self.make_sticker_view(&entity, cx);
            self.entity_views.push(EditorOverlayEntityView::Sticker(view));
        }
        self.layout_entities(cx);
        self.apply_entity_interaction_states(cx);
    }

    fn make_sticker_view(&mut self, entity: &EditorStickerEntity, cx: &mut Context<Self>) -> Entity<EditorStickerEntityView> {
        let canvas = cx.weak_entity();
        let entity_id = entity.id;
        cx.new(|view_cx| {
            let mut view = EditorStickerEntityView::new(entity_id, view_cx);
            let this = canvas.clone();
            view.on_update = Some(Rc::new(move |updated: EditorStickerEntity, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    let Some(slot) = this.state.stickers.iter_mut().flatten().find(|sticker| sticker.id == updated.id) else {
                        return;
                    };
                    *slot = updated;
                    this.state.was_edited = true;
                    this.fire_state_changed(cx);
                });
            }));
            let this = canvas.clone();
            view.on_select = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.select_entity(Some(id), cx));
            }));
            let this = canvas.clone();
            view.on_request_delete = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.delete_sticker(id, cx));
            }));
            let this = canvas.clone();
            view.on_snap_guides_changed = Some(Rc::new(move |vertical: bool, horizontal: bool, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.set_snap_guides(vertical, horizontal, cx));
            }));
            view
        })
    }

    fn delete_sticker(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let remaining: Vec<EditorStickerEntity> =
            self.state.stickers.take().unwrap_or_default().into_iter().filter(|sticker| sticker.id != id).collect();
        self.state.stickers = if remaining.is_empty() { None } else { Some(remaining) };
        self.state.synchronize_edit_flag();
        self.fire_state_changed(cx);
        self.rebuild_stickers(cx);
        if self.selected_entity_id == Some(id) {
            self.select_entity(None, cx);
        }
    }

    fn rebuild_photos(&mut self, cx: &mut Context<Self>) {
        self.photo_views.clear();
        let photos = self.state.story_canvas.as_ref().map(|canvas| canvas.photos.clone()).unwrap_or_default();
        let allows_delete = photos.len() > 1;
        for entity in &photos {
            let view = self.make_photo_view(entity, allows_delete, cx);
            self.photo_views.push(view);
        }
        self.layout_entities(cx);
        self.apply_entity_interaction_states(cx);
    }

    fn make_photo_view(
        &mut self,
        entity: &EditorCanvasPhotoEntity,
        allows_delete: bool,
        cx: &mut Context<Self>,
    ) -> Entity<EditorCanvasPhotoEntityView> {
        let canvas = cx.weak_entity();
        let entity_id = entity.id;
        cx.new(|view_cx| {
            let mut view = EditorCanvasPhotoEntityView::new(entity_id, view_cx);
            view.allows_delete = allows_delete;
            let this = canvas.clone();
            view.on_update = Some(Rc::new(move |updated: EditorCanvasPhotoEntity, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    let Some(slot) = this
                        .state
                        .story_canvas
                        .as_mut()
                        .and_then(|canvas| canvas.photos.iter_mut().find(|photo| photo.id == updated.id))
                    else {
                        return;
                    };
                    *slot = updated;
                    this.state.was_edited = true;
                    this.fire_state_changed(cx);
                });
            }));
            let this = canvas.clone();
            view.on_select = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.select_entity(Some(id), cx));
            }));
            let this = canvas.clone();
            view.on_request_delete = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.delete_photo(id, cx));
            }));
            let this = canvas.clone();
            view.on_snap_guides_changed = Some(Rc::new(move |vertical: bool, horizontal: bool, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.set_snap_guides(vertical, horizontal, cx));
            }));
            view
        })
    }

    fn delete_photo(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(canvas) = self.state.story_canvas.as_mut() else {
            return;
        };
        canvas.photos.retain(|photo| photo.id != id);
        self.state.synchronize_edit_flag();
        self.fire_state_changed(cx);
        self.rebuild_photos(cx);
        if self.selected_entity_id == Some(id) {
            self.select_entity(None, cx);
        }
    }

    fn rebuild_links(&mut self, cx: &mut Context<Self>) {
        self.entity_views.retain(|view| !matches!(view, EditorOverlayEntityView::Link(_)));
        for entity in self.state.links.clone().unwrap_or_default() {
            let view = self.make_link_view(&entity, cx);
            self.entity_views.push(EditorOverlayEntityView::Link(view));
        }
        self.layout_entities(cx);
        self.apply_entity_interaction_states(cx);
    }

    fn make_link_view(&mut self, entity: &EditorLinkEntity, cx: &mut Context<Self>) -> Entity<EditorLinkEntityView> {
        let canvas = cx.weak_entity();
        let entity_id = entity.id;
        cx.new(|view_cx| {
            let mut view = EditorLinkEntityView::new(entity_id, view_cx);
            let this = canvas.clone();
            view.on_update = Some(Rc::new(move |updated: EditorLinkEntity, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    let Some(slot) = this.state.links.iter_mut().flatten().find(|link| link.id == updated.id) else {
                        return;
                    };
                    *slot = updated;
                    this.state.was_edited = true;
                    this.fire_state_changed(cx);
                });
            }));
            let this = canvas.clone();
            view.on_select = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.select_entity(Some(id), cx));
            }));
            view.on_request_customize = Some(Rc::new(|id: Uuid, _cx: &mut App| {
                RiseAppEvents::default_center().post_with(EDITOR_LINK_CUSTOMIZE, id);
            }));
            let this = canvas.clone();
            view.on_request_delete = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.delete_link(id, cx));
            }));
            let this = canvas.clone();
            view.on_snap_guides_changed = Some(Rc::new(move |vertical: bool, horizontal: bool, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.set_snap_guides(vertical, horizontal, cx));
            }));
            view
        })
    }

    fn delete_link(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let remaining: Vec<EditorLinkEntity> =
            self.state.links.take().unwrap_or_default().into_iter().filter(|link| link.id != id).collect();
        self.state.links = if remaining.is_empty() { None } else { Some(remaining) };
        self.state.synchronize_edit_flag();
        self.fire_state_changed(cx);
        self.rebuild_links(cx);
        if self.selected_entity_id == Some(id) {
            self.select_entity(None, cx);
        }
    }

    fn begin_two_finger(&mut self, target: EditorOverlayTwoFingerTarget, cx: &mut Context<Self>) {
        self.two_finger_target = Some(target.clone());
        self.two_finger_scale = 1.0;
        App::defer(cx, move |cx| target.begin(cx));
    }

    fn end_two_finger(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.two_finger_target.take() {
            self.two_finger_scale = 1.0;
            App::defer(cx, move |cx| target.end(cx));
        }
    }

    fn two_finger_target_at(&self, location: Point<f32>, cx: &App) -> Option<EditorOverlayTwoFingerTarget> {
        if self.interaction_mode != EditorOverlayCanvasInteractionMode::EntityEdit {
            return None;
        }
        if let Some(focused) = self.selected_entity_id.and_then(|id| self.transformable_view(id, false, cx)) {
            return Some(focused);
        }
        let entity = self.entity_views.iter().rev().filter(|view| !self.is_hidden_view(view, cx)).find_map(|view| {
            let hit = match view {
                EditorOverlayEntityView::Text(view) => view.read(cx).contains_canvas_point(location),
                EditorOverlayEntityView::Sticker(view) => view.read(cx).contains_canvas_point(location),
                EditorOverlayEntityView::Link(view) => view.read(cx).point(location),
                EditorOverlayEntityView::MusicPlate(_) | EditorOverlayEntityView::PersonTag(_) => false,
            };
            if hit { Self::two_finger_target_of(view) } else { None }
        });
        if entity.is_some() {
            return entity;
        }
        let photo = self
            .photo_views
            .iter()
            .rev()
            .find(|view| view.read(cx).point(location))
            .map(|view| EditorOverlayTwoFingerTarget::Photo(view.clone()));
        if photo.is_some() {
            return photo;
        }
        self.selected_entity_id.and_then(|id| self.transformable_view(id, true, cx))
    }

    fn rebuild_person_tags(&mut self, cx: &mut Context<Self>) {
        self.entity_views.retain(|view| !matches!(view, EditorOverlayEntityView::PersonTag(_)));
        for entity in self.state.person_tags.clone().unwrap_or_default() {
            let view = self.make_person_tag_view(&entity, cx);
            self.entity_views.push(EditorOverlayEntityView::PersonTag(view));
        }
        self.layout_entities(cx);
        self.apply_entity_interaction_states(cx);
    }

    fn make_person_tag_view(&mut self, entity: &EditorPersonTagEntity, cx: &mut Context<Self>) -> Entity<EditorPersonTagEntityView> {
        let canvas = cx.weak_entity();
        let entity_id = entity.id;
        let display_zoom = self.display_zoom;
        cx.new(|view_cx| {
            let mut view = EditorPersonTagEntityView::new(entity_id, view_cx);
            view.set_display_zoom(display_zoom, view_cx);
            let this = canvas.clone();
            view.on_update = Some(Rc::new(move |updated: EditorPersonTagEntity, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    let Some(tags) = this.state.person_tags.as_mut() else {
                        return;
                    };
                    let Some(slot) = tags.iter_mut().find(|tag| tag.id == updated.id) else {
                        return;
                    };
                    *slot = updated;
                    this.state.was_edited = true;
                    this.fire_state_changed(cx);
                });
            }));
            let this = canvas.clone();
            view.on_select = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.select_entity(Some(id), cx));
            }));
            let this = canvas.clone();
            view.on_request_delete = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.delete_person_tag(id, cx));
            }));
            let this = canvas.clone();
            view.on_snap_guides_changed = Some(Rc::new(move |vertical: bool, horizontal: bool, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.set_snap_guides(vertical, horizontal, cx));
            }));
            view
        })
    }

    fn delete_person_tag(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let remaining: Vec<EditorPersonTagEntity> =
            self.state.person_tags.take().unwrap_or_default().into_iter().filter(|tag| tag.id != id).collect();
        self.state.person_tags = Some(remaining);
        self.state.synchronize_edit_flag();
        self.fire_state_changed(cx);
        self.rebuild_person_tags(cx);
        if self.selected_entity_id == Some(id) {
            self.select_entity(None, cx);
        }
    }

    /// new card is selected right away so its outline and handles show without another tap
    /// plain rebuild leaves selection alone
    fn rebuild_plate(&mut self, select_on_creation: bool, cx: &mut Context<Self>) {
        self.entity_views.retain(|view| !matches!(view, EditorOverlayEntityView::MusicPlate(_)));
        self.plate_view = None;
        let Some(plate_id) = self.state.music_plate.as_ref().map(|plate| plate.id) else {
            return;
        };
        let canvas = cx.weak_entity();
        let display_zoom = self.display_zoom;
        let view = cx.new(|view_cx| {
            let mut view = EditorMusicPlateEntityView::new(plate_id, view_cx);
            view.set_display_zoom(display_zoom, view_cx);
            let this = canvas.clone();
            view.on_update = Some(Rc::new(move |updated: EditorMusicPlateEntity, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    if this.state.music_plate.as_ref().map(|plate| plate.id) != Some(updated.id) {
                        return;
                    }
                    this.state.music_plate = Some(updated);
                    this.state.was_edited = true;
                    this.fire_state_changed(cx);
                });
            }));
            let this = canvas.clone();
            view.on_select = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.select_entity(Some(id), cx));
            }));
            let this = canvas.clone();
            view.on_request_delete = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.delete_plate(id, cx));
            }));
            let this = canvas.clone();
            view.on_snap_guides_changed = Some(Rc::new(move |vertical: bool, horizontal: bool, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.set_snap_guides(vertical, horizontal, cx));
            }));
            view
        });
        self.entity_views.push(EditorOverlayEntityView::MusicPlate(view.clone()));
        self.plate_view = Some(view);
        self.layout_entities(cx);
        self.apply_entity_interaction_states(cx);
        if select_on_creation {
            self.select_entity(Some(plate_id), cx);
        }
    }

    fn delete_plate(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.state.music_plate.as_ref().map(|plate| plate.id) != Some(id) {
            return;
        }
        self.state.music_plate = None;
        self.state.synchronize_edit_flag();
        self.fire_state_changed(cx);
        self.rebuild_plate(false, cx);
        if self.selected_entity_id == Some(id) {
            self.select_entity(None, cx);
        }
    }

    fn make_entity_view(&mut self, entity: &EditorTextEntity, cx: &mut Context<Self>) -> Entity<EditorTextEntityView> {
        let canvas = cx.weak_entity();
        let entity_id = entity.id;
        let display_zoom = self.display_zoom;
        cx.new(|view_cx| {
            let mut view = EditorTextEntityView::new(entity_id, view_cx);
            view.set_display_zoom(display_zoom, view_cx);
            let this = canvas.clone();
            view.on_update = Some(Rc::new(move |updated: EditorTextEntity, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    if let Some(slot) = this.state.texts.iter_mut().find(|text| text.id == updated.id) {
                        *slot = updated;
                        this.state.was_edited = true;
                        this.fire_state_changed(cx);
                    }
                });
            }));
            let this = canvas.clone();
            view.on_select = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.select_entity(Some(id), cx));
            }));
            let this = canvas.clone();
            view.on_request_edit_menu = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let on_request_edit_text = this.read_with(cx, |this, _| this.on_request_edit_text.clone()).ok().flatten();
                if let Some(on_request_edit_text) = on_request_edit_text {
                    on_request_edit_text(id, cx);
                }
            }));
            let this = canvas.clone();
            view.on_request_delete = Some(Rc::new(move |id: Uuid, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.delete_text(id, cx));
            }));
            let this = canvas.clone();
            view.on_snap_guides_changed = Some(Rc::new(move |vertical: bool, horizontal: bool, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.set_snap_guides(vertical, horizontal, cx));
            }));
            view
        })
    }

    fn delete_text(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.state.texts.retain(|text| text.id != id);
        self.state.was_edited = true;
        self.fire_state_changed(cx);
        self.rebuild_entities(cx);
    }

    pub fn select_entity(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        if let (Some(id), Some(target)) = (id, self.two_finger_target.as_ref())
            && target.entity_id(cx) != id
        {
            return;
        }
        self.selected_entity_id = id;
        if let Some(id) = id {
            self.focus_on_render = true;
            self.raise(id, cx);
        }
        self.wheel_rotation = None;
        self.update_gesture_enable(cx);
        self.layout_entities(cx);
        if let Some(on_text_focus_changed) = self.on_text_focus_changed.clone() {
            App::defer(cx, move |cx| on_text_focus_changed(id, cx));
        }
    }

    pub fn add(&mut self, entity: EditorTextEntity, cx: &mut Context<Self>) {
        let id = entity.id;
        self.state.texts.push(entity);
        self.state.was_edited = true;
        self.fire_state_changed(cx);
        self.rebuild_entities(cx);
        self.select_entity(Some(id), cx);
    }

    pub fn entity(&self, id: Uuid) -> Option<&EditorTextEntity> {
        self.state.texts.iter().find(|text| text.id == id)
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(id) = self.selected_entity_id else {
            return;
        };
        let step = if event.keystroke.modifiers.shift { SHIFT_NUDGE } else { NUDGE };
        match event.keystroke.key.as_str() {
            "delete" | "backspace" => self.delete_selected(id, cx),
            "left" => self.nudge_selected(id, -step, 0.0, cx),
            "right" => self.nudge_selected(id, step, 0.0, cx),
            "up" => self.nudge_selected(id, 0.0, -step, cx),
            "down" => self.nudge_selected(id, 0.0, step, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn delete_selected(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.state.texts.iter().any(|text| text.id == id) {
            self.delete_text(id, cx);
        } else if self.state.music_plate.as_ref().is_some_and(|plate| plate.id == id) {
            self.delete_plate(id, cx);
        } else if self.state.stickers.iter().flatten().any(|sticker| sticker.id == id) {
            self.delete_sticker(id, cx);
        } else if self.state.links.iter().flatten().any(|link| link.id == id) {
            self.delete_link(id, cx);
        } else if let Some(photos) = self.state.story_canvas.as_ref().map(|canvas| canvas.photos.as_slice())
            && photos.iter().any(|photo| photo.id == id)
        {
            if photos.len() > 1 {
                self.delete_photo(id, cx);
            }
        } else {
            self.delete_person_tag(id, cx);
        }
    }

    fn nudge_selected(&mut self, id: Uuid, dx: f32, dy: f32, cx: &mut Context<Self>) {
        let Some(normalized) = self.normalize(Point { x: dx, y: dy }) else {
            return;
        };
        let nudged = |center: Point<f32>| Point {
            x: (center.x + normalized.x).clamp(0.0, 1.0),
            y: (center.y + normalized.y).clamp(0.0, 1.0),
        };
        if let Some(text) = self.state.texts.iter_mut().find(|text| text.id == id) {
            text.center = nudged(text.center);
        } else if self.state.music_plate.as_ref().is_some_and(|plate| plate.id == id) {
            self.update_plate(|plate| plate.center = nudged(plate.center), cx);
        } else if let Some(tag) = self.state.person_tags.as_mut().and_then(|tags| tags.iter_mut().find(|tag| tag.id == id)) {
            tag.center = nudged(tag.center);
        } else if let Some(sticker) = self.state.stickers.iter_mut().flatten().find(|sticker| sticker.id == id) {
            sticker.center = nudged(sticker.center);
        } else if let Some(link) = self.state.links.iter_mut().flatten().find(|link| link.id == id) {
            link.center = nudged(link.center);
        } else if let Some(photo) =
            self.state.story_canvas.as_mut().and_then(|canvas| canvas.photos.iter_mut().find(|photo| photo.id == id))
        {
            photo.center = nudged(photo.center);
        } else {
            return;
        }
        self.state.was_edited = true;
        self.fire_state_changed(cx);
        self.layout_entities(cx);
    }

    /// changes the plate and keeps its rotated scaled box inside the canvas like its own drag does
    fn update_plate(&mut self, change: impl FnOnce(&mut EditorMusicPlateEntity), cx: &mut Context<Self>) {
        let size = self.bounds_size();
        let Some(mut plate) = self.state.music_plate.clone() else {
            return;
        };
        change(&mut plate);
        if let Some(view) = self.plate_view.as_ref().filter(|_| size.width > 0.0 && size.height > 0.0) {
            let wanted = Point { x: plate.center.x * size.width, y: plate.center.y * size.height };
            let kept = view.read(cx).kept_inside_canvas(wanted, &plate);
            plate.center = Point { x: kept.x / size.width, y: kept.y / size.height };
        }
        self.state.music_plate = Some(plate);
    }

    fn next_wheel_rotation(&mut self, id: Uuid, current: f32, delta: f32) -> f32 {
        let start = match self.wheel_rotation {
            Some((wheel_id, raw)) if wheel_id == id && EditorEntitySnap::snap_rotation(raw).0 == current => raw,
            _ => {
                self.snap_feedback.begin();
                current
            }
        };
        let raw = start + delta;
        let (angle, snapped) = EditorEntitySnap::snap_rotation(raw);
        self.snap_feedback.update_rotation(snapped);
        self.wheel_rotation = Some((id, raw));
        angle
    }

    /// scales and turns the selected text or plate, person tag has neither
    /// returns true if something took the input
    fn transform_selected(&mut self, scale_factor: f32, rotation_delta: f32, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected_entity_id else {
            return false;
        };
        let scaled = |scale: f32| (scale * scale_factor).max(EditorEntitySideHandle::SCALE_MIN).min(EditorEntitySideHandle::SCALE_MAX);
        if let Some(index) = self.state.texts.iter().position(|text| text.id == id) {
            let current = self.state.texts[index].rotation;
            let rotation = if rotation_delta == 0.0 { current } else { self.next_wheel_rotation(id, current, rotation_delta) };
            let text = &mut self.state.texts[index];
            text.scale = scaled(text.scale);
            text.rotation = rotation;
        } else if let Some(current) = self.state.music_plate.as_ref().filter(|plate| plate.id == id).map(|plate| plate.rotation) {
            let rotation = if rotation_delta == 0.0 { current } else { self.next_wheel_rotation(id, current, rotation_delta) };
            self.update_plate(
                |plate| {
                    plate.scale = scaled(plate.scale);
                    plate.rotation = rotation;
                },
                cx,
            );
        } else if let Some(target) = self.transformable_view(id, false, cx) {
            App::defer(cx, move |cx| {
                target.begin(cx);
                target.apply(scale_factor, rotation_delta, cx);
                target.end(cx);
            });
            return true;
        } else {
            return false;
        }
        self.state.was_edited = true;
        self.fire_state_changed(cx);
        self.layout_entities(cx);
        true
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(WHEEL_LINE_HEIGHT));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let handled = if event.modifiers.secondary() {
            self.transform_selected((1.0 + dy * WHEEL_SCALE_PER_PIXEL).max(0.01), 0.0, cx)
        } else if event.modifiers.shift {
            self.transform_selected(1.0, (dx + dy) * WHEEL_DEGREES_PER_PIXEL * PI / 180.0, cx)
        } else {
            false
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn handle_pinch(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            if self.two_finger_target.is_some() {
                self.end_two_finger(cx);
                cx.stop_propagation();
            }
            return;
        }
        if self.two_finger_target.is_none() {
            let location = self.local(event.position);
            if let Some(target) = self.two_finger_target_at(location, cx) {
                self.begin_two_finger(target, cx);
            }
        }
        if let Some(target) = self.two_finger_target.clone() {
            self.two_finger_scale = (self.two_finger_scale * (1.0 + event.delta)).max(0.01);
            let scale = self.two_finger_scale;
            App::defer(cx, move |cx| target.apply(scale, 0.0, cx));
            cx.stop_propagation();
            return;
        }
        if self.transform_selected((1.0 + event.delta).max(0.01), 0.0, cx) {
            cx.stop_propagation();
        }
    }

    fn entity_element(view: &EditorOverlayEntityView) -> AnyElement {
        match view {
            EditorOverlayEntityView::Text(view) => view.clone().into_any_element(),
            EditorOverlayEntityView::MusicPlate(view) => view.clone().into_any_element(),
            EditorOverlayEntityView::PersonTag(view) => view.clone().into_any_element(),
            EditorOverlayEntityView::Sticker(view) => view.clone().into_any_element(),
            EditorOverlayEntityView::Link(view) => view.clone().into_any_element(),
        }
    }
}

impl Render for EditorOverlayCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_hidden {
            return div().into_any_element();
        }
        if std::mem::take(&mut self.focus_on_render)
            && !self.focus_handle.is_focused(window)
            && (window.focused(cx).is_none() || self.focus_handle.within_focused(window, cx))
        {
            self.focus_handle.focus(window, cx);
        }
        let frame = EditorOverlayStrokesFrame {
            generation: self.strokes_generation,
            display_zoom: self.display_zoom,
            strokes: self.painted_strokes.clone(),
            preview_generation: self.preview_generation,
            preview: self.preview_layer.clone(),
            hidden: self.hidden_strokes.clone(),
            blur_layers: self.blur_layers.clone(),
            cache: self.stroke_cache.clone(),
        };
        let view = cx.weak_entity();
        let measure_view = view.clone();
        let surface = canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                Self::paint_strokes(&frame, bounds, window);

                let move_view = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    if move_view.read_with(cx, |view, _| view.press.is_some()).unwrap_or(false) {
                        let _ = move_view.update(cx, |view, cx| view.pointer_moved(event, cx));
                    }
                });
                let release_view = view.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    if release_view.read_with(cx, |view, _| view.press.is_some()).unwrap_or(false) {
                        let _ = release_view.update(cx, |view, cx| view.pointer_released(event.position, cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0();

        let size = self.bounds_size();
        let guide_color: Hsla = white().opacity(0.85);
        let vertical_guide = self.snap_guide_vertical.then(|| {
            div().absolute().left(px(size.width / 2.0 - 0.5)).top_0().w(px(1.0)).h(px(size.height)).bg(guide_color)
        });
        let horizontal_guide = self.snap_guide_horizontal.then(|| {
            div().absolute().left_0().top(px(size.height / 2.0 - 0.5)).w(px(size.width)).h(px(1.0)).bg(guide_color)
        });

        div()
            .id("editor-overlay-canvas")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .child(measure("editor-overlay-canvas-size", move |bounds, _, cx| {
                let _ = measure_view.update(cx, |view, cx| view.layout_subviews(bounds, cx));
            }))
            .child(EditorPassThroughView::new().children(self.photo_views.iter().cloned()))
            .child(surface)
            .child(
                EditorPassThroughView::new().children(
                    self.entity_views
                        .iter()
                        .filter(|view| !self.is_hidden_view(view, cx))
                        .map(Self::entity_element),
                ),
            )
            .children(vertical_guide)
            .children(horizontal_guide)
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_handle.focus(window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.pointer_pressed(event, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.handle_key_down(event, cx)))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.handle_scroll_wheel(event, cx)))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| this.handle_pinch(event, cx)))
            .into_any_element()
    }
}

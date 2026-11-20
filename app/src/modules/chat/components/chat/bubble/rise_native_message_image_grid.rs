use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{
    App, AppContext, AsyncApp, Bounds, ContentMask, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, ObjectFit, ParentElement, Pixels, Render, RenderImage, RenderOnce, SharedString, Size, Styled,
    StyledImage, Task, TextRun, Window, black, canvas, div, img, point, px, rgb, size, white,
};
use image::RgbaImage;
use image::codecs::jpeg::JpegEncoder;
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication};
use rise_engine::rise_upload::rise_upload_progress::{RiseUploadKey, RiseUploadProgress};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::animated_media_playback_scope::{
    ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE, ANIMATED_MEDIA_VIEWPORT_SCOPE_DID_CHANGE,
    AnimatedMediaViewportScopeProviding, nearest_animated_media_viewport_scope_view,
};
use rise_ui::loader_ui::ring_spin_degrees;
use rise_ui::media_spoiler_overlay_view::{MediaSpoilerOverlayView, MediaSpoilerReveals};
use rise_ui::message_bubble_geometry::{MessageBubbleRadii, MessageBubbleTailSide};
use rise_ui::message_media_tail_presentation::{
    MessageMediaTailBitmap, MessageMediaTailColorSampler, MessageMediaTailEdgeColors, MessageMediaTailGeometry,
    MessageMediaTailPresentation,
};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec};
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_foreground_gate::RISE_MEDIA_FOREGROUND_DID_CHANGE;
use crate::core::media::rise_media_transfer_hub::{
    RiseMediaCacheIdentity, RiseMediaTransferKey, RiseMediaTransferRequest, RiseMediaTransferRequestArgs,
};
use crate::core::media::rise_secure_media_location::RiseSecureMediaLocation;
use crate::core::media_viewer::media_viewer_countdown_badge::MediaViewerCountdownRing;
use crate::core::media_viewer::media_viewer_presentation_state::chat_media_presentation;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::media_viewer::media_viewer_types::MediaViewerFrameKeys;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RISE_ENGINE_CONNECTION_READY;
use crate::core::rise_engine_bridge::rise_engine_upload_phase::RiseEngineUploadPhase;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageDiskCache, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::rise_media_transfer_chip::{RiseMediaTransferChipView, RiseMediaTransferChipViewMetrics};
use crate::modules::chat::components::chat::bubble::bubble_tap_router::{BubbleTapAction, BubbleTapRouterModifiers};
use crate::modules::chat::components::chat::bubble::message_image_grid_layout::{
    MessageGridCell, MessageGridDownloadSeed, MessageGridLayout, MessageGridUploadSeed,
};
use crate::modules::chat::components::chat::bubble::rise_exact_upload_progress_ring::RiseUploadRingPresentation;
use crate::modules::chat::components::chat::bubble::rise_native_message_image_grid_hooks::RiseNativeMessageImageGridHooks;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageMediaItem, MessageMediaKind, MessageMediaViewPhase, RemoteMediaURL,
};

/// tap callback of a tile
/// gets tile index and frame in window coords, the tile and whether the grid is single
pub type RiseNativeMessageImageGridTap = Rc<dyn Fn(usize, Bounds<Pixels>, MessageGridCell, bool, &mut Window, &mut App)>;

/// cancel callback, gets the upload slot index of the tapped tile
pub type RiseNativeMessageImageGridCancelUpload = Rc<dyn Fn(i64, &mut Window, &mut App)>;

type RiseNativeMediaTailColorsHandler = Rc<dyn Fn(Option<MessageMediaTailEdgeColors>, &mut App)>;

pub const RISE_NATIVE_MEDIA_BACKDROP_APPLIED: RiseNotificationName =
    RiseNotificationName::new("riseNativeMediaBackdropApplied");

#[derive(IntoElement)]
pub struct RiseNativeMessageImageGrid {
    id: ElementId,
    pub owner_local_id: String,
    pub layout: MessageGridLayout,
    pub media_is_presented: bool,
    pub hero_lift_id: Option<String>,
    pub bubble_cell_hidden: bool,
    pub is_layout_measurement: bool,
    pub corners: MessageBubbleRadii,
    pub media_tail_side: MessageBubbleTailSide,
    pub separator_color: Hsla,
    pub placeholder_color: Hsla,
    pub has_sender_header: bool,
    pub on_tap: Option<RiseNativeMessageImageGridTap>,
    pub on_cancel_upload_slot: Option<RiseNativeMessageImageGridCancelUpload>,
}

impl RiseNativeMessageImageGrid {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        owner_local_id: String,
        layout: MessageGridLayout,
        media_is_presented: bool,
        hero_lift_id: Option<String>,
        bubble_cell_hidden: bool,
        is_layout_measurement: bool,
        corners: MessageBubbleRadii,
        media_tail_side: MessageBubbleTailSide,
        separator_color: Hsla,
        placeholder_color: Hsla,
        has_sender_header: bool,
        on_tap: Option<RiseNativeMessageImageGridTap>,
        on_cancel_upload_slot: Option<RiseNativeMessageImageGridCancelUpload>,
    ) -> Self {
        Self {
            id: id.into(),
            owner_local_id,
            layout,
            media_is_presented,
            hero_lift_id,
            bubble_cell_hidden,
            is_layout_measurement,
            corners,
            media_tail_side,
            separator_color,
            placeholder_color,
            has_sender_header,
            on_tap,
            on_cancel_upload_slot,
        }
    }
}

impl RenderOnce for RiseNativeMessageImageGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.use_keyed_state(self.id.clone(), cx, |_, cx| RiseNativeMessageImageGridView::new(cx));
        let container_size = self.layout.container_size;
        view.update(cx, |view, cx| {
            view.configure(
                self.owner_local_id,
                self.layout,
                self.media_is_presented,
                self.hero_lift_id,
                self.bubble_cell_hidden,
                self.is_layout_measurement,
                self.corners,
                self.media_tail_side,
                self.separator_color,
                self.placeholder_color,
                self.has_sender_header,
                SaiAccountCacheScope::active_user_id(),
                SaiAccountCacheScope::active_epoch(),
                self.on_tap,
                self.on_cancel_upload_slot,
                cx,
            );
        });
        div().w(container_size.width).h(container_size.height).child(view)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RiseNativeMediaTailConfiguration {
    registry_id: String,
    source: String,
    fallback_video_source: String,
    target_size: Size<Pixels>,
    uses_fit_foreground: bool,
    grid_radii: MessageBubbleRadii,
    source_radii: MessageBubbleRadii,
    side: MessageBubbleTailSide,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

pub struct RiseNativeMessageImageGridView {
    owner: Rc<dyn Any>,
    cells_by_id: HashMap<String, Entity<RiseNativeMessageImageCellView>>,
    recycled_cells: Vec<Entity<RiseNativeMessageImageCellView>>,
    cell_frames: Vec<(String, Bounds<Pixels>)>,
    container_size: Size<Pixels>,
    content_background_color: Option<Hsla>,
    corners: MessageBubbleRadii,
    media_tail_side: MessageBubbleTailSide,
    media_tail_registry_id: Option<String>,
    media_tail_configuration: Option<RiseNativeMediaTailConfiguration>,
    media_tail_presentation: Option<MessageMediaTailPresentation>,
    media_tail_is_lifted: bool,
    source_radii_registry_ids: HashSet<String>,
}

impl RiseNativeMessageImageGridView {
    const RECYCLED_CELL_LIMIT: usize = 12;

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            owner: Rc::new(()),
            cells_by_id: HashMap::new(),
            recycled_cells: Vec::new(),
            cell_frames: Vec::new(),
            container_size: Size::default(),
            content_background_color: Some(black()),
            corners: MessageBubbleRadii::uniform(px(0.0)),
            media_tail_side: MessageBubbleTailSide::None,
            media_tail_registry_id: None,
            media_tail_configuration: None,
            media_tail_presentation: None,
            media_tail_is_lifted: false,
            source_radii_registry_ids: HashSet::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn configure(
        &mut self,
        owner_local_id: String,
        layout: MessageGridLayout,
        media_is_presented: bool,
        hero_lift_id: Option<String>,
        bubble_cell_hidden: bool,
        is_layout_measurement: bool,
        corners: MessageBubbleRadii,
        media_tail_side: MessageBubbleTailSide,
        separator_color: Hsla,
        placeholder_color: Hsla,
        has_sender_header: bool,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        on_tap: Option<RiseNativeMessageImageGridTap>,
        on_cancel_upload_slot: Option<RiseNativeMessageImageGridCancelUpload>,
        cx: &mut Context<Self>,
    ) {
        self.corners = corners;
        self.media_tail_side = media_tail_side;
        self.container_size = layout.container_size;
        if is_layout_measurement {
            self.cell_frames.clear();
            return;
        }
        let has_lifted_tile = media_is_presented
            && bubble_cell_hidden
            && layout
                .cells
                .iter()
                .any(|cell| hero_lift_id.as_deref() == Some(MediaViewerFrameKeys::chat_bubble(&cell.render_id).as_str()));
        self.content_background_color = if has_lifted_tile { None } else { Some(separator_color) };
        let tail_model = Self::terminal_tail_model(&layout, media_tail_side).cloned();
        let next_tail_configuration = tail_model.as_ref().map(|model| RiseNativeMediaTailConfiguration {
            registry_id: MediaViewerFrameKeys::chat_bubble(&model.render_id),
            source: trimming_whitespaces_and_newlines(&model.display_source).to_owned(),
            fallback_video_source: if model.kind == MessageMediaKind::Video {
                trimming_whitespaces_and_newlines(&model.url).to_owned()
            } else {
                String::new()
            },
            target_size: model.cell_frame.size,
            uses_fit_foreground: layout.is_single,
            grid_radii: corners,
            source_radii: Self::source_radii(model, &layout, corners),
            side: media_tail_side,
            account_user_id: account_user_id.clone(),
            account_epoch: account_epoch.clone(),
        });
        if self.media_tail_configuration != next_tail_configuration {
            self.clear_media_tail_presentation();
            self.media_tail_configuration = next_tail_configuration.clone();
        }
        self.media_tail_registry_id = next_tail_configuration.as_ref().map(|configuration| configuration.registry_id.clone());
        self.media_tail_is_lifted = next_tail_configuration.as_ref().is_some_and(|configuration| {
            hero_lift_id.as_deref() == Some(configuration.registry_id.as_str()) && bubble_cell_hidden
        });

        let next_source_radii_registry_ids: HashSet<String> =
            layout.cells.iter().map(|cell| MediaViewerFrameKeys::chat_bubble(&cell.render_id)).collect();
        for id in self.source_radii_registry_ids.difference(&next_source_radii_registry_ids) {
            MediaCellScreenFrameRegistry::set_source_media_radii(None, id, &self.owner);
        }
        self.source_radii_registry_ids = next_source_radii_registry_ids;

        let retained_ids: HashSet<&str> = layout.cells.iter().map(|cell| cell.render_id.as_str()).collect();
        let removed_ids: Vec<String> =
            self.cells_by_id.keys().filter(|id| !retained_ids.contains(id.as_str())).cloned().collect();
        for id in removed_ids {
            let Some(cell) = self.cells_by_id.remove(&id) else { continue };
            cell.update(cx, |cell, cx| cell.prepare_for_reuse(cx));
            if self.recycled_cells.len() < Self::RECYCLED_CELL_LIMIT {
                self.recycled_cells.push(cell);
            }
        }

        for model in &layout.cells {
            let cell = match self.cells_by_id.get(&model.render_id) {
                Some(existing) => existing.clone(),
                None => {
                    let cell = self
                        .recycled_cells
                        .pop()
                        .unwrap_or_else(|| cx.new(RiseNativeMessageImageCellView::new));
                    self.cells_by_id.insert(model.render_id.clone(), cell.clone());
                    cell
                }
            };

            let registry_id = MediaViewerFrameKeys::chat_bubble(&model.render_id);
            let model_source_radii = Self::source_radii(model, &layout, corners);
            MediaCellScreenFrameRegistry::set_source_media_radii(Some(model_source_radii), &registry_id, &self.owner);
            let is_lifted =
                media_is_presented && hero_lift_id.as_deref() == Some(registry_id.as_str()) && bubble_cell_hidden;
            let tail_configuration = if Some(&model.render_id) == tail_model.as_ref().map(|tail| &tail.render_id) {
                next_tail_configuration.clone()
            } else {
                None
            };
            let tail_colors_handler: Option<RiseNativeMediaTailColorsHandler> = tail_configuration.clone().map(|configuration| {
                let grid = cx.weak_entity();
                let handler: RiseNativeMediaTailColorsHandler =
                    Rc::new(move |colors: Option<MessageMediaTailEdgeColors>, cx: &mut App| {
                        let grid = grid.clone();
                        let configuration = configuration.clone();
                        cx.defer(move |cx| {
                            let _ = grid.update(cx, |grid, cx| grid.apply_media_tail_colors(colors, &configuration, cx));
                        });
                    });
                handler
            });
            let slot_index = model.upload_slot_index;
            let on_cancel_upload_slot = on_cancel_upload_slot.clone();
            let on_cancel_upload: BubbleTapAction = Rc::new(move |window: &mut Window, cx: &mut App| {
                if let Some(on_cancel_upload_slot) = &on_cancel_upload_slot {
                    on_cancel_upload_slot(slot_index, window, cx);
                }
            });
            let tap_model = model.clone();
            let tap_registry_id = registry_id.clone();
            let is_single = layout.is_single;
            let on_tap = on_tap.clone();
            let on_tile_tap: BubbleTapAction = Rc::new(move |window: &mut Window, cx: &mut App| {
                let frame = MediaCellScreenFrameRegistry::frame(&tap_registry_id).unwrap_or_default();
                if let Some(on_tap) = &on_tap {
                    on_tap(tap_model.index, frame, tap_model.clone(), is_single, window, cx);
                }
            });
            let transfer_chip_top_inset = Self::transfer_chip_top_inset(model, has_sender_header);
            let duration_top_inset =
                if has_sender_header && model.cell_frame.top() <= px(1.0) { 33.0 } else { 6.0 };
            cell.update(cx, |cell, cx| {
                cell.configure(
                    model,
                    model_source_radii,
                    layout.is_single,
                    is_lifted,
                    is_layout_measurement,
                    placeholder_color,
                    transfer_chip_top_inset,
                    account_user_id.clone(),
                    account_epoch.clone(),
                    tail_configuration.is_some(),
                    tail_colors_handler,
                    &owner_local_id,
                    on_cancel_upload,
                    on_tile_tap,
                    cx,
                );
                cell.set_duration_top_inset(duration_top_inset, cx);
            });
        }
        self.cell_frames = layout.cells.iter().map(|cell| (cell.render_id.clone(), cell.cell_frame)).collect();
    }

    fn prepare_for_reuse(&mut self) {
        self.clear_media_tail_presentation();
        self.media_tail_registry_id = None;
        self.media_tail_configuration = None;
        self.media_tail_side = MessageBubbleTailSide::None;
        self.media_tail_is_lifted = false;
        for id in &self.source_radii_registry_ids {
            MediaCellScreenFrameRegistry::set_source_media_radii(None, id, &self.owner);
        }
        self.source_radii_registry_ids.clear();
        self.cells_by_id.clear();
        self.cell_frames.clear();
        self.recycled_cells.clear();
    }

    /// sender capsule is drawn over the container top left corner
    /// so only the tile in that corner steps below it
    fn transfer_chip_top_inset(model: &MessageGridCell, has_sender_header: bool) -> f32 {
        if !has_sender_header || model.cell_frame.left() > px(1.0) || model.cell_frame.top() > px(1.0) {
            return 8.0;
        }
        33.0
    }

    fn terminal_tail_model(layout: &MessageGridLayout, side: MessageBubbleTailSide) -> Option<&MessageGridCell> {
        if side == MessageBubbleTailSide::None {
            return None;
        }
        let bottom = layout.cells.iter().map(|cell| f32::from(cell.cell_frame.bottom())).max_by(f32::total_cmp)?;
        let bottom_cells =
            layout.cells.iter().filter(|cell| (f32::from(cell.cell_frame.bottom()) - bottom).abs() <= 1.0);
        match side {
            MessageBubbleTailSide::Leading => {
                bottom_cells.min_by(|a, b| f32::from(a.cell_frame.left()).total_cmp(&f32::from(b.cell_frame.left())))
            }
            MessageBubbleTailSide::Trailing => {
                bottom_cells.max_by(|a, b| f32::from(a.cell_frame.right()).total_cmp(&f32::from(b.cell_frame.right())))
            }
            MessageBubbleTailSide::None => None,
        }
    }

    fn source_radii(model: &MessageGridCell, layout: &MessageGridLayout, grid_radii: MessageBubbleRadii) -> MessageBubbleRadii {
        let tolerance = px(1.0);
        let is_leading = model.cell_frame.left() <= tolerance;
        let is_trailing = model.cell_frame.right() >= layout.container_size.width - tolerance;
        let is_top = model.cell_frame.top() <= tolerance;
        let is_bottom = model.cell_frame.bottom() >= layout.container_size.height - tolerance;
        MessageBubbleRadii {
            top_leading: if is_leading && is_top { grid_radii.top_leading } else { px(0.0) },
            top_trailing: if is_trailing && is_top { grid_radii.top_trailing } else { px(0.0) },
            bottom_leading: if is_leading && is_bottom { grid_radii.bottom_leading } else { px(0.0) },
            bottom_trailing: if is_trailing && is_bottom { grid_radii.bottom_trailing } else { px(0.0) },
        }
    }

    fn apply_media_tail_colors(
        &mut self,
        colors: Option<MessageMediaTailEdgeColors>,
        configuration: &RiseNativeMediaTailConfiguration,
        cx: &mut Context<Self>,
    ) {
        let is_current = self.media_tail_configuration.as_ref() == Some(configuration);
        let color = colors.and_then(|colors| colors.color(self.media_tail_side));
        let (Some(registry_id), Some(color), true) = (self.media_tail_registry_id.clone(), color, is_current) else {
            if is_current && self.media_tail_presentation.is_some() {
                self.clear_media_tail_presentation();
                cx.notify();
            }
            return;
        };
        let visible_color = color.darkened(MessageMediaTailGeometry::TERMINAL_OVERLAY_BLACK_OPACITY);
        let presentation = MessageMediaTailPresentation {
            side: self.media_tail_side,
            color: visible_color,
            radii: configuration.source_radii,
        };
        if self.media_tail_presentation == Some(presentation) {
            return;
        }
        self.media_tail_presentation = Some(presentation);
        MediaCellScreenFrameRegistry::set_media_tail_presentation(Some(presentation), &registry_id, &self.owner);
        cx.notify();
    }

    fn clear_media_tail_presentation(&mut self) {
        if let Some(registry_id) = &self.media_tail_registry_id {
            MediaCellScreenFrameRegistry::set_media_tail_presentation(None, registry_id, &self.owner);
        }
        self.media_tail_presentation = None;
    }

    fn media_tail_is_hidden(&self) -> bool {
        self.media_tail_side == MessageBubbleTailSide::None || self.media_tail_presentation.is_none() || self.media_tail_is_lifted
    }
}

impl Drop for RiseNativeMessageImageGridView {
    fn drop(&mut self) {
        self.prepare_for_reuse();
    }
}

impl Render for RiseNativeMessageImageGridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let corners = self.corners;
        let side = self.media_tail_side;
        let background = self.content_background_color;
        let tail_fill = if self.media_tail_is_hidden() {
            None
        } else {
            self.media_tail_presentation.map(|presentation| presentation.color.color())
        };
        let mut content = div().relative().w(self.container_size.width).h(self.container_size.height).child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let Some(background) = background else { return };
                    if let Some(path) = MessageMediaTailGeometry::content_mask_path(bounds, corners, side).fill_path() {
                        window.paint_path(path, background);
                    }
                },
            )
            .absolute()
            .inset_0(),
        );
        for (id, frame) in &self.cell_frames {
            let Some(cell) = self.cells_by_id.get(id) else { continue };
            let tile: AnyElement = if cell.read(cx).upload_overlay_is_visible {
                cell.clone().into_any_element()
            } else {
                cell.clone().cached(gpui::StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
            };
            content = content.child(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(tile),
            );
        }
        if let Some(fill) = tail_fill {
            content = content.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let cap_width = MessageMediaTailGeometry::extension_width(side);
                        let cap_height = MessageMediaTailGeometry::CAP_HEIGHT;
                        let cap_x =
                            if side == MessageBubbleTailSide::Leading { bounds.left() - cap_width } else { bounds.right() };
                        let cap_frame = Bounds::new(point(cap_x, bounds.bottom() - cap_height), size(cap_width, cap_height));
                        let path = MessageMediaTailGeometry::cap_path_for_media(bounds, corners, side).scaled_and_translated(
                            1.0,
                            1.0,
                            cap_frame.left(),
                            cap_frame.top(),
                        );
                        window.with_content_mask(Some(ContentMask { bounds: cap_frame }), |window| {
                            if let Some(path) = path.fill_path() {
                                window.paint_path(path, fill);
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        content
    }
}

struct RiseNativeMessageImageCellView {
    transfer_chip: Entity<RiseMediaTransferChipView>,
    registry_id: Option<String>,
    on_tap: Option<BubbleTapAction>,
    on_cancel_upload: Option<BubbleTapAction>,
    on_tail_colors: Option<RiseNativeMediaTailColorsHandler>,
    needs_tail_colors: bool,
    media_tail_colors: Option<MessageMediaTailEdgeColors>,
    media_tail_colors_resolved: bool,
    load_task: Option<Task<()>>,
    load_generation: u64,
    did_fail_load: bool,
    failed_load_attempts: usize,
    failed_load_retry_task: Option<Task<()>>,
    source: String,
    displayed_source: String,
    fallback_video_source: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    is_single: bool,
    is_lifted: bool,
    is_layout_measurement: bool,
    source_aspect: f32,
    decode_target_size: Size<Pixels>,
    is_visible: bool,
    last_reported_visibility: Option<bool>,
    visibility_recheck_scheduled: bool,
    visibility_observers: Vec<RiseSubscription>,
    visibility_events_task: Option<Task<()>>,
    viewport_scope_view: Option<Rc<dyn AnimatedMediaViewportScopeProviding>>,
    upload_key: Option<RiseUploadKey>,
    upload_seed: Option<MessageGridUploadSeed>,
    latest_upload_progress: Option<RiseUploadProgress>,
    upload_progress_task: Option<Task<()>>,
    upload_progress_subscription_generation: u64,
    has_received_upload_progress: bool,
    upload_overlay_is_visible: bool,
    model_is_video: bool,
    model_video_duration: Option<f64>,
    last_rendered_upload_start: f64,
    last_rendered_upload_fraction: f64,
    last_upload_label_update_time: Option<Duration>,
    last_upload_label_phase: Option<RiseEngineUploadPhase>,
    last_upload_was_awaiting_server: bool,
    download_seed: Option<MessageGridDownloadSeed>,
    transfer_chip_top_inset: f32,
    duration_top_inset: f32,
    spoiler_view: Entity<MediaSpoilerOverlayView>,
    spoiler_id: Option<String>,
    view_phase: Option<MessageMediaViewPhase>,
    is_in_window: bool,
    bounds_size: Size<Pixels>,
    screen_scale: f32,
    radii: MessageBubbleRadii,
    background_color: Hsla,
    image: Option<CleverImage>,
    backdrop_image: Option<CleverImage>,
    shows_backdrop: bool,
    duration_text: Option<String>,
    play_is_hidden: bool,
    duration_is_hidden: bool,
    upload_status_text: Option<String>,
    upload_stroke_color: Hsla,
    upload_stroke_end: Animated<f32>,
    upload_rotation_started_at: Option<Duration>,
    upload_ring_swaps_running: bool,
    upload_ring_painted: Rc<std::cell::Cell<bool>>,
    spoiler_is_hidden: bool,
    view_badge_is_hidden: bool,
    view_countdown: Option<(i64, i64)>,
    view_badge_symbol: &'static str,
    view_badge_text: String,
    keeps_replaced_crop: bool,
    stale_decode: bool,
    transfer_chip_was_drawn: bool,
    _transfer_chip_observer: gpui::Subscription,
}

/// what a tile draws, compared around configure so only a visible change renders it
#[derive(Clone, PartialEq)]
struct RiseNativeMediaCellDrawnState {
    registry_id: Option<String>,
    bounds_size: Size<Pixels>,
    radii: MessageBubbleRadii,
    background_color: Hsla,
    image: Option<usize>,
    backdrop_image: Option<usize>,
    shows_backdrop: bool,
    keeps_replaced_crop: bool,
    is_single: bool,
    is_lifted: bool,
    is_layout_measurement: bool,
    is_in_window: bool,
    spoiler_is_hidden: bool,
    play_is_hidden: bool,
    duration_is_hidden: bool,
    duration_text: Option<String>,
    view_badge_is_hidden: bool,
    view_countdown: Option<(i64, i64)>,
    view_badge_symbol: &'static str,
    view_badge_text: String,
    upload_overlay: (bool, Option<String>, Hsla, bool, bool, bool),
    upload_stroke_end: u32,
    transfer_chip_top_inset: u32,
}

impl RiseNativeMessageImageCellView {
    const FAILED_LOAD_RETRY_DELAYS: [u64; 5] = [1, 3, 8, 15, 30];
    const VISIBILITY_RECHECK_FRAMES: usize = 30;
    const VISIBILITY_RECHECK_INTERVAL: Duration = Duration::from_millis(16);
    const UPLOAD_ROTATION_PERIOD: Duration = Duration::from_millis(820);
    const UPLOAD_LABEL_THROTTLE: Duration = Duration::from_millis(80);
    const UPLOAD_STROKE_INITIAL_END: f32 = 0.08;

    fn new(cx: &mut Context<Self>) -> Self {
        let transfer_chip = cx.new(RiseMediaTransferChipView::new);
        let transfer_chip_observer = cx.observe(&transfer_chip, |cell, chip, cx| {
            if chip.read(cx).is_drawn() != cell.transfer_chip_was_drawn {
                cx.notify();
            }
        });
        Self {
            transfer_chip,
            registry_id: None,
            on_tap: None,
            on_cancel_upload: None,
            on_tail_colors: None,
            needs_tail_colors: false,
            media_tail_colors: None,
            media_tail_colors_resolved: false,
            load_task: None,
            load_generation: 0,
            did_fail_load: false,
            failed_load_attempts: 0,
            failed_load_retry_task: None,
            source: String::new(),
            displayed_source: String::new(),
            fallback_video_source: String::new(),
            account_user_id: None,
            account_epoch: None,
            is_single: false,
            is_lifted: false,
            is_layout_measurement: false,
            source_aspect: 1.0,
            decode_target_size: Size::default(),
            is_visible: false,
            last_reported_visibility: None,
            visibility_recheck_scheduled: false,
            visibility_observers: Vec::new(),
            visibility_events_task: None,
            viewport_scope_view: None,
            upload_key: None,
            upload_seed: None,
            latest_upload_progress: None,
            upload_progress_task: None,
            upload_progress_subscription_generation: 0,
            has_received_upload_progress: false,
            upload_overlay_is_visible: false,
            model_is_video: false,
            model_video_duration: None,
            last_rendered_upload_start: -1.0,
            last_rendered_upload_fraction: -1.0,
            last_upload_label_update_time: None,
            last_upload_label_phase: None,
            last_upload_was_awaiting_server: false,
            download_seed: None,
            transfer_chip_top_inset: 8.0,
            duration_top_inset: 6.0,
            is_in_window: false,
            bounds_size: Size::default(),
            screen_scale: 1.0,
            radii: MessageBubbleRadii::default(),
            background_color: black(),
            image: None,
            backdrop_image: None,
            shows_backdrop: false,
            duration_text: None,
            play_is_hidden: true,
            duration_is_hidden: true,
            upload_status_text: None,
            upload_stroke_color: white(),
            upload_stroke_end: Animated::new(Self::UPLOAD_STROKE_INITIAL_END),
            upload_rotation_started_at: None,
            upload_ring_swaps_running: false,
            upload_ring_painted: Rc::new(std::cell::Cell::new(false)),
            spoiler_view: cx.new(|_| MediaSpoilerOverlayView::new()),
            spoiler_id: None,
            view_phase: None,
            spoiler_is_hidden: true,
            view_badge_is_hidden: true,
            view_countdown: None,
            view_badge_symbol: "1.circle",
            view_badge_text: String::new(),
            keeps_replaced_crop: false,
            stale_decode: false,
            transfer_chip_was_drawn: false,
            _transfer_chip_observer: transfer_chip_observer,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn configure(
        &mut self,
        model: &MessageGridCell,
        radii: MessageBubbleRadii,
        is_single: bool,
        is_lifted: bool,
        is_layout_measurement: bool,
        placeholder_color: Hsla,
        transfer_chip_top_inset: f32,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        needs_tail_colors: bool,
        on_tail_colors: Option<RiseNativeMediaTailColorsHandler>,
        upload_owner_id: &str,
        on_cancel_upload: BubbleTapAction,
        on_tap: BubbleTapAction,
        cx: &mut Context<Self>,
    ) {
        let drawn = self.drawn_state();
        self.configure_model(
            model,
            radii,
            is_single,
            is_lifted,
            is_layout_measurement,
            placeholder_color,
            transfer_chip_top_inset,
            account_user_id,
            account_epoch,
            needs_tail_colors,
            on_tail_colors,
            upload_owner_id,
            on_cancel_upload,
            on_tap,
            cx,
        );
        if self.drawn_state() != drawn {
            cx.notify();
        }
    }

    fn set_duration_top_inset(&mut self, inset: f32, cx: &mut Context<Self>) {
        if self.duration_top_inset == inset {
            return;
        }
        self.duration_top_inset = inset;
        cx.notify();
    }

    /// everything the tile render reads from its own state, equal values draw the same tile
    fn drawn_state(&self) -> RiseNativeMediaCellDrawnState {
        RiseNativeMediaCellDrawnState {
            registry_id: self.registry_id.clone(),
            bounds_size: self.bounds_size,
            radii: self.radii,
            background_color: self.background_color,
            image: self.image.as_ref().map(|image| Arc::as_ptr(image) as usize),
            backdrop_image: self.backdrop_image.as_ref().map(|image| Arc::as_ptr(image) as usize),
            shows_backdrop: self.shows_backdrop,
            keeps_replaced_crop: self.keeps_replaced_crop,
            is_single: self.is_single,
            is_lifted: self.is_lifted,
            is_layout_measurement: self.is_layout_measurement,
            is_in_window: self.is_in_window,
            spoiler_is_hidden: self.spoiler_is_hidden,
            play_is_hidden: self.play_is_hidden,
            duration_is_hidden: self.duration_is_hidden,
            duration_text: self.duration_text.clone(),
            view_badge_is_hidden: self.view_badge_is_hidden,
            view_countdown: self.view_countdown,
            view_badge_symbol: self.view_badge_symbol,
            view_badge_text: self.view_badge_text.clone(),
            upload_overlay: self.upload_overlay_signature(),
            upload_stroke_end: self.upload_stroke_end.target().to_bits(),
            transfer_chip_top_inset: self.transfer_chip_top_inset.to_bits(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn configure_model(
        &mut self,
        model: &MessageGridCell,
        radii: MessageBubbleRadii,
        is_single: bool,
        is_lifted: bool,
        is_layout_measurement: bool,
        placeholder_color: Hsla,
        transfer_chip_top_inset: f32,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        needs_tail_colors: bool,
        on_tail_colors: Option<RiseNativeMediaTailColorsHandler>,
        upload_owner_id: &str,
        on_cancel_upload: BubbleTapAction,
        on_tap: BubbleTapAction,
        cx: &mut Context<Self>,
    ) {
        self.bounds_size = model.cell_frame.size;
        self.radii = radii;
        let normalized_source = trimming_whitespaces_and_newlines(&model.display_source).to_owned();
        let normalized_video_source = if model.kind == MessageMediaKind::Video {
            trimming_whitespaces_and_newlines(&model.url).to_owned()
        } else {
            String::new()
        };
        let registry_id = MediaViewerFrameKeys::chat_bubble(&model.render_id);
        let primary_source_changed = self.source != normalized_source;
        let fallback_video_source_changed = self.fallback_video_source != normalized_video_source;
        let account_scope_changed = self.account_user_id != account_user_id || self.account_epoch != account_epoch;
        let preserves_preview = self.registry_id.as_deref() == Some(registry_id.as_str())
            && !account_scope_changed
            && MessageMediaItem::is_local_preview_url(&self.source)
            && normalized_source.to_lowercase().starts_with("http");
        let keeps_replaced_image = self.registry_id.as_deref() == Some(registry_id.as_str())
            && !account_scope_changed
            && primary_source_changed
            && self.image.is_some();
        let presentation_changed = self.is_single != is_single;
        let tail_requirement_changed = self.needs_tail_colors != needs_tail_colors;
        let image_height = f32::from(model.image_frame.size.height);
        let next_source_aspect =
            if image_height > 0.0 { (f32::from(model.image_frame.size.width) / image_height).max(0.01) } else { 1.0 };
        let source_aspect_changed = (self.source_aspect - next_source_aspect).abs() > 0.01;
        let decode_geometry_changed = source_aspect_changed
            || (f32::from(self.decode_target_size.width) - f32::from(self.bounds_size.width)).abs() > 1.0
            || (f32::from(self.decode_target_size.height) - f32::from(self.bounds_size.height)).abs() > 1.0;

        if self.registry_id.as_deref() != Some(registry_id.as_str()) {
            self.registry_id = Some(registry_id);
        }
        self.on_tap = Some(on_tap);
        self.on_cancel_upload = Some(on_cancel_upload);
        self.on_tail_colors = on_tail_colors;
        self.needs_tail_colors = needs_tail_colors;
        self.is_single = is_single;
        self.is_lifted = is_lifted;
        self.is_layout_measurement = is_layout_measurement;
        self.background_color = placeholder_color;
        self.source_aspect = next_source_aspect;
        self.decode_target_size = self.bounds_size;
        self.account_user_id = account_user_id;
        self.account_epoch = account_epoch;
        self.model_is_video = model.kind == MessageMediaKind::Video;
        self.model_video_duration = model.duration;
        self.download_seed = model.download_seed.clone();
        self.transfer_chip_top_inset = transfer_chip_top_inset.max(0.0);

        let normalized_upload_owner_id = trimming_whitespaces_and_newlines(upload_owner_id);
        let next_upload_key = model.upload_seed.and_then(|_| {
            (!normalized_upload_owner_id.is_empty() && model.upload_slot_index >= 0)
                .then(|| RiseUploadKey::new(normalized_upload_owner_id, model.upload_slot_index))
        });
        if self.upload_key != next_upload_key {
            self.cancel_upload_progress_subscription();
            self.upload_key = next_upload_key;
            self.latest_upload_progress = None;
            self.has_received_upload_progress = false;
            self.last_rendered_upload_start = -1.0;
            self.last_rendered_upload_fraction = -1.0;
            self.last_upload_label_phase = None;
        }
        self.upload_seed = model.upload_seed;

        if primary_source_changed || fallback_video_source_changed {
            tracing::warn!(
                target: "riseonly::X625",
                "tile source id={:?} source={} fallback={}",
                self.registry_id,
                normalized_source,
                normalized_video_source
            );
        }
        self.source = normalized_source;
        self.fallback_video_source = normalized_video_source;
        if primary_source_changed || account_scope_changed {
            self.did_fail_load = false;
            self.failed_load_attempts = 0;
            self.cancel_load(!preserves_preview && !keeps_replaced_image, cx);
        } else if fallback_video_source_changed && self.image.is_none() {
            self.did_fail_load = false;
            self.failed_load_attempts = 0;
            self.cancel_load(false, cx);
        }
        if (presentation_changed || decode_geometry_changed) && (self.image.is_some() || self.load_task.is_some()) {
            self.did_fail_load = false;
            let keeps_drawn = self.image.is_some() && !primary_source_changed && !account_scope_changed;
            self.cancel_load(!keeps_replaced_image && !keeps_drawn, cx);
            self.stale_decode = keeps_drawn;
        }
        if keeps_replaced_image && (source_aspect_changed || presentation_changed) {
            self.keeps_replaced_crop = true;
        }
        if tail_requirement_changed && needs_tail_colors {
            self.media_tail_colors = None;
            self.media_tail_colors_resolved = false;
        }

        self.configure_video_overlay(model);
        self.configure_spoiler(model, cx);
        self.configure_upload_overlay_from_current_state(cx);
        self.configure_transfer_chip(cx);
        if self.image.is_none() || self.stale_decode || self.displayed_source != self.source {
            if let Some(cached) = RiseNativeMediaThumbnailPipeline::cached_thumbnail(
                &self.source,
                &self.fallback_video_source,
                self.bounds_size,
                self.screen_scale,
                self.source_aspect,
                self.is_single,
                self.needs_tail_colors,
                self.account_user_id.as_deref(),
                self.account_epoch.as_deref(),
            ) {
                self.apply(Some(cached.image), cached.backdrop, cached.tail_colors, cached.tail_colors_resolved, cx);
            } else if self.needs_tail_colors && self.media_tail_colors_resolved {
                self.report_tail_colors(cx);
            }
        } else if self.needs_tail_colors && self.media_tail_colors_resolved {
            self.report_tail_colors(cx);
        }

        if self.is_layout_measurement {
            self.stop_visibility_tracking();
            self.is_visible = false;
            self.last_reported_visibility = None;
            self.cancel_load(false, cx);
        } else if self.is_in_window {
            self.refresh_visibility_subscription(cx);
            self.report_visibility_if_changed(cx);
            if !self.is_visible {
                self.schedule_visibility_recheck(cx);
            }
            self.begin_load_if_needed(cx);
            self.begin_upload_progress_subscription_if_needed(cx);
            self.ensure_upload_progress_rotation(cx);
        }
    }

    fn report_tail_colors(&self, cx: &mut Context<Self>) {
        if let Some(on_tail_colors) = self.on_tail_colors.clone() {
            on_tail_colors(self.media_tail_colors, cx);
        }
    }

    fn revalidate_visibility(&mut self, cx: &mut Context<Self>) {
        if !self.is_in_window || self.is_layout_measurement {
            return;
        }
        self.report_visibility_if_changed(cx);
    }

    fn schedule_visibility_recheck(&mut self, cx: &mut Context<Self>) {
        if self.visibility_recheck_scheduled {
            return;
        }
        self.visibility_recheck_scheduled = true;
        cx.spawn(async move |this, cx| {
            for _ in 0..Self::VISIBILITY_RECHECK_FRAMES {
                cx.background_executor().timer(Self::VISIBILITY_RECHECK_INTERVAL).await;
                let settled = this.update(cx, |cell, cx| {
                    cell.revalidate_visibility(cx);
                    cell.is_visible || !cell.is_in_window || cell.is_layout_measurement
                });
                match settled {
                    Ok(false) => {}
                    Ok(true) | Err(_) => break,
                }
            }
            let _ = this.update(cx, |cell, cx| {
                cell.visibility_recheck_scheduled = false;
                if !cell.is_visible && cell.is_in_window && !cell.is_layout_measurement {
                    cell.trace_hidden_after_recheck(cx);
                }
            });
        })
        .detach();
    }

    fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.spoiler_id = None;
        self.view_phase = None;
        self.view_badge_is_hidden = true;
        self.view_countdown = None;
        self.spoiler_view.update(cx, |spoiler, cx| spoiler.reset(cx));
        self.spoiler_is_hidden = true;
        self.stop_visibility_tracking();
        self.cancel_load(true, cx);
        self.cancel_upload_progress_subscription();
        self.hide_upload_overlay(cx);
        self.transfer_chip.update(cx, |chip, cx| chip.prepare_for_reuse(cx));
        self.download_seed = None;
        self.transfer_chip_top_inset = 8.0;
        self.duration_top_inset = 6.0;
        self.registry_id = None;
        self.on_tap = None;
        self.on_cancel_upload = None;
        self.on_tail_colors = None;
        self.needs_tail_colors = false;
        self.source.clear();
        self.fallback_video_source.clear();
        self.account_user_id = None;
        self.account_epoch = None;
        self.is_single = false;
        self.is_lifted = false;
        self.is_layout_measurement = false;
        self.source_aspect = 1.0;
        self.decode_target_size = Size::default();
        self.is_visible = false;
        self.last_reported_visibility = None;
        self.upload_key = None;
        self.upload_seed = None;
        self.latest_upload_progress = None;
        self.has_received_upload_progress = false;
        self.model_is_video = false;
        self.model_video_duration = None;
        self.last_rendered_upload_start = -1.0;
        self.last_rendered_upload_fraction = -1.0;
        self.last_upload_label_phase = None;
        self.last_upload_was_awaiting_server = false;
        self.duration_text = None;
        self.is_in_window = false;
        cx.notify();
    }

    fn did_move_to_window(&mut self, cx: &mut Context<Self>) {
        if self.is_layout_measurement {
            return;
        }
        tracing::warn!(target: "riseonly::X590", "tile did_move_to_window id={}", self.registry_id.as_deref().unwrap_or_default());
        self.did_fail_load = false;
        self.cancel_failed_load_retry(true);
        self.refresh_visibility_subscription(cx);
        self.report_visibility_if_changed(cx);
        if !self.is_visible {
            self.schedule_visibility_recheck(cx);
        }
        self.begin_load_if_needed(cx);
        self.begin_upload_progress_subscription_if_needed(cx);
        self.ensure_upload_progress_rotation(cx);
        self.configure_transfer_chip(cx);
        self.transfer_chip.update(cx, |chip, cx| chip.did_move_to_window(true, cx));
    }

    fn configure_video_overlay(&mut self, model: &MessageGridCell) {
        let is_video = model.kind == MessageMediaKind::Video;
        self.play_is_hidden = !is_video || self.upload_overlay_is_visible;
        match model.duration.filter(|_| is_video) {
            Some(duration) => {
                self.duration_text = Some(RiseFormat::duration_time_interval(duration));
                self.duration_is_hidden = self.upload_overlay_is_visible;
            }
            None => {
                self.duration_text = None;
                self.duration_is_hidden = true;
            }
        }
    }

    fn configure_spoiler(&mut self, model: &MessageGridCell, cx: &mut Context<Self>) {
        self.view_phase = model.view_phase;
        let locked = matches!(model.view_phase, Some(MessageMediaViewPhase::Sealed | MessageMediaViewPhase::Consumed));
        let closed = locked || (model.has_spoiler && !MediaSpoilerReveals::is_opened(&model.render_id, cx));
        self.configure_view_badge(model, locked);
        self.spoiler_id = closed.then(|| model.render_id.clone());
        let (image, key) = (self.image.clone(), self.displayed_source.clone());
        let cell_hidden = self.spoiler_is_hidden;
        let next_hidden = self.spoiler_view.update(cx, |spoiler, cx| {
            if closed {
                if cell_hidden || spoiler.is_hidden() || spoiler.is_revealing() {
                    spoiler.reset(cx);
                }
                spoiler.set_source(image, &key, cx);
                false
            } else {
                cell_hidden || !spoiler.is_revealing()
            }
        });
        self.spoiler_is_hidden = next_hidden;
    }

    fn configure_view_badge(&mut self, model: &MessageGridCell, locked: bool) {
        if let Some(MessageMediaViewPhase::Countdown { deadline_ms, seconds }) = model.view_phase {
            self.view_badge_is_hidden = true;
            self.view_countdown = Some((deadline_ms, seconds));
            return;
        }
        self.view_countdown = None;
        let Some(phase) = model.view_phase else {
            self.view_badge_is_hidden = true;
            return;
        };
        if locked {
            self.play_is_hidden = true;
            self.duration_is_hidden = true;
        }
        let once_or_timed = if model.view_is_timed { "timer" } else { "1.circle" };
        let (symbol, text) = match phase {
            MessageMediaViewPhase::Sealed => (once_or_timed, tr("media_view_tap_to_open")),
            MessageMediaViewPhase::Consumed => (once_or_timed, String::new()),
            MessageMediaViewPhase::Countdown { .. } => return,
            MessageMediaViewPhase::Sent { viewed } => {
                let symbol = if viewed { "checkmark.circle" } else { once_or_timed };
                let text = if viewed {
                    tr("media_view_viewed")
                } else if model.view_is_timed && model.kind != MessageMediaKind::Video {
                    tr("media_view_destructed_photo_timed").replace("{count}", &model.view_seconds.unwrap_or(0).to_string())
                } else if model.kind == MessageMediaKind::Video {
                    tr("media_view_once_video")
                } else {
                    tr("media_view_once_photo")
                };
                (symbol, text)
            }
        };
        self.view_badge_symbol = symbol;
        self.view_badge_text = text;
        self.view_badge_is_hidden = false;
    }

    /// the centred view plate, or just the glyph in a round plate on a narrow tile
    fn view_badge(&mut self, window: &mut Window) -> Option<AnyElement> {
        if let Some((deadline_ms, seconds)) = self.view_countdown {
            let side = MediaViewerCountdownRing::SIDE;
            let opacity = if self.upload_overlay_is_visible { 0.0 } else { 1.0 };
            return Some(
                div()
                    .absolute()
                    .left(px((f32::from(self.bounds_size.width) - side) / 2.0))
                    .top(px((f32::from(self.bounds_size.height) - side) / 2.0))
                    .opacity(opacity)
                    .child(MediaViewerCountdownRing::new(
                        Date::from_time_interval_since_1970(deadline_ms as f64 / 1_000.0),
                        seconds as f64,
                    ))
                    .into_any_element(),
            );
        }
        if self.view_badge_is_hidden {
            return None;
        }
        const ICON: f32 = 16.0;
        const GAP: f32 = 5.0;
        const HORIZONTAL: f32 = 9.0;
        const HEIGHT: f32 = 26.0;
        let width = f32::from(self.bounds_size.width);
        let height = f32::from(self.bounds_size.height);
        let max_text = (width - 16.0 - HORIZONTAL * 2.0 - ICON - GAP).max(0.0);
        let font = Font::system_weight(12.0, FontWeight::SEMIBOLD);
        let full_text = if self.view_badge_text.is_empty() {
            0.0
        } else {
            let run = TextRun {
                len: self.view_badge_text.len(),
                font: gpui::Font {
                    family: font.family(),
                    features: font.features(),
                    fallbacks: None,
                    weight: font.weight,
                    style: font.style(),
                },
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(
                window
                    .text_system()
                    .shape_line(SharedString::from(self.view_badge_text.clone()), px(font.size), &[run], None)
                    .width(),
            )
            .ceil()
        };
        let icon_only = full_text > max_text || self.view_badge_text.is_empty();
        let text_width = if icon_only { 0.0 } else { full_text };
        let plate_width = if icon_only { HEIGHT } else { HORIZONTAL * 2.0 + ICON + GAP + text_width };
        self.play_is_hidden = true;
        let opacity = if self.upload_overlay_is_visible { 0.0 } else { 1.0 };
        Some(
            div()
                .absolute()
                .left(px((width - plate_width) / 2.0))
                .top(px((height - HEIGHT) / 2.0))
                .w(px(plate_width))
                .h(px(HEIGHT))
                .rounded(px(HEIGHT / 2.0))
                .overflow_hidden()
                .bg(black().opacity(0.45))
                .opacity(opacity)
                .child(
                    div()
                        .absolute()
                        .left(px(if icon_only { (HEIGHT - ICON) / 2.0 } else { HORIZONTAL }))
                        .top(px((HEIGHT - ICON) / 2.0))
                        .size(px(ICON))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized(self.view_badge_symbol, px(13.0), white(), false)),
                )
                .when(!icon_only, |this| {
                    this.child(
                        div()
                            .absolute()
                            .left(px(HORIZONTAL + ICON + GAP))
                            .top_0()
                            .w(px(text_width))
                            .h(px(HEIGHT))
                            .flex()
                            .items_center()
                            .swiftui_font(font)
                            .text_color(white())
                            .whitespace_nowrap()
                            .child(SharedString::from(self.view_badge_text.clone())),
                    )
                })
                .into_any_element(),
        )
    }

    fn open_spoiler(&mut self, cx: &mut Context<Self>) {
        let Some(spoiler_id) = self.spoiler_id.take() else {
            return;
        };
        if matches!(self.view_phase, Some(MessageMediaViewPhase::Sealed | MessageMediaViewPhase::Consumed)) {
            self.spoiler_id = Some(spoiler_id);
            return;
        }
        MediaSpoilerReveals::open(&spoiler_id, cx);
        self.spoiler_view.update(cx, |spoiler, cx| spoiler.reveal(None, true, None, cx));
    }

    fn configure_upload_overlay_from_current_state(&mut self, cx: &mut Context<Self>) {
        let (Some(upload_key), Some(upload_seed)) = (self.upload_key.clone(), self.upload_seed) else {
            self.hide_upload_overlay(cx);
            return;
        };
        if let Some(latest_upload_progress) = self.latest_upload_progress.clone() {
            self.apply_upload_progress(&latest_upload_progress, true, cx);
            return;
        }
        let phase = if upload_seed.is_compressing || (upload_seed.fraction <= 0.0 && upload_seed.completed_bytes <= 0) {
            RiseEngineUploadPhase::Preparing
        } else {
            RiseEngineUploadPhase::Uploading
        };
        let progress = RiseUploadProgress::new(
            upload_key,
            upload_seed.fraction,
            upload_seed.completed_bytes,
            upload_seed.total_bytes,
            phase,
        );
        self.apply_upload_progress(&progress, true, cx);
    }

    fn begin_upload_progress_subscription_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.is_in_window
            || !self.is_visible
            || self.is_layout_measurement
            || self.upload_seed.is_none()
            || self.upload_progress_task.is_some()
        {
            return;
        }
        let Some(key) = self.upload_key.clone() else { return };
        let Some(updates) = RiseNativeMessageImageGridHooks::upload_progress_updates_or_default(key.clone(), cx) else {
            return;
        };
        self.upload_progress_subscription_generation = self.upload_progress_subscription_generation.wrapping_add(1);
        let generation = self.upload_progress_subscription_generation;
        self.upload_progress_task = Some(cx.spawn(async move |this, cx| {
            let mut updates = updates.await;
            while let Some(progress) = updates.next().await {
                let is_current = this
                    .update(cx, |cell, cx| {
                        if cell.upload_key.as_ref() != Some(&key) || cell.upload_progress_subscription_generation != generation {
                            return false;
                        }
                        let shown_before = cell.upload_overlay_signature();
                        match progress {
                            Some(progress) => {
                                cell.has_received_upload_progress = true;
                                cell.latest_upload_progress = Some(progress.clone());
                                cell.apply_upload_progress(&progress, false, cx);
                            }
                            None if cell.has_received_upload_progress => {
                                cell.latest_upload_progress = None;
                                cell.hide_upload_overlay(cx);
                            }
                            None => {}
                        }
                        if !cell.upload_ring_swaps_running || cell.upload_overlay_signature() != shown_before {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !is_current {
                    return;
                }
            }
            let _ = this.update(cx, |cell, _| {
                if cell.upload_key.as_ref() == Some(&key) && cell.upload_progress_subscription_generation == generation {
                    cell.upload_progress_task = None;
                }
            });
        }));
    }

    fn cancel_upload_progress_subscription(&mut self) {
        self.upload_progress_subscription_generation = self.upload_progress_subscription_generation.wrapping_add(1);
        self.upload_progress_task = None;
    }

    fn apply_upload_progress(&mut self, progress: &RiseUploadProgress, force_label: bool, cx: &mut Context<Self>) {
        if Some(progress.key()) != self.upload_key.as_ref() {
            return;
        }
        match progress.phase() {
            RiseEngineUploadPhase::Completed => {
                self.hide_upload_overlay(cx);
                return;
            }
            RiseEngineUploadPhase::Failed { .. } => {
                self.show_upload_overlay(cx);
                self.upload_stroke_color = rgb(0xFF3B30).into();
                self.stop_upload_progress_rotation();
            }
            RiseEngineUploadPhase::Preparing | RiseEngineUploadPhase::Uploading | RiseEngineUploadPhase::Processing => {
                self.show_upload_overlay(cx);
                self.upload_stroke_color = white();
                self.ensure_upload_progress_rotation(cx);
            }
        }

        let phase_changed = self.last_upload_label_phase.as_ref() != Some(progress.phase());
        let is_awaiting_server = RiseUploadRingPresentation::is_awaiting_server(progress);
        let presentation_changed = phase_changed || self.last_upload_was_awaiting_server != is_awaiting_server;
        let visible_start = 0.0;
        let visible_fraction = RiseUploadRingPresentation::visible_fraction(progress);
        if presentation_changed
            || (visible_start - self.last_rendered_upload_start).abs() >= 0.002
            || (visible_fraction - self.last_rendered_upload_fraction).abs() >= 0.002
        {
            self.render_upload_presentation(visible_fraction, presentation_changed, cx);
            self.last_rendered_upload_start = visible_start;
            self.last_rendered_upload_fraction = visible_fraction;
        }

        let now = RiseClock::monotonic();
        let label_is_stale = self
            .last_upload_label_update_time
            .is_none_or(|last| now.saturating_sub(last) >= Self::UPLOAD_LABEL_THROTTLE);
        if force_label || presentation_changed || label_is_stale {
            self.upload_status_text = Some(RiseUploadRingPresentation::status_text(progress));
            self.last_upload_label_update_time = Some(now);
            self.last_upload_label_phase = Some(progress.phase().clone());
        }
        self.last_upload_was_awaiting_server = is_awaiting_server;
    }

    fn render_upload_presentation(&mut self, stroke_end: f64, phase_changed: bool, cx: &mut Context<Self>) {
        let target_end = stroke_end.clamp(0.0, 1.0) as f32;
        let has_rendered_progress = self.last_rendered_upload_fraction >= 0.0;
        let now = RiseClock::monotonic();
        let current_end = self.upload_stroke_end.value(now);
        if !has_rendered_progress || !self.is_in_window || reduce_motion(cx) {
            self.upload_stroke_end.jump(target_end);
            return;
        }
        if (target_end - current_end).abs() < 0.002 {
            self.upload_stroke_end.jump(target_end);
            return;
        }
        let duration = if phase_changed && target_end < current_end { 200 } else { 140 };
        let spec = MotionSpec::curve(CubicBezier::LINEAR, Duration::from_millis(duration));
        self.upload_stroke_end.set_in(target_end, spec, now, cx);
    }

    fn show_upload_overlay(&mut self, cx: &mut Context<Self>) {
        if !self.upload_overlay_is_visible {
            self.upload_overlay_is_visible = true;
            self.play_is_hidden = true;
            self.duration_is_hidden = true;
            self.configure_transfer_chip(cx);
        }
        self.duration_is_hidden = true;
        self.ensure_upload_progress_rotation(cx);
    }

    fn hide_upload_overlay(&mut self, cx: &mut Context<Self>) {
        let was_visible = self.upload_overlay_is_visible;
        self.upload_overlay_is_visible = false;
        self.upload_status_text = None;
        self.upload_stroke_color = white();
        let stroke_end = *self.upload_stroke_end.target();
        self.upload_stroke_end.jump(stroke_end);
        self.stop_upload_progress_rotation();
        self.play_is_hidden = !self.model_is_video || !self.view_badge_is_hidden;
        self.duration_is_hidden = !self.model_is_video || self.duration_text.is_none();
        if was_visible && self.download_seed.is_some() {
            self.configure_transfer_chip(cx);
        }
        self.last_rendered_upload_start = -1.0;
        self.last_rendered_upload_fraction = -1.0;
        self.last_upload_label_phase = None;
        self.last_upload_was_awaiting_server = false;
    }

    fn transfer_request(&self) -> Option<RiseMediaTransferRequest> {
        let download_seed = self.download_seed.as_ref()?;
        let remote = Url::from_string(&download_seed.remote_url)?;
        let suggested_filename = remote.last_path_component();
        Some(RiseMediaTransferRequest::new(
            remote,
            RiseMediaTransferRequestArgs {
                cache_key: Some(download_seed.cache_key.clone()),
                suggested_filename: Some(suggested_filename),
                declared_byte_count: download_seed.declared_byte_count,
                is_streamable: download_seed.is_streamable,
                account_user_id: self.account_user_id.clone(),
                account_epoch: self.account_epoch.clone(),
                ..RiseMediaTransferRequestArgs::default()
            },
        ))
    }

    fn configure_transfer_chip(&mut self, cx: &mut Context<Self>) {
        let seed = if self.upload_overlay_is_visible || self.is_layout_measurement { None } else { self.download_seed.clone() };
        let key = seed
            .as_ref()
            .map(|seed| RiseMediaTransferKey::new(seed.cache_key.clone(), self.account_epoch.clone()));
        let declared_byte_count = seed.as_ref().map_or(0, |seed| seed.declared_byte_count);
        let shows_idle_affordance = seed.as_ref().is_some_and(|seed| seed.shows_idle_affordance);
        let side = f32::from(self.bounds_size.width).min(f32::from(self.bounds_size.height));
        let shows_byte_counts = self.bounds_size.width >= px(112.0);
        let paused = !self.is_visible || self.is_lifted;
        self.transfer_chip.update(cx, |chip, cx| {
            chip.configure(
                key,
                declared_byte_count,
                RiseMediaTransferChipViewMetrics::fitting(side),
                shows_byte_counts,
                shows_idle_affordance,
                false,
                shows_idle_affordance,
                true,
                cx,
            );
            chip.set_paused(paused, cx);
        });
    }

    fn ensure_upload_progress_rotation(&mut self, cx: &mut Context<Self>) {
        if !self.upload_overlay_is_visible
            || !self.is_in_window
            || !self.is_visible
            || reduce_motion(cx)
            || self.upload_rotation_started_at.is_some()
        {
            return;
        }
        self.upload_rotation_started_at = Some(RiseClock::monotonic());
        cx.notify();
    }

    fn stop_upload_progress_rotation(&mut self) {
        self.upload_rotation_started_at = None;
    }

    fn upload_overlay_signature(&self) -> (bool, Option<String>, Hsla, bool, bool, bool) {
        (
            self.upload_overlay_is_visible,
            self.upload_status_text.clone(),
            self.upload_stroke_color,
            self.play_is_hidden,
            self.duration_is_hidden,
            self.upload_rotation_started_at.is_some(),
        )
    }

    fn ensure_upload_ring_swaps(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.upload_ring_swaps_running {
            return;
        }
        self.upload_ring_swaps_running = true;
        Self::schedule_upload_ring_swap(cx.weak_entity(), window);
    }

    fn schedule_upload_ring_swap(weak: gpui::WeakEntity<Self>, window: &Window) {
        window.on_next_frame(move |window, cx| {
            let _ = weak.clone().update(cx, |cell, cx| cell.swap_upload_ring(weak, window, cx));
        });
    }

    fn swap_upload_ring(&mut self, weak: gpui::WeakEntity<Self>, window: &mut Window, cx: &mut Context<Self>) {
        let started_at = self.upload_rotation_started_at.filter(|_| self.upload_overlay_is_visible && self.is_in_window);
        let swapped = started_at.is_some_and(|started_at| {
            let now = RiseClock::monotonic();
            let (ring_side, line_width, _) = self.upload_ring_metrics();
            let spin = ring_spin_degrees(now.saturating_sub(started_at), Self::UPLOAD_ROTATION_PERIOD);
            let stroke_end = self.upload_stroke_end.value(now);
            match upload_arc_path(ring_side, line_width, stroke_end, spin, gpui::Point::default()) {
                Some(path) => self.upload_ring_painted.get() && window.swap_path(cx.entity_id().as_u64(), path),
                None => !self.upload_ring_painted.get(),
            }
        });
        if swapped {
            Self::schedule_upload_ring_swap(weak, window);
        } else {
            self.upload_ring_swaps_running = false;
            cx.notify();
        }
    }

    fn begin_load_if_needed(&mut self, cx: &mut Context<Self>) {
        let needs_picture = self.image.is_none()
            || self.stale_decode
            || self.displayed_source != self.source
            || (self.needs_tail_colors && !self.media_tail_colors_resolved);
        if !self.is_in_window
            || self.is_layout_measurement
            || !self.is_visible
            || !needs_picture
            || self.load_task.is_some()
            || self.did_fail_load
        {
            return;
        }
        let resolved_source = self.source.clone();
        let resolved_video_source = self.fallback_video_source.clone();
        if resolved_source.is_empty() && resolved_video_source.is_empty() {
            return;
        }
        let generation = self.load_generation;
        let target_size = self.bounds_size;
        let screen_scale = self.screen_scale;
        let needs_backdrop = self.is_single;
        let media_aspect = self.source_aspect;
        let needs_tail_colors = self.needs_tail_colors;
        let expected_user_id = self.account_user_id.clone();
        let expected_epoch = self.account_epoch.clone();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let thumbnail = RiseNativeMediaThumbnailPipeline::thumbnail(
                &resolved_source,
                &resolved_video_source,
                target_size,
                screen_scale,
                media_aspect,
                needs_backdrop,
                needs_tail_colors,
                expected_user_id.clone(),
                expected_epoch.clone(),
                cx,
            )
            .await;
            let _ = this.update(cx, |cell, cx| {
                if !cell.is_in_window
                    || cell.load_generation != generation
                    || cell.source != resolved_source
                    || cell.fallback_video_source != resolved_video_source
                    || !SaiAccountCacheScope::is_current_account_epoch(expected_user_id.as_deref(), expected_epoch.as_deref())
                {
                    return;
                }
                cell.load_task = None;
                match thumbnail {
                    Some(thumbnail) => {
                        cell.failed_load_attempts = 0;
                        cell.apply(
                            Some(thumbnail.image),
                            thumbnail.backdrop,
                            thumbnail.tail_colors,
                            thumbnail.tail_colors_resolved,
                            cx,
                        );
                    }
                    None => {
                        cell.did_fail_load = true;
                        cell.schedule_failed_load_retry(cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn schedule_failed_load_retry(&mut self, cx: &mut Context<Self>) {
        self.failed_load_retry_task = None;
        if !self.is_visible || !self.is_in_window || self.is_layout_measurement {
            return;
        }
        let attempt = self.failed_load_attempts;
        let Some(delay) = Self::FAILED_LOAD_RETRY_DELAYS.get(attempt).copied() else { return };
        self.failed_load_attempts = attempt + 1;
        let generation = self.load_generation;
        self.failed_load_retry_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(delay)).await;
            let _ = this.update(cx, |cell, cx| {
                if cell.load_generation != generation || !cell.is_in_window || !cell.is_visible {
                    return;
                }
                cell.failed_load_retry_task = None;
                cell.did_fail_load = false;
                cell.begin_load_if_needed(cx);
            });
        }));
    }

    fn cancel_failed_load_retry(&mut self, resets_attempts: bool) {
        self.failed_load_retry_task = None;
        if resets_attempts {
            self.failed_load_attempts = 0;
        }
    }

    fn retry_failed_load_after_reconnect(&mut self, cx: &mut Context<Self>) {
        if !self.did_fail_load || !self.is_visible || !self.is_in_window || self.is_layout_measurement {
            return;
        }
        self.did_fail_load = false;
        self.cancel_failed_load_retry(true);
        self.begin_load_if_needed(cx);
    }

    fn apply(
        &mut self,
        image: Option<CleverImage>,
        backdrop: Option<CleverImage>,
        tail_colors: Option<MessageMediaTailEdgeColors>,
        tail_colors_resolved: bool,
        cx: &mut Context<Self>,
    ) {
        let has_image = image.is_some();
        self.displayed_source = if has_image { self.source.clone() } else { String::new() };
        if self.spoiler_id.is_some() {
            let (spoiler_image, key) = (image.clone(), self.displayed_source.clone());
            self.spoiler_view.update(cx, |spoiler, cx| spoiler.set_source(spoiler_image, &key, cx));
        }
        self.keeps_replaced_crop = false;
        self.stale_decode = false;
        let has_backdrop = backdrop.is_some();
        self.backdrop_image = if self.is_single { backdrop.or_else(|| image.clone()) } else { None };
        self.image = image;
        let shows_backdrop = self.is_single && has_image;
        self.shows_backdrop = shows_backdrop;
        if shows_backdrop && has_backdrop {
            RiseAppEvents::default_center().post(RISE_NATIVE_MEDIA_BACKDROP_APPLIED);
        }
        self.media_tail_colors = if has_image { tail_colors } else { None };
        self.media_tail_colors_resolved = has_image && tail_colors_resolved;
        if self.needs_tail_colors && (self.media_tail_colors_resolved || !has_image) {
            self.report_tail_colors(cx);
        }
    }

    fn cancel_load(&mut self, clear_image: bool, cx: &mut Context<Self>) {
        self.load_task = None;
        self.load_generation = self.load_generation.wrapping_add(1);
        self.cancel_failed_load_retry(clear_image);
        if clear_image {
            self.apply(None, None, None, false, cx);
        }
    }

    fn is_effectively_visible(&self, cx: &mut App) -> bool {
        if self.is_layout_measurement || !self.is_in_window {
            return false;
        }
        if RiseAppLifecycle::scene_phase(cx) == ScenePhase::Background {
            return false;
        }
        if self.viewport_scope_view.as_ref().is_some_and(|scope| !scope.is_animated_media_visible_in_viewport()) {
            return false;
        }
        if chat_media_presentation(cx).read(cx).covers_page() {
            return false;
        }
        let Some(registry_id) = &self.registry_id else { return false };
        MediaCellScreenFrameRegistry::attached_view(registry_id).is_some_and(|view| view.is_visible_in_active_clipped_viewport())
    }

    fn trace_hidden_after_foreground(&self, cx: &mut App) {
        let phase = RiseAppLifecycle::scene_phase(cx);
        let in_viewport = self.viewport_scope_view.as_ref().map(|scope| scope.is_animated_media_visible_in_viewport());
        let covered = chat_media_presentation(cx).read(cx).covers_page();
        let attached = self.registry_id.as_ref().and_then(|registry_id| MediaCellScreenFrameRegistry::attached_view(registry_id));
        let clipped = attached.as_ref().map(|view| view.is_visible_in_active_clipped_viewport());
        let clipped_any_scope = attached.as_ref().map(|view| view.is_visible_in_active_clipped_viewport_ignoring_playback_scope());
        tracing::warn!(
            target: "riseonly::X2157",
            "tile hidden after foreground id={} phase={phase:?} in_viewport={in_viewport:?} covered={covered} clipped={clipped:?} clipped_any_scope={clipped_any_scope:?}",
            self.registry_id.as_deref().unwrap_or_default()
        );
    }

    fn trace_hidden_after_recheck(&self, cx: &mut App) {
        let phase = RiseAppLifecycle::scene_phase(cx);
        let in_viewport = self.viewport_scope_view.as_ref().map(|scope| scope.is_animated_media_visible_in_viewport());
        let covered = chat_media_presentation(cx).read(cx).covers_page();
        let attached = self.registry_id.as_ref().and_then(|registry_id| MediaCellScreenFrameRegistry::attached_view(registry_id));
        let clipped = attached.as_ref().map(|view| view.is_visible_in_active_clipped_viewport());
        let clipped_any_scope = attached.as_ref().map(|view| view.is_visible_in_active_clipped_viewport_ignoring_playback_scope());
        let record = self.registry_id.as_deref().and_then(MediaCellScreenFrameRegistry::frame);
        tracing::warn!(
            target: "riseonly::X2300",
            "tile hidden after recheck id={} phase={phase:?} in_viewport={in_viewport:?} covered={covered} attached={} clipped={clipped:?} clipped_any_scope={clipped_any_scope:?} record={record:?} image={} loading={}",
            self.registry_id.as_deref().unwrap_or_default(),
            attached.is_some(),
            self.image.is_some(),
            self.load_task.is_some()
        );
    }

    fn refresh_visibility_subscription(&mut self, cx: &mut Context<Self>) {
        if !self.is_in_window || self.is_layout_measurement {
            self.stop_visibility_tracking();
            return;
        }
        if !self.visibility_observers.is_empty() {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<RiseNotificationName>();
        for name in [
            ANIMATED_MEDIA_VIEWPORT_SCOPE_DID_CHANGE,
            ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE,
            RISE_MEDIA_FOREGROUND_DID_CHANGE,
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION,
            RISE_ENGINE_CONNECTION_READY,
        ] {
            let sender = sender.clone();
            self.visibility_observers.push(RiseAppEvents::default_center().observe(name, move |_| {
                let _ = sender.unbounded_send(name);
            }));
        }
        self.visibility_events_task = Some(cx.spawn(async move |this, cx| {
            while let Some(name) = receiver.next().await {
                let alive = this
                    .update(cx, |cell, cx| {
                        if name == RISE_ENGINE_CONNECTION_READY {
                            cell.retry_failed_load_after_reconnect(cx);
                        } else {
                            cell.report_visibility_if_changed(cx);
                            let is_foreground_return = name == UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION
                                || name == RISE_MEDIA_FOREGROUND_DID_CHANGE;
                            if is_foreground_return && !cell.is_visible && cell.is_in_window && !cell.is_layout_measurement {
                                cell.trace_hidden_after_foreground(cx);
                                cell.schedule_visibility_recheck(cx);
                            }
                        }
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        }));
    }

    fn stop_visibility_tracking(&mut self) {
        self.visibility_observers.clear();
        self.visibility_events_task = None;
        self.viewport_scope_view = None;
    }

    fn report_visibility_if_changed(&mut self, cx: &mut Context<Self>) {
        let visible = self.is_effectively_visible(cx);
        if Some(visible) == self.last_reported_visibility {
            return;
        }
        self.last_reported_visibility = Some(visible);
        let drawn = (self.drawn_state(), self.upload_rotation_started_at.is_some());
        self.is_visible = visible;
        tracing::warn!(
            target: "riseonly::X590",
            "tile visibility id={} visible={visible}",
            self.registry_id.as_deref().unwrap_or_default()
        );
        let paused = !visible || self.is_lifted;
        self.transfer_chip.update(cx, |chip, cx| chip.set_paused(paused, cx));
        if visible {
            self.did_fail_load = false;
            self.cancel_failed_load_retry(true);
            self.begin_load_if_needed(cx);
            if self.load_task.is_none() {
                tracing::warn!(
                    target: "riseonly::X2300",
                    "tile visible without load id={} image={} same_source={} source_empty={}",
                    self.registry_id.as_deref().unwrap_or_default(),
                    self.image.is_some(),
                    self.displayed_source == self.source,
                    self.source.is_empty()
                );
            }
            self.begin_upload_progress_subscription_if_needed(cx);
            self.ensure_upload_progress_rotation(cx);
        } else if self.load_task.is_some() {
            self.cancel_load(false, cx);
            self.cancel_upload_progress_subscription();
            self.stop_upload_progress_rotation();
        } else {
            self.cancel_failed_load_retry(false);
            self.cancel_upload_progress_subscription();
            self.stop_upload_progress_rotation();
        }
        if (self.drawn_state(), self.upload_rotation_started_at.is_some()) != drawn {
            cx.notify();
        }
    }

    fn child_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("rise-native-media-{}-{suffix}", self.registry_id.as_deref().unwrap_or_default()).into())
    }

    fn rounded<E: Styled>(&self, element: E) -> E {
        element
            .rounded_tl(self.radii.top_leading)
            .rounded_tr(self.radii.top_trailing)
            .rounded_bl(self.radii.bottom_leading)
            .rounded_br(self.radii.bottom_trailing)
    }

    fn upload_ring_metrics(&self) -> (f32, f32, f32) {
        let fit = f32::from(self.bounds_size.width).min(f32::from(self.bounds_size.height)).max(1.0);
        let scale = (fit / 130.0).min(1.0);
        let ring_side = (52.0 * scale).min(52.0).max(28.0);
        let line_width = (2.5 * scale).max(1.8);
        let icon_side = (17.0 * scale).min(17.0).max(11.0);
        (ring_side, line_width, icon_side)
    }
}

impl Render for RiseNativeMessageImageCellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.screen_scale = window.scale_factor();
        self.viewport_scope_view = nearest_animated_media_viewport_scope_view(cx);
        if !self.is_in_window && !self.is_layout_measurement {
            self.is_in_window = true;
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |cell, cx| cell.did_move_to_window(cx));
            })
            .detach();
        }

        let now = RiseClock::monotonic();
        let width = f32::from(self.bounds_size.width);
        let height = f32::from(self.bounds_size.height);
        let opacity = if self.is_lifted { 0.0 } else { 1.0 };
        let mut tile = self.rounded(div().relative().size_full().opacity(opacity).bg(self.background_color));

        if self.shows_backdrop && !self.keeps_replaced_crop {
            if let Some(backdrop) = self.backdrop_image.clone() {
                tile = tile.child(self.rounded(img(backdrop).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)));
                tile = tile.child(self.rounded(div().absolute().inset_0().bg(black().opacity(0.24))));
            }
        }
        if let Some(image) = self.image.clone() {
            let fit = if self.is_single && !self.keeps_replaced_crop { ObjectFit::Contain } else { ObjectFit::Cover };
            tile = tile.child(self.rounded(img(image).absolute().inset_0().size_full().object_fit(fit)));
        }
        if !self.spoiler_is_hidden {
            tile = tile.child(self.rounded(div().absolute().inset_0().child(self.spoiler_view.clone())));
        }
        let view_badge = self.view_badge(window);
        if !self.play_is_hidden {
            let play_side = if width.min(height) > 70.0 { 44.0 } else { 24.0 };
            let icon_side = if play_side > 30.0 { 18.0 } else { 10.0 };
            let nudge = if play_side > 30.0 { 1.0 } else { 0.0 };
            tile = tile.child(
                div()
                    .absolute()
                    .left(px((width - play_side) / 2.0))
                    .top(px((height - play_side) / 2.0))
                    .size(px(play_side))
                    .rounded(px(play_side / 2.0))
                    .bg(black().opacity(0.44))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().pl(px(nudge)).children(IconUi::sized("play.fill", px(icon_side), white(), true))),
            );
        }
        if let (false, Some(duration_text)) = (self.duration_is_hidden, self.duration_text.clone()) {
            tile = tile.child(
                div()
                    .absolute()
                    .top(px(self.duration_top_inset))
                    .right(px(6.0))
                    .h(px(20.0))
                    .px(px(6.0))
                    .rounded(px(10.0))
                    .bg(black().opacity(0.55))
                    .flex()
                    .items_center()
                    .justify_center()
                    .swiftui_font(Font::system_weight(11.0, FontWeight::SEMIBOLD))
                    .text_color(white())
                    .whitespace_nowrap()
                    .child(SharedString::from(duration_text)),
            );
        }
        if self.upload_overlay_is_visible {
            let (ring_side, line_width, icon_side) = self.upload_ring_metrics();
            let swaps_ring = self.upload_rotation_started_at.is_some();
            let stroke_end = if swaps_ring {
                self.upload_stroke_end.value(now)
            } else {
                self.upload_stroke_end.frame_value(now, window)
            };
            let stroke_color = self.upload_stroke_color;
            let spin = self
                .upload_rotation_started_at
                .map_or(0.0, |started_at| ring_spin_degrees(now.saturating_sub(started_at), Self::UPLOAD_ROTATION_PERIOD));
            if swaps_ring {
                self.ensure_upload_ring_swaps(window, cx);
            }
            let ring_swap_key = cx.entity_id().as_u64();
            let ring_painted = self.upload_ring_painted.clone();
            tile = tile.child(self.rounded(div().absolute().inset_0().bg(black().opacity(0.30))));
            if let Some(status_text) = self.upload_status_text.clone() {
                tile = tile.child(
                    div()
                        .absolute()
                        .left(px(8.0))
                        .top(px(8.0))
                        .h(px(20.0))
                        .max_w(px((width - 16.0).max(40.0)))
                        .px(px(8.0))
                        .rounded(px(10.0))
                        .bg(black().opacity(0.48))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .swiftui_font(Font::system_weight(11.0, FontWeight::SEMIBOLD))
                        .text_color(white().opacity(0.96))
                        .child(
                            div()
                                .flex_shrink_1()
                                .min_w_0()
                                .truncate()
                                .child(SharedString::from(status_text)),
                        ),
                );
            }
            let ring = div()
                .relative()
                .size(px(ring_side))
                .rounded(px(ring_side / 2.0))
                .bg(black().opacity(0.50))
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            let painted = paint_upload_arc(bounds, line_width, stroke_end, spin, stroke_color, ring_swap_key, window);
                            ring_painted.set(painted);
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("xmark", px(icon_side), white(), false)),
                );
            let weak = cx.weak_entity();
            tile = tile.child(
                div()
                    .absolute()
                    .left(px(((width - ring_side) / 2.0).floor() - 8.0))
                    .top(px(((height - ring_side) / 2.0).floor() - 8.0))
                    .child(div().p(px(8.0)).child(ring).bubble_tap_target_priority(
                        self.child_id("upload-cancel"),
                        move |window: &mut Window, cx: &mut App| {
                            let on_cancel_upload = weak.read_with(&*cx, |cell, _| cell.on_cancel_upload.clone()).ok().flatten();
                            if let Some(on_cancel_upload) = on_cancel_upload {
                                on_cancel_upload(window, cx);
                            }
                        },
                    )),
            );
        }

        let chip_is_actionable = self.transfer_chip.read(cx).is_actionable();
        let chip_is_drawn = self.transfer_chip.read(cx).is_drawn();
        self.transfer_chip_was_drawn = chip_is_drawn;
        let chip = div()
            .absolute()
            .left(px(8.0))
            .top(px(self.transfer_chip_top_inset))
            .max_w(px((width - 16.0).max(0.0)))
            .child(self.transfer_chip.clone());
        if chip_is_actionable && !self.is_layout_measurement && !self.is_lifted {
            let weak = cx.weak_entity();
            tile = tile.child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(self.transfer_chip_top_inset - 8.0))
                    .child(div().p(px(8.0)).child(div().relative().max_w(px((width - 16.0).max(0.0))).child(self.transfer_chip.clone())).bubble_tap_target_priority(
                        self.child_id("transfer-chip"),
                        move |_: &mut Window, cx: &mut App| {
                            let target = weak
                                .read_with(&*cx, |cell, _| (cell.transfer_chip.clone(), cell.transfer_request()))
                                .ok();
                            if let Some((chip, Some(request))) = target {
                                chip.update(cx, |chip, cx| chip.handle_tap(request, cx));
                            }
                        },
                    )),
            );
        } else if chip_is_drawn {
            tile = tile.child(chip);
        }

        tile = tile.children(view_badge);
        if self.is_layout_measurement {
            return tile.into_any_element();
        }
        let Some(registry_id) = self.registry_id.clone() else { return tile.into_any_element() };
        let weak = cx.weak_entity();
        tile = tile.child(WindowScreenFrameReader::new(registry_id.clone()).on_change(
            move |_, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |cell, cx| {
                    cell.report_visibility_if_changed(cx);
                    cell.begin_load_if_needed(cx);
                    cell.begin_upload_progress_subscription_if_needed(cx);
                });
            },
        ));
        if self.is_lifted {
            return tile.into_any_element();
        }
        let weak = cx.weak_entity();
        tile.media_bubble_tap_target(self.child_id("tap"), registry_id, move |window: &mut Window, cx: &mut App| {
            let spoiler_closed = weak
                .update(cx, |cell, cx| {
                    if cell.view_phase == Some(MessageMediaViewPhase::Consumed) {
                        return true;
                    }
                    if cell.spoiler_id.is_some() && cell.view_phase != Some(MessageMediaViewPhase::Sealed) {
                        cell.open_spoiler(cx);
                        cx.notify();
                        return true;
                    }
                    false
                })
                .unwrap_or(true);
            if spoiler_closed {
                return;
            }
            let on_tap = weak.read_with(&*cx, |cell, _| cell.on_tap.clone()).ok().flatten();
            tracing::warn!(target: "riseonly::X638", "tile tap has_action={}", on_tap.is_some());
            if let Some(on_tap) = on_tap {
                on_tap(window, cx);
            }
        })
        .into_any_element()
    }
}

fn paint_upload_arc(
    bounds: Bounds<Pixels>,
    line_width: f32,
    stroke_end: f32,
    spin: f32,
    color: Hsla,
    swap_key: u64,
    window: &mut Window,
) -> bool {
    let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
    let Some(path) = upload_arc_path(side, line_width, stroke_end, spin, bounds.origin) else {
        return false;
    };
    window.paint_swappable_path(path, color, swap_key, bounds.origin);
    true
}

/// ring arc with round ends as one filled outline in a square at origin
/// so a path swap moves the ends with the arc
fn upload_arc_path(side: f32, line_width: f32, stroke_end: f32, spin: f32, origin: gpui::Point<Pixels>) -> Option<gpui::Path<Pixels>> {
    use std::f32::consts::{PI, TAU};
    if stroke_end <= 0.0 {
        return None;
    }
    let radius = side / 2.0 - line_width / 2.0 - 1.0;
    let half = line_width / 2.0;
    if radius <= half {
        return None;
    }
    let center = point(origin.x + px(side / 2.0), origin.y + px(side / 2.0));
    let at = |around: gpui::Point<Pixels>, distance: f32, angle: f32| point(around.x + px(distance * angle.cos()), around.y + px(distance * angle.sin()));
    let start = spin.to_radians();
    let sweep = stroke_end.min(1.0) * TAU;
    let end = start + sweep;
    let steps = ((sweep / TAU) * 96.0).ceil().max(2.0) as usize;
    const CAP_STEPS: usize = 10;
    let mut points = Vec::with_capacity(2 * steps + 2 * CAP_STEPS + 2);
    for step in 0..=steps {
        points.push(at(center, radius + half, start + sweep * step as f32 / steps as f32));
    }
    let end_cap = at(center, radius, end);
    for step in 1..CAP_STEPS {
        points.push(at(end_cap, half, end + PI * step as f32 / CAP_STEPS as f32));
    }
    for step in (0..=steps).rev() {
        points.push(at(center, radius - half, start + sweep * step as f32 / steps as f32));
    }
    let start_cap = at(center, radius, start);
    for step in 1..CAP_STEPS {
        points.push(at(start_cap, half, start + PI + PI * step as f32 / CAP_STEPS as f32));
    }
    let mut builder = gpui::PathBuilder::fill().with_style(gpui::PathStyle::Fill(gpui::FillOptions::non_zero()));
    builder.add_polygon(&points, true);
    builder.build().ok()
}

pub struct RiseNativeMediaThumbnail {
    pub image: CleverImage,
    pub backdrop: Option<CleverImage>,
    pub tail_colors: Option<MessageMediaTailEdgeColors>,
    pub tail_colors_resolved: bool,
}

#[derive(Clone, Copy)]
struct RiseNativeMediaTailColorsBox {
    colors: Option<MessageMediaTailEdgeColors>,
}

struct RiseNativeMediaWorkGateWaiter {
    continuation: oneshot::Sender<RiseNativeMediaWorkGatePermit>,
}

struct RiseNativeMediaWorkGateState {
    active_count: usize,
    waiters: Vec<RiseNativeMediaWorkGateWaiter>,
}

impl RiseNativeMediaWorkGateState {
    fn release(state: &Arc<Mutex<Self>>) {
        loop {
            let waiter = {
                let mut locked = state.lock();
                if locked.waiters.is_empty() {
                    locked.active_count = locked.active_count.saturating_sub(1);
                    return;
                }
                locked.waiters.remove(0)
            };
            let permit = RiseNativeMediaWorkGatePermit { state: Some(Arc::clone(state)) };
            match waiter.continuation.send(permit) {
                Ok(()) => return,
                Err(mut refused) => {
                    let _ = refused.state.take();
                }
            }
        }
    }
}

/// limits thumbnail decodes, at most limit run at once
struct RiseNativeMediaWorkGate {
    limit: usize,
    state: Arc<Mutex<RiseNativeMediaWorkGateState>>,
}

/// slot the gate granted, dropping it releases so a cancelled load never keeps one
struct RiseNativeMediaWorkGatePermit {
    state: Option<Arc<Mutex<RiseNativeMediaWorkGateState>>>,
}

impl RiseNativeMediaWorkGate {
    fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            state: Arc::new(Mutex::new(RiseNativeMediaWorkGateState { active_count: 0, waiters: Vec::new() })),
        }
    }

    async fn acquire(&self) -> Option<RiseNativeMediaWorkGatePermit> {
        let receiver = {
            let mut state = self.state.lock();
            if state.active_count < self.limit {
                state.active_count += 1;
                return Some(RiseNativeMediaWorkGatePermit { state: Some(Arc::clone(&self.state)) });
            }
            let (sender, receiver) = oneshot::channel();
            state.waiters.push(RiseNativeMediaWorkGateWaiter { continuation: sender });
            receiver
        };
        receiver.await.ok()
    }
}

impl Drop for RiseNativeMediaWorkGatePermit {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            RiseNativeMediaWorkGateState::release(&state);
        }
    }
}

fn client_limit(name: &str) -> usize {
    usize::try_from(RiseLimits::client_value(name)).unwrap_or(0)
}

fn image_cost(image: &RenderImage) -> usize {
    let size = image.size(0);
    let width = usize::try_from(size.width.0).unwrap_or(0);
    let height = usize::try_from(size.height.0).unwrap_or(0);
    width.saturating_mul(4).saturating_mul(height)
}

fn bounded_cache<V: Clone + Send + 'static>(count_limit: &str, cost_limit: Option<&str>) -> NSCache<String, V> {
    let cache = NSCache::new();
    cache.set_count_limit(client_limit(count_limit));
    if let Some(cost_limit) = cost_limit {
        cache.set_total_cost_limit(client_limit(cost_limit));
    }
    cache
}

static MEMORY_CACHE: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    bounded_cache("native_media_thumbnail_cache_entries", Some("native_media_thumbnail_cache_bytes"))
});
static BACKDROP_CACHE: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    bounded_cache("native_media_backdrop_cache_entries", Some("native_media_backdrop_cache_bytes"))
});
static TAIL_COLORS_CACHE: LazyLock<NSCache<String, RiseNativeMediaTailColorsBox>> =
    LazyLock::new(|| bounded_cache("native_media_tail_colors_cache_entries", None));
static WORK_GATE: LazyLock<RiseNativeMediaWorkGate> =
    LazyLock::new(|| RiseNativeMediaWorkGate::new(client_limit("native_media_thumbnail_work_concurrency")));

/// cache identity of one thumbnail request
struct RiseNativeMediaThumbnailKeys {
    normalized_source: String,
    fallback_video_source: Option<String>,
    source: String,
    max_pixel_size: f32,
    cache_key: String,
    backdrop_cache_key: String,
    tail_colors_cache_key: String,
}

pub enum RiseNativeMediaThumbnailPipeline {}

impl RiseNativeMediaThumbnailPipeline {
    /// soft fill a tile keeps behind its fitted picture, for hosts that continue it past the tile box
    /// memory only, never loads
    pub fn cached_backdrop(
        raw_source: &str,
        raw_fallback_video_source: &str,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Option<CleverImage> {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            return None;
        }
        let fallback_video_source = RemoteMediaURL::optional_normalized(Some(raw_fallback_video_source));
        let normalized_source =
            RemoteMediaURL::optional_normalized(Some(raw_source)).or(fallback_video_source).unwrap_or_default();
        if normalized_source.is_empty() {
            return None;
        }
        let source = CleverImageDiskCache::normalized_cache_key(&normalized_source);
        BACKDROP_CACHE.object(&Self::backdrop_cache_key(account_user_id, account_epoch, &source))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn cached_thumbnail(
        raw_source: &str,
        raw_fallback_video_source: &str,
        target_size: Size<Pixels>,
        screen_scale: f32,
        source_aspect: f32,
        needs_backdrop: bool,
        needs_tail_colors: bool,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Option<RiseNativeMediaThumbnail> {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            return None;
        }
        let keys = Self::keys(
            raw_source,
            raw_fallback_video_source,
            target_size,
            screen_scale,
            source_aspect,
            needs_backdrop,
            account_user_id,
            account_epoch,
        )?;
        let media_viewer_image = MediaViewerImageMemoryCache::memory_only(&keys.normalized_source, account_user_id);
        if let Some(media_viewer_image) = media_viewer_image {
            if MEMORY_CACHE.object(&keys.cache_key).is_none() {
                let cost = image_cost(&media_viewer_image);
                MEMORY_CACHE.set_object_cost(Arc::clone(&media_viewer_image), keys.cache_key.clone(), cost);
                if needs_backdrop {
                    BACKDROP_CACHE.set_object_cost(media_viewer_image, keys.backdrop_cache_key.clone(), cost);
                }
            }
        }
        let image = MEMORY_CACHE.object(&keys.cache_key)?;
        let backdrop = if needs_backdrop { BACKDROP_CACHE.object(&keys.backdrop_cache_key) } else { None };
        let tail_box = if needs_tail_colors { TAIL_COLORS_CACHE.object(&keys.tail_colors_cache_key) } else { None };
        Some(RiseNativeMediaThumbnail {
            image,
            backdrop,
            tail_colors: tail_box.and_then(|tail_box| tail_box.colors),
            tail_colors_resolved: !needs_tail_colors || tail_box.is_some(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn thumbnail(
        raw_source: &str,
        raw_fallback_video_source: &str,
        target_size: Size<Pixels>,
        screen_scale: f32,
        source_aspect: f32,
        needs_backdrop: bool,
        needs_tail_colors: bool,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Option<RiseNativeMediaThumbnail> {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return None;
        }
        if let Some(cached) = Self::cached_thumbnail(
            raw_source,
            raw_fallback_video_source,
            target_size,
            screen_scale,
            source_aspect,
            needs_backdrop,
            needs_tail_colors,
            account_user_id.as_deref(),
            account_epoch.as_deref(),
        ) {
            if (!needs_backdrop || cached.backdrop.is_some()) && cached.tail_colors_resolved {
                return Some(cached);
            }
        }
        let keys = Self::keys(
            raw_source,
            raw_fallback_video_source,
            target_size,
            screen_scale,
            source_aspect,
            needs_backdrop,
            account_user_id.as_deref(),
            account_epoch.as_deref(),
        )?;
        if let Some(cached) = Self::memory_hit(&keys, target_size, needs_backdrop, needs_tail_colors, cx).await {
            return Some(cached);
        }
        let _permit = WORK_GATE.acquire().await?;
        Self::load_thumbnail(&keys, screen_scale, target_size, needs_backdrop, needs_tail_colors, account_user_id, account_epoch, cx)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    fn keys(
        raw_source: &str,
        raw_fallback_video_source: &str,
        target_size: Size<Pixels>,
        screen_scale: f32,
        source_aspect: f32,
        needs_backdrop: bool,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Option<RiseNativeMediaThumbnailKeys> {
        let fallback_video_source = RemoteMediaURL::optional_normalized(Some(raw_fallback_video_source));
        let normalized_source =
            RemoteMediaURL::optional_normalized(Some(raw_source)).or_else(|| fallback_video_source.clone()).unwrap_or_default();
        if normalized_source.is_empty() {
            return None;
        }
        let source = CleverImageDiskCache::normalized_cache_key(&normalized_source);
        let scale = screen_scale.max(1.0);
        let source_aspect = source_aspect.max(0.01);
        let width = f32::from(target_size.width);
        let height = f32::from(target_size.height);
        let fill_dimension = if needs_backdrop {
            width.max(height)
        } else if source_aspect > width / height.max(1.0) {
            height * source_aspect
        } else {
            width / source_aspect
        };
        let max_pixel_size = (fill_dimension * scale).ceil().min(1_024.0).max(64.0);
        let cache_key = format!(
            "{}|{}|{}|{source}",
            account_user_id.unwrap_or("anonymous"),
            account_epoch.unwrap_or("none"),
            max_pixel_size as i64
        );
        let tail_colors_cache_key = Self::tail_colors_cache_key(&cache_key, target_size, needs_backdrop);
        Some(RiseNativeMediaThumbnailKeys {
            backdrop_cache_key: Self::backdrop_cache_key(account_user_id, account_epoch, &source),
            normalized_source,
            fallback_video_source,
            source,
            max_pixel_size,
            cache_key,
            tail_colors_cache_key,
        })
    }

    fn backdrop_cache_key(account_user_id: Option<&str>, account_epoch: Option<&str>, source: &str) -> String {
        format!("{}|{}|backdrop|{source}", account_user_id.unwrap_or("anonymous"), account_epoch.unwrap_or("none"))
    }

    /// cached picture, tail colors get sampled now when only they are missing
    async fn memory_hit(
        keys: &RiseNativeMediaThumbnailKeys,
        target_size: Size<Pixels>,
        needs_backdrop: bool,
        needs_tail_colors: bool,
        cx: &mut AsyncApp,
    ) -> Option<RiseNativeMediaThumbnail> {
        let cached_image = MEMORY_CACHE.object(&keys.cache_key)?;
        let cached_backdrop = if needs_backdrop { BACKDROP_CACHE.object(&keys.backdrop_cache_key) } else { None };
        if needs_backdrop && cached_backdrop.is_none() {
            return None;
        }
        let cached_tail_box = if needs_tail_colors { TAIL_COLORS_CACHE.object(&keys.tail_colors_cache_key) } else { None };
        if !needs_tail_colors || cached_tail_box.is_some() {
            return Some(RiseNativeMediaThumbnail {
                image: cached_image,
                backdrop: cached_backdrop,
                tail_colors: cached_tail_box.and_then(|tail_box| tail_box.colors),
                tail_colors_resolved: true,
            });
        }
        let colors = Self::resolve_tail_colors(
            Arc::clone(&cached_image),
            cached_backdrop.clone(),
            target_size,
            needs_backdrop,
            cx,
        )
        .await;
        TAIL_COLORS_CACHE.set_object(RiseNativeMediaTailColorsBox { colors }, keys.tail_colors_cache_key.clone());
        Some(RiseNativeMediaThumbnail {
            image: cached_image,
            backdrop: cached_backdrop,
            tail_colors: colors,
            tail_colors_resolved: true,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn load_thumbnail(
        keys: &RiseNativeMediaThumbnailKeys,
        screen_scale: f32,
        target_size: Size<Pixels>,
        needs_backdrop: bool,
        needs_tail_colors: bool,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Option<RiseNativeMediaThumbnail> {
        let is_current =
            || SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref());
        if !is_current() {
            return None;
        }
        if let Some(cached) = Self::memory_hit(keys, target_size, needs_backdrop, needs_tail_colors, cx).await {
            return Some(cached);
        }
        let cached_image = MEMORY_CACHE.object(&keys.cache_key);
        let cached_backdrop = if needs_backdrop { BACKDROP_CACHE.object(&keys.backdrop_cache_key) } else { None };
        let cached_tail_box = if needs_tail_colors { TAIL_COLORS_CACHE.object(&keys.tail_colors_cache_key) } else { None };

        let source = keys.source.clone();
        let max_pixel_size = keys.max_pixel_size;
        let source_is_video = Self::is_likely_video_url(&source);
        let cached_data: Option<Arc<[u8]>> = {
            let source = source.clone();
            let user_id = account_user_id.clone();
            cx.background_executor()
                .spawn(async move {
                    if source_is_video {
                        CleverImageDiskCache::image_data(&source, user_id.as_deref(), None)
                    } else {
                        Self::data_from_local_or_cache(&source, user_id.as_deref())
                    }
                })
                .await
                .map(Arc::from)
        };

        let lowercased_source = source.to_lowercase();
        let directly_loaded_image = if cached_data.is_none() && lowercased_source.starts_with("ph://") {
            MediaViewerBitmapLoader::load_into_memory(&source, true, false, CleverImageCachePolicy::MemoryOnly, cx).await
        } else if cached_data.is_none() && !source_is_video && lowercased_source.starts_with("http") {
            Self::load_bitmap(&source, max_pixel_size, account_user_id.clone(), account_epoch.clone(), cx).await
        } else {
            None
        };
        tracing::warn!(
            target: "riseonly::X625",
            "thumb load source={} fallback={:?} video={} cached={} direct={}",
            source,
            keys.fallback_video_source,
            source_is_video,
            cached_data.is_some(),
            directly_loaded_image.is_some()
        );
        if !is_current() {
            return None;
        }

        let needs_image_decode = cached_image.is_none();
        let needs_backdrop_decode = needs_backdrop && cached_backdrop.is_none();
        let decoded: (Option<CleverImage>, Option<CleverImage>) = match cached_data.clone() {
            Some(cached_data) => {
                cx.background_executor()
                    .spawn(async move {
                        let image = if needs_image_decode { Self::downsample(&cached_data, max_pixel_size) } else { None };
                        let backdrop = if needs_backdrop_decode {
                            Self::downsample(&cached_data, max_pixel_size.min(640.0).max(384.0))
                        } else {
                            None
                        };
                        (image, backdrop)
                    })
                    .await
            }
            None => (
                directly_loaded_image.clone().filter(|_| needs_image_decode),
                directly_loaded_image.filter(|_| needs_backdrop_decode),
            ),
        };

        let mut final_image = cached_image.or_else(|| decoded.0.clone());
        let mut final_backdrop = cached_backdrop.or_else(|| decoded.1.clone());
        if let (None, Some(cached_data), true) = (&final_image, &cached_data, lowercased_source.starts_with("http")) {
            let removed_source = source.clone();
            let cached_data = Arc::clone(cached_data);
            let user_id = account_user_id.clone();
            let epoch = account_epoch.clone();
            cx.background_executor()
                .spawn(async move {
                    if SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), epoch.as_deref()) {
                        CleverImageDiskCache::remove_cache_key(&removed_source, user_id.as_deref(), Some(&cached_data[..]));
                    }
                })
                .await;
            if !source_is_video {
                final_image =
                    Self::load_bitmap(&source, max_pixel_size, account_user_id.clone(), account_epoch.clone(), cx).await;
                if needs_backdrop && final_backdrop.is_none() {
                    final_backdrop = final_image.clone();
                }
            }
        }
        if final_image.is_none() {
            if let Some(fallback_video_source) = &keys.fallback_video_source {
                let poster = Self::video_poster(
                    fallback_video_source,
                    max_pixel_size,
                    account_user_id.clone(),
                    account_epoch.clone(),
                    cx,
                )
                .await;
                if let Some(poster) = poster {
                    final_image = Some(Arc::clone(&poster));
                    if needs_backdrop && final_backdrop.is_none() {
                        final_backdrop = Some(Arc::clone(&poster));
                    }
                    let source = source.clone();
                    let user_id = account_user_id.clone();
                    let epoch = account_epoch.clone();
                    cx.background_executor()
                        .spawn(async move {
                            let Some(poster_data) = Self::jpeg_data(&poster, 78) else { return };
                            if SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), epoch.as_deref()) {
                                CleverImageDiskCache::store_data(&poster_data, &source, user_id.as_deref(), None);
                            }
                        })
                        .await;
                }
            }
        }

        tracing::warn!(target: "riseonly::X625", "thumb final source={} image={}", source, final_image.is_some());
        let final_image = final_image?;
        if !is_current() {
            return None;
        }
        if let Some(image) = &decoded.0 {
            MEMORY_CACHE.set_object_cost(Arc::clone(image), keys.cache_key.clone(), image_cost(image));
        }
        if let Some(backdrop) = &decoded.1 {
            BACKDROP_CACHE.set_object_cost(Arc::clone(backdrop), keys.backdrop_cache_key.clone(), image_cost(backdrop));
        }
        if decoded.0.is_none() {
            MEMORY_CACHE.set_object_cost(Arc::clone(&final_image), keys.cache_key.clone(), image_cost(&final_image));
        }
        if needs_backdrop && decoded.1.is_none() {
            if let Some(final_backdrop) = &final_backdrop {
                BACKDROP_CACHE.set_object_cost(
                    Arc::clone(final_backdrop),
                    keys.backdrop_cache_key.clone(),
                    image_cost(final_backdrop),
                );
            }
        }
        let tail_colors = if !needs_tail_colors {
            None
        } else if let Some(cached_tail_box) = cached_tail_box {
            cached_tail_box.colors
        } else {
            let backdrop = if needs_backdrop { final_backdrop.clone().or_else(|| Some(Arc::clone(&final_image))) } else { None };
            let colors = Self::resolve_tail_colors(Arc::clone(&final_image), backdrop, target_size, needs_backdrop, cx).await;
            TAIL_COLORS_CACHE.set_object(RiseNativeMediaTailColorsBox { colors }, keys.tail_colors_cache_key.clone());
            colors
        };
        let _ = screen_scale;
        Some(RiseNativeMediaThumbnail {
            image: final_image,
            backdrop: if needs_backdrop { final_backdrop } else { None },
            tail_colors,
            tail_colors_resolved: true,
        })
    }

    async fn load_bitmap(
        source: &str,
        max_pixel_size: f32,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Option<CleverImage> {
        CleverImageBitmapLoader::load(
            source,
            true,
            true,
            account_user_id,
            account_epoch,
            Some(max_pixel_size as i64),
            CleverImageCachePolicy::Standard,
            CleverImageWorkLane::Visible,
            cx,
        )
        .await
    }

    async fn resolve_tail_colors(
        foreground: CleverImage,
        backdrop: Option<CleverImage>,
        target_size: Size<Pixels>,
        uses_fit_foreground: bool,
        cx: &mut AsyncApp,
    ) -> Option<MessageMediaTailEdgeColors> {
        let target_width = f64::from(f32::from(target_size.width));
        let target_height = f64::from(f32::from(target_size.height));
        cx.background_executor()
            .spawn(async move {
                let foreground = MessageMediaTailBitmap::from_render_image(&foreground)?;
                let backdrop = backdrop.as_deref().and_then(MessageMediaTailBitmap::from_render_image);
                MessageMediaTailColorSampler::visible_bottom_edge_colors(
                    foreground,
                    backdrop,
                    target_width,
                    target_height,
                    uses_fit_foreground,
                )
            })
            .await
    }

    fn tail_colors_cache_key(image_cache_key: &str, target_size: Size<Pixels>, needs_backdrop: bool) -> String {
        format!(
            "{image_cache_key}|tail|{}x{}|fit:{needs_backdrop}",
            f32::from(target_size.width).round() as i64,
            f32::from(target_size.height).round() as i64
        )
    }

    async fn video_poster(
        source: &str,
        max_pixel_size: f32,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Option<CleverImage> {
        let (local, remote) = if is_local_absolute_path(source) {
            (Some(PathBuf::from(source)), None)
        } else {
            let url = Url::from_string(source)?;
            if url.is_file_url() { (Some(url.to_file_path()?), None) } else { (None, Some(url)) }
        };
        let cached_path = match &remote {
            Some(remote) => {
                let remote = remote.clone();
                let user_id = account_user_id.clone();
                let epoch = account_epoch.clone();
                cx.background_executor()
                    .spawn(async move {
                        let cache_key = RiseMediaCacheIdentity::key(&remote);
                        RemoteMediaDiskCache::shared().existing_local_url(
                            &remote,
                            Some(&cache_key),
                            user_id.as_deref(),
                            epoch.as_deref(),
                        )
                    })
                    .await
            }
            None => None,
        };
        let lease = RemoteMediaDiskCache::lease(cached_path.as_deref());
        let resolved = match (cached_path, local, remote) {
            (Some(cached_path), _, _) => RiseNativeMediaPosterSource::File(cached_path),
            (None, Some(local), _) => RiseNativeMediaPosterSource::File(local),
            (None, None, Some(remote)) => match RiseSecureMediaLocation::resolve(&remote, cx).await {
                Some(location) => RiseNativeMediaPosterSource::Remote(location),
                None => RiseNativeMediaPosterSource::Remote(remote),
            },
            (None, None, None) => return None,
        };
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return None;
        }
        let max_side = (max_pixel_size.max(1.0)) as u32;
        let poster = cx
            .background_executor()
            .spawn(async move {
                let frame = match &resolved {
                    RiseNativeMediaPosterSource::File(path) => {
                        rise_media::video_export::poster(path, Self::POSTER_SECONDS, max_side).ok()?
                    }
                    RiseNativeMediaPosterSource::Remote(url) => {
                        rise_media::video_export::remote_poster(url.absolute_string(), Self::POSTER_SECONDS, max_side).ok()?
                    }
                };
                let mut rgba = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
                for pixel in rgba.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
                Some(CleverImageBitmapDecoder::render_image(rgba))
            })
            .await;
        drop(lease);
        poster
    }

    const POSTER_SECONDS: f64 = 0.05;

    fn is_likely_video_url(raw: &str) -> bool {
        let path_extension = Url::from_string(raw)
            .map(|url| url.path_extension())
            .filter(|extension| !extension.is_empty())
            .unwrap_or_else(|| {
                Path::new(raw).extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_owned()
            });
        ["mp4", "mov", "m4v", "webm"].contains(&path_extension.to_lowercase().as_str())
    }

    fn data_from_local_or_cache(source: &str, account_user_id: Option<&str>) -> Option<Vec<u8>> {
        if source.starts_with("file://") {
            let url = Url::from_string(source)?;
            return std::fs::read(url.to_file_path()?).ok();
        }
        if is_local_absolute_path(source) {
            return std::fs::read(source).ok();
        }
        CleverImageDiskCache::image_data(source, account_user_id, None)
    }

    fn downsample(data: &[u8], max_pixel_size: f32) -> Option<CleverImage> {
        CleverImageBitmapDecoder::decode(data, Some(max_pixel_size as i64))
    }

    fn jpeg_data(image: &RenderImage, quality: u8) -> Option<Vec<u8>> {
        let frame = image.size(0);
        let width = u32::try_from(frame.width.0).ok()?;
        let height = u32::try_from(frame.height.0).ok()?;
        let mut bytes = image.as_bytes(0)?.to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let rgb = image::DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, bytes)?).into_rgb8();
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, quality).encode_image(&rgb).ok()?;
        Some(data)
    }
}

/// what the poster opens, a file on disk or the resolved remote location
enum RiseNativeMediaPosterSource {
    File(PathBuf),
    Remote(Url),
}

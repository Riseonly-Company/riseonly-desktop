use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Bounds, Context, Div, ElementId, Entity, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce,
    SharedString, Size, Styled, Subscription, Window, black, div, linear_color_stop, linear_gradient, px, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::message_bubble_geometry::{MessageBubbleRadii, MessageBubbleTailSide};
use rise_ui::message_media_tail_presentation::MessageMediaTailGeometry;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::theme;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::media::rise_media_transfer_hub::{
    RiseMediaTransferHub, RiseMediaTransferKey, RiseMediaTransferRequest, RiseMediaTransferRequestArgs,
};
use crate::core::media_viewer::media_viewer_presentation_state::{MediaViewerPresentationState, chat_media_presentation};
use crate::core::media_viewer::media_viewer_types::MediaViewerFrameKeys;
use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::rise_media_transfer_chip::{RiseMediaTransferChip, RiseMediaTransferChipViewMetrics};
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_corners::BubbleCorners;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::media_letterbox_photo::MediaLetterboxPhoto;
use crate::modules::chat::components::chat::bubble::message_image_grid_layout::{
    MessageGridCell, MessageGridDownloadSeed, MessageGridLayout, MessageGridLayoutEngine,
};
use crate::modules::chat::components::chat::bubble::message_media_upload_overlay::MessageMediaUploadOverlay;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy, ChatLoopingVideoView, ChatWebMVideoStack,
};
use crate::modules::chat::components::chat::bubble::rise_native_message_image_grid::{
    RiseNativeMessageImageGrid, RiseNativeMessageImageGridCancelUpload, RiseNativeMessageImageGridTap,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind};

/// tap callback of a tile
/// gets tile index and frame in window coords, the tile and whether the grid is single
pub type MessageImageGridViewTap = Rc<dyn Fn(usize, Bounds<Pixels>, &MessageGridCell, bool, &mut Window, &mut App)>;

pub type MessageImageGridViewCancelUpload = Rc<dyn Fn(&MessageMediaItem, &mut Window, &mut App)>;

/// what a body reads from the chat media presentation
#[derive(Clone, Debug, Default, PartialEq)]
struct MessageImageGridPresentation {
    is_presented: bool,
    hero_lift_id: Option<String>,
    bubble_cell_hidden: bool,
}

impl MessageImageGridPresentation {
    fn of(presentation: &MediaViewerPresentationState) -> Self {
        Self {
            is_presented: presentation.is_presented(),
            hero_lift_id: presentation.hero_lift_id().map(str::to_owned),
            bubble_cell_hidden: presentation.bubble_cell_hidden(),
        }
    }

    /// what this grid draws from the state
    /// flags count only with a hero id of one of its own tiles so a hero elsewhere draws as no presentation
    fn projected(&self, lift_ids: &[String]) -> Self {
        match self.hero_lift_id.as_deref() {
            Some(hero) if lift_ids.iter().any(|id| id == hero) => self.clone(),
            _ => Self::default(),
        }
    }

    fn is_lifted(&self, lift_id: &str) -> bool {
        self.is_presented && self.hero_lift_id.as_deref() == Some(lift_id) && self.bubble_cell_hidden
    }
}

struct MessageImageGridPresentationObservation {
    presentation: MessageImageGridPresentation,
    lift_ids: Vec<String>,
    _subscription: Subscription,
}

impl MessageImageGridPresentationObservation {
    fn new(state: &Entity<MediaViewerPresentationState>, lift_ids: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self {
            presentation: MessageImageGridPresentation::of(state.read(cx)),
            lift_ids,
            _subscription: cx.observe(state, |this, state, cx| {
                let presentation = MessageImageGridPresentation::of(state.read(cx));
                let changed = presentation.projected(&this.lift_ids) != this.presentation.projected(&this.lift_ids);
                this.presentation = presentation;
                if changed {
                    cx.notify();
                }
            }),
        }
    }
}

#[derive(IntoElement)]
pub struct MessageImageGridView {
    id: ElementId,
    pub owner_local_id: String,
    pub items: Vec<MessageMediaItem>,
    pub max_width: f32,
    pub max_height: f32,
    pub corners: BubbleCorners,
    pub meta: Option<AnyElement>,
    pub sender_header: Option<AnyElement>,
    pub on_tap: Option<MessageImageGridViewTap>,
    pub on_cancel_upload: Option<MessageImageGridViewCancelUpload>,
}

impl MessageImageGridView {
    pub fn new(
        id: impl Into<ElementId>,
        owner_local_id: impl Into<String>,
        items: Vec<MessageMediaItem>,
        max_width: f32,
        max_height: f32,
    ) -> Self {
        Self {
            id: id.into(),
            owner_local_id: owner_local_id.into(),
            items,
            max_width,
            max_height,
            corners: BubbleCorners::uniform(px(MessageSizes::bubble_corner_radius())),
            meta: None,
            sender_header: None,
            on_tap: None,
            on_cancel_upload: None,
        }
    }

    pub fn corners(mut self, corners: BubbleCorners) -> Self {
        self.corners = corners;
        self
    }

    pub fn meta(mut self, meta: Option<AnyElement>) -> Self {
        self.meta = meta;
        self
    }

    pub fn sender_header(mut self, sender_header: Option<AnyElement>) -> Self {
        self.sender_header = sender_header;
        self
    }

    pub fn on_tap(mut self, on_tap: MessageImageGridViewTap) -> Self {
        self.on_tap = Some(on_tap);
        self
    }

    pub fn on_cancel_upload(mut self, on_cancel_upload: MessageImageGridViewCancelUpload) -> Self {
        self.on_cancel_upload = Some(on_cancel_upload);
        self
    }

    fn child_id(&self, name: impl Into<String>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name.into()))
    }

    fn visible_items(&self) -> Vec<MessageMediaItem> {
        self.items.iter().filter(|item| !item.is_cancelled).cloned().collect()
    }

    /// width the grid lays itself out at, so a column around it can be sized before layout
    pub fn layout_width(&self) -> f32 {
        let visible_items = self.visible_items();
        let layout = MessageGridLayoutEngine::compute(
            &visible_items,
            self.max_width,
            self.max_height,
            MessageSizes::media_mosaic_separator_width(),
            Some(self.owner_local_id.as_str()),
        );
        f32::from(layout.container_size.width)
    }

    fn uses_native_fast_path(visible_items: &[MessageMediaItem]) -> bool {
        !visible_items.is_empty()
            && visible_items.iter().all(|item| item.kind == MessageMediaKind::Image || item.kind == MessageMediaKind::Video)
    }

    fn source_radii(&self, cell: &MessageGridCell, layout: &MessageGridLayout) -> MessageBubbleRadii {
        let tolerance = px(1.0);
        let is_leading = cell.cell_frame.left() <= tolerance;
        let is_trailing = cell.cell_frame.right() >= layout.container_size.width - tolerance;
        let is_top = cell.cell_frame.top() <= tolerance;
        let is_bottom = cell.cell_frame.bottom() >= layout.container_size.height - tolerance;
        MessageBubbleRadii {
            top_leading: if is_leading && is_top { self.corners.top_leading } else { px(0.0) },
            top_trailing: if is_trailing && is_top { self.corners.top_trailing } else { px(0.0) },
            bottom_leading: if is_leading && is_bottom { self.corners.bottom_leading } else { px(0.0) },
            bottom_trailing: if is_trailing && is_bottom { self.corners.bottom_trailing } else { px(0.0) },
        }
    }

    fn animation_url(cell: &MessageGridCell, visible_items: &[MessageMediaItem]) -> String {
        if let Some(item) = visible_items.get(cell.index) {
            if let Some(local) = item.local_preview_url.as_deref().map(trimming_whitespaces_and_newlines) {
                let kind = ChatAnimatedMediaSourcePolicy::playback_kind(local, None, item.codec.as_deref());
                if kind != ChatAnimatedMediaPlaybackKind::Unknown && MessageMediaItem::is_available_local_preview_url(local) {
                    return local.to_owned();
                }
            }
            let full = trimming_whitespaces_and_newlines(&item.url);
            if !full.is_empty() {
                return full.to_owned();
            }
        }
        cell.display_source.clone()
    }

    fn animation_playback_kind(cell: &MessageGridCell, visible_items: &[MessageMediaItem]) -> ChatAnimatedMediaPlaybackKind {
        let source = Self::animation_url(cell, visible_items);
        match visible_items.get(cell.index) {
            Some(item) => {
                ChatAnimatedMediaSourcePolicy::playback_kind(&source, item.mime_type.as_deref(), item.codec.as_deref())
            }
            None => ChatAnimatedMediaSourcePolicy::playback_kind(&source, None, None),
        }
    }

    fn animation_poster_source(cell: &MessageGridCell) -> Option<String> {
        let thumbnail = cell
            .thumbnail_url
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|thumbnail| !thumbnail.is_empty() && Self::is_still_image_source(thumbnail));
        if let Some(thumbnail) = thumbnail {
            return Some(thumbnail.to_owned());
        }
        let source = trimming_whitespaces_and_newlines(&cell.display_source);
        (!source.is_empty() && Self::is_still_image_source(source)).then(|| source.to_owned())
    }

    fn is_still_image_source(source: &str) -> bool {
        let kind = ChatAnimatedMediaSourcePolicy::playback_kind(source, None, None);
        kind != ChatAnimatedMediaPlaybackKind::UnsupportedVideo && kind != ChatAnimatedMediaPlaybackKind::WebMVideo
    }

    fn letterbox(&self, name: &str, source: impl Into<SharedString>, cell: &MessageGridCell) -> MediaLetterboxPhoto {
        MediaLetterboxPhoto::new(self.child_id(format!("{name}-{}", cell.render_id)), source, cell.cell_frame.size)
            .persist_to_disk(true)
            .shows_progress_indicator(false)
    }

    fn animation_content(&self, cell: &MessageGridCell, is_single: bool, visible_items: &[MessageMediaItem]) -> AnyElement {
        let render_id = &cell.render_id;
        let gravity =
            if is_single { AVLayerVideoGravity::ResizeAspect } else { AVLayerVideoGravity::ResizeAspectFill };
        let url = Self::animation_url(cell, visible_items);
        match Self::animation_playback_kind(cell, visible_items) {
            ChatAnimatedMediaPlaybackKind::AvFoundationVideo => div()
                .relative()
                .size_full()
                .child(self.letterbox("still", cell.display_source.clone(), cell))
                .child(
                    div().absolute().inset_0().child(
                        ChatLoopingVideoView::new(self.child_id(format!("loop-{render_id}")), url).video_gravity(gravity),
                    ),
                )
                .into_any_element(),
            ChatAnimatedMediaPlaybackKind::WebMVideo => {
                let stack = ChatWebMVideoStack::new(self.child_id(format!("webm-{render_id}")), url).video_gravity(gravity);
                match Self::animation_poster_source(cell) {
                    Some(poster) => stack.poster(self.letterbox("poster", poster, cell)).into_any_element(),
                    None => stack.into_any_element(),
                }
            }
            ChatAnimatedMediaPlaybackKind::UnsupportedVideo => div()
                .size_full()
                .children(Self::animation_poster_source(cell).map(|poster| self.letterbox("poster", poster, cell)))
                .into_any_element(),
            _ if is_single => div()
                .relative()
                .size_full()
                .child(self.letterbox("still", cell.display_source.clone(), cell))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(
                            AnimatedGifView::new(self.child_id(format!("gif-{render_id}")), url)
                                .content_mode(AnimatedGifContentMode::ScaleAspectFit),
                        ),
                )
                .into_any_element(),
            _ => AnimatedGifView::new(self.child_id(format!("gif-{render_id}")), url)
                .content_mode(AnimatedGifContentMode::ScaleAspectFill)
                .into_any_element(),
        }
    }

    fn cell_content(&self, cell: &MessageGridCell, is_single: bool, visible_items: &[MessageMediaItem]) -> AnyElement {
        if cell.kind == MessageMediaKind::Animation || cell.kind == MessageMediaKind::Gif {
            return self.animation_content(cell, is_single, visible_items);
        }
        if is_single {
            return self.letterbox("photo", cell.display_source.clone(), cell).into_any_element();
        }
        let longest_side = f32::from(cell.cell_frame.size.width).max(f32::from(cell.cell_frame.size.height));
        let decode_side = ((longest_side * 3.0).ceil() as i64).clamp(64, 1_024);
        CleverImageUi::new(cell.display_source.clone())
            .id(self.child_id(format!("image-{}", cell.render_id)))
            .content_mode(ContentMode::Fill)
            .corner_radius(0.0)
            .persist_to_disk(true)
            .shows_progress_indicator(false)
            .maximum_decode_pixel_dimension(decode_side)
            .into_any_element()
    }

    fn cell_container(
        &self,
        cell: &MessageGridCell,
        is_single: bool,
        visible_items: &[MessageMediaItem],
        presentation: &MessageImageGridPresentation,
        is_layout_measurement: bool,
    ) -> AnyElement {
        let lift_id = MediaViewerFrameKeys::chat_bubble(&cell.render_id);
        let is_lifted = presentation.is_lifted(&lift_id);
        let cell_size = cell.cell_frame.size;
        let uploading_item = visible_items.get(cell.index).filter(|item| item.shows_upload_overlay()).cloned();
        let is_animated_kind = matches!(cell.kind, MessageMediaKind::Video | MessageMediaKind::Animation | MessageMediaKind::Gif);
        let video_overlay = (is_animated_kind && uploading_item.is_none())
            .then(|| VideoCellOverlay { duration: cell.duration, kind: cell.kind, cell_size, badge_top: self.cell_badge_top(cell) });

        let mut container = div()
            .relative()
            .w(cell_size.width)
            .h(cell_size.height)
            .bg(black())
            .opacity(if is_lifted { 0.0 } else { 1.0 })
            .child(div().size_full().overflow_hidden().child(self.cell_content(cell, is_single, visible_items)))
            .children(video_overlay);

        if let Some(uploading_item) = uploading_item {
            let mut overlay = MessageMediaUploadOverlay::new(
                self.child_id(format!("upload-{}", cell.render_id)),
                uploading_item.clone(),
            )
            .cell_size(cell_size);
            if let Some(on_cancel_upload) = self.on_cancel_upload.clone().filter(|_| !is_lifted) {
                overlay = overlay.on_cancel(move |window: &mut Window, cx: &mut App| {
                    on_cancel_upload(&uploading_item, window, cx);
                });
            }
            container = container.child(div().absolute().inset_0().child(overlay));
        } else if let Some(seed) = cell.download_seed.clone() {
            container = container.child(self.transfer_chip(seed, cell, is_lifted, is_layout_measurement));
        }
        container.into_any_element()
    }

    fn cell_badge_top(&self, cell: &MessageGridCell) -> f32 {
        if self.sender_header.is_some() && f32::from(cell.cell_frame.top()) <= 1.0 { 33.0 } else { 6.0 }
    }

    fn transfer_chip(
        &self,
        seed: MessageGridDownloadSeed,
        cell: &MessageGridCell,
        is_lifted: bool,
        is_layout_measurement: bool,
    ) -> Div {
        let width = f32::from(cell.cell_frame.size.width);
        let side = width.min(f32::from(cell.cell_frame.size.height));
        let metrics = RiseMediaTransferChipViewMetrics::fitting(side);
        let (height, ring_side, leading_inset) = (metrics.height, metrics.ring_side, metrics.leading_inset);
        let chip_top = if self.sender_header.is_some()
            && f32::from(cell.cell_frame.left()) <= 1.0
            && f32::from(cell.cell_frame.top()) <= 1.0
        {
            33.0
        } else {
            8.0
        };
        let chip = RiseMediaTransferChip::new(
            self.child_id(format!("chip-{}", cell.render_id)),
            Some(RiseMediaTransferKey::for_active_account(seed.cache_key.clone())),
        )
        .declared_byte_count(seed.declared_byte_count)
        .metrics(metrics)
        .shows_byte_counts(width >= 112.0)
        .shows_idle_affordance(seed.shows_idle_affordance)
        .is_interactive(seed.shows_idle_affordance)
        .is_layout_measurement(is_layout_measurement)
        .fills_sized_box(true);

        let tap_target = (seed.shows_idle_affordance && !is_lifted).then(|| {
            let tap_id = self.child_id(format!("chip-tap-{}", cell.render_id));
            let seed = seed.clone();
            let tap_width = px(ring_side + leading_inset * 2.0 + 14.0);
            div().absolute().top_0().left_0().w(tap_width).h(px(height)).child(
                div().w(tap_width).h(px(height)).bubble_tap_target_priority(
                    tap_id,
                    move |_window: &mut Window, cx: &mut App| {
                        Self::toggle_transfer(&seed, cx);
                    },
                ),
            )
        });

        div()
            .absolute()
            .top(px(chip_top))
            .left(px(8.0))
            .w(px((width - 16.0).max(0.0)))
            .h(px(height))
            .child(chip)
            .children(tap_target)
    }

    fn toggle_transfer(seed: &MessageGridDownloadSeed, cx: &mut App) {
        let Some(remote) = Url::from_string(&seed.remote_url) else {
            return;
        };
        let suggested_filename = remote.last_path_component();
        let request = RiseMediaTransferRequest::new(
            remote,
            RiseMediaTransferRequestArgs {
                cache_key: Some(seed.cache_key.clone()),
                suggested_filename: Some(suggested_filename),
                declared_byte_count: seed.declared_byte_count,
                is_streamable: seed.is_streamable,
                ..RiseMediaTransferRequestArgs::default()
            },
        );
        RiseMediaTransferHub::shared().toggle(request, cx);
    }

    fn tiles(
        &self,
        layout: &MessageGridLayout,
        visible_items: &[MessageMediaItem],
        presentation: &MessageImageGridPresentation,
        is_layout_measurement: bool,
    ) -> Vec<AnyElement> {
        layout
            .cells
            .iter()
            .map(|cell| {
                let registry_id = MediaViewerFrameKeys::chat_bubble(&cell.render_id);
                let tracker =
                    WindowScreenFrameReader::new(registry_id.clone()).source_media_radii(Some(self.source_radii(cell, layout)));
                let on_tap = self.on_tap.clone();
                let tapped = cell.clone();
                let is_single = layout.is_single;
                let tap_registry_id = registry_id.clone();
                div()
                    .absolute()
                    .left(cell.cell_frame.left())
                    .top(cell.cell_frame.top())
                    .w(cell.cell_frame.size.width)
                    .h(cell.cell_frame.size.height)
                    .child(tracker)
                    .child(self.cell_container(cell, is_single, visible_items, presentation, is_layout_measurement))
                    .media_bubble_tap_target(
                        self.child_id(format!("tile-{}", cell.render_id)),
                        registry_id,
                        move |window: &mut Window, cx: &mut App| {
                            let frame = MediaCellScreenFrameRegistry::frame(&tap_registry_id).unwrap_or_default();
                            if let Some(on_tap) = on_tap.as_ref() {
                                on_tap(tapped.index, frame, &tapped, is_single, window, cx);
                            }
                        },
                    )
                    .into_any_element()
            })
            .collect()
    }
}

fn rounded_corners(element: Div, corners: &BubbleCorners) -> Div {
    let radii = corners.uneven_shape(LayoutDirection::current());
    element
        .rounded_tl(radii.top_left)
        .rounded_tr(radii.top_right)
        .rounded_bl(radii.bottom_left)
        .rounded_br(radii.bottom_right)
}

impl RenderOnce for MessageImageGridView {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let visible_items = self.visible_items();
        let layout = MessageGridLayoutEngine::compute(
            &visible_items,
            self.max_width,
            self.max_height,
            MessageSizes::media_mosaic_separator_width(),
            Some(self.owner_local_id.as_str()),
        );
        let is_layout_measurement = is_message_layout_measurement(cx);
        let state = chat_media_presentation(cx);
        let observed_state = state.clone();
        let lift_ids: Vec<String> =
            layout.cells.iter().map(|cell| MediaViewerFrameKeys::chat_bubble(&cell.render_id)).collect();
        let initial_lift_ids = lift_ids.clone();
        let observation = window.use_keyed_state(self.child_id("presentation"), cx, move |_, cx| {
            MessageImageGridPresentationObservation::new(&observed_state, initial_lift_ids, cx)
        });
        if observation.read(cx).lift_ids != lift_ids {
            observation.update(cx, |observation, _| observation.lift_ids = lift_ids.clone());
        }
        let presentation = observation.read(cx).presentation.projected(&lift_ids);

        let current_theme = theme(cx);
        let separator_color = current_theme.primary._100;
        let placeholder_color = current_theme.bg._300;
        let media_radii = MessageBubbleRadii {
            top_leading: self.corners.top_leading,
            top_trailing: self.corners.top_trailing,
            bottom_leading: self.corners.bottom_leading,
            bottom_trailing: self.corners.bottom_trailing,
        };
        let uses_native_fast_path = Self::uses_native_fast_path(&visible_items);
        let media_tail_side = if uses_native_fast_path && !is_layout_measurement {
            MessageMediaTailGeometry::eligible_side(
                self.corners.draws_tail,
                self.corners.tail_side,
                self.corners.bottom_leading,
                self.corners.bottom_trailing,
            )
        } else {
            MessageBubbleTailSide::None
        };
        let container: Size<Pixels> = layout.container_size;
        let has_sender_header = self.sender_header.is_some();

        let content: AnyElement = if is_layout_measurement {
            rounded_corners(div().size_full().bg(black()), &self.corners).into_any_element()
        } else if uses_native_fast_path {
            let on_tap: Option<RiseNativeMessageImageGridTap> = self.on_tap.clone().map(|on_tap| -> RiseNativeMessageImageGridTap {
                Rc::new(
                    move |index: usize,
                          frame: Bounds<Pixels>,
                          cell: MessageGridCell,
                          is_single: bool,
                          window: &mut Window,
                          cx: &mut App| { on_tap(index, frame, &cell, is_single, window, cx) },
                )
            });
            let items = self.items.clone();
            let on_cancel_upload = self.on_cancel_upload.clone();
            let on_cancel_upload_slot: RiseNativeMessageImageGridCancelUpload =
                Rc::new(move |slot_index: i64, window: &mut Window, cx: &mut App| {
                    let Some(item) = items.iter().find(|item| item.local_slot_index == slot_index) else {
                        return;
                    };
                    if let Some(on_cancel_upload) = on_cancel_upload.as_ref() {
                        on_cancel_upload(item, window, cx);
                    }
                });
            RiseNativeMessageImageGrid::new(
                self.child_id("native"),
                self.owner_local_id.clone(),
                layout.clone(),
                presentation.is_presented,
                presentation.hero_lift_id.clone(),
                presentation.bubble_cell_hidden,
                is_layout_measurement,
                media_radii,
                media_tail_side,
                separator_color,
                placeholder_color,
                has_sender_header,
                on_tap,
                Some(on_cancel_upload_slot),
            )
            .into_any_element()
        } else {
            rounded_corners(div().relative().size_full().bg(separator_color).overflow_hidden(), &self.corners)
                .children(self.tiles(&layout, &visible_items, &presentation, is_layout_measurement))
                .into_any_element()
        };

        let sender_header = self.sender_header.take();
        let meta = self.meta.take();
        let radii = self.corners.uneven_shape(LayoutDirection::current());
        let overlay = div()
            .absolute()
            .top_0()
            .left_0()
            .w(container.width)
            .h(container.height)
            .when_some(sender_header, |this, sender_header| this.child(div().absolute().inset_0().child(sender_header)))
            .when_some(meta, |this, meta| {
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(64.0))
                        .rounded_bl(radii.bottom_left)
                        .rounded_br(radii.bottom_right)
                        .bg(linear_gradient(
                            180.0,
                            linear_color_stop(black().opacity(0.0), 0.5),
                            linear_color_stop(black().opacity(MessageMediaTailGeometry::TERMINAL_OVERLAY_BLACK_OPACITY), 1.0),
                        )),
                )
                .child(div().absolute().right(px(6.0)).bottom(px(6.0)).child(meta))
            });

        div().relative().w(container.width).h(container.height).child(content).child(overlay)
    }
}

#[derive(IntoElement)]
struct VideoCellOverlay {
    duration: Option<f64>,
    kind: MessageMediaKind,
    cell_size: Size<Pixels>,
    badge_top: f32,
}

impl VideoCellOverlay {
    fn play_badge(side: f32, glyph: f32, opacity: f32, nudge: f32) -> Div {
        div()
            .size(px(side))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(black().opacity(opacity))
            .child(div().relative().left(px(nudge)).children(IconUi::sized("play.fill", px(glyph), white(), true)))
    }

    fn duration_badge(duration: f64) -> Div {
        div()
            .px(px(6.0))
            .py(px(2.0))
            .rounded_full()
            .bg(black().opacity(0.55))
            .child(
                MainText::new(RiseFormat::duration_time_interval(duration))
                    .px(11.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .monospaced_digits(true)
                    .color(white()),
            )
    }

    fn gif_badge() -> Div {
        div()
            .px(px(5.0))
            .py(px(2.0))
            .rounded_full()
            .bg(black().opacity(0.55))
            .child(MainText::new(tr("chat_inline_type_gif")).px(9.0).font_weight(FontWeight::BOLD).color(white()))
    }
}

impl RenderOnce for VideoCellOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let is_animation = matches!(self.kind, MessageMediaKind::Animation | MessageMediaKind::Gif);
        let play = (self.kind == MessageMediaKind::Video).then(|| {
            if f32::from(self.cell_size.width) > 70.0 && f32::from(self.cell_size.height) > 70.0 {
                Self::play_badge(44.0, 18.0, 0.42, 1.0)
            } else {
                Self::play_badge(24.0, 10.0, 0.45, 0.0)
            }
        });
        let badge = if is_animation {
            Some(Self::gif_badge())
        } else {
            self.duration.map(Self::duration_badge)
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .children(play)
            .children(badge.map(|badge| div().absolute().top(px(self.badge_top)).right(px(6.0)).child(badge)))
    }
}

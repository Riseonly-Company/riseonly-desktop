use std::sync::Arc;

use gpui::{
    App, Background, Bounds, ContentMask, IntoElement, Pixels, Point, RenderOnce, Size,
    Styled, TransformationMatrix, Window, canvas, fill, linear_color_stop, linear_gradient, point,
    px, size, Hsla, PathBuilder, SharedString,
};
use rise_i18n::tr;
use rise_ui::icon_ui::IconUi;
use rise_ui::message_bubble_geometry::{MessageBubbleGeometry, MessageBubblePath, MessageBubbleRadii};

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::components::chat::async_render::async_text_bubble_predisplay::{
    AsyncTextBubblePredisplayCache, AsyncTextBubblePredisplayImage, AsyncTextBubblePredisplayKey,
};
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleCorners, AsyncTextBubbleDeliveryState, AsyncTextBubbleFill, AsyncTextBubbleLine,
    AsyncTextBubblePreparedLayout, AsyncTextBubbleSeed, AsyncTextBubbleVisualStyle,
};
use crate::modules::chat::components::chat::bubble::message_meta::MessageDeliveryStatusGeometry;
use crate::modules::chat::shared::chat_search_highlight::ChatSearchHighlightPalette;

/// bubble drawing of one text message
/// gpui paints only what is on screen each frame so the node keeps the predisplayed contour
/// when the cache has it
#[derive(Default)]
pub struct AsyncTextBubbleNode {
    prepared_layout: Option<Arc<AsyncTextBubblePreparedLayout>>,
    bubble_style: Option<AsyncTextBubbleVisualStyle>,
    corners: Option<AsyncTextBubbleCorners>,
    predisplay_key: Option<AsyncTextBubblePredisplayKey>,
    contents: Option<Arc<AsyncTextBubblePredisplayImage>>,
    slice: Option<Bounds<Pixels>>,
}

impl AsyncTextBubbleNode {
    pub fn new() -> Self {
        Self::default()
    }

    /// scale is the backing scale of the window that shows the bubble
    pub fn update(
        &mut self,
        prepared_layout: &Arc<AsyncTextBubblePreparedLayout>,
        style: &AsyncTextBubbleVisualStyle,
        corners: &AsyncTextBubbleCorners,
        slice: Option<Bounds<Pixels>>,
        scale: f32,
    ) {
        let predisplayed_image = if slice.is_none() {
            AsyncTextBubblePredisplayCache::shared().image(prepared_layout, style, corners, scale)
        } else {
            None
        };
        let same_layout = self
            .prepared_layout
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, prepared_layout));
        let changed = !same_layout
            || self.bubble_style.as_ref() != Some(style)
            || self.corners.as_ref() != Some(corners)
            || self.slice != slice
            || predisplayed_image
                .as_ref()
                .is_some_and(|image| self.predisplay_key.as_ref() != Some(&image.key));
        if !changed {
            return;
        }

        self.prepared_layout = Some(prepared_layout.clone());
        self.bubble_style = Some(style.clone());
        self.corners = Some(*corners);
        self.slice = slice;
        self.predisplay_key = predisplayed_image.as_ref().map(|image| image.key.clone());
        self.contents = predisplayed_image;
    }

    pub fn cancel_drawing(&mut self) {
        self.contents = None;
        self.predisplay_key = None;
    }

    /// paints the bubble with its top left at origin
    /// a slice limits the paint to that strip of the bubble
    pub fn paint(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let (Some(prepared_layout), Some(style), Some(corners)) =
            (&self.prepared_layout, &self.bubble_style, &self.corners)
        else {
            return;
        };
        let bounds = Bounds::new(origin, prepared_layout.size);
        let bubble_path = self.contents.as_ref().map(|image| &image.bubble_path);
        match self.slice {
            Some(slice) => {
                let mask = ContentMask {
                    bounds: Bounds::new(origin + slice.origin, slice.size),
                };
                window.with_content_mask(Some(mask), |window| {
                    Self::draw_bubble(bounds, prepared_layout, style, corners, bubble_path, window, cx);
                });
            }
            None => Self::draw_bubble(bounds, prepared_layout, style, corners, bubble_path, window, cx),
        }
    }

    /// bubble contour in bubble coordinates
    /// predisplay resolves it off the main thread and paint moves it to the bubble origin
    pub fn bubble_path(size: Size<Pixels>, corners: &AsyncTextBubbleCorners) -> MessageBubblePath {
        Self::rounded_path(Bounds::new(Point::default(), size), corners)
    }

    pub fn draw_bubble(
        bounds: Bounds<Pixels>,
        prepared_layout: &AsyncTextBubblePreparedLayout,
        style: &AsyncTextBubbleVisualStyle,
        corners: &AsyncTextBubbleCorners,
        bubble_path: Option<&MessageBubblePath>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let local_path = match bubble_path {
            Some(path) => path.clone(),
            None => Self::bubble_path(bounds.size, corners),
        };
        let path = local_path.scaled_and_translated(1.0, 1.0, bounds.origin.x, bounds.origin.y);
        Self::draw_fill(&style.fill, &path, bounds, window);

        Self::draw_text(prepared_layout, style, bounds, window);

        if let Some(views_icon_frame) = prepared_layout.views_icon_frame {
            Self::draw_views_icon(offset(views_icon_frame, bounds.origin), style.time_color, window, cx);
        }

        if let Some(status_frame) = prepared_layout.status_frame {
            Self::draw_status(
                prepared_layout.seed.delivery_state,
                offset(status_frame, bounds.origin),
                style,
                window,
            );
        }
    }

    pub fn accessibility_value(seed: &AsyncTextBubbleSeed) -> String {
        let time = RiseFormat::clock_in_time_zone(
            &seed.timestamp,
            &seed.time_zone_identifier,
            seed.twelve_hour_clock,
        );
        let prefix = seed
            .view_count_text
            .as_ref()
            .map(|views| format!("{views}, "))
            .unwrap_or_default();
        let status = match seed.delivery_state {
            AsyncTextBubbleDeliveryState::None => return prefix + &time,
            AsyncTextBubbleDeliveryState::Sending => tr("chat_message_status_sending"),
            AsyncTextBubbleDeliveryState::Sent => tr("chat_message_status_sent"),
            AsyncTextBubbleDeliveryState::Delivered => tr("chat_message_status_delivered"),
            AsyncTextBubbleDeliveryState::Read => tr("chat_message_status_read"),
            AsyncTextBubbleDeliveryState::Failed => tr("chat_message_status_failed"),
        };
        format!("{prefix}{time}, {status}")
    }

    fn draw_fill(
        fill_style: &AsyncTextBubbleFill,
        path: &MessageBubblePath,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) {
        let Some(fill_path) = path.fill_path() else { return };
        let background: Background = match fill_style {
            AsyncTextBubbleFill::Solid(color) => (*color).into(),
            AsyncTextBubbleFill::LinearGradient {
                start,
                end,
                start_point,
                end_point,
            } => {
                let dx = (f32::from(end_point.x) - f32::from(start_point.x)) * f32::from(bounds.size.width);
                let dy = (f32::from(end_point.y) - f32::from(start_point.y)) * f32::from(bounds.size.height);
                let angle = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
                linear_gradient(angle, linear_color_stop(*start, 0.0), linear_color_stop(*end, 1.0))
            }
        };
        window.paint_path(fill_path, background);
    }

    fn draw_text(
        prepared_layout: &AsyncTextBubblePreparedLayout,
        style: &AsyncTextBubbleVisualStyle,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) {
        for rect in prepared_layout.highlight_rects.iter() {
            window.paint_quad(
                fill(offset(*rect, bounds.origin), ChatSearchHighlightPalette::MATCH_FILL)
                    .corner_radii(ChatSearchHighlightPalette::CORNER_RADIUS),
            );
        }

        for placement in prepared_layout.text_lines.iter() {
            draw_line(
                &placement.line,
                bounds.origin + placement.baseline_origin,
                style.text_color,
                window,
            );
        }

        if let Some(views_line) = &prepared_layout.views_line {
            draw_line(
                views_line,
                bounds.origin + prepared_layout.views_baseline_origin,
                style.time_color,
                window,
            );
        }
        draw_line(
            &prepared_layout.time_line,
            bounds.origin + prepared_layout.time_baseline_origin,
            style.time_color,
            window,
        );
    }

    fn draw_views_icon(frame: Bounds<Pixels>, color: Hsla, window: &mut Window, cx: &mut App) {
        let Some(path) = AsyncTextBubbleMetaIconCache::eye() else { return };
        let _ = window.paint_svg(frame, path, None, TransformationMatrix::unit(), color, cx);
    }

    fn draw_status(
        state: AsyncTextBubbleDeliveryState,
        frame: Bounds<Pixels>,
        style: &AsyncTextBubbleVisualStyle,
        window: &mut Window,
    ) {
        match state {
            AsyncTextBubbleDeliveryState::None => {}
            AsyncTextBubbleDeliveryState::Sending => Self::draw_clock(frame, style.status_color, window),
            AsyncTextBubbleDeliveryState::Sent => Self::draw_check(frame, style.status_color, window),
            AsyncTextBubbleDeliveryState::Delivered => {
                Self::draw_double_check(frame, style.status_color, window)
            }
            AsyncTextBubbleDeliveryState::Read => {
                Self::draw_double_check(frame, style.read_status_color, window)
            }
            AsyncTextBubbleDeliveryState::Failed => Self::draw_failure(
                frame,
                style.failure_color,
                style.failure_glyph_color,
                window,
            ),
        }
    }

    fn draw_clock(frame: Bounds<Pixels>, color: Hsla, window: &mut Window) {
        let width = (f32::from(frame.size.width) / 11.0).max(0.8);
        let circle = inset(frame, width * 0.5);
        stroke_ellipse(circle, width, color, window);

        let center = frame.center();
        stroke_polyline(
            &[center, point(center.x, frame.top() + px(width * 3.0))],
            width,
            color,
            window,
        );
        stroke_polyline(
            &[center, point(frame.right() - px(width * 2.5), center.y)],
            width,
            color,
            window,
        );
    }

    fn draw_check(frame: Bounds<Pixels>, color: Hsla, window: &mut Window) {
        let scale_x = f32::from(frame.size.width) / MessageDeliveryStatusGeometry::REFERENCE_CHECK_WIDTH;
        let scale_y = f32::from(frame.size.height) / MessageDeliveryStatusGeometry::REFERENCE_CHECK_HEIGHT;
        let at = |x: f32, y: f32| point(frame.left() + px(x * scale_x), frame.top() + px(y * scale_y));
        stroke_polyline(
            &[at(1.0, 5.0), at(3.951_570_5, 7.951_570_5), at(10.5, 1.0)],
            (0.99 * scale_x).max(0.8),
            color,
            window,
        );
    }

    fn draw_partial_check(frame: Bounds<Pixels>, color: Hsla, window: &mut Window) {
        let scale_x = f32::from(frame.size.width) / MessageDeliveryStatusGeometry::REFERENCE_CHECK_WIDTH;
        let scale_y = f32::from(frame.size.height) / MessageDeliveryStatusGeometry::REFERENCE_CHECK_HEIGHT;
        stroke_polyline(
            &[
                point(frame.left() + px(1.5 * scale_x), frame.top() + px(8.0 * scale_y)),
                point(frame.left() + px(8.0 * scale_x), frame.top() + px(scale_y)),
            ],
            (0.99 * scale_x).max(0.8),
            color,
            window,
        );
    }

    fn draw_double_check(frame: Bounds<Pixels>, color: Hsla, window: &mut Window) {
        let scale = f32::from(frame.size.height) / MessageDeliveryStatusGeometry::REFERENCE_CHECK_HEIGHT;
        let check_size = size(
            px(MessageDeliveryStatusGeometry::REFERENCE_CHECK_WIDTH * scale),
            frame.size.height,
        );
        let full_frame = Bounds::new(frame.origin, check_size);
        let partial_frame = Bounds::new(
            point(frame.left() + px(6.0 * scale), frame.top()),
            check_size,
        );
        Self::draw_check(full_frame, color, window);
        Self::draw_partial_check(partial_frame, color, window);
    }

    fn draw_failure(frame: Bounds<Pixels>, color: Hsla, glyph_color: Hsla, window: &mut Window) {
        let width = f32::from(frame.size.width);
        let height = f32::from(frame.size.height);
        fill_ellipse(frame, color, window);
        let ring_width = (width * 0.11).max(1.0);
        stroke_ellipse(inset(frame, ring_width * 0.5), ring_width, glyph_color, window);
        let center_x = frame.center().x;
        stroke_polyline(
            &[
                point(center_x, frame.top() + px(height * 0.22)),
                point(center_x, frame.top() + px(height * 0.61)),
            ],
            (width * 0.13).max(1.0),
            glyph_color,
            window,
        );
        let dot_size = (width * 0.13).max(1.0);
        fill_ellipse(
            Bounds::new(
                point(center_x - px(dot_size * 0.5), frame.top() + px(height * 0.73)),
                size(px(dot_size), px(dot_size)),
            ),
            glyph_color,
            window,
        );
    }

    fn rounded_path(rect: Bounds<Pixels>, corners: &AsyncTextBubbleCorners) -> MessageBubblePath {
        MessageBubbleGeometry::path(
            rect,
            MessageBubbleRadii {
                top_leading: corners.top_left,
                top_trailing: corners.top_right,
                bottom_leading: corners.bottom_left,
                bottom_trailing: corners.bottom_right,
            },
            corners.tail_side,
            corners.draws_tail,
        )
    }
}

fn offset(rect: Bounds<Pixels>, origin: Point<Pixels>) -> Bounds<Pixels> {
    Bounds::new(origin + rect.origin, rect.size)
}

fn inset(rect: Bounds<Pixels>, amount: f32) -> Bounds<Pixels> {
    let amount = px(amount);
    Bounds::new(
        point(rect.left() + amount, rect.top() + amount),
        size(
            (rect.size.width - amount * 2.0).max(px(0.0)),
            (rect.size.height - amount * 2.0).max(px(0.0)),
        ),
    )
}

/// draws the line in the context fill color, shaped for this window and every glyph goes out
/// in one color
fn draw_line(line: &AsyncTextBubbleLine, baseline_origin: Point<Pixels>, color: Hsla, window: &mut Window) {
    let line = line.shape(color, window);
    for run in &line.runs {
        for glyph in &run.glyphs {
            let origin = point(
                baseline_origin.x + glyph.position.x,
                baseline_origin.y + glyph.position.y,
            );
            let _ = if glyph.is_emoji {
                window.paint_emoji(origin, run.font_id, glyph.id, line.font_size)
            } else {
                window.paint_glyph(origin, run.font_id, glyph.id, line.font_size, color)
            };
        }
    }
}

/// round caps and joins, gpui strokes end flat so every vertex gets a dot of the line width
fn stroke_polyline(points: &[Point<Pixels>], line_width: f32, color: Hsla, window: &mut Window) {
    if points.len() < 2 {
        return;
    }
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(points, false);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
    for vertex in points {
        let half = line_width * 0.5;
        fill_ellipse(
            Bounds::new(
                point(vertex.x - px(half), vertex.y - px(half)),
                size(px(line_width), px(line_width)),
            ),
            color,
            window,
        );
    }
}

fn stroke_ellipse(rect: Bounds<Pixels>, line_width: f32, color: Hsla, window: &mut Window) {
    let radii = point(rect.size.width * 0.5, rect.size.height * 0.5);
    let center = rect.center();
    let left = point(rect.left(), center.y);
    let right = point(rect.right(), center.y);
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.move_to(left);
    builder.arc_to(radii, px(0.0), false, true, right);
    builder.arc_to(radii, px(0.0), false, true, left);
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn fill_ellipse(rect: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let radius = rect.size.width.min(rect.size.height) * 0.5;
    window.paint_quad(fill(rect, color).corner_radii(radius));
}

enum AsyncTextBubbleMetaIconCache {}

impl AsyncTextBubbleMetaIconCache {
    fn eye() -> Option<SharedString> {
        static EYE: std::sync::LazyLock<Option<SharedString>> =
            std::sync::LazyLock::new(|| IconUi::asset_path("eye"));
        EYE.clone()
    }
}

#[derive(Default)]
pub struct AsyncTextBubbleHostView {
    bubble_node: AsyncTextBubbleNode,
    prepared_layout: Option<Arc<AsyncTextBubblePreparedLayout>>,
    style: Option<AsyncTextBubbleVisualStyle>,
    corners: Option<AsyncTextBubbleCorners>,
}

impl AsyncTextBubbleHostView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(
        &mut self,
        prepared_layout: &Arc<AsyncTextBubblePreparedLayout>,
        style: &AsyncTextBubbleVisualStyle,
        corners: &AsyncTextBubbleCorners,
        window: &Window,
    ) {
        let changes_message = self
            .prepared_layout
            .as_ref()
            .is_none_or(|current| current.seed.id != prepared_layout.seed.id);
        self.prepared_layout = Some(prepared_layout.clone());
        self.style = Some(style.clone());
        self.corners = Some(*corners);
        if changes_message {
            self.bubble_node.cancel_drawing();
        }
        self.bubble_node
            .update(prepared_layout, style, corners, None, window.scale_factor());
    }

    pub fn intrinsic_content_size(&self) -> Size<Pixels> {
        self.prepared_layout
            .as_ref()
            .map(|layout| layout.size)
            .unwrap_or_default()
    }

    pub fn size_that_fits(&self, _size: Size<Pixels>) -> Size<Pixels> {
        self.intrinsic_content_size()
    }

    pub fn paint(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
        self.bubble_node.paint(origin, window, cx);
    }
}

/// fixed size element that hosts the bubble at the prepared size
#[derive(IntoElement)]
pub struct AsyncTextBubbleView {
    pub prepared_layout: Arc<AsyncTextBubblePreparedLayout>,
    pub style: AsyncTextBubbleVisualStyle,
    pub corners: AsyncTextBubbleCorners,
}

impl RenderOnce for AsyncTextBubbleView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bubble_size = self.prepared_layout.size;
        canvas(
            move |_bounds, window, _cx| {
                let mut host = AsyncTextBubbleHostView::new();
                host.update(&self.prepared_layout, &self.style, &self.corners, window);
                host
            },
            |bounds, host, window, cx| host.paint(bounds.origin, window, cx),
        )
        .w(bubble_size.width)
        .h(bubble_size.height)
    }
}

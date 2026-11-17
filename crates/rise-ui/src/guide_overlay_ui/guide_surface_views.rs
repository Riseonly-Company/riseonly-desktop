use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Div, Empty, Entity, FontFeatures, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, PathBuilder, Pixels, Render, RenderOnce,
    SharedString, Styled, Subscription, TextRun, Window, canvas, div, px, white,
};
use rise_core::foundation::foundation_url::Url;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayerActionAtItemEnd, AVPlayerItem, AVPlayerItemNotification,
    AVPlayerLayer, AVPlayerLooper, AVQueuePlayer,
};
use rise_theme::{AppTheme, ThemeStore};
use rise_theme::theme::theme_environment::theme_store;

use crate::button_ui::ButtonUi;
use crate::expandable_text_ui::EdgeInsets;
use crate::guide_overlay_ui::guide_overlay_theme::GuideOverlayTheme;
use crate::main_text::{MainText, ignores_text_size_delta, system_font, text_size};
use crate::message_bubble_geometry::{
    MessageBubbleGeometry, MessageBubblePath, MessageBubbleRadii, MessageBubbleTailSide,
};
use crate::secondary_text::SecondaryText;
use crate::simple_button_ui::SimpleButtonUi;
use crate::swiftui::frame::{EdgeSet, SwiftUiFrame};
use crate::swiftui::measure::measure;
use crate::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke, pass_through};
use crate::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, spacer_min_w, vstack};

pub type GuideOverlayActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// a control a step offers, title arrives already localised
#[derive(Clone)]
pub struct GuideOverlayAction {
    pub title: String,
    pub handler: GuideOverlayActionHandler,
}

impl GuideOverlayAction {
    pub fn new(title: impl Into<String>, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { title: title.into(), handler: Rc::new(handler) }
    }
}

/// words attached to the narrator, canon requires a narrator for this surface
#[derive(Clone)]
pub struct GuideBubbleContent {
    pub body: String,
    pub step_label: Option<String>,
    pub primary: Option<GuideOverlayAction>,
    pub secondary: Option<GuideOverlayAction>,
}

impl GuideBubbleContent {
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into(), step_label: None, primary: None, secondary: None }
    }

    pub fn step_label(mut self, step_label: impl Into<Option<String>>) -> Self {
        self.step_label = step_label.into();
        self
    }

    pub fn primary(mut self, primary: impl Into<Option<GuideOverlayAction>>) -> Self {
        self.primary = primary.into();
        self
    }

    pub fn secondary(mut self, secondary: impl Into<Option<GuideOverlayAction>>) -> Self {
        self.secondary = secondary.into();
        self
    }
}

/// short line beside the anchor with no narrator, the hint tier surface
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideCaptionContent {
    pub body: String,
}

impl GuideCaptionContent {
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }
}

/// the centered card, canon forbids it on a stage that has a cutout
#[derive(Clone)]
pub struct GuideCardContent {
    pub title: Option<String>,
    pub body: String,
    pub step_label: Option<String>,
    pub primary: Option<GuideOverlayAction>,
    pub secondary: Option<GuideOverlayAction>,
}

impl GuideCardContent {
    pub fn new(body: impl Into<String>) -> Self {
        Self { title: None, body: body.into(), step_label: None, primary: None, secondary: None }
    }

    pub fn title(mut self, title: impl Into<Option<String>>) -> Self {
        self.title = title.into();
        self
    }

    pub fn step_label(mut self, step_label: impl Into<Option<String>>) -> Self {
        self.step_label = step_label.into();
        self
    }

    pub fn primary(mut self, primary: impl Into<Option<GuideOverlayAction>>) -> Self {
        self.primary = primary.into();
        self
    }

    pub fn secondary(mut self, secondary: impl Into<Option<GuideOverlayAction>>) -> Self {
        self.secondary = secondary.into();
        self
    }
}

/// fires once the clip has played through
pub type GuideVideoFinishedHandler = Rc<dyn Fn(&mut App)>;

/// silent looping clip, not a player
/// no controls and no scrubber, canon prefetches it so no spinner either
#[derive(Clone)]
pub struct GuideVideoContent {
    pub url: Url,
    pub title: Option<String>,
    pub body: String,
    pub on_finished: Option<GuideVideoFinishedHandler>,
}

impl GuideVideoContent {
    pub fn new(url: Url, body: impl Into<String>) -> Self {
        Self { url, title: None, body: body.into(), on_finished: None }
    }

    pub fn title(mut self, title: impl Into<Option<String>>) -> Self {
        self.title = title.into();
        self
    }

    pub fn on_finished(mut self, on_finished: impl Fn(&mut App) + 'static) -> Self {
        self.on_finished = Some(Rc::new(on_finished));
        self
    }
}

/// the four surfaces canon defines plus the step that says nothing
#[derive(Clone)]
pub enum GuideOverlaySurfaceContent {
    None,
    Bubble(GuideBubbleContent),
    Caption(GuideCaptionContent),
    Card(GuideCardContent),
    Video(GuideVideoContent),
}

impl GuideOverlaySurfaceContent {
    /// true for major surfaces, they take the screen center
    /// canon never pairs them with a cutout so the solver centers them
    pub fn is_major(&self) -> bool {
        match self {
            Self::Card(_) | Self::Video(_) => true,
            Self::None | Self::Bubble(_) | Self::Caption(_) => false,
        }
    }

    pub fn width_limit(&self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Bubble(_) => GuideOverlayTheme::SURFACE_MAX_WIDTH,
            Self::Caption(_) => GuideOverlayTheme::CAPTION_MAX_WIDTH,
            Self::Card(_) | Self::Video(_) => GuideOverlayTheme::CARD_MAX_WIDTH,
        }
    }

    pub fn has_action_row(&self) -> bool {
        match self {
            Self::Bubble(_) | Self::Card(_) => true,
            Self::None | Self::Caption(_) | Self::Video(_) => false,
        }
    }
}

/// physical edge the bubble tail grows out of, not leading or trailing
/// the caller resolves the direction once
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuideBubbleTailSide {
    Top,
    Bottom,
    Left,
    Right,
}

/// renders whichever surface the step carries, reads no store and fetches nothing
#[derive(IntoElement)]
pub struct GuideSurfaceView {
    pub content: GuideOverlaySurfaceContent,
    pub tail: GuideBubbleTailSide,
    pub is_right_to_left: bool,
    pub width: f32,
}

impl GuideSurfaceView {
    pub fn new(
        content: GuideOverlaySurfaceContent,
        tail: GuideBubbleTailSide,
        is_right_to_left: bool,
        width: f32,
    ) -> Self {
        Self { content, tail, is_right_to_left, width }
    }
}

impl RenderOnce for GuideSurfaceView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        match self.content {
            GuideOverlaySurfaceContent::None => Empty.into_any_element(),
            GuideOverlaySurfaceContent::Bubble(bubble) => GuideBubbleSurface {
                content: bubble,
                tail: self.tail,
                is_right_to_left: self.is_right_to_left,
                width: self.width,
            }
            .into_any_element(),
            GuideOverlaySurfaceContent::Caption(caption) => {
                GuideCaptionSurface { content: caption, width: self.width }.into_any_element()
            }
            GuideOverlaySurfaceContent::Card(card) => {
                GuideCardSurface { content: card, width: self.width }.into_any_element()
            }
            GuideOverlaySurfaceContent::Video(video) => {
                GuideVideoSurface { content: video, width: self.width }.into_any_element()
            }
        }
    }
}

/// pads an element, leading and trailing follow the layout direction
fn padded<S: Styled>(element: S, insets: EdgeInsets) -> S {
    element
        .padding_edges(EdgeSet::TOP, insets.top)
        .padding_edges(EdgeSet::LEADING, insets.leading)
        .padding_edges(EdgeSet::BOTTOM, insets.bottom)
        .padding_edges(EdgeSet::TRAILING, insets.trailing)
}

/// widest text line of a plate of this width with these insets
fn text_width(width: f32, insets: EdgeInsets) -> f32 {
    (width - insets.leading - insets.trailing).max(0.0)
}

/// background and stroke overlay for rounded plates
/// stroke is a child so the plate must be relative, occlude makes it eat its own taps
fn rounded_plate(plate: Div, shape: Shape, theme: &AppTheme) -> Div {
    shape
        .round(plate.relative())
        .bg(GuideOverlayTheme::surface_background(theme))
        .occlude()
        .child(centred_stroke(
            GuideOverlayTheme::surface_border(theme),
            GuideOverlayTheme::SURFACE_BORDER_WIDTH,
            shape,
        ))
}

#[derive(IntoElement)]
pub struct GuideBubbleSurface {
    pub content: GuideBubbleContent,
    pub tail: GuideBubbleTailSide,
    pub is_right_to_left: bool,
    pub width: f32,
}

impl GuideBubbleSurface {
    /// padding flips under rtl but the tail is drawn in physical coordinates, reconciled here
    fn padded_edge(&self) -> EdgeSet {
        match self.tail {
            GuideBubbleTailSide::Top => EdgeSet::TOP,
            GuideBubbleTailSide::Bottom => EdgeSet::BOTTOM,
            GuideBubbleTailSide::Left => {
                if self.is_right_to_left {
                    EdgeSet::TRAILING
                } else {
                    EdgeSet::LEADING
                }
            }
            GuideBubbleTailSide::Right => {
                if self.is_right_to_left {
                    EdgeSet::LEADING
                } else {
                    EdgeSet::TRAILING
                }
            }
        }
    }
}

impl RenderOnce for GuideBubbleSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme();
        let shape = GuideBubbleShape {
            corner_radius: GuideOverlayTheme::bubble_corner_radius(theme),
            tail: self.tail,
        };
        let fill = GuideOverlayTheme::surface_background(theme);
        let stroke = GuideOverlayTheme::surface_border(theme);
        let padded_edge = self.padded_edge();

        let mut column = vstack(GuideOverlayTheme::SURFACE_CONTENT_SPACING).align(HorizontalAlignment::Leading);
        if let Some(step_label) = self.content.step_label {
            column = column.child(
                SecondaryText::new(step_label)
                    .px(GuideOverlayTheme::META_SIZE)
                    .color(GuideOverlayTheme::surface_meta_color(theme)),
            );
        }
        column = column
            .child(
                MainText::new(self.content.body)
                    .right_to_left_wrap_width(text_width(self.width, GuideOverlayTheme::SURFACE_PADDING))
                    .px(GuideOverlayTheme::BODY_SIZE)
                    .color(GuideOverlayTheme::surface_body_color(theme))
                    .line_height(GuideOverlayTheme::BODY_LINE_HEIGHT),
            )
            .child(GuideActionRow::new(self.content.primary, self.content.secondary));
        let content = padded(column, GuideOverlayTheme::SURFACE_PADDING).max_w(px(self.width));

        let background = canvas(
            |_, _, _| (),
            move |bounds, (), window, _| shape.paint(bounds, fill, stroke, window),
        )
        .absolute()
        .inset_0();

        div()
            .relative()
            .padding_edges(padded_edge, f32::from(GuideOverlayTheme::BUBBLE_TAIL_SIZE.height))
            .occlude()
            .child(background)
            .child(content)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideBubbleShape {
    pub corner_radius: f32,
    pub tail: GuideBubbleTailSide,
}

impl GuideBubbleShape {
    pub fn path(&self, rect: Bounds<Pixels>) -> MessageBubblePath {
        let side = match self.tail {
            GuideBubbleTailSide::Left => MessageBubbleTailSide::Leading,
            GuideBubbleTailSide::Right => MessageBubbleTailSide::Trailing,
            GuideBubbleTailSide::Top | GuideBubbleTailSide::Bottom => MessageBubbleTailSide::None,
        };

        MessageBubbleGeometry::path(
            rect,
            MessageBubbleRadii::uniform(px(self.corner_radius)),
            side,
            side != MessageBubbleTailSide::None,
        )
    }

    /// paints fill and stroke, the stroke is centered on the same contour as the fill
    fn paint(&self, bounds: Bounds<Pixels>, fill: Hsla, stroke: Hsla, window: &mut Window) {
        let path = self.path(bounds);
        if let Some(filled) = path.fill_path() {
            window.paint_path(filled, fill);
        }
        if path.is_empty() {
            return;
        }
        let mut builder = PathBuilder::stroke(px(GuideOverlayTheme::SURFACE_BORDER_WIDTH));
        path.replay_into(&mut builder);
        if let Ok(outline) = builder.build() {
            window.paint_path(outline, stroke);
        }
    }
}

#[derive(IntoElement)]
pub struct GuideCaptionSurface {
    pub content: GuideCaptionContent,
    pub width: f32,
}

impl RenderOnce for GuideCaptionSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme();
        let shape = Shape::continuous(f32::from(GuideOverlayTheme::caption_corner_radius(theme)));

        let plate = padded(div(), GuideOverlayTheme::CAPTION_PADDING)
            .max_w(px(self.width))
            .child(
                MainText::new(self.content.body)
                    .right_to_left_wrap_width(text_width(self.width, GuideOverlayTheme::CAPTION_PADDING))
                    .px(GuideOverlayTheme::CAPTION_SIZE)
                    .color(GuideOverlayTheme::surface_body_color(theme))
                    .line_height(GuideOverlayTheme::CAPTION_LINE_HEIGHT),
            );
        rounded_plate(plate, shape, theme)
    }
}

#[derive(IntoElement)]
pub struct GuideCardSurface {
    pub content: GuideCardContent,
    pub width: f32,
}

impl RenderOnce for GuideCardSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme();
        let shape = Shape::continuous(f32::from(GuideOverlayTheme::card_corner_radius(theme)));

        let mut column = vstack(GuideOverlayTheme::SURFACE_CONTENT_SPACING).align(HorizontalAlignment::Leading);
        if let Some(step_label) = self.content.step_label {
            column = column.child(
                SecondaryText::new(step_label)
                    .px(GuideOverlayTheme::META_SIZE)
                    .color(GuideOverlayTheme::surface_meta_color(theme)),
            );
        }
        if let Some(title) = self.content.title {
            column = column.child(
                MainText::new(title)
                    .px(GuideOverlayTheme::TITLE_SIZE)
                    .font_weight(GuideOverlayTheme::TITLE_WEIGHT)
                    .color(GuideOverlayTheme::surface_title_color(theme)),
            );
        }
        column = column
            .child(
                MainText::new(self.content.body)
                    .right_to_left_wrap_width(text_width(self.width, GuideOverlayTheme::SURFACE_PADDING))
                    .px(GuideOverlayTheme::BODY_SIZE)
                    .color(GuideOverlayTheme::surface_body_color(theme))
                    .line_height(GuideOverlayTheme::BODY_LINE_HEIGHT),
            )
            .child(GuideActionRow::new(self.content.primary, self.content.secondary));
        let plate = padded(column, GuideOverlayTheme::SURFACE_PADDING).max_w(px(self.width));
        rounded_plate(plate, shape, theme)
    }
}

#[derive(IntoElement)]
pub struct GuideVideoSurface {
    pub content: GuideVideoContent,
    pub width: f32,
}

impl RenderOnce for GuideVideoSurface {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme();
        let shape = Shape::continuous(f32::from(GuideOverlayTheme::card_corner_radius(theme)));
        let clip_shape = Shape::continuous(f32::from(GuideOverlayTheme::video_corner_radius(theme)));
        let clip_width = (self.width
            - GuideOverlayTheme::SURFACE_PADDING.leading
            - GuideOverlayTheme::SURFACE_PADDING.trailing)
            .max(0.0);

        let mut column = vstack(GuideOverlayTheme::SURFACE_CONTENT_SPACING)
            .align(HorizontalAlignment::Leading)
            .child(
                div()
                    .frame_size(clip_width, clip_width / GuideOverlayTheme::VIDEO_ASPECT_RATIO)
                    .clip_shape(clip_shape)
                    .child(GuideLoopingVideo { url: self.content.url, on_finished: self.content.on_finished }),
            );
        if let Some(title) = self.content.title {
            column = column.child(
                MainText::new(title)
                    .px(GuideOverlayTheme::TITLE_SIZE)
                    .font_weight(GuideOverlayTheme::TITLE_WEIGHT)
                    .color(GuideOverlayTheme::surface_title_color(theme)),
            );
        }
        column = column.child(
            MainText::new(self.content.body)
                .right_to_left_wrap_width(clip_width)
                .px(GuideOverlayTheme::BODY_SIZE)
                .color(GuideOverlayTheme::surface_body_color(theme))
                .line_height(GuideOverlayTheme::BODY_LINE_HEIGHT),
        );
        let plate = padded(column, GuideOverlayTheme::SURFACE_PADDING).w(px(self.width));
        rounded_plate(plate, shape, theme)
    }
}

#[derive(IntoElement)]
struct GuideLoopingVideo {
    url: Url,
    on_finished: Option<GuideVideoFinishedHandler>,
}

impl RenderOnce for GuideLoopingVideo {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.use_keyed_state("guide-looping-video", cx, |_, _| GuideLoopingVideoView::new());
        view.update(cx, |view, cx| {
            view.on_finished = self.on_finished;
            view.configure(self.url, cx);
        });
        div().size_full().child(view)
    }
}

pub struct GuideLoopingVideoView {
    pub on_finished: Option<GuideVideoFinishedHandler>,
    queue_player: Option<Entity<AVQueuePlayer>>,
    looper: Option<AVPlayerLooper>,
    loaded_url: Option<Url>,
    end_observer: Option<Subscription>,
    did_report_finish: bool,
}

impl GuideLoopingVideoView {
    pub fn new() -> Self {
        Self {
            on_finished: None,
            queue_player: None,
            looper: None,
            loaded_url: None,
            end_observer: None,
            did_report_finish: false,
        }
    }

    pub fn configure(&mut self, url: Url, cx: &mut Context<Self>) {
        if self.loaded_url.as_ref() == Some(&url) {
            return;
        }
        self.teardown(cx);
        self.loaded_url = Some(url.clone());

        let item = AVPlayerItem::new(url);
        let player = cx.new(|cx| {
            let mut player = AVQueuePlayer::new(item.clone(), cx);
            player.set_is_muted(true);
            player.set_action_at_item_end(AVPlayerActionAtItemEnd::Advance);
            player.set_automatically_waits_to_minimize_stalling(true);
            player
        });

        self.looper = Some(AVPlayerLooper::new(&player, &item, cx));
        self.queue_player = Some(player.clone());

        self.end_observer = Some(cx.subscribe(&player, |this, _, notification: &AVPlayerItemNotification, cx| {
            match notification {
                AVPlayerItemNotification::DidPlayToEndTime { asset_url } => {
                    if this.did_report_finish || this.loaded_url.as_ref() != Some(asset_url) {
                        return;
                    }
                    this.did_report_finish = true;
                    if let Some(on_finished) = this.on_finished.clone() {
                        on_finished(cx);
                    }
                }
            }
        }));

        player.update(cx, |player, cx| player.play(cx));
        cx.notify();
    }

    pub fn teardown(&mut self, cx: &mut Context<Self>) {
        self.end_observer = None;
        if let Some(player) = &self.queue_player {
            player.update(cx, |player, cx| player.pause(cx));
        }
        self.looper = None;
        self.queue_player = None;
        self.loaded_url = None;
        self.did_report_finish = false;
        cx.notify();
    }
}

impl Default for GuideLoopingVideoView {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GuideLoopingVideoView {
    fn drop(&mut self) {
        self.end_observer = None;
        self.looper = None;
        self.queue_player = None;
    }
}

impl Render for GuideLoopingVideoView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        pass_through().size_full().child(
            AVPlayerLayer::new(self.queue_player.clone()).video_gravity(AVLayerVideoGravity::ResizeAspectFill),
        )
    }
}

#[derive(IntoElement)]
struct GuideActionRow {
    primary: Option<GuideOverlayAction>,
    secondary: Option<GuideOverlayAction>,
}

impl GuideActionRow {
    fn new(primary: Option<GuideOverlayAction>, secondary: Option<GuideOverlayAction>) -> Self {
        Self { primary, secondary }
    }
}

impl GuideActionRow {
    /// width a one line title takes in the shipped font
    fn title_width(title: &str, size: f32, weight: FontWeight, window: &Window, cx: &App) -> f32 {
        let size = text_size(size, ThemeStore::text_delta_snapshot(), ignores_text_size_delta(cx));
        let runs = [TextRun {
            len: title.len(),
            font: system_font(weight, FontFeatures::default()),
            color: white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        f32::from(
            window
                .text_system()
                .shape_line(SharedString::from(title.to_owned()), px(size), &runs, None)
                .width(),
        )
    }

    /// true when the row fits, meaning both titles keep their full width
    fn row_fits(&self, available: Option<f32>, window: &Window, cx: &App) -> bool {
        let Some(available) = available else {
            return true;
        };
        let mut needed = 0.0;
        let mut children = 1;
        if let Some(secondary) = &self.secondary {
            needed += Self::title_width(&secondary.title, GuideOverlayTheme::EXIT_TITLE_SIZE, FontWeight::NORMAL, window, cx)
                + GuideOverlayTheme::ACTION_HORIZONTAL_PADDING;
            children += 1;
        }
        if let Some(primary) = &self.primary {
            needed += Self::title_width(
                &primary.title,
                GuideOverlayTheme::ACTION_TITLE_SIZE,
                GuideOverlayTheme::ACTION_TITLE_WEIGHT,
                window,
                cx,
            ) + GuideOverlayTheme::ACTION_HORIZONTAL_PADDING * 2.0;
            children += 1;
        }
        needed + GuideOverlayTheme::SURFACE_ACTION_SPACING * (children - 1) as f32 <= available
    }

    fn secondary_button(secondary: Option<GuideOverlayAction>, theme: &AppTheme) -> Option<AnyElement> {
        let secondary = secondary?;
        let handler = secondary.handler;
        Some(
            SimpleButtonUi::new("guide-surface-secondary-action")
                .height(GuideOverlayTheme::action_height(theme))
                .on_press(move |window, cx| handler(window, cx))
                .child(
                    MainText::new(secondary.title)
                        .px(GuideOverlayTheme::EXIT_TITLE_SIZE)
                        .color(GuideOverlayTheme::exit_title_color(theme))
                        .number_of_lines(1)
                        .flex_none()
                        .padding_edges(EdgeSet::TRAILING, GuideOverlayTheme::ACTION_HORIZONTAL_PADDING),
                )
                .into_any_element(),
        )
    }

    fn primary_button(primary: Option<GuideOverlayAction>, theme: &AppTheme) -> Option<AnyElement> {
        let primary = primary?;
        let handler = primary.handler;
        Some(
            ButtonUi::new("guide-surface-primary-action")
                .height(GuideOverlayTheme::action_height(theme))
                .fit_content(true)
                .padding_horizontal(GuideOverlayTheme::ACTION_HORIZONTAL_PADDING)
                .on_press(move |window, cx| handler(window, cx))
                .child(
                    MainText::new(primary.title)
                        .px(GuideOverlayTheme::ACTION_TITLE_SIZE)
                        .font_weight(GuideOverlayTheme::ACTION_TITLE_WEIGHT)
                        .color(white())
                        .number_of_lines(1)
                        .flex_none(),
                )
                .into_any_element(),
        )
    }
}

impl RenderOnce for GuideActionRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.primary.is_none() && self.secondary.is_none() {
            return Empty.into_any_element();
        }
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme().clone();
        let available = window.use_keyed_state("guide-surface-action-row-width", cx, |_, _| None::<f32>);
        let fits = self.row_fits(*available.read(cx), window, cx);

        let has_primary = self.primary.is_some();
        let secondary = Self::secondary_button(self.secondary, &theme);
        let primary = Self::primary_button(self.primary, &theme);
        let content = if fits {
            hstack(GuideOverlayTheme::SURFACE_ACTION_SPACING)
                .w_full()
                .children(secondary)
                .child(spacer_min_w(0.0))
                .children(primary)
                .into_any_element()
        } else {
            let mut column = vstack(GuideOverlayTheme::SURFACE_ACTION_SPACING)
                .align(HorizontalAlignment::Leading)
                .w_full();
            if has_primary {
                column = column.child(hstack(0.0).w_full().child(spacer_min_w(0.0)).children(primary));
            }
            column.children(secondary).into_any_element()
        };
        let reported = available.clone();
        div()
            .relative()
            .w_full()
            .child(content)
            .child(measure("guide-surface-action-row-measure", move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                reported.update(cx, |holder, cx| {
                    if *holder != Some(width) {
                        *holder = Some(width);
                        cx.notify();
                    }
                });
            }))
            .into_any_element()
    }
}

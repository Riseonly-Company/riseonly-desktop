use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AsyncWindowContext, ElementId, Entity, FontWeight, Hsla, ImageSource, IntoElement, InteractiveElement,
    ObjectFit, ParentElement, Rgba, RenderOnce, SharedString, Styled, StyledImage, Window,
    black, div, img, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{Alignment, vstack, zstack};

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::media_count_plate::MediaCountPlate;
use crate::modules::post::components::post_moderation_disabled_overlay::PostModerationDisabledOverlay;
use crate::modules::story::shared::story_poster_cache::StoryPosterCache;
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt};
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileThumbnailDecodePolicy;

pub enum StoriesGridMetrics {}

impl StoriesGridMetrics {
    pub const COLUMNS: i64 = 3;
    pub const MAX_COLUMNS: i64 = 8;
    pub const WIDE_CELL_WIDTH: f32 = 180.0;
    pub const SPACING: f32 = 2.0;
    pub const HORIZONTAL_PADDING: f32 = 2.0;
    pub const CELL_ASPECT: f32 = 4.0 / 5.0;

    /// three columns on a narrow page, more as it widens up to MAX_COLUMNS
    pub fn columns(container_width: f32) -> i64 {
        if !container_width.is_finite() {
            return Self::COLUMNS;
        }
        let usable = container_width - Self::HORIZONTAL_PADDING * 2.0;
        let fit = ((usable + Self::SPACING) / (Self::WIDE_CELL_WIDTH + Self::SPACING)).floor() as i64;
        fit.clamp(Self::COLUMNS, Self::MAX_COLUMNS)
    }

    pub fn cell_width(container_width: f32) -> f32 {
        let columns = Self::columns(container_width);
        let gutters = Self::SPACING * (columns - 1) as f32 + Self::HORIZONTAL_PADDING * 2.0;
        ((container_width - gutters) / columns as f32).floor().max(1.0)
    }

    pub fn cell_height(container_width: f32) -> f32 {
        (Self::cell_width(container_width) / Self::CELL_ASPECT).ceil()
    }

    pub fn row_height(container_width: f32) -> f32 {
        Self::cell_height(container_width) + Self::SPACING
    }

    pub fn grid_height(item_count: i64, container_width: f32) -> f32 {
        let rows = (item_count as f64 / Self::columns(container_width) as f64).ceil() as i64;
        if rows <= 0 {
            return 0.0;
        }
        rows as f32 * Self::cell_height(container_width) + (rows - 1) as f32 * Self::SPACING
    }
}

#[derive(IntoElement)]
pub struct StoriesGridSelectionBadge {
    id: ElementId,
    pub order: Option<usize>,
}

impl StoriesGridSelectionBadge {
    const SIDE: f32 = 26.0;
    const ORDER_ANIMATION: f64 = 0.15;

    pub fn new(id: impl Into<ElementId>, order: Option<usize>) -> Self {
        Self { id: id.into(), order }
    }

    fn selected_fill() -> Hsla {
        Rgba { r: 1.0, g: 0.25, b: 0.25, a: 1.0 }.into()
    }
}

impl RenderOnce for StoriesGridSelectionBadge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target: f32 = if self.order.is_some() { 1.0 } else { 0.0 };
        let now = RiseClock::monotonic();
        let selected = window.use_keyed_state(self.id.clone(), cx, |_, _| Animated::new(target));
        selected.update(cx, |selected, cx| {
            selected.set_in(target, ease_out(Self::ORDER_ANIMATION), now, cx);
        });
        let progress = selected.read(cx).frame_value(now, window).clamp(0.0, 1.0);
        let side = px(Self::SIDE);

        zstack(Alignment::Center)
            .relative()
            .size(side)
            .child(div().absolute().inset_0().circle().bg(black().opacity(0.35 * (1.0 - progress))))
            .child(div().absolute().inset_0().circle().bg(Self::selected_fill().opacity(progress)))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .circle()
                    .stroke_border(white().opacity(0.85 + 0.15 * progress), 1.5),
            )
            .when_some(self.order, |badge, order| {
                badge.child(
                    div()
                        .swiftui_font(Font::system(11.0, FontWeight::BOLD, FontDesign::Default))
                        .text_color(white().opacity(progress))
                        .child(SharedString::from(order.to_string())),
                )
            })
    }
}

#[derive(IntoElement)]
pub struct StoriesGridCell {
    id: ElementId,
    pub story: StoryDTO,
    pub cell_width: f32,
    pub cell_height: f32,
    pub shows_view_count: bool,
    pub selection_order: Option<usize>,
    pub is_selecting: bool,
}

impl StoriesGridCell {
    pub fn new(
        story: StoryDTO,
        cell_width: f32,
        cell_height: f32,
        shows_view_count: bool,
        selection_order: Option<usize>,
        is_selecting: bool,
    ) -> Self {
        Self {
            id: ElementId::NamedInteger(SharedString::new_static("stories-grid-cell"), story.id as u64),
            story,
            cell_width,
            cell_height,
            shows_view_count,
            selection_order,
            is_selecting,
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn is_video(&self) -> bool {
        self.story.media_type.to_lowercase() == "video"
    }

    fn decode_dimension(&self, window: &Window) -> i64 {
        ProfileThumbnailDecodePolicy::maximum_pixel_dimension(
            size(px(self.cell_width), px(self.cell_height)),
            window.scale_factor(),
        )
    }

    /// a corner of the cell, plates are drawn over the already clipped plate
    fn corner_overlay(alignment: Alignment, content: impl IntoElement) -> impl IntoElement {
        div().absolute().inset_0().child(zstack(alignment).size_full().child(content))
    }

    fn media(&self, poster: Option<CleverImage>, source: &str, decode_dimension: i64, cx: &mut App) -> gpui::AnyElement {
        let width = px(self.cell_width);
        let height = px(self.cell_height);
        if let Some(poster) = poster {
            return img(ImageSource::Render(poster))
                .object_fit(ObjectFit::Cover)
                .w(width)
                .h(height)
                .into_any_element();
        }
        let is_video = self.is_video();
        if !source.is_empty() && !is_video {
            return div()
                .w(width)
                .h(height)
                .overflow_hidden()
                .child(
                    CleverImageUi::new(source.to_owned())
                        .content_mode(ContentMode::Fill)
                        .corner_radius(0.0)
                        .persist_to_disk(true)
                        .shows_progress_indicator(false)
                        .maximum_decode_pixel_dimension(decode_dimension),
                )
                .into_any_element();
        }
        let plate = ThemeStore::shared(cx).read(cx).current_theme().bg._300;
        zstack(Alignment::Center)
            .w(width)
            .h(height)
            .bg(plate)
            .when(is_video, |zstack| {
                zstack.children(IconUi::sized("play.fill", px(15.0), white().opacity(0.55), false))
            })
            .into_any_element()
    }

    fn badges(&self) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .when(self.story.is_pinned_in_profile(), |badges| {
                badges.child(Self::corner_overlay(
                    Alignment::TopLeading,
                    div().p(px(6.0)).child(
                        div()
                            .p(px(5.0))
                            .circle()
                            .bg(black().opacity(0.45))
                            .children(IconUi::sized("pin.fill", px(11.0), white(), false)),
                    ),
                ))
            })
            .child(Self::corner_overlay(
                Alignment::BottomLeading,
                vstack(3.0)
                    .items_start()
                    .p(px(6.0))
                    .when(self.story.was_edited(), |column| {
                        column.child(MediaCountPlate::new(tr("story_edited_badge"), None))
                    })
                    .when(self.shows_view_count, |column| {
                        column.child(MediaCountPlate::view_count(self.story.view_count))
                    }),
            ))
    }

    /// a fresh cell holds no poster already,
    /// a notify there would re render the grid page once more for every cell a scroll brings in
    fn clear_poster(poster: &Entity<Option<CleverImage>>, cx: &mut AsyncWindowContext) {
        poster.update(cx, |poster, cx| {
            if poster.take().is_some() {
                cx.notify();
            }
        });
    }

    async fn load_poster_if_needed(
        poster: Entity<Option<CleverImage>>,
        story_id: i64,
        source: String,
        is_video: bool,
        decode_dimension: i64,
        cx: &mut AsyncWindowContext,
    ) {
        if !is_video || source.is_empty() {
            Self::clear_poster(&poster, cx);
            return;
        }
        if let Some(hit) = StoryPosterCache::cached(story_id) {
            poster.update(cx, |poster, cx| {
                if !poster.as_ref().is_some_and(|current| Arc::ptr_eq(current, &hit)) {
                    *poster = Some(hit);
                    cx.notify();
                }
            });
            return;
        }
        Self::clear_poster(&poster, cx);
        let generated = StoryPosterCache::poster(story_id, &source, decode_dimension, &**cx).await;
        tracing::warn!(target: "riseonly::X1434", story_id, source = %source, decode_dimension, found = generated.is_some(), "grid poster done");
        poster.update(cx, |poster, cx| {
            *poster = generated;
            cx.notify();
        });
    }
}

impl RenderOnce for StoriesGridCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let story_id = self.story.id;
        let is_video = self.is_video();
        let source = self.story.media_source();
        let decode_dimension = self.decode_dimension(window);

        let poster: Entity<Option<CleverImage>> = window.use_keyed_state(self.child_id("poster"), cx, move |_, _| {
            if is_video { StoryPosterCache::cached(story_id) } else { None }
        });
        let task_poster = poster.clone();
        let task_source = source.clone();
        use_task(
            self.child_id("poster-task"),
            (story_id, is_video, decode_dimension, source.clone()),
            window,
            cx,
            async move |cx| {
                Self::load_poster_if_needed(task_poster, story_id, task_source, is_video, decode_dimension, cx).await;
            },
        );
        let poster = poster.read(cx).clone();
        let media = self.media(poster, &source, decode_dimension, cx);
        let badges = self.badges();
        let selection_badge = self.is_selecting.then(|| {
            Self::corner_overlay(
                Alignment::TopTrailing,
                div().p(px(6.0)).child(StoriesGridSelectionBadge::new(self.child_id("badge"), self.selection_order)),
            )
        });

        let hover_plate = div()
            .id(self.child_id("hover"))
            .absolute()
            .inset_0()
            .hover(|plate| plate.bg(white().opacity(0.06)));

        div()
            .relative()
            .w(px(self.cell_width))
            .h(px(self.cell_height))
            .overflow_hidden()
            .child(media)
            .child(hover_plate)
            .child(badges)
            .when(self.story.is_moderation_disabled(), |cell| {
                cell.child(div().absolute().inset_0().child(PostModerationDisabledOverlay))
            })
            .children(selection_badge)
    }
}

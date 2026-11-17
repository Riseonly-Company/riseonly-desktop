use std::rc::Rc;

use gpui::{
    App, AsyncWindowContext, Entity, FontWeight, ImageSource, ObjectFit, Pixels, Point, SharedString, StyledImage,
    Window, black, img, size, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, SelectionFeedbackGenerator, notification};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::media_count_plate::MediaCountPlate;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::story::components::stories_grid::stories_grid_view::container_width_reader;
use crate::modules::story::shared::story_poster_cache::StoryPosterCache;
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    HighlightDTO, HighlightScope, StoryDTOExt,
};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileThumbnailDecodePolicy;
use crate::modules::user::components::profile_content::pages::grid_posts::profile_grid_shared_element_origin::ProfileGridMetrics;

const SKELETON_ROWS: i64 = 4;
const TAP_MOVEMENT_TOLERANCE: f32 = 4.0;

/// archived highlights of the signed in user drawn as the publications grid
#[derive(IntoElement)]
pub struct HighlightsArchiveGrid {
    owner_user_id: String,
}

impl HighlightsArchiveGrid {
    pub fn new(owner_user_id: impl Into<String>) -> Self {
        Self { owner_user_id: owner_user_id.into() }
    }

    /// derived from the screen not measured, reporting a size out of the grid is the layout feedback loop
    fn grid_height(item_count: usize) -> f32 {
        let columns = ProfileGridMetrics::COLUMNS as usize;
        let rows = item_count.div_ceil(columns);
        if rows == 0 {
            return 0.0;
        }
        rows as f32 * ProfileGridMetrics::CELL_HEIGHT + (rows - 1) as f32 * ProfileGridMetrics::SPACING
    }

    fn grid(items: &[HighlightDTO], owner_user_id: &str, window: &mut Window, cx: &mut App) -> AnyElement {
        let width_state = window.use_keyed_state("highlights-archive-width", cx, |_, _| 0.0_f32);
        let is_measured = *width_state.read(cx) > 0.0;
        let width = width_state.read(cx).max(1.0);
        let columns = ProfileGridMetrics::COLUMNS as usize;
        let total_spacing = ProfileGridMetrics::SPACING * (ProfileGridMetrics::COLUMNS - 1) as f32;
        let cell_width = ((width - total_spacing) / ProfileGridMetrics::COLUMNS as f32).max(1.0);
        let visible_items = if is_measured { items } else { &[] };
        let rows: Vec<AnyElement> = visible_items
            .chunks(columns)
            .map(|row| {
                let cells: Vec<AnyElement> =
                    row.iter().map(|item| Self::cell(item, cell_width, owner_user_id)).collect();
                hstack(ProfileGridMetrics::SPACING).w_full().children(cells).into_any_element()
            })
            .collect();
        div()
            .relative()
            .w_full()
            .h(px(Self::grid_height(items.len())))
            .child(container_width_reader("highlights-archive-width-reader".into(), width_state))
            .child(vstack(ProfileGridMetrics::SPACING).w_full().align(HorizontalAlignment::Leading).children(rows))
            .into_any_element()
    }

    fn cell(item: &HighlightDTO, cell_width: f32, owner_user_id: &str) -> AnyElement {
        let source_identity = format!("highlight-archive-{}", item.id);
        let action_item = item.clone();
        let action_owner = owner_user_id.to_owned();
        let tap_item = item.clone();
        let content_item = item.clone();
        HoldContextMenuContainer::new(
            SharedString::from(source_identity.clone()),
            HoldContextMenuAlignment::Leading,
            Rc::new(|_: &App| {
                vec![
                    HoldContextMenuAction::new("unarchive", tr("highlight_menu_unarchive"))
                        .system_image("arrow.uturn.backward"),
                    HoldContextMenuAction::new("delete", tr("highlight_menu_delete"))
                        .system_image("trash")
                        .destructive(true),
                ]
            }),
            Rc::new(move |_: &mut Window, _: &mut App| {
                HighlightArchiveCell {
                    highlight: content_item.clone(),
                    cell_width,
                    cell_height: ProfileGridMetrics::CELL_HEIGHT,
                }
                .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .tap_movement_tolerance(TAP_MOVEMENT_TOLERANCE)
        .source_identity(Some(source_identity))
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_action(&id, &action_item, &action_owner, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| Self::open(&tap_item, cx)))
        .into_any_element()
    }

    fn handle_action(id: &str, item: &HighlightDTO, owner_user_id: &str, cx: &mut App) {
        match id {
            "unarchive" => {
                let _ = SelectionFeedbackGenerator::new().selection_changed();
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.set_highlight_archived(item, false, Some(owner_user_id), cx);
                });
            }
            "delete" => Self::present_delete(item.id, owner_user_id.to_owned(), cx),
            _ => {}
        }
    }

    fn present_delete(highlight_id: i64, owner_user_id: String, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("highlight_delete_title"),
            &tr("highlight_delete_message"),
            Some(tr("highlight_menu_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                let _ = notification(NotificationKind::Warning);
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.delete_highlight(highlight_id, Some(&owner_user_id), HighlightScope::Archive, cx);
                });
            },
            cx,
        );
    }

    fn open(item: &HighlightDTO, cx: &mut App) {
        let _ = SelectionFeedbackGenerator::new().selection_changed();
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.open_highlight(item.id, &item.owner_id, None, None, cx);
        });
    }
}

impl RenderOnce for HighlightsArchiveGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let owner_user_id = self.owner_user_id;
        let service = StoryServicesStore::shared(cx);
        let items =
            service.update(cx, |service, cx| service.highlights(&owner_user_id, HighlightScope::Archive, cx));
        let is_loaded = trimming_whitespaces_and_newlines(&owner_user_id).is_empty()
            || service.update(cx, |service, cx| {
                service.highlights_loaded(&owner_user_id, HighlightScope::Archive, cx)
            });

        let task_owner = owner_user_id.clone();
        use_task("highlights-archive-load", owner_user_id.clone(), window, cx, async move |cx| {
            if task_owner.is_empty() {
                return;
            }
            let _ = cx.update(|_, cx| {
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_user_highlights(None, HighlightScope::Archive, cx);
                });
            });
        });

        if !is_loaded {
            PostGridSkeleton::default()
                .columns(ProfileGridMetrics::COLUMNS)
                .spacing(ProfileGridMetrics::SPACING)
                .cell_height(ProfileGridMetrics::CELL_HEIGHT)
                .rows(SKELETON_ROWS)
                .into_any_element()
        } else if items.is_empty() {
            ChatProfileEmptyState::new("archivebox", tr("highlights_archive_empty")).into_any_element()
        } else {
            Self::grid(&items, &owner_user_id, window, cx)
        }
    }
}

#[derive(IntoElement)]
struct HighlightArchiveCell {
    highlight: HighlightDTO,
    cell_width: f32,
    cell_height: f32,
}

impl HighlightArchiveCell {
    fn media(&self, poster: Option<CleverImage>, source: &str, is_video: bool, decode_dimension: i64) -> AnyElement {
        if let Some(poster) = poster {
            return img(ImageSource::Render(poster))
                .object_fit(ObjectFit::Cover)
                .w(px(self.cell_width))
                .h(px(self.cell_height))
                .into_any_element();
        }
        if !source.is_empty() && !is_video {
            return div()
                .w(px(self.cell_width))
                .h(px(self.cell_height))
                .overflow_hidden()
                .child(
                    CleverImageUi::new(SharedString::from(source.to_owned()))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(0.0)
                        .persist_to_disk(true)
                        .shows_progress_indicator(false)
                        .maximum_decode_pixel_dimension(decode_dimension),
                )
                .into_any_element();
        }
        div()
            .children(IconUi::sized(
                if is_video { "play.fill" } else { "photo.on.rectangle" },
                px(18.0),
                white().opacity(0.55),
                false,
            ))
            .into_any_element()
    }

    fn caption(&self) -> impl IntoElement {
        let title = self.highlight.title.clone();
        vstack(3.0)
            .items_start()
            .p(px(6.0))
            .child(
                div()
                    .relative()
                    .child(
                        div().absolute().top(px(1.0)).left_0().child(
                            MainText::new(title.clone())
                                .px(12.0)
                                .tac(TextAlignT::Left)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(black().opacity(0.6))
                                .number_of_lines(1),
                        ),
                    )
                    .child(
                        MainText::new(title)
                            .px(12.0)
                            .tac(TextAlignT::Left)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white())
                            .number_of_lines(1),
                    ),
            )
            .child(MediaCountPlate::new(
                RiseFormat::compact_count(self.highlight.stories_count.max(0)),
                Some(SharedString::new_static("circle.grid.2x2.fill")),
            ))
    }

    async fn load_poster_if_needed(
        poster: Entity<Option<CleverImage>>,
        story_id: Option<i64>,
        source: String,
        is_video: bool,
        decode_dimension: i64,
        cx: &mut AsyncWindowContext,
    ) {
        let Some(story_id) = story_id.filter(|_| is_video && !source.is_empty()) else {
            poster.update(cx, |poster, cx| {
                *poster = None;
                cx.notify();
            });
            return;
        };
        if let Some(hit) = StoryPosterCache::cached(story_id) {
            poster.update(cx, |poster, cx| {
                *poster = Some(hit);
                cx.notify();
            });
            return;
        }
        poster.update(cx, |poster, cx| {
            *poster = None;
            cx.notify();
        });
        let generated = StoryPosterCache::poster(story_id, &source, decode_dimension, &**cx).await;
        poster.update(cx, |poster, cx| {
            *poster = generated;
            cx.notify();
        });
    }
}

impl RenderOnce for HighlightArchiveCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme: AppTheme = rise_ui::theme(cx).clone();
        let highlight_id = self.highlight.id;
        let cover = self.highlight.cover_story.clone();
        let source = cover.as_ref().map(|cover| cover.media_source()).unwrap_or_default();
        let is_video = cover.as_ref().is_some_and(|cover| cover.media_type.to_lowercase() == "video");
        let story_id = cover.as_ref().map(|cover| cover.id);
        let decode_dimension = ProfileThumbnailDecodePolicy::maximum_pixel_dimension(
            size(px(self.cell_width), px(self.cell_height)),
            window.scale_factor(),
        );

        let poster = window.use_keyed_state(
            SharedString::from(format!("highlight-archive-poster-{highlight_id}")),
            cx,
            |_, _| None::<CleverImage>,
        );
        let task_poster = poster.clone();
        let task_source = source.clone();
        use_task(
            SharedString::from(format!("highlight-archive-poster-task-{highlight_id}")),
            (story_id.unwrap_or(0), is_video, decode_dimension),
            window,
            cx,
            async move |cx| {
                Self::load_poster_if_needed(task_poster, story_id, task_source, is_video, decode_dimension, cx).await;
            },
        );

        let media = self.media(poster.read(cx).clone(), &source, is_video, decode_dimension);
        div()
            .relative()
            .flex_none()
            .w(px(self.cell_width))
            .h(px(self.cell_height))
            .bg(theme.bg._300)
            .overflow_hidden()
            .child(div().absolute().inset_0().flex().items_center().justify_center().child(media))
            .child(div().absolute().inset_0().child(zstack(Alignment::BottomLeading).size_full().child(self.caption())))
    }
}

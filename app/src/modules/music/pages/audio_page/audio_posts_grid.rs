use std::rc::Rc;

use gpui::{App, ElementId, Entity, IntoElement, RenderOnce, SharedString, Subscription, Window, div, px};
use rise_i18n::tr;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::modules::gifts::components::gift_shop_shelf_grid::{GiftScrollWindowProbe, GiftShopShelfRows};
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_audio_services::PostServicesStoreAudioPostsState;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileGridPostCell;

const AUDIO_GRID_CELL_HEIGHT: f32 = 150.0;
const AUDIO_GRID_COLUMNS: i64 = 3;
const AUDIO_GRID_SPACING: f32 = 2.0;
const AUDIO_GRID_SKELETON_ROWS: i64 = 2;

pub(crate) type AudioPageNavigate = Rc<dyn Fn(RootRoute, &mut Window, &mut App)>;

struct AudioPostsGridState {
    _subscriptions: [Subscription; 2],
}

impl AudioPostsGridState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = PostServicesStore::shared(cx);
        let revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&revisions, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct AudioPostsGrid {
    id: ElementId,
    track_id: String,
    on_navigate: Option<AudioPageNavigate>,
    on_retry: Option<Rc<dyn Fn(&mut App)>>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl AudioPostsGrid {
    pub fn new(id: impl Into<ElementId>, track_id: impl Into<String>) -> Self {
        Self { id: id.into(), track_id: track_id.into(), on_navigate: None, on_retry: None, nav_store: None }
    }

    pub fn on_navigate(mut self, on_navigate: Option<AudioPageNavigate>) -> Self {
        self.on_navigate = on_navigate;
        self
    }

    pub fn on_retry(mut self, on_retry: Option<Rc<dyn Fn(&mut App)>>) -> Self {
        self.on_retry = on_retry;
        self
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("{}-{name}", self.id)))
    }

    fn grid_body(
        self: &Rc<Self>,
        page: PostServicesStoreAudioPostsState,
        height: f32,
        width: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let cache_key = PostServicesStore::audio_posts_cache_key(&self.track_id);
        let posts = page.items;
        let item_count = posts.len() as i64;

        let w = width.max(0.0);
        let total_spacing = AUDIO_GRID_SPACING * (AUDIO_GRID_COLUMNS - 1) as f32;
        let cell_w = ((w - total_spacing) / AUDIO_GRID_COLUMNS as f32).max(0.0);

        let row_count = Self::row_count(posts.len());
        let row_pitch = AUDIO_GRID_CELL_HEIGHT + AUDIO_GRID_SPACING;
        let initial_rows = (f32::from(window.viewport_size().height) / row_pitch).ceil() as i64 + 1;
        let rows_state: Entity<Option<GiftShopShelfRows>> =
            window.use_keyed_state(self.child_id("rows"), cx, |_, _| None);
        let rows = rows_state.read(cx).unwrap_or(GiftShopShelfRows::leading(initial_rows)).clamped(row_count);
        let range = rows.items(AUDIO_GRID_COLUMNS, item_count);

        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        for position in range.start..range.end {
            let Some(post) = posts.get(position as usize) else { continue };
            let row = position / AUDIO_GRID_COLUMNS;
            let column = position % AUDIO_GRID_COLUMNS;
            let grid = self.clone();
            let opened = post.clone();
            let load_track_id = self.track_id.clone();
            let options = SaiPaginationPrefetchOptions::new(
                position as usize,
                posts.len(),
                cache_key.clone(),
                page.relative_id.clone(),
                page.has_more,
            );
            cells.push(
                div()
                    .absolute()
                    .left(px(column as f32 * (cell_w + AUDIO_GRID_SPACING)))
                    .top(px(row as f32 * row_pitch))
                    .w(px(cell_w))
                    .h(px(AUDIO_GRID_CELL_HEIGHT))
                    .child(
                        plain(self.child_id(&format!("cell-{}", post.id)))
                            .on_press(move |window, cx| grid.open_post(&opened, window, cx))
                            .child(ProfileGridPostCell::new(post.clone(), cell_w, AUDIO_GRID_CELL_HEIGHT))
                            .sai_pagination_prefetch(
                                self.child_id(&format!("prefetch-{}", post.id)),
                                options,
                                move |_, cx| {
                                    PostInteractionsStore::shared(cx)
                                        .update(cx, |store, cx| store.load_more_audio_posts(&load_track_id, cx));
                                },
                            ),
                    )
                    .into_any_element(),
            );
        }

        let probe_rows = rows_state;
        let window_probe = GiftScrollWindowProbe::new(self.child_id("window-probe"), move |band, band_height, cx| {
            let Some(band) = band else { return };
            if band_height <= 0.0 {
                return;
            }
            probe_rows.update(cx, |mounted, cx| {
                let next = GiftShopShelfRows::following(
                    &band,
                    band_height,
                    row_count,
                    AUDIO_GRID_SPACING,
                    mounted.unwrap_or(GiftShopShelfRows::leading(initial_rows)),
                );
                if Some(next) != *mounted {
                    *mounted = Some(next);
                    cx.notify();
                }
            });
        });

        div()
            .relative()
            .w(px(w))
            .h(px(height))
            .child(div().absolute().inset_0().children(cells))
            .child(div().absolute().inset_0().child(window_probe))
            .into_any_element()
    }

    fn open_post(&self, post: &GetPostFeedResponse, window: &mut Window, cx: &mut App) {
        let route = if post.is_riff {
            RootRoute::ShortViewer { post: post.clone(), origin: None, sound_track_id: Some(self.track_id.clone()) }
        } else {
            RootRoute::post_content(post, None)
        };
        if let Some(on_navigate) = self.on_navigate.as_ref() {
            on_navigate(route, window, cx);
        } else if let Some(nav_store) = self.nav_store.as_ref() {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn row_count(count: usize) -> i64 {
        (count as f64 / AUDIO_GRID_COLUMNS as f64).ceil() as i64
    }
}

impl RenderOnce for AudioPostsGrid {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.nav_store = env::<NavigationStoreKey>(cx);
        let grid = Rc::new(self);
        let _state = window.use_keyed_state(grid.child_id("state"), cx, |_, cx| AudioPostsGridState::new(cx));

        let initial_width = f32::from(window.viewport_size().width);
        let width_state: Entity<f32> = window.use_keyed_state(grid.child_id("width"), cx, move |_, _| initial_width);
        let width = *width_state.read(cx);

        let services = PostServicesStore::shared(cx);
        let page = services.update(cx, |service, cx| service.audio_posts(&grid.track_id, cx));
        let status = services.update(cx, |service, cx| service.audio_posts_status(&grid.track_id, cx));
        let is_empty = page.items.is_empty();
        let rows = Self::row_count(page.items.len());
        let grid_h = rows as f32 * AUDIO_GRID_CELL_HEIGHT + (rows - 1).max(0) as f32 * AUDIO_GRID_SPACING;
        let skeleton_h = AUDIO_GRID_SKELETON_ROWS as f32 * AUDIO_GRID_CELL_HEIGHT
            + (AUDIO_GRID_SKELETON_ROWS - 1) as f32 * AUDIO_GRID_SPACING;
        let outer_height =
            if is_empty && status == SaiWsSliceStatus::Pending { skeleton_h } else { grid_h.max(220.0) };

        let reader_state = width_state;
        let width_reader = measure(grid.child_id("width-reader"), move |bounds, _, cx| {
            let width = f32::from(bounds.size.width).max(0.0);
            reader_state.update(cx, |current, cx| {
                if *current != width {
                    *current = width;
                    cx.notify();
                }
            });
        });

        let render_id = grid.child_id("render");
        let retry_id = grid.child_id("retry");
        let on_retry = grid.on_retry.clone();
        let body_grid = grid.clone();
        div()
            .relative()
            .w_full()
            .h(px(outer_height))
            .overflow_hidden()
            .child(width_reader)
            .child(
                SaiDataRender::new(status, move |window, cx| body_grid.grid_body(page, grid_h, width, window, cx))
                    .id(render_id)
                    .is_empty(is_empty)
                    .no_data_text(tr("audio_posts_empty"))
                    .error_text(tr("feed_load_error"))
                    .when_some(on_retry, move |render, on_retry| {
                        render.sai_data_render_error(move |_, cx| {
                            let theme = rise_ui::theme(cx);
                            let on_retry = on_retry.clone();
                            vstack(12.0)
                                .w_full()
                                .pt(px(48.0))
                                .px(px(24.0))
                                .items_center()
                                .child(
                                    MainText::new(tr("feed_load_error"))
                                        .min_w_0()
                                        .px(16.0)
                                        .tac(TextAlignT::Center)
                                        .color(theme.text.primary),
                                )
                                .child(
                                    plain(retry_id.clone())
                                        .on_press(move |_, cx| on_retry(cx))
                                        .child(MainText::new(tr("retry")).px(17.0).color(theme.primary._100)),
                                )
                        })
                    })
                    .sai_data_render_loading(|_, _| {
                        PostGridSkeleton::default()
                            .spacing(AUDIO_GRID_SPACING)
                            .cell_height(AUDIO_GRID_CELL_HEIGHT)
                            .rows(AUDIO_GRID_SKELETON_ROWS)
                    }),
            )
    }
}

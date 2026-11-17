use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, IntoElement, ParentElement, Pixels, Point, RenderOnce, SharedString,
    Styled, Window, black, div, px, size, white,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAlignment;
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, vstack};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementReturnScrollMath;
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementSurface;
use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementOriginContainer;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementHostProbe;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::{
    SharedElementLiftModifiers, SharedElementSurfaceKey,
};
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::media_count_plate::MediaCountPlate;
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::components::post_collection_ui::{
    PostGridTextUi, PostPageStateUi, PostPaginationTriggerModifiers,
};
use crate::modules::post::components::post_media_upload_overlay::PostMediaUploadOverlay;
use crate::modules::post::components::post_moderation_disabled_overlay::PostModerationGridOverlay;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostPageScope, RisePostPageStatus, RisePostProfileKind,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::components::shorts_zoom_namespace::{
    ShortsZoomNamespaceId, ShortsZoomNamespaceKey, ShortsZoomSource,
};
use crate::modules::story::components::stories_grid::stories_grid_cell::StoriesGridMetrics;
use crate::modules::story::components::stories_grid::stories_grid_view::{container_width_reader, use_container_width};
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::{
    ProfileGridMenuAsOwnerKey, ProfileGridPostMenu, ProfileGridPreviewSourcePolicy, ProfileGridSharedElementInputs,
    ProfileThumbnailDecodePolicy,
};
use crate::modules::user::components::profile_content::pages::grid_posts::profile_grid_shared_element_origin::{
    ProfileGridSharedElementLayout, ProfileGridSharedElementOrigin,
};
use crate::modules::user::components::profile_content::pages::grid_reposts::grid_reposts::lazy_grid_mounted_range;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::{
    SharedElementReturnScrollerKey, SharedElementReturnScrollerRef,
};

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// geometry of the riffs grid, centered columns and a taller portrait cell
pub enum RiffsGridMetrics {}

impl RiffsGridMetrics {
    pub const COLUMNS: i64 = StoriesGridMetrics::COLUMNS;
    pub const MAX_COLUMNS: i64 = 6;
    pub const WIDE_CELL_WIDTH: f32 = 160.0;
    pub const SPACING: f32 = StoriesGridMetrics::SPACING;
    pub const HORIZONTAL_PADDING: f32 = StoriesGridMetrics::HORIZONTAL_PADDING;
    pub const CELL_ASPECT: f32 = 2.0 / 3.0;

    pub fn columns(container_width: f32) -> i64 {
        if !container_width.is_finite() {
            return Self::COLUMNS;
        }
        let inner = container_width - Self::HORIZONTAL_PADDING * 2.0;
        let fit = ((inner + Self::SPACING) / (Self::WIDE_CELL_WIDTH + Self::SPACING)).floor() as i64;
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

    pub fn grid_height(item_count: i64, container_width: f32) -> f32 {
        let rows = (item_count as f64 / Self::columns(container_width) as f64).ceil() as i64;
        if rows <= 0 {
            return 0.0;
        }
        rows as f32 * Self::cell_height(container_width) + (rows - 1) as f32 * Self::SPACING
    }
}

/// riffs tab of a profile, portrait columns that open the vertical riff viewer over the same list
#[derive(IntoElement)]
pub struct GridRiffs {
    id: ElementId,
    pub profile_tag: String,
    pub profile_user_id: String,
}

impl GridRiffs {
    const SKELETON_ROWS: i64 = 4;
    const MINIMUM_HEIGHT: f32 = 220.0;

    pub fn new(profile_tag: impl Into<String>, profile_user_id: impl Into<String>) -> Self {
        let profile_tag = profile_tag.into();
        let profile_user_id = profile_user_id.into();
        Self {
            id: ElementId::Name(SharedString::from(format!("grid-riffs-{profile_user_id}-{profile_tag}"))),
            profile_tag,
            profile_user_id,
        }
    }

    /// grid outer height. side padding is already in the cell width so dont inset again or the
    /// last row clips
    fn outer_height(item_count: i64, shows_skeleton: bool, container_width: f32) -> f32 {
        let laid_out_cells =
            if shows_skeleton { Self::SKELETON_ROWS * RiffsGridMetrics::columns(container_width) } else { item_count };
        Self::MINIMUM_HEIGHT.max(RiffsGridMetrics::grid_height(laid_out_cells, container_width))
    }

    /// anchors and host probe only exist where a flight can run
    fn shared_elements_active() -> bool {
        SharedElementNavigationPolicy::flights_enabled()
            && SharedElementNavigationPolicy::VIEWER_KINDS.contains(&RisePostProfileKind::Riffs)
    }

    fn shared_element_inputs(&self, scroller: Option<SharedElementReturnScrollerRef>) -> ProfileGridSharedElementInputs {
        ProfileGridSharedElementInputs {
            user_id: self.profile_user_id.clone(),
            tag: self.profile_tag.clone(),
            kind: RisePostProfileKind::Riffs,
            scroller,
        }
    }

    fn configure_shared_element_origin(
        origin: &ProfileGridSharedElementOrigin,
        inputs: &ProfileGridSharedElementInputs,
    ) {
        origin.configure(
            &inputs.user_id,
            &inputs.tag,
            RisePostProfileKind::Riffs,
            inputs.scroller.clone(),
            ProfileGridSharedElementLayout::Riffs,
        );
    }

    fn refresh(profile_user_id: &str, profile_tag: &str, cx: &mut App) {
        PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.refresh_profile(profile_user_id, profile_tag, RisePostProfileKind::Riffs, cx))
            .detach();
    }
}

impl RenderOnce for GridRiffs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let profile_grid_menu_as_owner = env::<ProfileGridMenuAsOwnerKey>(cx);
        let shorts_zoom_namespace = env::<ShortsZoomNamespaceKey>(cx);
        let return_scroller = env::<SharedElementReturnScrollerKey>(cx);
        let user_id = self.profile_user_id.clone();
        let tag = self.profile_tag.clone();
        let kind = RisePostProfileKind::Riffs;

        let content_owner_id = *window.use_keyed_state(child_id(&self.id, "owner"), cx, |_, _| Uuid::new_v4()).read(cx);
        let shared_element_origin = window
            .use_keyed_state(child_id(&self.id, "origin"), cx, |_, _| ProfileGridSharedElementOrigin::new())
            .read(cx)
            .clone();
        let width_state = use_container_width(child_id(&self.id, "width"), window, cx);
        let layout_width = *width_state.read(cx);

        let page = PostServicesStore::shared(cx)
            .update(cx, |service, cx| service.profile_page_with_tag(&user_id, &tag, kind, cx));
        let item_count = page.items.len() as i64;
        let has_items = item_count > 0;
        let has_subject = !user_id.is_empty();
        let shared_element_surface = ProfileGridSharedElementOrigin::surface(&user_id, &tag, kind);
        let anchor_surface = Self::shared_elements_active().then(|| shared_element_surface.clone());
        let shows_skeleton = has_subject
            && !has_items
            && matches!(page.status, RisePostPageStatus::Idle | RisePostPageStatus::Loading);
        let outer_height = Self::outer_height(item_count, shows_skeleton, layout_width);

        let configure_origin = shared_element_origin.clone();
        on_change(
            child_id(&self.id, "inputs"),
            self.shared_element_inputs(return_scroller),
            true,
            window,
            cx,
            move |_, inputs, _, _| Self::configure_shared_element_origin(&configure_origin, inputs),
        );

        let task_user = user_id.clone();
        let task_tag = tag.clone();
        use_task(
            child_id(&self.id, "observe"),
            format!("{user_id}|{}", tag.to_lowercase()),
            window,
            cx,
            async move |cx| {
                let settled = cx.update(|_, cx| {
                    SharedElementTransitionCoordinator::wait_until_settled(shared_element_surface, 1.0, cx)
                });
                if let Ok(settled) = settled {
                    settled.await;
                }
                if task_user.is_empty() {
                    return;
                }
                let _ = cx.update(|_, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.observe_profile(&task_user, &task_tag, kind, content_owner_id, cx);
                    });
                });
            },
        );

        let parts = Rc::new(GridRiffsParts {
            id: self.id.clone(),
            profile_tag: tag.clone(),
            profile_user_id: user_id.clone(),
            nav_store,
            profile_grid_menu_as_owner,
            shorts_zoom_namespace,
            shared_element_origin,
        });
        let retry_user = user_id.clone();
        let retry_tag = tag.clone();
        let error_text = page.error_message.clone().unwrap_or_else(|| tr("feed_load_error"));
        let empty_text = tr_args("shorts_empty", &[FormatArg::Text(Cow::Owned(tr("riffs")))]);
        let status = page.status;
        let page = Rc::new(page);
        let state = PostPageStateUi::new(
            child_id(&self.id, "state"),
            status,
            has_items,
            "play.rectangle",
            empty_text,
            error_text,
            move |cx: &mut App| Self::refresh(&retry_user, &retry_tag, cx),
        )
        .has_subject(has_subject)
        .loading(move |_, _| {
            div().w_full().px(px(RiffsGridMetrics::HORIZONTAL_PADDING)).child(
                PostGridSkeleton::default()
                    .columns(RiffsGridMetrics::columns(layout_width))
                    .spacing(RiffsGridMetrics::SPACING)
                    .cell_height(RiffsGridMetrics::cell_height(layout_width))
                    .rows(Self::SKELETON_ROWS),
            )
        })
        .content(move |window, cx| parts.grid_body(page, layout_width, anchor_surface, window, cx));

        let stop_user = user_id;
        let stop_tag = tag;
        div()
            .relative()
            .w_full()
            .h(px(outer_height))
            .child(container_width_reader(child_id(&self.id, "width-reader"), width_state))
            .child(state)
            .on_disappear(child_id(&self.id, "disappear"), move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.stop_observing_profile(&stop_user, &stop_tag, kind, content_owner_id, cx);
                });
            })
    }
}

/// what the handlers of one bodys cells share
struct GridRiffsParts {
    id: ElementId,
    profile_tag: String,
    profile_user_id: String,
    nav_store: Option<Entity<NavigationStore>>,
    profile_grid_menu_as_owner: bool,
    shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    shared_element_origin: Rc<ProfileGridSharedElementOrigin>,
}

impl GridRiffsParts {
    fn grid_body(
        self: Rc<Self>,
        page: Rc<RisePostPagePresentation>,
        width: f32,
        anchor_surface: Option<SharedElementSurface>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let width = width.max(1.0);
        let cell_width = RiffsGridMetrics::cell_width(width);
        let cell_height = RiffsGridMetrics::cell_height(width);
        let column_count = RiffsGridMetrics::columns(width);
        let columns = column_count as usize;
        let columns_width = columns as f32 * cell_width + (columns - 1) as f32 * RiffsGridMetrics::SPACING;
        let origin_x = ((width - columns_width) / 2.0).max(0.0);
        let item_count = page.items.len() as i64;
        let (range, window_probe) = lazy_grid_mounted_range(
            &self.id,
            item_count,
            column_count,
            RiffsGridMetrics::SPACING,
            cell_height,
            window,
            cx,
        );

        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        for index in range.start as usize..range.end as usize {
            let frame = SharedElementReturnScrollMath::grid_item_rect_with_origin_x(
                index,
                columns,
                cell_width,
                cell_height,
                RiffsGridMetrics::SPACING,
                0.0,
                origin_x,
            );
            cells.push(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(self.cell(index, &page, cell_width, cell_height, anchor_surface.clone(), cx))
                    .into_any_element(),
            );
        }

        let host_probe = anchor_surface.is_some().then(|| {
            let container: Rc<dyn SharedElementOriginContainer> = self.shared_element_origin.clone();
            let origin = self.shared_element_origin.clone();
            SharedElementHostProbe::origin(container).on_layout(move |probe, window, _| {
                origin.record_columns(column_count);
                origin.record_layout(probe, cell_width, Some(cell_height), origin_x, window);
            })
        });

        div()
            .relative()
            .w(px(width))
            .h(px(RiffsGridMetrics::grid_height(item_count, width)))
            .children(host_probe)
            .child(env_scope::<SharedElementSurfaceKey>(anchor_surface).child(div().absolute().inset_0().children(cells)))
            .child(div().absolute().inset_0().child(window_probe))
            .into_any_element()
    }

    fn cell(
        self: &Rc<Self>,
        index: usize,
        page: &RisePostPagePresentation,
        cell_width: f32,
        cell_height: f32,
        anchor_surface: Option<SharedElementSurface>,
        cx: &mut App,
    ) -> AnyElement {
        let post = Rc::new(page.items[index].clone());
        let presentation_id = post.presentation_id();
        let is_locked = post.shows_upload_overlay();
        let actions = if is_locked {
            Vec::new()
        } else {
            let mine = ProfileGridPostMenu::is_mine(&post, self.profile_grid_menu_as_owner, cx);
            ProfileGridPostMenu::actions(&post, mine, cx)
        };
        let view_count =
            PostServicesStore::shared(cx).update(cx, |service, cx| service.post_view_count(&post, cx));

        let content_post = post.clone();
        let impression_id = child_id(&self.id, format!("impression-{presentation_id}"));
        let action_post = post.clone();
        let action_parts = self.clone();
        let tap_post = post.clone();
        let tap_parts = self.clone();
        let load_parts = self.clone();
        let menu = HoldContextMenuContainer::new(
            child_id(&self.id, format!("hold-{presentation_id}")),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                let impression_post = content_post.clone();
                ProfileRiffGridCell::new(content_post.clone(), cell_width, cell_height, view_count)
                    .shared_element_anchor(
                        content_post.id.clone(),
                        ProfileGridSharedElementOrigin::part_layout(&content_post, ProfileGridSharedElementLayout::Riffs),
                    )
                    .on_appear(impression_id.clone(), move |_, cx| {
                        GridRiffsParts::fire_grid_impression(&impression_post, cx);
                    })
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .tap_movement_tolerance(4.0)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            let open_parts = action_parts.clone();
            let open_post = action_post.clone();
            ProfileGridPostMenu::handle(
                &id,
                &action_post,
                action_parts.nav_store.as_ref(),
                move |cx| {
                    if open_parts.shared_element_origin.arm_open_if_possible(&open_post, cx) {
                        HoldContextMenuPresenter::dismiss(cx);
                    }
                    open_parts.open_riff(&open_post, cx);
                },
                cx,
            );
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            if is_locked {
                return;
            }
            tap_parts.shared_element_origin.arm_open_if_possible(&tap_post, cx);
            tap_parts.open_riff(&tap_post, cx);
        }));
        self.grid_zoom_source(&post)
            .body(menu)
            .post_pagination_trigger(
                child_id(&self.id, format!("pagination-{presentation_id}")),
                index as i64,
                page.items.len() as i64,
                page.has_more,
                page.is_loading_more,
                move |cx| load_parts.load_more(anchor_surface.clone(), cx),
            )
            .into_any_element()
    }

    /// appending never moves earlier cells but still waits for a flight on this grid
    fn load_more(&self, deferred_until_settled: Option<SharedElementSurface>, cx: &mut App) {
        let user_id = self.profile_user_id.clone();
        let tag = self.profile_tag.clone();
        let load = move |cx: &mut App| {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_more_profile(&user_id, &tag, RisePostProfileKind::Riffs, cx);
            });
        };
        match deferred_until_settled {
            None => load(cx),
            Some(surface) => SharedElementTransitionCoordinator::run_or_defer_until_settled(surface, load, cx),
        }
    }

    fn grid_zoom_source(&self, post: &GetPostFeedResponse) -> ShortsZoomSource {
        let pushed = self.route(post);
        let namespace = if SharedElementNavigationPolicy::uses_shared_element_navigation_route(&pushed) {
            None
        } else {
            self.shorts_zoom_namespace
        };
        ShortsZoomSource::new(pushed, namespace)
    }

    fn route(&self, post: &GetPostFeedResponse) -> RootRoute {
        RootRoute::post_content(
            post,
            Some(RisePostPageScope::Profile {
                user_id: self.profile_user_id.clone(),
                tag: self.profile_tag.clone(),
                kind: RisePostProfileKind::Riffs,
            }),
        )
    }

    fn open_riff(&self, post: &GetPostFeedResponse, cx: &mut App) {
        let route = self.route(post);
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn fire_grid_impression(post: &GetPostFeedResponse, cx: &mut App) {
        let post_id = post.rise_view_target_post_id();
        let author_id = post.author.id.as_deref().unwrap_or(&post.author_id).to_owned();
        let is_short = post.is_riff;
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.impression(&post_id, &author_id, EngagementSurface::ProfileGrid, -1, is_short, 500, cx);
        });
    }
}

/// the still a riff shows in a grid, its first slide
/// video first shows its poster, else the first photo
/// uploading uses the first local photo
pub enum ProfileRiffCoverSourcePolicy {}

impl ProfileRiffCoverSourcePolicy {
    pub fn source(post: &GetPostFeedResponse) -> Option<String> {
        let poster = ProfileGridPreviewSourcePolicy::source(None, None, post.video_thumbnail.as_deref());
        let first_image = ProfileGridPreviewSourcePolicy::source(None, post.images.first().map(String::as_str), None);
        let preferred = if post.cover_slide().is_some_and(|slide| slide.is_video()) {
            poster.or(first_image)
        } else {
            first_image.or(poster)
        };
        preferred.or_else(|| ProfileGridPreviewSourcePolicy::source(post.local_preview_url.as_deref(), None, None))
    }
}

#[derive(IntoElement)]
struct ProfileRiffGridCell {
    post: Rc<GetPostFeedResponse>,
    cell_width: f32,
    cell_height: f32,
    view_count: i64,
}

impl ProfileRiffGridCell {
    fn new(post: Rc<GetPostFeedResponse>, cell_width: f32, cell_height: f32, view_count: i64) -> Self {
        Self { post, cell_width, cell_height, view_count }
    }

    fn badges(&self) -> Div {
        let post = &self.post;
        let shows_upload_overlay = post.shows_upload_overlay();
        div()
            .absolute()
            .inset_0()
            .when(post.track_id.is_some() && !shows_upload_overlay, |layer| {
                layer.child(div().absolute().top_0().left_0().p(px(6.0)).child(Self::corner_badge("music.note")))
            })
            .when(post.media_slide_count() > 1, |layer| {
                layer.child(div().absolute().top_0().right_0().p(px(6.0)).child(Self::corner_badge("photo.stack")))
            })
            .when(!shows_upload_overlay, |layer| {
                layer.child(
                    vstack(3.0)
                        .align(HorizontalAlignment::Leading)
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .p(px(6.0))
                        .when(post.coauthors_count > 0, |stack| {
                            stack.child(
                                div()
                                    .px(px(7.0))
                                    .py(px(5.0))
                                    .rounded_full()
                                    .bg(black().opacity(0.42))
                                    .children(IconUi::sized("person.2.fill", px(11.0), white(), false)),
                            )
                        })
                        .child(MediaCountPlate::view_count(self.view_count)),
                )
            })
    }

    fn corner_badge(icon: &str) -> Div {
        div()
            .p(px(6.0))
            .rounded_full()
            .bg(black().opacity(0.42))
            .children(IconUi::sized(icon, px(12.0), white(), false))
    }
}

impl RenderOnce for ProfileRiffGridCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let post = self.post.clone();
        let is_own_post = PostMediaCachePolicy::is_own_post(&post, cx);
        let cover_source = ProfileRiffCoverSourcePolicy::source(&post);
        let cell_size = size(px(self.cell_width), px(self.cell_height));
        let maximum_decode_pixel_dimension =
            ProfileThumbnailDecodePolicy::maximum_pixel_dimension(cell_size, window.scale_factor());

        let cover: AnyElement = match cover_source {
            Some(cover_source) => CleverImageUi::new(cover_source)
                .content_mode(ContentMode::Fill)
                .corner_radius(0.0)
                .persist_to_disk(is_own_post)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(maximum_decode_pixel_dimension)
                .cache_policy(PostMediaCachePolicy::image_policy(is_own_post))
                .into_any_element(),
            None => PostGridTextUi::new(post.clone()).into_any_element(),
        };
        let upload = post.shows_upload_overlay().then(|| {
            div().absolute().inset_0().child(PostMediaUploadOverlay::new(post.clone()).cell_size(cell_size))
        });
        let moderation = post
            .moderation_banner()
            .map(|banner| div().absolute().inset_0().child(PostModerationGridOverlay::new(banner)));

        div()
            .relative()
            .w(cell_size.width)
            .h(cell_size.height)
            .overflow_hidden()
            .child(div().absolute().inset_0().overflow_hidden().child(cover))
            .child(self.badges())
            .children(upload)
            .children(moderation)
    }
}

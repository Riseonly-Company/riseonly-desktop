use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Div, ElementId, Entity, FontWeight, IntoElement, ParentElement, Pixels, Point,
    Render, RenderOnce, SharedString, Size, StyleRefinement, Styled, Window, black, container_query, div, px, size,
    white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::ThemeStore;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::text_sizing::{GridTileTextFit, grid_title_font_size};
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
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
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageCustomEmojiEntity;
use crate::modules::gifts::components::gift_shop_shelf_grid::{GiftScrollWindowProbe, GiftShopShelfRows};
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::components::post_collection_ui::{PostPageStateUi, PostPaginationTriggerModifiers};
use crate::modules::post::components::post_moderation_disabled_overlay::PostModerationGridOverlay;
use crate::modules::post::components::repost::repost_overlay::{RepostSticker, RepostStickerStyle};
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostPageScope, RisePostProfileKind,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::components::shorts_zoom_namespace::{
    ShortsZoomNamespaceId, ShortsZoomNamespaceKey, ShortsZoomSource,
};
use crate::modules::story::components::stories_grid::stories_grid_view::{container_width_reader, use_container_width};
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::{
    ProfileGridMenuAsOwnerKey, ProfileGridPreviewSourcePolicy, ProfileGridSharedElementInputs,
    ProfileThumbnailDecodePolicy,
};
use crate::modules::user::components::profile_content::pages::grid_posts::profile_grid_shared_element_origin::{
    ProfileGridMetrics, ProfileGridSharedElementLayout, ProfileGridSharedElementOrigin,
};
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::{
    SharedElementReturnScrollerKey, SharedElementReturnScrollerRef,
};

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// rows a lazy grid would build near the screen, plus the probe that moves them
pub(crate) fn lazy_grid_mounted_range(
    id: &ElementId,
    item_count: i64,
    columns: i64,
    spacing: f32,
    row_height: f32,
    window: &mut Window,
    cx: &mut App,
) -> (Range<i64>, GiftScrollWindowProbe) {
    let row_count = GiftShopShelfRows::count(item_count, columns);
    let initial_rows = (f32::from(window.viewport_size().height) / (row_height + spacing).max(1.0)).ceil() as i64 + 1;
    let rows_state: Entity<Option<GiftShopShelfRows>> = window.use_keyed_state(child_id(id, "rows"), cx, |_, _| None);
    let range = rows_state
        .read(cx)
        .unwrap_or(GiftShopShelfRows::leading(initial_rows))
        .clamped(row_count)
        .items(columns, item_count);
    let probe = GiftScrollWindowProbe::new(child_id(id, "window-probe"), move |band, height, cx| {
        let Some(band) = band else { return };
        if height <= 0.0 {
            return;
        }
        rows_state.update(cx, |mounted, cx| {
            let next = GiftShopShelfRows::following(
                &band,
                height,
                row_count,
                spacing,
                mounted.unwrap_or(GiftShopShelfRows::leading(initial_rows)),
            );
            if Some(next) != *mounted {
                *mounted = Some(next);
                cx.notify();
            }
        });
    });
    (range, probe)
}

#[derive(IntoElement)]
pub struct GridReposts {
    id: ElementId,
    pub profile_tag: String,
    pub profile_user_id: String,
}

impl GridReposts {
    pub fn new(profile_tag: impl Into<String>, profile_user_id: impl Into<String>) -> Self {
        let profile_tag = profile_tag.into();
        let profile_user_id = profile_user_id.into();
        Self {
            id: ElementId::Name(SharedString::from(format!("grid-reposts-{profile_user_id}-{profile_tag}"))),
            profile_tag,
            profile_user_id,
        }
    }

    /// anchors and host probe only exist where a flight can run
    fn shared_elements_active() -> bool {
        SharedElementNavigationPolicy::flights_enabled()
            && SharedElementNavigationPolicy::ENABLED_KINDS.contains(&RisePostProfileKind::Reposts)
    }

    fn shared_element_inputs(&self, scroller: Option<SharedElementReturnScrollerRef>) -> ProfileGridSharedElementInputs {
        ProfileGridSharedElementInputs {
            user_id: self.profile_user_id.clone(),
            tag: self.profile_tag.clone(),
            kind: RisePostProfileKind::Reposts,
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
            RisePostProfileKind::Reposts,
            inputs.scroller.clone(),
            ProfileGridSharedElementLayout::Posts,
        );
    }

    fn row_count(count: usize, grid_width: f32) -> i64 {
        ProfileGridMetrics::row_count(count as i64, grid_width)
    }

    fn refresh(profile_user_id: &str, profile_tag: &str, cx: &mut App) {
        PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| {
                store.refresh_profile(profile_user_id, profile_tag, RisePostProfileKind::Reposts, cx)
            })
            .detach();
    }
}

impl RenderOnce for GridReposts {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let profile_grid_menu_as_owner = env::<ProfileGridMenuAsOwnerKey>(cx);
        let shorts_zoom_namespace = env::<ShortsZoomNamespaceKey>(cx);
        let return_scroller = env::<SharedElementReturnScrollerKey>(cx);
        let user_id = self.profile_user_id.clone();
        let tag = self.profile_tag.clone();
        let kind = RisePostProfileKind::Reposts;

        let content_owner_id = *window.use_keyed_state(child_id(&self.id, "owner"), cx, |_, _| Uuid::new_v4()).read(cx);
        let shared_element_origin = window
            .use_keyed_state(child_id(&self.id, "origin"), cx, |_, _| ProfileGridSharedElementOrigin::new())
            .read(cx)
            .clone();
        let width_state = use_container_width(child_id(&self.id, "width"), window, cx);
        let grid_width = *width_state.read(cx);

        let page = PostServicesStore::shared(cx)
            .update(cx, |service, cx| service.profile_page_with_tag(&user_id, &tag, kind, cx));
        let is_empty = page.items.is_empty();
        let rows = Self::row_count(page.items.len(), grid_width);
        let row_h = ProfileGridMetrics::row_height_container_width(grid_width);
        let grid_h = ProfileGridMetrics::grid_height(rows, row_h);
        let has_subject = !tag.is_empty() && !user_id.is_empty();
        let outer_height = ProfileGridMetrics::outer_height(grid_h, row_h, page.status, !is_empty, has_subject);
        let shared_element_surface = ProfileGridSharedElementOrigin::surface(&user_id, &tag, kind);
        let anchor_surface = Self::shared_elements_active().then(|| shared_element_surface.clone());

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
            format!("{}|{user_id}", tag.to_lowercase()),
            window,
            cx,
            async move |cx| {
                let settled = cx.update(|_, cx| {
                    SharedElementTransitionCoordinator::wait_until_settled(shared_element_surface, 1.0, cx)
                });
                if let Ok(settled) = settled {
                    settled.await;
                }
                if task_tag.is_empty() || task_user.is_empty() {
                    return;
                }
                let _ = cx.update(|_, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.observe_profile(&task_user, &task_tag, kind, content_owner_id, cx);
                    });
                });
            },
        );

        let parts = Rc::new(GridRepostsParts {
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
        let status = page.status;
        let page = Rc::new(page);
        let state = PostPageStateUi::new(
            child_id(&self.id, "state"),
            status,
            !is_empty,
            "arrow.2.squarepath",
            tr("profile_reposts_empty"),
            error_text,
            move |cx: &mut App| Self::refresh(&retry_user, &retry_tag, cx),
        )
        .has_subject(has_subject)
        .loading(move |_, _| {
            PostGridSkeleton::default()
                .columns(ProfileGridMetrics::columns(grid_width))
                .spacing(ProfileGridMetrics::SPACING)
                .cell_height(row_h)
                .rows(ProfileGridMetrics::SKELETON_ROWS)
        })
        .content(move |window, cx| parts.grid_body(page, grid_width, grid_h, anchor_surface, window, cx));

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

struct GridRepostsParts {
    id: ElementId,
    profile_tag: String,
    profile_user_id: String,
    nav_store: Option<Entity<NavigationStore>>,
    profile_grid_menu_as_owner: bool,
    shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    shared_element_origin: Rc<ProfileGridSharedElementOrigin>,
}

impl GridRepostsParts {
    fn grid_body(
        self: Rc<Self>,
        page: Rc<RisePostPagePresentation>,
        width: f32,
        height: f32,
        anchor_surface: Option<SharedElementSurface>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let w = width.max(0.0);
        let columns = ProfileGridMetrics::columns(w);
        let cell_w = ProfileGridMetrics::cell_width(w);
        let cell_h = ProfileGridMetrics::row_height(cell_w);
        let item_count = page.items.len() as i64;
        let (range, window_probe) = lazy_grid_mounted_range(
            &self.id,
            item_count,
            columns,
            ProfileGridMetrics::SPACING,
            cell_h,
            window,
            cx,
        );

        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        for index in range.start as usize..range.end as usize {
            let frame = SharedElementReturnScrollMath::grid_item_rect(
                index,
                columns as usize,
                cell_w,
                cell_h,
                ProfileGridMetrics::SPACING,
                0.0,
            );
            cells.push(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(self.cell(index, &page, cell_w, cell_h, anchor_surface.clone(), cx))
                    .into_any_element(),
            );
        }

        let host_probe = anchor_surface.is_some().then(|| {
            let container: Rc<dyn SharedElementOriginContainer> = self.shared_element_origin.clone();
            let origin = self.shared_element_origin.clone();
            SharedElementHostProbe::origin(container).on_layout(move |probe, window, _| {
                origin.record_columns(columns);
                origin.record_layout(probe, cell_w, None, 0.0, window);
            })
        });

        div()
            .relative()
            .w(px(w))
            .h(px(height))
            .children(host_probe)
            .child(env_scope::<SharedElementSurfaceKey>(anchor_surface).child(div().absolute().inset_0().children(cells)))
            .child(div().absolute().inset_0().child(window_probe))
            .into_any_element()
    }

    fn cell(
        self: &Rc<Self>,
        index: usize,
        page: &RisePostPagePresentation,
        cell_w: f32,
        cell_h: f32,
        anchor_surface: Option<SharedElementSurface>,
        cx: &mut App,
    ) -> AnyElement {
        let post = Rc::new(page.items[index].clone());
        let presentation_id = post.presentation_id();
        let actions = self.repost_actions(&post, cx);
        let opinion = self.repost_opinion(&post, cx);

        let content_post = post.clone();
        let action_post = post.clone();
        let action_parts = self.clone();
        let tap_post = post.clone();
        let tap_parts = self.clone();
        let load_parts = self.clone();
        let cell_view_key = SharedString::from(format!("profile-grid-repost-cell-view-{presentation_id}"));
        let menu = HoldContextMenuContainer::new(
            child_id(&self.id, format!("hold-{presentation_id}")),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let props = ProfileGridRepostCellProps {
                    post: content_post.clone(),
                    cell_width: cell_w,
                    cell_height: cell_h,
                    opinion: opinion.clone(),
                };
                ProfileGridRepostCellView::element(cell_view_key.clone(), props, window, cx)
                    .shared_element_anchor(
                        content_post.id.clone(),
                        ProfileGridSharedElementOrigin::part_kind(&content_post, RisePostProfileKind::Reposts),
                    )
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .tap_movement_tolerance(4.0)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            action_parts.handle_action(&action_post, &id, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            tap_parts.shared_element_origin.arm_open_if_possible(&tap_post, cx);
            tap_parts.open_repost(&tap_post, cx);
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

    fn repost_actions(&self, post: &GetPostFeedResponse, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut items = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if self.is_mine(post) || self.acts_as_repost_owner(cx) {
            items.push(
                HoldContextMenuAction::new("delete", tr("repost_delete_action"))
                    .system_image("trash")
                    .destructive(true),
            );
        }
        items
    }

    fn acts_as_repost_owner(&self, cx: &mut App) -> bool {
        !self.is_own_profile(cx) && AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author()
    }

    /// delete repost removes the viewers repost, so on another profile check is_reposted not the author
    fn is_mine(&self, post: &GetPostFeedResponse) -> bool {
        self.profile_grid_menu_as_owner || post.is_reposted
    }

    fn is_own_profile(&self, cx: &mut App) -> bool {
        self.profile_grid_menu_as_owner || AuthServicesStore::shared(cx).read(cx).is_current_user(&self.profile_user_id)
    }

    /// opinion shown in the cell bubble,
    /// the tab owners. on your own tab the viewer field wins since edits update it optimistically
    fn repost_opinion(&self, post: &GetPostFeedResponse, cx: &mut App) -> Option<ProfileGridRepostOpinion> {
        let own = trimming_whitespaces_and_newlines(post.my_repost_content.as_deref().unwrap_or_default());
        if self.is_own_profile(cx) && !own.is_empty() {
            return Some(ProfileGridRepostOpinion::new(own, post.my_repost_custom_emojis()));
        }
        let owner = trimming_whitespaces_and_newlines(post.reposter_content.as_deref().unwrap_or_default());
        if owner.is_empty() {
            return None;
        }
        Some(ProfileGridRepostOpinion::new(owner, post.reposter_custom_emojis()))
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
                kind: RisePostProfileKind::Reposts,
            }),
        )
    }

    fn open_repost(&self, post: &GetPostFeedResponse, cx: &mut App) {
        let route = self.route(post);
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn handle_action(&self, post: &GetPostFeedResponse, id: &str, cx: &mut App) {
        match id {
            "open" => {
                if self.shared_element_origin.arm_open_if_possible(post, cx) {
                    HoldContextMenuPresenter::dismiss(cx);
                }
                self.open_repost(post, cx);
            }
            "copy" => {
                let link = RiseonlyPublicLink::Post { id: post.id.clone() };
                if let Some(url) = link.web_url(AppEnvironment::current()) {
                    UIPasteboard::general().set_url(&url, cx);
                }
            }
            "share" => {
                let post = post.clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
            }
            "delete" => {
                let owner_id = (!self.is_mine(post)).then(|| self.profile_user_id.clone());
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.remove_repost(post, owner_id, cx));
            }
            _ => {}
        }
    }

    /// appending never moves earlier cells but still waits for a flight on this grid
    fn load_more(&self, deferred_until_settled: Option<SharedElementSurface>, cx: &mut App) {
        let user_id = self.profile_user_id.clone();
        let tag = self.profile_tag.clone();
        let load = move |cx: &mut App| {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_more_profile(&user_id, &tag, RisePostProfileKind::Reposts, cx);
            });
        };
        match deferred_until_settled {
            None => load(cx),
            Some(surface) => SharedElementTransitionCoordinator::run_or_defer_until_settled(surface, load, cx),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileGridRepostOpinion {
    pub text: String,
    pub custom_emojis: Vec<MessageCustomEmojiEntity>,
}

impl ProfileGridRepostOpinion {
    pub fn new(text: impl Into<String>, custom_emojis: Vec<MessageCustomEmojiEntity>) -> Self {
        Self { text: text.into(), custom_emojis }
    }
}

#[derive(IntoElement)]
struct ProfileGridRepostCell {
    post: Rc<GetPostFeedResponse>,
    cell_width: f32,
    cell_height: f32,
    opinion: Option<ProfileGridRepostOpinion>,
}

impl ProfileGridRepostCell {
    fn new(
        post: Rc<GetPostFeedResponse>,
        cell_width: f32,
        cell_height: f32,
        opinion: Option<ProfileGridRepostOpinion>,
    ) -> Self {
        Self { post, cell_width, cell_height, opinion }
    }

    fn corner_badge(icon: &str, top_trailing: bool) -> Div {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .when(top_trailing, |layer| layer.justify_end())
            .p(px(6.0))
            .child(div().p(px(6.0)).rounded_full().bg(black().opacity(0.42)).children(IconUi::sized(
                icon,
                px(12.0),
                white(),
                false,
            )))
    }
}

impl RenderOnce for ProfileGridRepostCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let post = &self.post;
        let is_own_original = PostMediaCachePolicy::is_own_post(post, cx);
        let preview = ProfileGridPreviewSourcePolicy::repost_source(
            post.local_preview_url.as_deref(),
            post.images.first().map(String::as_str),
            post.video_thumbnail.as_deref(),
            post.track.as_ref().and_then(|track| track.cover_url.as_deref()),
        );
        let cell_size = size(px(self.cell_width), px(self.cell_height));
        let maximum_decode_pixel_dimension =
            ProfileThumbnailDecodePolicy::bucketed_pixel_dimension(cell_size, window.scale_factor());

        let tile: AnyElement = match preview {
            Some(remote) => CleverImageUi::new(remote)
                .content_mode(ContentMode::Fill)
                .corner_radius(0.0)
                .persist_to_disk(is_own_original)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(maximum_decode_pixel_dimension)
                .cache_policy(PostMediaCachePolicy::image_policy(is_own_original))
                .into_any_element(),
            None => ProfileGridRepostTextUi::new(post.clone()).into_any_element(),
        };
        let kind_badge = if post.is_short {
            Some("play.fill")
        } else if post.is_image_reel {
            Some("photo.stack")
        } else {
            None
        };
        let sticker = self.opinion.map(|opinion| {
            div().absolute().inset_0().flex().items_end().justify_start().p(px(6.0)).child(
                RepostSticker::new(
                    ElementId::Name(SharedString::from(format!("profile-grid-repost-sticker-{}", post.id))),
                    opinion.text,
                    RepostStickerStyle::Grid,
                )
                .custom_emojis(opinion.custom_emojis),
            )
        });
        let moderation = post
            .moderation_banner()
            .map(|banner| div().absolute().inset_0().child(PostModerationGridOverlay::new(banner)));

        div()
            .relative()
            .w(cell_size.width)
            .h(cell_size.height)
            .overflow_hidden()
            .child(div().absolute().inset_0().overflow_hidden().child(tile))
            .children(kind_badge.map(|icon| Self::corner_badge(icon, true)))
            .when(post.track_id.is_some(), |cell| cell.child(Self::corner_badge("music.note", false)))
            .children(sticker)
            .children(moderation)
    }
}

/// what one grid cell draws, equal props keep the last frame
#[derive(Clone, PartialEq)]
struct ProfileGridRepostCellProps {
    post: Rc<GetPostFeedResponse>,
    cell_width: f32,
    cell_height: f32,
    opinion: Option<ProfileGridRepostOpinion>,
}

/// one grid cell as its own cached view
/// page frames replay it
/// text fit and images relayout only when post or size or opinion change
struct ProfileGridRepostCellView {
    props: Option<ProfileGridRepostCellProps>,
}

struct ProfileGridRepostCellHolder {
    cell: Entity<ProfileGridRepostCellView>,
}

impl ProfileGridRepostCellView {
    fn element(key: SharedString, props: ProfileGridRepostCellProps, window: &mut Window, cx: &mut App) -> AnyElement {
        let style = StyleRefinement::default().w(px(props.cell_width)).h(px(props.cell_height));
        let host = window
            .use_keyed_state(ElementId::Name(key), cx, |_, cx| ProfileGridRepostCellHolder {
                cell: cx.new(|_| Self { props: None }),
            })
            .read(cx)
            .cell
            .clone();
        let stale = host.update(cx, |host, _| host.set_props(props));
        if stale {
            AnyView::from(host).into_any_element()
        } else {
            host.cached(style).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, props: ProfileGridRepostCellProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }
}

impl Render for ProfileGridRepostCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) = self.props.as_ref().map_or((0.0, 0.0), |props| (props.cell_width, props.cell_height));
        let cell = self
            .props
            .clone()
            .map(|props| ProfileGridRepostCell::new(props.post, props.cell_width, props.cell_height, props.opinion));
        div().w(px(width)).h(px(height)).children(cell)
    }
}

/// text cell of the reposts tab, original title or text centered. the exit cover rasterizes it
#[derive(IntoElement)]
pub struct ProfileGridRepostTextUi {
    pub post: Rc<GetPostFeedResponse>,
}

impl ProfileGridRepostTextUi {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into() }
    }

    fn tile(post: &GetPostFeedResponse, tile_size: Size<Pixels>, window: &mut Window, cx: &mut App) -> AnyElement {
        let source = if post.title.is_empty() { &post.content } else { &post.title };
        let original_text = trimming_whitespaces_and_newlines(source);
        let theme_store = ThemeStore::shared(cx).read(cx);
        let delta = theme_store.text_delta();
        let background = theme_store.current_theme().bg._200;
        let fitted = GridTileTextFit::font_size(
            original_text,
            grid_title_font_size(if original_text.is_empty() { None } else { Some(original_text) }) + px(delta),
            px(10.0 + delta),
            FontWeight::NORMAL,
            tile_size,
            window,
        );
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(background)
            .child(
                MainText::new(original_text.to_owned())
                    .px(f32::from(fitted) - delta)
                    .tac(TextAlignT::Center)
                    .number_of_lines(4),
            )
            .into_any_element()
    }
}

impl RenderOnce for ProfileGridRepostTextUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let post = self.post;
        container_query(move |tile_size, window, cx| Self::tile(&post, tile_size, window, cx))
    }
}

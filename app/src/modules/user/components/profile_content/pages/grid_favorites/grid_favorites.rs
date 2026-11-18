use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, Entity, Pixels, Point, SharedString, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, use_task};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
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
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::gifts::components::gift_shop_shelf_grid::{GiftScrollWindowProbe, GiftShopShelfRows};
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::components::post_collection_ui::{PostPageStateUi, PostPaginationTriggerModifiers};
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostPageScope, RisePostProfileKind,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::components::shorts_zoom_namespace::{
    ShortsZoomNamespaceId, ShortsZoomNamespaceKey, ShortsZoomSource,
};
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileGridPostCell;
use crate::modules::user::components::profile_content::pages::grid_posts::profile_grid_shared_element_origin::{
    ProfileGridMetrics, ProfileGridSharedElementLayout, ProfileGridSharedElementOrigin,
};
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::SharedElementReturnScrollerKey;
use crate::modules::story::components::stories_grid::stories_grid_view::{initial_container_width, remember_container_width};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

pub(crate) type ProfilePostGridActions = Rc<dyn Fn(&GetPostFeedResponse) -> Vec<HoldContextMenuAction>>;
pub(crate) type ProfilePostGridExtraAction = Rc<dyn Fn(&str, &GetPostFeedResponse, &mut App)>;
pub(crate) type ProfilePostGridCellOverlay = Rc<dyn Fn(&GetPostFeedResponse) -> Option<AnyElement>>;

/// one post grid of a profile tab, page state and observation and shared element origin and cells
#[derive(IntoElement)]
pub(crate) struct ProfilePostGrid {
    pub id: ElementId,
    pub profile_tag: String,
    pub profile_user_id: String,
    pub kind: RisePostProfileKind,
    pub empty_icon: String,
    pub empty_text: String,
    pub actions: ProfilePostGridActions,
    pub on_extra_action: ProfilePostGridExtraAction,
    pub cell_overlay: Option<ProfilePostGridCellOverlay>,
}

impl RenderOnce for ProfilePostGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let user_id = self.profile_user_id.clone();
        let tag = self.profile_tag.clone();
        let kind = self.kind;
        let page = PostServicesStore::shared(cx)
            .update(cx, |service, cx| service.profile_page_with_tag(&user_id, &tag, kind, cx));
        let has_items = !page.items.is_empty();
        let has_subject = !user_id.is_empty();

        let initial_width = initial_container_width(window, cx);
        let width_state = window.use_keyed_state(child_id(&self.id, "width"), cx, move |_, _| initial_width);
        let grid_width = *width_state.read(cx);
        let rows = ProfileGridMetrics::row_count(page.items.len() as i64, grid_width);
        let row_height = ProfileGridMetrics::row_height_container_width(grid_width);
        let grid_height = ProfileGridMetrics::grid_height(rows, row_height);
        let outer_height =
            ProfileGridMetrics::outer_height(grid_height, row_height, page.status, has_items, has_subject);

        let surface = ProfileGridSharedElementOrigin::surface(&user_id, &tag, kind);
        let anchor_surface_active = SharedElementNavigationPolicy::flights_enabled()
            && SharedElementNavigationPolicy::ENABLED_KINDS.contains(&kind);

        let origin = window
            .use_keyed_state(child_id(&self.id, "origin"), cx, |_, _| ProfileGridSharedElementOrigin::new())
            .read(cx)
            .clone();
        origin.configure(
            &user_id,
            &tag,
            kind,
            env::<SharedElementReturnScrollerKey>(cx),
            ProfileGridSharedElementLayout::Posts,
        );

        let content_owner_id = *window.use_keyed_state(child_id(&self.id, "owner"), cx, |_, _| Uuid::new_v4()).read(cx);

        let task_user = user_id.clone();
        let task_tag = tag.clone();
        use_task(
            child_id(&self.id, "observe"),
            format!("{user_id}|{}", tag.to_lowercase()),
            window,
            cx,
            async move |cx| {
                let settled =
                    cx.update(|_, cx| SharedElementTransitionCoordinator::wait_until_settled(surface, 1.0, cx));
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

        let retry_user = user_id.clone();
        let retry_tag = tag.clone();
        let cell_height = row_height;
        let body = ProfilePostGridBody {
            id: child_id(&self.id, "grid"),
            profile_tag: tag.clone(),
            profile_user_id: user_id.clone(),
            kind,
            page: Rc::new(page.clone()),
            width: grid_width,
            height: grid_height,
            anchor_surface_active,
            nav_store: env::<NavigationStoreKey>(cx),
            shorts_zoom_namespace: env::<ShortsZoomNamespaceKey>(cx),
            origin,
            actions: self.actions,
            on_extra_action: self.on_extra_action,
            cell_overlay: self.cell_overlay,
        };
        let state = PostPageStateUi::new(
            child_id(&self.id, "state"),
            page.status,
            has_items,
            self.empty_icon,
            self.empty_text,
            page.error_message.clone().unwrap_or_else(|| tr("feed_load_error")),
            move |cx: &mut App| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.refresh_profile(&retry_user, &retry_tag, kind, cx))
                    .detach();
            },
        )
        .has_subject(has_subject)
        .loading(move |_, _| {
            PostGridSkeleton::default()
                .columns(ProfileGridMetrics::columns(grid_width))
                .spacing(ProfileGridMetrics::SPACING)
                .cell_height(cell_height)
                .rows(ProfileGridMetrics::SKELETON_ROWS)
        })
        .content(move |_, _| body);

        let reader_state = width_state;
        let stop_user = user_id;
        let stop_tag = tag;
        div()
            .relative()
            .w_full()
            .h(px(outer_height))
            .child(measure(child_id(&self.id, "width-reader"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                remember_container_width(width);
                reader_state.update(cx, |current, cx| {
                    if *current != width {
                        *current = width;
                        cx.notify();
                    }
                });
            }))
            .child(state)
            .on_disappear(child_id(&self.id, "disappear"), move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.stop_observing_profile(&stop_user, &stop_tag, kind, content_owner_id, cx);
                });
            })
    }
}

#[derive(IntoElement)]
struct ProfilePostGridBody {
    id: ElementId,
    profile_tag: String,
    profile_user_id: String,
    kind: RisePostProfileKind,
    page: Rc<RisePostPagePresentation>,
    width: f32,
    height: f32,
    anchor_surface_active: bool,
    origin: Rc<ProfileGridSharedElementOrigin>,
    nav_store: Option<Entity<NavigationStore>>,
    shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    actions: ProfilePostGridActions,
    on_extra_action: ProfilePostGridExtraAction,
    cell_overlay: Option<ProfilePostGridCellOverlay>,
}

impl ProfilePostGridBody {
    fn route(&self, post: &GetPostFeedResponse) -> RootRoute {
        let origin = RisePostPageScope::Profile {
            user_id: self.profile_user_id.clone(),
            tag: self.profile_tag.clone(),
            kind: self.kind,
        };
        RootRoute::post_content(post, Some(origin))
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

    fn handle_action(
        id: &str,
        post: &GetPostFeedResponse,
        route: RootRoute,
        nav_store: Option<&Entity<NavigationStore>>,
        origin: &Rc<ProfileGridSharedElementOrigin>,
        on_extra_action: &ProfilePostGridExtraAction,
        cx: &mut App,
    ) {
        match id {
            "open" => {
                if origin.arm_open_if_possible(post, cx) {
                    HoldContextMenuPresenter::dismiss(cx);
                }
                Self::open_post(route, nav_store, cx);
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
            _ => on_extra_action(id, post, cx),
        }
    }

    fn open_post(route: RootRoute, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn load_more(
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        deferred_surface: Option<SharedElementSurface>,
        cx: &mut App,
    ) {
        let user_id = user_id.to_owned();
        let tag = tag.to_owned();
        let load = move |cx: &mut App| {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_more_profile(&user_id, &tag, kind, cx);
            });
        };
        match deferred_surface {
            None => load(cx),
            Some(surface) => SharedElementTransitionCoordinator::run_or_defer_until_settled(surface, load, cx),
        }
    }

    fn cell(&self, index: usize, post: &GetPostFeedResponse, cell_width: f32, cell_height: f32) -> AnyElement {
        let rc_post = Rc::new(post.clone());
        let presentation_id = post.presentation_id();
        let route = self.route(post);
        let kind = self.kind;

        let actions = self.actions.clone();
        let actions_post = rc_post.clone();
        let content_post = rc_post.clone();
        let overlay = self.cell_overlay.clone();
        let action_post = rc_post.clone();
        let action_route = route.clone();
        let action_origin = self.origin.clone();
        let extra_action = self.on_extra_action.clone();
        let tap_post = rc_post;
        let action_nav_store = self.nav_store.clone();
        let tap_nav_store = self.nav_store.clone();
        let tap_origin = self.origin.clone();
        let load_user = self.profile_user_id.clone();
        let load_tag = self.profile_tag.clone();
        let load_surface = self
            .anchor_surface_active
            .then(|| ProfileGridSharedElementOrigin::surface(&self.profile_user_id, &self.profile_tag, kind));

        let menu = HoldContextMenuContainer::new(
            child_id(&self.id, format!("hold-{presentation_id}")),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions(&actions_post)),
            Rc::new(move |_: &mut Window, _: &mut App| {
                let cell = ProfileGridPostCell::new(content_post.clone(), cell_width, cell_height);
                div()
                    .relative()
                    .child(cell)
                    .children(overlay.as_ref().and_then(|overlay| overlay(&content_post)))
                    .shared_element_anchor(content_post.id.clone(), ProfileGridSharedElementOrigin::part_kind(&content_post, kind))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .tap_movement_tolerance(4.0)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_action(
                &id,
                &action_post,
                action_route.clone(),
                action_nav_store.as_ref(),
                &action_origin,
                &extra_action,
                cx,
            );
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            tap_origin.arm_open_if_possible(&tap_post, cx);
            Self::open_post(route.clone(), tap_nav_store.as_ref(), cx);
        }));
        self.grid_zoom_source(post)
            .body(menu)
            .post_pagination_trigger(
                child_id(&self.id, format!("pagination-{presentation_id}")),
                index as i64,
                self.page.items.len() as i64,
                self.page.has_more,
                self.page.is_loading_more,
                move |cx| Self::load_more(&load_user, &load_tag, kind, load_surface.clone(), cx),
            )
            .into_any_element()
    }
}

impl RenderOnce for ProfilePostGridBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let columns = ProfileGridMetrics::columns(self.width);
        let spacing = ProfileGridMetrics::SPACING;
        let cell_width = ProfileGridMetrics::cell_width(self.width);
        let cell_height = ProfileGridMetrics::row_height(cell_width);
        let item_count = self.page.items.len() as i64;
        let row_count = GiftShopShelfRows::count(item_count, columns);
        let initial_rows = (f32::from(window.viewport_size().height) / cell_height.max(1.0)).ceil() as i64 + 1;
        let rows_state: Entity<Option<GiftShopShelfRows>> =
            window.use_keyed_state(child_id(&self.id, "rows"), cx, |_, _| None);
        let range = rows_state
            .read(cx)
            .unwrap_or(GiftShopShelfRows::leading(initial_rows))
            .clamped(row_count)
            .items(columns, item_count);

        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        for index in range.start as usize..range.end as usize {
            let frame = SharedElementReturnScrollMath::grid_item_rect(
                index,
                columns as usize,
                cell_width,
                cell_height,
                spacing,
                0.0,
            );
            cells.push(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(self.cell(index, &self.page.items[index], cell_width, cell_height))
                    .into_any_element(),
            );
        }

        let window_probe = GiftScrollWindowProbe::new(child_id(&self.id, "window-probe"), move |band, height, cx| {
            let Some(band) = band else {
                return;
            };
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

        let surface = self
            .anchor_surface_active
            .then(|| ProfileGridSharedElementOrigin::surface(&self.profile_user_id, &self.profile_tag, self.kind));
        let host_probe = self.anchor_surface_active.then(|| {
            let container: Rc<dyn SharedElementOriginContainer> = self.origin.clone();
            let origin = self.origin.clone();
            SharedElementHostProbe::origin(container).on_layout(move |bounds, window, _| {
                origin.record_columns(columns);
                origin.record_layout(bounds, cell_width, None, 0.0, window);
            })
        });

        div()
            .relative()
            .w_full()
            .h(px(self.height))
            .children(host_probe)
            .child(env_scope::<SharedElementSurfaceKey>(surface).child(div().absolute().inset_0().children(cells)))
            .child(div().absolute().inset_0().child(window_probe))
    }
}

/// private shelves of the owner, saved and liked posts on one grid
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ProfilePostShelf {
    #[default]
    Favorites,
    Liked,
}

impl ProfilePostShelf {
    pub fn kind(self) -> RisePostProfileKind {
        match self {
            Self::Favorites => RisePostProfileKind::Favorites,
            Self::Liked => RisePostProfileKind::Liked,
        }
    }

    pub fn empty_icon(self) -> &'static str {
        match self {
            Self::Favorites => "bookmark",
            Self::Liked => "heart",
        }
    }

    fn remove_action(self) -> HoldContextMenuAction {
        match self {
            Self::Favorites => HoldContextMenuAction::new("favorite_remove", tr("favourite_remove"))
                .system_image("bookmark.slash")
                .destructive(true),
            Self::Liked => HoldContextMenuAction::new("like_remove", tr("ctx_unlike"))
                .system_image("heart.slash")
                .destructive(true),
        }
    }
}

#[derive(IntoElement)]
pub struct GridFavorites {
    id: ElementId,
    pub profile_tag: String,
    pub profile_user_id: String,
    pub shelf: ProfilePostShelf,
}

impl GridFavorites {
    pub fn new(id: impl Into<ElementId>, profile_tag: impl Into<String>, profile_user_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            profile_tag: profile_tag.into(),
            profile_user_id: profile_user_id.into(),
            shelf: ProfilePostShelf::Favorites,
        }
    }

    pub fn shelf(mut self, shelf: ProfilePostShelf) -> Self {
        self.shelf = shelf;
        self
    }

    fn shelf_actions(shelf: ProfilePostShelf) -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
            shelf.remove_action(),
        ]
    }
}

impl RenderOnce for GridFavorites {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shelf = self.shelf;
        ProfilePostGrid {
            id: self.id,
            profile_tag: self.profile_tag,
            profile_user_id: self.profile_user_id,
            kind: shelf.kind(),
            empty_icon: shelf.empty_icon().to_owned(),
            empty_text: tr("chat_empty_favourites_title"),
            actions: Rc::new(move |_: &GetPostFeedResponse| Self::shelf_actions(shelf)),
            on_extra_action: Rc::new(|id: &str, post: &GetPostFeedResponse, cx: &mut App| match id {
                "favorite_remove" if post.is_favorited => {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(post, cx));
                }
                "like_remove" if post.is_liked => {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(post, cx));
                }
                _ => {}
            }),
            cell_overlay: None,
        }
    }
}

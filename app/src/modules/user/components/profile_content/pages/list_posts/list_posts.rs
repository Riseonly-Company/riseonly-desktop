use std::cell::Cell;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, AppContext, Context, ElementId, Entity, IntoElement, ParentElement, Pixels, Render, RenderOnce,
    SharedString, StyleRefinement, Styled, WeakEntity, Window, canvas, div, px,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, use_task};

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::modules::gifts::components::gift_shop_shelf_grid::GiftScrollWindowProbe;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_skeleton::{PostCardSkeletonVariant, PostFeedSkeleton};
use crate::modules::post::components::post_collection_ui::{PostPageStateUi, PostPaginationTriggerModifiers};
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostProfileKind, RisePostViewSurface,
};
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::{
    ProfileGridMenuAsOwnerKey, ProfileGridPostMenu, ProfileOwnerResolvingKey,
};

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

#[derive(IntoElement)]
pub struct ListPosts {
    id: ElementId,
    pub profile_tag: String,
    pub profile_user_id: String,
    pub kind: RisePostProfileKind,
}

impl ListPosts {
    const LAZY_ROWS_STEP: usize = 6;
    const LAZY_ROWS_PER_REPORT: usize = 1;
    const LAZY_OVERSCAN: f32 = 800.0;
    const WINDOW_KEEP: f32 = 1200.0;
    const WINDOW_SLACK: f32 = 400.0;
    const ROW_SEPARATION: f32 = 11.0;

    pub fn new(
        profile_tag: impl Into<String>,
        profile_user_id: impl Into<String>,
        kind: RisePostProfileKind,
    ) -> Self {
        let profile_tag = profile_tag.into();
        let profile_user_id = profile_user_id.into();
        Self {
            id: ElementId::Name(SharedString::from(format!(
                "list-posts-{profile_user_id}-{profile_tag}-{}",
                kind.wire()
            ))),
            profile_tag,
            profile_user_id,
            kind,
        }
    }

    fn refresh(profile_user_id: &str, profile_tag: &str, kind: RisePostProfileKind, cx: &mut App) {
        PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.refresh_profile(profile_user_id, profile_tag, kind, cx))
            .detach();
    }
}

impl RenderOnce for ListPosts {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let profile_grid_menu_as_owner = env::<ProfileGridMenuAsOwnerKey>(cx);
        let profile_owner_resolving = env::<ProfileOwnerResolvingKey>(cx);
        let user_id = self.profile_user_id.clone();
        let tag = self.profile_tag.clone();
        let kind = self.kind;
        let content_owner_id = *window.use_keyed_state(child_id(&self.id, "owner"), cx, |_, _| Uuid::new_v4()).read(cx);

        let page = PostServicesStore::shared(cx)
            .update(cx, |service, cx| service.profile_page_with_tag(&user_id, &tag, kind, cx));
        let has_items = !page.items.is_empty();
        let is_threads = kind == RisePostProfileKind::Threads;

        let task_user = user_id.clone();
        let task_tag = tag.clone();
        use_task(
            child_id(&self.id, "observe"),
            (user_id.clone(), tag.to_lowercase(), kind),
            window,
            cx,
            async move |cx| {
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
        let error_text = page.error_message.clone().unwrap_or_else(|| tr("feed_load_error"));
        let status = page.status;
        let content = ListPostsRows {
            id: child_id(&self.id, "rows"),
            profile_tag: tag.clone(),
            profile_user_id: user_id.clone(),
            kind,
            page: Rc::new(page),
            menu_as_owner: profile_grid_menu_as_owner,
        };
        let state = PostPageStateUi::new(
            child_id(&self.id, "state"),
            status,
            has_items,
            if is_threads { "text.bubble" } else { "photo.on.rectangle.angled" },
            tr(if is_threads { "profile_threads_empty" } else { "profile_publications_empty" }),
            error_text,
            move |cx: &mut App| Self::refresh(&retry_user, &retry_tag, kind, cx),
        )
        .has_subject(!user_id.is_empty() || profile_owner_resolving)
        .loading(move |_, _| {
            PostFeedSkeleton::default().card_count(3).top_padding(0.0).fixed_variant(if is_threads {
                PostCardSkeletonVariant::Text
            } else {
                PostCardSkeletonVariant::Media { aspect: PostMediaBox::PLACEHOLDER_ASPECT }
            })
        })
        .content(move |_, _| content);

        let stop_user = user_id;
        let stop_tag = tag;
        div()
            .w_full()
            .min_h(px(220.0))
            .child(state)
            .on_disappear(child_id(&self.id, "disappear"), move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.stop_observing_profile(&stop_user, &stop_tag, kind, content_owner_id, cx);
                });
            })
    }
}

#[derive(IntoElement)]
struct ListPostsRows {
    id: ElementId,
    profile_tag: String,
    profile_user_id: String,
    kind: RisePostProfileKind,
    page: Rc<RisePostPagePresentation>,
    menu_as_owner: bool,
}

impl RenderOnce for ListPostsRows {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let border = ThemeStore::shared(cx).read(cx).current_theme().border._100;
        let mounted_state: Entity<usize> =
            window.use_keyed_state(child_id(&self.id, "mounted"), cx, |_, _| ListPosts::LAZY_ROWS_STEP);
        let item_count = self.page.items.len();
        let mounted = (*mounted_state.read(cx)).min(item_count);
        let window_state: Entity<ProfileListRowsWindow> =
            window.use_keyed_state(child_id(&self.id, "window"), cx, |_, _| ProfileListRowsWindow::default());
        let band = window_state.update(cx, |state, _| state.take_band_for_render());
        let mut row_top = Some(0.0_f32);

        let mut rows = div().relative().w_full().flex().flex_col().gap(px(5.0));
        for (index, post) in self.page.items.iter().take(mounted).enumerate() {
            let presentation_id = post.presentation_id();
            let load_user = self.profile_user_id.clone();
            let load_tag = self.profile_tag.clone();
            let kind = self.kind;
            let row_id = child_id(&self.id, format!("post-{presentation_id}"));
            let treat_as_own_post = ProfileGridPostMenu::owner_menu_applies(post, self.menu_as_owner);
            let host = mounted_state.downgrade();
            let row_view = window
                .use_keyed_state(child_id(&self.id, format!("row-view-{presentation_id}")), cx, {
                    let row_id = row_id.clone();
                    move |_, cx| ProfileListPostRowHolder {
                        row: cx.new(|_| ProfileListPostRowView::new(row_id, post.clone(), treat_as_own_post, host)),
                    }
                })
                .read(cx)
                .row
                .clone();
            row_view.update(cx, |view, cx| view.set_row(post, treat_as_own_post, cx));
            let measured = row_view.read(cx).height.get();
            let far = match (&band, row_top, measured) {
                (Some(shown), Some(top), Some(height)) => {
                    let bottom = top + f32::from(height);
                    bottom < *shown.start() - ListPosts::WINDOW_KEEP || top > *shown.end() + ListPosts::WINDOW_KEEP
                }
                _ => false,
            };
            row_top = row_top.zip(measured).map(|(top, height)| top + f32::from(height) + ListPosts::ROW_SEPARATION);
            let cell = match measured {
                Some(height) if far => div().w_full().h(height).into_any_element(),
                _ => ProfileListPostRowView::slot(&row_view, cx),
            };
            rows = rows
                .child(
                    cell.post_pagination_trigger(
                        child_id(&self.id, format!("pagination-{presentation_id}")),
                        index as i64,
                        item_count as i64,
                        self.page.has_more,
                        self.page.is_loading_more,
                        move |cx: &mut App| {
                            PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.load_more_profile(&load_user, &load_tag, kind, cx);
                            });
                        },
                    ),
                )
                .child(div().w_full().h(px(1.0)).bg(border));
        }

        let probe = GiftScrollWindowProbe::new(child_id(&self.id, "window-probe"), move |band, height, cx| {
            window_state.update(cx, |state, cx| state.observe_band(band.clone(), cx));
            let Some(band) = band else { return };
            if *band.end() + ListPosts::LAZY_OVERSCAN < height {
                return;
            }
            mounted_state.update(cx, |mounted, cx| {
                if *mounted < item_count {
                    *mounted = (*mounted + ListPosts::LAZY_ROWS_PER_REPORT).min(item_count);
                    cx.notify();
                }
            });
        });
        rows.child(div().absolute().inset_0().child(probe))
    }
}

/// band of rows the scroll shows as last reported by the probe,
/// none off screen means build every mounted row
#[derive(Default)]
struct ProfileListRowsWindow {
    band: Option<RangeInclusive<f32>>,
    picked_for: Option<RangeInclusive<f32>>,
}

impl ProfileListRowsWindow {
    fn take_band_for_render(&mut self) -> Option<RangeInclusive<f32>> {
        self.picked_for = self.band.clone();
        self.band.clone()
    }

    fn observe_band(&mut self, band: Option<RangeInclusive<f32>>, cx: &mut Context<Self>) {
        self.band = band;
        let moved = match (&self.band, &self.picked_for) {
            (Some(band), Some(picked)) => {
                (band.start() - picked.start()).abs() > ListPosts::WINDOW_SLACK
                    || (band.end() - picked.end()).abs() > ListPosts::WINDOW_SLACK
            }
            (None, None) => false,
            _ => true,
        };
        if moved {
            cx.notify();
        }
    }
}

struct ProfileListPostRowHolder {
    row: Entity<ProfileListPostRowView>,
}

/// one post row as its own cached view so a playing video doesnt relayout every row
struct ProfileListPostRowView {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    treat_as_own_post: bool,
    height: Rc<Cell<Option<Pixels>>>,
    host: WeakEntity<usize>,
}

impl ProfileListPostRowView {
    fn new(id: ElementId, post: GetPostFeedResponse, treat_as_own_post: bool, host: WeakEntity<usize>) -> Self {
        Self { id, post: Rc::new(post), treat_as_own_post, height: Rc::new(Cell::new(None)), host }
    }

    fn set_row(&mut self, post: &GetPostFeedResponse, treat_as_own_post: bool, cx: &mut Context<Self>) {
        if *self.post == *post && self.treat_as_own_post == treat_as_own_post {
            return;
        }
        self.post = Rc::new(post.clone());
        self.treat_as_own_post = treat_as_own_post;
        cx.notify();
    }

    /// cached row once its height is known, first render is plain and measures
    fn slot(row: &Entity<Self>, cx: &App) -> gpui::AnyElement {
        let view = AnyView::from(row.clone());
        match row.read(cx).height.get() {
            Some(height) => div()
                .w_full()
                .child(view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved())
                .into_any_element(),
            None => div().w_full().child(view).into_any_element(),
        }
    }
}

impl Render for ProfileListPostRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.height.clone();
        let host = self.host.clone();
        div()
            .relative()
            .w_full()
            .child(ProfileInlinePostRow::new(self.id.clone(), self.post.clone(), self.treat_as_own_post))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let measured = bounds.size.height;
                        let settled =
                            height.get().is_some_and(|last| (f32::from(last) - f32::from(measured)).abs() < 0.5);
                        if measured <= px(0.0) || settled {
                            return;
                        }
                        height.set(Some(measured));
                        window.defer(cx, move |_, cx| {
                            if let Some(host) = host.upgrade() {
                                host.update(cx, |_, cx| cx.notify());
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[derive(IntoElement)]
struct ProfileInlinePostRow {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    treat_as_own_post: bool,
}

impl ProfileInlinePostRow {
    fn new(id: ElementId, post: Rc<GetPostFeedResponse>, treat_as_own_post: bool) -> Self {
        Self { id, post, treat_as_own_post }
    }
}

impl RenderOnce for ProfileInlinePostRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let comments_post = self.post.clone();
        let avatar_post = self.post.clone();
        let like_post = self.post.clone();
        let fav_post = self.post.clone();
        Post::new(self.id, self.post)
            .feed_row(true)
            .view_surface(RisePostViewSurface::ProfileList)
            .treat_as_own_post(self.treat_as_own_post)
            .on_open_comments(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
            })
            .on_avatar_tap(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    let route = RootRoute::UserPage {
                        user_id: avatar_post.author.id.clone(),
                        tag: Some(avatar_post.author.tag.clone()),
                    };
                    nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .on_like(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx));
            })
            .on_fav(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&fav_post, cx));
            })
    }
}

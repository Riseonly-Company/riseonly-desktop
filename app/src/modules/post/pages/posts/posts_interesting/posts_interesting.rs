use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, ElementId, Entity, IntoElement, ListAlignment, ParentElement, Render, RenderOnce, SharedString,
    StyleRefinement, Styled, Subscription, Task, Window, div, prelude::*, px,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::ListUi;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stacks::vstack;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_skeleton::PostFeedSkeleton;
use crate::modules::post::components::post_feed_media_prefetch::PostFeedMediaPrefetchController;
use crate::modules::post::components::post_feed_column::PostFeedColumn;
use crate::modules::post::pages::posts::post_detail::profile_post_list_body::splice_list_rows;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

type PostRowAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct PostsInteresting {
    selected_post: Option<GetPostFeedResponse>,
}

impl PostsInteresting {
    pub fn new(selected_post: Option<GetPostFeedResponse>) -> Self {
        Self { selected_post }
    }
}

impl RenderOnce for PostsInteresting {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.with_global_id(ElementId::from("posts-interesting"), |global_id, window| {
            window.with_element_state(global_id, |state: Option<Entity<PostsInterestingView>>, _| {
                let view = state.unwrap_or_else(|| cx.new(PostsInterestingView::new));
                (view.clone(), view)
            })
        });
        view.update(cx, |view, cx| {
            let changed = view.selected_post.as_ref().map(|post| &post.id) != self.selected_post.as_ref().map(|post| &post.id);
            view.selected_post = self.selected_post;
            if changed {
                cx.notify();
            }
        });
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}

struct PostsInterestingView {
    selected_post: Option<GetPostFeedResponse>,
    pinned_copy: Option<GetPostFeedResponse>,
    media_prefetch: Rc<RefCell<PostFeedMediaPrefetchController>>,
    list: Rc<ListUi>,
    rendered_row_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl PostsInterestingView {
    const PINNED_ROW_ID: &'static str = "posts-interesting-pinned";

    fn new(cx: &mut Context<Self>) -> Self {
        let services = PostServicesStore::shared(cx);
        let interactions = PostInteractionsStore::shared(cx);
        let presenter = HoldContextMenuPresenter::shared(cx);
        let theme = rise_ui::theme(cx).clone();
        Self {
            selected_post: None,
            pinned_copy: None,
            media_prefetch: Rc::new(RefCell::new(PostFeedMediaPrefetchController::new(&Uuid::new_v4().uuid_string()))),
            list: Rc::new(ListUi::new(&theme, 0, ListAlignment::Top)),
            rendered_row_ids: Vec::new(),
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&interactions, |_, _, cx| cx.notify()),
                cx.observe(&presenter, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// tapped tile goes first, then the feed continues from the post after it
    fn items(selected: Option<&GetPostFeedResponse>, feed: &[GetPostFeedResponse]) -> Vec<GetPostFeedResponse> {
        let Some(selected) = selected else {
            return feed.to_vec();
        };
        match feed.iter().position(|item| item.id == selected.id) {
            Some(index) => feed[index + 1..].to_vec(),
            None => feed.to_vec(),
        }
    }

    fn post_row(
        post: Rc<GetPostFeedResponse>,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
        on_like: Option<PostRowAction>,
        on_fav: Option<PostRowAction>,
    ) -> AnyElement {
        let comments_post = post.clone();
        let avatar_post = post.clone();
        let like_post = post.clone();
        let fav_post = post.clone();
        let nav_store = nav_store.clone();
        let row = Post::new(SharedString::from(post.presentation_id()), post)
            .feed_row(true)
            .on_open_comments(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
            })
            .on_avatar_tap(move |_, cx| {
                let route = RootRoute::UserPage {
                    user_id: avatar_post.author.id.clone(),
                    tag: Some(avatar_post.author.tag.clone()),
                };
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .on_like(move |window, cx| match &on_like {
                Some(on_like) => on_like(window, cx),
                None => PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx)),
            })
            .on_fav(move |window, cx| match &on_fav {
                Some(on_fav) => on_fav(window, cx),
                None => PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&fav_post, cx)),
            });
        vstack(5.0)
            .w_full()
            .child(row)
            .child(div().w_full().h(px(1.0)).bg(theme.border._100))
            .into_any_element()
    }

    fn pinned_row(
        this: &Entity<Self>,
        sp: &GetPostFeedResponse,
        feed: &[GetPostFeedResponse],
        pinned_copy: Option<&GetPostFeedResponse>,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
    ) -> AnyElement {
        let in_feed = feed.iter().find(|item| item.id == sp.id);
        let is_in_feed = in_feed.is_some();
        let post = Rc::new(in_feed.or(pinned_copy).unwrap_or(sp).clone());
        let like_post = post.clone();
        let like_view = this.clone();
        let fav_post = post.clone();
        let fav_view = this.clone();
        Self::post_row(
            post,
            theme,
            nav_store,
            Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx));
                if is_in_feed {
                    return;
                }
                let mut copy = (*like_post).clone();
                copy.is_liked = !copy.is_liked;
                copy.likes_count = (copy.likes_count + if copy.is_liked { 1 } else { -1 }).max(0);
                like_view.update(cx, |view, cx| {
                    view.pinned_copy = Some(copy);
                    cx.notify();
                });
            })),
            Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&fav_post, cx));
                if is_in_feed {
                    return;
                }
                let mut copy = (*fav_post).clone();
                copy.is_favorited = !copy.is_favorited;
                copy.favorites_count = (copy.favorites_count + if copy.is_favorited { 1 } else { -1 }).max(0);
                fav_view.update(cx, |view, cx| {
                    view.pinned_copy = Some(copy);
                    cx.notify();
                });
            })),
        )
    }
}

impl Render for PostsInterestingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let this = cx.entity();
        let entity_id = cx.entity_id();
        let selected_post = self.selected_post.clone();
        let services = PostServicesStore::shared(cx);
        let (feed_items, relative_id, has_more, pinned_post) = services.update(cx, |store, cx| {
            (
                store.feed_items(cx),
                store.feed_relative_id(),
                store.feed_has_more(cx),
                selected_post.as_ref().filter(|post| !store.is_post_hidden(&post.id)).cloned(),
            )
        });
        let visible_items = Rc::new(Self::items(selected_post.as_ref(), &feed_items));
        let is_feed_empty = visible_items.is_empty() && pinned_post.is_none();
        let cache_key: SharedString = PostServicesStore::feed_cache_key().into();
        let status = if is_feed_empty {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.feed_status(cx))
        } else {
            SaiWsSliceStatus::Fulfilled
        };

        let has_pin = pinned_post.is_some();
        let mut row_ids = Vec::with_capacity(visible_items.len() + 1);
        if has_pin {
            row_ids.push(Self::PINNED_ROW_ID.to_owned());
        }
        row_ids.extend(visible_items.iter().map(GetPostFeedResponse::presentation_id));
        splice_list_rows(self.list.state(), &mut self.rendered_row_ids, &row_ids);

        let drain_items_empty = visible_items.is_empty();
        let drain_relative_id = relative_id.clone();
        on_change(
            ("posts-interesting-relative-id", entity_id),
            relative_id.clone(),
            true,
            window,
            cx,
            move |_, _, _, cx| {
                if drain_items_empty && has_more && drain_relative_id.is_some() {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_feed(cx));
                }
            },
        );

        let content_list = self.list.clone();
        let content_theme = theme.clone();
        let content_nav = nav_store.clone();
        let content_prefetch = self.media_prefetch.clone();
        let pinned_copy = self.pinned_copy.clone();
        let hold_is_active = HoldContextMenuPresenter::has_active_request(cx);
        let data = SaiDataRender::new(status, move |_window: &mut Window, cx: &mut App| {
            let top_inset = list_shell_floating_top_inset(cx);
            let feed_items = Rc::new(feed_items);
            let background = content_theme.bg._100;
            let rows = content_list.refreshable(
                "posts-interesting-list",
                |_, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_feed(cx));
                    Task::ready(())
                },
                move |row_index, _window, _cx| {
                    let pin_offset = usize::from(has_pin);
                    let top_margin = if row_index == 0 { top_inset } else { 0.0 };
                    let spacing = if row_index + 1 < visible_items.len() + pin_offset { 5.0 } else { 0.0 };
                    let row = if let (0, Some(sp)) = (row_index, pinned_post.as_ref()) {
                        Self::pinned_row(&this, sp, &feed_items, pinned_copy.as_ref(), &content_theme, &content_nav)
                    } else {
                        let offset = row_index - pin_offset;
                        let Some(post) = visible_items.get(offset) else {
                            return div().into_any_element();
                        };
                        let presentation_id = post.presentation_id();
                        let prefetch = content_prefetch.clone();
                        let prefetch_items = visible_items.clone();
                        Self::post_row(Rc::new(post.clone()), &content_theme, &content_nav, None, None)
                            .sai_pagination_prefetch(
                                SharedString::from(format!("posts-interesting-page-{presentation_id}")),
                                SaiPaginationPrefetchOptions::new(
                                    offset,
                                    prefetch_items.len(),
                                    cache_key.clone(),
                                    relative_id.clone(),
                                    has_more,
                                ),
                                |_, cx| {
                                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_feed(cx));
                                },
                            )
                            .on_appear(
                                SharedString::from(format!("posts-interesting-appear-{presentation_id}")),
                                move |_, cx| {
                                    prefetch.borrow_mut().update(
                                        "posts-interesting",
                                        offset as i64,
                                        &prefetch_items,
                                        cx,
                                    );
                                },
                            )
                            .into_any_element()
                    };
                    div().w_full().pt(px(top_margin)).pb(px(spacing)).child(row).into_any_element()
                },
            );
            div().relative().size_full().bg(background).child(rows).children(
                hold_is_active.then(|| div().absolute().inset_0().occlude()),
            )
        })
        .id("posts-interesting-data")
        .is_empty(is_feed_empty)
        .no_data_text(tr("no_posts"))
        .error_text(tr("feed_load_error"))
        .message_height_percent(40.0)
        .no_data_height_percent(25.0)
        .sai_data_render_loading({
            let background = theme.bg._100;
            move |_, cx| {
                scroll_view("posts-interesting-loading")
                    .scroll_disabled(true)
                    .shows_indicators(false)
                    .list_shell_scroll_under_chrome(cx)
                    .bg(background)
                    .child(PostFeedSkeleton::default().top_padding(0.0))
            }
        });

        let back_nav = nav_store;
        let disappear_prefetch = self.media_prefetch.clone();
        SettingsWrapperUi::new(tr("post_interesting"), move |_, cx| {
            if let Some(nav_store) = &back_nav {
                nav_store.update(cx, |store, cx| store.pop(cx));
            }
        })
        .content(PostFeedColumn::new(("posts-interesting-column", entity_id), data))
        .on_appear(("posts-interesting-root", entity_id), |_, cx| {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_initial_feed(cx));
        })
        .and_on_disappear(move |_, cx| disappear_prefetch.borrow_mut().cancel_all(cx))
    }
}

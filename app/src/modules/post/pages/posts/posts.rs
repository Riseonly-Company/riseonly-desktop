use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, Entity, IntoElement, ListAlignment, Render, SharedString, Subscription, Window,
    div, prelude::*,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::{EdgeState, ListUi, PaginationEdge, TabItem, TabsEvent, TabsUiState};
use rise_widgets::{PageHeaderUi, PagePresentation, PageStateUi, ScreenShellUi};

use crate::modules::post::components::post::{
    PostAction, PostCard, PostFeedSkeleton, PostHandler, PostSurface,
};
use crate::modules::post::engine::rise_post_rpc::{FeedKind, PostScope};
use crate::modules::post::stores::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::PostServicesStore;

const HEART_LINGER: Duration = Duration::from_millis(550);

pub struct Posts {
    interactions: PostInteractionsStore,
    services: Entity<PostServicesStore>,
    tabs: Entity<TabsUiState>,
    lists: Vec<ListUi>,
    active: FeedKind,
    heart: Option<SharedString>,
    lengths: Vec<usize>,
    _subscriptions: Vec<Subscription>,
}

impl Posts {
    pub fn new(
        interactions: PostInteractionsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = rise_ui::theme(cx as &App).clone();
        let services = interactions.services().clone();

        let tabs = cx.new(|cx| {
            TabsUiState::new(
                "feed.tabs",
                FeedKind::ALL
                    .iter()
                    .map(|kind| TabItem::new(kind.tab_id(), tr(kind.title_key())))
                    .collect(),
                cx,
            )
        });

        let mut subscriptions = vec![
            cx.subscribe(&tabs, |view, _, event: &TabsEvent, cx| {
                let TabsEvent::Selected(index) = event;
                view.select(*index, cx);
            }),
            cx.observe(&services, |view, _, cx| {
                view.sync_lengths(cx);
                cx.notify();
            }),
        ];

        let lists: Vec<ListUi> = FeedKind::ALL
            .iter()
            .map(|_| ListUi::new(&theme, 0, ListAlignment::Top))
            .collect();

        let entity = cx.entity();
        for (index, list) in lists.iter().enumerate() {
            let kind = FeedKind::ALL[index];
            list.on_edge(
                &entity,
                move |view: &Posts, cx: &App| {
                    let services = view.services.read(cx);
                    EdgeState {
                        has_more_top: false,
                        has_more_bottom: services.has_more(&PostScope::Feed(kind)),
                        top_in_flight: false,
                        bottom_in_flight: services.is_loading_more(&PostScope::Feed(kind)),
                    }
                },
                move |view: &mut Posts, edge, cx| {
                    if edge == PaginationEdge::Bottom {
                        view.interactions.load_more(PostScope::Feed(kind), cx);
                    }
                },
            );
        }

        interactions.activate_feed(PostScope::Feed(FeedKind::All));

        let _ = window;
        subscriptions.shrink_to_fit();

        let mut screen = Self {
            interactions,
            services,
            tabs,
            lists,
            active: FeedKind::All,
            heart: None,
            lengths: vec![0; FeedKind::ALL.len()],
            _subscriptions: subscriptions,
        };

        // A screen opened over rows that are already cached gets no notification
        // to size its lists from, and draws a Content state holding zero rows.
        screen.sync_lengths(cx);
        screen
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(kind) = FeedKind::ALL.get(index).copied() else {
            return;
        };

        self.active = kind;
        self.interactions.activate_feed(PostScope::Feed(kind));
        cx.notify();
    }

    fn sync_lengths(&mut self, cx: &mut Context<Self>) {
        let services = self.services.read(cx);

        for (index, kind) in FeedKind::ALL.iter().enumerate() {
            let length = services.items(&PostScope::Feed(*kind)).len();
            let previous = self.lengths[index];
            if length == previous {
                continue;
            }

            if length > previous {
                self.lists[index].appended(length - previous);
            } else {
                self.lists[index].replaced(length);
            }
            self.lengths[index] = length;
        }
    }

    fn index_of(&self, kind: FeedKind) -> usize {
        FeedKind::ALL
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or(0)
    }

    fn handler(&self) -> PostHandler<Self> {
        Rc::new(|view: &mut Self, action, row_id, _window, cx| {
            view.dispatch(action, row_id, cx);
        })
    }

    fn dispatch(&mut self, action: PostAction, row_id: String, cx: &mut Context<Self>) {
        let Some(post) = self
            .services
            .read(cx)
            .items(&PostScope::Feed(self.active))
            .iter()
            .find(|item| item.row_id() == row_id)
            .cloned()
        else {
            return;
        };

        match action {
            PostAction::Like => self.interactions.toggle_like(&post),
            PostAction::DoubleTapLike => {
                self.interactions.like_from_double_tap(&post);
                self.show_heart(row_id.into(), cx);
            }
            PostAction::Favorite => self.interactions.toggle_favorite(&post),
            PostAction::Repost => self.interactions.toggle_repost(&post),
            PostAction::Comments => self.interactions.open_comments(&post, cx),
            PostAction::OpenAuthor => self.interactions.open_profile(post.author.id.clone(), cx),
            PostAction::ShowLikes | PostAction::OpenOptions(_) | PostAction::OpenHashtag(_) => {}
        }
    }

    fn show_heart(&mut self, row_id: SharedString, cx: &mut Context<Self>) {
        self.heart = Some(row_id.clone());
        cx.notify();

        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(HEART_LINGER).await;
            let _ = view.update(cx, |view, cx| {
                if view.heart.as_ref() == Some(&row_id) {
                    view.heart = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn body(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let kind = self.active;
        let index = self.index_of(kind);
        let services = self.services.read(cx);
        let scope = PostScope::Feed(kind);
        let status = services
            .page(&scope)
            .map(|page| page.status)
            .unwrap_or_default();
        let has_items = !services.items(&scope).is_empty();

        match status.presentation(has_items) {
            PagePresentation::Loading => PostFeedSkeleton::render(theme).into_any_element(),
            PagePresentation::Empty => {
                PageStateUi::empty(theme, kind.empty_symbol(), tr(kind.empty_key()))
                    .into_any_element()
            }
            PagePresentation::Error => PageStateUi::error(
                theme,
                tr("feed_load_error"),
                move |view: &mut Self, _, _| {
                    view.interactions.refresh_feed(PostScope::Feed(kind));
                },
                cx,
            )
            .into_any_element(),
            PagePresentation::Content => self.list(theme, index, kind, cx).into_any_element(),
            PagePresentation::ContentWithRefreshError => {
                let pill = PageStateUi::refresh_error(
                    theme,
                    tr("feed_load_error"),
                    move |view: &mut Self, _, _| {
                        view.interactions.refresh_feed(PostScope::Feed(kind))
                    },
                    cx,
                )
                .into_any_element();

                PageStateUi::over_content(theme, self.list(theme, index, kind, cx), Some(pill))
                    .into_any_element()
            }
        }
    }

    fn list(
        &self,
        theme: &AppTheme,
        index: usize,
        kind: FeedKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let handler = self.handler();
        let services = self.services.clone();
        let heart = self.heart.clone();
        let theme = theme.clone();
        let entity = cx.entity();

        let now_ms = now_ms();

        div()
            .size_full()
            .child(self.lists[index].element(move |row, _window, cx| {
                let Some(post) = services.read(cx).item(&PostScope::Feed(kind), row).cloned()
                else {
                    return div().into_any_element();
                };

                let shows_heart = heart.as_deref() == Some(post.row_id());
                entity.update(cx, |_, cx| {
                    // The cap has to sit on a ROW's main axis: in a column a
                    // child's height is measured at its resolved `w_full` and
                    // only clamped to `max_w` afterwards, so the card wraps its
                    // text twice — once at the list's width to be measured, once
                    // at this width to be drawn — and paints past its own row.
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .w_full()
                                .max_w(theme.feed.content_width)
                                .flex()
                                .flex_col()
                                .child(PostCard::render(
                                    &theme,
                                    &post,
                                    PostSurface::FeedRow,
                                    now_ms,
                                    shows_heart,
                                    &handler,
                                    cx,
                                ))
                                .child(PostCard::separator(&theme)),
                        )
                        .into_any_element()
                })
            }))
    }
}

impl Render for Posts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App).clone();
        let body = self.body(&theme, cx);

        ScreenShellUi::new()
            .header(PageHeaderUi::new().center(self.tabs.clone()), &theme)
            .content(body)
            .render(&theme)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feed_kind_has_a_list_of_its_own() {
        assert_eq!(FeedKind::ALL.len(), 3);
    }

    #[test]
    fn the_heart_lingers_about_as_long_as_the_reference_animates_it() {
        assert_eq!(HEART_LINGER, Duration::from_millis(550));
    }

    #[test]
    fn the_clock_is_read_as_milliseconds_since_the_epoch() {
        assert!(now_ms() > 1_700_000_000_000);
    }
}

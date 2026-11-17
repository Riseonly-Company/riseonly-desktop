use std::cell::Cell;
use std::ops::{Range, RangeInclusive};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, AppContext, Context, ElementId, Entity, FontWeight, Pixels, Point, Render, SharedString, StyleRefinement,
    WeakEntity, Window, canvas,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::SkeletonShimmerExt;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, use_task};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouter;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::comment::components::comment::{CommentActionsRow, CommentReplyActionStyle, CommentRow};
use crate::modules::comment::components::native_list::comment_list_models::CommentRowMetrics;
use crate::modules::comment::components::comment_skeleton::{CommentRowSkeleton, CommentRowSkeletonStyle};
use crate::modules::comment::engine::rise_comment_engine_models::RiseCommentListSnapshot;
use crate::modules::comment::stores::comment_actions::comment_types::{GetCommentsResponse, UserCommentEntry};
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::comment::stores::comment_services::comment_services::CommentServicesStore;
use crate::modules::gifts::components::gift_shop_shelf_grid::GiftScrollWindowProbe;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_skeleton::{PostCardSkeleton, PostCardSkeletonVariant};
use crate::modules::post::engine::rise_post_engine_models::{RisePostPageStatus, RisePostViewSurface};
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn interactions(cx: &mut App) -> Entity<CommentInteractionsStore> {
    CommentInteractionsStore::shared(cx)
}

/// profile comments tab,
/// the commented post as a list card and the comment below it,
/// a reply shows the thread root first and indented
#[derive(IntoElement)]
pub struct ProfileComments {
    id: ElementId,
    pub profile_user_id: String,
}

impl ProfileComments {
    const PREFETCH_ROWS: usize = 8;
    const LAZY_FIRST_ROWS: usize = 2;
    const LAZY_ROWS_STEP: usize = 1;
    const LAZY_OVERSCAN: f32 = 800.0;

    pub fn new(id: impl Into<ElementId>, profile_user_id: impl Into<String>) -> Self {
        Self { id: id.into(), profile_user_id: profile_user_id.into() }
    }

    fn empty_state() -> AnyElement {
        ChatProfileEmptyState::new("bubble.left.and.bubble.right", tr("user_comments_empty")).into_any_element()
    }

    fn page_footer(
        &self,
        theme: &AppTheme,
        snapshot: &RiseCommentListSnapshot,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let page_failed = snapshot.error_text.is_some();
        let is_refreshing = snapshot.is_refreshing;
        let can_request = !snapshot.is_loading_more && !page_failed && !is_refreshing;
        let task_user = self.profile_user_id.clone();
        use_task(
            child_id(&self.id, "page-request"),
            format!("{}-{can_request}", snapshot.user_entries.len()),
            window,
            cx,
            async move |cx| {
                if !can_request {
                    return;
                }
                let _ = cx.update(|_, cx| {
                    interactions(cx).update(cx, |store, cx| store.load_more_user_comments(&task_user, cx));
                });
            },
        );
        let body = if page_failed && !is_refreshing {
            let user_id = self.profile_user_id.clone();
            hstack(8.0)
                .child(MainText::new(tr("error_fetching_data")).px(12.0).color(theme.text.secondary))
                .child(
                    plain(child_id(&self.id, "page-retry"))
                        .on_press(move |_, cx| {
                            interactions(cx).update(cx, |store, cx| store.load_more_user_comments(&user_id, cx));
                        })
                        .child(
                            hstack(6.0)
                                .px(px(13.0))
                                .h(px(34.0))
                                .bg(theme.bg._100)
                                .rounded_full()
                                .children(IconUi::sized("arrow.clockwise", px(12.0), theme.text.primary, false))
                                .child(
                                    MainText::new(tr("common_retry"))
                                        .px(13.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.text.primary),
                                ),
                        ),
                )
                .into_any_element()
        } else {
            progress_view().tint(theme.text.secondary).into_any_element()
        };
        div().w_full().min_h(px(34.0)).py(px(14.0)).flex().items_center().justify_center().child(body).into_any_element()
    }

    fn list_body(&self, snapshot: &RiseCommentListSnapshot, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let entries = &snapshot.user_entries;
        let count = entries.len();
        let has_more = snapshot.has_more;
        let is_loading_more = snapshot.is_loading_more;
        let page_failed = snapshot.error_text.is_some();
        let prefetch_index = count.saturating_sub(Self::PREFETCH_ROWS);
        let mounted_state: Entity<usize> =
            window.use_keyed_state(child_id(&self.id, "mounted"), cx, |_, _| Self::LAZY_FIRST_ROWS);
        let mounted = (*mounted_state.read(cx)).min(count);
        let shown_state: Entity<Option<Range<usize>>> =
            window.use_keyed_state(child_id(&self.id, "shown-rows"), cx, |_, _| None);
        let shown = shown_state.read(cx).clone();
        let mut heights: Vec<Rc<Cell<Option<Pixels>>>> = Vec::with_capacity(mounted);

        let rows: Vec<AnyElement> = entries
            .iter()
            .take(mounted)
            .enumerate()
            .map(|(index, entry)| {
                let row_id = child_id(&self.id, format!("row-{}", entry.comment.id));
                let appear_id = child_id(&self.id, format!("appear-{}", entry.comment.id));
                let appear_user = self.profile_user_id.clone();
                let host = mounted_state.downgrade();
                let row_view = window
                    .use_keyed_state(child_id(&self.id, format!("row-view-{}", entry.comment.id)), cx, move |_, cx| {
                        ProfileCommentRowHolder { row: cx.new(|_| ProfileCommentRowView::new(row_id, entry.clone(), host)) }
                    })
                    .read(cx)
                    .row
                    .clone();
                row_view.update(cx, |view, cx| view.set_entry(entry, cx));
                let height = row_view.read(cx).height.clone();
                heights.push(height.clone());
                if let (Some(row_height), Some(shown)) = (height.get(), shown.as_ref())
                    && !shown.contains(&index)
                {
                    return vstack(5.0)
                        .w_full()
                        .child(div().w_full().flex_none().h(row_height))
                        .child(div().w_full().h(px(1.0)).bg(theme.border._100))
                        .into_any_element();
                }
                vstack(5.0)
                    .w_full()
                    .child(ProfileCommentRowView::slot(&row_view, cx).on_appear(
                        appear_id,
                        move |_, cx| {
                            if !has_more || is_loading_more || page_failed || index < prefetch_index {
                                return;
                            }
                            interactions(cx).update(cx, |store, cx| store.load_more_user_comments(&appear_user, cx));
                        },
                    ))
                    .child(div().w_full().h(px(1.0)).bg(theme.border._100))
                    .into_any_element()
            })
            .collect();

        let footer = (has_more && mounted == count).then(|| self.page_footer(&theme, snapshot, window, cx));
        let heights: Rc<[Rc<Cell<Option<Pixels>>>]> = heights.into();
        let probe = GiftScrollWindowProbe::new(child_id(&self.id, "window-probe"), move |band, height, cx| {
            let Some(band) = band else {
                return;
            };
            let next = Self::shown_rows(&heights, &band);
            shown_state.update(cx, |shown, cx| {
                if shown.as_ref() != Some(&next) {
                    *shown = Some(next);
                    cx.notify();
                }
            });
            if *band.end() + Self::LAZY_OVERSCAN < height {
                return;
            }
            mounted_state.update(cx, |mounted, cx| {
                if *mounted < count {
                    *mounted = (*mounted + Self::LAZY_ROWS_STEP).min(count);
                    cx.notify();
                }
            });
        });
        div()
            .relative()
            .w_full()
            .child(vstack(5.0).w_full().children(rows).children(footer))
            .child(div().absolute().inset_0().child(probe))
            .into_any_element()
    }

    fn shown_rows(heights: &[Rc<Cell<Option<Pixels>>>], band: &RangeInclusive<f32>) -> Range<usize> {
        const ESTIMATED_ROW: f32 = 400.0;
        const BLOCK_EXTRA: f32 = 6.0;
        const GAP: f32 = 5.0;
        let low = *band.start() - Self::LAZY_OVERSCAN;
        let high = *band.end() + Self::LAZY_OVERSCAN;
        let mut top = 0.0_f32;
        let mut first = None;
        let mut last = 0;
        for (index, height) in heights.iter().enumerate() {
            let bottom = top + height.get().map_or(ESTIMATED_ROW, f32::from) + BLOCK_EXTRA;
            if top > high {
                break;
            }
            if bottom >= low {
                first.get_or_insert(index);
                last = index + 1;
            }
            top = bottom + GAP;
        }
        match first {
            Some(first) => first..last,
            None => 0..0,
        }
    }

    fn hidden_state(theme: &AppTheme) -> AnyElement {
        vstack(10.0)
            .w_full()
            .pt(px(60.0))
            .px(px(24.0))
            .children(IconUi::sized("lock.fill", px(28.0), theme.text.secondary, true))
            .child(
                MainText::new(tr("user_comments_hidden"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
            .into_any_element()
    }

    fn error_state(&self, theme: &AppTheme) -> AnyElement {
        let user_id = self.profile_user_id.clone();
        vstack(12.0)
            .w_full()
            .pt(px(60.0))
            .child(MainText::new(tr("feed_load_error")).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary))
            .child(
                plain(child_id(&self.id, "reload"))
                    .on_press(move |_, cx| {
                        interactions(cx).update(cx, |store, cx| store.reload_user_comments(&user_id, cx));
                    })
                    .child(
                        MainText::new(tr("privacy_settings_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
            .into_any_element()
    }
}

impl RenderOnce for ProfileComments {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let user_id = self.profile_user_id.clone();
        let snapshot =
            CommentServicesStore::shared(cx).update(cx, |service, cx| service.user_comments_snapshot(&user_id, cx));

        let body = if snapshot.is_private {
            Self::hidden_state(&theme)
        } else if user_id.is_empty() {
            Self::empty_state()
        } else if snapshot.shows_initial_loader() {
            ProfileCommentsSkeleton.into_any_element()
        } else if snapshot.shows_blocking_error() {
            self.error_state(&theme)
        } else if snapshot.user_entries.is_empty() {
            Self::empty_state()
        } else {
            self.list_body(&snapshot, window, cx)
        };

        let load_user = user_id.clone();
        use_task(child_id(&self.id, "load"), user_id.clone(), window, cx, async move |cx| {
            if load_user.is_empty() {
                return;
            }
            let _ = cx.update(|_, cx| {
                interactions(cx).update(cx, |store, cx| store.load_user_comments_if_needed(&load_user, cx));
            });
        });

        let appear_user = user_id.clone();
        let disappear_user = user_id;
        div()
            .w_full()
            .min_h(px(220.0))
            .child(body)
            .on_appear(child_id(&self.id, "observe"), move |_, cx| {
                interactions(cx).update(cx, |store, cx| store.observe_user_comments(&appear_user, cx));
            })
            .and_on_disappear(move |_, cx| {
                interactions(cx).update(cx, |store, cx| store.stop_observing_user_comments(&disappear_user, cx));
            })
    }
}

#[derive(IntoElement)]
struct ProfileCommentsSkeleton;

impl RenderOnce for ProfileCommentsSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let border = rise_ui::theme(cx).border._100;
        let rows = (0..2_i64).map(|index| {
            let variant = if index == 0 {
                PostCardSkeletonVariant::Media { aspect: PostMediaBox::PLACEHOLDER_ASPECT }
            } else {
                PostCardSkeletonVariant::Text
            };
            vstack(5.0)
                .w_full()
                .child(PostCardSkeleton::default().variant(variant).alternation_index(index).shimmers(false))
                .child(CommentRowSkeleton::default().style(CommentRowSkeletonStyle::Sheet).alternation_index(index))
                .child(div().w_full().h(px(1.0)).bg(border))
        });
        div().size_full().overflow_hidden().child(vstack(5.0).w_full().children(rows).skeleton_shimmer(None))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    Root,
    Reply,
}

/// one comment row as its own cached view
/// playing video would redraw it every frame
/// renders only when entry or stores or width change
struct ProfileCommentRowView {
    id: ElementId,
    entry: Rc<UserCommentEntry>,
    height: Rc<Cell<Option<Pixels>>>,
    host: WeakEntity<usize>,
}

struct ProfileCommentRowHolder {
    row: Entity<ProfileCommentRowView>,
}

impl ProfileCommentRowView {
    fn new(id: ElementId, entry: UserCommentEntry, host: WeakEntity<usize>) -> Self {
        Self { id, entry: Rc::new(entry), height: Rc::new(Cell::new(None)), host }
    }

    fn set_entry(&mut self, entry: &UserCommentEntry, cx: &mut Context<Self>) {
        if *self.entry == *entry {
            return;
        }
        self.entry = Rc::new(entry.clone());
        cx.notify();
    }

    /// cached row once its height is known, the first render is plain and measures
    fn slot(row: &Entity<Self>, cx: &App) -> AnyElement {
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

impl Render for ProfileCommentRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.height.clone();
        let host = self.host.clone();
        div()
            .relative()
            .w_full()
            .child(ProfileCommentRow { id: self.id.clone(), entry: self.entry.clone() })
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let measured = bounds.size.height;
                        if measured <= px(0.0) || height.get() == Some(measured) {
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
struct ProfileCommentRow {
    id: ElementId,
    entry: Rc<UserCommentEntry>,
}

/// what a row handlers need after the frame that built them
#[derive(Clone)]
struct ProfileCommentRowActions {
    entry: Rc<UserCommentEntry>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ProfileCommentRowActions {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn handle_action(&self, id: &str, comment: &GetCommentsResponse, cx: &mut App) {
        match id {
            "copy" => UIPasteboard::general().set_string(&comment.content, cx),
            "delete" => {
                interactions(cx).update(cx, |store, cx| store.delete_user_comment(&comment.id, cx));
            }
            "report" => {
                if let Ok(comment_id) = comment.id.parse::<i64>() {
                    ReportSheetStore::shared(cx)
                        .update(cx, |sheet, cx| sheet.present(ReportTarget::Comment(comment_id), None, None, cx));
                }
            }
            _ => {}
        }
    }

    /// opens the comments sheet focused on the comment,
    /// without a post card opens the detail page with that focus
    fn open_comments(&self, comment: &GetCommentsResponse, replying: bool, cx: &mut App) {
        if let Some(post) = &self.entry.post_card {
            interactions(cx).update(cx, |store, cx| {
                store.open_comments_sheet(post, Some(comment.id.as_str()), false, replying, cx);
            });
            return;
        }
        self.open_post(Some(comment.id.as_str()), replying, cx);
    }

    fn open_post(&self, focus_comment_id: Option<&str>, replying: bool, cx: &mut App) {
        let post_id = self.entry.comment.post_id.clone();
        if post_id.is_empty() {
            return;
        }
        let comment_id = focus_comment_id.unwrap_or(self.entry.comment.id.as_str());
        interactions(cx).update(cx, |store, cx| {
            store.arm_comment_focus(&post_id, Some(comment_id), replying, cx);
        });
        self.push(RootRoute::PostDetail { post_id: Some(post_id), origin: None }, cx);
    }

    fn author_target(comment: &GetCommentsResponse) -> Option<(String, Option<String>)> {
        let stable = trimming_whitespaces_and_newlines(&comment.author_id);
        let nested = comment.author.id.as_deref().map_or("", trimming_whitespaces_and_newlines);
        let user_id = if stable.is_empty() { nested } else { stable };
        if user_id.is_empty() {
            return None;
        }
        let tag = (!comment.author.tag.is_empty()).then(|| comment.author.tag.clone());
        Some((user_id.to_owned(), tag))
    }

    fn open_author_profile(&self, comment: &GetCommentsResponse, cx: &mut App) {
        if let Some((user_id, tag)) = Self::author_target(comment) {
            self.push(RootRoute::UserPage { user_id: Some(user_id), tag }, cx);
        }
    }

    fn open_author_story(&self, comment: &GetCommentsResponse, cx: &mut App) {
        let Some((user_id, tag)) = Self::author_target(comment) else {
            return;
        };
        let actions = self.clone();
        let unavailable_user = user_id.clone();
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.open_stories_for_user_id(
                &user_id,
                Some(Rc::new(move |cx: &mut App| {
                    let route = RootRoute::UserPage { user_id: Some(unavailable_user.clone()), tag: tag.clone() };
                    actions.push(route, cx);
                })),
                cx,
            );
        });
    }
}

impl ProfileCommentRow {
    fn is_mine(comment: &GetCommentsResponse, me: &str) -> bool {
        !me.is_empty() && (comment.author_id == me || comment.author.id.as_deref() == Some(me))
    }

    fn hold_actions(comment: &GetCommentsResponse, own_comment_id: &str, me: &str) -> Vec<HoldContextMenuAction> {
        let mut items = vec![HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc")];
        if Self::is_mine(comment, me) {
            if comment.id == own_comment_id {
                items.push(
                    HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true),
                );
            }
        } else {
            items.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        items
    }

    fn comment_cell(
        &self,
        comment: &GetCommentsResponse,
        placement: Placement,
        me: &str,
        shows_dislike_counts: bool,
        actions: &ProfileCommentRowActions,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let suffix = if placement == Placement::Root { "root" } else { "reply" };
        let comment = Rc::new(comment.clone());
        let menu_items = Self::hold_actions(&comment, &self.entry.comment.id, me);

        let action_this = actions.clone();
        let action_comment = comment.clone();
        let tap_this = actions.clone();
        let tap_comment = comment.clone();
        let double_tap_gate = comment.clone();
        let double_tap_comment = comment.clone();
        let row_comment = comment.clone();
        let row_this = actions.clone();
        let row_id = child_id(&self.id, format!("comment-{suffix}-{}", comment.id));

        let hold = HoldContextMenuContainer::new(
            child_id(&self.id, format!("hold-{suffix}-{}", comment.id)),
            HoldContextMenuAlignment::Trailing,
            Rc::new(move |_: &App| menu_items.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                let profile_this = row_this.clone();
                let profile_comment = row_comment.clone();
                let story_this = row_this.clone();
                let story_comment = row_comment.clone();
                CommentRow::new(row_id.clone(), (*row_comment).clone())
                    .show_actions(false)
                    .on_avatar_profile(move |_, cx| profile_this.open_author_profile(&profile_comment, cx))
                    .on_avatar_story(move |_, cx| story_this.open_author_story(&story_comment, cx))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .uses_flat_chrome(true)
        .lifted_background(Some(theme.bg._100))
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            action_this.handle_action(&id, &action_comment, cx);
        }))
        .on_tap(Rc::new(move |point: Point<Pixels>, window: &mut Window, cx: &mut App| {
            if BubbleTapRouter::dispatch(point, None, window, cx) {
                return;
            }
            tap_this.open_comments(&tap_comment, false, cx);
        }))
        .can_double_tap(Rc::new(move |_: &App| !double_tap_gate.is_temp && !double_tap_gate.is_moderation_disabled()))
        .on_double_tap(Some(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            let _ = impact(ImpactStyle::Light);
            interactions(cx).update(cx, |store, cx| store.like_comment_from_double_tap(&double_tap_comment, cx));
        })));

        let reply_action_style = match placement {
            Placement::Root => CommentReplyActionStyle::Thread { count: comment.replies_count },
            Placement::Reply => CommentReplyActionStyle::Reply,
        };
        let like_comment = comment.clone();
        let dislike_comment = comment.clone();
        let reply_comment = comment.clone();
        let reply_this = actions.clone();
        let actions_row = CommentActionsRow::new(
            child_id(&self.id, format!("comment-actions-{suffix}-{}", comment.id)),
            (*comment).clone(),
        )
        .reply_action_style(reply_action_style)
        .shows_dislike_count(shows_dislike_counts)
        .on_like(move |_, cx| {
            interactions(cx).update(cx, |store, cx| store.toggle_like_on_comment(&like_comment, cx));
        })
        .on_dislike(move |_, cx| {
            interactions(cx).update(cx, |store, cx| store.toggle_dislike_on_comment(&dislike_comment, cx));
        })
        .on_reply(move |_, cx| reply_this.open_comments(&reply_comment, true, cx));

        vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .when(placement == Placement::Reply, |cell| cell.pl(px(22.0)))
            .child(div().w_full().flex().flex_col().child(hold))
            .child(
                div()
                    .w_full()
                    .pl(px(CommentRowMetrics::ACTIONS_LEADING_PADDING))
                    .pr(px(CommentRowMetrics::HORIZONTAL_PADDING))
                    .pb(px(8.0))
                    .child(actions_row),
            )
    }

    fn post_element(&self, post: GetPostFeedResponse, actions: &ProfileCommentRowActions) -> AnyElement {
        let post = Rc::new(post);
        let comments_post = post.clone();
        let avatar_post = post.clone();
        let avatar_this = actions.clone();
        let like_post = post.clone();
        let fav_post = post.clone();
        Post::new(child_id(&self.id, "post"), post)
            .feed_row(true)
            .view_surface(RisePostViewSurface::ProfileList)
            .on_open_comments(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
            })
            .on_avatar_tap(move |_, cx| {
                let route = RootRoute::UserPage {
                    user_id: avatar_post.author.id.clone(),
                    tag: Some(avatar_post.author.tag.clone()),
                };
                avatar_this.push(route, cx);
            })
            .on_like(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx));
            })
            .on_fav(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&fav_post, cx));
            })
            .into_any_element()
    }

    fn unavailable_post_line(&self, theme: &AppTheme, actions: &ProfileCommentRowActions) -> AnyElement {
        let actions = actions.clone();
        plain(child_id(&self.id, "post-unavailable"))
            .w_full()
            .px(px(12.0))
            .pt(px(12.0))
            .pb(px(4.0))
            .on_press(move |_, cx| actions.open_post(None, false, cx))
            .child(
                MainText::new(tr("user_comments_post_unavailable"))
                    .px(13.0)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            )
            .into_any_element()
    }
}

impl RenderOnce for ProfileCommentRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let entry = self.entry.clone();
        let post_id = entry.comment.post_id.clone();
        let actions = ProfileCommentRowActions { entry: entry.clone(), nav_store: env::<NavigationStoreKey>(cx) };

        let service = PostServicesStore::shared(cx);
        let resolved_post = service
            .update(cx, |service, cx| service.resolved_post(&post_id, cx))
            .or_else(|| entry.post_card.clone());
        let post_is_gone = matches!(
            service.update(cx, |service, cx| service.detail_page(&post_id, cx)).status,
            RisePostPageStatus::Loaded | RisePostPageStatus::Failed
        );

        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let shows_dislike_counts = entry.post_card.as_ref().is_none_or(|post| {
            post.shows_count(|visibility| visibility.comment_dislikes, viewer.as_deref())
        });
        let me = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();

        let post_element = match resolved_post {
            Some(post) => self.post_element(post, &actions),
            None if post_is_gone => self.unavailable_post_line(&theme, &actions),
            None => PostCardSkeleton::default().variant(PostCardSkeletonVariant::Text).into_any_element(),
        };
        let parent_cell = entry
            .parent
            .as_ref()
            .map(|parent| self.comment_cell(parent, Placement::Root, &me, shows_dislike_counts, &actions, &theme));
        let own_placement = if entry.parent.is_none() { Placement::Root } else { Placement::Reply };
        let own_cell = self.comment_cell(&entry.comment, own_placement, &me, shows_dislike_counts, &actions, &theme);

        let detail_owner_id =
            *window.use_keyed_state(child_id(&self.id, "detail-owner"), cx, |_, _| Uuid::new_v4()).read(cx);
        let observes_detail = entry.post_card.is_none() && !post_id.is_empty();
        let task_post = post_id.clone();
        use_task(child_id(&self.id, "detail"), post_id.clone(), window, cx, async move |cx| {
            if !observes_detail {
                return;
            }
            let _ = cx.update(|_, cx| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.observe_post_detail(&task_post, detail_owner_id, cx));
            });
        });

        vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(post_element)
            .children(parent_cell)
            .child(own_cell)
            .on_disappear(child_id(&self.id, "detail-stop"), move |_, cx| {
                if observes_detail {
                    PostInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.stop_observing_post_detail(&post_id, detail_owner_id, cx));
                }
            })
    }
}

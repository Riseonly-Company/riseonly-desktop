use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, BACKDROP_BLUR_SUPPORTED, Bounds, Context, Corners, ElementId, Empty,
    Entity, FontWeight, IntoElement, Keystroke, KeystrokeEvent, Pixels, Point, Render, RenderOnce, StyleRefinement,
    Subscription, Window, canvas, px, relative, transparent_black,
};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{AppTheme, ShellMetrics, ThemeStore};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::scroll_edge_blur_ui::{ScrollEdgeBlurFalloff, ScrollEdgeBlurUi};
use rise_ui::scroll_chrome_shadow_ui::{
    ScrollChromeShadowStyle, ScrollChromeShadowStyleAnchor, ScrollChromeShadowUi,
};
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::environment_values::accessibility_reduce_transparency;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::transition::UnitPoint;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_page_back_swipe_gate::{
    BottomSheetPageBackSwipeCallbacks, BottomSheetPageBackSwipeConfiguration, BottomSheetPageBackSwipeGate,
    BottomSheetPageBackSwipePolicy,
};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouter;
use crate::modules::chat::components::chat::bubble::chat_mention_tap_layer::ChatLinkOpen;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::StickerHoldPreviewPresenter;
use crate::modules::comment::components::comment::{CommentActionsRow, CommentReplyActionStyle, CommentRow};
use crate::modules::comment::components::comment_skeleton::{CommentListSkeleton, CommentRowSkeletonStyle};
use crate::modules::comment::components::native_list::comment_list_models::{
    CommentListRow, CommentListRowKind, CommentListRowPlacement, CommentRowMetrics,
};
use crate::modules::comment::components::native_list::comment_hosted_row_view::CommentRowFacts;
use crate::modules::comment::components::native_list::comment_native_list_view::CommentNativeListView;
use crate::modules::comment::engine::rise_comment_engine_models::{RiseCommentListSnapshot, RiseCommentScope};
use crate::modules::comment::engine::rise_comment_presentation::{RiseCommentListBox, RiseCommentPresentation};
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentPhotoUpload, CommentSortType, GetCommentsResponse, RepliesSortType,
};
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::comment::stores::comment_services::comment_services::CommentServicesStore;
use crate::modules::comment::stores::comment_services::comments_sheet_motion_relay::CommentsSheetMotionRelay;
use crate::modules::comment::widgets::comment_input_hooks::comment_input_hooks;
use crate::modules::comment::widgets::comments_sheet_aside_hooks::comments_sheet_aside_hooks;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::engine::rise_post_engine_models::RisePostSearchKind;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::components::shorts_comments_header::ShortsCommentsHeader;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

const ESCAPE_KEY: &str = "escape";
const EXPANDED_SNAP_FRACTION: f32 = 0.92;

#[derive(IntoElement, Default)]
pub struct CommentsSheet {
    presentation_override: Option<Binding<bool>>,
    preview_comments: Option<Vec<GetCommentsResponse>>,
    docked: bool,
}

/// stable handles of the sheet
/// a hosted row outlives the frame that built it so changing stuff is read live through these
#[derive(Clone)]
struct CommentsSheetScope {
    state: Entity<CommentsSheetState>,
    interactions: Entity<CommentInteractionsStore>,
    services: Entity<CommentServicesStore>,
    post: Entity<PostServicesStore>,
    nav_store: Option<Entity<NavigationStore>>,
    presentation_override: Option<Binding<bool>>,
    preview_comments: Option<Rc<Vec<GetCommentsResponse>>>,
    docked: bool,
}

struct CommentsSheetRowsMemo {
    snapshot: Option<Arc<RiseCommentListSnapshot>>,
    extra: u64,
    rows: Vec<CommentListRow>,
}

struct CommentsSheetStoryIdsMemo {
    list: Option<Arc<RiseCommentListSnapshot>>,
    thread: Option<Arc<RiseCommentListSnapshot>>,
    root_id: Option<String>,
    user_ids: Rc<Vec<String>>,
}

/// whether a keystroke is the escape the replies page answers
/// interceptors run for every window so a key in the call window or its pip must stay there
fn is_thread_escape(
    is_preview: bool,
    keystroke: &Keystroke,
    sheet_window: AnyWindowHandle,
    key_window: AnyWindowHandle,
) -> bool {
    !is_preview && key_window == sheet_window && keystroke.key == ESCAPE_KEY && !keystroke.modifiers.modified()
}

struct CommentsSheetState {
    thread_back_motion: CommentsThreadBackMotion,
    selected_sheet_fraction: f32,
    footer_height: f32,
    picker_lift: f32,
    content_width: f32,
    thread_presence: Animated<f32>,
    presented_root: Option<GetCommentsResponse>,
    thread_close_spec: Option<MotionSpec>,
    sort_menu_presented: Rc<Cell<bool>>,
    root_rows_memo: Option<CommentsSheetRowsMemo>,
    thread_rows_memo: Option<CommentsSheetRowsMemo>,
    story_ids_memo: Option<CommentsSheetStoryIdsMemo>,
    _escape_interceptor: Subscription,
}

impl CommentsSheetState {
    fn new(is_preview: bool, sheet_window: AnyWindowHandle, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let escape_interceptor = cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
            if !is_thread_escape(is_preview, &event.keystroke, sheet_window, window.window_handle()) {
                return;
            }
            let Some(state) = this.upgrade() else {
                return;
            };
            if CommentsSheet::handle_thread_escape(&state, cx) {
                cx.stop_propagation();
            }
        });
        Self {
            thread_back_motion: CommentsThreadBackMotion::default(),
            selected_sheet_fraction: CommentsSheet::INITIAL_SNAP_FRACTION,
            footer_height: 0.0,
            picker_lift: 0.0,
            content_width: 1.0,
            thread_presence: Animated::new(0.0),
            presented_root: None,
            thread_close_spec: None,
            sort_menu_presented: Rc::new(Cell::new(false)),
            root_rows_memo: None,
            thread_rows_memo: None,
            story_ids_memo: None,
            _escape_interceptor: escape_interceptor,
        }
    }

    /// the composer floats over the list so the list ends above it
    fn list_bottom_inset(&self, docked: bool) -> f32 {
        let picker_lift = if docked { 0.0 } else { self.picker_lift };
        f32::max(20.0, self.footer_height + 6.0 + f32::max(CommentsSheet::window_safe_bottom(), picker_lift))
    }

    fn sync_thread(&mut self, root: Option<&GetCommentsResponse>, docked: bool, now: MotionTime, cx: &mut Context<Self>) {
        let target = if root.is_some() { 1.0 } else { 0.0 };
        if *self.thread_presence.target() != target {
            let spec = self.thread_close_spec.take().unwrap_or_else(|| spring(0.34, 0.88));
            if docked {
                self.thread_presence.jump(target);
            } else {
                self.thread_presence.set_in(target, spec, now, cx);
            }
        }
        match root {
            Some(root) => {
                if self.presented_root.as_ref() != Some(root) {
                    self.presented_root = Some(root.clone());
                }
            }
            None => {
                if self.presented_root.is_some() && !self.thread_presence.is_running(now) {
                    self.presented_root = None;
                }
            }
        }
    }

    fn reset_thread_motion(&mut self) {
        self.thread_back_motion.offset.jump(0.0);
        self.thread_presence.jump(0.0);
        self.thread_close_spec = None;
        self.presented_root = None;
    }
}

impl CommentsSheet {
    pub const INITIAL_SNAP_FRACTION: f32 = 0.56;
    const HEADER_HEIGHT: f32 = 56.0;
    const HANDLE_CLEARANCE: f32 =
        BottomSheetMetrics::HANDLE_TOP_PADDING + BottomSheetMetrics::HANDLE_HEIGHT + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;
    const HEADER_FADE: f32 = 12.0;
    const FOOTER_BLUR_RADIUS: f32 = 20.0;
    const FOOTER_TINT_SOLID_END: f32 = 0.35;

    /// space above the header, the handle band of the sheet
    /// in the right panel there is no handle so the header row is centred on the back arrow and
    /// the cross
    fn top_clearance(docked: bool) -> f32 {
        if docked { (ShellMetrics::WINDOW_DRAG_HEIGHT - Self::HEADER_HEIGHT) / 2.0 } else { Self::HANDLE_CLEARANCE }
    }

    fn header_button_insets(docked: bool, theme: &AppTheme, cx: &App) -> (f32, f32) {
        if !docked {
            return (14.0, 14.0);
        }
        let shell = &theme.shell;
        let padding = f32::from(shell.rail_padding);
        let button = f32::from(shell.rail_icon_size + shell.rail_padding) + f32::from(shell.rail_item_gap);
        let steps_back = comments_sheet_aside_hooks().is_some_and(|hooks| (hooks.steps_back)(cx));
        let leading = if steps_back { padding + button } else { padding };
        (leading, padding + button)
    }

    /// where the list starts, under the top clearance and the header
    fn list_top(docked: bool) -> f32 {
        Self::HEADER_HEIGHT + Self::top_clearance(docked)
    }

    /// a desktop window has no home indicator gutter under its content
    fn window_safe_bottom() -> f32 {
        0.0
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn presentation_override(mut self, presentation_override: Binding<bool>) -> Self {
        self.presentation_override = Some(presentation_override);
        self
    }

    pub fn preview_comments(mut self, preview_comments: Vec<GetCommentsResponse>) -> Self {
        self.preview_comments = Some(preview_comments);
        self
    }

    /// desktop mode, the same comments page and reply field drawn in the right panel instead of
    /// a sheet over the window
    pub fn docked(mut self, docked: bool) -> Self {
        self.docked = docked;
        self
    }

    fn docked_frame(content: AnyElement, footer: AnyElement, picker_lift: f32, theme: &AppTheme) -> AnyElement {
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg._100)
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().relative().w_full().flex_1().min_h(px(0.0)).child(content))
                    .child(div().flex_none().w_full().h(px(picker_lift))),
            )
            .child(div().absolute().left_0().right_0().bottom(px(picker_lift)).child(footer))
            .into_any_element()
    }

    fn theme(cx: &mut App) -> AppTheme {
        ThemeStore::shared(cx).read(cx).current_theme().clone()
    }

    fn element_id(prefix: &str, id: &str) -> ElementId {
        ElementId::Name(format!("{prefix}-{id}").into())
    }

    fn list_snapshot(scope: &CommentsSheetScope, cx: &mut App) -> Option<Arc<RiseCommentListSnapshot>> {
        scope.services.update(cx, |services, cx| services.list_snapshot(cx))
    }

    fn thread_snapshot(scope: &CommentsSheetScope, cx: &mut App) -> Option<Arc<RiseCommentListSnapshot>> {
        scope.interactions.update(cx, |interactions, cx| interactions.thread_snapshot(cx))
    }

    /// turns off hit testing for a subtree that has its own handlers
    fn hit_testing_disabled(content: impl IntoElement) -> Div {
        div().relative().child(content).child(div().absolute().inset_0().block_mouse_except_scroll())
    }

    fn navigation_content(scope: &Rc<CommentsSheetScope>, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = Self::theme(cx);
        let now = RiseClock::monotonic();
        let root = scope.interactions.read(cx).thread_root().cloned();
        let showing_thread = root.is_some();
        scope.state.update(cx, |state, cx| state.sync_thread(root.as_ref(), scope.docked, now, cx));

        let (width, presence, back_offset, presented_root) = {
            let state = scope.state.read(cx);
            (
                f32::max(1.0, state.content_width),
                state.thread_presence.frame_value(now, window),
                state.thread_back_motion.offset.frame_value(now, window),
                state.presented_root.clone(),
            )
        };

        let measured = scope.state.clone();
        let comments_motion = CommentsThreadPageMotion {
            role: CommentsThreadPageMotionRole::Comments,
            width,
            presence,
            back_offset,
        };
        let comments_page = comments_motion
            .apply(Self::comments_page(scope, &theme, cx))
            .when(showing_thread, |page| page.child(div().absolute().inset_0().occlude()));

        div()
            .id("comments-sheet-navigation")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg._100)
            .child(measure("comments-sheet-navigation-width", move |bounds, _, cx| {
                measured.update(cx, |state, cx| {
                    state.content_width = f32::from(bounds.size.width);
                    cx.notify();
                });
            }))
            .child(Self::thread_back_swipe_gate(scope, cx))
            .child(comments_page)
            .when_some(presented_root, |navigation, root| {
                let replies_motion = CommentsThreadPageMotion {
                    role: CommentsThreadPageMotionRole::Replies,
                    width,
                    presence,
                    back_offset,
                };
                navigation.child(
                    replies_motion
                        .apply(Self::replies_page(scope, &root, &theme, cx))
                        .bg(theme.bg._100)
                        .occlude(),
                )
            })
            .into_any_element()
    }

    fn comments_page(scope: &Rc<CommentsSheetScope>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let subtitle = match &scope.preview_comments {
            Some(preview) => preview.len() as i64,
            None => {
                let count = scope.post.update(cx, |post, cx| post.selected_post_comments_count(cx));
                count.unwrap_or_else(|| {
                    Self::list_snapshot(scope, cx).map_or(0, |snapshot| snapshot.items.len() as i64)
                })
            }
        };
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let count_hidden = scope.preview_comments.is_none()
            && scope.post.read(cx).selected_post.as_ref().is_some_and(|post| {
                !post.shows_count(|visibility| visibility.comments, viewer.as_deref())
            });
        let subtitle = if count_hidden { String::new() } else { subtitle.to_string() };
        let trailing = Self::root_sort_menu(scope, theme, cx);
        let trailing = if scope.preview_comments.is_some() {
            Self::hit_testing_disabled(trailing).into_any_element()
        } else {
            trailing.into_any_element()
        };
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(Self::comments_data(scope, theme, cx))
            .child(
                div().absolute().top_0().left_0().right_0().child(Self::floating_header(
                    scope,
                    tr("comment_notify_title"),
                    subtitle,
                    HeaderLeading::Close,
                    trailing,
                    theme,
                    cx,
                )),
            )
            .into_any_element()
    }

    /// a reel carries its author, caption and date into the sheet
    /// scrolls with the list so an unfolded caption never pins the comments down
    fn reel_post(scope: &CommentsSheetScope, cx: &mut App) -> Option<(Rc<GetPostFeedResponse>, i64)> {
        let selected = scope.post.read(cx).selected_post.as_ref().filter(|selected| selected.is_riff)?;
        let mut version = DefaultHasher::new();
        selected.id.hash(&mut version);
        Some((Rc::new(selected.clone()), version.finish() as i64))
    }

    /// the author choice for strangers, the author keeps the numbers
    /// server already zeroes a hidden count, this only keeps a 0 label off the row
    fn shows_comment_dislike_counts(_scope: &CommentsSheetScope, cx: &mut App) -> bool {
        CommentRowFacts::shared(cx).read(cx).shows_dislike_counts
    }

    fn reel_header(scope: &Rc<CommentsSheetScope>, reel: Rc<GetPostFeedResponse>) -> ShortsCommentsHeader {
        let caption_expanded = Binding::entity(
            scope.interactions.clone(),
            |interactions| interactions.caption_expanded,
            |interactions, expanded, cx| {
                interactions.caption_expanded = expanded;
                cx.notify();
            },
        );
        let author_scope = scope.clone();
        let author_reel = reel.clone();
        let hashtag_scope = scope.clone();
        ShortsCommentsHeader::new(
            "comments-sheet-reel-header",
            reel,
            caption_expanded,
            move |_, cx| Self::open_reel_author_profile(&author_scope, &author_reel, cx),
            move |tag, _, cx| Self::open_reel_hashtag(&hashtag_scope, tag, cx),
        )
    }

    fn open_reel_hashtag(scope: &Rc<CommentsSheetScope>, tag: &str, cx: &mut App) {
        let tag = tag.to_owned();
        let navigation = scope.clone();
        Self::close_comments_for_avatar_navigation(
            scope,
            move |cx| {
                Self::nav_push(&navigation, RootRoute::HashtagSearch { tag, kind: RisePostSearchKind::Publications }, cx);
            },
            cx,
        );
    }

    fn open_reel_author_profile(scope: &Rc<CommentsSheetScope>, reel: &GetPostFeedResponse, cx: &mut App) {
        let author = RiseIdentity::author(Some(reel.author_id.as_str()), None, reel.author.id.as_deref());
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let mine = RiseIdentity::is_own(&author, viewer.as_deref(), None);
        let tag = trimming_whitespaces_and_newlines(&reel.author.tag).to_string();
        let user_id = author.wire_id().map(str::to_owned);
        let navigation = scope.clone();
        Self::close_comments_for_avatar_navigation(
            scope,
            move |cx| {
                if mine {
                    Self::nav_push(&navigation, RootRoute::MyProfile, cx);
                } else if let Some(user_id) = user_id {
                    let tag = (!tag.is_empty()).then_some(tag);
                    Self::nav_push(&navigation, RootRoute::UserPage { user_id: Some(user_id), tag }, cx);
                }
            },
            cx,
        );
    }

    fn nav_push(scope: &CommentsSheetScope, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &scope.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn no_comments_text(theme: &AppTheme) -> MainText {
        MainText::new(tr("no_comments")).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary)
    }

    fn comments_data(scope: &Rc<CommentsSheetScope>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let snapshot = Self::list_snapshot(scope, cx);
        let is_empty = snapshot.as_ref().is_none_or(|snapshot| snapshot.items.is_empty());
        let reel = Self::reel_post(scope, cx);

        if let Some(preview_comments) = scope.preview_comments.clone() {
            if preview_comments.is_empty() {
                return div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pt(px(Self::list_top(scope.docked)))
                    .child(Self::no_comments_text(theme))
                    .into_any_element();
            }
            let cells: Vec<AnyElement> = preview_comments
                .iter()
                .map(|comment| {
                    Self::hit_testing_disabled(Self::comment_cell(
                        scope,
                        comment,
                        CommentListRowPlacement::RootList,
                        theme,
                        cx,
                    ))
                    .w_full()
                    .bg(theme.bg._100)
                    .into_any_element()
                })
                .collect();
            return scroll_view("comments-sheet-preview-list")
                .child(vstack(0.0).w_full().pt(px(Self::list_top(scope.docked) + 3.0)).children(cells))
                .into_any_element();
        }

        if snapshot.as_ref().is_none_or(|snapshot| snapshot.shows_initial_loader()) {
            tracing::warn!(
                target: "riseonly::X2027",
                "initial loader docked={} scope={:?} load_state={:?} revision={:?}",
                scope.docked,
                snapshot.as_ref().map(|snapshot| &snapshot.scope),
                snapshot.as_ref().map(|snapshot| snapshot.load_state),
                snapshot.as_ref().map(|snapshot| snapshot.revision)
            );
            return scroll_view("comments-sheet-initial-loader")
                .scroll_disabled(true)
                .shows_indicators(false)
                .child(
                    vstack(0.0)
                        .w_full()
                        .pt(px(Self::list_top(scope.docked) + 3.0))
                        .when_some(reel, |column, (reel, _)| column.child(Self::reel_header(scope, reel)))
                        .child(CommentListSkeleton::default().style(CommentRowSkeletonStyle::Sheet)),
                )
                .into_any_element();
        }
        if snapshot.as_ref().is_some_and(|snapshot| snapshot.shows_blocking_error()) && reel.is_none() {
            return Self::list_error_state(scope, theme)
                .size_full()
                .justify_center()
                .pt(px(Self::list_top(scope.docked)))
                .into_any_element();
        }
        if is_empty && reel.is_none() {
            tracing::warn!(
                target: "riseonly::X1903",
                "empty state docked={} scope={:?} load_state={:?} revision={:?}",
                scope.docked,
                snapshot.as_ref().map(|snapshot| &snapshot.scope),
                snapshot.as_ref().map(|snapshot| snapshot.load_state),
                snapshot.as_ref().map(|snapshot| snapshot.revision)
            );
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .pt(px(Self::list_top(scope.docked)))
                .child(Self::no_comments_text(theme))
                .into_any_element();
        }

        let rows = Self::root_rows(scope, &snapshot, reel.as_ref().map(|(_, version)| *version), cx);
        let identity = {
            let services = scope.services.read(cx);
            format!(
                "post|{}|{}",
                services.list_owner_post_id().unwrap_or(""),
                services.list_sort().raw_value()
            )
        };
        let (focus_id, reveal_id) = {
            let interactions = scope.interactions.read(cx);
            (
                interactions.focus_comment_id.clone(),
                interactions.pending_scroll_comment_id().map(str::to_owned),
            )
        };
        let bottom_inset = scope.state.read(cx).list_bottom_inset(scope.docked);
        let load_more = scope.interactions.clone();
        let focus_resolved = scope.interactions.clone();
        let focus_missing = scope.interactions.clone();
        let revealed = scope.interactions.clone();
        CommentNativeListView::new(
            identity,
            rows,
            snapshot.as_ref().is_some_and(|snapshot| snapshot.has_more),
            snapshot.as_ref().is_some_and(|snapshot| snapshot.is_loading_more),
            Self::list_top(scope.docked) + 3.0,
            bottom_inset,
            Self::is_sheet_interaction_locked(cx),
            Rc::new(move |_: &mut Window, cx: &mut App| {
                load_more.update(cx, |interactions, cx| interactions.load_more_comments(cx));
            }),
            Self::row_content(scope),
        )
        .id("comments-sheet-root-list")
        .focus_id(focus_id)
        .reveal_id(reveal_id)
        .on_focus_resolved(Rc::new(move |id: &str, _: &mut Window, cx: &mut App| {
            focus_resolved.update(cx, |interactions, cx| interactions.resolve_focus(id, cx));
        }))
        .on_focus_missing(Rc::new(move |_: &mut Window, cx: &mut App| {
            focus_missing.update(cx, |interactions, cx| interactions.focus_maybe_load_more(cx));
        }))
        .on_revealed(Rc::new(move |_: &str, _: &mut Window, cx: &mut App| {
            revealed.update(cx, |interactions, cx| interactions.consume_pending_scroll_comment_id(cx));
        }))
        .into_any_element()
    }

    fn row_content(
        scope: &Rc<CommentsSheetScope>,
    ) -> Rc<dyn Fn(&CommentListRow, &mut Window, &mut App) -> AnyView> {
        let scope = scope.clone();
        Rc::new(move |row: &CommentListRow, _: &mut Window, cx: &mut App| -> AnyView {
            let scope = scope.clone();
            let row = row.clone();
            AnyView::from(cx.new(|_| CommentsSheetRowView { scope, row }))
        })
    }

    /// what a comment row draws besides the comment
    /// part of the row version so a moving upload ring or a hidden dislike count re-renders it
    fn row_extra(
        comment: &GetCommentsResponse,
        shows_dislikes: bool,
        uploads: &HashMap<String, CommentPhotoUpload>,
    ) -> (&'static str, String) {
        let upload = uploads.get(&comment.id).map_or_else(
            || "-".to_owned(),
            |upload| {
                format!(
                    "{}|{}|{}",
                    upload.is_compressing, upload.compression_progress as i64, upload.upload_progress as i64
                )
            },
        );
        (if shows_dislikes { "d1" } else { "d0" }, upload)
    }

    /// rebuilds the row list only when something it draws moved
    /// render runs per frame during a swipe and a version hash per comment per frame is too much
    fn memoized_rows(
        scope: &CommentsSheetScope,
        thread: bool,
        snapshot: &Option<Arc<RiseCommentListSnapshot>>,
        extra: u64,
        build: impl FnOnce(&mut App) -> Vec<CommentListRow>,
        cx: &mut App,
    ) -> Vec<CommentListRow> {
        let hit = {
            let state = scope.state.read(cx);
            let memo = if thread { &state.thread_rows_memo } else { &state.root_rows_memo };
            memo.as_ref()
                .filter(|memo| {
                    memo.extra == extra
                        && match (&memo.snapshot, snapshot) {
                            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
                            (None, None) => true,
                            _ => false,
                        }
                })
                .map(|memo| memo.rows.clone())
        };
        if let Some(rows) = hit {
            return rows;
        }
        let rows = build(cx);
        let memo = CommentsSheetRowsMemo { snapshot: snapshot.clone(), extra, rows: rows.clone() };
        scope.state.update(cx, |state, _| {
            if thread {
                state.thread_rows_memo = Some(memo);
            } else {
                state.root_rows_memo = Some(memo);
            }
        });
        rows
    }

    fn rows_extra_key(
        shows_dislikes: bool,
        uploads: &HashMap<String, CommentPhotoUpload>,
        reel_version: Option<i64>,
        caption_expanded: bool,
        expanded_ids: &HashSet<String>,
        root_id: Option<&str>,
    ) -> u64 {
        let uploads_key = uploads.iter().fold(0_u64, |key, (id, upload)| {
            let mut hasher = DefaultHasher::new();
            id.hash(&mut hasher);
            upload.is_compressing.hash(&mut hasher);
            (upload.compression_progress as i64).hash(&mut hasher);
            (upload.upload_progress as i64).hash(&mut hasher);
            key ^ hasher.finish()
        });
        let expanded_key = expanded_ids.iter().fold(0_u64, |key, id| {
            let mut hasher = DefaultHasher::new();
            id.hash(&mut hasher);
            key ^ hasher.finish()
        });
        let mut hasher = DefaultHasher::new();
        (shows_dislikes, uploads_key, expanded_key, reel_version, caption_expanded, root_id).hash(&mut hasher);
        hasher.finish()
    }

    fn root_rows(
        scope: &Rc<CommentsSheetScope>,
        snapshot: &Option<Arc<RiseCommentListSnapshot>>,
        reel_version: Option<i64>,
        cx: &mut App,
    ) -> Vec<CommentListRow> {
        let shows_dislikes = Self::shows_comment_dislike_counts(scope, cx);
        let interactions = scope.interactions.read(cx);
        let caption_expanded = interactions.caption_expanded;
        let uploads = interactions.photo_uploads().clone();
        let expanded = interactions.expanded_comment_ids().clone();
        let extra = Self::rows_extra_key(shows_dislikes, &uploads, reel_version, caption_expanded, &expanded, None);
        let source = snapshot.clone();
        Self::memoized_rows(
            scope,
            false,
            snapshot,
            extra,
            move |_| {
                let comments = source.as_ref().map_or(&[][..], |snapshot| snapshot.items.as_slice());
                let mut rows = Vec::with_capacity(comments.len() + 3);
                if let Some(version) = reel_version {
                    rows.push(CommentListRow::accessory(Accessory::REEL_HEADER, (version, caption_expanded)));
                    if comments.is_empty() {
                        let failed = source.as_ref().is_some_and(|snapshot| snapshot.shows_blocking_error());
                        let name = if failed { Accessory::LIST_ERROR } else { Accessory::EMPTY };
                        rows.push(CommentListRow::accessory(name, 0_i64));
                    }
                }
                for comment in comments {
                    rows.push(CommentListRow::comment(
                        comment.clone(),
                        CommentListRowPlacement::RootList,
                        expanded.contains(&comment.id),
                        Some(Self::row_extra(comment, shows_dislikes, &uploads)),
                    ));
                }
                if let Some(snapshot) = source.as_ref().filter(|_| !comments.is_empty()) {
                    if snapshot.is_loading_more {
                        rows.push(CommentListRow::accessory(Accessory::PAGE_LOADER, 0_i64));
                    } else if snapshot.has_more && snapshot.error_text.is_some() {
                        rows.push(CommentListRow::accessory(Accessory::PAGE_RETRY, 0_i64));
                    }
                }
                rows
            },
            cx,
        )
    }

    fn root_row_content(
        scope: &Rc<CommentsSheetScope>,
        row: &CommentListRow,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        match &row.kind {
            CommentListRowKind::Comment(comment, placement) => {
                Self::comment_cell(scope, comment, *placement, theme, cx).into_any_element()
            }
            CommentListRowKind::Accessory(name) => Self::accessory_content(scope, name, theme, cx),
        }
    }

    fn accessory_content(scope: &Rc<CommentsSheetScope>, name: &str, theme: &AppTheme, cx: &mut App) -> AnyElement {
        match name {
            Accessory::REEL_HEADER => match Self::reel_post(scope, cx) {
                Some((reel, _)) => Self::reel_header(scope, reel).into_any_element(),
                None => Empty.into_any_element(),
            },
            Accessory::EMPTY => div()
                .w_full()
                .flex()
                .justify_center()
                .py(px(28.0))
                .child(Self::no_comments_text(theme))
                .into_any_element(),
            Accessory::LIST_ERROR => Self::list_error_state(scope, theme).py(px(28.0)).into_any_element(),
            Accessory::PAGE_LOADER => div()
                .w_full()
                .flex()
                .justify_center()
                .py(px(16.0))
                .child(progress_view().tint(theme.text.secondary))
                .into_any_element(),
            Accessory::PAGE_RETRY => Self::list_inline_retry(scope, theme).into_any_element(),
            Accessory::THREAD_SKELETON => div()
                .pl(px(CommentRowMetrics::REPLY_INDENT))
                .child(CommentListSkeleton::default().style(CommentRowSkeletonStyle::Sheet))
                .into_any_element(),
            Accessory::THREAD_ERROR => Self::replies_error_state(scope, theme).into_any_element(),
            Accessory::THREAD_EMPTY => Self::replies_empty_state(theme).into_any_element(),
            Accessory::THREAD_RETRY => Self::replies_inline_retry(scope, theme).into_any_element(),
            _ => Empty.into_any_element(),
        }
    }

    fn retry_button(
        id: &'static str,
        background: gpui::Hsla,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        plain(id)
            .on_press(action)
            .flex()
            .flex_none()
            .items_center()
            .px(px(13.0))
            .h(px(34.0))
            .rounded_full()
            .bg(background)
            .child(
                hstack(6.0)
                    .items_center()
                    .children(IconUi::sized("arrow.clockwise", px(12.0), theme.text.primary, false))
                    .child(
                        MainText::new(tr("common_retry"))
                            .px(13.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary),
                    ),
            )
    }

    fn list_error_state(scope: &Rc<CommentsSheetScope>, theme: &AppTheme) -> Div {
        let interactions = scope.interactions.clone();
        vstack(12.0)
            .w_full()
            .items_center()
            .px(px(28.0))
            .children(IconUi::sized("wifi.exclamationmark", px(25.0), theme.text.secondary.opacity(0.75), false))
            .child(
                MainText::new(tr("error_fetching_data"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
            .child(Self::retry_button("comments-sheet-list-retry", theme.bg._200, theme, move |_, cx| {
                interactions.update(cx, |interactions, cx| interactions.retry_comments(cx));
            }))
    }

    fn replies_page(
        scope: &Rc<CommentsSheetScope>,
        root: &GetCommentsResponse,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let snapshot = Self::thread_snapshot(scope, cx);
        let rows = Self::thread_rows(scope, root, &snapshot, cx);
        let (identity, focus_id, reveal_id) = {
            let interactions = scope.interactions.read(cx);
            (
                format!("replies|{}|{}", root.id, interactions.replies_sort().raw_value()),
                interactions.thread_focus_reply_id().map(str::to_owned),
                interactions.pending_scroll_reply_id().map(str::to_owned),
            )
        };
        let bottom_inset = scope.state.read(cx).list_bottom_inset(scope.docked);
        let load_more = scope.interactions.clone();
        let focus_missing = scope.interactions.clone();
        let revealed = scope.interactions.clone();
        let trailing = Self::replies_sort_menu(scope, theme, cx).into_any_element();

        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(
                CommentNativeListView::new(
                    identity,
                    rows,
                    snapshot.as_ref().is_some_and(|snapshot| snapshot.has_more),
                    snapshot.as_ref().is_some_and(|snapshot| snapshot.is_loading_more),
                    Self::list_top(scope.docked) + 3.0,
                    bottom_inset,
                    Self::is_sheet_interaction_locked(cx),
                    Rc::new(move |_: &mut Window, cx: &mut App| {
                        load_more.update(cx, |interactions, cx| interactions.load_more_replies(cx));
                    }),
                    Self::row_content(scope),
                )
                .id("comments-sheet-replies-list")
                .focus_id(focus_id)
                .reveal_id(reveal_id)
                .on_focus_missing(Rc::new(move |_: &mut Window, cx: &mut App| {
                    focus_missing.update(cx, |interactions, cx| interactions.thread_focus_maybe_load_more(cx));
                }))
                .on_revealed(Rc::new(move |_: &str, _: &mut Window, cx: &mut App| {
                    revealed.update(cx, |interactions, cx| interactions.consume_pending_scroll_reply_id(cx));
                })),
            )
            .child(
                div().absolute().top_0().left_0().right_0().child(Self::floating_header(
                    scope,
                    tr("comment_replies_title"),
                    root.author.display_name(),
                    HeaderLeading::Back,
                    trailing,
                    theme,
                    cx,
                )),
            )
            .into_any_element()
    }

    fn thread_rows(
        scope: &Rc<CommentsSheetScope>,
        root: &GetCommentsResponse,
        snapshot: &Option<Arc<RiseCommentListSnapshot>>,
        cx: &mut App,
    ) -> Vec<CommentListRow> {
        let shows_dislikes = Self::shows_comment_dislike_counts(scope, cx);
        let uploads = scope.interactions.read(cx).photo_uploads().clone();
        let expanded = scope.interactions.read(cx).expanded_comment_ids().clone();
        let mut root_hasher = DefaultHasher::new();
        root.hash(&mut root_hasher);
        let root_key = root_hasher.finish().to_string();
        let extra = Self::rows_extra_key(shows_dislikes, &uploads, None, false, &expanded, Some(root_key.as_str()));
        let source = snapshot.clone();
        let root = root.clone();
        Self::memoized_rows(
            scope,
            true,
            snapshot,
            extra,
            move |_| {
                let replies = source.as_ref().map_or(&[][..], |snapshot| snapshot.items.as_slice());
                let mut rows = Vec::with_capacity(replies.len() + 3);
                let root_extra = Self::row_extra(&root, shows_dislikes, &uploads);
                rows.push(CommentListRow::comment(
                    root.clone(),
                    CommentListRowPlacement::ThreadRoot,
                    expanded.contains(&root.id),
                    Some(root_extra),
                ));
                let has_error = source.as_ref().is_some_and(|snapshot| snapshot.error_text.is_some());
                if replies.is_empty() {
                    if source.as_ref().is_none_or(|snapshot| snapshot.shows_initial_loader()) {
                        rows.push(CommentListRow::accessory(Accessory::THREAD_SKELETON, 0_i64));
                    } else if has_error {
                        rows.push(CommentListRow::accessory(Accessory::THREAD_ERROR, 0_i64));
                    } else {
                        rows.push(CommentListRow::accessory(Accessory::THREAD_EMPTY, 0_i64));
                    }
                    return rows;
                }
                for reply in replies.iter().filter(|reply| reply.id != root.id) {
                    rows.push(CommentListRow::comment(
                        reply.clone(),
                        CommentListRowPlacement::ThreadReply,
                        expanded.contains(&reply.id),
                        Some(Self::row_extra(reply, shows_dislikes, &uploads)),
                    ));
                }
                if source.as_ref().is_some_and(|snapshot| snapshot.is_loading_more) {
                    rows.push(CommentListRow::accessory(Accessory::PAGE_LOADER, 0_i64));
                } else if has_error {
                    rows.push(CommentListRow::accessory(Accessory::THREAD_RETRY, 0_i64));
                }
                rows
            },
            cx,
        )
    }

    fn replies_empty_state(theme: &AppTheme) -> Div {
        vstack(8.0)
            .w_full()
            .items_center()
            .px(px(28.0))
            .py(px(28.0))
            .children(IconUi::sized(
                "bubble.left.and.bubble.right",
                px(25.0),
                theme.text.secondary.opacity(0.7),
                false,
            ))
            .child(
                MainText::new(tr("comment_replies_empty"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
    }

    fn replies_error_state(scope: &Rc<CommentsSheetScope>, theme: &AppTheme) -> Div {
        vstack(12.0)
            .w_full()
            .items_center()
            .px(px(28.0))
            .py(px(28.0))
            .children(IconUi::sized("wifi.exclamationmark", px(25.0), theme.text.secondary.opacity(0.75), false))
            .child(
                MainText::new(tr("comment_replies_load_error"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
            .child(Self::retry_replies_button(scope, "comments-sheet-replies-error-retry", theme))
    }

    fn list_inline_retry(scope: &Rc<CommentsSheetScope>, theme: &AppTheme) -> Div {
        let interactions = scope.interactions.clone();
        hstack(8.0)
            .w_full()
            .items_center()
            .justify_center()
            .py(px(14.0))
            .child(MainText::new(tr("error_fetching_data")).px(12.0).color(theme.text.secondary))
            .child(Self::retry_button("comments-sheet-page-retry", theme.bg._100, theme, move |_, cx| {
                interactions.update(cx, |interactions, cx| interactions.load_more_comments(cx));
            }))
    }

    fn replies_inline_retry(scope: &Rc<CommentsSheetScope>, theme: &AppTheme) -> Div {
        hstack(8.0)
            .w_full()
            .items_center()
            .justify_center()
            .py(px(14.0))
            .child(MainText::new(tr("comment_replies_load_more_error")).px(12.0).color(theme.text.secondary))
            .child(Self::retry_replies_button(scope, "comments-sheet-replies-page-retry", theme))
    }

    fn retry_replies_button(scope: &Rc<CommentsSheetScope>, id: &'static str, theme: &AppTheme) -> PlainButton {
        let interactions = scope.interactions.clone();
        Self::retry_button(id, theme.bg._100, theme, move |_, cx| {
            interactions.update(cx, |interactions, cx| interactions.retry_thread_replies(cx));
        })
    }

    fn floating_header(
        scope: &Rc<CommentsSheetScope>,
        title: String,
        subtitle: String,
        leading: HeaderLeading,
        trailing: AnyElement,
        theme: &AppTheme,
        cx: &App,
    ) -> Div {
        let leading_button = match leading {
            HeaderLeading::Close if scope.docked => None,
            HeaderLeading::Close => {
                let scope = scope.clone();
                Some(Self::flat_header_button("comments-sheet-close", "xmark", 15.0, theme, move |window, cx| {
                    if let Some(presentation_override) = &scope.presentation_override {
                        presentation_override.set(false, window, cx);
                    } else {
                        scope.interactions.update(cx, |interactions, cx| interactions.close_comments_sheet(cx));
                    }
                }))
            }
            HeaderLeading::Back => {
                let scope = scope.clone();
                Some(Self::flat_header_button("comments-sheet-back", "chevron.backward", 16.0, theme, move |_, cx| {
                    Self::close_thread_page(&scope.state, &scope.interactions, cx);
                }))
            }
        };
        let clearance = Self::top_clearance(scope.docked);
        let (leading_inset, trailing_inset) = Self::header_button_insets(scope.docked, theme, cx);
        div()
            .relative()
            .w_full()
            .h(px(Self::HEADER_HEIGHT + clearance))
            .child(Self::header_chrome(clearance, theme))
            .child(
                div()
                    .absolute()
                    .top(px(clearance))
                    .left_0()
                    .right_0()
                    .h(px(Self::HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                    vstack(1.0)
                        .items_center()
                        .max_w(px(210.0))
                        .child(
                            MainText::new(title)
                                .px(16.0)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::SEMIBOLD)
                                .number_of_lines(1),
                        )
                        .when(!subtitle.is_empty(), |column| {
                            column.child(
                                MainText::new(subtitle)
                                    .px(11.0)
                                    .tac(TextAlignT::Center)
                                    .color(theme.text.secondary)
                                    .number_of_lines(1),
                            )
                        }),
                ),
            )
            .child(
                hstack(10.0)
                    .absolute()
                    .top(px(clearance))
                    .left_0()
                    .right_0()
                    .h(px(Self::HEADER_HEIGHT))
                    .pl(px(leading_inset))
                    .pr(px(trailing_inset))
                    .items_center()
                    .children(leading_button.map(|button| div().occlude().child(button)))
                    .child(div().flex_1())
                    .child(div().occlude().child(trailing)),
            )
    }

    fn flat_header_button(
        id: &'static str,
        system_image: &'static str,
        font_size: f32,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        plain(id)
            .on_press(action)
            .flex()
            .items_center()
            .justify_center()
            .size(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .rounded_full()
            .bg(theme.bg._100)
            .children(IconUi::sized(system_image, px(font_size), theme.text.primary, false))
    }

    /// header chrome, a blur of the rows under the handle band and header
    /// thins out over a short soft edge
    /// no blur available means the tint stays near opaque
    fn header_chrome(clearance: f32, theme: &AppTheme) -> Div {
        let plate = Self::HEADER_HEIGHT + clearance;
        let height = plate + Self::HEADER_FADE;
        div().absolute().top_0().left_0().right_0().h(px(height)).child(
            ScrollEdgeBlurUi::new()
                .solid_end(plate / height)
                .tint_peak(ScrollEdgeBlurFalloff::header_tint_peak(theme.bg._100.l < 0.5)),
        )
    }

    fn footer_chrome(theme: &AppTheme, height: f32, cx: &App) -> Div {
        let blurs = BACKDROP_BLUR_SUPPORTED && !accessibility_reduce_transparency(cx);
        let bleed = 48.0;
        let peak = if blurs { ScrollChromeShadowStyle::list_shell_blurred_header().peak_opacity } else { 0.84 };
        let ramp = height.max((height + 66.0) * 0.6);
        let (band_scale, tint_solid_end) = ScrollEdgeBlurFalloff::stretched(CommentsSheet::FOOTER_TINT_SOLID_END);
        div()
            .absolute()
            .inset_0()
            .child(div().absolute().left_0().right_0().bottom_0().h(px(ramp)).child(ScrollChromeShadowUi::new(
                ScrollChromeShadowStyle {
                    anchor: ScrollChromeShadowStyleAnchor::FromBottom,
                    peak_opacity: peak,
                    color: theme.bg._100,
                    mid_location: 0.58,
                    mid_opacity_factor: 0.68,
                    top_plateau_end: 0.16,
                    top_plateau_opacity_factor: 0.97,
                    ..Default::default()
                },
            )))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(-bleed))
                    .h(px(bleed))
                    .bg(theme.bg._100.opacity(peak as f32)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(height))
                    .when(!blurs, |band| band.bg(theme.bg._100))
                    .when(blurs, |band| {
                        band.child(
                            canvas(
                                |_, _, _| (),
                                |bounds, _, window, _| {
                                    window.paint_progressive_backdrop_blur(
                                        bounds,
                                        Corners::default(),
                                        px(CommentsSheet::FOOTER_BLUR_RADIUS),
                                        px(0.0),
                                        true,
                                    );
                                },
                            )
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .w_full()
                            .h(relative(band_scale)),
                        )
                        .child(div().absolute().bottom_0().left_0().w_full().h(relative(band_scale)).child(LinearGradient::new(
                            ScrollEdgeBlurFalloff::tint_stops_with_peak(
                                theme.bg._100,
                                tint_solid_end,
                                ScrollEdgeBlurFalloff::header_tint_peak(theme.bg._100.l < 0.5),
                                false,
                            ),
                            UnitPoint::BOTTOM,
                            UnitPoint::TOP,
                        )))
                    }),
            )
    }

    fn root_sort_menu(scope: &Rc<CommentsSheetScope>, theme: &AppTheme, cx: &mut App) -> Div {
        let has_profile = AuthServicesStore::shared(cx).read(cx).profile().is_some();
        let selected = scope.services.read(cx).list_sort();
        let presented = scope.state.read(cx).sort_menu_presented.clone();
        let mut items = vec![
            Self::root_sort_button(0, CommentSortType::Feed, tr("contextMenu_interesting"), "chart.line.uptrend.xyaxis"),
            Self::root_sort_button(1, CommentSortType::New, tr("contextMenu_new"), RiseSymbol::SORT_NEWEST),
            Self::root_sort_button(2, CommentSortType::Old, tr("contextMenu_old"), RiseSymbol::SORT_OLDEST),
        ];
        if has_profile {
            items.push(Self::root_sort_button(3, CommentSortType::My, tr("contextMenu_my"), "person.crop.circle"));
        }
        div().rounded_full().bg(theme.bg._100).child(
            ContextMenuUi::new(
                "comments-sheet-root-sort-menu",
                items,
                Self::header_menu_label(Self::root_sort_icon(selected), theme),
            )
            .shows_item_icons()
            .on_presented_change(move |is_presented, _, _| presented.set(is_presented)),
        )
    }

    fn replies_sort_menu(scope: &Rc<CommentsSheetScope>, theme: &AppTheme, cx: &mut App) -> Div {
        let has_profile = AuthServicesStore::shared(cx).read(cx).profile().is_some();
        let selected = scope.interactions.read(cx).replies_sort();
        let presented = scope.state.read(cx).sort_menu_presented.clone();
        let mut items = vec![
            Self::replies_sort_button(0, RepliesSortType::New, tr("contextMenu_new"), RiseSymbol::SORT_NEWEST),
            Self::replies_sort_button(1, RepliesSortType::Old, tr("contextMenu_old"), RiseSymbol::SORT_OLDEST),
            Self::replies_sort_button(
                2,
                RepliesSortType::Popular,
                tr("contextMenu_interesting"),
                "chart.line.uptrend.xyaxis",
            ),
        ];
        if has_profile {
            items.push(Self::replies_sort_button(3, RepliesSortType::My, tr("contextMenu_my"), "person.crop.circle"));
        }
        div().rounded_full().bg(theme.bg._100).child(
            ContextMenuUi::new(
                "comments-sheet-replies-sort-menu",
                items,
                Self::header_menu_label(Self::replies_sort_icon(selected), theme),
            )
            .shows_item_icons()
            .on_presented_change(move |is_presented, _, _| presented.set(is_presented)),
        )
    }

    fn header_menu_label(system_image: &str, theme: &AppTheme) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .children(IconUi::sized(system_image, px(15.0), theme.text.primary, false))
    }

    fn root_sort_button(index: i64, sort: CommentSortType, label: String, system_image: &str) -> ContextMenuItem {
        ContextMenuItem::new(index, label, Some(system_image.to_owned()), sort.raw_value(), move |_, cx| {
            CommentInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.change_sort(sort, cx));
        })
    }

    fn replies_sort_button(index: i64, sort: RepliesSortType, label: String, system_image: &str) -> ContextMenuItem {
        ContextMenuItem::new(index, label, Some(system_image.to_owned()), sort.raw_value(), move |_, cx| {
            CommentInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.change_replies_sort(sort, cx));
        })
    }

    fn comment_cell(
        scope: &Rc<CommentsSheetScope>,
        comment: &GetCommentsResponse,
        placement: CommentListRowPlacement,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Div {
        let actions = Self::hold_actions(scope, comment, cx);
        let is_uploading = CommentRowFacts::shared(cx).read(cx).uploads.contains_key(&comment.id);
        let shows_dislike_count = Self::shows_comment_dislike_counts(scope, cx);
        let id = comment.id.clone();

        let row_scope = scope.clone();
        let row_comment = comment.clone();
        let source: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement> =
            Rc::new(move |_: &mut Window, cx: &mut App| Self::comment_row(&row_scope, &row_comment, cx));

        let action_scope = scope.clone();
        let action_comment = comment.clone();
        let double_tap_comment = comment.clone();
        let double_tap_interactions = scope.interactions.clone();
        let can_double_tap = !comment.is_temp && !comment.is_moderation_disabled();

        let hold = HoldContextMenuContainer::new(
            Self::element_id("comment-hold", &id),
            HoldContextMenuAlignment::Trailing,
            Rc::new(move |_: &App| actions.clone()),
            source,
        )
        .show_emoji_list(false)
        .uses_flat_chrome(true)
        .lifted_background(Some(theme.bg._100))
        .on_action(Rc::new(move |action_id: String, _: &mut Window, cx: &mut App| {
            Self::handle_action(&action_scope, &action_id, &action_comment, placement.is_inside_thread(), cx);
        }))
        .on_tap(Rc::new(move |point: Point<Pixels>, window: &mut Window, cx: &mut App| {
            BubbleTapRouter::dispatch(point, None, window, cx);
        }))
        .can_double_tap(Rc::new(move |_: &App| can_double_tap))
        .on_double_tap(Some(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            let _ = impact(ImpactStyle::Light);
            double_tap_interactions
                .update(cx, |interactions, cx| interactions.like_comment_from_double_tap(&double_tap_comment, cx));
        })));

        let like_interactions = scope.interactions.clone();
        let like_comment = comment.clone();
        let dislike_interactions = scope.interactions.clone();
        let dislike_comment = comment.clone();
        let reply_scope = scope.clone();
        let reply_comment = comment.clone();
        let actions_row = CommentActionsRow::new(Self::element_id("comment-actions", &id), comment.clone())
            .reply_action_style(if placement == CommentListRowPlacement::RootList {
                CommentReplyActionStyle::Thread { count: comment.replies_count }
            } else {
                CommentReplyActionStyle::Reply
            })
            .shows_dislike_count(shows_dislike_count)
            .on_like(move |_, cx| {
                like_interactions.update(cx, |interactions, cx| interactions.toggle_like_on_comment(&like_comment, cx));
            })
            .on_dislike(move |_, cx| {
                dislike_interactions
                    .update(cx, |interactions, cx| interactions.toggle_dislike_on_comment(&dislike_comment, cx));
            })
            .on_reply(move |_, cx| {
                if placement == CommentListRowPlacement::RootList {
                    Self::open_thread(&reply_scope, &reply_comment, reply_comment.replies_count == 0, cx);
                } else {
                    reply_scope
                        .interactions
                        .update(cx, |interactions, cx| interactions.start_reply(&reply_comment, cx));
                }
            });

        div()
            .relative()
            .w_full()
            .pl(px(placement.leading_indent()))
            .child(hold)
            .child(
                div()
                    .pl(px(CommentRowMetrics::ACTIONS_LEADING_PADDING))
                    .pr(px(CommentRowMetrics::HORIZONTAL_PADDING))
                    .pb(px(CommentRowMetrics::ACTIONS_BOTTOM_PADDING))
                    .opacity(if comment.is_temp && !is_uploading { 0.55 } else { 1.0 })
                    .child(actions_row),
            )
    }

    fn comment_row(scope: &Rc<CommentsSheetScope>, comment: &GetCommentsResponse, cx: &mut App) -> AnyElement {
        let upload = CommentRowFacts::shared(cx).read(cx).uploads.get(&comment.id).copied();
        let profile_scope = scope.clone();
        let profile_comment = comment.clone();
        let story_scope = scope.clone();
        let story_comment = comment.clone();
        let cancel_interactions = scope.interactions.clone();
        let cancel_id = comment.id.clone();
        let mention_scope = scope.clone();
        let link_scope = scope.clone();
        let is_text_expanded = CommentRowFacts::shared(cx).read(cx).expanded.contains(&comment.id);
        let expand_interactions = scope.interactions.clone();
        let expand_id = comment.id.clone();
        CommentRow::new(Self::element_id("comment-row", &comment.id), comment.clone())
            .show_actions(false)
            .on_avatar_profile(move |_, cx| Self::open_comment_author_profile(&profile_scope, &profile_comment, cx))
            .on_avatar_story(move |_, cx| Self::open_comment_author_story(&story_scope, &story_comment, cx))
            .photo_upload(upload)
            .on_cancel_photo_upload(move |_, cx| {
                cancel_interactions.update(cx, |interactions, cx| interactions.cancel_photo_upload(&cancel_id, cx));
            })
            .on_mention_tap(Some(Rc::new(move |tag: &str, _: &mut Window, cx: &mut App| {
                Self::open_comment_mention(&mention_scope, tag, cx);
            })))
            .on_link_tap(Some(Rc::new(move |raw: &str, _: &mut Window, cx: &mut App| {
                Self::open_comment_link(&link_scope, raw, cx);
            })))
            .is_text_expanded(is_text_expanded)
            .on_text_expanded_change(move |expanded, _, cx| {
                expand_interactions
                    .update(cx, |interactions, cx| interactions.set_comment_text_expanded(&expand_id, expanded, cx));
            })
            .into_any_element()
    }

    fn open_thread(
        scope: &Rc<CommentsSheetScope>,
        comment: &GetCommentsResponse,
        focus_composer: bool,
        cx: &mut App,
    ) {
        let is_other = scope.interactions.read(cx).thread_root().map(|root| root.id.as_str()) != Some(comment.id.as_str());
        if is_other {
            scope.state.update(cx, |state, cx| {
                state.thread_back_motion.offset.jump(0.0);
                cx.notify();
            });
            scope.interactions.update(cx, |interactions, cx| interactions.open_thread(comment, cx));
        }
        if focus_composer {
            scope.interactions.update(cx, |interactions, cx| interactions.start_reply(comment, cx));
        }
    }

    fn close_thread_page(
        state: &Entity<CommentsSheetState>,
        interactions: &Entity<CommentInteractionsStore>,
        cx: &mut App,
    ) {
        if interactions.read(cx).thread_root().is_none() {
            return;
        }
        let now = RiseClock::monotonic();
        state.update(cx, |state, cx| {
            state.thread_close_spec = Some(spring(0.32, 0.9));
            state.thread_back_motion.offset.set_in(0.0, spring(0.32, 0.9), now, cx);
            cx.notify();
        });
        interactions.update(cx, |interactions, cx| interactions.close_thread(cx));
    }

    fn handle_thread_escape(state: &Entity<CommentsSheetState>, cx: &mut App) -> bool {
        let interactions = CommentInteractionsStore::shared(cx);
        {
            let interactions = interactions.read(cx);
            if !interactions.comments_sheet_visible
                || interactions.thread_root().is_none()
                || interactions.photo_picker_visible
            {
                return false;
            }
        }
        let is_covered = {
            let state = state.read(cx);
            state.picker_lift > 0.0 || state.sort_menu_presented.get()
        };
        if is_covered
            || Self::is_sheet_interaction_locked(cx)
            || ShareSheetStore::shared(cx).read(cx).is_visible()
        {
            return false;
        }
        Self::close_thread_page(state, &interactions, cx);
        true
    }

    fn thread_back_swipe_gate(scope: &Rc<CommentsSheetScope>, cx: &mut App) -> BottomSheetPageBackSwipeGate {
        let is_enabled =
            scope.interactions.read(cx).thread_root().is_some() && !Self::is_sheet_interaction_locked(cx);
        let changed = scope.state.clone();
        let ended = scope.clone();
        let cancelled = scope.state.clone();
        BottomSheetPageBackSwipeGate::new(
            "comments-sheet-thread-back-swipe",
            BottomSheetPageBackSwipeConfiguration { is_enabled },
            BottomSheetPageBackSwipeCallbacks {
                on_began: Rc::new(|_: &mut Window, _: &mut App| {}),
                on_changed: Rc::new(move |translation: Pixels, _: &mut Window, cx: &mut App| {
                    Self::track_thread_back(&changed, f32::from(translation), cx);
                }),
                on_ended: Rc::new(move |translation: Pixels, velocity: Pixels, _: &mut Window, cx: &mut App| {
                    Self::release_thread_back(&ended, translation, velocity, cx);
                }),
                on_cancelled: Rc::new(move |_: &mut Window, cx: &mut App| {
                    Self::cancel_thread_back(&cancelled, cx);
                }),
            },
        )
    }

    fn is_sheet_interaction_locked(cx: &mut App) -> bool {
        HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some()
            || StickerHoldPreviewPresenter::shared(cx).read(cx).is_active()
    }

    fn track_thread_back(state: &Entity<CommentsSheetState>, translation: f32, cx: &mut App) {
        state.update(cx, |state, cx| {
            let clamped = translation.clamp(0.0, state.content_width.max(0.0));
            let current = *state.thread_back_motion.offset.target();
            if (clamped - current).abs() <= BottomSheetMetrics::PAGE_DRAG_EPSILON
                && !(clamped == 0.0 && current != 0.0)
            {
                return;
            }
            state.thread_back_motion.offset.jump(clamped);
            cx.notify();
        });
    }

    fn release_thread_back(scope: &Rc<CommentsSheetScope>, translation: Pixels, velocity: Pixels, cx: &mut App) {
        let width = px(scope.state.read(cx).content_width);
        if !BottomSheetPageBackSwipePolicy::commits(translation, velocity, width) {
            Self::cancel_thread_back(&scope.state, cx);
            return;
        }
        Self::close_thread_page(&scope.state, &scope.interactions, cx);
    }

    fn cancel_thread_back(state: &Entity<CommentsSheetState>, cx: &mut App) {
        let now = RiseClock::monotonic();
        state.update(cx, |state, cx| {
            if *state.thread_back_motion.offset.target() == 0.0 {
                return;
            }
            state.thread_back_motion.offset.set_in(0.0, spring(0.28, 0.9), now, cx);
            cx.notify();
        });
    }

    fn hold_actions(
        scope: &CommentsSheetScope,
        comment: &GetCommentsResponse,
        cx: &mut App,
    ) -> Vec<HoldContextMenuAction> {
        let facts = CommentRowFacts::shared(cx);
        let (viewer, acts_as_author, is_expanded) = {
            let facts = facts.read(cx);
            (facts.viewer.clone(), facts.acts_as_author, facts.expanded.contains(&comment.id))
        };
        let mine = RiseIdentity::is_own(
            &RiseIdentity::author(Some(comment.author_id.as_str()), None, comment.author.id.as_deref()),
            viewer.as_deref(),
            None,
        );
        let mut items = Vec::new();
        if !comment.is_moderation_disabled() {
            items.push(
                HoldContextMenuAction::new("reply", tr("contextMenu_reply")).system_image("arrowshape.turn.up.left"),
            );
            if !comment.content.is_empty() {
                items.push(HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"));
            }
            if is_expanded {
                let less = tr("profile_bio_less");
                let mut chars = less.chars();
                let title = chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default();
                items.push(HoldContextMenuAction::new("collapse", title).system_image("chevron.up"));
            }
        }
        if Self::comment_share_post_id(scope, comment, cx).is_some() && !comment.is_temp {
            items.push(HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"));
        }
        if mine || acts_as_author {
            items.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        }
        if !mine {
            items.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        items
    }

    fn handle_action(
        scope: &Rc<CommentsSheetScope>,
        id: &str,
        comment: &GetCommentsResponse,
        is_inside_thread: bool,
        cx: &mut App,
    ) {
        match id {
            "reply" => {
                if is_inside_thread {
                    scope.interactions.update(cx, |interactions, cx| interactions.start_reply(comment, cx));
                } else {
                    Self::open_thread(scope, comment, true, cx);
                }
            }
            "collapse" => {
                let interactions = scope.interactions.clone();
                let comment_id = comment.id.clone();
                HoldContextMenuPresenter::after_dismiss(
                    move |cx: &mut App| {
                        interactions
                            .update(cx, |interactions, cx| interactions.set_comment_text_expanded(&comment_id, false, cx));
                    },
                    cx,
                );
            }
            "copy" => {
                let text =
                    if comment.original_content.is_empty() { &comment.content } else { &comment.original_content };
                UIPasteboard::general().set_string(text, cx);
            }
            "share" => {
                let Some(post_id) = Self::comment_share_post_id(scope, comment, cx) else {
                    return;
                };
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_comment(comment, &post_id, cx));
            }
            "delete" => {
                scope.interactions.update(cx, |interactions, cx| {
                    interactions.selected_comment = Some(comment.clone());
                    interactions.delete_selected_comment(cx);
                });
            }
            "report" => {
                let comment_id = comment.id.clone();
                let request_id =
                    HoldContextMenuPresenter::shared(cx).read(cx).active_request().map(|request| request.id);
                if let Some(request_id) = request_id {
                    HoldContextMenuPresenter::dismiss_if_active(request_id, cx);
                }
                Self::close_comments_for_avatar_navigation(
                    scope,
                    move |cx| {
                        if let Some(id) = int_from_string(&comment_id) {
                            ReportSheetStore::shared(cx)
                                .update(cx, |store, cx| store.present(ReportTarget::Comment(id), None, None, cx));
                        }
                    },
                    cx,
                );
            }
            _ => {}
        }
    }

    fn comment_share_post_id(
        scope: &CommentsSheetScope,
        comment: &GetCommentsResponse,
        cx: &mut App,
    ) -> Option<String> {
        let value = if comment.post_id.is_empty() {
            CommentRowFacts::shared(cx).read(cx).list_owner_post_id.clone().unwrap_or_default()
        } else {
            comment.post_id.clone()
        };
        RiseonlyPublicLink::PostComment { post_id: value.clone(), comment_id: comment.id.clone() }
            .web_url(AppEnvironment::current())?;
        Some(value)
    }

    fn resolved_author_id(comment: &GetCommentsResponse) -> Option<String> {
        let stable = trimming_whitespaces_and_newlines(&comment.author_id).to_string();
        let nested = comment
            .author
            .id
            .as_deref()
            .map(|id| trimming_whitespaces_and_newlines(id).to_string())
            .unwrap_or_default();
        let resolved = if stable.is_empty() { nested } else { stable };
        (!resolved.is_empty()).then_some(resolved)
    }

    fn visible_story_user_ids(scope: &CommentsSheetScope, cx: &mut App) -> Rc<Vec<String>> {
        let list = Self::list_snapshot(scope, cx);
        let thread = Self::thread_snapshot(scope, cx);
        let root = scope.interactions.read(cx).thread_root().cloned();
        let root_id = root.as_ref().map(|root| root.id.clone());
        let same = |old: &Option<Arc<RiseCommentListSnapshot>>, new: &Option<Arc<RiseCommentListSnapshot>>| match (old, new)
        {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        let unchanged = scope.state.read(cx).story_ids_memo.as_ref().filter(|memo| {
            memo.root_id == root_id && same(&memo.list, &list) && same(&memo.thread, &thread)
        });
        if let Some(memo) = unchanged {
            return memo.user_ids.clone();
        }
        let mut ids = BTreeSet::new();
        for snapshot in [&list, &thread].into_iter().flatten() {
            ids.extend(snapshot.items.iter().filter_map(Self::resolved_author_id));
        }
        if let Some(root) = &root {
            ids.extend(Self::resolved_author_id(root));
        }
        let user_ids = Rc::new(ids.into_iter().collect::<Vec<_>>());
        let memo = CommentsSheetStoryIdsMemo { list, thread, root_id, user_ids: user_ids.clone() };
        scope.state.update(cx, |state, _| state.story_ids_memo = Some(memo));
        user_ids
    }

    fn load_visible_story_states(user_ids: &[String], cx: &mut App) {
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_stories_state(user_ids, cx));
    }

    fn open_comment_author_profile(scope: &Rc<CommentsSheetScope>, comment: &GetCommentsResponse, cx: &mut App) {
        let Some((user_id, tag)) = Self::comment_author_target(comment) else {
            return;
        };
        let navigation = scope.clone();
        Self::close_comments_for_avatar_navigation(
            scope,
            move |cx| Self::nav_push(&navigation, RootRoute::UserPage { user_id: Some(user_id), tag }, cx),
            cx,
        );
    }

    fn open_comment_mention(scope: &Rc<CommentsSheetScope>, tag: &str, cx: &mut App) {
        let tag = tag.to_owned();
        Self::close_comments_for_avatar_navigation(
            scope,
            move |cx| {
                PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
                    store.enqueue(PushDeepLink::PublicTag { tag }, cx);
                });
            },
            cx,
        );
    }

    fn open_comment_link(scope: &Rc<CommentsSheetScope>, raw: &str, cx: &mut App) {
        let is_internal = ChatLinkOpen::url(raw)
            .is_some_and(|url| UniversalLinkParser::parse(&url, AppEnvironment::current()).is_some());
        if !is_internal {
            ChatLinkOpen::open(raw, cx);
            return;
        }
        let raw = raw.to_owned();
        Self::close_comments_for_avatar_navigation(scope, move |cx| ChatLinkOpen::open(&raw, cx), cx);
    }

    fn open_comment_author_story(scope: &Rc<CommentsSheetScope>, comment: &GetCommentsResponse, cx: &mut App) {
        let Some((user_id, tag)) = Self::comment_author_target(comment) else {
            return;
        };
        let navigation = scope.clone();
        Self::close_comments_for_avatar_navigation(
            scope,
            move |cx| {
                let on_unavailable_id = user_id.clone();
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.open_stories_for_user_id(
                        &user_id,
                        Some(Rc::new(move |cx: &mut App| {
                            Self::nav_push(
                                &navigation,
                                RootRoute::UserPage { user_id: Some(on_unavailable_id.clone()), tag: tag.clone() },
                                cx,
                            );
                        })),
                        cx,
                    );
                });
            },
            cx,
        );
    }

    fn comment_author_target(comment: &GetCommentsResponse) -> Option<(String, Option<String>)> {
        if comment.author.is_deleted {
            return None;
        }
        let user_id = Self::resolved_author_id(comment)?;
        let tag = (!comment.author.tag.is_empty()).then(|| comment.author.tag.clone());
        Some((user_id, tag))
    }

    fn close_comments_for_avatar_navigation(
        scope: &CommentsSheetScope,
        action: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        scope.interactions.update(cx, |interactions, cx| {
            interactions.comments_sheet_visible = false;
            interactions.close_comments_sheet(cx);
        });
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(BottomSheetMetrics::DISMISS_DURATION + 0.05)).await;
            let _ = cx.update(action);
        })
        .detach();
    }

    /// the composer as its own cached view in a slot as tall as its last measured height
    fn footer_slot(scope: &Rc<CommentsSheetScope>, window: &mut Window, cx: &mut App) -> AnyElement {
        if scope.post.read(cx).selected_post.is_none() {
            return Empty.into_any_element();
        }
        let height = scope.state.read(cx).footer_height;
        let (state, post) = (scope.state.clone(), scope.post.clone());
        let view = window.use_keyed_state("comments-sheet-footer-view", cx, {
            let (state, post) = (state.clone(), post.clone());
            move |_, _| CommentsSheetFooterView { state, post }
        });
        view.update(cx, |footer, cx| {
            if footer.state != state || footer.post != post {
                footer.state = state;
                footer.post = post;
                cx.notify();
            }
        });
        div()
            .w_full()
            .h(px(height))
            .block_mouse_except_scroll()
            .child(AnyView::from(view).cached(StyleRefinement::default().size_full()))
            .into_any_element()
    }

    fn footer_content(state: &Entity<CommentsSheetState>, post: &Entity<PostServicesStore>, cx: &mut App) -> AnyElement {
        if post.read(cx).selected_post.is_none() {
            return Empty.into_any_element();
        }
        let theme = Self::theme(cx);
        let keyboard_state = state.clone();
        let lift_state = state.clone();
        let height_state = state.clone();
        div()
            .relative()
            .w_full()
            .child(Self::footer_chrome(&theme, state.read(cx).footer_height, cx))
            .children(comment_input_hooks().map(|hooks| {
                (hooks.composer)(
                    "comments-sheet-composer",
                    Box::new(move |is_keyboard_visible: bool, _: f64, cx: &mut App| {
                        if !is_keyboard_visible {
                            return;
                        }
                        keyboard_state.update(cx, |state, cx| {
                            state.selected_sheet_fraction = EXPANDED_SNAP_FRACTION;
                            cx.notify();
                        });
                    }),
                    Box::new(move |lift: f32, cx: &mut App| {
                        lift_state.update(cx, |state, cx| {
                            state.picker_lift = lift;
                            cx.notify();
                        });
                    }),
                )
            }))
            .child(measure("comments-sheet-footer-height", move |bounds: Bounds<Pixels>, _, cx| {
                let mut height = CommentsFooterHeightKey::DEFAULT_VALUE;
                CommentsFooterHeightKey::reduce(&mut height, f32::from(bounds.size.height));
                height_state.update(cx, |state, cx| {
                    if (height - state.footer_height).abs() <= 0.5 {
                        return;
                    }
                    state.footer_height = height;
                    cx.notify();
                });
            }))
            .into_any_element()
    }

    fn root_sort_icon(sort: CommentSortType) -> &'static str {
        match sort {
            CommentSortType::Feed => "chart.line.uptrend.xyaxis",
            CommentSortType::New => RiseSymbol::SORT_NEWEST,
            CommentSortType::Old => RiseSymbol::SORT_OLDEST,
            CommentSortType::My => "person.crop.circle",
        }
    }

    fn replies_sort_icon(sort: RepliesSortType) -> &'static str {
        match sort {
            RepliesSortType::Popular => "chart.line.uptrend.xyaxis",
            RepliesSortType::New => RiseSymbol::SORT_NEWEST,
            RepliesSortType::Old => RiseSymbol::SORT_OLDEST,
            RepliesSortType::My => "person.crop.circle",
        }
    }
}

impl RenderOnce for CommentsSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Self::theme(cx);
        let is_preview = self.preview_comments.is_some();
        let docked = self.docked;
        if !docked && !is_preview && comments_sheet_aside_hooks().is_some_and(|hooks| (hooks.docks_comments)(cx)) {
            tracing::warn!(target: "riseonly::X1421", "window comments sheet skipped, panel docks");
            return Empty.into_any_element();
        }
        let state = window.use_keyed_state("comments-sheet-state", cx, move |window, cx| {
            CommentsSheetState::new(is_preview, window.window_handle(), cx)
        });
        let interactions = CommentInteractionsStore::shared(cx);
        let scope = Rc::new(CommentsSheetScope {
            state: state.clone(),
            interactions: interactions.clone(),
            services: CommentServicesStore::shared(cx),
            post: PostServicesStore::shared(cx),
            nav_store: env::<NavigationStoreKey>(cx),
            presentation_override: self.presentation_override.clone(),
            preview_comments: self.preview_comments.map(Rc::new),
            docked,
        });

        let visible = self.presentation_override.unwrap_or_else(|| {
            let reader = interactions.clone();
            let writer = interactions.clone();
            Binding::new(
                move |cx: &App| reader.read(cx).comments_sheet_visible,
                move |new_value: bool, _: &mut Window, cx: &mut App| {
                    writer.update(cx, |interactions, cx| {
                        interactions.comments_sheet_visible = new_value;
                        if !new_value {
                            interactions.close_comments_sheet(cx);
                        }
                        cx.notify();
                    });
                },
            )
        });

        let window_height = window.viewport_size().height;
        let picker_lift = state.read(cx).picker_lift;
        let has_reel = Self::reel_post(&scope, cx).is_some();
        let relay = CommentsSheetMotionRelay::shared(cx);

        if !is_preview {
            let is_visible = interactions.read(cx).comments_sheet_visible;
            let change_scope = scope.clone();
            let change_relay = relay.clone();
            on_change("comments-sheet-visible", is_visible, false, window, cx, move |_, is_visible, _, cx| {
                let scope = change_scope;
                scope.state.update(cx, |state, cx| {
                    if *is_visible {
                        state.reset_thread_motion();
                        state.selected_sheet_fraction = Self::INITIAL_SNAP_FRACTION;
                    } else {
                        state.thread_back_motion.offset.jump(0.0);
                    }
                    cx.notify();
                });
                if *is_visible {
                    let selected_id = scope.post.read(cx).selected_post.as_ref().map(|selected| selected.id.clone());
                    if let Some(selected_id) = selected_id.filter(|_| !docked) {
                        change_relay.update(cx, |relay, cx| {
                            relay.engage(&selected_id, Self::INITIAL_SNAP_FRACTION, window_height, cx);
                        });
                    }
                    let user_ids = Self::visible_story_user_ids(&scope, cx);
                    Self::load_visible_story_states(&user_ids, cx);
                } else {
                    scope.interactions.update(cx, |interactions, cx| interactions.close_thread(cx));
                }
            });

            let user_ids = Self::visible_story_user_ids(&scope, cx);
            on_change("comments-sheet-story-user-ids", user_ids, false, window, cx, move |_, user_ids, _, cx| {
                Self::load_visible_story_states(user_ids, cx);
            });
        }

        let selected_snap_fraction = Binding::entity(
            state.clone(),
            |state| state.selected_sheet_fraction,
            |state, fraction, cx| {
                state.selected_sheet_fraction = fraction;
                cx.notify();
            },
        );

        let content_scope = scope.clone();
        let footer_scope = scope.clone();
        if docked {
            let content = Self::navigation_content(&content_scope, window, cx);
            let footer = Self::footer_slot(&footer_scope, window, cx);
            return Self::docked_frame(content, footer, picker_lift, &theme);
        }
        let mut sheet = BottomSheetUi::new(
            "comments-sheet",
            visible,
            move |window: &mut Window, cx: &mut App| {
                if content_scope.preview_comments.is_some() {
                    let theme = Self::theme(cx);
                    Self::comments_page(&content_scope, &theme, cx)
                } else {
                    Self::navigation_content(&content_scope, window, cx)
                }
            },
            move |window: &mut Window, cx: &mut App| {
                if footer_scope.preview_comments.is_some() {
                    Self::hit_testing_disabled(comment_input_hooks().map_or_else(
                        || Empty.into_any_element(),
                        |hooks| (hooks.preview_composer)("comments-sheet-preview-composer", Some(String::new())),
                    ))
                    .w_full()
                    .into_any_element()
                } else {
                    Self::footer_slot(&footer_scope, window, cx)
                }
            },
        )
        .snap_fractions(vec![Self::INITIAL_SNAP_FRACTION, EXPANDED_SNAP_FRACTION])
        .selected_snap_fraction(Some(selected_snap_fraction))
        .snap_animation_duration(0.40)
        .content_under_handle(true)
        .handle_over_content_is_light(false)
        .experimental_bottom_sheet(true)
        .experimental_uses_liquid_glass_surface(false)
        .full_page_shows_drag_indicator(true)
        .keyboard_avoidance_height(picker_lift)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(true))
        .footer_horizontal_padding(Some(0.0))
        .footer_top_padding(Some(0.0))
        .content_scrollable(true)
        .full_page_interaction_locked(Self::is_sheet_interaction_locked(cx))
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()))
        .backdrop_opacity(if is_preview || !has_reel { None } else { Some(0.0) });

        if !is_preview {
            let tap_relay = relay.clone();
            let frame_relay = relay.clone();
            let dismiss_relay = relay;
            sheet = sheet
                .backdrop_tap_interceptor(move |point, _, cx| {
                    tap_relay.update(cx, |relay, cx| relay.handle_backdrop_tap(point, cx))
                })
                .on_live_sheet_frame(move |frame, cx| {
                    frame_relay.update(cx, |relay, cx| relay.update(frame, window_height, cx));
                })
                .on_did_dismiss(move |_, cx| {
                    dismiss_relay.update(cx, |relay, cx| relay.disengage(cx));
                });
        }
        sheet.into_any_element()
    }
}

/// window comments sheet as its own view for the root stack
pub struct CommentsSheetHost;

impl Render for CommentsSheetHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        CommentsSheet::new()
    }
}

/// comments docked in the right panel as their own view
/// sheet animation frames dirty this host and the panel page under it stays cached
#[derive(Default)]
pub struct DockedCommentsSheetHost {
    heard_list: Option<(RiseCommentScope, Entity<RiseCommentListBox>, Subscription)>,
}

impl DockedCommentsSheetHost {
    fn hear_open_list(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = CommentServicesStore::shared(cx).read(cx).list_scope() else {
            self.heard_list = None;
            return;
        };
        let list = RiseCommentPresentation::shared(cx).update(cx, |presentation, cx| presentation.box_for(&scope, cx));
        let is_heard = self
            .heard_list
            .as_ref()
            .is_some_and(|(heard_scope, heard, _)| *heard_scope == scope && heard.entity_id() == list.entity_id());
        if is_heard {
            return;
        }
        let subscription = cx.observe(&list, |_, list, cx| {
            let snapshot = list.read(cx).snapshot();
            tracing::warn!(
                target: "riseonly::X2027",
                "docked list commit scope={:?} load_state={:?} items={} revision={}",
                snapshot.scope,
                snapshot.load_state,
                snapshot.items.len(),
                snapshot.revision
            );
            cx.notify();
        });
        self.heard_list = Some((scope, list, subscription));
    }
}

impl Render for DockedCommentsSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.hear_open_list(cx);
        CommentsSheet::new().docked(true)
    }
}

struct CommentsSheetFooterView {
    state: Entity<CommentsSheetState>,
    post: Entity<PostServicesStore>,
}

impl Render for CommentsSheetFooterView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .child(div().flex_none().w_full().child(CommentsSheet::footer_content(&self.state, &self.post, cx)))
    }
}

struct CommentsSheetRowView {
    scope: Rc<CommentsSheetScope>,
    row: CommentListRow,
}

impl Render for CommentsSheetRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = CommentsSheet::theme(cx);
        CommentsSheet::root_row_content(&self.scope, &self.row, &theme, cx)
    }
}

enum Accessory {}

impl Accessory {
    const REEL_HEADER: &'static str = "reel_header";
    const EMPTY: &'static str = "empty";
    const LIST_ERROR: &'static str = "list_error";
    const PAGE_LOADER: &'static str = "page_loader";
    const PAGE_RETRY: &'static str = "page_retry";
    const THREAD_SKELETON: &'static str = "thread_skeleton";
    const THREAD_ERROR: &'static str = "thread_error";
    const THREAD_EMPTY: &'static str = "thread_empty";
    const THREAD_RETRY: &'static str = "thread_retry";
}

enum HeaderLeading {
    Close,
    Back,
}

enum CommentsFooterHeightKey {}

impl CommentsFooterHeightKey {
    const DEFAULT_VALUE: f32 = 0.0;

    fn reduce(value: &mut f32, next_value: f32) {
        *value = value.max(next_value);
    }
}

/// live position of the replies page during its back swipe
pub struct CommentsThreadBackMotion {
    pub offset: Animated<f32>,
}

impl Default for CommentsThreadBackMotion {
    fn default() -> Self {
        Self { offset: Animated::new(0.0) }
    }
}

enum CommentsThreadPageMotionRole {
    Comments,
    Replies,
}

/// transform only, offsets one page by the live back swipe position
/// comments page parks slightly left under the replies and eases back, replies page follows
/// the finger
struct CommentsThreadPageMotion {
    role: CommentsThreadPageMotionRole,
    width: f32,
    presence: f32,
    back_offset: f32,
}

impl CommentsThreadPageMotion {
    fn x(&self) -> f32 {
        let back_offset = self.back_offset.clamp(0.0, self.width);
        match self.role {
            CommentsThreadPageMotionRole::Comments => -self.width * 0.08 * self.presence + back_offset * 0.08,
            CommentsThreadPageMotionRole::Replies => self.width * (1.0 - self.presence) + back_offset,
        }
    }

    fn apply(self, content: impl IntoElement) -> Div {
        div().absolute().top_0().bottom_0().w_full().left(px(self.x())).child(content)
    }
}

#[cfg(test)]
#[path = "comments_sheet_tests.rs"]
mod tests;

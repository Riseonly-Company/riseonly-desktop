use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, ElementId, Empty, Entity, Hsla, ListAlignment, ListOffset, Pixels, Point, SharedString, Subscription,
    StyleRefinement, Task, Window, transparent_black,
};
use rise_i18n::tr;
use rise_limits::service_limit::POST_LIKES_SEARCH_MAX_CHARS;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAlignment, HoldContextSocialMenu};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::user_logo_story_hooks::user_logo_story_hooks;
use rise_ui::{MainText, TextAlignT};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::premium_badge::UserNameWithBadges;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::core::widgets::sheets::report_sheet::{ReportSheetStore, ReportSubject};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::stores::post_actions::post_likes_types::PostLikeUser;
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_likes_services::PostLikesTarget;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::components::user_block_scope_dialog::UserBlockScopeDialogModifiers;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;
use crate::modules::user::components::user_relationship_action_button::UserRelationshipActionButton;
use crate::modules::user::components::user_relationship_list_contract::UserRelationshipRowMetrics;
use crate::modules::user::stores::user::user_actions::user_types::{FollowButtonState, UserListRelationship};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const SHEET_ID: &str = "post-likes-sheet";
const COLLAPSED_SHEET_FRACTION: f32 = 0.58;
const EXPANDED_SHEET_FRACTION: f32 = 0.93;
const HEADER_HEIGHT: f32 = 116.0;

fn element_id(suffix: impl AsRef<str>) -> ElementId {
    ElementId::Name(format!("{SHEET_ID}-{}", suffix.as_ref()).into())
}

/// view state and the row list for the lazy list
/// their observers notify whichever view renders this sheet
struct PostLikesSheetState {
    selected_sheet_fraction: f32,
    query: String,
    search_active: bool,
    pending_friend_removal: Option<PostLikeUser>,
    pending_block_user_id: Option<String>,
    list: PostLikesSheetList,
    search_list: PostLikesSheetList,
    full_list_anchor: Option<(String, Pixels)>,
    was_shown: bool,
    _subscriptions: Vec<Subscription>,
}

impl PostLikesSheetState {
    fn notify_if_shown(cx: &mut Context<Self>) {
        if PostServicesStore::shared(cx).read(cx).post_likes_sheet_visible {
            cx.notify();
        }
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let service = PostServicesStore::shared(cx);
        let user_service = UserServicesStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self {
            selected_sheet_fraction: COLLAPSED_SHEET_FRACTION,
            query: String::new(),
            search_active: false,
            pending_friend_removal: None,
            pending_block_user_id: None,
            list: PostLikesSheetList::new(cx),
            search_list: PostLikesSheetList::new(cx),
            full_list_anchor: None,
            was_shown: service.read(cx).post_likes_sheet_visible,
            _subscriptions: vec![
                cx.observe(&service, |state, service, cx| {
                    let shown = service.read(cx).post_likes_sheet_visible;
                    if shown || shown != state.was_shown {
                        state.was_shown = shown;
                        cx.notify();
                    }
                }),
                cx.observe(&user_service, |_, _, cx| Self::notify_if_shown(cx)),
                cx.observe(&cache_revisions, |_, _, cx| Self::notify_if_shown(cx)),
                cx.observe(&pending_requests, |_, _, cx| Self::notify_if_shown(cx)),
                cx.observe(&hold_context_menu, |_, _, cx| Self::notify_if_shown(cx)),
            ],
        }
    }

    fn reset_presentation(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.search_active = false;
        self.pending_friend_removal = None;
        self.pending_block_user_id = None;
        self.selected_sheet_fraction = COLLAPSED_SHEET_FRACTION;
        self.list = PostLikesSheetList::new(cx);
        self.search_list = PostLikesSheetList::new(cx);
        self.full_list_anchor = None;
        cx.notify();
    }

    fn set_query(&mut self, value: String) {
        let value: String = value.chars().take(POST_LIKES_SEARCH_MAX_CHARS as usize).collect();
        let was_full = PostServicesStore::normalized_post_likes_search(Some(self.query.as_str())).is_none();
        let is_full = PostServicesStore::normalized_post_likes_search(Some(value.as_str())).is_none();
        if was_full && !is_full {
            self.full_list_anchor = self.list.top_anchor();
        }
        self.query = value;
    }

    fn sync_list(&mut self, query: &str, ids: Vec<String>) {
        if !query.is_empty() {
            self.search_list.sync(ids);
            return;
        }
        self.list.sync(ids);
        if let Some((id, offset)) = self.full_list_anchor.take() {
            tracing::warn!(
                target: "riseonly::X1443",
                "likes full list restore id={id} rows={} found={:?} top_before={}",
                self.list.ids.len(),
                self.list.ids.iter().position(|next| *next == id),
                self.list.list.state().logical_scroll_top().item_ix
            );
            self.list.scroll_to_id(&id, offset);
        }
    }

    fn list_for(&self, query: &str) -> &PostLikesSheetList {
        if query.is_empty() { &self.list } else { &self.search_list }
    }
}

struct PostLikesSheetList {
    list: ListUi,
    ids: Vec<String>,
}

impl PostLikesSheetList {
    fn new(cx: &mut Context<PostLikesSheetState>) -> Self {
        Self { list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top), ids: Vec::new() }
    }

    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        if !self.ids.is_empty() && ids.len() > self.ids.len() && ids.starts_with(&self.ids) {
            self.list.appended(ids.len() - self.ids.len());
        } else {
            let top = self.list.state().logical_scroll_top();
            let anchor = self.ids.get(top.item_ix).and_then(|id| ids.iter().position(|next| next == id));
            self.list.replaced(ids.len());
            if let Some(item_ix) = anchor {
                self.list.state().scroll_to(ListOffset { item_ix, offset_in_item: top.offset_in_item });
            }
        }
        self.ids = ids;
    }

    fn top_anchor(&self) -> Option<(String, Pixels)> {
        let top = self.list.state().logical_scroll_top();
        self.ids.get(top.item_ix).map(|id| (id.clone(), top.offset_in_item))
    }

    fn scroll_to_id(&self, id: &str, offset_in_item: Pixels) {
        if let Some(item_ix) = self.ids.iter().position(|next| next == id) {
            self.list.state().scroll_to(ListOffset { item_ix, offset_in_item });
        }
    }
}

#[derive(IntoElement, Default)]
pub struct PostLikesSheet;

impl PostLikesSheet {
    pub fn new() -> Self {
        Self
    }
}

/// likes sheet as its own view
/// its scroll and drag frames dirty this host only and the root with the page under stays cached
pub struct PostLikesSheetHost;

impl Render for PostLikesSheetHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        PostLikesSheet::new()
    }
}

impl RenderOnce for PostLikesSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(element_id("state"), cx, |_, cx: &mut Context<PostLikesSheetState>| {
            PostLikesSheetState::new(cx)
        });
        let service = PostServicesStore::shared(cx);
        let is_visible = service.read(cx).post_likes_sheet_visible;

        let change_state = state.clone();
        on_change(element_id("visible"), is_visible, false, window, cx, move |_, _, _, cx| {
            change_state.update(cx, |state, cx| state.reset_presentation(cx));
        });

        let visible = Binding::new(
            {
                let service = service.clone();
                move |cx: &App| service.read(cx).post_likes_sheet_visible
            },
            |value: bool, _, cx| {
                if !value {
                    PostLikesSheetParts::close_post_likes(cx);
                }
            },
        );

        let parts = PostLikesSheetParts { state: state.clone(), service, nav_store: env::<NavigationStoreKey>(cx) };
        let dismiss_state = state.clone();
        let hold_is_active = HoldContextMenuPresenter::has_active_request(cx);

        BottomSheetUi::new(SHEET_ID, visible, move |window, cx| parts.sheet_content(window, cx), |_, _| Empty)
            .snap_fractions(vec![COLLAPSED_SHEET_FRACTION, EXPANDED_SHEET_FRACTION])
            .selected_snap_fraction(Some(Binding::entity(
                state,
                |state: &PostLikesSheetState| state.selected_sheet_fraction,
                |state, value, _| state.selected_sheet_fraction = value,
            )))
            .snap_animation_duration(0.4)
            .experimental_bottom_sheet(true)
            .experimental_uses_liquid_glass_surface(false)
            .full_page_shows_drag_indicator(true)
            .full_page_hides_footer(true)
            .full_page_avoids_keyboard_when_footer_hidden(true)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .full_page_interaction_locked(hold_is_active)
            .sheet_background(Some(theme.bg._100))
            .footer_background(Some(transparent_black()))
            .on_dismiss(move |_, cx| dismiss_state.update(cx, |state, cx| state.reset_presentation(cx)))
    }
}

#[derive(Clone)]
struct PostLikesSheetParts {
    state: Entity<PostLikesSheetState>,
    service: Entity<PostServicesStore>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PostLikesSheetParts {
    fn sheet_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let block_target = Binding::entity(
            self.state.clone(),
            |state: &PostLikesSheetState| state.pending_block_user_id.clone(),
            |state, value, _| state.pending_block_user_id = value,
        );
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100.opacity(0.94))
            .child(div().absolute().inset_0().child(self.likes_data(&theme, window, cx)))
            .child(div().absolute().top_0().left_0().right_0().child(self.header(&theme, cx)))
            .user_block_scope_dialog(element_id("block-scope"), block_target)
            .into_any_element()
    }

    fn likes_data(&self, theme: &AppTheme, _window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(target) = self.service.read(cx).post_likes_target.clone() else {
            return Empty.into_any_element();
        };
        let query = self.state.read(cx).query.clone();
        let search = Some(query.as_str());
        let (users, status, has_more, relative_id) = self.service.update(cx, |service, cx| {
            (
                service.post_likes(&target, search, cx),
                service.post_likes_status(&target, search, cx),
                service.post_likes_has_more(&target, search, cx),
                service.post_likes_relative_id(&target, search, cx),
            )
        });
        let cache_key = PostServicesStore::post_likes_cache_key(&target, search);
        let is_empty = users.is_empty();
        let no_data_text = if query.is_empty() { tr("post_likes_empty") } else { tr("post_likes_search_empty") };

        let list_parts = self.clone();
        let list_theme = theme.clone();
        let list_target = target.clone();
        let list_query = query.clone();
        let error_parts = self.clone();
        let error_theme = theme.clone();
        SaiDataRender::new(status, move |_, cx| {
            list_parts.likes_list(users, list_target, list_query, cache_key, has_more, relative_id, list_theme, cx)
        })
        .is_empty(is_empty)
        .no_data_text(no_data_text)
        .error_text(tr("post_likes_load_error"))
        .message_height_percent(42.0)
        .no_data_height_percent(35.0)
        .sai_data_render_loading(|_, _| {
            scroll_view(element_id("skeleton"))
                .scroll_disabled(true)
                .shows_indicators(false)
                .child(div().w_full().pt(px(HEADER_HEIGHT)).child(UserListSkeleton::new().rows(8)))
        })
        .sai_data_render_error(move |_, _| error_parts.load_error(&target, &query, &error_theme))
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn likes_list(
        &self,
        users: Vec<PostLikeUser>,
        target: PostLikesTarget,
        query: String,
        cache_key: String,
        has_more: bool,
        relative_id: Option<i64>,
        theme: AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let ids: Vec<String> = users.iter().map(|user| user.id.clone()).collect();
        self.state.update(cx, |state, _| state.sync_list(&query, ids));

        let parts = self.clone();
        let cache_key = SharedString::from(cache_key);
        let relative_id = relative_id.map(|id| id.to_string());
        let item_count = users.len();
        let refresh_target = target.clone();
        let refresh_query = query.clone();
        let state = self.state.read(cx);
        let list_id = if query.is_empty() { "list" } else { "search-list" };
        state
            .list_for(&query)
            .list
            .refreshable(
                element_id(list_id),
                move |_, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.refresh_post_likes(&refresh_target, Some(refresh_query.as_str()), cx);
                    });
                    Task::ready(())
                },
                move |index, window, cx| {
                    let Some(user) = users.get(index) else {
                        return Empty.into_any_element();
                    };
                    let load_target = target.clone();
                    let load_query = query.clone();
                    vstack(0.0)
                        .w_full()
                        .when(index == 0, |row| row.pt(px(HEADER_HEIGHT)))
                        .child(parts.like_row(user, &theme, window, cx))
                        .child(div().w_full().h(px(0.5)).bg(theme.border._100))
                        .sai_pagination_prefetch(
                            element_id(format!("row-{}", user.id)),
                            SaiPaginationPrefetchOptions::new(
                                index,
                                item_count,
                                cache_key.clone(),
                                relative_id.clone(),
                                has_more,
                            )
                            .threshold(8),
                            move |_, cx| {
                                PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                    interactions.load_more_post_likes(&load_target, Some(load_query.as_str()), cx);
                                });
                            },
                        )
                        .into_any_element()
                },
            )
            .into_any_element()
    }

    fn load_error(&self, target: &PostLikesTarget, query: &str, theme: &AppTheme) -> Div {
        let target = target.clone();
        let query = query.to_owned();
        vstack(14.0)
            .size_full()
            .justify_center()
            .pt(px(HEADER_HEIGHT))
            .px(px(24.0))
            .child(
                MainText::new(tr("post_likes_load_error")).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary),
            )
            .child(
                LiquidGlassButton::new(element_id("retry"), LiquidGlassButtonShape::Capsule)
                    .accessibility_label(Some(tr("common_retry")))
                    .on_press(move |_, cx| {
                        PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.refresh_post_likes(&target, Some(query.as_str()), cx);
                        });
                    })
                    .child(MainText::new(tr("common_retry")).px(14.0).color(theme.text.primary)),
            )
    }

    fn header(&self, theme: &AppTheme, cx: &App) -> Div {
        let search_active = self.state.read(cx).search_active;
        let query_binding = Binding::entity(
            self.state.clone(),
            |state: &PostLikesSheetState| state.query.clone(),
            |state, value: String, _| state.set_query(value),
        );
        let placeholder = SharedString::from(tr("search_placeholder"));
        let search_field = if search_active {
            let search_parts = self.clone();
            SearchFieldUi::new(element_id("search-inline"), query_binding)
                .placeholder(placeholder)
                .on_change(move |value, _, cx| search_parts.search(value, cx))
                .uses_inline_text_field(true)
                .auto_focus(true)
        } else {
            let activate_parts = self.clone();
            SearchFieldUi::new(element_id("search"), query_binding)
                .placeholder(placeholder)
                .on_activate(move |_, cx| activate_parts.activate_search(cx))
        };

        vstack(4.0)
            .w_full()
            .bg(theme.bg._100)
            .occlude()
            .child(
                SheetHeaderBar::new(element_id("header"), tr("post_likes_title"), |_, cx| Self::close_post_likes(cx))
                .right_system_image(None),
            )
            .child(div().w_full().px(px(12.0)).pb(px(10.0)).child(search_field))
    }

    fn like_row(&self, user: &PostLikeUser, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let user_service = UserServicesStore::shared(cx);
        let (relationship, viewer_blocked, is_pending) = user_service.update(cx, |service, cx| {
            (
                service.resolved_follow_relationship(&user.id, user.relationship(), cx),
                service.viewer_block_scope(&user.id, cx).is_some(),
                service.is_follow_interaction_pending(&user.id),
            )
        });
        let current_user_id =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        let story_ring = user_logo_story_hooks().map_or((false, false), |hooks| {
            let has_active = (hooks.has_active_stories)(Some(&user.id), cx);
            (has_active, has_active && (hooks.has_unseen_stories)(Some(&user.id), cx))
        });
        let props = PostLikeRowProps {
            user: user.clone(),
            story_ring,
            background: theme.bg._100,
            relationship,
            viewer_blocked,
            is_pending,
            is_self: user.id == current_user_id,
        };
        PostLikeRowView::element(self, props, window, cx)
    }

    /// row a cached like row view draws from its props
    fn row_body(&self, props: &PostLikeRowProps, theme: &AppTheme) -> AnyElement {
        let user = &props.user;
        let relationship = props.relationship;
        let is_pending = props.is_pending;
        let viewer_blocked = props.viewer_blocked;

        let actions = HoldContextSocialMenu::user_list_actions_viewer_blocked(viewer_blocked);
        let action_parts = self.clone();
        let action_user = user.clone();
        let tap_parts = self.clone();
        let tap_user = user.clone();
        let content_parts = self.clone();
        let content_user = user.clone();
        let content_theme = theme.clone();
        let content_ring = props.story_ring;
        let hold = HoldContextMenuContainer::new(
            element_id(format!("hold-{}", user.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |window: &mut Window, cx: &mut App| {
                PostLikeIdentityView::element(&content_parts, &content_user, content_ring, &content_theme, window, cx)
            }),
        )
        .show_emoji_list(false)
        .lifted_background(Some(props.background))
        .on_action(Rc::new(move |action: String, _: &mut Window, cx: &mut App| {
            action_parts.social_user_action(&action_user, &action, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            tap_parts.open_profile(&tap_user, cx);
        }));

        let button = (!props.is_self).then(|| {
            let button_parts = self.clone();
            let button_user = user.clone();
            let is_friend = relationship.follow_button_state() == FollowButtonState::Friends;
            UserRelationshipActionButton::new(
                element_id(format!("relationship-{}", user.id)),
                relationship,
                is_pending,
                move |_: &mut Window, cx: &mut App| {
                    if is_friend {
                        button_parts.present_friend_removal(&button_user, cx);
                    } else {
                        button_parts.toggle_relationship(&button_user, cx);
                    }
                },
            )
        });

        hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
            .py(px(UserRelationshipRowMetrics::VERTICAL_PADDING))
            .bg(props.background.opacity(0.9))
            .child(div().flex_1().min_w_0().child(hold))
            .children(button)
            .into_any_element()
    }

    fn identity(&self, user: &PostLikeUser, story_ring: (bool, bool), theme: &AppTheme) -> Div {
        let handle = format!("@{}", user.tag);
        let press_parts = self.clone();
        let press_user = user.clone();

        let name_row = UserNameWithBadges::new(
            Some(SharedString::from(user.id.clone())),
            if user.name.is_empty() { handle.clone() } else { user.name.clone() },
        )
        .official(user.is_official)
        .premium(user.is_premium, user.premium_badge_emoji.clone(), user.name_color.clone());

        let mut names = vstack(4.0).align(HorizontalAlignment::Leading).flex_1().min_w_0().child(name_row);
        if !user.tag.is_empty() {
            names = names.child(MainText::new(handle).px(13.0).color(theme.text.secondary).number_of_lines(1).w_full());
        }

        hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .min_h(px(UserRelationshipRowMetrics::AVATAR_SIZE))
            .child(
                UserLogoUi::new(SharedString::from(format!("{SHEET_ID}-logo-{}", user.id)))
                    .source(SharedString::from(user.logo.clone()))
                    .name(SharedString::from(user.name.clone()))
                    .user_id(SharedString::from(user.id.clone()))
                    .story_ring_state(story_ring.0, story_ring.1)
                    .size(UserRelationshipRowMetrics::AVATAR_SIZE)
                    .is_button(true)
                    .on_press(move |_, cx| press_parts.open_profile(&press_user, cx))
                    .gift_worn_frame(
                        SharedString::from(format!("{SHEET_ID}-frame-{}", user.id)),
                        Some(user.id.clone()),
                        UserRelationshipRowMetrics::AVATAR_SIZE,
                    ),
            )
            .child(names)
    }

    fn activate_search(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.selected_sheet_fraction = EXPANDED_SHEET_FRACTION;
            state.search_active = true;
            cx.notify();
        });
    }

    fn search(&self, value: &str, cx: &mut App) {
        let Some(target) = self.service.read(cx).post_likes_target.clone() else {
            return;
        };
        let clears_to_loaded_list = PostServicesStore::normalized_post_likes_search(Some(value)).is_none()
            && !self.state.read(cx).list.ids.is_empty();
        if clears_to_loaded_list {
            return;
        }
        PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.search_post_likes(&target, value, cx);
        });
    }

    fn present_friend_removal(&self, user: &PostLikeUser, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.pending_friend_removal = Some(user.clone());
            cx.notify();
        });
        let cancel_state = self.state.clone();
        let confirm_parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("profile_remove_friend_title"),
            &tr("profile_remove_friend_message"),
            Some(tr("profile_remove_friend_confirm")),
            None,
            false,
            None,
            Some(Box::new(move |cx: &mut App| {
                cancel_state.update(cx, |state, cx| {
                    state.pending_friend_removal = None;
                    cx.notify();
                });
            })),
            move |cx| {
                let user = confirm_parts.state.update(cx, |state, cx| {
                    cx.notify();
                    state.pending_friend_removal.take()
                });
                if let Some(user) = user {
                    confirm_parts.toggle_relationship(&user, cx);
                }
            },
            cx,
        );
    }

    fn toggle_relationship(&self, user: &PostLikeUser, cx: &mut App) {
        let relationship = UserServicesStore::shared(cx)
            .update(cx, |service, cx| service.resolved_follow_relationship(&user.id, user.relationship(), cx));
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.toggle_subscribe_relationship(&user.id, &user.tag, relationship, cx);
        });
    }

    fn close_post_likes(cx: &mut App) {
        PostInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.close_post_likes(cx));
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn open_profile(&self, user: &PostLikeUser, cx: &mut App) {
        Self::close_post_likes(cx);
        self.push(RootRoute::UserPage { user_id: Some(user.id.clone()), tag: Some(user.tag.clone()) }, cx);
    }

    fn social_user_action(&self, user: &PostLikeUser, action: &str, cx: &mut App) {
        match action {
            "profile" => self.open_profile(user, cx),
            "message" => {
                let target = User {
                    id: user.id.clone(),
                    name: user.name.clone(),
                    tag: user.tag.clone(),
                    more: AuthorInfoMore {
                        logo: user.logo.clone(),
                        is_premium: user.is_premium,
                        is_official: user.is_official,
                        premium_badge_emoji: user.premium_badge_emoji.clone(),
                        name_color: user.name_color.clone(),
                        ..AuthorInfoMore::default()
                    },
                    ..User::default()
                };
                let chat_id = ChatInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.direct_conversation_id(&target, cx));
                if chat_id.is_empty() {
                    return;
                }
                Self::close_post_likes(cx);
                self.push(RootRoute::Chat { chat_id: Some(chat_id) }, cx);
            }
            "copy" => {
                let value = if user.tag.is_empty() { user.name.clone() } else { format!("@{}", user.tag) };
                UIPasteboard::general().set_string(&value, cx);
            }
            "mention" => {
                if !user.tag.is_empty() {
                    UIPasteboard::general().set_string(&format!("@{}", user.tag), cx);
                }
            }
            "block" => {
                if user.id != Self::current_user_id(cx) {
                    self.state.update(cx, |state, cx| {
                        state.pending_block_user_id = Some(user.id.clone());
                        cx.notify();
                    });
                }
            }
            "unblock" => {
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.set_user_block_scope(&user.id, None, |_, _| {}, cx);
                });
            }
            "report" => {
                if user.id != Self::current_user_id(cx) {
                    Self::close_post_likes(cx);
                    let subject = ReportSubject {
                        name: user.name.clone(),
                        tag: user.tag.clone(),
                        avatar_url: user.logo.clone(),
                        user_id: Some(user.id.clone()),
                    };
                    ReportSheetStore::shared(cx).update(cx, |store, cx| {
                        store.present(ReportTarget::User(user.id.clone()), Some(subject), None, cx);
                    });
                }
            }
            _ => {}
        }
    }

    fn current_user_id(cx: &mut App) -> String {
        AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default()
    }
}

/// what one like row draws, equal props keep the last frame
#[derive(Clone, PartialEq)]
struct PostLikeRowProps {
    user: PostLikeUser,
    story_ring: (bool, bool),
    background: Hsla,
    relationship: UserListRelationship,
    viewer_blocked: bool,
    is_pending: bool,
    is_self: bool,
}

/// one whole like row as a cached view
/// the list renders visible rows on every wheel frame and each row rebuilt and laid out its
/// hold container again
struct PostLikeRowView {
    parts: Option<PostLikesSheetParts>,
    props: Option<PostLikeRowProps>,
}

struct PostLikeRowHolder {
    row: Entity<PostLikeRowView>,
}

impl PostLikeRowView {
    fn element(parts: &PostLikesSheetParts, props: PostLikeRowProps, window: &mut Window, cx: &mut App) -> AnyElement {
        let height = UserRelationshipRowMetrics::AVATAR_SIZE
            .max(UserRelationshipRowMetrics::relationship_button_minimum_height())
            + 2.0 * UserRelationshipRowMetrics::VERTICAL_PADDING;
        let style = StyleRefinement::default().w_full().h(px(height));
        let row_id = element_id(format!("row-view-{}", props.user.id));
        let host = window
            .use_keyed_state(row_id, cx, |_, cx| PostLikeRowHolder { row: cx.new(|_| Self { parts: None, props: None }) })
            .read(cx)
            .row
            .clone();
        let parts = parts.clone();
        let stale = host.update(cx, |host, _| host.set_props(parts, props));
        if stale {
            AnyView::from(host).into_any_element()
        } else {
            host.cached(style).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, parts: PostLikesSheetParts, props: PostLikeRowProps) -> bool {
        self.parts = Some(parts);
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }
}

impl Render for PostLikeRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(parts), Some(props)) = (self.parts.as_ref(), self.props.as_ref()) else {
            return div().into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        parts.row_body(props, &theme)
    }
}

/// what one row identity draws, equal props keep the last frame
#[derive(Clone, PartialEq)]
struct PostLikeIdentityProps {
    user: PostLikeUser,
    story_ring: (bool, bool),
    primary: Hsla,
    secondary: Hsla,
}

/// avatar, name and handle of one like row as their own cached view
/// the list rebuilt them on every wheel frame
struct PostLikeIdentityView {
    parts: Option<PostLikesSheetParts>,
    props: Option<PostLikeIdentityProps>,
}

struct PostLikeIdentityHolder {
    identity: Entity<PostLikeIdentityView>,
}

impl PostLikeIdentityView {
    fn element(
        parts: &PostLikesSheetParts,
        user: &PostLikeUser,
        story_ring: (bool, bool),
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let props = PostLikeIdentityProps { user: user.clone(), story_ring, primary: theme.text.primary, secondary: theme.text.secondary };
        let style = StyleRefinement::default().w_full().h(px(UserRelationshipRowMetrics::AVATAR_SIZE));
        let host = window
            .use_keyed_state(element_id(format!("identity-{}", user.id)), cx, |_, cx| PostLikeIdentityHolder {
                identity: cx.new(|_| Self { parts: None, props: None }),
            })
            .read(cx)
            .identity
            .clone();
        let parts = parts.clone();
        let stale = host.update(cx, |host, _| host.set_props(parts, props));
        if stale {
            AnyView::from(host).into_any_element()
        } else {
            host.cached(style).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, parts: PostLikesSheetParts, props: PostLikeIdentityProps) -> bool {
        self.parts = Some(parts);
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }
}

impl Render for PostLikeIdentityView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(parts), Some(props)) = (self.parts.as_ref(), self.props.as_ref()) else {
            return div().into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        parts.identity(&props.user, props.story_ring, &theme).into_any_element()
    }
}

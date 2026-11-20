use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyView, App, Bounds, ElementId, Empty, Entity, ListAlignment, Pixels, Point, SharedString,
    StyleRefinement, Subscription, Task, WeakEntity, Window, canvas,
};
use rise_canon::RisePrivacyRule;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAlignment, HoldContextSocialMenu};
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::MotionSpec;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::modules::rtc::media::rise_rtc_audio_settings::RiseRTCStreamerMode;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::premium_badge::UserNameWithBadges;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::core::widgets::sheets::report_sheet::{ReportSheetStore, ReportSubject};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::stores::post_actions::post_types::AuthorInfo;
use crate::modules::user::components::user_block_scope_dialog::UserBlockScopeDialogModifiers;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;
use crate::modules::user::components::user_relationship_action_button::UserRelationshipActionButton;
use crate::modules::user::components::user_relationship_list_contract::{
    UserRelationshipRowMetrics, UserRelationshipSearchHero,
};
use crate::modules::user::stores::user::user_actions::user_types::{
    FollowButtonState, UserListEntry, UserListKind, UserListRelationship, UserRelationshipSearchContract,
};
use crate::modules::user::stores::user::user_actions::user_wire_types::UserGetProfileByTagGatewayDTOExt;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const BUTTON_SIDE: f32 = 42.0;
const SEARCH_FIELD_HEIGHT: f32 = 42.0;

type OpenProfileHandler = Rc<dyn Fn(&AuthorInfo, &mut Window, &mut App)>;
type ConfirmFriendRemovalHandler = Rc<dyn Fn(&UserListEntry, &mut App)>;
type BlockHandler = Rc<dyn Fn(String, &mut App)>;

fn current_user_id(cx: &mut App) -> String {
    AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default()
}

/// list state,
/// the search layer and its hero,
/// pending confirmations and the two result scopes
struct UserRelationshipListState {
    kind: UserListKind,
    owner: String,
    nav_store: Option<Entity<NavigationStore>>,
    query: String,
    search_overlay: bool,
    search_presence: Presence<()>,
    search_motion: Animated<f32>,
    collapsed_field_frame: Option<Bounds<Pixels>>,
    overlay_field_frame: Option<Bounds<Pixels>>,
    pending_friend_removal: Option<UserListEntry>,
    pending_block_user_id: Option<String>,
    full_results: Entity<UserRelationshipResults>,
    search_results: Entity<UserRelationshipResults>,
    _auth_observation: Subscription,
}

impl UserRelationshipListState {
    fn new(kind: UserListKind, cx: &mut Context<Self>) -> Self {
        let weak = cx.weak_entity();
        let on_open_profile: OpenProfileHandler = Rc::new({
            let weak = weak.clone();
            move |author: &AuthorInfo, window: &mut Window, cx: &mut App| {
                if let Some(state) = weak.upgrade() {
                    Self::open_profile(&state, author, window, cx);
                }
            }
        });
        let on_confirm_friend_removal: ConfirmFriendRemovalHandler = Rc::new({
            let weak = weak.clone();
            move |entry: &UserListEntry, cx: &mut App| {
                if let Some(state) = weak.upgrade() {
                    Self::confirm_friend_removal(&state, entry, cx);
                }
            }
        });
        let on_block: BlockHandler = Rc::new(move |user_id: String, cx: &mut App| {
            let _ = weak.update(cx, |state, cx| {
                state.pending_block_user_id = Some(user_id);
                cx.notify();
            });
        });
        let make_results = |scope: &'static str, cx: &mut Context<Self>| {
            let on_open_profile = on_open_profile.clone();
            let on_confirm_friend_removal = on_confirm_friend_removal.clone();
            let on_block = on_block.clone();
            cx.new(|cx| {
                UserRelationshipResults::new(scope, kind, on_open_profile, on_confirm_friend_removal, on_block, cx)
            })
        };
        let full_results = make_results("full", cx);
        let search_results = make_results("search", cx);
        let auth = AuthServicesStore::shared(cx);
        let auth_observation = cx.observe(&auth, |_, _, cx| cx.notify());
        Self {
            kind,
            owner: String::new(),
            nav_store: None,
            query: String::new(),
            search_overlay: false,
            search_presence: Presence::new(None).with_transition(Transition::opacity()),
            search_motion: Animated::new(0.0),
            collapsed_field_frame: None,
            overlay_field_frame: None,
            pending_friend_removal: None,
            pending_block_user_id: None,
            full_results,
            search_results,
            _auth_observation: auth_observation,
        }
    }

    /// syncs props of the current render,
    /// results compare them so a keystroke reaches only the scope whose query changed
    fn sync(
        &mut self,
        kind: UserListKind,
        owner: String,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut Context<Self>,
    ) {
        self.kind = kind;
        self.owner = owner;
        self.nav_store = nav_store.clone();
        let query = self.query.clone();
        let owner = self.owner.clone();
        let full_nav_store = nav_store.clone();
        let full_owner = owner.clone();
        self.full_results.update(cx, |results, cx| results.set_scope(kind, full_owner, "", full_nav_store, cx));
        self.search_results.update(cx, |results, cx| results.set_scope(kind, owner, &query, nav_store, cx));
    }

    fn set_search_overlay(&mut self, open: bool, spec: Option<MotionSpec>, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        self.search_overlay = open;
        self.search_presence.set_in(open.then_some(()), spec, now, cx);
        self.search_motion.set_in(if open { 1.0 } else { 0.0 }, spec, now, cx);
        cx.notify();
    }

    fn open_search(state: &Entity<Self>, cx: &mut App) {
        state.update(cx, |state, cx| state.set_search_overlay(true, Some(UserRelationshipSearchHero::animation()), cx));
    }

    fn close_search(state: &Entity<Self>, window: &mut Window, cx: &mut App) {
        ScreenFocus::release_text_field(window, cx);
        state.update(cx, |state, cx| state.set_search_overlay(false, Some(UserRelationshipSearchHero::animation()), cx));
    }

    fn open_profile(state: &Entity<Self>, author: &AuthorInfo, window: &mut Window, cx: &mut App) {
        if state.read(cx).search_overlay {
            ScreenFocus::release_text_field(window, cx);
            state.update(cx, |state, cx| {
                state.set_search_overlay(false, None, cx);
                state.query.clear();
            });
        }
        let Some(nav_store) = state.read(cx).nav_store.clone() else {
            return;
        };
        let route = RootRoute::UserPage { user_id: author.id.clone(), tag: Some(author.tag.clone()) };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }

    fn confirm_friend_removal(state: &Entity<Self>, entry: &UserListEntry, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.pending_friend_removal = Some(entry.clone());
            cx.notify();
        });
        let cancel_state = state.clone();
        let confirm_state = state.clone();
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
                let (entry, kind, owner) = confirm_state.update(cx, |state, cx| {
                    cx.notify();
                    (state.pending_friend_removal.take(), state.kind, state.owner.clone())
                });
                let Some(entry) = entry else {
                    return;
                };
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.toggle_subscribe_entry(&entry, kind, &owner, cx);
                });
            },
            cx,
        );
    }
}

#[derive(IntoElement)]
pub struct UserRelationshipList {
    user_id: Option<String>,
    kind: UserListKind,
    title: String,
}

impl UserRelationshipList {
    pub fn new(user_id: Option<String>, kind: UserListKind, title: impl Into<String>) -> Self {
        Self { user_id, kind, title: title.into() }
    }

    fn resolved_owner_id(&self, cx: &mut App) -> String {
        if let Some(user_id) = self.user_id.as_ref().filter(|user_id| !user_id.is_empty()) {
            return user_id.clone();
        }
        current_user_id(cx)
    }

    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(
            format!("user-relationship-list-{}-{}-{suffix}", self.kind.raw_value(), self.user_id.as_deref().unwrap_or(""))
                .into(),
        )
    }

    fn collapsed_search(&self, state: &Entity<UserRelationshipListState>) -> Div {
        let activate_state = state.clone();
        let measured = state.clone();
        div()
            .relative()
            .w_full()
            .child(
                SearchFieldUi::new(self.element_id("search-collapsed"), Binding::constant(String::new()))
                    .on_activate(move |_, cx| UserRelationshipListState::open_search(&activate_state, cx)),
            )
            .child(measure(self.element_id("search-collapsed-frame"), move |bounds, _, cx| {
                measured.update(cx, |state, _| state.collapsed_field_frame = Some(bounds));
            }))
    }

    fn search_overlay(
        &self,
        state: &Entity<UserRelationshipListState>,
        owner: &str,
        theme: &AppTheme,
        progress: f32,
        cx: &App,
    ) -> Div {
        let list_state = state.read(cx);
        let query_binding = Binding::entity(
            state.clone(),
            |state: &UserRelationshipListState| state.query.clone(),
            |state, value: String, cx| {
                state.query = UserRelationshipSearchContract::limited_input(&value);
                cx.notify();
            },
        );
        let search_kind = self.kind;
        let search_owner = owner.to_owned();
        let cancel_state = state.clone();
        let field = SearchFieldUi::new(self.element_id("search-inline"), query_binding)
            .placeholder(SharedString::from(tr("search_placeholder")))
            .on_change(move |value, _, cx| {
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.apply_list_search(search_kind, &search_owner, value, cx);
                });
            })
            .on_cancel(move |window, cx| UserRelationshipListState::close_search(&cancel_state, window, cx))
            .uses_inline_text_field(true)
            .auto_focus(true);

        let mut hero = div().relative().w_full();
        if let (Some(collapsed), Some(slot)) = (list_state.collapsed_field_frame, list_state.overlay_field_frame)
            && (1.0 - progress).abs() > f32::EPSILON
        {
            let remaining = 1.0 - progress;
            let collapsed_width = f32::from(collapsed.size.width);
            let slot_width = f32::from(slot.size.width);
            hero = hero
                .left(px((f32::from(collapsed.origin.x) - f32::from(slot.origin.x)) * remaining))
                .top(px((f32::from(collapsed.origin.y) - f32::from(slot.origin.y)) * remaining))
                .w(px(slot_width + (collapsed_width - slot_width) * remaining));
        }

        let measured = state.clone();
        let close_state = state.clone();
        vstack(0.0)
            .size_full()
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(12.0))
                    .pt(px(10.0))
                    .pb(px(10.0))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .child(hero.child(field))
                            .child(measure(self.element_id("search-inline-frame"), move |bounds, _, cx| {
                                measured.update(cx, |state, cx| {
                                    state.overlay_field_frame = Some(bounds);
                                    cx.notify();
                                });
                            })),
                    )
                    .child(
                        LiquidGlassButton::new(self.element_id("search-close"), LiquidGlassButtonShape::Circle)
                            .size(BUTTON_SIDE)
                            .accessibility_label(Some(tr("cancel")))
                            .on_press(move |window, cx| UserRelationshipListState::close_search(&close_state, window, cx))
                            .children(IconUi::sized("xmark", px(15.0), theme.text.secondary, false)),
                    ),
            )
            .child(
                div().flex_1().min_h_0().w_full().child(UserRelationshipResultsHost {
                    results: list_state.search_results.clone(),
                }),
            )
    }
}

impl RenderOnce for UserRelationshipList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let owner = self.resolved_owner_id(cx);
        let nav_store = env::<NavigationStoreKey>(cx);
        let kind = self.kind;
        let state = window.use_keyed_state(self.element_id("state"), cx, move |_, cx: &mut Context<UserRelationshipListState>| {
            UserRelationshipListState::new(kind, cx)
        });
        let sync_owner = owner.clone();
        state.update(cx, |state, cx| state.sync(kind, sync_owner, nav_store, cx));

        let now = RiseClock::monotonic();
        let (search_overlay, progress, settled_closed, full_results) = {
            let list = state.read(cx);
            (
                list.search_overlay,
                list.search_motion.frame_value(now, window),
                !list.search_overlay && !list.search_motion.is_running(now) && !list.query.is_empty(),
                list.full_results.clone(),
            )
        };
        if settled_closed {
            state.update(cx, |state, _| state.query.clear());
        }

        let below_header = div().w_full().px(px(10.0)).pt(px(8.0)).pb(px(10.0)).child(if search_overlay {
            div().w_full().h(px(SEARCH_FIELD_HEIGHT))
        } else {
            self.collapsed_search(&state)
        });
        let veil_rule = match kind {
            UserListKind::Friends => Some(RisePrivacyRule::Friend),
            UserListKind::Subscriptions => Some(RisePrivacyRule::Subs),
            UserListKind::Subscribers => None,
        }
        .filter(|_| owner == current_user_id(cx));
        let results = UserRelationshipResultsHost { results: full_results }.into_any_element();
        let results = match veil_rule {
            Some(rule) => RiseRTCStreamerMode::veil(self.element_id("streamer-veil"), rule, results, cx),
            None => results,
        };
        let shell = ListShellUi::new()
            .title(SharedString::from(self.title.clone()))
            .on_back({
                let nav_store = state.read(cx).nav_store.clone();
                move |_, cx| {
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                }
            })
            .below_header(below_header)
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .content(results);

        let shared_cx: &App = cx;
        let overlay = state.read(shared_cx).search_presence.render_transitioned(now, window, |_| {
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(theme.bg._100)
                .child(self.search_overlay(&state, &owner, &theme, progress, shared_cx))
        });

        let block_target = Binding::entity(
            state.clone(),
            |state: &UserRelationshipListState| state.pending_block_user_id.clone(),
            |state, value, _| state.pending_block_user_id = value,
        );
        let appear_owner = owner;
        let disappear_state = state.clone();
        div()
            .relative()
            .size_full()
            .child(shell)
            .children(overlay)
            .user_block_scope_dialog(self.element_id("block-scope"), block_target)
            .on_appear(self.element_id("lifecycle"), move |_, cx| {
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.ensure_list_loaded(kind, &appear_owner, None, cx);
                });
            })
            .and_on_disappear(move |_, cx| {
                disappear_state.update(cx, |state, cx| {
                    state.set_search_overlay(false, None, cx);
                    state.query.clear();
                });
            })
    }
}

/// reads the environment where the scope is mounted and embeds the view,
/// a cached view recycles its frame so env comes as a prop
#[derive(IntoElement)]
struct UserRelationshipResultsHost {
    results: Entity<UserRelationshipResults>,
}

impl RenderOnce for UserRelationshipResultsHost {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        self.results.update(cx, |results, cx| results.set_top_inset(top_inset, cx));
        AnyView::from(self.results).cached(StyleRefinement::default().size_full())
    }
}

/// one list scope,
/// the full list or one search query. equality skips keystroke updates so the full list
/// doesnt rebuild while typing
struct UserRelationshipResults {
    scope: &'static str,
    kind: UserListKind,
    owner: String,
    search: String,
    top_inset: f32,
    theme_epoch: u64,
    data_epoch: u64,
    snapshot: Option<UserRelationshipResultsSnapshot>,
    decoded: Option<UserRelationshipDecodedList>,
    rows: UserRelationshipRowViews,
    nav_store: Option<Entity<NavigationStore>>,
    on_open_profile: OpenProfileHandler,
    on_confirm_friend_removal: ConfirmFriendRemovalHandler,
    on_block: BlockHandler,
    list: ListUi,
    list_ids: Vec<String>,
    list_scope_changed: bool,
    viewer: Option<String>,
    idle_asks: u8,
    shown_at: Option<(usize, Option<RootRoute>)>,
    held_while_covered: bool,
    _nav_observation: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

/// list read of one data epoch
/// reused by scroll renders so the payload isnt cloned and decoded on each wheel step
#[derive(Clone)]
struct UserRelationshipResultsSnapshot {
    epoch: u64,
    entries: Rc<Vec<UserListEntry>>,
    has_more: bool,
    relative_id: Option<String>,
    status: SaiWsSliceStatus,
    cache_key: SharedString,
    no_data_text: String,
}

/// decoded list page of one cache revision,
/// so a page arrival doesnt decode every loaded entry again and again
struct UserRelationshipDecodedList {
    cache_key: String,
    revision: i64,
    account: String,
    entries: Rc<Vec<UserListEntry>>,
    has_more: bool,
    relative_id: Option<String>,
}

/// row views by entry id, they outlive a scroll out of view so a returning row keeps data and height
type UserRelationshipRowViews = Rc<RefCell<HashMap<String, Entity<UserRelationshipRowView>>>>;

impl UserRelationshipResults {
    fn new(
        scope: &'static str,
        kind: UserListKind,
        on_open_profile: OpenProfileHandler,
        on_confirm_friend_removal: ConfirmFriendRemovalHandler,
        on_block: BlockHandler,
        cx: &mut Context<Self>,
    ) -> Self {
        let service = UserServicesStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            scope,
            kind,
            owner: String::new(),
            search: String::new(),
            top_inset: 0.0,
            theme_epoch: 0,
            data_epoch: 0,
            snapshot: None,
            decoded: None,
            rows: Rc::new(RefCell::new(HashMap::new())),
            nav_store: None,
            on_open_profile,
            on_confirm_friend_removal,
            on_block,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            list_scope_changed: false,
            viewer: None,
            idle_asks: 0,
            shown_at: None,
            held_while_covered: false,
            _nav_observation: None,
            _subscriptions: vec![
                cx.observe(&service, |results, _, cx| results.data_changed(cx)),
                cx.observe(&cache_revisions, |results, _, cx| results.data_changed(cx)),
                cx.observe(&pending_requests, |results, _, cx| results.data_changed(cx)),
                cx.observe(&auth, |results, _, cx| results.data_changed(cx)),
                cx.observe(&theme_store, |results, _, cx| {
                    results.theme_epoch = results.theme_epoch.wrapping_add(1);
                    results.data_changed(cx);
                }),
            ],
        }
    }

    fn data_changed(&mut self, cx: &mut Context<Self>) {
        self.data_epoch = self.data_epoch.wrapping_add(1);
        if self.is_covered(cx) {
            self.held_while_covered = true;
            return;
        }
        cx.notify();
    }

    /// true while the nav stack holds a page pushed over the one this list sits on
    fn is_covered(&self, cx: &App) -> bool {
        let (Some(nav_store), Some((depth, route))) = (self.nav_store.as_ref(), self.shown_at.as_ref()) else {
            return false;
        };
        let path = &nav_store.read(cx).path;
        path.len() > *depth && (*depth == 0 || path.get(*depth - 1) == route.as_ref())
    }

    fn note_shown_at(&mut self, cx: &App) {
        let Some(nav_store) = self.nav_store.as_ref() else {
            return;
        };
        let path = &nav_store.read(cx).path;
        let depth = path.len();
        if self.shown_at.as_ref().is_none_or(|(shown, _)| depth <= *shown) {
            self.shown_at = Some((depth, path.last().cloned()));
        }
    }

    fn navigation_changed(&mut self, cx: &mut Context<Self>) {
        if self.held_while_covered && !self.is_covered(cx) {
            self.held_while_covered = false;
            cx.notify();
        }
    }

    fn set_scope(
        &mut self,
        kind: UserListKind,
        owner: String,
        search: &str,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut Context<Self>,
    ) {
        if self.nav_store != nav_store {
            self._nav_observation =
                nav_store.as_ref().map(|nav_store| cx.observe(nav_store, |results, _, cx| results.navigation_changed(cx)));
            self.nav_store = nav_store;
            self.shown_at = None;
            self.data_epoch = self.data_epoch.wrapping_add(1);
        }
        if self.kind == kind && self.owner == owner && self.search == search {
            return;
        }
        self.kind = kind;
        self.owner = owner;
        self.search = search.to_owned();
        self.rows.borrow_mut().clear();
        self.decoded = None;
        self.list_scope_changed = true;
        self.idle_asks = 0;
        self.data_changed(cx);
    }

    fn keep_list_asked(&mut self, status: SaiWsSliceStatus, cx: &mut Context<Self>) {
        const MAX_IDLE_ASKS: u8 = 3;
        let account = current_user_id(cx);
        if account.is_empty() || self.owner.is_empty() {
            return;
        }
        let viewer_changed = self.viewer.as_ref().is_some_and(|viewer| *viewer != account);
        self.viewer = Some(account);
        let kind = self.kind;
        let owner = self.owner.clone();
        if viewer_changed {
            tracing::info!(target: "riseonly::X2062", owner = %owner, "viewer changed, list read again");
            self.idle_asks = 0;
            cx.defer(move |cx| {
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_list(kind, &owner, None, cx));
            });
            return;
        }
        match status {
            SaiWsSliceStatus::Fulfilled => self.idle_asks = 0,
            SaiWsSliceStatus::Idle if self.idle_asks < MAX_IDLE_ASKS => {
                self.idle_asks += 1;
                tracing::info!(target: "riseonly::X2062", owner = %owner, asks = self.idle_asks, "idle list asked again");
                cx.defer(move |cx| {
                    UserInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.ensure_list_loaded(kind, &owner, None, cx));
                });
            }
            _ => {}
        }
    }

    fn set_top_inset(&mut self, top_inset: f32, cx: &mut Context<Self>) {
        if (self.top_inset - top_inset).abs() > f32::EPSILON {
            self.top_inset = top_inset;
            self.data_changed(cx);
        }
    }

    fn sync_list(&mut self, ids: Vec<String>) {
        let scope_changed = std::mem::take(&mut self.list_scope_changed);
        if ids == self.list_ids && !scope_changed {
            return;
        }
        let same_list = !scope_changed && !self.list_ids.is_empty() && !ids.is_empty();
        if same_list && ids.len() > self.list_ids.len() && ids.starts_with(&self.list_ids) {
            self.list.appended(ids.len() - self.list_ids.len());
        } else if let Some(removed) = same_list.then(|| removed_indices(&self.list_ids, &ids)).flatten() {
            for index in removed.into_iter().rev() {
                self.list.removed(index..index + 1);
            }
            self.rows.borrow_mut().retain(|id, _| ids.contains(id));
        } else if same_list {
            let new_index: HashMap<&str, usize> =
                ids.iter().enumerate().map(|(index, id)| (id.as_str(), index)).collect();
            let old_ids = &self.list_ids;
            self.list.rebuilt(ids.len(), |old| old_ids.get(old).and_then(|id| new_index.get(id.as_str()).copied()));
            self.rows.borrow_mut().retain(|id, _| ids.contains(id));
        } else {
            self.list.replaced(ids.len());
            self.rows.borrow_mut().retain(|id, _| ids.contains(id));
        }
        self.list_ids = ids;
    }

    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("user-relationship-results-{}-{suffix}", self.scope).into())
    }

    fn parts(&self) -> UserRelationshipResultParts {
        UserRelationshipResultParts {
            scope: self.scope,
            kind: self.kind,
            owner: self.owner.clone(),
            search: self.search.clone(),
            nav_store: self.nav_store.clone(),
            on_open_profile: self.on_open_profile.clone(),
            on_confirm_friend_removal: self.on_confirm_friend_removal.clone(),
            on_block: self.on_block.clone(),
        }
    }

    /// list read of the current data epoch, reread only after a bump
    fn snapshot(&mut self, parts: &UserRelationshipResultParts, cx: &mut Context<Self>) -> UserRelationshipResultsSnapshot {
        let epoch = self.data_epoch;
        if let Some(snapshot) = self.snapshot.as_ref().filter(|snapshot| snapshot.epoch == epoch) {
            return snapshot.clone();
        }
        let kind = self.kind;
        let search = Some(parts.search.as_str());
        let is_short_query = UserRelationshipSearchContract::is_short_nonempty_query(search);
        let service = UserServicesStore::shared(cx);
        let cache_key = UserServicesStore::list_cache_key(kind, &parts.owner, search);
        let (entries, has_more, relative_id, status) = if is_short_query {
            (Rc::new(Vec::new()), false, None, SaiWsSliceStatus::Fulfilled)
        } else {
            let revision = SaiCacheRevisionTracker::shared(cx).read(cx).revision(&cache_key);
            let account = current_user_id(cx);
            let reused = self
                .decoded
                .as_ref()
                .filter(|decoded| {
                    revision != 0
                        && decoded.revision == revision
                        && decoded.cache_key == cache_key
                        && decoded.account == account
                })
                .map(|decoded| (decoded.entries.clone(), decoded.has_more, decoded.relative_id.clone()));
            let status = service.update(cx, |service, cx| service.list_render_status(kind, &parts.owner, search, cx));
            match reused {
                Some((entries, has_more, relative_id)) => (entries, has_more, relative_id, status),
                None => {
                    let (entries, has_more, continuation) = service.update(cx, |service, cx| {
                        (
                            service.list_entries(kind, &parts.owner, search, cx),
                            service.list_has_more(kind, &parts.owner, search, cx),
                            service.list_continuation(kind, &parts.owner, search, cx),
                        )
                    });
                    let entries = Rc::new(entries);
                    let relative_id = continuation.map(|continuation| continuation.pagination_token());
                    self.decoded = (!entries.is_empty()).then(|| UserRelationshipDecodedList {
                        cache_key: cache_key.clone(),
                        revision,
                        account,
                        entries: entries.clone(),
                        has_more,
                        relative_id: relative_id.clone(),
                    });
                    (entries, has_more, relative_id, status)
                }
            }
        };
        if self.scope == "full" {
            self.keep_list_asked(status, cx);
        }
        let no_data_text = if is_short_query {
            tr("relationship_search_minimum_hint")
        } else {
            tr(if entries.is_empty() && parts.list_hidden_by_owner(cx) { "relationship_list_hidden" } else { "no_users" })
        };
        self.sync_list(entries.iter().map(UserListEntry::id).collect());
        let snapshot = UserRelationshipResultsSnapshot {
            epoch,
            entries,
            has_more,
            relative_id,
            status,
            cache_key: SharedString::from(cache_key),
            no_data_text,
        };
        self.snapshot = Some(snapshot.clone());
        snapshot
    }
}

impl Render for UserRelationshipResults {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.note_shown_at(cx);
        self.held_while_covered = false;
        let theme = rise_ui::theme(cx).clone();
        let parts = self.parts();
        let top_inset = self.top_inset;
        let theme_epoch = self.theme_epoch;
        let data_epoch = self.data_epoch;
        let host = cx.weak_entity();
        let rows = self.rows.clone();
        let UserRelationshipResultsSnapshot { entries, has_more, relative_id, status, cache_key, no_data_text, .. } =
            self.snapshot(&parts, cx);
        let is_empty = entries.is_empty();

        let item_count = entries.len();
        let refresh_parts = parts.clone();
        let row_parts = parts.clone();
        let row_theme = theme.clone();
        let list = self.list.refreshable(
            self.element_id("list"),
            move |_, cx| {
                refresh_parts.refresh(cx);
                Task::ready(())
            },
            move |index, _window, cx| {
                let Some(entry) = entries.get(index) else {
                    return Empty.into_any_element();
                };
                let load_parts = row_parts.clone();
                let row = UserRelationshipRowView::element(&rows, entry.id(), data_epoch, host.clone(), cx, |cx| {
                    (
                        row_parts.row_data(entry, (index == 0).then_some(top_inset), theme_epoch, cx),
                        row_parts.clone(),
                        row_theme.clone(),
                    )
                });
                vstack(0.0)
                    .w_full()
                    .child(row)
                    .sai_pagination_prefetch(
                        row_parts.element_id(&format!("row-{}", entry.id())),
                        SaiPaginationPrefetchOptions::new(
                            index,
                            item_count,
                            cache_key.clone(),
                            relative_id.clone(),
                            has_more,
                        )
                        .threshold(8),
                        move |_, cx| load_parts.load_more(cx),
                    )
                    .into_any_element()
            },
        );

        let background = theme.bg._100;
        let skeleton_id = self.element_id("skeleton");
        let error_parts = parts.clone();
        let error_theme = theme.clone();
        SaiDataRender::new(status, move |_, _| div().size_full().bg(background).child(list))
            .id(self.element_id("data-render"))
            .is_empty(is_empty)
            .no_data_text(no_data_text)
            .error_text(tr("error_fetching_data"))
            .sai_data_render_loading(move |_, _| {
                scroll_view(skeleton_id.clone())
                    .scroll_disabled(true)
                    .shows_indicators(false)
                    .bg(background)
                    .child(div().w_full().pt(px(top_inset)).child(UserListSkeleton::new()))
            })
            .sai_data_render_error(move |_, _| error_parts.load_error(&error_theme, top_inset))
    }
}

fn removed_indices(old: &[String], new: &[String]) -> Option<Vec<usize>> {
    if new.len() >= old.len() {
        return None;
    }
    let mut removed = Vec::with_capacity(old.len() - new.len());
    let mut next = 0;
    for (index, id) in old.iter().enumerate() {
        if new.get(next) == Some(id) {
            next += 1;
        } else {
            removed.push(index);
        }
    }
    (next == new.len()).then_some(removed)
}

/// what one row shows, equal data means the cached row replays its last frame
#[derive(Clone, PartialEq)]
struct UserRelationshipRowData {
    entry: UserListEntry,
    relationship: UserListRelationship,
    viewer_blocked: bool,
    is_pending: bool,
    current_user_id: String,
    top_inset: Option<f32>,
    theme_epoch: u64,
    kind: UserListKind,
    owner: String,
    search: String,
    nav_store: Option<Entity<NavigationStore>>,
}

/// one row as a cached view,
/// wheel steps used to rebuild every visible row so an unchanged row replays its frame
struct UserRelationshipRowView {
    row: Option<(UserRelationshipRowData, UserRelationshipResultParts, AppTheme)>,
    epoch: Option<u64>,
    height: Rc<Cell<Option<Pixels>>>,
    host: WeakEntity<UserRelationshipResults>,
}

impl UserRelationshipRowView {
    fn element(
        rows: &UserRelationshipRowViews,
        entry_id: String,
        epoch: u64,
        host: WeakEntity<UserRelationshipResults>,
        cx: &mut App,
        read_row: impl FnOnce(&mut App) -> (UserRelationshipRowData, UserRelationshipResultParts, AppTheme),
    ) -> AnyElement {
        let existing = rows.borrow().get(&entry_id).cloned();
        let view = match existing {
            Some(view) => view,
            None => {
                let view = cx.new(|_| Self { row: None, epoch: None, height: Rc::new(Cell::new(None)), host });
                rows.borrow_mut().insert(entry_id, view.clone());
                view
            }
        };
        let stale = if view.read(cx).epoch == Some(epoch) {
            false
        } else {
            let (data, parts, theme) = read_row(cx);
            view.update(cx, |row, _| row.set_row(data, parts, theme, epoch))
        };
        let height = view.read(cx).height.get();
        let any_view = AnyView::from(view);
        match height {
            Some(height) if !stale => div()
                .w_full()
                .child(any_view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved())
                .into_any_element(),
            _ => div().w_full().child(any_view).into_any_element(),
        }
    }

    fn set_row(
        &mut self,
        data: UserRelationshipRowData,
        parts: UserRelationshipResultParts,
        theme: AppTheme,
        epoch: u64,
    ) -> bool {
        self.epoch = Some(epoch);
        if self.row.as_ref().is_some_and(|(current, _, _)| *current == data) {
            return false;
        }
        self.row = Some((data, parts, theme));
        true
    }
}

impl Render for UserRelationshipRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.height.clone();
        let host = self.host.clone();
        let row = self.row.as_ref().map(|(data, parts, theme)| {
            vstack(0.0)
                .w_full()
                .when_some(data.top_inset, |row, top_inset| row.pt(px(top_inset)))
                .child(parts.relationship_row(data, theme))
                .child(div().w_full().h(px(0.5)).bg(theme.border._100))
        });
        div().relative().w_full().children(row).child(
            canvas(
                move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
                    let measured = bounds.size.height;
                    if measured <= px(0.0) || height.get() == Some(measured) {
                        return;
                    }
                    if height.replace(Some(measured)).is_none() {
                        return;
                    }
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

/// what every row and state view of one scope reads
#[derive(Clone)]
struct UserRelationshipResultParts {
    scope: &'static str,
    kind: UserListKind,
    owner: String,
    search: String,
    nav_store: Option<Entity<NavigationStore>>,
    on_open_profile: OpenProfileHandler,
    on_confirm_friend_removal: ConfirmFriendRemovalHandler,
    on_block: BlockHandler,
}

impl UserRelationshipResultParts {
    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("user-relationship-results-{}-{suffix}", self.scope).into())
    }

    fn refresh(&self, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.refresh_list(self.kind, &self.owner, Some(self.search.as_str()), cx);
        });
    }

    fn load_more(&self, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_more_list(self.kind, &self.owner, Some(self.search.as_str()), cx);
        });
    }

    fn load_error(&self, theme: &AppTheme, top_inset: f32) -> Div {
        let retry_parts = self.clone();
        vstack(14.0)
            .size_full()
            .justify_center()
            .pt(px(top_inset))
            .px(px(24.0))
            .bg(theme.bg._100)
            .child(
                MainText::new(tr("error_fetching_data")).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary),
            )
            .child(
                LiquidGlassButton::new(self.element_id("retry"), LiquidGlassButtonShape::Capsule)
                    .accessibility_label(Some(tr("common_retry")))
                    .on_press(move |_, cx| retry_parts.refresh(cx))
                    .child(MainText::new(tr("common_retry")).px(14.0).color(theme.text.primary)),
            )
    }

    fn list_hidden_by_owner(&self, cx: &mut App) -> bool {
        if !trimming_whitespaces_and_newlines(&self.search).is_empty() {
            return false;
        }
        let viewer_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        if viewer_id.as_deref() == Some(self.owner.as_str()) {
            return false;
        }
        let payload = UserServicesStore::shared(cx).update(cx, |service, cx| service.foreign_profile_payload(&self.owner, cx));
        let Some(profile) = payload.and_then(|payload| payload.as_profile_model()) else {
            return false;
        };
        match self.kind {
            UserListKind::Friends => profile.friends_count > 0,
            UserListKind::Subscribers => profile.subscribers_count > 0,
            UserListKind::Subscriptions => profile.subs_count > 0 || profile.subs_count_hidden,
        }
    }

    fn row_data(
        &self,
        entry: &UserListEntry,
        top_inset: Option<f32>,
        theme_epoch: u64,
        cx: &mut App,
    ) -> UserRelationshipRowData {
        let (relationship, viewer_blocked, is_pending) = UserServicesStore::shared(cx).update(cx, |service, cx| {
            let author_id = entry.author.id.as_deref();
            (
                service.resolved_list_relationship(entry, self.kind, &self.owner, cx),
                author_id.is_some_and(|id| service.viewer_block_scope(id, cx).is_some()),
                author_id.is_some_and(|id| service.is_follow_interaction_pending(id)),
            )
        });
        UserRelationshipRowData {
            entry: entry.clone(),
            relationship,
            viewer_blocked,
            is_pending,
            current_user_id: current_user_id(cx),
            top_inset,
            theme_epoch,
            kind: self.kind,
            owner: self.owner.clone(),
            search: self.search.clone(),
            nav_store: self.nav_store.clone(),
        }
    }

    fn relationship_row(&self, data: &UserRelationshipRowData, theme: &AppTheme) -> AnyElement {
        let entry = &data.entry;
        let author = &entry.author;
        let entry_id = entry.id();
        let relationship = data.relationship;
        let viewer_blocked = data.viewer_blocked;
        let current_user_id = data.current_user_id.as_str();

        let actions = HoldContextSocialMenu::user_list_actions_viewer_blocked(viewer_blocked);
        let action_parts = self.clone();
        let action_author = author.clone();
        let tap_handler = self.on_open_profile.clone();
        let tap_author = author.clone();
        let content_parts = self.clone();
        let content_author = author.clone();
        let content_theme = theme.clone();
        let hold = HoldContextMenuContainer::new(
            self.element_id(&format!("hold-{entry_id}")),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                content_parts.relationship_identity(&content_author, &content_theme).into_any_element()
            }),
        )
        .show_emoji_list(false)
        .lifted_background(Some(theme.bg._100))
        .on_action(Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
            action_parts.social_user_action(&action_author, &action, window, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
            tap_handler(&tap_author, window, cx);
        }));

        let button = author
            .id
            .as_deref()
            .filter(|author_id| !author_id.is_empty() && *author_id != current_user_id)
            .map(|_| self.relationship_button(entry, relationship, data.is_pending, &entry_id));

        hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
            .py(px(UserRelationshipRowMetrics::VERTICAL_PADDING))
            .bg(theme.bg._100)
            .child(div().flex_1().min_w_0().child(hold))
            .children(button)
            .into_any_element()
    }

    fn relationship_identity(&self, author: &AuthorInfo, theme: &AppTheme) -> Div {
        let press_handler = self.on_open_profile.clone();
        let press_author = author.clone();
        let logo_key = author.id.clone().unwrap_or_else(|| author.tag.clone());

        let name_row = UserNameWithBadges::new(author.id.clone().map(SharedString::from), author.name.clone())
            .official(author.more.is_official)
            .premium(author.more.is_premium, author.more.premium_badge_emoji.clone(), author.more.name_color.clone());

        let mut names = vstack(4.0).align(HorizontalAlignment::Leading).flex_1().min_w_0().child(name_row);
        if !author.tag.is_empty() {
            names = names.child(
                MainText::new(format!("@{}", author.tag))
                    .px(13.0)
                    .color(theme.text.secondary)
                    .number_of_lines(1)
                    .w_full(),
            );
        }

        hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .min_h(px(UserRelationshipRowMetrics::AVATAR_SIZE))
            .child(
                UserLogoUi::new(SharedString::from(format!("user-relationship-logo-{}-{logo_key}", self.scope)))
                    .source(SharedString::from(author.more.logo.clone()))
                    .name(SharedString::from(author.name.clone()))
                    .user_id(author.id.clone().map(SharedString::from))
                    .size(UserRelationshipRowMetrics::AVATAR_SIZE)
                    .is_button(true)
                    .on_press(move |window, cx| press_handler(&press_author, window, cx))
                    .gift_worn_frame(
                        SharedString::from(format!("user-relationship-frame-{}-{logo_key}", self.scope)),
                        author.id.clone(),
                        UserRelationshipRowMetrics::AVATAR_SIZE,
                    ),
            )
            .child(names)
    }

    fn relationship_button(
        &self,
        entry: &UserListEntry,
        relationship: UserListRelationship,
        is_pending: bool,
        entry_id: &str,
    ) -> UserRelationshipActionButton {
        let is_friends = relationship.follow_button_state() == FollowButtonState::Friends;
        let parts = self.clone();
        let entry = entry.clone();
        UserRelationshipActionButton::new(
            self.element_id(&format!("relationship-{entry_id}")),
            relationship,
            is_pending,
            move |_: &mut Window, cx: &mut App| {
                if is_friends {
                    (parts.on_confirm_friend_removal)(&entry, cx);
                } else {
                    UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.toggle_subscribe_entry(&entry, parts.kind, &parts.owner, cx);
                    });
                }
            },
        )
    }

    fn social_user_action(&self, author: &AuthorInfo, id: &str, window: &mut Window, cx: &mut App) {
        let author_id = author.id.as_deref().filter(|author_id| !author_id.is_empty());
        match id {
            "profile" => (self.on_open_profile)(author, window, cx),
            "message" => {
                let Some(user_id) = author_id else {
                    return;
                };
                let target = User {
                    id: user_id.to_owned(),
                    name: author.name.clone(),
                    tag: author.tag.clone(),
                    more: author.more.clone(),
                    ..User::default()
                };
                let chat_id = ChatInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.direct_conversation_id(&target, cx));
                if chat_id.is_empty() {
                    return;
                }
                if let Some(nav_store) = &self.nav_store {
                    nav_store.update(cx, |nav_store, cx| {
                        nav_store.push(RootRoute::Chat { chat_id: Some(chat_id) }, NavigationStoreSettlingPush::Drop, cx);
                    });
                }
            }
            "copy" => {
                let value = if author.tag.is_empty() { author.name.clone() } else { format!("@{}", author.tag) };
                UIPasteboard::general().set_string(&value, cx);
            }
            "mention" => {
                if !author.tag.is_empty() {
                    UIPasteboard::general().set_string(&format!("@{}", author.tag), cx);
                }
            }
            "block" => {
                if let Some(user_id) = author_id
                    && user_id != current_user_id(cx)
                {
                    (self.on_block)(user_id.to_owned(), cx);
                }
            }
            "unblock" => {
                if let Some(user_id) = author_id {
                    UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.set_user_block_scope(user_id, None, |_, _| {}, cx);
                    });
                }
            }
            "report" => {
                if let Some(user_id) = author_id
                    && user_id != current_user_id(cx)
                {
                    let subject = ReportSubject {
                        name: author.name.clone(),
                        tag: author.tag.clone(),
                        avatar_url: author.more.logo.clone(),
                        user_id: Some(user_id.to_owned()),
                    };
                    ReportSheetStore::shared(cx).update(cx, |store, cx| {
                        store.present(ReportTarget::User(user_id.to_owned()), Some(subject), None, cx);
                    });
                }
            }
            _ => {}
        }
    }
}

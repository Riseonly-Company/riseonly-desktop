use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AsyncApp, BoxShadow, Context, ElementId, Entity, FontWeight, Hsla, Pixels, Point, RenderImage, SharedString,
    Subscription, Task, TextRun, WeakEntity, Window, black, point, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::emoji_raster::{TextWeight, render_text};
use rise_platform::haptics::selection_changed;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::IconUi;
use rise_ui::bg_wrapper_ui::BgWrapperUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAction, HoldContextMenuAlignment, HoldContextMenuLocksScrollKey,
};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderMetrics, PageHeaderToolbarSlot};
use rise_ui::page_header_ui::{PageHeaderUi, PanelToggleHeaderReserveKey};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_fullscreen_pop::NavigationInteractivePopGuard;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{CreatePostIntent, MainTab, RootRoute};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapLoader, CleverImageDiskCache, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeFloatingAction;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::core::widgets::wrappers::list_shell_ui::ListShellUi;
use crate::modules::auth::stores::auth_actions::auth_types::AuthAccountSummary;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileEmptyState, ChatProfileTabErrorState,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::guides::engine::guide_runner::GuideRunner;
use crate::modules::moderation::components::admin_ban_entry::{AdminBanEntry, AdminBanTarget, admin_ban_sheet};
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::notify::components::notifications_hub_sheet_store::NotificationsHubSheetStore;
use crate::modules::post::engine::rise_post_engine_models::RisePostProfileKind;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_refresh_control::PostListRefreshControl;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::story::components::stories_grid::stories_grid_action_bar::{
    StoriesGridActionBar, StoriesGridDoneButton,
};
use crate::modules::story::components::story_composer_host::StoryComposerHost;
use crate::modules::story::shared::story_grid_selection::StoryGridSelection;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryAuthorDTO;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::user::components::account_limit_sheet::AccountLimitSheet;
use crate::modules::user::components::premium_badge_picker::premium_badge_picker_popover::PremiumBadgePickerPopover;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::{
    ProfileGridMenuAsOwnerKey, ProfileOwnerResolvingKey,
};
use crate::modules::user::components::profile_content::profile_content::{
    ProfileContent, ProfileContentTab, ProfileViewportLayoutPolicy,
};
pub use crate::modules::user::components::profile_content::profile_content::ProfileCreateActionMetrics;
use crate::modules::user::components::profile_content::profile_performance_trace::ProfilePerformanceTrace;
use crate::modules::user::components::profile_content::profile_performance_trace_control::ProfilePerformanceTraceControl;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileHorizontalPagerPolicy, ProfileMasterPinnedModifier, ProfileOuterScrollIntrospector,
    ProfileScrollCoordinator, ProfileScrollMetrics,
};
use crate::modules::user::components::profile_top::profile_top::ProfileTop;
use crate::modules::user::components::user_block_scope_dialog::UserBlockAndReport;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::shared::profile_missing_tag_state::ProfileMissingTagState;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_types::{GoalItem, PlanItem, ProfileModel};
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    ProfileViewerPrivacy, UserGetProfileByTagGatewayDTO, UserGetProfileByTagGatewayDTOExt, UserGoalWireEntryExt,
    UserPlanWireEntryExt,
};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

fn child_id(id: &ElementId, name: &str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(name.to_owned()))
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProfileRouteTarget {
    Own,
    ForeignTag(String),
    ForeignUserId(String),
    Invalid,
}

impl ProfileRouteTarget {
    pub fn resolve(is_user: bool, user_id: Option<&str>, tag: Option<&str>) -> Self {
        if let Some(tag) = Self::normalized(tag) {
            return Self::ForeignTag(tag);
        }
        if let Some(user_id) = Self::normalized(user_id) {
            return Self::ForeignUserId(user_id);
        }
        if is_user { Self::Invalid } else { Self::Own }
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        (!value.is_empty()).then(|| value.to_owned())
    }
}

const PULL_REFRESH_TRANSITION_SCALE: f32 = 0.6;
const PULL_REFRESH_TRANSITION_DURATION: f64 = 0.2;
const PULL_REFRESH_INDICATOR_GAP: f32 = 12.0;
const DONE_BUTTON_TRANSITION_DURATION: f64 = 0.2;
const DONE_BUTTON_TRANSITION_SCALE: f32 = 0.86;
const VIEWPORT_MINIMUM_HEIGHT: f32 = 220.0;
const PAGE_STATE_TOP_PADDING: f32 = 40.0;
const HEADER_CONTROLS_SPACING: f32 = 4.0;
const ACCOUNT_MENU_ROW_FONT_SIZE: f32 = 17.0;
const ACCOUNT_MENU_ROW_TITLE_LIMIT: f32 = 180.0;
const ACCOUNT_MENU_PILL_PADDING: f32 = 12.0;
const ACCOUNT_MENU_PILL_SPACING: f32 = 6.0;
const ACCOUNT_MENU_CHEVRON_SIDE: f32 = 10.0;
const TAG_PILL_FONT_SIZE: f32 = 14.0;
const TAG_PILL_HORIZONTAL_PADDING: f32 = 12.0;
const TAG_PILL_VERTICAL_PADDING: f32 = 6.0;
const FOREIGN_MENU_ICON_SIDE: f32 = 17.0;
const CREATE_ACTION_ICON_SIDE: f32 = 21.0;
const ACCOUNT_MENU_AVATAR_PIXELS: i64 = 112;

/// screen draws its own spinner below the header since the native refresh control sits under the banner
pub struct ProfilePullRefreshState {
    pub is_refreshing: bool,
}

impl ProfilePullRefreshState {
    pub fn new() -> Self {
        Self { is_refreshing: false }
    }
}

impl Default for ProfilePullRefreshState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(IntoElement)]
struct ProfilePullRefreshIndicator {
    id: ElementId,
    state: Entity<ProfilePullRefreshState>,
}

impl RenderOnce for ProfilePullRefreshIndicator {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let shown = self.state.read(cx).is_refreshing.then_some(());
        let now = RiseClock::monotonic();
        let presence = window.use_keyed_state(self.id.clone(), cx, move |_, _| {
            Presence::new(shown)
                .with_transition(Transition::opacity().combined(Transition::scale(PULL_REFRESH_TRANSITION_SCALE)))
        });
        presence.update(cx, |presence, cx| {
            presence.set_in(shown, ease_out(PULL_REFRESH_TRANSITION_DURATION), now, cx);
        });
        let shadow = BoxShadow {
            color: black().opacity(0.35),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(4.0),
            spread_radius: px(0.0),
            inset: false,
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .pt(px(ProfileScrollMetrics::PINNED_TABS_TOP + PULL_REFRESH_INDICATOR_GAP))
            .children(presence.read(cx).render_transitioned(now, window, move |_| {
                div()
                    .rounded_full()
                    .shadow(vec![shadow])
                    .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(white())))
            }))
    }
}

/// picked scope plus the id it was picked for,
/// the dialog outlives the menu so confirming must not hit another profile
#[derive(Clone, PartialEq)]
struct BlockIntent {
    target_user_id: String,
    scope: RiseBlockScope,
}

struct ProfileState {
    show_account_limit_sheet: bool,
    admin_ban_target: Option<AdminBanTarget>,
    show_story_composer: bool,
    is_foreign_menu_presented: bool,
    is_create_menu_presented: bool,
    is_account_menu_presented: bool,
    account_menu_avatars: HashMap<String, CleverImage>,
    account_menu_avatar_task: Option<Task<()>>,
    block_intent: Option<BlockIntent>,
    selected_profile_tab: ProfileContentTab,
    profile_effect_replay: i64,
    publications_use_grid: bool,
    threads_use_grid: bool,
    profile_scroll_coordinator: Entity<ProfileScrollCoordinator>,
    pull_refresh: Entity<ProfilePullRefreshState>,
    configured_profile_scroll_scope: String,
    navigation_pop_paging_owner: Uuid,
    foreign_profile_revalidated: bool,
    foreign_load_stalled_offline: bool,
    viewport_height: f32,
    hold_menu_locked: bool,
    _subscriptions: Vec<Subscription>,
}

impl ProfileState {
    fn new(cx: &mut Context<Self>) -> Self {
        let profile_scroll_coordinator = cx.new(ProfileScrollCoordinator::new);
        let pull_refresh = cx.new(|_| ProfilePullRefreshState::new());

        let auth = AuthServicesStore::shared(cx);
        let user_actions = UserActionsStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        let story_interactions = StoryInteractionsStore::shared(cx);
        let story_selection = StoryGridSelection::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let missing_tags = ProfileMissingTagState::shared(cx);
        let guide_runner = GuideRunner::shared(cx);
        let socket = SaiWs::manager(cx);
        let subscriptions = vec![
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&user_actions, |_, _, cx| cx.notify()),
            cx.observe(&user_services, |_, _, cx| cx.notify()),
            cx.observe(&story_interactions, |_, _, cx| cx.notify()),
            cx.observe(&story_selection, |_, _, cx| cx.notify()),
            cx.observe(&hold_context_menu, |state, menu, cx| {
                let locked = menu.read(cx).active_request().is_some();
                if locked != state.hold_menu_locked {
                    state.hold_menu_locked = locked;
                    cx.notify();
                }
            }),
            cx.observe(&missing_tags, |_, _, cx| cx.notify()),
            cx.observe(&guide_runner, |_, _, cx| cx.notify()),
            cx.observe(&socket, |_, _, cx| cx.notify()),
            cx.observe(&pull_refresh, |_, _, cx| cx.notify()),
        ];

        Self {
            show_account_limit_sheet: false,
            admin_ban_target: None,
            show_story_composer: false,
            is_foreign_menu_presented: false,
            is_create_menu_presented: false,
            is_account_menu_presented: false,
            account_menu_avatars: HashMap::new(),
            account_menu_avatar_task: None,
            block_intent: None,
            selected_profile_tab: ProfileContentTab::Publications,
            profile_effect_replay: 0,
            publications_use_grid: true,
            threads_use_grid: true,
            profile_scroll_coordinator,
            pull_refresh,
            configured_profile_scroll_scope: String::new(),
            navigation_pop_paging_owner: Uuid::new_v4(),
            foreign_profile_revalidated: false,
            foreign_load_stalled_offline: false,
            viewport_height: VIEWPORT_MINIMUM_HEIGHT,
            hold_menu_locked: hold_context_menu.read(cx).active_request().is_some(),
            _subscriptions: subscriptions,
        }
    }
}

#[derive(IntoElement)]
pub struct Profile {
    id: ElementId,
    pub is_user: bool,
    pub user_id: Option<String>,
    pub tag: Option<String>,
    pub shows_back_button: bool,
}

impl Profile {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), is_user: false, user_id: None, tag: None, shows_back_button: true }
    }

    pub fn is_user(mut self, is_user: bool) -> Self {
        self.is_user = is_user;
        self
    }

    pub fn user_id(mut self, user_id: impl Into<Option<String>>) -> Self {
        self.user_id = user_id.into();
        self
    }

    pub fn tag(mut self, tag: impl Into<Option<String>>) -> Self {
        self.tag = tag.into();
        self
    }

    pub fn shows_back_button(mut self, shows_back_button: bool) -> Self {
        self.shows_back_button = shows_back_button;
        self
    }

    /// link to this account opens the own profile from the first frame instead of a foreign skeleton
    fn route_target(&self, cx: &mut App) -> ProfileRouteTarget {
        let target = ProfileRouteTarget::resolve(self.is_user, self.user_id.as_deref(), self.tag.as_deref());
        let Some((my_id, my_tag)) =
            AuthServicesStore::shared(cx).read(cx).profile().map(|me| (me.id.clone(), me.tag.clone()))
        else {
            return target;
        };
        if my_id.is_empty() {
            return target;
        }
        match &target {
            ProfileRouteTarget::ForeignUserId(id) if *id == my_id => ProfileRouteTarget::Own,
            ProfileRouteTarget::ForeignTag(route_tag) if Self::same_tag(route_tag, &my_tag) => ProfileRouteTarget::Own,
            _ => target,
        }
    }

    fn same_tag(lhs: &str, rhs: &str) -> bool {
        fn bare(value: &str) -> String {
            let trimmed = trimming_whitespaces_and_newlines(value);
            trimmed.strip_prefix('@').unwrap_or(trimmed).to_lowercase()
        }
        let left = bare(lhs);
        !left.is_empty() && left == bare(rhs)
    }
}

struct ProfileBody {
    id: ElementId,
    is_user: bool,
    shows_back_button: bool,
    route_target: ProfileRouteTarget,
    state: Entity<ProfileState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl RenderOnce for Profile {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx| ProfileState::new(cx));
        let body = Rc::new(ProfileBody {
            route_target: self.route_target(cx),
            id: self.id,
            is_user: self.is_user,
            shows_back_button: self.shows_back_button,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
        });
        body.body(&theme, window, cx)
    }
}

impl ProfileBody {
    fn body(self: &Rc<Self>, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let foreign_payload = self.matching_foreign_payload(cx);
        let profile = self.resolve_profile(foreign_payload.as_ref(), cx);
        let own = self.viewing_own(&profile, cx);
        let foreign_more = if own { None } else { foreign_payload.as_ref().and_then(|p| p.profile.as_ref()?.more.as_ref()) };
        let foreign_goals: Vec<GoalItem> =
            foreign_more.map(|more| more.goals.iter().map(|goal| goal.as_goal_item()).collect()).unwrap_or_default();
        let foreign_plans: Vec<PlanItem> = foreign_more
            .map(|more| more.plans_struct.iter().map(|plan| plan.as_plan_item()).collect())
            .unwrap_or_default();
        let foreign_closed_tabs =
            if own { HashSet::new() } else { Self::closed_tabs(foreign_payload.as_ref()) };
        let foreign_owner_totally_blocked = !own
            && (profile.is_deleted
                || foreign_payload
                .as_ref()
                .and_then(|payload| payload.resolved_owner_block_scope())
                .is_some_and(|scope| {
                    trimming_whitespaces_and_newlines(scope).to_lowercase() == RiseBlockScope::Total.raw_value()
                }));
        let profile_user_id = if own {
            AuthServicesStore::shared(cx).read(cx).profile().map(|me| me.id.clone()).unwrap_or_default()
        } else {
            foreign_payload.as_ref().and_then(|payload| payload.as_profile_model()).map(|m| m.id).unwrap_or_default()
        };
        let visible_tabs =
            ProfileContentTab::visible(own, !foreign_goals.is_empty(), !foreign_plans.is_empty());
        let scroll_scope = self.profile_scroll_scope(&profile, own);
        let show_loader = self.shows_full_screen_loader(foreign_payload.as_ref(), cx);

        self.install_changes(
            &profile_user_id,
            own,
            &visible_tabs,
            &scroll_scope,
            window,
            cx,
        );

        let page = if self.missing_foreign_tag(foreign_payload.as_ref(), cx).is_some() {
            self.missing_profile_page().into_any_element()
        } else if self.foreign_load_failed(foreign_payload.as_ref(), cx) {
            self.foreign_load_failed_page().into_any_element()
        } else {
            let viewport_height = self.state.read(cx).viewport_height;
            let state = self.state.clone();
            BgWrapperUi::new()
                .extend_content_through_top(true)
                .child(
                    div()
                        .relative()
                        .size_full()
                        .flex()
                        .justify_center()
                        .child(measure(child_id(&self.id, "viewport"), move |bounds, _, cx| {
                            let height = f32::from(bounds.size.height).max(VIEWPORT_MINIMUM_HEIGHT);
                            state.update(cx, |state, cx| {
                                if state.viewport_height != height {
                                    state.viewport_height = height;
                                    cx.notify();
                                }
                            });
                        }))
                        .child(div().relative().h_full().w_full().child(
                            self.profile_scroll_view(
                                profile,
                                profile_user_id.clone(),
                                own,
                                foreign_goals,
                                foreign_plans,
                                foreign_closed_tabs,
                                foreign_owner_totally_blocked,
                                visible_tabs.clone(),
                                scroll_scope.clone(),
                                show_loader,
                                theme,
                                viewport_height,
                                window,
                                cx,
                            ),
                        )),
                )
                .into_any_element()
        };

        let appear = self.clone();
        let appear_scope = scroll_scope.clone();
        let appear_tabs = visible_tabs.clone();
        let appear_user_id = profile_user_id;
        let disappear = self.clone();
        let disappear_scope = scroll_scope;
        let auth = AuthServicesStore::shared(cx);
        let is_premium = auth.read(cx).profile().is_some_and(|me| me.more.is_premium);
        let nav_upgrade = self.clone();
        let limit_state = self.state.clone();
        let ban_state = self.state.clone();

        div()
            .relative()
            .size_full()
            .child(page)
            .child(AccountLimitSheet::new(
                Binding::entity(
                    limit_state,
                    |state: &ProfileState| state.show_account_limit_sheet,
                    |state: &mut ProfileState, value, cx| {
                        state.show_account_limit_sheet = value;
                        cx.notify();
                    },
                ),
                is_premium,
                move |_, cx| nav_upgrade.push(RootRoute::SubscriptionSettings, cx),
            ))
            .child(admin_ban_sheet(
                child_id(&self.id, "admin-ban"),
                Binding::entity(
                    ban_state,
                    |state: &ProfileState| state.admin_ban_target.clone(),
                    |state: &mut ProfileState, value, cx| {
                        state.admin_ban_target = value;
                        cx.notify();
                    },
                ),
            ))
            .on_appear(child_id(&self.id, "appear"), move |_, cx| {
                appear.on_appear(&appear_user_id, &appear_scope, &appear_tabs, cx);
            })
            .and_on_disappear(move |_, cx| disappear.on_disappear(&disappear_scope, cx))
            .into_any_element()
    }

    fn on_appear(&self, profile_user_id: &str, scroll_scope: &str, visible_tabs: &[ProfileContentTab], cx: &mut App) {
        let opening = !self.state.read(cx).foreign_profile_revalidated;
        self.state.update(cx, |state, _| state.foreign_profile_revalidated = true);
        if opening && !matches!(self.route_target, ProfileRouteTarget::Own | ProfileRouteTarget::Invalid) {
            UserServicesStore::shared(cx).update(cx, |service, _| service.forget_list_freshness(profile_user_id));
        }
        let interactions = UserInteractionsStore::shared(cx);
        match &self.route_target {
            ProfileRouteTarget::Own => {
                interactions.update(cx, |store, cx| store.ensure_my_profile_loaded(cx));
            }
            ProfileRouteTarget::ForeignTag(tag) => {
                let cached = self.matching_payload_by_tag(tag, cx).is_some();
                interactions.update(cx, |store, cx| {
                    if opening && cached {
                        store.refresh_foreign_profile_tag(tag.clone(), cx);
                    } else {
                        store.load_foreign_profile_tag(tag.clone(), cx);
                    }
                    store.refresh_block_state(profile_user_id, cx);
                });
            }
            ProfileRouteTarget::ForeignUserId(user_id) => {
                let cached = self.matching_payload_by_id(user_id, cx).is_some();
                interactions.update(cx, |store, cx| {
                    if opening && cached {
                        store.refresh_foreign_profile_user_id(user_id.clone(), cx);
                    } else {
                        store.load_foreign_profile_user_id(user_id.clone(), cx);
                    }
                    store.refresh_block_state(user_id, cx);
                });
            }
            ProfileRouteTarget::Invalid => {}
        }
        self.reset_profile_scroll_if_needed(scroll_scope, visible_tabs, cx);
        let selected = self.state.read(cx).selected_profile_tab;
        self.update_navigation_pop_priority(selected, visible_tabs, cx);
        let locked = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let coordinator = self.state.read(cx).profile_scroll_coordinator.clone();
        coordinator.update(cx, |coordinator, cx| coordinator.set_interaction_locked(locked, cx));
    }

    fn on_disappear(&self, scroll_scope: &str, cx: &mut App) {
        ProfilePerformanceTrace::shared().stop_if_recording(scroll_scope, "profile_disappear", None);
        let (coordinator, owner) = {
            let state = self.state.read(cx);
            (state.profile_scroll_coordinator.clone(), state.navigation_pop_paging_owner)
        };
        coordinator.update(cx, |coordinator, cx| coordinator.pause_interaction(cx));
        PremiumBadgePickerPopover::shared(cx).update(cx, |popover, cx| popover.dismiss(cx));
        NavigationInteractivePopGuard::set_profile_tab_paging_back(false, owner, None, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn install_changes(
        self: &Rc<Self>,
        profile_user_id: &str,
        own: bool,
        visible_tabs: &[ProfileContentTab],
        scroll_scope: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (coordinator, block_intent) = {
            let state = self.state.read(cx);
            (state.profile_scroll_coordinator.clone(), state.block_intent.clone())
        };

        let (current_tab, path_is_empty) = self
            .nav_store
            .as_ref()
            .map(|nav_store| {
                let nav_store = nav_store.read(cx);
                (Some(nav_store.current_tab), nav_store.path.is_empty())
            })
            .unwrap_or((None, false));
        let this = self.clone();
        on_change(child_id(&self.id, "current-tab"), current_tab, false, window, cx, move |_, tab, _, cx| {
            if this.shows_back_button || *tab != Some(MainTab::Profile) || !path_is_empty {
                return;
            }
            this.state.update(cx, |state, cx| {
                state.profile_effect_replay = state.profile_effect_replay.wrapping_add(1);
                cx.notify();
            });
        });

        let socket_dropped = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.is_socket_dropped(cx));
        let this = self.clone();
        on_change(child_id(&self.id, "socket-dropped"), socket_dropped, true, window, cx, move |_, dropped, _, cx| {
            if *dropped && this.is_foreign_route() && this.matching_foreign_payload(cx).is_none() {
                this.state.update(cx, |state, cx| {
                    state.foreign_load_stalled_offline = true;
                    cx.notify();
                });
            }
        });

        let connected = RiseTransportStatus::is_connected(cx);
        let this = self.clone();
        on_change(child_id(&self.id, "connected"), connected, false, window, cx, move |_, connected, _, cx| {
            if !*connected {
                return;
            }
            this.state.update(cx, |state, cx| {
                state.foreign_load_stalled_offline = false;
                cx.notify();
            });
            if this.is_foreign_route() && this.matching_foreign_payload(cx).is_none() {
                this.refresh_foreign_route(cx);
            }
        });

        let this = self.clone();
        let tabs = visible_tabs.to_vec();
        on_change(
            child_id(&self.id, "scroll-scope"),
            scroll_scope.to_owned(),
            false,
            window,
            cx,
            move |previous_scope, scope, _, cx| {
                ProfilePerformanceTrace::shared().stop_if_recording(previous_scope, "profile_scope_changed", None);
                PremiumBadgePickerPopover::shared(cx).update(cx, |popover, cx| popover.dismiss(cx));
                this.reset_profile_scroll_if_needed(scope, &tabs, cx);
            },
        );

        let this = self.clone();
        on_change(child_id(&self.id, "visible-tabs"), visible_tabs.to_vec(), false, window, cx, move |_, tabs, _, cx| {
            let selected = this.state.read(cx).selected_profile_tab;
            match tabs.first() {
                Some(first) if !tabs.contains(&selected) => this.select_profile_tab(*first, tabs, false, cx),
                _ => this.update_navigation_pop_priority(selected, tabs, cx),
            }
        });

        on_change(
            child_id(&self.id, "profile-user-id"),
            profile_user_id.to_owned(),
            false,
            window,
            cx,
            move |_, id, _, cx| {
                if own {
                    return;
                }
                UserInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_block_state(id, cx));
            },
        );

        let locked = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        on_change(child_id(&self.id, "hold-locked"), locked, false, window, cx, move |_, locked, _, cx| {
            let locked = *locked;
            coordinator.update(cx, |coordinator, cx| coordinator.set_interaction_locked(locked, cx));
        });

        let this = self.clone();
        on_change(child_id(&self.id, "block-intent"), block_intent, false, window, cx, move |_, intent, _, cx| {
            let Some(intent) = intent.clone() else {
                return;
            };
            this.state.update(cx, |state, cx| {
                state.block_intent = None;
                cx.notify();
            });
            let scope = intent.scope;
            let target_user_id = intent.target_user_id;
            let report_target = target_user_id.clone();
            NativeConfirmAlert::present_delete(
                &tr("contact_block_confirmation_title"),
                &tr(Self::block_scope_description_key(scope)),
                Some(tr(Self::block_scope_title_key(scope))),
                Some(tr("user_block_and_report")),
                true,
                Some(Box::new(move |cx: &mut App| {
                    UserBlockAndReport::run(&report_target, Some(scope), None, cx);
                })),
                None,
                move |cx| {
                    UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.set_user_block_scope(&target_user_id, Some(scope), |_, _| {}, cx);
                    });
                },
                cx,
            );
        });

        let switcher_error = if own {
            AuthServicesStore::shared(cx).read(cx).account_switcher_error().map(str::to_owned)
        } else {
            None
        };
        on_change(child_id(&self.id, "switcher-error"), switcher_error, true, window, cx, move |_, error, _, cx| {
            let Some(error) = error.clone() else {
                return;
            };
            NativeConfirmAlert::present_notice(
                &tr("account_switcher_error"),
                &error,
                None,
                None,
                |cx| {
                    AuthServicesStore::shared(cx).update(cx, |auth, cx| auth.set_account_switcher_error(None, cx));
                },
                cx,
            );
        });
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.pop(cx));
        }
    }

    /// starts a pull refresh, the action returns at once and the screens own spinner covers the wait
    fn start_pull_refresh(&self, profile_user_id: String, profile_tag: String, cx: &mut App) {
        let pull_refresh = self.state.read(cx).pull_refresh.clone();
        if pull_refresh.read(cx).is_refreshing {
            return;
        }
        pull_refresh.update(cx, |state, cx| {
            state.is_refreshing = true;
            cx.notify();
        });
        let started_at = RiseClock::monotonic();
        let refresh = self.refresh_profile(profile_user_id, profile_tag, cx);
        cx.spawn(async move |cx: &mut AsyncApp| {
            refresh.await;
            let minimum = Duration::from_secs_f64(PostListRefreshControl::MINIMUM_VISIBLE_DURATION);
            if let Some(remaining) = minimum.checked_sub(RiseClock::monotonic().saturating_sub(started_at)) {
                cx.background_executor().timer(remaining).await;
            }
            cx.update(|cx| {
                pull_refresh.update(cx, |state, cx| {
                    state.is_refreshing = false;
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn refresh_profile(&self, profile_user_id: String, profile_tag: String, cx: &mut App) -> Task<()> {
        let interactions = UserInteractionsStore::shared(cx);
        match &self.route_target {
            ProfileRouteTarget::Own => interactions.update(cx, |store, cx| store.refresh_my_profile(cx)),
            ProfileRouteTarget::ForeignTag(tag) => interactions.update(cx, |store, cx| {
                store.refresh_foreign_profile_tag(tag.clone(), cx);
                store.refresh_block_state(&profile_user_id, cx);
            }),
            ProfileRouteTarget::ForeignUserId(user_id) => interactions.update(cx, |store, cx| {
                store.refresh_foreign_profile_user_id(user_id.clone(), cx);
                store.refresh_block_state(user_id, cx);
            }),
            ProfileRouteTarget::Invalid => return Task::ready(()),
        }
        if profile_user_id.is_empty() {
            return Task::ready(());
        }
        let selected = self.state.read(cx).selected_profile_tab;
        let kind = match selected {
            ProfileContentTab::Publications => Some(RisePostProfileKind::Publications),
            ProfileContentTab::Riffs => Some(RisePostProfileKind::Riffs),
            ProfileContentTab::Threads => Some(RisePostProfileKind::Threads),
            ProfileContentTab::Reposts => Some(RisePostProfileKind::Reposts),
            ProfileContentTab::Mentions => Some(RisePostProfileKind::Mentions),
            ProfileContentTab::Favorites => Some(RisePostProfileKind::Favorites),
            ProfileContentTab::Liked => Some(RisePostProfileKind::Liked),
            _ => None,
        };
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.refresh_profile_stories(Some(&profile_user_id), StoryProfileScope::Profile, cx);
        });
        let posts = kind.map(|kind| {
            PostInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.refresh_profile(&profile_user_id, &profile_tag, kind, cx))
        });
        let is_gifts = selected == ProfileContentTab::Gifts;
        cx.spawn(async move |cx: &mut AsyncApp| {
            if let Some(posts) = posts {
                posts.await;
            }
            if is_gifts {
                GiftsActionsStore::refresh_tab(&profile_user_id, cx).await;
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn profile_scroll_view(
        self: &Rc<Self>,
        profile: ProfileModel,
        profile_user_id: String,
        own: bool,
        foreign_goals: Vec<GoalItem>,
        foreign_plans: Vec<PlanItem>,
        foreign_closed_tabs: HashSet<ProfileContentTab>,
        foreign_owner_totally_blocked: bool,
        visible_tabs: Vec<ProfileContentTab>,
        scroll_scope: String,
        owner_resolving: bool,
        theme: &AppTheme,
        viewport_height: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let (coordinator, pull_refresh, effect_replay) = {
            let state = self.state.read(cx);
            (state.profile_scroll_coordinator.clone(), state.pull_refresh.clone(), state.profile_effect_replay)
        };
        let hold_active = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let outer = coordinator.read(cx).outer_host_scroll_view();
        let bottom_inset = main_tab_bar_underlap(cx);

        let select_state = self.clone();
        let select_tabs = visible_tabs.clone();
        let swipe_state = self.clone();
        let swipe_tabs = visible_tabs.clone();
        let can_swipe_state = self.state.clone();
        let can_swipe_tabs = visible_tabs.clone();
        let introspector = ProfileOuterScrollIntrospector::new(
            coordinator.clone(),
            Rc::new(move |direction: i64, cx: &mut App| {
                swipe_state.select_adjacent_profile_tab(direction, &swipe_tabs, cx);
            }),
            Rc::new(move |direction: i64, cx: &App| {
                let selected = can_swipe_state.read(cx).selected_profile_tab;
                Self::can_select_adjacent_profile_tab(selected, direction, &can_swipe_tabs)
            }),
        );

        let content = ProfileContent::new(
            child_id(&self.id, "content"),
            visible_tabs,
            Binding::entity(
                self.state.clone(),
                |state: &ProfileState| state.selected_profile_tab,
                |state: &mut ProfileState, value, cx| {
                    state.selected_profile_tab = value;
                    cx.notify();
                },
            ),
            Binding::entity(
                self.state.clone(),
                |state: &ProfileState| state.publications_use_grid,
                |state: &mut ProfileState, value, cx| {
                    state.publications_use_grid = value;
                    cx.notify();
                },
            ),
            Binding::entity(
                self.state.clone(),
                |state: &ProfileState| state.threads_use_grid,
                |state: &mut ProfileState, value, cx| {
                    state.threads_use_grid = value;
                    cx.notify();
                },
            ),
            viewport_height,
            bottom_inset,
            scroll_scope.clone(),
            coordinator.clone(),
            Rc::new(move |tab: ProfileContentTab, _: &mut Window, cx: &mut App| {
                select_state.select_profile_tab(tab, &select_tabs, true, cx);
            }),
        )
        .profile_tag(profile.tag.clone())
        .profile_user_id(profile_user_id.clone())
        .is_own_profile(own)
        .foreign_goals(foreign_goals)
        .foreign_plans(foreign_plans)
        .foreign_closed_tabs(foreign_closed_tabs)
        .foreign_owner_totally_blocked(foreign_owner_totally_blocked);

        let pinned = ProfileMasterPinnedModifier::new(coordinator).body(div().relative().w_full().child(content), window, cx);

        let counts_resolving = owner_resolving || (own && Self::own_counts_pending(&profile, cx));
        let stack = env_scope::<ProfileGridMenuAsOwnerKey>(own).child(
            env_scope::<ProfileOwnerResolvingKey>(owner_resolving).child(
                vstack(0.0)
                    .w_full()
                    .child(introspector)
                    .child(
                        ProfileTop::new(child_id(&self.id, "top"), profile.clone())
                            .is_user(self.is_user)
                            .is_resolving(owner_resolving)
                            .counts_resolving(counts_resolving)
                            .effect_replay(effect_replay)
                            .cached_slot(child_id(&self.id, "top-host"), window, cx),
                    )
                    .child(div().h(px(ProfileViewportLayoutPolicy::OUTER_SPACING)))
                    .child(pinned),
            ),
        );

        let pull_state = self.clone();
        let pull_user_id = profile_user_id.clone();
        let pull_tag = profile.tag.clone();
        let scroller = scroll_view(child_id(&self.id, "outer-scroll"))
            .size_full()
            .scroll_handle(outer.handle())
            .restrict_scroll_to_axis(true)
            .shows_indicators(false)
            .scroll_disabled(hold_active || !outer.pan_enabled())
            .refreshable(move |_, cx| {
                pull_state.start_pull_refresh(pull_user_id.clone(), pull_tag.clone(), cx);
                Task::ready(())
            })
            .child(stack);

        let keys_state = self.clone();
        let keys_user_id = profile_user_id.clone();
        let keys_tag = profile.tag.clone();
        let refreshable = PageRefreshKeys::new(
            child_id(&self.id, "refresh-keys"),
            move |_, cx| keys_state.start_pull_refresh(keys_user_id.clone(), keys_tag.clone(), cx),
            env_scope::<HoldContextMenuLocksScrollKey>(hold_active).child(scroller),
        );

        let is_selecting_stories = StoryGridSelection::shared(cx)
            .read(cx)
            .is_active_for(&profile_user_id, StoryProfileScope::Profile);

        let floating = if !own {
            None
        } else if is_selecting_stories {
            Some(
                BottomChromeFloatingAction::new(
                    child_id(&self.id, "floating-stories"),
                    StoriesGridActionBar::new(profile_user_id.clone(), StoryProfileScope::Profile),
                )
                .trailing_padding(0.0)
                .into_any_element(),
            )
        } else {
            Some(
                BottomChromeFloatingAction::new(
                    child_id(&self.id, "floating-create"),
                    div()
                        .size(px(ProfileCreateActionMetrics::SIDE))
                        .rounded_full()
                        .bg(theme.bg._100)
                        .shadow(vec![BoxShadow {
                            color: black().opacity(0.18),
                            offset: point(px(0.0), px(2.0)),
                            blur_radius: px(8.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                        .child(self.create_action(theme)),
                )
                .into_any_element(),
            )
        };

        let composer = own.then(|| {
            StoryComposerHost::new(
                child_id(&self.id, "story-composer"),
                Binding::entity(
                    self.state.clone(),
                    |state: &ProfileState| state.show_story_composer,
                    |state: &mut ProfileState, value, cx| {
                        state.show_story_composer = value;
                        cx.notify();
                    },
                ),
            )
        });

        let loader = owner_resolving.then(|| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg._100.opacity(0.35))
                .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(theme.primary._100)))
        });

        let header = self.profile_header(&profile, own, theme, &scroll_scope, &profile_user_id, is_selecting_stories, window, cx);

        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .children(composer)
            .child(refreshable)
            .child(
                div().absolute().top_0().left_0().right_0().child(ProfilePullRefreshIndicator {
                    id: child_id(&self.id, "pull-indicator"),
                    state: pull_refresh,
                }),
            )
            .child(div().absolute().top_0().left_0().right_0().child(header))
            .children(floating.map(|floating| div().absolute().inset_0().child(floating)))
            .children(loader)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn profile_header(
        self: &Rc<Self>,
        profile: &ProfileModel,
        own: bool,
        theme: &AppTheme,
        trace_scope: &str,
        profile_user_id: &str,
        is_selecting_stories: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let tag = trimming_whitespaces_and_newlines(&profile.tag).to_owned();

        let now = RiseClock::monotonic();
        let shown = is_selecting_stories.then_some(());
        let done_presence = window.use_keyed_state(child_id(&self.id, "done-presence"), cx, move |_, _| {
            Presence::new(shown)
                .with_transition(Transition::opacity().combined(Transition::scale(DONE_BUTTON_TRANSITION_SCALE)))
        });
        done_presence.update(cx, |presence, cx| {
            presence.set_in(shown, ease_out(DONE_BUTTON_TRANSITION_DURATION), now, cx);
        });
        let done_button = done_presence
            .read(cx)
            .render_transitioned(now, window, |_| div().child(StoriesGridDoneButton));

        let mut right = hstack(HEADER_CONTROLS_SPACING).children(done_button);
        if !is_selecting_stories {
            right = right.child(
                ProfilePerformanceTraceControl::new(SharedString::from(trace_scope.to_owned())).header_chrome(true),
            );
            if own {
                let settings = self.clone();
                let anchor = if self.shows_back_button { None } else { Some(RiseGuideAnchor::MoreGuides) };
                right = right.child(
                    PageHeaderIconButton::new(child_id(&self.id, "settings"), "line.3.horizontal", move |_, cx| {
                        settings.push(RootRoute::Settings, cx);
                    })
                    .guide_anchor(anchor),
                );
            } else {
                right = right.child(self.foreign_profile_menu(profile, theme, cx));
            }
        }

        let back = self.clone();
        let mut header = PageHeaderUi::new()
            .shows_back_button(self.shows_back_button)
            .on_back(move |_, cx| back.pop(cx))
            .right_jsx(right);
        if own || !tag.is_empty() {
            header = header.center_slot(self.account_menu(&tag, own, theme, window, cx)).center_slot_self_styled(true);
        }
        let toggle_reserve = rise_ui::swiftui::environment::env::<PanelToggleHeaderReserveKey>(cx);
        div().pt(px(ProfileScrollMetrics::HEADER_TOP_PADDING)).pr(toggle_reserve).child(header)
    }

    fn foreign_profile_menu(self: &Rc<Self>, profile: &ProfileModel, theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let mut actions = Self::foreign_profile_actions(profile, cx);
        actions.extend(Self::admin_ban_actions(profile, cx));
        let profile = profile.clone();
        let handler = self.clone();
        let opener = self.state.clone();
        let source_theme = theme.clone();
        let source: HoldContextMenuTapSource =
            Rc::new(move |_: &mut Window, _: &mut App| Self::foreign_menu_glyph(&source_theme).into_any_element());
        plain(child_id(&self.id, "foreign-menu"))
            .on_press(move |_, cx| {
                opener.update(cx, |state, cx| {
                    state.is_foreign_menu_presented = true;
                    cx.notify();
                });
            })
            .child(Self::foreign_menu_glyph(theme))
            .tap_context_menu(
                child_id(&self.id, "foreign-menu-anchor"),
                Binding::entity(
                    self.state.clone(),
                    |state: &ProfileState| state.is_foreign_menu_presented,
                    |state: &mut ProfileState, value, cx| {
                        state.is_foreign_menu_presented = value;
                        cx.notify();
                    },
                ),
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                Rc::new(move |_: &App| actions.clone()),
                Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                    handler.handle_foreign_profile_action(&id, &profile, cx);
                }),
                source,
            )
    }

    fn foreign_menu_glyph(theme: &AppTheme) -> impl IntoElement + use<> {
        PageHeaderToolbarSlot::new(
            div()
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("ellipsis", px(FOREIGN_MENU_ICON_SIDE), theme.text.primary, false)),
        )
        .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
    }

    fn foreign_profile_actions(profile: &ProfileModel, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let id = profile.id.clone();
        let story = StoryInteractionsStore::shared(cx);
        let is_close = story.update(cx, |store, cx| store.is_close_friend(Some(&id), cx));
        let services = UserServicesStore::shared(cx);
        let block_scope = services.update(cx, |store, cx| store.viewer_block_scope(&id, cx));
        let block_pending = services.read(cx).contact_block_pending_targets.contains(&id);
        let mut actions = vec![
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
            HoldContextMenuAction::new("notifications", tr("ctx_notifications")).system_image("bell"),
            HoldContextMenuAction::new(
                "close_friend",
                if is_close { tr("story_close_friends_remove") } else { tr("story_close_friends_add") },
            )
            .system_image(if is_close { "star.slash" } else { "star" }),
        ];
        if story.update(cx, |store, cx| store.is_author_muted(Some(&id), cx)) {
            actions.push(HoldContextMenuAction::new("show_stories", tr("story_menu_show_stories")).system_image("eye"));
        }
        if !Self::can_manage_block(profile, cx) {
            return actions;
        }
        actions.push(
            HoldContextMenuAction::new("report", tr("ctx_report"))
                .system_image("exclamationmark.bubble")
                .destructive(true),
        );
        if profile.is_bot && profile.more.is_official && block_scope.is_none() {
            return actions;
        }
        if block_scope.is_some() {
            actions.push(
                HoldContextMenuAction::new("unblock", tr("contact_unblock_action"))
                    .system_image("hand.raised.slash")
                    .disabled(block_pending),
            );
            actions.push(
                HoldContextMenuAction::new("block_change", tr("contact_block_scope_change_action"))
                    .system_image("arrow.left.arrow.right")
                    .disabled(block_pending),
            );
        } else {
            actions.push(
                HoldContextMenuAction::new("block_standard", tr("contact_block_scope_standard"))
                    .system_image("hand.raised.fill")
                    .destructive(true)
                    .disabled(block_pending),
            );
            actions.push(
                HoldContextMenuAction::new("block_total", tr("contact_block_scope_total"))
                    .system_image("hand.raised.slash.fill")
                    .destructive(true)
                    .disabled(block_pending),
            );
        }
        actions
    }

    fn admin_ban_actions(profile: &ProfileModel, cx: &mut App) -> Vec<HoldContextMenuAction> {
        if !AdminBanEntry::is_available(&profile.id, Some(profile.more.role.as_str()), cx) {
            return Vec::new();
        }
        vec![AdminBanEntry::action()]
    }

    fn handle_foreign_profile_action(&self, id: &str, profile: &ProfileModel, cx: &mut App) {
        match id {
            AdminBanEntry::ACTION_ID => {
                let target = AdminBanTarget { user_id: profile.id.clone(), name: Some(profile.name.clone()) };
                self.state.update(cx, |state, cx| {
                    state.admin_ban_target = Some(target);
                    cx.notify();
                });
            }
            "share" => {
                let profile = profile.clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_profile(profile, cx));
            }
            "notifications" => {
                let chats = ChatServicesStore::shared(cx);
                let chat_id = chats.read(cx).known_direct_chat_id(&profile.id, cx);
                NotificationsHubSheetStore::shared(cx).update(cx, |store, cx| {
                    store.present(Some(chat_id.as_str()), None, Some(&profile.id), cx);
                });
            }
            "close_friend" => Self::toggle_close_friend(profile, cx),
            "show_stories" => {
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.unmute_author(Some(&profile.id), cx));
            }
            "report" => {
                let target = ReportTarget::User(profile.id.clone());
                ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, None, cx));
            }
            "unblock" => {
                UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.set_user_block_scope(&profile.id, None, |_, _| {}, cx);
                });
            }
            "block_change" => {
                let current =
                    UserServicesStore::shared(cx).update(cx, |store, cx| store.viewer_block_scope(&profile.id, cx));
                let next =
                    if current == Some(RiseBlockScope::Total) { RiseBlockScope::Standard } else { RiseBlockScope::Total };
                self.set_block_intent(&profile.id, next, cx);
            }
            "block_standard" => self.set_block_intent(&profile.id, RiseBlockScope::Standard, cx),
            "block_total" => self.set_block_intent(&profile.id, RiseBlockScope::Total, cx),
            _ => {}
        }
    }

    fn set_block_intent(&self, target_user_id: &str, scope: RiseBlockScope, cx: &mut App) {
        let intent = BlockIntent { target_user_id: target_user_id.to_owned(), scope };
        self.state.update(cx, |state, cx| {
            state.block_intent = Some(intent);
            cx.notify();
        });
    }

    fn can_manage_block(profile: &ProfileModel, cx: &mut App) -> bool {
        let id = trimming_whitespaces_and_newlines(&profile.id);
        if id.is_empty() {
            return false;
        }
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().as_deref() != Some(id)
    }

    fn block_scope_title_key(scope: RiseBlockScope) -> &'static str {
        match scope {
            RiseBlockScope::Standard => "contact_block_scope_standard",
            RiseBlockScope::Total => "contact_block_scope_total",
        }
    }

    fn block_scope_description_key(scope: RiseBlockScope) -> &'static str {
        match scope {
            RiseBlockScope::Standard => "contact_block_scope_standard_desc",
            RiseBlockScope::Total => "contact_block_scope_total_desc",
        }
    }

    fn toggle_close_friend(profile: &ProfileModel, cx: &mut App) {
        let author = StoryAuthorDTO {
            id: profile.id.clone(),
            is_official: false,
            is_online: false,
            is_premium: profile.more.is_premium,
            last_seen_ms: None,
            logo: profile.more.logo.clone(),
            name: profile.name.clone(),
            name_color: profile.more.name_color.clone(),
            premium_badge_emoji: profile.more.premium_badge_emoji.clone(),
            tag: profile.tag.clone(),
        };
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.toggle_close_friend(&author, None, None, cx);
        });
    }

    fn create_action(self: &Rc<Self>, theme: &AppTheme) -> impl IntoElement + use<> {
        let opener = self.state.clone();
        let handler = self.clone();
        let source_theme = theme.clone();
        let source: HoldContextMenuTapSource =
            Rc::new(move |_: &mut Window, _: &mut App| Self::create_action_glyph(&source_theme).into_any_element());
        div()
            .id(child_id(&self.id, "create-label"))
            .tooltip_label(tr("create_post_text"))
            .child(
                plain(child_id(&self.id, "create-action"))
                    .on_press(move |_, cx| {
                        opener.update(cx, |state, cx| {
                            state.is_create_menu_presented = true;
                            cx.notify();
                        });
                    })
                    .child(Self::create_action_glyph(theme))
                    .tap_context_menu(
                        child_id(&self.id, "create-menu"),
                        Binding::entity(
                            self.state.clone(),
                            |state: &ProfileState| state.is_create_menu_presented,
                            |state: &mut ProfileState, value, cx| {
                                state.is_create_menu_presented = value;
                                cx.notify();
                            },
                        ),
                        HoldContextMenuAlignment::Trailing,
                        false,
                        false,
                        false,
                        None,
                        Rc::new(|_: &App| {
                            vec![
                                HoldContextMenuAction::new("story", tr("create_menu_story"))
                                    .system_image(rise_ui::rise_symbol::RiseSymbol::STORY_CREATE),
                                HoldContextMenuAction::new("publication", tr("create_post_kind_publication"))
                                    .system_image("photo.on.rectangle.angled"),
                                HoldContextMenuAction::new("reel", tr("riffs")).system_image("play.rectangle"),
                                HoldContextMenuAction::new("thread", tr("create_post_kind_thread"))
                                    .system_image("text.bubble"),
                            ]
                        }),
                        Rc::new(move |id: String, _: &mut Window, cx: &mut App| handler.handle_create_action(&id, cx)),
                        source,
                    ),
            )
            .guide_anchor(Some(RiseGuideAnchor::ProfileCreatePost))
    }

    fn create_action_glyph(theme: &AppTheme) -> impl IntoElement + use<> {
        PageHeaderToolbarSlot::new(
            div()
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("plus", px(CREATE_ACTION_ICON_SIDE), theme.text.primary, false)),
        )
        .size(ProfileCreateActionMetrics::SIDE)
    }

    fn handle_create_action(&self, id: &str, cx: &mut App) {
        match id {
            "story" => self.state.update(cx, |state, cx| {
                state.show_story_composer = true;
                cx.notify();
            }),
            "publication" => self.push(
                RootRoute::CreatePostMedia { intent: CreatePostIntent::Publication, track_seed: None, journey_id: None },
                cx,
            ),
            "reel" => self.push(
                RootRoute::CreatePostMedia { intent: CreatePostIntent::Reel, track_seed: None, journey_id: None },
                cx,
            ),
            "thread" => self.push(RootRoute::CreatePost { intent: CreatePostIntent::Thread, journey_id: None }, cx),
            _ => {}
        }
    }

    fn account_menu(
        self: &Rc<Self>,
        tag: &str,
        own: bool,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if !own {
            let copied = format!("@{tag}");
            let shown = copied.clone();
            return LiquidGlassButton::new(child_id(&self.id, "tag-pill"), LiquidGlassButtonShape::Capsule)
                .horizontal_padding(TAG_PILL_HORIZONTAL_PADDING)
                .vertical_padding(TAG_PILL_VERTICAL_PADDING)
                .on_press(move |_, cx| {
                    UIPasteboard::general().set_string(&copied, cx);
                    let _ = selection_changed();
                })
                .child(
                    MainText::new(shown)
                        .px(TAG_PILL_FONT_SIZE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary)
                        .number_of_lines(1),
                )
                .into_any_element();
        }

        let accounts = AuthServicesStore::shared(cx).read(cx).sorted_accounts();
        let title = Self::own_account_menu_title(tag, &accounts);

        let loader_state = self.state.clone();
        on_change(
            child_id(&self.id, "account-menu-avatars"),
            Self::account_menu_avatar_key(&accounts),
            true,
            window,
            cx,
            move |_, _, _, cx| {
                let accounts = AuthServicesStore::shared(cx).read(cx).sorted_accounts();
                let weak = loader_state.downgrade();
                let task = cx.spawn(async move |cx: &mut AsyncApp| {
                    let avatars = AccountMenuAvatarLoader::load(accounts, cx).await;
                    let _ = weak.update(cx, |state, cx| {
                        state.account_menu_avatars = avatars;
                        cx.notify();
                    });
                });
                loader_state.update(cx, |state, _| state.account_menu_avatar_task = Some(task));
            },
        );

        let actions = self.account_menu_actions(&accounts, window, cx);
        let opener = self.state.clone();
        let handler = self.clone();
        let source_title = title.clone();
        let source_theme = theme.clone();
        let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
            Self::account_menu_pill(&source_title, &source_theme).into_any_element()
        });
        plain(child_id(&self.id, "account-menu"))
            .on_press(move |_, cx| {
                tracing::warn!(target: "riseonly::X1271", "account menu pill pressed");
                opener.update(cx, |state, cx| {
                    state.is_account_menu_presented = true;
                    cx.notify();
                });
            })
            .child(Self::account_menu_pill(&title, theme))
            .tap_context_menu(
                child_id(&self.id, "account-menu-anchor"),
                Binding::entity(
                    self.state.clone(),
                    |state: &ProfileState| state.is_account_menu_presented,
                    |state: &mut ProfileState, value, cx| {
                        state.is_account_menu_presented = value;
                        cx.notify();
                    },
                ),
                HoldContextMenuAlignment::Leading,
                false,
                false,
                true,
                None,
                Rc::new(move |_: &App| actions.clone()),
                Rc::new(move |id: String, _: &mut Window, cx: &mut App| handler.handle_account_menu_action(&id, cx)),
                source,
            )
            .guide_anchor(Some(RiseGuideAnchor::ProfileAccountMenu))
            .into_any_element()
    }

    fn account_menu_pill(title: &str, theme: &AppTheme) -> impl IntoElement + use<> {
        hstack(ACCOUNT_MENU_PILL_SPACING)
            .child(
                MainText::new(title.to_owned())
                    .px(TAG_PILL_FONT_SIZE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary)
                    .number_of_lines(1),
            )
            .children(IconUi::sized("chevron.down", px(ACCOUNT_MENU_CHEVRON_SIDE), theme.text.secondary, false))
            .liquid_glass_nav_pill(ACCOUNT_MENU_PILL_PADDING, None)
    }

    fn account_menu_avatar_key(accounts: &[AuthAccountSummary]) -> String {
        accounts
            .iter()
            .map(|account| format!("{}|{}", account.user_id, account.avatar_url.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn account_menu_actions(
        &self,
        accounts: &[AuthAccountSummary],
        window: &Window,
        cx: &App,
    ) -> Vec<HoldContextMenuAction> {
        let avatars = &self.state.read(cx).account_menu_avatars;
        let mut actions: Vec<HoldContextMenuAction> = accounts
            .iter()
            .enumerate()
            .map(|(index, account)| {
                let row_title =
                    if account.tag.is_empty() { account.name.clone() } else { format!("@{}", account.tag) };
                HoldContextMenuAction::new(
                    format!("account:{}", account.user_id),
                    Self::account_menu_row_title(&row_title, window),
                )
                .system_image("person.crop.circle")
                .section_break(index == 1)
                .checked(index == 0)
                .leading_image(
                    avatars.get(&account.user_id).cloned().or_else(|| AccountMenuAvatarLoader::letter(account)),
                )
            })
            .collect();
        actions.push(HoldContextMenuAction::new("add_account", tr("add_account")).system_image("plus").section_break(true));
        actions
    }

    fn handle_account_menu_action(&self, id: &str, cx: &mut App) {
        tracing::warn!(target: "riseonly::X1529", action = %id, "account menu action");
        if id == "add_account" {
            let auth = AuthServicesStore::shared(cx);
            if auth.read(cx).can_add_account() {
                auth.update(cx, |auth, cx| auth.begin_adding_account(cx));
            } else {
                self.state.update(cx, |state, cx| {
                    state.show_account_limit_sheet = true;
                    cx.notify();
                });
            }
            return;
        }
        let Some(user_id) = id.strip_prefix("account:") else {
            return;
        };
        let user_id = user_id.to_owned();
        AuthServicesStore::shared(cx).update(cx, |auth, cx| auth.switch_account(&user_id, cx));
    }

    fn account_menu_row_title(title: &str, window: &Window) -> String {
        let font = window.text_style().font();
        let width = |text: &str| -> f32 {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(
                window
                    .text_system()
                    .shape_line(SharedString::from(text.to_owned()), px(ACCOUNT_MENU_ROW_FONT_SIZE), &[run], None)
                    .width(),
            )
        };
        if width(title) <= ACCOUNT_MENU_ROW_TITLE_LIMIT {
            return title.to_owned();
        }
        let mut characters: Vec<&str> = rise_core::foundation::foundation_text::characters(title).collect();
        while !characters.is_empty() {
            let candidate = characters.concat() + "\u{2026}";
            if width(&candidate) <= ACCOUNT_MENU_ROW_TITLE_LIMIT {
                return candidate;
            }
            characters.pop();
        }
        "\u{2026}".to_owned()
    }

    fn own_account_menu_title(tag: &str, sorted_accounts: &[AuthAccountSummary]) -> String {
        if !tag.is_empty() {
            return format!("@{tag}");
        }
        let Some(current) = sorted_accounts.first() else {
            return tr("account_switcher_error");
        };
        if !current.tag.is_empty() {
            return format!("@{}", current.tag);
        }
        if !current.name.is_empty() {
            return current.name.clone();
        }
        tr("account_switcher_error")
    }

    fn select_profile_tab(&self, tab: ProfileContentTab, visible_tabs: &[ProfileContentTab], haptic: bool, cx: &mut App) {
        if !visible_tabs.contains(&tab) || tab == self.state.read(cx).selected_profile_tab {
            return;
        }
        if haptic {
            let _ = selection_changed();
        }
        let coordinator = self.state.update(cx, |state, cx| {
            state.selected_profile_tab = tab;
            cx.notify();
            state.profile_scroll_coordinator.clone()
        });
        coordinator.update(cx, |coordinator, cx| coordinator.set_active_tab(tab, cx));
        self.update_navigation_pop_priority(tab, visible_tabs, cx);
    }

    fn can_select_adjacent_profile_tab(
        selected: ProfileContentTab,
        direction: i64,
        visible_tabs: &[ProfileContentTab],
    ) -> bool {
        let Some(current) = visible_tabs.iter().position(|tab| *tab == selected) else {
            return false;
        };
        ProfileHorizontalPagerPolicy::target_index(current as i64, direction, visible_tabs.len() as i64).is_some()
    }

    fn select_adjacent_profile_tab(&self, direction: i64, visible_tabs: &[ProfileContentTab], cx: &mut App) {
        let selected = self.state.read(cx).selected_profile_tab;
        let Some(current) = visible_tabs.iter().position(|tab| *tab == selected) else {
            return;
        };
        let Some(next) =
            ProfileHorizontalPagerPolicy::target_index(current as i64, direction, visible_tabs.len() as i64)
        else {
            return;
        };
        self.select_profile_tab(visible_tabs[next as usize], visible_tabs, true, cx);
    }

    fn reset_profile_scroll_if_needed(&self, scope: &str, visible_tabs: &[ProfileContentTab], cx: &mut App) {
        let Some(first) = visible_tabs.first().copied() else {
            return;
        };
        if self.state.read(cx).configured_profile_scroll_scope == scope {
            return;
        }
        let coordinator = self.state.update(cx, |state, cx| {
            state.configured_profile_scroll_scope = scope.to_owned();
            state.selected_profile_tab = first;
            cx.notify();
            state.profile_scroll_coordinator.clone()
        });
        coordinator.update(cx, |coordinator, cx| coordinator.set_scope(scope, first, cx));
        self.update_navigation_pop_priority(first, visible_tabs, cx);
    }

    /// right swipe is taken from screen dismissal only inside tab pages,
    /// in the header back swipe still pops
    fn update_navigation_pop_priority(&self, tab: ProfileContentTab, visible_tabs: &[ProfileContentTab], cx: &mut App) {
        let consumes_back_swipe = visible_tabs.iter().position(|candidate| *candidate == tab).unwrap_or(0) > 0;
        let (coordinator, owner) = {
            let state = self.state.read(cx);
            (state.profile_scroll_coordinator.clone(), state.navigation_pop_paging_owner)
        };
        let pager = coordinator.read(cx).horizontal_pager_host_scroll_view();
        NavigationInteractivePopGuard::set_profile_tab_paging_back(
            consumes_back_swipe,
            owner,
            Some(Rc::new(move |point: Point<Pixels>| pager.is_in_window() && pager.frame().contains(&point))),
            cx,
        );
    }

    fn profile_scroll_scope(&self, profile: &ProfileModel, own: bool) -> String {
        let identity = trimming_whitespaces_and_newlines(&profile.id);
        if own {
            return format!("own:{identity}");
        }
        match &self.route_target {
            ProfileRouteTarget::ForeignTag(tag) => format!("tag:{}|{identity}", tag.to_lowercase()),
            ProfileRouteTarget::ForeignUserId(user_id) => format!("user:{user_id}"),
            ProfileRouteTarget::Own => format!("own:{identity}"),
            ProfileRouteTarget::Invalid => "invalid".to_owned(),
        }
    }

    fn is_own_profile_tab(&self) -> bool {
        self.route_target == ProfileRouteTarget::Own
    }

    fn viewing_own(&self, profile: &ProfileModel, cx: &mut App) -> bool {
        if self.is_own_profile_tab() {
            return true;
        }
        let auth = AuthServicesStore::shared(cx);
        match auth.read(cx).profile().map(|me| me.id.as_str()) {
            Some(my_id) if !my_id.is_empty() => profile.id == my_id,
            _ => false,
        }
    }

    fn resolve_profile(&self, foreign_payload: Option<&UserGetProfileByTagGatewayDTO>, cx: &mut App) -> ProfileModel {
        let matching = foreign_payload.and_then(|payload| payload.as_profile_model());
        match &self.route_target {
            ProfileRouteTarget::Own => {
                AuthServicesStore::shared(cx).read(cx).profile().cloned().unwrap_or_default()
            }
            ProfileRouteTarget::ForeignTag(tag) => match matching {
                Some(model) => UserServicesStore::shared(cx).read(cx).apply_follow_overlay(model),
                None => ProfileModel { tag: tag.clone(), ..ProfileModel::default() },
            },
            ProfileRouteTarget::ForeignUserId(user_id) => match matching {
                Some(model) => UserServicesStore::shared(cx).read(cx).apply_follow_overlay(model),
                None => ProfileModel { id: user_id.clone(), is_deleted: self.foreign_deleted(cx), ..ProfileModel::default() },
            },
            ProfileRouteTarget::Invalid => ProfileModel::default(),
        }
    }

    fn own_counts_pending(profile: &ProfileModel, cx: &mut App) -> bool {
        let read = UserServicesStore::shared(cx).read(cx).has_read_my_profile();
        let failed = UserActionsStore::shared(cx).read(cx).my_profile_sai.status() == SaiWsSliceStatus::Rejected;
        !read
            && !failed
            && profile.posts_count == 0
            && profile.subscribers_count == 0
            && profile.subs_count == 0
            && profile.friends_count == 0
    }

    /// erased account opened from a chat, no server profile exists so the page is a placeholder
    fn foreign_deleted(&self, cx: &mut App) -> bool {
        match &self.route_target {
            ProfileRouteTarget::ForeignUserId(user_id) => {
                ProfileMissingTagState::shared(cx).read(cx).is_deleted_user(user_id)
            }
            _ => false,
        }
    }

    fn shows_full_screen_loader(&self, foreign_payload: Option<&UserGetProfileByTagGatewayDTO>, cx: &mut App) -> bool {
        if self.foreign_deleted(cx) {
            return false;
        }
        let matched = foreign_payload.and_then(|payload| payload.as_profile_model()).is_some();
        let services = UserServicesStore::shared(cx);
        let actions = UserActionsStore::shared(cx);
        match &self.route_target {
            ProfileRouteTarget::Own => {
                actions.read(cx).my_profile_sai.is_pending() && AuthServicesStore::shared(cx).read(cx).profile().is_none()
            }
            ProfileRouteTarget::ForeignTag(tag) => {
                let pending = services.update(cx, |store, cx| store.is_foreign_profile_pending_tag(tag, cx));
                (pending || actions.read(cx).get_profile_by_tag_sai.is_pending()) && !matched
            }
            ProfileRouteTarget::ForeignUserId(user_id) => {
                let pending = services.update(cx, |store, cx| store.is_foreign_profile_pending_user_id(user_id, cx));
                (pending || actions.read(cx).get_profile_by_id_sai.is_pending()) && !matched
            }
            ProfileRouteTarget::Invalid => false,
        }
    }

    /// slot key of the foreign route, the tag or the prefixed user id
    fn foreign_route_key(&self) -> Option<String> {
        match &self.route_target {
            ProfileRouteTarget::ForeignTag(tag) => Some(tag.clone()),
            ProfileRouteTarget::ForeignUserId(user_id) => Some(ProfileMissingTagState::user_id_key(user_id)),
            ProfileRouteTarget::Own | ProfileRouteTarget::Invalid => None,
        }
    }

    fn foreign_route_settled(&self, cx: &mut App) -> bool {
        let services = UserServicesStore::shared(cx);
        match &self.route_target {
            ProfileRouteTarget::ForeignTag(tag) => self.foreign_tag_settled(tag, cx),
            ProfileRouteTarget::ForeignUserId(user_id) => {
                !UserActionsStore::shared(cx).read(cx).get_profile_by_id_sai.is_pending()
                    && !services.update(cx, |store, cx| store.is_foreign_profile_pending_user_id(user_id, cx))
            }
            ProfileRouteTarget::Own | ProfileRouteTarget::Invalid => false,
        }
    }

    fn missing_foreign_tag(&self, foreign_payload: Option<&UserGetProfileByTagGatewayDTO>, cx: &mut App) -> Option<String> {
        let key = self.foreign_route_key()?;
        if foreign_payload.is_some() || self.foreign_deleted(cx) || !self.foreign_route_settled(cx) {
            return None;
        }
        ProfileMissingTagState::shared(cx)
            .read(cx)
            .is_missing(
                &key,
                SaiAccountCacheScope::active_user_id().as_deref(),
                SaiAccountCacheScope::active_epoch().as_deref(),
            )
            .then_some(key)
    }

    fn foreign_tag_settled(&self, tag: &str, cx: &mut App) -> bool {
        !UserActionsStore::shared(cx).read(cx).get_profile_by_tag_sai.is_pending()
            && !UserServicesStore::shared(cx).update(cx, |store, cx| store.is_foreign_profile_pending_tag(tag, cx))
    }

    fn missing_profile_page(self: &Rc<Self>) -> impl IntoElement + use<> {
        let back = self.clone();
        let route = self.clone();
        let refresh = move |cx: &mut App| route.refresh_foreign_route(cx);
        let pull = refresh.clone();
        ListShellUi::new().on_back(move |_, cx| back.pop(cx)).content(PageRefreshKeys::new(
            child_id(&self.id, "missing-keys"),
            move |_, cx| refresh(cx),
            scroll_view(child_id(&self.id, "missing-scroll"))
                .size_full()
                .refreshable(move |_, cx| {
                    pull(cx);
                    Task::ready(())
                })
                .child(
                    div().pt(px(PAGE_STATE_TOP_PADDING)).child(ChatProfileEmptyState::new(
                        "person.fill.questionmark",
                        tr("mention_not_found_title"),
                    )),
                ),
        ))
    }

    fn is_foreign_route(&self) -> bool {
        matches!(self.route_target, ProfileRouteTarget::ForeignTag(_) | ProfileRouteTarget::ForeignUserId(_))
    }

    fn foreign_load_failed(&self, payload: Option<&UserGetProfileByTagGatewayDTO>, cx: &mut App) -> bool {
        if !self.is_foreign_route() || payload.is_some() || self.foreign_deleted(cx) {
            return false;
        }
        if self.state.read(cx).foreign_load_stalled_offline {
            return true;
        }
        let Some(key) = self.foreign_route_key() else {
            return false;
        };
        if !self.foreign_route_settled(cx) {
            return false;
        }
        ProfileMissingTagState::shared(cx).read(cx).is_failed(
            &key,
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
        )
    }

    fn refresh_foreign_route(&self, cx: &mut App) {
        let interactions = UserInteractionsStore::shared(cx);
        match &self.route_target {
            ProfileRouteTarget::ForeignTag(tag) => {
                interactions.update(cx, |store, cx| store.refresh_foreign_profile_tag(tag.clone(), cx));
            }
            ProfileRouteTarget::ForeignUserId(user_id) => {
                interactions.update(cx, |store, cx| store.refresh_foreign_profile_user_id(user_id.clone(), cx));
            }
            ProfileRouteTarget::Own | ProfileRouteTarget::Invalid => {}
        }
    }

    /// offline the error page stays, a retry has no socket
    /// dropping the flag showed an empty profile with no name and zero counters
    /// reconnecting clears it
    fn retry_foreign_load(&self, cx: &mut App) {
        let socket_dropped = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.is_socket_dropped(cx));
        self.state.update(cx, |state, cx| {
            state.foreign_load_stalled_offline = socket_dropped;
            cx.notify();
        });
        self.refresh_foreign_route(cx);
    }

    fn foreign_load_failed_page(self: &Rc<Self>) -> impl IntoElement + use<> {
        let back = self.clone();
        let retry = self.clone();
        let keys = self.clone();
        let pull = self.clone();
        ListShellUi::new().on_back(move |_, cx| back.pop(cx)).content(PageRefreshKeys::new(
            child_id(&self.id, "failed-keys"),
            move |_, cx| keys.retry_foreign_load(cx),
            scroll_view(child_id(&self.id, "failed-scroll"))
                .size_full()
                .refreshable(move |_, cx| {
                    pull.retry_foreign_load(cx);
                    Task::ready(())
                })
                .child(div().pt(px(PAGE_STATE_TOP_PADDING)).child(ChatProfileTabErrorState::new(
                    tr("profile_load_error"),
                    move |_, cx| retry.retry_foreign_load(cx),
                ))),
        ))
    }

    fn matching_foreign_payload(&self, cx: &mut App) -> Option<UserGetProfileByTagGatewayDTO> {
        match &self.route_target {
            ProfileRouteTarget::ForeignTag(tag) => self.matching_payload_by_tag(tag, cx),
            ProfileRouteTarget::ForeignUserId(user_id) => self.matching_payload_by_id(user_id, cx),
            ProfileRouteTarget::Own | ProfileRouteTarget::Invalid => None,
        }
    }

    fn closed_tabs(payload: Option<&UserGetProfileByTagGatewayDTO>) -> HashSet<ProfileContentTab> {
        let mut closed = HashSet::new();
        let Some(payload) = payload else {
            return closed;
        };
        let unblocked =
            payload.resolved_owner_block_scope().is_none_or(|scope| trimming_whitespaces_and_newlines(scope).is_empty());
        let Some(privacy) = payload.profile.as_ref().and_then(|profile| profile.privacy_settings.as_ref()) else {
            return closed;
        };
        if !unblocked {
            return closed;
        }
        if ProfileViewerPrivacy::closes(Some(&privacy.goal_rule)) {
            closed.insert(ProfileContentTab::Goals);
        }
        if ProfileViewerPrivacy::closes(Some(&privacy.plan_rule)) {
            closed.insert(ProfileContentTab::Plans);
        }
        if ProfileViewerPrivacy::closes(Some(&privacy.gifts_rule)) {
            closed.insert(ProfileContentTab::Gifts);
        }
        if ProfileViewerPrivacy::closes(Some(&privacy.reposts_rule)) {
            closed.insert(ProfileContentTab::Reposts);
        }
        closed
    }

    fn matching_payload_by_tag(&self, tag: &str, cx: &mut App) -> Option<UserGetProfileByTagGatewayDTO> {
        let cached =
            UserServicesStore::shared(cx).update(cx, |store, cx| store.cached_foreign_profile_tag(tag, cx));
        if cached.is_some() {
            return cached;
        }
        let payload = UserActionsStore::shared(cx).read(cx).get_profile_by_tag_sai.data()?;
        UserServicesStore::foreign_profile_answers(&payload, tag).then_some(payload)
    }

    fn matching_payload_by_id(&self, user_id: &str, cx: &mut App) -> Option<UserGetProfileByTagGatewayDTO> {
        let cached = UserServicesStore::shared(cx)
            .update(cx, |store, cx| store.cached_foreign_profile_user_id(user_id, None, cx));
        if cached.is_some() {
            return cached;
        }
        let payload = UserActionsStore::shared(cx).read(cx).get_profile_by_id_sai.data()?;
        payload.as_profile_model().is_some_and(|model| model.id == user_id).then_some(payload)
    }
}

/// avatars of the signed-in accounts for the account menu,
/// cached bytes first and network only when missing
pub enum AccountMenuAvatarLoader {}

impl AccountMenuAvatarLoader {
    const SIDE: u32 = 112;

    pub async fn load(accounts: Vec<AuthAccountSummary>, cx: &mut AsyncApp) -> HashMap<String, CleverImage> {
        let mut result = HashMap::new();
        for account in accounts {
            let Some(source) = AvatarPalette::resolved_logo_url(account.avatar_url.as_deref()) else {
                continue;
            };
            let user_id = account.user_id;
            let disk_key = source.clone();
            let disk_user = user_id.clone();
            let cached = cx
                .background_executor()
                .spawn(async move {
                    CleverImageDiskCache::ui_image(&disk_key, Some(&disk_user)).map(|cached| Self::circular(&cached))
                })
                .await;
            if let Some(image) = cached {
                if let Some(image) = image {
                    result.insert(user_id, image);
                }
                continue;
            }
            let is_active =
                cx.update(|cx| AuthServicesStore::shared(cx).read(cx).active_account_id() == Some(user_id.as_str()));
            let image = CleverImageBitmapLoader::load(
                &source,
                is_active,
                true,
                SaiAccountCacheScope::active_user_id(),
                SaiAccountCacheScope::active_epoch(),
                Some(ACCOUNT_MENU_AVATAR_PIXELS),
                CleverImageCachePolicy::Standard,
                CleverImageWorkLane::Visible,
                &*cx,
            )
            .await;
            let Some(image) = image else {
                continue;
            };
            let store_user = user_id.clone();
            let circular = cx
                .background_executor()
                .spawn(async move {
                    CleverImageDiskCache::store_image(&image, &source, Some(&store_user), None);
                    Self::circular(&image)
                })
                .await;
            if let Some(circular) = circular {
                result.insert(user_id, circular);
            }
        }
        result
    }

    /// letter circle in the usual color of this person, none while the account has a photo
    pub(crate) fn letter(account: &AuthAccountSummary) -> Option<CleverImage> {
        if AvatarPalette::resolved_logo_url(account.avatar_url.as_deref()).is_some() {
            return None;
        }
        let background_color = AvatarPalette::color_for_user(Some(&account.user_id), Some(&account.name));
        let ink = AvatarPalette::letter_color_on(background_color).to_rgb();
        let background = background_color.to_rgb();
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        let fill = [channel(background.r), channel(background.g), channel(background.b)];
        let ink = [channel(ink.r), channel(ink.g), channel(ink.b)];
        let letter = AvatarPalette::display_letter(Some(&account.name));
        let side = Self::SIDE as f32;
        let radius = side / 2.0;
        let mut canvas = image::RgbaImage::new(Self::SIDE, Self::SIDE);
        for (x, y, pixel) in canvas.enumerate_pixels_mut() {
            let dx = x as f32 + 0.5 - radius;
            let dy = y as f32 + 0.5 - radius;
            let coverage = (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            *pixel = image::Rgba([fill[0], fill[1], fill[2], (coverage * 255.0).round() as u8]);
        }
        if let Ok(raster) = render_text(&letter, side * 0.46, TextWeight::Semibold) {
            let is_neutral = raster.rgba.chunks_exact(4).all(|px| px[0] == px[1] && px[1] == px[2]);
            let columns = raster.width.max(1);
            let left = (i64::from(Self::SIDE) - i64::from(raster.width)) / 2;
            let top = (i64::from(Self::SIDE) - i64::from(raster.height)) / 2;
            for (index, px) in raster.rgba.chunks_exact(4).enumerate() {
                let column = i64::from(index as u32 % columns) + left;
                let row = i64::from(index as u32 / columns) + top;
                let (Ok(column), Ok(row)) = (u32::try_from(column), u32::try_from(row)) else {
                    continue;
                };
                if column >= Self::SIDE || row >= Self::SIDE {
                    continue;
                }
                let color = if is_neutral { ink } else { [px[0], px[1], px[2]] };
                let alpha = f32::from(px[3]) / 255.0;
                let target = canvas.get_pixel_mut(column, row);
                for (under, over) in target.0.iter_mut().take(3).zip(color) {
                    *under = (f32::from(*under) * (1.0 - alpha) + f32::from(over) * alpha).round() as u8;
                }
            }
        }
        for pixel in canvas.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(canvas)])))
    }

    pub fn circular(source: &RenderImage) -> Option<CleverImage> {
        let source_size = source.size(0);
        let (width, height) = (source_size.width.0 as u32, source_size.height.0 as u32);
        if width == 0 || height == 0 {
            return None;
        }
        let buffer = image::RgbaImage::from_raw(width, height, source.as_bytes(0)?.to_vec())?;
        let side = Self::SIDE as f32;
        let scale = (side / width as f32).max(side / height as f32);
        let draw_width = ((width as f32 * scale).ceil() as u32).max(1);
        let draw_height = ((height as f32 * scale).ceil() as u32).max(1);
        let scaled = image::imageops::resize(&buffer, draw_width, draw_height, image::imageops::FilterType::Triangle);
        let offset_x = (draw_width.saturating_sub(Self::SIDE)) / 2;
        let offset_y = (draw_height.saturating_sub(Self::SIDE)) / 2;

        let radius = side / 2.0;
        let mut circular = image::RgbaImage::new(Self::SIDE, Self::SIDE);
        for (x, y, pixel) in circular.enumerate_pixels_mut() {
            let dx = x as f32 + 0.5 - radius;
            let dy = y as f32 + 0.5 - radius;
            let coverage = (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let mut drawn = *scaled.get_pixel(
                (x + offset_x).min(draw_width - 1),
                (y + offset_y).min(draw_height - 1),
            );
            drawn.0[3] = (drawn.0[3] as f32 * coverage) as u8;
            *pixel = drawn;
        }
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(circular)])))
    }
}

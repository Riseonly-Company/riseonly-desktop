use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{FutureExt, LocalBoxFuture};
use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, AsyncApp, AsyncWindowContext, Context, Entity, ListAlignment, ListOffset, ListScrollEvent,
    KeystrokeEvent, ListState, RenderOnce, ScrollWheelEvent, SharedString, StyleRefinement, Subscription, Task, TextRun, WeakEntity,
    Window, div,
    list, point, prelude::*, px,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuLocksScrollKey;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::Font;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stacks::{spacer_min_h, vstack};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{RISE_ENGINE_CONNECTION_READY, RiseEngineAppRuntime};
use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsHeader;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerLiveState};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop;
use crate::app::router::navigation_types::MainTab;
use crate::core::media_viewer::media_viewer_presentation_state::media_viewer_presentation;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::page_refresh::is_page_scroll_to_top_keystroke;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::notify::components::notifications_bell_button::NotificationsBellButton;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_skeleton::{PostCardSkeletonVariant, PostFeedSkeleton};
use crate::modules::post::components::post_collection_ui::{
    PostPageRefreshErrorUi, PostPageStateUi, PostPaginationTriggerModifiers,
};
use crate::modules::post::components::post_feed_media_prefetch::PostFeedMediaPrefetchController;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostCacheSource, RisePostFeedKind, RisePostPagePresentation, RisePostPageScope, RisePostPageStatus,
    RisePostViewSurface,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_feed_native_list_view::{
    PostFeedNativeListView, PostFeedRestoredFade,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_configuration::PostListHeader;
use crate::modules::post::pages::posts::post_detail::native_list::post_list_engine_policy::PostListEnginePolicy;
use crate::modules::post::pages::posts::post_detail::list_rows_splice_hooks::splice_list_rows;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::story::components::stories_tray::StoriesTray;
use crate::modules::story::components::story_composer_host::StoryFeedComposerMount;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

const LIST_OVERDRAW: f32 = 600.0;
const STORIES_ROW_ID: &str = "feed-stories";
const BOTTOM_ROW_ID: &str = "feed-bottom";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum HomeFeedTab {
    All,
    Publications,
    Threads,
}

impl HomeFeedTab {
    const ALL_CASES: [HomeFeedTab; 3] = [HomeFeedTab::All, HomeFeedTab::Publications, HomeFeedTab::Threads];

    fn raw_value(self) -> i64 {
        match self {
            HomeFeedTab::All => 0,
            HomeFeedTab::Publications => 1,
            HomeFeedTab::Threads => 2,
        }
    }

    fn from_raw_value(raw_value: i64) -> Option<Self> {
        match raw_value {
            0 => Some(HomeFeedTab::All),
            1 => Some(HomeFeedTab::Publications),
            2 => Some(HomeFeedTab::Threads),
            _ => None,
        }
    }

    fn kind(self) -> RisePostFeedKind {
        match self {
            HomeFeedTab::All => RisePostFeedKind::All,
            HomeFeedTab::Publications => RisePostFeedKind::Publications,
            HomeFeedTab::Threads => RisePostFeedKind::Threads,
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            HomeFeedTab::All => "post_feed_tab_all",
            HomeFeedTab::Publications => "post_feed_tab_publications",
            HomeFeedTab::Threads => "post_feed_tab_threads",
        }
    }
}

pub struct Posts {
    auth: Entity<AuthServicesStore>,
    selected_tab: HomeFeedTab,
    mounted_tabs: HashSet<HomeFeedTab>,
    scroll_anchors: HashMap<HomeFeedTab, String>,
    mount_window_generation: u64,
    pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    observed_mount_anchor: i64,
    settled_drag_progress: f32,
    page_width: f32,
    center_width: Option<f32>,
    center_mid_x: Option<f32>,
    hold_context_menu: Entity<HoldContextMenuPresenter>,
    hold_menu_locked: bool,
    feed_owner_id: Uuid,
    pages: HashMap<HomeFeedTab, Entity<PostsFeedPage>>,
    scroll_keys_host: Option<(WeakEntity<NavigationStore>, AnyWindowHandle)>,
    _subscriptions: Vec<Subscription>,
}

impl Posts {
    fn scroll_to_top_from_key(posts: &Entity<Self>, window: &mut Window, cx: &mut App) -> bool {
        let Some((nav_store, host_window)) = posts.read(cx).scroll_keys_host.clone() else {
            return false;
        };
        if window.window_handle() != host_window || ScreenFocus::text_field_is_focused(window, cx) {
            return false;
        }
        let Some(nav_store) = nav_store.upgrade() else {
            return false;
        };
        let on_posts_root = {
            let nav = nav_store.read(cx);
            nav.current_tab == MainTab::Posts && nav.path.is_empty()
        };
        if !on_posts_root
            || HoldContextMenuPresenter::has_active_request(cx)
            || media_viewer_presentation(cx).read(cx).is_presented()
        {
            return false;
        }
        MainTabScrollToTop::perform(true, window, cx);
        true
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let pager_live_state = cx.new(|_| AnimatedTabsPagerLiveState::new());
        let selected_tab = HomeFeedTab::All;
        let scroll_keys_owner = cx.weak_entity();
        let subscriptions = vec![
            cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
                if !is_page_scroll_to_top_keystroke(&event.keystroke) {
                    return;
                }
                let Some(posts) = scroll_keys_owner.upgrade() else {
                    return;
                };
                if Self::scroll_to_top_from_key(&posts, window, cx) {
                    cx.stop_propagation();
                }
            }),
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&hold_context_menu, |view, menu, cx| {
                let locked = menu.read(cx).active_request().is_some();
                if locked != view.hold_menu_locked {
                    view.hold_menu_locked = locked;
                    cx.notify();
                }
            }),
            cx.observe(&pager_live_state, |view, live, cx| {
                let anchor = live.read(cx).resolved_mount_anchor(view.selected_tab.raw_value());
                if anchor != view.observed_mount_anchor {
                    view.observed_mount_anchor = anchor;
                    cx.notify();
                }
            }),
        ];
        Self {
            auth,
            selected_tab,
            mounted_tabs: HashSet::from([HomeFeedTab::All, HomeFeedTab::Publications]),
            scroll_anchors: HashMap::new(),
            mount_window_generation: 0,
            pager_live_state,
            observed_mount_anchor: selected_tab.raw_value(),
            settled_drag_progress: 0.0,
            page_width: 0.0,
            center_width: None,
            center_mid_x: None,
            hold_menu_locked: hold_context_menu.read(cx).active_request().is_some(),
            hold_context_menu,
            feed_owner_id: Uuid::new_v4(),
            pages: HashMap::new(),
            scroll_keys_host: None,
            _subscriptions: subscriptions,
        }
    }

    fn active_tab_binding(this: &Entity<Self>) -> Binding<i64> {
        Binding::entity(
            this.clone(),
            |view: &Posts| view.selected_tab.raw_value(),
            |view, raw_value, _| {
                if let Some(tab) = HomeFeedTab::from_raw_value(raw_value) {
                    view.selected_tab = tab;
                }
            },
        )
    }

    fn select_tab(&mut self, raw_value: i64, cx: &mut Context<Self>) {
        let Some(tab) = HomeFeedTab::from_raw_value(raw_value) else {
            return;
        };
        if tab == self.selected_tab {
            return;
        }
        let _ = selection_changed();
        self.selected_tab = tab;
        cx.notify();
    }

    fn mount_window(&self, tab: HomeFeedTab) -> HashSet<HomeFeedTab> {
        let index = tab.raw_value();
        [index - 1, index, index + 1].into_iter().filter_map(HomeFeedTab::from_raw_value).collect()
    }

    fn prepare_mount_window(&mut self, tab: HomeFeedTab, cx: &mut Context<Self>) {
        let window = self.mount_window(tab);
        self.mounted_tabs.extend(window);
        cx.notify();
    }

    fn schedule_mount_window_trim(&mut self, tab: HomeFeedTab, delay: f64, cx: &mut Context<Self>) {
        self.mount_window_generation = self.mount_window_generation.wrapping_add(1);
        let generation = self.mount_window_generation;
        cx.spawn(async move |this, cx| {
            if delay > 0.0 {
                cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            }
            let _ = this.update(cx, |view, cx| {
                if view.mount_window_generation != generation || view.selected_tab != tab {
                    return;
                }
                view.mounted_tabs = view.mount_window(tab);
                cx.notify();
            });
        })
        .detach();
    }

    fn scroll_anchor_binding(this: &Entity<Self>, tab: HomeFeedTab) -> Binding<Option<String>> {
        Binding::entity(
            this.clone(),
            move |view: &Posts| view.scroll_anchors.get(&tab).cloned(),
            move |view, value, _| match value {
                Some(value) => {
                    view.scroll_anchors.insert(tab, value);
                }
                None => {
                    view.scroll_anchors.remove(&tab);
                }
            },
        )
    }

    fn update_header_tap_suppression(
        live_state: &Entity<AnimatedTabsPagerLiveState>,
        suppressed: bool,
        cx: &mut App,
    ) {
        live_state.update(cx, |live, cx| {
            live.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
        });
    }

    fn push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

impl Render for Posts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.entity();
        let weak = cx.weak_entity();
        let entity_id = cx.entity_id();
        let nav_store = env::<NavigationStoreKey>(cx);
        self.scroll_keys_host = nav_store.as_ref().map(|store| (store.downgrade(), window.window_handle()));
        let selected_tab = self.selected_tab;
        let feed_owner_id = self.feed_owner_id;
        let live_state = self.pager_live_state.clone();

        on_change(
            ("posts-selected-tab", entity_id),
            selected_tab,
            false,
            window,
            cx,
            move |previous: &HomeFeedTab, tab: &HomeFeedTab, _, cx| {
                let (previous, tab) = (*previous, *tab);
                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.stop_observing_feed(previous.kind(), feed_owner_id, cx);
                    store.observe_feed(tab.kind(), feed_owner_id, cx);
                });
            },
        );

        let anchor = live_state.read(cx).resolved_mount_anchor(selected_tab.raw_value());
        let anchor_view = weak.clone();
        on_change(("posts-mount-anchor", entity_id), anchor, true, window, cx, move |_, anchor: &i64, _, cx| {
            let Some(tab) = HomeFeedTab::from_raw_value(*anchor) else {
                return;
            };
            let _ = anchor_view.update(cx, |view, cx| {
                view.prepare_mount_window(tab, cx);
                view.schedule_mount_window_trim(tab, 0.0, cx);
            });
        });

        self.pages.retain(|tab, _| self.mounted_tabs.contains(tab));
        let mut pages: Vec<AnyElement> = Vec::with_capacity(HomeFeedTab::ALL_CASES.len());
        for tab in HomeFeedTab::ALL_CASES {
            let content = if self.mounted_tabs.contains(&tab) {
                let scroll_anchor = Self::scroll_anchor_binding(&this, tab);
                let is_active = tab == selected_tab;
                let page = self
                    .pages
                    .entry(tab)
                    .or_insert_with(|| cx.new(|cx| PostsFeedPage::new(tab, scroll_anchor.clone(), is_active, cx)))
                    .clone();
                let anchor_value = self.scroll_anchors.get(&tab).cloned();
                let props_changed = page.update(cx, |page, _| {
                    page.is_active = is_active;
                    page.scroll_anchor = scroll_anchor;
                    page.posts_id = Some(entity_id);
                    page.drawn_props.as_ref() != Some(&(is_active, anchor_value))
                });
                let view = AnyView::from(page);
                if props_changed {
                    view.into_any_element()
                } else {
                    view.cached(StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
                }
            } else {
                div().into_any_element()
            };
            pages.push(div().size_full().child(content).into_any_element());
        }

        let select_view = weak.clone();
        let header = PostsFeedTabsHeader::new(
            Self::active_tab_binding(&this),
            live_state.clone(),
            Rc::new(move |raw_value: i64, _window: &mut Window, cx: &mut App| {
                let _ = select_view.update(cx, |view, cx| view.select_tab(raw_value, cx));
            }),
        );
        let bar_width = PostsFeedTabsHeader::bar_width(self.center_width, window);

        let bell_nav = nav_store.clone();
        let refresh_view = weak.clone();
        let logo_nav = nav_store.clone();
        let profile = self.auth.read(cx).profile();
        let source = profile.map(|profile| {
            SharedString::from(profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()))
        });
        let name = profile.map(|profile| SharedString::from(profile.name.clone()));
        let user_id = profile.map(|profile| SharedString::from(profile.id.clone()));

        let tap_state = live_state.clone();
        let pager = AnimatedTabsPager::new(
            ("posts-pager", entity_id),
            HomeFeedTab::ALL_CASES.len() as i64,
            Self::active_tab_binding(&this),
            Binding::entity(
                this.clone(),
                |view: &Posts| view.settled_drag_progress,
                |view, value, _| view.settled_drag_progress = value,
            ),
            Binding::entity(this.clone(), |view: &Posts| view.page_width, |view, value, _| view.page_width = value),
        )
        .live_drag_state(Some(live_state))
        .yields_first_tab_back_swipe_to_navigation(true)
        .drag_locked(self.hold_context_menu.read(cx).active_request().is_some())
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed: bool, _window: &mut Window, cx: &mut App| {
            Self::update_header_tap_suppression(&tap_state, suppressed, cx);
        })))
        .children(pages);

        let appear_view = weak.clone();
        let center_view = weak.clone();
        let wheel_view = weak.clone();
        let feed_width = f32::from(theme.feed.content_width);
        ListShellUi::new()
            .center_slot(div().w(px(bar_width)).child(header))
            .center_slot_self_styled(true)
            .shows_back_button(false)
            .right_jsx(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .child(NotificationsBellButton::new(("posts-notifications-bell", entity_id), move |_, cx| {
                        Self::push(&bell_nav, RootRoute::Notifications, cx);
                    })),
            )
            .left_jsx(
                div().id(("posts-user-logo-label", entity_id)).tooltip_label(tr("more_tab_profile")).child(
                    UserLogoUi::new(("posts-user-logo", entity_id))
                        .source(source)
                        .name(name)
                        .user_id(user_id)
                        .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
                        .is_button(true)
                        .show_border(false)
                        .show_story_ring(false)
                        .open_story_on_press(false)
                        .on_press(move |_, cx| Self::push(&logo_nav, RootRoute::MyProfile, cx)),
                ),
            )
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_blurred_header()))
            .content(
                PageRefreshKeys::new(
                    ("posts-refresh-keys", entity_id),
                    move |window, cx| {
                        if MainTabScrollToTop::perform(true, window, cx) {
                            return;
                        }
                        let _ = refresh_view.update(cx, |view, cx| {
                            PostsFeedPage::refresh_feed_and_stories(view.selected_tab, cx).detach();
                        });
                    },
                    div()
                        .relative()
                        .size_full()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .child(div().absolute().top_0().left_0().size_full().on_scroll_wheel(
                            move |event: &ScrollWheelEvent, window, cx| {
                                let Some(mid_x) = wheel_view.upgrade().and_then(|view| view.read(cx).center_mid_x) else {
                                    return;
                                };
                                if (f32::from(event.position.x) - mid_x).abs() <= feed_width / 2.0 {
                                    return;
                                }
                                MainTabScrollToTop::forward_wheel(point(px(mid_x), event.position.y), event, window, cx);
                            },
                        ))
                        .child(div().h_full().w_full().max_w(theme.feed.content_width).child(pager))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_0()
                                .child(StoryFeedComposerMount::new(("posts-story-composer", entity_id))),
                        )
                        .child(measure(("posts-center-width", entity_id), move |bounds, window, cx| {
                            let width = f32::from(bounds.size.width);
                            let mid_x = f32::from(bounds.center().x);
                            let _ = center_view.update(cx, |view, cx| {
                                view.center_mid_x = Some(mid_x);
                                if view.center_width == Some(width) {
                                    return;
                                }
                                let before = PostsFeedTabsHeader::bar_width(view.center_width, window);
                                view.center_width = Some(width);
                                if (PostsFeedTabsHeader::bar_width(Some(width), window) - before).abs() > 0.5 {
                                    cx.notify();
                                }
                            });
                        })),
                )
                .on_scroll_to_top(|window, cx| {
                    MainTabScrollToTop::perform(true, window, cx);
                }),
            )
            .on_appear(("posts-root", entity_id), move |_, cx| {
                let _ = appear_view.update(cx, |view, cx| {
                    view.mounted_tabs = view.mount_window(view.selected_tab);
                    cx.notify();
                });
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.observe_feed(selected_tab.kind(), feed_owner_id, cx));
            })
            .and_on_disappear(move |_, cx| {
                let _ = weak.update(cx, |view, _| {
                    view.mount_window_generation = view.mount_window_generation.wrapping_add(1);
                });
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.stop_observing_feed(selected_tab.kind(), feed_owner_id, cx));
            })
    }
}

#[derive(IntoElement)]
struct PostsFeedTabsHeader {
    selected_tab: Binding<i64>,
    live_state: Entity<AnimatedTabsPagerLiveState>,
    on_select: Rc<dyn Fn(i64, &mut Window, &mut App)>,
}

impl PostsFeedTabsHeader {
    fn new(
        selected_tab: Binding<i64>,
        live_state: Entity<AnimatedTabsPagerLiveState>,
        on_select: Rc<dyn Fn(i64, &mut Window, &mut App)>,
    ) -> Self {
        Self { selected_tab, live_state, on_select }
    }

    /// width of the titles in this language, capped by the room between header buttons
    /// past the cap the strip scrolls
    /// first frame uses natural width
    fn bar_width(center_width: Option<f32>, window: &Window) -> f32 {
        let font = Font::system_weight(f32::from(AnimatedTabsMetrics::TAB_FONT_SIZE), gpui::FontWeight::SEMIBOLD);
        let horizontal_padding = f32::from(AnimatedTabsMetrics::TAB_HORIZONTAL_PADDING);
        let labels = HomeFeedTab::ALL_CASES.iter().fold(0.0_f32, |sum, tab| {
            let text = SharedString::from(tr(tab.title_key()));
            let run = TextRun {
                len: text.len(),
                font: gpui::Font {
                    family: font.family(),
                    features: font.features(),
                    fallbacks: None,
                    weight: font.weight,
                    style: font.style(),
                },
                color: gpui::Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let width = f32::from(window.text_system().shape_line(text, px(font.size), &[run], None).width());
            sum + width.ceil() + horizontal_padding * 2.0
        });
        let natural = labels + f32::from(AnimatedTabsMetrics::BAR_INNER_PADDING) * 2.0;
        let side = 8.0 + PageHeaderMetrics::TOOLBAR_BUTTON_SIZE + 4.0;
        center_width.map_or(natural, |width| natural.min(width - 2.0 * side)).max(0.0)
    }
}

impl RenderOnce for PostsFeedTabsHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tabs = HomeFeedTab::ALL_CASES
            .iter()
            .map(|tab| TabHeaderConfig::new(tab.raw_value(), tr(tab.title_key())))
            .collect();
        let progress_state = self.live_state.clone();
        let tap_state = self.live_state;
        AnimatedTabsHeader::new(
            "posts-feed-tabs-header",
            tabs,
            self.selected_tab,
            Rc::new(move |cx: &App| progress_state.read(cx).drag_progress()),
            0.0,
            self.on_select,
        )
        .suppresses_tap(Rc::new(move |cx: &App| tap_state.read(cx).suppresses_tab_taps()))
        .force_scrollable(true)
    }
}

struct StoriesHeaderRow;

impl Render for StoriesHeaderRow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        StoriesTray::new(STORIES_ROW_ID).guide_anchor(RiseGuideAnchor::HomeStoriesRow)
    }
}

struct PostsFeedPage {
    tab: HomeFeedTab,
    scroll_anchor: Binding<Option<String>>,
    is_active: bool,
    hold_context_menu: Entity<HoldContextMenuPresenter>,
    hold_menu_locked: bool,
    media_prefetch: Rc<std::cell::RefCell<PostFeedMediaPrefetchController>>,
    native_feed_mount: u64,
    refresh_after_remount: Rc<std::cell::Cell<bool>>,
    fallback_list: ListState,
    rendered_row_ids: Vec<String>,
    did_restore_scroll_anchor: bool,
    geometry_height: f32,
    held_page: Option<RisePostPagePresentation>,
    last_live_page: Option<RisePostPagePresentation>,
    drawn_page: Option<RisePostPagePresentation>,
    drawn_props: Option<(bool, Option<String>)>,
    posts_id: Option<gpui::EntityId>,
    _subscriptions: Vec<Subscription>,
    _connection_ready: RiseSubscription,
    _delivery: Task<()>,
}

impl PostsFeedPage {
    fn new(
        tab: HomeFeedTab,
        scroll_anchor: Binding<Option<String>>,
        is_active: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let service = PostServicesStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let connection_ready = RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
            let _ = sender.unbounded_send(());
        });
        let delivery = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                if this.upgrade().is_none() {
                    return;
                }
                Self::load_visible_stories_when_connected(this.clone(), cx).await;
            }
        });
        let fallback_list = ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW));
        let scroll_view_entity = cx.weak_entity();
        fallback_list.set_scroll_handler(move |event: &ListScrollEvent, window: &mut Window, cx: &mut App| {
            let first_visible = event.visible_range.start;
            let _ = scroll_view_entity.update(cx, |page, cx| {
                let value = page
                    .rendered_row_ids
                    .get(first_visible)
                    .filter(|id| first_visible > 0 && id.as_str() != STORIES_ROW_ID && id.as_str() != BOTTOM_ROW_ID)
                    .cloned();
                if page.scroll_anchor.get(cx) != value {
                    page.scroll_anchor.set(value, window, cx);
                }
            });
        });
        Self {
            tab,
            scroll_anchor,
            is_active,
            media_prefetch: Rc::new(std::cell::RefCell::new(PostFeedMediaPrefetchController::new(
                &Uuid::new_v4().uuid_string(),
            ))),
            native_feed_mount: 0,
            refresh_after_remount: Rc::new(std::cell::Cell::new(false)),
            fallback_list,
            rendered_row_ids: Vec::new(),
            did_restore_scroll_anchor: false,
            geometry_height: 0.0,
            held_page: None,
            last_live_page: None,
            drawn_page: None,
            drawn_props: None,
            posts_id: None,
            _subscriptions: vec![
                cx.observe(&service, |view, service, cx| {
                    let kind = view.tab.kind();
                    let next = service.update(cx, |service, cx| service.feed_page(kind, cx));
                    if view.drawn_page.as_ref() != Some(&next) {
                        cx.notify();
                    }
                }),
                cx.observe(&hold_context_menu, |view, menu, cx| {
                    let locked = menu.read(cx).active_request().is_some();
                    if locked != view.hold_menu_locked {
                        view.hold_menu_locked = locked;
                        cx.notify();
                    }
                }),
            ],
            hold_menu_locked: hold_context_menu.read(cx).active_request().is_some(),
            hold_context_menu,
            _connection_ready: connection_ready,
            _delivery: delivery,
        }
    }

    async fn load_visible_stories_when_connected(this: WeakEntity<Self>, cx: &mut AsyncApp) {
        let Ok(Some(ready)) = this.update(cx, |page, cx| {
            if !page.is_active || page.tab != HomeFeedTab::All {
                return None;
            }
            let account_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()?;
            Some(RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.ready_connection(&account_id, cx)))
        }) else {
            return;
        };
        if ready.await.is_none() {
            return;
        }
        let _ = this.update(cx, |page, cx| {
            if !page.is_active {
                return;
            }
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_stories_feed(cx));
        });
    }

    fn hold_restored_rows_if_reader_scrolled(&mut self, next: RisePostPagePresentation, cx: &mut Context<Self>) {
        let previous = self.last_live_page.replace(next.clone());
        if self.held_page.is_some() && (next.cache_source != RisePostCacheSource::Network || next.items.is_empty()) {
            self.held_page = None;
            cx.notify();
            return;
        }
        let Some(previous) = previous else {
            return;
        };
        if self.held_page.is_some() || !PostFeedRestoredFade::replaces_restored_rows(&previous, &next) {
            return;
        }
        let Some(anchor) = self.scroll_anchor.get(cx) else {
            return;
        };
        if !previous.items.iter().any(|item| item.presentation_id() == anchor)
            || next.items.iter().any(|item| item.presentation_id() == anchor)
        {
            return;
        }
        self.held_page = Some(RisePostPagePresentation {
            items: previous.items,
            status: RisePostPageStatus::Loaded,
            error_message: None,
            has_more: false,
            is_refreshing: false,
            is_loading_more: false,
            cache_source: previous.cache_source,
            revision: previous.revision,
        });
        cx.notify();
    }

    /// clears the held page before a pull or refresh so the fresh feed takes its place
    fn release_held_page(weak: &WeakEntity<Self>, cx: &mut App) {
        let weak = weak.clone();
        cx.defer(move |cx| {
            let _ = weak.update(cx, |page, cx| {
                if page.held_page.take().is_some() {
                    cx.notify();
                }
            });
        });
    }

    fn refresh_feed_and_stories(tab: HomeFeedTab, cx: &mut App) -> Task<()> {
        let refresh = PostInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_feed_kind(tab.kind(), cx));
        cx.spawn(async move |cx: &mut AsyncApp| {
            refresh.await;
            if tab == HomeFeedTab::All {
                cx.update(|cx| StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_stories_feed(cx)));
            }
        })
    }

    fn native_feed(&self, page: RisePostPagePresentation, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.tab;
        let kind = tab.kind();
        let is_active = self.is_active;
        let weak = cx.weak_entity();
        let scroll_anchor = self.scroll_anchor.clone();
        let window_handle = window.window_handle();
        let refresh_view = weak.clone();
        let refresh: Rc<dyn Fn(&mut App) -> LocalBoxFuture<'static, ()>> = Rc::new(move |cx: &mut App| {
            Self::release_held_page(&refresh_view, cx);
            let task = Self::refresh_feed_and_stories(tab, cx);
            async move { task.await }.boxed_local()
        });
        let view = PostFeedNativeListView::new(
            SharedString::from(format!("post-feed-native-list-{}-{}", tab.raw_value(), self.native_feed_mount)),
            RisePostPageScope::Feed(kind),
            page,
            RisePostViewSurface::Feed,
            self.scroll_anchor.clone(),
            move |cx| {
                if is_active {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_feed_kind(kind, cx));
                }
            },
            move |cx| {
                if is_active {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.activate_feed(kind, cx));
                }
            },
        )
        .header((tab == HomeFeedTab::All).then(Self::stories_header))
        .top_padding(5.0)
        .bottom_inset(main_tab_bar_underlap(cx).max(80.0))
        .is_active(is_active)
        .refresh(refresh)
        .on_retry(move |cx| Self::refresh(kind, cx))
        .error_text(tr("feed_load_error"))
        .refresh_after_remount(self.refresh_after_remount.clone())
        .remount_at_top(move |cx| {
            let scroll_anchor = scroll_anchor.clone();
            let weak = weak.clone();
            cx.defer(move |cx| {
                let _ = window_handle.update(cx, |_, window, cx| scroll_anchor.set(None, window, cx));
                let _ = weak.update(cx, |page, cx| {
                    page.native_feed_mount = page.native_feed_mount.wrapping_add(1);
                    cx.notify();
                });
            });
        });
        view.into_any_element()
    }

    fn stories_header() -> PostListHeader {
        PostListHeader {
            id: "feed.header.stories".to_owned(),
            height: 102.0,
            content: Rc::new(|_: &mut Window, cx: &mut App| AnyView::from(cx.new(|_| StoriesHeaderRow))),
        }
    }

    fn swiftui_feed(&mut self, page: RisePostPagePresentation, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.hold_context_menu.read(cx).active_request().is_some();
        let body = if page.items.is_empty() { self.state_feed(&page, cx) } else { self.fallback_feed(&page, cx) };
        let entity_id = cx.entity_id();
        let measured = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .child(env_scope::<HoldContextMenuLocksScrollKey>(locked).child(body))
            .children(locked.then(|| div().absolute().inset_0().occlude()))
            .child(measure(("posts-feed-geometry", entity_id), move |bounds, _, cx| {
                let _ = measured.update(cx, |page, cx| {
                    let height = f32::from(bounds.size.height);
                    if page.geometry_height != height {
                        page.geometry_height = height;
                        cx.notify();
                    }
                });
            }))
            .into_any_element()
    }

    fn state_feed(&self, page: &RisePostPagePresentation, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.tab;
        let kind = tab.kind();
        let entity_id = cx.entity_id();
        let tabbed_bottom = main_tab_bar_underlap(cx).max(80.0);
        let min_height = (self.geometry_height - if tab == HomeFeedTab::All { 102.0 } else { 20.0 }).max(280.0);
        let top_inset = list_shell_floating_top_inset(cx);
        let skeleton_variant = Self::skeleton_variant(tab);
        let refresh_view = cx.weak_entity();
        scroll_view(("posts-feed-scroll", entity_id))
            .shows_indicators(false)
            .refreshable(move |_, cx| {
                Self::release_held_page(&refresh_view, cx);
                Self::refresh_feed_and_stories(tab, cx)
            })
            .child(
                vstack(5.0)
                    .w_full()
                    .pt(px(top_inset + 5.0))
                    .children(
                        (tab == HomeFeedTab::All).then(|| {
                            StoriesTray::new(STORIES_ROW_ID).guide_anchor(RiseGuideAnchor::HomeStoriesRow)
                        }),
                    )
                    .child(
                        div().w_full().min_h(px(min_height)).child(
                            PostPageStateUi::new(
                                ("posts-feed-state", entity_id),
                                page.status,
                                !page.items.is_empty(),
                                Self::empty_icon(tab),
                                tr(Self::empty_text_key(tab)),
                                tr("feed_load_error"),
                                move |cx| Self::refresh(kind, cx),
                            )
                            .loading(move |_, _| {
                                PostFeedSkeleton::default()
                                    .card_count(5)
                                    .top_padding(0.0)
                                    .fixed_variant(skeleton_variant)
                            }),
                        ),
                    )
                    .child(spacer_min_h(tabbed_bottom)),
            )
            .into_any_element()
    }

    fn fallback_feed(&mut self, page: &RisePostPagePresentation, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.tab;
        let kind = tab.kind();
        let is_all = tab == HomeFeedTab::All;
        let is_active = self.is_active;
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let base = usize::from(is_all);
        let items: Rc<Vec<GetPostFeedResponse>> = Rc::new(page.items.clone());
        let mut row_ids = Vec::with_capacity(items.len() + base + 1);
        if is_all {
            row_ids.push(STORIES_ROW_ID.to_owned());
        }
        row_ids.extend(items.iter().map(GetPostFeedResponse::presentation_id));
        row_ids.push(BOTTOM_ROW_ID.to_owned());
        splice_list_rows(&self.fallback_list, &mut self.rendered_row_ids, &row_ids);

        if !self.did_restore_scroll_anchor {
            self.did_restore_scroll_anchor = true;
            if let Some(anchor) = self.scroll_anchor.get(cx)
                && let Some(item_ix) = row_ids.iter().position(|id| *id == anchor)
            {
                self.fallback_list.scroll_to(ListOffset { item_ix, offset_in_item: px(0.0) });
            }
        }

        let has_more = page.has_more;
        let is_loading_more = page.is_loading_more;
        let bottom = main_tab_bar_underlap(cx).max(80.0);
        let prefetch = self.media_prefetch.clone();
        let prefetch_page_id = Self::prefetch_page_id(tab);
        let row_items = items.clone();
        let row_theme = theme;
        let rows = list(self.fallback_list.clone(), move |index, _window, _cx| {
            if is_all && index == 0 {
                return div()
                    .w_full()
                    .pt(px(5.0))
                    .pb(px(5.0))
                    .child(StoriesTray::new(STORIES_ROW_ID).guide_anchor(RiseGuideAnchor::HomeStoriesRow))
                    .into_any_element();
            }
            let Some(post) = index.checked_sub(base).and_then(|offset| row_items.get(offset)) else {
                return div().w_full().h(px(bottom)).into_any_element();
            };
            let offset = index - base;
            let presentation_id = post.presentation_id();
            let appear_items = row_items.clone();
            let appear_prefetch = prefetch.clone();
            let appear_page_id = prefetch_page_id.clone();
            div()
                .w_full()
                .when(index == 0, |row| row.pt(px(5.0)))
                .pb(px(5.0))
                .child(
                    Self::post_row(post.clone(), &row_theme, nav_store.clone())
                        .post_pagination_trigger(
                            SharedString::from(format!("posts-feed-page-{presentation_id}")),
                            offset as i64,
                            row_items.len() as i64,
                            has_more,
                            is_loading_more,
                            move |cx| {
                                if is_active {
                                    PostInteractionsStore::shared(cx)
                                        .update(cx, |store, cx| store.load_more_feed_kind(kind, cx));
                                }
                            },
                        )
                        .on_appear(SharedString::from(format!("posts-feed-appear-{presentation_id}")), move |_, cx| {
                            appear_prefetch.borrow_mut().update(&appear_page_id, offset as i64, &appear_items, cx);
                        }),
                )
                .into_any_element()
        })
        .size_full()
        .list_shell_scroll_under_chrome(cx);

        let entity_id = cx.entity_id();
        let pill_top = list_shell_floating_top_inset(cx) + 8.0;
        div()
            .relative()
            .size_full()
            .child(rows)
            .children((page.status == RisePostPageStatus::Failed).then(|| {
                div().absolute().top_0().left_0().right_0().flex().justify_center().pt(px(pill_top)).child(
                    PostPageRefreshErrorUi::new(("posts-feed-refresh-error", entity_id), tr("feed_load_error"), move |cx| {
                        Self::refresh(kind, cx)
                    }),
                )
            }))
            .into_any_element()
    }

    /// keyed by pager page not tab, all three feeds stay
    /// mounted so each prefetch must be replaced by its own
    fn prefetch_page_id(tab: HomeFeedTab) -> String {
        format!("tab:{}", tab.raw_value())
    }

    fn empty_icon(tab: HomeFeedTab) -> &'static str {
        match tab {
            HomeFeedTab::All => "sparkles",
            HomeFeedTab::Publications => "photo.on.rectangle.angled",
            HomeFeedTab::Threads => "text.bubble",
        }
    }

    fn empty_text_key(tab: HomeFeedTab) -> &'static str {
        match tab {
            HomeFeedTab::All => "post_feed_empty_all",
            HomeFeedTab::Publications => "post_feed_empty_publications",
            HomeFeedTab::Threads => "post_feed_empty_threads",
        }
    }

    fn skeleton_variant(tab: HomeFeedTab) -> Option<PostCardSkeletonVariant> {
        match tab {
            HomeFeedTab::All => None,
            HomeFeedTab::Publications => Some(PostCardSkeletonVariant::Media { aspect: PostMediaBox::PLACEHOLDER_ASPECT }),
            HomeFeedTab::Threads => Some(PostCardSkeletonVariant::Text),
        }
    }

    fn refresh(kind: RisePostFeedKind, cx: &mut App) {
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_feed_kind(kind, cx)).detach();
    }

    fn post_row(post: GetPostFeedResponse, theme: &AppTheme, nav_store: Option<Entity<NavigationStore>>) -> gpui::Div {
        let post = Rc::new(post);
        let comments_post = post.clone();
        let avatar_post = post.clone();
        let like_post = post.clone();
        let fav_post = post.clone();
        vstack(0.0)
            .w_full()
            .child(
                Post::new(SharedString::from(post.presentation_id()), post)
                    .feed_row(true)
                    .on_open_comments(move |_, cx| {
                        PostInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
                    })
                    .on_avatar_tap(move |_, cx| {
                        let route = RootRoute::UserPage {
                            user_id: avatar_post.author.id.clone(),
                            tag: Some(avatar_post.author.tag.clone()),
                        };
                        Posts::push(&nav_store, route, cx);
                    })
                    .on_like(move |_, cx| {
                        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx));
                    })
                    .on_fav(move |_, cx| {
                        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&fav_post, cx));
                    }),
            )
            .child(div().w_full().h(px(1.0)).bg(theme.border._100))
    }
}

impl Render for PostsFeedPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        let kind = tab.kind();
        let is_active = self.is_active;
        let entity_id = cx.entity_id();
        let weak = cx.weak_entity();
        let live_page = PostServicesStore::shared(cx).update(cx, |service, cx| service.feed_page(kind, cx));
        self.drawn_page = Some(live_page.clone());
        let page = self.held_page.clone().unwrap_or_else(|| live_page.clone());
        let anchor_value = self.scroll_anchor.get(cx);
        self.drawn_props = Some((is_active, anchor_value.clone()));
        if let Some(posts_id) = self.posts_id {
            gpui::hear_entity_through_events_only(posts_id);
        }

        let mark_view = weak.clone();
        on_change(
            ("posts-feed-live-mark", entity_id),
            PostsFeedLiveMark::new(&live_page),
            true,
            window,
            cx,
            move |_, _, _, cx| {
                let _ = mark_view.update(cx, |view, cx| view.hold_restored_rows_if_reader_scrolled(live_page, cx));
            },
        );
        let anchor_view = weak.clone();
        on_change(
            ("posts-feed-held-anchor", entity_id),
            anchor_value,
            false,
            window,
            cx,
            move |_, anchor: &Option<String>, _, cx| {
                if anchor.is_some() {
                    return;
                }
                let _ = anchor_view.update(cx, |view, cx| {
                    if view.held_page.take().is_some() {
                        cx.notify();
                    }
                });
            },
        );

        use_task(
            ("posts-feed-page-task", entity_id),
            is_active,
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                if !is_active {
                    return;
                }
                let _ = cx.update(|_, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.activate_feed(kind, cx));
                    if tab == HomeFeedTab::All {
                        StoryInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.restore_stories_feed_from_cache(cx));
                    }
                });
                Self::load_visible_stories_when_connected(weak, cx).await;
            },
        );

        let body = if PostListEnginePolicy::uses_native_list() && !page.items.is_empty() {
            self.native_feed(page, window, cx)
        } else {
            self.swiftui_feed(page, cx)
        };
        let prefetch = self.media_prefetch.clone();
        div()
            .size_full()
            .child(body)
            .on_disappear(("posts-feed-page", entity_id), move |_, cx| prefetch.borrow_mut().cancel_all(cx))
    }
}

#[derive(Clone, PartialEq)]
struct PostsFeedLiveMark {
    revision: i64,
    cache_source: RisePostCacheSource,
    count: usize,
    first_id: Option<String>,
}

impl PostsFeedLiveMark {
    fn new(page: &RisePostPagePresentation) -> Self {
        Self {
            revision: page.revision,
            cache_source: page.cache_source,
            count: page.items.len(),
            first_id: page.items.first().map(GetPostFeedResponse::presentation_id),
        }
    }
}

use gpui::{AnyView, App, Context, ElementId, Entity, Hsla, SharedString, StyleRefinement, Task, Window};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::animated_tabs_ui::animated_tabs_types::TabConfig;

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::ui::profile_grid_date_scrubber::ProfileGridDateScrubberLayer;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::story::components::stories_grid::stories_grid_action_bar::{
    StoriesGridActionBar, StoriesGridDoneButton,
};
use crate::modules::story::components::stories_grid::stories_grid_cell::StoriesGridMetrics;
use crate::modules::story::components::stories_grid::stories_grid_page::StoriesGridPage;
use crate::modules::story::pages::stories_archive::highlights_archive_grid::HighlightsArchiveGrid;
use crate::modules::story::shared::story_grid_selection::StoryGridSelection;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileInnerScrollIntrospector, ProfileScrollCoordinator,
};

const STORIES_TAB: i64 = 0;
const HIGHLIGHTS_TAB: i64 = 1;
const SCRUBBER_TAB_KEY: i64 = 0;
const TOP_SPACER: f32 = 8.0;
const MIN_BOTTOM_INSET: f32 = 96.0;
const BOTTOM_EXTRA: f32 = 24.0;
const ACTION_BAR_MIN_BOTTOM: f32 = 24.0;

/// archive with two tabs over what the profile no longer shows
/// stories tab holds hidden or expired stories
/// restoring puts a story back in the grid by date
#[derive(IntoElement, Default)]
pub struct StoriesArchive;

impl StoriesArchive {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for StoriesArchive {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let owner_user_id =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        let selection = StoryGridSelection::shared(cx);
        let is_selecting = selection.read(cx).is_active_for(&owner_user_id, StoryProfileScope::Archive);
        let nav_store = env::<NavigationStoreKey>(cx);

        let shell = SettingsWrapperUi::new(tr("archive_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |store, cx| store.pop(cx));
            }
        });
        let shell = if is_selecting { shell.right_jsx(StoriesGridDoneButton) } else { shell };
        shell
            .content(StoriesArchiveTabs { owner_user_id })
            .on_disappear("stories-archive-lifecycle", move |_, cx| {
                selection.update(cx, |selection, cx| selection.end(cx));
            })
    }
}

#[derive(IntoElement)]
struct StoriesArchiveTabs {
    owner_user_id: String,
}

impl RenderOnce for StoriesArchiveTabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let props = StoriesArchiveTabsProps {
            owner_user_id: self.owner_user_id,
            top_inset: list_shell_floating_top_inset(cx),
            tab_underlap: main_tab_bar_underlap(cx),
            background: rise_ui::theme(cx).bg._100,
        };
        let view = window
            .use_keyed_state(ElementId::Name(SharedString::new_static("stories-archive-tabs-view")), cx, |_, cx| {
                let view = cx.new(|_| StoriesArchiveTabsView { props: None, drawn: false });
                StoriesArchiveTabsHolder { view }
            })
            .read(cx)
            .view
            .clone();
        let stale = view.update(cx, |view, _| view.set_props(props));
        if stale {
            AnyView::from(view).into_any_element()
        } else {
            AnyView::from(view).cached(StyleRefinement::default().size_full()).into_any_element()
        }
    }
}

#[derive(Clone, PartialEq)]
struct StoriesArchiveTabsProps {
    owner_user_id: String,
    top_inset: f32,
    tab_underlap: f32,
    background: Hsla,
}

/// the tabs as their own cached view, props by value
struct StoriesArchiveTabsView {
    props: Option<StoriesArchiveTabsProps>,
    drawn: bool,
}

impl StoriesArchiveTabsView {
    fn set_props(&mut self, props: StoriesArchiveTabsProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        self.props = Some(props);
        let drawn_before = self.drawn;
        self.drawn = true;
        drawn_before
    }
}

struct StoriesArchiveTabsHolder {
    view: Entity<StoriesArchiveTabsView>,
}

impl Render for StoriesArchiveTabsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.props.clone().map(|props| stories_archive_tabs_body(props, window, cx));
        div().size_full().children(body)
    }
}

fn stories_archive_tabs_body(props: StoriesArchiveTabsProps, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let StoriesArchiveTabsProps { owner_user_id, top_inset, tab_underlap, background } = props;
    let selection = StoryGridSelection::shared(cx);
    let is_selecting = selection.read(cx).is_active_for(&owner_user_id, StoryProfileScope::Archive);
    let active_tab = window.use_keyed_state("stories-archive-active-tab", cx, |_, _| STORIES_TAB);
    let active = *active_tab.read(cx);

    let change_owner = owner_user_id.clone();
    on_change("stories-archive-tab-change", active, false, window, cx, move |_, _, _, cx| {
        let selection = StoryGridSelection::shared(cx);
        if selection.read(cx).is_active_for(&change_owner, StoryProfileScope::Archive) {
            selection.update(cx, |selection, cx| selection.end(cx));
        }
    });

    let stories_owner = owner_user_id.clone();
    let highlights_owner = owner_user_id.clone();
    let binding = Binding::entity(active_tab, |tab: &i64| *tab, |tab, value, _| *tab = value);
    let tabs = AnimatedTabs::new(
        "stories-archive-tabs",
        vec![
            TabConfig::new(STORIES_TAB, tr("archive_tab_stories"), move |_, _| {
                StoriesArchiveStoriesTab { owner_user_id: stories_owner.clone(), tab_underlap, background }
                    .into_any_element()
            }),
            TabConfig::new(HIGHLIGHTS_TAB, tr("archive_tab_highlights"), move |_, _| {
                StoriesArchiveHighlightsTab { owner_user_id: highlights_owner.clone(), tab_underlap, background }
                    .into_any_element()
            }),
        ],
        binding,
    )
    .horizontal_padding(10.0);

    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .pt(px(top_inset))
        .bg(background)
        .child(div().flex_1().min_h_0().w_full().child(tabs))
        .when(is_selecting && active == STORIES_TAB, |root| {
            root.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .pb(px(ACTION_BAR_MIN_BOTTOM.max(tab_underlap)))
                    .flex()
                    .justify_center()
                    .child(StoriesGridActionBar::new(owner_user_id, StoryProfileScope::Archive)),
            )
        })
}

#[derive(IntoElement)]
struct StoriesArchiveStoriesTab {
    owner_user_id: String,
    tab_underlap: f32,
    background: Hsla,
}

impl RenderOnce for StoriesArchiveStoriesTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let props = StoriesArchiveStoriesProps {
            owner_user_id: self.owner_user_id,
            bottom_inset: MIN_BOTTOM_INSET.max(self.tab_underlap) + BOTTOM_EXTRA,
            background: self.background,
        };
        let view = window
            .use_keyed_state(ElementId::Name(SharedString::new_static("stories-archive-stories-view")), cx, |_, cx| {
                let view = cx.new(|_| StoriesArchiveStoriesView { props: None, drawn: false });
                StoriesArchiveStoriesHolder { view }
            })
            .read(cx)
            .view
            .clone();
        let stale = view.update(cx, |view, _| view.set_props(props));
        if stale {
            AnyView::from(view).into_any_element()
        } else {
            AnyView::from(view).cached(StyleRefinement::default().size_full()).into_any_element()
        }
    }
}

#[derive(Clone, PartialEq)]
struct StoriesArchiveStoriesProps {
    owner_user_id: String,
    bottom_inset: f32,
    background: Hsla,
}

/// stories tab body as its own cached view, scroll view and scrubber notify the view that drew them
struct StoriesArchiveStoriesView {
    props: Option<StoriesArchiveStoriesProps>,
    drawn: bool,
}

impl StoriesArchiveStoriesView {
    fn set_props(&mut self, props: StoriesArchiveStoriesProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        self.props = Some(props);
        let drawn_before = self.drawn;
        self.drawn = true;
        drawn_before
    }
}

struct StoriesArchiveStoriesHolder {
    view: Entity<StoriesArchiveStoriesView>,
}

impl Render for StoriesArchiveStoriesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.props.clone().map(|props| stories_archive_stories_body(props, window, cx));
        div().size_full().children(body)
    }
}

fn stories_archive_stories_body(
    props: StoriesArchiveStoriesProps,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let StoriesArchiveStoriesProps { owner_user_id, bottom_inset, background } = props;
    let scroll_coordinator =
        window.use_keyed_state("stories-archive-scroll-coordinator", cx, |_, cx| ProfileScrollCoordinator::new(cx));
    let timestamps: Vec<TimeInterval> = StoryServicesStore::shared(cx)
        .update(cx, |service, cx| service.profile_stories(&owner_user_id, StoryProfileScope::Archive, cx))
        .iter()
        .map(|story| story.created_at_ms as TimeInterval / 1_000.0)
        .collect();
    let page_scroll = scroll_coordinator.read(cx).page_scroll_view(SCRUBBER_TAB_KEY);

    let refresh_owner = owner_user_id.clone();
    let appear_coordinator = scroll_coordinator.clone();
    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .child(
            scroll_view("stories-archive-stories-scroll")
                .scroll_handle(page_scroll.handle())
                .size_full()
                .bg(background)
                .refreshable(move |_, cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.refresh_profile_stories(Some(&refresh_owner), StoryProfileScope::Archive, cx);
                    });
                    Task::ready(())
                })
                .child(
                    div()
                        .relative()
                        .w_full()
                        .child(ProfileInnerScrollIntrospector::new_key(
                            SCRUBBER_TAB_KEY,
                            scroll_coordinator.clone(),
                        ))
                        .child(
                            vstack(0.0)
                                .w_full()
                                .child(div().w_full().h(px(TOP_SPACER)))
                                .child(
                                    StoriesGridPage::new(owner_user_id, true)
                                        .scope(StoryProfileScope::Archive)
                                        .empty_icon("archivebox")
                                        .empty_text(tr("stories_archive_empty")),
                                )
                                .child(div().w_full().h(px(bottom_inset))),
                        ),
                ),
        )
        .child(
            div().absolute().top_0().right_0().bottom_0().w(px(ProfileGridDateScrubberLayer::LAYER_WIDTH)).child(
                ProfileGridDateScrubberLayer::new(
                    "stories-archive-scrubber",
                    timestamps,
                    StoriesGridMetrics::COLUMNS,
                    StoriesGridMetrics::CELL_ASPECT,
                    StoriesGridMetrics::SPACING,
                    StoriesGridMetrics::HORIZONTAL_PADDING,
                    SCRUBBER_TAB_KEY,
                    scroll_coordinator,
                    0.0,
                    bottom_inset,
                )
                .columns_for_width(StoriesGridMetrics::columns),
            ),
        )
        .on_appear("stories-archive-stories-tab", move |_, cx| {
            appear_coordinator.update(cx, |coordinator, cx| {
                coordinator.set_visible_horizontal_tab_keys(vec![SCRUBBER_TAB_KEY], cx);
                coordinator.set_active_tab_key(SCRUBBER_TAB_KEY, cx);
            });
        })
}

#[derive(IntoElement)]
struct StoriesArchiveHighlightsTab {
    owner_user_id: String,
    tab_underlap: f32,
    background: Hsla,
}

impl RenderOnce for StoriesArchiveHighlightsTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bottom_inset = MIN_BOTTOM_INSET.max(self.tab_underlap) + BOTTOM_EXTRA;
        scroll_view("stories-archive-highlights-scroll")
            .shows_indicators(false)
            .size_full()
            .bg(self.background)
            .child(
                vstack(0.0)
                    .w_full()
                    .child(div().w_full().h(px(TOP_SPACER)))
                    .child(HighlightsArchiveGrid::new(self.owner_user_id))
                    .child(div().w_full().h(px(bottom_inset))),
            )
    }
}

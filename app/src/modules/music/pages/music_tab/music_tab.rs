use std::collections::HashSet;

use gpui::{App, ElementId, Entity, FocusHandle, KeyDownEvent, SharedString, Window};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::app::aside::aside_navigator;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::guide_anchor_ui::guide_anchor_scroller::GuideAnchorScroller;
use crate::core::widgets::wrappers::list_shell_header_collapse::{
    ListShellHeaderCollapse, ListShellHeaderCollapseContentLift, ListShellHeaderCollapseProbe,
    ListShellHeaderCollapseSlide,
};
use crate::core::widgets::wrappers::list_shell_ui::ListShellUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::pages::music_home::music_home::MusicHome;
use crate::modules::music::pages::music_home::music_search_overlay::{
    MusicSearchHero, MusicSearchHeroNamespace, MusicSearchOverlay,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

const FIND_KEY: &str = "f";
const HEADER_BUTTON_SPACING: f32 = 8.0;

struct MusicTabState {
    show_search: bool,
    search_presence: Presence<()>,
    search_namespace: Entity<MusicSearchHeroNamespace>,
    chrome_collapse: Entity<ListShellHeaderCollapse>,
    focus: FocusHandle,
}

impl MusicTabState {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            show_search: false,
            search_presence: Presence::new(None).with_transition(Transition::opacity()),
            search_namespace: cx.new(|_| MusicSearchHeroNamespace::new()),
            chrome_collapse: cx.new(|_| ListShellHeaderCollapse::new()),
            focus: cx.focus_handle(),
        }
    }

    /// sets show search, animated with the search hero animation
    fn set_show_search(&mut self, open: bool, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        self.show_search = open;
        self.search_presence.set_in(open.then_some(()), MusicSearchHero::animation(), now, cx);
        self.search_namespace.update(cx, |namespace, cx| namespace.set_presented(open, now, cx));
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct MusicTab {
    id: ElementId,
    shows_back_button: bool,
}

impl MusicTab {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), shows_back_button: false }
    }

    pub fn shows_back_button(mut self, shows_back_button: bool) -> Self {
        self.shows_back_button = shows_back_button;
        self
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    fn open_library(nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if aside_navigator::has_frontmost(cx) {
            aside_navigator::open(RootRoute::MusicLibrary, cx);
            return;
        }
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.push(RootRoute::MusicLibrary, NavigationStoreSettlingPush::Drop, cx);
            });
        }
    }

    fn below_header(&self, state: &Entity<MusicTabState>, cx: &App) -> Div {
        let tab_state = state.read(cx);
        let namespace = tab_state.search_namespace.clone();
        let collapse = tab_state.chrome_collapse.clone();

        let search_slot = if tab_state.show_search {
            div().w_full().h(px(MusicChromeMetrics::SEARCH_FIELD_HEIGHT))
        } else {
            let activate_state = state.clone();
            div()
                .relative()
                .w_full()
                .h(px(MusicChromeMetrics::SEARCH_FIELD_HEIGHT))
                .child(
                    SearchFieldUi::new(self.child_id("search-collapsed"), Binding::constant(String::new()))
                        .placeholder(SharedString::from(tr("music_search_placeholder")))
                        .on_activate(move |_, cx| {
                            activate_state.update(cx, |state, cx| state.set_show_search(true, cx));
                        })
                        .guide_anchor(RiseGuideAnchor::MusicSearchField),
                )
                .child(measure(self.child_id("search-collapsed-frame"), move |bounds, _, cx| {
                    namespace.update(cx, |namespace, _| namespace.collapsed_field_frame = Some(bounds));
                }))
        };

        vstack(0.0).w_full().overflow_hidden().child(
            div().w_full().px(px(10.0)).pt(px(8.0)).pb(px(10.0)).child(
                ListShellHeaderCollapseSlide::new(
                    collapse,
                    div().w_full().h(px(MusicChromeMetrics::SEARCH_FIELD_HEIGHT)).child(search_slot),
                )
                .hides_when_folded(true),
            ),
        )
    }

    fn home(&self, state: &Entity<MusicTabState>, cx: &App) -> impl IntoElement {
        let context = MusicHeaderCollapseContext {
            collapse: state.read(cx).chrome_collapse.clone(),
            page_id: MusicChromeMetrics::PAGE_ID.to_owned(),
        };
        div()
            .id(self.child_id("page-home"))
            .size_full()
            .child(env_scope::<MusicHeaderCollapseKey>(Some(context)).child(MusicHome::new(self.child_id("home"))))
    }
}

impl RenderOnce for MusicTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| MusicTabState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();

        use_task(self.child_id("bootstrap"), user_id, window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.bootstrap_music_tab(cx));
            });
        });

        let back_store = nav_store.clone();
        let library_store = nav_store.clone();
        let settings_store = nav_store;
        let shell = ListShellUi::new()
            .title(SharedString::from(tr("navbtn_music")))
            .shows_back_button(self.shows_back_button)
            .on_back(move |_, cx| {
                if let Some(nav_store) = &back_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .right_jsx(
                hstack(HEADER_BUTTON_SPACING)
                    .child(
                        div()
                            .id(self.child_id("library-tooltip"))
                            .tooltip_label(tr("music_library"))
                            .child(PageHeaderIconButton::new(self.child_id("library"), "music.note.list", move |_, cx| {
                                Self::open_library(library_store.as_ref(), cx);
                            }))
                            .guide_anchor(RiseGuideAnchor::MusicLibraryTab),
                    )
                    .child(
                        PageHeaderIconButton::new(self.child_id("settings"), "gearshape", move |_, cx| {
                            if let Some(nav_store) = &settings_store {
                                nav_store.update(cx, |nav_store, cx| {
                                    nav_store.push(RootRoute::MusicSettings, NavigationStoreSettlingPush::Drop, cx);
                                });
                            }
                        })
                        .guide_anchor(RiseGuideAnchor::MusicSettingsButton),
                    ),
            )
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_blurred_header()))
            .header_collapse(state.read(cx).chrome_collapse.clone())
            .below_header(self.below_header(&state, cx))
            .content(div().size_full().child(ListShellHeaderCollapseContentLift::new(
                state.read(cx).chrome_collapse.clone(),
                self.home(&state, cx),
            )));

        let now = RiseClock::monotonic();
        let overlay_id = self.child_id("search-overlay");
        let (focus, namespace) = {
            let tab_state = state.read(cx);
            (tab_state.focus.clone(), tab_state.search_namespace.clone())
        };
        let presented_reader = state.clone();
        let presented_writer = state.clone();
        let refocus = focus.clone();
        let is_presented = Binding::new(
            move |cx: &App| presented_reader.read(cx).show_search,
            move |open: bool, window, cx| {
                presented_writer.update(cx, |state, cx| state.set_show_search(open, cx));
                if !open {
                    window.focus(&refocus, cx);
                }
            },
        );
        let search_covers_tab = {
            let presence = &state.read(cx).search_presence;
            presence.is_present() && !presence.is_running(now)
        };
        let shell = div().size_full().child(shell);
        let shell = if search_covers_tab { shell.invisible() } else { shell };
        let overlay = state.read(cx).search_presence.render_transitioned(now, window, |_| {
            div()
                .absolute()
                .inset_0()
                .child(MusicSearchOverlay::new(overlay_id.clone(), is_presented.clone(), namespace.clone()))
        });

        let key_state = state.clone();
        let page_focus = PageFocus::keyed_with(self.child_id("page-focus"), focus.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let appear_collapse = state.read(cx).chrome_collapse.clone();
        div()
            .id(self.child_id("keys"))
            .relative()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key == FIND_KEY && keystroke.modifiers.secondary() {
                    cx.stop_propagation();
                    key_state.update(cx, |state, cx| {
                        if !state.show_search {
                            state.set_show_search(true, cx);
                        }
                    });
                }
            })
            .child(shell)
            .children(overlay)
            .on_appear(self.child_id("lifecycle"), move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
                appear_collapse.update(cx, |collapse, cx| {
                    collapse.set_range(MusicChromeMetrics::COLLAPSE_RANGE, cx);
                    collapse.retain(&HashSet::from([MusicChromeMetrics::PAGE_ID.to_owned()]));
                    collapse.activate(MusicChromeMetrics::PAGE_ID, None, cx);
                });
                GuideAnchorScroller::register_chrome(&[RiseGuideAnchor::MusicSearchField], &appear_collapse);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

/// geometry of the music chrome that folds away as the page scrolls
/// the search field, same as the chat list without folders
pub struct MusicChromeMetrics;

impl MusicChromeMetrics {
    pub const SEARCH_FIELD_HEIGHT: f32 = 42.0;
    pub const COLLAPSE_RANGE: f32 = Self::SEARCH_FIELD_HEIGHT;
    pub const PAGE_ID: &'static str = "music-home";
}

/// the music tab header collapse that a page reports its scroll to
#[derive(Clone, PartialEq)]
pub struct MusicHeaderCollapseContext {
    pub collapse: Entity<ListShellHeaderCollapse>,
    pub page_id: String,
}

pub struct MusicHeaderCollapseKey;

impl EnvKey for MusicHeaderCollapseKey {
    type Value = Option<MusicHeaderCollapseContext>;

    fn default_value() -> Self::Value {
        None
    }
}

/// a page feeds its scroller to the tab header collapse
/// caller hands it over with scroll_view or list_state
pub fn music_header_collapse_probe(
    id: impl Into<ElementId>,
    context: Option<&MusicHeaderCollapseContext>,
) -> Option<ListShellHeaderCollapseProbe> {
    context.map(|context| ListShellHeaderCollapseProbe::new(id, context.collapse.clone(), context.page_id.clone()))
}

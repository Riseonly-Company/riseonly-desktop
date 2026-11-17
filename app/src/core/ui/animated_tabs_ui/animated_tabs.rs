use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::ThemeStore;
use rise_ui::animated_tabs_ui::animated_tabs_types::{
    AnimatedTabsHeaderPlacement, AnimatedTabsMetrics, TabConfig, TabHeaderConfig,
};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::prelude::*;

use super::animated_tabs_header::AnimatedTabsLiveHeader;
use super::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerAction, AnimatedTabsPagerLiveState};
use super::keyboard_docked_bottom_strip::KeyboardDockedBottomStripModifiers;
use crate::core::widgets::navigations::bottom_chrome_insets::{BottomChromeInsetsModifiers, BottomChromeOwner};

struct AnimatedTabsState {
    page_width: f32,
    page_heights: HashMap<i64, f32>,
    live_drag_state: Entity<AnimatedTabsPagerLiveState>,
    pager_height: Option<Animated<f32>>,
}

#[derive(IntoElement)]
pub struct AnimatedTabs {
    id: ElementId,
    tabs: Vec<TabConfig>,
    active_tab: Binding<i64>,
    horizontal_padding: f32,
    top_padding: f32,
    bottom_padding: f32,
    sizes_pages_vertically_to_content: bool,
    minimum_page_height: f32,
    header_placement: AnimatedTabsHeaderPlacement,
    detached_live_state: Option<Entity<AnimatedTabsPagerLiveState>>,
    on_swipe_back_beyond_first: Option<AnimatedTabsPagerAction>,
}

impl AnimatedTabs {
    const DEFAULT_VERTICAL_PADDING: f32 = 8.0;

    pub fn new(id: impl Into<ElementId>, tabs: Vec<TabConfig>, active_tab: Binding<i64>) -> Self {
        Self {
            id: id.into(),
            tabs,
            active_tab,
            horizontal_padding: 10.0,
            top_padding: Self::DEFAULT_VERTICAL_PADDING,
            bottom_padding: Self::DEFAULT_VERTICAL_PADDING,
            sizes_pages_vertically_to_content: false,
            minimum_page_height: 1.0,
            header_placement: AnimatedTabsHeaderPlacement::Top,
            detached_live_state: None,
            on_swipe_back_beyond_first: None,
        }
    }

    pub fn header_strip(
        id: impl Into<ElementId>,
        tabs: Vec<TabHeaderConfig>,
        active_tab: Binding<i64>,
        live_drag_state: Entity<AnimatedTabsPagerLiveState>,
        horizontal_padding: f32,
    ) -> Div {
        let selected = active_tab.clone();
        let header = AnimatedTabsLiveHeader::new(
            id,
            tabs,
            active_tab,
            live_drag_state,
            Rc::new(move |id, window, cx| Self::select_tab(&selected, id, window, cx)),
        )
        .horizontal_padding(horizontal_padding);
        div()
            .w_full()
            .pt(px(Self::DEFAULT_VERTICAL_PADDING))
            .pb(px(Self::DEFAULT_VERTICAL_PADDING))
            .child(header)
    }

    pub fn detached_header(mut self, live_drag_state: Entity<AnimatedTabsPagerLiveState>) -> Self {
        self.detached_live_state = Some(live_drag_state);
        self
    }

    pub fn horizontal_padding(mut self, padding: f32) -> Self {
        self.horizontal_padding = padding;
        self
    }

    pub fn top_padding(mut self, padding: f32) -> Self {
        self.top_padding = padding;
        self
    }

    pub fn bottom_padding(mut self, padding: f32) -> Self {
        self.bottom_padding = padding;
        self
    }

    pub fn sizes_pages_vertically_to_content(mut self, sizes: bool) -> Self {
        self.sizes_pages_vertically_to_content = sizes;
        self
    }

    pub fn minimum_page_height(mut self, height: f32) -> Self {
        self.minimum_page_height = height;
        self
    }

    pub fn header_placement(mut self, placement: AnimatedTabsHeaderPlacement) -> Self {
        self.header_placement = placement;
        self
    }

    pub fn on_swipe_back_beyond_first(mut self, action: Option<AnimatedTabsPagerAction>) -> Self {
        self.on_swipe_back_beyond_first = action;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name))
    }

    fn tabs_header(&self, live_drag_state: &Entity<AnimatedTabsPagerLiveState>) -> Div {
        let active_tab = self.active_tab.clone();
        let header = AnimatedTabsLiveHeader::new(
            self.child_id("header"),
            self.tabs.iter().map(TabHeaderConfig::from).collect(),
            self.active_tab.clone(),
            live_drag_state.clone(),
            Rc::new(move |id, window, cx| Self::select_tab(&active_tab, id, window, cx)),
        )
        .horizontal_padding(self.horizontal_padding);
        div().w_full().pt(px(self.top_padding)).pb(px(self.bottom_padding)).child(header)
    }

    fn resolved_pager_height(&self, page_heights: &HashMap<i64, f32>, active_tab: i64) -> Option<f32> {
        if !self.sizes_pages_vertically_to_content {
            return None;
        }
        let height = page_heights.get(&active_tab).copied().unwrap_or(self.minimum_page_height);
        Some(self.minimum_page_height.max(height))
    }

    fn select_tab(active_tab: &Binding<i64>, id: i64, window: &mut Window, cx: &mut App) {
        if id == active_tab.get(cx) {
            return;
        }
        active_tab.set(id, window, cx);
    }
}

/// measured height per page, a later report for a page wins
struct AnimatedTabsPageHeightKey;

impl AnimatedTabsPageHeightKey {
    fn default_value() -> HashMap<i64, f32> {
        HashMap::new()
    }

    fn reduce(value: &mut HashMap<i64, f32>, next_value: HashMap<i64, f32>) {
        value.extend(next_value);
    }

    /// keeps finite positive heights that moved by more than half a point
    /// returns true if anything changed
    fn apply(page_heights: &mut HashMap<i64, f32>, heights: HashMap<i64, f32>) -> bool {
        let mut updated = page_heights.clone();
        let mut changes = HashMap::new();
        for (id, height) in heights {
            if !height.is_finite() || height <= 0.0 {
                continue;
            }
            if (updated.get(&id).copied().unwrap_or(0.0) - height).abs() <= 0.5 {
                continue;
            }
            changes.insert(id, height);
        }
        Self::reduce(&mut updated, changes);
        if updated == *page_heights {
            return false;
        }
        *page_heights = updated;
        true
    }
}

impl RenderOnce for AnimatedTabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let active_tab = self.active_tab.get(cx);
        let state = window.with_global_id(self.id.clone(), |_, window| {
            window.use_keyed_state("animated-tabs-state", cx, |_, cx| AnimatedTabsState {
                page_width: 0.0,
                page_heights: AnimatedTabsPageHeightKey::default_value(),
                live_drag_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
                pager_height: None,
            })
        });
        let draws_header = self.detached_live_state.is_none();
        let live_drag_state =
            self.detached_live_state.clone().unwrap_or_else(|| state.read(cx).live_drag_state.clone());

        let resolved_pager_height = self.resolved_pager_height(&state.read(cx).page_heights, active_tab);
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let pager_height = state.update(cx, |state, _| match resolved_pager_height {
            None => {
                state.pager_height = None;
                None
            }
            Some(target) => {
                let animated = state.pager_height.get_or_insert_with(|| Animated::new(target));
                animated.set(target, AnimatedTabsMetrics::release_animation(), now, reduce_motion);
                Some(animated.frame_value(now, window))
            }
        });

        let header_top = (draws_header && self.header_placement == AnimatedTabsHeaderPlacement::Top)
            .then(|| self.tabs_header(&live_drag_state));
        let header_bottom = (draws_header && self.header_placement == AnimatedTabsHeaderPlacement::Bottom)
            .then(|| {
                self.tabs_header(&live_drag_state)
                    .bg(ThemeStore::shared(cx).read(cx).current_theme().bg._100)
                    .occlude()
                    .marks_bottom_chrome(BottomChromeOwner::BottomTabStrip)
            });
        let bottom_strip = self.header_placement == AnimatedTabsHeaderPlacement::Bottom;

        let sizes = self.sizes_pages_vertically_to_content;
        let pages: Vec<AnyElement> = self
            .tabs
            .iter()
            .map(|tab| {
                let heights_state = state.clone();
                let page_id = tab.id;
                AnimatedTabsPage {
                    id: ElementId::NamedInteger("animated-tabs-page".into(), tab.id as u64),
                    sizes_vertically_to_content: sizes,
                    content: (tab.content)(window, cx),
                    on_height: Rc::new(move |height, cx| {
                        if !sizes {
                            return;
                        }
                        heights_state.update(cx, |state, cx| {
                            if AnimatedTabsPageHeightKey::apply(&mut state.page_heights, HashMap::from([(page_id, height)])) {
                                cx.notify();
                            }
                        });
                    }),
                }
                .into_any_element()
            })
            .collect();

        let page_width_state = state.clone();
        let page_width = Binding::new(
            {
                let state = state.clone();
                move |cx: &App| state.read(cx).page_width
            },
            move |width, _window, cx| {
                page_width_state.update(cx, |state, _| state.page_width = width);
            },
        );
        let live_for_suppress = live_drag_state.clone();
        let pager = AnimatedTabsPager::new(
            self.child_id("pager"),
            self.tabs.len() as i64,
            self.active_tab.clone(),
            Binding::constant(0.0),
            page_width,
        )
        .live_drag_state(Some(live_drag_state))
        .yields_first_tab_back_swipe_to_navigation(self.on_swipe_back_beyond_first.is_none())
        .on_swipe_back_beyond_first(self.on_swipe_back_beyond_first.clone())
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed, window: &mut Window, cx: &mut App| {
            if suppressed && bottom_strip {
                window.blur();
            }
            live_for_suppress.update(cx, |live, cx| {
                live.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
            });
        })))
        .children(pages);

        let pager_box = div()
            .w_full()
            .when_some(pager_height, |pager_box, height| pager_box.h(px(height)))
            .when(pager_height.is_none(), |pager_box| pager_box.flex_1().min_h(px(0.0)))
            .child(pager);

        let tabs = div()
            .flex()
            .flex_col()
            .w_full()
            .when(!sizes, |tabs| tabs.size_full())
            .children(header_top)
            .child(pager_box);
        match header_bottom {
            Some(header) => tabs
                .keyboard_docked_bottom_strip(
                    true,
                    f32::from(AnimatedTabsMetrics::BAR_HEIGHT) + self.top_padding + self.bottom_padding,
                    false,
                    header,
                )
                .into_any_element(),
            None => tabs.into_any_element(),
        }
    }
}

type AnimatedTabsPageHeight = Rc<dyn Fn(f32, &mut App)>;

#[derive(IntoElement)]
struct AnimatedTabsPage {
    id: ElementId,
    sizes_vertically_to_content: bool,
    content: AnyElement,
    on_height: AnimatedTabsPageHeight,
}

impl RenderOnce for AnimatedTabsPage {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.sizes_vertically_to_content {
            let on_height = self.on_height;
            return div()
                .id(self.id)
                .w_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .relative()
                        .w_full()
                        .flex_none()
                        .child(self.content)
                        .child(measure("animated-tabs-page-height", move |bounds, _window, cx| {
                            on_height(f32::from(bounds.size.height), cx);
                        })),
                );
        }
        div().id(self.id).size_full().child(self.content)
    }
}

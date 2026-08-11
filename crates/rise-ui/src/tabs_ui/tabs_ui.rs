use gpui::{
    App, ClickEvent, Context, ElementId, EventEmitter, FontWeight, IntoElement, Pixels, Render,
    SharedString, TextRun, Window, div, prelude::*, px,
};
use rise_theme::AppTheme;

use super::tabs_geometry::{self, TabsLayout};

/// One tab: a stable caller-owned id and the string it draws. The id must not be
/// a position, or reordering changes which tab a click addresses.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TabItem {
    pub id: SharedString,
    pub label: SharedString,
}

impl TabItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum TabsEvent {
    /// A tab was chosen by pointer or keyboard. The bar has ALREADY moved its
    /// own selection; the screen reacts by moving its pages.
    Selected(usize),
}

/// The segmented tab bar. It owns its own selection and announces it with
/// [`TabsEvent::Selected`]; the pager feeds `drag_progress` back in so the
/// indicator tracks the finger.
pub struct TabsUiState {
    id: ElementId,
    tabs: Vec<TabItem>,
    active: usize,
    drag_progress: f32,
    force_scrollable: bool,
    /// Label widths parallel to `tabs`, empty until the first render — before
    /// that there is no text system to shape with.
    measured: Vec<Pixels>,
    measured_key: Option<MeasureKey>,
}

#[derive(Clone, PartialEq, Debug)]
struct MeasureKey {
    labels: Vec<SharedString>,
    font_size: Pixels,
    padding_x: Pixels,
}

impl EventEmitter<TabsEvent> for TabsUiState {}

impl TabsUiState {
    pub fn new(id: impl Into<ElementId>, tabs: Vec<TabItem>, _cx: &mut Context<Self>) -> Self {
        Self {
            id: id.into(),
            tabs,
            active: 0,
            drag_progress: 0.0,
            force_scrollable: true,
            measured: Vec::new(),
            measured_key: None,
        }
    }

    /// Stretch the tabs to fill the bar when they fit. Off by default.
    pub fn distribute_when_it_fits(mut self, distribute: bool) -> Self {
        self.force_scrollable = !distribute;
        self
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn active_id(&self) -> Option<&SharedString> {
        self.tabs.get(self.active).map(|tab| &tab.id)
    }

    pub fn tabs(&self) -> &[TabItem] {
        &self.tabs
    }

    /// Moves the selection without announcing it, for a screen restoring state.
    pub fn set_active(&mut self, active: usize, cx: &mut Context<Self>) {
        if active >= self.tabs.len() || active == self.active {
            return;
        }
        self.active = active;
        self.drag_progress = 0.0;
        cx.notify();
    }

    /// The pager's live progress, a signed fraction of a page. Safe to call on
    /// every frame: a sub-pixel change does not notify.
    pub fn set_drag_progress(&mut self, progress: f32, cx: &mut Context<Self>) {
        let progress = if progress.is_finite() { progress } else { 0.0 };
        if (progress - self.drag_progress).abs() < Self::PROGRESS_EPSILON {
            return;
        }
        self.drag_progress = progress;
        cx.notify();
    }

    const PROGRESS_EPSILON: f32 = 0.002;

    pub fn set_tabs(&mut self, tabs: Vec<TabItem>, cx: &mut Context<Self>) {
        if tabs == self.tabs {
            return;
        }
        self.active = self.active.min(tabs.len().saturating_sub(1));
        self.tabs = tabs;
        self.measured.clear();
        self.measured_key = None;
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        self.active = index;
        self.drag_progress = 0.0;
        cx.emit(TabsEvent::Selected(index));
        cx.notify();
    }

    fn measure(&mut self, theme: &AppTheme, window: &Window) {
        let token = theme
            .typography
            .style(theme.tabs.tab_font_size, FontWeight::SEMIBOLD);
        let key = MeasureKey {
            labels: self.tabs.iter().map(|tab| tab.label.clone()).collect(),
            font_size: token.size,
            padding_x: theme.tabs.tab_padding_x,
        };

        if self.measured_key.as_ref() == Some(&key) {
            return;
        }

        // SEMIBOLD for every tab, active or not: measuring in the face each one
        // currently has would change tab widths as the selection moves.
        self.measured = self
            .tabs
            .iter()
            .map(|tab| {
                let run = TextRun {
                    len: tab.label.len(),
                    font: token.font.clone(),
                    color: theme.text.primary,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped =
                    window
                        .text_system()
                        .shape_line(tab.label.clone(), token.size, &[run], None);

                shaped.width + theme.tabs.tab_padding_x * 2.0
            })
            .collect();
        self.measured_key = Some(key);
    }

    fn layout_for(&self, theme: &AppTheme, available: Pixels) -> TabsLayout {
        tabs_geometry::layout(
            &self.measured,
            available,
            &theme.tabs,
            self.force_scrollable,
        )
    }
}

impl Render for TabsUiState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui_theme(cx);
        self.measure(&theme, window);

        let layout = self.layout_for(&theme, window.viewport_size().width);
        let indicator = tabs_geometry::indicator(&layout, self.active, self.drag_progress);
        let token = theme
            .typography
            .style(theme.tabs.tab_font_size, FontWeight::SEMIBOLD);
        let inactive_token = theme
            .typography
            .style(theme.tabs.tab_font_size, FontWeight::MEDIUM);

        let mut row = div()
            .id(self.id.clone())
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .h(theme.tabs.indicator_height());

        if let Some(geometry) = indicator {
            row = row.child(
                div()
                    .absolute()
                    .left(geometry.x)
                    .top(px(0.0))
                    .w(geometry.width)
                    .h(theme.tabs.indicator_height())
                    .rounded(theme.tabs.indicator_radius)
                    .bg(theme.bg._500),
            );
        }

        for (index, tab) in self.tabs.iter().enumerate() {
            let is_active = index == self.active;
            let width = layout.widths.get(index).copied();
            let style = if is_active { &token } else { &inactive_token };

            let mut label = div()
                .id(ElementId::Name(tab.id.clone()))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(style.size)
                .line_height(style.line_height)
                .font(style.font.clone())
                .text_color(if is_active {
                    theme.text.primary
                } else {
                    theme.text.secondary
                })
                .on_click(cx.listener(move |state, _: &ClickEvent, _, cx| {
                    state.select(index, cx);
                }))
                .child(tab.label.clone());

            if let Some(width) = width {
                label = label.w(width);
            } else {
                label = label.px(theme.tabs.tab_padding_x);
            }

            row = row.child(label);
        }

        div()
            .h(theme.tabs.bar_height)
            .p(theme.tabs.bar_inner_padding)
            .rounded(theme.tabs.bar_height)
            .bg(theme.bg._200)
            .border(theme.avatar.border_width)
            .border_color(theme.border._200)
            .overflow_hidden()
            .child(row)
    }
}

/// Cloned so the borrow of `cx` is over before the listeners are built.
fn rise_ui_theme(cx: &App) -> AppTheme {
    crate::theme(cx).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use rise_theme::AppTheme;

    fn tabs() -> Vec<TabItem> {
        vec![
            TabItem::new("all", "All"),
            TabItem::new("publications", "Publications"),
            TabItem::new("threads", "Threads"),
        ]
    }

    fn state(cx: &mut TestAppContext) -> gpui::Entity<TabsUiState> {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                crate::install_theme(AppTheme::dark(), cx);
            }
        });
        cx.new(|cx| TabsUiState::new("tabs.test", tabs(), cx))
    }

    #[gpui::test]
    fn a_bar_opens_on_its_first_tab(cx: &mut TestAppContext) {
        let bar = state(cx);
        cx.update(|cx| {
            assert_eq!(bar.read(cx).active(), 0);
            assert_eq!(
                bar.read(cx).active_id().map(|id| id.to_string()),
                Some("all".into())
            );
        });
    }

    #[gpui::test]
    fn selecting_a_tab_announces_it_and_restoring_one_does_not(cx: &mut TestAppContext) {
        let bar = state(cx);
        let announced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

        cx.update(|cx| {
            let seen = announced.clone();
            cx.subscribe(&bar, move |_, event: &TabsEvent, _| {
                let TabsEvent::Selected(index) = event;
                seen.borrow_mut().push(*index);
            })
            .detach();
        });

        bar.update(cx, |bar, cx| bar.select(2, cx));
        cx.run_until_parked();
        assert_eq!(*announced.borrow(), vec![2]);

        bar.update(cx, |bar, cx| bar.set_active(1, cx));
        cx.run_until_parked();
        assert_eq!(
            *announced.borrow(),
            vec![2],
            "a screen restoring its own state must not be told about it again"
        );
        cx.update(|cx| assert_eq!(bar.read(cx).active(), 1));
    }

    #[gpui::test]
    fn selecting_the_tab_that_is_already_active_announces_nothing(cx: &mut TestAppContext) {
        let bar = state(cx);
        let announced = std::rc::Rc::new(std::cell::RefCell::new(0usize));

        cx.update(|cx| {
            let seen = announced.clone();
            cx.subscribe(&bar, move |_, _: &TabsEvent, _| *seen.borrow_mut() += 1)
                .detach();
        });

        bar.update(cx, |bar, cx| bar.select(0, cx));
        cx.run_until_parked();
        assert_eq!(*announced.borrow(), 0);
    }

    #[gpui::test]
    fn an_out_of_range_selection_is_refused_rather_than_panicking(cx: &mut TestAppContext) {
        let bar = state(cx);
        bar.update(cx, |bar, cx| {
            bar.select(9, cx);
            bar.set_active(9, cx);
        });
        cx.update(|cx| assert_eq!(bar.read(cx).active(), 0));
    }

    #[gpui::test]
    fn a_resting_finger_does_not_redraw_the_bar(cx: &mut TestAppContext) {
        let bar = state(cx);
        bar.update(cx, |bar, cx| {
            bar.set_drag_progress(0.0005, cx);
            assert_eq!(
                bar.drag_progress, 0.0,
                "a change below a pixel must not reach the frame path"
            );

            bar.set_drag_progress(0.4, cx);
            assert_eq!(bar.drag_progress, 0.4);

            bar.set_drag_progress(f32::NAN, cx);
            assert_eq!(bar.drag_progress, 0.0, "a broken value settles at rest");
        });
    }

    #[gpui::test]
    fn replacing_the_tabs_drops_the_measurements_taken_for_the_old_ones(cx: &mut TestAppContext) {
        let bar = state(cx);
        bar.update(cx, |bar, cx| {
            bar.measured = vec![px(10.0), px(10.0), px(10.0)];
            bar.set_tabs(vec![TabItem::new("only", "Only")], cx);

            assert!(bar.measured.is_empty());
            assert_eq!(bar.active(), 0, "the old index would be past the new end");
        });
    }
}

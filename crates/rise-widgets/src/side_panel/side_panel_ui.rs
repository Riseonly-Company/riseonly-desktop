use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, Entity, EventEmitter, Global, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Render, SharedString, Window, actions, div,
    prelude::*,
};
use rise_theme::{AppTheme, PanelMetrics};
use rise_ui::page_header_toolbar_button::PageHeaderAction;
use rise_ui::page_header_ui::{PanelChrome, PanelChromeKey, PanelChromeSlot};
use rise_ui::swiftui::environment::env_scope;

use super::panel_gestures::{self, ResizeOutcome};
use super::side_panel_preferences::SidePanelPreferences;
use crate::block_ui::BlockUi;

pub const KEY_CONTEXT: &str = "SidePanel";

actions!(rise_side_panel, [ToggleSidePanel, CloseSidePanel]);

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

/// idempotent, the side panel constructor already calls it
/// the toggle is global since a closed panel isnt drawn and cant hold focus, the shell root handles it
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);

    cx.bind_keys([
        KeyBinding::new("secondary-alt-0", ToggleSidePanel, None),
        KeyBinding::new("escape", CloseSidePanel, Some(KEY_CONTEXT)),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SidePanelEvent {
    Toggled { open: bool },
}

#[derive(Clone, Copy, Debug)]
struct ResizeDrag {
    start_x: Pixels,
    start_width: Pixels,
}

struct ResizeGhost;

impl Render for ResizeGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// right panel of the desktop frame, the context of the screen in the centre
/// eg the open chat profile or the music queue, owns the frame and width and open state per context
pub struct SidePanelUi {
    context: SharedString,
    content: AnyView,
    width: Pixels,
    width_scope: SharedString,
    chosen: bool,
    column_width: Option<Pixels>,
    open: bool,
    drag: Option<ResizeDrag>,
    preferences: SidePanelPreferences,
    shows_close: bool,
    chrome_taken: Rc<Cell<bool>>,
    grip_trailing: bool,
    grip_outside: bool,
}

impl EventEmitter<SidePanelEvent> for SidePanelUi {}

impl SidePanelUi {
    pub fn new(
        context: impl Into<SharedString>,
        content: AnyView,
        preferences: SidePanelPreferences,
        cx: &mut Context<Self>,
    ) -> Self {
        install_key_bindings(cx);
        let context = context.into();
        let theme = rise_ui::theme(cx);
        let width = preferences.width(&theme.panel);
        let open = preferences.is_open(&context);

        Self {
            context,
            content,
            width,
            width_scope: SharedString::default(),
            chosen: false,
            column_width: None,
            open,
            drag: None,
            preferences,
            shows_close: true,
            chrome_taken: Rc::default(),
            grip_trailing: false,
            grip_outside: false,
        }
    }

    pub fn context(&self) -> &SharedString {
        &self.context
    }

    pub fn width(&self) -> Pixels {
        self.width
    }

    /// true if the frame lays the panel out at width even beside a reading centre with room to spare
    /// the person chose it or is dragging it now
    pub fn has_chosen_width(&self) -> bool {
        self.chosen || self.drag.is_some()
    }

    /// frame names the section on screen and the panel takes its remembered width, default if never dragged
    /// a drag in progress keeps its section
    pub fn set_width_scope(&mut self, scope: SharedString, cx: &mut Context<Self>) {
        if self.width_scope == scope || self.drag.is_some() {
            return;
        }
        let metrics = rise_ui::theme(cx).panel;
        let chosen = self.preferences.chosen_width(&scope, &metrics);
        let width = chosen.unwrap_or(metrics.default_width);
        self.width_scope = scope;
        if self.width != width || self.chosen != chosen.is_some() {
            self.width = width;
            self.chosen = chosen.is_some();
            cx.notify();
        }
    }

    pub fn set_column_width(&mut self, width: Option<Pixels>, cx: &mut Context<Self>) {
        if self.column_width != width {
            self.column_width = width;
            cx.notify();
        }
    }

    /// the divider shares the column so column width is the divider plus this
    fn drawn_width(&self, theme: &AppTheme) -> Pixels {
        match self.column_width {
            Some(column) => (column - theme.shell.column_divider).max(Pixels::ZERO),
            None => self.width,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_resizing(&self) -> bool {
        self.drag.is_some()
    }

    /// another screen with another context came to the centre, its own remembered state applies
    pub fn set_context(
        &mut self,
        context: impl Into<SharedString>,
        content: AnyView,
        cx: &mut Context<Self>,
    ) {
        let context = context.into();
        let open = self.preferences.is_open(&context);
        let changed = open != self.open;

        self.context = context;
        self.content = content;
        self.chrome_taken = Rc::default();
        self.open = open;
        cx.notify();
        if changed {
            cx.emit(SidePanelEvent::Toggled { open });
        }
    }

    pub fn set_content(&mut self, content: AnyView, cx: &mut Context<Self>) {
        self.content = content;
        self.chrome_taken = Rc::default();
        cx.notify();
    }

    pub fn set_shows_close(&mut self, shows_close: bool, cx: &mut Context<Self>) {
        if self.shows_close == shows_close {
            return;
        }
        self.shows_close = shows_close;
        cx.notify();
    }

    /// the panel stands before the centre, its resize grip moves to the edge that faces the centre
    pub fn set_grip_trailing(&mut self, trailing: bool, cx: &mut Context<Self>) {
        if self.grip_trailing == trailing {
            return;
        }
        self.grip_trailing = trailing;
        cx.notify();
    }

    pub fn set_grip_outside(&mut self, outside: bool, cx: &mut Context<Self>) {
        if self.grip_outside == outside {
            return;
        }
        self.grip_outside = outside;
        cx.notify();
    }

    pub fn begin_resize(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let metrics = rise_ui::theme(cx).panel;
        self.begin(event, &metrics, cx);
    }

    pub fn resize_to_pointer(&mut self, pointer_x: Pixels, cx: &mut Context<Self>) {
        let theme = rise_ui::theme(cx).clone();
        self.resize_to(pointer_x, &theme, cx);
    }

    pub fn end_resize(&mut self, cx: &mut Context<Self>) {
        let theme = rise_ui::theme(cx).clone();
        self.release(&theme, cx);
    }

    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.drag = None;
        self.preferences.remember_open(&self.context, open);
        cx.notify();
        cx.emit(SidePanelEvent::Toggled { open });
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.set_open(!self.open, cx);
    }

    pub fn toggle_listener(
        panel: Entity<Self>,
    ) -> impl Fn(&ToggleSidePanel, &mut Window, &mut App) + 'static {
        move |_: &ToggleSidePanel, _window: &mut Window, cx: &mut App| {
            panel.update(cx, |panel, cx| panel.toggle(cx));
        }
    }

    fn begin(&mut self, event: &MouseDownEvent, metrics: &PanelMetrics, cx: &mut Context<Self>) {
        if event.click_count >= 2 {
            self.reset_width(metrics, cx);
            return;
        }
        let start_width = self.column_width.unwrap_or(self.width).min(metrics.max_width);
        self.drag = Some(ResizeDrag {
            start_x: event.position.x,
            start_width,
        });
        self.width = start_width;
        cx.notify();
    }

    fn reset_width(&mut self, metrics: &PanelMetrics, cx: &mut Context<Self>) {
        self.drag = None;
        self.chosen = false;
        self.width = metrics.default_width;
        self.preferences.forget_chosen_width(&self.width_scope);
        cx.notify();
    }

    fn resize_to(&mut self, pointer_x: Pixels, theme: &AppTheme, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else {
            return;
        };

        let travel = if self.grip_trailing { drag.start_x - pointer_x } else { pointer_x - drag.start_x };
        let width = panel_gestures::width_during_drag(&theme.panel, drag.start_width, travel);

        if width != self.width {
            self.width = width;
            cx.notify();
        }
    }

    fn release(&mut self, theme: &AppTheme, cx: &mut Context<Self>) {
        if self.drag.take().is_none() {
            return;
        }

        match panel_gestures::release(&theme.panel, self.width) {
            ResizeOutcome::Settle(width) => {
                self.width = width;
                self.chosen = true;
                self.preferences.remember_chosen_width(&self.width_scope, width);
                cx.notify();
            }
            ResizeOutcome::Dismiss => {
                self.width = theme.panel.min_width;
                self.chosen = true;
                self.preferences.remember_chosen_width(&self.width_scope, theme.panel.min_width);
                self.set_open(false, cx);
            }
        }
    }

    fn content_with_chrome(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.shows_close {
            return self.content.clone().into_any_element();
        }
        let panel = cx.entity().downgrade();
        let close: PageHeaderAction = Rc::new(move |_, cx| {
            let _ = panel.update(cx, |panel, cx| panel.set_open(false, cx));
        });
        let chrome = PanelChrome::new(self.chrome_taken.clone()).close(Some(close));
        let row = chrome.row();
        let page = env_scope::<PanelChromeKey>(Some(chrome)).child(self.content.clone());
        PanelChromeSlot::new(self.chrome_taken.clone(), page, Some(row)).into_any_element()
    }
}

impl Render for SidePanelUi {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        if !self.open {
            return div().into_any_element();
        }

        let move_theme = theme.clone();
        let grip_metrics = theme.panel;
        let collapses = self.drag.is_some() && self.width < theme.panel.dismiss_width();
        let release_theme = theme.clone();
        let release_out_theme = theme.clone();
        let content = self.content_with_chrome(cx);

        div()
            .flex()
            .flex_row()
            .h_full()
            .flex_shrink_0()
            .child(BlockUi::divider(&theme))
            .child(
                BlockUi::column(&theme)
                    .id("side_panel")
                    .key_context(KEY_CONTEXT)
                    .w(self.drawn_width(&theme))
                    .relative()
                    .flex()
                    .flex_col()
                    .bg(theme.bg._100)
                    .on_action(cx.listener(|panel, _: &CloseSidePanel, _window, cx| {
                        panel.set_open(false, cx);
                    }))
                    .on_drag_move(cx.listener(
                        move |panel, event: &gpui::DragMoveEvent<ResizeDrag>, _window, cx| {
                            panel.resize_to(event.event.position.x, &move_theme, cx);
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |panel, _: &MouseUpEvent, _window, cx| {
                            panel.release(&release_theme, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |panel, _: &MouseUpEvent, _window, cx| {
                            panel.release(&release_out_theme, cx);
                        }),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .overflow_hidden()
                            .when(collapses, |content| content.opacity(0.4))
                            .child(content),
                    )
                    .when(!self.grip_outside, |column| column.child(
                        div()
                            .id("side_panel.grip")
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .when(self.grip_trailing, |grip| grip.right_0())
                            .when(!self.grip_trailing, |grip| grip.left_0())
                            .w(theme.panel.grip_width)
                            .cursor_col_resize()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |panel, event: &MouseDownEvent, _window, cx| {
                                    panel.begin(event, &grip_metrics, cx);
                                }),
                            )
                            .on_drag(
                                ResizeDrag {
                                    start_x: Pixels::ZERO,
                                    start_width: self.width,
                                },
                                |_, _, _, cx| cx.new(|_| ResizeGhost),
                            ),
                    )),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, px};

    struct Nothing;

    impl Render for Nothing {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn panel(cx: &mut TestAppContext, preferences: SidePanelPreferences) -> Entity<SidePanelUi> {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });

        let content = cx.new(|_| Nothing);
        cx.new(|cx| SidePanelUi::new("chat_profile", content.into(), preferences, cx))
    }

    fn fresh() -> SidePanelPreferences {
        SidePanelPreferences::in_memory()
    }

    #[gpui::test]
    fn a_panel_opens_at_its_default_width(cx: &mut TestAppContext) {
        let panel = panel(cx, fresh());
        let theme = AppTheme::dark();
        cx.update(|cx| {
            assert!(panel.read(cx).is_open());
            assert_eq!(panel.read(cx).width(), theme.panel.default_width);
        });
    }

    #[gpui::test]
    fn pulling_the_left_edge_widens_it_and_the_width_is_kept(cx: &mut TestAppContext) {
        let preferences = fresh();
        let panel = panel(cx, preferences.clone());
        let theme = AppTheme::dark();

        panel.update(cx, |panel, cx| {
            panel.drag = Some(ResizeDrag {
                start_x: px(900.0),
                start_width: theme.panel.default_width,
            });
            panel.resize_to(px(860.0), &theme, cx);
            assert_eq!(panel.width(), theme.panel.default_width + px(40.0));

            panel.release(&theme, cx);
            assert!(!panel.is_resizing());
        });

        assert_eq!(
            preferences.width(&theme.panel),
            theme.panel.default_width + px(40.0)
        );
        cx.update(|cx| assert!(panel.read(cx).has_chosen_width()));
    }

    #[gpui::test]
    fn each_section_opens_at_its_own_width(cx: &mut TestAppContext) {
        let preferences = fresh();
        let theme = AppTheme::dark();
        preferences.remember_chosen_width("aside_posts", theme.panel.min_width + px(10.0));
        let panel = panel(cx, preferences);

        panel.update(cx, |panel, cx| {
            panel.set_width_scope("aside_posts".into(), cx);
            assert_eq!(panel.width(), theme.panel.min_width + px(10.0));
            assert!(panel.has_chosen_width());

            panel.set_width_scope("aside_chats".into(), cx);
            assert_eq!(panel.width(), theme.panel.default_width);
            assert!(!panel.has_chosen_width(), "a section never dragged hands its room to the panel");
        });
    }

    #[gpui::test]
    fn dragging_it_away_closes_it_and_remembers_that(cx: &mut TestAppContext) {
        let preferences = fresh();
        let panel = panel(cx, preferences.clone());
        let theme = AppTheme::dark();
        let toggled = std::rc::Rc::new(std::cell::Cell::new(None));

        cx.update(|cx| {
            let seen = toggled.clone();
            cx.subscribe(&panel, move |_, event: &SidePanelEvent, _| {
                let SidePanelEvent::Toggled { open } = *event;
                seen.set(Some(open));
            })
            .detach();
        });

        panel.update(cx, |panel, cx| {
            panel.drag = Some(ResizeDrag {
                start_x: px(900.0),
                start_width: theme.panel.default_width,
            });
            panel.resize_to(px(1200.0), &theme, cx);
            assert!(panel.width() < theme.panel.dismiss_width());
            panel.release(&theme, cx);
        });
        cx.run_until_parked();

        assert_eq!(toggled.get(), Some(false));
        assert!(!preferences.is_open("chat_profile"));
        cx.update(|cx| {
            assert_eq!(
                panel.read(cx).width(),
                theme.panel.min_width,
                "it comes back at its minimum, not the sliver it was dragged to"
            );
        });
    }

    #[gpui::test]
    fn the_toggle_brings_a_closed_panel_back(cx: &mut TestAppContext) {
        let preferences = fresh();
        let panel = panel(cx, preferences.clone());

        panel.update(cx, |panel, cx| panel.toggle(cx));
        assert!(!preferences.is_open("chat_profile"));

        panel.update(cx, |panel, cx| panel.toggle(cx));
        assert!(preferences.is_open("chat_profile"));
        cx.update(|cx| assert!(panel.read(cx).is_open()));
    }

    #[gpui::test]
    fn each_context_keeps_its_own_state(cx: &mut TestAppContext) {
        let preferences = fresh();
        preferences.remember_open("music_queue", false);
        let panel = panel(cx, preferences);

        let content = cx.new(|_| Nothing);
        panel.update(cx, |panel, cx| {
            panel.set_context("music_queue", content.into(), cx);
            assert!(!panel.is_open());
        });
    }

    #[gpui::test]
    fn a_release_with_no_drag_behind_it_does_nothing(cx: &mut TestAppContext) {
        let panel = panel(cx, fresh());
        let theme = AppTheme::dark();

        panel.update(cx, |panel, cx| panel.release(&theme, cx));
        cx.update(|cx| {
            assert!(panel.read(cx).is_open());
            assert_eq!(panel.read(cx).width(), theme.panel.default_width);
        });
    }
}

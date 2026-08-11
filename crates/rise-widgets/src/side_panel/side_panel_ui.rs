use gpui::{
    AnyView, App, Context, EventEmitter, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, Render, Window, div, prelude::*,
};
use rise_platform::materials::Material;
use rise_theme::AppTheme;

use super::panel_gestures::{self, ResizeOutcome};
use crate::block_ui::BlockUi;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SidePanelEvent {
    /// The user dragged it away. The shell closes it.
    Dismissed,
}

#[derive(Clone, Copy, Debug)]
struct ResizeDrag {
    start_x: Pixels,
    start_width: Pixels,
}

/// gpui wants an entity to draw under the pointer; a resize has nothing to show.
struct ResizeGhost;

impl Render for ResizeGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The extra column between the rail and the content: drag the inner edge to
/// resize between a floor and a ceiling, drag past half the default width to
/// dismiss. It owns the frame, the width and the dismissal; the page stack is
/// the caller's to fill, with [`super::PanelPages`] and [`super::should_pop`].
pub struct SidePanelUi {
    content: AnyView,
    width: Pixels,
    drag: Option<ResizeDrag>,
}

impl EventEmitter<SidePanelEvent> for SidePanelUi {}

impl SidePanelUi {
    pub const REGION_KEY: &'static str = "shell.side_panel";

    pub fn new(content: AnyView, theme: &AppTheme) -> Self {
        Self {
            content,
            width: theme.panel.default_width,
            drag: None,
        }
    }

    pub fn width(&self) -> Pixels {
        self.width
    }

    pub fn is_resizing(&self) -> bool {
        self.drag.is_some()
    }

    pub fn set_content(&mut self, content: AnyView, cx: &mut Context<Self>) {
        self.content = content;
        cx.notify();
    }

    fn begin(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.drag = Some(ResizeDrag {
            start_x: event.position.x,
            start_width: self.width,
        });
        cx.notify();
    }

    fn resize_to(&mut self, pointer_x: Pixels, theme: &AppTheme, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else {
            return;
        };

        let width = panel_gestures::width_during_drag(
            &theme.panel,
            drag.start_width,
            pointer_x - drag.start_x,
        );

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
                cx.notify();
            }
            ResizeOutcome::Dismiss => {
                // Restored before the dismissal is announced, so a panel reopened
                // later is not the sliver it was dragged to.
                self.width = theme.panel.default_width;
                cx.emit(SidePanelEvent::Dismissed);
            }
        }
    }
}

impl Render for SidePanelUi {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App).clone();
        let grip_theme = theme.clone();
        let release_theme = theme.clone();
        let release_out_theme = theme.clone();

        BlockUi::surface(Self::REGION_KEY, Material::Panel, &theme, cx)
            .w(self.width)
            .h_full()
            .relative()
            .flex()
            .flex_col()
            .on_drag_move(cx.listener(
                move |panel, event: &gpui::DragMoveEvent<ResizeDrag>, _window, cx| {
                    panel.resize_to(event.event.position.x, &grip_theme, cx);
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |panel, _: &MouseUpEvent, _window, cx| {
                    panel.release(&release_theme, cx);
                }),
            )
            // A dismiss drag ends with the pointer outside the panel's bounds.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |panel, _: &MouseUpEvent, _window, cx| {
                    panel.release(&release_out_theme, cx);
                }),
            )
            .child(div().flex_1().overflow_hidden().child(self.content.clone()))
            .child(
                div()
                    .id("panel.grip")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(theme.panel.grip_width)
                    .cursor_col_resize()
                    .when(self.drag.is_some(), |grip| grip.bg(theme.primary._100))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|panel, event: &MouseDownEvent, _window, cx| {
                            panel.begin(event, cx);
                        }),
                    )
                    .on_drag(
                        ResizeDrag {
                            start_x: Pixels::ZERO,
                            start_width: self.width,
                        },
                        |_, _, _, cx| cx.new(|_| ResizeGhost),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, TestAppContext, px};
    use rise_theme::AppTheme;

    struct Nothing;

    impl Render for Nothing {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn panel(cx: &mut TestAppContext) -> Entity<SidePanelUi> {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });

        let content = cx.new(|_| Nothing);
        let theme = AppTheme::dark();
        cx.new(|_| SidePanelUi::new(content.into(), &theme))
    }

    #[gpui::test]
    fn a_panel_opens_at_its_default_width(cx: &mut TestAppContext) {
        let panel = panel(cx);
        let theme = AppTheme::dark();
        cx.update(|cx| assert_eq!(panel.read(cx).width(), theme.panel.default_width));
    }

    #[gpui::test]
    fn dragging_the_edge_resizes_it_and_letting_go_keeps_the_width(cx: &mut TestAppContext) {
        let panel = panel(cx);
        let theme = AppTheme::dark();

        panel.update(cx, |panel, cx| {
            panel.drag = Some(ResizeDrag {
                start_x: px(400.0),
                start_width: theme.panel.default_width,
            });
            panel.resize_to(px(480.0), &theme, cx);
            assert_eq!(panel.width(), theme.panel.default_width + px(80.0));

            panel.release(&theme, cx);
            assert_eq!(panel.width(), theme.panel.default_width + px(80.0));
            assert!(!panel.is_resizing());
        });
    }

    #[gpui::test]
    fn dragging_past_half_closes_it_and_restores_a_usable_width(cx: &mut TestAppContext) {
        let panel = panel(cx);
        let theme = AppTheme::dark();
        let dismissed = std::rc::Rc::new(std::cell::Cell::new(false));

        cx.update(|cx| {
            let seen = dismissed.clone();
            cx.subscribe(&panel, move |_, _: &SidePanelEvent, _| seen.set(true))
                .detach();
        });

        panel.update(cx, |panel, cx| {
            panel.drag = Some(ResizeDrag {
                start_x: px(400.0),
                start_width: theme.panel.default_width,
            });
            // Far enough left to cross the dismiss threshold.
            panel.resize_to(px(100.0), &theme, cx);
            assert!(panel.width() < theme.panel.dismiss_width());

            panel.release(&theme, cx);
        });
        cx.run_until_parked();

        assert!(dismissed.get());
        cx.update(|cx| {
            assert_eq!(
                panel.read(cx).width(),
                theme.panel.default_width,
                "reopening at the sliver it was dragged to would be unusable"
            );
        });
    }

    #[gpui::test]
    fn a_release_with_no_drag_behind_it_does_nothing(cx: &mut TestAppContext) {
        let panel = panel(cx);
        let theme = AppTheme::dark();
        let dismissed = std::rc::Rc::new(std::cell::Cell::new(false));

        cx.update(|cx| {
            let seen = dismissed.clone();
            cx.subscribe(&panel, move |_, _: &SidePanelEvent, _| seen.set(true))
                .detach();
        });

        // Every click in the panel raises a mouse-up, drag or no drag.
        panel.update(cx, |panel, cx| panel.release(&theme, cx));
        cx.run_until_parked();

        assert!(!dismissed.get());
        cx.update(|cx| assert_eq!(panel.read(cx).width(), theme.panel.default_width));
    }

    #[gpui::test]
    fn a_resize_beyond_the_ceiling_stops_at_it(cx: &mut TestAppContext) {
        let panel = panel(cx);
        let theme = AppTheme::dark();

        panel.update(cx, |panel, cx| {
            panel.drag = Some(ResizeDrag {
                start_x: px(0.0),
                start_width: theme.panel.default_width,
            });
            panel.resize_to(px(5_000.0), &theme, cx);
            assert_eq!(panel.width(), theme.panel.max_width);
        });
    }
}

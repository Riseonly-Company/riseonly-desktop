use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, AsyncApp, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, StyleRefinement, Styled, Task, WeakEntity, Window,
};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;

pub type ChatSlowModeCountdownAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ChatSlowModeCountdownButton {
    pub id: ElementId,
    pub size: f32,
    pub until: Date,
    pub theme: AppTheme,
    pub on_tap: ChatSlowModeCountdownAction,
    pub on_expire: ChatSlowModeCountdownAction,
}

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

impl RenderOnce for ChatSlowModeCountdownButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let until = self.until;
        let color = self.theme.text.secondary;

        let on_expire = self.on_expire.clone();
        use_task(child_id(&self.id, "expire"), until, window, cx, async move |cx| {
            let delay = until.time_interval_since_now();
            if delay > 0.0 {
                cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            }
            let _ = cx.update(|window, cx| on_expire(window, cx));
        });

        let label = window.with_global_id(child_id(&self.id, "label"), |global_id, window| {
            window.with_element_state(global_id, |held: Option<Entity<ChatSlowModeCountdownLabel>>, _| {
                let label = held.unwrap_or_else(|| cx.new(|cx| ChatSlowModeCountdownLabel::new(until, color, cx)));
                (label.clone(), label)
            })
        });
        label.update(cx, |label, cx| label.configure(until, color, cx));

        let on_tap = self.on_tap.clone();
        LiquidGlassButton::new(child_id(&self.id, "button"), LiquidGlassButtonShape::Circle)
            .size(self.size)
            .accessibility_label(Some(tr("chat_perm_slowmode")))
            .on_press(move |window, cx| on_tap(window, cx))
            .child(label.cached(StyleRefinement::default().size_full()))
    }
}

impl ChatSlowModeCountdownButton {
    fn remaining_seconds(until: Date, date: Date) -> i64 {
        until.time_interval_since(&date).ceil().max(0.0) as i64
    }
}

/// countdown text, ticks by itself and notifies only when the shown second changes
pub struct ChatSlowModeCountdownLabel {
    until: Date,
    color: Hsla,
    remaining: i64,
    _tick: Option<Task<()>>,
}

impl ChatSlowModeCountdownLabel {
    fn new(until: Date, color: Hsla, cx: &mut Context<Self>) -> Self {
        let mut label = Self {
            until,
            color,
            remaining: ChatSlowModeCountdownButton::remaining_seconds(until, Date::now()),
            _tick: None,
        };
        label.restart_tick(cx);
        label
    }

    fn configure(&mut self, until: Date, color: Hsla, cx: &mut Context<Self>) {
        let mut changed = false;
        if self.until != until {
            self.until = until;
            self.remaining = ChatSlowModeCountdownButton::remaining_seconds(until, Date::now());
            self.restart_tick(cx);
            changed = true;
        }
        if self.color != color {
            self.color = color;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn restart_tick(&mut self, cx: &mut Context<Self>) {
        if self.remaining <= 0 {
            self._tick = None;
            return;
        }
        self._tick = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let running = this.update(cx, |label, cx| {
                    let remaining = ChatSlowModeCountdownButton::remaining_seconds(label.until, Date::now());
                    if remaining != label.remaining {
                        label.remaining = remaining;
                        cx.notify();
                    }
                    remaining > 0
                });
                if !matches!(running, Ok(true)) {
                    return;
                }
            }
        }));
    }
}

impl Render for ChatSlowModeCountdownLabel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().items_center().justify_center().child(
            MainText::new(RiseFormat::duration(self.remaining))
                .px(12.)
                .font_weight(FontWeight::SEMIBOLD)
                .color(self.color)
                .number_of_lines(1)
                .monospaced_digits(true)
                .minimum_scale_factor(0.6),
        )
    }
}

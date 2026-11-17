use std::time::Duration;

use gpui::{App, ElementId, IntoElement, ParentElement, RenderOnce, Styled, Task, Window, px};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::lottie_view::LottieView;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::shapes::pass_through;

/// confetti burst, loops by default or a one shot that fades itself out
#[derive(IntoElement, Clone, Debug, PartialEq)]
pub struct ConfettiAnimation {
    pub id: ElementId,
    pub size: f32,
    pub looping: bool,
}

/// spent state held as the opacity it drives, plus the task sleeping towards it
struct ConfettiAnimationState {
    opacity: Animated<f32>,
    _spend: Option<Task<()>>,
}

impl ConfettiAnimation {
    const DURATION_SECONDS: f64 = 3.0;

    const FADE_SECONDS: f64 = 0.45;

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            size: 300.0,
            looping: true,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
}

impl RenderOnce for ConfettiAnimation {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let looping = self.looping;
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            let spend = (!looping).then(|| {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_secs_f64(Self::DURATION_SECONDS))
                        .await;
                    let _ = this.update(cx, |state: &mut ConfettiAnimationState, cx| {
                        state.opacity.set(0.0, ease_out(Self::FADE_SECONDS), RiseClock::monotonic(), false);
                        cx.notify();
                    });
                })
            });
            ConfettiAnimationState {
                opacity: Animated::new(1.0),
                _spend: spend,
            }
        });
        let opacity = state.read(cx).opacity.frame_value(RiseClock::monotonic(), window);
        let side = px(self.size);

        pass_through()
            .w(side)
            .h(side)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .opacity(opacity)
            .child(LottieView::new("confeti").size(self.size).looping(looping))
    }
}

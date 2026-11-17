use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, black, div, px,
    white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::IconUi;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize, LoaderUiType};
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::lifecycle::{on_change, use_task};
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes};
use rise_ui::swiftui::{Animated, linear};

use crate::modules::story::shared::story_publish_staging::StoryUploadProgressCenter;
use crate::modules::story::stores::story::story_actions::story_types::StoryPendingPublishPhase;

pub type StoryPendingUploadCancel = Rc<dyn Fn(&mut Window, &mut App)>;

/// overlay for a story still uploading,
/// progress is subscribed here so a chunk callback repaints only this label
#[derive(IntoElement)]
pub struct StoryPendingUploadOverlay {
    id: ElementId,
    phase: StoryPendingPublishPhase,
    on_cancel: StoryPendingUploadCancel,
}

impl StoryPendingUploadOverlay {
    const RING_SIZE: f32 = 58.0;
    const HIT_SIZE: f32 = 78.0;

    pub fn new(
        id: impl Into<ElementId>,
        phase: StoryPendingPublishPhase,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), phase, on_cancel: Rc::new(on_cancel) }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn ring_progress(&self, fraction: f64) -> f64 {
        match self.phase {
            StoryPendingPublishPhase::Preparing => fraction.min(1.0).max(0.08),
            StoryPendingPublishPhase::Creating => 1.0,
            StoryPendingPublishPhase::Uploading | StoryPendingPublishPhase::Failed => fraction.min(1.0).max(0.08),
        }
    }

    fn status_text(&self, fraction: f64) -> String {
        match self.phase {
            StoryPendingPublishPhase::Preparing => tr("chat_media_upload_processing"),
            StoryPendingPublishPhase::Creating => tr("chat_media_upload_sending"),
            StoryPendingPublishPhase::Uploading | StoryPendingPublishPhase::Failed => {
                let percent = (fraction.max(0.0).min(1.0) * 100.0).floor() as i64;
                tr_args("story_upload_progress", &[FormatArg::Int(percent)])
            }
        }
    }
}

impl RenderOnce for StoryPendingUploadOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fraction_state = window.use_keyed_state(self.child_id("fraction"), cx, |_, _| 0.0_f64);
        let fraction = *fraction_state.read(cx);
        let ring_progress = self.ring_progress(fraction);

        let ring = window.use_keyed_state(self.child_id("ring"), cx, move |_, _| Animated::new(ring_progress));
        {
            let ring = ring.clone();
            on_change(self.child_id("ring-progress"), ring_progress, false, window, cx, move |_, value, _, cx| {
                let target = *value;
                ring.update(cx, |ring, cx| {
                    ring.set_in(target, linear(0.12), RiseClock::monotonic(), cx);
                    cx.notify();
                });
            });
        }
        let now = RiseClock::monotonic();
        let shown_progress = ring.read(cx).value(now);
        if ring.read(cx).is_running(now) {
            window.request_animation_frame();
        }

        {
            let fraction_state = fraction_state.clone();
            use_task(self.child_id("progress"), (), window, cx, async move |cx| {
                let key = StoryUploadProgressCenter::key();
                let mut updates = StoryUploadProgressCenter::hub().updates_for(key).await;
                while let Some(progress) = updates.next().await {
                    let next = progress.map(|progress| progress.fraction()).unwrap_or(0.0);
                    let _ = fraction_state.update(cx, |fraction, cx| {
                        if (next * 100.0) as i64 != (*fraction * 100.0) as i64 {
                            *fraction = next;
                            cx.notify();
                        }
                    });
                }
            });
        }

        let on_cancel = self.on_cancel.clone();
        let cancel_button = plain(self.child_id("cancel"))
            .on_press(move |window, cx| on_cancel(window, cx))
            .size(px(Self::HIT_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .clip_shape(Shape::Circle)
            .child(
                div()
                    .relative()
                    .size(px(Self::RING_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().absolute().inset_0().rounded_full().bg(black().opacity(0.48)))
                    .child(
                        div().absolute().inset_0().child(
                            LoaderUi::new()
                                .size(LoaderUiSize::Custom(Self::RING_SIZE))
                                .progress(shown_progress)
                                .r#type(LoaderUiType::Progress)
                                .thickness(2.6)
                                .track_color(Some(white().opacity(0.28)))
                                .ring_animated(true)
                                .speed(3.0)
                                .gap(3.0),
                        ),
                    )
                    .children(IconUi::sized("xmark", px(19.0), white(), false)),
            );

        let label = div()
            .px(px(12.0))
            .py(px(6.0))
            .rounded_full()
            .bg(black().opacity(0.45))
            .swiftui_font(Font::system_weight(13.0, FontWeight::SEMIBOLD))
            .text_color(white().opacity(0.95))
            .whitespace_nowrap()
            .text_ellipsis()
            .child(self.status_text(fraction));

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.0))
            .bg(black().opacity(0.28))
            .child(cancel_button)
            .child(label)
    }
}

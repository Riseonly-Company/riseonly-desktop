use std::collections::HashMap;

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_media::lottie::lottie_view::LottieView;
use rise_theme::AppTheme;
use rise_ui::{BoxUi, ButtonTone, ButtonUi, MainText, TextTone};

use crate::core::animations;

use super::slides::onboarding_slide::SLIDES;

/// The first screen, and the only one that runs before there is an account.
pub struct Onboarding {
    current: usize,
    /// One player per slide, built once. Rebuilding on every slide change would
    /// re-rasterise a whole timeline for a swipe the user can make four times in
    /// two seconds.
    animations: HashMap<&'static str, Entity<LottieView>>,
}

pub enum OnboardingEvent {
    /// Skip and "get started" are the same intent in the reference: both leave
    /// onboarding for sign-up. Kept as one event so the shell has one thing to
    /// route rather than two that must not diverge.
    Finished,
}

impl EventEmitter<OnboardingEvent> for Onboarding {}

impl Onboarding {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let side = rise_ui::theme(cx as &App).auth.slide_art_size;
        let scale = window.scale_factor();

        // Every slide's animation starts loading at once. Four of them, one
        // visible at a time, and the ones behind are what the user reaches in
        // the next second — waiting until they are on screen would show an empty
        // square for as long as a raster takes.
        let animations = SLIDES
            .iter()
            .map(|slide| {
                (
                    slide.animation,
                    animations::lottie(slide.animation, side, scale, cx),
                )
            })
            .collect();

        Self {
            current: 0,
            animations,
        }
    }

    pub fn current_slide(&self) -> usize {
        self.current
    }

    fn is_last(&self) -> bool {
        self.current + 1 == SLIDES.len()
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        if self.is_last() {
            cx.emit(OnboardingEvent::Finished);
            return;
        }
        self.current += 1;
        cx.notify();
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        cx.emit(OnboardingEvent::Finished);
    }

    /// The dots under the slide.
    ///
    /// The current one is a pill rather than a dot, which is the reference's
    /// treatment. It does not animate between the two: on gpui a permanently
    /// running transition costs compositor work that never shows up in a frame
    /// trace, and this one would run on the screen the user sees first.
    fn pagination(&self, theme: &AppTheme) -> impl IntoElement {
        let metrics = theme.auth;

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(metrics.pagination_gap)
            .children(SLIDES.iter().map(|slide| {
                let is_current = slide.index == self.current;
                div()
                    .h(metrics.pagination_dot_size)
                    .w(if is_current {
                        metrics.pagination_dot_active_width
                    } else {
                        metrics.pagination_dot_size
                    })
                    .rounded(theme.radius._600)
                    .bg(if is_current {
                        theme.primary._100
                    } else {
                        theme.border._200
                    })
            }))
    }
}

impl Render for Onboarding {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App).clone();
        let metrics = theme.auth;
        let slide = SLIDES[self.current.min(SLIDES.len() - 1)];

        let mut stage = div()
            .relative()
            .flex_1()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .px(metrics.content_padding);

        if let Some(animation) = self.animations.get(slide.animation) {
            stage = stage.child(animation.clone());
        }

        stage = stage.child(
            div()
                .mt(metrics.slide_art_gap)
                .max_w(metrics.content_width)
                .flex()
                .flex_col()
                .items_center()
                .text_center()
                .gap(metrics.slide_text_gap)
                .child(
                    div()
                        .text_size(theme.typography.display().size)
                        .line_height(theme.typography.display().line_height)
                        .font(theme.typography.display().font)
                        .text_color(theme.text.primary)
                        .child(tr(slide.title_key)),
                )
                .child(
                    MainText::body(&theme, TextTone::Secondary).child(tr(slide.description_key)),
                ),
        );

        if !self.is_last() {
            stage = stage.child(
                div()
                    .id("onboarding.skip")
                    .absolute()
                    .top(metrics.skip_inset)
                    .right(metrics.skip_inset)
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.skip(cx)))
                    .child(MainText::body(&theme, TextTone::Secondary).child(tr("skip"))),
            );
        }

        BoxUi::screen(&theme)
            .flex()
            .flex_col()
            .items_center()
            .child(stage)
            .child(
                div()
                    .my(metrics.pagination_margin)
                    .child(self.pagination(&theme)),
            )
            .child(
                div()
                    .id("onboarding.next")
                    .w(metrics.content_width)
                    .max_w_full()
                    .px(metrics.content_padding)
                    .mb(metrics.pagination_margin)
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.advance(cx)))
                    .child(
                        // The same height as a form field, from the same token:
                        // the primary action is never a different size from the
                        // control above it.
                        ButtonUi::sized(&theme, ButtonTone::Primary, metrics.field_height)
                            .w_full()
                            .child(tr(if self.is_last() { "get_started" } else { "next" })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use rise_theme::AppTheme;

    fn onboarding(cx: &mut TestAppContext) -> gpui::WindowHandle<Onboarding> {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });
        cx.add_window(Onboarding::new)
    }

    #[gpui::test]
    fn next_walks_the_slides_and_stops_at_the_last_one(cx: &mut TestAppContext) {
        let window = onboarding(cx);

        for expected in 1..SLIDES.len() {
            window
                .update(cx, |view, _, cx| view.advance(cx))
                .expect("window is open");
            assert_eq!(
                window
                    .update(cx, |view, _, _| view.current_slide())
                    .unwrap(),
                expected
            );
        }

        window.update(cx, |view, _, cx| view.advance(cx)).unwrap();
        assert_eq!(
            window
                .update(cx, |view, _, _| view.current_slide())
                .unwrap(),
            SLIDES.len() - 1,
            "the last slide hands over rather than walking past the end of the array"
        );
    }

    #[gpui::test]
    fn the_skip_control_is_gone_on_the_last_slide(cx: &mut TestAppContext) {
        let window = onboarding(cx);

        assert!(!window.update(cx, |view, _, _| view.is_last()).unwrap());
        for _ in 1..SLIDES.len() {
            window.update(cx, |view, _, cx| view.advance(cx)).unwrap();
        }
        assert!(
            window.update(cx, |view, _, _| view.is_last()).unwrap(),
            "skip and get-started would otherwise sit on the same screen saying the same thing"
        );
    }

    #[gpui::test]
    fn every_slide_has_its_own_player_and_they_are_built_once(cx: &mut TestAppContext) {
        let window = onboarding(cx);

        let ids = |cx: &mut TestAppContext| {
            window
                .update(cx, |view, _, _| {
                    let mut ids: Vec<_> = view
                        .animations
                        .values()
                        .map(gpui::Entity::entity_id)
                        .collect();
                    ids.sort();
                    ids
                })
                .unwrap()
        };

        let before = ids(cx);
        assert_eq!(before.len(), SLIDES.len());

        window.update(cx, |view, _, cx| view.advance(cx)).unwrap();
        assert_eq!(
            ids(cx),
            before,
            "changing slide must not re-rasterise a whole timeline"
        );
    }
}

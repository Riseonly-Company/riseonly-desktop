use gpui::{App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, Rgba, Styled, Window, div, px};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::angular_gradient::{AngularGradient, angular_gradient_circle_stroke};
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, pass_through};

use crate::modules::story::components::story_upload_avatar_ring::StoryUploadAvatarRing;

/// ring around an author avatar, publishing arc while uploading
/// theme gradient unseen and grey seen, nothing without a story
#[derive(IntoElement)]
pub struct StoryAvatarRing {
    id: ElementId,
    pub is_uploading: bool,
    pub has_story: bool,
    pub has_unseen_story: bool,
    pub size: f32,
    pub line_width: f32,
}

impl StoryAvatarRing {
    /// id keys the publishing arc own state
    pub fn new(
        id: impl Into<ElementId>,
        is_uploading: bool,
        has_story: bool,
        has_unseen_story: bool,
        size: f32,
    ) -> Self {
        Self {
            id: id.into(),
            is_uploading,
            has_story,
            has_unseen_story,
            size,
            line_width: 2.5,
        }
    }

    pub fn line_width(mut self, line_width: f32) -> Self {
        self.line_width = line_width;
        self
    }

    fn gradient_colors(cx: &mut App) -> Vec<Hsla> {
        let primary = &ThemeStore::shared(cx).read(cx).current_theme().primary;
        vec![primary._100, primary._200, primary._300, primary._100]
    }

    fn primary_gradient(cx: &mut App) -> AngularGradient {
        AngularGradient::new(Self::gradient_colors(cx))
    }

    fn viewed_ring_color() -> Hsla {
        Rgba { r: 0.24, g: 0.24, b: 0.26, a: 1.0 }.into()
    }
}

impl RenderOnce for StoryAvatarRing {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content = if self.is_uploading {
            Some(
                div()
                    .size(px(self.size))
                    .child(StoryUploadAvatarRing::new(
                        self.id.clone(),
                        self.line_width,
                        Self::gradient_colors(cx),
                        Self::viewed_ring_color().opacity(0.7),
                    ))
                    .into_any_element(),
            )
        } else if self.has_story {
            if self.has_unseen_story {
                Some(
                    angular_gradient_circle_stroke(
                        Self::primary_gradient(cx),
                        self.line_width,
                        (self.size - self.line_width).max(0.0),
                    )
                    .into_any_element(),
                )
            } else {
                Some(
                    div()
                        .relative()
                        .size(px(self.size))
                        .child(
                            Shape::Circle
                                .round(div().absolute().inset_0())
                                .stroke_border(Self::viewed_ring_color(), self.line_width),
                        )
                        .into_any_element(),
                )
            }
        } else {
            None
        };

        pass_through()
            .relative()
            .size(px(self.size))
            .flex()
            .items_center()
            .justify_center()
            .children(content)
    }
}

use gpui::{AnyElement, App, Div, IntoElement, ParentElement, RenderOnce, StyleRefinement, Styled, Window, div};
use rise_ui::swiftui::prelude::*;

use crate::modules::comment::stores::comment_services::comments_sheet_motion_relay::CommentsSheetMotionRelay;

/// per frame opacity driven by the comments sheet motion relay
/// leaf on purpose, the only view reading the relay so a drag re-renders this node alone
#[derive(IntoElement)]
pub struct CommentsSheetProgressFade {
    engaged: bool,
    hides_with_sheet: bool,
    base: Div,
}

impl Default for CommentsSheetProgressFade {
    fn default() -> Self {
        Self::new()
    }
}

impl CommentsSheetProgressFade {
    pub fn new() -> Self {
        Self { engaged: true, hides_with_sheet: true, base: div() }
    }

    pub fn engaged(mut self, engaged: bool) -> Self {
        self.engaged = engaged;
        self
    }

    pub fn hides_with_sheet(mut self, hides_with_sheet: bool) -> Self {
        self.hides_with_sheet = hides_with_sheet;
        self
    }
}

impl Styled for CommentsSheetProgressFade {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for CommentsSheetProgressFade {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for CommentsSheetProgressFade {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let progress = if self.engaged { CommentsSheetMotionRelay::shared(cx).read(cx).progress() } else { 0.0 };
        let opacity = if self.hides_with_sheet { 1.0 - progress } else { progress };
        self.base.relative().opacity(opacity).when(opacity <= 0.5, |content| {
            content.child(div().absolute().inset_0().block_mouse_except_scroll())
        })
    }
}

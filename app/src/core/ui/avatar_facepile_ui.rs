use gpui::{App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::shapes::{Shape, centred_stroke};

use crate::core::ui::user_logo_ui::UserLogoUi;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AvatarFacepileUiFace {
    pub id: String,
    pub logo: Option<String>,
    pub name: String,
}

impl AvatarFacepileUiFace {
    pub fn new(id: impl Into<String>, logo: Option<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            logo,
            name: name.into(),
        }
    }
}

/// overlapping avatars, who is here in one row
/// shared because the layout existed twice with different numbers like post likes and voice chat
/// face type isnt tied to a domain model so a third caller doesnt need a converter
#[derive(IntoElement)]
pub struct AvatarFacepileUi {
    faces: Vec<AvatarFacepileUiFace>,
    max_count: i64,
    size: f32,
    step: f32,
    ring_color: Option<Hsla>,
}

impl AvatarFacepileUi {
    pub fn new(faces: Vec<AvatarFacepileUiFace>) -> Self {
        Self {
            faces,
            max_count: 3,
            size: 23.0,
            step: 15.0,
            ring_color: None,
        }
    }

    pub fn max_count(mut self, max_count: i64) -> Self {
        self.max_count = max_count;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    pub fn ring_color(mut self, ring_color: impl Into<Option<Hsla>>) -> Self {
        self.ring_color = ring_color.into();
        self
    }

    fn visible(&self) -> &[AvatarFacepileUiFace] {
        let count = usize::try_from(self.max_count.max(0)).unwrap_or(0);
        &self.faces[..self.faces.len().min(count)]
    }
}

impl RenderOnce for AvatarFacepileUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ring = self.ring_color.unwrap_or_else(|| {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().bg._100
        });
        let shown = self.visible();
        let width = if shown.is_empty() {
            0.0
        } else {
            self.size + (shown.len() - 1) as f32 * self.step
        };

        let faces = shown.iter().enumerate().rev().map(|(index, face)| {
            div()
                .absolute()
                .left(px(index as f32 * self.step))
                .top(px(0.0))
                .size(px(self.size))
                .child(
                    UserLogoUi::new(ElementId::Name(SharedString::from(format!("facepile-{}", face.id))))
                        .source(face.logo.clone().map(SharedString::from))
                        .name(Some(SharedString::from(face.name.clone())))
                        .user_id(Some(SharedString::from(face.id.clone())))
                        .size(self.size)
                        .show_border(true)
                        .show_story_ring(false)
                        .open_story_on_press(false),
                )
                .child(centred_stroke(ring, 1.0, Shape::Circle))
        });

        div()
            .relative()
            .w(px(width))
            .h(px(self.size))
            .overflow_hidden()
            .children(faces)
    }
}

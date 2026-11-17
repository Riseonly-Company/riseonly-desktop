use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div,
    px, transparent_black, white,
};
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::swiftui::frame::SwiftUiAlignedFrame;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::stacks::{Alignment, zstack};

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::notify::stores::notify_actions::notify_types::NotificationActorItem;

/// what an actorless row draws instead of the anonymous face
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationAvatarGlyph {
    pub symbol: String,
    pub background: Hsla,
    pub foreground: Hsla,
}

#[derive(IntoElement)]
pub struct NotificationAvatarStack {
    id: ElementId,
    actors: Vec<NotificationActorItem>,
    badge_symbol: Option<String>,
    badge_color: Hsla,
    container_size: f32,
    empty_glyph: Option<NotificationAvatarGlyph>,
}

impl NotificationAvatarStack {
    pub fn new(id: impl Into<ElementId>, actors: Vec<NotificationActorItem>) -> Self {
        Self {
            id: id.into(),
            actors,
            badge_symbol: None,
            badge_color: transparent_black(),
            container_size: 46.0,
            empty_glyph: None,
        }
    }

    pub fn badge_symbol(mut self, badge_symbol: impl Into<Option<String>>) -> Self {
        self.badge_symbol = badge_symbol.into();
        self
    }

    pub fn badge_color(mut self, badge_color: Hsla) -> Self {
        self.badge_color = badge_color;
        self
    }

    pub fn container_size(mut self, container_size: f32) -> Self {
        self.container_size = container_size;
        self
    }

    pub fn empty_glyph(mut self, empty_glyph: impl Into<Option<NotificationAvatarGlyph>>) -> Self {
        self.empty_glyph = empty_glyph.into();
        self
    }

    fn draws_badge(&self) -> bool {
        !(self.actors.is_empty() && self.empty_glyph.is_some())
    }

    fn faces(&self, theme: &AppTheme) -> AnyElement {
        let ring = theme.bg._100;
        let size = self.container_size;
        match self.actors.len().min(3) {
            0 => {
                let glyph = self.empty_glyph.as_ref();
                let symbol = glyph.map_or("person.fill", |glyph| glyph.symbol.as_str());
                let foreground = glyph.map_or(theme.text.secondary, |glyph| glyph.foreground);
                zstack(Alignment::Center)
                    .size(px(size))
                    .child(div().size_full().circle().bg(glyph.map_or(theme.bg._300, |glyph| glyph.background)))
                    .children(IconUi::sized(symbol, px(size * 0.42), foreground, is_filled_symbol(symbol)))
                    .into_any_element()
            }
            1 => self.avatar(0, size, ring, 0.0),
            2 => {
                let side = size * 0.66;
                zstack(Alignment::Center)
                    .size(px(size))
                    .child(self.avatar(0, side, ring, 1.5).frame_max(Alignment::TopLeading))
                    .child(self.avatar(1, side, ring, 1.5).frame_max(Alignment::BottomTrailing))
                    .into_any_element()
            }
            _ => {
                let side = size * 0.58;
                zstack(Alignment::Center)
                    .size(px(size))
                    .child(self.avatar(0, side, ring, 1.5).frame_max_width(Alignment::Center).frame_max_height(Alignment::Top))
                    .child(self.avatar(1, side, ring, 1.5).frame_max(Alignment::BottomLeading))
                    .child(self.avatar(2, side, ring, 1.5).frame_max(Alignment::BottomTrailing))
                    .into_any_element()
            }
        }
    }

    fn avatar(&self, index: usize, size: f32, ring: Hsla, ring_width: f32) -> AnyElement {
        let Some(actor) = self.actors.get(index) else {
            return div().into_any_element();
        };
        let source = actor.actor_avatar.clone().filter(|avatar| !avatar.is_empty()).map(SharedString::from);
        let name = if actor.actor_name.is_empty() { actor.actor_tag.clone() } else { actor.actor_name.clone() };
        let user_id = (!actor.actor_id.is_empty()).then(|| SharedString::from(actor.actor_id.clone()));
        let logo = UserLogoUi::new(ElementId::NamedChild(
            Arc::new(self.id.clone()),
            SharedString::from(format!("avatar-{index}")),
        ))
        .source(source)
        .name(Some(SharedString::from(name)))
        .user_id(user_id)
        .size(size)
        .show_border(false);
        let face = div().relative().size(px(size)).child(div().size_full().clip_shape(Shape::Circle).child(logo));
        if ring_width > 0.0 {
            face.child(centred_stroke(ring, ring_width, Shape::Circle)).into_any_element()
        } else {
            face.into_any_element()
        }
    }

    fn badge(&self, symbol: &str, theme: &AppTheme) -> AnyElement {
        let side = 18.0;
        zstack(Alignment::Center)
            .relative()
            .left(px(3.0))
            .top(px(3.0))
            .size(px(side))
            .child(div().size_full().circle().bg(self.badge_color))
            .child(div().relative().size_full().child(centred_stroke(theme.bg._100, 2.0, Shape::Circle)))
            .children(IconUi::sized(symbol, px(9.0), white(), is_filled_symbol(symbol)))
            .into_any_element()
    }
}

fn is_filled_symbol(symbol: &str) -> bool {
    symbol.ends_with(".fill") || symbol.contains(".fill.")
}

impl RenderOnce for NotificationAvatarStack {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let size = self.container_size;
        let badge = match self.badge_symbol.as_deref() {
            Some(symbol) if self.draws_badge() => Some(self.badge(symbol, &theme)),
            _ => None,
        };
        zstack(Alignment::BottomTrailing)
            .size(px(size))
            .child(div().size(px(size)).child(self.faces(&theme)))
            .children(badge)
    }
}

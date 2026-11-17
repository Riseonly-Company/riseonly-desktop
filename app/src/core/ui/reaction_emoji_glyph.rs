use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::ThemeStore;
use rise_ui::swiftui::lifecycle::use_task;

use crate::core::ui::custom_emoji_glyph::{CustomEmojiGlyph, CustomEmojiGlyphMetrics};
use crate::core::ui::custom_emoji_resource_warmup::CustomEmojiResourceWarmup;
use crate::core::ui::custom_emoji_stickers_hooks::custom_emoji_stickers_hooks;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;

type ReactionEmojiFallback = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(IntoElement)]
pub struct ReactionEmojiGlyph {
    pub value: SharedString,
    pub size: f32,
    pub is_playing: bool,
    fallback: Option<ReactionEmojiFallback>,
}

impl ReactionEmojiGlyph {
    pub fn new(value: impl Into<SharedString>, size: f32) -> Self {
        Self {
            value: value.into(),
            size,
            is_playing: true,
            fallback: None,
        }
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn fallback<E: IntoElement>(mut self, fallback: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self {
        self.fallback = Some(Rc::new(move |window, cx| fallback(window, cx).into_any_element()));
        self
    }

    fn fallback_element(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        match &self.fallback {
            Some(fallback) => fallback(window, cx),
            None => ReactionEmojiPlaceholder::new(self.size).into_any_element(),
        }
    }
}

impl RenderOnce for ReactionEmojiGlyph {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let trimmed = trimming_whitespaces_and_newlines(&self.value);
        if let Some(id) = trimmed.strip_prefix(MessageReaction::CUSTOM_EMOJI_PREFIX) {
            if let Some(item) = custom_emoji_stickers_hooks().and_then(|hooks| (hooks.custom_emoji_item)(id, cx)) {
                return CustomEmojiGlyph::new(item, CustomEmojiGlyphMetrics::ink_height(self.size))
                    .is_playing(self.is_playing)
                    .into_any_element();
            }
            let warmup = CustomEmojiResourceWarmup::shared(cx);
            let revision = warmup.read(cx).resource_revision(cx);
            let id = id.to_string();
            use_task(
                ElementId::Name(SharedString::from(format!("reaction_emoji_glyph/{id}"))),
                revision,
                window,
                cx,
                async move |cx| {
                    let _ = warmup.update(cx, |warmup, cx| warmup.ensure_custom_emoji(&id, cx));
                },
            );
            return self.fallback_element(window, cx);
        }
        if trimmed.is_empty() {
            return self.fallback_element(window, cx);
        }
        div()
            .text_size(px(self.size))
            .child(SharedString::from(trimmed.to_string()))
            .into_any_element()
    }
}

/// neutral stand in for a reference this client cant resolve yet
/// not a question mark or a default emoji since a wrong glyph reads as the wrong reaction
#[derive(IntoElement)]
pub struct ReactionEmojiPlaceholder {
    pub size: f32,
}

impl ReactionEmojiPlaceholder {
    pub fn new(size: f32) -> Self {
        Self { size }
    }
}

impl RenderOnce for ReactionEmojiPlaceholder {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let fill = ThemeStore::shared(cx).read(cx).current_theme().bg._300.opacity(0.6);
        div()
            .w(px(self.size))
            .h(px(self.size))
            .flex_none()
            .rounded(px(self.size * 0.22))
            .bg(fill)
    }
}

use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AsyncWindowContext, ElementId, Entity, FontWeight, Hsla, ImageSource, IntoElement, ObjectFit, ParentElement, RenderOnce,
    SharedString, Styled, StyledImage, Window, div, img, px,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::theme::app_theme::AppTheme;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::stores::message::message_actions::message_types::MessageStoryReply;
use crate::modules::story::shared::story_poster_cache::StoryPosterCache;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

pub type MessageStoryReplyHeaderOpenStory = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct MessageStoryReplyHeader {
    id: ElementId,
    snapshot: MessageStoryReply,
    is_own: bool,
    on_open_story: Option<MessageStoryReplyHeaderOpenStory>,
    max_width: Option<f32>,
}

impl MessageStoryReplyHeader {
    const THUMBNAIL_PIXEL_DIMENSION: i64 = 96;

    pub fn new(id: impl Into<ElementId>, story: MessageStoryReply) -> Self {
        Self { id: id.into(), snapshot: story, is_own: false, on_open_story: None, max_width: None }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn on_open_story(mut self, on_open_story: impl Into<Option<MessageStoryReplyHeaderOpenStory>>) -> Self {
        self.on_open_story = on_open_story.into();
        self
    }

    /// bubble content width
    /// a long author name answers its whole unwrapped line so without a cap the quote ran past the edge
    pub fn max_width(mut self, max_width: Option<f32>) -> Self {
        self.max_width = max_width.filter(|width| *width > 0.0);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    /// the snapshot plus what this session learned since
    /// a deleted story makes the quote unavailable without a tap
    fn story(&self, cx: &mut App) -> MessageStoryReply {
        let mut resolved = self.snapshot.clone();
        let story_id = self.snapshot.story_id;
        resolved.is_known_gone = StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.is_story_gone(story_id, cx));
        resolved
    }

    fn poster_task_key(&self, story: &MessageStoryReply) -> String {
        format!("{}|{}", self.snapshot.story_id, story.video_poster_source().unwrap_or_default())
    }

    async fn load_poster_if_needed(
        poster: Entity<Option<CleverImage>>,
        story_id: i64,
        source: Option<String>,
        cx: &mut AsyncWindowContext,
    ) {
        let Some(source) = source else {
            return;
        };
        if let Some(hit) = StoryPosterCache::cached(story_id) {
            poster.update(cx, |poster, cx| {
                *poster = Some(hit);
                cx.notify();
            });
            return;
        }
        let generated = StoryPosterCache::poster(story_id, &source, Self::THUMBNAIL_PIXEL_DIMENSION, &**cx).await;
        let Some(generated) = generated else {
            return;
        };
        poster.update(cx, |poster, cx| {
            *poster = Some(generated);
            cx.notify();
        });
    }

    fn quote(&self, story: &MessageStoryReply, theme: &AppTheme, accent: Hsla, poster: Option<CleverImage>) -> gpui::Div {
        let muted = self.muted_color(theme);
        hstack(6.0)
            .items_center()
            .w_full()
            .when_some(self.max_width, |row, width| row.max_w(px(width)))
            .min_h(px(34.0))
            .child(div().flex_none().w(px(3.0)).h(px(34.0)).rounded(px(1.5)).bg(accent))
            .child(div().flex_none().child(self.thumbnail(story, accent, poster)))
            .child(
                vstack(1.0)
                    .items_start()
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .min_w_0()
                    .child(
                        MainText::new(Self::author_name(story))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(accent)
                            .number_of_lines(1),
                    )
                    .child(
                        hstack(4.0)
                            .min_w_0()
                            .children(IconUi::sized("arrowshape.turn.up.left.circle.fill", px(13.0), muted, true))
                            .child(MainText::new(Self::label(story)).px(14.0).color(muted).number_of_lines(1)),
                    ),
            )
            .child(spacer_min_w(0.0))
    }

    /// a story has no server poster, video shows its generated first frame
    /// placeholder until ready, a failed load stays an empty tile
    /// known dead story keeps the clock tile
    fn thumbnail(&self, story: &MessageStoryReply, accent: Hsla, poster: Option<CleverImage>) -> gpui::AnyElement {
        if story.thumbnail_source().is_none()
            && story.video_poster_source().is_some()
            && let Some(poster) = poster
        {
            return img(ImageSource::Render(poster))
                .object_fit(ObjectFit::Cover)
                .w(px(32.0))
                .h(px(32.0))
                .continuous_corners(4.0)
                .into_any_element();
        }
        match story.thumbnail_source() {
            Some(source) => div()
                .size(px(32.0))
                .continuous_corners(4.0)
                .overflow_hidden()
                .child(
                    CleverImageUi::new(source)
                        .id(self.child_id("thumbnail"))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(4.0)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(Self::THUMBNAIL_PIXEL_DIMENSION)
                        .shows_failure_icon(false),
                )
                .into_any_element(),
            None => div()
                .size(px(32.0))
                .continuous_corners(4.0)
                .bg(accent.opacity(0.18))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized(
                    if story.is_unavailable() { "clock" } else { "play.fill" },
                    px(13.0),
                    accent,
                    !story.is_unavailable(),
                ))
                .into_any_element(),
        }
    }

    fn muted_color(&self, theme: &AppTheme) -> Hsla {
        if self.is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary }
    }

    fn author_name(story: &MessageStoryReply) -> String {
        let name = trimming_whitespaces_and_newlines(&story.author.name);
        if !name.is_empty() {
            return name.to_owned();
        }
        let tag = trimming_whitespaces_and_newlines(&story.author.tag);
        if tag.is_empty() { tr("chat_story_reply_label") } else { format!("@{tag}") }
    }

    fn label(story: &MessageStoryReply) -> String {
        if story.is_unavailable() {
            tr("chat_story_reply_expired")
        } else {
            tr("chat_story_reply_label")
        }
    }
}

impl RenderOnce for MessageStoryReplyHeader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme_store(cx).read(cx).current_theme().clone();
        let accent = if self.is_own { theme.content_on(theme.primary._100).opacity(0.85) } else { theme.primary._100 };
        let story = self.story(cx);

        let story_id = self.snapshot.story_id;
        let initial_poster = self.snapshot.is_video;
        let poster: Entity<Option<CleverImage>> = window.use_keyed_state(self.child_id("poster"), cx, move |_, _| {
            if initial_poster { StoryPosterCache::cached(story_id) } else { None }
        });
        let task_poster = poster.clone();
        let poster_source = story.video_poster_source();
        use_task(self.child_id("poster-task"), self.poster_task_key(&story), window, cx, async move |cx| {
            Self::load_poster_if_needed(task_poster, story_id, poster_source, cx).await;
        });
        let shown_poster = poster.read(cx).clone();

        let quote = self.quote(&story, &theme, accent, shown_poster);
        match self.on_open_story.clone() {
            Some(on_open_story) if story.is_openable() => {
                quote.bubble_tap_target_priority(self.child_id("open"), move |window, cx| on_open_story(window, cx)).into_any_element()
            }
            _ => quote.into_any_element(),
        }
    }
}

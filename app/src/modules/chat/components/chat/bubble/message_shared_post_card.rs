use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled,
    Window, black, div, px, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::button_style::{ButtonAction, plain};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::MessageSharedPost;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post_visibility_reader::PostVisibilityReader;
use crate::modules::post::engine::rise_post_engine_models::RisePostViewSurface;

#[derive(IntoElement)]
pub struct MessageSharedPostCard {
    id: ElementId,
    post: MessageSharedPost,
    is_own: bool,
    on_open_comments: Option<ButtonAction>,
}

impl MessageSharedPostCard {
    pub fn new(id: impl Into<ElementId>, post: MessageSharedPost) -> Self {
        Self { id: id.into(), post, is_own: false, on_open_comments: None }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn on_open_comments(mut self, on_open_comments: impl Into<Option<ButtonAction>>) -> Self {
        self.on_open_comments = on_open_comments.into();
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn thumbnail(&self, cx: &mut App) -> Option<gpui::Div> {
        let thumb = self.post.display_thumbnail().filter(|thumb| !thumb.is_empty())?;
        let is_own_post = PostMediaCachePolicy::is_own(None, Some(self.post.author.id.as_str()), cx);
        let (width, height) = if self.post.is_short { (54.0, 72.0) } else { (56.0, 56.0) };
        Some(
            div()
                .relative()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .frame_size(width, height)
                .clip_shape(Shape::continuous(10.0))
                .child(
                    div().absolute().inset_0().child(
                        CleverImageUi::new(thumb)
                            .id(self.child_id("thumbnail"))
                            .content_mode(ContentMode::Fill)
                            .corner_radius(10.0)
                            .persist_to_disk(is_own_post)
                            .maximum_decode_pixel_dimension(216_i64)
                            .cache_policy(PostMediaCachePolicy::image_policy(is_own_post)),
                    ),
                )
                .when(self.post.is_short, |thumbnail| {
                    thumbnail.child(
                        div()
                            .relative()
                            .p(px(7.0))
                            .rounded_full()
                            .bg(black().opacity(0.42))
                            .children(IconUi::sized("play.fill", px(13.0), white(), true)),
                    )
                }),
        )
    }

    fn stats_row(&self, theme: &AppTheme) -> gpui::Div {
        let color = if self.is_own { theme.content_on(theme.primary._100).opacity(0.72) } else { theme.text.secondary };
        hstack(12.0).child(Self::stat("heart.fill", self.post.likes_count, color)).child(self.comments_stat(color))
    }

    fn comments_stat(&self, color: Hsla) -> gpui::AnyElement {
        let content = Self::stat("bubble.right.fill", self.post.comments_count, color);
        match self.on_open_comments.clone() {
            Some(on_open_comments) => content
                .id(self.child_id("comments"))
                .occlude()
                .cursor_pointer()
                .aria_label(tr("navbtn_comments"))
                .bubble_tap_target_priority(self.child_id("comments-tap"), move |window: &mut Window, cx: &mut App| {
                    on_open_comments(window, cx)
                })
                .into_any_element(),
            None => content.into_any_element(),
        }
    }

    fn stat(icon: &'static str, count: i64, color: Hsla) -> gpui::Div {
        hstack(3.0).children(IconUi::sized(icon, px(10.5), color, true)).child(
            MainText::new(SharedPostCount::short(count)).px(11.5).font_weight(FontWeight::SEMIBOLD).color(color),
        )
    }

    fn title(&self) -> &str {
        trimming_whitespaces_and_newlines(&self.post.title)
    }

    fn snippet(&self) -> &str {
        trimming_whitespaces_and_newlines(&self.post.snippet)
    }
}

impl RenderOnce for MessageSharedPostCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let accent = if self.is_own { theme.content_on(theme.primary._100).opacity(0.85) } else { theme.primary._100 };
        let nav_store = env::<NavigationStoreKey>(cx);
        let title = self.title().to_owned();
        let snippet = self.snippet().to_owned();
        let thumbnail = self.thumbnail(cx);
        let open_post = self.post.clone();
        let has_post_id = !self.post.post_id.is_empty();

        div()
            .relative()
            .w_full()
            .max_w(px(280.0))
            .child(
                div().absolute().inset_0().child(
                    PostVisibilityReader::new(
                        self.child_id("visibility"),
                        self.post.post_id.clone(),
                        RisePostViewSurface::SharedPost,
                    )
                    .enabled(has_post_id),
                ),
            )
            .child(
                plain(self.child_id("button"))
                    .relative()
                    .w_full()
                    .on_press(move |_: &mut Window, cx: &mut App| {
                        if open_post.post_id.is_empty() {
                            return;
                        }
                        if let Some(nav_store) = nav_store.as_ref() {
                            let route = open_post.open_route();
                            nav_store.update(cx, |store, cx| {
                                store.push(route, NavigationStoreSettlingPush::default(), cx);
                            });
                        }
                    })
                    .child(
                        hstack(10.0)
                            .w_full()
                            .child(div().flex_none().self_stretch().w(px(3.0)).continuous_corners(1.5).bg(accent))
                            .child(
                                vstack(4.0)
                                    .align(HorizontalAlignment::Leading)
                                    .flex_shrink(1.0)
                                    .min_w_0()
                                    .child(
                                        MainText::new(if self.post.is_riff {
                                            tr("riffs")
                                        } else {
                                            tr("shared_post_label")
                                        })
                                        .px(MessageSizes::link_preview_site_font_size())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(accent)
                                        .number_of_lines(1),
                                    )
                                    .when(!title.is_empty(), |texts| {
                                        texts.child(
                                            MainText::new(title.clone())
                                                .px(MessageSizes::link_preview_title_font_size())
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary })
                                                .number_of_lines(2),
                                        )
                                    })
                                    .when(!snippet.is_empty(), |texts| {
                                        texts.child(
                                            MainText::new(snippet.clone())
                                                .px(MessageSizes::link_preview_description_small_font_size())
                                                .font_weight(FontWeight::NORMAL)
                                                .color(if self.is_own {
                                                    theme.content_on(theme.primary._100).opacity(0.78)
                                                } else {
                                                    theme.text.secondary
                                                })
                                                .number_of_lines(2),
                                        )
                                    })
                                    .child(self.stats_row(&theme).mt(px(1.0))),
                            )
                            .child(spacer_min_w(0.0))
                            .children(thumbnail),
                    ),
            )
    }
}

pub enum SharedPostCount {}

impl SharedPostCount {
    pub fn short(n: i64) -> String {
        RiseFormat::compact_count(n)
    }
}

impl MessageSharedPost {
    pub fn open_route(&self) -> RootRoute {
        let feed_post = self.as_feed_post();
        if self.is_riff {
            RootRoute::ShortViewer { post: feed_post, origin: None, sound_track_id: None }
        } else {
            RootRoute::post_content(&feed_post, None)
        }
    }
}

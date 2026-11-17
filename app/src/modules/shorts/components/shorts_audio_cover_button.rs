use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, black,
    div, px, white,
};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::alpha;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};

use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

const COVER_SIZE: f32 = 38.0;
const IMAGE_CORNER_RADIUS: f32 = 8.0;
const CLIP_CORNER_RADIUS: f32 = 9.0;

#[derive(IntoElement)]
pub struct ShortsAudioCoverButton {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ShortsAudioCoverButton {
    pub fn new(
        id: impl Into<ElementId>,
        post: Rc<GetPostFeedResponse>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), post, action: Rc::new(action) }
    }

    fn uses_author_avatar(&self) -> bool {
        self.post.track_model().is_none_or(|track| {
            track.status == RiseMusicTrackStatus::OriginalAudio.wire() || track.cover_image_url.is_empty()
        })
    }

    fn author_logo(&self, cx: &mut App) -> String {
        let remote = trimming_whitespaces_and_newlines(&self.post.author.more.logo);
        if !remote.is_empty() {
            return remote.to_string();
        }
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        if !auth.is_current_user(&self.post.author_id) {
            return String::new();
        }
        let Some(profile) = auth.profile() else {
            return String::new();
        };
        profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone())
    }

    fn author_letter_tile(&self, author_logo: &str) -> AnyElement {
        let owner_id = AvatarPalette::generated_avatar_owner_id(Some(author_logo))
            .unwrap_or_else(|| self.post.author.id.clone().unwrap_or_else(|| self.post.author_id.clone()));
        let name = self.post.author.name.as_str();
        div()
            .relative()
            .size(px(COVER_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .bg(AvatarPalette::color_for_user(Some(owner_id.as_str()), Some(name)))
            .child(
                MainText::new(AvatarPalette::display_letter(Some(name)))
                    .px(17.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(AvatarPalette::letter_color_on(AvatarPalette::color_for_user(Some(owner_id.as_str()), Some(name))))
                    .minimum_scale_factor(0.55)
                    .line_limit(1),
            )
            .into_any_element()
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }
}

impl RenderOnce for ShortsAudioCoverButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut cover = div().relative().size(px(COVER_SIZE)).flex_none();
        if self.uses_author_avatar() {
            let author_logo = self.author_logo(cx);
            if AvatarPalette::resolved_logo_url(Some(author_logo.as_str())).is_none() {
                cover = cover.child(self.author_letter_tile(&author_logo));
            } else {
                cover = cover.child(
                    CleverImageUi::new(author_logo)
                        .id(self.child_id("avatar"))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(IMAGE_CORNER_RADIUS)
                        .persist_to_disk(true),
                );
            }
        } else if let Some(track) = self.post.track_model() {
            cover = cover.child(
                MusicTrackCoverImage::new(self.child_id("cover"), track.cover_image_url)
                    .size(COVER_SIZE)
                    .corner_radius(IMAGE_CORNER_RADIUS),
            );
        }

        let action = self.action.clone();
        let label = tr("original_audio_title");
        div()
            .id(self.id.clone())
            .tooltip_label(label)
            .child(
                plain(self.child_id("button"))
                    .on_press(move |window, cx| action(window, cx))
                    .relative()
                    .size(px(COVER_SIZE))
                    .rounded_rect(CLIP_CORNER_RADIUS)
                    .shadow(vec![gpui::BoxShadow {
                        color: alpha(black(), 0.38),
                        offset: gpui::point(px(0.0), px(1.0)),
                        blur_radius: px(3.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .child(div().size(px(COVER_SIZE)).rounded_rect(CLIP_CORNER_RADIUS).overflow_hidden().child(cover))
                    .child(centred_stroke(alpha(white(), 0.9), 1.5, Shape::continuous(CLIP_CORNER_RADIUS))),
            )
    }
}

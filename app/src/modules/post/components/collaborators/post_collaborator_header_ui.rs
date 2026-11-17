use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, white};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::prelude::*;
use rise_ui::{ButtonUi, MainText, TextAlignT};

use crate::core::ui::collaborator_avatars_ui::{CollaboratorAvatar, CollaboratorAvatarsUi};
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::post::components::collaborators::post_collaborators_presenter::{
    PostCollaboratorConfirmKind, PostCollaboratorsPresenter,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// coauthor header computed once per card
/// stack and names come from the feed body so no sorting or list walk
/// first two users and one count
#[derive(Clone, Debug, PartialEq)]
pub struct PostCollaboratorHeaderPresentation {
    pub author_avatar: CollaboratorAvatar,
    pub coauthor_avatars: Vec<CollaboratorAvatar>,
    pub total_count: i64,
    pub names_line: String,
}

impl PostCollaboratorHeaderPresentation {
    pub fn first_coauthor(&self) -> &CollaboratorAvatar {
        &self.coauthor_avatars[0]
    }

    pub fn make(post: &GetPostFeedResponse) -> Option<PostCollaboratorHeaderPresentation> {
        let coauthors = &post.coauthors;
        let first = coauthors.first()?;

        let author_avatar = CollaboratorAvatar::new(
            post.author.id.clone().unwrap_or_else(|| post.author_id.clone()),
            Some(post.author.more.logo.clone()),
            post.author.name.clone(),
            post.author.tag.clone(),
        );
        let mut visible =
            vec![CollaboratorAvatar::new(first.id.clone(), Some(first.logo.clone()), first.name.clone(), first.tag.clone())];
        if let Some(second) = coauthors.get(1) {
            visible.push(CollaboratorAvatar::new(
                second.id.clone(),
                Some(second.logo.clone()),
                second.name.clone(),
                second.tag.clone(),
            ));
        }

        let total_count = 1 + (coauthors.len() as i64).max(post.coauthors_count);

        Some(PostCollaboratorHeaderPresentation {
            author_avatar,
            coauthor_avatars: visible,
            total_count,
            names_line: CollaboratorAvatarsUi::names_line(
                &post.author.name,
                std::slice::from_ref(&first.name),
                total_count,
            ),
        })
    }
}

/// inline row above the actions, no card
/// declining stays in the post menu
#[derive(IntoElement)]
pub struct PostCoauthorInviteBanner {
    post: Rc<GetPostFeedResponse>,
    on_dark: bool,
    on_open_author: ButtonAction,
}

impl PostCoauthorInviteBanner {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into(), on_dark: false, on_open_author: Rc::new(|_, _| {}) }
    }

    pub fn on_dark(mut self, on_dark: bool) -> Self {
        self.on_dark = on_dark;
        self
    }

    pub fn on_open_author(mut self, on_open_author: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open_author = Rc::new(on_open_author);
        self
    }
}

impl RenderOnce for PostCoauthorInviteBanner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let post = self.post;
        let secondary = if self.on_dark { white().opacity(0.8) } else { theme.text.secondary };
        let author_id = post.author.id.clone().map(SharedString::from);
        let name_color = if self.on_dark {
            Some(white())
        } else {
            PremiumNameColor::color_for_user(post.author.id.as_deref(), Some(&post.author.more), cx)
        };
        let on_open_author = self.on_open_author;
        let accept_post_id = post.id.clone();

        let mut name_row = hstack(5.0).min_w_0().max_w_full().child(
            MainText::new(post.author.name.clone())
                .px(15.0)
                .tac(TextAlignT::Left)
                .font_weight(FontWeight::SEMIBOLD)
                .color(name_color)
                .number_of_lines(1),
        );
        if post.author.more.is_official {
            name_row = name_row.child(OfficialBadge::new().size(px(16.0)));
        }
        if post.author.more.is_premium {
            name_row = name_row.child(
                PremiumBadge::new()
                    .user_id(author_id.clone())
                    .emoji(post.author.more.premium_badge_emoji.clone().map(SharedString::from))
                    .size(16.0),
            );
        }

        let mut logo = UserLogoUi::new(ElementId::Name(format!("post-coauthor-invite-logo-{}", post.id).into()))
            .source(SharedString::from(post.author.more.logo.clone()))
            .name(SharedString::from(post.author.name.clone()))
            .is_deleted(post.author.is_deleted)
            .size(42.0)
            .show_story_ring(false)
            .open_story_on_press(false);
        if let Some(author_id) = author_id {
            logo = logo.user_id(author_id);
        }

        hstack(12.0)
            .w_full()
            .flex_wrap()
            .child(
                plain(ElementId::Name(format!("post-coauthor-invite-author-{}", post.id).into()))
                    .min_w_0()
                    .on_press(move |window, cx| on_open_author(window, cx))
                    .child(
                        hstack(10.0).min_w_0().child(logo).child(
                            vstack(3.0)
                                .align(HorizontalAlignment::Leading)
                                .min_w_0()
                                .max_w_full()
                                .child(name_row)
                                .child(
                                    MainText::new(tr("post_coauthor_invite_banner_subtitle"))
                                        .px(13.0)
                                        .tac(TextAlignT::Left)
                                        .color(secondary)
                                        .number_of_lines(2),
                                ),
                        ),
                    ),
            )
            .child(spacer_min_w(8.0))
            .child(
                div().flex_none().child(
                    ButtonUi::new(ElementId::Name(format!("post-coauthor-invite-accept-{}", post.id).into()))
                        .background_color(theme.primary._100)
                        .height(34.0)
                        .fit_content(true)
                        .padding_horizontal(16.0)
                        .on_press(move |_, cx| {
                            PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
                                presenter.ask(PostCollaboratorConfirmKind::AcceptInvite, &accept_post_id, cx);
                            });
                        })
                        .child(
                            MainText::new(tr("post_coauthor_accept_button"))
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(theme.primary._100))
                                .number_of_lines(1),
                        )
                        .guide_anchor(RiseGuideAnchor::PostCoauthorAccept),
                ),
            )
    }
}

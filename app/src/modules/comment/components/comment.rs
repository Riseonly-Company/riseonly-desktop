use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, Hsla, IntoElement, MouseButton, MouseDownEvent, Pixels, RenderOnce,
    SharedString, Size, Window, linear_color_stop, linear_gradient,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::pressable_ui::PressableUi;
use rise_ui::swiftui::font::NATURAL_LINE_HEIGHT;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, MediaViewerPresentationState, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerGalleryItemKind, MediaViewerHeroSkin,
};
use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::core::ui::formatted_text_ui::{FormattedTextOptions, FormattedTextStyle};
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::rich_text::rich_text_view::{RichTextTap, RichTextView};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_text::InlineCustomEmojiText;
use crate::modules::chat::components::chat::bubble::message_media_upload_overlay::MessageMediaUploadOverlay;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::ChatInputStickerAsset;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageCustomEmojiEntity, RemoteMediaURL};
use crate::modules::comment::components::first_comment_badge::FirstCommentBadge;
use crate::modules::comment::components::native_list::comment_list_models::CommentRowMetrics;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentMedia, CommentMediaType, CommentPhotoUpload, GetCommentsResponse,
};
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    PackKind, StickerAssetType, StickerAssetTypeInfer, StickerItem,
};

type CommentRowAction = Rc<dyn Fn(&mut Window, &mut App)>;
type CommentTextExpandedChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn no_action() -> CommentRowAction {
    Rc::new(|_: &mut Window, _: &mut App| {})
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CommentReplyActionStyle {
    #[default]
    Reply,
    Thread {
        count: i64,
    },
}

#[derive(IntoElement)]
pub struct CommentRow {
    id: ElementId,
    comment: GetCommentsResponse,
    show_actions: bool,
    reply_action_style: CommentReplyActionStyle,
    on_like: CommentRowAction,
    on_dislike: CommentRowAction,
    on_reply: CommentRowAction,
    on_avatar_profile: CommentRowAction,
    on_avatar_story: CommentRowAction,
    photo_upload: Option<CommentPhotoUpload>,
    on_cancel_photo_upload: CommentRowAction,
    on_mention_tap: Option<RichTextTap>,
    on_link_tap: Option<RichTextTap>,
    is_text_expanded: bool,
    on_text_expanded_change: Option<CommentTextExpandedChange>,
}

impl CommentRow {
    pub fn new(id: impl Into<ElementId>, comment: GetCommentsResponse) -> Self {
        Self {
            id: id.into(),
            comment,
            show_actions: true,
            reply_action_style: CommentReplyActionStyle::Reply,
            on_like: no_action(),
            on_dislike: no_action(),
            on_reply: no_action(),
            on_avatar_profile: no_action(),
            on_avatar_story: no_action(),
            photo_upload: None,
            on_cancel_photo_upload: no_action(),
            on_mention_tap: None,
            on_link_tap: None,
            is_text_expanded: false,
            on_text_expanded_change: None,
        }
    }

    pub fn show_actions(mut self, show_actions: bool) -> Self {
        self.show_actions = show_actions;
        self
    }

    pub fn reply_action_style(mut self, reply_action_style: CommentReplyActionStyle) -> Self {
        self.reply_action_style = reply_action_style;
        self
    }

    pub fn on_like(mut self, on_like: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_like = Rc::new(on_like);
        self
    }

    pub fn on_dislike(mut self, on_dislike: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dislike = Rc::new(on_dislike);
        self
    }

    pub fn on_reply(mut self, on_reply: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_reply = Rc::new(on_reply);
        self
    }

    pub fn on_avatar_profile(mut self, on_avatar_profile: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_avatar_profile = Rc::new(on_avatar_profile);
        self
    }

    pub fn on_avatar_story(mut self, on_avatar_story: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_avatar_story = Rc::new(on_avatar_story);
        self
    }

    pub fn photo_upload(mut self, photo_upload: Option<CommentPhotoUpload>) -> Self {
        self.photo_upload = photo_upload;
        self
    }

    pub fn on_cancel_photo_upload(mut self, on_cancel_photo_upload: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel_photo_upload = Rc::new(on_cancel_photo_upload);
        self
    }

    pub fn on_mention_tap(mut self, on_mention_tap: Option<RichTextTap>) -> Self {
        self.on_mention_tap = on_mention_tap;
        self
    }

    pub fn on_link_tap(mut self, on_link_tap: Option<RichTextTap>) -> Self {
        self.on_link_tap = on_link_tap;
        self
    }

    pub fn is_text_expanded(mut self, is_text_expanded: bool) -> Self {
        self.is_text_expanded = is_text_expanded;
        self
    }

    pub fn on_text_expanded_change(
        mut self,
        on_text_expanded_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_text_expanded_change = Some(Rc::new(on_text_expanded_change));
        self
    }

    fn resolved_author_id(&self) -> String {
        let stable = trimming_whitespaces_and_newlines(&self.comment.author_id);
        let nested = self.comment.author.id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
        if stable.is_empty() { nested.to_string() } else { stable.to_string() }
    }

    /// an erased account has no profile to open
    fn open_author_profile(&self) -> CommentRowAction {
        let is_deleted = self.comment.author.is_deleted;
        let on_avatar_profile = self.on_avatar_profile.clone();
        Rc::new(move |window: &mut Window, cx: &mut App| {
            if is_deleted {
                return;
            }
            on_avatar_profile(window, cx);
        })
    }

    fn comment_text(&self, text: String, entity_shift: i64, theme: &AppTheme) -> AnyElement {
        let emojis = self.shifted_custom_emojis(entity_shift);
        let style = FormattedTextStyle::app_standard_line_spacing(
            theme,
            CommentRowMetrics::TEXT_FONT_SIZE,
            CommentRowMetrics::TEXT_LINE_SPACING,
        );
        let id = child_id(&self.id, "text");
        if emojis.is_empty() {
            let mut view = RichTextView::new(id, text, style)
                .block_spacing(6.0)
                .code_font_size(12.5)
                .options(Self::text_options());
            if let Some(tap) = self.on_mention_tap.clone() {
                view = view.on_mention_tap(move |value, window, cx| tap(value, window, cx));
            }
            if let Some(tap) = self.on_link_tap.clone() {
                view = view.on_link_tap(move |value, window, cx| tap(value, window, cx));
            }
            view.into_any_element()
        } else {
            InlineCustomEmojiText::new(id, text, style, emojis)
                .options(Self::text_options())
                .on_mention_tap(self.on_mention_tap.clone())
                .on_link_tap(self.on_link_tap.clone())
                .into_any_element()
        }
    }

    /// links in a comment are plain text, not marked not tappable and no preview
    fn text_options() -> FormattedTextOptions {
        FormattedTextOptions { no_url_format: true, no_url_subtitle: true, ..FormattedTextOptions::default() }
    }

    fn shifted_custom_emojis(&self, shift: i64) -> Vec<MessageCustomEmojiEntity> {
        if self.comment.custom_emojis.is_empty() {
            return Vec::new();
        }
        if shift == 0 {
            return self.comment.custom_emojis.clone();
        }
        self.comment
            .custom_emojis
            .iter()
            .filter_map(|entity| {
                let offset = entity.offset + shift;
                if offset < 0 {
                    return None;
                }
                let mut shifted = entity.clone();
                shifted.offset = offset;
                Some(shifted)
            })
            .collect()
    }

    fn author_line(&self, theme: &AppTheme, cx: &App) -> impl IntoElement + use<> {
        let author = &self.comment.author;
        let open_author_profile = self.open_author_profile();
        let name = MainText::new(author.display_name())
            .px(CommentRowMetrics::AUTHOR_FONT_SIZE)
            .font_weight(FontWeight::SEMIBOLD)
            .color(PremiumNameColor::color_for_user(author.id.as_deref(), Some(&author.more), cx))
            .number_of_lines(1)
            .bubble_tap_target_priority(child_id(&self.id, "author-name"), move |window, cx| {
                open_author_profile(window, cx);
            });

        hstack(5.0)
            .w_full()
            .child(div().min_w_0().child(name))
            .when(author.more.is_official, |line| line.child(OfficialBadge::new().size(px(15.0))))
            .when(author.more.is_premium, |line| {
                line.child(
                    PremiumBadge::new()
                        .user_id(author.id.clone().map(SharedString::from))
                        .emoji(author.more.premium_badge_emoji.clone().map(SharedString::from))
                        .size(15.0),
                )
            })
            .child(spacer_min_w(0.0))
            .child(CommentLiveRelativeTime {
                id: child_id(&self.id, "time"),
                iso: self.comment.created_at.clone(),
                color: theme.text.secondary,
            })
    }

    fn moderation_placeholder(&self, theme: &AppTheme) -> impl IntoElement + use<> {
        hstack(5.0)
            .children(IconUi::sized("eye.slash", px(12.0), theme.text.secondary, false))
            .child(MainText::new(tr("comment_disabled_by_moderation")).px(13.0).color(theme.text.secondary).italic())
    }
}

impl RenderOnce for CommentRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let comment = &self.comment;
        let resolved_author_id = self.resolved_author_id();
        let (resolved_text, entity_shift) = CommentRowMetrics::resolved_body(comment);
        let is_deleted = comment.author.is_deleted;
        let open_author_profile = self.open_author_profile();

        let avatar_press = open_author_profile.clone();
        let avatar_open_profile = open_author_profile.clone();
        let avatar_story = self.on_avatar_story.clone();
        let avatar = UserLogoUi::new(child_id(&self.id, "avatar"))
            .source(SharedString::from(comment.author.more.logo.clone()))
            .name(SharedString::from(comment.author.display_name()))
            .user_id(SharedString::from(resolved_author_id.clone()))
            .is_deleted(is_deleted)
            .size(CommentRowMetrics::AVATAR_SIZE)
            .is_button(!is_deleted)
            .show_border(false)
            .show_avatar_context_menu(!is_deleted)
            .on_open_profile(move |window, cx| avatar_open_profile(window, cx))
            .on_open_story(move |_, window, cx| avatar_story(window, cx))
            .on_press(move |window, cx| avatar_press(window, cx))
            .bubble_tap_target_priority(child_id(&self.id, "avatar-tap"), move |window, cx| {
                open_author_profile(window, cx);
            })
            .gift_worn_frame(
                child_id(&self.id, "avatar-frame"),
                (!is_deleted).then(|| resolved_author_id.clone()),
                CommentRowMetrics::AVATAR_SIZE,
            );

        let is_moderation_disabled = comment.is_moderation_disabled();
        let body = vstack(CommentRowMetrics::BODY_SPACING)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .min_w_0()
            .child(self.author_line(&theme, cx))
            .when(is_moderation_disabled, |body| body.child(self.moderation_placeholder(&theme)))
            .when(!is_moderation_disabled && !resolved_text.is_empty(), |body| {
                let text = self.comment_text(resolved_text.clone(), entity_shift, &theme);
                match self.on_text_expanded_change.clone() {
                    Some(on_expanded_change) => body.child(CommentTextClamp {
                        id: child_id(&self.id, "text-clamp"),
                        is_expanded: self.is_text_expanded,
                        toggle_color: theme.text.secondary,
                        fade_color: theme.bg._100,
                        on_expanded_change,
                        content: text,
                    }),
                    None => body.child(text),
                }
            })
            .when_some(comment.media.clone().filter(|_| !is_moderation_disabled), |body, media| {
                let top = if resolved_text.is_empty() {
                    CommentRowMetrics::MEDIA_TOP_PADDING_ALONE
                } else {
                    CommentRowMetrics::MEDIA_TOP_PADDING_WITH_TEXT
                };
                body.child(
                    div().pt(px(top)).child(CommentMediaView {
                        id: child_id(&self.id, "media"),
                        media,
                        comment_id: comment.id.clone(),
                        author_id: resolved_author_id.clone(),
                        author_name: comment.author.display_name(),
                        author_logo: Some(comment.author.more.logo.clone()),
                        created_at_iso: comment.created_at.clone(),
                        upload: self.photo_upload.clone(),
                        is_temporary: comment.is_temp,
                        on_cancel_upload: self.on_cancel_photo_upload.clone(),
                    }),
                )
            })
            .when(self.show_actions, |body| {
                body.child(
                    div().w_full().pt(px(1.0)).child(
                        CommentActionsRow::new(child_id(&self.id, "actions"), comment.clone())
                            .reply_action_style(self.reply_action_style)
                            .on_like_action(self.on_like.clone())
                            .on_dislike_action(self.on_dislike.clone())
                            .on_reply_action(self.on_reply.clone()),
                    ),
                )
            });

        hstack(CommentRowMetrics::AVATAR_SPACING)
            .align(VerticalAlignment::Top)
            .w_full()
            .px(px(CommentRowMetrics::HORIZONTAL_PADDING))
            .py(px(CommentRowMetrics::VERTICAL_PADDING))
            .opacity(if comment.is_temp && self.photo_upload.is_none() { 0.55 } else { 1.0 })
            .child(avatar)
            .child(body)
    }
}

#[derive(IntoElement)]
pub struct CommentActionsRow {
    id: ElementId,
    comment: GetCommentsResponse,
    reply_action_style: CommentReplyActionStyle,
    shows_dislike_count: bool,
    on_like: CommentRowAction,
    on_dislike: CommentRowAction,
    on_reply: CommentRowAction,
}

impl CommentActionsRow {
    pub fn new(id: impl Into<ElementId>, comment: GetCommentsResponse) -> Self {
        Self {
            id: id.into(),
            comment,
            reply_action_style: CommentReplyActionStyle::Reply,
            shows_dislike_count: true,
            on_like: no_action(),
            on_dislike: no_action(),
            on_reply: no_action(),
        }
    }

    pub fn reply_action_style(mut self, reply_action_style: CommentReplyActionStyle) -> Self {
        self.reply_action_style = reply_action_style;
        self
    }

    pub fn shows_dislike_count(mut self, shows_dislike_count: bool) -> Self {
        self.shows_dislike_count = shows_dislike_count;
        self
    }

    pub fn on_like(mut self, on_like: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_like = Rc::new(on_like);
        self
    }

    pub fn on_dislike(mut self, on_dislike: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dislike = Rc::new(on_dislike);
        self
    }

    pub fn on_reply(mut self, on_reply: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_reply = Rc::new(on_reply);
        self
    }

    fn on_like_action(mut self, on_like: CommentRowAction) -> Self {
        self.on_like = on_like;
        self
    }

    fn on_dislike_action(mut self, on_dislike: CommentRowAction) -> Self {
        self.on_dislike = on_dislike;
        self
    }

    fn on_reply_action(mut self, on_reply: CommentRowAction) -> Self {
        self.on_reply = on_reply;
        self
    }

    #[allow(clippy::too_many_arguments)]
    fn action_button(
        &self,
        name: &'static str,
        system_image: &'static str,
        title: Option<String>,
        accessibility_label: String,
        is_active: bool,
        disabled: bool,
        theme: &AppTheme,
        action: CommentRowAction,
    ) -> impl IntoElement + use<> {
        let color = if is_active { theme.primary._100 } else { theme.text.secondary };
        let is_filled = system_image.ends_with(".fill");
        let title = title.filter(|title| !title.is_empty());
        let button = plain(child_id(&self.id, name))
            .disabled(disabled)
            .on_press(move |window, cx| action(window, cx))
            .child(
                hstack(5.0)
                    .flex_none()
                    .min_h(px(CommentRowMetrics::ACTIONS_MIN_HEIGHT))
                    .children(IconUi::sized(system_image, px(15.0), color, is_filled))
                    .when_some(title, |row, title| row.child(MainText::new(title).px(12.0).color(color).number_of_lines(1))),
            );
        div().id(child_id(&self.id, SharedString::from(format!("{name}-hint")))).tooltip_label(accessibility_label).child(button)
    }

    fn like_button(&self, disabled: bool, theme: &AppTheme) -> impl IntoElement + use<> {
        let is_active = self.comment.user_liked;
        let tint = if is_active { theme.primary._100 } else { theme.text.secondary };
        let count = Self::compact_count(self.comment.likes_count);
        let system_image = if is_active { "heart.fill" } else { "heart" };
        let on_like = self.on_like.clone();
        let hold_on_like = self.on_like.clone();
        let hold_comment = self.comment.clone();
        let heart = PressableUi::new(child_id(&self.id, "like"))
            .tap_scale(1.0)
            .long_press_duration(0.42)
            .disabled(disabled)
            .on_press(move |window, cx| on_like(window, cx))
            .on_long_press(move |window, cx| {
                if hold_comment.likes_count > 0 {
                    let comment = hold_comment.clone();
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comment_likes(&comment, cx));
                } else {
                    hold_on_like(window, cx);
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_h(px(CommentRowMetrics::ACTIONS_MIN_HEIGHT))
                    .when(count.is_some(), |icon| icon.pr(px(5.0)))
                    .children(IconUi::sized(system_image, px(15.0), tint, is_active)),
            );
        let heart = div()
            .id(child_id(&self.id, "like-hint"))
            .tooltip_label(tr("comment_action_like"))
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| cx.stop_propagation())
            .child(heart);

        hstack(0.0).flex_none().child(heart).when_some(count, |row, count| {
            let comment = self.comment.clone();
            let button = plain(child_id(&self.id, "like-count"))
                .disabled(disabled)
                .on_press(move |_, cx| {
                    let comment = comment.clone();
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comment_likes(&comment, cx));
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .min_h(px(CommentRowMetrics::ACTIONS_MIN_HEIGHT))
                        .child(MainText::new(count).px(12.0).color(tint).number_of_lines(1)),
                );
            row.child(div().id(child_id(&self.id, "like-count-hint")).tooltip_label(tr("post_likes_title")).child(button))
        })
    }

    fn reply_system_image(&self) -> &'static str {
        match self.reply_action_style {
            CommentReplyActionStyle::Reply => "arrowshape.turn.up.left",
            CommentReplyActionStyle::Thread { .. } => "bubble.right",
        }
    }

    fn reply_title(&self) -> Option<String> {
        match self.reply_action_style {
            CommentReplyActionStyle::Reply => Some(tr("contextMenu_reply")),
            CommentReplyActionStyle::Thread { count } => {
                if count > 0 { Self::compact_count(count) } else { Some(tr("contextMenu_reply")) }
            }
        }
    }

    /// zero hides the label entirely so this returns none not 0
    fn compact_count(value: i64) -> Option<String> {
        if value <= 0 {
            return None;
        }
        Some(RiseFormat::compact_count(value))
    }
}

impl RenderOnce for CommentActionsRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let comment = &self.comment;
        let disabled = comment.is_temp || comment.is_moderation_disabled();

        hstack(16.0)
            .w_full()
            .when(disabled, |row| row.opacity(rise_ui::swiftui::button_style::DISABLED_OPACITY))
            .child(self.like_button(disabled, &theme))
            .child(self.action_button(
                "dislike",
                if comment.user_disliked { "hand.thumbsdown.fill" } else { "hand.thumbsdown" },
                if self.shows_dislike_count { Self::compact_count(comment.dislikes_count) } else { None },
                tr("comment_action_dislike"),
                comment.user_disliked,
                disabled,
                &theme,
                self.on_dislike.clone(),
            ))
            .child(self.action_button(
                "reply",
                self.reply_system_image(),
                self.reply_title(),
                tr("contextMenu_reply"),
                false,
                disabled,
                &theme,
                self.on_reply.clone(),
            ))
            .child(spacer_min_w(0.0))
            .when(comment.is_first, |row| row.child(FirstCommentBadge::new(child_id(&self.id, "first-badge"))))
    }
}

#[derive(IntoElement)]
struct CommentMediaView {
    id: ElementId,
    media: CommentMedia,
    comment_id: String,
    author_id: String,
    author_name: String,
    author_logo: Option<String>,
    created_at_iso: String,
    upload: Option<CommentPhotoUpload>,
    is_temporary: bool,
    on_cancel_upload: CommentRowAction,
}

struct CommentMediaHeroObserver {
    _subscription: gpui::Subscription,
}

impl CommentMediaView {
    const RADIUS: f32 = 14.0;

    fn registry_id(&self) -> String {
        MediaViewerFrameKeys::comment_media(&self.comment_id)
    }

    fn upload_overlay(&self, upload: &CommentPhotoUpload, size: Size<Pixels>) -> impl IntoElement + use<> {
        let on_cancel_upload = self.on_cancel_upload.clone();
        div().absolute().inset_0().rounded(px(Self::RADIUS)).overflow_hidden().child(
            MessageMediaUploadOverlay::new(
                child_id(&self.id, "upload"),
                upload.media_item(self.comment_id.clone(), &self.media),
            )
            .cell_size(size)
            .on_cancel(move |window, cx| on_cancel_upload(window, cx)),
        )
    }

    fn photo_tile(&self, size: Size<Pixels>, window: &mut Window, cx: &mut App) -> gpui::Div {
        let presenter = media_viewer_presentation(cx);
        let observed = presenter.clone();
        let _ = window.use_keyed_state(child_id(&self.id, "hero-observer"), cx, move |_, cx| CommentMediaHeroObserver {
            _subscription: cx.observe(&observed, |_, _, cx| cx.notify()),
        });
        let registry_id = self.registry_id();
        let is_lifted = {
            let state = presenter.read(cx);
            state.hero_lift_id() == Some(registry_id.as_str()) && state.bubble_cell_hidden()
        };
        let scale = window.scale_factor().min(3.0);
        let longest = f32::from(size.width).max(f32::from(size.height));

        div()
            .relative()
            .w(size.width)
            .h(size.height)
            .rounded(px(Self::RADIUS))
            .overflow_hidden()
            .opacity(if is_lifted { 0.0 } else { 1.0 })
            .child(
                pass_through().absolute().inset_0().child(
                    WindowScreenFrameReader::new(registry_id)
                        .source_media_radii(Some(MessageBubbleRadii::uniform(px(Self::RADIUS)))),
                ),
            )
            .child(
                CleverImageUi::new(self.media.file_url.clone())
                    .id(child_id(&self.id, "photo"))
                    .content_mode(ContentMode::Fill)
                    .corner_radius(Self::RADIUS)
                    .persist_to_disk(true)
                    .maximum_decode_pixel_dimension((longest * scale).ceil() as i64),
            )
    }

    fn photo_body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let size = CommentRowMetrics::photo_size(&self.media);
        let tile = self.photo_tile(size, window, cx);
        if let Some(upload) = &self.upload {
            return tile.child(self.upload_overlay(upload, size)).into_any_element();
        }
        if self.is_temporary {
            return tile.into_any_element();
        }
        let open = self.open_photo_action();
        tile.media_bubble_tap_target(child_id(&self.id, "photo-tap"), self.registry_id(), move |window, cx| {
            open(window, cx);
        })
        .into_any_element()
    }

    fn open_photo_action(&self) -> CommentRowAction {
        let media = self.media.clone();
        let registry_id = self.registry_id();
        let author_id = self.author_id.clone();
        let author_name = self.author_name.clone();
        let author_logo = self.author_logo.clone();
        let created_at_iso = self.created_at_iso.clone();
        Rc::new(move |_: &mut Window, cx: &mut App| {
            let source = trimming_whitespaces_and_newlines(&media.file_url).to_string();
            if source.is_empty() {
                return;
            }
            let item = MediaViewerGalleryItem {
                id: registry_id.clone(),
                kind: MediaViewerGalleryItemKind::Image,
                url: source.clone(),
                thumbnail_url: media.thumbnail_url.clone(),
                width: media.width.max(0) as f32,
                height: media.height.max(0) as f32,
                duration: None,
                sender_id: author_id.clone(),
                sender_name: author_name.clone(),
                sender_logo: RemoteMediaURL::optional_normalized(author_logo.as_deref()),
                created_at: CommentRelativeTime::parsed_date(&created_at_iso)
                    .map(|date| date.time_interval_since_1970())
                    .unwrap_or(0.0),
                caption_title: String::new(),
                caption: String::new(),
                message_id: String::new(),
                index_in_message: 0,
                media_item_id: registry_id.clone(),
                uses_fit_letterbox: false,
                frame_registry_id: registry_id.clone(),
                is_own: false,
                can_delete: false,
                can_save: true,
                can_forward: false,
                can_reply: false,
                can_show_in_context: false,
                can_delete_for_me: false,
                can_delete_for_everyone: false,
                can_set_as_main: false,
                is_album_row: false,
                display_source_override: None,
                persist_media_to_disk: true,
                deletes_from_favourites: false,
                post_download: None,
                image_cache_policy: CleverImageCachePolicy::Standard,
            };
            let hero_skin = MediaViewerHeroSkin {
                display_source: source,
                uses_fit_foreground: false,
                media_tail: None,
                source_media_radii: Some(MessageBubbleRadii::uniform(px(Self::RADIUS))),
            };
            let source_frame = MediaCellScreenFrameRegistry::frame(&registry_id).unwrap_or_default();
            let frame_registry_id = registry_id.clone();
            let presenter: Entity<MediaViewerPresentationState> = media_viewer_presentation(cx);
            cx.spawn(async move |cx| {
                MediaViewerOpen::present(
                    presenter,
                    vec![item],
                    0,
                    source_frame,
                    frame_registry_id,
                    hero_skin,
                    MediaViewerChrome { shows_bottom_bar: true, can_report: false, ..Default::default() },
                    MediaViewerGalleryScope::Complete,
                    MediaViewerPresentationHandlers::default(),
                    cx,
                )
                .await;
            })
            .detach();
        })
    }

    fn gif_body(&self) -> AnyElement {
        let size = CommentRowMetrics::gif_size(&self.media);
        div()
            .relative()
            .w(size.width)
            .h(size.height)
            .rounded(px(Self::RADIUS))
            .overflow_hidden()
            .child(
                AnimatedGifView::new(child_id(&self.id, "gif"), self.gif_source())
                    .content_mode(AnimatedGifContentMode::ScaleAspectFill)
                    .is_playing(true),
            )
            .when_some(self.upload.as_ref(), |tile, upload| tile.child(self.upload_overlay(upload, size)))
            .into_any_element()
    }

    fn gif_source(&self) -> String {
        let original = trimming_whitespaces_and_newlines(&self.media.file_url);
        if original.is_empty() {
            self.media.thumbnail_url.clone().unwrap_or_default()
        } else {
            original.to_string()
        }
    }

    fn sticker_body(&self) -> AnyElement {
        let pack_id = self.media.pack_id.clone();
        ChatInputStickerAsset::new(self.sticker_item(), CommentRowMetrics::STICKER_SIDE)
            .is_playing(true)
            .bubble_tap_target_priority(child_id(&self.id, "sticker-tap"), move |_, cx| {
                let pack_id = pack_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("").to_string();
                if pack_id.is_empty() {
                    return;
                }
                CommentInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.open_sticker_pack(&pack_id, PackKind::Sticker, cx);
                });
            })
            .into_any_element()
    }

    fn sticker_item(&self) -> StickerItem {
        let media = &self.media;
        StickerItem {
            id: media.sticker_id.clone().unwrap_or_else(|| media.file_url.clone()),
            pack_id: media.pack_id.clone().unwrap_or_default(),
            file_url: media.file_url.clone(),
            thumbnail_url: media.thumbnail_url.clone(),
            width: media.width,
            height: media.height,
            duration_ms: media.duration_ms,
            position: 0,
            created_at_ms: 0,
            associated_emojis: media.associated_emojis.clone(),
            asset_type: StickerAssetType::infer(media.sticker_type.as_deref(), &media.file_url),
            mime_type: None,
            codec: None,
        }
    }
}

impl RenderOnce for CommentMediaView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        match self.media.r#type {
            CommentMediaType::Gif => self.gif_body(),
            CommentMediaType::Sticker => self.sticker_body(),
            CommentMediaType::Photo => self.photo_body(window, cx),
        }
    }
}

#[derive(IntoElement)]
struct CommentLiveRelativeTime {
    id: ElementId,
    iso: String,
    color: Hsla,
}

struct CommentLiveRelativeTimeTick;

impl CommentLiveRelativeTime {
    fn refresh_dates(date: &Date) -> Vec<Date> {
        let now = Date::now();
        let hour = date.adding_time_interval(3600.0);
        let mut dates = Vec::new();
        let mut next = date.adding_time_interval(60.0);
        while next <= hour {
            if next > now {
                dates.push(next);
            }
            next = next.adding_time_interval(60.0);
        }
        dates
    }
}

impl RenderOnce for CommentLiveRelativeTime {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let date = CommentRelativeTime::date(&self.iso);
        let tick = window.use_keyed_state(child_id(&self.id, "tick"), cx, |_, _| CommentLiveRelativeTimeTick);
        use_task(child_id(&self.id, "schedule"), self.iso.clone(), window, cx, async move |cx| {
            for next in Self::refresh_dates(&date) {
                let wait = next.time_interval_since_1970() - Date::now().time_interval_since_1970();
                if wait > 0.0 {
                    cx.background_executor().timer(Duration::from_secs_f64(wait)).await;
                }
                let tick = tick.clone();
                let _ = cx.update(|_, cx| tick.update(cx, |_, cx| cx.notify()));
            }
        });
        MainText::new(CommentRelativeTime::string(&date, &Date::now())).px(10.0).color(self.color)
    }
}

enum CommentRelativeTime {}

thread_local! {
    static COMMENT_FRACTIONAL_FORMATTER: ISO8601DateFormatter = {
        let mut formatter = ISO8601DateFormatter::new();
        formatter.set_format_options(
            ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
        );
        formatter
    };
    static COMMENT_INTERNET_FORMATTER: ISO8601DateFormatter = ISO8601DateFormatter::new();
}

impl CommentRelativeTime {
    fn parsed_date(iso: &str) -> Option<Date> {
        COMMENT_FRACTIONAL_FORMATTER
            .with(|formatter| formatter.date_from(iso))
            .or_else(|| COMMENT_INTERNET_FORMATTER.with(|formatter| formatter.date_from(iso)))
    }

    fn date(iso: &str) -> Date {
        Self::parsed_date(iso).unwrap_or_else(Date::now)
    }

    /// canon bands like post headers and stories, the first minute reads now
    fn string(date: &Date, now: &Date) -> String {
        RiseFormat::elapsed_since_date(if date < now { date } else { now }, now)
    }
}

#[derive(IntoElement)]
struct CommentTextClamp {
    id: ElementId,
    is_expanded: bool,
    toggle_color: Hsla,
    fade_color: Hsla,
    on_expanded_change: CommentTextExpandedChange,
    content: AnyElement,
}

struct CommentTextClampState {
    natural_height: f32,
}

/// last natural text height seen per clamp id, kept outside the element tree
#[derive(Default)]
struct CommentTextClampHeights {
    heights: HashMap<ElementId, f32>,
    order: VecDeque<ElementId>,
}

thread_local! {
    static COMMENT_TEXT_CLAMP_HEIGHTS: RefCell<CommentTextClampHeights> = RefCell::new(CommentTextClampHeights::default());
}

impl CommentTextClampHeights {
    fn height(id: &ElementId) -> Option<f32> {
        COMMENT_TEXT_CLAMP_HEIGHTS.with(|heights| heights.borrow().heights.get(id).copied())
    }

    fn insert(id: ElementId, height: f32) {
        let capacity = RiseLimits::client_value("comment_row_heights_memory").max(0) as usize;
        COMMENT_TEXT_CLAMP_HEIGHTS.with(|heights| {
            let mut heights = heights.borrow_mut();
            if heights.heights.insert(id.clone(), height).is_none() {
                heights.order.push_back(id);
            }
            while heights.order.len() > capacity {
                let Some(oldest) = heights.order.pop_front() else { break };
                heights.heights.remove(&oldest);
            }
        });
    }
}

impl CommentTextClamp {
    fn line_height(text_delta: f32) -> f32 {
        (CommentRowMetrics::TEXT_FONT_SIZE + text_delta).max(1.0) * NATURAL_LINE_HEIGHT
    }

    fn clamp_height(line_height: f32) -> f32 {
        let lines = CommentRowMetrics::TEXT_LINE_LIMIT as f32;
        (lines * line_height + (lines - 1.0) * CommentRowMetrics::TEXT_LINE_SPACING).ceil()
    }
}

impl RenderOnce for CommentTextClamp {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let line_height = Self::line_height(ThemeStore::shared(cx).read(cx).text_delta());
        let clamp_height = Self::clamp_height(line_height);
        let slack = line_height * 0.5;
        let state = window.use_keyed_state(child_id(&self.id, "natural-height"), cx, |_, _| CommentTextClampState {
            natural_height: 0.0,
        });
        let keyed_height = state.read(cx).natural_height;
        let natural_height = if keyed_height > 0.0 {
            keyed_height
        } else {
            CommentTextClampHeights::height(&self.id).unwrap_or(0.0)
        };
        let is_clampable =
            if natural_height > 0.0 { natural_height > clamp_height + slack } else { self.is_expanded };
        let clamps = is_clampable && !self.is_expanded;
        let cap = match (self.is_expanded, is_clampable) {
            (true, _) => None,
            (false, true) => Some(clamp_height),
            (false, false) => Some(clamp_height + slack),
        };

        let measured = state.clone();
        let clampable_above = clamp_height + slack;
        let remembered_id = self.id.clone();
        let rendered_clampable = is_clampable;
        let natural = div()
            .relative()
            .w_full()
            .flex_none()
            .child(self.content)
            .child(measure(child_id(&self.id, "measure"), move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                if CommentTextClampHeights::height(&remembered_id).is_none_or(|known| (known - height).abs() > 0.5) {
                    CommentTextClampHeights::insert(remembered_id.clone(), height);
                }
                measured.update(cx, |state, cx| {
                    if (state.natural_height - height).abs() > 0.5 {
                        state.natural_height = height;
                        if rendered_clampable != (height > clampable_above) {
                            cx.notify();
                        }
                    }
                });
            }));
        let fade = div().absolute().left_0().right_0().bottom_0().h(px(line_height)).bg(linear_gradient(
            180.0,
            linear_color_stop(self.fade_color.opacity(0.0), 0.0),
            linear_color_stop(self.fade_color, 1.0),
        ));
        let text = div()
            .relative()
            .w_full()
            .overflow_hidden()
            .when_some(cap, |text, cap| text.max_h(px(cap)))
            .child(natural)
            .when(clamps, |text| text.child(fade));

        let is_expanded = self.is_expanded;
        let tap = self.on_expanded_change.clone();
        let routed = self.on_expanded_change;
        let toggle_color = self.toggle_color;
        let toggle_id = self.id.clone();
        vstack(0.0).align(HorizontalAlignment::Leading).w_full().child(text).when(is_clampable, |clamp| {
            clamp.child(
                div().pt(px(CommentRowMetrics::TEXT_TOGGLE_GAP)).child(
                    div()
                        .id(child_id(&toggle_id, "toggle"))
                        .flex_none()
                        .cursor_pointer()
                        .on_click(move |_, window, cx| tap(!is_expanded, window, cx))
                        .child(
                            MainText::new(tr(if is_expanded { "profile_bio_less" } else { "profile_bio_more" }))
                                .px(CommentRowMetrics::TEXT_TOGGLE_FONT_SIZE)
                                .font_weight(FontWeight::MEDIUM)
                                .color(toggle_color)
                                .number_of_lines(1),
                        )
                        .bubble_tap_target_priority(child_id(&toggle_id, "toggle-tap"), move |window, cx| {
                            routed(!is_expanded, window, cx)
                        }),
                ),
            )
        })
    }
}

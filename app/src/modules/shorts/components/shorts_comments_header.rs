use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, IntoElement, RenderOnce, SharedString, Window};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_i18n::Locale;
use rise_theme::ThemeStore;
use rise_ui::expandable_text_ui::{EdgeInsets, ExpandableTextHashtags, ExpandableTextInlineLinks, ExpandableTextUi};
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::ui::formatted_text_ui::FormattedTextStyle;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::shorts_follow_button::ShortsFollowButton;

#[derive(IntoElement)]
pub struct ShortsCommentsHeader {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    caption_expanded: Binding<bool>,
    on_open_author: Rc<dyn Fn(&mut Window, &mut App)>,
    on_open_hashtag: Rc<dyn Fn(&str, &mut Window, &mut App)>,
}

thread_local! {
    static FRACTIONAL_ISO: ISO8601DateFormatter = {
        let mut formatter = ISO8601DateFormatter::new();
        formatter.set_format_options(
            ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
        );
        formatter
    };
    static PLAIN_ISO: ISO8601DateFormatter = {
        let mut formatter = ISO8601DateFormatter::new();
        formatter.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
        formatter
    };
}

impl ShortsCommentsHeader {
    pub fn new(
        id: impl Into<ElementId>,
        post: impl Into<Rc<GetPostFeedResponse>>,
        caption_expanded: Binding<bool>,
        on_open_author: impl Fn(&mut Window, &mut App) + 'static,
        on_open_hashtag: impl Fn(&str, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            post: post.into(),
            caption_expanded,
            on_open_author: Rc::new(on_open_author),
            on_open_hashtag: Rc::new(on_open_hashtag),
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn display_tag(&self) -> Option<String> {
        let tag = trimming_whitespaces_and_newlines(&self.post.author.tag);
        if tag.is_empty() { None } else { Some(tag.to_string()) }
    }

    fn authored_by_me(&self, cx: &mut App) -> bool {
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        RiseIdentity::is_own(
            &RiseIdentity::author(Some(self.post.author_id.as_str()), None, self.post.author.id.as_deref()),
            viewer.as_deref(),
            None,
        )
    }

    fn avatar_source(&self, authored_by_me: bool, cx: &mut App) -> String {
        let logo = trimming_whitespaces_and_newlines(&self.post.author.more.logo);
        if !logo.is_empty() {
            return logo.to_string();
        }
        if authored_by_me {
            let auth = AuthServicesStore::shared(cx);
            let auth = auth.read(cx);
            return auth
                .profile()
                .map(|me| me.avatar_url.clone().unwrap_or_else(|| me.more.logo.clone()))
                .unwrap_or_default();
        }
        logo.to_string()
    }

    pub fn date_label(post: &GetPostFeedResponse) -> Option<String> {
        let date = Self::published_date(post)?;
        Some(RiseFormat::date_time_label(
            &date,
            &RiseClock::now(),
            &Calendar::current(),
            Locale::app_locale().identifier(),
        ))
    }

    pub fn published_date(post: &GetPostFeedResponse) -> Option<Date> {
        if let Some(ms) = post.created_at_ms.filter(|ms| *ms > 0) {
            return Some(Date::from_time_interval_since_1970(ms as f64 / 1000.0));
        }
        let trimmed = trimming_whitespaces_and_newlines(&post.created_at);
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(number) = trimmed.parse::<f64>() {
            let seconds = if number > 1_000_000_000_000.0 { number / 1000.0 } else { number };
            return Some(Date::from_time_interval_since_1970(seconds));
        }
        FRACTIONAL_ISO
            .with(|formatter| formatter.date_from(trimmed))
            .or_else(|| PLAIN_ISO.with(|formatter| formatter.date_from(trimmed)))
    }
}

impl RenderOnce for ShortsCommentsHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let post = self.post.clone();
        let title = trimming_whitespaces_and_newlines(&post.title).to_string();
        let caption = trimming_whitespaces_and_newlines(&post.content).to_string();
        let has_caption = !caption.is_empty() || !post.hashtags.is_empty();
        let authored_by_me = self.authored_by_me(cx);
        let avatar_source = self.avatar_source(authored_by_me, cx);
        let on_open_author = self.on_open_author.clone();
        let on_open_hashtag = self.on_open_hashtag.clone();

        let names = vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .min_w_0()
            .child(
                hstack(5.0)
                    .min_w_0()
                    .max_w_full()
                    .child(
                        MainText::new(post.author.name.clone())
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .when(post.author.more.is_official, |line| line.child(OfficialBadge::new().size(px(15.0))))
                    .when(post.author.more.is_premium, |line| {
                        line.child(
                            PremiumBadge::new()
                                .user_id(post.author.id.clone().map(SharedString::from))
                                .emoji(post.author.more.premium_badge_emoji.clone().map(SharedString::from))
                                .size(16.0),
                        )
                    }),
            )
            .when_some(self.display_tag(), |names, tag| {
                names.child(
                    div().flex().min_w_0().max_w_full().child(
                        MainText::new(format!("@{tag}")).px(12.0).color(theme.text.secondary).number_of_lines(1),
                    ),
                )
            });

        let author_line = hstack(10.0)
            .w_full()
            .child(
                plain(self.child_id("author"))
                    .min_w_0()
                    .on_press(move |window, cx| on_open_author(window, cx))
                    .child(
                        hstack(10.0)
                            .min_w_0()
                            .child(
                                UserLogoUi::new(self.child_id("avatar"))
                                    .source(SharedString::from(avatar_source))
                                    .name(SharedString::from(post.author.name.clone()))
                                    .user_id(post.author.id.clone().map(SharedString::from))
                                    .size(40.0),
                            )
                            .child(names),
                    ),
            )
            .when(!authored_by_me, |line| {
                line.child(ShortsFollowButton::new(post.clone()).on_dark(false).is_enabled(!post.shows_upload_overlay()))
            })
            .child(spacer_min_w(0.0));

        let date = Self::date_label(&post);
        let content = vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(14.0))
            .pt(px(6.0))
            .pb(px(14.0))
            .child(author_line)
            .when(!title.is_empty(), |content| {
                content.child(
                    MainText::new(title.clone()).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary),
                )
            })
            .when(has_caption, |content| {
                content.child(
                    ExpandableTextUi::new(
                        self.child_id("caption"),
                        caption,
                        theme.text.primary.opacity(0.92),
                        theme.text.secondary,
                    )
                    .hashtags(
                        ExpandableTextHashtags::new(post.hashtags.clone(), theme.primary._100)
                            .on_tap(move |tag, window, cx| on_open_hashtag(tag, window, cx)),
                    )
                    .inline_links(ExpandableTextInlineLinks::new(FormattedTextStyle::standard_link_color()))
                    .font_size(15.0)
                    .line_limit(2)
                    .line_spacing(2.0)
                    .is_expanded(self.caption_expanded.clone())
                    .collapse_control_insets(EdgeInsets { top: 6.0, leading: 0.0, bottom: 0.0, trailing: 48.0 }),
                )
            })
            .when_some(date, |content, date| {
                let top = if has_caption || !title.is_empty() { 2.0 } else { 0.0 };
                content.child(div().pt(px(top)).child(MainText::new(date).px(12.0).color(theme.text.secondary)))
            });

        vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(content)
            .child(div().w_full().h(px(0.5)).mb(px(6.0)).bg(theme.border._200.opacity(0.35)))
    }
}

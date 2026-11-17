use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::{
    App, ElementId, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, StyledText, TextOverflow, Window, div, px, transparent_black,
    white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_theme::AppTheme;
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, ignores_text_size_delta};
use rise_ui::swiftui::open_url::open_url;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke, pass_through};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::user_logo_ui::UserLogoUi;

#[derive(Clone, Debug, PartialEq)]
pub struct LinkPreviewCardPresentation {
    pub url: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub site_name: Option<String>,
    pub image_url: Option<String>,
    pub image_aspect: f32,
    pub summary_is_skeleton: bool,
}

impl Eq for LinkPreviewCardPresentation {}

impl Hash for LinkPreviewCardPresentation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.url.hash(state);
        self.title.hash(state);
        self.summary.hash(state);
        self.site_name.hash(state);
        self.image_url.hash(state);
        self.image_aspect.to_bits().hash(state);
        self.summary_is_skeleton.hash(state);
    }
}

impl LinkPreviewCardPresentation {
    /// constructor with image_aspect defaulted to 1.91
    pub fn new(
        url: impl Into<String>,
        title: Option<String>,
        summary: Option<String>,
        site_name: Option<String>,
        image_url: Option<String>,
    ) -> Self {
        Self {
            url: url.into(),
            title,
            summary,
            site_name,
            image_url,
            image_aspect: 1.91,
            summary_is_skeleton: false,
        }
    }

    pub fn display_site_name(&self) -> Option<String> {
        let explicit = self
            .site_name
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default();
        if !explicit.is_empty() {
            return Some(explicit.to_owned());
        }
        Url::from_string(&self.url)?.host()
    }

    pub fn is_avatar_link(&self) -> bool {
        Self::is_avatar_link_url(&self.url)
    }

    /// true for a riseonly profile, chat
    /// invite or bot link on any environment host
    /// its image is an avatar, small and round like telegram and not a wide banner
    pub fn is_avatar_link_url(url: &str) -> bool {
        let Some(parsed) = Url::from_string(url) else {
            return false;
        };
        for environment in [
            AppEnvironment::current(),
            AppEnvironment::Prod,
            AppEnvironment::Staging,
            AppEnvironment::Dev,
        ] {
            match UniversalLinkParser::parse(&parsed, environment) {
                Some(
                    PushDeepLink::Profile { .. }
                    | PushDeepLink::PublicTag { .. }
                    | PushDeepLink::Chat { .. }
                    | PushDeepLink::BotChat { .. }
                    | PushDeepLink::JoinChat { .. },
                ) => return true,
                Some(_) => return false,
                None => continue,
            }
        }
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkPreviewCardVariant {
    Compact,
    Expanded,
}

#[derive(IntoElement)]
pub struct LinkPreviewCardUi {
    id: ElementId,
    presentation: LinkPreviewCardPresentation,
    variant: LinkPreviewCardVariant,
    is_on_accent_fill: bool,
    shows_card_chrome: bool,
    opens_url_on_tap: bool,
    compact_media_size: f32,
    reserves_compact_media_space: bool,
    expanded_media_height: f32,
}

impl LinkPreviewCardUi {
    /// id keys the tap target and the images
    pub fn new(id: impl Into<ElementId>, presentation: LinkPreviewCardPresentation) -> Self {
        Self {
            id: id.into(),
            presentation,
            variant: LinkPreviewCardVariant::Compact,
            is_on_accent_fill: false,
            shows_card_chrome: true,
            opens_url_on_tap: true,
            compact_media_size: 72.0,
            reserves_compact_media_space: true,
            expanded_media_height: 150.0,
        }
    }

    pub fn variant(mut self, variant: LinkPreviewCardVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn is_on_accent_fill(mut self, is_on_accent_fill: bool) -> Self {
        self.is_on_accent_fill = is_on_accent_fill;
        self
    }

    pub fn shows_card_chrome(mut self, shows_card_chrome: bool) -> Self {
        self.shows_card_chrome = shows_card_chrome;
        self
    }

    pub fn opens_url_on_tap(mut self, opens_url_on_tap: bool) -> Self {
        self.opens_url_on_tap = opens_url_on_tap;
        self
    }

    pub fn compact_media_size(mut self, compact_media_size: f32) -> Self {
        self.compact_media_size = compact_media_size;
        self
    }

    pub fn reserves_compact_media_space(mut self, reserves_compact_media_space: bool) -> Self {
        self.reserves_compact_media_space = reserves_compact_media_space;
        self
    }

    pub fn expanded_media_height(mut self, expanded_media_height: f32) -> Self {
        self.expanded_media_height = expanded_media_height;
        self
    }

    pub const COMPACT_TOTAL_LINES: usize = 4;

    fn card(&self, theme: &AppTheme, window: &Window, cx: &App) -> gpui::Div {
        let delta = if ignores_text_size_delta(cx) { 0.0 } else { ThemeStore::text_delta_snapshot() };
        let shape = Shape::continuous(12.0);
        let mut column = div().flex().flex_col().items_start().gap(px(3.0)).flex_1().min_w_0();
        if let Some(site) = Self::non_empty(self.presentation.display_site_name().as_deref()) {
            column = column.child(
                MainText::new(site)
                    .px(13.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(self.accent_color(theme))
                    .number_of_lines(1),
            );
        }
        if self.variant == LinkPreviewCardVariant::Compact {
            let site_lines = if Self::non_empty(self.presentation.display_site_name().as_deref()).is_none() { 0 } else { 1 };
            column = column.children(self.compact_title_and_description(
                theme,
                Self::COMPACT_TOTAL_LINES - site_lines,
                delta,
                window,
                cx,
            ));
        } else {
            if let Some(title) = Self::non_empty(self.presentation.title.as_deref()) {
                column = column.child(
                    MainText::new(title)
                        .px(14.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(self.primary_text_color(theme))
                        .number_of_lines(2),
                );
            }
            if let Some(summary) = Self::non_empty(self.presentation.summary.as_deref()) {
                column = column.child(self.description_text(summary, theme, delta, window, cx));
            }
        }
        if self.variant == LinkPreviewCardVariant::Expanded {
            column = column.children(self.expanded_media());
        }

        let mut row = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.0))
            .child(
                div()
                    .w(px(3.0))
                    .flex_shrink_0()
                    .self_stretch()
                    .bg(self.accent_color(theme))
                    .clip_shape(Shape::rounded_rect(1.5)),
            )
            .child(column);
        if self.variant == LinkPreviewCardVariant::Compact {
            row = row.children(self.compact_media(theme));
        }

        let mut card = row
            .relative()
            .w_full()
            .p(px(if self.shows_card_chrome { 10.0 } else { 0.0 }))
            .overflow_hidden();
        if let Some(height) = self.compact_minimum_height() {
            card = card.min_h(px(height));
        }
        if let Some(height) = self.compact_maximum_height() {
            card = card.max_h(px(height));
        }
        if self.shows_card_chrome {
            card = card
                .bg(theme.bg._200)
                .continuous_corners(12.0)
                .child(centred_stroke(theme.border._200, 0.5, shape));
        }
        card
    }

    fn compact_media(&self, theme: &AppTheme) -> Option<gpui::AnyElement> {
        let size = self.compact_media_size;
        if let Some(image_url) = Self::non_empty(self.presentation.image_url.as_deref())
            && self.presentation.is_avatar_link()
            && AvatarPalette::generated_avatar_owner_id(Some(&image_url)).is_some()
        {
            let name = Self::non_empty(self.presentation.title.as_deref())
                .or_else(|| Self::non_empty(self.presentation.site_name.as_deref()));
            let logo = UserLogoUi::new(ElementId::NamedChild(Arc::new(self.id.clone()), "compact-logo".into()))
                .source(SharedString::from(image_url))
                .name(name.map(SharedString::from))
                .size(size)
                .show_border(false)
                .show_story_ring(false)
                .open_story_on_press(false);
            return Some(pass_through().flex_shrink_0().child(logo).into_any_element());
        }
        if let Some(image_url) = Self::non_empty(self.presentation.image_url.as_deref()) {
            let corner_radius = if self.presentation.is_avatar_link() { size / 2.0 } else { 8.0 };
            return Some(
                div()
                    .size(px(size))
                    .flex_shrink_0()
                    .clip_shape(Shape::continuous(corner_radius))
                    .child(
                        CleverImageUi::new(image_url)
                            .id(ElementId::NamedChild(Arc::new(self.id.clone()), "compact-media".into()))
                            .content_mode(ContentMode::Fill)
                            .corner_radius(8.0)
                            .persist_to_disk(true)
                            .shows_progress_indicator(false)
                            .maximum_decode_pixel_dimension((size * 3.0).ceil() as i64),
                    )
                    .into_any_element(),
            );
        }
        if self.reserves_compact_media_space {
            return Some(
                div()
                    .size(px(size))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg._300)
                    .clip_shape(Shape::continuous(8.0))
                    .children(IconUi::sized("link", px(18.0), theme.text.secondary, false))
                    .into_any_element(),
            );
        }
        None
    }

    fn expanded_media(&self) -> Option<gpui::AnyElement> {
        let image_url = Self::non_empty(self.presentation.image_url.as_deref())?;
        let aspect = self.presentation.image_aspect;
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.91 };
        Some(
            div()
                .w_full()
                .mt(px(5.0))
                .child(
                    div()
                        .relative()
                        .w_full()
                        .clip_shape(Shape::continuous(8.0))
                        .child(div().w_full().max_h(px(self.expanded_media_height)).aspect_ratio(aspect))
                        .child(
                            div().absolute().inset_0().overflow_hidden().child(
                                CleverImageUi::new(image_url)
                                    .id(ElementId::NamedChild(Arc::new(self.id.clone()), "expanded-media".into()))
                                    .content_mode(ContentMode::Fill)
                                    .corner_radius(8.0)
                                    .persist_to_disk(true)
                                    .shows_progress_indicator(false)
                                    .maximum_decode_pixel_dimension(900),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn description_text(&self, summary: String, theme: &AppTheme, delta: f32, window: &Window, cx: &App) -> gpui::Div {
        let color = if self.presentation.summary_is_skeleton { transparent_black() } else { self.secondary_text_color(theme) };
        let text = MainText::new(summary.clone())
            .px(13.0)
            .line_height(14.0)
            .color(color)
            .number_of_lines(if self.variant == LinkPreviewCardVariant::Compact { 5 } else { 8 });
        let mut block = div().relative().w_full().child(text);
        if self.presentation.summary_is_skeleton {
            block = block.child(
                div().absolute().top_0().left_0().child(self.summary_skeleton(&summary, 13.0 + delta, 1.0, theme, window, cx)),
            );
        }
        block
    }

    fn compact_title_and_description(
        &self,
        theme: &AppTheme,
        lines: usize,
        delta: f32,
        window: &Window,
        cx: &App,
    ) -> Option<gpui::AnyElement> {
        let title = Self::non_empty(self.presentation.title.as_deref());
        let summary = Self::non_empty(self.presentation.summary.as_deref());
        if (title.is_none() && summary.is_none()) || lines == 0 {
            return None;
        }
        let summary_color =
            if self.presentation.summary_is_skeleton { transparent_black() } else { self.secondary_text_color(theme) };
        let mut joined = String::new();
        let mut highlights = Vec::new();
        if let Some(title) = title.as_deref() {
            joined.push_str(title);
            highlights.push((
                0..joined.len(),
                HighlightStyle {
                    font_weight: Some(FontWeight::SEMIBOLD),
                    color: Some(self.primary_text_color(theme)),
                    ..HighlightStyle::default()
                },
            ));
        }
        if let Some(summary) = summary.as_deref() {
            if !joined.is_empty() {
                joined.push('\n');
            }
            let start = joined.len();
            joined.push_str(summary);
            highlights.push((start..joined.len(), HighlightStyle { color: Some(summary_color), ..HighlightStyle::default() }));
        }
        let size = (13.0 + delta).max(1.0);
        let line_height = Self::line_height(size, window, cx);
        let text = div()
            .w_full()
            .text_size(px(size))
            .line_height(px(line_height))
            .line_clamp(lines)
            .text_overflow(TextOverflow::Truncate(SharedString::new_static("\u{2026}")))
            .child(StyledText::new(joined).with_highlights(highlights));
        let mut block = div().relative().w_full().child(text);
        if self.presentation.summary_is_skeleton
            && let Some(summary) = summary.as_deref()
        {
            block = block.child(
                div().absolute().bottom_0().left_0().child(self.summary_skeleton(summary, 13.0 + delta, 0.0, theme, window, cx)),
            );
        }
        Some(block.into_any_element())
    }

    /// one static bar per placeholder line on the line own height so nothing moves when the real text
    /// replaces it
    fn summary_skeleton(
        &self,
        summary: &str,
        px_size: f32,
        line_spacing: f32,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> gpui::AnyElement {
        let line_height = Self::line_height(px_size.max(1.0), window, cx);
        let count = summary.split('\n').count();
        let bar = self.secondary_text_color(theme).opacity(0.25);
        pass_through()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(line_spacing))
            .children((0..count).map(|index| {
                div().h(px(line_height)).flex().items_center().child(
                    div()
                        .w(px(if index == 0 { 120.0 } else { 84.0 }))
                        .h(px((line_height - 7.0).max(4.0)))
                        .bg(bar)
                        .continuous_corners(4.0),
                )
            }))
            .into_any_element()
    }

    /// font own line height and not the gpui phi spacing
    fn line_height(size: f32, window: &Window, cx: &App) -> f32 {
        let text_system = cx.text_system();
        let font_id = text_system.resolve_font(&window.text_style().font());
        let ascent = f32::from(text_system.ascent(font_id, px(size)));
        let descent = f32::from(text_system.descent(font_id, px(size)));
        ascent + descent.abs()
    }

    fn open_preview_url(url: &str, window: &mut Window, cx: &mut App) {
        let Some(url) = Url::from_string(url) else {
            return;
        };
        let Some(scheme) = url.scheme().map(str::to_lowercase) else {
            return;
        };
        if scheme != "https" && scheme != "http" {
            return;
        }
        open_url(cx).call_as_function(&url, window, cx);
    }

    fn accent_color(&self, theme: &AppTheme) -> Hsla {
        if self.is_on_accent_fill { white().opacity(0.85) } else { theme.primary._100 }
    }

    fn primary_text_color(&self, theme: &AppTheme) -> Hsla {
        if self.is_on_accent_fill { white() } else { theme.text.primary }
    }

    fn secondary_text_color(&self, theme: &AppTheme) -> Hsla {
        if self.is_on_accent_fill { white().opacity(0.85) } else { theme.text.secondary }
    }

    fn non_empty(value: Option<&str>) -> Option<String> {
        let trimmed = value.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    fn compact_minimum_height(&self) -> Option<f32> {
        if self.variant != LinkPreviewCardVariant::Compact || !self.reserves_compact_media_space {
            return None;
        }
        Some(self.compact_media_size + if self.shows_card_chrome { 20.0 } else { 0.0 })
    }

    fn compact_maximum_height(&self) -> Option<f32> {
        self.compact_minimum_height()
    }

    pub fn accessibility_text(&self) -> String {
        let values: Vec<String> = [
            self.presentation.display_site_name(),
            self.presentation.title.clone(),
            self.presentation.summary.clone(),
        ]
        .iter()
        .filter_map(|value| Self::non_empty(value.as_deref()))
        .collect();
        if values.is_empty() { self.presentation.url.clone() } else { values.join(". ") }
    }
}

impl RenderOnce for LinkPreviewCardUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let card = self.card(&theme, window, cx);
        if !self.opens_url_on_tap {
            return card.into_any_element();
        }
        let url: SharedString = self.presentation.url.clone().into();
        card.id(self.id.clone())
            .cursor_pointer()
            .on_click(move |_, window, cx| Self::open_preview_url(&url, window, cx))
            .into_any_element()
    }
}

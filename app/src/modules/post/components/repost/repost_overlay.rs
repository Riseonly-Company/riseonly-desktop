use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Background, Bounds, BoxShadow, ElementId, FontWeight, Hsla, IntoElement, Path, PathBuilder, Pixels,
    RenderOnce, Rgba, SharedString, Window, black, canvas, linear_color_stop, linear_gradient, point,
    transparent_black, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::material::UIBlurEffectStyle;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::prelude::*;
use rise_ui::main_text::IgnoresTextSizeDeltaKey;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::{IconUi, MainText, TextAlignT};

use crate::core::ui::animated_tabs_ui::inline_custom_emoji_text_hooks::inline_custom_emoji_text_element;
use crate::core::ui::formatted_text_ui::{FormattedTextOptions, FormattedTextStyle};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatComposerCustomEmojiRun;
use crate::modules::chat::stores::message::message_actions::message_types::MessageCustomEmojiEntity;
use crate::modules::post::components::repost::repost_opinion_editor::RepostOpinionEditorPresenter;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// namespace of the stickers flight
/// gpui has no matched geometry so the source and the flying copy share this
/// the frame a source reports under its matched id
#[derive(Clone, Default)]
pub struct RepostTransitionNamespace {
    frames: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
}

impl PartialEq for RepostTransitionNamespace {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.frames, &other.frames)
    }
}

impl RepostTransitionNamespace {
    fn record(&self, key: String, frame: Bounds<Pixels>) {
        self.frames.borrow_mut().insert(key, frame);
    }

    fn frame(&self, key: &str) -> Option<Bounds<Pixels>> {
        self.frames.borrow().get(key).copied()
    }

    pub fn forget_frames(&self) {
        self.frames.borrow_mut().clear();
    }
}

pub struct RepostTransitionNamespaceKey;

impl EnvKey for RepostTransitionNamespaceKey {
    type Value = Option<RepostTransitionNamespace>;

    fn default_value() -> Self::Value {
        None
    }
}

pub enum RepostOverlayMetrics {}

impl RepostOverlayMetrics {
    pub const MEDIA_BOTTOM_SAFE_ZONE: f32 = 44.0;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepostStickerStyle {
    Feed,
    Grid,
}

impl RepostStickerStyle {
    pub fn avatar_size(self) -> f32 {
        38.0
    }

    pub fn bubble_width(self) -> f32 {
        match self {
            Self::Feed => 76.0,
            Self::Grid => 96.0,
        }
    }

    pub fn text_size(self) -> f32 {
        10.0
    }

    pub fn shows_avatar(self) -> bool {
        match self {
            Self::Feed => true,
            Self::Grid => false,
        }
    }
}

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

#[derive(IntoElement)]
pub struct RepostOverlay {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    style: RepostStickerStyle,
}

impl RepostOverlay {
    /// default style is feed
    pub fn new(post: Rc<GetPostFeedResponse>) -> Self {
        let id = ElementId::Name(format!("repost-overlay-{}", post.id).into());
        Self { id, post, style: RepostStickerStyle::Feed }
    }

    pub fn style(mut self, style: RepostStickerStyle) -> Self {
        self.style = style;
        self
    }
}

impl RenderOnce for RepostOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source_instance_id = window
            .use_keyed_state(child_id(&self.id, "source-instance"), cx, |_, _| Uuid::new_v4().uuid_string())
            .read(cx)
            .clone();
        let namespace = env::<RepostTransitionNamespaceKey>(cx);
        let presenter = RepostOpinionEditorPresenter::shared(cx);
        let (matched_id, hides_source, can_start_transition) = {
            let presenter = presenter.read(cx);
            (
                presenter.matched_id(&source_instance_id),
                presenter.should_hide_source(&source_instance_id),
                presenter.can_start_transition(),
            )
        };

        let opinion = self.post.my_repost_content.clone().unwrap_or_default();
        let custom_emojis = self.post.my_repost_custom_emojis();

        let sticker = div()
            .flex()
            .flex_col()
            .items_start()
            .flex_none()
            .when(hides_source, |sticker| sticker.invisible())
            .child(RepostSticker::new(child_id(&self.id, "sticker"), opinion, self.style).custom_emojis(custom_emojis));
        let sized = match matched_id {
            Some(id) => RepostMatchedGeometry { id, namespace, is_source: true }.body(child_id(&self.id, "source"), sticker),
            None => sticker.into_any_element(),
        };

        let post = self.post;
        let style = self.style;
        let press_source = source_instance_id;
        let button = plain(child_id(&self.id, "button"))
            .disabled(!can_start_transition)
            .on_press(move |_, cx| {
                let post = GetPostFeedResponse::clone(&post);
                let source = press_source.clone();
                RepostOpinionEditorPresenter::shared(cx).update(cx, |presenter, cx| presenter.present(post, source, style, cx));
            })
            .flex()
            .flex_col()
            .items_start()
            .child(div().relative().flex().flex_col().items_start().flex_none().child(sized));
        div()
            .id(child_id(&self.id, "hint"))
            .flex()
            .flex_col()
            .items_start()
            .tooltip_label(tr("repost_reposted_label"))
            .child(button)
    }
}

#[derive(IntoElement)]
pub struct RepostSticker {
    id: ElementId,
    opinion: String,
    custom_emojis: Vec<MessageCustomEmojiEntity>,
    live_custom_emoji_runs: Vec<ChatComposerCustomEmojiRun>,
    style: RepostStickerStyle,
    layout_only: bool,
}

impl RepostSticker {
    pub const EXPANDED_SCALE: f32 = 1.28;

    /// defaults
    /// custom_emojis and live_custom_emoji_runs empty, layout_only false
    pub fn new(id: impl Into<ElementId>, opinion: impl Into<String>, style: RepostStickerStyle) -> Self {
        Self {
            id: id.into(),
            opinion: opinion.into(),
            custom_emojis: Vec::new(),
            live_custom_emoji_runs: Vec::new(),
            style,
            layout_only: false,
        }
    }

    pub fn custom_emojis(mut self, custom_emojis: Vec<MessageCustomEmojiEntity>) -> Self {
        self.custom_emojis = custom_emojis;
        self
    }

    pub fn live_custom_emoji_runs(mut self, live_custom_emoji_runs: Vec<ChatComposerCustomEmojiRun>) -> Self {
        self.live_custom_emoji_runs = live_custom_emoji_runs;
        self
    }

    pub fn layout_only(mut self, layout_only: bool) -> Self {
        self.layout_only = layout_only;
        self
    }

    fn normalized_opinion(&self) -> &str {
        trimming_whitespaces_and_newlines(&self.opinion)
    }

    fn resolved_entities(&self) -> Vec<MessageCustomEmojiEntity> {
        if self.live_custom_emoji_runs.is_empty() {
            return self.custom_emojis.clone();
        }
        self.live_custom_emoji_runs
            .iter()
            .map(|run| MessageCustomEmojiEntity {
                offset: run.range.location as i64,
                length: run.range.length as i64,
                custom_emoji_id: run.item.id.clone(),
                file_url: run.item.file_url.clone(),
                asset_type: run.item.asset_type.wire().to_owned(),
                width: run.item.width,
                height: run.item.height,
                pack_id: run.item.pack_id.clone(),
            })
            .collect()
    }

    fn bubble_text(&self) -> AnyElement {
        let normalized_opinion = self.normalized_opinion();
        let is_empty = normalized_opinion.is_empty();
        let entities = self.resolved_entities();
        if is_empty || entities.is_empty() {
            let text = if is_empty { tr("repost_add_opinion") } else { normalized_opinion.to_owned() };
            let label = MainText::new(text)
                .px(self.style.text_size())
                .tac(TextAlignT::Center)
                .font_weight(if is_empty { FontWeight::MEDIUM } else { FontWeight::SEMIBOLD })
                .color(if is_empty { white().opacity(0.58) } else { white() })
                .number_of_lines(3);
            return env_scope::<IgnoresTextSizeDeltaKey>(true).child(label).into_any_element();
        }
        let mut style = FormattedTextStyle::new(white(), white(), white().opacity(0.7));
        style.font_size = self.style.text_size();
        style.line_spacing = 1.0;
        style.code_foreground = Some(white());
        style.code_background = Some(transparent_black());
        let options = FormattedTextOptions {
            no_url_format: true,
            no_url_subtitle: true,
            no_mention_format: true,
            no_markdown: true,
            ..FormattedTextOptions::default()
        };
        div()
            .w_full()
            .text_center()
            .line_clamp(3)
            .child(inline_custom_emoji_text_element(
                child_id(&self.id, "text"),
                self.opinion.clone(),
                style,
                entities,
                options,
                None,
                None,
            ))
            .into_any_element()
    }

    fn bubble_layout(&self) -> Div {
        div()
            .relative()
            .flex_none()
            .w(px(self.style.bubble_width()))
            .pl(px(7.0))
            .pr(px(7.0))
            .pt(px(7.0))
            .pb(px(11.0))
    }

    fn bubble_layout_painted(&self) -> Div {
        let shadow = pass_through()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom(px(RepostBubbleShape::MAXIMUM_TAIL_HEIGHT))
            .rounded(px(RepostBubbleShape::MAXIMUM_RADIUS))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.2),
                offset: point(px(0.0), px(2.0)),
                blur_radius: px(5.0),
                spread_radius: px(0.0),
                inset: false,
            }]);
        self.bubble_layout().child(shadow).child(RepostBubbleGlass.view()).child(div().relative().child(self.bubble_text()))
    }

    fn avatar(&self, cx: &mut App) -> impl IntoElement + use<> {
        let (source, name, user_id) = {
            let store = AuthServicesStore::shared(cx);
            let store = store.read(cx);
            match store.profile() {
                Some(profile) => (
                    Some(SharedString::from(profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()))),
                    Some(SharedString::from(profile.name.clone())),
                    Some(SharedString::from(profile.id.clone())),
                ),
                None => (None, None, None),
            }
        };
        let size = self.style.avatar_size();
        let badge_size = size * 0.43;
        let bg_100 = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let gradient: Background = linear_gradient(
            135.0,
            linear_color_stop(Rgba { r: 0.67, g: 0.25, b: 1.0, a: 1.0 }, 0.0),
            linear_color_stop(Rgba { r: 0.39, g: 0.12, b: 0.96, a: 1.0 }, 1.0),
        );
        div()
            .relative()
            .flex_none()
            .w(px(size))
            .h(px(size))
            .child(
                UserLogoUi::new(child_id(&self.id, "avatar"))
                    .source(source)
                    .name(name)
                    .user_id(user_id)
                    .size(size)
                    .show_border(false)
                    .show_story_ring(false),
            )
            .child(
                div()
                    .absolute()
                    .right(px(-size * 0.06))
                    .bottom(px(-size * 0.06))
                    .w(px(badge_size))
                    .h(px(badge_size))
                    .rounded_full()
                    .bg(gradient)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("arrow.2.squarepath", px(badge_size * 0.48), white(), false))
                    .child(centred_stroke(bg_100, 1.5, Shape::Circle)),
            )
    }
}

impl RenderOnce for RepostSticker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style = self.style;
        let mut stack = vstack(0.0).align(HorizontalAlignment::Center).flex_none();
        if self.layout_only {
            stack = stack.child(self.bubble_layout().child(self.bubble_text()));
            if style.shows_avatar() {
                stack = stack.child(div().flex_none().mt(px(-5.0)).w(px(style.avatar_size())).h(px(style.avatar_size())));
            }
        } else {
            let bubble_height = window.use_keyed_state(child_id(&self.id, "bubble-height"), cx, |_, _| 0.0_f32);
            let pinned = *bubble_height.read(cx);
            let avatar_extra = if style.shows_avatar() { style.avatar_size() - 5.0 } else { 0.0 };
            if pinned > 0.0 {
                stack = stack.min_h(px(pinned + avatar_extra));
            }
            let reader = bubble_height.clone();
            let bubble = self.bubble_layout_painted().child(measure(child_id(&self.id, "bubble-measure"), move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                reader.update(cx, |pinned, cx| {
                    if (*pinned - height).abs() > 0.5 {
                        tracing::warn!(target: "riseonly::X574", "repost bubble height {height} (was {})", *pinned);
                        *pinned = height;
                        cx.notify();
                    }
                });
            }));
            stack = stack.child(bubble);
            if style.shows_avatar() {
                stack = stack.child(div().flex_none().mt(px(-5.0)).child(self.avatar(cx)));
            }
        }
        stack
    }
}

pub struct RepostBubbleShape;

impl RepostBubbleShape {
    const MAXIMUM_TAIL_HEIGHT: f32 = 8.0;
    const MAXIMUM_RADIUS: f32 = 16.0;

    pub fn path(rect: Bounds<Pixels>, mut path: PathBuilder) -> Option<Path<Pixels>> {
        let min_x = f32::from(rect.origin.x);
        let min_y = f32::from(rect.origin.y);
        let width = f32::from(rect.size.width);
        let height = f32::from(rect.size.height);
        let max_x = min_x + width;
        let max_y = min_y + height;

        let tail_height = Self::MAXIMUM_TAIL_HEIGHT.min(height * 0.18);
        let body_bottom = max_y - tail_height;
        let radius = Self::MAXIMUM_RADIUS.min((body_bottom - min_y) * 0.42);
        let tail_half_width = 10.0_f32.min(width * 0.14);
        let middle = min_x + width / 2.0;
        let at = |x: f32, y: f32| point(px(x), px(y));

        path.move_to(at(min_x + radius, min_y));
        path.line_to(at(max_x - radius, min_y));
        path.curve_to(at(max_x, min_y + radius), at(max_x, min_y));
        path.line_to(at(max_x, body_bottom - radius));
        path.curve_to(at(max_x - radius, body_bottom), at(max_x, body_bottom));
        path.line_to(at(middle + tail_half_width, body_bottom));
        path.cubic_bezier_to(at(middle + 3.0, max_y - 2.4), at(middle + 7.0, body_bottom + 0.5), at(middle + 3.8, max_y - 4.0));
        path.curve_to(at(middle, max_y), at(middle + 2.9, max_y));
        path.curve_to(at(middle - 3.0, max_y - 2.4), at(middle - 2.9, max_y));
        path.cubic_bezier_to(at(middle - tail_half_width, body_bottom), at(middle - 3.8, max_y - 4.0), at(middle - 7.0, body_bottom + 0.5));
        path.line_to(at(min_x + radius, body_bottom));
        path.curve_to(at(min_x, body_bottom - radius), at(min_x, body_bottom));
        path.line_to(at(min_x, min_y + radius));
        path.curve_to(at(min_x + radius, min_y), at(min_x, min_y));
        path.close();
        path.build().ok()
    }
}

/// modifier where the source reports its frame into the namespace
/// the flying copy reads it and moves itself since gpui has no matched geometry effect
pub struct RepostMatchedGeometry {
    pub id: String,
    pub namespace: Option<RepostTransitionNamespace>,
    pub is_source: bool,
}

impl RepostMatchedGeometry {
    fn key(&self) -> String {
        format!("repost-opinion-{}", self.id)
    }

    pub fn body(self, measure_id: impl Into<ElementId>, content: impl IntoElement) -> AnyElement {
        let Some(namespace) = self.namespace.clone().filter(|_| self.is_source) else {
            return content.into_any_element();
        };
        let key = self.key();
        let measure_id = ElementId::NamedChild(Arc::new(measure_id.into()), key.clone().into());
        div()
            .relative()
            .child(content)
            .child(measure(measure_id, move |frame, _, _| namespace.record(key, frame)))
            .into_any_element()
    }

    pub fn source_frame(&self) -> Option<Bounds<Pixels>> {
        self.namespace.as_ref()?.frame(&self.key())
    }
}

struct RepostStrongBlur;

impl RepostStrongBlur {
    fn color() -> Hsla {
        Rgba { r: 0.11, g: 0.11, b: 0.118, a: UIBlurEffectStyle::SystemChromeMaterialDark.painted_opacity() }.into()
    }
}

struct RepostBubbleGlass;

impl RepostBubbleGlass {
    /// blur plus black tint, clipped to the bubble and stroked
    fn view(&self) -> impl IntoElement + use<> {
        let material = RepostStrongBlur::color();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if let Some(path) = RepostBubbleShape::path(bounds, PathBuilder::fill()) {
                    window.paint_path(path, material);
                }
                if let Some(path) = RepostBubbleShape::path(bounds, PathBuilder::fill()) {
                    window.paint_path(path, black().opacity(0.12));
                }
                if let Some(path) = RepostBubbleShape::path(bounds, PathBuilder::stroke(px(0.35))) {
                    window.paint_path(path, white().opacity(0.2));
                }
            },
        )
        .absolute()
        .inset_0()
    }
}

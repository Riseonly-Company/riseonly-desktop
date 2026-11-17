use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::sync::{Arc, LazyLock};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, ElementId, Hitbox, HitboxBehavior, IntoElement, ParentElement, Pixels,
    RenderOnce, SharedString, Styled, Window, canvas, div, point, px, size,
};
use rise_core::foundation::foundation_data_detector::{NSDataDetector, NSTextCheckingType};
use rise_core::foundation::foundation_regex::NSMatchingOptions;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, removing_percent_encoding};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationFeedbackGenerator, NotificationKind, SelectionFeedbackGenerator};
use rise_platform::open_url::open_url;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::core::ui::formatted_text_ui::{
    FormattedTextOptions, FormattedTextPreparation, FormattedTextRenderSeed, FormattedTextResult, FormattedTextSelection,
    FormattedTextStyle, FormattedTextUi, RoleMentionSpan,
};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::chat::components::chat::bubble::bubble_tap_router::{
    BubbleTapRouter, BubbleTapRowHostKey, BubbleTapView,
};
use crate::modules::chat::components::chat::bubble::chat_prepared_text_layout::{
    ChatPreparedTextLayout, ChatPreparedTextLayoutCache, ChatTextHitLayoutSeed,
};
use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer::TopicMentionComposer;

/// callback a tapped mention, link or tag reports its tag to
pub type ChatMentionTap = Rc<dyn Fn(&str, &mut Window, &mut App)>;

#[derive(IntoElement)]
struct MentionHitReporter {
    id: ElementId,
    seed: FormattedTextRenderSeed,
    width_hint: Option<f32>,
    display_range: Option<NSRange>,
    revealed_spoilers: Option<HashSet<usize>>,
    on_tap: ChatMentionTap,
}

impl RenderOnce for MentionHitReporter {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        ChatPreparedTextLayoutCache::install_text_system(cx);
        let row = env::<BubbleTapRowHostKey>(cx);
        let reporter = window.use_keyed_state(self.id, cx, |_, _| ReporterView::new());
        reporter.update(cx, |reporter, _| {
            reporter.configure(self.seed, self.width_hint, self.display_range, self.revealed_spoilers, self.on_tap);
        });
        canvas(
            move |bounds, window, cx| reporter.update(cx, |reporter, _| reporter.layout_subviews(bounds, row, window)),
            |_, hitboxes: Vec<Hitbox>, window, _| {
                for hitbox in hitboxes.iter() {
                    window.set_cursor_style(CursorStyle::PointingHand, hitbox);
                }
            },
        )
        .absolute()
        .inset_0()
    }
}

struct ReporterView {
    seed: Option<FormattedTextRenderSeed>,
    display_range: Option<NSRange>,
    layout_seed: Option<ChatTextHitLayoutSeed>,
    preparation: Option<Arc<FormattedTextPreparation<ChatPreparedTextLayout>>>,
    registered_seed: Option<ChatTextHitLayoutSeed>,
    revealed_spoilers: Option<HashSet<usize>>,
    on_tap: Rc<RefCell<ChatMentionTap>>,
    registrations: Vec<(Uuid, String)>,
    view: BubbleTapView,
}

impl ReporterView {
    fn new() -> Self {
        Self {
            seed: None,
            display_range: None,
            layout_seed: None,
            preparation: None,
            registered_seed: None,
            revealed_spoilers: None,
            on_tap: Rc::new(RefCell::new(Self::noop())),
            registrations: Vec::new(),
            view: BubbleTapView::new(),
        }
    }

    fn noop() -> ChatMentionTap {
        Rc::new(|_: &str, _: &mut Window, _: &mut App| {})
    }

    fn configure(
        &mut self,
        seed: FormattedTextRenderSeed,
        width_hint: Option<f32>,
        display_range: Option<NSRange>,
        revealed_spoilers: Option<HashSet<usize>>,
        on_tap: ChatMentionTap,
    ) {
        if self.seed.as_ref() != Some(&seed) || self.display_range != display_range {
            self.unregister_all();
            self.layout_seed = None;
            self.preparation = None;
        } else if self.revealed_spoilers != revealed_spoilers {
            self.unregister_all();
        }
        self.revealed_spoilers = revealed_spoilers;
        self.seed = Some(seed);
        self.display_range = display_range;
        *self.on_tap.borrow_mut() = on_tap;
        if let Some(width_hint) = width_hint {
            self.prepare(width_hint);
        }
    }

    fn layout_subviews(&mut self, bounds: Bounds<Pixels>, row: Option<ElementId>, window: &mut Window) -> Vec<Hitbox> {
        self.view.report_frame(bounds, row, window);
        self.sync();
        let Some(preparation) = self.preparation.as_ref() else {
            return Vec::new();
        };
        if f32::from(bounds.size.width) <= 1.0 {
            return Vec::new();
        }
        preparation
            .value_for_immediate_display()
            .hits
            .iter()
            .filter(|hit| self.is_live(&hit.tag))
            .map(|hit| {
                let rect = Self::tap_frame(hit.rect);
                let in_window = Bounds { origin: bounds.origin + rect.origin, size: rect.size };
                window.insert_hitbox(in_window, HitboxBehavior::Normal)
            })
            .collect()
    }

    fn prepare(&mut self, width: f32) {
        let Some(seed) = self.seed.clone() else {
            return;
        };
        if !width.is_finite() || width <= 1.0 {
            return;
        }
        let key = ChatTextHitLayoutSeed::new(seed, width, self.display_range);
        if self.layout_seed.as_ref() == Some(&key) {
            return;
        }
        self.preparation = Some(ChatPreparedTextLayoutCache::request(&key));
        self.layout_seed = Some(key);
    }

    fn sync(&mut self) {
        let width = self.view.bounds().map_or(0.0, |bounds| f32::from(bounds.size.width));
        if !width.is_finite() || width <= 1.0 {
            self.unregister_all();
            return;
        }
        self.prepare(width);
        let (Some(layout_seed), Some(preparation)) = (self.layout_seed.clone(), self.preparation.clone()) else {
            return;
        };
        if self.registered_seed.as_ref() == Some(&layout_seed) {
            return;
        }

        let locals: Vec<_> = preparation
            .value_for_immediate_display()
            .hits
            .into_iter()
            .filter(|hit| self.is_live(&hit.tag))
            .collect();
        let same_tags = locals.len() == self.registrations.len()
            && locals.iter().zip(self.registrations.iter()).all(|(hit, (_, tag))| hit.tag == *tag);
        if same_tags {
            BubbleTapRouter::shared(|router| {
                for (hit, (id, _)) in locals.iter().zip(self.registrations.iter()) {
                    router.update(*id, Self::tap_frame(hit.rect));
                }
            });
        } else {
            self.unregister_all();
            for hit in locals {
                let tag = hit.tag.clone();
                let weak_tap: Weak<RefCell<ChatMentionTap>> = Rc::downgrade(&self.on_tap);
                let callback_tag = tag.clone();
                let view = self.view.clone();
                let id = BubbleTapRouter::shared(|router| {
                    router.register_local(
                        &view,
                        Self::tap_frame(hit.rect),
                        false,
                        Rc::new(move |window: &mut Window, cx: &mut App| {
                            let current = weak_tap.upgrade().map(|tap| tap.borrow().clone());
                            if let Some(on_tap) = current {
                                on_tap(&callback_tag, window, cx);
                            }
                        }),
                    )
                });
                self.registrations.push((id, tag));
            }
        }
        self.registered_seed = Some(layout_seed);
    }

    /// tap rect grown by 4 horizontally and 3 vertically
    fn tap_frame(rect: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: rect.origin - point(px(4.0), px(3.0)),
            size: rect.size + size(px(8.0), px(6.0)),
        }
    }

    fn is_live(&self, tag: &str) -> bool {
        let Some(rest) = tag.strip_prefix(ChatPreparedTextLayoutCache::SPOILER_MARKER) else {
            return true;
        };
        let (Some(revealed), Ok(index)) = (self.revealed_spoilers.as_ref(), rest.parse::<usize>()) else {
            return false;
        };
        !revealed.contains(&index)
    }

    fn unregister_all(&mut self) {
        let registrations = std::mem::take(&mut self.registrations);
        if !registrations.is_empty() {
            BubbleTapRouter::shared_if_alive(|router| {
                for (id, _) in registrations {
                    router.unregister(id);
                }
            });
        }
        self.registered_seed = None;
    }
}

impl ReporterView {
    fn dismantle(&mut self) {
        self.unregister_all();
        self.seed = None;
        self.display_range = None;
        self.revealed_spoilers = None;
        self.layout_seed = None;
        self.preparation = None;
    }
}

impl Drop for ReporterView {
    fn drop(&mut self) {
        self.dismantle();
    }
}

/// defaulted params of the chat mention taps modifier
#[derive(Clone, Default)]
pub struct ChatMentionTapsOptions {
    pub options: FormattedTextOptions,
    pub role_mentions: Vec<RoleMentionSpan>,
    pub width_hint: Option<f32>,
    pub display_range: Option<NSRange>,
    pub revealed_spoilers: Option<Binding<HashSet<usize>>>,
    pub on_link: Option<ChatMentionTap>,
}

#[derive(IntoElement)]
struct ChatMentionTaps {
    id: ElementId,
    content: AnyElement,
    seed: FormattedTextRenderSeed,
    width_hint: Option<f32>,
    display_range: Option<NSRange>,
    revealed_spoilers: Option<Binding<HashSet<usize>>>,
    on_link: Option<ChatMentionTap>,
    on_tap: ChatMentionTap,
}

impl RenderOnce for ChatMentionTaps {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let revealed_now = self.revealed_spoilers.as_ref().map(|binding| binding.get(cx));
        let revealed_spoilers = self.revealed_spoilers.clone();
        let on_link = self.on_link.clone();
        let on_tap = self.on_tap.clone();
        let reporter_tap: ChatMentionTap = Rc::new(move |value: &str, window: &mut Window, cx: &mut App| {
            if let Some(rest) = value.strip_prefix(ChatPreparedTextLayoutCache::SPOILER_MARKER) {
                let (Some(binding), Ok(index)) = (revealed_spoilers.as_ref(), rest.parse::<usize>()) else {
                    return;
                };
                let _ = SelectionFeedbackGenerator::new().selection_changed();
                let mut revealed = binding.get(cx);
                revealed.insert(index);
                binding.set(revealed, window, cx);
            } else if let Some(raw) = value.strip_prefix(ChatPreparedTextLayoutCache::URL_MARKER) {
                match on_link.as_ref() {
                    Some(on_link) => on_link(raw, window, cx),
                    None => ChatLinkOpen::open(raw, cx),
                }
            } else {
                on_tap(value, window, cx);
            }
        });
        div().relative().child(self.content).child(MentionHitReporter {
            id: self.id,
            seed: self.seed,
            width_hint: self.width_hint,
            display_range: self.display_range,
            revealed_spoilers: revealed_now,
            on_tap: reporter_tap,
        })
    }
}

/// opens chat links, own urls go in app through the universal link parser and foreign ones go out
/// one place so parsers dont diverge
pub enum ChatLinkOpen {}

static LINK_DETECTOR: LazyLock<Option<NSDataDetector>> =
    LazyLock::new(|| NSDataDetector::new(NSTextCheckingType::LINK).ok());

const MAILTO_SCHEME: &str = "mailto";
const MAILTO_PREFIX: &str = "mailto:";

impl ChatLinkOpen {
    pub fn url(from: &str) -> Option<Url> {
        let trimmed = trimming_whitespaces_and_newlines(from);
        if trimmed.is_empty() {
            return None;
        }
        let lowered = trimmed.to_lowercase();
        if lowered.starts_with("http") || lowered.starts_with(MAILTO_PREFIX) || lowered.starts_with("tel:") || trimmed.contains("://") {
            return Url::from_string(trimmed);
        }
        if let Some(mail) = Self::mail_url(trimmed) {
            return Some(mail);
        }
        Url::from_string(&format!("https://{trimmed}"))
    }

    fn mail_url(text: &str) -> Option<Url> {
        let range = NSRange::full(text);
        let detected = LINK_DETECTOR.as_ref()?.first_match(text, NSMatchingOptions::NONE, range)?;
        if detected.range != range {
            return None;
        }
        let url = detected.url?;
        (url.scheme().map(str::to_lowercase).as_deref() == Some(MAILTO_SCHEME)).then_some(url)
    }

    pub fn open(raw: &str, cx: &mut App) {
        let Some(url) = Self::url(raw) else {
            return;
        };
        if let Some(link) = UniversalLinkParser::parse(&url, AppEnvironment::current()) {
            if let PushDeepLink::ChatTopic { chat_id, topic_id } = &link
                && TopicMentionComposer::open(chat_id, topic_id, cx)
            {
                return;
            }
            let store = PushDeepLinkStore::shared(cx);
            store.update(cx, |store, cx| {
                store.enqueue(link, cx);
            });
            return;
        }
        if url.scheme().map(str::to_lowercase).as_deref() != Some(MAILTO_SCHEME) {
            if !InAppLinkBrowser::open(&url, cx) {
                open_url(&url, cx);
            }
            return;
        }
        if open_url(&url, cx) {
            return;
        }
        let absolute = url.absolute_string();
        let address = absolute.get(MAILTO_PREFIX.len()..).unwrap_or_default();
        let copied = removing_percent_encoding(address).unwrap_or_else(|| address.to_owned());
        UIPasteboard::general().set_string(&copied, cx);
        let _ = NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Success);
        Notifier::send(tr("copied"), NotifierSendOptions { r#type: NotifierType::Success, ..NotifierSendOptions::default() }, cx);
    }
}

/// makes mentions and links and tags and spoilers answer taps
/// id keys the hit layer across frames
pub trait ChatMentionTapLayerModifiers: IntoElement + Sized {
    fn chat_mention_taps(
        self,
        id: impl Into<ElementId>,
        text: String,
        style: FormattedTextStyle,
        options: ChatMentionTapsOptions,
        on_tap: ChatMentionTap,
    ) -> impl IntoElement {
        ChatMentionTaps {
            id: id.into(),
            content: self.into_any_element(),
            seed: FormattedTextRenderSeed::new(text, style, options.options, options.role_mentions),
            width_hint: options.width_hint,
            display_range: options.display_range,
            revealed_spoilers: options.revealed_spoilers,
            on_link: options.on_link,
            on_tap,
        }
    }
}

impl<E: IntoElement> ChatMentionTapLayerModifiers for E {}

/// chat text where spoilers open one by one, only the opened ones show
#[derive(IntoElement)]
pub struct ChatSpoilerFormattedText {
    id: ElementId,
    pub text: String,
    pub style: FormattedTextStyle,
    pub options: FormattedTextOptions,
    pub role_mentions: Vec<RoleMentionSpan>,
    pub preformatted: Option<Arc<FormattedTextResult>>,
    pub width_hint: Option<f32>,
    pub text_selection: Option<FormattedTextSelection>,
    pub on_tap: ChatMentionTap,
}

impl ChatSpoilerFormattedText {
    pub fn new(id: impl Into<ElementId>, text: impl Into<String>, style: FormattedTextStyle, on_tap: ChatMentionTap) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            style,
            options: FormattedTextOptions::default(),
            role_mentions: Vec::new(),
            preformatted: None,
            width_hint: None,
            text_selection: None,
            on_tap,
        }
    }

    pub fn options(mut self, options: FormattedTextOptions) -> Self {
        self.options = options;
        self
    }

    pub fn role_mentions(mut self, role_mentions: Vec<RoleMentionSpan>) -> Self {
        self.role_mentions = role_mentions;
        self
    }

    pub fn preformatted(mut self, preformatted: Option<Arc<FormattedTextResult>>) -> Self {
        self.preformatted = preformatted;
        self
    }

    pub fn width_hint(mut self, width_hint: Option<f32>) -> Self {
        self.width_hint = width_hint;
        self
    }

    pub fn text_selection(mut self, text_selection: Option<FormattedTextSelection>) -> Self {
        self.text_selection = text_selection;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

impl RenderOnce for ChatSpoilerFormattedText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let revealed_state = window.use_keyed_state(self.child_id("revealed"), cx, |_, _| HashSet::<usize>::new());
        let revealed = Binding::entity(
            revealed_state,
            |revealed: &HashSet<usize>| revealed.clone(),
            |revealed: &mut HashSet<usize>, value: HashSet<usize>, _| *revealed = value,
        );
        let taps_id = self.child_id("mention-taps");
        FormattedTextUi::new(self.child_id("text"), self.text.clone(), self.style.clone())
            .options(self.options.clone())
            .shows_subtitle(false)
            .role_mentions(self.role_mentions.clone())
            .preformatted(self.preformatted)
            .revealed_spoilers(Some(revealed.clone()))
            .text_selection(self.text_selection)
            .chat_mention_taps(
                taps_id,
                self.text,
                self.style,
                ChatMentionTapsOptions {
                    options: self.options,
                    role_mentions: self.role_mentions,
                    width_hint: self.width_hint,
                    revealed_spoilers: Some(revealed),
                    ..ChatMentionTapsOptions::default()
                },
                self.on_tap,
            )
    }
}

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, AppContext, AsyncApp, Bounds, Context, Edges, ElementId, Entity, Focusable, FontWeight, Global, Hsla, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Pixels, RenderOnce, StyleRefinement, Styled, Subscription, Task,
    WeakEntity, Window, WindowId, actions, div, point, px,
};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_cache::NSCache;
use rise_core::foundation::ns_range::{self, NSRange, byte_to_utf16_range, length_utf16, utf16_to_byte_range};
use rise_i18n::tr;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::input_ui::{
    Confirm, InputEditSubmenu, InputMode, InputUiEvent, InputUiState, InputUiTextViewDelegate, RunAttributes, Selection,
    TextAttachment, TextAttachmentId,
};
use rise_ui::keyboard_dismiss::{EditMenuPresentation, ScreenFocus};
use rise_ui::rich_text::rich_text_markup::{RichEditorStyle, RichTextMarkup, RichTextMarkupFormat};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::rich_text::{AttributeContainer, AttributedString, RichFont};
use rise_ui::theme;
use rise_ui::ui_text_view_content_height::{UITextViewContentHeight, UITextViewContentLayout};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapLoader, CleverImageDiskCache, CleverImageRenderMemoryCache, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::custom_emoji_glyph::CustomEmojiGlyphMetrics;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{LottieStickerLoader, LottieStickerPayload};
use crate::core::ui::lottie_sticker::lottie_sticker_prefetch::LottieStickerPrefetch;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::{
    RISE_LOTTIE_FIRST_FRAME_CACHE_DID_CHANGE, RiseLottieFirstFrameCache,
};
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatComposerCustomEmojiAttachmentPayload, ChatComposerCustomEmojiPlaceholder, ChatComposerCustomEmojiRun,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    StickerAssetType, StickerAssetTypeInfer, StickerItem,
};

/// encoded payload an emoji attachment carries so a run survives the attachment being rebuilt
#[derive(Clone, PartialEq, Debug)]
struct ChatCustomEmojiMetadata(String);

const RISE_CHAT_COMPOSER_STATIC_EMOJI_CACHE_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("riseonly.chat-composer.static-emoji-cache-did-change");

const PREVIEW_MEMORY_COUNT_LIMIT: usize = 96;
const PREVIEW_MEMORY_TOTAL_COST_LIMIT: usize = 12 * 1_024 * 1_024;

static PREVIEW_MEMORY: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(PREVIEW_MEMORY_COUNT_LIMIT);
    cache.set_total_cost_limit(PREVIEW_MEMORY_TOTAL_COST_LIMIT);
    cache
});

enum ChatComposerCustomEmojiPreviewCache {}

impl ChatComposerCustomEmojiPreviewCache {
    fn raster_image(url_string: &str, account_user_id: Option<&str>, account_epoch: Option<&str>) -> Option<CleverImage> {
        let raw = trimming_whitespaces_and_newlines(url_string);
        if raw.is_empty() {
            return None;
        }
        let normalized = RemoteMediaURL::optional_normalized(Some(raw)).unwrap_or_else(|| raw.to_owned());
        let mut seen = HashSet::new();
        let cache_keys: Vec<String> = [normalized, raw.to_owned()]
            .into_iter()
            .filter(|key| !key.is_empty() && seen.insert(key.clone()))
            .collect();
        for cache_key in &cache_keys {
            if let Some(image) = PREVIEW_MEMORY.object(&Self::scoped_key(cache_key, account_user_id, account_epoch)) {
                return Some(image);
            }
            if let Some(image) = MediaViewerImageMemoryCache::memory_only(cache_key, account_user_id) {
                Self::store(&image, &cache_keys, account_user_id, account_epoch);
                return Some(image);
            }
            if let Some(image) = CleverImageRenderMemoryCache::latest_image(
                &CleverImageDiskCache::normalized_cache_key(cache_key),
                account_user_id,
                account_epoch,
            ) {
                Self::store(&image, &cache_keys, account_user_id, account_epoch);
                return Some(image);
            }
        }
        None
    }

    fn store(image: &CleverImage, aliases: &[String], account_user_id: Option<&str>, account_epoch: Option<&str>) {
        let size = image.size(0);
        let pixels = i64::from(size.width.0).saturating_mul(i64::from(size.height.0));
        let cost = usize::try_from(pixels.saturating_mul(4)).unwrap_or(0);
        for alias in aliases {
            PREVIEW_MEMORY.set_object_cost(
                image.clone(),
                Self::scoped_key(alias, account_user_id, account_epoch),
                cost,
            );
        }
    }

    fn scoped_key(source: &str, account_user_id: Option<&str>, account_epoch: Option<&str>) -> String {
        format!(
            "{}|{}|{}",
            account_user_id.unwrap_or("anonymous"),
            account_epoch.unwrap_or("none"),
            source
        )
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct ChatComposerStaticEmojiPreviewPrefetcherRequest {
    url: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

/// one piece of state behind a lock and every entry runs on the main executor
struct ChatComposerStaticEmojiPreviewPrefetcher {
    active_requests: Mutex<HashSet<ChatComposerStaticEmojiPreviewPrefetcherRequest>>,
}

static SHARED_PREFETCHER: LazyLock<ChatComposerStaticEmojiPreviewPrefetcher> =
    LazyLock::new(|| ChatComposerStaticEmojiPreviewPrefetcher {
        active_requests: Mutex::new(HashSet::new()),
    });

impl ChatComposerStaticEmojiPreviewPrefetcher {
    fn shared() -> &'static Self {
        &SHARED_PREFETCHER
    }

    fn warm(urls: Vec<String>, cx: &mut App) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |cx| {
            Self::shared().enqueue(&urls, account_user_id, account_epoch, cx);
        })
        .detach();
    }

    fn enqueue(
        &self,
        urls: &[String],
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) {
        let mut seen = HashSet::new();
        for raw_url in urls {
            let url = RemoteMediaURL::optional_normalized(Some(raw_url.as_str()))
                .unwrap_or_else(|| trimming_whitespaces_and_newlines(raw_url).to_owned());
            if url.is_empty() || !seen.insert(url.clone()) {
                continue;
            }
            if ChatComposerCustomEmojiPreviewCache::raster_image(&url, account_user_id.as_deref(), account_epoch.as_deref())
                .is_some()
            {
                RiseAppEvents::default_center().post_with(RISE_CHAT_COMPOSER_STATIC_EMOJI_CACHE_DID_CHANGE, url);
                continue;
            }
            let request = ChatComposerStaticEmojiPreviewPrefetcherRequest {
                url,
                account_user_id: account_user_id.clone(),
                account_epoch: account_epoch.clone(),
            };
            if !self.active_requests.lock().insert(request.clone()) {
                continue;
            }
            cx.spawn(async move |cx| {
                let disk_image = CleverImageBitmapLoader::load(
                    &request.url,
                    true,
                    false,
                    request.account_user_id.clone(),
                    request.account_epoch.clone(),
                    None,
                    CleverImageCachePolicy::Standard,
                    CleverImageWorkLane::Visible,
                    cx,
                )
                .await;
                let loaded = if disk_image.is_some() {
                    true
                } else {
                    CleverImageBitmapLoader::load(
                        &request.url,
                        false,
                        true,
                        request.account_user_id.clone(),
                        request.account_epoch.clone(),
                        None,
                        CleverImageCachePolicy::Standard,
                        CleverImageWorkLane::Visible,
                        cx,
                    )
                    .await
                    .is_some()
                };
                Self::shared().finish(&request, loaded);
            })
            .detach();
        }
    }

    fn finish(&self, request: &ChatComposerStaticEmojiPreviewPrefetcherRequest, loaded: bool) {
        self.active_requests.lock().remove(request);
        if !loaded {
            return;
        }
        RiseAppEvents::default_center().post_with(RISE_CHAT_COMPOSER_STATIC_EMOJI_CACHE_DID_CHANGE, request.url.clone());
    }
}

/// text view whose intrinsic height is the height the coordinator fitted
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChatComposerSizingTextView {
    layout_content_height: f32,
}

impl Default for ChatComposerSizingTextView {
    fn default() -> Self {
        Self {
            layout_content_height: 44.0,
        }
    }
}

impl ChatComposerSizingTextView {
    pub fn layout_content_height(&self) -> f32 {
        self.layout_content_height
    }

    pub fn set_layout_content_height(&mut self, layout_content_height: f32) -> bool {
        let old_value = self.layout_content_height;
        self.layout_content_height = layout_content_height;
        (layout_content_height - old_value).abs() > 0.5
    }

    pub fn intrinsic_content_size(&self) -> f32 {
        self.layout_content_height
    }
}

actions!(chat_composer_text_view, [FormatBold, FormatItalic, FormatStrikethrough, InsertLineBreak]);

const KEY_CONTEXT: &str = "ChatComposerTextView";

struct ChatComposerTextViewKeyBindingsInstalled;

impl Global for ChatComposerTextViewKeyBindingsInstalled {}

#[derive(Default)]
struct ChatComposerTextViewFocusCarry {
    last_focused: Option<(WindowId, ElementId, Duration)>,
    last_selection: Option<(String, NSRange)>,
}

impl ChatComposerTextViewFocusCarry {
    const WINDOW: Duration = Duration::from_secs(1);
    const IDLE_WINDOW: Duration = Duration::from_secs(30);
}

impl Global for ChatComposerTextViewFocusCarry {}

/// binds the composer chords once, called at launch with the app menu
/// a binding added later refreshes every window and redrew the whole first opened chat uncached
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<ChatComposerTextViewKeyBindingsInstalled>() {
        return;
    }
    cx.set_global(ChatComposerTextViewKeyBindingsInstalled);
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-b", FormatBold, context),
        KeyBinding::new("secondary-i", FormatItalic, context),
        KeyBinding::new("secondary-shift-x", FormatStrikethrough, context),
        KeyBinding::new("shift-enter", InsertLineBreak, context),
    ]);
}

pub type ChatComposerSelectionChange = Rc<dyn Fn(NSRange, &mut App)>;
pub type ChatComposerCustomEmojiRunsChange = Rc<dyn Fn(Vec<ChatComposerCustomEmojiRun>, &mut App)>;
pub type ChatComposerSend = Rc<dyn Fn(&mut Window, &mut App)>;
pub type ChatComposerPasteHandler = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

#[derive(IntoElement)]
pub struct ChatComposerTextView {
    id: ElementId,
    text: Binding<String>,
    width: f32,
    min_height: f32,
    max_height: f32,
    text_color: Hsla,
    editor_style: RichEditorStyle,
    measured_height: Binding<f32>,
    custom_emoji_runs: Vec<ChatComposerCustomEmojiRun>,
    focus_token: i64,
    requested_selection: Option<NSRange>,
    selection_token: i64,
    on_selection_change: ChatComposerSelectionChange,
    on_custom_emoji_runs_change: ChatComposerCustomEmojiRunsChange,
    on_send: Option<ChatComposerSend>,
    on_cancel: Option<ChatComposerSend>,
    on_paste: Option<ChatComposerPasteHandler>,
    holds_custom_emoji_line_height: bool,
}

/// font numbers cap height, ascender and descender (negative)
/// plus line height
#[derive(Clone, Copy, Debug)]
pub struct ChatComposerFontMetrics {
    pub point_size: f32,
    pub cap_height: f32,
    pub ascender: f32,
    pub descender: f32,
    pub line_height: f32,
}

impl ChatComposerFontMetrics {
    pub fn system(font_size: f32, weight: FontWeight, cx: &App) -> Self {
        let text_system = cx.text_system();
        let font_id = text_system.resolve_font(&theme(cx).typography.font(weight));
        let size = px(font_size);
        let cap_height = f32::from(text_system.cap_height(font_id, size));
        let ascender = f32::from(text_system.ascent(font_id, size)).abs();
        let descender = -f32::from(text_system.descent(font_id, size)).abs();
        Self { point_size: font_size, cap_height, ascender, descender, line_height: ascender - descender }
    }
}

impl ChatComposerTextView {
    /// extra height a held line has over the plain font line, for placeholders laid over it
    pub fn held_line_extra(font_size: f32, cx: &App) -> f32 {
        let font = ChatComposerFontMetrics::system(font_size, FontWeight::NORMAL, cx);
        ChatComposerTextViewCoordinator::held_line_height(&font) - font.line_height
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        text: Binding<String>,
        width: f32,
        min_height: f32,
        max_height: f32,
        text_color: Hsla,
        editor_style: RichEditorStyle,
        measured_height: Binding<f32>,
    ) -> Self {
        Self {
            id: id.into(),
            text,
            width,
            min_height,
            max_height,
            text_color,
            editor_style,
            measured_height,
            custom_emoji_runs: Vec::new(),
            focus_token: 0,
            requested_selection: None,
            selection_token: 0,
            on_selection_change: Rc::new(|_, _| {}),
            on_custom_emoji_runs_change: Rc::new(|_, _| {}),
            on_send: None,
            on_cancel: None,
            on_paste: None,
            holds_custom_emoji_line_height: false,
        }
    }

    /// every line is as tall as one holding a custom emoji
    /// so inserting the first one neither grows a one line field nor moves its text
    pub fn holds_custom_emoji_line_height(mut self, holds_custom_emoji_line_height: bool) -> Self {
        self.holds_custom_emoji_line_height = holds_custom_emoji_line_height;
        self
    }

    pub fn custom_emoji_runs(mut self, custom_emoji_runs: Vec<ChatComposerCustomEmojiRun>) -> Self {
        self.custom_emoji_runs = custom_emoji_runs;
        self
    }

    pub fn focus_token(mut self, focus_token: i64) -> Self {
        self.focus_token = focus_token;
        self
    }

    pub fn requested_selection(mut self, requested_selection: Option<NSRange>) -> Self {
        self.requested_selection = requested_selection;
        self
    }

    pub fn selection_token(mut self, selection_token: i64) -> Self {
        self.selection_token = selection_token;
        self
    }

    pub fn on_selection_change(mut self, on_selection_change: impl Fn(NSRange, &mut App) + 'static) -> Self {
        self.on_selection_change = Rc::new(on_selection_change);
        self
    }

    pub fn on_custom_emoji_runs_change(
        mut self,
        on_custom_emoji_runs_change: impl Fn(Vec<ChatComposerCustomEmojiRun>, &mut App) + 'static,
    ) -> Self {
        self.on_custom_emoji_runs_change = Rc::new(on_custom_emoji_runs_change);
        self
    }

    /// desktop enter, runs the parent send button. without it enter breaks the line
    pub fn on_send(mut self, on_send: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_send = Some(Rc::new(on_send));
        self
    }

    /// desktop escape once the field has no composition left to take back, runs the parent close control
    /// without it escape does nothing
    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    /// asked before a paste edits the text, true means the parent took the clipboard
    /// pasted images, videos and files go to the paste sheet instead
    pub fn on_paste(mut self, on_paste: impl Fn(&mut Window, &mut App) -> bool + 'static) -> Self {
        self.on_paste = Some(Rc::new(on_paste));
        self
    }

    fn make_coordinator(
        &self,
        window: &mut Window,
        cx: &mut Context<ChatComposerTextViewCoordinator>,
    ) -> ChatComposerTextViewCoordinator {
        ChatComposerTextViewCoordinator::new(self, window, cx)
    }

    fn make_ui_view(&self, cx: &mut Context<ChatComposerTextViewCoordinator>) -> Entity<InputUiState> {
        let text = self.text.get(cx);
        cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::MultiLine, cx);
            field.set_chromeless(true, cx);
            field.set_escape_to_layer(false);
            field.set_wraps(true, cx);
            field.set_font_size(Some(px(MessageSizes::composer_text_font_size())), cx);
            field.reset(text, cx);
            field
        })
    }

    fn text_container_inset() -> Edges<Pixels> {
        Edges {
            top: px(10.0),
            left: px(12.0),
            bottom: px(10.0),
            right: px(44.0),
        }
    }
}

/// what the delegate callbacks read while the field is mid update
/// they cant touch the field or coordinator then so the coordinator mirrors this state here
struct ChatComposerTextViewDelegateState {
    text_binding: RefCell<Binding<String>>,
    selection_handler: RefCell<ChatComposerSelectionChange>,
    last_synced_text: RefCell<String>,
    shown_text: RefCell<String>,
    cancel_handler: RefCell<Option<ChatComposerSend>>,
    paste_handler: RefCell<Option<ChatComposerPasteHandler>>,
    is_composing: Cell<bool>,
    applying_pending_text: Cell<bool>,
    user_edit: Cell<bool>,
    typed_over_pending_text: Cell<bool>,
    carry_key: RefCell<Option<(WindowId, ElementId)>>,
}

pub struct ChatComposerTextViewCoordinator {
    field: Entity<InputUiState>,
    delegate_state: Rc<ChatComposerTextViewDelegateState>,
    sizing: ChatComposerSizingTextView,
    height_binding: Binding<f32>,
    width: f32,
    min_height: f32,
    max_height: f32,
    last_measured_width: f32,
    last_focus_token: i64,
    last_selection_token: i64,
    editor_style: Option<RichEditorStyle>,
    was_styled: bool,
    last_text_color: Option<Hsla>,
    custom_emoji_runs_handler: ChatComposerCustomEmojiRunsChange,
    custom_emoji_runs: Vec<ChatComposerCustomEmojiRun>,
    holds_custom_emoji_line_height: bool,
    attachment_items: HashMap<TextAttachmentId, StickerItem>,
    requested_first_frames: HashSet<String>,
    _observers: Vec<RiseSubscription>,
    _observer_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl ChatComposerTextViewCoordinator {
    fn new(view: &ChatComposerTextView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);
        let seed = view.text.get(cx);
        let field = view.make_ui_view(cx);
        let delegate_state = Rc::new(ChatComposerTextViewDelegateState {
            text_binding: RefCell::new(view.text.clone()),
            selection_handler: RefCell::new(view.on_selection_change.clone()),
            last_synced_text: RefCell::new(String::new()),
            shown_text: RefCell::new(seed),
            cancel_handler: RefCell::new(view.on_cancel.clone()),
            paste_handler: RefCell::new(view.on_paste.clone()),
            is_composing: Cell::new(false),
            applying_pending_text: Cell::new(false),
            user_edit: Cell::new(false),
            carry_key: RefCell::new(None),
            typed_over_pending_text: Cell::new(false),
        });
        let handle = field.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, _, event: &InputUiEvent, window, cx| {
                if *event == InputUiEvent::Changed {
                    this.field_did_change(window, cx);
                } else if *event == InputUiEvent::Cancelled {
                    let handler = this.delegate_state.cancel_handler.borrow().clone();
                    if let Some(handler) = handler {
                        window.defer(cx, move |window, cx| handler(window, cx));
                    } else {
                        window.defer(cx, ScreenFocus::release_text_field);
                    }
                }
            }),
            cx.on_blur(&handle, window, |this, _, _| this.text_view_did_end_editing()),
        ];
        let (changed, mut on_changed) = mpsc::unbounded::<()>();
        let first_frame_changed = changed.clone();
        let observers = vec![
            RiseAppEvents::default_center().observe(RISE_LOTTIE_FIRST_FRAME_CACHE_DID_CHANGE, move |_| {
                let _ = first_frame_changed.unbounded_send(());
            }),
            RiseAppEvents::default_center().observe(RISE_CHAT_COMPOSER_STATIC_EMOJI_CACHE_DID_CHANGE, move |_| {
                let _ = changed.unbounded_send(());
            }),
        ];
        let observer_task = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while on_changed.next().await.is_some() {
                let alive = this
                    .update(cx, |this, cx| {
                        if !this.custom_emoji_runs.is_empty() {
                            this.restyle(true, cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });
        let mut coordinator = Self {
            field,
            delegate_state,
            sizing: ChatComposerSizingTextView::default(),
            height_binding: view.measured_height.clone(),
            width: 0.0,
            min_height: 42.0,
            max_height: 120.0,
            last_measured_width: 0.0,
            last_focus_token: 0,
            last_selection_token: view.selection_token,
            editor_style: None,
            was_styled: false,
            last_text_color: None,
            custom_emoji_runs_handler: view.on_custom_emoji_runs_change.clone(),
            custom_emoji_runs: Vec::new(),
            holds_custom_emoji_line_height: view.holds_custom_emoji_line_height,
            attachment_items: HashMap::new(),
            requested_first_frames: HashSet::new(),
            _observers: observers,
            _observer_task: observer_task,
            _subscriptions: subscriptions,
        };
        coordinator.install_delegate(cx);
        coordinator.install_format_menu(cx);
        coordinator.sync_parent(view, cx);
        coordinator.mark_synced(cx);
        coordinator.apply_text_color_if_changed(view.text_color, cx);
        coordinator.restyle(true, cx);
        coordinator.apply_height(window, cx);
        coordinator.reclaim_carried_focus(view, window, cx);
        coordinator
    }

    fn reclaim_carried_focus(&mut self, view: &ChatComposerTextView, window: &mut Window, cx: &mut Context<Self>) {
        let window_id = window.window_handle().window_id();
        let carry = cx.default_global::<ChatComposerTextViewFocusCarry>();
        let Some((_, _, at)) = carry
            .last_focused
            .clone()
            .filter(|(carried_window, carried_id, _)| *carried_window == window_id && *carried_id == view.id)
        else {
            return;
        };
        carry.last_focused = None;
        let carried_selection = carry.last_selection.take();
        let age = RiseClock::monotonic().saturating_sub(at);
        let other_field_focused = ScreenFocus::text_field_is_focused(window, cx);
        let reclaim = age <= ChatComposerTextViewFocusCarry::WINDOW
            || (age <= ChatComposerTextViewFocusCarry::IDLE_WINDOW && !other_field_focused);
        tracing::warn!(target: "riseonly::X524", "composer remount age_ms={} other_field_focused={other_field_focused} reclaim={reclaim}", age.as_millis());
        if !reclaim {
            return;
        }
        if let Some((text, selection)) = carried_selection
            && self.field.read(cx).text() == text
        {
            self.apply_selection(selection, cx);
        }
        let handle = self.field.read(cx).focus_handle(cx);
        window.defer(cx, move |window, cx| handle.focus(window, cx));
    }

    fn record_focus(&self, view: &ChatComposerTextView, window: &Window, cx: &mut Context<Self>) {
        let (focused, text, selection) = {
            let field = self.field.read(cx);
            (field.focus_handle(cx).is_focused(window), field.text().to_owned(), field.selected_range())
        };
        let window_id = window.window_handle().window_id();
        let held_carry = self.delegate_state.carry_key.replace(focused.then(|| (window_id, view.id.clone()))).is_some();
        let carry = cx.default_global::<ChatComposerTextViewFocusCarry>();
        if focused {
            carry.last_focused = Some((window_id, view.id.clone(), RiseClock::monotonic()));
            carry.last_selection = Some((text, selection));
        } else if held_carry
            && carry.last_focused.as_ref().is_some_and(|(carried_window, id, _)| *carried_window == window_id && *id == view.id)
        {
            carry.last_focused = None;
            carry.last_selection = None;
        }
    }

    fn update_ui_view(&mut self, view: &ChatComposerTextView, window: &mut Window, cx: &mut Context<Self>) {
        let custom_emoji_runs_changed = self.sync_parent(view, cx);
        let text = view.text.get(cx);
        let text_changed = self.field.read(cx).text() != text;
        if text_changed {
            self.field.update(cx, |field, cx| field.set_text(&text, cx));
        }
        self.mark_synced(cx);
        let color_changed = self.apply_text_color_if_changed(view.text_color, cx);
        if text_changed || color_changed || custom_emoji_runs_changed {
            self.restyle(true, cx);
        }
        self.did_width_change(view.width);
        self.apply_height(window, cx);
        if text_changed {
            let coordinator = cx.weak_entity();
            window.defer(cx, move |window, cx| {
                if let Some(coordinator) = coordinator.upgrade() {
                    coordinator.update(cx, |coordinator, cx| coordinator.apply_height(window, cx));
                }
            });
        }
        if self.consume_focus_token(view.focus_token) {
            let handle = self.field.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| handle.focus(window, cx));
        }
        self.record_focus(view, window, cx);
        let typed_over_pending_text = self.delegate_state.typed_over_pending_text.replace(false);
        if self.consume_selection_token(view.selection_token)
            && !typed_over_pending_text
            && let Some(requested_selection) = view.requested_selection
        {
            self.apply_selection(requested_selection, cx);
        }
    }

    fn apply_text_color_if_changed(&mut self, color: Hsla, cx: &mut Context<Self>) -> bool {
        if self.last_text_color == Some(color) {
            return false;
        }
        self.last_text_color = Some(color);
        self.field.update(cx, |field, cx| field.set_text_color(Some(color), cx));
        true
    }

    fn consume_focus_token(&mut self, token: i64) -> bool {
        if token == self.last_focus_token {
            return false;
        }
        self.last_focus_token = token;
        token > 0
    }

    fn consume_selection_token(&mut self, token: i64) -> bool {
        if token == self.last_selection_token {
            return false;
        }
        self.last_selection_token = token;
        token > 0
    }

    fn apply_selection(&mut self, requested: NSRange, cx: &mut Context<Self>) {
        self.field.update(cx, |field, cx| {
            let length = length_utf16(field.text());
            let location = requested.location.min(length);
            let selection = NSRange::new(location, requested.length.min(length - location));
            field.set_selected_range(selection, cx);
        });
    }

    fn sync_parent(&mut self, view: &ChatComposerTextView, cx: &mut Context<Self>) -> bool {
        *self.delegate_state.text_binding.borrow_mut() = view.text.clone();
        self.height_binding = view.measured_height.clone();
        self.width = view.width;
        self.min_height = view.min_height;
        self.max_height = view.max_height;
        self.editor_style = Some(view.editor_style);
        self.holds_custom_emoji_line_height = view.holds_custom_emoji_line_height;
        *self.delegate_state.selection_handler.borrow_mut() = view.on_selection_change.clone();
        *self.delegate_state.cancel_handler.borrow_mut() = view.on_cancel.clone();
        *self.delegate_state.paste_handler.borrow_mut() = view.on_paste.clone();
        self.custom_emoji_runs_handler = view.on_custom_emoji_runs_change.clone();
        let custom_emoji_runs_changed = self.custom_emoji_runs != view.custom_emoji_runs;
        self.custom_emoji_runs = view.custom_emoji_runs.clone();
        if custom_emoji_runs_changed {
            let lottie_urls: Vec<String> = self
                .custom_emoji_runs
                .iter()
                .filter_map(|run| {
                    let source = RemoteMediaURL::optional_normalized(Some(run.item.file_url.as_str()))
                        .unwrap_or_else(|| run.item.file_url.clone());
                    if StickerAssetType::infer(Some(run.item.asset_type.wire()), &source) != StickerAssetType::Lottie {
                        return None;
                    }
                    Some(source)
                })
                .collect();
            if !lottie_urls.is_empty() {
                LottieStickerPrefetch::warm(&lottie_urls, cx);
            }
            let raster_urls: Vec<String> = self
                .custom_emoji_runs
                .iter()
                .flat_map(|run| {
                    let file_url = RemoteMediaURL::optional_normalized(Some(run.item.file_url.as_str()))
                        .unwrap_or_else(|| run.item.file_url.clone());
                    let asset_type = StickerAssetType::infer(Some(run.item.asset_type.wire()), &file_url);
                    let mut urls: Vec<String> = run.item.thumbnail_url.iter().cloned().collect();
                    if asset_type == StickerAssetType::StaticImage {
                        urls.push(file_url);
                    }
                    urls
                })
                .collect();
            if !raster_urls.is_empty() {
                ChatComposerStaticEmojiPreviewPrefetcher::warm(raster_urls, cx);
            }
        }
        custom_emoji_runs_changed
    }

    fn did_width_change(&mut self, new_width: f32) -> bool {
        if (new_width - self.last_measured_width).abs() <= 0.5 {
            return false;
        }
        self.last_measured_width = new_width;
        true
    }

    fn mark_synced(&mut self, cx: &mut Context<Self>) {
        let text = self.field.read(cx).text().to_owned();
        *self.delegate_state.last_synced_text.borrow_mut() = text.clone();
        *self.delegate_state.shown_text.borrow_mut() = text;
    }

    fn install_delegate(&mut self, cx: &mut Context<Self>) {
        let coordinator = cx.weak_entity();
        let should_change_state = self.delegate_state.clone();
        let did_change_state = self.delegate_state.clone();
        let selection_state = self.delegate_state.clone();
        let delegate = InputUiTextViewDelegate::default()
            .should_change_text_in(move |_range, replacement, cx| {
                Self::should_change_text_in(&should_change_state, &coordinator, replacement, cx)
            })
            .did_change(move |field, _| {
                *did_change_state.shown_text.borrow_mut() = field.text().to_owned();
                did_change_state.is_composing.set(field.is_composing());
                did_change_state.user_edit.set(true);
            })
            .did_change_selection(move |range, cx| Self::text_view_did_change_selection(&selection_state, range, cx));
        let paste_state = self.delegate_state.clone();
        self.field.update(cx, |field, _| {
            field.set_delegate(delegate);
            field.set_on_edit_menu_presented_change(Some(Rc::new(|is_visible: bool, _: &mut Window, _: &mut App| {
                EditMenuPresentation::set_is_visible(is_visible);
            })));
            field.set_on_paste(Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                let handler = paste_state.paste_handler.borrow().clone();
                handler.is_some_and(|handler| handler(window, cx))
            })));
        });
    }

    fn text_view_did_change_selection(state: &ChatComposerTextViewDelegateState, range: NSRange, cx: &mut App) {
        if let Some((window_id, id)) = state.carry_key.borrow().as_ref() {
            let carry = cx.default_global::<ChatComposerTextViewFocusCarry>();
            if carry.last_focused.as_ref().is_some_and(|(carried_window, carried_id, _)| carried_window == window_id && carried_id == id) {
                carry.last_selection = Some((state.shown_text.borrow().clone(), range));
            }
        }
        let handler = state.selection_handler.borrow().clone();
        handler(range, cx);
    }

    /// a send clears the binding but the view takes the empty text only on the next parent render
    /// keys typed before it edited the old text and wrote it back over the clear
    fn should_change_text_in(
        state: &ChatComposerTextViewDelegateState,
        coordinator: &WeakEntity<Self>,
        replacement: &str,
        cx: &mut App,
    ) -> bool {
        if state.applying_pending_text.get() || state.is_composing.get() {
            return true;
        }
        let pending = state.text_binding.borrow().get(cx);
        let shown = state.shown_text.borrow().clone();
        if pending == shown || shown != *state.last_synced_text.borrow() {
            return true;
        }
        let coordinator = coordinator.clone();
        let replacement = replacement.to_owned();
        cx.defer(move |cx| {
            if let Some(coordinator) = coordinator.upgrade() {
                coordinator.update(cx, |coordinator, cx| coordinator.apply_pending_text(pending, replacement, cx));
            }
        });
        false
    }

    fn apply_pending_text(&mut self, pending: String, replacement: String, cx: &mut Context<Self>) {
        self.delegate_state.applying_pending_text.set(true);
        self.field.update(cx, |field, cx| field.set_text(&pending, cx));
        self.mark_synced(cx);
        self.restyle(true, cx);
        self.field.update(cx, |field, cx| {
            let end = field.text().len();
            field.set_selection(Selection::cursor(end), cx);
        });
        if replacement.is_empty() {
            let selected = self.field.read(cx).selected_range();
            let handler = self.delegate_state.selection_handler.borrow().clone();
            handler(selected, cx);
            cx.notify();
        } else {
            self.field.update(cx, |field, cx| field.insert(&replacement, cx));
            *self.delegate_state.shown_text.borrow_mut() = self.field.read(cx).text().to_owned();
            self.delegate_state.user_edit.set(true);
            self.delegate_state.typed_over_pending_text.set(true);
        }
        self.delegate_state.applying_pending_text.set(false);
    }

    fn field_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        {
            let field = self.field.read(cx);
            *self.delegate_state.shown_text.borrow_mut() = field.text().to_owned();
            self.delegate_state.is_composing.set(field.is_composing());
        }
        if !self.delegate_state.user_edit.replace(false) {
            return;
        }
        self.text_view_did_change(window, cx);
    }

    fn text_view_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.field.read(cx).text().to_owned();
        let binding = self.delegate_state.text_binding.borrow().clone();
        binding.set(text, window, cx);
        self.mark_synced(cx);
        let current_runs = self.current_custom_emoji_runs(cx);
        self.custom_emoji_runs = current_runs.clone();
        let runs_handler = self.custom_emoji_runs_handler.clone();
        runs_handler(current_runs, cx);
        let selected = self.field.read(cx).selected_range();
        let selection_handler = self.delegate_state.selection_handler.borrow().clone();
        selection_handler(selected, cx);
        self.restyle(false, cx);
        self.apply_height(window, cx);
    }

    fn text_view_did_end_editing(&mut self) {
        EditMenuPresentation::set_is_visible(false);
    }

    fn restyle(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(style) = self.editor_style else {
            return;
        };
        let (raw, composing) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.is_composing())
        };
        let needs = RichTextMarkup::has_any_markup(&raw);
        if !(needs || self.was_styled || force) {
            return;
        }
        if composing {
            return;
        }
        let base = AttributeContainer {
            font: Some(style.font),
            foreground_color: Some(style.text_color),
            ..AttributeContainer::default()
        };
        let attributed = if needs {
            self.was_styled = true;
            RichTextMarkup::editor_attributed(&raw, &style)
        } else {
            self.was_styled = false;
            AttributedString::with_attributes(raw, base.clone())
        };
        let pieces: Vec<(String, RunAttributes)> = attributed
            .runs()
            .map(|(range, attributes)| {
                (attributed.characters()[range].to_owned(), RunAttributes::new(attributes.clone()))
            })
            .collect();
        let held_line_height = self.holds_custom_emoji_line_height.then(|| {
            let metrics = ChatComposerFontMetrics::system(style.font.size, style.font.weight, cx);
            px(Self::held_line_height(&metrics))
        });
        let pieces = self.apply_custom_emoji_attachments(pieces, &style.font, cx);
        self.field.update(cx, |field, cx| {
            field.set_minimum_line_height(held_line_height, cx);
            field.set_attributed_text(pieces, RunAttributes::new(base), cx)
        });
    }

    fn apply_custom_emoji_attachments(
        &mut self,
        pieces: Vec<(String, RunAttributes)>,
        font: &RichFont,
        cx: &mut Context<Self>,
    ) -> Vec<(String, RunAttributes)> {
        self.attachment_items.clear();
        if self.custom_emoji_runs.is_empty() {
            return pieces;
        }
        let source: String = pieces.iter().map(|(piece, _)| piece.as_str()).collect();
        let source_length = length_utf16(&source);
        let metrics = ChatComposerFontMetrics::system(font.size, font.weight, cx);
        let ink_height = Self::composer_ink_height(&metrics);
        let holds_line_height = self.holds_custom_emoji_line_height;

        let mut cuts: Vec<(Range<usize>, RunAttributes)> = Vec::new();
        let mut missing_first_frames: Vec<String> = Vec::new();
        for run in self.custom_emoji_runs.iter().rev() {
            let range = run.range;
            if range.length != 1
                || range.location + range.length > source_length
                || ns_range::substring(&source, range) != Some("\u{FFFC}")
            {
                continue;
            }
            let Some(bytes) = utf16_to_byte_range(&source, range) else {
                continue;
            };
            let ink = CustomEmojiGlyphMetrics::ink_size_capped(
                ink_height,
                CustomEmojiGlyphMetrics::aspect(
                    run.item.width,
                    run.item.height,
                    run.item.asset_type == StickerAssetType::Lottie,
                ),
                CustomEmojiGlyphMetrics::TEXT_MAXIMUM_ASPECT,
                None,
            );
            let image = Self::custom_emoji_image(&run.item);
            if let Some(source) = Self::missing_lottie_first_frame(&run.item) {
                missing_first_frames.push(source);
            }
            let attachment = TextAttachment::new(
                Some(image),
                Bounds::new(
                    point(
                        px(0.0),
                        if holds_line_height {
                            px(Self::held_glyph_origin_y(&metrics, f32::from(ink.height)))
                        } else {
                            (px(metrics.cap_height) - ink.height) / 2.0
                        },
                    ),
                    ink,
                ),
            );
            self.attachment_items.insert(attachment.id(), run.item.clone());
            let replacement = match ChatComposerCustomEmojiAttachmentPayload::new(&run.item).encoded_string() {
                Some(metadata) => RunAttributes::tagged(AttributeContainer::default(), ChatCustomEmojiMetadata(metadata)),
                None => RunAttributes::new(AttributeContainer::default()),
            };
            cuts.push((bytes, replacement.with_attachment(attachment)));
        }
        cuts.sort_by_key(|(range, _)| range.start);
        self.request_missing_first_frames(missing_first_frames, cx);
        Self::splice_attachments(pieces, cuts)
    }

    fn missing_lottie_first_frame(item: &StickerItem) -> Option<String> {
        let raw_file_url = trimming_whitespaces_and_newlines(&item.file_url).to_owned();
        let file_url = RemoteMediaURL::optional_normalized(Some(raw_file_url.as_str())).unwrap_or_else(|| raw_file_url.clone());
        if file_url.is_empty() || StickerAssetType::infer(Some(item.asset_type.wire()), &file_url) != StickerAssetType::Lottie {
            return None;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let cached = [file_url.as_str(), raw_file_url.as_str()].into_iter().any(|source| {
            RiseLottieFirstFrameCache::memory_image(source, LottieStickerPrefetch::FIRST_FRAME_POINT_SIZE, account_user_id.as_deref())
                .or_else(|| RiseLottieFirstFrameCache::latest_memory_image(source, account_user_id.as_deref()))
                .is_some()
        });
        (!cached).then_some(file_url)
    }

    fn request_missing_first_frames(&mut self, sources: Vec<String>, cx: &mut Context<Self>) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        for source in sources {
            if !self.requested_first_frames.insert(source.clone()) {
                continue;
            }
            let account_user_id = account_user_id.clone();
            let account_epoch = account_epoch.clone();
            cx.spawn(async move |this: WeakEntity<Self>, cx| {
                if let Ok(LottieStickerPayload::Animation(render_source)) = LottieStickerLoader::shared().payload(&source).await {
                    RiseLottieFirstFrameCache::prepare(
                        render_source,
                        LottieStickerPrefetch::FIRST_FRAME_POINT_SIZE,
                        account_user_id,
                        account_epoch,
                        None,
                    )
                    .await;
                }
                let _ = this.update(cx, |coordinator, _| {
                    coordinator.requested_first_frames.remove(&source);
                });
            })
            .detach();
        }
    }

    fn splice_attachments(
        pieces: Vec<(String, RunAttributes)>,
        cuts: Vec<(Range<usize>, RunAttributes)>,
    ) -> Vec<(String, RunAttributes)> {
        let mut result = Vec::with_capacity(pieces.len() + cuts.len() * 2);
        let mut cuts = cuts.into_iter().peekable();
        let mut offset = 0;
        for (piece, attributes) in pieces {
            let start = offset;
            let end = start + piece.len();
            offset = end;
            let mut cursor = start;
            while let Some((range, replacement)) = cuts.next_if(|(range, _)| range.start < end) {
                if range.start < cursor || range.end > end {
                    continue;
                }
                if let Some(before) = piece.get(cursor - start..range.start - start)
                    && !before.is_empty()
                {
                    result.push((before.to_owned(), attributes.clone()));
                }
                if let Some(placeholder) = piece.get(range.start - start..range.end - start) {
                    result.push((placeholder.to_owned(), replacement));
                }
                cursor = range.end;
            }
            if let Some(after) = piece.get(cursor - start..)
                && !after.is_empty()
            {
                result.push((after.to_owned(), attributes));
            }
        }
        result
    }

    fn current_custom_emoji_runs(&self, cx: &App) -> Vec<ChatComposerCustomEmojiRun> {
        let field = self.field.read(cx);
        let text = field.text();
        if text.is_empty() {
            return Vec::new();
        }
        let mut runs = Vec::new();
        let mut offset = 0;
        for (piece, attributes) in field.attributed_pieces() {
            let start = offset;
            offset += piece.len();
            let Some(attachment) = attributes.attachment.as_ref() else {
                continue;
            };
            let item = attributes
                .tag::<ChatCustomEmojiMetadata>()
                .and_then(|metadata| ChatComposerCustomEmojiAttachmentPayload::decode(Some(metadata.0.as_str())))
                .map(|payload| payload.item())
                .or_else(|| self.attachment_items.get(&attachment.id()).cloned());
            let Some(item) = item else {
                continue;
            };
            runs.push(ChatComposerCustomEmojiRun {
                range: byte_to_utf16_range(text, start..offset),
                item,
            });
        }
        runs.sort_by_key(|run| run.range.location);
        runs
    }

    fn custom_emoji_image(item: &StickerItem) -> CleverImage {
        let raw_file_url = trimming_whitespaces_and_newlines(&item.file_url).to_owned();
        let file_url = RemoteMediaURL::optional_normalized(Some(raw_file_url.as_str())).unwrap_or_else(|| raw_file_url.clone());
        let asset_type = StickerAssetType::infer(Some(item.asset_type.wire()), &file_url);
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();

        if asset_type == StickerAssetType::Lottie {
            let mut seen = HashSet::new();
            let sources: Vec<&str> = [file_url.as_str(), raw_file_url.as_str()]
                .into_iter()
                .filter(|source| !source.is_empty() && seen.insert(*source))
                .collect();
            for source in sources {
                let frame = RiseLottieFirstFrameCache::memory_image(
                    source,
                    LottieStickerPrefetch::FIRST_FRAME_POINT_SIZE,
                    account_user_id.as_deref(),
                )
                .or_else(|| RiseLottieFirstFrameCache::latest_memory_image(source, account_user_id.as_deref()));
                if let Some(frame) = frame {
                    return frame;
                }
            }
        }

        let candidates: Vec<Option<String>> = if asset_type == StickerAssetType::StaticImage {
            vec![item.thumbnail_url.clone(), Some(raw_file_url)]
        } else {
            vec![item.thumbnail_url.clone()]
        };
        for candidate in candidates.into_iter().flatten() {
            if let Some(image) = ChatComposerCustomEmojiPreviewCache::raster_image(
                &candidate,
                account_user_id.as_deref(),
                account_epoch.as_deref(),
            ) {
                return image;
            }
        }

        ChatComposerCustomEmojiPlaceholder::transparent_image()
    }

    fn install_format_menu(&mut self, cx: &mut Context<Self>) {
        let coordinator = cx.weak_entity();
        let items: Vec<ContextMenuItem> = RichTextMarkupFormat::ALL_CASES
            .iter()
            .enumerate()
            .map(|(index, format)| {
                let format = *format;
                let coordinator = coordinator.clone();
                ContextMenuItem::new(
                    index as i64,
                    tr(format.title_key()),
                    Some(format.sf_symbol().to_owned()),
                    format.title_key(),
                    move |window, cx| {
                        if let Some(coordinator) = coordinator.upgrade() {
                            coordinator.update(cx, |coordinator, cx| coordinator.apply_format(format, window, cx));
                        }
                    },
                )
            })
            .collect();
        let submenu = InputEditSubmenu {
            title: tr("format_title"),
            icon: Some("textformat".to_owned()),
            items,
        };
        self.field
            .update(cx, |field, cx| field.set_edit_submenu(Some(submenu), cx));
    }

    fn apply_format(&mut self, format: RichTextMarkupFormat, window: &mut Window, cx: &mut Context<Self>) {
        let (raw, selection) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.selected_range())
        };
        if selection.length == 0 {
            return;
        }
        let (text, result_selection) = RichTextMarkup::apply_format(format, &raw, selection);
        self.field.update(cx, |field, cx| field.set_text(&text, cx));
        let binding = self.delegate_state.text_binding.borrow().clone();
        binding.set(text.clone(), window, cx);
        self.mark_synced(cx);
        self.restyle(true, cx);
        let length = length_utf16(&text);
        let loc = result_selection.location.min(length);
        let clamped = NSRange::new(loc, result_selection.length.min(length - loc));
        self.field.update(cx, |field, cx| field.set_selected_range(clamped, cx));
        self.apply_height(window, cx);
    }

    pub fn composer_ink_height(font: &ChatComposerFontMetrics) -> f32 {
        18.0_f32.max(CustomEmojiGlyphMetrics::ink_height(font.point_size))
    }

    pub fn held_glyph_origin_y(font: &ChatComposerFontMetrics, ink_height: f32) -> f32 {
        ((font.cap_height - ink_height) / 2.0).max(font.descender)
    }

    pub fn held_line_height(font: &ChatComposerFontMetrics) -> f32 {
        let ink = Self::composer_ink_height(font);
        let top = Self::held_glyph_origin_y(font, ink) + ink;
        font.line_height.max(font.ascender.max(top) - font.descender).ceil()
    }

    fn apply_height(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.width <= 0.0 {
            return;
        }
        let (text, line_height) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.line_height())
        };
        if line_height <= px(0.0) {
            return;
        }
        let font_size = self
            .editor_style
            .map(|style| style.font.size)
            .unwrap_or_else(MessageSizes::composer_text_font_size);
        let mut layout = UITextViewContentLayout::new(
            text,
            theme(cx).typography.font(FontWeight::NORMAL),
            px(font_size),
            line_height,
        );
        layout.text_container_inset = ChatComposerTextView::text_container_inset();
        layout.line_fragment_padding = px(0.0);
        layout.width_tracks_text_view = false;
        let fitted = f32::from(layout.content_height(px(self.width), window));
        let h = fitted.ceil().max(self.min_height).min(self.max_height);
        let scrolls = fitted > self.max_height + 0.5;
        if self.field.read(cx).scroll_enabled() != Some(scrolls) {
            self.field.update(cx, |field, cx| field.set_scroll_enabled(scrolls, cx));
        }
        if self.sizing.set_layout_content_height(h) {
            cx.notify();
        }
        if (self.height_binding.get(cx) - h).abs() > 0.5 {
            self.height_binding.set(h, window, cx);
        }
    }
}

impl RenderOnce for ChatComposerTextView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            self.make_coordinator(window, cx)
        });
        coordinator.update(cx, |coordinator, cx| coordinator.update_ui_view(&self, window, cx));
        let (field, height) = {
            let state = coordinator.read(cx);
            (state.field.clone(), state.sizing.intrinsic_content_size())
        };
        let inset = ChatComposerTextView::text_container_inset();
        let bold = coordinator.downgrade();
        let italic = coordinator.downgrade();
        let strikethrough = coordinator.downgrade();
        let send = self.on_send.clone();
        let line_break_field = field.downgrade();
        div()
            .key_context(KEY_CONTEXT)
            .capture_action(move |_: &Confirm, window, cx| {
                if let Some(send) = &send {
                    send(window, cx);
                    cx.stop_propagation();
                }
            })
            .on_action(move |_: &InsertLineBreak, window, cx| {
                if let Some(field) = line_break_field.upgrade() {
                    field.update(cx, |field, cx| field.confirm(&Confirm, window, cx));
                }
            })
            .on_action(move |_: &FormatBold, window, cx| {
                if let Some(coordinator) = bold.upgrade() {
                    coordinator.update(cx, |coordinator, cx| {
                        coordinator.apply_format(RichTextMarkupFormat::Bold, window, cx)
                    });
                }
            })
            .on_action(move |_: &FormatItalic, window, cx| {
                if let Some(coordinator) = italic.upgrade() {
                    coordinator.update(cx, |coordinator, cx| {
                        coordinator.apply_format(RichTextMarkupFormat::Italic, window, cx)
                    });
                }
            })
            .on_action(move |_: &FormatStrikethrough, window, cx| {
                if let Some(coordinator) = strikethrough.upgrade() {
                    coordinator.update(cx, |coordinator, cx| {
                        coordinator.apply_format(RichTextMarkupFormat::Strikethrough, window, cx)
                    });
                }
            })
            .w_full()
            .h(px(height))
            .pt(inset.top)
            .pl(inset.left)
            .pb(inset.bottom)
            .pr(inset.right)
            .child(field.cached(StyleRefinement::default().size_full()))
    }
}

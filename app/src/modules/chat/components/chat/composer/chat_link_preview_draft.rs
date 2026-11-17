use std::sync::LazyLock;

use gpui::{Context, Task};
use rise_chat::LinkPreviewSize as ChatLinkPreviewSize;
use rise_core::foundation::foundation_data_detector::{NSDataDetector, NSTextCheckingType};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_regex::NSMatchingOptions;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::{self, NSRange};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::link_preview_card_ui::LinkPreviewCardPresentation;
use crate::modules::chat::components::chat::composer::chat_link_preview_draft_hooks::chat_link_preview_draft_hooks;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageLinkPreview, MessageLinkPreviewSize,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageLinkMetadataWireDTO;

#[derive(Clone, Debug)]
pub struct ChatLinkPreviewSelection {
    pub wire_type: String,
    pub metadata: Option<MessageLinkMetadataWireDTO>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLinkPreviewPreference {
    pub url: Option<String>,
    pub is_suppressed: bool,
    pub is_above_text: bool,
    pub is_large: bool,
}

pub struct ChatLinkPreviewDraft {
    url: Option<String>,
    metadata: Option<MessageLinkMetadataWireDTO>,
    is_loading: bool,
    failed: bool,
    is_suppressed: bool,
    is_blocked_by_chat: bool,
    is_above_text: bool,
    is_large: bool,
    settings_presented: bool,
    settings_opened_at: TimeInterval,
    request: Option<Task<()>>,
    generation: Uuid,
    previous_text: String,
    suppressed_range: Option<NSRange>,
    edit_baseline: Option<ChatLinkPreviewPreference>,
}

impl Default for ChatLinkPreviewDraft {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatLinkPreviewDraft {
    pub fn new() -> Self {
        Self {
            url: None,
            metadata: None,
            is_loading: false,
            failed: false,
            is_suppressed: false,
            is_blocked_by_chat: false,
            is_above_text: false,
            is_large: true,
            settings_presented: false,
            settings_opened_at: Date::now().time_interval_since_1970(),
            request: None,
            generation: Uuid::new_v4(),
            previous_text: String::new(),
            suppressed_range: None,
            edit_baseline: None,
        }
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub fn metadata(&self) -> Option<&MessageLinkMetadataWireDTO> {
        self.metadata.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn is_suppressed(&self) -> bool {
        self.is_suppressed
    }

    pub fn is_blocked_by_chat(&self) -> bool {
        self.is_blocked_by_chat
    }

    pub fn is_above_text(&self) -> bool {
        self.is_above_text
    }

    pub fn set_is_above_text(&mut self, is_above_text: bool, cx: &mut Context<Self>) {
        self.is_above_text = is_above_text;
        cx.notify();
    }

    pub fn is_large(&self) -> bool {
        self.is_large
    }

    pub fn set_is_large(&mut self, is_large: bool, cx: &mut Context<Self>) {
        self.is_large = is_large;
        cx.notify();
    }

    pub fn settings_presented(&self) -> bool {
        self.settings_presented
    }

    pub fn set_settings_presented(&mut self, settings_presented: bool, cx: &mut Context<Self>) {
        self.settings_presented = settings_presented;
        cx.notify();
    }

    pub fn settings_opened_at(&self) -> TimeInterval {
        self.settings_opened_at
    }

    pub fn is_visible(&self) -> bool {
        self.url.is_some() && !self.is_suppressed && !self.is_blocked_by_chat
    }

    pub fn preview(&self) -> Option<MessageLinkPreview> {
        let url = self.url.as_ref()?;
        let metadata = self.metadata.as_ref()?;
        if self.is_suppressed || self.is_blocked_by_chat {
            return None;
        }
        Some(
            MessageLinkPreview {
                url: url.clone(),
                title: metadata.title.clone(),
                description: metadata.description.clone(),
                image_url: metadata.image_url.clone(),
                size: if self.is_large { MessageLinkPreviewSize::Big } else { MessageLinkPreviewSize::Small },
                is_above_text: self.is_above_text,
                ..MessageLinkPreview::default()
            }
            .localized_for_own_link(),
        )
    }

    pub fn preference(&self) -> ChatLinkPreviewPreference {
        ChatLinkPreviewPreference {
            url: self.url.clone(),
            is_suppressed: self.is_suppressed,
            is_above_text: self.is_above_text,
            is_large: self.is_large,
        }
    }

    pub fn restore(&mut self, preference: Option<ChatLinkPreviewPreference>, text: &str, cx: &mut Context<Self>) {
        self.reset(cx);
        let first = ChatLinkPreviewURL::first(text);
        let restored = preference.filter(|preference| preference.url == first);
        if let Some(restored) = &restored {
            self.is_suppressed = restored.is_suppressed;
            self.is_above_text = restored.is_above_text;
            self.previous_text = text.to_owned();
            self.suppressed_range = if self.is_suppressed {
                ChatLinkPreviewURL::first_match(text, None).map(|(_, range)| range)
            } else {
                None
            };
        }
        self.update(text, cx);
        if let Some(restored) = restored {
            self.is_large = restored.is_large;
        }
        cx.notify();
    }

    pub fn update(&mut self, text: &str, cx: &mut Context<Self>) {
        let next = ChatLinkPreviewURL::first_match(text, None).map(|(url, _)| url);
        let was_suppressed = self.is_suppressed;
        if self.is_suppressed
            && let Some(range) = self.suppressed_range
        {
            let rebased = ChatLinkPreviewURL::rebased(range, &self.previous_text, text);
            if let Some((_, suppressed)) = ChatLinkPreviewURL::first_match(text, Some(rebased)) {
                self.suppressed_range = Some(suppressed);
            } else {
                self.is_suppressed = false;
                self.suppressed_range = None;
            }
        }
        self.previous_text = text.to_owned();
        let url_changed = next != self.url;
        if !(url_changed || was_suppressed != self.is_suppressed) {
            return;
        }
        self.request = None;
        self.generation = Uuid::new_v4();
        self.metadata = None;
        self.failed = false;
        self.is_loading = false;
        self.url = next.clone();
        self.settings_presented = false;
        let Some(next) = next else {
            self.is_suppressed = false;
            self.is_above_text = false;
            self.is_large = true;
            cx.notify();
            return;
        };
        if url_changed {
            self.is_large = !LinkPreviewCardPresentation::is_avatar_link_url(&next);
        }
        cx.notify();
        if self.is_suppressed {
            return;
        }
        self.load(cx);
    }

    pub fn set_blocked_by_chat(&mut self, blocked: bool, cx: &mut Context<Self>) {
        if blocked == self.is_blocked_by_chat {
            return;
        }
        self.is_blocked_by_chat = blocked;
        if blocked {
            self.request = None;
            self.generation = Uuid::new_v4();
            self.metadata = None;
            self.is_loading = false;
            self.failed = false;
            self.settings_presented = false;
            cx.notify();
        } else {
            cx.notify();
            self.load(cx);
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.url.clone() else {
            return;
        };
        if self.is_suppressed || self.is_blocked_by_chat {
            return;
        }
        self.request = None;
        self.generation = Uuid::new_v4();
        let expected = self.generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        self.is_loading = true;
        self.failed = false;
        cx.notify();
        let pending = chat_link_preview_draft_hooks().map(|hooks| (hooks.load_link_preview)(url, cx));
        self.request = Some(cx.spawn(async move |this, cx| {
            let outcome = match pending {
                Some(pending) => pending.await,
                None => Err(anyhow::anyhow!("chat link preview draft hooks are not installed")),
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation != expected
                    || SaiAccountCacheScope::active_user_id() != account
                    || SaiAccountCacheScope::active_epoch() != epoch
                {
                    return;
                }
                match outcome {
                    Ok(value) => {
                        this.metadata = Some(value);
                        this.is_loading = false;
                    }
                    Err(_) => {
                        this.is_loading = false;
                        this.failed = true;
                    }
                }
                cx.notify();
            });
        }));
    }

    pub fn present_settings(&mut self, cx: &mut Context<Self>) {
        if self.preview().is_none() {
            return;
        }
        self.settings_opened_at = Date::now().time_interval_since_1970();
        self.settings_presented = true;
        cx.notify();
    }

    pub fn suppress(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        self.generation = Uuid::new_v4();
        self.is_suppressed = true;
        self.suppressed_range = ChatLinkPreviewURL::first_match(&self.previous_text, None).map(|(_, range)| range);
        self.is_loading = false;
        self.metadata = None;
        self.settings_presented = false;
        cx.notify();
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        self.generation = Uuid::new_v4();
        self.url = None;
        self.previous_text = String::new();
        self.suppressed_range = None;
        self.metadata = None;
        self.is_loading = false;
        self.failed = false;
        self.is_suppressed = false;
        self.is_above_text = false;
        self.is_large = true;
        self.settings_presented = false;
        self.edit_baseline = None;
        cx.notify();
    }

    pub fn seed_for_edit(&mut self, text: &str, preview: Option<&MessageLinkPreview>, cx: &mut Context<Self>) {
        self.reset(cx);
        self.previous_text = text.to_owned();
        let Some((url, range)) = ChatLinkPreviewURL::first_match(text, None) else {
            return;
        };
        self.url = Some(url);
        if let Some(preview) = preview {
            self.metadata = Some(MessageLinkMetadataWireDTO {
                title: preview.title.clone(),
                description: preview.description.clone(),
                image_url: preview.image_url.clone(),
                link_preview_type: None,
            });
            self.is_above_text = preview.is_above_text;
            self.is_large = preview.size == MessageLinkPreviewSize::Big;
        } else {
            self.is_suppressed = true;
            self.suppressed_range = Some(range);
        }
        self.edit_baseline = Some(self.preference());
        cx.notify();
    }

    pub fn edit_wire_type(&mut self, text: &str, cx: &mut Context<Self>) -> Option<String> {
        let baseline = self.edit_baseline.clone();
        let selection = self.selection(text, cx)?;
        if baseline.is_some_and(|baseline| baseline == self.preference()) {
            return None;
        }
        Some(selection.wire_type)
    }

    pub fn selection(&mut self, text: &str, cx: &mut Context<Self>) -> Option<ChatLinkPreviewSelection> {
        self.update(text, cx);
        let first = ChatLinkPreviewURL::first(text)?;
        let is_current = self.url.as_deref() == Some(first.as_str());
        let failed_here = is_current && self.failed && self.metadata.is_none();
        let mode = if self.is_suppressed || self.is_blocked_by_chat || failed_here {
            ChatLinkPreviewSize::None
        } else if self.is_above_text {
            if self.is_large { ChatLinkPreviewSize::BigAbove } else { ChatLinkPreviewSize::SmallAbove }
        } else if self.is_large {
            ChatLinkPreviewSize::Big
        } else {
            ChatLinkPreviewSize::Small
        };
        let source = if is_current && !self.is_suppressed && !self.is_blocked_by_chat {
            self.metadata.as_ref()
        } else {
            None
        };
        Some(ChatLinkPreviewSelection {
            wire_type: mode.wire().to_owned(),
            metadata: Some(MessageLinkMetadataWireDTO {
                title: source.and_then(|source| source.title.clone()),
                description: source.and_then(|source| source.description.clone()),
                image_url: source.and_then(|source| source.image_url.clone()),
                link_preview_type: Some(mode.wire().to_owned()),
            }),
        })
    }
}

static CHAT_LINK_PREVIEW_URL_DETECTOR: LazyLock<Option<NSDataDetector>> =
    LazyLock::new(|| NSDataDetector::new(NSTextCheckingType::LINK).ok());

pub enum ChatLinkPreviewURL {}

impl ChatLinkPreviewURL {
    pub fn first(text: &str) -> Option<String> {
        Self::first_match(text, None).map(|(url, _)| url)
    }

    /// first match in the text, overlapping defaults to none
    pub fn first_match(text: &str, overlapping: Option<NSRange>) -> Option<(String, NSRange)> {
        let detector = CHAT_LINK_PREVIEW_URL_DETECTOR.as_ref()?;
        let range = NSRange::full(text);
        for found in detector.matches(text, NSMatchingOptions::NONE, range) {
            if let Some(tracked_range) = overlapping
                && tracked_range.intersection(found.range).is_none()
            {
                continue;
            }
            let Some(matched) = ns_range::substring(text, found.range) else {
                continue;
            };
            let Some(detected) = found.url.as_ref() else {
                continue;
            };
            let scheme = detected.scheme().map(str::to_lowercase).unwrap_or_default();
            if !(scheme == "http" || scheme == "https") {
                continue;
            }
            let lowered = matched.to_lowercase();
            let has_scheme = lowered.starts_with("http://") || lowered.starts_with("https://");
            if !has_scheme {
                if lowered.contains("://") || matched.contains('@') {
                    continue;
                }
                if ns_range::substring(text, NSRange::new(0, found.range.location)).is_some_and(|before| before.ends_with('@')) {
                    continue;
                }
            }
            let bare_url;
            let url = if has_scheme {
                detected
            } else {
                let Some(parsed) = Url::from_string(&format!("https://{matched}")) else {
                    continue;
                };
                bare_url = parsed;
                &bare_url
            };
            let Some(host) = url.host() else {
                continue;
            };
            if host.ends_with('.') {
                continue;
            }
            let labels: Vec<&str> = host.split('.').collect();
            let Some(suffix) = labels.last() else {
                continue;
            };
            if labels.len() <= 1
                || labels.iter().any(|label| label.is_empty())
                || suffix.chars().count() < 2
                || !suffix.chars().any(char::is_alphabetic)
            {
                continue;
            }
            return Some((url.absolute_string().to_owned(), found.range));
        }
        None
    }

    pub fn rebased(range: NSRange, old_text: &str, new_text: &str) -> NSRange {
        let old: Vec<u16> = old_text.encode_utf16().collect();
        let new: Vec<u16> = new_text.encode_utf16().collect();
        let shared = old.len().min(new.len());
        let mut prefix = 0;
        while prefix < shared && old[prefix] == new[prefix] {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < shared - prefix && old[old.len() - suffix - 1] == new[new.len() - suffix - 1] {
            suffix += 1;
        }
        let change_end = (old.len() - suffix) as i64;
        let delta = new.len() as i64 - old.len() as i64;
        let location = range.location as i64;
        let max_range = range.upper_bound() as i64;
        let prefix = prefix as i64;
        let (start, end) = if change_end <= location {
            (location + delta, max_range + delta)
        } else if prefix >= max_range {
            (location, max_range)
        } else {
            let start = location.min(prefix);
            (start, start.max(max_range + delta))
        };
        let count = new.len() as i64;
        let clamped_start = count.min(start.max(0));
        NSRange::new(clamped_start as usize, (count.min(end) - clamped_start).max(0) as usize)
    }
}

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, Hsla, SharedString, Window, rgb, white};
use rise_core::foundation::foundation_crypto::hex_string;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{
    character_count, character_prefix, is_newline, trimming_whitespaces_and_newlines,
};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::user_defaults::{DefaultsValue, UserDefaults};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::text_selection::selectable_text;
use rise_ui::swiftui::transition::{Presence, TransitionModifiers};
use serde_json::{Map, Number, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;

const BLOCK_BACKGROUND: u32 = 0x1a3a52;
const LINK_BLUE: u32 = 0x29b6f6;
const STORE_BADGE: u32 = 0x1565c0;
const TYPE_BADGE: u32 = 0x6a1b9a;
const WS_CACHE_BADGE: u32 = 0x2e7d32;
const WS_PAYLOAD_BACKGROUND: u32 = 0x1b5e20;
const DESTRUCTIVE_RED: u32 = 0xe11d48;

pub enum SaiDebugInspectorMetrics {}

impl SaiDebugInspectorMetrics {
    pub const JSON_BODY_SCROLL_MAX_HEIGHT: f32 = 300.0;
    pub const COLLAPSED_PREVIEW_MAX_CHARS: i64 = 140;
}

pub enum SaiDebugBodyFormat {}

impl SaiDebugBodyFormat {
    /// size label from a char count, debug text in english with one decimal
    pub fn size_label_char_count(char_count: i64) -> String {
        if char_count <= 0 {
            return "empty".to_owned();
        }
        if char_count < 1024 {
            return format!("{char_count} chars");
        }
        Self::kilo_or_mega(char_count)
    }

    pub fn size_label_byte_count(byte_count: i64) -> String {
        if byte_count <= 0 {
            return "empty".to_owned();
        }
        if byte_count < 1024 {
            return format!("{byte_count} B");
        }
        Self::kilo_or_mega(byte_count)
    }

    fn kilo_or_mega(count: i64) -> String {
        let kb = count as f64 / 1024.0;
        if kb < 1024.0 {
            return format!("{kb:.1} KB");
        }
        format!("{:.1} MB", kb / 1024.0)
    }

    /// one line preview of text cut to max_chars, pass the collapsed preview constant for the default
    pub fn one_line_preview(text: &str, max_chars: i64) -> String {
        let t = trimming_whitespaces_and_newlines(text)
            .split(is_newline)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if t.is_empty() {
            return "(empty)".to_owned();
        }
        let limit = usize::try_from(max_chars.max(0)).unwrap_or(0);
        if character_count(&t) <= limit {
            return t;
        }
        format!("{}\u{2026}", character_prefix(&t, limit))
    }
}

pub(crate) fn line_limited(mut text: Div, lines: usize) -> Div {
    text.text_style().line_clamp = Some(lines);
    text
}

pub type SaiDebugLoadFullText = Rc<dyn Fn(&mut App) -> String>;

#[derive(Clone, Debug)]
pub struct SaiDebugExpandableMonospaceBlockState {
    pub expanded: bool,
    pub loaded_full_text: Option<String>,
    full_presence: Presence<String>,
    summary_presence: Presence<()>,
}

impl Default for SaiDebugExpandableMonospaceBlockState {
    fn default() -> Self {
        Self {
            expanded: false,
            loaded_full_text: None,
            full_presence: Presence::new(None),
            summary_presence: Presence::new(Some(())),
        }
    }
}

impl SaiDebugExpandableMonospaceBlockState {
    fn apply_expanded_branch(&mut self, cx: &App) {
        let now = RiseClock::monotonic();
        let full = if self.expanded { self.loaded_full_text.clone() } else { None };
        let summary = full.is_none().then_some(());
        self.full_presence.set_in(full, ease_in_out(0.18), now, cx);
        self.summary_presence.set_in(summary, ease_in_out(0.18), now, cx);
    }

    pub fn resolved_full_text_for_copy(&self, load_full_text: &SaiDebugLoadFullText, cx: &mut App) -> String {
        if let Some(loaded_full_text) = &self.loaded_full_text {
            return loaded_full_text.clone();
        }
        load_full_text(cx)
    }
}

#[derive(IntoElement)]
pub struct SaiDebugExpandableMonospaceBlock {
    id: ElementId,
    title: SharedString,
    summary: SharedString,
    theme: AppTheme,
    background: Hsla,
    load_full_text: SaiDebugLoadFullText,
}

impl SaiDebugExpandableMonospaceBlock {
    /// id keeps the expanded state across frames, it comes from the entry and not a position
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        summary: impl Into<SharedString>,
        theme: AppTheme,
        load_full_text: SaiDebugLoadFullText,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            summary: summary.into(),
            theme,
            background: Hsla::from(rgb(BLOCK_BACKGROUND)).opacity(0.18),
            load_full_text,
        }
    }

    pub fn background(mut self, background: Hsla) -> Self {
        self.background = background;
        self
    }
}

impl RenderOnce for SaiDebugExpandableMonospaceBlock {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            SaiDebugExpandableMonospaceBlockState::default()
        });
        let current = state.read(cx).clone();
        let load_full_text = self.load_full_text.clone();
        let toggle_state = state.clone();

        let toggle = plain(ElementId::NamedChild(Arc::new(self.id.clone()), "toggle".into()))
            .on_press(move |_window, cx| {
                let needs_load = {
                    let state = toggle_state.read(cx);
                    !state.expanded && state.loaded_full_text.is_none()
                };
                let loaded = needs_load.then(|| load_full_text(cx));
                toggle_state.update(cx, |state, cx| {
                    if state.expanded {
                        state.expanded = false;
                    } else {
                        if let Some(loaded) = loaded {
                            state.loaded_full_text = Some(loaded);
                        }
                        state.expanded = true;
                    }
                    state.apply_expanded_branch(cx);
                    cx.notify();
                });
            })
            .child(
                div()
                    .child(if current.expanded { "Hide" } else { "Show" })
                    .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                    .text_color(rgb(LINK_BLUE)),
            );

        let now = RiseClock::monotonic();
        let block_id = self.id.clone();
        let body = if current.full_presence.is_present() {
            current.full_presence.render_transitioned(now, window, |full| {
                div().w_full().child(
                    div()
                        .id(ElementId::NamedChild(Arc::new(block_id.clone()), "scroll".into()))
                        .w_full()
                        .max_h(px(SaiDebugInspectorMetrics::JSON_BODY_SCROLL_MAX_HEIGHT))
                        .overflow_y_scroll()
                        .child(
                            div()
                                .w_full()
                                .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                                .text_color(theme.text.primary)
                                .child(selectable_text(
                                    ElementId::NamedChild(Arc::new(block_id.clone()), "full".into()),
                                    full.clone(),
                                )),
                        ),
                )
            })
        } else {
            let summary = self.summary;
            current.summary_presence.render_transitioned(now, window, |_| {
                line_limited(
                    div()
                        .w_full()
                        .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                        .text_color(theme.text.secondary.opacity(0.9))
                        .child(summary),
                    3,
                )
            })
        };

        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .child(
                hstack(8.)
                    .align(VerticalAlignment::FirstTextBaseline)
                    .w_full()
                    .child(
                        div()
                            .child(self.title)
                            .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Default))
                            .text_color(theme.text.secondary.opacity(0.85))
                            .tracking(0.5),
                    )
                    .child(spacer_min_w(0.))
                    .child(toggle),
            )
            .children(body)
            .p(px(10.))
            .w_full()
            .bg(self.background)
            .continuous_corners(10.)
            .flex_shrink_0()
    }
}

#[derive(IntoElement)]
pub struct SaiDebugJsonScrollBlock {
    id: ElementId,
    title: SharedString,
    text: SharedString,
    theme: AppTheme,
    background: Hsla,
}

impl SaiDebugJsonScrollBlock {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        text: impl Into<SharedString>,
        theme: AppTheme,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            text: text.into(),
            theme,
            background: Hsla::from(rgb(BLOCK_BACKGROUND)).opacity(0.18),
        }
    }

    pub fn background(mut self, background: Hsla) -> Self {
        self.background = background;
        self
    }
}

impl RenderOnce for SaiDebugJsonScrollBlock {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .child(
                div()
                    .child(self.title)
                    .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Default))
                    .text_color(theme.text.secondary.opacity(0.85))
                    .tracking(0.5),
            )
            .child(
                div()
                    .id(ElementId::NamedChild(Arc::new(self.id.clone()), "scroll".into()))
                    .w_full()
                    .max_h(px(SaiDebugInspectorMetrics::JSON_BODY_SCROLL_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                            .text_color(theme.text.primary)
                            .child(selectable_text(
                                ElementId::NamedChild(Arc::new(self.id), "text".into()),
                                self.text,
                            )),
                    ),
            )
            .p(px(10.))
            .w_full()
            .bg(self.background)
            .continuous_corners(10.)
            .flex_shrink_0()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiDebugStorageEntry {
    pub id: String,
    pub key: String,
    pub type_label: String,
    pub summary_text: String,
}

impl SaiDebugStorageEntry {
    pub fn copy_block_text(&self, body: &str) -> String {
        format!("{}\n{}\n{}\n\n{}", self.key, self.type_label, self.summary_text, body)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiDebugUserDefaultsPanelKind {
    LocalStorage,
    ApplicationSupportWs,
}

pub enum SaiDebugUserDefaultsInspector {}

impl SaiDebugUserDefaultsInspector {
    const MAX_BODY_CHARS: usize = 48_000;
    const LARGE_DATA_THRESHOLD: usize = 512 * 1024;

    pub fn filtered_keys(for_kind: SaiDebugUserDefaultsPanelKind) -> Vec<String> {
        match for_kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => {
                let dict = UserDefaults::standard().dictionary_representation();
                dict.into_keys()
                    .filter(|k| {
                        let lower = k.to_lowercase();
                        if lower.starts_with("ws") {
                            return false;
                        }
                        if Self::is_media_like_key(k) {
                            return false;
                        }
                        !Self::is_likely_system_key(k)
                    })
                    .collect()
            }
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => SaiWsDiskCache::debug_payload_digest_stems(),
        }
    }

    pub fn entries(for_kind: SaiDebugUserDefaultsPanelKind) -> Vec<SaiDebugStorageEntry> {
        match for_kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => {
                let dict = UserDefaults::standard().dictionary_representation();
                Self::filtered_keys(for_kind)
                    .into_iter()
                    .map(|key| {
                        let (type_label, summary) = match dict.get(&key) {
                            Some(raw) => Self::summary_representation(raw),
                            None => ("MISSING".to_owned(), "(nil)".to_owned()),
                        };
                        SaiDebugStorageEntry { id: key.clone(), key, type_label, summary_text: summary }
                    })
                    .collect()
            }
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => SaiWsDiskCache::debug_list_payload_summaries()
                .into_iter()
                .map(|row| {
                    let display_key = row.cache_key.clone().unwrap_or_else(|| row.digest.clone());
                    let label =
                        if row.cache_key.is_some() { "APP SUPPORT \u{b7} indexed" } else { "APP SUPPORT \u{b7} file" };
                    let summary =
                        format!("{label} \u{b7} {}", SaiDebugBodyFormat::size_label_byte_count(row.file_bytes));
                    SaiDebugStorageEntry {
                        id: row.digest,
                        key: display_key,
                        type_label: label.to_owned(),
                        summary_text: summary,
                    }
                })
                .collect(),
        }
    }

    pub fn load_body_text(identifier: &str, kind: SaiDebugUserDefaultsPanelKind) -> String {
        match kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => {
                match UserDefaults::standard().dictionary_representation().get(identifier) {
                    Some(raw) => Self::string_representation(raw).1,
                    None => "(nil)".to_owned(),
                }
            }
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => SaiWsDiskCache::debug_body_text(identifier),
        }
    }

    fn is_likely_system_key(key: &str) -> bool {
        const SYSTEM_PREFIXES: [&str; 10] =
            ["Apple", "NS", "AK", "com.apple.", "CarKit", "INNext", "WebKit", "METAL", "INTEGRATION", "PK"];
        SYSTEM_PREFIXES.iter().any(|prefix| key.starts_with(prefix))
    }

    fn is_media_like_key(key: &str) -> bool {
        let lower = key.to_lowercase();
        if ["media", "thumbnail", "lottie", "image_cache", "video_cache"]
            .iter()
            .any(|needle| lower.contains(needle))
        {
            return true;
        }
        if [".mp4", ".jpg", ".jpeg", ".png"].iter().any(|needle| lower.contains(needle)) {
            return true;
        }
        lower.contains("photo") && lower.contains("cache")
    }

    /// number as text, whole doubles print without a fraction
    fn number_string(value: &DefaultsValue) -> Option<String> {
        match value {
            DefaultsValue::Integer(n) => Some(n.to_string()),
            DefaultsValue::Double(d) => Some(d.to_string()),
            _ => None,
        }
    }

    /// json value only for arrays and dicts of plain scalars, raw bytes anywhere inside disqualify it
    fn json_value(value: &DefaultsValue) -> Option<Value> {
        match value {
            DefaultsValue::Bool(b) => Some(Value::Bool(*b)),
            DefaultsValue::Integer(n) => Some(Value::Number(Number::from(*n))),
            DefaultsValue::Double(d) => Number::from_f64(*d).map(Value::Number),
            DefaultsValue::String(s) => Some(Value::String(s.clone())),
            DefaultsValue::Data(_) => None,
            DefaultsValue::Array(items) => items.iter().map(Self::json_value).collect::<Option<Vec<_>>>().map(Value::Array),
            DefaultsValue::Dictionary(entries) => entries
                .iter()
                .map(|(k, v)| Self::json_value(v).map(|v| (k.clone(), v)))
                .collect::<Option<Map<String, Value>>>()
                .map(Value::Object),
        }
    }

    fn json_container(value: &DefaultsValue) -> Option<Value> {
        match value {
            DefaultsValue::Array(_) | DefaultsValue::Dictionary(_) => Self::json_value(value),
            _ => None,
        }
    }

    fn summary_representation(value: &DefaultsValue) -> (String, String) {
        match value {
            DefaultsValue::String(s) => {
                ("STRING".to_owned(), SaiDebugBodyFormat::size_label_char_count(character_count(s) as i64))
            }
            DefaultsValue::Bool(b) => ("BOOL".to_owned(), b.to_string()),
            DefaultsValue::Integer(_) | DefaultsValue::Double(_) => {
                ("NUMBER".to_owned(), Self::number_string(value).unwrap_or_default())
            }
            DefaultsValue::Data(data) => {
                let size = SaiDebugBodyFormat::size_label_byte_count(data.len() as i64);
                if data.len() > Self::LARGE_DATA_THRESHOLD {
                    return ("DATA".to_owned(), format!("{size} (omitted until Show)"));
                }
                ("DATA".to_owned(), size)
            }
            DefaultsValue::Array(_) | DefaultsValue::Dictionary(_) => {
                if let Some(s) = Self::json_container(value).and_then(|json| serde_json::to_string(&json).ok()) {
                    return ("JSON".to_owned(), SaiDebugBodyFormat::size_label_char_count(character_count(&s) as i64));
                }
                let s = format!("{value:?}");
                ("OBJECT".to_owned(), SaiDebugBodyFormat::size_label_char_count(character_count(&s) as i64))
            }
        }
    }

    fn string_representation(value: &DefaultsValue) -> (String, String) {
        match value {
            DefaultsValue::String(s) => ("STRING".to_owned(), Self::truncate(s, Self::MAX_BODY_CHARS)),
            DefaultsValue::Bool(b) => ("BOOL".to_owned(), b.to_string()),
            DefaultsValue::Integer(_) | DefaultsValue::Double(_) => {
                ("NUMBER".to_owned(), Self::number_string(value).unwrap_or_default())
            }
            DefaultsValue::Data(data) => {
                if data.len() > Self::LARGE_DATA_THRESHOLD {
                    return (
                        "DATA".to_owned(),
                        format!("(omitted \u{2014} {} bytes; open Disk / media tooling later)", data.len()),
                    );
                }
                if let Ok(s) = std::str::from_utf8(data) {
                    let pretty = serde_json::from_slice::<Value>(data)
                        .ok()
                        .filter(|json| json.is_object() || json.is_array())
                        .and_then(|json| serde_json::to_string_pretty(&json).ok());
                    if let Some(pretty) = pretty {
                        return ("DATA (JSON)".to_owned(), Self::truncate(&pretty, Self::MAX_BODY_CHARS));
                    }
                    return ("DATA (UTF-8)".to_owned(), Self::truncate(s, Self::MAX_BODY_CHARS));
                }
                let head = &data[..data.len().min(64)];
                let suffix = if data.len() > 64 {
                    format!("\u{2026} ({} bytes)", data.len())
                } else {
                    format!(" ({} bytes)", data.len())
                };
                ("DATA (binary)".to_owned(), format!("{}{suffix}", hex_string(head)))
            }
            DefaultsValue::Array(_) | DefaultsValue::Dictionary(_) => {
                if let Some(s) = Self::json_container(value).and_then(|json| serde_json::to_string_pretty(&json).ok()) {
                    return ("JSON".to_owned(), Self::truncate(&s, Self::MAX_BODY_CHARS));
                }
                let s = format!("{value:?}");
                ("OBJECT".to_owned(), Self::truncate(&s, Self::MAX_BODY_CHARS))
            }
        }
    }

    fn truncate(s: &str, max: usize) -> String {
        if character_count(s) <= max {
            return s.to_owned();
        }
        format!("{}\u{2026}", character_prefix(s, max))
    }

    pub fn remove_key(identifier: &str, kind: SaiDebugUserDefaultsPanelKind, cx: &mut App) {
        match kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => {
                UserDefaults::standard().remove_object(identifier);
                SaiAccountCacheScope::reload_from_defaults();
            }
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => {
                if let Some(cache_key) = SaiWsDiskCache::remove_payload_file(identifier) {
                    cx.defer(move |cx| SaiWsResponseCache::drop_memory_if_present(&cache_key, cx));
                }
            }
        }
    }

    pub fn remove_all_keys(for_kind: SaiDebugUserDefaultsPanelKind, cx: &mut App) {
        match for_kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => {
                let defaults = UserDefaults::standard();
                for key in Self::filtered_keys(for_kind) {
                    defaults.remove_object(&key);
                }
                SaiAccountCacheScope::reload_from_defaults();
            }
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => {
                cx.defer(SaiWsResponseCache::clear_all_for_debug);
            }
        }
    }
}

pub type SaiDebugCardDelete = Rc<dyn Fn(&mut Window, &mut App)>;

fn badge(text: impl Into<SharedString>, fill: u32) -> Div {
    div()
        .child(text.into())
        .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
        .text_color(white())
        .line_clamp(1)
        .px(px(7.))
        .py(px(3.))
        .bg(rgb(fill))
        .capsule()
}

fn icon_button(
    id: ElementId,
    symbol: &str,
    color: Hsla,
    theme: &AppTheme,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> PlainButton {
    plain(id).on_press(action).child(
        div()
            .flex()
            .items_center()
            .justify_center()
            .frame_size(36., 32.)
            .bg(theme.bg._300.opacity(0.9))
            .clip_shape(Shape::continuous(8.))
            .children(IconUi::sized(symbol, px(15.), color, false)),
    )
}

/// card chrome both storage cards share, clipped body then the centered stroke over it
fn card_chrome(theme: &AppTheme, body: Div) -> Div {
    div()
        .relative()
        .w_full()
        .child(body.w_full().bg(theme.bg._200.opacity(0.92)).clip_shape(Shape::continuous(12.)))
        .child(centred_stroke(theme.border._200.opacity(0.65), 1., Shape::continuous(12.)))
}

#[derive(IntoElement)]
pub struct SaiDebugStorageCardView {
    entry: SaiDebugStorageEntry,
    kind: SaiDebugUserDefaultsPanelKind,
    theme: AppTheme,
    on_delete: SaiDebugCardDelete,
}

impl SaiDebugStorageCardView {
    pub fn new(entry: SaiDebugStorageEntry, kind: SaiDebugUserDefaultsPanelKind, theme: AppTheme) -> Self {
        Self { entry, kind, theme, on_delete: Rc::new(|_, _| {}) }
    }

    pub fn on_delete(mut self, on_delete: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_delete = Rc::new(on_delete);
        self
    }

    fn element_id(&self, part: &'static str) -> ElementId {
        ElementId::Name(SharedString::from(format!("sai-debug-storage-{}-{}", self.entry.id, part)))
    }

    fn header_row(&self) -> Div {
        let theme = &self.theme;
        let on_delete = self.on_delete.clone();
        let copy_entry = self.entry.clone();
        let kind = self.kind;

        hstack(6.)
            .w_full()
            .child(badge("STORE", STORE_BADGE))
            .child(badge(self.entry.type_label.clone(), TYPE_BADGE))
            .child(
                div()
                    .min_w(px(0.))
                    .child(self.entry.id.clone())
                    .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary)
                    .line_clamp(1)
                    .text_ellipsis_middle(),
            )
            .child(spacer_min_w(0.))
            .child(icon_button(self.element_id("delete"), "trash", rgb(DESTRUCTIVE_RED).into(), theme, move |window, cx| {
                on_delete(window, cx)
            }))
            .child(icon_button(self.element_id("copy"), "doc.on.doc", theme.text.primary, theme, move |_window, cx| {
                let body = SaiDebugUserDefaultsInspector::load_body_text(&copy_entry.id, kind);
                UIPasteboard::general().set_string(&copy_entry.copy_block_text(&body), cx);
            }))
            .px(px(10.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.45))
    }
}

impl RenderOnce for SaiDebugStorageCardView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let entry_id = self.entry.id.clone();
        let kind = self.kind;
        let header = self.header_row();

        let body = vstack(0.)
            .align(HorizontalAlignment::Leading)
            .child(header)
            .child(div().w_full().h(px(1.)).bg(theme.border._200))
            .child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(8.))
                    .swiftui_font(Font::system(11., FontWeight::SEMIBOLD, FontDesign::Monospaced))
                    .text_color(theme.text.primary)
                    .child(selectable_text(self.element_id("key"), self.entry.key.clone())),
            )
            .child(
                vstack(12.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .p(px(10.))
                    .bg(theme.bg._100)
                    .child(SaiDebugExpandableMonospaceBlock::new(
                        self.element_id("value"),
                        "VALUE",
                        self.entry.summary_text.clone(),
                        theme.clone(),
                        Rc::new(move |_cx| SaiDebugUserDefaultsInspector::load_body_text(&entry_id, kind)),
                    )),
            );

        card_chrome(&theme, body)
    }
}

#[derive(IntoElement)]
pub struct SaiWsCacheDebugCardView {
    cache_key: String,
    layers: String,
    theme: AppTheme,
    on_delete: SaiDebugCardDelete,
}

impl SaiWsCacheDebugCardView {
    pub fn new(cache_key: impl Into<String>, layers: impl Into<String>, theme: AppTheme) -> Self {
        Self { cache_key: cache_key.into(), layers: layers.into(), theme, on_delete: Rc::new(|_, _| {}) }
    }

    pub fn on_delete(mut self, on_delete: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_delete = Rc::new(on_delete);
        self
    }

    fn element_id(&self, part: &'static str) -> ElementId {
        ElementId::Name(SharedString::from(format!("sai-ws-cache-{}-{}", self.cache_key, part)))
    }

    fn summary_text(&self, cx: &mut App) -> String {
        format!("{} \u{b7} {}", self.layers, SaiWsResponseCache::debug_size_label(&self.cache_key, cx))
    }
}

impl RenderOnce for SaiWsCacheDebugCardView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let summary_text = self.summary_text(cx);
        let on_delete = self.on_delete.clone();
        let copy_key = self.cache_key.clone();
        let copy_summary = summary_text.clone();
        let load_key = self.cache_key.clone();

        let header = hstack(6.)
            .w_full()
            .child(badge("WS CACHE", WS_CACHE_BADGE))
            .child(badge(self.layers.clone(), TYPE_BADGE))
            .child(spacer_min_w(0.))
            .child(icon_button(self.element_id("delete"), "trash", rgb(DESTRUCTIVE_RED).into(), &theme, move |window, cx| {
                on_delete(window, cx)
            }))
            .child(icon_button(self.element_id("copy"), "doc.on.doc", theme.text.primary, &theme, move |_window, cx| {
                let body = SaiWsResponseCache::debug_body_text(&copy_key, cx);
                let copied = [copy_key.as_str(), copy_summary.as_str(), "", body.as_str()].join("\n");
                UIPasteboard::general().set_string(&copied, cx);
            }))
            .px(px(10.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.45));

        let body = vstack(0.)
            .align(HorizontalAlignment::Leading)
            .child(header)
            .child(div().w_full().h(px(1.)).bg(theme.border._200))
            .child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(8.))
                    .swiftui_font(Font::system(11., FontWeight::SEMIBOLD, FontDesign::Monospaced))
                    .text_color(theme.text.primary)
                    .child(selectable_text(self.element_id("key"), self.cache_key.clone())),
            )
            .child(
                vstack(12.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .p(px(10.))
                    .bg(theme.bg._100)
                    .child(
                        SaiDebugExpandableMonospaceBlock::new(
                            self.element_id("payload"),
                            "PAYLOAD",
                            summary_text,
                            theme.clone(),
                            Rc::new(move |cx| SaiWsResponseCache::debug_body_text(&load_key, cx)),
                        )
                        .background(Hsla::from(rgb(WS_PAYLOAD_BACKGROUND)).opacity(0.12)),
                    ),
            );

        card_chrome(&theme, body)
    }
}

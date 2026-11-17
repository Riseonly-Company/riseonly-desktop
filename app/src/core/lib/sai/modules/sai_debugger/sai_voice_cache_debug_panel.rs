#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{App, AppContext, Context, ElementId, FontWeight, Hsla, ListAlignment, SharedString, Task, Window, rgb, white};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text::TruncationMode;
use rise_ui::swiftui::text_selection::selectable_text;

use super::sai_debug_storage_panel_views::SaiDebugBodyFormat;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::{
    VoiceMessageDiskCache, VoiceMessageDiskCacheChatGroup, VoiceMessageDiskCacheListedEntry,
};

const DESTRUCTIVE_RED: u32 = 0xe11d48;
const LINK_BLUE: u32 = 0x29b6f6;
const VOICE_BADGE: u32 = 0x1565c0;
const SIZE_BADGE: u32 = 0x3949ab;

const LIST_MAX_HEIGHT: f32 = 500.;
const LIST_PADDING: f32 = 14.;
const SECTION_SPACING: f32 = 16.;
const SECTION_INNER_SPACING: f32 = 8.;

pub type SaiVoiceCacheRequestDelete = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type SaiVoiceCacheRequestClearAll = Rc<dyn Fn(&mut Window, &mut App)>;

fn color(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

/// one row of the flattened voice cache list, a chat header or a cached message under it
#[derive(Clone, PartialEq)]
enum SaiVoiceCacheRow {
    ChatHeader { chat_id: String, title: String, summary: String },
    Entry(VoiceMessageDiskCacheListedEntry),
}

impl SaiVoiceCacheRow {
    fn flattened(groups: &[VoiceMessageDiskCacheChatGroup]) -> Vec<Self> {
        let mut rows = Vec::new();
        for group in groups {
            rows.push(Self::ChatHeader {
                chat_id: group.chat_id.clone(),
                title: if group.chat_name.is_empty() { group.chat_id.clone() } else { group.chat_name.clone() },
                summary: format!(
                    "{} \u{b7} {}",
                    group.entries.len(),
                    SaiDebugBodyFormat::size_label_byte_count(group.total_bytes()),
                ),
            });
            rows.extend(group.entries.iter().cloned().map(Self::Entry));
        }
        rows
    }
}

/// voice cache debug panel, disk read runs on a worker once per snapshot tick
/// frames only commit what it returned
pub struct SaiVoiceCacheDebugPanel {
    theme: AppTheme,
    snapshot_tick: Binding<i64>,
    on_request_delete_message: SaiVoiceCacheRequestDelete,
    on_request_delete_chat: SaiVoiceCacheRequestDelete,
    on_request_clear_all: SaiVoiceCacheRequestClearAll,
    rows: Rc<Vec<SaiVoiceCacheRow>>,
    total_entries: usize,
    folder_path: String,
    loaded_tick: Option<i64>,
    list: ListUi,
    _load_task: Option<Task<()>>,
}

impl SaiVoiceCacheDebugPanel {
    pub fn new(
        theme: AppTheme,
        snapshot_tick: Binding<i64>,
        on_request_delete_message: SaiVoiceCacheRequestDelete,
        on_request_delete_chat: SaiVoiceCacheRequestDelete,
        on_request_clear_all: SaiVoiceCacheRequestClearAll,
        _cx: &mut Context<Self>,
    ) -> Self {
        let list = ListUi::new(&theme, 0, ListAlignment::Top);
        Self {
            theme,
            snapshot_tick,
            on_request_delete_message,
            on_request_delete_chat,
            on_request_clear_all,
            rows: Rc::new(Vec::new()),
            total_entries: 0,
            folder_path: String::new(),
            loaded_tick: None,
            list,
            _load_task: None,
        }
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn load_groups(&mut self, tick: i64, cx: &mut Context<Self>) {
        self.loaded_tick = Some(tick);
        self._load_task = Some(cx.spawn(async move |this, cx| {
            let (groups, folder_path) = cx
                .background_spawn(async move {
                    (VoiceMessageDiskCache::list_groups(), VoiceMessageDiskCache::folder_path_for_inspector())
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                let rows = SaiVoiceCacheRow::flattened(&groups);
                panel.splice_changed_rows(&rows);
                panel.total_entries = groups.iter().map(|group| group.entries.len()).sum();
                panel.rows = Rc::new(rows);
                panel.folder_path = folder_path;
                cx.notify();
            });
        }));
    }

    fn splice_changed_rows(&self, new_rows: &[SaiVoiceCacheRow]) {
        let old_rows = self.rows.as_slice();
        let prefix = old_rows.iter().zip(new_rows).take_while(|(old, new)| old == new).count();
        let suffix = old_rows[prefix..]
            .iter()
            .rev()
            .zip(new_rows[prefix..].iter().rev())
            .take_while(|(old, new)| old == new)
            .count();
        let old_end = old_rows.len() - suffix;
        let new_count = new_rows.len() - suffix - prefix;
        if old_end > prefix || new_count > 0 {
            self.list.state().splice(prefix..old_end, new_count);
        }
    }

    fn header(&self, has_groups: bool) -> Div {
        let theme = &self.theme;
        let on_request_clear_all = self.on_request_clear_all.clone();
        let snapshot_tick = self.snapshot_tick.clone();
        hstack(8.)
            .w_full()
            .child(
                div()
                    .child("VoiceCache")
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary),
            )
            .child(
                div()
                    .child(format!("({})", self.total_entries))
                    .swiftui_font(Font::caption2().monospaced_digit())
                    .text_color(theme.text.secondary.opacity(0.75)),
            )
            .child(spacer_min_w(0.))
            .children(has_groups.then(|| {
                plain("sai-voice-cache-clear-all")
                    .on_press(move |window, cx| on_request_clear_all(window, cx))
                    .child(
                        div()
                            .child("Clear all")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(DESTRUCTIVE_RED)),
                    )
            }))
            .child(
                plain("sai-voice-cache-refresh")
                    .on_press(move |window, cx| {
                        let tick = snapshot_tick.get(cx);
                        snapshot_tick.set(tick + 1, window, cx);
                    })
                    .child(
                        div()
                            .child("Refresh")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(LINK_BLUE)),
                    ),
            )
            .px(px(14.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.35))
    }

    fn path_line(&self) -> Div {
        div()
            .w_full()
            .px(px(12.))
            .pb(px(6.))
            .text_center()
            .child(self.folder_path.clone())
            .swiftui_font(Font::caption2())
            .text_color(self.theme.text.secondary.opacity(0.78))
            .line_clamp(3)
    }
}

fn row_id(kind: &str, id: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("sai-voice-cache-{kind}-{id}")))
}

fn badge(text: String, fill: u32) -> Div {
    div()
        .child(text)
        .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
        .text_color(white())
        .px(px(7.))
        .py(px(3.))
        .bg(rgb(fill))
        .capsule()
}

fn icon_button(id: ElementId, symbol: &'static str, tint: Hsla, on_press: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    plain(id).on_press(on_press).children(IconUi::sized(symbol, px(14.), tint, false))
}

fn chat_header_row(
    chat_id: &str,
    title: &str,
    summary: &str,
    theme: &AppTheme,
    on_request_delete_chat: SaiVoiceCacheRequestDelete,
) -> AnyElement {
    let delete_chat_id = chat_id.to_owned();
    hstack(8.)
        .w_full()
        .child(
            vstack(2.)
                .align(HorizontalAlignment::Leading)
                .min_w_0()
                .child(
                    div()
                        .child(title.to_owned())
                        .swiftui_font(Font::subheadline().weight(FontWeight::SEMIBOLD))
                        .text_color(theme.text.primary)
                        .line_clamp(1),
                )
                .child(
                    div()
                        .child(chat_id.to_owned())
                        .swiftui_font(Font::caption2().monospaced())
                        .text_color(theme.text.secondary)
                        .line_clamp(1),
                ),
        )
        .child(spacer_min_w(0.))
        .child(
            div()
                .child(summary.to_owned())
                .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                .text_color(theme.text.secondary),
        )
        .child(icon_button(row_id("delete-chat", chat_id), "trash", color(DESTRUCTIVE_RED), move |window, cx| {
            on_request_delete_chat(delete_chat_id.clone(), window, cx);
        }))
        .px(px(10.))
        .py(px(8.))
        .bg(theme.bg._300.opacity(0.45))
        .clip_shape(Shape::continuous(10.))
        .into_any_element()
}

fn voice_entry_card(
    entry: &VoiceMessageDiskCacheListedEntry,
    theme: &AppTheme,
    on_request_delete_message: SaiVoiceCacheRequestDelete,
) -> AnyElement {
    let delete_message_id = entry.message_id.clone();
    let copy_message_id = entry.message_id.clone();
    let remote_line = entry.remote_url.clone().map(|remote| {
        TruncationMode::Middle
            .apply(div().w_full().whitespace_nowrap().overflow_hidden())
            .child(remote)
            .swiftui_font(Font::caption2())
            .text_color(theme.text.secondary.opacity(0.65))
    });
    div()
        .relative()
        .w_full()
        .child(
            vstack(6.)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .child(
                    hstack(6.)
                        .w_full()
                        .child(badge("VOICE".to_owned(), VOICE_BADGE))
                        .child(badge(SaiDebugBodyFormat::size_label_byte_count(entry.file_bytes), SIZE_BADGE))
                        .child(spacer_min_w(0.))
                        .child(icon_button(
                            row_id("delete-message", &entry.message_id),
                            "trash",
                            color(DESTRUCTIVE_RED),
                            move |window, cx| on_request_delete_message(delete_message_id.clone(), window, cx),
                        ))
                        .child(icon_button(
                            row_id("copy-message", &entry.message_id),
                            "doc.on.doc",
                            theme.text.primary,
                            move |_window, cx| UIPasteboard::general().set_string(&copy_message_id, cx),
                        )),
                )
                .child(
                    div()
                        .w_full()
                        .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                        .text_color(theme.text.primary)
                        .line_clamp(2)
                        .child(selectable_text(row_id("message-id", &entry.message_id), entry.message_id.clone())),
                )
                .child(
                    TruncationMode::Middle
                        .apply(div().w_full().whitespace_nowrap().overflow_hidden())
                        .child(entry.file_name.clone())
                        .swiftui_font(Font::caption2())
                        .text_color(theme.text.secondary.opacity(0.75)),
                )
                .children(remote_line)
                .p(px(10.))
                .bg(theme.bg._200.opacity(0.92))
                .clip_shape(Shape::continuous(10.)),
        )
        .child(centred_stroke(theme.border._200.opacity(0.65), 1., Shape::continuous(10.)))
        .into_any_element()
}

impl Render for SaiVoiceCacheDebugPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tick = self.snapshot_tick.get(cx);
        if self.loaded_tick != Some(tick) {
            self.load_groups(tick, cx);
        }
        let theme = self.theme.clone();
        let rows = self.rows.clone();

        let content: AnyElement = if rows.is_empty() {
            div()
                .w_full()
                .flex()
                .justify_center()
                .px(px(LIST_PADDING))
                .pt(px(LIST_PADDING + 28.))
                .child(
                    div()
                        .text_center()
                        .child(
                            "No cached voice messages yet. Play a voice bubble to download into Application Support / VoiceMessageCache.",
                        )
                        .swiftui_font(Font::caption())
                        .text_color(theme.text.secondary),
                )
                .into_any_element()
        } else {
            let row_theme = theme.clone();
            let on_request_delete_message = self.on_request_delete_message.clone();
            let on_request_delete_chat = self.on_request_delete_chat.clone();
            self.list
                .element(move |index, _window, _cx| {
                    let Some(row) = rows.get(index) else {
                        return div().into_any_element();
                    };
                    let (top, element) = match row {
                        SaiVoiceCacheRow::ChatHeader { chat_id, title, summary } => (
                            if index == 0 { LIST_PADDING } else { SECTION_SPACING },
                            chat_header_row(chat_id, title, summary, &row_theme, on_request_delete_chat.clone()),
                        ),
                        SaiVoiceCacheRow::Entry(entry) => (
                            SECTION_INNER_SPACING,
                            voice_entry_card(entry, &row_theme, on_request_delete_message.clone()),
                        ),
                    };
                    div()
                        .w_full()
                        .px(px(LIST_PADDING))
                        .pt(px(top))
                        .pb(px(if index + 1 == rows.len() { LIST_PADDING } else { 0. }))
                        .child(element)
                        .into_any_element()
                })
                .into_any_element()
        };

        vstack(0.)
            .w_full()
            .child(self.header(!self.rows.is_empty()))
            .child(self.path_line())
            .child(div().w_full().h(px(LIST_MAX_HEIGHT)).bg(theme.bg._100).child(content))
    }
}

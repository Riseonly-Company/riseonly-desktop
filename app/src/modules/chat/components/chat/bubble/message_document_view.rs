use std::ffi::OsStr;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AsyncApp, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Task, Window, black, div, px, rgb, white,
};
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_platform::open_url::open_url;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text::TruncationMode;

use crate::core::lib::byte_format::ByteFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_transfer_hub::{
    RiseMediaTransferHub, RiseMediaTransferKey, RiseMediaTransferPhase, RiseMediaTransferRequest,
};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::rise_media_transfer_chip::{
    RiseMediaTransferChip, RiseMediaTransferChipViewMetrics, RiseMediaTransferPhaseObserver,
};
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::components::chat::bubble::rise_exact_upload_progress_ring::{
    RiseExactUploadProgressRing, RiseExactUploadStatusLabel,
};
use crate::modules::chat::components::chat::media_viewer::chat_media_presentation::ChatGalleryMessagesKey;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::utils::chat_document_open::ChatDocumentOpen;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaItem};

const ICON_LEADING_INSET: f32 = 3.0;

pub type MessageDocumentViewAction = Rc<dyn Fn(&mut Window, &mut App)>;

struct MessageDocumentViewState {
    is_opening: bool,
    transfer_phase: RiseMediaTransferPhase,
    opening_request_id: Option<String>,
    opening_task: Option<Task<()>>,
}

impl Default for MessageDocumentViewState {
    fn default() -> Self {
        Self {
            is_opening: false,
            transfer_phase: RiseMediaTransferPhase::Idle,
            opening_request_id: None,
            opening_task: None,
        }
    }
}

impl MessageDocumentViewState {
    fn transfer_is_active(&self) -> bool {
        matches!(self.transfer_phase, RiseMediaTransferPhase::Queued | RiseMediaTransferPhase::Downloading)
    }

    fn cancel_document_opening(&mut self) {
        self.opening_task = None;
        self.opening_request_id = None;
        self.is_opening = false;
    }

    fn clear_document_opening(&mut self, request_id: &str) {
        if self.opening_request_id.as_deref() != Some(request_id) {
            return;
        }
        if let Some(task) = self.opening_task.take() {
            task.detach();
        }
        self.opening_request_id = None;
        self.is_opening = false;
    }
}

/// identity for the open or transfer state, not just the transfer key
/// a slot reused for another document keeps transfer key none on both sides
/// so keying on it never resets
#[derive(Clone, PartialEq, Eq)]
struct DocumentObservationIdentity {
    document: String,
    transfer: Option<RiseMediaTransferKey>,
}

#[derive(IntoElement)]
pub struct MessageDocumentView {
    id: ElementId,
    item: MessageMediaItem,
    message_id: String,
    chat_id: String,
    caption: String,
    is_own: bool,
    on_cancel_upload: Option<MessageDocumentViewAction>,
}

impl MessageDocumentView {
    pub fn new(
        id: impl Into<ElementId>,
        item: MessageMediaItem,
        message_id: impl Into<String>,
        chat_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            item,
            message_id: message_id.into(),
            chat_id: chat_id.into(),
            caption: String::new(),
            is_own: false,
            on_cancel_upload: None,
        }
    }

    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = caption.into();
        self
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn on_cancel_upload(mut self, on_cancel_upload: Option<MessageDocumentViewAction>) -> Self {
        self.on_cancel_upload = on_cancel_upload;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn is_transferring(&self) -> bool {
        self.item.is_uploading || self.item.is_compressing
    }

    fn document_icon(
        &self,
        ext: &str,
        theme: &AppTheme,
        preview: Option<&str>,
        state: &MessageDocumentViewState,
        transfer_key: Option<&RiseMediaTransferKey>,
        is_layout_measurement: bool,
    ) -> AnyElement {
        let side = Self::icon_side(preview.is_some());
        let surface = self
            .document_icon_surface(ext, theme, preview, state, transfer_key, is_layout_measurement)
            .frame_size(side, side)
            .rounded(px(Self::icon_corner_radius(preview.is_some())))
            .overflow_hidden();
        let inset = div().flex_none().padding_edges(EdgeSet::LEADING, ICON_LEADING_INSET);
        if self.is_transferring() {
            let on_cancel_upload = self.on_cancel_upload.clone();
            inset
                .child(surface.bubble_tap_target_priority(self.child_id("cancel-upload"), move |window, cx| {
                    if let Some(on_cancel_upload) = &on_cancel_upload {
                        on_cancel_upload(window, cx);
                    }
                }))
                .into_any_element()
        } else {
            inset.child(surface).into_any_element()
        }
    }

    fn document_icon_surface(
        &self,
        ext: &str,
        theme: &AppTheme,
        preview: Option<&str>,
        state: &MessageDocumentViewState,
        transfer_key: Option<&RiseMediaTransferKey>,
        is_layout_measurement: bool,
    ) -> gpui::Div {
        let surface = div().relative().flex().items_center().justify_center();
        let surface = match preview {
            Some(preview) => surface.child(
                div().absolute().inset_0().child(
                    CleverImageUi::new(preview.to_owned())
                        .id(self.child_id("preview"))
                        .content_mode(ContentMode::Fill)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(222),
                ),
            ),
            None => surface.child(div().absolute().inset_0().rounded_full().bg(self.ext_color(ext, theme))),
        };
        let dimmer = || div().absolute().size(px(44.0)).rounded_full().bg(black().opacity(0.42));
        let ink = if preview.is_some() { white() } else { theme.content_on(self.ext_color(ext, theme)) };
        if self.is_transferring() {
            return surface
                .when(preview.is_some(), |surface| surface.child(dimmer()))
                .child(
                    div().absolute().frame_size(38.0, 38.0).child(
                        RiseExactUploadProgressRing::new(
                            self.child_id("upload-ring"),
                            self.message_id.clone(),
                            self.item.clone(),
                            is_layout_measurement,
                        )
                        .progress_color(ink)
                        .track_color(ink.opacity(0.28))
                        .line_width(2.2)
                        .ring_inset(3.0),
                    ),
                )
                .child(div().relative().children(IconUi::sized("xmark", px(13.0), ink, false)));
        }
        if let Some(key) = transfer_key.filter(|_| state.transfer_is_active() || state.is_opening) {
            return surface.when(preview.is_some(), |surface| surface.child(dimmer())).child(
                div().absolute().frame_size(38.0, 38.0).child(
                    RiseMediaTransferChip::new(self.child_id("transfer-ring"), Some(key.clone()))
                        .declared_byte_count(self.item.effective_total_bytes())
                        .metrics(RiseMediaTransferChipViewMetrics::ring(38.0, 2.2, None, None))
                        .shows_byte_counts(false)
                        .shows_idle_affordance(false)
                        .is_interactive(true)
                        .hides_small_transfers(false)
                        .is_layout_measurement(is_layout_measurement)
                        .fills_sized_box(true),
                ),
            );
        }
        surface.when(preview.is_none() && !state.transfer_is_active() && !state.is_opening, |surface| {
            surface.child(
                div()
                    .relative()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(ink)
                    .child(MinimumScaleText::new(character_prefix(ext, 4).to_owned(), 0.5).line_limit(1)),
            )
        })
    }

    fn icon_side(has_preview: bool) -> f32 {
        if has_preview { 60.0 } else { 44.0 }
    }

    fn icon_corner_radius(has_preview: bool) -> f32 {
        if has_preview { 10.0 } else { 22.0 }
    }

    fn preview_thumbnail(&self) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(self.item.thumbnail_url.as_deref()?);
        (!value.is_empty() && MessageMediaItem::is_decodable_image_source(value)).then(|| value.to_owned())
    }

    fn transfer_subtitle(&self) -> Option<String> {
        if self.item.is_compressing {
            return Some(tr("chat_media_upload_processing"));
        }
        if self.item.is_uploading {
            let total = self.item.effective_total_bytes();
            if self.item.upload_progress >= 99.5 || (total > 0 && self.item.clamped_uploaded_bytes() >= total) {
                return Some(tr("chat_media_upload_sending"));
            }
            if total > 0 {
                return Some(format_printf(
                    &tr("chat_media_upload_progress"),
                    &[
                        FormatArg::Text(
                            ByteFormat::telegram_transferred(self.item.clamped_uploaded_bytes(), total).into(),
                        ),
                        FormatArg::Text(ByteFormat::telegram(total).into()),
                    ],
                ));
            }
            return Some(tr("chat_media_upload_loading"));
        }
        let size = self.item.effective_total_bytes();
        (size > 0).then(|| ByteFormat::telegram(size))
    }

    fn reserved_transfer_subtitle(&self) -> Option<String> {
        let total = self.item.effective_total_bytes();
        if total <= 0 {
            return None;
        }
        let formatted = ByteFormat::telegram(total);
        Some(format_printf(
            &tr("chat_media_upload_progress"),
            &[FormatArg::Text(formatted.as_str().into()), FormatArg::Text(formatted.as_str().into())],
        ))
    }

    fn subtitle_color(&self, theme: &AppTheme) -> Hsla {
        if self.is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary }
    }

    fn transfer_subtitle_text(&self, text: String, theme: &AppTheme) -> MainText {
        MainText::new(text)
            .px(13.0)
            .monospaced_digits(true)
            .color(self.subtitle_color(theme))
            .number_of_lines(1)
            .truncation(TruncationMode::Middle)
    }

    fn hidden_transfer_subtitle(&self, text: String, theme: &AppTheme) -> AnyElement {
        div().opacity(0.0).child(self.transfer_subtitle_text(text, theme)).into_any_element()
    }

    fn subtitle(
        &self,
        theme: &AppTheme,
        transfer_key: Option<RiseMediaTransferKey>,
        is_layout_measurement: bool,
    ) -> Option<AnyElement> {
        let reserved = self.reserved_transfer_subtitle();
        if let Some(key) = transfer_key {
            let reserved = reserved.map(|reserved| self.hidden_transfer_subtitle(reserved, theme));
            return Some(
                zstack(Alignment::Leading)
                    .children(reserved)
                    .child(
                        div().flex_none().child(
                            RiseMediaTransferChip::new(self.child_id("transfer-inline"), Some(key))
                                .declared_byte_count(self.item.effective_total_bytes())
                                .metrics(RiseMediaTransferChipViewMetrics::inline_label(
                                    13.0,
                                    self.subtitle_color(theme),
                                ))
                                .shows_idle_affordance(true)
                                .shows_size_when_cached(true)
                                .is_interactive(false)
                                .is_layout_measurement(is_layout_measurement)
                                .hugs_content_cached(true),
                        ),
                    )
                    .into_any_element(),
            );
        }
        let subtitle = self.transfer_subtitle()?;
        let shows_live_upload = self.is_transferring() && reserved.is_some() && !is_layout_measurement;
        let reserved = reserved.map(|reserved| self.hidden_transfer_subtitle(reserved, theme));
        let processing_reserve = shows_live_upload.then(|| {
            self.hidden_transfer_subtitle(format!("{} 100%", tr("chat_media_upload_processing")), theme)
        });
        let text = if shows_live_upload {
            self.hidden_transfer_subtitle(subtitle.clone(), theme)
        } else {
            self.transfer_subtitle_text(subtitle.clone(), theme).into_any_element()
        };
        Some(
            div()
                .relative()
                .child(zstack(Alignment::Leading).children(reserved).children(processing_reserve).child(text))
                .when(shows_live_upload, |this| {
                    this.child(
                        div().absolute().inset_0().flex().items_center().child(
                            RiseExactUploadStatusLabel::new(
                                self.child_id("upload-status"),
                                self.message_id.clone(),
                                self.item.clone(),
                                subtitle,
                                is_layout_measurement,
                            )
                            .font_size(13.0)
                            .text_color(self.subtitle_color(theme)),
                        ),
                    )
                })
                .into_any_element(),
        )
    }

    fn document_observation_identity(&self) -> DocumentObservationIdentity {
        DocumentObservationIdentity {
            document: format!("{}|{}|{}", self.chat_id, self.message_id, self.item.id),
            transfer: self.transfer_key(),
        }
    }

    fn transfer_key(&self) -> Option<RiseMediaTransferKey> {
        if self.is_transferring() {
            return None;
        }
        self.transfer_request().map(|request| request.key())
    }

    fn transfer_request(&self) -> Option<RiseMediaTransferRequest> {
        ChatDocumentOpen::transfer_request(&self.item)
    }

    fn accessibility_document_state(&self, state: &MessageDocumentViewState) -> String {
        if self.is_transferring() || state.transfer_is_active() {
            return tr("cancel");
        }
        if matches!(state.transfer_phase, RiseMediaTransferPhase::Failed { .. }) {
            return tr("chat_media_download_failed");
        }
        self.transfer_subtitle().unwrap_or_default()
    }

    fn open_document(
        item: &MessageMediaItem,
        message_id: &str,
        chat_id: &str,
        gallery_messages: Option<&Vec<Arc<GetMessageMessage>>>,
        state: &Entity<MessageDocumentViewState>,
        cx: &mut App,
    ) {
        if ChatDocumentOpen::plays_in_app_player(item) {
            let chat_queue = gallery_messages
                .map(|messages| ChatAudioTrackAdapter::tracks(chat_id, messages.as_slice()))
                .unwrap_or_default();
            ChatDocumentOpen::toggle_in_app_playback(item, message_id, None, chat_id, chat_queue, cx);
            return;
        }
        let raw_url = item.resolved_local_preview_url().unwrap_or_else(|| item.url.clone());
        let Some(source) = ChatDocumentOpen::source_url(&raw_url) else {
            return;
        };
        let file_name = item.file_name.clone();
        if source.is_file_url() {
            let Some(path) = source.to_file_path() else {
                return;
            };
            cx.spawn(async move |cx: &mut AsyncApp| {
                let named = ChatDocumentOpen::named_for_opening(path, file_name, cx).await;
                cx.update(|cx| {
                    ChatDocumentOpen::present(&named, None, cx);
                });
            })
            .detach();
            return;
        }
        let Some(request) = ChatDocumentOpen::transfer_request(item) else {
            open_url(&source, cx);
            return;
        };
        let (transfer_is_active, is_opening) = {
            let state = state.read(cx);
            (state.transfer_is_active(), state.is_opening)
        };
        tracing::warn!(target: "riseonly::X1921", transfer_is_active, is_opening, "document tap");
        if transfer_is_active || is_opening {
            state.update(cx, |state, cx| {
                state.cancel_document_opening();
                cx.notify();
            });
            let key = request.key();
            cx.spawn(async move |_: &mut AsyncApp| RiseMediaTransferHub::shared().cancel(&key).await).detach();
            return;
        }

        let request_id = Uuid::new_v4().uuid_string();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let weak_state = state.downgrade();
        let task_request_id = request_id.clone();
        state.update(cx, |state, cx| {
            state.cancel_document_opening();
            state.opening_request_id = Some(request_id);
            state.is_opening = true;
            cx.notify();
        });
        let task = cx.spawn(async move |cx: &mut AsyncApp| {
            let hub = RiseMediaTransferHub::shared();
            let fetched = hub.fetch(request, cx).await;
            let is_current_account = || {
                SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            };
            let is_current_request = cx.update(|cx| {
                let Some(state) = weak_state.upgrade() else {
                    return false;
                };
                if state.read(cx).opening_request_id.as_deref() != Some(task_request_id.as_str()) {
                    return false;
                }
                state.update(cx, |state, cx| {
                    state.clear_document_opening(&task_request_id);
                    cx.notify();
                });
                true
            });
            tracing::warn!(
                target: "riseonly::X1921",
                ok = fetched.is_ok(),
                error = ?fetched.as_ref().err(),
                is_current_request,
                "document fetch settled"
            );
            if !is_current_request || !is_current_account() {
                return;
            }
            match fetched {
                Ok(local) => {
                    let lease = RemoteMediaDiskCache::lease(Some(local.as_path()));
                    ChatDocumentOpen::mark_as_downloaded(&local, &source, cx).await;
                    let named = ChatDocumentOpen::named_for_opening(local, file_name, cx).await;
                    cx.update(|cx| {
                        ChatDocumentOpen::present(&named, lease, cx);
                    });
                }
                Err(_) => cx.update(|cx| ChatDocumentOpen::open_after_failed_download(&source, cx)),
            }
        });
        state.update(cx, |state, _| state.opening_task = Some(task));
    }

    fn display_file_name(&self) -> String {
        let raw = trimming_whitespaces_and_newlines(self.item.file_name.as_deref().unwrap_or(""));
        let resolved_url = ChatDocumentOpen::source_url(&self.item.url);
        let url_name = resolved_url.as_ref().map(|url| url.last_path_component()).unwrap_or_default();
        let fallback = if raw.is_empty() { url_name } else { raw.to_owned() };
        if fallback.is_empty() {
            return tr("msg_preview_document_fallback");
        }
        let url_extension = resolved_url.map(|url| url.path_extension()).unwrap_or_default();
        if Path::new(&fallback).extension().and_then(OsStr::to_str).is_some_and(|ext| !ext.is_empty())
            || url_extension.is_empty()
            || url_extension.to_lowercase() == "bin"
        {
            return fallback;
        }
        format!("{fallback}.{url_extension}")
    }

    fn wrappable_file_name(name: &str) -> String {
        match name.rfind('.') {
            Some(dot) if dot > 0 => format!("{}\u{200B}{}", &name[..dot], &name[dot..]),
            _ => name.to_owned(),
        }
    }

    fn file_ext(name: &str) -> String {
        match name.rfind('.') {
            Some(dot) => name[dot + 1..].to_owned(),
            None => "FILE".to_owned(),
        }
    }

    fn ext_color(&self, ext: &str, theme: &AppTheme) -> Hsla {
        match ext {
            "PDF" => rgb(0xE03B3B).into(),
            "ZIP" | "RAR" | "7Z" => rgb(0x7E57C2).into(),
            "DOC" | "DOCX" | "TXT" | "RTF" => rgb(0x3D7BD9).into(),
            "XLS" | "XLSX" | "CSV" | "NUMBERS" => rgb(0x34A853).into(),
            "PPT" | "KEY" => rgb(0xF59E2B).into(),
            "FIG" | "FIGMA" => rgb(0xA259FF).into(),
            "PSD" | "AI" | "SKETCH" => rgb(0xFF7A00).into(),
            "MP4" | "MOV" | "AVI" => rgb(0x5E81F4).into(),
            "MP3" | "WAV" | "FLAC" | "M4A" => rgb(0x1DB954).into(),
            _ => theme.primary._100,
        }
    }
}

impl RenderOnce for MessageDocumentView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let is_layout_measurement = is_message_layout_measurement(cx);
        let gallery_messages = env::<ChatGalleryMessagesKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| MessageDocumentViewState::default());
        let name = self.display_file_name();
        let ext = Self::file_ext(&name).to_uppercase();
        let preview = self.preview_thumbnail();
        let transfer_key = self.transfer_key();

        let task_state = state.clone();
        let task_key = transfer_key.clone();
        use_task(self.child_id("observation"), self.document_observation_identity(), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                task_state.update(cx, |state, cx| {
                    let changed = state.transfer_phase != RiseMediaTransferPhase::Idle || state.is_opening;
                    state.transfer_phase = RiseMediaTransferPhase::Idle;
                    state.cancel_document_opening();
                    if changed {
                        cx.notify();
                    }
                })
            });
            if is_layout_measurement {
                return;
            }
            let Some(key) = task_key else {
                return;
            };
            RiseMediaTransferPhaseObserver::observe(key, cx, move |phase, cx| {
                task_state.update(cx, |state, cx| {
                    if state.transfer_phase != phase {
                        state.transfer_phase = phase;
                        cx.notify();
                    }
                });
            })
            .await;
        });

        let accessibility_label = {
            let state = state.read(cx);
            format!(
                "{}: {name}, {}",
                tr("msg_preview_document_fallback"),
                self.accessibility_document_state(state)
            )
        };
        let icon = self.document_icon(
            &ext,
            &theme,
            preview.as_deref(),
            state.read(cx),
            transfer_key.as_ref(),
            is_layout_measurement,
        );
        let subtitle = self.subtitle(&theme, transfer_key, is_layout_measurement);
        let side = Self::icon_side(preview.is_some());
        let tap_id = self.child_id("tap");
        let disappear_id = self.child_id("disappear");
        let row_id = self.child_id("row");

        let content = vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .max_w(px(280.0))
            .min_w_0()
            .cursor_pointer()
            .id(row_id)
            .aria_label(accessibility_label)
            .child(
                hstack(8.0)
                    .w_full()
                    .min_h(px(side))
                    .child(icon)
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .min_w_0()
                            .child(
                                MainText::new(Self::wrappable_file_name(&name))
                                    .px(16.0)
                                    .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary })
                                    .number_of_lines(1)
                                    .truncation(TruncationMode::Middle),
                            )
                            .children(subtitle),
                    )
                    .child(spacer_min_w(0.0)),
            )
            .when(!self.caption.is_empty(), |this| {
                this.child(
                    MainText::new(self.caption.clone())
                        .px(MessageSizes::text_font_size())
                        .color(if self.is_own { theme.content_on(theme.primary._100).opacity(0.95) } else { theme.text.primary }),
                )
            });

        let disappear_state = state.clone();
        let MessageDocumentView { item, message_id, chat_id, .. } = self;
        content
            .bubble_tap_target(tap_id, move |_, cx| {
                if item.is_uploading || item.is_compressing {
                    return;
                }
                Self::open_document(&item, &message_id, &chat_id, gallery_messages.as_deref(), &state, cx);
            })
            .on_disappear(disappear_id, move |_, cx| {
                disappear_state.update(cx, |state, _| state.cancel_document_opening());
            })
    }
}

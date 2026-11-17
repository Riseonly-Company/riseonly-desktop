use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, RenderOnce, SharedString, Window};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerResult};
use crate::modules::stickers::engine::rise_sticker_models::{RiseStickerPerformancePolicy, StickerMediaUploadDraft};
use crate::modules::stickers::engine::rise_sticker_repository::RiseStickerRepository;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{PackKind, StickerItem, StickerPack};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Idle,
    Composer,
    ClosingComposerForMedia,
    ClosingComposerForSubmission,
    ClosingComposerForDismissal,
    MediaPicker,
    ClosingMediaForComposer,
}

#[derive(Clone, Debug)]
pub struct ActiveSubmission {
    pub pack: StickerPack,
    pub item: Option<MediaPickerItem>,
    pub original_sticker: Option<StickerItem>,
    pub emojis: Vec<String>,
    pub request_id: String,
    pub draft: Option<StickerMediaUploadDraft>,
    pub owned_temporary_urls: HashSet<Url>,
    pub upload_dispatched: bool,
}

pub struct StickerAssetComposerStore {
    route: Route,
    editor_pack: Option<StickerPack>,
    editing_sticker: Option<StickerItem>,
    selected_emojis: Vec<String>,
    selected_media: Option<MediaPickerItem>,
    shows_system_emoji_picker: bool,
    active_submission: Option<ActiveSubmission>,
    pub error_text: Option<String>,
}

impl StickerAssetComposerStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_: &mut Context<Self>) -> Self {
        Self {
            route: Route::Idle,
            editor_pack: None,
            editing_sticker: None,
            selected_emojis: Vec::new(),
            selected_media: None,
            shows_system_emoji_picker: false,
            active_submission: None,
            error_text: None,
        }
    }

    pub fn route(&self) -> Route {
        self.route
    }

    pub fn editor_pack(&self) -> Option<&StickerPack> {
        self.editor_pack.as_ref()
    }

    pub fn editing_sticker(&self) -> Option<&StickerItem> {
        self.editing_sticker.as_ref()
    }

    pub fn selected_emojis(&self) -> &[String] {
        &self.selected_emojis
    }

    pub fn selected_media(&self) -> Option<&MediaPickerItem> {
        self.selected_media.as_ref()
    }

    pub fn shows_system_emoji_picker(&self) -> bool {
        self.shows_system_emoji_picker
    }

    pub fn active_submission(&self) -> Option<&ActiveSubmission> {
        self.active_submission.as_ref()
    }

    fn set_route(&mut self, route: Route, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X1603", from = ?self.route, to = ?route, "composer route");
        self.route = route;
        GuideFacts::set(RiseGuideSignal::StickerComposerOpen, route == Route::Composer, cx);
        cx.notify();
    }

    fn set_selected_emojis(&mut self, emojis: Vec<String>, cx: &mut Context<Self>) {
        self.selected_emojis = emojis;
        GuideFacts::set(RiseGuideSignal::StickerEmojiChosen, !self.selected_emojis.is_empty(), cx);
        cx.notify();
    }

    fn set_selected_media(&mut self, media: Option<MediaPickerItem>, cx: &mut Context<Self>) {
        self.selected_media = media;
        GuideFacts::set(RiseGuideSignal::StickerMediaChosen, self.selected_media.is_some(), cx);
        cx.notify();
    }

    pub fn target_pack(&self) -> Option<&StickerPack> {
        self.editor_pack.as_ref().or(self.active_submission.as_ref().map(|submission| &submission.pack))
    }

    pub fn target_sticker(&self) -> Option<&StickerItem> {
        self.editing_sticker
            .as_ref()
            .or(self.active_submission.as_ref().and_then(|submission| submission.original_sticker.as_ref()))
    }

    pub fn active_pack_id(&self) -> Option<&str> {
        self.active_submission.as_ref().map(|submission| submission.pack.id.as_str())
    }

    pub fn active_sticker_id(&self) -> Option<&str> {
        self.active_submission
            .as_ref()
            .and_then(|submission| submission.original_sticker.as_ref())
            .map(|sticker| sticker.id.as_str())
    }

    pub fn is_editing(&self) -> bool {
        self.editing_sticker.is_some()
    }

    pub fn mounts_composer(&self) -> bool {
        matches!(
            self.route,
            Route::Composer
                | Route::ClosingComposerForMedia
                | Route::ClosingComposerForSubmission
                | Route::ClosingComposerForDismissal
        )
    }

    pub fn is_composer_presented(&self) -> bool {
        self.route == Route::Composer
    }

    pub fn preserves_input_picker_presentation(&self) -> bool {
        self.route != Route::Idle
    }

    pub fn mounts_media_picker(&self) -> bool {
        self.route == Route::MediaPicker || self.route == Route::ClosingMediaForComposer
    }

    pub fn is_media_picker_presented(&self) -> bool {
        self.route == Route::MediaPicker
    }

    pub fn can_submit(&self) -> bool {
        if self.route != Route::Composer || self.selected_emojis.is_empty() || self.active_submission.is_some() {
            return false;
        }
        if let Some(editing_sticker) = &self.editing_sticker {
            return self.selected_media.is_some()
                || Self::normalized_emojis(&editing_sticker.associated_emojis) != self.selected_emojis;
        }
        self.selected_media.is_some()
    }

    pub fn can_select_more_emoji(&self) -> bool {
        (self.selected_emojis.len() as i64) < RiseStickerPerformancePolicy::associated_emojis_limit()
    }

    /// server dedupes and caps the list so send its shape,
    /// order kept and duplicates dropped and truncated to the cap
    fn normalized_emojis(values: &[String]) -> Vec<String> {
        let limit = RiseStickerPerformancePolicy::associated_emojis_limit();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for value in values {
            let Some(normalized) = EmojiPickerCatalog::normalized_selectable_emoji(Some(value)) else {
                continue;
            };
            if !seen.insert(normalized.clone()) {
                continue;
            }
            out.push(normalized);
            if out.len() as i64 == limit {
                break;
            }
        }
        out
    }

    pub fn present(&mut self, pack: StickerPack, cx: &mut Context<Self>) {
        self.drop_failed_submission(cx);
        let services = StickersServicesStore::shared(cx);
        tracing::warn!(
            target: "riseonly::X1603",
            route = ?self.route,
            active_submission = self.active_submission.as_ref().map(|submission| submission.request_id.as_str()),
            can_add = pack.can_add_stickers(),
            pack_id = %pack.id,
            uploading = services.read(cx).is_uploading_sticker(&pack.id),
            count = services.read(cx).pack_stickers(&pack.id).len(),
            "composer present"
        );
        if self.route != Route::Idle
            || self.active_submission.is_some()
            || !pack.can_add_stickers()
            || pack.id.is_empty()
            || services.read(cx).is_uploading_sticker(&pack.id)
        {
            return;
        }
        let count = services.read(cx).pack_stickers(&pack.id).len() as i64;
        if count >= RiseStickerPerformancePolicy::stickers_per_pack_limit() {
            let failure = anyhow::Error::new(RiseStickerRepository::pack_full_failure());
            Notifier::send(
                CanonErrors::refusal_text(&failure),
                NotifierSendOptions { r#type: NotifierType::Error, ..Default::default() },
                cx,
            );
            return;
        }
        self.editor_pack = Some(pack);
        self.editing_sticker = None;
        self.set_selected_emojis(Vec::new(), cx);
        self.set_selected_media(None, cx);
        self.shows_system_emoji_picker = false;
        self.error_text = None;
        self.set_route(Route::Composer, cx);
    }

    pub fn present_sticker(&mut self, pack: StickerPack, sticker: StickerItem, cx: &mut Context<Self>) {
        self.drop_failed_submission(cx);
        let services = StickersServicesStore::shared(cx);
        tracing::warn!(
            target: "riseonly::X1603",
            route = ?self.route,
            active_submission = self.active_submission.as_ref().map(|submission| submission.request_id.as_str()),
            can_edit = pack.can_edit_stickers(),
            pack_id = %pack.id,
            sticker_pack_id = %sticker.pack_id,
            uploading = services.read(cx).sticker_upload(&sticker.id).is_some(),
            "composer present sticker"
        );
        if self.route != Route::Idle
            || self.active_submission.is_some()
            || !pack.can_edit_stickers()
            || pack.id.is_empty()
            || sticker.pack_id != pack.id
            || services.read(cx).sticker_upload(&sticker.id).is_some()
        {
            return;
        }
        self.editor_pack = Some(pack);
        let emojis = Self::normalized_emojis(&sticker.associated_emojis);
        self.editing_sticker = Some(sticker);
        self.set_selected_emojis(emojis, cx);
        self.set_selected_media(None, cx);
        self.shows_system_emoji_picker = false;
        self.error_text = None;
        self.set_route(Route::Composer, cx);
    }

    pub fn set_composer_presented(&mut self, open: bool, cx: &mut Context<Self>) {
        if open || self.route != Route::Composer {
            return;
        }
        self.set_route(Route::ClosingComposerForDismissal, cx);
    }

    pub fn composer_did_dismiss(&mut self, cx: &mut Context<Self>) {
        match self.route {
            Route::ClosingComposerForMedia => self.set_route(Route::MediaPicker, cx),
            Route::ClosingComposerForSubmission | Route::ClosingComposerForDismissal => {
                self.clear_editor(cx);
                self.set_route(Route::Idle, cx);
            }
            _ => {}
        }
    }

    pub fn toggle_system_emoji_picker(&mut self, cx: &mut Context<Self>) {
        if self.route != Route::Composer {
            return;
        }
        self.shows_system_emoji_picker = !self.shows_system_emoji_picker;
        cx.notify();
    }

    pub fn select_emoji(&mut self, emoji: &str, cx: &mut Context<Self>) {
        if self.route != Route::Composer {
            return;
        }
        let Some(normalized) = EmojiPickerCatalog::normalized_selectable_emoji(Some(emoji)) else {
            return;
        };
        if self.selected_emojis.contains(&normalized) {
            self.remove_emoji(&normalized, cx);
            return;
        }
        if !self.can_select_more_emoji() {
            return;
        }
        let mut emojis = self.selected_emojis.clone();
        emojis.push(normalized);
        self.set_selected_emojis(emojis, cx);
    }

    pub fn remove_emoji(&mut self, emoji: &str, cx: &mut Context<Self>) {
        if self.route != Route::Composer || !self.selected_emojis.iter().any(|selected| selected == emoji) {
            return;
        }
        let emojis = self.selected_emojis.iter().filter(|selected| *selected != emoji).cloned().collect();
        self.set_selected_emojis(emojis, cx);
    }

    pub fn open_media_picker(&mut self, cx: &mut Context<Self>) {
        if self.route != Route::Composer {
            return;
        }
        self.shows_system_emoji_picker = false;
        self.set_route(Route::ClosingComposerForMedia, cx);
    }

    pub fn set_media_picker_presented(&mut self, open: bool, cx: &mut Context<Self>) {
        if open || self.route != Route::MediaPicker {
            return;
        }
        self.set_route(Route::ClosingMediaForComposer, cx);
    }

    pub fn receive_media_picker_result(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
        if !result.was_cancelled
            && let Some(item) = result.items.into_iter().next()
        {
            self.set_selected_media(Some(item), cx);
        }
        if self.route == Route::MediaPicker {
            self.set_route(Route::ClosingMediaForComposer, cx);
        }
    }

    pub fn media_picker_did_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.route != Route::ClosingMediaForComposer {
            return;
        }
        let route = if self.editor_pack.is_none() { Route::Idle } else { Route::Composer };
        self.set_route(route, cx);
    }

    pub fn begin_submission(&mut self, cx: &mut Context<Self>) -> Option<ActiveSubmission> {
        if !self.can_submit() {
            return None;
        }
        let pack = self.editor_pack.clone()?;
        let submission = ActiveSubmission {
            pack,
            item: self.selected_media.clone(),
            original_sticker: self.editing_sticker.clone(),
            emojis: self.selected_emojis.clone(),
            request_id: Uuid::new_v4().uuid_string(),
            draft: None,
            owned_temporary_urls: HashSet::new(),
            upload_dispatched: false,
        };
        self.active_submission = Some(submission.clone());
        self.error_text = None;
        self.set_route(Route::ClosingComposerForSubmission, cx);
        Some(submission)
    }

    pub fn attach_prepared_draft(
        &mut self,
        draft: StickerMediaUploadDraft,
        owned_temporary_urls: HashSet<Url>,
        request_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(submission) = self.active_submission.as_mut().filter(|active| active.request_id == request_id) else {
            return;
        };
        submission.draft = Some(draft);
        submission.owned_temporary_urls.extend(owned_temporary_urls);
        cx.notify();
    }

    pub fn mark_upload_dispatched(&mut self, request_id: &str, cx: &mut Context<Self>) {
        let Some(submission) = self.active_submission.as_mut().filter(|active| active.request_id == request_id) else {
            return;
        };
        submission.upload_dispatched = true;
        cx.notify();
    }

    pub fn show_error(&mut self, value: String, cx: &mut Context<Self>) {
        self.set_route(Route::Idle, cx);
        self.clear_editor(cx);
        self.error_text = Some(value);
        cx.notify();
    }

    pub fn retry_submission(&mut self, cx: &mut Context<Self>) -> Option<ActiveSubmission> {
        let submission = self.active_submission.as_mut()?;
        submission.upload_dispatched = false;
        let submission = submission.clone();
        self.error_text = None;
        cx.notify();
        Some(submission)
    }

    pub fn is_active(&self, request_id: &str, pack_id: &str) -> bool {
        self.active_submission
            .as_ref()
            .is_some_and(|submission| submission.request_id == request_id && submission.pack.id == pack_id)
    }

    pub fn cancel_upload(&mut self, request_id: &str, pack_id: &str, cx: &mut Context<Self>) -> bool {
        if !self.is_active(request_id, pack_id) {
            return false;
        }
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.cancel_sticker_upload(pack_id, request_id, cx);
        });
        self.clear_all(cx);
        true
    }

    pub fn complete_upload(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if self.active_submission.as_ref().is_none_or(|submission| submission.request_id != request_id) {
            return;
        }
        self.clear_all(cx);
    }

    /// failed request waiting on its retry toast, it never blocks a new add tap
    pub fn failed_request_id(&self) -> Option<&str> {
        if self.error_text.is_none() {
            return None;
        }
        self.active_submission.as_ref().map(|submission| submission.request_id.as_str())
    }

    pub fn dismiss_failed(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if self.failed_request_id() != Some(request_id) {
            return;
        }
        self.dismiss(cx);
    }

    fn drop_failed_submission(&mut self, cx: &mut Context<Self>) {
        if self.failed_request_id().is_some() {
            self.dismiss(cx);
        }
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if let Some(submission) = self.active_submission.clone() {
            StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                match &submission.original_sticker {
                    Some(sticker) => interactions.clear_sticker_edit_error(&sticker.id, cx),
                    None => interactions.clear_sticker_upload_error(&submission.pack.id, cx),
                }
                interactions.cancel_sticker_upload(&submission.pack.id, &submission.request_id, cx);
                interactions.discard_sticker_upload_attempt(&submission.request_id, cx);
            });
        }
        self.clear_all(cx);
    }

    fn clear_editor(&mut self, cx: &mut Context<Self>) {
        self.editor_pack = None;
        self.editing_sticker = None;
        self.set_selected_emojis(Vec::new(), cx);
        self.set_selected_media(None, cx);
        self.shows_system_emoji_picker = false;
    }

    fn clear_all(&mut self, cx: &mut Context<Self>) {
        let owned_temporary_urls =
            self.active_submission.as_ref().map(|submission| submission.owned_temporary_urls.clone()).unwrap_or_default();
        self.set_route(Route::Idle, cx);
        self.clear_editor(cx);
        self.active_submission = None;
        self.error_text = None;
        cx.notify();
        Self::remove_owned_temporary_files(owned_temporary_urls, cx);
    }

    fn remove_owned_temporary_files(urls: HashSet<Url>, cx: &mut Context<Self>) {
        if urls.is_empty() {
            return;
        }
        cx.background_spawn(async move {
            for path in urls.iter().filter(|url| url.is_file_url()).filter_map(Url::to_file_path) {
                let _ = std::fs::remove_file(path);
            }
        })
        .detach();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.dismiss(cx);
    }
}

#[derive(IntoElement)]
pub struct StickerPackAddCell {
    id: ElementId,
    kind: PackKind,
    size: f32,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl StickerPackAddCell {
    pub fn new(
        id: impl Into<ElementId>,
        kind: PackKind,
        size: f32,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), kind, size, action: Rc::new(action) }
    }
}

impl RenderOnce for StickerPackAddCell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let visual_size = (self.size - 5.0).max(1.0);
        let corner_radius = (visual_size * 0.22).min(15.0);
        let shows_title = visual_size >= 58.0;
        let title: SharedString =
            tr(if self.kind == PackKind::Emoji { "emoji_pack_add_emoji" } else { "sticker_pack_add_sticker" }).into();

        let tile = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(if shows_title { 3.0 } else { 0.0 }))
            .size(px(visual_size))
            .rounded(px(corner_radius))
            .bg(theme.bg._200.opacity(0.8))
            .children(IconUi::sized("plus", px((visual_size * 0.42).min(32.0)), theme.text.primary, false))
            .when(shows_title, |tile| {
                tile.child(
                    MainText::new(title.clone())
                        .px((visual_size * 0.15).min(11.0))
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::NORMAL)
                        .color(theme.text.secondary)
                        .number_of_lines(2)
                        .minimum_scale_factor(0.78),
                )
            })
            .liquid_chrome_rounded_rectangle_with(corner_radius, 0.10, None, true);

        let action = self.action;
        div().id((self.id.clone(), "hint")).tooltip_label(title).child(
            plain(self.id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(self.size))
                .on_press(move |window, cx| action(window, cx))
                .child(tile),
        )
    }
}

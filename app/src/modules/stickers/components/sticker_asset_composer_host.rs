use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, ready};
use gpui::{
    AnyElement, App, AsyncApp, Context, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, Render,
    RenderOnce, Rgba, SharedString, Subscription, Window, anchored, black, deferred, relative,
};
use rise_canon::{RiseGuideAnchor, RiseLimits};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::video_export::probe;
use rise_platform::haptics;
use rise_platform::uniform_type::UTType;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::window_cover::WindowCoverFrame;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::sai_outgoing_media_hooks::{OutgoingMediaIsCancelled, OutgoingMediaProgress};
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerFloorKey;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierRetryOptions};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::ChatInputStickerAsset;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::messages::rise_message_media_upload_pipeline::{
    RiseDeferredMediaSource, RiseDeferredPhotoImageSource, RiseDeferredPhotoVideoSource,
};
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerItemKind, MediaPickerResult,
};
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::stickers::components::sticker_asset_composer_emoji_hooks::sticker_asset_composer_system_emoji_tab;
use crate::modules::stickers::components::sticker_asset_composer_hooks::{
    StickerAssetComposerHooks, StickerAssetPickerComplete, StickerAssetPickerDismissed,
};
use crate::modules::stickers::components::sticker_pack_add_cell::{ActiveSubmission, StickerAssetComposerStore};
use crate::modules::stickers::engine::rise_sticker_models::{
    RiseStickerMutationID, RiseStickerMutationState, RiseStickerPerformancePolicy, StickerMediaUploadDraft,
};
use crate::modules::stickers::engine::rise_sticker_repository::RiseStickerRepository;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    PackKind, StickerAssetType, StickerAssetTypeInfer, StickerItem, StickerPack,
};
use crate::modules::stickers::stores::stickers::stickers_guide_sandbox_hooks::stickers_guide_sandbox_hooks;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

const EMOJI_EXPANDED_SLOT_HEIGHT: f32 = 390.0;
const EMOJI_CHIP_ROW_HEIGHT: f32 = 46.0;
const ENTER_KEY: &str = "enter";
const SCRIM_OPACITY: f32 = 0.5;
const DISMISS_POLL_MS: u64 = 50;
const PANEL_CORNER_RADIUS: f32 = 34.0;
const PANEL_MAX_WIDTH: f32 = 480.0;
const PANEL_MAX_HEIGHT_FRACTION: f32 = 0.88;
const PANEL_TOP_PADDING: f32 = 16.0;

pub struct StickerAssetComposerHost {
    store: Entity<StickerAssetComposerStore>,
    services: Entity<StickersServicesStore>,
    scrim: Presence<()>,
    panel: Presence<()>,
    emoji_slot_height: Animated<f32>,
    focus_handle: FocusHandle,
    previous_error_text: Option<String>,
    previous_mutation_error: Option<String>,
    previous_mutation_state: Option<RiseStickerMutationState>,
    focused_presentation: bool,
    drawn_busy: bool,
    previous_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl StickerAssetComposerHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = StickerAssetComposerStore::shared(cx);
        let services = StickersServicesStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&store, |host, _, cx| host.stores_changed(cx)),
            cx.observe(&services, |host, _, cx| host.stores_changed(cx)),
        ];
        let mut host = Self {
            store,
            services,
            scrim: Presence::new(None).with_transition(Transition::opacity()),
            panel: Presence::new(None).with_transition(Transition::opacity().combined(Transition::scale(0.96))),
            emoji_slot_height: Animated::new(0.0),
            focus_handle: cx.focus_handle(),
            previous_error_text: None,
            previous_mutation_error: None,
            previous_mutation_state: None,
            focused_presentation: false,
            drawn_busy: false,
            previous_focus: None,
            _subscriptions: subscriptions,
        };
        host.previous_mutation_state = host.active_mutation_state(cx);
        host.previous_mutation_error = host.active_mutation_error(cx);
        host
    }

    /// nothing mounted and no focus to hand back,
    /// so the root may cache the host and it renders only on its own notify
    pub fn is_idle(&self, cx: &App) -> bool {
        let store = self.store.read(cx);
        !store.mounts_composer() && !store.mounts_media_picker() && !self.focused_presentation
    }

    fn active_mutation_state(&self, cx: &App) -> Option<RiseStickerMutationState> {
        let store = self.store.read(cx);
        let pack_id = store.active_pack_id()?;
        let mutation = match store.active_sticker_id() {
            Some(sticker_id) => RiseStickerMutationID::EditSticker { sticker_id: sticker_id.to_owned() },
            None => RiseStickerMutationID::AddSticker { pack_id: pack_id.to_owned() },
        };
        self.services.read(cx).mutation_state(&mutation)
    }

    fn active_mutation_error(&self, cx: &App) -> Option<String> {
        let store = self.store.read(cx);
        let pack_id = store.active_pack_id()?;
        match store.active_sticker_id() {
            Some(sticker_id) => self.services.read(cx).sticker_edit_error(sticker_id),
            None => self.services.read(cx).sticker_upload_error(pack_id),
        }
    }

    /// runs when either store changes
    fn stores_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_presentation(cx);
        let store = self.store.clone();

        let error_text = store.read(cx).error_text.clone();
        if error_text != self.previous_error_text {
            self.previous_error_text = error_text.clone();
            let failed_request_id = store.read(cx).failed_request_id().map(str::to_owned);
            if let (Some(text), Some(request_id)) = (error_text, failed_request_id) {
                Self::send_retry_toast(text, request_id, cx);
            }
        }

        let mutation_error = self.active_mutation_error(cx);
        if mutation_error != self.previous_mutation_error {
            self.previous_mutation_error = mutation_error.clone();
            if let Some(error) = mutation_error.clone().filter(|error| !error.is_empty()) {
                store.update(cx, |store, cx| store.show_error(error, cx));
            }
        }

        let mutation_state = self.active_mutation_state(cx);
        if mutation_state != self.previous_mutation_state {
            let previous = std::mem::replace(&mut self.previous_mutation_state, mutation_state);
            if previous == Some(RiseStickerMutationState::Submitting)
                && mutation_state.is_none()
                && mutation_error.is_none()
                && let Some(request_id) = store
                    .read(cx)
                    .active_submission()
                    .filter(|submission| submission.upload_dispatched)
                    .map(|submission| submission.request_id.clone())
            {
                store.update(cx, |store, cx| store.complete_upload(&request_id, cx));
            }
        }
        if self.drawn_busy || !self.is_idle(cx) {
            cx.notify();
        }
    }

    fn finish_dismissal_after_exit(&mut self, cx: &mut Context<Self>) {
        let store = self.store.downgrade();
        cx.spawn(async move |host, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(DISMISS_POLL_MS)).await;
                let Ok(leaving) = host.update(cx, |host, _| host.panel.is_running(RiseClock::monotonic())) else {
                    return;
                };
                if !leaving {
                    break;
                }
            }
            let _ = store.update(cx, |store, cx| store.composer_did_dismiss(cx));
        })
        .detach();
    }

    fn sync_presentation(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let presented = store.is_composer_presented();
        let target = if store.shows_system_emoji_picker() {
            EMOJI_EXPANDED_SLOT_HEIGHT
        } else if store.selected_emojis().is_empty() {
            0.0
        } else {
            EMOJI_CHIP_ROW_HEIGHT
        };
        let now = RiseClock::monotonic();
        if presented != self.panel.is_present() {
            tracing::warn!(target: "riseonly::X1603", presented, route = ?store.route(), "composer host presence");
            if presented {
                self.emoji_slot_height.jump(target);
            }
            let animation = spring(0.36, 0.88);
            self.scrim.set_in(presented.then_some(()), animation, now, cx);
            self.panel.set_in(presented.then_some(()), animation, now, cx);
            if !presented {
                self.finish_dismissal_after_exit(cx);
            }
        }
        if !presented {
            self.emoji_slot_height.jump(target);
        } else if *self.emoji_slot_height.target() != target {
            self.emoji_slot_height.set_in(target, spring(0.34, 0.86), now, cx);
        }
    }

    fn send_retry_toast(text: String, request_id: String, cx: &mut App) {
        let expired_request_id = request_id.clone();
        Notifier::send_retry(
            text,
            NotifierRetryOptions::default(),
            move |cx| {
                StickerAssetComposerStore::shared(cx)
                    .update(cx, |store, cx| store.dismiss_failed(&expired_request_id, cx));
            },
            move |cx| {
                let store = StickerAssetComposerStore::shared(cx);
                if store.read(cx).failed_request_id() != Some(request_id.as_str()) {
                    return;
                }
                Self::retry_submission(cx);
            },
            cx,
        );
    }

    fn composer_content(&self, pack: &StickerPack, theme: &AppTheme, slot_height: f32, cx: &App) -> Div {
        let store = self.store.clone();
        let is_editing = store.read(cx).is_editing();
        let title = tr(match (is_editing, pack.pack_kind == PackKind::Emoji) {
            (true, true) => "emoji_pack_edit_emoji_title",
            (true, false) => "sticker_pack_edit_sticker_title",
            (false, true) => "emoji_pack_asset_composer_title",
            (false, false) => "sticker_pack_asset_composer_title",
        });
        let close_store = store.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .px(px(16.0))
            .pt(px(PANEL_TOP_PADDING))
            .pb(px(92.0))
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        MainText::new(title).px(17.0).tac(TextAlignT::Center).font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(div().absolute().right_0().child(
                        PageHeaderIconButton::new("sticker-composer-close", "xmark", move |_, cx| {
                            close_store.update(cx, |store, cx| store.set_composer_presented(false, cx));
                        })
                        .font_size(14.0),
                    )),
            )
            .child(
                GroupedBtns::new("sticker-composer-rows", self.composer_rows(pack, theme, slot_height, cx))
                    .horizontal_padding(0.0),
            )
    }

    fn composer_rows(
        &self,
        pack: &StickerPack,
        theme: &AppTheme,
        slot_height: f32,
        cx: &App,
    ) -> Vec<GroupedBtn> {
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        let is_sticker_pack = pack.pack_kind == PackKind::Sticker;
        let is_emoji_pack = pack.pack_kind == PackKind::Emoji;

        let emoji_store = store_entity.clone();
        let mut emoji_row = GroupedBtn::new("sticker-composer-emoji", tr("sticker_pack_choose_associated_emoji"))
            .icon("face.smiling")
            .icon_background(rgb_color(0.98, 0.60, 0.16))
            .subtitle(tr("sticker_pack_choose_associated_emoji_hint"))
            .guide_anchor(is_sticker_pack.then_some(RiseGuideAnchor::StickerComposerEmoji))
            .show_chevron_when_trailing_slot(true)
            .callback(move |_, cx| {
                haptics::selection_changed();
                emoji_store.update(cx, |store, cx| store.toggle_system_emoji_picker(cx));
            });
        if !store.selected_emojis().is_empty() {
            emoji_row = emoji_row.trailing_slot(
                MainText::new(format!(
                    "{}/{}",
                    store.selected_emojis().len(),
                    RiseStickerPerformancePolicy::associated_emojis_limit()
                ))
                .px(15.0)
                .color(theme.text.secondary),
            );
        }
        if let Some(slot) = self.merged_emoji_slot(theme, slot_height, cx) {
            emoji_row = emoji_row.expanded_slot(slot);
        }

        let media_store = store_entity.clone();
        let mut media_row = GroupedBtn::new(
            "sticker-composer-media",
            tr(if is_emoji_pack { "emoji_pack_choose_media" } else { "sticker_pack_choose_media" }),
        )
        .icon(if is_emoji_pack { "face.smiling.inverse" } else { "photo.on.rectangle.angled" })
        .icon_background(rgb_color(0.20, 0.52, 0.98))
        .subtitle(tr("sticker_pack_choose_media_hint"))
        .guide_anchor(is_sticker_pack.then_some(RiseGuideAnchor::StickerComposerMedia))
        .show_chevron_when_trailing_slot(true)
        .callback(move |_, cx| {
            haptics::selection_changed();
            media_store.update(cx, |store, cx| store.open_media_picker(cx));
        });
        if let Some(item) = store.selected_media() {
            media_row =
                media_row.trailing_slot(div().size(px(30.0)).child(StickerComposerMediaPreview::new(item.clone())));
        } else if let Some(sticker) = store.target_sticker() {
            media_row = media_row
                .trailing_slot(div().size(px(30.0)).child(ChatInputStickerAsset::new(sticker.clone(), 30.0).is_playing(true)));
        }

        vec![emoji_row, media_row]
    }

    fn merged_emoji_slot(&self, theme: &AppTheme, slot_height: f32, cx: &App) -> Option<Div> {
        let store = self.store.read(cx);
        let has_chips = !store.selected_emojis().is_empty();
        let shows_picker = store.shows_system_emoji_picker();
        if slot_height < 0.5 && !has_chips && !shows_picker {
            return None;
        }
        let chips_height = if has_chips { EMOJI_CHIP_ROW_HEIGHT } else { 0.0 };
        let mounts_picker = shows_picker || slot_height > chips_height + 0.5;
        let chips = has_chips.then(|| self.selected_emoji_chips(theme, cx));
        let picker = mounts_picker.then(|| self.system_emoji_picker(theme, chips_height, cx));
        Some(div().flex().flex_col().w_full().h(px(slot_height)).overflow_hidden().children(chips).children(picker))
    }

    fn selected_emoji_chips(&self, theme: &AppTheme, cx: &App) -> Div {
        let store_entity = self.store.clone();
        let label: SharedString = tr("sticker_pack_remove_associated_emoji").into();
        let chips = store_entity.read(cx).selected_emojis().iter().map(|emoji| {
            let remove_store = store_entity.clone();
            let removed = emoji.clone();
            div()
                .id(ElementId::Name(format!("sticker-composer-chip-hint-{emoji}").into()))
                .tooltip_label(label.clone())
                .child(
                    plain(ElementId::Name(format!("sticker-composer-chip-{emoji}").into()))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(9.0))
                        .py(px(6.0))
                        .rounded_full()
                        .bg(theme.bg._300)
                        .on_press(move |_, cx| {
                            haptics::selection_changed();
                            remove_store.update(cx, |store, cx| store.remove_emoji(&removed, cx));
                        })
                        .child(div().text_size(px(20.0)).child(SharedString::from(emoji.clone())))
                        .children(IconUi::sized("xmark", px(9.0), theme.text.secondary, false)),
                )
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .px(px(14.0))
            .w_full()
            .h(px(EMOJI_CHIP_ROW_HEIGHT))
            .children(chips.collect::<Vec<_>>())
    }

    fn system_emoji_picker(&self, theme: &AppTheme, chips_height: f32, cx: &App) -> Div {
        let store = self.store.clone();
        let is_active = store.read(cx).shows_system_emoji_picker();
        div().flex_none().w_full().h(px(EMOJI_EXPANDED_SLOT_HEIGHT - chips_height)).bg(theme.bg._200).children(
            sticker_asset_composer_system_emoji_tab(
                "sticker-composer-emoji-tab",
                move |emoji: String, _: &mut Window, cx: &mut App| {
                    store.update(cx, |store, cx| store.select_emoji(&emoji, cx));
                },
                |_: StickerItem, _: &mut Window, _: &mut App| {},
                |_: &mut Window, _: &mut App| {},
                is_active,
            ),
        )
    }

    fn submit_button(&self, pack: &StickerPack, theme: &AppTheme, cx: &App) -> AnyElement {
        let store = self.store.read(cx);
        let can_submit = store.can_submit();
        let title = if store.is_editing() {
            tr("save")
        } else {
            tr(if pack.pack_kind == PackKind::Emoji { "emoji_pack_add_emoji" } else { "sticker_pack_add_sticker" })
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                LiquidGlassButton::new("sticker-composer-submit", LiquidGlassButtonShape::Capsule)
                    .horizontal_padding(22.0)
                    .vertical_padding(13.0)
                    .tint(Some(if can_submit { theme.primary._100 } else { theme.bg._300 }))
                    .haptic_on_press(can_submit)
                    .disabled(!can_submit)
                    .accessibility_label(Some(title.clone()))
                    .on_press(|_, cx| Self::begin_submission(cx))
                    .child(
                        div().w_full().flex().justify_center().child(
                            MainText::new(title)
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(if can_submit { theme.content_on(theme.primary._100) } else { theme.text.secondary }),
                        ),
                    ),
            )
            .guide_anchor((pack.pack_kind == PackKind::Sticker).then_some(RiseGuideAnchor::StickerComposerSubmit))
            .into_any_element()
    }

    fn begin_submission(cx: &mut App) {
        let Some(submission) =
            StickerAssetComposerStore::shared(cx).update(cx, |store, cx| store.begin_submission(cx))
        else {
            return;
        };
        cx.spawn(async move |cx: &mut AsyncApp| Self::run_submission(submission, cx).await).detach();
    }

    fn retry_submission(cx: &mut App) {
        let Some(submission) =
            StickerAssetComposerStore::shared(cx).update(cx, |store, cx| store.retry_submission(cx))
        else {
            return;
        };
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| match &submission.original_sticker {
            Some(sticker) => interactions.clear_sticker_edit_error(&sticker.id, cx),
            None => interactions.clear_sticker_upload_error(&submission.pack.id, cx),
        });
        cx.spawn(async move |cx: &mut AsyncApp| Self::run_submission(submission, cx).await).detach();
    }

    fn is_active(request_id: &str, pack_id: &str, cx: &AsyncApp) -> bool {
        cx.update(|cx| {
            let store = StickerAssetComposerStore::shared(cx);
            store.read(cx).is_active(request_id, pack_id)
        })
    }

    async fn on_account_runtime<Value, Work>(work: Work) -> anyhow::Result<Value>
    where
        Value: Send + 'static,
        Work: Future<Output = anyhow::Result<Value>> + Send + 'static,
    {
        match RiseAccountRuntime::current().spawn(work).await {
            Ok(result) => result,
            Err(_) => Err(CancellationError.into()),
        }
    }

    async fn cancel_on_engine(request_id: &str, pack_id: &str, cx: &mut AsyncApp) {
        let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
            return;
        };
        let stickers = Arc::clone(&domain.stickers);
        let (request_id, pack_id) = (request_id.to_owned(), pack_id.to_owned());
        let _ = RiseAccountRuntime::current()
            .spawn(async move { stickers.cancel_sticker_upload(&request_id, &pack_id).await })
            .await;
    }

    async fn run_submission(submission: ActiveSubmission, cx: &mut AsyncApp) {
        let request_id = submission.request_id.clone();
        let pack_id = submission.pack.id.clone();
        if !Self::is_active(&request_id, &pack_id, cx) {
            return;
        }
        let owns_pack = cx.update(|cx| (stickers_guide_sandbox_hooks().owns_sticker_pack)(&pack_id, cx));
        if owns_pack && let Some(item) = &submission.item {
            let source = item
                .video_payload
                .as_ref()
                .and_then(|video| video.poster_url.clone())
                .unwrap_or_else(|| item.url.clone());
            cx.update(|cx| {
                (stickers_guide_sandbox_hooks().add_practice_sticker)(&source, &submission.emojis, cx);
                StickerAssetComposerStore::shared(cx).update(cx, |store, cx| store.complete_upload(&request_id, cx));
            });
            return;
        }

        let Err(error) = Self::prepare_and_submit(&submission, cx).await else {
            return;
        };
        if error.is::<CancellationError>() {
            if !Self::is_active(&request_id, &pack_id, cx) {
                return;
            }
            Self::cancel_on_engine(&request_id, &pack_id, cx).await;
            if !Self::is_active(&request_id, &pack_id, cx) {
                return;
            }
            Self::show_error(tr("sticker_pack_upload_failed"), cx);
            return;
        }
        Self::cancel_on_engine(&request_id, &pack_id, cx).await;
        if !Self::is_active(&request_id, &pack_id, cx) {
            return;
        }
        let text = match error.downcast_ref::<StickerMediaDraftError>() {
            Some(draft_error) => draft_error.localized_description(),
            None => RiseStickerRepository::failure_text(&error),
        };
        Self::show_error(text, cx);
    }

    fn show_error(text: String, cx: &AsyncApp) {
        cx.update(|cx| {
            StickerAssetComposerStore::shared(cx).update(cx, |store, cx| store.show_error(text, cx));
        });
    }

    async fn prepare_and_submit(submission: &ActiveSubmission, cx: &mut AsyncApp) -> anyhow::Result<()> {
        let request_id = submission.request_id.clone();
        let pack_id = submission.pack.id.clone();
        let sticker_id = submission.original_sticker.as_ref().map(|sticker| sticker.id.clone());
        let domain = RiseConversationActionExecutor::require_domain(cx).await?;
        let stickers = Arc::clone(&domain.stickers);

        if let Some(sticker_id) = sticker_id.clone() {
            let (stickers, pack_id, request_id) = (Arc::clone(&stickers), pack_id.clone(), request_id.clone());
            Self::on_account_runtime(async move {
                stickers.stage_sticker_edit_preparation(&pack_id, &sticker_id, &request_id).await
            })
            .await?;
        } else if let Some(item) = &submission.item {
            let preview = StickerMediaUploadDraftBuilder::preview(item);
            let emojis = submission.emojis.clone();
            let (stickers, pack_id, request_id) = (Arc::clone(&stickers), pack_id.clone(), request_id.clone());
            Self::on_account_runtime(async move {
                stickers
                    .stage_sticker_preparation(
                        &pack_id,
                        &request_id,
                        &preview.file_url,
                        preview.thumbnail_url,
                        preview.width,
                        preview.height,
                        preview.duration_ms,
                        emojis,
                        preview.asset_type,
                        preview.mime_type,
                    )
                    .await
            })
            .await?;
        } else {
            return Err(StickerMediaDraftError::Unsupported.into());
        }
        if !Self::is_active(&request_id, &pack_id, cx) {
            Self::cancel_on_engine(&request_id, &pack_id, cx).await;
            return Ok(());
        }

        let prepared_draft = cx.update(|cx| {
            let store = StickerAssetComposerStore::shared(cx);
            store.read(cx).active_submission().and_then(|active| active.draft.clone())
        });
        let draft = if let Some(prepared) = prepared_draft {
            Some(prepared)
        } else if let Some(item) = submission.item.clone() {
            let on_progress = Self::preparation_progress(&stickers, &request_id, &pack_id, sticker_id.clone());
            let is_cancelled = Self::preparation_is_cancelled(&request_id, &pack_id);
            let prepared =
                Self::on_account_runtime(StickerMediaUploadDraftBuilder::make(item, on_progress, is_cancelled)).await?;
            if !Self::is_active(&request_id, &pack_id, cx) {
                let _ = Self::on_account_runtime(async move {
                    StickerMediaUploadDraftBuilder::discard(&prepared);
                    anyhow::Ok(())
                })
                .await;
                Self::cancel_on_engine(&request_id, &pack_id, cx).await;
                return Ok(());
            }
            let draft = prepared.draft;
            let (attached, owned) = (draft.clone(), prepared.owned_temporary_urls);
            let attach_id = request_id.clone();
            cx.update(|cx| {
                StickerAssetComposerStore::shared(cx)
                    .update(cx, |store, cx| store.attach_prepared_draft(attached, owned, &attach_id, cx));
            });
            Some(draft)
        } else {
            None
        };

        let local_url = draft.as_ref().map(|draft| draft.local_url.clone());
        let total_bytes = Self::on_account_runtime(async move {
            anyhow::Ok(local_url.map_or(0, |url| StickerMediaUploadDraftBuilder::file_size(&url)))
        })
        .await?;
        {
            let (stickers, pack_id, request_id, sticker_id) =
                (Arc::clone(&stickers), pack_id.clone(), request_id.clone(), sticker_id.clone());
            let _ = RiseAccountRuntime::current()
                .spawn(async move {
                    stickers
                        .update_sticker_preparation_progress(
                            &request_id,
                            &pack_id,
                            sticker_id.as_deref(),
                            1.0,
                            total_bytes,
                        )
                        .await;
                })
                .await;
        }
        cx.update(|cx| Self::submit_prepared(draft, submission, cx));
        Ok(())
    }

    fn preparation_progress(
        stickers: &Arc<RiseStickerRepository>,
        request_id: &str,
        pack_id: &str,
        sticker_id: Option<String>,
    ) -> OutgoingMediaProgress {
        let stickers = Arc::clone(stickers);
        let (request_id, pack_id) = (request_id.to_owned(), pack_id.to_owned());
        Arc::new(move |fraction: f64| {
            let (stickers, request_id, pack_id, sticker_id) =
                (Arc::clone(&stickers), request_id.clone(), pack_id.clone(), sticker_id.clone());
            RiseAccountRuntime::current().spawn(async move {
                stickers
                    .update_sticker_preparation_progress(&request_id, &pack_id, sticker_id.as_deref(), fraction, 0)
                    .await;
            });
        })
    }

    fn preparation_is_cancelled(request_id: &str, pack_id: &str) -> OutgoingMediaIsCancelled {
        let (request_id, pack_id) = (request_id.to_owned(), pack_id.to_owned());
        Arc::new(move || -> BoxFuture<'static, bool> {
            let Some(hop) = RiseEngineMainHop::installed() else {
                return ready(true).boxed();
            };
            let (request_id, pack_id) = (request_id.clone(), pack_id.clone());
            hop.run(move |cx: AsyncApp| {
                async move { Ok::<bool, anyhow::Error>(!Self::is_active(&request_id, &pack_id, &cx)) }.boxed_local()
            })
            .map(|answer| answer.unwrap_or(true))
            .boxed()
        })
    }

    fn submit_prepared(draft: Option<StickerMediaUploadDraft>, submission: &ActiveSubmission, cx: &mut App) {
        let store = StickerAssetComposerStore::shared(cx);
        let request_id = submission.request_id.clone();
        if !store.read(cx).is_active(&request_id, &submission.pack.id) {
            return;
        }
        let auth = AuthServicesStore::shared(cx);
        let user_id = auth.read(cx).effective_user_id_for_wire().unwrap_or_default();
        if user_id.is_empty() {
            store.update(cx, |store, cx| {
                store.show_error(StickerMediaDraftError::Unsupported.localized_description(), cx);
            });
            return;
        }

        store.update(cx, |store, cx| store.mark_upload_dispatched(&request_id, cx));
        let interactions = StickersInteractionsStore::shared(cx);
        let completed_id = request_id.clone();
        let complete = move |_: StickerItem, cx: &mut App| {
            StickerAssetComposerStore::shared(cx).update(cx, |store, cx| store.complete_upload(&completed_id, cx));
        };
        if let Some(original_sticker) = &submission.original_sticker {
            interactions.update(cx, |interactions, cx| {
                interactions.edit_sticker(
                    &user_id,
                    &submission.pack.id,
                    &original_sticker.id,
                    draft,
                    submission.emojis.clone(),
                    None,
                    None,
                    &request_id,
                    complete,
                    cx,
                );
            });
        } else if let Some(draft) = draft {
            interactions.update(cx, |interactions, cx| {
                interactions.add_sticker(
                    &user_id,
                    &submission.pack.id,
                    draft,
                    submission.emojis.clone(),
                    None,
                    None,
                    &request_id,
                    complete,
                    cx,
                );
            });
        } else {
            store.update(cx, |store, cx| {
                store.show_error(StickerMediaDraftError::Unsupported.localized_description(), cx);
            });
        }
    }
}

impl StickerAssetComposerHost {
    pub const LAYER_PRIORITY: usize = 1300;
}

impl Render for StickerAssetComposerHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        let mounts_composer = store.mounts_composer();
        let mounts_media_picker = store.mounts_media_picker();
        let presented = store.is_composer_presented();
        let route = store.route();
        let pack = store.editor_pack().cloned();
        self.drawn_busy = mounts_composer || mounts_media_picker || self.focused_presentation;
        if !presented && self.focused_presentation {
            self.focused_presentation = false;
            match self.previous_focus.take() {
                Some(previous) => window.focus(&previous, cx),
                None => window.blur(),
            }
        }
        if !mounts_composer && !mounts_media_picker {
            return div().into_any_element();
        }

        if mounts_composer && !self.panel.is_present() && !self.panel.is_running(now) {
            tracing::warn!(target: "riseonly::X1603", ?route, "composer host did dismiss");
            let dismissed = store_entity.clone();
            cx.defer(move |cx| dismissed.update(cx, |store, cx| store.composer_did_dismiss(cx)));
        }
        if presented && !self.focused_presentation {
            self.focused_presentation = true;
            self.previous_focus = window.focused(cx);
            window.focus(&self.focus_handle, cx);
        }

        let theme = rise_ui::theme(cx).clone();
        let scrim_store = store_entity.clone();
        let scrim = self.scrim.render_transitioned(now, window, |_| {
            div().id("sticker-composer-scrim").absolute().inset_0().bg(black().opacity(SCRIM_OPACITY)).when(
                presented,
                |scrim| {
                    scrim.occlude().on_click(move |_, _, cx| {
                        scrim_store.update(cx, |store, cx| store.set_composer_presented(false, cx));
                    })
                },
            )
        });

        let panel = match pack.as_ref().filter(|_| mounts_composer) {
            Some(pack) => {
                let slot_height = self.emoji_slot_height.frame_value(now, window);
                let content = self.composer_content(pack, &theme, slot_height, cx);
                let submit = self.submit_button(pack, &theme, cx);
                let background = theme.bg._100;
                self.panel.render_transitioned(now, window, move |_| {
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        div()
                            .id("sticker-composer-panel")
                            .occlude()
                            .relative()
                            .flex()
                            .flex_col()
                            .w_full()
                            .max_w(px(PANEL_MAX_WIDTH))
                            .max_h(relative(PANEL_MAX_HEIGHT_FRACTION))
                            .overflow_hidden()
                            .bg(background)
                            .rounded(px(PANEL_CORNER_RADIUS))
                            .child(
                                div()
                                    .id("sticker-composer-scroll")
                                    .flex_shrink(1.0)
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(content),
                            )
                            .child(div().absolute().bottom_0().left_0().right_0().px(px(14.0)).pt(px(8.0)).pb(px(12.0)).child(submit)),
                    )
                })
            }
            None => None,
        };

        let media_picker = pack.as_ref().filter(|_| mounts_media_picker).map(|pack| {
            let (is_presented_store, set_store) = (store_entity.clone(), store_entity.clone());
            let is_presented = Binding::new(
                move |cx: &App| is_presented_store.read(cx).is_media_picker_presented(),
                move |open: bool, _: &mut Window, cx: &mut App| {
                    set_store.update(cx, |store, cx| store.set_media_picker_presented(open, cx));
                },
            );
            let (complete_store, dismissed_store) = (store_entity.clone(), store_entity.clone());
            let on_complete: StickerAssetPickerComplete =
                Rc::new(move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                    complete_store.update(cx, |store, cx| store.receive_media_picker_result(result, cx));
                });
            let on_dismissed: StickerAssetPickerDismissed = Rc::new(move |_: &mut Window, cx: &mut App| {
                dismissed_store.update(cx, |store, cx| store.media_picker_did_dismiss(cx));
            });
            StickerAssetComposerHooks::media_picker_sheet(
                "sticker-composer-media-picker".into(),
                is_presented,
                pack.pack_kind,
                ThemeStore::shared(cx),
                on_complete,
                on_dismissed,
            )
        });

        let focus_handle = self.focus_handle.clone();
        let key_store = store_entity;
        let cover = WindowCoverFrame::resolve(window, cx);
        let host = div()
            .id("sticker-composer-host")
            .relative()
            .w(cover.size.width)
            .h(cover.size.height)
            .when(presented, |host| {
                host.track_focus(&self.focus_handle)
                    .layer_keys(LayerCancel::dismiss(move |_, cx| {
                        key_store.update(cx, |store, cx| store.set_composer_presented(false, cx));
                    }))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == ENTER_KEY
                            && !event.keystroke.modifiers.modified()
                            && focus_handle.is_focused(window)
                        {
                            cx.stop_propagation();
                            Self::begin_submission(cx);
                        }
                    })
            })
            .children(scrim)
            .children(panel)
            .children(media_picker);
        let layer: AnyElement = anchored()
            .position(cover.origin)
            .child(env_scope::<SheetLayerFloorKey>(Self::LAYER_PRIORITY).child(host))
            .into_any_element();
        div().size(px(0.0)).child(deferred(layer).with_priority(Self::LAYER_PRIORITY)).into_any_element()
    }
}

fn rgb_color(r: f32, g: f32, b: f32) -> Hsla {
    Hsla::from(Rgba { r, g, b, a: 1.0 })
}

#[derive(IntoElement)]
struct StickerComposerMediaPreview {
    item: MediaPickerItem,
}

impl StickerComposerMediaPreview {
    fn new(item: MediaPickerItem) -> Self {
        Self { item }
    }
}

impl RenderOnce for StickerComposerMediaPreview {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let preview = StickerMediaUploadDraftBuilder::preview(&self.item);
        div().rounded(px(7.0)).overflow_hidden().bg(black().opacity(0.10)).child(
            ChatInputStickerAsset::new(
                StickerItem {
                    id: format!("composer-preview-{}", self.item.id),
                    pack_id: String::new(),
                    file_url: preview.file_url,
                    thumbnail_url: preview.thumbnail_url,
                    width: preview.width.unwrap_or(0),
                    height: preview.height.unwrap_or(0),
                    duration_ms: preview.duration_ms,
                    position: 0,
                    created_at_ms: 0,
                    associated_emojis: Vec::new(),
                    asset_type: preview.asset_type,
                    mime_type: preview.mime_type,
                    codec: None,
                },
                30.0,
            )
            .is_playing(true),
        )
    }
}

struct Prepared {
    draft: StickerMediaUploadDraft,
    owned_temporary_urls: HashSet<Url>,
}

struct Preview {
    file_url: String,
    thumbnail_url: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    duration_ms: Option<i64>,
    asset_type: StickerAssetType,
    mime_type: Option<String>,
}

struct OwnedTemporaryFiles {
    urls: HashSet<Url>,
    did_succeed: bool,
}

impl Drop for OwnedTemporaryFiles {
    fn drop(&mut self) {
        if !self.did_succeed {
            StickerMediaUploadDraftBuilder::remove_temporary_files(&self.urls);
        }
    }
}

const ALLOWED_EXTENSIONS: [&str; 14] = [
    "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "webm", "mp4", "mov", "m4v", "json", "lottie", "tgs",
];
const ROS_EXTENSION: &str = "ros";
const VIDEO_EXTENSIONS: [&str; 4] = ["webm", "mp4", "mov", "m4v"];

enum StickerMediaUploadDraftBuilder {}

impl StickerMediaUploadDraftBuilder {
    fn ros_maximum_bytes() -> i64 {
        RiseLimits::value("ros_sticker")
    }

    fn pixel_dimension(value: f32) -> i64 {
        value.round().max(1.0) as i64
    }

    fn duration_ms(seconds: f64) -> i64 {
        (seconds.max(0.0) * 1_000.0).ceil() as i64
    }

    fn preview(item: &MediaPickerItem) -> Preview {
        let source_url = item.video_payload.as_ref().map_or(&item.url, |video| &video.url);
        let file_extension = Self::resolved_extension(source_url, &item.suggested_file_name);
        let dimensions = Self::dimensions(item);
        Preview {
            file_url: source_url.absolute_string().to_owned(),
            thumbnail_url: item
                .video_payload
                .as_ref()
                .and_then(|video| video.poster_url.as_ref())
                .map(|poster| poster.absolute_string().to_owned()),
            width: dimensions.map(|size| Self::pixel_dimension(size.width)),
            height: dimensions.map(|size| Self::pixel_dimension(size.height)),
            duration_ms: item.video_payload.as_ref().map(|video| Self::duration_ms(video.duration)),
            asset_type: StickerAssetType::infer(None, &item.suggested_file_name),
            mime_type: Some(Self::mime_type(&file_extension)),
        }
    }

    fn dimensions(item: &MediaPickerItem) -> Option<gpui::Size<f32>> {
        item.image_payload
            .as_ref()
            .map(|image| image.pixel_size)
            .or_else(|| item.video_payload.as_ref().map(|video| video.pixel_size))
    }

    async fn make(
        item: MediaPickerItem,
        on_progress: OutgoingMediaProgress,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> anyhow::Result<Prepared> {
        if is_cancelled().await {
            return Err(CancellationError.into());
        }
        let mut owned = OwnedTemporaryFiles { urls: HashSet::new(), did_succeed: false };

        let has_deferred_image = item.deferred_photo_library_image_asset_identifier().is_some();
        let has_deferred_photo_video = item.deferred_photo_library_video_asset_identifier().is_some();
        let requires_deferred_video_processing =
            item.video_payload.as_ref().is_some_and(|video| video.is_processing_deferred);
        let materialization_weight = if has_deferred_photo_video {
            0.55
        } else if has_deferred_image {
            1.0
        } else {
            0.0
        };
        let materialization_progress: OutgoingMediaProgress = {
            let on_progress = Arc::clone(&on_progress);
            Arc::new(move |fraction: f64| on_progress(fraction * materialization_weight))
        };
        let materialized: Option<RiseDeferredMediaSource> = if has_deferred_image {
            RiseDeferredPhotoImageSource::materialize_if_needed(
                Some(&item),
                false,
                materialization_progress,
                Arc::clone(&is_cancelled),
            )
            .await?
        } else if has_deferred_photo_video {
            RiseDeferredPhotoVideoSource::materialize_if_needed(
                Some(&item),
                materialization_progress,
                Arc::clone(&is_cancelled),
            )
            .await?
        } else {
            None
        };

        let original_source_url = match &materialized {
            Some(source) => Url::from_file_path(&source.url).ok_or(StickerMediaDraftError::Unsupported)?,
            None => item.video_payload.as_ref().map_or_else(|| item.url.clone(), |video| video.url.clone()),
        };
        let mut source_url = original_source_url.clone();
        let generated_materialized_url = match materialized.as_ref().and_then(|source| source.generated_temporary_url.as_ref()) {
            Some(path) => Some(Url::from_file_path(path).ok_or(StickerMediaDraftError::Unsupported)?),
            None => None,
        };
        if let Some(generated) = &generated_materialized_url {
            owned.urls.insert(generated.standardized());
        }
        let mut compressed_mime_type = None;
        let mut compressed_width = None;
        let mut compressed_height = None;

        if requires_deferred_video_processing {
            let source_extension = Self::resolved_extension(&source_url, &item.suggested_file_name);
            let source_path = source_url.to_file_path().ok_or(StickerMediaDraftError::Unsupported)?;
            let mut local_file = RiseOutgoingMediaFile::new(
                source_path,
                item.suggested_file_name.clone(),
                Self::mime_type(&source_extension),
            );
            local_file.file_context = Some("sticker".to_owned());
            local_file.media_type = Some("video".to_owned());
            local_file.byte_count = None;
            let compression_progress: OutgoingMediaProgress = {
                let on_progress = Arc::clone(&on_progress);
                Arc::new(move |fraction: f64| {
                    on_progress(materialization_weight + fraction * (1.0 - materialization_weight));
                })
            };
            let compressed = OutgoingMediaCompressor::compress_if_needed(
                local_file,
                item.video_payload.clone(),
                MediaCompressionConfiguration::chat(),
                Some(compression_progress),
                Some(Arc::clone(&is_cancelled)),
            )
            .await?;
            compressed_mime_type = Some(compressed.content_type);
            compressed_width = compressed.pixel_width;
            compressed_height = compressed.pixel_height;
            source_url = compressed.url;
            if source_url.standardized() != original_source_url.standardized() {
                owned.urls.insert(source_url.standardized());
            }
            if let Some(generated) = &generated_materialized_url
                && generated.standardized() != source_url.standardized()
            {
                if let Some(path) = generated.to_file_path() {
                    let _ = std::fs::remove_file(path);
                }
                owned.urls.remove(&generated.standardized());
            }
        }

        let source_exists = source_url.is_file_url()
            && source_url.to_file_path().is_some_and(|path| path.exists());
        if !source_exists {
            return Err(StickerMediaDraftError::Unsupported.into());
        }

        let file_extension = Self::resolved_extension(&source_url, &item.suggested_file_name);
        if !ALLOWED_EXTENSIONS.contains(&file_extension.as_str()) && file_extension != ROS_EXTENSION {
            return Err(StickerMediaDraftError::Unsupported.into());
        }
        if file_extension == ROS_EXTENSION && Self::file_size(&source_url) > Self::ros_maximum_bytes() {
            return Err(StickerMediaDraftError::RosTooLarge.into());
        }

        let video_like = matches!(item.kind, MediaPickerItemKind::Video)
            || VIDEO_EXTENSIONS.contains(&file_extension.as_str());
        let mut duration_ms = item.video_payload.as_ref().map(|video| Self::duration_ms(video.duration));
        if video_like && duration_ms.is_none() {
            duration_ms = Self::video_duration_ms(&source_url, file_extension != "webm").await?;
        }
        if duration_ms.is_some_and(|duration| duration > RiseLimits::value("sticker_asset_video_duration")) {
            return Err(StickerMediaDraftError::VideoTooLong.into());
        }

        let dimensions = Self::dimensions(&item);
        let filename = Self::resolved_filename(&source_url, &item.suggested_file_name, &file_extension);
        let thumbnail = item
            .video_payload
            .as_ref()
            .and_then(|video| video.poster_url.clone())
            .filter(Url::is_file_url);
        on_progress(1.0);
        owned.did_succeed = true;
        Ok(Prepared {
            draft: StickerMediaUploadDraft::new(
                source_url,
                filename,
                compressed_mime_type.unwrap_or_else(|| Self::mime_type(&file_extension)),
                compressed_width.or(dimensions.map(|size| Self::pixel_dimension(size.width))),
                compressed_height.or(dimensions.map(|size| Self::pixel_dimension(size.height))),
                duration_ms,
                thumbnail,
            ),
            owned_temporary_urls: std::mem::take(&mut owned.urls),
        })
    }

    fn remove_temporary_files(urls: &HashSet<Url>) {
        for path in urls.iter().filter(|url| url.is_file_url()).filter_map(Url::to_file_path) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn discard(prepared: &Prepared) {
        Self::remove_temporary_files(&prepared.owned_temporary_urls);
    }

    fn file_size(url: &Url) -> i64 {
        url.to_file_path()
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
    }

    async fn video_duration_ms(url: &Url, requires_readable_duration: bool) -> anyhow::Result<Option<i64>> {
        let path = url.to_file_path().ok_or(StickerMediaDraftError::Unsupported)?;
        let probed = RiseAccountRuntime::current()
            .spawn_blocking(move || probe(&path))
            .await
            .map_err(|_| CancellationError)?;
        let seconds = probed.ok().map(|probed| probed.duration_seconds).filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
        match seconds {
            Some(seconds) => Ok(Some(Self::duration_ms(seconds))),
            None if requires_readable_duration => Err(StickerMediaDraftError::Unsupported.into()),
            None => Ok(None),
        }
    }

    fn resolved_extension(url: &Url, suggested_filename: &str) -> String {
        let direct = url.path_extension().to_lowercase();
        if !direct.is_empty() {
            return direct;
        }
        Path::new(suggested_filename)
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }

    fn resolved_filename(url: &Url, suggested_filename: &str, file_extension: &str) -> String {
        let last_path_component = url.last_path_component();
        let local = trimming_whitespaces_and_newlines(&last_path_component);
        if !local.is_empty() && Path::new(local).extension().is_some() {
            return local.to_owned();
        }
        let suggested = trimming_whitespaces_and_newlines(suggested_filename);
        if !suggested.is_empty() && Path::new(suggested).extension().is_some() {
            return suggested.to_owned();
        }
        format!("sticker-{}.{}", Uuid::new_v4().uuid_string(), file_extension)
    }

    fn mime_type(file_extension: &str) -> String {
        match file_extension {
            "webp" => "image/webp".to_owned(),
            "webm" => "video/webm".to_owned(),
            "json" => "application/json".to_owned(),
            "lottie" => "application/zip".to_owned(),
            "tgs" => "application/gzip".to_owned(),
            "ros" => "application/x-rosticker".to_owned(),
            _ => UTType::from_filename_extension(file_extension)
                .and_then(|uniform_type| uniform_type.preferred_mime_type())
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StickerMediaDraftError {
    Unsupported,
    VideoTooLong,
    RosTooLarge,
}

impl StickerMediaDraftError {
    fn localized_description(&self) -> String {
        match self {
            Self::Unsupported => tr("sticker_pack_media_unsupported"),
            Self::VideoTooLong => tr("sticker_pack_media_too_long"),
            Self::RosTooLarge => tr_args(
                "sticker_pack_ros_too_large",
                &[FormatArg::from(RiseFormat::file_size(StickerMediaUploadDraftBuilder::ros_maximum_bytes()).as_str())],
            ),
        }
    }
}

impl std::fmt::Display for StickerMediaDraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.localized_description())
    }
}

impl std::error::Error for StickerMediaDraftError {}

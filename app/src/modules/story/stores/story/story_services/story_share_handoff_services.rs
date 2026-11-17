use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, AppContext, Context};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use rise_platform::haptics::{NotificationKind, notification};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapDecoder;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerTrackSelection};
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorOutcome;
use crate::modules::media_picker::export::export_progress_view_state::ExportProgressViewState;
use crate::modules::media_picker::export::media_exporter::MediaExporter;
use crate::modules::media_picker::library::media_asset::{MediaAsset, MediaAssetMediaType};
use crate::modules::media_picker::selection::media_selection_state::{MediaSelectionEdit, MediaSelectionEditPayload};
use crate::modules::share_extension::shared::rise_share_container::RiseShareContainer;
use crate::modules::share_extension::shared::rise_share_models::RiseShareStoryMediaKind;
use crate::modules::story::stores::story::story_actions::story_actions::{StoryActionDoneHandler, StoryActionTextHandler};
use crate::modules::story::stores::story::story_actions::story_types::StoryAudience;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::story::stores::story::story_services::story_share_handoff_models::{
    StoryShareEditorDraft, StoryShareEditorDraftFile, StoryShareHandoffComposition, StoryShareHandoffContext,
    StoryShareHandoffDraft, StoryShareHandoffExport, StoryShareHandoffFinalization, StoryShareHandoffLoadError,
    StoryShareHandoffLoader, StoryShareHandoffPhase, StoryShareHandoffRecovery, StorySharePublishContext,
    StorySharePublishRequest,
};

impl StoryServicesStore {
    pub fn has_pending_shared_story_handoff(&self) -> bool {
        self.story_share_handoff_phase.active_session_id().is_some() || !self.pending_story_share_session_ids.is_empty()
    }

    pub fn discover_pending_shared_story_handoffs(&mut self, cx: &mut Context<Self>) {
        let auth = AuthServicesStore::shared(cx);
        let (is_authenticated, account_id) = {
            let auth = auth.read(cx);
            (auth.is_authenticated(), auth.effective_user_id_for_wire())
        };
        if !is_authenticated {
            return;
        }
        let Some(account_id) = account_id.filter(|id| !id.is_empty()) else {
            return;
        };

        RiseShareContainer::remove_expired_files();
        RiseShareContainer::claim_unbound_story_sessions(&account_id);
        let active_session_id = self.story_share_handoff_phase.active_session_id().map(str::to_owned);
        self.pending_story_share_session_ids = RiseShareContainer::pending_story_session_ids(&account_id)
            .into_iter()
            .filter(|id| Some(id.as_str()) != active_session_id.as_deref())
            .collect();
        cx.notify();
    }

    pub fn enqueue_shared_story_handoff(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if !RiseShareContainer::is_valid_session_identifier(session_id) {
            Self::notify_invalid_shared_story(cx);
            return;
        }

        if self.story_share_handoff_phase.active_session_id() == Some(session_id) {
            return;
        }

        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            if !self.pending_story_share_session_ids.iter().any(|id| id == session_id) {
                self.pending_story_share_session_ids.push(session_id.to_owned());
                cx.notify();
            }
            Notifier::send(
                tr("story_share_sign_in_title"),
                NotifierSendOptions {
                    content: Some(tr("story_share_sign_in_message")),
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("person.crop.circle.badge.checkmark".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }

        self.discover_pending_shared_story_handoffs(cx);
        if !self.pending_story_share_session_ids.iter().any(|id| id == session_id) {
            Self::notify_invalid_shared_story(cx);
            return;
        }

        if self.story_share_handoff_phase.active_session_id().is_some() || self.is_publishing {
            Notifier::send(
                tr("story_share_queued_title"),
                NotifierSendOptions {
                    content: Some(tr("story_share_queued_message")),
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("rectangle.stack.badge.plus".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }

        self.resume_pending_shared_story_handoff_if_possible(cx);
    }

    pub fn resume_pending_shared_story_handoff_if_possible(&mut self, cx: &mut Context<Self>) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        self.discover_pending_shared_story_handoffs(cx);
        if self.is_publishing || self.story_share_handoff_phase.active_session_id().is_some() {
            return;
        }
        if self.pending_story_share_session_ids.is_empty() {
            return;
        }

        let session_id = self.pending_story_share_session_ids.remove(0);
        cx.notify();
        self.start_shared_story_handoff(session_id, cx);
    }

    pub fn export_shared_story_draft(
        &mut self,
        draft: &StoryShareHandoffDraft,
        outcome: MediaEditorOutcome,
        caption: String,
        attached_track: Option<MediaPickerTrackSelection>,
        cx: &mut Context<Self>,
    ) {
        let StoryShareHandoffPhase::Editing(current) = &self.story_share_handoff_phase else {
            return;
        };
        if current.id != draft.id {
            return;
        }

        let edit = match outcome.edit.payload {
            MediaSelectionEditPayload::Image(mut state) => {
                state.caption = caption.clone();
                MediaSelectionEdit { payload: MediaSelectionEditPayload::Image(state) }
            }
            MediaSelectionEditPayload::Video(mut state) => {
                state.caption = caption.clone();
                MediaSelectionEdit { payload: MediaSelectionEditPayload::Video(state) }
            }
        };

        let saved = StoryShareEditorDraft { edit: edit.clone(), caption: caption.clone(), track: attached_track.clone() };
        let saved_context = draft.context.clone();
        cx.spawn(async move |_, _| StoryShareEditorDraftFile::save(saved, &saved_context).await).detach();

        self.start_shared_story_export(
            draft.clone(),
            edit,
            caption,
            attached_track,
            false,
            None,
            cx,
        );
    }

    pub fn record_shared_story_publish_request(
        &mut self,
        request: StorySharePublishRequest,
        publish_context: &StorySharePublishContext,
        cx: &mut Context<Self>,
    ) {
        let StoryShareHandoffPhase::Composing(composition) = &self.story_share_handoff_phase else {
            return;
        };
        if composition.context.publish_context().idempotency_key() != publish_context.idempotency_key() {
            return;
        }
        let context = composition.context.clone();
        cx.spawn(async move |_, _| StoryShareEditorDraftFile::save_publish_request(&request, &context).await)
            .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn start_shared_story_export(
        &mut self,
        draft: StoryShareHandoffDraft,
        edit: MediaSelectionEdit,
        caption: String,
        attached_track: Option<MediaPickerTrackSelection>,
        immediate_send: bool,
        publish_request: Option<StorySharePublishRequest>,
        cx: &mut Context<Self>,
    ) {
        let sends_immediately = immediate_send || publish_request.is_some();
        let export = StoryShareHandoffExport {
            draft: draft.clone(),
            edit: edit.clone(),
            caption: caption.clone(),
            immediate_send: sends_immediately,
        };
        self.story_share_handoff_phase = StoryShareHandoffPhase::Exporting(export);
        self.story_share_handoff_task = None;
        let exporter = MediaExporter::new();
        self.story_share_handoff_exporter = Some(exporter.clone());
        let progress = cx.new(ExportProgressViewState::new);
        cx.notify();

        self.story_share_handoff_task = Some(cx.spawn(async move |this, cx| {
            let result = exporter
                .export_external(
                    draft.source_url.clone(),
                    draft.source_image.clone(),
                    draft.media_asset.clone(),
                    edit,
                    None,
                    progress.downgrade(),
                    cx,
                )
                .await;
            drop(progress);
            let outcome = this.update(cx, |this, cx| {
                let is_current_export = matches!(
                    &this.story_share_handoff_phase,
                    StoryShareHandoffPhase::Exporting(current) if current.draft.id == draft.id
                );
                match result {
                    Ok(item) => {
                        if !is_current_export {
                            Self::remove_generated_files(&item);
                            return;
                        }
                        this.story_share_handoff_exporter = None;
                        this.release_story_share_handoff_task();
                        this.reset_publish_state(cx);
                        let composition = StoryShareHandoffComposition {
                            context: draft.context.clone(),
                            item,
                            caption: publish_request.as_ref().map(|request| request.caption.clone()).unwrap_or(caption),
                            immediate_send: sends_immediately,
                            attached_track: publish_request
                                .as_ref()
                                .map(|request| request.track.clone())
                                .unwrap_or(attached_track),
                            publish_request,
                        };
                        this.story_share_handoff_phase = StoryShareHandoffPhase::Composing(composition.clone());
                        cx.notify();
                        if composition.immediate_send {
                            this.publish_shared_story_composition_in_background(&composition, cx);
                        }
                    }
                    Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                        if is_current_export {
                            this.story_share_handoff_exporter = None;
                            this.release_story_share_handoff_task();
                        }
                    }
                    Err(error) => {
                        let StoryShareHandoffPhase::Exporting(current) = &this.story_share_handoff_phase else {
                            return;
                        };
                        if current.draft.id != draft.id {
                            return;
                        }
                        let retry = current.draft.retrying(current.edit.clone(), current.caption.clone());
                        this.story_share_handoff_exporter = None;
                        this.release_story_share_handoff_task();
                        this.story_share_handoff_phase = StoryShareHandoffPhase::Editing(retry);
                        cx.notify();
                        Sai::log(
                            "StoryShare",
                            move || format!("external media export failed: {error}"),
                            SaiLogKind::Network,
                            cx,
                        );
                        Notifier::send(
                            tr("story_share_export_error_title"),
                            NotifierSendOptions {
                                content: Some(tr("story_share_export_error_message")),
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Top,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    }
                }
            });
            outcome.ok();
        }));
    }

    pub fn shared_story_did_publish_current(&mut self, cx: &mut Context<Self>) {
        let finalization = match &self.story_share_handoff_phase {
            StoryShareHandoffPhase::Composing(composition) => StoryShareHandoffFinalization {
                context: composition.context.clone(),
                generated_item: Some(composition.item.clone()),
                error_message: None,
            },
            StoryShareHandoffPhase::Recovering(recovery) => StoryShareHandoffFinalization {
                context: recovery.context.clone(),
                generated_item: None,
                error_message: None,
            },
            _ => return,
        };
        self.story_share_handoff_phase = StoryShareHandoffPhase::Finalizing(finalization.clone());
        cx.notify();
        self.finalize_published_shared_story(finalization, cx);
    }

    pub fn retry_shared_story_finalization(&mut self, cx: &mut Context<Self>) {
        let StoryShareHandoffPhase::Finalizing(finalization) = &self.story_share_handoff_phase else {
            return;
        };
        if finalization.error_message.is_none() {
            return;
        }
        let retry = StoryShareHandoffFinalization {
            context: finalization.context.clone(),
            generated_item: finalization.generated_item.clone(),
            error_message: None,
        };
        self.publish_error = None;
        self.story_share_handoff_phase = StoryShareHandoffPhase::Finalizing(retry.clone());
        cx.notify();
        self.finalize_published_shared_story(retry, cx);
    }

    pub fn retry_shared_story_background_work(&mut self, cx: &mut Context<Self>) {
        if self.is_publishing {
            return;
        }
        match self.story_share_handoff_phase.clone() {
            StoryShareHandoffPhase::Composing(composition) if composition.immediate_send => {
                self.publish_shared_story_composition_in_background(&composition, cx);
            }
            StoryShareHandoffPhase::Recovering(recovery) => {
                self.resume_shared_story_recovery_in_background(&recovery, cx);
            }
            StoryShareHandoffPhase::Finalizing(finalization) if finalization.error_message.is_some() => {
                self.retry_shared_story_finalization(cx);
            }
            _ => {}
        }
    }

    pub fn cancel_shared_story_handoff(&mut self, cx: &mut Context<Self>) {
        if self.active_shared_story_requires_recovery_preservation(cx) {
            self.discard_shared_story_handoff(false, false, cx);
            return;
        }
        self.discard_shared_story_handoff(true, true, cx);
    }

    fn active_shared_story_requires_recovery_preservation(&self, cx: &mut App) -> bool {
        match &self.story_share_handoff_phase {
            StoryShareHandoffPhase::Recovering(_) | StoryShareHandoffPhase::Finalizing(_) => true,
            StoryShareHandoffPhase::Composing(composition) => {
                match RiseShareContainer::story_publish_checkpoint(
                    &composition.context.session_id,
                    &composition.context.account_id,
                ) {
                    Ok(checkpoint) => checkpoint.is_some(),
                    Err(error) => {
                        Sai::log(
                            "StoryShare",
                            move || format!("checkpoint check failed during cancel: {error}"),
                            SaiLogKind::Network,
                            cx,
                        );
                        true
                    }
                }
            }
            _ => false,
        }
    }

    pub fn discard_shared_story_handoff(&mut self, cleanup_session: bool, resume_pending: bool, cx: &mut Context<Self>) {
        let active_session_id = self.story_share_handoff_phase.active_session_id().map(str::to_owned);
        match &self.story_share_handoff_phase {
            StoryShareHandoffPhase::Composing(composition) => Self::remove_generated_files(&composition.item),
            StoryShareHandoffPhase::Finalizing(finalization) => {
                if let Some(generated_item) = finalization.generated_item.as_ref() {
                    Self::remove_generated_files(generated_item);
                }
            }
            _ => {}
        }

        self.story_share_handoff_task = None;
        if let Some(exporter) = self.story_share_handoff_exporter.take() {
            exporter.cancel();
        }
        self.story_share_handoff_phase = StoryShareHandoffPhase::Idle;
        cx.notify();

        if cleanup_session && let Some(active_session_id) = active_session_id.as_deref() {
            RiseShareContainer::remove_story_session(active_session_id);
        }

        if resume_pending {
            self.resume_pending_shared_story_handoff_if_possible(cx);
        } else {
            if cleanup_session {
                for session_id in &self.pending_story_share_session_ids {
                    RiseShareContainer::remove_story_session(session_id);
                }
            }
            self.pending_story_share_session_ids.clear();
            cx.notify();
        }
    }

    fn start_shared_story_handoff(&mut self, session_id: String, cx: &mut Context<Self>) {
        let Some(expected_account_id) = AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|id| !id.is_empty())
        else {
            Self::notify_invalid_shared_story(cx);
            return;
        };
        if self.is_publishing {
            if !self.pending_story_share_session_ids.contains(&session_id) {
                self.pending_story_share_session_ids.push(session_id);
                cx.notify();
            }
            return;
        }
        self.reset_publish_state(cx);
        self.pending_story_share_session_ids.retain(|id| *id != session_id);
        self.story_share_handoff_phase =
            StoryShareHandoffPhase::Preparing { session_id: session_id.clone(), current: 1, total: None };
        self.story_share_handoff_task = None;
        cx.notify();
        self.story_share_handoff_task = Some(cx.spawn(async move |this, cx| {
            let loaded = StoryShareHandoffLoader::load_context(session_id.clone(), expected_account_id).await;
            let outcome = this.update(cx, |this, cx| match loaded {
                Ok(context) => {
                    if this.story_share_handoff_phase.active_session_id() != Some(session_id.as_str()) {
                        return;
                    }
                    this.release_story_share_handoff_task();
                    if let Some(checkpoint) = context.publish_checkpoint.clone() {
                        let recovery = StoryShareHandoffRecovery { context, checkpoint };
                        this.story_share_handoff_phase = StoryShareHandoffPhase::Recovering(recovery.clone());
                        cx.notify();
                        this.resume_shared_story_recovery_in_background(&recovery, cx);
                    } else {
                        this.prepare_shared_story_item(context, cx);
                    }
                }
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {}
                Err(error) => this.fail_shared_story_handoff(&session_id, &error, cx),
            });
            outcome.ok();
        }));
    }

    fn prepare_shared_story_item(&mut self, context: StoryShareHandoffContext, cx: &mut Context<Self>) {
        self.story_share_handoff_task = None;
        self.story_share_handoff_phase = StoryShareHandoffPhase::Preparing {
            session_id: context.session_id.clone(),
            current: context.display_index(),
            total: Some(context.total_count()),
        };
        cx.notify();
        self.story_share_handoff_task = Some(cx.spawn(async move |this, cx| {
            let session_id = context.session_id.clone();
            let mut prepared = match StoryShareHandoffLoader::prepare(context.clone()).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    if error.downcast_ref::<CancellationError>().is_none() {
                        this.update(cx, |this, cx| this.fail_shared_story_handoff(&session_id, &error, cx)).ok();
                    }
                    return;
                }
            };
            let still_active = this
                .update(cx, |this, _| this.story_share_handoff_phase.active_session_id() == Some(session_id.as_str()))
                .unwrap_or(false);
            if !still_active {
                return;
            }
            let image_data = prepared.image_data.take();
            let source_image = cx
                .background_executor()
                .spawn(async move { image_data.as_deref().and_then(|data| CleverImageBitmapDecoder::decode(data, None)) })
                .await;
            if prepared.kind == RiseShareStoryMediaKind::Image && source_image.is_none() {
                let error = anyhow::Error::from(StoryShareHandoffLoadError::InvalidFile);
                this.update(cx, |this, cx| this.fail_shared_story_handoff(&session_id, &error, cx)).ok();
                return;
            }
            let media_asset = MediaAsset::new(
                prepared.context.current_item().id.clone(),
                if prepared.kind == RiseShareStoryMediaKind::Image {
                    MediaAssetMediaType::Image
                } else {
                    MediaAssetMediaType::Video
                },
                prepared.pixel_width,
                prepared.pixel_height,
                prepared.duration,
                None,
            );
            let saved = StoryShareEditorDraftFile::load(&context).await;
            let publish_request = if saved.is_none() {
                None
            } else {
                StoryShareEditorDraftFile::load_publish_request(&context).await
            };
            this.update(cx, |this, cx| {
                if this.story_share_handoff_phase.active_session_id() != Some(context.session_id.as_str()) {
                    return;
                }
                this.release_story_share_handoff_task();
                let draft = StoryShareHandoffDraft {
                    id: Uuid::new_v4(),
                    context: context.clone(),
                    source_url: prepared.source_url.clone(),
                    media_asset,
                    source_image,
                    initial_edit: saved.as_ref().map(|saved| saved.edit.clone()),
                    initial_caption: saved.as_ref().map(|saved| saved.caption.clone()).unwrap_or_default(),
                    initial_track: saved.as_ref().and_then(|saved| saved.track.clone()),
                };
                this.story_share_handoff_phase = StoryShareHandoffPhase::Editing(draft.clone());
                cx.notify();
                if let (Some(saved), Some(publish_request)) = (saved, publish_request) {
                    this.start_shared_story_export(
                        draft,
                        saved.edit,
                        saved.caption,
                        saved.track,
                        true,
                        Some(publish_request),
                        cx,
                    );
                }
            })
            .ok();
        }));
    }

    fn fail_shared_story_handoff(&mut self, session_id: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        if self.story_share_handoff_phase.active_session_id() != Some(session_id) {
            return;
        }
        let description = error.to_string();
        Sai::log(
            "StoryShare",
            move || format!("handoff load failed: {description}"),
            SaiLogKind::Network,
            cx,
        );
        self.release_story_share_handoff_task();
        if let Some(exporter) = self.story_share_handoff_exporter.take() {
            exporter.cancel();
        }
        self.story_share_handoff_phase = StoryShareHandoffPhase::Idle;
        cx.notify();
        RiseShareContainer::remove_story_session(session_id);
        Self::notify_invalid_shared_story(cx);
        self.resume_pending_shared_story_handoff_if_possible(cx);
    }

    fn finalize_published_shared_story(&mut self, finalization: StoryShareHandoffFinalization, cx: &mut Context<Self>) {
        let advanced = RiseShareContainer::advance_story_session(
            &finalization.context.session_id,
            &finalization.context.account_id,
            &finalization.context.current_item().id,
        );
        match advanced {
            Ok(updated) => {
                if let Some(generated_item) = finalization.generated_item.as_ref() {
                    Self::remove_generated_files(generated_item);
                }
                let Some(updated) = updated else {
                    self.story_share_handoff_phase = StoryShareHandoffPhase::Idle;
                    cx.notify();
                    self.resume_pending_shared_story_handoff_if_possible(cx);
                    return;
                };
                let next = StoryShareHandoffContext {
                    session_id: updated.id,
                    account_id: updated.account_id,
                    directory: finalization.context.directory.clone(),
                    items: updated.items,
                    current_index: usize::try_from(updated.next_item_index).unwrap_or(0),
                    publish_checkpoint: updated.publish_checkpoint,
                };
                self.prepare_shared_story_item(next, cx);
            }
            Err(error) => {
                Sai::log(
                    "StoryShare",
                    move || format!("progress commit failed: {error}"),
                    SaiLogKind::Network,
                    cx,
                );
                let message = tr("story_share_progress_error_message");
                self.story_share_handoff_phase = StoryShareHandoffPhase::Finalizing(StoryShareHandoffFinalization {
                    context: finalization.context,
                    generated_item: finalization.generated_item,
                    error_message: Some(message.clone()),
                });
                self.publish_error = Some(message.clone());
                cx.notify();
                self.notify_shared_story_background_failure(&message, cx);
            }
        }
    }

    fn publish_shared_story_composition_in_background(
        &mut self,
        composition: &StoryShareHandoffComposition,
        cx: &mut Context<Self>,
    ) {
        if !composition.immediate_send || self.is_publishing {
            return;
        }
        let StoryShareHandoffPhase::Composing(current) = &self.story_share_handoff_phase else {
            return;
        };
        let idempotency_key = composition.context.publish_context().idempotency_key();
        if current.context.publish_context().idempotency_key() != idempotency_key {
            return;
        }

        self.dismiss_publish_error(cx);
        let request = composition.publish_request.as_ref();
        self.publish_story(
            composition.item.clone(),
            composition.caption.clone(),
            request.map(|request| request.audience).unwrap_or(StoryAudience::Everyone),
            request.is_none_or(|request| request.allow_screenshots),
            request.is_none_or(|request| request.post_to_profile),
            request.map(|request| request.duration_days).unwrap_or(1),
            request.map(|request| request.selected_user_ids.clone()).unwrap_or_default(),
            composition.attached_track.clone(),
            Some(idempotency_key),
            Some(composition.context.publish_context()),
            Self::shared_story_publish_success(cx),
            Self::shared_story_publish_failure(cx),
            cx,
        );
    }

    fn resume_shared_story_recovery_in_background(&mut self, recovery: &StoryShareHandoffRecovery, cx: &mut Context<Self>) {
        if self.is_publishing {
            return;
        }
        let StoryShareHandoffPhase::Recovering(current) = &self.story_share_handoff_phase else {
            return;
        };
        if current.context.publish_context().idempotency_key() != recovery.context.publish_context().idempotency_key() {
            return;
        }

        self.dismiss_publish_error(cx);
        self.resume_persisted_shared_story_publish(
            recovery.context.publish_context(),
            recovery.checkpoint.clone(),
            Self::shared_story_publish_success(cx),
            Self::shared_story_publish_failure(cx),
            cx,
        );
    }

    fn shared_story_publish_success(cx: &mut Context<Self>) -> StoryActionDoneHandler {
        let this = cx.weak_entity();
        Rc::new(move |cx: &mut App| {
            notification(NotificationKind::Success);
            let this = this.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.shared_story_did_publish_current(cx)).ok();
            });
        })
    }

    fn shared_story_publish_failure(cx: &mut Context<Self>) -> StoryActionTextHandler {
        let this = cx.weak_entity();
        Rc::new(move |message: String, cx: &mut App| {
            notification(NotificationKind::Error);
            let this = this.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.notify_shared_story_background_failure(&message, cx)).ok();
            });
        })
    }

    fn notify_shared_story_background_failure(&mut self, raw_message: &str, cx: &mut Context<Self>) {
        let message = trimming_whitespaces_and_newlines(raw_message);
        let resolved = if message.is_empty() { tr("story_create_error") } else { message.to_owned() };
        self.publish_error = Some(resolved.clone());
        cx.notify();
        Notifier::send(
            tr("story_create_error"),
            NotifierSendOptions {
                content: Some(resolved),
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn notify_invalid_shared_story(cx: &mut App) {
        Notifier::send(
            tr("story_share_invalid_title"),
            NotifierSendOptions {
                content: Some(tr("story_share_invalid_message")),
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn release_story_share_handoff_task(&mut self) {
        if let Some(task) = self.story_share_handoff_task.take() {
            task.detach();
        }
    }

    fn remove_generated_files(item: &MediaPickerItem) {
        let mut urls: Vec<Url> = vec![item.url.clone()];
        if let Some(poster_url) = item.video_payload.as_ref().and_then(|video| video.poster_url.clone()) {
            urls.push(poster_url);
        }
        if let Some(original_url) = item.image_payload.as_ref().and_then(|image| image.original_image_url.clone()) {
            urls.push(original_url);
        }
        let unique: HashSet<Url> = urls.iter().map(Url::standardized).collect();
        for url in unique {
            if let Some(path) = url.to_file_path() {
                let _ = FileManager.remove_item(&path);
            }
        }
    }
}

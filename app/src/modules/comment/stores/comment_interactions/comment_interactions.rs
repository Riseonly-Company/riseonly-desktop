use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{AbortHandle, Abortable};
use gpui::{App, AsyncApp, Context, Entity, Size, Task, Window};
use image::metadata::Orientation;
use image::{ImageDecoder, ImageReader};
use rise_canon::{RiseGuideSignal, RiseIdentityCanon, RiseRequestBody, RiseTimingCanon, RiseTimingDebounce, rise_request};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::NSRange;
use rise_engine::CancellationError;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::sai::Sai;
use crate::core::restriction::restriction_store::RestrictionStore;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy,
};
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatComposerCustomEmojiRun, ChatDraftCustomEmoji,
    resign_chat_keyboard,
};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CustomEmojiEntityWire, MessageEntitiesCodec,
};
use crate::modules::comment::engine::rise_comment_engine_models::{
    RiseCommentCreateDraft, RiseCommentCreateOutcome, RiseCommentListSnapshot, RiseCommentScope,
};
use crate::modules::comment::engine::rise_comment_photo_upload::RiseCommentPhotoUpload;
use crate::modules::comment::engine::rise_comment_presentation::RiseCommentPresentation;
use crate::modules::comment::stores::comment_actions::comment_actions::CommentActionsStore;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentMedia, CommentMediaType, CommentPhotoUpload, CommentPhotoUploadProgress, CommentReaction, CommentSortType,
    GetCommentsResponse, RepliesSortType,
};
use crate::modules::comment::stores::comment_services::comment_services::CommentServicesStore;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_guide_sandbox_hooks::PostGuideSandboxHooks;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, PackKind, StickerItem};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;

static TEXT_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| {
    RiseFieldRule::new(<rise_request::comment::CreateComment as RiseRequestBody>::CONTRACT, "/original_content")
});

const COMMENT_REACTION_GROUP: &str = "comment_reaction";
const COMMENT_REACTION_MAX_CACHE_DATA: i64 = 500;
const HIGHLIGHT_DURATION: Duration = Duration::from_millis(1800);
const FOCUS_LOAD_ATTEMPTS_LIMIT: i64 = 6;
const CUSTOM_EMOJI_PLACEHOLDER: &str = "\u{FFFC}";

#[derive(Clone)]
struct FailedCommentRetry {
    post_id: String,
    text: String,
    media: Option<CommentMedia>,
    custom_emoji_drafts: Vec<ChatDraftCustomEmoji>,
    reply_target_id: Option<String>,
    reply_root_id: Option<String>,
    reply_target_tag: Option<String>,
    reply_target_name: Option<String>,
    reply_target_snippet: Option<String>,
    reply_target_is_official: bool,
    reply_target_is_premium: bool,
}

pub struct CommentInteractionsStore {
    pub comments_sheet_visible: bool,
    pub caption_expanded: bool,
    expanded_comment_ids: HashSet<String>,
    comment_text: String,
    pub reply_target_id: Option<String>,
    pub reply_target_tag: Option<String>,
    pub reply_target_name: Option<String>,
    pub reply_target_snippet: Option<String>,
    pub reply_target_is_official: bool,
    pub reply_target_is_premium: bool,
    pub focus_bump_signal: i64,
    pub selected_comment: Option<GetCommentsResponse>,
    pub sort_menu_visible: bool,
    custom_emoji_drafts: Vec<ChatDraftCustomEmoji>,
    draft_post_id: Option<String>,
    post_drafts: HashMap<String, (String, Vec<ChatDraftCustomEmoji>)>,
    pub photo_picker_visible: bool,
    pending_scroll_reply_id: Option<String>,
    pending_scroll_comment_id: Option<String>,
    photo_uploads: HashMap<String, CommentPhotoUpload>,
    photo_upload_tasks: HashMap<String, AbortHandle>,
    failed_comment_retry: Option<FailedCommentRetry>,
    thread_root: Option<GetCommentsResponse>,
    replies_sort: RepliesSortType,
    reply_root_id: Option<String>,
    pending_focus_post_id: Option<String>,
    pending_focus_comment_id: Option<String>,
    pending_focus_replying: bool,
    focus_target: Option<(String, bool)>,
    thread_focus_target: Option<(String, bool)>,
    pub focus_comment_id: Option<String>,
    pub highlighted_comment_id: Option<String>,
    highlight_task: Option<Task<()>>,
    highlight_generation: Uuid,
    focus_load_attempts: i64,
    thread_focus_load_attempts: i64,
    observed_list_scope: Option<RiseCommentScope>,
    observed_thread_scope: Option<RiseCommentScope>,
}

impl CommentInteractionsStore {
    const COMMENTS_WATCH_OWNER: &'static str = "comments.sheet";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            comments_sheet_visible: false,
            caption_expanded: false,
            expanded_comment_ids: HashSet::new(),
            comment_text: String::new(),
            reply_target_id: None,
            reply_target_tag: None,
            reply_target_name: None,
            reply_target_snippet: None,
            reply_target_is_official: false,
            reply_target_is_premium: false,
            focus_bump_signal: 0,
            selected_comment: None,
            sort_menu_visible: false,
            custom_emoji_drafts: Vec::new(),
            draft_post_id: None,
            post_drafts: HashMap::new(),
            photo_picker_visible: false,
            pending_scroll_reply_id: None,
            pending_scroll_comment_id: None,
            photo_uploads: HashMap::new(),
            photo_upload_tasks: HashMap::new(),
            failed_comment_retry: None,
            thread_root: None,
            replies_sort: RepliesSortType::Old,
            reply_root_id: None,
            pending_focus_post_id: None,
            pending_focus_comment_id: None,
            pending_focus_replying: false,
            focus_target: None,
            thread_focus_target: None,
            focus_comment_id: None,
            highlighted_comment_id: None,
            highlight_task: None,
            highlight_generation: Uuid::new_v4(),
            focus_load_attempts: 0,
            thread_focus_load_attempts: 0,
            observed_list_scope: None,
            observed_thread_scope: None,
        }
    }

    fn actions(cx: &mut App) -> Entity<CommentActionsStore> {
        CommentActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<CommentServicesStore> {
        CommentServicesStore::shared(cx)
    }

    fn service_list_scope(cx: &mut App) -> Option<RiseCommentScope> {
        Self::service(cx).read(cx).list_scope()
    }

    fn service_has_more(cx: &mut App) -> bool {
        Self::service(cx).update(cx, |service, cx| service.has_more(cx))
    }

    fn service_items(cx: &mut App) -> Vec<GetCommentsResponse> {
        Self::service(cx).update(cx, |service, cx| service.items(cx))
    }

    fn selected_post_id(cx: &mut App) -> Option<String> {
        PostServicesStore::shared(cx).read(cx).selected_post.as_ref().map(|post| post.id.clone())
    }

    fn selected_post_can_comment(cx: &mut App) -> bool {
        PostServicesStore::shared(cx).read(cx).selected_post.as_ref().is_some_and(|post| post.can_comment)
    }

    fn wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty())
    }

    pub fn text_rule() -> RiseFieldRule {
        *TEXT_RULE
    }

    pub fn reaction_debounce() -> RiseTimingDebounce {
        RiseTimingCanon::debounce(COMMENT_REACTION_GROUP)
    }

    pub fn comment_text(&self) -> &str {
        &self.comment_text
    }

    pub fn set_comment_text(&mut self, value: &str, cx: &mut Context<Self>) {
        self.comment_text = TEXT_RULE.clamped(value);
        cx.notify();
    }

    pub fn custom_emoji_drafts(&self) -> &[ChatDraftCustomEmoji] {
        &self.custom_emoji_drafts
    }

    pub fn pending_scroll_reply_id(&self) -> Option<&str> {
        self.pending_scroll_reply_id.as_deref()
    }

    pub fn pending_scroll_comment_id(&self) -> Option<&str> {
        self.pending_scroll_comment_id.as_deref()
    }

    pub fn photo_uploads(&self) -> &HashMap<String, CommentPhotoUpload> {
        &self.photo_uploads
    }

    pub fn thread_root(&self) -> Option<&GetCommentsResponse> {
        self.thread_root.as_ref()
    }

    pub fn replies_sort(&self) -> RepliesSortType {
        self.replies_sort
    }

    pub fn pending_focus_post_id(&self) -> Option<&str> {
        self.pending_focus_post_id.as_deref()
    }

    pub fn pending_focus_comment_id(&self) -> Option<&str> {
        self.pending_focus_comment_id.as_deref()
    }

    pub fn thread_scope(&self) -> Option<RiseCommentScope> {
        let root = self.thread_root.as_ref()?;
        Some(RiseCommentScope::Replies { root_id: root.id.clone(), sort: self.replies_sort })
    }

    pub fn thread_snapshot(&self, cx: &mut App) -> Option<Arc<RiseCommentListSnapshot>> {
        let scope = self.thread_scope()?;
        Some(RiseCommentPresentation::shared(cx).update(cx, |presentation, cx| presentation.snapshot(&scope, cx)))
    }

    pub fn thread_items(&self, cx: &mut App) -> Vec<GetCommentsResponse> {
        self.thread_snapshot(cx).map(|snapshot| snapshot.items.clone()).unwrap_or_default()
    }

    pub fn thread_has_more(&self, cx: &mut App) -> bool {
        self.thread_snapshot(cx).is_some_and(|snapshot| snapshot.has_more)
    }

    pub fn thread_is_loading(&self, cx: &mut App) -> bool {
        self.thread_snapshot(cx).is_some_and(|snapshot| snapshot.shows_initial_loader() || snapshot.is_loading_more)
    }

    pub fn thread_error_text(&self, cx: &mut App) -> Option<String> {
        self.thread_snapshot(cx).and_then(|snapshot| snapshot.error_text.clone())
    }

    pub fn open_comments_sheet(
        &mut self,
        post: &GetPostFeedResponse,
        focus_comment_id: Option<&str>,
        caption_expanded: bool,
        replying: bool,
        cx: &mut Context<Self>,
    ) {
        PostServicesStore::shared(cx).update(cx, |service, cx| service.select_post_for_comments(post.clone(), cx));
        self.swap_draft_for_post(&post.id);
        self.caption_expanded = caption_expanded;
        self.expanded_comment_ids.clear();
        self.cancel_reply(cx);
        self.close_thread(cx);
        self.focus_comment_id = None;
        self.clear_comment_highlight(cx);
        self.focus_load_attempts = 0;
        self.focus_target = focus_comment_id.map(|comment_id| (comment_id.to_owned(), replying));
        self.thread_focus_target = None;
        self.stop_observing_list(cx);
        Self::service(cx).update(cx, |service, cx| {
            let sort = service.list_sort();
            service.prepare_for_post(&post.id, sort, cx);
        });
        self.comments_sheet_visible = true;
        cx.notify();
        self.load_list(focus_comment_id, cx);
        self.refresh_selected_post(&post.id, cx);
    }

    fn swap_draft_for_post(&mut self, post_id: &str) {
        if self.draft_post_id.as_deref() == Some(post_id) {
            return;
        }
        if let Some(previous) = self.draft_post_id.take() {
            let text = std::mem::take(&mut self.comment_text);
            let emojis = std::mem::take(&mut self.custom_emoji_drafts);
            if text.is_empty() {
                self.post_drafts.remove(&previous);
            } else {
                self.post_drafts.insert(previous, (text, emojis));
            }
        }
        if let Some((text, emojis)) = self.post_drafts.remove(post_id) {
            self.comment_text = text;
            self.custom_emoji_drafts = emojis;
        }
        self.draft_post_id = Some(post_id.to_owned());
    }

    fn refresh_selected_post(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if PostGuideSandboxHooks::post(post_id, cx).is_some() || int_from_string(post_id).unwrap_or(0) <= 0 {
            return;
        }
        let post_id = post_id.to_owned();
        let refresh = PostActionsStore::shared(cx).update(cx, |actions, cx| actions.refresh_post_detail_action(&post_id, cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            refresh.await;
            cx.update(|cx| {
                PostServicesStore::shared(cx).update(cx, |service, cx| service.sync_selected_post_comment_state(&post_id, cx));
            });
        })
        .detach();
    }

    fn load_list(&mut self, focus_comment_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(scope) = Self::service_list_scope(cx) else {
            return;
        };
        if self.observed_list_scope.as_ref() != Some(&scope) {
            self.stop_observing_list(cx);
            self.observed_list_scope = Some(scope.clone());
            let begin_scope = scope.clone();
            Self::actions(cx).update(cx, |actions, cx| actions.begin_observing_action(begin_scope, cx));
            if let RiseCommentScope::Post { post_id, .. } = &scope {
                let post_id = post_id.clone();
                cx.defer(move |cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.watch_posts(vec![post_id], Self::COMMENTS_WATCH_OWNER, cx);
                    });
                });
            }
        }
        let this = cx.weak_entity();
        let focused_scope = scope.clone();
        let focus_comment_id = focus_comment_id.map(str::to_owned);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.open_list_action(
                scope,
                false,
                focus_comment_id,
                Some(Box::new(move |focused_id: Option<String>, cx: &mut App| {
                    let _ = this.update(cx, |store, cx| {
                        if Self::service_list_scope(cx).as_ref() != Some(&focused_scope) {
                            return;
                        }
                        store.focus_comment_id = focused_id;
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }

    fn stop_observing_list(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.observed_list_scope.take() else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| actions.end_observing_action(scope, cx));
        cx.defer(|cx| {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.stop_watching_posts(Self::COMMENTS_WATCH_OWNER, cx);
            });
        });
    }

    fn load_thread(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.thread_scope() else {
            return;
        };
        if self.observed_thread_scope.as_ref() != Some(&scope) {
            self.stop_observing_thread(cx);
            self.observed_thread_scope = Some(scope.clone());
            let begin_scope = scope.clone();
            Self::actions(cx).update(cx, |actions, cx| actions.begin_observing_action(begin_scope, cx));
        }
        Self::actions(cx).update(cx, |actions, cx| actions.open_list_action(scope, false, None, None, cx));
    }

    fn stop_observing_thread(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.observed_thread_scope.take() else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| actions.end_observing_action(scope, cx));
    }

    pub fn close_comments_sheet(&mut self, cx: &mut Context<Self>) {
        self.comments_sheet_visible = false;
        self.stop_observing_list(cx);
        self.focus_comment_id = None;
        self.focus_target = None;
        self.thread_focus_target = None;
        self.pending_scroll_comment_id = None;
        self.clear_comment_highlight(cx);
        self.close_thread(cx);
        cx.notify();
    }

    pub fn open_sticker_pack(&mut self, pack_id: &str, kind: PackKind, cx: &mut Context<Self>) {
        StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.present(pack_id, kind, cx));
    }

    pub fn open_thread(&mut self, root: &GetCommentsResponse, cx: &mut Context<Self>) {
        if root.id.is_empty() {
            return;
        }
        let is_same_thread = self.thread_root.as_ref().is_some_and(|current| current.id == root.id);
        if !is_same_thread {
            self.clear_comment_highlight(cx);
        }
        self.thread_root = Some(root.clone());
        self.reply_root_id = Some(root.id.clone());
        cx.notify();
        if is_same_thread {
            return;
        }
        self.replies_sort = RepliesSortType::Old;
        self.load_thread(cx);
    }

    pub fn close_thread(&mut self, cx: &mut Context<Self>) {
        self.clear_comment_highlight(cx);
        self.stop_observing_thread(cx);
        self.thread_focus_target = None;
        self.thread_root = None;
        self.replies_sort = RepliesSortType::Old;
        self.pending_scroll_reply_id = None;
        self.cancel_reply(cx);
        cx.notify();
    }

    pub fn load_more_replies(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.thread_scope() else {
            return;
        };
        if !self.thread_has_more(cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_action(scope, cx));
    }

    pub fn retry_thread_replies(&mut self, cx: &mut Context<Self>) {
        let (Some(scope), Some(snapshot)) = (self.thread_scope(), self.thread_snapshot(cx)) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            if snapshot.items.is_empty() {
                actions.open_list_action(scope, true, None, None, cx);
            } else {
                actions.load_more_action(scope, cx);
            }
        });
    }

    pub fn change_replies_sort(&mut self, sort: RepliesSortType, cx: &mut Context<Self>) {
        if self.thread_root.is_none() || self.replies_sort == sort {
            return;
        }
        self.replies_sort = sort;
        cx.notify();
        self.load_thread(cx);
    }

    pub fn arm_comment_focus(&mut self, post_id: &str, comment_id: Option<&str>, replying: bool, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X666", "arm focus: post {post_id}, comment {comment_id:?}, replying {replying}");
        self.pending_focus_post_id = Some(post_id.to_owned());
        self.pending_focus_comment_id = comment_id.map(str::to_owned);
        self.pending_focus_replying = replying;
        cx.notify();
    }

    pub fn consume_pending_focus_if_matches(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        if self.pending_focus_post_id.as_deref() != Some(post.id.as_str()) {
            return;
        }
        let comment_id = self.pending_focus_comment_id.take();
        let replying = self.pending_focus_replying;
        self.pending_focus_post_id = None;
        self.pending_focus_replying = false;
        self.open_comments_sheet(post, comment_id.as_deref(), false, replying, cx);
    }

    pub fn focus_maybe_load_more(&mut self, cx: &mut Context<Self>) {
        if self.focus_comment_id.is_none() {
            return;
        }
        if self.focus_load_attempts >= FOCUS_LOAD_ATTEMPTS_LIMIT || !Self::service_has_more(cx) {
            self.focus_comment_id = None;
            self.focus_target = None;
            cx.notify();
            return;
        }
        self.focus_load_attempts += 1;
        self.load_more_comments(cx);
    }

    pub fn resolve_focus(&mut self, comment_id: &str, cx: &mut Context<Self>) {
        self.focus_comment_id = None;
        cx.notify();
        let target = self.focus_target.take();
        let root = Self::service_items(cx).into_iter().find(|item| item.id == comment_id);
        tracing::warn!(
            target: "riseonly::X666",
            "resolve focus: on {comment_id}, target {target:?}, root found {}, replies {}",
            root.is_some(),
            root.as_ref().map_or(-1, |root| i64::from(root.replies_count))
        );
        let (Some((target_comment_id, target_replying)), Some(root)) = (target, root) else {
            self.highlight_focused(comment_id, cx);
            return;
        };
        if target_comment_id == comment_id {
            self.highlight_focused(comment_id, cx);
            if !target_replying {
                return;
            }
            cx.spawn(async move |this, cx: &mut AsyncApp| {
                let _ = this.update(cx, |store, cx| {
                    if store.comments_sheet_visible {
                        store.start_reply(&root, cx);
                    }
                });
            })
            .detach();
            return;
        }
        if root.replies_count <= 0 {
            self.highlight_focused(comment_id, cx);
            return;
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let _ = this.update(cx, |store, cx| {
                if !store.comments_sheet_visible || !Self::service_items(cx).iter().any(|item| item.id == root.id) {
                    return;
                }
                store.thread_focus_target = Some((target_comment_id.clone(), target_replying));
                store.thread_focus_load_attempts = 0;
                store.open_thread(&root, cx);
                store.pending_scroll_reply_id = Some(target_comment_id);
                cx.notify();
            });
        })
        .detach();
    }

    fn highlight_focused(&mut self, comment_id: &str, cx: &mut Context<Self>) {
        self.clear_comment_highlight(cx);
        self.highlighted_comment_id = Some(comment_id.to_owned());
        cx.notify();
        let comment_id = comment_id.to_owned();
        let generation = self.highlight_generation;
        let account_id = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned);
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let list_scope = Self::service_list_scope(cx);
        let reply_scope = self.thread_scope();
        self.highlight_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(HIGHLIGHT_DURATION).await;
            let _ = this.update(cx, |store, cx| {
                if store.highlight_generation != generation
                    || !store.comments_sheet_visible
                    || store.highlighted_comment_id.as_deref() != Some(comment_id.as_str())
                    || AuthServicesStore::shared(cx).read(cx).active_account_id() != account_id.as_deref()
                    || SaiAccountCacheScope::active_epoch() != account_epoch
                    || Self::service_list_scope(cx) != list_scope
                    || store.thread_scope() != reply_scope
                {
                    return;
                }
                store.highlight_task = None;
                store.highlighted_comment_id = None;
                cx.notify();
            });
        }));
    }

    fn clear_comment_highlight(&mut self, cx: &mut Context<Self>) {
        self.highlight_task = None;
        self.highlight_generation = Uuid::new_v4();
        self.highlighted_comment_id = None;
        cx.notify();
    }

    pub fn load_more_comments(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = Self::service_list_scope(cx) else {
            return;
        };
        if !Self::service_has_more(cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_action(scope, cx));
    }

    pub fn retry_comments(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = Self::service_list_scope(cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| actions.open_list_action(scope, true, None, None, cx));
    }

    pub fn change_sort(&mut self, sort: CommentSortType, cx: &mut Context<Self>) {
        let (post_id, list_sort) = {
            let service = Self::service(cx);
            let service = service.read(cx);
            (service.list_owner_post_id().map(str::to_owned), service.list_sort())
        };
        let Some(post_id) = post_id else {
            return;
        };
        if list_sort == sort {
            return;
        }
        self.close_thread(cx);
        Self::service(cx).update(cx, |service, cx| service.prepare_for_post(&post_id, sort, cx));
        self.load_list(None, cx);
    }

    pub fn start_reply(&mut self, comment: &GetCommentsResponse, cx: &mut Context<Self>) {
        let is_thread_root = self.thread_root.as_ref().is_some_and(|root| root.id == comment.id);
        if comment.parent_id.is_none() && !is_thread_root {
            self.open_thread(comment, cx);
        }
        let display_tag = comment.author.display_tag();
        let display_name = comment.author.display_name();
        self.reply_target_id = Some(comment.id.clone());
        self.reply_root_id = Some(comment.parent_id.clone().unwrap_or_else(|| comment.id.clone()));
        self.reply_target_name = Some(if !display_name.is_empty() {
            display_name
        } else if display_tag.is_empty() {
            tr("notify_unknown_user")
        } else {
            format!("@{display_tag}")
        });
        self.reply_target_tag = Some(display_tag);
        self.reply_target_snippet = Some(comment.content.clone());
        self.reply_target_is_official = comment.author.more.is_official;
        self.reply_target_is_premium = comment.author.more.is_premium;
        self.focus_bump_signal = self.focus_bump_signal.wrapping_add(1);
        cx.notify();
    }

    pub fn cancel_reply(&mut self, cx: &mut Context<Self>) {
        self.reply_target_id = None;
        self.reply_root_id = self.thread_root.as_ref().map(|root| root.id.clone());
        self.reply_target_tag = None;
        self.reply_target_name = None;
        self.reply_target_snippet = None;
        self.reply_target_is_official = false;
        self.reply_target_is_premium = false;
        cx.notify();
    }

    pub fn insert_emoji(&mut self, emoji: &str, selection: Option<NSRange>, cx: &mut Context<Self>) -> NSRange {
        let edit =
            ChatComposerCaretEdit::replace(&self.comment_text, selection, emoji, &mut self.custom_emoji_drafts, None);
        self.apply_caret_edit(edit, cx)
    }

    pub fn append_custom_emoji(
        &mut self,
        item: &StickerItem,
        selection: Option<NSRange>,
        cx: &mut Context<Self>,
    ) -> Option<NSRange> {
        if item.id.is_empty() || item.file_url.is_empty() {
            return None;
        }
        let edit = ChatComposerCaretEdit::replace(
            &self.comment_text,
            selection,
            CUSTOM_EMOJI_PLACEHOLDER,
            &mut self.custom_emoji_drafts,
            Some(ChatDraftCustomEmoji { placeholder: CUSTOM_EMOJI_PLACEHOLDER.to_owned(), item: item.clone() }),
        );
        Some(self.apply_caret_edit(edit, cx))
    }

    pub fn delete_backward(&mut self, selection: Option<NSRange>, cx: &mut Context<Self>) -> Option<NSRange> {
        let edit = ChatComposerCaretEdit::delete_backward(&self.comment_text, selection, &mut self.custom_emoji_drafts)?;
        Some(self.apply_caret_edit(edit, cx))
    }

    fn apply_caret_edit(&mut self, edit: ChatComposerCaretEditResult, cx: &mut Context<Self>) -> NSRange {
        self.set_comment_text(&edit.text, cx);
        edit.caret
    }

    pub fn composer_custom_emoji_runs(&self, text: &str) -> Vec<ChatComposerCustomEmojiRun> {
        if self.custom_emoji_drafts.is_empty() {
            return Vec::new();
        }
        ChatComposerCaretEdit::custom_emoji_runs(text, &self.custom_emoji_drafts)
    }

    pub fn sync_custom_emoji_drafts(&mut self, runs: &[ChatComposerCustomEmojiRun], cx: &mut Context<Self>) {
        if runs.iter().map(|run| &run.item).eq(self.custom_emoji_drafts.iter().map(|draft| &draft.item)) {
            return;
        }
        self.custom_emoji_drafts = runs
            .iter()
            .map(|run| ChatDraftCustomEmoji { placeholder: CUSTOM_EMOJI_PLACEHOLDER.to_owned(), item: run.item.clone() })
            .collect();
        cx.notify();
    }

    fn composer_entities(&self, trimmed_text: &str) -> Option<String> {
        let entities: Vec<CustomEmojiEntityWire> = self
            .composer_custom_emoji_runs(trimmed_text)
            .into_iter()
            .map(|run| CustomEmojiEntityWire {
                offset: run.range.location as i64,
                length: run.range.length as i64,
                custom_emoji_id: run.item.id,
                file_url: RemoteMediaURL::optional_normalized(Some(run.item.file_url.as_str())).unwrap_or(run.item.file_url),
                asset_type: run.item.asset_type.wire().to_owned(),
                width: run.item.width,
                height: run.item.height,
                pack_id: run.item.pack_id,
            })
            .collect();
        if entities.is_empty() {
            return None;
        }
        MessageEntitiesCodec::encode(&entities, None)
    }

    pub fn failed_photo_draft(&self, cx: &mut App) -> Option<CommentMedia> {
        let post_id = Self::selected_post_id(cx)?;
        self.failed_draft_media(&post_id)
            .filter(|media| media.r#type == CommentMediaType::Photo && !media.file_url.is_empty())
    }

    pub fn send_current_comment(&mut self, cx: &mut Context<Self>) {
        let media = self.failed_photo_draft(cx);
        self.send_current_comment_media(media, cx);
    }

    pub fn dismiss_failed_photo_draft(&mut self, cx: &mut Context<Self>) {
        let Some(post_id) = Self::selected_post_id(cx) else {
            return;
        };
        self.dismiss_failed_draft(&post_id, cx);
    }

    pub fn send_gif(&mut self, gif: &GifItem, cx: &mut Context<Self>) {
        if gif.file_url.is_empty() {
            return;
        }
        self.send_current_comment_media(Some(CommentMedia::from_gif(gif)), cx);
        if let Some(user_id) = Self::wire_user_id(cx) {
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.add_gif_favourite(&user_id, gif, cx));
        }
    }

    pub fn send_sticker(&mut self, sticker: &StickerItem, cx: &mut Context<Self>) {
        if sticker.file_url.is_empty() || sticker.id.is_empty() {
            return;
        }
        self.send_current_comment_media(Some(CommentMedia::from_sticker(sticker)), cx);
        if let Some(user_id) = Self::wire_user_id(cx) {
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.write_recent(&user_id, &sticker.id, Some(PackKind::Sticker), cx);
            });
        }
    }

    pub fn send_photo(&mut self, item: MediaPickerItem, cx: &mut Context<Self>) {
        if !Self::selected_post_can_comment(cx) {
            return;
        }
        if item.kind != MediaPickerItemKind::Image || item.is_file {
            return;
        }

        let pixel_size = Self::pixel_size(&item);
        let animated = matches!(
            ChatAnimatedMediaSourcePolicy::playback_kind(item.url.absolute_string(), None, None),
            ChatAnimatedMediaPlaybackKind::AnimatedImage
        );
        let local_media = CommentMedia {
            width: pixel_size.width as i64,
            height: pixel_size.height as i64,
            ..CommentMedia::new(
                if animated { CommentMediaType::Gif } else { CommentMediaType::Photo },
                item.url.absolute_string().to_owned(),
            )
        };
        let parent_id = self.current_parent_id();
        let Some(draft) = self.stage_draft(Some(local_media), parent_id, cx) else {
            return;
        };
        let temp_id = draft.temp_id.clone();

        self.photo_uploads.insert(temp_id.clone(), CommentPhotoUpload::QUEUED);
        let (abort, registration) = AbortHandle::new_pair();
        self.photo_upload_tasks.insert(temp_id.clone(), abort);
        cx.notify();

        let (progress_sender, mut progress_receiver) = mpsc::unbounded::<CommentPhotoUploadProgress>();
        let progress_temp_id = temp_id.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while let Some(progress) = progress_receiver.next().await {
                let _ = this.update(cx, |store, cx| store.apply_photo_upload_progress(&progress_temp_id, progress, cx));
            }
        })
        .detach();
        let on_progress = Arc::new(move |progress: CommentPhotoUploadProgress| {
            let _ = progress_sender.unbounded_send(progress);
        });

        let staged_draft = draft.clone();
        let staged = Self::actions(cx).update(cx, |actions, cx| actions.create_action(staged_draft, false, cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let _ = staged.await;
            let still_tracked =
                this.read_with(cx, |store, _| store.photo_upload_tasks.contains_key(&temp_id)).unwrap_or(false);
            if !still_tracked {
                cx.update(|cx| {
                    Self::actions(cx).update(cx, |actions, cx| actions.discard_create_action(&temp_id, cx));
                });
                return;
            }
            let uploaded =
                Abortable::new(RiseCommentPhotoUpload::upload(&item, &temp_id, on_progress, cx), registration).await;
            let Ok(uploaded) = uploaded else {
                return;
            };
            match uploaded {
                Ok(uploaded) => {
                    let complete = this.update(cx, |store, cx| -> Option<Task<RiseCommentCreateOutcome>> {
                        store.photo_upload_tasks.remove(&temp_id)?;
                        let total_bytes = store.photo_uploads.get(&temp_id).map_or(0, |upload| upload.total_bytes);
                        store.photo_uploads.insert(temp_id.clone(), CommentPhotoUpload::sent(total_bytes));
                        cx.notify();
                        let mut media = CommentMedia::from_photo(&uploaded);
                        if animated {
                            media.r#type = CommentMediaType::Gif;
                        }
                        Some(Self::actions(cx).update(cx, |actions, cx| actions.complete_create_action(&temp_id, media, cx)))
                    });
                    let Ok(Some(complete)) = complete else {
                        return;
                    };
                    let outcome = complete.await;
                    let _ = this.update(cx, |store, cx| store.finish_create(outcome, &temp_id, cx));
                }
                Err(error) => {
                    if error.is::<CancellationError>() {
                        return;
                    }
                    let _ = this.update(cx, |store, cx| store.photo_upload_failed(&draft, &error, cx));
                }
            }
        })
        .detach();
    }

    fn photo_upload_failed(&mut self, draft: &RiseCommentCreateDraft, error: &anyhow::Error, cx: &mut Context<Self>) {
        let temp_id = draft.temp_id.as_str();
        if !self.photo_upload_tasks.contains_key(temp_id) {
            return;
        }
        let message = format!("comment photo upload failed: {error}");
        Sai::log("Comment", move || message, SaiLogKind::Network, cx);
        self.photo_upload_tasks.remove(temp_id);
        self.photo_uploads.remove(temp_id);
        Self::actions(cx).update(cx, |actions, cx| actions.discard_create_action(temp_id, cx));
        let explained = RestrictionStore::shared(cx).update(cx, |restriction, cx| restriction.explains_error(error, cx));
        if !explained {
            Notifier::send(
                tr("comment_media_send_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    ..Default::default()
                },
                cx,
            );
        }
        self.restore_draft_after_failure_for_draft(draft, None, !explained, cx);
    }

    pub fn open_photo_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !Self::selected_post_can_comment(cx) {
            return;
        }
        resign_chat_keyboard(window, cx);
        self.photo_picker_visible = true;
        cx.notify();
    }

    pub fn thread_focus_reply_id(&self) -> Option<&str> {
        let (reply_id, _) = self.thread_focus_target.as_ref()?;
        if self.pending_scroll_reply_id.as_deref() != Some(reply_id.as_str()) {
            return None;
        }
        Some(reply_id.as_str())
    }

    pub fn thread_focus_maybe_load_more(&mut self, cx: &mut Context<Self>) {
        if self.thread_focus_reply_id().is_none() {
            return;
        }
        let Some(snapshot) = self.thread_snapshot(cx) else {
            return;
        };
        if snapshot.shows_initial_loader() || snapshot.is_loading_more {
            return;
        }
        if self.thread_focus_load_attempts >= FOCUS_LOAD_ATTEMPTS_LIMIT || !snapshot.has_more {
            self.thread_focus_target = None;
            self.pending_scroll_reply_id = None;
            cx.notify();
            return;
        }
        self.thread_focus_load_attempts += 1;
        self.load_more_replies(cx);
    }

    pub fn consume_pending_scroll_reply_id(&mut self, cx: &mut Context<Self>) {
        let revealed = self.pending_scroll_reply_id.take();
        cx.notify();
        let Some((reply_id, replying)) = self.thread_focus_target.clone() else {
            return;
        };
        if revealed.as_deref() != Some(reply_id.as_str()) {
            return;
        }
        self.thread_focus_target = None;
        self.highlight_focused(&reply_id, cx);
        if !replying {
            return;
        }
        let Some(reply) = self.thread_items(cx).into_iter().find(|item| item.id == reply_id) else {
            return;
        };
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let _ = this.update(cx, |store, cx| {
                if store.thread_root.is_some() {
                    store.start_reply(&reply, cx);
                }
            });
        })
        .detach();
    }

    pub fn consume_pending_scroll_comment_id(&mut self, cx: &mut Context<Self>) {
        self.pending_scroll_comment_id = None;
        cx.notify();
    }

    pub fn cancel_photo_upload(&mut self, temp_id: &str, cx: &mut Context<Self>) {
        let Some(task) = self.photo_upload_tasks.remove(temp_id) else {
            return;
        };
        task.abort();
        self.photo_uploads.remove(temp_id);
        cx.notify();
        Self::actions(cx).update(cx, |actions, cx| actions.discard_create_action(temp_id, cx));
    }

    pub fn forget_photo_upload(&mut self, temp_id: &str, cx: &mut Context<Self>) {
        if let Some(task) = self.photo_upload_tasks.remove(temp_id) {
            task.abort();
        }
        self.photo_uploads.remove(temp_id);
        cx.notify();
    }

    fn apply_photo_upload_progress(
        &mut self,
        temp_id: &str,
        progress: CommentPhotoUploadProgress,
        cx: &mut Context<Self>,
    ) {
        if !self.photo_upload_tasks.contains_key(temp_id) {
            return;
        }
        let Some(mut current) = self.photo_uploads.get(temp_id).copied() else {
            return;
        };
        match progress {
            CommentPhotoUploadProgress::Compressing(fraction) => {
                let percent = (fraction * 100.0).clamp(0.0, 100.0);
                if percent < 100.0 && percent - current.compression_progress < 3.0 {
                    return;
                }
                current.is_compressing = true;
                current.compression_progress = percent;
            }
            CommentPhotoUploadProgress::Uploading { completed_bytes, total_bytes } => {
                let percent =
                    if total_bytes > 0 { (completed_bytes as f64 / total_bytes as f64 * 100.0).min(100.0) } else { 0.0 };
                if !current.is_compressing
                    && completed_bytes < total_bytes
                    && percent - current.upload_progress < 3.0
                {
                    return;
                }
                current.is_compressing = false;
                current.compression_progress = 100.0;
                current.upload_progress = percent;
                current.uploaded_bytes = completed_bytes;
                current.total_bytes = total_bytes;
            }
        }
        self.photo_uploads.insert(temp_id.to_owned(), current);
        cx.notify();
    }

    fn pixel_size(item: &MediaPickerItem) -> Size<f32> {
        if let Some(payload) = &item.image_payload
            && payload.pixel_size.width > 0.0
            && payload.pixel_size.height > 0.0
        {
            return Size { width: payload.pixel_size.width, height: payload.pixel_size.height };
        }
        Self::file_pixel_size(item).unwrap_or(Size { width: 0.0, height: 0.0 })
    }

    fn file_pixel_size(item: &MediaPickerItem) -> Option<Size<f32>> {
        let path = item.url.to_file_path()?;
        let mut decoder = ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
        let (width, height) = decoder.dimensions();
        if width == 0 || height == 0 {
            return None;
        }
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let is_quarter_turn = matches!(
            orientation,
            Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH
        );
        Some(if is_quarter_turn {
            Size { width: height as f32, height: width as f32 }
        } else {
            Size { width: width as f32, height: height as f32 }
        })
    }

    fn current_parent_id(&self) -> Option<String> {
        self.thread_root.as_ref().map(|root| root.id.clone()).or_else(|| self.reply_root_id.clone())
    }

    pub fn send_current_comment_media(&mut self, media: Option<CommentMedia>, cx: &mut Context<Self>) {
        let parent_id = self.current_parent_id();
        self.submit_current_comment(media, parent_id, cx);
    }

    fn submit_current_comment(&mut self, media: Option<CommentMedia>, parent_id: Option<String>, cx: &mut Context<Self>) {
        let Some(draft) = self.stage_draft(media, parent_id, cx) else {
            return;
        };
        let temp_id = draft.temp_id.clone();
        let create = Self::actions(cx).update(cx, |actions, cx| actions.create_action(draft, true, cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = create.await;
            let _ = this.update(cx, |store, cx| store.finish_create(outcome, &temp_id, cx));
        })
        .detach();
        GuideSignalBus::emit(RiseGuideSignal::CommentCreated);
    }

    fn stage_draft(
        &mut self,
        media: Option<CommentMedia>,
        parent_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<RiseCommentCreateDraft> {
        let post_id = Self::selected_post_id(cx)?;
        let content = trimming_whitespaces_and_newlines(&self.comment_text).to_owned();
        if content.is_empty() && media.is_none() {
            return None;
        }

        let temp_id = format!("{}{}", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX, Uuid::new_v4());
        let profile = AuthServicesStore::shared(cx).read(cx).profile().cloned();
        let (list_sort, author) = {
            let service = Self::service(cx);
            let service = service.read(cx);
            (service.list_sort(), service.optimistic_author(profile.as_ref()))
        };
        let draft = RiseCommentCreateDraft {
            temp_id: temp_id.clone(),
            post_id: post_id.clone(),
            entities: self.composer_entities(&content),
            content,
            parent_id: parent_id.clone(),
            replies_sort: self.replies_sort,
            list_sort,
            author,
            media,
            custom_emoji_drafts: self.custom_emoji_drafts.clone(),
            reply_target_id: self.reply_target_id.clone(),
            reply_root_id: self.reply_root_id.clone(),
            reply_target_tag: self.reply_target_tag.clone(),
            reply_target_name: self.reply_target_name.clone(),
            reply_target_snippet: self.reply_target_snippet.clone(),
            reply_target_is_official: self.reply_target_is_official,
            reply_target_is_premium: self.reply_target_is_premium,
        };

        self.comment_text = String::new();
        self.custom_emoji_drafts.clear();
        let is_thread_reply = parent_id.is_some() && self.thread_root.as_ref().map(|root| &root.id) == parent_id.as_ref();
        if is_thread_reply {
            self.pending_scroll_reply_id = Some(temp_id);
        } else if parent_id.is_none() {
            self.pending_scroll_comment_id = Some(temp_id);
        }
        if self.failed_comment_retry.as_ref().is_some_and(|retry| retry.post_id == post_id) {
            self.failed_comment_retry = None;
        }
        self.cancel_reply(cx);
        cx.notify();
        Some(draft)
    }

    fn finish_create(&mut self, outcome: RiseCommentCreateOutcome, temp_id: &str, cx: &mut Context<Self>) {
        match outcome {
            RiseCommentCreateOutcome::Staged | RiseCommentCreateOutcome::Withdrawn => {}
            RiseCommentCreateOutcome::Confirmed { server_id, .. } => {
                self.forget_photo_upload(temp_id, cx);
                if !server_id.is_empty() {
                    if self.pending_scroll_reply_id.as_deref() == Some(temp_id) {
                        self.pending_scroll_reply_id = Some(server_id.clone());
                    }
                    if self.pending_scroll_comment_id.as_deref() == Some(temp_id) {
                        self.pending_scroll_comment_id = Some(server_id);
                    }
                }
                cx.notify();
            }
            RiseCommentCreateOutcome::Failed { draft, refusal } => {
                self.forget_photo_upload(temp_id, cx);
                let explained =
                    RestrictionStore::shared(cx).update(cx, |restriction, cx| restriction.explains(refusal.as_ref(), cx));
                if !explained {
                    let title = refusal.as_ref().and_then(CanonErrors::text_for).unwrap_or_else(|| {
                        tr(if draft.media.is_none() { "comment_send_failed" } else { "comment_media_send_failed" })
                    });
                    Notifier::send(
                        title,
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Top,
                            ..Default::default()
                        },
                        cx,
                    );
                }
                self.restore_draft_after_failure_for_draft(&draft, draft.media.clone(), !explained, cx);
            }
        }
    }

    fn restore_draft_after_failure_for_draft(
        &mut self,
        draft: &RiseCommentCreateDraft,
        media: Option<CommentMedia>,
        focuses: bool,
        cx: &mut Context<Self>,
    ) {
        self.restore_draft_after_failure(
            &draft.post_id,
            &draft.content,
            media,
            draft.custom_emoji_drafts.clone(),
            draft.reply_target_id.clone(),
            draft.reply_root_id.clone().or_else(|| draft.parent_id.clone()),
            draft.reply_target_tag.clone(),
            draft.reply_target_name.clone(),
            draft.reply_target_snippet.clone(),
            draft.reply_target_is_official,
            draft.reply_target_is_premium,
            focuses,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore_draft_after_failure(
        &mut self,
        post_id: &str,
        text: &str,
        media: Option<CommentMedia>,
        custom_emoji_drafts: Vec<ChatDraftCustomEmoji>,
        reply_target_id: Option<String>,
        reply_root_id: Option<String>,
        reply_target_tag: Option<String>,
        reply_target_name: Option<String>,
        reply_target_snippet: Option<String>,
        reply_target_is_official: bool,
        reply_target_is_premium: bool,
        focuses: bool,
        cx: &mut Context<Self>,
    ) {
        self.failed_comment_retry = Some(FailedCommentRetry {
            post_id: post_id.to_owned(),
            text: text.to_owned(),
            media,
            custom_emoji_drafts: custom_emoji_drafts.clone(),
            reply_target_id: reply_target_id.clone(),
            reply_root_id: reply_root_id.clone(),
            reply_target_tag: reply_target_tag.clone(),
            reply_target_name: reply_target_name.clone(),
            reply_target_snippet: reply_target_snippet.clone(),
            reply_target_is_official,
            reply_target_is_premium,
        });
        cx.notify();
        if Self::selected_post_id(cx).as_deref() != Some(post_id) {
            return;
        }
        if self.comment_text.is_empty() {
            self.set_comment_text(text, cx);
            self.custom_emoji_drafts = custom_emoji_drafts;
        }
        if self.reply_target_id.is_none() {
            self.reply_target_id = reply_target_id;
            self.reply_root_id = reply_root_id.or_else(|| self.thread_root.as_ref().map(|root| root.id.clone()));
            self.reply_target_tag = reply_target_tag;
            self.reply_target_name = reply_target_name;
            self.reply_target_snippet = reply_target_snippet;
            self.reply_target_is_official = reply_target_is_official;
            self.reply_target_is_premium = reply_target_is_premium;
        }
        cx.notify();
        if !focuses || text.is_empty() {
            return;
        }
        self.focus_bump_signal = self.focus_bump_signal.wrapping_add(1);
        cx.notify();
    }

    pub fn has_failed_draft(&self, post_id: &str) -> bool {
        self.failed_comment_retry.as_ref().is_some_and(|retry| retry.post_id == post_id)
    }

    pub fn failed_draft_media(&self, post_id: &str) -> Option<CommentMedia> {
        self.failed_comment_retry.as_ref().filter(|retry| retry.post_id == post_id).and_then(|retry| retry.media.clone())
    }

    pub fn retry_failed_draft(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let Some(retry) = self.failed_comment_retry.clone().filter(|retry| retry.post_id == post_id) else {
            return;
        };
        if Self::selected_post_id(cx).as_deref() != Some(post_id) {
            return;
        }
        let current_text = self.comment_text.clone();
        let current_custom_emoji_drafts = self.custom_emoji_drafts.clone();
        let current_target_id = self.reply_target_id.clone();
        let current_root_id = self.reply_root_id.clone();
        let current_target_tag = self.reply_target_tag.clone();
        let current_target_name = self.reply_target_name.clone();
        let current_target_snippet = self.reply_target_snippet.clone();
        let current_target_is_official = self.reply_target_is_official;
        let current_target_is_premium = self.reply_target_is_premium;
        let preserves_current_draft = current_text != retry.text
            || current_target_id != retry.reply_target_id
            || current_root_id != retry.reply_root_id;

        self.set_comment_text(&retry.text, cx);
        self.custom_emoji_drafts = retry.custom_emoji_drafts;
        self.reply_target_id = retry.reply_target_id;
        self.reply_root_id = retry.reply_root_id.clone();
        self.reply_target_tag = retry.reply_target_tag;
        self.reply_target_name = retry.reply_target_name;
        self.reply_target_snippet = retry.reply_target_snippet;
        self.reply_target_is_official = retry.reply_target_is_official;
        self.reply_target_is_premium = retry.reply_target_is_premium;
        self.submit_current_comment(retry.media, retry.reply_root_id, cx);

        if preserves_current_draft {
            self.set_comment_text(&current_text, cx);
            self.custom_emoji_drafts = current_custom_emoji_drafts;
            self.reply_target_id = current_target_id;
            self.reply_root_id = current_root_id;
            self.reply_target_tag = current_target_tag;
            self.reply_target_name = current_target_name;
            self.reply_target_snippet = current_target_snippet;
            self.reply_target_is_official = current_target_is_official;
            self.reply_target_is_premium = current_target_is_premium;
        }
        cx.notify();
    }

    pub fn dismiss_failed_draft(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if !self.has_failed_draft(post_id) {
            return;
        }
        self.failed_comment_retry = None;
        cx.notify();
    }

    pub fn set_reaction(&mut self, comment: &GetCommentsResponse, desired: CommentReaction, cx: &mut Context<Self>) {
        if comment.id.is_empty() || comment.is_temp || comment.is_moderation_disabled() {
            return;
        }
        let comment_id = comment.id.clone();
        Self::actions(cx).update(cx, |actions, cx| actions.set_desired_reaction_action(&comment_id, desired, cx));
        let action_comment_id = comment_id.clone();
        let invalidate_comment_id = comment_id.clone();
        SaiDebouncer::debounced_action_delay_milliseconds(
            comment_id,
            Box::new(move |cx: &mut App| {
                Self::actions(cx).update(cx, |actions, cx| actions.flush_reaction_action(&action_comment_id, cx));
            }),
            Self::reaction_debounce().client_debounce_ms,
            COMMENT_REACTION_GROUP,
            COMMENT_REACTION_MAX_CACHE_DATA,
            Some(Box::new(move |cx: &mut App| {
                Self::actions(cx).update(cx, |actions, cx| actions.flush_reaction_action(&invalidate_comment_id, cx));
            })),
            cx,
        );
    }

    pub fn toggle_like_on_comment(&mut self, comment: &GetCommentsResponse, cx: &mut Context<Self>) {
        let desired =
            if comment.reaction() == CommentReaction::Like { CommentReaction::None } else { CommentReaction::Like };
        self.set_reaction(comment, desired, cx);
    }

    pub fn like_comment_from_double_tap(&mut self, comment: &GetCommentsResponse, cx: &mut Context<Self>) {
        if comment.is_temp || comment.is_moderation_disabled() || comment.reaction() == CommentReaction::Like {
            return;
        }
        self.set_reaction(comment, CommentReaction::Like, cx);
    }

    pub fn toggle_dislike_on_comment(&mut self, comment: &GetCommentsResponse, cx: &mut Context<Self>) {
        let desired = if comment.reaction() == CommentReaction::Dislike {
            CommentReaction::None
        } else {
            CommentReaction::Dislike
        };
        self.set_reaction(comment, desired, cx);
    }

    pub fn load_user_comments_if_needed(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        let scope = RiseCommentScope::User { user_id: user_id.to_owned() };
        Self::actions(cx).update(cx, |actions, cx| actions.open_list_action(scope, false, None, None, cx));
    }

    pub fn reload_user_comments(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        let scope = RiseCommentScope::User { user_id: user_id.to_owned() };
        Self::actions(cx).update(cx, |actions, cx| actions.open_list_action(scope, true, None, None, cx));
    }

    pub fn load_more_user_comments(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        let scope = RiseCommentScope::User { user_id: user_id.to_owned() };
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_action(scope, cx));
    }

    pub fn observe_user_comments(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        let scope = RiseCommentScope::User { user_id: user_id.to_owned() };
        Self::actions(cx).update(cx, |actions, cx| actions.begin_observing_action(scope, cx));
    }

    pub fn stop_observing_user_comments(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        let scope = RiseCommentScope::User { user_id: user_id.to_owned() };
        Self::actions(cx).update(cx, |actions, cx| actions.end_observing_action(scope, cx));
    }

    pub fn apply_moderation_status(&mut self, comment_id: &str, status: &str, cx: &mut Context<Self>) {
        if comment_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.apply_moderation_status_action(comment_id, status, cx));
    }

    pub fn delete_user_comment(&mut self, comment_id: &str, cx: &mut Context<Self>) {
        if comment_id.is_empty() {
            return;
        }
        let comment_id = comment_id.to_owned();
        HoldContextMenuPresenter::after_dismiss(
            move |cx: &mut App| {
                NativeConfirmAlert::present_delete(
                    &tr("comment_delete_confirm_title"),
                    &tr("comment_delete_confirm_message"),
                    None,
                    None,
                    false,
                    None,
                    None,
                    move |cx: &mut App| {
                        Self::actions(cx).update(cx, |actions, cx| actions.delete_comment_action(&comment_id, cx));
                    },
                    cx,
                );
            },
            cx,
        );
    }

    pub fn delete_selected_comment(&mut self, cx: &mut Context<Self>) {
        let Some(comment) = self.selected_comment.take() else {
            return;
        };
        cx.notify();
        let this = cx.weak_entity();
        HoldContextMenuPresenter::after_dismiss(
            move |cx: &mut App| {
                if this.upgrade().is_none() {
                    return;
                }
                NativeConfirmAlert::present_delete(
                    &tr("comment_delete_confirm_title"),
                    &tr("comment_delete_confirm_message"),
                    None,
                    None,
                    false,
                    None,
                    None,
                    move |cx: &mut App| {
                        Self::actions(cx).update(cx, |actions, cx| actions.delete_comment_action(&comment.id, cx));
                        let _ = this.update(cx, |store, cx| {
                            if store.thread_root.as_ref().is_some_and(|root| root.id == comment.id) {
                                store.close_thread(cx);
                            }
                        });
                    },
                    cx,
                );
            },
            cx,
        );
    }

    pub fn comments_did_change(&mut self, comments: &[GetCommentsResponse], cx: &mut Context<Self>) {
        let Some(root) = &self.thread_root else {
            return;
        };
        let Some(fresh) = comments.iter().find(|comment| comment.id == root.id) else {
            return;
        };
        if fresh == root {
            return;
        }
        self.thread_root = Some(fresh.clone());
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        SaiDebouncer::cancel_all_in_group(COMMENT_REACTION_GROUP, cx);
        self.observed_list_scope = None;
        self.observed_thread_scope = None;
        self.pending_scroll_reply_id = None;
        self.pending_scroll_comment_id = None;
        self.photo_picker_visible = false;
        for task in self.photo_upload_tasks.values() {
            task.abort();
        }
        self.photo_upload_tasks.clear();
        self.photo_uploads.clear();
        self.comments_sheet_visible = false;
        self.comment_text = String::new();
        self.custom_emoji_drafts.clear();
        self.draft_post_id = None;
        self.post_drafts.clear();
        self.cancel_reply(cx);
        self.thread_root = None;
        self.replies_sort = RepliesSortType::Old;
        self.failed_comment_retry = None;
        self.focus_bump_signal = 0;
        self.selected_comment = None;
        self.pending_focus_post_id = None;
        self.pending_focus_comment_id = None;
        self.pending_focus_replying = false;
        self.focus_target = None;
        self.thread_focus_target = None;
        self.focus_comment_id = None;
        self.clear_comment_highlight(cx);
        self.focus_load_attempts = 0;
        self.sort_menu_visible = false;
        cx.notify();
    }
}

impl CommentInteractionsStore {
    pub fn expanded_comment_ids(&self) -> &HashSet<String> {
        &self.expanded_comment_ids
    }

    pub fn set_comment_text_expanded(&mut self, comment_id: &str, expanded: bool, cx: &mut Context<Self>) {
        if self.expanded_comment_ids.contains(comment_id) == expanded {
            return;
        }
        if expanded {
            self.expanded_comment_ids.insert(comment_id.to_owned());
        } else {
            self.expanded_comment_ids.remove(comment_id);
        }
        cx.notify();
    }
}

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, Entity, Task};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_limits::LimitSpec;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatComposerCustomEmojiRun, ChatDraftCustomEmoji,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_interactions::chat_folders_interactions::ChatFolderOnResult;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageCustomEmojiEntity, RemoteMediaURL};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{CustomEmojiEntityWire, MessageEntitiesCodec};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    StickerAssetType, StickerAssetTypeInfer, StickerItem,
};

const OBJECT_REPLACEMENT: &str = "\u{FFFC}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddMode {
    Included,
    Excluded,
}

pub struct ChatFolderEditorStore {
    is_presented: bool,
    editing_folder_id: Option<String>,
    pub title: String,
    custom_emoji_drafts: Vec<ChatDraftCustomEmoji>,
    pub color: Option<String>,
    pub included_chat_ids: Vec<String>,
    pub included_chat_types: Vec<String>,
    pub excluded_chat_ids: Vec<String>,
    pub excluded_chat_types: Vec<String>,
    pub add_mode: Option<AddMode>,
    pub is_saving: bool,
    selected_chat_models: HashMap<String, ChatInfo>,
    baseline: String,
}

impl ChatFolderEditorStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            is_presented: false,
            editing_folder_id: None,
            title: String::new(),
            custom_emoji_drafts: Vec::new(),
            color: None,
            included_chat_ids: Vec::new(),
            included_chat_types: Vec::new(),
            excluded_chat_ids: Vec::new(),
            excluded_chat_types: Vec::new(),
            add_mode: None,
            is_saving: false,
            selected_chat_models: HashMap::new(),
            baseline: String::new(),
        })
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn editing_folder_id(&self) -> Option<&str> {
        self.editing_folder_id.as_deref()
    }

    pub fn selected_chat_models(&self) -> &HashMap<String, ChatInfo> {
        &self.selected_chat_models
    }

    fn report_guide_facts(&self, cx: &mut App) {
        let has_chats = !self.included_chat_ids.is_empty() || !self.included_chat_types.is_empty();
        let trimmed_title_filled = !self.trimmed_title().is_empty();
        let can_save = self.can_save();
        GuideFacts::set(RiseGuideSignal::FolderEditorOpen, self.is_presented, cx);
        GuideFacts::set(RiseGuideSignal::FolderNameFilled, self.is_presented && trimmed_title_filled, cx);
        GuideFacts::set(
            RiseGuideSignal::FolderChatsPickerOpen,
            self.is_presented && self.add_mode == Some(AddMode::Included),
            cx,
        );
        GuideFacts::set(RiseGuideSignal::FolderHasChats, self.is_presented && has_chats, cx);
        GuideFacts::set(RiseGuideSignal::FolderCanSave, self.is_presented && self.add_mode.is_none() && can_save, cx);
    }

    pub fn set_title(&mut self, title: String, cx: &mut Context<Self>) {
        self.title = title;
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn set_color(&mut self, color: Option<String>, cx: &mut Context<Self>) {
        self.color = color;
        cx.notify();
    }

    pub fn set_included_chat_ids(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.included_chat_ids = ids;
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn set_included_chat_types(&mut self, types: Vec<String>, cx: &mut Context<Self>) {
        self.included_chat_types = types;
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn set_excluded_chat_ids(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.excluded_chat_ids = ids;
        cx.notify();
    }

    pub fn set_excluded_chat_types(&mut self, types: Vec<String>, cx: &mut Context<Self>) {
        self.excluded_chat_types = types;
        cx.notify();
    }

    fn selected_ids(&self) -> Vec<String> {
        let unique: HashSet<&String> = self.included_chat_ids.iter().chain(self.excluded_chat_ids.iter()).collect();
        let mut ids: Vec<String> = unique.into_iter().cloned().collect();
        ids.sort();
        ids
    }

    pub fn resolve_selected_chat_models(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let ids = self.selected_ids();
        let active = RiseConversationDomainRegistry::shared(cx).read(cx).active();
        let Some(domain) = active.filter(|_| self.is_presented) else {
            self.selected_chat_models.clear();
            cx.notify();
            return Task::ready(());
        };
        let epoch = SaiAccountCacheScope::active_epoch();
        let work = {
            let domain = domain.clone();
            let ids = ids.clone();
            RiseAccountRuntime::current().spawn(async move { domain.chats.selected_models(&ids).await })
        };
        cx.spawn(async move |this, cx| {
            let Ok(Ok(models)) = work.await else {
                return;
            };
            let _ = this.update(cx, |store, cx| {
                let registry_active = RiseConversationDomainRegistry::shared(cx).read(cx).active();
                let is_active = registry_active.is_some_and(|active| Arc::ptr_eq(&active, &domain));
                let is_account =
                    SaiAccountCacheScope::is_current_account_epoch(Some(domain.account_id.as_str()), epoch.as_deref());
                if !is_active || !is_account || !store.is_presented || ids != store.selected_ids() {
                    return;
                }
                store.selected_chat_models = models.chats;
                cx.notify();
            });
        })
    }

    pub fn title_rule() -> RiseFieldRule {
        RiseFieldRule::new("chat_create_chat_folder", "/title")
    }

    pub fn is_editing(&self) -> bool {
        self.editing_folder_id.is_some()
    }

    pub fn trimmed_title(&self) -> String {
        match Self::title_rule().schema {
            Some(schema) => schema.normalized_clamped(&self.title),
            None => trimming_whitespaces_and_newlines(&self.title).to_owned(),
        }
    }

    pub fn can_save(&self) -> bool {
        !self.trimmed_title().is_empty() && (!self.included_chat_ids.is_empty() || !self.included_chat_types.is_empty())
    }

    pub fn is_dirty(&self) -> bool {
        self.snapshot_string() != self.baseline
    }

    fn folders_limit() -> Option<&'static LimitSpec> {
        rise_limits::spec("chat_folders")
    }

    fn folder_chats_limit() -> Option<&'static LimitSpec> {
        rise_limits::spec("chat_folder_chats")
    }

    fn refuses_new_folder(&self, cx: &mut App) -> bool {
        if GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_folders()) {
            return false;
        }
        let count = ChatServicesStore::shared(cx).read(cx).folders.len();
        self.refuses_past_tier_limit(Self::folders_limit(), count, cx)
    }

    fn refuses_past_tier_limit(&self, spec: Option<&'static LimitSpec>, count: usize, cx: &mut App) -> bool {
        let Some(spec) = spec else {
            return false;
        };
        let is_premium =
            AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        let limit = if is_premium { spec.premium.unwrap_or(spec.value) } else { spec.value };
        if (count as u64) < limit {
            return false;
        }
        Notifier::send(
            tr("server_error_limit_reached"),
            NotifierSendOptions {
                content: Some(format!("{}: {}", tr(spec.i18n_key), limit)),
                r#type: NotifierType::Warning,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("folder.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
        true
    }

    pub fn present_create(&mut self, cx: &mut Context<Self>) {
        if self.refuses_new_folder(cx) {
            return;
        }
        self.editing_folder_id = None;
        self.title = String::new();
        self.custom_emoji_drafts = Vec::new();
        self.color = None;
        self.included_chat_ids = Vec::new();
        self.included_chat_types = Vec::new();
        self.excluded_chat_ids = Vec::new();
        self.excluded_chat_types = Vec::new();
        self.reset_sub_sheets();
        self.baseline = self.snapshot_string();
        self.is_presented = true;
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn present_edit(&mut self, folder: &ChatFolder, cx: &mut Context<Self>) {
        self.editing_folder_id = Some(folder.id.clone());
        self.title = folder.title.clone();
        self.custom_emoji_drafts = Self::drafts(&folder.title_custom_emojis(), &folder.title);
        self.color = folder.color.clone();
        self.included_chat_ids = folder.included_chat_ids.clone();
        self.included_chat_types = folder.included_chat_types.clone();
        self.excluded_chat_ids = folder.excluded_chat_ids.clone();
        self.excluded_chat_types = folder.excluded_chat_types.clone();
        self.reset_sub_sheets();
        self.baseline = self.snapshot_string();
        self.is_presented = true;
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.selected_chat_models.clear();
        self.is_presented = false;
        self.reset_sub_sheets();
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.selected_chat_models.clear();
        self.is_presented = false;
        self.editing_folder_id = None;
        self.title = String::new();
        self.custom_emoji_drafts = Vec::new();
        self.color = None;
        self.included_chat_ids = Vec::new();
        self.included_chat_types = Vec::new();
        self.excluded_chat_ids = Vec::new();
        self.excluded_chat_types = Vec::new();
        self.reset_sub_sheets();
        self.baseline = String::new();
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn reopen(&mut self, cx: &mut Context<Self>) {
        self.is_presented = true;
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn save(&mut self, on_done: Rc<dyn Fn(&mut App)>, cx: &mut Context<Self>) {
        if !self.can_save() || self.is_saving {
            return;
        }
        self.is_saving = true;
        cx.notify();
        let folder = self.build_folder(cx);
        let is_editing = self.is_editing();
        let store = cx.entity();
        let finish: ChatFolderOnResult = Rc::new(move |ok: bool, cx: &mut App| {
            store.update(cx, |store, cx| {
                store.is_saving = false;
                if !ok {
                    Notifier::send(
                        tr("save_failed"),
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Top,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                    if !store.is_presented {
                        store.reopen(cx);
                    }
                }
                cx.notify();
            });
            if ok {
                on_done(cx);
            }
        });
        cx.defer(move |cx| {
            let interactions = ChatInteractionsStore::shared(cx);
            if is_editing {
                interactions.update(cx, |interactions, cx| interactions.update_folder(folder, finish, cx));
            } else {
                let created: ChatFolderOnResult = Rc::new(move |ok: bool, cx: &mut App| {
                    if ok {
                        GuideSignalBus::emit(RiseGuideSignal::FolderSaved);
                    }
                    finish(ok, cx);
                });
                interactions.update(cx, |interactions, cx| interactions.create_folder(folder, created, cx));
            }
        });
    }

    pub fn insert_emoji(&mut self, emoji: &str, selection: Option<NSRange>, cx: &mut Context<Self>) -> Option<NSRange> {
        if emoji.is_empty() || Self::title_rule().clamped(&format!("{}{}", self.title, emoji)) == self.title {
            return None;
        }
        let edit = ChatComposerCaretEdit::replace(&self.title, selection, emoji, &mut self.custom_emoji_drafts, None);
        Some(self.apply_caret_edit(edit, cx))
    }

    pub fn append_custom_emoji(
        &mut self,
        item: StickerItem,
        selection: Option<NSRange>,
        cx: &mut Context<Self>,
    ) -> Option<NSRange> {
        if item.id.is_empty() || item.file_url.is_empty() {
            return None;
        }
        if Self::title_rule().clamped(&format!("{}{}", self.title, OBJECT_REPLACEMENT)) == self.title {
            return None;
        }
        let edit = ChatComposerCaretEdit::replace(
            &self.title,
            selection,
            OBJECT_REPLACEMENT,
            &mut self.custom_emoji_drafts,
            Some(ChatDraftCustomEmoji { placeholder: OBJECT_REPLACEMENT.to_owned(), item }),
        );
        Some(self.apply_caret_edit(edit, cx))
    }

    pub fn delete_backward(&mut self, selection: Option<NSRange>, cx: &mut Context<Self>) -> Option<NSRange> {
        let edit = ChatComposerCaretEdit::delete_backward(&self.title, selection, &mut self.custom_emoji_drafts)?;
        Some(self.apply_caret_edit(edit, cx))
    }

    fn apply_caret_edit(&mut self, edit: ChatComposerCaretEditResult, cx: &mut Context<Self>) -> NSRange {
        self.set_title(edit.text, cx);
        edit.caret
    }

    pub fn composer_custom_emoji_runs(&self, text: &str) -> Vec<ChatComposerCustomEmojiRun> {
        ChatComposerCaretEdit::custom_emoji_runs(text, &self.custom_emoji_drafts)
    }

    pub fn sync_custom_emoji_drafts(&mut self, runs: Vec<ChatComposerCustomEmojiRun>, cx: &mut Context<Self>) {
        let unchanged = runs.len() == self.custom_emoji_drafts.len()
            && runs.iter().zip(&self.custom_emoji_drafts).all(|(run, draft)| run.item == draft.item);
        if unchanged {
            return;
        }
        self.custom_emoji_drafts = runs
            .into_iter()
            .map(|run| ChatDraftCustomEmoji { placeholder: OBJECT_REPLACEMENT.to_owned(), item: run.item })
            .collect();
        cx.notify();
    }

    fn title_entities_json(&self, trimmed_title: &str) -> Option<String> {
        let wires: Vec<CustomEmojiEntityWire> = self
            .composer_custom_emoji_runs(trimmed_title)
            .into_iter()
            .map(|run| CustomEmojiEntityWire {
                offset: run.range.location as i64,
                length: run.range.length as i64,
                custom_emoji_id: run.item.id,
                file_url: RemoteMediaURL::optional_normalized(Some(run.item.file_url.as_str()))
                    .unwrap_or(run.item.file_url),
                asset_type: run.item.asset_type.wire().to_owned(),
                width: run.item.width,
                height: run.item.height,
                pack_id: run.item.pack_id,
            })
            .collect();
        if wires.is_empty() {
            return None;
        }
        MessageEntitiesCodec::encode(&wires, None)
    }

    fn drafts(entities: &[MessageCustomEmojiEntity], text: &str) -> Vec<ChatDraftCustomEmoji> {
        if entities.is_empty() || !text.contains(OBJECT_REPLACEMENT) {
            return Vec::new();
        }
        let mut sorted: Vec<&MessageCustomEmojiEntity> = entities.iter().collect();
        sorted.sort_by_key(|entity| entity.offset);
        sorted
            .into_iter()
            .map(|entity| ChatDraftCustomEmoji {
                placeholder: OBJECT_REPLACEMENT.to_owned(),
                item: StickerItem {
                    id: entity.custom_emoji_id.clone(),
                    pack_id: entity.pack_id.clone(),
                    file_url: entity.file_url.clone(),
                    thumbnail_url: None,
                    width: entity.width,
                    height: entity.height,
                    duration_ms: None,
                    position: 0,
                    created_at_ms: 0,
                    associated_emojis: Vec::new(),
                    asset_type: StickerAssetType::infer(Some(entity.asset_type.as_str()), &entity.file_url),
                    mime_type: None,
                    codec: None,
                },
            })
            .collect()
    }

    pub fn open_add_chats(&mut self, mode: AddMode, cx: &mut Context<Self>) {
        self.add_mode = Some(mode);
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn close_add_chats(&mut self, cx: &mut Context<Self>) {
        self.add_mode = None;
        self.report_guide_facts(cx);
        cx.notify();
    }

    pub fn is_selected_chat(&self, id: &str) -> bool {
        match self.add_mode {
            Some(AddMode::Included) => self.included_chat_ids.iter().any(|chat_id| chat_id == id),
            Some(AddMode::Excluded) => self.excluded_chat_ids.iter().any(|chat_id| chat_id == id),
            None => false,
        }
    }

    pub fn toggle_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.add_mode {
            Some(AddMode::Included) => {
                let is_selected = self.included_chat_ids.iter().any(|chat_id| chat_id == id);
                let count = self.included_chat_ids.len();
                if !is_selected && self.refuses_past_tier_limit(Self::folder_chats_limit(), count, cx) {
                    return;
                }
                Self::toggle(&mut self.included_chat_ids, id, false);
                self.report_guide_facts(cx);
            }
            Some(AddMode::Excluded) => Self::toggle(&mut self.excluded_chat_ids, id, false),
            None => {}
        }
        cx.notify();
    }

    pub fn is_selected_type(&self, raw: &str) -> bool {
        match self.add_mode {
            Some(AddMode::Included) => self.included_chat_types.iter().any(|value| value == raw),
            Some(AddMode::Excluded) => self.excluded_chat_types.iter().any(|value| value == raw),
            None => false,
        }
    }

    pub fn toggle_type(&mut self, raw: &str, cx: &mut Context<Self>) {
        match self.add_mode {
            Some(AddMode::Included) => {
                Self::toggle(&mut self.included_chat_types, raw, false);
                self.report_guide_facts(cx);
            }
            Some(AddMode::Excluded) => Self::toggle(&mut self.excluded_chat_types, raw, false),
            None => {}
        }
        cx.notify();
    }

    pub fn active_selected_chat_ids(&self) -> Vec<String> {
        match self.add_mode {
            Some(AddMode::Included) => self.included_chat_ids.clone(),
            Some(AddMode::Excluded) => self.excluded_chat_ids.clone(),
            None => Vec::new(),
        }
    }

    pub fn active_selected_types(&self) -> Vec<String> {
        match self.add_mode {
            Some(AddMode::Included) => self.included_chat_types.clone(),
            Some(AddMode::Excluded) => self.excluded_chat_types.clone(),
            None => Vec::new(),
        }
    }

    pub fn add_mode_title_key(&self) -> &'static str {
        match self.add_mode {
            Some(AddMode::Excluded) => "folder_exceptions_sheet_title",
            _ => "folder_chats_sheet_title",
        }
    }

    pub fn remove_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        Self::toggle(&mut self.included_chat_ids, id, true);
        Self::toggle(&mut self.excluded_chat_ids, id, true);
        self.report_guide_facts(cx);
        cx.notify();
    }

    fn build_folder(&self, cx: &mut App) -> ChatFolder {
        let editing_id = self.editing_folder_id.clone().unwrap_or_default();
        let services = ChatServicesStore::shared(cx);
        let existing = services.read(cx).folder(&editing_id, cx);
        let pinned_chat_ids = existing.as_ref().map_or_else(Vec::new, |folder| folder.pinned_chat_ids.clone());
        let position = existing.as_ref().map_or(0, |folder| folder.position);
        let saved_title = self.trimmed_title();
        let title_entities = self.title_entities_json(&saved_title);
        ChatFolder {
            id: editing_id,
            title: saved_title,
            title_entities,
            icon: None,
            color: self.color.clone(),
            included_chat_ids: self.included_chat_ids.clone(),
            included_chat_types: self.included_chat_types.clone(),
            excluded_chat_ids: self.excluded_chat_ids.clone(),
            excluded_chat_types: self.excluded_chat_types.clone(),
            pinned_chat_ids,
            position,
            ..ChatFolder::default()
        }
    }

    fn reset_sub_sheets(&mut self) {
        self.add_mode = None;
        self.is_saving = false;
    }

    fn snapshot_string(&self) -> String {
        fn sorted_joined(values: &[String]) -> String {
            let mut sorted = values.to_vec();
            sorted.sort();
            sorted.join(",")
        }
        [
            self.title.clone(),
            self.custom_emoji_drafts.iter().map(|draft| draft.item.id.as_str()).collect::<Vec<_>>().join(","),
            self.color.clone().unwrap_or_default(),
            sorted_joined(&self.included_chat_ids),
            sorted_joined(&self.included_chat_types),
            sorted_joined(&self.excluded_chat_ids),
            sorted_joined(&self.excluded_chat_types),
        ]
        .join("|")
    }

    fn toggle(values: &mut Vec<String>, value: &str, removing_only: bool) {
        if let Some(index) = values.iter().position(|existing| existing == value) {
            values.remove(index);
        } else if !removing_only {
            values.push(value.to_owned());
        }
    }
}

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_canon::rise_key::bot as key;
use rise_canon::rise_request::bot::ConfirmMiniAppLoaded;
use rise_canon::{RiseEvent, RiseLimits, rise_wire_methods};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_events::name;
use rise_i18n::{LanguageStore, tr};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::{SaiCacheCleanLayer, SaiCacheCleaner};
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::{
    SaiWsOptimisticError, SaiWsOptimisticFiles, SaiWsOptimisticLocalFile,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::BotFieldRules;
use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::{
    BotInlineCommandsModel, BotInlineIdentityWireDTOExt, BotInlineResultWireDTOExt, BotInlineResultsPage,
    BotMentionCandidateModel, BotMentionCandidateWireDTOExt,
};
use crate::modules::bot::stores::bot::bot_actions::bot_inline_wire_types::{
    BotChatCommandsGatewayDTO, BotInlineCommandsGatewayDTO, BotInlineResultsGatewayDTO, BotMentionCandidatesGatewayDTO,
};
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotCallbackAnswerModel, BotCommandModel, BotMediaKind, BotMiniAppModel, BotMiniAppUpdateValues, BotModel,
    BotPublicCommandsModel, BotUpdateValues, BotWebhookInfoModel,
};
use crate::modules::bot::stores::bot::bot_actions::bot_wire_types::{
    BotCommandWireDTOExt, BotCommandsGatewayDTO, BotGatewayDTO, BotListGatewayDTO, BotMiniAppGatewayDTO,
    BotMiniAppLaunchGatewayDTO, BotMiniAppListGatewayDTO, BotMiniAppWireDTO, BotMiniAppWireDTOExt,
    BotPublicCommandsGatewayDTO, BotPublicCommandsGatewayDTOExt, BotStatusGatewayDTO, BotWebhookGatewayDTO,
    BotWebhookInfoWireDTOExt, BotWireDTO, BotWireDTOExt,
};
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::music::shared::utils::music::music_cover_upload::MusicCoverUpload;

use super::bot_guide_sandbox_hooks::bot_guide_sandbox_hooks;

const IS_TEMP: &str = "is_temp";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(30);
const EARLY_CALLBACK_ANSWERS_CAPACITY: usize = 128;
const INLINE_QUERY_MAX_CHARACTERS: usize = 64;
const INLINE_RESULT_QUERY_MAX_CHARACTERS: usize = 256;
const MINI_APP_LOAD_RETRY_MAX_EXPONENT: i64 = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct BotCacheDocumentSnapshot {
    pub cache_key: String,
    pub payload: Option<Map<String, Value>>,
}

#[derive(Clone)]
struct MiniAppLoadReceipt {
    token: String,
    user_id: String,
    expires_at: i64,
    page_loaded: bool,
    attempts: i64,
}

#[derive(Default)]
pub struct BotServicesStore {
    mini_app_loads: HashMap<String, MiniAppLoadReceipt>,
    mini_app_load_requests: HashMap<String, Rc<SaiTypedSlice<BotStatusGatewayDTO>>>,
    mini_app_load_retries: HashMap<String, Task<()>>,
    pub media_upload_progress_by_key: HashMap<String, f64>,
    pub active_media_upload_keys: HashSet<String>,
    pub pending_callback_scopes: HashSet<String>,
    pub callback_answers_by_scope: HashMap<String, BotCallbackAnswerModel>,
    pub callback_scope_by_id: HashMap<String, String>,
    pub early_callback_answers_by_id: HashMap<String, BotCallbackAnswerModel>,
    pub callback_timeout_tasks: HashMap<String, Task<()>>,
    pub inline_composer_generation_by_scope: HashMap<String, i64>,
    pub inline_composer_pending_key_by_scope: HashMap<String, String>,
    pub inline_composer_loading_keys: HashSet<String>,
    pub inline_composer_errors_by_key: HashMap<String, String>,
    pub sending_inline_result_keys: HashSet<String>,
    pub inline_composer_tasks: HashMap<String, Task<()>>,
}

impl BotServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self::default()
    }

    pub fn page_limit() -> i64 {
        RiseLimits::client_value("bot_services_store_page_limit")
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.mini_app_load_retries.clear();
        self.mini_app_load_requests.clear();
        self.mini_app_loads.clear();
        self.media_upload_progress_by_key.clear();
        self.active_media_upload_keys.clear();
        self.pending_callback_scopes.clear();
        self.callback_answers_by_scope.clear();
        self.callback_scope_by_id.clear();
        self.early_callback_answers_by_id.clear();
        self.callback_timeout_tasks.clear();
        self.inline_composer_tasks.clear();
        self.inline_composer_generation_by_scope.clear();
        self.inline_composer_pending_key_by_scope.clear();
        self.inline_composer_loading_keys.clear();
        self.inline_composer_errors_by_key.clear();
        self.sending_inline_result_keys.clear();
        cx.notify();
    }

    pub fn begin_callback(&mut self, scope: &str, cx: &mut Context<Self>) {
        if scope.is_empty() {
            return;
        }
        self.callback_timeout_tasks.remove(scope);
        self.callback_answers_by_scope.remove(scope);
        self.pending_callback_scopes.insert(scope.to_owned());
        let timed_out_scope = scope.to_owned();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CALLBACK_TIMEOUT).await;
            let _ = this.update(cx, |store, cx| store.fail_callback(&timed_out_scope, cx));
        });
        self.callback_timeout_tasks.insert(scope.to_owned(), task);
        cx.notify();
    }

    pub fn fail_callback(&mut self, scope: &str, cx: &mut Context<Self>) {
        if !self.pending_callback_scopes.contains(scope) {
            return;
        }
        self.finish_callback(scope, false, cx);
        Notifier::send(
            tr("bot_callback_failed"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..Default::default()
            },
            cx,
        );
    }

    pub fn bind_callback(&mut self, scope: &str, callback_query_id: &str, cx: &mut Context<Self>) {
        if !self.pending_callback_scopes.contains(scope) || callback_query_id.is_empty() {
            return;
        }
        self.callback_scope_by_id.insert(callback_query_id.to_owned(), scope.to_owned());
        if let Some(answer) = self.early_callback_answers_by_id.remove(callback_query_id) {
            self.callback_answers_by_scope.insert(scope.to_owned(), answer);
            self.finish_callback(scope, true, cx);
        }
        cx.notify();
    }

    pub fn finish_callback(&mut self, scope: &str, keep_answer: bool, cx: &mut Context<Self>) {
        self.pending_callback_scopes.remove(scope);
        self.callback_timeout_tasks.remove(scope);
        self.callback_scope_by_id.retain(|_, value| value != scope);
        if !keep_answer {
            self.callback_answers_by_scope.remove(scope);
        }
        cx.notify();
    }

    pub fn callback_answer(&self, scope: &str) -> Option<BotCallbackAnswerModel> {
        self.callback_answers_by_scope.get(scope).cloned()
    }

    pub fn consume_callback_answer(&mut self, scope: &str, cx: &mut Context<Self>) {
        self.callback_answers_by_scope.remove(scope);
        cx.notify();
    }

    pub fn callback_query_answered_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::CallbackQueryAnswered(event)) =
            RiseEvent::decode_payload(name::CALLBACK_QUERY_ANSWERED, payload)
        else {
            return;
        };
        if event.callback_query_id.is_empty() {
            return;
        }
        let callback_query_id = event.callback_query_id;
        let answer = BotCallbackAnswerModel {
            callback_query_id: callback_query_id.clone(),
            text: event.text.unwrap_or_default(),
            show_alert: event.show_alert,
            url: event.url.unwrap_or_default(),
        };
        if let Some(scope) = self.callback_scope_by_id.get(&callback_query_id).cloned() {
            self.callback_answers_by_scope.insert(scope.clone(), answer);
            self.finish_callback(&scope, true, cx);
        } else {
            if self.early_callback_answers_by_id.len() >= EARLY_CALLBACK_ANSWERS_CAPACITY
                && let Some(evicted_key) = self.early_callback_answers_by_id.keys().next().cloned()
            {
                self.early_callback_answers_by_id.remove(&evicted_key);
            }
            self.early_callback_answers_by_id.insert(callback_query_id, answer);
            cx.notify();
        }
    }

    pub fn owned_bots_cache_key(user_id: &str) -> String {
        SaiCache::new(["botOwnedBots", user_id]).key
    }

    pub fn bot_cache_key(user_id: &str, bot_id: &str) -> String {
        SaiCache::new(["botDetails", user_id, bot_id]).key
    }

    pub fn bot_by_tag_cache_key(tag: &str) -> String {
        SaiCache::new(["botByTag", tag]).key
    }

    pub fn commands_cache_key(user_id: &str, bot_id: &str, scope: &str, language_code: &str) -> String {
        SaiCache::new(["botCommands", user_id, bot_id, scope, language_code]).key
    }

    pub fn public_commands_cache_key(
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        chat_id: Option<&str>,
        language_code: &str,
        chat_type: Option<&str>,
    ) -> String {
        SaiCache::new([
            "botPublicCommands",
            bot_id.unwrap_or(""),
            tag.unwrap_or(""),
            bot_user_id.unwrap_or(""),
            chat_id.unwrap_or(""),
            language_code,
            chat_type.unwrap_or(""),
        ])
        .key
    }

    pub fn webhook_cache_key(user_id: &str, bot_id: &str) -> String {
        SaiCache::new(["botWebhook", user_id, bot_id]).key
    }

    pub fn mini_apps_cache_key(user_id: &str, bot_id: &str) -> String {
        SaiCache::new(["botMiniApps", user_id, bot_id]).key
    }

    pub fn public_mini_apps_cache_key(bot_id: Option<&str>, tag: Option<&str>, bot_user_id: Option<&str>) -> String {
        SaiCache::new(["botPublicMiniApps", bot_id.unwrap_or(""), tag.unwrap_or(""), bot_user_id.unwrap_or("")]).key
    }

    pub fn mini_app_cache_key(user_id: &str, bot_id: &str, mini_app_id: &str) -> String {
        SaiCache::new(["botMiniApp", user_id, bot_id, mini_app_id]).key
    }

    pub fn sensitive_request_options(method: &str, reflect_pending_in_ui: bool) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: method.to_owned(),
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            track_until_acknowledged: false,
            exclude_from_debugger: true,
            ..Default::default()
        })
    }

    pub fn media_upload_progress(&self, bot_id: &str, kind: BotMediaKind) -> Option<f64> {
        self.media_upload_progress_by_key.get(&Self::media_upload_key(bot_id, kind)).copied()
    }

    pub fn is_media_uploading(&self, bot_id: &str, kind: BotMediaKind) -> bool {
        self.active_media_upload_keys.contains(&Self::media_upload_key(bot_id, kind))
    }

    pub fn begin_media_upload(&mut self, bot_id: &str, kind: BotMediaKind, cx: &mut Context<Self>) {
        let key = Self::media_upload_key(bot_id, kind);
        self.active_media_upload_keys.insert(key.clone());
        self.media_upload_progress_by_key.insert(key, 0.0);
        cx.notify();
    }

    pub fn set_media_upload_progress(&mut self, bot_id: &str, kind: BotMediaKind, progress: f64, cx: &mut Context<Self>) {
        self.media_upload_progress_by_key.insert(Self::media_upload_key(bot_id, kind), progress.clamp(0.0, 1.0));
        cx.notify();
    }

    pub fn finish_media_upload(&mut self, bot_id: &str, kind: BotMediaKind, succeeded: bool, cx: &mut Context<Self>) {
        let key = Self::media_upload_key(bot_id, kind);
        self.active_media_upload_keys.remove(&key);
        if succeeded {
            self.media_upload_progress_by_key.insert(key, 1.0);
        } else {
            self.media_upload_progress_by_key.remove(&key);
        }
        cx.notify();
    }

    pub fn make_media_optimistic_files(
        bot_id: &str,
        kind: BotMediaKind,
        local_file: SaiWsOptimisticLocalFile,
        on_remote_url: impl Fn(String, &mut App) + 'static,
        on_failure: impl Fn(&anyhow::Error, &mut App) + 'static,
    ) -> SaiWsOptimisticFiles {
        let mut files = SaiWsOptimisticFiles::new(kind.file_context().to_owned());
        files.compression = MediaCompressionConfiguration::avatar();
        files.resource_id = Some(bot_id.to_owned());
        files.local_files = vec![local_file];
        files.merge_uploaded_slots_into_body_root = true;
        files.extract_uploaded_files = Some(Arc::new(
            move |done: &Map<String, Value>, _: usize, _: &SaiWsOptimisticLocalFile, cx: &mut App| {
                let Some(url) = MusicCoverUpload::extract_remote_url(done) else {
                    return Err(anyhow::Error::new(SaiWsOptimisticError::FileUploadInvalidSlotPayload));
                };
                on_remote_url(url.clone(), cx);
                Ok(Value::Object(Self::document([(kind.wire_key(), Value::from(url))])))
            },
        ));
        let progress_bot_id = bot_id.to_owned();
        files.on_progress = Some(Arc::new(move |_: usize, progress: f64, cx: &mut App| {
            Self::shared(cx).update(cx, |store, cx| {
                store.set_media_upload_progress(&progress_bot_id, kind, progress, cx);
            });
        }));
        files.on_file_error = Some(Arc::new(move |_: usize, error: &anyhow::Error, cx: &mut App| {
            on_failure(error, cx);
        }));
        files
    }

    pub fn apply_bot_media_optimistic(user_id: &str, bot_id: &str, kind: BotMediaKind, remote_url: &str, cx: &mut App) {
        let values = match kind {
            BotMediaKind::Avatar => BotUpdateValues { avatar_url: Some(remote_url.to_owned()), ..Default::default() },
            BotMediaKind::Banner => BotUpdateValues { banner_url: Some(remote_url.to_owned()), ..Default::default() },
        };
        Self::apply_bot_update_optimistic(user_id, bot_id, &values, cx);
    }

    pub fn owned_bots(cx: &mut App) -> Vec<BotModel> {
        let Some(user_id) = Self::non_empty_user_id(cx) else {
            return Vec::new();
        };
        let real = Self::owned_bots_user_id(&user_id, cx);
        (bot_guide_sandbox_hooks().owned_bots_snapshot)(real, cx)
    }

    pub fn owned_bots_user_id(user_id: &str, cx: &mut App) -> Vec<BotModel> {
        let cache_key = Self::owned_bots_cache_key(user_id);
        let Some(dto) = Self::observed_dto::<BotListGatewayDTO>(&cache_key, cx) else {
            return Vec::new();
        };
        dto.bots.iter().map(|bot| bot.to_bot_model()).collect()
    }

    pub fn owned_bots_has_more(cx: &mut App) -> bool {
        let Some(user_id) = Self::non_empty_user_id(cx) else {
            return false;
        };
        let cache_key = Self::owned_bots_cache_key(&user_id);
        Self::observed_dto::<BotListGatewayDTO>(&cache_key, cx).is_some_and(|dto| dto.has_more)
    }

    pub fn owned_bots_relative_id(cx: &mut App) -> Option<String> {
        let user_id = Self::non_empty_user_id(cx)?;
        let cache_key = Self::owned_bots_cache_key(&user_id);
        Self::observed_dto::<BotListGatewayDTO>(&cache_key, cx)?.relative_id
    }

    pub fn bot(bot_id: &str, cx: &mut App) -> Option<BotModel> {
        if let Some(practice) = (bot_guide_sandbox_hooks().practice_bot)(bot_id, cx) {
            return Some(practice);
        }
        let user_id = Self::non_empty_user_id(cx)?;
        let cache_key = Self::bot_cache_key(&user_id, bot_id);
        if let Some(bot) = Self::observed_dto::<BotGatewayDTO>(&cache_key, cx).and_then(|dto| dto.bot) {
            return Some(bot.to_bot_model());
        }
        Self::owned_bots_user_id(&user_id, cx).into_iter().find(|bot| bot.id == bot_id)
    }

    pub fn bot_by_tag(tag: &str, cx: &mut App) -> Option<BotModel> {
        let cache_key = Self::bot_by_tag_cache_key(tag);
        let bot = Self::observed_dto::<BotGatewayDTO>(&cache_key, cx)?.bot?;
        Some(bot.to_bot_model())
    }

    pub fn commands(bot_id: &str, scope: &str, language_code: &str, cx: &mut App) -> Vec<BotCommandModel> {
        if let Some(practice) = (bot_guide_sandbox_hooks().practice_commands)(bot_id, scope, language_code, cx) {
            return practice;
        }
        let Some(user_id) = Self::non_empty_user_id(cx) else {
            return Vec::new();
        };
        let cache_key = Self::commands_cache_key(&user_id, bot_id, scope, language_code);
        let Some(dto) = Self::observed_dto::<BotCommandsGatewayDTO>(&cache_key, cx) else {
            return Vec::new();
        };
        dto.commands.iter().map(|command| command.to_bot_command_model()).collect()
    }

    pub fn public_commands(
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        chat_id: Option<&str>,
        language_code: Option<&str>,
        chat_type: Option<&str>,
        cx: &mut App,
    ) -> Option<BotPublicCommandsModel> {
        let language_code = match language_code {
            Some(language_code) => language_code.to_owned(),
            None => LanguageStore::shared(cx).code().to_owned(),
        };
        let cache_key =
            Self::public_commands_cache_key(bot_id, tag, bot_user_id, chat_id, &language_code, chat_type);
        let dto = Self::observed_dto::<BotPublicCommandsGatewayDTO>(&cache_key, cx)?;
        Some(dto.to_bot_public_commands_model())
    }

    pub fn webhook(bot_id: &str, cx: &mut App) -> Option<BotWebhookInfoModel> {
        if let Some(practice) = (bot_guide_sandbox_hooks().practice_webhook)(bot_id, cx) {
            return practice;
        }
        let user_id = Self::non_empty_user_id(cx)?;
        let cache_key = Self::webhook_cache_key(&user_id, bot_id);
        let webhook = Self::observed_dto::<BotWebhookGatewayDTO>(&cache_key, cx)?.webhook?;
        Some(webhook.to_bot_webhook_info_model())
    }

    pub fn mini_apps(bot_id: &str, cx: &mut App) -> Vec<BotMiniAppModel> {
        if let Some(practice) = (bot_guide_sandbox_hooks().practice_mini_apps)(bot_id, cx) {
            return practice;
        }
        let Some(user_id) = Self::non_empty_user_id(cx) else {
            return Vec::new();
        };
        let cache_key = Self::mini_apps_cache_key(&user_id, bot_id);
        let Some(dto) = Self::observed_dto::<BotMiniAppListGatewayDTO>(&cache_key, cx) else {
            return Vec::new();
        };
        dto.mini_apps.iter().map(|mini_app| mini_app.to_bot_mini_app_model()).collect()
    }

    pub fn mini_apps_has_more(bot_id: &str, cx: &mut App) -> bool {
        let Some(user_id) = Self::non_empty_user_id(cx) else {
            return false;
        };
        let cache_key = Self::mini_apps_cache_key(&user_id, bot_id);
        Self::observed_dto::<BotMiniAppListGatewayDTO>(&cache_key, cx).is_some_and(|dto| dto.has_more)
    }

    pub fn mini_apps_relative_id(bot_id: &str, cx: &mut App) -> Option<String> {
        let user_id = Self::non_empty_user_id(cx)?;
        let cache_key = Self::mini_apps_cache_key(&user_id, bot_id);
        Self::observed_dto::<BotMiniAppListGatewayDTO>(&cache_key, cx)?.relative_id
    }

    pub fn public_mini_apps(
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        cx: &mut App,
    ) -> Vec<BotMiniAppModel> {
        let cache_key = Self::public_mini_apps_cache_key(bot_id, tag, bot_user_id);
        let Some(dto) = Self::observed_dto::<BotMiniAppListGatewayDTO>(&cache_key, cx) else {
            return Vec::new();
        };
        dto.mini_apps.iter().map(|mini_app| mini_app.to_bot_mini_app_model()).collect()
    }

    pub fn public_mini_apps_has_more(
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        cx: &mut App,
    ) -> bool {
        let cache_key = Self::public_mini_apps_cache_key(bot_id, tag, bot_user_id);
        Self::observed_dto::<BotMiniAppListGatewayDTO>(&cache_key, cx).is_some_and(|dto| dto.has_more)
    }

    pub fn public_mini_apps_relative_id(
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        cx: &mut App,
    ) -> Option<String> {
        let cache_key = Self::public_mini_apps_cache_key(bot_id, tag, bot_user_id);
        Self::observed_dto::<BotMiniAppListGatewayDTO>(&cache_key, cx)?.relative_id
    }

    pub fn mini_app(bot_id: &str, mini_app_id: &str, cx: &mut App) -> Option<BotMiniAppModel> {
        let user_id = Self::non_empty_user_id(cx)?;
        let cache_key = Self::mini_app_cache_key(&user_id, bot_id, mini_app_id);
        if let Some(mini_app) = Self::observed_dto::<BotMiniAppGatewayDTO>(&cache_key, cx).and_then(|dto| dto.mini_app) {
            return Some(mini_app.to_bot_mini_app_model());
        }
        Self::mini_apps(bot_id, cx).into_iter().find(|mini_app| mini_app.id == mini_app_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_create_bot_optimistic(
        user_id: &str,
        request_id: &str,
        name: &str,
        tag: &str,
        description: &str,
        avatar_url: &str,
        banner_url: &str,
        can_join_groups: bool,
        privacy_mode: bool,
        cx: &mut App,
    ) -> Vec<BotCacheDocumentSnapshot> {
        let cache_key = Self::owned_bots_cache_key(user_id);
        let snapshots = Self::capture_documents(vec![cache_key.clone()], cx);
        let now = RiseClock::now().ms();
        let temp_id = format!("temp_{request_id}");
        let row = Value::Object(Self::document([
            (key::ID, Value::from(temp_id.as_str())),
            (key::OWNER_USER_ID, Value::from(user_id)),
            (key::BOT_USER_ID, Value::from("")),
            (key::PRIVATE_CHAT_ID, Value::from("")),
            (key::TAG, Value::from(tag)),
            (key::NAME, Value::from(name)),
            (key::DESCRIPTION, Value::from(description)),
            (key::AVATAR_URL, Value::from(avatar_url)),
            (key::BANNER_URL, Value::from(banner_url)),
            (key::IS_ACTIVE, Value::from(true)),
            (key::CAN_JOIN_GROUPS, Value::from(can_join_groups)),
            (key::PRIVACY_MODE, Value::from(privacy_mode)),
            (key::CREATED_AT, Value::from(now)),
            (key::UPDATED_AT, Value::from(now)),
            (key::BOT_USER_CHAT_ID, Value::from("")),
            (key::IS_OWNED_BY_REQUESTER, Value::from(true)),
            (IS_TEMP, Value::from(true)),
        ]));

        Self::mutate_list(
            &cache_key,
            key::BOTS,
            |rows| {
                let mut next = vec![row];
                next.extend(rows.into_iter().filter(|item| Self::row_id(item) != temp_id));
                next
            },
            cx,
        );
        snapshots
    }

    pub fn finalize_created_bot(user_id: &str, request_id: &str, bot: &BotWireDTO, cx: &mut App) {
        let cache_key = Self::owned_bots_cache_key(user_id);
        let Some(row) = SaiJSONCodec::encode(bot).map(Value::Object) else {
            return;
        };
        let temp_id = format!("temp_{request_id}");

        Self::mutate_list(
            &cache_key,
            key::BOTS,
            |rows| {
                let mut replaced = false;
                let mut next: Vec<Value> = rows
                    .into_iter()
                    .filter_map(|item| {
                        let id = Self::row_id(&item);
                        if id == temp_id {
                            replaced = true;
                            return Some(row.clone());
                        }
                        if id == bot.id {
                            if replaced {
                                return None;
                            }
                            replaced = true;
                            return Some(row.clone());
                        }
                        Some(item)
                    })
                    .collect();
                if !replaced {
                    next.insert(0, row.clone());
                }
                Self::sorted_rows(next)
            },
            cx,
        );
        Self::install_bot_detail(user_id, bot, cx);
    }

    pub fn bot_mutation_snapshot(user_id: &str, bot_id: &str, cx: &mut App) -> Vec<BotCacheDocumentSnapshot> {
        let mut keys = vec![Self::owned_bots_cache_key(user_id), Self::bot_cache_key(user_id, bot_id)];
        if let Some(tag) = Self::bot(bot_id, cx).map(|bot| bot.tag)
            && !tag.is_empty()
        {
            keys.push(Self::bot_by_tag_cache_key(&tag));
        }
        Self::capture_documents(keys, cx)
    }

    pub fn apply_bot_update_optimistic(user_id: &str, bot_id: &str, values: &BotUpdateValues, cx: &mut App) {
        let previous_tag = Self::bot(bot_id, cx).map(|bot| bot.tag);
        let patch = Self::bot_patch(values);
        let list_key = Self::owned_bots_cache_key(user_id);

        let _ = SaiUpdater::patch_row(&list_key, key::BOTS, key::ID, bot_id, &patch, SaiUpdaterWriteScope::Cache, cx);
        Self::patch_bot_detail(&Self::bot_cache_key(user_id, bot_id), bot_id, &patch, cx);
        if let Some(previous_tag) = previous_tag
            && !previous_tag.is_empty()
        {
            Self::patch_bot_detail(&Self::bot_by_tag_cache_key(&previous_tag), bot_id, &patch, cx);
        }
    }

    pub fn confirm_updated_bot(user_id: &str, bot: &BotWireDTO, previous_tag: Option<&str>, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(bot).map(Value::Object) else {
            return;
        };

        Self::mutate_list(
            &Self::owned_bots_cache_key(user_id),
            key::BOTS,
            |rows| rows.into_iter().map(|item| if Self::row_id(&item) == bot.id { row.clone() } else { item }).collect(),
            cx,
        );
        Self::install_bot_detail(user_id, bot, cx);
        if !bot.tag.is_empty() {
            Self::install_root(&Self::bot_by_tag_cache_key(&bot.tag), Self::success_root(key::BOT, row), cx);
        }
        if let Some(previous_tag) = previous_tag
            && previous_tag != bot.tag
        {
            SaiCacheCleaner::clean(&[Self::bot_by_tag_cache_key(previous_tag)], SaiCacheCleanLayer::Memory, cx);
        }
    }

    pub fn apply_delete_bot_optimistic(user_id: &str, bot_id: &str, cx: &mut App) {
        let _ = SaiUpdater::remove_row(
            &Self::owned_bots_cache_key(user_id),
            key::BOTS,
            key::ID,
            bot_id,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        SaiCacheCleaner::clean(&[Self::bot_cache_key(user_id, bot_id)], SaiCacheCleanLayer::Memory, cx);
    }

    pub fn commands_snapshot(
        user_id: &str,
        bot_id: &str,
        scope: &str,
        language_code: &str,
        cx: &mut App,
    ) -> Vec<BotCacheDocumentSnapshot> {
        Self::capture_documents(vec![Self::commands_cache_key(user_id, bot_id, scope, language_code)], cx)
    }

    pub fn apply_commands_optimistic(
        user_id: &str,
        bot_id: &str,
        scope: &str,
        language_code: &str,
        commands: &[BotCommandModel],
        cx: &mut App,
    ) {
        let rows: Vec<Value> = commands
            .iter()
            .map(|command| {
                Value::Object(Self::document([
                    (key::COMMAND, Value::from(command.command.as_str())),
                    (key::DESCRIPTION, Value::from(command.description.as_str())),
                ]))
            })
            .collect();
        let cache_key = Self::commands_cache_key(user_id, bot_id, scope, language_code);
        Self::install_root(&cache_key, Self::success_root(key::COMMANDS, Value::Array(rows)), cx);
    }

    pub fn confirm_commands(
        user_id: &str,
        bot_id: &str,
        scope: &str,
        language_code: &str,
        dto: &BotCommandsGatewayDTO,
        cx: &mut App,
    ) {
        let Some(root) = SaiJSONCodec::encode(dto) else {
            return;
        };
        Self::install_root(&Self::commands_cache_key(user_id, bot_id, scope, language_code), root, cx);
    }

    pub fn webhook_snapshot(user_id: &str, bot_id: &str, cx: &mut App) -> Vec<BotCacheDocumentSnapshot> {
        Self::capture_documents(vec![Self::webhook_cache_key(user_id, bot_id)], cx)
    }

    pub fn apply_webhook_optimistic(
        user_id: &str,
        bot_id: &str,
        url: &str,
        max_connections: i64,
        allowed_updates: &[String],
        enabled: bool,
        cx: &mut App,
    ) {
        let current = Self::webhook(bot_id, cx);
        let row = Value::Object(Self::document([
            (key::BOT_ID, Value::from(bot_id)),
            (key::URL, Value::from(url)),
            (key::ENABLED, Value::from(enabled)),
            (
                key::HAS_CUSTOM_CERTIFICATE,
                Value::from(current.as_ref().is_some_and(|webhook| webhook.has_custom_certificate)),
            ),
            (
                key::PENDING_UPDATE_COUNT,
                Value::from(current.as_ref().map_or(0, |webhook| webhook.pending_update_count)),
            ),
            (key::LAST_ERROR_DATE, Value::from(current.as_ref().map_or(0, |webhook| webhook.last_error_date))),
            (
                key::LAST_ERROR_MESSAGE,
                Value::from(current.as_ref().map_or("", |webhook| webhook.last_error_message.as_str())),
            ),
            (key::MAX_CONNECTIONS, Value::from(max_connections)),
            (key::ALLOWED_UPDATES, Value::from(allowed_updates.to_vec())),
            (key::UPDATED_AT, Value::from(RiseClock::now().ms())),
        ]));
        Self::install_root(&Self::webhook_cache_key(user_id, bot_id), Self::success_root(key::WEBHOOK, row), cx);
    }

    pub fn confirm_webhook(user_id: &str, bot_id: &str, dto: &BotWebhookGatewayDTO, cx: &mut App) {
        let Some(root) = SaiJSONCodec::encode(dto) else {
            return;
        };
        Self::install_root(&Self::webhook_cache_key(user_id, bot_id), root, cx);
    }

    pub fn mini_app_mutation_snapshot(
        user_id: &str,
        bot_id: &str,
        mini_app_id: Option<&str>,
        cx: &mut App,
    ) -> Vec<BotCacheDocumentSnapshot> {
        let mut keys = vec![Self::mini_apps_cache_key(user_id, bot_id)];
        if let Some(mini_app_id) = mini_app_id {
            keys.push(Self::mini_app_cache_key(user_id, bot_id, mini_app_id));
        }
        Self::capture_documents(keys, cx)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_create_mini_app_optimistic(
        user_id: &str,
        bot_id: &str,
        temp_id: &str,
        short_name: &str,
        title: &str,
        description: &str,
        production_url: &str,
        development_url: &str,
        icon_url: &str,
        cx: &mut App,
    ) {
        let now = RiseClock::now().ms();
        let row = Value::Object(Self::document([
            (key::ID, Value::from(temp_id)),
            (key::BOT_ID, Value::from(bot_id)),
            (key::SHORT_NAME, Value::from(short_name)),
            (key::TITLE, Value::from(title)),
            (key::DESCRIPTION, Value::from(description)),
            (key::PRODUCTION_URL, Value::from(production_url)),
            (key::DEVELOPMENT_URL, Value::from(development_url)),
            (key::ICON_URL, Value::from(icon_url)),
            (key::IS_ACTIVE, Value::from(true)),
            (key::CREATED_AT, Value::from(now)),
            (key::UPDATED_AT, Value::from(now)),
            (IS_TEMP, Value::from(true)),
        ]));

        Self::mutate_list(
            &Self::mini_apps_cache_key(user_id, bot_id),
            key::MINI_APPS,
            |rows| {
                let mut next = vec![row];
                next.extend(rows.into_iter().filter(|item| Self::row_id(item) != temp_id));
                next
            },
            cx,
        );
    }

    pub fn finalize_created_mini_app(
        user_id: &str,
        bot_id: &str,
        temp_id: &str,
        mini_app: &BotMiniAppWireDTO,
        cx: &mut App,
    ) {
        let Some(row) = SaiJSONCodec::encode(mini_app).map(Value::Object) else {
            return;
        };

        Self::mutate_list(
            &Self::mini_apps_cache_key(user_id, bot_id),
            key::MINI_APPS,
            |rows| {
                let mut replaced = false;
                let mut next: Vec<Value> = rows
                    .into_iter()
                    .filter_map(|item| {
                        let id = Self::row_id(&item);
                        if id == temp_id || id == mini_app.id {
                            if replaced {
                                return None;
                            }
                            replaced = true;
                            return Some(row.clone());
                        }
                        Some(item)
                    })
                    .collect();
                if !replaced {
                    next.insert(0, row.clone());
                }
                Self::sorted_rows(next)
            },
            cx,
        );
        Self::install_mini_app_detail(user_id, bot_id, mini_app, cx);
    }

    pub fn apply_mini_app_update_optimistic(
        user_id: &str,
        bot_id: &str,
        mini_app_id: &str,
        values: &BotMiniAppUpdateValues,
        cx: &mut App,
    ) {
        let patch = Self::mini_app_patch(values);

        let _ = SaiUpdater::patch_row(
            &Self::mini_apps_cache_key(user_id, bot_id),
            key::MINI_APPS,
            key::ID,
            mini_app_id,
            &patch,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        Self::patch_mini_app_detail(&Self::mini_app_cache_key(user_id, bot_id, mini_app_id), mini_app_id, &patch, cx);
    }

    pub fn confirm_updated_mini_app(user_id: &str, bot_id: &str, mini_app: &BotMiniAppWireDTO, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(mini_app).map(Value::Object) else {
            return;
        };

        Self::mutate_list(
            &Self::mini_apps_cache_key(user_id, bot_id),
            key::MINI_APPS,
            |rows| {
                rows.into_iter().map(|item| if Self::row_id(&item) == mini_app.id { row.clone() } else { item }).collect()
            },
            cx,
        );
        Self::install_mini_app_detail(user_id, bot_id, mini_app, cx);
    }

    pub fn apply_delete_mini_app_optimistic(user_id: &str, bot_id: &str, mini_app_id: &str, cx: &mut App) {
        let _ = SaiUpdater::remove_row(
            &Self::mini_apps_cache_key(user_id, bot_id),
            key::MINI_APPS,
            key::ID,
            mini_app_id,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        SaiCacheCleaner::clean(
            &[Self::mini_app_cache_key(user_id, bot_id, mini_app_id)],
            SaiCacheCleanLayer::Memory,
            cx,
        );
    }

    pub fn restore_documents(snapshots: &[BotCacheDocumentSnapshot], cx: &mut App) {
        for snapshot in snapshots {
            match &snapshot.payload {
                Some(payload) => Self::install_root(&snapshot.cache_key, payload.clone(), cx),
                None => {
                    SaiCacheCleaner::clean(std::slice::from_ref(&snapshot.cache_key), SaiCacheCleanLayer::Memory, cx);
                }
            }
        }
    }

    fn non_empty_user_id(cx: &mut App) -> Option<String> {
        Self::effective_user_id_for_wire(cx).filter(|user_id| !user_id.is_empty())
    }

    fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn observed_payload(cache_key: &str, cx: &mut App) -> Option<Value> {
        SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
    }

    fn observed_dto<T: DeserializeOwned>(cache_key: &str, cx: &mut App) -> Option<T> {
        Self::decode(&Self::observed_payload(cache_key, cx)?)
    }

    fn decode<T: DeserializeOwned>(payload: &Value) -> Option<T> {
        SaiJSONCodec::decode::<T>(Some(payload))
    }

    fn current_payload(cache_key: &str, cx: &mut App) -> Option<Value> {
        SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
    }

    fn capture_documents(keys: Vec<String>, cx: &mut App) -> Vec<BotCacheDocumentSnapshot> {
        keys.into_iter()
            .collect::<HashSet<String>>()
            .into_iter()
            .map(|cache_key| {
                let payload = AnyDictionaryBridge::dictionary(Self::current_payload(&cache_key, cx).as_ref());
                BotCacheDocumentSnapshot { cache_key, payload }
            })
            .collect()
    }

    fn install_root(cache_key: &str, root: Map<String, Value>, cx: &mut App) {
        let _ = SaiUpdater::replace_root(cache_key, root, SaiUpdaterWriteScope::Cache, cx);
    }

    fn document(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Map<String, Value> {
        entries.into_iter().map(|(name, value)| (name.to_owned(), value)).collect()
    }

    fn success_root(field: &'static str, value: Value) -> Map<String, Value> {
        Self::document([(key::SUCCESS, Value::from(true)), (key::ERROR, Value::from("")), (field, value)])
    }

    fn mutate_list(
        cache_key: &str,
        path: &str,
        transform: impl FnOnce(Vec<Value>) -> Vec<Value>,
        cx: &mut App,
    ) {
        let _ = SaiUpdater::update_json_value(
            cache_key,
            move |context| {
                let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                let rows = Self::array(root.get(path));
                root.insert(path.to_owned(), Value::Array(transform(rows)));
                if !root.contains_key(key::SUCCESS) {
                    root.insert(key::SUCCESS.to_owned(), Value::from(true));
                }
                if !root.contains_key(key::ERROR) {
                    root.insert(key::ERROR.to_owned(), Value::from(""));
                }
                if !root.contains_key(key::HAS_MORE) {
                    root.insert(key::HAS_MORE.to_owned(), Value::from(false));
                }
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    fn sorted_rows(rows: Vec<Value>) -> Vec<Value> {
        let mut entries: Vec<(usize, Value)> = rows.into_iter().enumerate().collect();
        entries.sort_by(|lhs, rhs| {
            Self::created_at(&rhs.1)
                .cmp(&Self::created_at(&lhs.1))
                .then_with(|| Self::row_id(&lhs.1).cmp(&Self::row_id(&rhs.1)))
                .then_with(|| lhs.0.cmp(&rhs.0))
        });
        entries.into_iter().map(|(_, row)| row).collect()
    }

    fn created_at(row: &Value) -> i64 {
        let Some(value) = row.get(key::CREATED_AT) else {
            return 0;
        };
        value.as_i64().or_else(|| value.as_f64().map(|number| number as i64)).unwrap_or(0)
    }

    fn row_id(row: &Value) -> String {
        match row.get(key::ID) {
            Some(Value::String(value)) => value.clone(),
            Some(Value::Number(value)) => value.to_string(),
            _ => String::new(),
        }
    }

    fn bot_patch(values: &BotUpdateValues) -> Map<String, Value> {
        let mut patch = Self::document([(key::UPDATED_AT, Value::from(RiseClock::now().ms()))]);
        if let Some(name) = &values.name {
            patch.insert(key::NAME.to_owned(), Value::from(name.as_str()));
        }
        if let Some(description) = &values.description {
            patch.insert(key::DESCRIPTION.to_owned(), Value::from(description.as_str()));
        }
        if let Some(avatar_url) = &values.avatar_url {
            patch.insert(key::AVATAR_URL.to_owned(), Value::from(avatar_url.as_str()));
        }
        if let Some(banner_url) = &values.banner_url {
            patch.insert(key::BANNER_URL.to_owned(), Value::from(banner_url.as_str()));
        }
        if let Some(can_join_groups) = values.can_join_groups {
            patch.insert(key::CAN_JOIN_GROUPS.to_owned(), Value::from(can_join_groups));
        }
        if let Some(privacy_mode) = values.privacy_mode {
            patch.insert(key::PRIVACY_MODE.to_owned(), Value::from(privacy_mode));
        }
        if let Some(tag) = &values.tag {
            patch.insert(key::TAG.to_owned(), Value::from(tag.as_str()));
        }
        patch
    }

    fn mini_app_patch(values: &BotMiniAppUpdateValues) -> Map<String, Value> {
        let mut patch = Self::document([(key::UPDATED_AT, Value::from(RiseClock::now().ms()))]);
        if let Some(title) = &values.title {
            patch.insert(key::TITLE.to_owned(), Value::from(title.as_str()));
        }
        if let Some(description) = &values.description {
            patch.insert(key::DESCRIPTION.to_owned(), Value::from(description.as_str()));
        }
        if let Some(production_url) = &values.production_url {
            patch.insert(key::PRODUCTION_URL.to_owned(), Value::from(production_url.as_str()));
        }
        if let Some(development_url) = &values.development_url {
            patch.insert(key::DEVELOPMENT_URL.to_owned(), Value::from(development_url.as_str()));
        }
        if let Some(icon_url) = &values.icon_url {
            patch.insert(key::ICON_URL.to_owned(), Value::from(icon_url.as_str()));
        }
        if let Some(is_active) = values.is_active {
            patch.insert(key::IS_ACTIVE.to_owned(), Value::from(is_active));
        }
        patch
    }

    fn patch_bot_detail(cache_key: &str, bot_id: &str, patch: &Map<String, Value>, cx: &mut App) {
        Self::patch_detail_row(cache_key, key::BOT, bot_id, patch, cx);
    }

    fn patch_mini_app_detail(cache_key: &str, mini_app_id: &str, patch: &Map<String, Value>, cx: &mut App) {
        Self::patch_detail_row(cache_key, key::MINI_APP, mini_app_id, patch, cx);
    }

    fn patch_detail_row(cache_key: &str, field: &str, id: &str, patch: &Map<String, Value>, cx: &mut App) {
        if Self::current_payload(cache_key, cx).is_none() {
            return;
        }
        let _ = SaiUpdater::update_json_value(
            cache_key,
            |context| {
                let root = AnyDictionaryBridge::dictionary(context.payload.as_ref());
                let row = root
                    .as_ref()
                    .and_then(|root| root.get(field))
                    .filter(|row| Self::row_id(row) == id)
                    .and_then(|row| AnyDictionaryBridge::dictionary(Some(row)));
                let (Some(mut root), Some(mut row)) = (root, row) else {
                    return Ok(context.payload.clone().unwrap_or_else(|| Value::Object(Map::new())));
                };
                for (name, value) in patch {
                    row.insert(name.clone(), value.clone());
                }
                root.insert(field.to_owned(), Value::Object(row));
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    fn install_bot_detail(user_id: &str, bot: &BotWireDTO, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(bot).map(Value::Object) else {
            return;
        };
        Self::install_root(&Self::bot_cache_key(user_id, &bot.id), Self::success_root(key::BOT, row), cx);
    }

    fn install_mini_app_detail(user_id: &str, bot_id: &str, mini_app: &BotMiniAppWireDTO, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(mini_app).map(Value::Object) else {
            return;
        };
        Self::install_root(
            &Self::mini_app_cache_key(user_id, bot_id, &mini_app.id),
            Self::success_root(key::MINI_APP, row),
            cx,
        );
    }

    fn array(value: Option<&Value>) -> Vec<Value> {
        match value {
            Some(Value::Array(values)) => values.clone(),
            _ => Vec::new(),
        }
    }

    fn media_upload_key(bot_id: &str, kind: BotMediaKind) -> String {
        format!("{bot_id}::{}", kind.raw_value())
    }

    pub fn inline_page_limit() -> i64 {
        RiseLimits::client_value("bot_services_store_inline_page_limit")
    }

    pub fn normalized_bot_tag(value: &str) -> String {
        let lowered = trimming_whitespaces_and_newlines(value).to_lowercase();
        BotFieldRules::tag().clamped(lowered.trim_start_matches('@'))
    }

    pub fn normalized_inline_query(value: &str) -> String {
        let lowered = trimming_whitespaces_and_newlines(value).to_lowercase();
        character_prefix(&lowered, INLINE_QUERY_MAX_CHARACTERS).to_owned()
    }

    pub fn normalized_inline_result_query(value: &str) -> String {
        character_prefix(trimming_whitespaces_and_newlines(value), INLINE_RESULT_QUERY_MAX_CHARACTERS).to_owned()
    }

    pub fn mention_candidates_cache_key_user_id(
        user_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
    ) -> String {
        let query = Self::normalized_inline_query(query);
        SaiCache::new(["botMentionCandidates", user_id, chat_id, topic_id.unwrap_or(""), query.as_str()]).key
    }

    pub fn inline_commands_cache_key_user_id(
        user_id: &str,
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
    ) -> String {
        let tag = Self::normalized_bot_tag(bot_tag);
        let language_code = language_code.to_lowercase();
        SaiCache::new(["botInlineCommands", user_id, tag.as_str(), chat_id, topic_id.unwrap_or(""), language_code.as_str()])
            .key
    }

    pub fn chat_commands_cache_key_user_id(
        user_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
    ) -> String {
        let language_code = language_code.to_lowercase();
        SaiCache::new(["botChatCommands", user_id, chat_id, topic_id.unwrap_or(""), language_code.as_str()]).key
    }

    pub fn inline_results_cache_key_user_id(
        user_id: &str,
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
    ) -> String {
        let tag = Self::normalized_bot_tag(bot_tag);
        let query = Self::normalized_inline_result_query(query);
        SaiCache::new(["botInlineResults", user_id, tag.as_str(), chat_id, topic_id.unwrap_or(""), query.as_str()]).key
    }

    pub fn inline_send_key(query_id: &str, result_id: &str) -> String {
        format!("{query_id}::{result_id}")
    }

    pub fn mention_candidates_cache_key(chat_id: &str, topic_id: Option<&str>, query: &str, cx: &mut App) -> String {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return String::new();
        };
        Self::mention_candidates_cache_key_user_id(&user_id, chat_id, topic_id, query)
    }

    pub fn inline_commands_cache_key(
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
        cx: &mut App,
    ) -> String {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return String::new();
        };
        Self::inline_commands_cache_key_user_id(&user_id, bot_tag, chat_id, topic_id, language_code)
    }

    pub fn chat_commands_cache_key(chat_id: &str, topic_id: Option<&str>, language_code: &str, cx: &mut App) -> String {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return String::new();
        };
        Self::chat_commands_cache_key_user_id(&user_id, chat_id, topic_id, language_code)
    }

    pub fn inline_results_cache_key(
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        cx: &mut App,
    ) -> String {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return String::new();
        };
        Self::inline_results_cache_key_user_id(&user_id, bot_tag, chat_id, topic_id, query)
    }

    pub fn mention_candidates(
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        cx: &mut App,
    ) -> Vec<BotMentionCandidateModel> {
        let Some(dto) = Self::mention_candidates_dto(chat_id, topic_id, query, cx) else {
            return Vec::new();
        };
        let viewer_id = Self::effective_user_id_for_wire(cx).unwrap_or_default();
        dto.candidates
            .iter()
            .filter_map(|candidate| {
                let model = candidate.to_model(cx);
                (viewer_id.is_empty() || model.user_id != viewer_id).then_some(model)
            })
            .collect()
    }

    pub fn has_mention_candidates(chat_id: &str, topic_id: Option<&str>, query: &str, cx: &mut App) -> bool {
        Self::mention_candidates_dto(chat_id, topic_id, query, cx).is_some()
    }

    pub fn mention_candidates_has_more(chat_id: &str, topic_id: Option<&str>, query: &str, cx: &mut App) -> bool {
        Self::mention_candidates_dto(chat_id, topic_id, query, cx).is_some_and(|dto| dto.has_more)
    }

    pub fn mention_candidates_cursor(
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        cx: &mut App,
    ) -> Option<String> {
        Self::mention_candidates_dto(chat_id, topic_id, query, cx)?.cursor
    }

    pub fn inline_commands(
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
        cx: &mut App,
    ) -> Option<BotInlineCommandsModel> {
        let cache_key = Self::inline_commands_cache_key(bot_tag, chat_id, topic_id, language_code, cx);
        if cache_key.is_empty() {
            return None;
        }
        let payload = Self::observed_payload(&cache_key, cx)?;
        let dto = Self::decode_inline_payload::<BotInlineCommandsGatewayDTO>(&payload)?;
        Some(BotInlineCommandsModel {
            bot: dto.bot.as_ref().map(|bot| bot.to_model()),
            commands: dto.commands.iter().map(|command| command.to_bot_command_model()).collect(),
        })
    }

    /// commands of every bot seated in the chat in server order, none until the first answer
    pub fn chat_commands(
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
        cx: &mut App,
    ) -> Option<Vec<BotInlineCommandsModel>> {
        let cache_key = Self::chat_commands_cache_key(chat_id, topic_id, language_code, cx);
        if cache_key.is_empty() {
            return None;
        }
        let payload = Self::observed_payload(&cache_key, cx)?;
        let dto = Self::decode_inline_payload::<BotChatCommandsGatewayDTO>(&payload)?;
        Some(
            dto.bots
                .iter()
                .map(|group| BotInlineCommandsModel {
                    bot: group.bot.as_ref().map(|bot| bot.to_model()),
                    commands: group.commands.iter().map(|command| command.to_bot_command_model()).collect(),
                })
                .collect(),
        )
    }

    pub fn inline_results(
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        cx: &mut App,
    ) -> BotInlineResultsPage {
        let cache_key = Self::inline_results_cache_key(bot_tag, chat_id, topic_id, query, cx);
        let dto = if cache_key.is_empty() {
            None
        } else {
            Self::observed_payload(&cache_key, cx)
                .and_then(|payload| Self::decode_inline_payload::<BotInlineResultsGatewayDTO>(&payload))
        };
        let Some(dto) = dto else {
            return BotInlineResultsPage {
                query_id: String::new(),
                results: Vec::new(),
                next_offset: None,
                has_more: false,
                cache_time: 0,
            };
        };
        BotInlineResultsPage {
            results: dto.results.iter().map(|result| result.to_model(&dto.query_id)).collect(),
            query_id: dto.query_id,
            next_offset: dto.next_offset,
            has_more: dto.has_more,
            cache_time: dto.cache_time,
        }
    }

    pub fn is_inline_composer_loading(&self, cache_key: &str) -> bool {
        !cache_key.is_empty() && self.inline_composer_loading_keys.contains(cache_key)
    }

    pub fn inline_composer_error(&self, cache_key: &str) -> Option<String> {
        if cache_key.is_empty() {
            return None;
        }
        self.inline_composer_errors_by_key.get(cache_key).cloned()
    }

    pub fn schedule_inline_composer_request(
        &mut self,
        scope: &str,
        cache_key: &str,
        delay_milliseconds: i64,
        response_timeout: Option<Duration>,
        action: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.cancel_inline_composer_request(scope, cx);
        let generation = self.inline_composer_generation_by_scope.get(scope).copied().unwrap_or(0).wrapping_add(1);
        self.inline_composer_generation_by_scope.insert(scope.to_owned(), generation);
        self.inline_composer_pending_key_by_scope.insert(scope.to_owned(), cache_key.to_owned());
        self.inline_composer_loading_keys.insert(cache_key.to_owned());
        self.inline_composer_errors_by_key.remove(cache_key);
        cx.notify();

        if delay_milliseconds <= 0 && response_timeout.is_none() {
            cx.defer(move |cx: &mut App| action(cx));
            return;
        }

        let task_scope = scope.to_owned();
        let task_cache_key = cache_key.to_owned();
        let task = cx.spawn(async move |this, cx| {
            if delay_milliseconds > 0 {
                cx.background_executor().timer(Duration::from_millis(delay_milliseconds as u64)).await;
            }
            let is_current = this
                .read_with(cx, |store, _| store.is_inline_composer_request_current(&task_scope, &task_cache_key, generation))
                .unwrap_or(false);
            if !is_current {
                return;
            }
            cx.update(|cx| action(cx));
            let Some(response_timeout) = response_timeout else {
                return;
            };
            cx.background_executor().timer(response_timeout).await;
            let _ = this.update(cx, |store, cx| {
                if !store.is_inline_composer_request_current(&task_scope, &task_cache_key, generation) {
                    return;
                }
                let error = anyhow::Error::msg(tr("chat_inline_error"));
                store.fail_inline_composer_request(&task_scope, &task_cache_key, &error, Some(generation), cx);
            });
        });
        self.inline_composer_tasks.insert(scope.to_owned(), task);
    }

    fn is_inline_composer_request_current(&self, scope: &str, cache_key: &str, generation: i64) -> bool {
        self.inline_composer_generation_by_scope.get(scope) == Some(&generation)
            && self.inline_composer_pending_key_by_scope.get(scope).map(String::as_str) == Some(cache_key)
    }

    pub fn inline_composer_request_generation(&self, scope: &str, cache_key: &str) -> Option<i64> {
        if self.inline_composer_pending_key_by_scope.get(scope).map(String::as_str) != Some(cache_key) {
            return None;
        }
        self.inline_composer_generation_by_scope.get(scope).copied()
    }

    pub fn cancel_inline_composer_request(&mut self, scope: &str, cx: &mut Context<Self>) {
        self.inline_composer_tasks.remove(scope);
        let generation = self.inline_composer_generation_by_scope.entry(scope.to_owned()).or_insert(0);
        *generation = generation.wrapping_add(1);
        if let Some(key) = self.inline_composer_pending_key_by_scope.remove(scope) {
            self.inline_composer_loading_keys.remove(&key);
        }
        cx.notify();
    }

    pub fn finish_inline_composer_request(
        &mut self,
        scope: &str,
        cache_key: &str,
        generation: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if !self.matches_inline_composer_request(scope, cache_key, generation) {
            return;
        }
        self.inline_composer_loading_keys.remove(cache_key);
        self.inline_composer_pending_key_by_scope.remove(scope);
        self.inline_composer_tasks.remove(scope);
        cx.notify();
    }

    pub fn fail_inline_composer_request(
        &mut self,
        scope: &str,
        cache_key: &str,
        error: &anyhow::Error,
        generation: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if !self.matches_inline_composer_request(scope, cache_key, generation) {
            return;
        }
        let failure = error.downcast_ref::<SaiWsResponseError>();
        let empty_details = HashMap::new();
        let text = CanonErrors::text(
            failure.and_then(|failure| failure.code.as_deref()),
            failure.map_or(&empty_details, |failure| &failure.details),
        )
        .unwrap_or_else(|| tr("error"));
        self.inline_composer_errors_by_key.insert(cache_key.to_owned(), text);
        self.finish_inline_composer_request(scope, cache_key, generation, cx);
    }

    fn matches_inline_composer_request(&self, scope: &str, cache_key: &str, generation: Option<i64>) -> bool {
        self.inline_composer_pending_key_by_scope.get(scope).map(String::as_str) == Some(cache_key)
            && (generation.is_none() || self.inline_composer_generation_by_scope.get(scope).copied() == generation)
    }

    pub fn begin_inline_result_send(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if key.is_empty() || self.sending_inline_result_keys.contains(key) {
            return false;
        }
        self.sending_inline_result_keys.insert(key.to_owned());
        cx.notify();
        true
    }

    pub fn finish_inline_result_send(&mut self, key: &str, cx: &mut Context<Self>) {
        self.sending_inline_result_keys.remove(key);
        cx.notify();
    }

    pub fn is_inline_result_sending(&self, query_id: &str, result_id: &str) -> bool {
        self.sending_inline_result_keys.contains(&Self::inline_send_key(query_id, result_id))
    }

    fn mention_candidates_dto(
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        cx: &mut App,
    ) -> Option<BotMentionCandidatesGatewayDTO> {
        let cache_key = Self::mention_candidates_cache_key(chat_id, topic_id, query, cx);
        if cache_key.is_empty() {
            return None;
        }
        Self::decode_inline_payload(&Self::observed_payload(&cache_key, cx)?)
    }

    fn decode_inline_payload<T: DeserializeOwned>(payload: &Value) -> Option<T> {
        SaiJSONCodec::decode::<T>(Some(payload))
    }

    pub fn remember_mini_app_load(&mut self, dto: &BotMiniAppLaunchGatewayDTO, user_id: &str, cx: &mut Context<Self>) {
        let url = dto.launch_url.as_str();
        let expires_at = dto.expires_at;
        let Some(token) = dto.load_receipt.as_deref().filter(|token| !token.is_empty()) else {
            return;
        };
        if url.is_empty()
            || expires_at <= Self::mini_app_load_now()
            || Self::effective_user_id_for_wire(cx).as_deref() != Some(user_id)
        {
            return;
        }
        let now = Self::mini_app_load_now();
        let expired: Vec<String> = self
            .mini_app_loads
            .iter()
            .filter(|(_, receipt)| receipt.expires_at <= now)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.forget_mini_app_load(&key);
        }
        if self.mini_app_loads.contains_key(url) {
            return;
        }
        let capacity = RiseLimits::client_value("mini_app_load_receipts_capacity");
        if self.mini_app_loads.len() as i64 >= capacity
            && let Some(oldest) =
                self.mini_app_loads.iter().min_by_key(|(_, receipt)| receipt.expires_at).map(|(key, _)| key.clone())
        {
            self.forget_mini_app_load(&oldest);
        }
        self.mini_app_loads.insert(
            url.to_owned(),
            MiniAppLoadReceipt {
                token: token.to_owned(),
                user_id: user_id.to_owned(),
                expires_at,
                page_loaded: false,
                attempts: 0,
            },
        );
    }

    pub fn confirm_mini_app_page_loaded(&mut self, launch_url: &Url, cx: &mut Context<Self>) {
        let key = launch_url.absolute_string();
        let Some(receipt) = self.mini_app_loads.get_mut(key) else {
            return;
        };
        receipt.page_loaded = true;
        self.send_mini_app_load_confirmation(key, cx);
    }

    fn mini_app_load_now() -> i64 {
        RiseClock::now().ms()
    }

    fn forget_mini_app_load(&mut self, key: &str) {
        self.mini_app_load_retries.remove(key);
        self.mini_app_load_requests.remove(key);
        self.mini_app_loads.remove(key);
    }

    fn send_mini_app_load_confirmation(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(receipt) = self.mini_app_loads.get(key).cloned() else {
            return;
        };
        if !receipt.page_loaded || self.mini_app_load_requests.contains_key(key) {
            return;
        }
        if receipt.expires_at <= Self::mini_app_load_now()
            || Self::effective_user_id_for_wire(cx).as_deref() != Some(receipt.user_id.as_str())
        {
            self.forget_mini_app_load(key);
            return;
        }
        self.mini_app_load_retries.remove(key);
        let slice = SaiTypedSlice::<BotStatusGatewayDTO>::new(cx);
        self.mini_app_load_requests.insert(key.to_owned(), slice.clone());

        let this = cx.weak_entity();
        let on_success: SaiWsOnSuccess = {
            let (this, key, token) = (this.clone(), key.to_owned(), receipt.token.clone());
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                let (this, key, token) = (this.clone(), key.clone(), token.clone());
                cx.defer(move |cx: &mut App| {
                    let _ = this.update(cx, |store, _| {
                        if store.mini_app_loads.get(&key).is_some_and(|current| current.token == token) {
                            store.forget_mini_app_load(&key);
                        }
                    });
                });
            })
        };
        let on_error: SaiWsOnError = {
            let (key, token) = (key.to_owned(), receipt.token.clone());
            Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                let (this, key, token) = (this.clone(), key.clone(), token.clone());
                cx.defer(move |cx: &mut App| {
                    let _ = this.update(cx, |store, cx| store.handle_mini_app_load_failure(&key, &token, cx));
                });
            })
        };
        SaiWs::request_slice(
            &slice,
            ConfirmMiniAppLoaded { load_receipt: receipt.token.clone() },
            Self::sensitive_request_options(rise_wire_methods::bot::CONFIRM_MINI_APP_LOADED, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    fn handle_mini_app_load_failure(&mut self, key: &str, token: &str, cx: &mut Context<Self>) {
        let Some(current) = self.mini_app_loads.get_mut(key).filter(|current| current.token == token) else {
            return;
        };
        current.attempts += 1;
        let attempts = current.attempts;
        let expires_at = current.expires_at;
        self.mini_app_load_requests.remove(key);
        let base = RiseLimits::client_value("mini_app_load_retry_delay_ms");
        let maximum = RiseLimits::client_value("mini_app_load_retry_max_delay_ms");
        let delay = maximum.min(base * (1_i64 << (attempts - 1).min(MINI_APP_LOAD_RETRY_MAX_EXPONENT)));
        if expires_at <= Self::mini_app_load_now() + delay {
            self.forget_mini_app_load(key);
            return;
        }
        let retry_key = key.to_owned();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay as u64)).await;
            let _ = this.update(cx, |store, cx| store.send_mini_app_load_confirmation(&retry_key, cx));
        });
        self.mini_app_load_retries.insert(key.to_owned(), task);
    }
}

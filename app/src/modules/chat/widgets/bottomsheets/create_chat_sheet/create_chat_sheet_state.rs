use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{App, Context, Entity, RenderImage, Subscription};
use rise_canon::{RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_validate::CHARSET_TAG;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{SaiDebouncer, SaiDebouncerOptions};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWsOnError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::stores::tag::tag_actions::TagActionsStore;
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::post::stores::post_actions::post_types::PostCollaboratorUser;

pub type ChatPrivacyType = rise_chat::PrivacyType;

/// chat field rules addressed by wire pointer
/// shared by the create sheet, its state and the chat type settings screen so they stay in sync
pub enum ChatFieldRules {}

static TITLE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("chat_create_chat", "/title"));
static DESCRIPTION: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("chat_create_chat", "/description"));
static TAG: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("chat_create_chat", "/tag"));

impl ChatFieldRules {
    pub fn title() -> RiseFieldRule {
        *TITLE
    }

    pub fn description() -> RiseFieldRule {
        *DESCRIPTION
    }

    pub fn tag() -> RiseFieldRule {
        *TAG
    }
}

type CreateChatSheetAfterDismiss = Box<dyn FnOnce(&mut App) + 'static>;

pub struct CreateChatSheetState {
    is_open: bool,
    current_page: String,
    group_title: String,
    pub group_description: String,
    pub group_tag: String,
    pub group_privacy: ChatPrivacyType,
    pub forward_in_group_blocked: bool,
    pub group_members: Vec<PostCollaboratorUser>,
    pub group_local_logo_file: Option<SaiWsOptimisticLocalFile>,
    pub group_logo_preview: Option<Arc<RenderImage>>,

    pub channel_title: String,
    pub channel_tag: String,
    pub channel_description: String,
    pub channel_privacy: ChatPrivacyType,
    pub forward_in_channel_blocked: bool,
    pub channel_local_logo_file: Option<SaiWsOptimisticLocalFile>,
    pub channel_logo_preview: Option<Arc<RenderImage>>,

    pub logo_upload_progress: f64,
    pub logo_uploading: bool,
    pub is_creating: bool,
    group_create_request_id: String,
    channel_create_request_id: String,
    pub create_error: Option<String>,
    pub create_error_is_channel: bool,
    pending_open_chat_id: Option<String>,
    after_dismiss: Option<CreateChatSheetAfterDismiss>,
    after_dismiss_generation: u64,

    pub channel_tag_error: String,
    pub channel_tag_checking: bool,
    channel_tag_check_generation: u64,
    pub group_tag_error: String,
    pub group_tag_checking: bool,
    group_tag_check_generation: u64,

    paywall_request_count: u64,
    _upsell_observation: Subscription,
}

impl CreateChatSheetState {
    const CHANNEL_TAG_CHECK_DEBOUNCE_ID: &'static str = "channelTagCheck";
    const CHANNEL_TAG_CHECK_GROUP_KEY: &'static str = "createChannelTag";
    const GROUP_TAG_CHECK_DEBOUNCE_ID: &'static str = "groupTagCheck";
    const GROUP_TAG_CHECK_GROUP_KEY: &'static str = "createGroupTag";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let upsell = PremiumUpsellStore::shared(cx);
        let paywall_request_count = upsell.read(cx).paywall_request_count();
        let upsell_observation = cx.observe(&upsell, |this, upsell, cx| {
            let count = upsell.read(cx).paywall_request_count();
            if count == this.paywall_request_count {
                return;
            }
            this.paywall_request_count = count;
            if this.is_open {
                this.close(cx);
            }
        });
        Self {
            is_open: false,
            current_page: String::new(),
            group_title: String::new(),
            group_description: String::new(),
            group_tag: String::new(),
            group_privacy: ChatPrivacyType::PrivateChat,
            forward_in_group_blocked: false,
            group_members: Vec::new(),
            group_local_logo_file: None,
            group_logo_preview: None,
            channel_title: String::new(),
            channel_tag: String::new(),
            channel_description: String::new(),
            channel_privacy: ChatPrivacyType::PrivateChat,
            forward_in_channel_blocked: false,
            channel_local_logo_file: None,
            channel_logo_preview: None,
            logo_upload_progress: 0.0,
            logo_uploading: false,
            is_creating: false,
            group_create_request_id: Self::new_request_id(),
            channel_create_request_id: Self::new_request_id(),
            create_error: None,
            create_error_is_channel: false,
            pending_open_chat_id: None,
            after_dismiss: None,
            after_dismiss_generation: 0,
            channel_tag_error: String::new(),
            channel_tag_checking: false,
            channel_tag_check_generation: 0,
            group_tag_error: String::new(),
            group_tag_checking: false,
            group_tag_check_generation: 0,
            paywall_request_count,
            _upsell_observation: upsell_observation,
        }
    }

    fn new_request_id() -> String {
        Uuid::new_v4().uuid_string().to_lowercase()
    }

    fn tag_check_delay_ms() -> i64 {
        static DELAY: LazyLock<i64> =
            LazyLock::new(|| RiseTimingCanon::debounce("chat_tag_availability").client_debounce_ms);
        *DELAY
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// canon fact create_chat_sheet_opened, by level
    pub fn set_is_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        self.is_open = is_open;
        GuideFacts::set(RiseGuideSignal::CreateChatSheetOpened, is_open, cx);
        cx.notify();
    }

    pub fn current_page(&self) -> &str {
        &self.current_page
    }

    /// the page the deck has on top, fed by the sheet
    /// two canon facts hang off it and a closed sheet has no page
    pub fn set_current_page(&mut self, current_page: &str, cx: &mut Context<Self>) {
        self.current_page = current_page.to_owned();
        let open = self.is_open;
        let page = self.current_page.as_str();
        GuideFacts::set(RiseGuideSignal::CreateGroupFormOpened, open && page == "group", cx);
        GuideFacts::set(RiseGuideSignal::CreateGroupSettingsOpened, open && page == "groupSettings", cx);
        GuideFacts::set(RiseGuideSignal::CreateBotFormOpened, open && page == "bot", cx);
        GuideFacts::set(RiseGuideSignal::CreateBotSettingsOpened, open && page == "botSettingsCreate", cx);
        cx.notify();
    }

    pub fn group_title(&self) -> &str {
        &self.group_title
    }

    /// canon fact create_group_name_filled, same rule the next button enables on
    pub fn set_group_title(&mut self, group_title: &str, cx: &mut Context<Self>) {
        self.group_title = group_title.to_owned();
        GuideFacts::set(RiseGuideSignal::CreateGroupNameFilled, !self.group_form_disabled(), cx);
        cx.notify();
    }

    pub fn group_create_request_id(&self) -> &str {
        &self.group_create_request_id
    }

    pub fn channel_create_request_id(&self) -> &str {
        &self.channel_create_request_id
    }

    pub fn group_form_disabled(&self) -> bool {
        !ChatFieldRules::title().is_satisfied(&self.group_title)
    }

    pub fn channel_tag_debouncing(&self, cx: &App) -> bool {
        SaiDebouncer::is_debouncing(Self::CHANNEL_TAG_CHECK_DEBOUNCE_ID, Self::CHANNEL_TAG_CHECK_GROUP_KEY, cx)
    }

    pub fn channel_tag_validating(&self, cx: &App) -> bool {
        self.channel_tag_checking || self.channel_tag_debouncing(cx)
    }

    pub fn group_tag_validating(&self, cx: &App) -> bool {
        self.group_tag_checking
            || SaiDebouncer::is_debouncing(Self::GROUP_TAG_CHECK_DEBOUNCE_ID, Self::GROUP_TAG_CHECK_GROUP_KEY, cx)
    }

    pub fn channel_form_disabled(&self) -> bool {
        !ChatFieldRules::title().is_satisfied(&self.channel_title)
    }

    pub fn group_settings_form_disabled(&self, cx: &App) -> bool {
        self.group_privacy == ChatPrivacyType::PublicChat
            && !self.group_tag.is_empty()
            && (!self.group_tag_error.is_empty() || self.group_tag_validating(cx))
    }

    pub fn channel_settings_form_disabled(&self, cx: &App) -> bool {
        self.channel_privacy == ChatPrivacyType::PublicChat
            && (self.channel_tag.is_empty() || !self.channel_tag_error.is_empty() || self.channel_tag_validating(cx))
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X844", had_logo = self.group_local_logo_file.is_some(), "create chat sheet reset");
        self.set_group_title("", cx);
        self.group_description = String::new();
        self.group_tag = String::new();
        self.group_privacy = ChatPrivacyType::PrivateChat;
        self.forward_in_group_blocked = false;
        self.group_members = Vec::new();
        self.group_local_logo_file = None;
        self.group_logo_preview = None;
        self.channel_title = String::new();
        self.channel_tag = String::new();
        self.channel_description = String::new();
        self.channel_privacy = ChatPrivacyType::PrivateChat;
        self.forward_in_channel_blocked = false;
        self.channel_local_logo_file = None;
        self.channel_logo_preview = None;
        self.channel_tag_error = String::new();
        self.channel_tag_checking = false;
        self.channel_tag_check_generation = 0;
        self.group_tag_error = String::new();
        self.group_tag_checking = false;
        self.group_tag_check_generation = 0;
        SaiDebouncer::cancel(Self::CHANNEL_TAG_CHECK_DEBOUNCE_ID, Self::CHANNEL_TAG_CHECK_GROUP_KEY, cx);
        SaiDebouncer::cancel(Self::GROUP_TAG_CHECK_DEBOUNCE_ID, Self::GROUP_TAG_CHECK_GROUP_KEY, cx);
        self.logo_upload_progress = 0.0;
        self.logo_uploading = false;
        self.is_creating = false;
        self.group_create_request_id = Self::new_request_id();
        self.channel_create_request_id = Self::new_request_id();
        self.create_error = None;
        self.create_error_is_channel = false;
        self.pending_open_chat_id = None;
        cx.notify();
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.reset(cx);
        self.set_is_open(true, cx);
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.set_is_open(false, cx);
        self.set_current_page("", cx);
    }

    pub fn close_then(&mut self, action: impl FnOnce(&mut App) + 'static, cx: &mut Context<Self>) {
        self.after_dismiss_generation += 1;
        let generation = self.after_dismiss_generation;
        self.after_dismiss = Some(Box::new(action));
        self.close(cx);
        let window = Duration::from_secs_f64(BottomSheetMetrics::PRESENTATION_RETRY_WINDOW);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(window).await;
            this.update(cx, |state, cx| {
                if state.after_dismiss_generation == generation {
                    state.run_after_dismiss(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        self.run_after_dismiss(cx);
    }

    fn run_after_dismiss(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.after_dismiss.take() else {
            return;
        };
        cx.defer(move |cx| action(cx));
    }

    pub fn close_after_creating(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if !self.is_open {
            return;
        }
        if !chat_id.is_empty() {
            self.pending_open_chat_id = Some(chat_id.to_owned());
        }
        self.close(cx);
    }

    pub fn take_pending_open_chat_id(&mut self) -> Option<String> {
        self.pending_open_chat_id.take()
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.set_is_open(false, cx);
        self.reset(cx);
    }

    pub fn patch_logo_upload(&mut self, progress: f64, uploading: bool, cx: &mut Context<Self>) {
        self.logo_upload_progress = progress;
        self.logo_uploading = uploading;
        cx.notify();
    }

    pub fn set_channel_tag(&mut self, raw: &str, cx: &mut Context<Self>) {
        let typed = Self::tag_input(raw);
        let sanitized = Self::tag_alphabet(&typed);
        let has_invalid_characters = typed != sanitized;

        if sanitized != self.channel_tag {
            self.channel_tag.clone_from(&sanitized);
        }

        if has_invalid_characters {
            self.channel_tag_error = tr("tag_invalid_characters");
            SaiDebouncer::cancel(Self::CHANNEL_TAG_CHECK_DEBOUNCE_ID, Self::CHANNEL_TAG_CHECK_GROUP_KEY, cx);
            self.channel_tag_checking = false;
            self.channel_tag_check_generation += 1;
            cx.notify();
            return;
        }

        self.channel_tag_error = String::new();
        self.schedule_channel_tag_check(sanitized, cx);
        cx.notify();
    }

    pub fn set_group_tag(&mut self, raw: &str, cx: &mut Context<Self>) {
        let typed = Self::tag_input(raw);
        let sanitized = Self::tag_alphabet(&typed);
        let has_invalid_characters = typed != sanitized;
        self.group_tag.clone_from(&sanitized);
        if has_invalid_characters {
            self.group_tag_error = tr("tag_invalid_characters");
            self.group_tag_checking = false;
            self.group_tag_check_generation += 1;
            SaiDebouncer::cancel(Self::GROUP_TAG_CHECK_DEBOUNCE_ID, Self::GROUP_TAG_CHECK_GROUP_KEY, cx);
            cx.notify();
            return;
        }
        self.group_tag_error = String::new();
        self.schedule_group_tag_check(sanitized, cx);
        cx.notify();
    }

    /// what the rule leaves in the field while typing, leading @ and spaces go
    /// case folds, chat and user tags share one unique index
    /// headroom past the ceiling stays so the field says too long
    fn tag_input(value: &str) -> String {
        match ChatFieldRules::tag().schema {
            Some(schema) => schema.input(value),
            None => value.to_owned(),
        }
    }

    /// drops what the canon alphabet doesnt allow, anything dropped is a foreign char the field
    /// complains about
    fn tag_alphabet(value: &str) -> String {
        value.chars().filter(|character| CHARSET_TAG.allows(*character)).collect()
    }

    fn schedule_channel_tag_check(&mut self, tag: String, cx: &mut Context<Self>) {
        SaiDebouncer::cancel(Self::CHANNEL_TAG_CHECK_DEBOUNCE_ID, Self::CHANNEL_TAG_CHECK_GROUP_KEY, cx);

        if let Some(error_key) = ChatFieldRules::tag().schema.and_then(|schema| schema.error_key(&tag)) {
            self.channel_tag_checking = false;
            self.channel_tag_check_generation += 1;
            if !tag.is_empty() {
                self.channel_tag_error = tr(error_key);
            }
            return;
        }

        let this = cx.weak_entity();
        SaiDebouncer::debounced_action_id(
            Self::CHANNEL_TAG_CHECK_DEBOUNCE_ID,
            Box::new(move |cx: &mut App| {
                this.update(cx, |state, cx| state.run_channel_tag_check(tag, cx)).ok();
            }),
            SaiDebouncerOptions {
                delay_milliseconds: Self::tag_check_delay_ms(),
                group_key: Self::CHANNEL_TAG_CHECK_GROUP_KEY.to_owned(),
                ..SaiDebouncerOptions::default()
            },
            cx,
        );
    }

    fn run_channel_tag_check(&mut self, tag: String, cx: &mut Context<Self>) {
        self.channel_tag_check_generation += 1;
        let generation = self.channel_tag_check_generation;
        self.channel_tag_checking = true;
        cx.notify();

        let on_success_state = cx.weak_entity();
        let on_error_state = cx.weak_entity();
        let checked_tag = tag.clone();
        let on_error: SaiWsOnError = Rc::new(move |_error: &anyhow::Error, cx: &mut App| {
            on_error_state
                .update(cx, |state, cx| {
                    if generation != state.channel_tag_check_generation {
                        return;
                    }
                    state.channel_tag_checking = false;
                    cx.notify();
                })
                .ok();
        });
        cx.defer(move |cx| {
            TagActionsStore::shared(cx).update(cx, |store, cx| {
                store.check_tag_exist_action(
                    &tag,
                    move |dto, cx| {
                        on_success_state
                            .update(cx, |state, cx| {
                                if generation != state.channel_tag_check_generation
                                    || trimming_whitespaces_and_newlines(&state.channel_tag) != checked_tag
                                {
                                    return;
                                }
                                state.channel_tag_checking = false;
                                state.channel_tag_error =
                                    if dto.exists { tr("tag_already_exists") } else { String::new() };
                                cx.notify();
                            })
                            .ok();
                    },
                    Some(on_error),
                    cx,
                );
            });
        });
    }

    fn schedule_group_tag_check(&mut self, tag: String, cx: &mut Context<Self>) {
        SaiDebouncer::cancel(Self::GROUP_TAG_CHECK_DEBOUNCE_ID, Self::GROUP_TAG_CHECK_GROUP_KEY, cx);
        if tag.is_empty() {
            self.group_tag_checking = false;
            self.group_tag_error = String::new();
            return;
        }
        if let Some(error_key) = ChatFieldRules::tag().schema.and_then(|schema| schema.error_key(&tag)) {
            self.group_tag_checking = false;
            self.group_tag_check_generation += 1;
            self.group_tag_error = tr(error_key);
            return;
        }
        let this = cx.weak_entity();
        SaiDebouncer::debounced_action_id(
            Self::GROUP_TAG_CHECK_DEBOUNCE_ID,
            Box::new(move |cx: &mut App| {
                this.update(cx, |state, cx| state.run_group_tag_check(tag, cx)).ok();
            }),
            SaiDebouncerOptions {
                delay_milliseconds: Self::tag_check_delay_ms(),
                group_key: Self::GROUP_TAG_CHECK_GROUP_KEY.to_owned(),
                ..SaiDebouncerOptions::default()
            },
            cx,
        );
    }

    fn run_group_tag_check(&mut self, tag: String, cx: &mut Context<Self>) {
        self.group_tag_check_generation += 1;
        let generation = self.group_tag_check_generation;
        self.group_tag_checking = true;
        cx.notify();

        let on_success_state = cx.weak_entity();
        let on_error_state = cx.weak_entity();
        let checked_tag = tag.clone();
        let on_error: SaiWsOnError = Rc::new(move |_error: &anyhow::Error, cx: &mut App| {
            on_error_state
                .update(cx, |state, cx| {
                    if generation != state.group_tag_check_generation {
                        return;
                    }
                    state.group_tag_checking = false;
                    state.group_tag_error = tr("tag_check_failed");
                    cx.notify();
                })
                .ok();
        });
        cx.defer(move |cx| {
            TagActionsStore::shared(cx).update(cx, |store, cx| {
                store.check_tag_exist_action(
                    &tag,
                    move |dto, cx| {
                        on_success_state
                            .update(cx, |state, cx| {
                                if generation != state.group_tag_check_generation || state.group_tag != checked_tag {
                                    return;
                                }
                                state.group_tag_checking = false;
                                state.group_tag_error =
                                    if dto.exists { tr("tag_already_exists") } else { String::new() };
                                cx.notify();
                            })
                            .ok();
                    },
                    Some(on_error),
                    cx,
                );
            });
        });
    }
}

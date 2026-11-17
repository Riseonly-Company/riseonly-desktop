use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{
    App, Axis, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, PromptButton, PromptLevel, SharedString,
    Subscription, Task, Window,
};
use rise_canon::rise_key::user as rise_key_user;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_i18n::{AppDateFormatter, tr};
use rise_platform::haptics::selection_changed;
use rise_theme::theme::app_theme::AppTheme;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::guide_overlay_ui::guide_overlay_window::GuideOverlayHost;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::section_reveal_ui::SectionRevealUi;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::date_picker::{DatePicker, DatePickerComponents};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::layer_keys::LayerKeysExt;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_fullscreen_pop::NavigationInteractivePopGuard;
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::{RiseFieldRule, RiseFocusedTextInput, RiseFocusedTextInputOutcome};
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::stores::tag::tag_actions::TagActionsStore;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::settings_field_row::{SettingsFieldRow, SettingsFieldStatus};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::story::shared::story_stealth_preference::StoryStealthPreference;
use crate::modules::user::components::premium_badge_picker::premium_badge_emoji_picker::PremiumBadgeEmojiPicker;
use crate::modules::user::pages::profile::settings::profile_settings::profile_avatar_picker_block::{
    ProfileAvatarPickerBlock, ProfileEditorAvatar,
};
use crate::modules::user::pages::profile::settings::profile_settings::profile_banner_picker_block::ProfileBannerPickerBlock;
use crate::modules::user::shared::profile_editor_draft::{ProfileEditorDraft, ProfileEditorDraftStore};
use crate::modules::user::shared::profile_field_rules::ProfileFieldRules;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const PASTE_KEY: &str = "v";
const DEFAULT_BIRTHDAY_AGE_YEARS: i64 = 18;
const BIRTHDAY_DAY_FORMAT: &str = "yyyy-MM-dd";
const BIRTHDAY_DAY_LOCALE: &str = "en_US_POSIX";
const EARLIEST_BIRTHDAY_SECONDS: f64 = -62_135_596_800.0;
const BIRTHDAY_PICKER_MAXIMUM_HEIGHT: f32 = 480.0;
const BADGE_PICKER_MAXIMUM_HEIGHT: f32 = 376.0;
const BADGE_PICKER_VIEWPORT_FRACTION: f32 = 0.44;
const NAME_COLOR_EXPANDED_HEIGHT: f32 = 132.0;

const NAME_COLOR_PALETTE: [&str; 10] = [
    "#FF3B30", "#FF9500", "#FFCC00", "#34C759", "#00C7BE", "#007AFF", "#5856D6", "#AF52DE", "#FF2D55", "#A2845E",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum TagAvailability {
    Idle,
    Checking,
    Available,
    Taken,
    Failed,
}

#[derive(Default)]
struct EditPatch {
    name: Option<String>,
    tag: Option<String>,
    description: Option<String>,
    hb: Option<String>,
}

impl EditPatch {
    fn is_empty(&self) -> bool {
        self.name.is_none() && self.tag.is_none() && self.description.is_none() && self.hb.is_none()
    }
}

struct ProfileSettingsState {
    values: HashMap<&'static str, String>,
    field_errors: HashMap<&'static str, String>,
    hb: Date,
    has_hb: bool,
    hb_picker_open: bool,
    hb_picker_motion: Animated<f32>,
    did_hydrate_from_auth: bool,
    initial_name: String,
    initial_tag: String,
    initial_description: String,
    initial_hb_iso: Option<String>,
    badge_picker_open: bool,
    name_color_picker_open: bool,
    tag_availability: TagAvailability,
    tag_check_task: Option<Task<()>>,
    unsaved_alert_shown: bool,
    exit_resolved: bool,
    story_stealth_enabled: bool,
    profile_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl ProfileSettingsState {
    /// seeded at build time not only on appear,
    /// a field copies its text when built and a re-pushed screen showed empty fields
    fn new(nav_store: Option<Entity<NavigationStore>>, cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let user_service = UserServicesStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let now = Date::now();
        let mut state = Self {
            values: HashMap::from([
                (rise_key_user::NAME, String::new()),
                (rise_key_user::TAG, String::new()),
                (rise_key_user::DESCRIPTION, String::new()),
            ]),
            field_errors: HashMap::new(),
            hb: Calendar::current()
                .date_by_adding(CalendarComponent::Year, -DEFAULT_BIRTHDAY_AGE_YEARS, &now)
                .unwrap_or(now),
            has_hb: false,
            hb_picker_open: false,
            hb_picker_motion: Animated::new(0.0),
            did_hydrate_from_auth: false,
            initial_name: String::new(),
            initial_tag: String::new(),
            initial_description: String::new(),
            initial_hb_iso: None,
            badge_picker_open: false,
            name_color_picker_open: false,
            tag_availability: TagAvailability::Idle,
            tag_check_task: None,
            unsaved_alert_shown: false,
            exit_resolved: false,
            story_stealth_enabled: StoryStealthPreference::is_enabled(),
            profile_id: None,
            nav_store,
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![
                cx.observe(&auth, |state: &mut Self, _, cx| {
                    let profile_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
                    if state.profile_id != profile_id {
                        state.profile_id = profile_id;
                        state.hydrate_from_profile_if_needed(cx);
                    }
                    cx.notify();
                }),
                cx.observe(&user_service, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        };
        if let Some(profile) = auth.read(cx).profile().cloned() {
            let description = profile.description.clone().unwrap_or_default();
            state.profile_id = Some(profile.id.clone());
            state.values.insert(rise_key_user::NAME, profile.name.clone());
            state.values.insert(rise_key_user::TAG, profile.tag.clone());
            state.values.insert(rise_key_user::DESCRIPTION, description.clone());
            state.initial_name = profile.name;
            state.initial_tag = profile.tag;
            state.initial_description = description;
            let raw_hb = trimming_whitespaces_and_newlines(profile.hb.as_deref().unwrap_or_default()).to_owned();
            if let Some(date) = Self::parse_birthday(&raw_hb) {
                state.hb = date;
                state.has_hb = true;
                state.initial_hb_iso = Some(Self::iso_string(&date));
            }
        }
        state
    }

    fn value(&self, key: &str) -> String {
        self.values.get(key).cloned().unwrap_or_default()
    }

    fn trimmed_tag(&self) -> String {
        trimming_whitespaces_and_newlines(&self.value(rise_key_user::TAG)).to_owned()
    }

    fn tag_changed(&self) -> bool {
        self.trimmed_tag() != self.initial_tag
    }

    fn hb_iso_string(&self) -> String {
        Self::iso_string(&self.hb)
    }

    /// what would go on the wire if the tick were pressed now, nothing is sent on leaving
    fn pending_patch(&self) -> EditPatch {
        let new_name = trimming_whitespaces_and_newlines(&self.value(rise_key_user::NAME)).to_owned();
        let new_tag = self.trimmed_tag();
        let new_description = trimming_whitespaces_and_newlines(&self.value(rise_key_user::DESCRIPTION)).to_owned();
        let new_hb = self.has_hb.then(|| self.hb_iso_string());
        let hb_changed = new_hb != self.initial_hb_iso;
        EditPatch {
            name: (new_name != self.initial_name).then_some(new_name),
            tag: (new_tag != self.initial_tag).then_some(new_tag),
            description: (new_description != self.initial_description).then_some(new_description),
            hb: hb_changed.then(|| new_hb.unwrap_or_default()),
        }
    }

    fn has_unsaved_changes(&self) -> bool {
        !self.pending_patch().is_empty()
    }

    fn tag_error_message(&self) -> Option<String> {
        if let Some(message) = self.field_errors.get(rise_key_user::TAG) {
            return Some(message.clone());
        }
        if self.tag_changed() && self.tag_availability == TagAvailability::Taken {
            return Some(tr("tag_already_exists"));
        }
        if self.tag_changed() && self.tag_availability == TagAvailability::Failed {
            return Some(tr("auth_tag_check_failed"));
        }
        None
    }

    fn tag_check_is_retryable(&self) -> bool {
        !self.field_errors.contains_key(rise_key_user::TAG)
            && self.tag_changed()
            && self.tag_availability == TagAvailability::Failed
    }

    fn tag_field_status(&self) -> Option<SettingsFieldStatus> {
        if !self.tag_changed() {
            return None;
        }
        match self.tag_availability {
            TagAvailability::Checking => Some(SettingsFieldStatus::Checking),
            TagAvailability::Available => Some(SettingsFieldStatus::Accepted),
            TagAvailability::Idle | TagAvailability::Taken | TagAvailability::Failed => None,
        }
    }

    fn pop(&self, cx: &mut Context<Self>) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn account_id(cx: &mut Context<Self>) -> String {
        AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default()
    }

    fn commit_and_pop(&mut self, cx: &mut Context<Self>) {
        if self.exit_resolved {
            return;
        }
        self.exit_resolved = true;
        let mut patch = self.pending_patch();
        if patch.tag.as_deref().is_some_and(|tag| {
            !ProfileFieldRules::tag().is_satisfied(tag) || self.tag_availability == TagAvailability::Taken
        }) {
            patch.tag = None;
        }
        if !patch.is_empty() {
            UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.save_profile_edits(
                    patch.name,
                    patch.tag,
                    patch.description,
                    patch.hb,
                    None,
                    None,
                    None,
                    cx,
                );
            });
        }
        ProfileEditorDraftStore::clear(&Self::account_id(cx));
        self.pop(cx);
    }

    fn discard_and_pop(&mut self, cx: &mut Context<Self>) {
        if self.exit_resolved {
            return;
        }
        self.exit_resolved = true;
        ProfileEditorDraftStore::clear(&Self::account_id(cx));
        self.pop(cx);
    }

    fn keep_for_later_and_pop(&mut self, cx: &mut Context<Self>) {
        if self.exit_resolved {
            return;
        }
        self.exit_resolved = true;
        let draft = ProfileEditorDraft {
            name: self.value(rise_key_user::NAME),
            tag: self.value(rise_key_user::TAG),
            description: self.value(rise_key_user::DESCRIPTION),
            hb_iso: self.has_hb.then(|| self.hb_iso_string()),
            has_hb: self.has_hb,
        };
        ProfileEditorDraftStore::save(&draft, &Self::account_id(cx));
        self.pop(cx);
    }

    /// unsaved alert,
    /// buttons are discard and keep,
    /// a dialog dismissed without a button keeps the draft
    fn present_unsaved_alert(state: &Entity<Self>, cx: &mut App) {
        if state.read(cx).unsaved_alert_shown {
            return;
        }
        state.update(cx, |state, _| state.unsaved_alert_shown = true);
        let deferred_state = state.clone();
        cx.defer(move |cx| Self::show_unsaved_alert(&deferred_state, cx));
    }

    fn show_unsaved_alert(state: &Entity<Self>, cx: &mut App) {
        let guide = GuideOverlayHost::shared(cx);
        let host = if guide.is_presenting() {
            guide.alert_host()
        } else {
            cx.active_window().or_else(|| cx.windows().first().copied())
        };
        let title = tr("profile_unsaved_title");
        let message = tr("profile_unsaved_message");
        let buttons = [PromptButton::ok(tr("profile_unsaved_discard")), PromptButton::cancel(tr("profile_unsaved_keep"))];
        let receiver: Option<oneshot::Receiver<usize>> = host.and_then(|host| {
            host.update(cx, |_, window, cx| {
                window.prompt(PromptLevel::Warning, &title, Some(message.as_str()), &buttons, cx)
            })
            .ok()
        });
        let Some(receiver) = receiver else {
            state.update(cx, |state, _| state.unsaved_alert_shown = false);
            return;
        };
        let answering_state = state.clone();
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let answer = receiver.await;
            answering_state.update(cx, |state, cx| {
                state.unsaved_alert_shown = false;
                if answer == Ok(0) {
                    state.discard_and_pop(cx);
                } else if state.has_unsaved_changes() {
                    state.keep_for_later_and_pop(cx);
                }
            });
        })
        .detach();
    }

    fn request_exit(state: &Entity<Self>, cx: &mut App) {
        if state.read(cx).exit_resolved {
            return;
        }
        if !state.read(cx).has_unsaved_changes() {
            state.update(cx, |state, cx| {
                state.exit_resolved = true;
                state.pop(cx);
            });
            return;
        }
        Self::present_unsaved_alert(state, cx);
    }

    fn schedule_tag_check(&mut self, cx: &mut Context<Self>) {
        self.tag_check_task = None;
        let candidate = self.trimmed_tag();
        if candidate == self.initial_tag
            || candidate.is_empty()
            || self.field_errors.contains_key(rise_key_user::TAG)
            || !ProfileFieldRules::tag().is_satisfied(&candidate)
        {
            self.tag_availability = TagAvailability::Idle;
            cx.notify();
            return;
        }
        self.tag_availability = TagAvailability::Checking;
        cx.notify();
        let debounce_ms = RiseTimingCanon::debounce("chat_tag_availability").client_debounce_ms;
        self.tag_check_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(u64::try_from(debounce_ms).unwrap_or(0))).await;
            if !this.update(cx, |state, _| candidate == state.trimmed_tag()).unwrap_or(false) {
                return;
            }
            cx.update(|cx| {
                let success_candidate = candidate.clone();
                let failure_candidate = candidate.clone();
                let on_success_state = this.clone();
                let on_error_state = this.clone();
                TagActionsStore::shared(cx).update(cx, |tags, cx| {
                    tags.check_tag_exist_action(
                        &candidate,
                        move |response, cx| {
                            let _ = on_success_state.update(cx, |state, cx| {
                                if success_candidate != state.trimmed_tag() {
                                    return;
                                }
                                state.tag_availability =
                                    if response.exists { TagAvailability::Taken } else { TagAvailability::Available };
                                cx.notify();
                            });
                        },
                        Some(Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                            let _ = on_error_state.update(cx, |state, cx| {
                                if failure_candidate != state.trimmed_tag() {
                                    return;
                                }
                                state.tag_availability = TagAvailability::Failed;
                                cx.notify();
                            });
                        })),
                        cx,
                    );
                });
            });
        }));
    }

    fn hydrate_from_profile_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.did_hydrate_from_auth {
            return;
        }
        let Some(profile) = AuthServicesStore::shared(cx).read(cx).profile().cloned() else {
            return;
        };
        self.did_hydrate_from_auth = true;
        let description = profile.description.clone().unwrap_or_default();
        self.values.insert(rise_key_user::NAME, profile.name.clone());
        self.values.insert(rise_key_user::TAG, profile.tag.clone());
        self.values.insert(rise_key_user::DESCRIPTION, description.clone());
        self.initial_name = profile.name;
        self.initial_tag = profile.tag;
        self.initial_description = description;

        let raw_hb = trimming_whitespaces_and_newlines(profile.hb.as_deref().unwrap_or_default()).to_owned();
        if let Some(date) = Self::parse_birthday(&raw_hb) {
            self.hb = date;
            self.has_hb = true;
            self.initial_hb_iso = Some(Self::iso_string(&date));
        } else {
            self.has_hb = false;
            self.initial_hb_iso = None;
        }
        self.apply_draft_if_any(&profile.id, cx);
        cx.notify();
    }

    fn apply_draft_if_any(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let Some(draft) = ProfileEditorDraftStore::load(user_id) else {
            return;
        };
        self.set_value(rise_key_user::NAME, ProfileFieldRules::name(), draft.name, cx);
        self.set_value(rise_key_user::TAG, ProfileFieldRules::tag(), draft.tag, cx);
        self.set_value(rise_key_user::DESCRIPTION, ProfileFieldRules::description(), draft.description, cx);
        let draft_date = if draft.has_hb { draft.hb_iso.as_deref().and_then(Self::parse_birthday) } else { None };
        if let Some(date) = draft_date {
            self.hb = date;
            self.has_hb = true;
        } else {
            self.has_hb = false;
        }
    }

    /// what the field keeps of the typed text
    /// canon input slack wider than the rule (tag 40 vs 32) is the clamp
    fn kept_value(rule: RiseFieldRule, raw: &str) -> String {
        match rule.schema {
            Some(schema) if schema.limit_while_typing().is_none() => schema.clamped(raw),
            Some(schema) => schema.input(raw),
            None => raw.to_owned(),
        }
    }

    /// sets a value with the full canon rule
    /// value is clamped and a violated rule lands in field_errors under the field
    fn set_value(&mut self, key: &'static str, rule: RiseFieldRule, raw: String, cx: &mut Context<Self>) {
        let Some(schema) = rule.schema else {
            self.values.insert(key, raw);
            cx.notify();
            return;
        };
        let value = Self::kept_value(rule, &raw);
        match schema.error_text(&value) {
            Some(message) => {
                self.field_errors.insert(key, message);
            }
            None => {
                self.field_errors.remove(key);
            }
        }
        self.values.insert(key, value);
        if key == rise_key_user::TAG {
            self.schedule_tag_check(cx);
        }
        cx.notify();
    }

    fn value_binding(state: &Entity<Self>, key: &'static str, rule: RiseFieldRule) -> Binding<String> {
        let reader = state.clone();
        let writer = state.clone();
        Binding::new(
            move |cx| reader.read(cx).value(key),
            move |raw, window, cx| {
                let kept = Self::kept_value(rule, &raw);
                let value = if kept == raw {
                    raw
                } else {
                    match RiseFocusedTextInput::clamp(&raw, &kept, window, cx) {
                        RiseFocusedTextInputOutcome::Composing => raw,
                        RiseFocusedTextInputOutcome::Clamped | RiseFocusedTextInputOutcome::NotFocused => kept,
                    }
                };
                writer.update(cx, |state, cx| state.set_value(key, rule, value, cx));
            },
        )
    }

    fn set_hb_picker_open(&mut self, hb_picker_open: bool, response: f64, damping_fraction: f64, cx: &mut Context<Self>) {
        self.hb_picker_open = hb_picker_open;
        self.hb_picker_motion.set_in(
            if hb_picker_open { 1.0 } else { 0.0 },
            spring(response, damping_fraction),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
    }

    fn formatted_hb(&self) -> String {
        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_style(Style::Long);
        formatter.set_time_style(Style::None);
        formatter.string_from(&self.hb)
    }

    fn iso_string(date: &Date) -> String {
        let mut formatter = ISO8601DateFormatter::new();
        formatter.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
        formatter.string_from(date)
    }

    fn parse_birthday(raw: &str) -> Option<Date> {
        if raw.is_empty() {
            return None;
        }
        if let Some(date) = ISO8601DateFormatter::new().date_from(raw) {
            return Some(date);
        }
        let mut day_formatter = DateFormatter::new();
        day_formatter.set_locale(BIRTHDAY_DAY_LOCALE);
        day_formatter.set_date_format(BIRTHDAY_DAY_FORMAT);
        day_formatter.date_from(raw)
    }
}

#[derive(IntoElement)]
pub struct ProfileSettings;

impl ProfileSettings {
    pub fn new() -> Self {
        Self
    }

    fn profile_field_items(state: &Entity<ProfileSettingsState>, theme: &AppTheme, window: &Window, cx: &mut App) -> Vec<GroupedBtn> {
        let is_premium = AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        let owns_profile_look_scene = GuideSandbox::shared(cx).read(cx).owns_profile_look_scene();
        let mut items = Self::profile_text_field_items(state, cx);
        if is_premium || owns_profile_look_scene {
            items.push(Self::premium_badge_item(state, theme, window, cx));
        }
        if is_premium {
            items.push(Self::name_color_item(state, theme, cx));
        }
        items
    }

    fn profile_text_field_items(state: &Entity<ProfileSettingsState>, cx: &mut App) -> Vec<GroupedBtn> {
        let (name_error, description_error, tag_error, tag_status, tag_retryable) = {
            let state = state.read(cx);
            (
                state.field_errors.get(rise_key_user::NAME).cloned(),
                state.field_errors.get(rise_key_user::DESCRIPTION).cloned(),
                state.tag_error_message(),
                state.tag_field_status(),
                state.tag_check_is_retryable(),
            )
        };
        let mut tag_row = SettingsFieldRow::new(
            "profile-settings-tag",
            "at",
            SettingsIconPalette::customization(),
            tr("tag_placeholder"),
            ProfileSettingsState::value_binding(state, rise_key_user::TAG, ProfileFieldRules::tag()),
        )
        .error_message(tag_error)
        .rule(ProfileFieldRules::tag())
        .status(tag_status);
        if tag_retryable {
            let retry_state = state.clone();
            tag_row = tag_row
                .on_error_tap(move |_, cx| retry_state.update(cx, |state, cx| state.schedule_tag_check(cx)));
        }
        vec![
            GroupedBtn::new(rise_key_user::NAME, "").content_slot(
                SettingsFieldRow::new(
                    "profile-settings-name",
                    "person.fill",
                    SettingsIconPalette::profile(),
                    tr("name_placeholder"),
                    ProfileSettingsState::value_binding(state, rise_key_user::NAME, ProfileFieldRules::name()),
                )
                .error_message(name_error)
                .rule(ProfileFieldRules::name()),
            ),
            GroupedBtn::new(rise_key_user::DESCRIPTION, "").content_slot(
                SettingsFieldRow::new(
                    "profile-settings-description",
                    "text.alignleft",
                    SettingsIconPalette::privacy_description(),
                    tr("description_placeholder"),
                    ProfileSettingsState::value_binding(
                        state,
                        rise_key_user::DESCRIPTION,
                        ProfileFieldRules::description(),
                    ),
                )
                .error_message(description_error)
                .rule(ProfileFieldRules::description())
                .axis(Axis::Vertical),
            ),
            GroupedBtn::new(rise_key_user::TAG, "").content_slot(tag_row),
        ]
    }

    #[allow(dead_code)]
    fn story_stealth_item(state: &Entity<ProfileSettingsState>) -> GroupedBtn {
        let reader = state.clone();
        let writer = state.clone();
        GroupedBtn::new("story_stealth", tr("settings_premium_story_stealth_title"))
            .icon("eye.slash")
            .icon_background(SettingsIconPalette::subscription())
            .show_chevron(false)
            .trailing_slot(SwitchUi::new(
                "profile-settings-story-stealth",
                Binding::new(
                    move |cx| reader.read(cx).story_stealth_enabled,
                    move |value, _, cx| {
                        writer.update(cx, |state, cx| {
                            state.story_stealth_enabled = value;
                            cx.notify();
                        });
                        StoryStealthPreference::set_is_enabled(value);
                        let _ = selection_changed();
                    },
                ),
            ))
    }

    fn name_color_item(state: &Entity<ProfileSettingsState>, theme: &AppTheme, cx: &mut App) -> GroupedBtn {
        let toggle_state = state.clone();
        GroupedBtn::new("premium_name_color", tr("settings_premium_name_color_choose"))
            .icon("paintpalette")
            .icon_background(SettingsIconPalette::subscription())
            .trailing_slot(Self::name_color_trailing(theme, cx))
            .show_chevron_when_trailing_slot(true)
            .expanded_slot(Self::name_color_expansion(state, theme, cx))
            .callback(move |_, cx| {
                let _ = selection_changed();
                toggle_state.update(cx, |state, cx| {
                    state.name_color_picker_open = !state.name_color_picker_open;
                    cx.notify();
                });
            })
    }

    fn current_name_color(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).profile().and_then(|profile| profile.more.name_color.clone())
    }

    fn name_color_trailing(theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let is_pending = {
            let user_service = UserServicesStore::shared(cx);
            user_service.read(cx).name_color_mutation_pending
        };
        let color = PremiumNameColor::parse(Self::current_name_color(cx).as_deref()).unwrap_or(theme.text.primary);
        let secondary = theme.text.secondary;
        hstack(8.0)
            .items_center()
            .when(is_pending, |trailing| {
                trailing.child(progress_view().control_size(ControlSize::Small).tint(secondary))
            })
            .child(
                div()
                    .size(px(22.0))
                    .rounded_full()
                    .bg(color)
                    .border(px(1.0))
                    .border_color(secondary.opacity(0.25)),
            )
    }

    fn name_color_expansion(state: &Entity<ProfileSettingsState>, theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let is_expanded = state.read(cx).name_color_picker_open;
        let current = Self::current_name_color(cx).unwrap_or_default();
        let selected_border = theme.text.primary;
        let swatches = hstack(8.0).w_full().children(NAME_COLOR_PALETTE.into_iter().map(|hex| {
            let mut swatch = div()
                .id(hex)
                .flex_1()
                .max_w(px(28.0))
                .aspect_square()
                .rounded_full()
                .cursor_pointer();
            if let Some(color) = PremiumNameColor::parse(Some(hex)) {
                swatch = swatch.bg(color);
            }
            if current == hex {
                swatch = swatch.border(px(2.0)).border_color(selected_border);
            }
            swatch.on_click(move |_, _, cx| {
                let _ = selection_changed();
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.set_name_color(Some(hex), cx));
            })
        }));

        SectionRevealUi::new(
            "profile-settings-name-color-reveal",
            is_expanded,
            NAME_COLOR_EXPANDED_HEIGHT,
            vstack(10.0)
                .w_full()
                .items_start()
                .pl(px(16.0))
                .pr(px(16.0))
                .py(px(8.0))
                .child(MainText::new(tr("settings_premium_name_color_hint")).px(13.0).secondary(true))
                .child(swatches)
                .when(!current.is_empty(), |content| {
                    content.child(
                        plain("profile-settings-name-color-reset")
                            .on_press(|_, cx| {
                                let _ = selection_changed();
                                UserInteractionsStore::shared(cx)
                                    .update(cx, |interactions, cx| interactions.set_name_color(None, cx));
                            })
                            .child(MainText::new(tr("settings_premium_name_color_reset")).px(13.0).primary(true)),
                    )
                }),
        )
    }

    fn premium_badge_item(state: &Entity<ProfileSettingsState>, theme: &AppTheme, window: &Window, cx: &mut App) -> GroupedBtn {
        let toggle_state = state.clone();
        GroupedBtn::new("premium_badge", tr("settings_premium_badge_choose"))
            .icon("sparkles")
            .icon_background(SettingsIconPalette::subscription())
            .guide_anchor(RiseGuideAnchor::ProfileBadgeRow)
            .trailing_slot(Self::premium_badge_trailing(theme, cx))
            .show_chevron_when_trailing_slot(true)
            .expanded_slot(Self::premium_badge_expansion(state, window, cx))
            .callback(move |_, cx| {
                let _ = selection_changed();
                toggle_state.update(cx, |state, cx| {
                    state.badge_picker_open = !state.badge_picker_open;
                    cx.notify();
                });
            })
    }

    fn premium_badge_trailing(theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let is_pending = {
            let user_service = UserServicesStore::shared(cx);
            user_service.read(cx).premium_badge_mutation_pending
        };
        let emoji = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .and_then(|profile| profile.more.premium_badge_emoji.clone())
            .map(SharedString::from);
        let secondary = theme.text.secondary;
        hstack(8.0)
            .items_center()
            .when(is_pending, |trailing| {
                trailing.child(progress_view().control_size(ControlSize::Small).tint(secondary))
            })
            .child(PremiumBadge::new().emoji(emoji).size(26.0))
    }

    fn badge_picker_viewport_height(window: &Window) -> f32 {
        let viewport_height = window.viewport_size().height / px(1.0);
        BADGE_PICKER_MAXIMUM_HEIGHT.min(viewport_height * BADGE_PICKER_VIEWPORT_FRACTION)
    }

    fn premium_badge_expansion(state: &Entity<ProfileSettingsState>, window: &Window, cx: &mut App) -> impl IntoElement + use<> {
        let is_expanded = state.read(cx).badge_picker_open;
        let viewport_height = Self::badge_picker_viewport_height(window);
        let commit_state = state.clone();
        SectionRevealUi::new(
            "profile-settings-badge-reveal",
            is_expanded,
            viewport_height + 8.0,
            div()
                .w_full()
                .pl(px(8.0))
                .pr(px(8.0))
                .pb(px(8.0))
                .child(
                    div().w_full().h(px(viewport_height)).child(PremiumBadgeEmojiPicker::new().on_committed(
                        move |_, cx| {
                            commit_state.update(cx, |state, cx| {
                                state.badge_picker_open = false;
                                cx.notify();
                            });
                        },
                    )),
                )
                .guide_anchor(RiseGuideAnchor::ProfileBadgePicker),
        )
    }

    fn profile_details_group(
        state: &Entity<ProfileSettingsState>,
        nav_store: Option<Entity<NavigationStore>>,
        theme: &AppTheme,
        window: &Window,
        cx: &mut App,
    ) -> GroupedBtns {
        let link_count = AuthServicesStore::shared(cx).read(cx).profile().map_or(0, |profile| profile.social_links.len());
        let (trailing, hb_picker_open, hb_picker_progress) = {
            let state = state.read(cx);
            (
                if state.has_hb { state.formatted_hb() } else { tr("not_selected") },
                state.hb_picker_open,
                state.hb_picker_motion.frame_value(RiseClock::monotonic(), window),
            )
        };
        let toggle_state = state.clone();
        let mut birthday = GroupedBtn::new("birthday", tr("hb"))
            .icon("calendar")
            .icon_background(SettingsIconPalette::privacy_birthday())
            .trailing(trailing)
            .trailing_keeps_width(true)
            .show_chevron(true)
            .callback(move |_, cx| {
                toggle_state.update(cx, |state, cx| {
                    let hb_picker_open = !state.hb_picker_open;
                    state.set_hb_picker_open(hb_picker_open, 0.4, 0.88, cx);
                });
            });
        if hb_picker_open || hb_picker_progress > 0.0 {
            birthday = birthday.expanded_slot(Self::birthday_picker(state, theme, hb_picker_progress));
        }
        let mut social_links = GroupedBtn::new("social-links", tr("social_links_title"))
            .icon("link")
            .icon_background(SettingsIconPalette::chat_folders())
            .callback(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| {
                        nav_store.push(RootRoute::SocialLinksEditor, NavigationStoreSettlingPush::Drop, cx);
                    });
                }
            });
        if link_count > 0 {
            social_links = social_links.trailing(link_count.to_string());
        }
        GroupedBtns::new("profile-settings-details", vec![birthday, social_links]).corner_radius(30.0)
    }

    fn birthday_picker(state: &Entity<ProfileSettingsState>, theme: &AppTheme, progress: f32) -> impl IntoElement + use<> {
        let delete_state = state.clone();
        let selection = Binding::entity(
            state.clone(),
            |state: &ProfileSettingsState| state.hb,
            |state: &mut ProfileSettingsState, hb, cx| {
                state.hb = hb;
                state.has_hb = true;
                cx.notify();
            },
        );
        let latest = Date::now();
        div()
            .w_full()
            .overflow_hidden()
            .max_h(px(BIRTHDAY_PICKER_MAXIMUM_HEIGHT * progress.max(0.0)))
            .opacity(progress.clamp(0.0, 1.0))
            .child(
                vstack(8.0)
                    .w_full()
                    .pb(px(6.0))
                    .child(
                        div().w_full().pl(px(52.0)).child(
                            div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.5)),
                        ),
                    )
                    .child(
                        div().w_full().px(px(16.0)).flex().justify_center().child(
                            DatePicker::new("profile-settings-birthday", selection)
                                .range(Date::from_time_interval_since_1970(EARLIEST_BIRTHDAY_SECONDS)..=latest)
                                .displayed_components(DatePickerComponents::Date)
                                .tint(theme.primary._100),
                        ),
                    )
                    .child(
                        plain("profile-settings-birthday-delete")
                            .on_press(move |_, cx| {
                                delete_state.update(cx, |state, cx| {
                                    state.has_hb = false;
                                    state.set_hb_picker_open(false, 0.36, 0.88, cx);
                                });
                            })
                            .w_full()
                            .child(
                                div().w_full().py(px(12.0)).child(
                                    MainText::new(tr("delete_hb"))
                                        .px(14.0)
                                        .tac(TextAlignT::Center)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.semantic.error_100),
                                ),
                            ),
                    ),
            )
    }

    fn header_block(state: &Entity<ProfileSettingsState>, cx: &mut App) -> impl IntoElement + use<> {
        let profile = AuthServicesStore::shared(cx).read(cx).profile().cloned();
        let user_id = profile.as_ref().map(|profile| profile.id.clone()).unwrap_or_default();
        let user_name = state.read(cx).value(rise_key_user::NAME);
        let avatar_source =
            profile.as_ref().map(|profile| profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()));
        let profile_photos_count = profile.as_ref().map_or(0, |profile| profile.profile_photos_count);
        let cover_image_url = profile.and_then(|profile| profile.cover_image_url);
        vstack(6.0)
            .w_full()
            .child(ProfileBannerPickerBlock::new(
                cover_image_url,
                ProfileEditorAvatar::new(user_id.clone(), user_name, avatar_source, profile_photos_count),
            ))
            .child(ProfileAvatarPickerBlock::new(user_id))
    }

    fn handle_key_down(event: &KeyDownEvent, cx: &mut App) {
        if event.keystroke.key != PASTE_KEY || !event.keystroke.modifiers.secondary() {
            return;
        }
        let user_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        if let Some(user_id) = user_id.filter(|user_id| !user_id.is_empty()) {
            cx.stop_propagation();
            ProfileAvatarPickerBlock::import_pasteboard(user_id, cx);
        }
    }
}

impl RenderOnce for ProfileSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let initial_nav_store = nav_store.clone();
        let state = window.use_keyed_state("profile-settings-state", cx, move |_, cx| {
            ProfileSettingsState::new(initial_nav_store, cx)
        });
        let badge_picker_open = state.read(cx).badge_picker_open;
        on_change("profile-settings-badge-fact", badge_picker_open, false, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::ProfileBadgePickerOpen, *open, cx);
        });

        let premium_badge_mutation_failed = {
            let user_service = UserServicesStore::shared(cx);
            user_service.read(cx).premium_badge_mutation_failed
        };
        let focus_handle = state.read(cx).focus_handle.clone();
        let header_block = Self::header_block(&state, cx);
        let field_items = Self::profile_field_items(&state, &theme, window, cx);
        let profile_details_group = Self::profile_details_group(&state, nav_store, &theme, window, cx);

        let commit_state = state.clone();
        let submit_state = state.clone();
        let exit_state = state.clone();
        let appear_state = state.clone();
        let disappear_state = state.clone();

        SettingsWrapperUi::new(SharedString::from(tr("settings_profile_title")), move |_, cx| {
            ProfileSettingsState::request_exit(&exit_state, cx);
        })
        .right_jsx(PageHeaderIconButton::new("profile-settings-commit", "checkmark", move |_, cx| {
            commit_state.update(cx, |state, cx| state.commit_and_pop(cx));
        }))
        .content(
            div()
                .id("profile-settings-paste-scope")
                .size_full()
                .track_focus(&focus_handle)
                .on_key_down(|event: &KeyDownEvent, _, cx| Self::handle_key_down(event, cx))
                .form_primary(true, move |_, cx| submit_state.update(cx, |state, cx| state.commit_and_pop(cx)))
                .child(
                    scroll_view("profile-settings-scroll")
                        .size_full()
                        .bg(theme.bg._100)
                        .list_shell_scroll_under_chrome(cx)
                        .child(
                            vstack(32.0)
                                .w_full()
                                .pt(px(6.0))
                                .child(header_block)
                                .child(GroupedBtns::new("profile-settings-fields", field_items).corner_radius(30.0))
                                .when(premium_badge_mutation_failed, |content| {
                                    content.child(
                                        div().w_full().pl(px(18.0)).pr(px(18.0)).child(
                                            MainText::new(tr("settings_premium_badge_save_failed"))
                                                .px(13.0)
                                                .tac(TextAlignT::Left)
                                                .color(theme.semantic.error_100),
                                        ),
                                    )
                                })
                                .child(profile_details_group)
                                .child(div().flex_none().h(px(80.0))),
                        ),
                ),
        )
        .on_appear("profile-settings-appear", move |window, cx| {
            UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.dismiss_premium_badge_failure(cx));
            let guard_state = appear_state.clone();
            NavigationInteractivePopGuard::set_unsaved_changes_guard(
                Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                    if !guard_state.read(cx).has_unsaved_changes() {
                        return false;
                    }
                    ProfileSettingsState::present_unsaved_alert(&guard_state, cx);
                    true
                })),
                cx,
            );
            appear_state.update(cx, |state, cx| state.hydrate_from_profile_if_needed(cx));
            let focus_handle = appear_state.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
        })
        .and_on_disappear(move |_, cx| {
            NavigationInteractivePopGuard::set_unsaved_changes_guard(None, cx);
            disappear_state.update(cx, |state, _| state.tag_check_task = None);
            GuideFacts::set(RiseGuideSignal::ProfileBadgePickerOpen, false, cx);
        })
    }
}

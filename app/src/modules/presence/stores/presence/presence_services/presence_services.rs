use std::collections::{HashMap, HashSet};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Context, Entity, WeakEntity};
use rise_canon::RiseEvent;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::date_format::relative_date_time_formatter::{RelativeDateTimeFormatter, UnitsStyle};
use rise_i18n::language_store::Locale;
use rise_i18n::{FormatArg, tr, tr_plural_with};
use serde_json::Value;

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::presence::stores::presence::presence_actions::presence_actions::PresenceActionsStore;
use crate::modules::presence::stores::presence::presence_actions::presence_types::PresenceState;

pub use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{
    SAI_WS_DID_CONNECT, SAI_WS_DID_DISCONNECT,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceIndicatorValue {
    Hidden,
    Online,
    InChat,
}

pub struct PresenceIndicatorProjection {
    value: PresenceIndicatorValue,
}

impl PresenceIndicatorProjection {
    pub fn new(value: PresenceIndicatorValue) -> Self {
        Self { value }
    }

    pub fn value(&self) -> PresenceIndicatorValue {
        self.value
    }

    fn apply(&mut self, value: PresenceIndicatorValue, cx: &mut Context<Self>) {
        if self.value == value {
            return;
        }
        self.value = value;
        cx.notify();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PresenceIndicatorProjectionRegistryKey {
    chat_id: String,
    fallback_online: bool,
}

struct PresenceIndicatorProjectionRegistryWeakProjection {
    value: WeakEntity<PresenceIndicatorProjection>,
}

impl PresenceIndicatorProjectionRegistryWeakProjection {
    fn new(value: &Entity<PresenceIndicatorProjection>) -> Self {
        Self { value: value.downgrade() }
    }
}

#[derive(Default)]
pub struct PresenceIndicatorProjectionRegistry {
    projections_by_user:
        HashMap<String, HashMap<PresenceIndicatorProjectionRegistryKey, PresenceIndicatorProjectionRegistryWeakProjection>>,
}

impl PresenceIndicatorProjectionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn projection(
        &mut self,
        user_id: &str,
        chat_id: &str,
        fallback_online: bool,
        source: Option<&PresenceState>,
        cx: &mut App,
    ) -> Entity<PresenceIndicatorProjection> {
        let key = PresenceIndicatorProjectionRegistryKey { chat_id: chat_id.to_owned(), fallback_online };
        let mut projections = self.projections_by_user.remove(user_id).unwrap_or_default();
        projections.retain(|_, reference| reference.value.upgrade().is_some());

        if let Some(existing) = projections.get(&key).and_then(|reference| reference.value.upgrade()) {
            self.projections_by_user.insert(user_id.to_owned(), projections);
            return existing;
        }

        let value = Self::value(source, chat_id, fallback_online);
        let projection = cx.new(|_| PresenceIndicatorProjection::new(value));
        projections.insert(key, PresenceIndicatorProjectionRegistryWeakProjection::new(&projection));
        self.projections_by_user.insert(user_id.to_owned(), projections);
        projection
    }

    pub fn update(&mut self, user_id: &str, source: Option<&PresenceState>, cx: &mut App) {
        let Some(mut projections) = self.projections_by_user.remove(user_id) else {
            return;
        };
        projections.retain(|key, reference| {
            let Some(projection) = reference.value.upgrade() else {
                return false;
            };
            let value = Self::value(source, &key.chat_id, key.fallback_online);
            projection.update(cx, |projection, cx| projection.apply(value, cx));
            true
        });

        if !projections.is_empty() {
            self.projections_by_user.insert(user_id.to_owned(), projections);
        }
    }

    pub fn reset(&mut self, cx: &mut App) {
        let user_ids: Vec<String> = self.projections_by_user.keys().cloned().collect();
        for user_id in user_ids {
            self.update(&user_id, None, cx);
        }
    }

    fn value(source: Option<&PresenceState>, chat_id: &str, fallback_online: bool) -> PresenceIndicatorValue {
        if let Some(in_chat_id) = source.and_then(|source| source.in_chat_id.as_deref())
            && !in_chat_id.is_empty()
            && in_chat_id == chat_id
        {
            return PresenceIndicatorValue::InChat;
        }
        if source.map_or(fallback_online, |source| source.is_online) {
            return PresenceIndicatorValue::Online;
        }
        PresenceIndicatorValue::Hidden
    }
}

pub struct PresenceServicesStore {
    stored_presence_by_user: HashMap<String, PresenceState>,
    indicator_projections: PresenceIndicatorProjectionRegistry,
    desired_by_owner: HashMap<String, HashSet<String>>,
    currently_subscribed: HashSet<String>,
    reconcile_scheduled: bool,
}

impl PresenceServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (did_connect, mut on_did_connect) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(SAI_WS_DID_CONNECT, move |_| {
                let _ = did_connect.unbounded_send(());
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while on_did_connect.next().await.is_some() {
                if this.update(cx, |store, cx| store.resubscribe_all(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            stored_presence_by_user: HashMap::new(),
            indicator_projections: PresenceIndicatorProjectionRegistry::new(),
            desired_by_owner: HashMap::new(),
            currently_subscribed: HashSet::new(),
            reconcile_scheduled: false,
        }
    }

    pub fn presence_by_user(&self) -> &HashMap<String, PresenceState> {
        &self.stored_presence_by_user
    }

    pub fn set_presence_by_user(&mut self, new_value: HashMap<String, PresenceState>, cx: &mut Context<Self>) {
        self.replace_presence_by_user(new_value, cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.stored_presence_by_user.clear();
        cx.notify();
        self.indicator_projections.reset(cx);
        self.desired_by_owner.clear();
        self.currently_subscribed.clear();
        self.reconcile_scheduled = false;
    }

    pub fn apply_online_status(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::UserOnlineStatusChanged(event)) =
            RiseEvent::decode_payload(rise_events::name::USER_ONLINE_STATUS_CHANGED, any)
        else {
            return;
        };
        if event.user_id.is_empty() {
            return;
        }

        let user_id = event.user_id;
        let is_online = event.is_online;
        let last_seen = event.last_seen;

        let in_chat_id = if is_online {
            self.stored_presence_by_user.get(&user_id).and_then(|state| state.in_chat_id.clone())
        } else {
            None
        };
        self.set_presence_state(
            PresenceState {
                is_online,
                last_seen,
                in_chat_id,
                updated_at: RiseClock::now().time_interval_since_1970(),
                status_known: true,
            },
            &user_id,
            cx,
        );
    }

    pub fn apply_in_chat_update(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::InChatUpdate(event)) = RiseEvent::decode_payload(rise_events::name::IN_CHAT_UPDATE, any)
        else {
            return;
        };
        if event.user_id.is_empty() {
            return;
        }

        let user_id = event.user_id;
        let in_chat = event.in_chat;
        let chat_id = event.chat_id;

        let mut state = self.stored_presence_by_user.get(&user_id).cloned().unwrap_or_default();
        state.in_chat_id = if in_chat && chat_id.as_deref().is_some_and(|chat_id| !chat_id.is_empty()) {
            chat_id
        } else {
            None
        };
        state.updated_at = RiseClock::now().time_interval_since_1970();
        self.set_presence_state(state, &user_id, cx);
    }

    pub fn indicator_projection(
        &mut self,
        user_id: &str,
        chat_id: &str,
        fallback_online: bool,
        cx: &mut Context<Self>,
    ) -> Entity<PresenceIndicatorProjection> {
        self.indicator_projections.projection(
            user_id,
            chat_id,
            fallback_online,
            self.stored_presence_by_user.get(user_id),
            cx,
        )
    }

    pub fn is_online(&self, user_id: &str, fallback: bool) -> bool {
        self.stored_presence_by_user.get(user_id).map_or(fallback, |state| state.is_online)
    }

    pub fn in_chat_id(&self, user_id: &str) -> Option<&str> {
        self.stored_presence_by_user.get(user_id).and_then(|state| state.in_chat_id.as_deref())
    }

    pub fn is_in_chat(&self, user_id: &str, chat_id: &str) -> bool {
        let Some(in_chat) = self.in_chat_id(user_id).filter(|in_chat| !in_chat.is_empty()) else {
            return false;
        };
        in_chat == chat_id
    }

    pub fn last_seen(&self, user_id: &str, fallback: Option<i64>) -> Option<i64> {
        if let Some(state) = self.stored_presence_by_user.get(user_id)
            && state.status_known
        {
            return state.last_seen;
        }
        fallback
    }

    pub fn status_text(
        &self,
        user_id: &str,
        fallback_online: bool,
        fallback_last_seen: Option<i64>,
        cx: &mut App,
    ) -> String {
        if self.is_online(user_id, fallback_online) {
            return tr("chat_status_online");
        }
        Self::last_seen_text(self.last_seen(user_id, fallback_last_seen), cx)
    }

    pub fn status_text_for_chat(&self, chat: &ChatInfo, cx: &mut App) -> String {
        if chat.is_bot_chat() {
            if let Some(users) = chat.participant.bot_users {
                return tr_plural_with("chat_status_bot_users", users, FormatArg::from(RiseFormat::compact_count(users)));
            }
            return tr(if chat.participant.more.is_official { "chat_status_official_bot" } else { "chat_status_bot" });
        }
        self.status_text(&chat.participant.id, chat.participant.is_online, chat.participant.last_seen, cx)
    }

    pub fn last_seen_text(raw: Option<i64>, cx: &mut App) -> String {
        let Some(raw) = raw.filter(|raw| *raw > 0) else {
            return tr("chat_status_recent");
        };

        let date = Date::from_time_interval_since_1970(raw as f64 / 1000.0);
        let now = MaintenanceGate::shared(cx).read(cx).server_now();
        let delta = now.time_interval_since(&date);
        let prefix = tr("chat_status_last_seen");

        if delta < 60.0 {
            return format!("{prefix} {}", tr("chat_status_just_now"));
        }

        if delta < 3_600.0 {
            let mut formatter = RelativeDateTimeFormatter::new();
            formatter.set_locale(Locale::app_locale().identifier());
            formatter.set_units_style(UnitsStyle::Full);
            let relative = formatter.localized_string_for(&date, &now);
            return format!("{prefix} {relative}");
        }
        format!("{prefix} {}", Self::seen_at_text(&date, &now))
    }

    /// like telegram, today at 14:30 or yesterday at 9:05 or 12 november at 18:00, the year only when its another
    fn seen_at_text(date: &Date, now: &Date) -> String {
        let calendar = Calendar::current();
        let format = if calendar.is_date_in_today(date) {
            "date_today_at_format"
        } else if calendar.is_date_in_yesterday(date) {
            "date_yesterday_at_format"
        } else if calendar.is_date_equal_to(date, now, CalendarComponent::Year) {
            "date_day_month_at_format"
        } else {
            "date_day_month_year_at_format"
        };
        let mut formatter = DateFormatter::new();
        formatter.set_locale(Locale::app_locale().identifier());
        formatter.set_date_format(tr(format));
        let text = formatter.string_from(date);
        let mut chars = text.chars();
        match chars.next() {
            Some(first) => first.to_lowercase().chain(chars).collect(),
            None => text,
        }
    }

    pub fn int_value(any: Option<&Value>) -> Option<i64> {
        match any? {
            Value::Number(number) => {
                if let Some(integer) = number.as_i64() {
                    return Some(integer);
                }
                let truncated = number.as_f64()?.trunc();
                if truncated.is_finite() && truncated >= i64::MIN as f64 && truncated < i64::MAX as f64 {
                    Some(truncated as i64)
                } else {
                    None
                }
            }
            Value::Bool(flag) => Some(i64::from(*flag)),
            Value::String(text) => int_from_string(text),
            _ => None,
        }
    }

    fn set_presence_state(&mut self, state: PresenceState, user_id: &str, cx: &mut Context<Self>) {
        self.stored_presence_by_user.insert(user_id.to_owned(), state);
        cx.notify();
        self.indicator_projections.update(user_id, self.stored_presence_by_user.get(user_id), cx);
    }

    fn replace_presence_by_user(&mut self, new_value: HashMap<String, PresenceState>, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.stored_presence_by_user, new_value);
        cx.notify();

        let affected_user_ids: HashSet<String> = previous
            .keys()
            .chain(self.stored_presence_by_user.keys())
            .filter(|user_id| previous.get(*user_id) != self.stored_presence_by_user.get(*user_id))
            .cloned()
            .collect();
        for user_id in affected_user_ids {
            self.indicator_projections.update(&user_id, self.stored_presence_by_user.get(&user_id), cx);
        }
    }

    pub fn set_desired(&mut self, user_ids: HashSet<String>, owner: &str, cx: &mut Context<Self>) {
        let my_id = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .unwrap_or_default();
        let cleaned: HashSet<String> =
            user_ids.into_iter().filter(|user_id| !user_id.is_empty() && *user_id != my_id).collect();

        if cleaned.is_empty() {
            self.desired_by_owner.remove(owner);
        } else {
            self.desired_by_owner.insert(owner.to_owned(), cleaned);
        }
        self.schedule_reconcile(cx);
    }

    pub fn resubscribe_all(&self, cx: &mut App) {
        if self.currently_subscribed.is_empty() {
            return;
        }
        let user_ids: Vec<String> = self.currently_subscribed.iter().cloned().collect();
        PresenceActionsStore::shared(cx).update(cx, |store, cx| store.subscribe_presence_action(user_ids, cx));
    }

    fn schedule_reconcile(&mut self, cx: &mut Context<Self>) {
        if self.reconcile_scheduled {
            return;
        }
        self.reconcile_scheduled = true;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |store, cx| {
                store.reconcile_scheduled = false;
                store.reconcile(cx);
            });
        })
        .detach();
    }

    fn reconcile(&mut self, cx: &mut App) {
        let union: HashSet<String> =
            self.desired_by_owner.values().flat_map(|user_ids| user_ids.iter().cloned()).collect();
        let to_add: Vec<String> = union.difference(&self.currently_subscribed).cloned().collect();
        let to_remove: Vec<String> = self.currently_subscribed.difference(&union).cloned().collect();
        self.currently_subscribed = union;

        if !to_add.is_empty() {
            PresenceActionsStore::shared(cx).update(cx, |store, cx| store.subscribe_presence_action(to_add, cx));
        }
        if !to_remove.is_empty() {
            PresenceActionsStore::shared(cx).update(cx, |store, cx| store.unsubscribe_presence_action(to_remove, cx));
        }
    }
}

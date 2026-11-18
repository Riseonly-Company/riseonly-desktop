use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_request::message::GetPollVoters;
use rise_canon::rise_view::message::{PollVoter, PollVoters};
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_compare::localized_case_insensitive_contains;
use serde_json::Value;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsRequestOptions, SaiWsRequestOptionsInit};
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::shared::poll_publicity::PollPublicity;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::MessagePollInfo;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PollVoterRow {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: Option<String>,
    pub is_official: bool,
    pub is_premium: bool,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
}

impl PollVoterRow {
    pub fn from_canon(voter: &PollVoter) -> Option<Self> {
        let user = voter.user.as_ref().filter(|user| !user.id.is_empty())?;
        Some(Self {
            id: user.id.clone(),
            name: user.name.clone(),
            tag: user.tag.clone(),
            logo: user.more.as_ref().map(|more| more.logo.clone()),
            is_official: user.is_official,
            is_premium: user.is_premium,
            premium_badge_emoji: user.premium_badge_emoji.clone(),
            name_color: user.name_color.clone(),
        })
    }

    pub fn matches(&self, query: &str) -> bool {
        localized_case_insensitive_contains(&self.name, query) || localized_case_insensitive_contains(&self.tag, query)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PollOptionVotersLoadState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollOptionVoters {
    pub voters: Vec<PollVoterRow>,
    pub total_count: i64,
    pub has_more: bool,
    pub load_state: PollOptionVotersLoadState,
}

impl PollOptionVoters {
    pub fn new(total_count: i64) -> Self {
        Self { voters: Vec::new(), total_count, has_more: false, load_state: PollOptionVotersLoadState::Idle }
    }

    pub fn hidden_count(&self) -> i64 {
        (self.total_count - self.voters.len() as i64).max(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollVotersPresentationOption {
    pub id: String,
    pub text: String,
    pub percent: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollVotersPresentation {
    pub chat_id: String,
    pub message_id: String,
    pub question: String,
    pub options: Vec<PollVotersPresentationOption>,
    pub voters: HashMap<String, PollOptionVoters>,
}

pub struct PollVotersStore {
    presentation: Option<PollVotersPresentation>,
    get_poll_voters_sai: Rc<SaiTypedSlice<PollVoters>>,
    generation: u64,
}

impl PollVotersStore {
    const FIRST_PAGE_SIZE: i64 = 10;
    const NEXT_PAGE_SIZE: i64 = 50;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self { presentation: None, get_poll_voters_sai: SaiTypedSlice::new(cx), generation: 0 }
    }

    pub fn presentation(&self) -> Option<&PollVotersPresentation> {
        self.presentation.as_ref()
    }

    pub fn can_show_voters(poll: &MessagePollInfo, chat: &ChatInfo) -> bool {
        !poll.is_anonymous && PollPublicity::names_voters(chat) && poll.total_voters > 0
    }

    pub fn present(&mut self, chat_id: &str, message_id: &str, poll: &MessagePollInfo, cx: &mut Context<Self>) {
        if chat_id.is_empty() || message_id.is_empty() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let total = poll.total_voters.max(1);
        let options = poll
            .options
            .iter()
            .map(|option| PollVotersPresentationOption {
                id: option.id.clone(),
                text: option.text.clone(),
                percent: (option.voter_count as f64 * 100.0 / total as f64).round() as i64,
            })
            .collect();
        let voters = poll
            .options
            .iter()
            .map(|option| (option.id.clone(), PollOptionVoters::new(option.voter_count)))
            .collect();
        self.presentation = Some(PollVotersPresentation {
            chat_id: chat_id.to_owned(),
            message_id: message_id.to_owned(),
            question: poll.question.clone(),
            options,
            voters,
        });
        cx.notify();
        for option in poll.options.iter().filter(|option| option.voter_count > 0) {
            self.load(&option.id, None, Self::FIRST_PAGE_SIZE, cx);
        }
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.presentation = None;
        cx.notify();
    }

    pub fn load_more(&mut self, option_id: &str, cx: &mut Context<Self>) {
        let Some(state) = self.presentation.as_ref().and_then(|presentation| presentation.voters.get(option_id))
        else {
            return;
        };
        if state.load_state == PollOptionVotersLoadState::Loading
            || !(state.has_more || state.load_state == PollOptionVotersLoadState::Failed)
        {
            return;
        }
        let after = state.voters.last().map(|voter| voter.id.clone());
        self.load(option_id, after, Self::NEXT_PAGE_SIZE, cx);
    }

    fn set_load_state(&mut self, option_id: &str, load_state: PollOptionVotersLoadState) {
        if let Some(state) =
            self.presentation.as_mut().and_then(|presentation| presentation.voters.get_mut(option_id))
        {
            state.load_state = load_state;
        }
    }

    fn load(&mut self, option_id: &str, after: Option<String>, limit: i64, cx: &mut Context<Self>) {
        let Some(current) = self.presentation.as_ref() else {
            return;
        };
        let token = self.generation;
        let mut body = GetPollVoters::new(current.chat_id.clone(), current.message_id.clone(), option_id.to_owned());
        body.limit = Some(limit);
        body.relative_id = after;
        self.set_load_state(option_id, PollOptionVotersLoadState::Loading);
        cx.notify();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::message::SERVICE.to_owned(),
            method: rise_wire_methods::message::GET_POLL_VOTERS.to_owned(),
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });
        let success_store = cx.entity().downgrade();
        let success_option_id = option_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let page = Self::decoded_page(payload);
            let _ = success_store.update(cx, |store, cx| {
                if store.generation != token {
                    return;
                }
                match page {
                    Some(page) => store.apply(&page, &success_option_id),
                    None => store.set_load_state(&success_option_id, PollOptionVotersLoadState::Failed),
                }
                cx.notify();
            });
        });
        let error_store = cx.entity().downgrade();
        let error_option_id = option_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let still_current = error_store
                .update(cx, |store, cx| {
                    if store.generation != token {
                        return false;
                    }
                    store.set_load_state(&error_option_id, PollOptionVotersLoadState::Failed);
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !still_current {
                return;
            }
            let line = format!("get_poll_voters({error_option_id}) failed: {error}");
            Sai::log("Message", move || line, SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(&self.get_poll_voters_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    fn decoded_page(payload: Option<Box<dyn Any>>) -> Option<PollVoters> {
        match payload?.downcast::<PollVoters>() {
            Ok(page) => Some(*page),
            Err(payload) => SaiJSONCodec::decode::<PollVoters>(payload.downcast_ref::<Value>()),
        }
    }

    fn apply(&mut self, page: &PollVoters, option_id: &str) {
        let Some(state) = self.presentation.as_mut().and_then(|presentation| presentation.voters.get_mut(option_id))
        else {
            return;
        };
        let mut seen: HashSet<String> = state.voters.iter().map(|voter| voter.id.clone()).collect();
        for row in page.voters.iter().filter_map(PollVoterRow::from_canon) {
            if seen.insert(row.id.clone()) {
                state.voters.push(row);
            }
        }
        state.total_count = page.total_count.max(state.voters.len() as i64);
        state.has_more = page.is_have_more;
        state.load_state = PollOptionVotersLoadState::Loaded;
    }
}

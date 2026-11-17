use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_request::gateway::{EnterChat, PresenceSubscribe, PresenceUnsubscribe, SetAway};
use rise_canon::rise_wire_methods::gateway;

use super::presence_wire_types::LeaveChatBody;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};

pub struct PresenceActionsStore {
    pub presence_subscribe_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub presence_unsubscribe_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub set_away_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub enter_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub leave_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
}

impl PresenceActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            presence_subscribe_sai: SaiTypedSlice::new(cx),
            presence_unsubscribe_sai: SaiTypedSlice::new(cx),
            set_away_sai: SaiTypedSlice::new(cx),
            enter_chat_sai: SaiTypedSlice::new(cx),
            leave_chat_sai: SaiTypedSlice::new(cx),
        }
    }

    pub fn subscribe_presence_action(&self, user_ids: Vec<String>, cx: &mut App) {
        if user_ids.is_empty() {
            return;
        }

        SaiWs::request_slice(
            &self.presence_subscribe_sai,
            PresenceSubscribe { users: Some(user_ids) },
            Self::presence_options(gateway::PRESENCE_SUBSCRIBE),
            Some(Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {})),
            Some(Rc::new(|_: &anyhow::Error, _: &mut App| {})),
            cx,
        );
    }

    pub fn unsubscribe_presence_action(&self, user_ids: Vec<String>, cx: &mut App) {
        if user_ids.is_empty() {
            return;
        }

        SaiWs::request_slice(
            &self.presence_unsubscribe_sai,
            PresenceUnsubscribe { users: Some(user_ids) },
            Self::presence_options(gateway::PRESENCE_UNSUBSCRIBE),
            Some(Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {})),
            Some(Rc::new(|_: &anyhow::Error, _: &mut App| {})),
            cx,
        );
    }

    pub fn set_away_action(&self, status: String, cx: &mut App) {
        SaiWs::request_slice(
            &self.set_away_sai,
            SetAway { status: Some(status) },
            Self::presence_options(gateway::SET_AWAY),
            Some(Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {})),
            Some(Rc::new(|_: &anyhow::Error, _: &mut App| {})),
            cx,
        );
    }

    pub fn enter_chat_action(&self, chat_id: &str, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        SaiWs::request_slice(
            &self.enter_chat_sai,
            EnterChat { chat_id: Some(chat_id.to_owned()) },
            Self::presence_options(gateway::ENTER_CHAT),
            Some(Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {})),
            Some(Rc::new(|_: &anyhow::Error, _: &mut App| {})),
            cx,
        );
    }

    pub fn leave_chat_action(&self, _chat_id: &str, cx: &mut App) {
        SaiWs::request_slice_encodable(
            &self.leave_chat_sai,
            LeaveChatBody {},
            Self::presence_options(gateway::LEAVE_CHAT),
            Some(Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {})),
            Some(Rc::new(|_: &anyhow::Error, _: &mut App| {})),
            cx,
        );
    }

    fn presence_options(method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: gateway::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }
}

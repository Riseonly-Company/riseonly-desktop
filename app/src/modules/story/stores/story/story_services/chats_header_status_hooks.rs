use rise_i18n::tr;

use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManagerPhase;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportLink, RiseTransportShown};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatsHeaderStatus {
    Title,
    Maintenance,
    WaitingForNetwork,
    Connecting,
    Updating,
}

impl ChatsHeaderStatus {
    /// maintenance outranks every socket state
    /// the product being closed means the socket is deliberately off
    /// so every other rule would say waiting for network and that is wrong
    pub fn resolve(
        socket_phase: &SaiWebSocketManagerPhase,
        is_updating_chats: bool,
        is_closed_for_maintenance: bool,
        is_retrying_after_failure: bool,
    ) -> ChatsHeaderStatus {
        if is_closed_for_maintenance {
            return ChatsHeaderStatus::Maintenance;
        }
        match socket_phase {
            SaiWebSocketManagerPhase::Connected(..) => {
                if is_updating_chats {
                    ChatsHeaderStatus::Updating
                } else {
                    ChatsHeaderStatus::Title
                }
            }
            SaiWebSocketManagerPhase::Connecting(..) => {
                if is_retrying_after_failure {
                    ChatsHeaderStatus::WaitingForNetwork
                } else {
                    ChatsHeaderStatus::Connecting
                }
            }
            SaiWebSocketManagerPhase::Idle
            | SaiWebSocketManagerPhase::Disconnected
            | SaiWebSocketManagerPhase::Failed(..) => ChatsHeaderStatus::WaitingForNetwork,
        }
    }

    pub fn resolve_shown(shown: RiseTransportShown, is_updating_chats: bool, is_closed_for_maintenance: bool) -> ChatsHeaderStatus {
        if is_closed_for_maintenance {
            return ChatsHeaderStatus::Maintenance;
        }
        match shown.link {
            RiseTransportLink::Connected if is_updating_chats => ChatsHeaderStatus::Updating,
            RiseTransportLink::Connected => ChatsHeaderStatus::Title,
            RiseTransportLink::Connecting => ChatsHeaderStatus::Connecting,
            RiseTransportLink::WaitingForNetwork => ChatsHeaderStatus::WaitingForNetwork,
        }
    }

    pub fn text(&self) -> String {
        match self {
            ChatsHeaderStatus::Title => tr("chats_title"),
            ChatsHeaderStatus::Maintenance => tr("chats_status_maintenance"),
            ChatsHeaderStatus::WaitingForNetwork => tr("chats_status_waiting_for_network"),
            ChatsHeaderStatus::Connecting => tr("chats_status_connecting"),
            ChatsHeaderStatus::Updating => tr("chats_status_updating"),
        }
    }

    pub fn shows_activity(&self) -> bool {
        match self {
            ChatsHeaderStatus::Title | ChatsHeaderStatus::Maintenance => false,
            ChatsHeaderStatus::WaitingForNetwork | ChatsHeaderStatus::Connecting | ChatsHeaderStatus::Updating => true,
        }
    }
}

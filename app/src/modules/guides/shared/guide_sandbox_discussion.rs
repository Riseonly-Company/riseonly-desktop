use gpui::{App, Context};
use rise_canon::{ChatPerm, RiseGuideSignal};
use rise_i18n::tr;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, LinkedChatInfo};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

/// reader's fake channel and the group it can take for comments
/// both are standalone details so neither shows in the chat list
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideSandboxDiscussionScene {
    pub linked_group_id: Option<String>,
}

impl GuideSandbox {
    pub fn invents(&self, chat_id: Option<&str>) -> bool {
        let Some(id) = chat_id.filter(|_| self.is_armed) else {
            return false;
        };
        Self::fixture_ids().contains(id)
    }

    pub fn owns_discussion(&self, channel_id: Option<&str>) -> bool {
        self.is_armed && self.discussion.is_some() && channel_id == Some(Self::channel_id().as_str())
    }

    pub fn discussion_channel(&self) -> ChatInfo {
        let mut chat = ChatInfo::new(Self::channel_id());
        chat.title = tr("guide_scene_channel_title");
        chat.r#type = ChatType::Channel;
        chat.member_count = 1;
        chat.role = ChatPerm::OWNER_ROLES.first().map(|role| role.to_lowercase());
        chat.created_at = Self::scene_stamp();
        chat.updated_at = Self::scene_stamp();
        chat
    }

    pub fn discussion_group(&self) -> ChatInfo {
        let mut chat = ChatInfo::new(Self::discussion_group_id());
        chat.title = tr("guide_scene_discussion_group");
        chat.r#type = ChatType::Group;
        chat.member_count = 2;
        chat.role = ChatPerm::OWNER_ROLES.first().map(|role| role.to_lowercase());
        chat.created_at = Self::scene_stamp();
        chat.updated_at = Self::scene_stamp();
        chat
    }

    pub fn discussion_candidates(&self, channel_id: &str) -> Option<Vec<ChatInfo>> {
        if !self.owns_discussion(Some(channel_id)) {
            return None;
        }
        Some(vec![self.discussion_group()])
    }

    pub fn linked_discussion(&self, chat_id: &str) -> Option<Option<LinkedChatInfo>> {
        let scene = self.discussion.as_ref().filter(|_| self.is_armed)?;
        let group = self.discussion_group();
        if chat_id == Self::channel_id() {
            if scene.linked_group_id.as_deref() != Some(group.id.as_str()) {
                return Some(None);
            }
            return Some(Some(LinkedChatInfo {
                id: group.id,
                title: group.title,
                logo_url: None,
                r#type: ChatType::Group.wire().to_uppercase(),
            }));
        }
        if chat_id == Self::discussion_group_id() {
            if scene.linked_group_id.is_none() {
                return Some(None);
            }
            let channel = self.discussion_channel();
            return Some(Some(LinkedChatInfo {
                id: channel.id,
                title: channel.title,
                logo_url: None,
                r#type: ChatType::Channel.wire().to_uppercase(),
            }));
        }
        None
    }

    pub fn link_discussion(&mut self, group_id: &str, cx: &mut Context<Self>) {
        if group_id != Self::discussion_group_id() {
            return;
        }
        let Some(scene) = self.discussion.as_mut() else {
            return;
        };
        scene.linked_group_id = Some(group_id.to_owned());
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::ChatDiscussionLinked);
    }

    pub fn unlink_discussion(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.discussion.as_mut() else {
            return;
        };
        scene.linked_group_id = None;
        cx.notify();
    }

    pub fn publish_discussion(&self, cx: &mut App) {
        if !self.is_armed || self.discussion.is_none() {
            return;
        }
        let channel = self.discussion_channel().to_wire_dto(None);
        let group = self.discussion_group().to_wire_dto(None);
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            let Some(account_id) = presentation.account_id().filter(|id| !id.is_empty()).map(str::to_owned) else {
                return;
            };
            presentation.publish_chat_detail(channel, &account_id, Some(false), cx);
            presentation.publish_chat_detail(group, &account_id, Some(false), cx);
        });
    }

    pub fn withdraw_discussion(&self, cx: &mut App) {
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.remove_chat_detail(&Self::channel_id(), cx);
            presentation.remove_chat_detail(&Self::discussion_group_id(), cx);
        });
    }
}

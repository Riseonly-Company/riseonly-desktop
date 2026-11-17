use std::collections::HashSet;

use gpui::Context;

use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// lesson about muting every chat but the important ones
/// real switch and list, answers stay on device and are restored at the end
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxMuteScene {
    pub mute_all_before: bool,
    pub exceptions_before: HashSet<String>,
}

impl GuideSandbox {
    pub fn owns_mute_scene(&self) -> bool {
        self.is_armed && self.mute_scene.is_some()
    }

    pub fn arm_mute(&mut self, cx: &mut Context<Self>) {
        let store = UserServicesStore::shared(cx);
        let (mute_all_before, exceptions_before) = {
            let services = store.read(cx);
            (
                services.mute_all_chats_enabled(),
                services.chat_mute_override_ids().clone(),
            )
        };
        self.mute_scene = Some(GuideSandboxMuteScene {
            mute_all_before,
            exceptions_before,
        });
        store.update(cx, |services, cx| {
            services.set_chat_mute_override_ids(HashSet::new(), cx);
        });
        cx.notify();
    }

    pub fn withdraw_mute(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.mute_scene.take() else {
            return;
        };
        cx.notify();
        let store = UserServicesStore::shared(cx);
        store.update(cx, |services, cx| {
            services.set_mute_all_chats_enabled(scene.mute_all_before, cx);
            services.set_chat_mute_override_ids(scene.exceptions_before, cx);
        });
    }

    /// answers the switch on the device, false means no lesson so send the request
    pub fn set_practice_mute_all(&mut self, mute_all: bool, cx: &mut Context<Self>) -> bool {
        if !self.owns_mute_scene() {
            return false;
        }
        let store = UserServicesStore::shared(cx);
        store.update(cx, |services, cx| {
            services.set_mute_all_chats_enabled(mute_all, cx);
        });
        true
    }

    /// answers an exception on the device for any chat the reader holds in the lesson
    pub fn set_practice_mute_exception(
        &mut self,
        chat_id: &str,
        add: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.owns_mute_scene() {
            return false;
        }
        let store = UserServicesStore::shared(cx);
        store.update(cx, |services, cx| {
            services.apply_chat_mute_override_optimistic(chat_id, add, cx);
        });
        true
    }

    /// takes a server answer as the state to put back, false means no lesson so apply it
    pub fn absorb_mute_overrides(
        &mut self,
        mute_all: Option<bool>,
        chat_ids: Option<&[String]>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.owns_mute_scene() {
            return false;
        }
        if let Some(scene) = self.mute_scene.as_mut() {
            if let Some(mute_all) = mute_all {
                scene.mute_all_before = mute_all;
            }
            if let Some(chat_ids) = chat_ids {
                scene.exceptions_before = chat_ids.iter().cloned().collect();
            }
        }
        cx.notify();
        true
    }

    pub fn listed_chats(&self, chats: Vec<ChatInfo>) -> Vec<ChatInfo> {
        if !self.owns_mute_scene() || chats.iter().any(|chat| chat.id == Self::chat_id()) {
            return chats;
        }
        let mut listed = Vec::with_capacity(chats.len() + 1);
        listed.push(self.chat());
        listed.extend(chats);
        listed
    }
}

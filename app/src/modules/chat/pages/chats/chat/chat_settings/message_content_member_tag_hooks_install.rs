use gpui::{AnyElement, IntoElement, SharedString};

use super::chat_settings_shared::MemberTagChip;
use crate::modules::chat::components::chat::bubble::message_content_member_tag_hooks::{
    MessageContentMemberTagHooks, set_message_content_member_tag_hooks,
};

fn member_tag_chip(title: SharedString, color_hex: SharedString) -> AnyElement {
    MemberTagChip::new(title).color_hex(color_hex).into_any_element()
}

pub fn install_message_content_member_tag_hooks() {
    set_message_content_member_tag_hooks(MessageContentMemberTagHooks { member_tag_chip });
}

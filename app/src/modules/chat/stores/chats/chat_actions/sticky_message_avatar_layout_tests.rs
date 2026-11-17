use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListBottomStickyHeaderLayout;
use crate::modules::chat::components::chat::video_note::video_note_message_view::VideoNoteMessageView;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};

fn chat(id: &str, chat_type: ChatType) -> ChatInfo {
    let mut chat = ChatInfo::new(id.to_owned());
    chat.r#type = chat_type;
    chat
}

#[test]
fn avatar_uses_natural_bottom_when_group_ends_above_composer() {
    let y = RiseManualListBottomStickyHeaderLayout::origin_y(120.0, 420.0, 700.0, 34.0);

    assert_eq!(y, 386.0);
}

#[test]
fn avatar_sticks_above_composer_while_group_continues_below_it() {
    let y = RiseManualListBottomStickyHeaderLayout::origin_y(120.0, 920.0, 700.0, 34.0);

    assert_eq!(y, 666.0);
}

#[test]
fn avatar_cannot_move_above_its_sender_group() {
    let y = RiseManualListBottomStickyHeaderLayout::origin_y(680.0, 920.0, 700.0, 34.0);

    assert_eq!(y, 680.0);
}

#[test]
fn adjacent_groups_cannot_overlap_their_avatars() {
    let first_y = RiseManualListBottomStickyHeaderLayout::origin_y(100.0, 500.0, 700.0, 34.0);
    let second_y = RiseManualListBottomStickyHeaderLayout::origin_y(504.0, 900.0, 700.0, 34.0);

    assert!(first_y + 34.0 <= second_y);
}

#[test]
fn private_and_bot_chats_do_not_reserve_avatar_space() {
    let direct = chat("direct", ChatType::Direct);
    let mut bot = chat("bot", ChatType::Direct);
    bot.participant.is_bot = true;

    assert!(!MessageSizes::reserves_avatar_space(&direct, false));
    assert!(!MessageSizes::reserves_avatar_space(&bot, false));
    assert!(MessageSizes::reserves_avatar_space(&chat("group", ChatType::Group), false));
    assert!(!MessageSizes::reserves_avatar_space(&chat("group", ChatType::Group), true));
}

#[test]
fn compact_video_notes_use_telegram_size_regardless_of_avatar_reservation() {
    let direct_size = VideoNoteMessageView::expanded_size(320.0, false);
    let group_incoming_size = VideoNoteMessageView::expanded_size(320.0, true);

    assert_eq!(direct_size, VideoNoteMessageView::BUBBLE_SIZE);
    assert_eq!(group_incoming_size, VideoNoteMessageView::BUBBLE_SIZE);
}

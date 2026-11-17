use image::{DynamicImage, RgbImage, Rgba, RgbaImage};
use rise_canon::RisePushType;
use rise_canon::rise_key::{push, push_route};
use rise_platform::emoji_raster::EmojiRaster;
use rise_platform::notifications::NotificationTag;
use rise_theme::avatar_palette::AvatarPalette;
use serde_json::{Map, Value, json};

use super::{MessageNotification, NotificationRequest, NotificationService};

fn request(title: &str, entries: &[(&str, Value)]) -> NotificationRequest {
    let mut user_info = Map::new();
    for (key, value) in entries {
        user_info.insert((*key).to_owned(), value.clone());
    }
    NotificationRequest {
        tag: NotificationTag::conversation("chat"),
        title: title.to_owned(),
        body: "body".to_owned(),
        user_info,
    }
}

#[test]
fn private_chat_is_the_person() {
    let request = request(
        "Title",
        &[
            (push_route::CHAT_ID, json!("c1")),
            (push_route::SENDER_ID, json!("u1")),
            (push::SENDER_NAME, json!("Ann")),
            (push::CHAT_TYPE, json!("private")),
        ],
    );
    let message = MessageNotification::read(&request, None);
    assert_eq!(message.chat_id, "c1");
    assert_eq!(message.sender_id, "u1");
    assert_eq!(message.sender_name, "Ann");
}

#[test]
fn group_and_channel_are_the_chat_and_fall_back_to_the_title() {
    let titled = request(
        "Title",
        &[
            (push_route::CHAT_ID, json!("c1")),
            (push_route::SENDER_ID, json!("u1")),
            (push::SENDER_NAME, json!("Ann")),
            (push::CHAT_TYPE, json!("group")),
            (push::CHAT_TITLE, json!("  Team ")),
        ],
    );
    let message = MessageNotification::read(&titled, None);
    assert_eq!(message.sender_id, "c1");
    assert_eq!(message.sender_name, "Team");

    let untitled = request("Title", &[(push_route::CHAT_ID, json!("c1")), (push::CHAT_TYPE, json!("CHANNEL"))]);
    assert_eq!(MessageNotification::read(&untitled, None).sender_name, "Title");
}

#[test]
fn a_reaction_takes_the_name_from_the_alert_title() {
    let reaction = request("  Bob  ", &[(push::SENDER_NAME, json!("Ann"))]);
    assert_eq!(MessageNotification::read(&reaction, Some(&reaction.title)).sender_name, "Bob");
    let blank = request("  ", &[(push::SENDER_NAME, json!("Ann"))]);
    assert_eq!(MessageNotification::read(&blank, Some(&blank.title)).sender_name, "Ann");
}

#[test]
fn numbers_in_the_payload_read_as_strings_only_where_ios_does() {
    let numeric = request("Title", &[(push_route::CHAT_ID, json!(5)), (push::CHAT_TYPE, json!("GROUP")), (push::CHAT_TITLE, json!(7))]);
    let message = MessageNotification::read(&numeric, None);
    assert_eq!(message.chat_id, "", "chat_id is `as? String`");
    assert_eq!(message.sender_name, "7", "chat_title goes through stringValue");
}

fn message(sender_id: &str, avatar_url: Option<&str>) -> MessageNotification {
    MessageNotification {
        chat_id: String::new(),
        sender_id: sender_id.to_owned(),
        sender_name: String::new(),
        avatar_url: avatar_url.map(str::to_owned),
        author: None,
    }
}

#[test]
fn the_letter_belongs_to_the_owner_of_a_server_letter_or_to_the_sender_without_a_photo() {
    assert_eq!(NotificationService::letter_owner(None, "u1").as_deref(), Some("u1"));
    assert_eq!(NotificationService::letter_owner(Some(""), "u1").as_deref(), Some("u1"));
    assert_eq!(NotificationService::letter_owner(None, ""), None);
    assert_eq!(NotificationService::letter_owner(Some("https://cdn/a/photo.jpg"), "u1"), None);
    assert_eq!(
        NotificationService::letter_owner(Some("https://cdn/avatars/generated/owner.3_5_A.png"), "u1").as_deref(),
        Some("owner")
    );
}

#[test]
fn target_title_prefers_the_title_then_the_name_then_the_app() {
    let mut named = message("u1", None);
    named.sender_name = "Ann".to_owned();
    assert_eq!(named.target_title("Title"), "Title");
    assert_eq!(named.target_title("  "), "Ann");
    assert_eq!(message("u1", None).target_title(""), "Riseonly");
}

#[test]
fn every_chat_message_uses_the_canon_category() {
    assert!(!NotificationService::message_category_identifier().is_empty());
    assert!(!RisePushType::NEW_MESSAGE.is_empty());
}

fn raster(width: u32, height: u32, pixel: [u8; 4]) -> EmojiRaster {
    EmojiRaster { width, height, rgba: pixel.repeat((width * height) as usize) }
}

#[test]
fn a_neutral_letter_takes_the_ink_and_stays_centred() {
    let mut image = RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]));
    NotificationService::draw_centered(&mut image, &raster(2, 2, [0, 0, 0, 255]), [200, 100, 50]);
    assert_eq!(image.get_pixel(1, 1).0, [200, 100, 50]);
    assert_eq!(image.get_pixel(2, 2).0, [200, 100, 50]);
    assert_eq!(image.get_pixel(0, 0).0, [10, 20, 30]);
}

#[test]
fn a_colour_emoji_keeps_its_colours_and_an_oversized_raster_is_clipped() {
    let mut image = RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30]));
    NotificationService::draw_centered(&mut image, &raster(4, 4, [9, 8, 7, 255]), [200, 100, 50]);
    assert_eq!(image.get_pixel(0, 0).0, [9, 8, 7]);
    assert_eq!(image.get_pixel(1, 1).0, [9, 8, 7]);
}

#[test]
fn the_letter_background_is_the_canon_colour_of_the_owner() {
    let image = NotificationService::letter_avatar_image("owner", "Ann");
    let (background, _, _) = AvatarPalette::letter_avatar("owner", "Ann");
    assert_eq!((image.width(), image.height()), (256, 256));
    assert_eq!(image.get_pixel(0, 0).0, [background.red, background.green, background.blue]);
}

#[test]
fn only_a_larger_side_shrinks_to_the_notification_size_keeping_the_aspect() {
    let large = DynamicImage::ImageRgba8(RgbaImage::new(1024, 512));
    let resized = NotificationService::resize_for_notification_avatar(&large);
    assert_eq!((resized.width(), resized.height()), (512, 256));
    let small = DynamicImage::ImageRgba8(RgbaImage::new(100, 40));
    let kept = NotificationService::resize_for_notification_avatar(&small);
    assert_eq!((kept.width(), kept.height()), (100, 40));
}

#[test]
fn the_composite_has_the_white_badge_in_the_lower_corner() {
    let avatar = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 255])));
    let icon = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([200, 0, 0, 255])));
    let composite = NotificationService::composite(&avatar, &icon);
    assert_eq!((composite.width(), composite.height()), (1024, 1024));
    assert_eq!(composite.get_pixel(0, 0).0, [10, 20, 30]);
    let badge_center = composite.get_pixel(829, 829).0;
    assert!(badge_center[0] > 150 && badge_center[1] < 50, "the icon sits in the badge: {badge_center:?}");
    let badge_rim = composite.get_pixel(829, 700).0;
    assert_eq!(badge_rim, [255, 255, 255], "the white circle shows around the icon");
}

#[test]
fn the_notification_title_is_the_target_title_on_every_system() {
    let message = MessageNotification {
        chat_id: "chat-1".to_owned(),
        sender_id: "sender-1".to_owned(),
        sender_name: "Ann".to_owned(),
        avatar_url: None,
        author: None,
    };

    let blank = NotificationService::build_communication_notification(request("  ", &[]), &message, None, None);
    assert_eq!(blank.notification.title, "Ann");

    let titled = NotificationService::build_communication_notification(request("Group", &[]), &message, None, None);
    assert_eq!(titled.notification.title, "Group");
}

#[test]
fn a_notification_without_a_chat_is_the_conversation_of_its_sender() {
    let message = MessageNotification {
        chat_id: String::new(),
        sender_id: "sender-1".to_owned(),
        sender_name: "Ann".to_owned(),
        avatar_url: None,
        author: None,
    };
    let content = NotificationService::build_communication_notification(request("Ann", &[]), &message, None, None);
    assert_eq!(content.notification.thread.as_deref(), Some("sender-1"));

    let in_chat = MessageNotification { chat_id: "chat-1".to_owned(), ..message };
    let content = NotificationService::build_communication_notification(request("Ann", &[]), &in_chat, None, None);
    assert_eq!(content.notification.thread.as_deref(), Some("chat-1"));
}

#[test]
fn a_group_message_is_written_by_its_member_and_a_channel_or_reaction_by_the_chat() {
    let group = request(
        "Team",
        &[
            (push_route::CHAT_ID, json!("c1")),
            (push_route::SENDER_ID, json!("u1")),
            (push::SENDER_NAME, json!(" Ann ")),
            (push::SENDER_AVATAR, json!("https://cdn/a/ann.jpg")),
            (push::CHAT_TYPE, json!("group")),
            (push::CHAT_TITLE, json!("Team")),
        ],
    );
    let message = MessageNotification::read(&group, None);
    let author = message.author.clone().expect("a group message has an author");
    assert_eq!((author.id.as_str(), author.name.as_str()), ("u1", "Ann"));
    assert_eq!(author.avatar_url.as_deref(), Some("https://cdn/a/ann.jpg"));
    assert!(message.needs_any_download());

    let content = NotificationService::build_communication_notification(group.clone(), &message, None, None);
    assert_eq!(content.notification.sender.as_deref(), Some("Ann"));
    assert_eq!(content.notification.title, "Team");
    assert_eq!(content.notification.thread.as_deref(), Some("c1"));

    assert_eq!(MessageNotification::read(&group, Some("Team")).author, None);
    let mut channel = group.clone();
    channel.user_info.insert(push::CHAT_TYPE.to_owned(), json!("channel"));
    assert_eq!(MessageNotification::read(&channel, None).author, None);
}

#[test]
fn the_body_drops_formatting_like_the_chat_list_row() {
    let mut plain = request("Ann", &[]);
    plain.body = "hello".to_owned();
    let content = NotificationService::build_communication_notification(plain, &message("u1", None), None, None);
    assert_eq!(content.notification.body, "hello");
}

#[test]
fn only_a_photo_url_waits_for_a_download() {
    assert!(!NotificationService::needs_download(None));
    assert!(!NotificationService::needs_download(Some("")));
    assert!(!NotificationService::needs_download(Some("https://cdn/avatars/generated/owner.3_5_A.png")));
    assert!(NotificationService::needs_download(Some("https://cdn/a/photo.jpg")));
}

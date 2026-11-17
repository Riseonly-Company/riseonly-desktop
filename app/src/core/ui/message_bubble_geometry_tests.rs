use std::sync::Arc;

use gpui::{App, Bounds, Pixels, Point, TestAppContext, point, px, size};
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_i18n::language_store::Locale;
use rise_ui::message_bubble_geometry::{
    MessageBubbleDescriptor, MessageBubbleGeometry, MessageBubblePath, MessageBubblePathElement, MessageBubbleRadii,
    MessageBubbleTailSide,
};
use rise_ui::message_media_tail_presentation::{
    MessageMediaTailBitmap, MessageMediaTailColor, MessageMediaTailColorSampler, MessageMediaTailGeometry,
};
use rise_ui::swiftui::environment::LayoutDirection;

use crate::modules::chat::components::chat::async_render::async_text_bubble_types::AsyncTextBubbleDeliveryState;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntry, ChatMessageRenderEntryGroupFlags, ChatMessageRenderEntryKind,
};
use crate::modules::chat::components::chat::async_render::rise_async_text_message_row::RiseAsyncTextMessageEligibility;
use crate::modules::chat::components::chat::async_render::rise_message_fallback_height_cache::{
    RiseMessageFallbackGeometry, RiseMessageFallbackHeightCache, RiseMessageFallbackHeightCacheKey,
    RiseMessageGeometryVersion,
};
use crate::modules::chat::components::chat::bubble::message_content::MessageForwardSourceTarget;
use crate::modules::chat::components::chat::bubble::message_meta::MessageDeliveryStatusGeometry;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy, MessageStickerLayout,
};
use crate::modules::chat::components::chat::video_note::video_note_message_view::VideoNoteMessageView;
use crate::modules::chat::shared::config::message_sizes::{MessageAccessoryStackGeometry, MessageSizes};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageForwardInfo, MessageMediaItem, MessageMediaKind, MessageStickerKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    MessageForwardInfoWireDTO, MessageWireDTO,
};

const FLOAT_TOLERANCE: f32 = 0.001;
const CURVE_STEPS: usize = 64;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn plain(point: Point<Pixels>) -> (f32, f32) {
    (f32::from(point.x), f32::from(point.y))
}

fn quad_at(from: (f32, f32), control: (f32, f32), to: (f32, f32), t: f32) -> (f32, f32) {
    let rest = 1.0 - t;
    (
        rest * rest * from.0 + 2.0 * rest * t * control.0 + t * t * to.0,
        rest * rest * from.1 + 2.0 * rest * t * control.1 + t * t * to.1,
    )
}

fn cubic_at(from: (f32, f32), control1: (f32, f32), control2: (f32, f32), to: (f32, f32), t: f32) -> (f32, f32) {
    let rest = 1.0 - t;
    let (a, b, c, d) = (rest * rest * rest, 3.0 * rest * rest * t, 3.0 * rest * t * t, t * t * t);
    (
        a * from.0 + b * control1.0 + c * control2.0 + d * to.0,
        a * from.1 + b * control1.1 + c * control2.1 + d * to.1,
    )
}

/// contour as polylines
/// the kit path keeps only the elements gpui replays while tests ask for bounding box and contains
fn flattened(path: &MessageBubblePath) -> Vec<Vec<(f32, f32)>> {
    let mut contours = Vec::new();
    let mut contour: Vec<(f32, f32)> = Vec::new();
    for element in path.elements() {
        match *element {
            MessageBubblePathElement::MoveTo(to) => {
                if !contour.is_empty() {
                    contours.push(std::mem::take(&mut contour));
                }
                contour.push(plain(to));
            }
            MessageBubblePathElement::LineTo(to) => contour.push(plain(to)),
            MessageBubblePathElement::QuadCurveTo { to, control } => {
                let from = contour.last().copied().unwrap_or_else(|| plain(to));
                for step in 1..=CURVE_STEPS {
                    let t = step as f32 / CURVE_STEPS as f32;
                    contour.push(quad_at(from, plain(control), plain(to), t));
                }
            }
            MessageBubblePathElement::CurveTo { to, control1, control2 } => {
                let from = contour.last().copied().unwrap_or_else(|| plain(to));
                for step in 1..=CURVE_STEPS {
                    let t = step as f32 / CURVE_STEPS as f32;
                    contour.push(cubic_at(from, plain(control1), plain(control2), plain(to), t));
                }
            }
            MessageBubblePathElement::CloseSubpath => {
                if !contour.is_empty() {
                    contours.push(std::mem::take(&mut contour));
                }
            }
        }
    }
    if !contour.is_empty() {
        contours.push(contour);
    }
    contours
}

#[derive(Clone, Copy, Debug)]
struct PathBox {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl PathBox {
    fn height(self) -> f32 {
        self.max_y - self.min_y
    }
}

/// box of the contour itself, without control points
fn bounding_box_of_path(path: &MessageBubblePath) -> PathBox {
    let mut result =
        PathBox { min_x: f32::INFINITY, max_x: f32::NEG_INFINITY, min_y: f32::INFINITY, max_y: f32::NEG_INFINITY };
    for (x, y) in flattened(path).into_iter().flatten() {
        result.min_x = result.min_x.min(x);
        result.max_x = result.max_x.max(x);
        result.min_y = result.min_y.min(y);
        result.max_y = result.max_y.max(y);
    }
    result
}

/// point in path test with the default non zero winding rule
fn path_contains(path: &MessageBubblePath, at: (f32, f32)) -> bool {
    let mut winding = 0_i32;
    for contour in flattened(path) {
        for index in 0..contour.len() {
            let from = contour[index];
            let to = contour[(index + 1) % contour.len()];
            let side = (to.0 - from.0) * (at.1 - from.1) - (at.0 - from.0) * (to.1 - from.1);
            if from.1 <= at.1 {
                if to.1 > at.1 && side > 0.0 {
                    winding += 1;
                }
            } else if to.1 <= at.1 && side < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

/// bgra bytes of a 2x2 picture with two uniform columns, each argument is red green blue alpha
fn two_column_image(leading: (u8, u8, u8, u8), trailing: (u8, u8, u8, u8)) -> Vec<u8> {
    let row = [
        leading.2, leading.1, leading.0, leading.3, trailing.2, trailing.1, trailing.0, trailing.3,
    ];
    let mut bytes = Vec::with_capacity(row.len() * 2);
    bytes.extend_from_slice(&row);
    bytes.extend_from_slice(&row);
    bytes
}

fn message_entry(message: GetMessageMessage) -> ChatMessageRenderEntry {
    ChatMessageRenderEntry::new(
        message.id.clone(),
        ChatMessageRenderEntryKind::Message,
        String::new(),
        None,
        None,
        Some(Arc::new(message)),
        Some(ChatMessageRenderEntryGroupFlags { is_first: true, is_last: true }),
        None,
        None,
        None,
        None,
        1,
    )
}

fn direct_chat(id: &str) -> ChatInfo {
    let mut chat = ChatInfo::new(id.to_owned());
    chat.r#type = ChatType::Direct;
    chat
}

fn gmt() -> TimeZone {
    TimeZone::with_seconds_from_gmt(0).expect("zero offset is a valid time zone")
}

fn fallback_version(message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> RiseMessageGeometryVersion {
    RiseMessageFallbackGeometry::version(
        &message_entry(message.clone()),
        chat,
        &Calendar::gregorian(),
        &gmt(),
        None,
        cx,
    )
}

#[test]
fn message_item_insets_match_telegram_merged_geometry() {
    let unmerged_spacing = 2.0 + MessageSizes::device_pixel();

    assert_eq!(MessageSizes::list_row_spacing(), 0.0);
    assert_eq!(MessageSizes::list_first_in_group_top_padding(), unmerged_spacing);
    assert_eq!(MessageSizes::list_continuation_top_padding(), 2.0);
    assert_eq!(MessageSizes::list_last_in_group_bottom_padding(), unmerged_spacing);

    let merged_interior_gap = MessageSizes::list_continuation_top_padding();
    assert_eq!(merged_interior_gap, 2.0);
    let unmerged_gap = MessageSizes::list_last_in_group_bottom_padding() + MessageSizes::list_first_in_group_top_padding();
    assert!((unmerged_gap - (4.0 + MessageSizes::device_pixel() * 2.0)).abs() < FLOAT_TOLERANCE);
}

#[test]
fn accessory_stack_accounts_for_embedded_reactions() {
    assert_eq!(MessageAccessoryStackGeometry::additional_height(Some(42.0), None), 43.0);
    assert_eq!(MessageAccessoryStackGeometry::additional_height(None, Some(30.0)), 34.0);
    assert_eq!(MessageAccessoryStackGeometry::additional_height(Some(42.0), Some(30.0)), 77.0);
    assert_eq!(MessageAccessoryStackGeometry::embedded_reaction_top_spacing(), MessageSizes::bubble_column_spacing());
}

#[test]
fn animated_media_uses_mime_type_for_signed_urls_and_routes_web_m_to_its_player() {
    assert_eq!(
        ChatAnimatedMediaSourcePolicy::playback_kind("https://cdn.example/media/signed?id=42", Some("video/mp4"), None),
        ChatAnimatedMediaPlaybackKind::AvFoundationVideo
    );
    assert_eq!(
        ChatAnimatedMediaSourcePolicy::playback_kind(
            "https://cdn.example/media/signed?id=43",
            Some("image/gif; charset=binary"),
            None
        ),
        ChatAnimatedMediaPlaybackKind::AnimatedImage
    );
    assert_eq!(
        ChatAnimatedMediaSourcePolicy::playback_kind("https://cdn.example/sticker.webm", Some("video/webm"), None),
        ChatAnimatedMediaPlaybackKind::WebMVideo
    );
}

#[test]
fn shared_post_comments_keep_post_and_channel_discussion_contracts_separate() {
    let payload = r#"
    {
      "id": "channel-message-1",
      "chat_id": "channel-1",
      "content_type": "post_short",
      "shared_post": {
        "post_id": 73,
        "comments_count": 12
      },
      "discussion_chat_id": "discussion-group-1",
      "discussion_message_id": "discussion-root-1",
      "comments_count": 4
    }
    "#;
    let message = serde_json::from_str::<MessageWireDTO>(payload).expect("message payload decodes").to_message();

    let shared_post = message.shared_post.as_ref().expect("the payload carries a shared post");
    assert_eq!(shared_post.post_id, "73");
    assert_eq!(shared_post.comments_count, 12);
    assert_eq!(message.discussion_chat_id.as_deref(), Some("discussion-group-1"));
    assert_eq!(message.discussion_message_id.as_deref(), Some("discussion-root-1"));
    assert_eq!(message.comments_count, Some(4));
}

#[test]
fn forwarded_source_keeps_tag_and_nested_sender_fallbacks() {
    let payload = r#"
    {
      "sender_id": "   ",
      "sender_tag": "",
      "sender_name": "",
      "sender_logo": " ",
      "sender": {
        "id": "user-7",
        "name": "Alice",
        "tag": "alice",
        "more": { "logo": "https://cdn.example/alice.jpg" }
      },
      "date": 1700000000000
    }
    "#;
    let wire = serde_json::from_str::<MessageForwardInfoWireDTO>(payload).expect("forward payload decodes");
    let model = wire.to_forward().expect("the nested sender resolves a forward");

    assert_eq!(model.sender_id.as_deref(), Some("user-7"));
    assert_eq!(model.sender_tag.as_deref(), Some("alice"));
    assert_eq!(model.sender_name, "Alice");
    assert_eq!(model.sender_logo.as_deref(), Some("https://cdn.example/alice.jpg"));
    assert_eq!(model.date, 1_700_000_000.0);
}

#[test]
fn forwarded_source_distinguishes_channel_user_and_private_attribution() {
    let channel = MessageForwardInfo {
        from_chat_id: Some("channel-1".to_owned()),
        sender_id: Some("channel-1".to_owned()),
        sender_name: "Channel".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        MessageForwardSourceTarget::resolve(&channel),
        Some(MessageForwardSourceTarget::ChatProfile { chat_id: "channel-1".to_owned() })
    );

    let public_user = MessageForwardInfo {
        from_chat_id: Some("private-chat-1".to_owned()),
        sender_id: Some("user-7".to_owned()),
        sender_tag: Some("@alice".to_owned()),
        sender_name: "Alice".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        MessageForwardSourceTarget::resolve(&public_user),
        Some(MessageForwardSourceTarget::User {
            user_id: Some("user-7".to_owned()),
            tag: Some("alice".to_owned()),
        })
    );

    let private_attribution = MessageForwardInfo {
        from_chat_id: Some("private-chat-1".to_owned()),
        sender_name: "Hidden Sender".to_owned(),
        ..Default::default()
    };
    assert_eq!(MessageForwardSourceTarget::resolve(&private_attribution), None);
}

#[test]
fn delivery_status_uses_one_telegram_metric_across_renderers() {
    assert_eq!(
        MessageDeliveryStatusGeometry::size(AsyncTextBubbleDeliveryState::Sending, px(11.0)),
        size(px(11.0), px(11.0))
    );
    assert_eq!(
        MessageDeliveryStatusGeometry::size(AsyncTextBubbleDeliveryState::Sent, px(11.0)),
        size(px(11.0), px(9.0))
    );
    assert_eq!(
        MessageDeliveryStatusGeometry::size(AsyncTextBubbleDeliveryState::Read, px(11.0)),
        size(px(17.0), px(9.0))
    );
}

#[test]
fn delivery_status_slot_never_changes_with_state() {
    let live = [
        AsyncTextBubbleDeliveryState::Sending,
        AsyncTextBubbleDeliveryState::Failed,
        AsyncTextBubbleDeliveryState::Sent,
        AsyncTextBubbleDeliveryState::Delivered,
        AsyncTextBubbleDeliveryState::Read,
    ];
    for state in live {
        assert_eq!(MessageDeliveryStatusGeometry::slot_size(state, px(11.0)), size(px(17.0), px(11.0)));
    }
    assert_eq!(
        MessageDeliveryStatusGeometry::slot_size(AsyncTextBubbleDeliveryState::None, px(11.0)),
        size(px(0.0), px(0.0))
    );
}

#[test]
fn non_square_stickers_aspect_fit_telegram_bounds() {
    let mut static_sticker = MessageMediaItem::new(
        "static-wide".to_owned(),
        MessageMediaKind::Sticker,
        "https://example.com/sticker.webp".to_owned(),
    );
    static_sticker.width = 512.0;
    static_sticker.height = 256.0;
    static_sticker.sticker_kind = Some(MessageStickerKind::Static);

    let mut animated_sticker = MessageMediaItem::new(
        "animated-tall".to_owned(),
        MessageMediaKind::Sticker,
        "https://example.com/sticker.mp4".to_owned(),
    );
    animated_sticker.width = 256.0;
    animated_sticker.height = 512.0;
    animated_sticker.sticker_kind = Some(MessageStickerKind::Animated);

    assert_eq!(MessageStickerLayout::display_size(&static_sticker), size(px(184.0), px(92.0)));
    assert_eq!(MessageStickerLayout::display_size(&animated_sticker), size(px(90.0), px(180.0)));
}

#[test]
fn grouped_descriptor_uses_telegram_corners_and_terminal_tail() {
    let first_outgoing = MessageBubbleDescriptor::grouped(true, true, false, px(16.0), px(8.0));
    assert_eq!(first_outgoing.radii.top_leading, px(16.0));
    assert_eq!(first_outgoing.radii.top_trailing, px(16.0));
    assert_eq!(first_outgoing.radii.bottom_leading, px(16.0));
    assert_eq!(first_outgoing.radii.bottom_trailing, px(8.0));
    assert_eq!(first_outgoing.tail_side, MessageBubbleTailSide::Trailing);
    assert!(!first_outgoing.draws_tail);

    let last_incoming = MessageBubbleDescriptor::grouped(false, false, true, px(16.0), px(8.0));
    assert_eq!(last_incoming.radii.top_leading, px(8.0));
    assert_eq!(last_incoming.radii.top_trailing, px(16.0));
    assert_eq!(last_incoming.radii.bottom_leading, px(16.0));
    assert_eq!(last_incoming.radii.bottom_trailing, px(16.0));
    assert_eq!(last_incoming.tail_side, MessageBubbleTailSide::Leading);
    assert!(last_incoming.draws_tail);
}

#[test]
fn terminal_tail_extends_into_six_point_allowance() {
    let bounds = rect(0.0, 0.0, 40.0, 35.0);
    let radii = MessageBubbleRadii::uniform(px(16.0));
    let terminal =
        bounding_box_of_path(&MessageBubbleGeometry::path(bounds, radii, MessageBubbleTailSide::Trailing, true));
    let merged =
        bounding_box_of_path(&MessageBubbleGeometry::path(bounds, radii, MessageBubbleTailSide::Trailing, false));
    let bounds_max_x = f32::from(bounds.right());

    assert!((merged.max_x - (bounds_max_x - f32::from(MessageBubbleGeometry::TAIL_INSET))).abs() < FLOAT_TOLERANCE);
    assert!(terminal.max_x > merged.max_x + 4.0);
    assert!(terminal.max_x <= bounds_max_x + FLOAT_TOLERANCE);
}

#[test]
fn incoming_and_outgoing_contours_mirror_exactly() {
    let bounds = rect(0.0, 0.0, 120.0, 52.0);
    let radii = MessageBubbleRadii::uniform(px(16.0));
    let outgoing =
        bounding_box_of_path(&MessageBubbleGeometry::path(bounds, radii, MessageBubbleTailSide::Trailing, true));
    let incoming =
        bounding_box_of_path(&MessageBubbleGeometry::path(bounds, radii, MessageBubbleTailSide::Leading, true));
    let bounds_max_x = f32::from(bounds.right());

    assert!((incoming.min_x - (bounds_max_x - outgoing.max_x)).abs() < FLOAT_TOLERANCE);
    assert!((incoming.max_x - (bounds_max_x - outgoing.min_x)).abs() < FLOAT_TOLERANCE);
    assert!((incoming.height() - outgoing.height()).abs() < FLOAT_TOLERANCE);
}

#[test]
fn media_tail_requires_terminal_full_bottom_corner() {
    assert_eq!(
        MessageMediaTailGeometry::eligible_side(true, MessageBubbleTailSide::Leading, px(16.0), px(16.0)),
        MessageBubbleTailSide::Leading
    );
    assert_eq!(
        MessageMediaTailGeometry::eligible_side(false, MessageBubbleTailSide::Trailing, px(16.0), px(16.0)),
        MessageBubbleTailSide::None
    );
    assert_eq!(
        MessageMediaTailGeometry::eligible_side(true, MessageBubbleTailSide::Trailing, px(16.0), px(0.0)),
        MessageBubbleTailSide::None
    );
}

#[test]
fn media_tail_cap_is_the_adjacent_slice_of_the_content_contour() {
    let media_bounds = rect(0.0, 0.0, 120.0, 80.0);
    let radii = MessageBubbleRadii::uniform(px(16.0));
    let content_path = MessageMediaTailGeometry::content_mask_path(media_bounds, radii, MessageBubbleTailSide::Trailing);
    let cap_path = MessageMediaTailGeometry::cap_path_for_media(media_bounds, radii, MessageBubbleTailSide::Trailing);

    assert_eq!(MessageMediaTailGeometry::extension_width(MessageBubbleTailSide::Leading), px(4.0));
    assert_eq!(MessageMediaTailGeometry::extension_width(MessageBubbleTailSide::Trailing), px(5.0));
    assert_eq!(MessageMediaTailGeometry::CAP_HEIGHT, px(7.0));
    assert_eq!(MessageMediaTailGeometry::MASK_REFERENCE_HEIGHT, px(8.0));
    for local_point in [(0.25, 0.25), (1.5, 3.5), (4.75, 6.75)] {
        let media_point = (
            f32::from(media_bounds.right()) + local_point.0,
            f32::from(media_bounds.bottom()) - f32::from(MessageMediaTailGeometry::CAP_HEIGHT) + local_point.1,
        );
        assert_eq!(path_contains(&cap_path, local_point), path_contains(&content_path, media_point));
    }
}

#[test]
fn media_tail_samples_true_leading_and_trailing_bottom_pixels() {
    let image = two_column_image((255, 0, 0, 255), (0, 0, 255, 255));
    let bitmap = MessageMediaTailBitmap::new(2, 2, &image).expect("2x2 BGRA bitmap");
    let colors = MessageMediaTailColorSampler::visible_bottom_edge_colors(bitmap, None, 2.0, 2.0, false)
        .expect("both edges sample");

    assert!(colors.leading.red > 0.99);
    assert!(colors.leading.blue < 0.01);
    assert!(colors.trailing.red < 0.01);
    assert!(colors.trailing.blue > 0.99);
    assert_eq!(colors.leading.alpha, 1.0);
    assert_eq!(colors.trailing.alpha, 1.0);
}

#[test]
fn media_tail_composites_transparent_edges_to_opaque_visible_colors() {
    let image = two_column_image((0, 0, 0, 0), (0, 255, 0, 128));
    let bitmap = MessageMediaTailBitmap::new(2, 2, &image).expect("2x2 BGRA bitmap");
    let colors = MessageMediaTailColorSampler::visible_bottom_edge_colors(bitmap, None, 2.0, 2.0, false)
        .expect("both edges sample");

    assert_eq!(colors.leading, MessageMediaTailColor::opaque(0.0, 0.0, 0.0));
    assert_eq!(colors.trailing.alpha, 1.0);
    assert!(colors.trailing.green > 0.49);
    assert!(colors.trailing.green < 0.51);
}

#[test]
fn async_text_insets_reserve_tail_without_changing_bubble_size() {
    let outgoing_spec = RiseAsyncTextMessageEligibility::layout_spec(220.0, 390.0, Some(true));
    let incoming_spec = RiseAsyncTextMessageEligibility::layout_spec(220.0, 390.0, Some(false));
    let padding = MessageSizes::bubble_inner_horizontal_padding();
    let tail = MessageSizes::bubble_tail_inset();

    assert_eq!(outgoing_spec.leading_content_inset(), px(padding));
    assert_eq!(outgoing_spec.trailing_content_inset(), px(padding + tail));
    assert_eq!(incoming_spec.leading_content_inset(), px(padding + tail));
    assert_eq!(incoming_spec.trailing_content_inset(), px(padding));
}

#[test]
fn regular_width_text_uses_telegram_ten_point_inset() {
    let spec = RiseAsyncTextMessageEligibility::layout_spec(490.0, 768.0, Some(true));

    assert_eq!(spec.horizontal_padding, px(10.0));
    assert_eq!(spec.leading_content_inset(), px(10.0));
    assert_eq!(spec.trailing_content_inset(), px(16.0));
}

#[test]
fn instant_video_uses_telegram_compact_width_breakpoint() {
    assert_eq!(VideoNoteMessageView::expanded_size(320.0, false), 212.0);
    assert_eq!(VideoNoteMessageView::expanded_size(321.0, false), 240.0);
    assert_eq!(VideoNoteMessageView::expanded_size(768.0, true), 240.0);
}

#[gpui::test]
fn fallback_cache_separates_equal_widths_across_metric_classes(cx: &mut TestAppContext) {
    let mut message = GetMessageMessage::new("cache-message".to_owned());
    message.chat_id = "cache-chat".to_owned();
    message.sender_id = "another-user".to_owned();
    message.text = "https://example.com".to_owned();
    let entry = message_entry(message);
    let chat = direct_chat("cache-chat");
    let locale = Locale::new("en_US");
    let calendar = Calendar::gregorian();
    let time_zone = gmt();

    let (compact, regular) = cx.update(|cx| {
        let compact = RiseMessageFallbackHeightCache::make_key(
            &entry,
            &chat,
            520.0,
            &locale,
            &calendar,
            &time_zone,
            LayoutDirection::LeftToRight,
            3.0,
            None,
            cx,
        );
        let regular = RiseMessageFallbackHeightCache::make_key(
            &entry,
            &chat,
            681.0,
            &locale,
            &calendar,
            &time_zone,
            LayoutDirection::LeftToRight,
            3.0,
            None,
            cx,
        );
        (compact, regular)
    });

    assert_eq!(compact.effective_bubble_width_bucket, regular.effective_bubble_width_bucket);
    assert_ne!(compact.layout_metrics_bucket, regular.layout_metrics_bucket);
    assert_ne!(compact, regular);
}

#[test]
fn fallback_height_cache_evicts_least_recent_entry_in_constant_order() {
    let mut cache = RiseMessageFallbackHeightCache::new(2);
    let key = |value: u64| RiseMessageFallbackHeightCacheKey {
        geometry_version: RiseMessageGeometryVersion { raw_value: value },
        effective_bubble_width_bucket: 900,
        display_scale_bucket: 3_000,
        layout_metrics_bucket: 0,
        locale_identifier: "en_US".to_owned(),
        layout_direction: LayoutDirection::LeftToRight,
    };
    let first = key(1);
    let second = key(2);
    let third = key(3);

    cache.insert(10.0, &first);
    cache.insert(20.0, &second);
    assert_eq!(cache.height(&first), Some(10.0));
    cache.insert(30.0, &third);

    assert_eq!(cache.count(), 2);
    assert_eq!(cache.height(&second), None);
    assert_eq!(cache.height(&first), Some(10.0));
    assert_eq!(cache.height(&third), Some(30.0));
}

#[gpui::test]
fn document_thumbnail_presence_invalidates_fallback_geometry(cx: &mut TestAppContext) {
    let document = |thumbnail_url: Option<&str>| {
        let mut item = MessageMediaItem::new(
            "document-item".to_owned(),
            MessageMediaKind::Document,
            "https://example.com/report.pdf".to_owned(),
        );
        item.thumbnail_url = thumbnail_url.map(str::to_owned);
        item.file_name = Some("report.pdf".to_owned());
        item.file_size = Some(42_000);
        item
    };

    let mut without_thumbnail = GetMessageMessage::new("document-message".to_owned());
    without_thumbnail.chat_id = "document-chat".to_owned();
    without_thumbnail.sender_id = "another-user".to_owned();
    without_thumbnail.media_items = vec![document(None)];

    let mut with_thumbnail = without_thumbnail.clone();
    with_thumbnail.media_items = vec![document(Some("https://example.com/report-preview.jpg"))];

    let chat = direct_chat("document-chat");
    let (without, with) = cx.update(|cx| {
        (fallback_version(&without_thumbnail, &chat, cx), fallback_version(&with_thumbnail, &chat, cx))
    });

    assert_ne!(without, with);
}

#[gpui::test]
fn sticker_kind_invalidates_fallback_geometry(cx: &mut TestAppContext) {
    let sticker = |url: &str, kind: MessageStickerKind| {
        let mut item = MessageMediaItem::new("sticker-item".to_owned(), MessageMediaKind::Sticker, url.to_owned());
        item.width = 512.0;
        item.height = 512.0;
        item.sticker_kind = Some(kind);
        item
    };

    let mut static_message = GetMessageMessage::new("sticker-message".to_owned());
    static_message.chat_id = "sticker-chat".to_owned();
    static_message.sender_id = "another-user".to_owned();
    static_message.media_items = vec![sticker("https://example.com/sticker.webp", MessageStickerKind::Static)];

    let mut animated_message = static_message.clone();
    animated_message.media_items = vec![sticker("https://example.com/sticker.mp4", MessageStickerKind::Animated)];

    let chat = direct_chat("sticker-chat");
    let (static_version, animated_version) = cx.update(|cx| {
        (fallback_version(&static_message, &chat, cx), fallback_version(&animated_message, &chat, cx))
    });

    assert_ne!(static_version, animated_version);
}

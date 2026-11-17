use futures::executor::block_on;
use gpui::{TestAppContext, px};
use rise_core::foundation::foundation_date::Date;
use rise_ui::message_bubble_geometry::{MessageBubbleGeometry, MessageBubbleTailSide};

use crate::modules::chat::components::chat::async_render::async_text_bubble_preparer::AsyncTextBubblePreparer;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleDeliveryState, AsyncTextBubbleLayoutSpec, AsyncTextBubbleSeed,
};

fn spec() -> AsyncTextBubbleLayoutSpec {
    AsyncTextBubbleLayoutSpec::new(
        px(240.0),
        px(48.0),
        px(34.0),
        px(12.0),
        px(7.0),
        None,
        None,
        MessageBubbleTailSide::None,
        MessageBubbleGeometry::TAIL_INSET,
        px(17.0),
        px(10.0),
        px(2.0),
        px(8.0),
        px(3.0),
        px(10.0),
        px(3.0),
    )
}

fn seed(text: &str, delivery_state: AsyncTextBubbleDeliveryState) -> AsyncTextBubbleSeed {
    AsyncTextBubbleSeed::new("test-message", text, Date::from_time_interval_since_1970(1_700_000_000.0))
        .time_zone_identifier("UTC")
        .delivery_state(delivery_state)
}

#[gpui::test]
fn short_outgoing_text_keeps_metadata_inline(cx: &mut TestAppContext) {
    let spec = spec();
    let layout = AsyncTextBubblePreparer::prepare_synchronously(
        seed("Hello", AsyncTextBubbleDeliveryState::Read),
        spec,
        cx.text_system(),
    );

    assert_eq!(layout.text_lines.len(), 1);
    assert!(layout.status_frame.is_some());
    assert!(layout.size.width >= spec.minimum_width);
    assert!(layout.size.width <= spec.maximum_width);
    assert_eq!(layout.time_baseline_origin.y, layout.text_lines[0].baseline_origin.y);
}

#[gpui::test]
fn long_text_wraps_within_maximum_width(cx: &mut TestAppContext) {
    let spec = spec();
    let layout = AsyncTextBubblePreparer::prepare_synchronously(
        seed(
            "A deliberately long message that must wrap across multiple CoreText lines without making the bubble wider than its layout contract.",
            AsyncTextBubbleDeliveryState::None,
        ),
        spec,
        cx.text_system(),
    );

    assert!(layout.text_lines.len() > 1);
    assert!(layout.status_frame.is_none());
    assert!(layout.size.width <= spec.maximum_width);
    assert!(layout.size.height > spec.minimum_height);
}

#[gpui::test]
fn detached_preparation_matches_synchronous_geometry(cx: &mut TestAppContext) {
    let spec = spec();
    let text_system = cx.text_system().clone();
    let input = seed("Prepared away from the main thread", AsyncTextBubbleDeliveryState::Sent);
    let synchronous = AsyncTextBubblePreparer::prepare_synchronously(input.clone(), spec, &text_system);
    let asynchronous = block_on(AsyncTextBubblePreparer::prepare(input, spec, text_system));

    assert_eq!(asynchronous.size, synchronous.size);
    assert_eq!(asynchronous.text_lines.len(), synchronous.text_lines.len());
    assert_eq!(asynchronous.time_baseline_origin, synchronous.time_baseline_origin);
    assert_eq!(asynchronous.status_frame, synchronous.status_frame);
}

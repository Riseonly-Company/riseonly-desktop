use gpui::{TestAppContext, px, size};
use rise_core::foundation::foundation_date::Date;

use crate::modules::chat::components::chat::async_render::async_text_bubble_preparer::AsyncTextBubblePreparer;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleDeliveryState, AsyncTextBubbleLayoutSpec, AsyncTextBubbleSeed, bidi_line_mark, first_strong_is_rtl,
};

fn seed(delivery_state: AsyncTextBubbleDeliveryState) -> AsyncTextBubbleSeed {
    AsyncTextBubbleSeed::new(
        "status-reuse-message",
        "hello",
        Date::from_time_interval_since_1970(1_700_000_000.0),
    )
    .time_zone_identifier("UTC")
    .delivery_state(delivery_state)
}

#[gpui::test]
fn status_metric_change_invalidates_geometry_reuse(cx: &mut TestAppContext) {
    let text_system = cx.text_system().clone();
    let spec = AsyncTextBubbleLayoutSpec {
        maximum_width: px(240.0),
        status_icon_size: px(11.0),
        ..AsyncTextBubbleLayoutSpec::default()
    };
    let sent_seed = seed(AsyncTextBubbleDeliveryState::Sent);
    let delivered_seed = seed(AsyncTextBubbleDeliveryState::Delivered);
    let read_seed = seed(AsyncTextBubbleDeliveryState::Read);
    let sent_layout = AsyncTextBubblePreparer::prepare_synchronously(sent_seed.clone(), spec, &text_system);
    let delivered_layout = AsyncTextBubblePreparer::prepare_synchronously(delivered_seed.clone(), spec, &text_system);

    assert_eq!(sent_layout.status_frame.map(|frame| frame.size), Some(size(px(11.0), px(9.0))));
    assert_eq!(delivered_layout.status_frame.map(|frame| frame.size), Some(size(px(17.0), px(9.0))));
    assert!(sent_layout.reusing_geometry(&delivered_seed).is_none());
    assert!(delivered_layout.reusing_geometry(&sent_seed).is_none());

    let reused_read_layout = delivered_layout.reusing_geometry(&read_seed);
    assert_eq!(
        reused_read_layout.as_ref().map(|layout| layout.seed.delivery_state),
        Some(AsyncTextBubbleDeliveryState::Read)
    );
    assert_eq!(
        reused_read_layout.as_ref().and_then(|layout| layout.status_frame),
        delivered_layout.status_frame
    );
}

#[test]
fn the_first_letter_decides_the_direction() {
    assert_eq!(first_strong_is_rtl("123 \u{05E2}\u{05D1}\u{05E8}\u{05D9}\u{05EA} abc"), Some(true));
    assert_eq!(first_strong_is_rtl("\u{1F600} \u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064A}\u{0629}"), Some(true));
    assert_eq!(first_strong_is_rtl("\u{0428}\u{0440}\u{0438}\u{0444}\u{0442} \u{05D0}"), Some(false));
    assert_eq!(first_strong_is_rtl("12:30 \u{1F600}"), None);
}

#[test]
fn a_wrapped_line_keeps_the_direction_of_its_paragraph() {
    let hebrew_tail = "\u{05D9}\u{05EA} \u{0E44}\u{0E17}\u{0E22} \u{1F600}";
    assert_eq!(bidi_line_mark(Some(false), hebrew_tail), Some('\u{200E}'));
    assert_eq!(bidi_line_mark(Some(true), "abc \u{05D0}"), Some('\u{200F}'));
    assert_eq!(bidi_line_mark(Some(false), "abc \u{05D0}"), None);
    assert_eq!(bidi_line_mark(Some(true), "\u{05D0} abc"), None);
    assert_eq!(bidi_line_mark(None, hebrew_tail), None);
    assert_eq!(bidi_line_mark(Some(false), "123 \u{1F600}"), None);
}

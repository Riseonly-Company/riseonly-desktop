use super::*;

fn solid(width: usize, height: usize, rgba: [u8; 4]) -> Vec<u8> {
    let [r, g, b, a] = rgba;
    std::iter::repeat_n([b, g, r, a], width * height).flatten().collect()
}

#[test]
fn colors_are_clamped_into_the_unit_range() {
    let color = MessageMediaTailColor::new(1.5, -0.2, 0.5, 2.0);
    assert_eq!(color, MessageMediaTailColor::new(1.0, 0.0, 0.5, 1.0));
}

#[test]
fn darkening_scales_the_channels_and_keeps_alpha() {
    let color = MessageMediaTailColor::new(1.0, 0.5, 0.0, 0.5).darkened(0.5);
    assert_eq!(color, MessageMediaTailColor::new(0.5, 0.25, 0.0, 0.5));
}

#[test]
fn only_a_tail_corner_round_enough_is_eligible() {
    let eligible = MessageMediaTailGeometry::eligible_side(
        true,
        MessageBubbleTailSide::Leading,
        px(14.0),
        px(0.0),
    );
    assert_eq!(eligible, MessageBubbleTailSide::Leading);
    let too_tight = MessageMediaTailGeometry::eligible_side(
        true,
        MessageBubbleTailSide::Trailing,
        px(16.0),
        px(13.0),
    );
    assert_eq!(too_tight, MessageBubbleTailSide::None);
    let no_tail = MessageMediaTailGeometry::eligible_side(
        false,
        MessageBubbleTailSide::Leading,
        px(16.0),
        px(16.0),
    );
    assert_eq!(no_tail, MessageBubbleTailSide::None);
}

#[test]
fn no_side_has_no_cap() {
    let rect = Bounds::new(point(px(0.0), px(0.0)), size(px(4.0), px(7.0)));
    assert!(MessageMediaTailGeometry::cap_path(rect, MessageBubbleTailSide::None, None).is_empty());
    let empty = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(7.0)));
    assert!(MessageMediaTailGeometry::cap_path(empty, MessageBubbleTailSide::Leading, None).is_empty());
}

#[test]
fn a_cap_is_drawn_for_a_tail_side() {
    let rect = Bounds::new(point(px(10.0), px(20.0)), size(px(4.0), px(7.0)));
    assert!(!MessageMediaTailGeometry::cap_path(rect, MessageBubbleTailSide::Leading, None).is_empty());
}

#[test]
fn a_filled_photo_reports_its_bottom_corners_on_black() {
    let bytes = solid(4, 4, [255, 0, 0, 255]);
    let photo = MessageMediaTailBitmap::new(4, 4, &bytes).expect("bitmap");
    let colors =
        MessageMediaTailColorSampler::visible_bottom_edge_colors(photo, None, 100.0, 100.0, false)
            .expect("colors");
    assert_eq!(colors.leading, MessageMediaTailColor::opaque(1.0, 0.0, 0.0));
    assert_eq!(colors.trailing, colors.leading);
}

#[test]
fn a_fit_photo_outside_its_rect_shows_the_darkened_backdrop() {
    let wide = solid(4, 1, [255, 255, 255, 255]);
    let photo = MessageMediaTailBitmap::new(4, 1, &wide).expect("bitmap");
    let colors =
        MessageMediaTailColorSampler::visible_bottom_edge_colors(photo, None, 100.0, 100.0, true)
            .expect("colors");
    assert_eq!(colors.leading, MessageMediaTailColor::opaque(0.0, 0.0, 0.0));
}

#[test]
fn a_target_under_two_points_has_no_colors() {
    let bytes = solid(1, 1, [0, 0, 0, 255]);
    let photo = MessageMediaTailBitmap::new(1, 1, &bytes).expect("bitmap");
    assert!(
        MessageMediaTailColorSampler::visible_bottom_edge_colors(photo, None, 1.0, 100.0, false)
            .is_none()
    );
}

#[test]
fn a_short_buffer_is_not_a_bitmap() {
    assert!(MessageMediaTailBitmap::new(2, 2, &[0; 15]).is_none());
}

#[test]
fn hero_clip_radii_round_trip_through_their_animatable_data() {
    let shape = MessageMediaHeroClipShape {
        radii: MessageBubbleRadii {
            top_leading: px(1.0),
            top_trailing: px(2.0),
            bottom_leading: px(3.0),
            bottom_trailing: px(4.0),
        },
    };
    assert_eq!(MessageMediaHeroClipShape::from_components(&shape.components()), shape);
}

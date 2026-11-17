use gpui::{Edges, point, px, size};

use crate::window_cover::WindowCoverFrame;

#[test]
fn a_cover_without_a_band_or_a_client_frame_is_the_viewport() {
    let frame = WindowCoverFrame::laid_out(size(px(800.0), px(600.0)), Edges::default(), 0.0, 46.0);
    assert_eq!(frame.origin, point(px(0.0), px(0.0)));
    assert_eq!(frame.size, size(px(800.0), px(600.0)));
    assert_eq!(frame.safe_top, px(46.0));
    assert_eq!(frame.sheet_container_height(), 554.0);
}

#[test]
fn a_cover_starts_below_the_band_and_inside_the_client_frame() {
    let client = Edges { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 };
    let frame = WindowCoverFrame::laid_out(size(px(800.0), px(600.0)), client, 37.0, 0.0);
    assert_eq!(frame.origin, point(px(10.0), px(47.0)));
    assert_eq!(frame.size, size(px(780.0), px(543.0)));
    assert_eq!(frame.sheet_container_height(), 543.0);
}

#[test]
fn a_tiny_window_never_gives_a_negative_size() {
    let frame = WindowCoverFrame::laid_out(size(px(4.0), px(4.0)), Edges::default(), 37.0, 0.0);
    assert_eq!(frame.size.height, px(0.0));
}

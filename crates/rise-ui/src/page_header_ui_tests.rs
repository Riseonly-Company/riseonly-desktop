use super::*;

#[test]
fn a_short_centre_sits_in_the_middle_of_the_bar() {
    let layout = PageHeaderBarLayout::new();
    let placement = layout.center_placement(0.0, 400.0, 45.0, 45.0, 100.0);
    assert_eq!(placement, PageHeaderCenterPlacement { mid_x: 200.0, width: 100.0 });
}

#[test]
fn a_centre_slides_away_from_the_wider_side() {
    let layout = PageHeaderBarLayout::new();
    let placement = layout.center_placement(0.0, 400.0, 45.0, 140.0, 180.0);
    assert_eq!(placement.width, 180.0);
    assert_eq!(placement.mid_x, 158.0);
}

#[test]
fn a_centre_wider_than_the_room_is_offered_the_room() {
    let layout = PageHeaderBarLayout::new();
    let placement = layout.center_placement(0.0, 400.0, 45.0, 45.0, 1000.0);
    assert_eq!(placement.width, 286.0);
    assert_eq!(placement.mid_x, 200.0);
}

#[test]
fn sides_wider_than_the_bar_leave_the_centre_no_width() {
    let layout = PageHeaderBarLayout::new();
    let placement = layout.center_placement(0.0, 100.0, 80.0, 80.0, 50.0);
    assert_eq!(placement.width, 0.0);
}

#[test]
fn blank_copy_shows_no_text_centre() {
    let header = PageHeaderUi::new()
        .title(SharedString::from("  \n"))
        .subtitle(SharedString::from(""));
    assert!(header.trimmed_title().is_none());
    assert!(header.trimmed_subtitle().is_none());
    assert!(!header.shows_default_text_center());
}

#[test]
fn a_title_is_trimmed_before_it_shows() {
    let header = PageHeaderUi::new().title(SharedString::from("  Chats \n"));
    assert_eq!(header.trimmed_title(), Some(SharedString::from("Chats")));
    assert!(header.shows_default_text_center());
}

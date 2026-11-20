use super::window_zoom_preferences::{
    WINDOW_ZOOM_DEFAULT, WINDOW_ZOOM_STEPS, next_zoom_step, previous_zoom_step, stored_zoom_percent, zoom_factor,
};

#[test]
fn zoom_walks_the_browser_steps_and_stops_at_both_ends() {
    assert_eq!(next_zoom_step(100), 110);
    assert_eq!(previous_zoom_step(100), 90);
    assert_eq!(next_zoom_step(200), 200);
    assert_eq!(previous_zoom_step(60), 60);
    assert_eq!(previous_zoom_step(70), 60);
    let mut percent = WINDOW_ZOOM_STEPS[0];
    let mut walked = vec![percent];
    while next_zoom_step(percent) != percent {
        percent = next_zoom_step(percent);
        walked.push(percent);
    }
    assert_eq!(walked, WINDOW_ZOOM_STEPS);
}

#[test]
fn a_stored_zoom_outside_the_steps_falls_back_to_one_hundred() {
    assert_eq!(stored_zoom_percent(0), WINDOW_ZOOM_DEFAULT);
    assert_eq!(stored_zoom_percent(-50), WINDOW_ZOOM_DEFAULT);
    assert_eq!(stored_zoom_percent(50), WINDOW_ZOOM_DEFAULT);
    assert_eq!(stored_zoom_percent(250), WINDOW_ZOOM_DEFAULT);
    assert_eq!(stored_zoom_percent(150), 150);
    assert!((zoom_factor(125) - 1.25).abs() < f32::EPSILON);
}

#[test]
fn an_old_step_stored_before_the_round_steps_moves_to_the_nearest_new_one() {
    assert_eq!(WINDOW_ZOOM_STEPS[0], 60);
    assert_eq!(stored_zoom_percent(67), 70);
    assert_eq!(stored_zoom_percent(75), 80);
    assert_eq!(stored_zoom_percent(133), 125);
    assert_eq!(stored_zoom_percent(60), 60);
}

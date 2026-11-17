use gpui::{bounds, point, size};

use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorVideoState};
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;

#[test]
fn versioned_image_state_round_trips() {
    let mut state = EditorImageState::empty(Some("asset-image".to_string()), size(4032.0, 3024.0));
    state.rotation_steps = 3;
    state.free_rotation_degrees = -12.5;
    state.is_mirrored = true;
    state.crop_rect_relative = Some(bounds(point(0.1, 0.2), size(0.7, 0.6)));
    state.caption = "caption".to_string();
    state.was_edited = true;

    let data = EditorStateCodec::encode_image(&state).expect("image state encodes");
    let decoded = EditorStateCodec::decode_image(&data).expect("image state decodes");

    assert_eq!(decoded, state);
}

#[test]
fn legacy_unwrapped_image_state_still_decodes() {
    let mut state = EditorImageState::empty(Some("legacy-image".to_string()), size(1200.0, 1600.0));
    state.adjustments.contrast = 0.25;
    state.was_edited = true;

    let legacy_data = serde_json::to_vec(&state).expect("legacy image state encodes");
    let decoded = EditorStateCodec::decode_image(&legacy_data).expect("legacy image state decodes");

    assert_eq!(decoded, state);
}

#[test]
fn versioned_video_state_round_trips() {
    let mut state = EditorVideoState::empty(Some("asset-video".to_string()), 18.0);
    state.start_seconds = 2.0;
    state.end_seconds = 14.0;
    state.is_muted = true;
    state.caption = "video".to_string();
    state.was_edited = true;

    let data = EditorStateCodec::encode_video(&state).expect("video state encodes");
    let decoded = EditorStateCodec::decode_video(&data).expect("video state decodes");

    assert_eq!(decoded, state);
}

#[test]
fn edit_flags_return_to_clean_after_every_visual_setting_is_reset() {
    let mut image = EditorImageState::empty(Some("reset-image".to_string()), size(1000.0, 1000.0));
    image.was_edited = true;
    image.synchronize_edit_flag();

    let mut video = EditorVideoState::empty(Some("reset-video".to_string()), 10.0);
    video.was_edited = true;

    assert!(!image.was_edited);
    assert!(!image.has_meaningful_edits());
    assert!(!video.has_meaningful_edits());
}

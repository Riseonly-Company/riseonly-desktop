use std::collections::HashSet;

use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseMessagePreparedGeometryPolicy, RiseMessageRuntimeHeightCorrectionPolicy,
};

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn id_set(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn equal_identity_lists_remain_eligible_for_zero_edge_commit() {
    assert!(
        RiseMessagePreparedGeometryPolicy::identities_can_form_scroll_safe_mutation(
            &ids(&["a", "b", "c"]),
            &ids(&["a", "b", "c"]),
        )
    );
    assert!(
        !RiseMessagePreparedGeometryPolicy::identities_can_form_scroll_safe_mutation(
            &ids(&["a", "a"]),
            &ids(&["a", "a"]),
        )
    );
}

#[test]
fn changed_retained_row_never_adopts_previous_height() {
    assert!(
        !RiseMessagePreparedGeometryPolicy::should_adopt_previous_fallback_height(
            Some(1),
            2,
            Some(88.0),
            Some(64.0),
        )
    );
    assert!(
        !RiseMessagePreparedGeometryPolicy::should_adopt_previous_fallback_height(
            Some(1),
            2,
            None,
            Some(64.0),
        )
    );
}

#[test]
fn unchanged_retained_row_only_fills_missing_exact_height() {
    assert!(
        RiseMessagePreparedGeometryPolicy::should_adopt_previous_fallback_height(
            Some(3),
            3,
            None,
            Some(72.0),
        )
    );
    assert!(
        !RiseMessagePreparedGeometryPolicy::should_adopt_previous_fallback_height(
            Some(3),
            3,
            Some(80.0),
            Some(72.0),
        )
    );
}

#[test]
fn retained_item_geometry_requires_matching_version_and_valid_height() {
    assert!(
        RiseMessagePreparedGeometryPolicy::can_reuse_retained_item_geometry(
            Some(9),
            9,
            Some(60.0),
        )
    );
    assert!(
        !RiseMessagePreparedGeometryPolicy::can_reuse_retained_item_geometry(
            Some(8),
            9,
            Some(60.0),
        )
    );
    assert!(
        !RiseMessagePreparedGeometryPolicy::can_reuse_retained_item_geometry(
            Some(9),
            9,
            Some(f32::NAN),
        )
    );
    assert!(!RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(Some(0.0)));
    assert!(!RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(None));
}

#[test]
fn runtime_correction_waits_only_for_user_scroll_after_reveal() {
    assert!(RiseMessageRuntimeHeightCorrectionPolicy::can_apply(
        false,
        true,
        &id_set(&["message-a"]),
        &id_set(&["message-a"]),
    ));
    assert!(!RiseMessageRuntimeHeightCorrectionPolicy::can_apply(
        true,
        true,
        &id_set(&[]),
        &id_set(&["message-a"]),
    ));
    assert!(RiseMessageRuntimeHeightCorrectionPolicy::can_apply(
        true,
        false,
        &id_set(&["message-a"]),
        &id_set(&["message-a"]),
    ));
    assert!(RiseMessageRuntimeHeightCorrectionPolicy::can_apply(
        true,
        false,
        &id_set(&["message-a"]),
        &id_set(&["message-b"]),
    ));
}

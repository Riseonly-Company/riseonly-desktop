use std::collections::HashSet;

use super::*;

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn set(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn mounts_only_resolved_sections_and_one_next_placeholder() {
    let plan = ChatInputPackSectionPolicy::plan(
        &ids(&["a", "b", "c", "d"]),
        &set(&["a", "c"]),
        RiseStickerPerformancePolicy::packs_limit(),
    );

    assert_eq!(plan.resolved_pack_ids, ids(&["a", "c"]));
    assert_eq!(plan.next_unresolved_pack_id.as_deref(), Some("b"));
}

#[test]
fn deduplicates_and_bounds_pack_sections() {
    let plan = ChatInputPackSectionPolicy::plan(&ids(&["", "a", "a", "b", "c"]), &set(&["a", "b", "c"]), 2);

    assert_eq!(plan.resolved_pack_ids, ids(&["a", "b"]));
    assert_eq!(plan.next_unresolved_pack_id, None);
}

#[test]
fn placeholder_uses_stable_grid_rows() {
    assert!(ChatInputPackSectionPolicy::PLACEHOLDER_ROWS > 0);
}

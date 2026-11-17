use super::chat_pinned_bar::ChatPinnedCyclePolicy;

#[test]
fn test_cycles_from_newest_through_older_pins_and_wraps() {
    let mut index = 2;

    index = ChatPinnedCyclePolicy::next_index(index, 3);
    assert_eq!(index, 1);

    index = ChatPinnedCyclePolicy::next_index(index, 3);
    assert_eq!(index, 0);

    index = ChatPinnedCyclePolicy::next_index(index, 3);
    assert_eq!(index, 2);
}

#[test]
fn test_single_pin_remains_selected() {
    assert_eq!(ChatPinnedCyclePolicy::next_index(0, 1), 0);
}

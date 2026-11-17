use super::feature_hooks::FeatureHookSlot;

#[test]
fn a_filled_slot_keeps_the_first_install() {
    static SLOT: FeatureHookSlot<u8> = FeatureHookSlot::new("SLOT");
    assert!(!SLOT.is_installed());
    assert_eq!(SLOT.set(1), Ok(()));
    assert_eq!(SLOT.set(2), Err(2));
    assert_eq!(SLOT.get(), Some(&1));
}

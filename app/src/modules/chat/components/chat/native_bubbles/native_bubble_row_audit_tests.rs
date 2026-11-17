use crate::modules::chat::components::chat::native_bubbles::native_bubble_row_audit::NativeBubbleRowAudit;

#[test]
fn equal_heights_pass() {
    assert_eq!(NativeBubbleRowAudit::mismatch(184.0, Some(184.0)), None);
}

#[test]
fn drift_within_one_point_passes() {
    assert_eq!(NativeBubbleRowAudit::mismatch(72.0, Some(72.6)), None);
    assert_eq!(NativeBubbleRowAudit::mismatch(73.0, Some(72.0)), None);
    assert_eq!(NativeBubbleRowAudit::mismatch(71.0, Some(72.0)), None);
}

#[test]
fn drift_over_one_point_reports_signed_delta() {
    assert_eq!(NativeBubbleRowAudit::mismatch(70.5, Some(72.0)), Some(-1.5));
    assert_eq!(NativeBubbleRowAudit::mismatch(186.0, Some(184.0)), Some(2.0));
}

#[test]
fn regular_row_without_exact_height_is_skipped() {
    assert_eq!(NativeBubbleRowAudit::mismatch(240.0, None), None);
}

#[test]
fn audit_logs_under_chat_rows_target() {
    assert_eq!(NativeBubbleRowAudit::LOG_TARGET, "riseonly::chat_rows");
}

use rise_canon::rise_request::report::SubmitDiagnosticReport;

use super::{RiseBugReportFields, RiseDiagnosticsLocale, SubmitDiagnosticReportDiagnostics};

fn bug() -> RiseBugReportFields {
    RiseBugReportFields {
        client_report_id: "client-1".to_owned(),
        parent_report_id: None,
        message: "broken".to_owned(),
        pages: Vec::new(),
        component: None,
        screenshot_file_urls: Vec::new(),
        screenshot_file_ids: vec!["orphan".to_owned()],
        video_file_url: None,
        video_file_id: None,
        video_duration_ms: None,
        feedback_kind: None,
    }
}

fn request(bug: Option<&RiseBugReportFields>) -> SubmitDiagnosticReport {
    SubmitDiagnosticReport::diagnostic(
        "user_report".to_owned(),
        "{}".to_owned(),
        "1.0.0".to_owned(),
        "1".to_owned(),
        "15.0".to_owned(),
        "Mac".to_owned(),
        Some("en_US".to_owned()),
        None,
        bug,
    )
}

#[test]
fn locale_keywords_are_cut() {
    assert_eq!(RiseDiagnosticsLocale::clamped("ru_KZ@calendar=gregorian").as_deref(), Some("ru_KZ"));
}

#[test]
fn blank_locale_is_absent() {
    assert_eq!(RiseDiagnosticsLocale::clamped("  "), None);
    assert_eq!(RiseDiagnosticsLocale::clamped("@calendar=gregorian"), None);
}

#[test]
fn locale_never_exceeds_the_canon_ceiling() {
    let long = "x".repeat(200);
    let clamped = RiseDiagnosticsLocale::clamped(&long).expect("not blank");
    assert!(clamped.chars().count() < long.chars().count());
}

#[test]
fn automatic_report_carries_no_bug_half() {
    let request = request(None);
    assert_eq!(request.client_report_id, None);
    assert_eq!(request.message, None);
    assert_eq!(request.pages, None);
    assert_eq!(request.screenshot_file_ids, None);
}

#[test]
fn empty_lists_are_left_out_and_ids_follow_urls() {
    let bug = bug();
    let request = request(Some(&bug));
    assert_eq!(request.client_report_id.as_deref(), Some("client-1"));
    assert_eq!(request.message.as_deref(), Some("broken"));
    assert_eq!(request.pages, None);
    assert_eq!(request.screenshot_file_urls, None);
    assert_eq!(request.screenshot_file_ids, None);
    assert_eq!(request.parent_report_id, None);
}

use super::{
    PAYLOAD_CEILING, RiseBugReportEnvelope, RiseBugReportEnvelopeBreadcrumb,
    RiseBugReportEnvelopeContext, RiseBugReportEnvelopeMetrics,
};

fn envelope(breadcrumbs: usize, message: &str) -> RiseBugReportEnvelope {
    RiseBugReportEnvelope {
        breadcrumbs: (0..breadcrumbs)
            .map(|index| RiseBugReportEnvelopeBreadcrumb {
                at_ms: 1_793_886_545_000 + i64::try_from(index).unwrap_or(0),
                channel: "rpc".to_owned(),
                message: message.to_owned(),
            })
            .collect(),
        captured_at_ms: 1_793_886_545_000,
        context: RiseBugReportEnvelopeContext {
            is_debug_build: true,
            locale: "ru_KZ".to_owned(),
            preferred_languages: vec!["ru".to_owned()],
            socket_connected: false,
            time_zone: "Asia/Almaty".to_owned(),
        },
        dropped_breadcrumbs: 0,
        metrics: RiseBugReportEnvelopeMetrics {
            active_processor_count: 8,
            available_disk_mb: -1,
            low_power_mode: false,
            physical_memory_mb: 16_384,
            process_uptime_seconds: 12,
            resident_memory_mb: 300,
            thermal_state: "nominal".to_owned(),
        },
        schema: RiseBugReportEnvelope::CURRENT_SCHEMA,
    }
}

#[test]
fn small_report_keeps_every_breadcrumb() {
    let text = envelope(3, "connected").encoded_payload().expect("encodes");
    assert!(text.contains(r#""droppedBreadcrumbs":0"#));
    assert!(text.contains(r#""residentMemoryMB":300"#));
    assert!(text.contains(r#""timeZone":"Asia\/Almaty""#));
}

#[test]
fn keys_are_sorted() {
    let text = envelope(0, "").encoded_payload().expect("encodes");
    let breadcrumbs = text.find(r#""breadcrumbs""#).expect("key");
    let captured = text.find(r#""capturedAtMs""#).expect("key");
    let schema = text.find(r#""schema""#).expect("key");
    assert!(breadcrumbs < captured && captured < schema);
}

#[test]
fn oversized_trail_is_trimmed_from_the_front_until_it_fits() {
    let message = "\u{0436}".repeat(200);
    let text = envelope(600, &message).encoded_payload().expect("encodes");
    assert!(text.chars().count() <= PAYLOAD_CEILING);
    assert!(!text.contains(r#""droppedBreadcrumbs":0"#));
    assert!(text.contains("1793886545599"));
}

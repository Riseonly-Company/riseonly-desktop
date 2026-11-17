use std::collections::HashMap;

use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::language_store::Locale;
use serde_json::{Map, Value, json};

use crate::core::push_notifications::admin_console_link::AdminConsoleLink;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::push_notification_presentation::{
    PushForegroundNotificationPresentation, PushNotificationRouteParser,
};
use crate::modules::notify::engine::core::rise_announcements_resource_repository::RiseAnnouncementsCacheSource;
use crate::modules::notify::engine::core::rise_notifications_wire_models::{
    RiseNotificationDTO, RiseNotificationDTOItem,
};
use crate::modules::notify::engine::rise_notifications_presentation::{
    NotifyFeedChrome, RiseNotificationRow, RiseNotificationsLoadState,
    RiseNotificationsPrepareSeed, RiseNotificationsSnapshotBuilder,
};
use crate::modules::notify::stores::notify_actions::notify_types::{
    NotificationItem, NotificationKind,
};

const STAGING_PAGE: &str = "https://admin.staging.riseonly.net/otp/providers";

fn url(raw: &str) -> Url {
    Url::from_string(raw).expect("test url parses")
}

fn payload(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("test payload is an object")
}

fn absolute(raw: &str) -> Option<String> {
    AdminConsoleLink::url(raw).map(|url| url.absolute_string().to_owned())
}

fn item_payload(item: &NotificationItem) -> Map<String, Value> {
    item.data
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect()
}

/// decodes a feed row as the gateway prints it, snake case and data as a json string
fn decode_feed_item(id: i64, title: &str, body: &str, data: Value) -> Option<NotificationItem> {
    let data_json = serde_json::to_string(&data).expect("test data encodes");
    let wire = json!({
        "id": id,
        "notification_type": "system",
        "title": title,
        "body": body,
        "data": data_json,
        "is_read": false,
        "created_at": "2026-10-06T09:00:00.000Z",
        "group_count": 1,
        "actors": [],
    });
    let dto: RiseNotificationDTO = serde_json::from_value(wire).expect("test row decodes");
    dto.to_notification_item()
}

fn rows(items: Vec<NotificationItem>) -> Vec<RiseNotificationRow> {
    let chrome = NotifyFeedChrome {
        strings: HashMap::from([(
            RiseNotificationsSnapshotBuilder::ADMIN_HINT_KEY.to_owned(),
            "Open the admin panel".to_owned(),
        )]),
        short_date_pattern: "d MMM".to_owned(),
        language_code: "en".to_owned(),
        locale: Locale::new("en_US_POSIX"),
    };
    let seed = RiseNotificationsPrepareSeed {
        items,
        has_more: false,
        is_loading_more: false,
        total_unread: 0,
        load_state: RiseNotificationsLoadState::Loaded,
        is_refreshing: false,
        cache_source: RiseAnnouncementsCacheSource::Network,
        revision: 1,
        has_newer: false,
        is_loading_newer: false,
        head_generation: 0,
    };
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = TimeZone::gmt();
    let snapshot = RiseNotificationsSnapshotBuilder::build(
        &seed,
        &chrome,
        Date::from_time_interval_since_1970(1_791_280_800.0),
        &calendar,
        &chrome.locale,
    );
    snapshot.sections.into_iter().flat_map(|section| section.rows).collect()
}

#[test]
fn production_and_environment_admin_pages_are_accepted() {
    let accepted = [
        "https://admin.riseonly.net/otp/providers",
        "https://admin.riseonly.net",
        STAGING_PAGE,
        "https://admin.dev-2.riseonly.net/reports/bugs?status=new#top",
    ];

    for raw in accepted {
        assert_eq!(absolute(raw).as_deref(), Some(raw), "{raw}");
    }
    assert_eq!(absolute(&format!("  {STAGING_PAGE}\n")).as_deref(), Some(STAGING_PAGE));
    assert!(AdminConsoleLink::url("HTTPS://Admin.Staging.RISEONLY.net/otp/providers").is_some());
}

#[test]
fn foreign_or_disguised_addresses_are_rejected() {
    let rejected = [
        "http://admin.riseonly.net/otp/providers",
        "https://riseonly.net/otp/providers",
        "https://staging.riseonly.net/otp/providers",
        "https://www.riseonly.net/otp/providers",
        "https://evil.com/otp/providers",
        "https://evil.com/#admin.riseonly.net",
        "https://admin.riseonly.net.evil.com/otp/providers",
        "https://admin.riseonly.com/otp/providers",
        "https://adminriseonly.net/",
        "https://xadmin.riseonly.net/",
        "https://admin-staging.riseonly.net/",
        "https://panel.staging.riseonly.net/",
        "https://admin.eu.staging.riseonly.net/",
        "https://admin..riseonly.net/",
        "https://admin.-staging.riseonly.net/",
        "https://admin.staging-.riseonly.net/",
        "https://admin.stag_ing.riseonly.net/",
        "https://admin.riseonly.net./",
        "https://admin.riseonly.net%2Eevil.com/",
        "https://user@admin.riseonly.net/otp/providers",
        "https://user:secret@admin.riseonly.net/otp/providers",
        "https://:secret@admin.riseonly.net/otp/providers",
        "https://admin.riseonly.net@evil.com/otp/providers",
        "https://admin.riseonly.net:8443/otp/providers",
        "https://admin.riseonly.net\\@evil.com/",
        "https://admin.riseonly.net/otp providers",
        "https://admin.riseonly.net/otp\tproviders",
        "https://admin.riseonly.net/\u{043E}\u{0442}\u{043F}",
        "riseonly://admin.riseonly.net/otp/providers",
        "javascript:alert(1)",
        "//admin.riseonly.net/otp/providers",
        "admin.riseonly.net/otp/providers",
        "https:///admin.riseonly.net/otp/providers",
        "",
        "   ",
    ];

    for raw in rejected {
        assert!(AdminConsoleLink::url(raw).is_none(), "{raw}");
    }
}

#[test]
fn admin_alert_push_opens_its_admin_page() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "admin_alert",
        "alert": "otp_balance_low",
        "admin_url": STAGING_PAGE,
        "account_name": "main",
        "provider": "mobizon",
        "balance": "1.2",
        "currency": "USD",
        "threshold": "5",
    })));

    assert_eq!(route, Some(PushDeepLink::AdminConsole { url: url(STAGING_PAGE) }));
}

#[test]
fn any_notification_naming_an_admin_page_opens_it() {
    let page = "https://admin.riseonly.net/reports/bugs/report-1";
    let bug_report = PushNotificationRouteParser::route(&payload(json!({
        "type": "bug_report_new",
        "report_id": "report-1",
        "admin_url": page,
    })));
    let routed_type = PushNotificationRouteParser::route(&payload(json!({
        "type": "new_message",
        "chat_id": "chat-1",
        "admin_url": page,
    })));

    let expected = Some(PushDeepLink::AdminConsole { url: url(page) });
    assert_eq!(bug_report, expected);
    assert_eq!(routed_type, expected);
}

#[test]
fn a_refused_admin_page_opens_nothing() {
    assert_eq!(
        PushNotificationRouteParser::route(&payload(json!({
            "type": "admin_alert",
            "alert": "otp_balance_low",
            "admin_url": "https://admin.riseonly.net.evil.com/otp/providers",
        }))),
        None
    );
    assert_eq!(
        PushNotificationRouteParser::route(&payload(json!({
            "type": "new_message",
            "chat_id": "chat-1",
            "admin_url": "http://admin.riseonly.net/otp/providers",
        }))),
        None
    );
}

#[test]
fn an_empty_admin_page_leaves_the_type_route() {
    let route = PushNotificationRouteParser::route(&payload(json!({
        "type": "new_message",
        "chat_id": "chat-1",
        "admin_url": "  ",
    })));

    assert_eq!(route, Some(PushDeepLink::Chat { chat_id: "chat-1".to_owned() }));
}

#[test]
fn foreground_admin_alert_keeps_server_copy_and_opens_the_admin_page() {
    let presentation = PushForegroundNotificationPresentation::make_with_active_screen(
        &payload(json!({
            "type": "admin_alert",
            "alert": "otp_balance_low",
            "admin_url": STAGING_PAGE,
        })),
        "SMS balance is low",
        "Mobizon, main: 1.2 USD, threshold 5 USD",
        None,
    )
    .expect("the alert has a presentation");

    assert_eq!(presentation.title, "SMS balance is low");
    assert_eq!(presentation.body.as_deref(), Some("Mobizon, main: 1.2 USD, threshold 5 USD"));
    assert_eq!(presentation.route, Some(PushDeepLink::AdminConsole { url: url(STAGING_PAGE) }));
}

#[test]
fn admin_alert_feed_row_shows_server_copy_under_a_system_tile() {
    let item = decode_feed_item(
        7,
        "SMS balance is low",
        "Mobizon, main: 1.2 USD, threshold 5 USD",
        json!({
            "type": "admin_alert",
            "alert": "otp_balance_low",
            "admin_url": STAGING_PAGE,
            "account_name": "main",
            "provider": "mobizon",
            "balance": 1.2,
            "currency": "USD",
            "threshold": 5,
        }),
    )
    .expect("the row decodes to an item");

    assert_eq!(item.kind, NotificationKind::System);
    assert!(item.is_admin_alert());
    assert_eq!(item.admin_url().as_deref(), Some(STAGING_PAGE));
    assert_eq!(item.data.get("alert").map(String::as_str), Some("otp_balance_low"));
    assert_eq!(item.data.get("balance").map(String::as_str), Some("1.2"));
    assert_eq!(item.data.get("threshold").map(String::as_str), Some("5"));
    assert_eq!(
        PushNotificationRouteParser::route(&item_payload(&item)),
        Some(PushDeepLink::AdminConsole { url: url(STAGING_PAGE) })
    );

    let rows = rows(vec![item]);
    let row = rows.first().expect("the item has a row");
    assert_eq!(row.leading, "SMS balance is low");
    assert_eq!(row.text, "Mobizon, main: 1.2 USD, threshold 5 USD");
    assert!(row.leading_is_heading);
    assert!(row.facepile.is_empty());
    assert_eq!(row.badge_symbol, "exclamationmark.triangle.fill");
    assert_eq!(row.accessibility_hint, "Open the admin panel");
}

#[test]
fn refused_admin_page_gets_no_hint_and_ordinary_system_rows_stay_as_before() {
    let forged = decode_feed_item(
        8,
        "SMS balance is low",
        "Mobizon, main: 1.2 USD, threshold 5 USD",
        json!({
            "type": "admin_alert",
            "admin_url": "https://admin.riseonly.net.evil.com/otp/providers",
        }),
    )
    .expect("the forged row decodes to an item");
    let ordinary = decode_feed_item(9, "Riseonly", "Something changed", json!({ "type": "maintenance" }))
        .expect("the ordinary row decodes to an item");

    let rows = rows(vec![forged.clone(), ordinary.clone()]);
    let forged_row = rows.iter().find(|row| row.id == forged.id).expect("the forged row is listed");
    let ordinary_row =
        rows.iter().find(|row| row.id == ordinary.id).expect("the ordinary row is listed");

    assert!(forged_row.leading_is_heading);
    assert!(forged_row.accessibility_hint.is_empty());
    assert_eq!(PushNotificationRouteParser::route(&item_payload(&forged)), None);

    assert!(!ordinary.is_admin_alert());
    assert_eq!(ordinary.badge_symbol(), "bell.fill");
    assert!(!ordinary_row.leading_is_heading);
    assert!(ordinary_row.leading.is_empty());
    assert_eq!(ordinary_row.text, "Something changed");
    assert!(ordinary_row.accessibility_hint.is_empty());
}

#[test]
fn security_incident_feed_row_is_an_admin_alert() {
    let item = decode_feed_item(
        10,
        "Activity spike - comments",
        "Comments per minute above the threshold",
        json!({
            "type": AdminConsoleLink::INCIDENT_TYPE,
            "rule": "activity_spike",
            "admin_path": "/security",
            "incident_id": "inc-1",
        }),
    )
    .expect("the incident row decodes to an item");

    assert!(item.is_admin_alert());
    assert_eq!(item.badge_symbol(), "exclamationmark.triangle.fill");

    let rows = rows(vec![item]);
    let row = rows.first().expect("the incident has a row");
    assert!(row.leading_is_heading);
    assert_eq!(row.leading, "Activity spike - comments");
    assert_eq!(row.text, "Comments per minute above the threshold");
    assert!(row.facepile.is_empty());
    assert_eq!(row.badge_symbol, "exclamationmark.triangle.fill");
}

use gpui::TestAppContext;
use rise_core::foundation::foundation_calendar::{Calendar, DateComponents, TimeZone};

use super::app_date_formatter::AppDateFormatter;
use super::language_store::{LanguageStore, Locale};

#[test]
fn date_formatter_uses_requested_language_instead_of_device_language() {
    let mut calendar = Calendar::gregorian();
    let Some(gmt) = TimeZone::with_seconds_from_gmt(0) else {
        panic!("a zero offset zone must exist");
    };
    calendar.time_zone = gmt;
    let Some(date) = calendar.date_from_components(&DateComponents::ymd(2025, 7, 1)) else {
        panic!("2025-07-01 must be a valid date");
    };

    let mut portuguese = AppDateFormatter::make(
        Some(Locale::new("pt_BR")),
        Some(calendar.clone()),
        Some(calendar.time_zone.clone()),
    );
    portuguese.set_date_format("LLLL");

    let mut russian = AppDateFormatter::make(
        Some(Locale::new("ru_RU")),
        Some(calendar.clone()),
        Some(calendar.time_zone.clone()),
    );
    russian.set_date_format("LLLL");

    assert_eq!(portuguese.string_from(&date), "julho");
    assert_eq!(russian.string_from(&date), "\u{0438}\u{044e}\u{043b}\u{044c}");
}

#[gpui::test]
fn app_and_environment_share_one_language_store(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let shared: *const LanguageStore = LanguageStore::shared(cx);
        let Some(active) = LanguageStore::active(cx) else {
            panic!("shared must install the active store");
        };
        assert!(std::ptr::eq(active, shared));
    });
}

#[test]
fn a_region_is_named_in_the_language_of_the_locale_and_its_script() {
    let andorra_in_russian = "\u{410}\u{43D}\u{434}\u{43E}\u{440}\u{440}\u{430}";
    assert_eq!(Locale::new("ru").localized_string_for_region_code("AD").as_deref(), Some(andorra_in_russian));
    assert_eq!(Locale::new("zh_Hant_TW").localized_string_for_region_code("AD").as_deref(), Some("\u{5B89}\u{9053}\u{723E}"));
    assert_eq!(Locale::new("zh-Hans").localized_string_for_region_code("AD").as_deref(), Some("\u{5B89}\u{9053}\u{5C14}"));
    assert_eq!(Locale::new("ru").localized_string_for_region_code("ZZ9"), None);
}

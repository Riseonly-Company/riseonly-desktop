pub(super) fn native_identifiers() -> Vec<String> {
    use objc2_foundation::NSLocale;

    let mut identifiers: Vec<String> = NSLocale::preferredLanguages()
        .iter()
        .map(|language| language.to_string())
        .collect();

    let current = NSLocale::currentLocale();
    identifiers.push(current.localeIdentifier().to_string());
    identifiers
}

pub(super) fn native_week_rules() -> Option<(usize, usize)> {
    use objc2_foundation::NSCalendar;

    let calendar = NSCalendar::currentCalendar();
    Some((calendar.firstWeekday(), calendar.minimumDaysInFirstWeek()))
}

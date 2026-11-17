use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::Locale;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubscriptionBillingDateKind {
    Renewal,
    Expiration,
}

/// why a plan receipt goes to the server. a plan held by another account gets offered premium
/// for this one,
/// a store update is left alone
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanVerifyOrigin {
    Purchase,
    HeldPlan,
    StoreUpdate,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionStatusModel {
    pub status: String,
    pub plan: Option<String>,
    pub billing_date: Option<Date>,
    pub billing_date_kind: SubscriptionBillingDateKind,
    pub auto_renew: bool,
    pub is_active: bool,
}

impl SubscriptionStatusModel {
    /// billing date formatted, locale defaults to the app locale and time zone to the current one
    pub fn formatted_billing_date(&self, locale: &Locale, time_zone: &TimeZone) -> Option<String> {
        let billing_date = self.billing_date?;

        let mut formatter = DateFormatter::new();
        formatter.set_locale(locale.identifier());
        formatter.set_time_zone(time_zone.clone());
        formatter.set_date_style(Style::Long);
        formatter.set_time_style(Style::None);
        Some(formatter.string_from(&billing_date))
    }
}

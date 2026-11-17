use rise_canon::RiseResponseJSON;
use rise_canon::rise_request::subscription::VerifyPurchase;
use rise_canon::rise_request::subscription::verify_purchase::Receipt;
use rise_canon::rise_view::subscription::{
    GiftNotice, PromoLink, PromoRedeemed, PurchaseVerified, PurchasesRestored, RestoredPurchase, SubscriptionStatus,
};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::subscription_types::{SubscriptionBillingDateKind, SubscriptionStatusModel};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IAPPurchaseReceiptBody {
    pub platform: String,
    pub receipt: String,
    pub transaction_id: String,
    pub product_id: String,
    pub recipient_user_id: Option<String>,
}

impl IAPPurchaseReceiptBody {
    pub fn new(
        platform: String,
        receipt: String,
        transaction_id: String,
        product_id: String,
        recipient_user_id: Option<String>,
    ) -> Self {
        Self { platform, receipt, transaction_id, product_id, recipient_user_id }
    }

    pub fn verify_request(&self) -> VerifyPurchase {
        VerifyPurchase {
            receipt: Some(Receipt {
                platform: Some(self.platform.clone()),
                product_id: Some(self.product_id.clone()),
                receipt: Some(self.receipt.clone()),
                transaction_id: Some(self.transaction_id.clone()),
                ..Default::default()
            }),
            recipient_user_id: self.recipient_user_id.clone().filter(|recipient| !recipient.is_empty()),
        }
    }
}

/// preview a promo link shows before claim. grant_days 0 with lifetime true is a lifetime
/// grant,
/// refusal_code names the first failed gate
pub type PremiumPromoLinkDTO = PromoLink;

pub type PremiumPromoRedeemDTO = PromoRedeemed;
/// a gift that turned premium on for this account, celebrated once
pub type PremiumGiftNoticeDTO = GiftNotice;
pub type SubscriptionStatusGatewayDTO = SubscriptionStatus;
pub type VerifyPurchaseGatewayDTO = PurchaseVerified;
pub type RestoredPurchaseGatewayDTO = RestoredPurchase;

pub type RestorePurchasesGatewayDTO = PurchasesRestored;

/// view keeps these three as open json,
/// read them as the scalars the server prints. a trait since the canon type is foreign
pub trait VerifyPurchaseGatewayScalars {
    fn active_flag(&self) -> Option<bool>;
    fn subscription_id_text(&self) -> Option<String>;
    fn expires_at_value(&self) -> Option<i64>;
}

impl VerifyPurchaseGatewayScalars for VerifyPurchaseGatewayDTO {
    fn active_flag(&self) -> Option<bool> {
        match &self.is_active {
            Some(RiseResponseJSON::Bool(value)) => Some(*value),
            _ => None,
        }
    }

    fn subscription_id_text(&self) -> Option<String> {
        match &self.subscription_id {
            Some(RiseResponseJSON::String(value)) => Some(value.clone()),
            Some(RiseResponseJSON::Number(value)) => value.as_f64().map(|number| (number as i64).to_string()),
            _ => None,
        }
    }

    fn expires_at_value(&self) -> Option<i64> {
        match &self.expires_at {
            Some(RiseResponseJSON::Number(value)) => value.as_f64().map(|number| number as i64),
            Some(RiseResponseJSON::String(value)) => value.parse::<i64>().ok(),
            _ => None,
        }
    }
}

/// converts the gateway status dto into the status model
pub trait SubscriptionStatusGatewayModel {
    /// now defaults to the current clock time
    fn as_subscription_status_model(&self, now: Date) -> SubscriptionStatusModel;
}

impl SubscriptionStatusGatewayModel for SubscriptionStatusGatewayDTO {
    fn as_subscription_status_model(&self, now: Date) -> SubscriptionStatusModel {
        let expiration_date = subscription_status_date(self.expires_at);
        let renewal_date = subscription_status_date(self.next_billing_date);
        let renews = self.auto_renew;
        let billing_date_candidates =
            if renews { [renewal_date, expiration_date] } else { [expiration_date, renewal_date] };
        let entitlement_dates: Vec<Date> = billing_date_candidates.into_iter().flatten().collect();
        let billing_date = entitlement_dates.iter().copied().find(|date| *date > now);
        let plan_label = self.product_id.clone();
        let trimmed_status = trimming_whitespaces_and_newlines(&self.status).to_lowercase();
        let normalized_status = if trimmed_status.is_empty() { None } else { Some(trimmed_status) };
        let server_active = self.is_active;
        let active = server_active && (entitlement_dates.is_empty() || billing_date.is_some());

        SubscriptionStatusModel {
            status: normalized_status.unwrap_or_else(|| if active { "active" } else { "inactive" }.to_owned()),
            plan: plan_label,
            billing_date: if active { billing_date } else { None },
            billing_date_kind: if renews {
                SubscriptionBillingDateKind::Renewal
            } else {
                SubscriptionBillingDateKind::Expiration
            },
            auto_renew: renews,
            is_active: active,
        }
    }
}

fn subscription_status_date(timestamp: Option<i64>) -> Option<Date> {
    let timestamp = timestamp.filter(|timestamp| *timestamp > 0)?;

    let seconds: TimeInterval =
        if timestamp >= 100_000_000_000 { timestamp as TimeInterval / 1_000.0 } else { timestamp as TimeInterval };
    let date = Date::from_time_interval_since_1970(seconds);
    let year = Calendar::gregorian().component(CalendarComponent::Year, &date);
    if !(2000..=2100).contains(&year) {
        return None;
    }

    Some(date)
}

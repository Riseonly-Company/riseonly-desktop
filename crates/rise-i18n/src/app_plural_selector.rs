use rise_i18n_catalog::{PluralCategory, PluralCount, category_for};

use crate::app_language_catalog::AppPluralRules;

pub struct AppPluralSelector;

impl AppPluralSelector {
    pub fn category(for_rules: AppPluralRules, count: i64) -> PluralCategory {
        category_for(for_rules, PluralCount::whole(count))
    }
}

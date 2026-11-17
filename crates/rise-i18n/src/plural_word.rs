use rise_i18n_catalog::PluralCategory;

use crate::i18n::I18n;

pub fn plural_word<'a>(n: i64, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    match I18n::shared().plural_category(n) {
        PluralCategory::One => one,
        PluralCategory::Few => few,
        PluralCategory::Zero | PluralCategory::Two | PluralCategory::Many | PluralCategory::Other => {
            many
        }
    }
}

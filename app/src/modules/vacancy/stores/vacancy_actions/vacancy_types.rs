use std::sync::LazyLock;

use rise_catalog::{VACANCY_CURRENCIES_DEFAULT, VacancyApplicationStatus, VacancyCurrency, VacancyReviewStatus};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{FormatArg, Locale, format_printf, plural_word, tr};
use rise_moderation::ContentStatus;
use rise_platform::text_capabilities::draws_text;

use crate::modules::vacancy::shared::resume_option_catalog::ResumeOptionCatalog;

const SECONDS_PER_DAY: f64 = 86_400.0;
const OPEN_ENDED_EXPERIENCE_YEARS: i64 = 99;
const DAY_MONTH_FORMAT: &str = "d MMM";
const DECIMAL_GROUPING_SEPARATOR: char = ' ';
const DECIMAL_GROUPING_SIZE: usize = 3;

fn decimal_grouped(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / DECIMAL_GROUPING_SIZE + 1);
    if value < 0 {
        grouped.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % DECIMAL_GROUPING_SIZE == 0 {
            grouped.push(DECIMAL_GROUPING_SEPARATOR);
        }
        grouped.push(digit);
    }
    grouped
}

fn whole_days_since(date: &Date) -> i64 {
    (Date::now().time_interval_since(date) / SECONDS_PER_DAY) as i64
}

fn day_month(date: &Date) -> String {
    let mut formatter = DateFormatter::new();
    formatter.set_date_format(DAY_MONTH_FORMAT);
    formatter.set_locale(Locale::app_locale().identifier().to_owned());
    formatter.string_from(date)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VacancyModel {
    pub id: i64,
    pub title: String,
    pub company: String,
    pub city: String,
    pub salary_range: Vec<i64>,
    pub experience: Vec<i64>,
    pub schedule: String,
    pub job_format: String,
    pub is_relocation_available: bool,
    pub skills: Vec<String>,
    pub description: String,
    pub company_logo: String,
    pub active: bool,
    pub author_id: String,
    pub created_at_ms: i64,
    pub is_applied: Option<bool>,
    pub moderation_status: String,
    pub review_status: String,
    pub salary_currency: String,
}

impl VacancyModel {
    pub fn default_moderation_status() -> String {
        ContentStatus::Active.wire().to_owned()
    }

    pub fn default_review_status() -> String {
        VacancyReviewStatus::Approved.wire().to_owned()
    }

    pub fn is_moderation_disabled(&self) -> bool {
        self.moderation_status == ContentStatus::Disabled.wire()
    }

    pub fn is_awaiting_review(&self) -> bool {
        self.review_status == VacancyReviewStatus::Pending.wire()
    }

    pub fn salary_from(&self) -> i64 {
        self.salary_range.first().copied().unwrap_or(0)
    }

    pub fn salary_to(&self) -> i64 {
        self.salary_range.get(1).copied().unwrap_or(0)
    }

    pub fn formatted_salary(&self) -> String {
        let salary_from = self.salary_from();
        let salary_to = self.salary_to();
        if salary_from == 0 && salary_to == 0 {
            return tr("vacancy_salary_negotiable");
        }
        let symbol = ResumeCurrency::symbol(&self.salary_currency);
        let fs = decimal_grouped(salary_from);
        let ts = decimal_grouped(salary_to);
        if salary_to == 0 {
            return format!("{} {} {}", tr("vacancy_salary_from_label"), fs, symbol);
        }
        if salary_from == 0 {
            return format!("{} {} {}", tr("vacancy_salary_to_label"), ts, symbol);
        }
        format!("{fs} - {ts} {symbol}")
    }

    pub fn schedule_display(&self) -> String {
        ResumeOptionCatalog::schedule_title(&self.schedule)
    }

    pub fn job_format_display(&self) -> String {
        ResumeOptionCatalog::job_format_title(&self.job_format)
    }

    pub fn experience_display(&self) -> String {
        let mn = self.experience.first().copied().unwrap_or(0);
        let mx = self.experience.get(1).copied().unwrap_or(0);
        if mn == 0 && mx == 0 {
            return tr("vacancy_exp_no");
        }
        if mx == 0 || mx >= OPEN_ENDED_EXPERIENCE_YEARS {
            return format!("{} {}", tr("vacancy_exp_from"), Self::years_after_preposition(mn));
        }
        if mn == 0 {
            return format!("{} {}", tr("vacancy_exp_up_to"), Self::years_after_preposition(mx));
        }
        let one = tr("age_year_one");
        let few = tr("age_year_few");
        let many = tr("age_year_many");
        let word = plural_word(mx, &one, &few, &many);
        let count = mx.to_string();
        format!("{}-{}", mn, format_printf(word, &[FormatArg::from(&count)]))
    }

    fn years_after_preposition(n: i64) -> String {
        let few = tr("age_year_few");
        let many = tr("age_year_many");
        let word = plural_word(n, &few, &many, &many);
        let count = n.to_string();
        format_printf(word, &[FormatArg::from(&count)])
    }

    pub fn published_at(&self) -> String {
        if self.created_at_ms <= 0 {
            return String::new();
        }
        let date = Date::from_ms(self.created_at_ms);
        let days = whole_days_since(&date);
        if days == 0 {
            return tr("vacancy_published_today");
        }
        if days == 1 {
            return tr("vacancy_published_yesterday");
        }
        if days < 7 {
            let word = Self::days_word(days);
            return format!("{} {} {}", days, word, tr("vacancy_published_ago"));
        }
        day_month(&date)
    }

    fn days_word(n: i64) -> String {
        if n % 10 == 1 && n % 100 != 11 {
            return tr("vacancy_day_1");
        }
        if [2, 3, 4].contains(&(n % 10)) && ![12, 13, 14].contains(&(n % 100)) {
            return tr("vacancy_day_2_4");
        }
        tr("vacancy_day_5")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResumeModel {
    pub id: i64,
    pub profession: String,
    pub first_name: String,
    pub second_name: String,
    pub last_name: String,
    pub city: String,
    pub experience_level: String,
    pub skills: Vec<String>,
    pub expected_salary_range: Vec<i64>,
    pub updated_at_ms: i64,
    pub work_experience: Vec<WorkExperienceModel>,
    pub professional_summary: String,
    pub ready_to_relocation: bool,
    pub ready_to_army: bool,
    pub gender: String,
    pub birth_date: String,
    pub education: String,
    pub phone: String,
    pub country: String,
    pub salary_currency: String,
    pub avatar: String,
}

impl ResumeModel {
    pub fn full_name(&self) -> String {
        [&self.first_name, &self.second_name, &self.last_name]
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn location_display(&self) -> String {
        [&self.city, &self.country]
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn currency_symbol(&self) -> &'static str {
        ResumeCurrency::symbol(&self.salary_currency)
    }

    pub fn updated_at(&self) -> String {
        if self.updated_at_ms <= 0 {
            return String::new();
        }
        let date = Date::from_ms(self.updated_at_ms);
        if whole_days_since(&date) == 0 {
            return tr("vacancy_resume_updated_today");
        }
        format!("{} {}", tr("vacancy_resume_updated"), day_month(&date))
    }

    pub fn formatted_salary(&self) -> String {
        let from = self.expected_salary_range.first().copied().unwrap_or(0);
        let to = self.expected_salary_range.get(1).copied().unwrap_or(0);
        if from == 0 && to == 0 {
            return tr("vacancy_salary_negotiable");
        }
        let sym = self.currency_symbol();
        let fs = decimal_grouped(from);
        let ts = decimal_grouped(to);
        if to == 0 {
            return format!("{} {} {}", tr("vacancy_salary_from_label"), fs, sym);
        }
        if from == 0 {
            return format!("{} {} {}", tr("vacancy_salary_to_label"), ts, sym);
        }
        format!("{fs} - {ts} {sym}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResumeCurrencyOption {
    pub code: &'static str,
    pub symbol: &'static str,
    pub name_key: &'static str,
}

impl ResumeCurrencyOption {
    pub fn id(&self) -> &'static str {
        self.code
    }
}

static RESUME_CURRENCY_SYMBOLS: LazyLock<Vec<String>> = LazyLock::new(|| {
    VacancyCurrency::ALL.into_iter().map(|currency| ResumeCurrency::display_symbol(currency.symbol(), currency.wire())).collect()
});

static RESUME_CURRENCIES: LazyLock<Vec<ResumeCurrencyOption>> = LazyLock::new(|| {
    let symbols: &'static [String] = LazyLock::force(&RESUME_CURRENCY_SYMBOLS).as_slice();
    VacancyCurrency::ALL
        .into_iter()
        .zip(symbols)
        .map(|(currency, symbol)| ResumeCurrencyOption {
            code: currency.wire(),
            symbol: symbol.as_str(),
            name_key: currency.i18n_key(),
        })
        .collect()
});

pub enum ResumeCurrency {}

impl ResumeCurrency {
    pub const DEFAULT_CODE: &'static str = VACANCY_CURRENCIES_DEFAULT.wire();

    pub fn all() -> &'static [ResumeCurrencyOption] {
        &RESUME_CURRENCIES
    }

    fn display_symbol(symbol: &str, code: &str) -> String {
        if !draws_text(symbol) {
            return code.to_string();
        }
        format!("\u{2068}{symbol}\u{2069}")
    }

    pub fn symbol(code: &str) -> &'static str {
        Self::all()
            .iter()
            .find(|option| option.code == code)
            .map_or(VACANCY_CURRENCIES_DEFAULT.symbol(), |option| option.symbol)
    }

    pub fn option(code: &str) -> ResumeCurrencyOption {
        Self::all()
            .iter()
            .find(|option| option.code == code)
            .or_else(|| Self::all().first())
            .copied()
            .unwrap_or(ResumeCurrencyOption {
                code: VACANCY_CURRENCIES_DEFAULT.wire(),
                symbol: VACANCY_CURRENCIES_DEFAULT.symbol(),
                name_key: VACANCY_CURRENCIES_DEFAULT.i18n_key(),
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkExperienceModel {
    pub id: Uuid,
    pub company: String,
    pub position: String,
    pub from_date: String,
    pub to_date: Option<String>,
    pub is_current: bool,
    pub description: String,
    pub company_logo_url: Option<String>,
}

impl WorkExperienceModel {
    pub fn new(
        company: String,
        position: String,
        from_date: String,
        to_date: Option<String>,
        is_current: bool,
        description: String,
        company_logo_url: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            company,
            position,
            from_date,
            to_date,
            is_current,
            description,
            company_logo_url,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ApplicationModel {
    pub id: i64,
    pub vacancy_id: i64,
    pub vacancy_title: String,
    pub company: String,
    pub status: String,
    pub created_at_ms: i64,
}

impl ApplicationModel {
    pub fn status_display(&self) -> String {
        match VacancyApplicationStatus::from_wire(&self.status) {
            Some(VacancyApplicationStatus::Pending) => tr("vacancy_app_status_pending"),
            Some(VacancyApplicationStatus::Viewed) => tr("vacancy_app_status_viewed"),
            Some(VacancyApplicationStatus::Accepted) => tr("vacancy_app_status_accepted"),
            Some(VacancyApplicationStatus::Rejected) => tr("vacancy_app_status_rejected"),
            None => self.status.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EmployerStatusModel {
    pub is_employer: bool,
    pub company_name: Option<String>,
    pub status: String,
}

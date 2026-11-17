use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    App, AsyncApp, Bounds, Entity, ExternalPaths, FocusHandle, FontWeight, Hsla, KeyDownEvent, Pixels, SharedString,
    Subscription, Window,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_calendar::{Calendar, DateComponents};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{LanguageStore, tr};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::date_picker::{DatePicker, DatePickerComponents};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::profile_field_formatting::PhoneNumberFormatting;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::input_ui::InputOutlinedRowUi;
use crate::core::ui::select_media_ui::SelectMediaUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::pages::gift_shop_sheet::GiftSheetScrollReader;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::user::pages::profile::settings::profile_settings::profile_avatar_picker_block::ProfileAvatarPickerBlock;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::vacancy::components::resume_form_ui::{
    GeoCityPickerSheet, GeoCountryPickerSheet, ResumeChipsField, ResumeFormGroup, ResumeInlineInputRow, ResumeToggleRow,
    ResumeValueRow, SearchablePickerSheet,
};
use crate::modules::vacancy::components::vacancy_form_support::{
    VacancyTextEditor, guide_owns_jobs_scene, hex_color, keeps_form_focus, pop_screen,
};
use crate::modules::vacancy::shared::geo_provider::GeoProvider;
use crate::modules::vacancy::shared::picker_option::PickerOption;
use crate::modules::vacancy::shared::resume_option_catalog::ResumeOptionCatalog;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeCurrency, ResumeModel};
use crate::modules::vacancy::stores::vacancy_actions::work_experience_entry::WorkExperienceEntry;
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

const RESUME_ENDPOINT: &str = "vacancy_create_resume";
static FIRST_NAME_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/first_name"));
static SECOND_NAME_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/second_name"));
static SUMMARY_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/professional_summary"));
static CITY_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/city"));
static COUNTRY_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/country"));
static SKILL_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/skills/*"));
static EXP_COMPANY_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/work_experience/*/company"));
static EXP_POSITION_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/work_experience/*/position"));
static EXP_DATE_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/work_experience/*/from_date"));
static EXP_DESCRIPTION_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(RESUME_ENDPOINT, "/work_experience/*/description"));

const DEFAULT_GENDER: &str = "male";
const DEFAULT_EDUCATION: &str = "high";
const DEFAULT_EXPERIENCE_LEVEL: &str = "1-3";
const PHONE_PLACEHOLDER: &str = "+7 777 777 77 77";
const SALARY_FROM_PLACEHOLDER: &str = "200 000";
const SALARY_TO_PLACEHOLDER: &str = "400 000";
const COMPANY_PLACEHOLDER: &str = "Google";
const POSITION_PLACEHOLDER: &str = "iOS Developer";
const POSIX_LOCALE: &str = "en_US_POSIX";
const WIRE_DATE_FORMAT: &str = "yyyy-MM-dd";
const DISPLAY_DATE_FORMAT: &str = "dd.MM.yyyy";
const WIRE_DATE_LENGTH: usize = 10;

type ResumeAreaBounds = Rc<RefCell<HashMap<ResumeTextArea, Bounds<Pixels>>>>;
type ResumeSectionBounds = Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>;

const MISSING_REVEAL_ANCHOR: f32 = 0.5;
const PERSONAL_SECTION_ID: &str = "resume_personal_section";
const LOCATION_SECTION_ID: &str = "resume_location_section";

/// which required rows the form marks red after a press with gaps
#[derive(Clone, Copy, Default)]
struct ResumeMissingMarks {
    first_name: bool,
    second_name: bool,
    phone: bool,
    profession: bool,
    birth_date: bool,
    country: bool,
    city: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ResumePicker {
    Gender,
    Country,
    City,
    Currency,
    Education,
    Experience,
    BirthDate,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ResumeTextArea {
    Summary,
    Experience(Uuid),
}

impl ResumeTextArea {
    fn key(self) -> String {
        match self {
            Self::Summary => "summary".to_owned(),
            Self::Experience(id) => id.uuid_string(),
        }
    }
}

fn trimmed(value: &str) -> String {
    trimming_whitespaces_and_newlines(value).to_owned()
}

fn birth_date_picker_start() -> Date {
    Calendar::current().date_from_components(&DateComponents::ymd(2000, 1, 1)).unwrap_or_else(Date::now)
}

struct CreateResumeState {
    first_name: String,
    second_name: String,
    phone: String,
    profession: String,
    city: String,
    country: String,
    birth_date: Option<Date>,
    gender: String,
    education: String,
    experience_level: String,
    salary_from: String,
    salary_to: String,
    salary_currency: String,
    skills: Vec<String>,
    summary: String,
    ready_to_relocation: bool,
    ready_to_army: bool,
    experiences: Vec<WorkExperienceEntry>,
    focused_area: Option<ResumeTextArea>,
    area_bounds: ResumeAreaBounds,
    active_picker: Option<ResumePicker>,
    mounted_picker: Option<ResumePicker>,
    did_hydrate: bool,
    show_missing: bool,
    missing_reveal_tick: u64,
    section_bounds: ResumeSectionBounds,
    editing_resume_id: Option<i64>,
    editing_account_id: Option<String>,
    editing_account_epoch: Option<String>,
    focus_handle: FocusHandle,
    _subscriptions: [Subscription; 5],
}

impl CreateResumeState {
    fn new(cx: &mut Context<Self>) -> Self {
        let service = VacancyServicesStore::shared(cx);
        let geo = GeoProvider::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            first_name: String::new(),
            second_name: String::new(),
            phone: String::new(),
            profession: String::new(),
            city: String::new(),
            country: String::new(),
            birth_date: None,
            gender: DEFAULT_GENDER.to_owned(),
            education: DEFAULT_EDUCATION.to_owned(),
            experience_level: DEFAULT_EXPERIENCE_LEVEL.to_owned(),
            salary_from: String::new(),
            salary_to: String::new(),
            salary_currency: ResumeCurrency::DEFAULT_CODE.to_owned(),
            skills: Vec::new(),
            summary: String::new(),
            ready_to_relocation: false,
            ready_to_army: false,
            experiences: Vec::new(),
            focused_area: None,
            area_bounds: Rc::new(RefCell::new(HashMap::new())),
            active_picker: None,
            mounted_picker: None,
            did_hydrate: false,
            show_missing: false,
            missing_reveal_tick: 0,
            section_bounds: Rc::new(RefCell::new(HashMap::new())),
            editing_resume_id: None,
            editing_account_id: None,
            editing_account_epoch: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: [
                cx.observe(&service, |_, _, cx| cx.notify()),
                cx.observe(&geo, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
                cx.observe_global::<LanguageStore>(|_, cx| cx.notify()),
            ],
        }
    }

    fn is_editing(&self) -> bool {
        self.editing_resume_id.is_some()
    }

    fn missing_birth_date(&self) -> bool {
        self.birth_date.is_none()
    }

    fn missing_personal(&self) -> bool {
        trimming_whitespaces_and_newlines(&self.first_name).is_empty()
            || trimming_whitespaces_and_newlines(&self.second_name).is_empty()
            || trimming_whitespaces_and_newlines(&self.phone).is_empty()
            || trimming_whitespaces_and_newlines(&self.profession).is_empty()
            || self.missing_birth_date()
    }

    fn is_form_complete(&self) -> bool {
        !self.missing_personal() && !trimming_whitespaces_and_newlines(&self.city).is_empty()
    }

    fn can_press(&self, cx: &App) -> bool {
        let is_submitting = VacancyServicesStore::try_read(cx).is_some_and(|service| service.is_submitting_resume());
        !is_submitting
            && (!self.is_editing()
                || guide_owns_jobs_scene(cx)
                || SaiAccountCacheScope::is_current_account_epoch(
                    self.editing_account_id.as_deref(),
                    self.editing_account_epoch.as_deref(),
                ))
    }

    fn can_submit(&self, cx: &App) -> bool {
        self.is_form_complete() && self.can_press(cx)
    }

    fn is_missing(&self, value: &str) -> bool {
        self.show_missing && trimming_whitespaces_and_newlines(value).is_empty()
    }

    fn missing_marks(&self) -> ResumeMissingMarks {
        ResumeMissingMarks {
            first_name: self.is_missing(&self.first_name),
            second_name: self.is_missing(&self.second_name),
            phone: self.is_missing(&self.phone),
            profession: self.is_missing(&self.profession),
            birth_date: self.show_missing && self.missing_birth_date(),
            country: self.is_missing(&self.city) && self.country.is_empty(),
            city: self.is_missing(&self.city),
        }
    }

    fn is_guide_form_ready(&self, cx: &App) -> bool {
        self.can_submit(cx) && !trimming_whitespaces_and_newlines(&self.country).is_empty() && self.active_picker.is_none()
    }

    fn focused_area_text(&self) -> String {
        match self.focused_area {
            Some(ResumeTextArea::Summary) => self.summary.clone(),
            Some(ResumeTextArea::Experience(id)) => {
                self.experiences.iter().find(|entry| entry.id == id).map(|entry| entry.job_description.clone()).unwrap_or_default()
            }
            None => String::new(),
        }
    }

    fn formatted_birth_date(&self) -> String {
        let Some(birth_date) = &self.birth_date else {
            return String::new();
        };
        let mut f = DateFormatter::new();
        f.set_date_format(DISPLAY_DATE_FORMAT);
        f.string_from(birth_date)
    }

    fn entry_mut(&mut self, id: Uuid) -> Option<&mut WorkExperienceEntry> {
        self.experiences.iter_mut().find(|entry| entry.id == id)
    }

    fn apply_resume(&mut self, resume: ResumeModel) {
        self.editing_resume_id = Some(resume.id);
        if let Some(d) = Self::parse_wire_date(&resume.birth_date) {
            self.birth_date = Some(d);
        }
        if let Some(from) = resume.expected_salary_range.first().filter(|from| **from > 0) {
            self.salary_from = from.to_string();
        }
        if let Some(to) = resume.expected_salary_range.get(1).filter(|to| **to > 0) {
            self.salary_to = to.to_string();
        }
        self.first_name = resume.first_name;
        self.second_name = resume.second_name;
        self.phone = resume.phone;
        self.profession = resume.profession;
        self.city = resume.city;
        self.country = resume.country;
        self.gender = if resume.gender.is_empty() { DEFAULT_GENDER.to_owned() } else { resume.gender };
        self.education = if resume.education.is_empty() { DEFAULT_EDUCATION.to_owned() } else { resume.education };
        self.experience_level =
            if resume.experience_level.is_empty() { DEFAULT_EXPERIENCE_LEVEL.to_owned() } else { resume.experience_level };
        self.salary_currency =
            if resume.salary_currency.is_empty() { ResumeCurrency::DEFAULT_CODE.to_owned() } else { resume.salary_currency };
        self.skills = resume.skills;
        self.summary = resume.professional_summary;
        self.ready_to_relocation = resume.ready_to_relocation;
        self.ready_to_army = resume.ready_to_army;
        self.experiences = resume
            .work_experience
            .into_iter()
            .map(|experience| WorkExperienceEntry {
                company: experience.company,
                position: experience.position,
                from_date: experience.from_date,
                to_date: experience.to_date.unwrap_or_default(),
                is_current: experience.is_current,
                job_description: experience.description,
                company_logo_url: experience.company_logo_url,
                ..WorkExperienceEntry::default()
            })
            .collect();
    }

    fn parse_wire_date(raw: &str) -> Option<Date> {
        if raw.is_empty() {
            return None;
        }
        let mut f = DateFormatter::new();
        f.set_locale(POSIX_LOCALE);
        f.set_date_format(WIRE_DATE_FORMAT);
        f.date_from(character_prefix(raw, WIRE_DATE_LENGTH))
    }
}

#[derive(IntoElement, Default)]
pub struct CreateResume;

impl CreateResume {
    pub fn new() -> Self {
        Self
    }

    fn string_binding(
        state: &Entity<CreateResumeState>,
        get: fn(&CreateResumeState) -> String,
        set: fn(&mut CreateResumeState, String),
    ) -> Binding<String> {
        Binding::entity(state.clone(), get, move |state, value, cx| {
            set(state, value);
            cx.notify();
        })
    }

    fn bool_binding(
        state: &Entity<CreateResumeState>,
        get: fn(&CreateResumeState) -> bool,
        set: fn(&mut CreateResumeState, bool),
    ) -> Binding<bool> {
        Binding::entity(state.clone(), get, move |state, value, cx| {
            set(state, value);
            cx.notify();
        })
    }

    /// binding to one entry by id, a row whose entry is gone reads the fallback and writes nowhere
    fn entry_binding<T: Clone + 'static>(
        state: &Entity<CreateResumeState>,
        id: Uuid,
        fallback: T,
        get: fn(&WorkExperienceEntry) -> T,
        set: fn(&mut WorkExperienceEntry, T),
    ) -> Binding<T> {
        Binding::entity(
            state.clone(),
            move |form: &CreateResumeState| {
                form.experiences.iter().find(|entry| entry.id == id).map_or_else(|| fallback.clone(), get)
            },
            move |form: &mut CreateResumeState, value: T, cx| {
                if let Some(entry) = form.entry_mut(id) {
                    set(entry, value);
                    cx.notify();
                }
            },
        )
    }

    fn open_picker(state: &Entity<CreateResumeState>, picker: ResumePicker, cx: &mut App) {
        state.update(cx, |form, cx| {
            form.active_picker = Some(picker);
            form.mounted_picker = Some(picker);
            cx.notify();
        });
    }

    fn avatar_header(theme: &AppTheme, cx: &App) -> Div {
        const SIDE: f32 = 92.0;
        let avatar_url = AuthServicesStore::try_read(cx)
            .and_then(|auth| auth.profile())
            .and_then(|profile| profile.avatar_url.clone())
            .filter(|url| !url.is_empty());
        let placeholder = div()
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("person.fill", px(34.0), theme.text.secondary, true));
        vstack(10.0)
            .w_full()
            .items_center()
            .child(
                div()
                    .relative()
                    .w(px(SIDE))
                    .h(px(SIDE))
                    .rounded_full()
                    .overflow_hidden()
                    .bg(theme.bg._300)
                    .child(placeholder)
                    .when_some(avatar_url, |circle, url| {
                        circle.child(
                            div().absolute().size_full().child(
                                CleverImageUi::new(url)
                                    .corner_radius(SIDE / 2.0)
                                    .shows_progress_indicator(false)
                                    .shows_failure_icon(false),
                            ),
                        )
                    }),
            )
            .child(SecondaryText::new(tr("vacancy_resume_photo_from_profile")).px(12.0).tac(TextAlignT::Center))
    }

    fn input_row(
        id: &'static str,
        label: String,
        text: Binding<String>,
        placeholder: String,
        icon: &str,
        icon_background: u32,
        is_error: bool,
    ) -> AnyElement {
        keeps_form_focus(
            ResumeInlineInputRow::new(id, label, text)
                .placeholder(placeholder)
                .icon(icon.to_owned())
                .icon_background(hex_color(icon_background))
                .is_error(is_error),
        )
        .into_any_element()
    }

    fn value_row(
        id: &'static str,
        label: String,
        icon: &str,
        icon_background: u32,
        value: String,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> ResumeValueRow {
        ResumeValueRow::new(id, label, on_tap).value(value).icon(icon.to_owned()).icon_background(hex_color(icon_background))
    }

    fn identity_section(
        state: &Entity<CreateResumeState>,
        gender: &str,
        birth_date: String,
        marks: ResumeMissingMarks,
    ) -> ResumeFormGroup {
        let gender_state = state.clone();
        let birth_state = state.clone();
        ResumeFormGroup::new(vec![
            Self::input_row(
                "create-resume-first-name",
                tr("vacancy_resume_first_name"),
                FIRST_NAME_RULE.binding(Self::string_binding(state, |form| form.first_name.clone(), |form, value| form.first_name = value)),
                tr("vacancy_resume_first_name_placeholder"),
                "person.fill",
                0x008DE5,
                marks.first_name,
            ),
            Self::input_row(
                "create-resume-second-name",
                tr("vacancy_resume_last_name"),
                SECOND_NAME_RULE.binding(Self::string_binding(state, |form| form.second_name.clone(), |form, value| form.second_name = value)),
                tr("vacancy_resume_last_name_placeholder"),
                "person.text.rectangle.fill",
                0x00B8D9,
                marks.second_name,
            ),
            Self::input_row(
                "create-resume-phone",
                tr("vacancy_resume_phone"),
                Self::string_binding(state, |form| form.phone.clone(), |form, value| form.phone = value),
                PHONE_PLACEHOLDER.to_owned(),
                "phone.fill",
                0x00C781,
                marks.phone,
            ),
            Self::input_row(
                "create-resume-profession",
                tr("vacancy_resume_profession"),
                Self::string_binding(state, |form| form.profession.clone(), |form, value| form.profession = value),
                tr("vacancy_resume_profession_placeholder"),
                "briefcase.fill",
                0x7C5CFC,
                marks.profession,
            ),
            Self::value_row(
                "create-resume-gender",
                tr("vacancy_resume_gender"),
                "figure.stand",
                0xF19A1A,
                tr(ResumeOptionCatalog::gender_key(gender)),
                move |_, cx| Self::open_picker(&gender_state, ResumePicker::Gender, cx),
            )
            .into_any_element(),
            Self::value_row(
                "create-resume-birth-date",
                tr("vacancy_resume_birth_date"),
                "calendar",
                0xFF5C8A,
                birth_date,
                move |_, cx| Self::open_picker(&birth_state, ResumePicker::BirthDate, cx),
            )
            .placeholder(tr("vacancy_resume_select"))
            .is_error(marks.birth_date)
            .into_any_element(),
        ])
        .title(tr("vacancy_resume_section_personal"))
    }

    fn location_section(
        state: &Entity<CreateResumeState>,
        country_title: String,
        country_flag: String,
        city: String,
        country: String,
        marks: ResumeMissingMarks,
    ) -> ResumeFormGroup {
        let country_state = state.clone();
        let city_state = state.clone();
        ResumeFormGroup::new(vec![
            Self::value_row(
                "create-resume-country",
                tr("vacancy_resume_country"),
                "globe",
                0x008DE5,
                country_title,
                move |_, cx| Self::open_picker(&country_state, ResumePicker::Country, cx),
            )
            .placeholder(tr("vacancy_resume_select"))
            .leading_emoji(country_flag)
            .is_error(marks.country)
            .into_any_element(),
            Self::value_row(
                "create-resume-city",
                tr("vacancy_field_city"),
                "building.2.fill",
                0x00B8D9,
                city,
                move |_, cx| {
                    if !country.is_empty() {
                        GeoProvider::shared(cx).update(cx, |geo, cx| geo.ensure_cities_loaded(&country, cx));
                    }
                    Self::open_picker(&city_state, ResumePicker::City, cx);
                },
            )
            .placeholder(tr("vacancy_resume_select"))
            .is_error(marks.city)
            .into_any_element(),
        ])
        .title(tr("vacancy_resume_section_location"))
    }

    fn salary_section(state: &Entity<CreateResumeState>, salary_currency: &str) -> ResumeFormGroup {
        let currency_state = state.clone();
        ResumeFormGroup::new(vec![
            Self::value_row(
                "create-resume-currency",
                tr("vacancy_resume_currency"),
                "creditcard.fill",
                0x00C781,
                format!("{}  {}", ResumeCurrency::symbol(salary_currency), salary_currency),
                move |_, cx| Self::open_picker(&currency_state, ResumePicker::Currency, cx),
            )
            .into_any_element(),
            Self::input_row(
                "create-resume-salary-from",
                tr("vacancy_salary_from"),
                Self::string_binding(state, |form| form.salary_from.clone(), |form, value| form.salary_from = value),
                SALARY_FROM_PLACEHOLDER.to_owned(),
                "arrow.down.to.line",
                0x008DE5,
                false,
            ),
            Self::input_row(
                "create-resume-salary-to",
                tr("vacancy_salary_to"),
                Self::string_binding(state, |form| form.salary_to.clone(), |form, value| form.salary_to = value),
                SALARY_TO_PLACEHOLDER.to_owned(),
                "arrow.up.to.line",
                0xFF5C8A,
                false,
            ),
        ])
        .title(tr("vacancy_resume_section_salary"))
        .footer(tr("vacancy_salary_optional_hint"))
    }

    fn details_section(state: &Entity<CreateResumeState>, education: &str, experience_level: &str) -> ResumeFormGroup {
        let education_state = state.clone();
        let experience_state = state.clone();
        ResumeFormGroup::new(vec![
            Self::value_row(
                "create-resume-education",
                tr("vacancy_resume_education"),
                "graduationcap.fill",
                0x7C5CFC,
                tr(ResumeOptionCatalog::education_key(education)),
                move |_, cx| Self::open_picker(&education_state, ResumePicker::Education, cx),
            )
            .into_any_element(),
            Self::value_row(
                "create-resume-experience-level",
                tr("vacancy_resume_exp_level"),
                "chart.bar.fill",
                0xF19A1A,
                tr(ResumeOptionCatalog::experience_key(experience_level)),
                move |_, cx| Self::open_picker(&experience_state, ResumePicker::Experience, cx),
            )
            .into_any_element(),
        ])
        .title(tr("vacancy_resume_section_education"))
    }

    fn section_title(title: String, theme: &AppTheme) -> Div {
        div().px(px(28.0)).child(MainText::new(title.to_uppercase()).px(12.0).color(theme.text.secondary))
    }

    fn skills_section(state: &Entity<CreateResumeState>, theme: &AppTheme) -> Div {
        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(Self::section_title(tr("vacancy_field_skills"), theme))
            .child(keeps_form_focus(
                ResumeChipsField::new(
                    "create-resume-skills",
                    Binding::entity(
                        state.clone(),
                        |form: &CreateResumeState| form.skills.clone(),
                        |form: &mut CreateResumeState, value, cx| {
                            form.skills = value;
                            cx.notify();
                        },
                    ),
                    tr("vacancy_field_skills_placeholder"),
                )
                .rule(*SKILL_RULE),
            ))
    }

    /// text area, focus writes the area and measured bounds stand in for the scroll lookup by id
    fn text_area(
        state: &Entity<CreateResumeState>,
        area: ResumeTextArea,
        text: Binding<String>,
        font_size: f32,
        min_height: f32,
        placeholder: Option<(String, Hsla)>,
        area_bounds: &ResumeAreaBounds,
    ) -> Div {
        let key = area.key();
        let focus_state = state.clone();
        let mut editor = VacancyTextEditor::new(SharedString::from(format!("create-resume-area-{key}")), text)
            .font_size(font_size)
            .min_height(min_height)
            .on_focus_change(move |focused, _, cx| {
                focus_state.update(cx, |form, cx| {
                    if focused {
                        form.focused_area = Some(area);
                    } else if form.focused_area == Some(area) {
                        form.focused_area = None;
                    }
                    cx.notify();
                });
            });
        if let Some((placeholder, color)) = placeholder {
            editor = editor.placeholder(placeholder, color);
        }
        let area_bounds = area_bounds.clone();
        div().relative().w_full().child(editor).child(
            measure(SharedString::from(format!("create-resume-area-bounds-{key}")), move |bounds, _, _| {
                area_bounds.borrow_mut().insert(area, bounds);
            })
            .absolute()
            .size_full(),
        )
    }

    fn summary_section(state: &Entity<CreateResumeState>, area_bounds: &ResumeAreaBounds, theme: &AppTheme) -> Div {
        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(Self::section_title(tr("vacancy_resume_section_summary"), theme))
            .child(
                div().w_full().px(px(16.0)).child(
                    div().w_full().p(px(12.0)).bg(theme.bg._200).text_color(theme.text.primary).continuous_corners(18.0).child(
                        Self::text_area(
                            state,
                            ResumeTextArea::Summary,
                            SUMMARY_RULE.binding(Self::string_binding(state, |form| form.summary.clone(), |form, value| form.summary = value)),
                            15.0,
                            120.0,
                            Some((tr("vacancy_resume_summary_placeholder"), theme.text.secondary.opacity(0.55))),
                            area_bounds,
                        ),
                    ),
                ),
            )
    }

    fn inline_field(id: SharedString, label: String, value: Binding<String>, placeholder: String) -> Div {
        vstack(4.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SecondaryText::new(label).px(11.0))
            .child(keeps_form_focus(InputOutlinedRowUi::new(id, value, placeholder, div())))
    }

    fn receive_dropped_logo(state: &Entity<CreateResumeState>, entry_id: Uuid, paths: Vec<PathBuf>, cx: &mut App) {
        let Some((path, (url, filename, content_type))) =
            paths.into_iter().find_map(|path| ProfileAvatarPickerBlock::local_image_file(&path).map(|file| (path, file)))
        else {
            return;
        };
        state.update(cx, |form, cx| {
            if let Some(entry) = form.entry_mut(entry_id) {
                entry.logo_file = Some(SaiWsOptimisticLocalFile::new(url, filename, content_type));
                cx.notify();
            }
        });
        let weak = state.downgrade();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let decoded = executor
                .spawn(async move { std::fs::read(path).ok().and_then(|data| CleverImageBitmapDecoder::decode(&data, None)) })
                .await;
            if let Some(image) = decoded {
                let _ = weak.update(cx, |form, cx| {
                    if let Some(entry) = form.entry_mut(entry_id) {
                        entry.logo_preview = Some(image);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn upload_logo(state: &Entity<CreateResumeState>, entry_id: Uuid, cx: &mut App) {
        let file = state.update(cx, |form, cx| {
            let entry = form.entry_mut(entry_id)?;
            let file = entry.logo_file.clone()?;
            entry.logo_uploading = true;
            entry.logo_progress = 0.0;
            cx.notify();
            Some(file)
        });
        let Some(file) = file else {
            return;
        };
        let progress_state = state.downgrade();
        let upload = VacancyInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.upload_resume_logo(
                file,
                Some(Rc::new(move |progress: f64, cx: &mut App| {
                    let _ = progress_state.update(cx, |form, cx| {
                        if let Some(entry) = form.entry_mut(entry_id) {
                            entry.logo_progress = progress;
                            cx.notify();
                        }
                    });
                })),
                cx,
            )
        });
        let weak = state.downgrade();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let url = upload.await;
            let _ = weak.update(cx, |form, cx| {
                if let Some(entry) = form.entry_mut(entry_id) {
                    entry.logo_uploading = false;
                    if url.is_none() {
                        entry.logo_preview = None;
                        entry.logo_file = None;
                    }
                    entry.company_logo_url = url;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn experience_card(
        state: &Entity<CreateResumeState>,
        entry: &WorkExperienceEntry,
        area_bounds: &ResumeAreaBounds,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let entry_id = entry.id;
        let key = entry_id.uuid_string();
        let child_id = |name: &str| SharedString::from(format!("create-resume-exp-{key}-{name}"));

        let logo_state = state.clone();
        on_change(
            child_id("logo-url"),
            entry.logo_file.as_ref().map(|file| file.url.clone()),
            false,
            window,
            cx,
            move |_, new_url, _, cx| {
                if new_url.is_some() {
                    Self::upload_logo(&logo_state, entry_id, cx);
                }
            },
        );

        let logo_file = Self::entry_binding::<Option<SaiWsOptimisticLocalFile>>(
            state,
            entry_id,
            None,
            |entry| entry.logo_file.clone(),
            |entry, value| entry.logo_file = value,
        );
        let logo_preview = Self::entry_binding::<Option<CleverImage>>(
            state,
            entry_id,
            None,
            |entry| entry.logo_preview.clone(),
            |entry, value| entry.logo_preview = value,
        );
        let drop_state = state.clone();
        let remove_state = state.clone();

        let logo_row = hstack(12.0)
            .w_full()
            .child(
                div()
                    .id(child_id("logo-drop"))
                    .flex_none()
                    .rounded_full()
                    .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(0.6))
                    .on_drop(move |paths: &ExternalPaths, _, cx| {
                        Self::receive_dropped_logo(&drop_state, entry_id, paths.paths().to_vec(), cx);
                    })
                    .child(
                        SelectMediaUi::new(child_id("logo"), logo_file, logo_preview)
                            .size(48.0)
                            .placeholder_system_image("building.2.fill")
                            .cover_upload_progress(entry.logo_progress)
                            .show_upload_overlay(entry.logo_uploading)
                            .picker_configuration(MediaPickerConfiguration::company_logo())
                            .uses_app_sheet_presentation(true),
                    ),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(tr("vacancy_experience_logo")).px(13.0).color(theme.text.primary))
                    .child(SecondaryText::new(tr("vacancy_experience_logo_hint")).px(11.0)),
            )
            .child(spacer())
            .child(
                plain(child_id("remove"))
                    .on_press(move |_, cx| {
                        remove_state.update(cx, |form, cx| {
                            form.experiences.retain(|entry| entry.id != entry_id);
                            cx.notify();
                        });
                    })
                    .children(IconUi::sized("trash", px(15.0), theme.semantic.error_100, false)),
            );

        let dates = hstack(12.0)
            .w_full()
            .child(div().flex_1().min_w(px(0.0)).child(Self::inline_field(
                child_id("from"),
                tr("vacancy_experience_from"),
                EXP_DATE_RULE.binding(Self::entry_binding(
                    state,
                    entry_id,
                    String::new(),
                    |entry| entry.from_date.clone(),
                    |entry, value| entry.from_date = value,
                )),
                tr("vacancy_experience_from_placeholder"),
            )))
            .when(!entry.is_current, |row| {
                row.child(div().flex_1().min_w(px(0.0)).child(Self::inline_field(
                    child_id("to"),
                    tr("vacancy_experience_to"),
                    EXP_DATE_RULE.binding(Self::entry_binding(
                        state,
                        entry_id,
                        String::new(),
                        |entry| entry.to_date.clone(),
                        |entry, value| entry.to_date = value,
                    )),
                    tr("vacancy_experience_to_placeholder"),
                )))
            });

        div().w_full().px(px(16.0)).child(
            vstack(14.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .p(px(14.0))
                .bg(theme.bg._200)
                .continuous_corners(18.0)
                .child(logo_row)
                .child(Self::inline_field(
                    child_id("company"),
                    tr("vacancy_experience_company"),
                    EXP_COMPANY_RULE.binding(Self::entry_binding(
                        state,
                        entry_id,
                        String::new(),
                        |entry| entry.company.clone(),
                        |entry, value| entry.company = value,
                    )),
                    COMPANY_PLACEHOLDER.to_owned(),
                ))
                .child(Self::inline_field(
                    child_id("position"),
                    tr("vacancy_experience_position"),
                    EXP_POSITION_RULE.binding(Self::entry_binding(
                        state,
                        entry_id,
                        String::new(),
                        |entry| entry.position.clone(),
                        |entry, value| entry.position = value,
                    )),
                    POSITION_PLACEHOLDER.to_owned(),
                ))
                .child(dates)
                .child(
                    hstack(0.0)
                        .w_full()
                        .child(MainText::new(tr("vacancy_experience_current")).px(14.0).color(theme.text.primary))
                        .child(spacer())
                        .child(SwitchUi::new(
                            child_id("current"),
                            Self::entry_binding(state, entry_id, false, |entry| entry.is_current, |entry, value| entry.is_current = value),
                        )),
                )
                .child(
                    vstack(6.0)
                        .align(HorizontalAlignment::Leading)
                        .w_full()
                        .child(SecondaryText::new(tr("vacancy_experience_description")).px(12.0))
                        .child(
                            div().w_full().p(px(8.0)).bg(theme.bg._300).text_color(theme.text.primary).continuous_corners(12.0).child(
                                Self::text_area(
                                    state,
                                    ResumeTextArea::Experience(entry_id),
                                    EXP_DESCRIPTION_RULE.binding(Self::entry_binding(
                                        state,
                                        entry_id,
                                        String::new(),
                                        |entry| entry.job_description.clone(),
                                        |entry, value| entry.job_description = value,
                                    )),
                                    14.0,
                                    72.0,
                                    None,
                                    area_bounds,
                                ),
                            ),
                        ),
                ),
        )
    }

    fn experience_section(
        state: &Entity<CreateResumeState>,
        experiences: &[WorkExperienceEntry],
        area_bounds: &ResumeAreaBounds,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let accent = theme.primary._100;
        let add_state = state.clone();
        let mut column = vstack(12.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(Self::section_title(tr("vacancy_experience_section"), theme));
        for entry in experiences {
            column = column.child(Self::experience_card(state, entry, area_bounds, theme, window, cx));
        }
        column.child(
            div().w_full().px(px(16.0)).child(
                plain("create-resume-experience-add")
                    .w_full()
                    .on_press(move |_, cx| {
                        add_state.update(cx, |form, cx| {
                            form.experiences.push(WorkExperienceEntry::default());
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .child(
                                hstack(8.0)
                                    .w_full()
                                    .justify_center()
                                    .py(px(14.0))
                                    .bg(accent.opacity(0.08))
                                    .capsule()
                                    .children(IconUi::sized("plus.circle.fill", px(16.0), accent, true))
                                    .child(MainText::new(tr("vacancy_experience_add")).px(14.0).color(accent)),
                            )
                            .child(div().absolute().size_full().continuous_corners(14.0).stroke_border(accent.opacity(0.3), 1.0)),
                    ),
            ),
        )
    }

    fn preferences_section(state: &Entity<CreateResumeState>) -> ResumeFormGroup {
        ResumeFormGroup::new(vec![
            ResumeToggleRow::new(
                "create-resume-relocation",
                tr("vacancy_relocation_available"),
                Self::bool_binding(state, |form| form.ready_to_relocation, |form, value| form.ready_to_relocation = value),
            )
            .icon("airplane".to_owned())
            .icon_background(hex_color(0x008DE5))
            .into_any_element(),
            ResumeToggleRow::new(
                "create-resume-army",
                tr("vacancy_resume_army"),
                Self::bool_binding(state, |form| form.ready_to_army, |form, value| form.ready_to_army = value),
            )
            .icon("shield.fill".to_owned())
            .icon_background(hex_color(0x00C781))
            .into_any_element(),
        ])
        .title(tr("vacancy_resume_section_work"))
    }

    fn submit_button(
        state: &Entity<CreateResumeState>,
        nav_store: Option<Entity<NavigationStore>>,
        can_press: bool,
        is_submitting: bool,
        is_editing: bool,
        theme: &AppTheme,
    ) -> Div {
        let press_state = state.clone();
        let label = if is_submitting {
            tr("vacancy_resume_saving")
        } else if is_editing {
            tr("vacancy_resume_save")
        } else {
            tr("vacancy_resume_create_btn")
        };
        vstack(10.0)
            .w_full()
            .px(px(16.0))
            .pt(px(6.0))
            .child(
                ButtonUi::new("create-resume-submit")
                    .disabled(!can_press)
                    .on_press(move |window, cx| Self::submit(&press_state, nav_store.clone(), window, cx))
                    .child(MainText::new(label).color(theme.content_on(theme.primary._100))),
            )
            .child(SecondaryText::new(tr("vacancy_resume_single_hint")).px(12.0).tac(TextAlignT::Center))
    }

    fn picker_sheet(state: &Entity<CreateResumeState>, picker: ResumePicker, cx: &mut App) -> AnyElement {
        let is_presented = Binding::entity(
            state.clone(),
            move |form: &CreateResumeState| form.active_picker == Some(picker),
            move |form: &mut CreateResumeState, value: bool, cx| {
                if !value && form.active_picker == Some(picker) {
                    form.active_picker = None;
                    cx.notify();
                }
            },
        );
        let form_state = state.clone();
        let select = move |set: fn(&mut CreateResumeState, String)| {
            let form_state = form_state.clone();
            move |option: PickerOption, _: &mut Window, cx: &mut App| {
                form_state.update(cx, |form, cx| {
                    set(form, option.id);
                    cx.notify();
                });
            }
        };
        match picker {
            ResumePicker::Gender => SearchablePickerSheet::new(
                "create-resume-gender-sheet",
                tr("vacancy_resume_gender"),
                ResumeOptionCatalog::gender_options(),
                select(|form, id| form.gender = id),
            )
            .presented("create-resume-gender-host", is_presented, cx)
            .into_any_element(),
            ResumePicker::Education => SearchablePickerSheet::new(
                "create-resume-education-sheet",
                tr("vacancy_resume_education"),
                ResumeOptionCatalog::education_options(),
                select(|form, id| form.education = id),
            )
            .presented("create-resume-education-host", is_presented, cx)
            .into_any_element(),
            ResumePicker::Experience => SearchablePickerSheet::new(
                "create-resume-experience-sheet",
                tr("vacancy_resume_exp_level"),
                ResumeOptionCatalog::experience_options(),
                select(|form, id| form.experience_level = id),
            )
            .presented("create-resume-experience-host", is_presented, cx)
            .into_any_element(),
            ResumePicker::Currency => SearchablePickerSheet::new(
                "create-resume-currency-sheet",
                tr("vacancy_resume_currency"),
                ResumeOptionCatalog::currency_options(),
                select(|form, id| form.salary_currency = id),
            )
            .presented("create-resume-currency-host", is_presented, cx)
            .into_any_element(),
            ResumePicker::Country => {
                let country_state = state.clone();
                GeoCountryPickerSheet::new("create-resume-country-sheet", move |picked: String, _, cx| {
                    let next = COUNTRY_RULE.clamped(&picked);
                    country_state.update(cx, |form, cx| {
                        if next != form.country {
                            form.city = String::new();
                        }
                        form.country = next;
                        cx.notify();
                    });
                })
                .presented("create-resume-country-host", is_presented, cx)
                .into_any_element()
            }
            ResumePicker::City => {
                let city_state = state.clone();
                let country = state.read(cx).country.clone();
                GeoCityPickerSheet::new("create-resume-city-sheet", country, move |picked: String, _, cx| {
                    city_state.update(cx, |form, cx| {
                        form.city = CITY_RULE.clamped(&picked);
                        cx.notify();
                    });
                })
                .presented("create-resume-city-host", is_presented, cx)
                .into_any_element()
            }
            ResumePicker::BirthDate => {
                let sheet_state = state.clone();
                let background = rise_ui::theme(cx).bg._100;
                NativeBottomSheetPresentationHost::new("create-resume-birth-date-host", is_presented, move |_, cx| {
                    Self::birth_date_sheet(&sheet_state, cx)
                })
                .presentation_detents([PresentationDetent::Medium, PresentationDetent::Large])
                .presentation_drag_indicator(true)
                .presentation_background(background)
                .presentation_corner_radius(20.0)
                .into_any_element()
            }
        }
    }

    fn birth_date_sheet(state: &Entity<CreateResumeState>, cx: &mut App) -> Lifecycle {
        let theme = rise_ui::theme(cx).clone();
        let done_state = state.clone();
        let appear_state = state.clone();
        let selection = Binding::entity(
            state.clone(),
            |form: &CreateResumeState| form.birth_date.unwrap_or_else(birth_date_picker_start),
            |form: &mut CreateResumeState, value: Date, cx| {
                form.birth_date = Some(value);
                cx.notify();
            },
        );
        vstack(16.0)
            .w_full()
            .h_full()
            .bg(theme.bg._100)
            .child(
                hstack(0.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(18.0))
                    .child(MainText::new(tr("vacancy_resume_birth_date")).px(17.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
                    .child(spacer())
                    .child(
                        plain("create-resume-birth-date-done")
                            .on_press(move |_, cx| {
                                done_state.update(cx, |form, cx| {
                                    form.active_picker = None;
                                    cx.notify();
                                });
                            })
                            .child(div().text_color(theme.primary._100).child(tr("done"))),
                    ),
            )
            .child(
                div().w_full().px(px(8.0)).child(
                    DatePicker::new("create-resume-birth-date-picker", selection)
                        .range(Date::distant_past()..=Date::now())
                        .displayed_components(DatePickerComponents::Date)
                        .tint(theme.primary._100),
                ),
            )
            .child(spacer_min_h(0.0))
            .on_appear("create-resume-birth-date-appear", move |_, cx| {
                appear_state.update(cx, |form, cx| {
                    if form.birth_date.is_none() {
                        form.birth_date = Some(birth_date_picker_start());
                        cx.notify();
                    }
                });
            })
    }

    fn hydrate(state: &Entity<CreateResumeState>, cx: &mut App) {
        let my_resume = VacancyInteractionsStore::shared(cx).update(cx, |store, cx| store.my_resume(cx));
        if let Some(resume) = my_resume {
            let account_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
            let country = resume.country.clone();
            state.update(cx, |form, cx| {
                form.apply_resume(resume);
                form.editing_account_id = account_id;
                form.editing_account_epoch = SaiAccountCacheScope::active_epoch();
                cx.notify();
            });
            if !country.is_empty() {
                GeoProvider::shared(cx).update(cx, |geo, cx| geo.ensure_cities_loaded(&country, cx));
            }
            return;
        }

        let Some((name, phone, description)) = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| (profile.name.clone(), profile.more.phone.clone(), profile.description.clone()))
        else {
            return;
        };
        let names = VacancyServicesStore::shared(cx).read(cx).split_name(&name);
        let birth_date = VacancyServicesStore::birth_date_from_profile_hb(cx);
        state.update(cx, |form, cx| {
            if form.first_name.is_empty() {
                form.first_name = names.0;
            }
            if form.second_name.is_empty() {
                form.second_name = names.1;
            }
            if let Some(phone) = phone.filter(|phone| !phone.is_empty())
                && form.phone.is_empty()
            {
                form.phone = PhoneNumberFormatting::format(&phone);
            }
            if let Some(description) = description.filter(|description| !description.is_empty())
                && form.summary.is_empty()
            {
                form.summary = description;
            }
            if let Some(birth_date) = birth_date {
                form.birth_date = Some(birth_date);
            }
            cx.notify();
        });
    }

    fn submit(
        state: &Entity<CreateResumeState>,
        nav_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !state.read(cx).can_press(cx) {
            return;
        }
        if !state.read(cx).is_form_complete() {
            let page_focus = state.read(cx).focus_handle.clone();
            window.focus(&page_focus, cx);
            state.update(cx, |form, cx| {
                form.show_missing = true;
                form.missing_reveal_tick += 1;
                cx.notify();
            });
            return;
        }
        let birth_date = state.read(cx).birth_date;
        let birth_date_wire = birth_date
            .map(|date| VacancyServicesStore::shared(cx).read(cx).wire_date_string(&date))
            .unwrap_or_default();
        let form = state.read(cx);
        let resume_id = form.editing_resume_id;
        let is_editing = form.is_editing();
        let first_name = trimmed(&form.first_name);
        let second_name = trimmed(&form.second_name);
        let phone = trimmed(&form.phone);
        let city = trimmed(&form.city);
        let country = trimmed(&form.country);
        let profession = trimmed(&form.profession);
        let gender = form.gender.clone();
        let education = form.education.clone();
        let experience_level = form.experience_level.clone();
        let skills = form.skills.clone();
        let experiences = form.experiences.clone();
        let summary = trimmed(&form.summary);
        let salary_from = form.salary_from.clone();
        let salary_to = form.salary_to.clone();
        let salary_currency = form.salary_currency.clone();
        let ready_to_relocation = form.ready_to_relocation;
        let ready_to_army = form.ready_to_army;
        VacancyInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.submit_create_resume(
                resume_id,
                first_name,
                second_name,
                phone,
                city,
                country,
                profession,
                gender,
                birth_date_wire,
                education,
                experience_level,
                skills,
                experiences,
                summary,
                salary_from,
                salary_to,
                salary_currency,
                ready_to_relocation,
                ready_to_army,
                Box::new(move |_: ResumeModel, cx: &mut App| {
                    if guide_owns_jobs_scene(cx) {
                        pop_screen(nav_store.as_ref(), cx);
                        return;
                    }
                    let title_key = if is_editing { "vacancy_resume_updated" } else { "vacancy_resume_success_title" };
                    NativeConfirmAlert::present_notice(
                        &tr(title_key),
                        &tr("vacancy_resume_success_msg"),
                        None,
                        None,
                        move |cx: &mut App| pop_screen(nav_store.as_ref(), cx),
                        cx,
                    );
                }),
                cx,
            );
        });
    }

    fn salary_filter(
        id: &'static str,
        value: String,
        state: &Entity<CreateResumeState>,
        set: fn(&mut CreateResumeState, String),
        window: &mut Window,
        cx: &mut App,
    ) {
        let state = state.clone();
        on_change(id, value, false, window, cx, move |_, value: &String, _, cx| {
            let digits = VacancyInteractionsStore::shared(cx).read(cx).salary_input(value);
            if digits != *value {
                state.update(cx, |form, cx| {
                    set(form, digits);
                    cx.notify();
                });
            }
        });
    }

    /// section id, measured bounds stand in for the scroll lookup by id
    fn section_id(section: impl IntoElement, id: &'static str, section_bounds: &ResumeSectionBounds) -> Div {
        let section_bounds = section_bounds.clone();
        div().relative().w_full().child(section).child(
            measure(SharedString::from(format!("create-resume-section-bounds-{id}")), move |bounds, _, _| {
                section_bounds.borrow_mut().insert(id, bounds);
            })
            .absolute()
            .size_full(),
        )
    }

    fn reveal_area(
        area: ResumeTextArea,
        area_bounds: &ResumeAreaBounds,
        reader: &Entity<GiftSheetScrollReader>,
        duration: f64,
        cx: &mut App,
    ) {
        let bounds = area_bounds.borrow().get(&area).copied();
        if let Some(bounds) = bounds {
            reader.update(cx, |reader, cx| reader.reveal(bounds, true, duration, cx));
        }
    }
}

impl RenderOnce for CreateResume {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("create-resume-state", cx, |_, cx| CreateResumeState::new(cx));
        let reader = window.use_keyed_state("create-resume-scroll-reader", cx, |_, _| GiftSheetScrollReader::new());
        reader.update(cx, |reader, _| reader.frame(window));
        let is_submitting = VacancyServicesStore::shared(cx).read(cx).is_submitting_resume();
        let geo = GeoProvider::shared(cx);
        let locale = LanguageStore::shared(cx).locale();

        let form = state.read(cx);
        let is_editing = form.is_editing();
        let can_press = form.can_press(cx);
        let marks = form.missing_marks();
        let missing_reveal_tick = form.missing_reveal_tick;
        let section_bounds = form.section_bounds.clone();
        let is_guide_ready = form.is_guide_form_ready(cx);
        let focused_area = form.focused_area;
        let focused_area_text = form.focused_area_text();
        let birth_date = form.formatted_birth_date();
        let gender = form.gender.clone();
        let education = form.education.clone();
        let experience_level = form.experience_level.clone();
        let salary_currency = form.salary_currency.clone();
        let salary_from = form.salary_from.clone();
        let salary_to = form.salary_to.clone();
        let city = form.city.clone();
        let country = form.country.clone();
        let experiences = form.experiences.clone();
        let area_bounds = form.area_bounds.clone();
        let focus_handle = form.focus_handle.clone();
        let mounted_picker = form.mounted_picker;

        let (country_flag, country_iso2) = geo
            .read(cx)
            .countries()
            .iter()
            .find(|geo_country| geo_country.name == country)
            .map_or((String::new(), String::new()), |geo_country| (geo_country.flag(), geo_country.iso2.clone()));
        let country_title = if country_iso2.is_empty() {
            country.clone()
        } else {
            locale.localized_string_for_region_code(&country_iso2).unwrap_or_else(|| country.clone())
        };

        Self::salary_filter("create-resume-salary-from-change", salary_from, &state, |form, digits| form.salary_from = digits, window, cx);
        Self::salary_filter("create-resume-salary-to-change", salary_to, &state, |form, digits| form.salary_to = digits, window, cx);
        on_change("create-resume-guide-ready", is_guide_ready, true, window, cx, |_, ready: &bool, _, cx| {
            GuideFacts::set(RiseGuideSignal::ResumeFormReady, *ready, cx);
        });

        let focus_bounds = area_bounds.clone();
        let focus_reader = reader.clone();
        on_change("create-resume-focused-area", focused_area, false, window, cx, move |_, area: &Option<ResumeTextArea>, _, cx| {
            if let Some(area) = *area {
                Self::reveal_area(area, &focus_bounds, &focus_reader, 0.2, cx);
            }
        });
        let text_bounds = area_bounds.clone();
        let text_reader = reader.clone();
        on_change("create-resume-focused-text", focused_area_text, false, window, cx, move |_, _, _, cx| {
            if let Some(area) = focused_area {
                Self::reveal_area(area, &text_bounds, &text_reader, 0.15, cx);
            }
        });

        let reveal_state = state.clone();
        let reveal_bounds = section_bounds.clone();
        let reveal_reader = reader.clone();
        on_change("create-resume-missing-reveal", missing_reveal_tick, false, window, cx, move |_, _: &u64, _, cx| {
            let target = if reveal_state.read(cx).missing_personal() { PERSONAL_SECTION_ID } else { LOCATION_SECTION_ID };
            let bounds = reveal_bounds.borrow().get(target).copied();
            if let Some(bounds) = bounds {
                reveal_reader.update(cx, |reader, cx| reader.scroll_to(bounds, MISSING_REVEAL_ANCHOR, 0.25, cx));
            }
        });

        let experience_section = Self::experience_section(&state, &experiences, &area_bounds, &theme, window, cx);
        let content = vstack(22.0)
            .w_full()
            .pt(px(12.0))
            .pb(px(48.0))
            .child(Self::avatar_header(&theme, cx))
            .child(Self::section_id(
                Self::identity_section(&state, &gender, birth_date, marks).guide_anchor(RiseGuideAnchor::ResumePersonal),
                PERSONAL_SECTION_ID,
                &section_bounds,
            ))
            .child(Self::section_id(
                Self::location_section(&state, country_title, country_flag, city, country, marks)
                    .guide_anchor(RiseGuideAnchor::ResumeLocation),
                LOCATION_SECTION_ID,
                &section_bounds,
            ))
            .child(Self::salary_section(&state, &salary_currency))
            .child(Self::details_section(&state, &education, &experience_level))
            .child(Self::skills_section(&state, &theme))
            .child(Self::summary_section(&state, &area_bounds, &theme))
            .child(experience_section)
            .child(Self::preferences_section(&state))
            .child(
                Self::submit_button(&state, nav_store.clone(), can_press, is_submitting, is_editing, &theme)
                    .guide_anchor(RiseGuideAnchor::ResumeSubmit),
            );

        let sheet = mounted_picker.map(|picker| Self::picker_sheet(&state, picker, cx));
        let key_state = state.clone();
        let key_nav = nav_store.clone();
        let appear_state = state.clone();
        let page_focus = PageFocus::keyed_with("create-resume-focus", focus_handle.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let handle = reader.read(cx).handle();
        let title = if is_editing { tr("vacancy_resume_edit_title") } else { tr("vacancy_resume_create_title") };

        SettingsWrapperUi::new(title, move |_, cx| pop_screen(nav_store.as_ref(), cx))
            .content(
                div()
                    .relative()
                    .size_full()
                    .key_context("CreateResume")
                    .track_focus(&focus_handle)
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let modifiers = &event.keystroke.modifiers;
                        if event.keystroke.key == "enter"
                            && modifiers.secondary()
                            && !modifiers.shift
                            && !modifiers.alt
                            && key_state.read(cx).active_picker.is_none()
                        {
                            cx.stop_propagation();
                            Self::submit(&key_state, key_nav.clone(), window, cx);
                        }
                    })
                    .child(
                        scroll_view("create-resume-scroll")
                            .scroll_handle(handle)
                            .list_shell_scroll_under_chrome(cx)
                            .bg(theme.bg._100)
                            .child(content),
                    )
                    .children(sheet),
            )
            .on_appear("create-resume-appear", move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
                if !appear_state.read(cx).did_hydrate {
                    appear_state.update(cx, |form, _| form.did_hydrate = true);
                    UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_my_profile_loaded(cx));
                    GeoProvider::shared(cx).update(cx, |geo, cx| geo.ensure_countries_loaded(cx));
                    Self::hydrate(&appear_state, cx);
                }
                let ready = appear_state.read(cx).is_guide_form_ready(cx);
                GuideFacts::set(RiseGuideSignal::ResumeFormReady, ready, cx);
            })
            .on_disappear("create-resume-disappear", move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
                GuideFacts::set(RiseGuideSignal::ResumeFormReady, false, cx);
            })
    }
}

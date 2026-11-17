use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    App, Bounds, Entity, FocusHandle, FontWeight, KeyDownEvent, Pixels, SharedString, Subscription, Window,
    transparent_black,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_catalog::VacancySchedule;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::{Animated, Lerp};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::ui::input_ui::InputOutlinedRowUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::gifts::pages::gift_shop_sheet::GiftSheetScrollReader;
use crate::modules::vacancy::components::resume_form_ui::ResumeFlowLayout;
use crate::modules::vacancy::components::vacancy_form_support::{
    VacancyTextEditor, guide_owns_jobs_scene, keeps_form_focus, pop_screen,
};
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeCurrency, VacancyModel};
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

static TITLE_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("vacancy_create_vacancy", "/title"));
static COMPANY_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("vacancy_create_vacancy", "/company"));
static CITY_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("vacancy_create_vacancy", "/city"));
static DESCRIPTION_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("vacancy_create_vacancy", "/description"));
static SKILL_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("vacancy_create_vacancy", "/skills/*"));

const SCHEDULES: [VacancySchedule; 3] = [VacancySchedule::FullTime, VacancySchedule::Flexible, VacancySchedule::Remote];
const FORMATS: [(&str, &str); 3] = [
    ("office", "vacancy_format_office"),
    ("remote", "vacancy_format_remote"),
    ("hybrid", "vacancy_format_hybrid"),
];
const SALARY_FROM_PLACEHOLDER: &str = "100 000";
const SALARY_TO_PLACEHOLDER: &str = "300 000";
const DEFAULT_SCHEDULE: &str = "full-time";
const DEFAULT_FORMAT: &str = "office";
const SKILLS_KEYBOARD_GAP: f32 = 8.0;
const MISSING_REVEAL_ANCHOR: f32 = 0.2;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum FieldID {
    Title,
    Company,
    City,
    Salary,
    Skills,
    Description,
}

impl FieldID {
    fn key(self) -> &'static str {
        match self {
            FieldID::Title => "title",
            FieldID::Company => "company",
            FieldID::City => "city",
            FieldID::Salary => "salary",
            FieldID::Skills => "skills",
            FieldID::Description => "description",
        }
    }
}

type VacancyFieldBounds = Rc<RefCell<HashMap<FieldID, Bounds<Pixels>>>>;
type VacancySkillsBlockBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

fn trimmed(value: &str) -> String {
    trimming_whitespaces_and_newlines(value).to_owned()
}

struct CreateVacancyState {
    title: String,
    company: String,
    city: String,
    salary_from: String,
    salary_to: String,
    salary_currency: String,
    selected_schedule: String,
    selected_format: String,
    description: String,
    skills: Vec<String>,
    skill_input: String,
    skill_focused: bool,
    is_currency_menu_presented: bool,
    is_relocation: bool,
    show_missing: bool,
    missing_reveal_tick: u64,
    field_bounds: VacancyFieldBounds,
    skills_block_bounds: VacancySkillsBlockBounds,
    focus_handle: FocusHandle,
    _subscriptions: [Subscription; 2],
}

impl CreateVacancyState {
    fn new(cx: &mut Context<Self>) -> Self {
        let service = VacancyServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            title: String::new(),
            company: String::new(),
            city: String::new(),
            salary_from: String::new(),
            salary_to: String::new(),
            salary_currency: ResumeCurrency::DEFAULT_CODE.to_owned(),
            selected_schedule: DEFAULT_SCHEDULE.to_owned(),
            selected_format: DEFAULT_FORMAT.to_owned(),
            description: String::new(),
            skills: Vec::new(),
            skill_input: String::new(),
            skill_focused: false,
            is_currency_menu_presented: false,
            is_relocation: false,
            show_missing: false,
            missing_reveal_tick: 0,
            field_bounds: Rc::new(RefCell::new(HashMap::new())),
            skills_block_bounds: Rc::new(Cell::new(None)),
            focus_handle: cx.focus_handle(),
            _subscriptions: [
                cx.observe(&service, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// everything the server requires so the form never sends what will be refused. a lesson
    /// needs only title and company
    fn is_lesson(cx: &App) -> bool {
        guide_owns_jobs_scene(cx)
    }

    fn missing_title(&self, cx: &App) -> bool {
        trimming_whitespaces_and_newlines(&self.title).is_empty() || (!Self::is_lesson(cx) && !TITLE_RULE.is_satisfied(&self.title))
    }

    fn missing_company(&self, cx: &App) -> bool {
        trimming_whitespaces_and_newlines(&self.company).is_empty()
            || (!Self::is_lesson(cx) && !COMPANY_RULE.is_satisfied(&self.company))
    }

    fn missing_city(&self, cx: &App) -> bool {
        !Self::is_lesson(cx) && !CITY_RULE.is_satisfied(&self.city)
    }

    fn salary_inverted(&self, cx: &App) -> bool {
        let Some(service) = VacancyServicesStore::try_read(cx) else {
            return false;
        };
        let from = service.parse_salary_int(&self.salary_from);
        let to = service.parse_salary_int(&self.salary_to);
        !Self::is_lesson(cx) && from > 0 && to > 0 && from > to
    }

    fn missing_skills(&self, cx: &App) -> bool {
        !Self::is_lesson(cx) && self.skills.is_empty() && !SKILL_RULE.is_satisfied(&self.skill_input)
    }

    fn missing_description(&self, cx: &App) -> bool {
        !Self::is_lesson(cx) && !DESCRIPTION_RULE.is_satisfied(&self.description)
    }

    fn is_form_complete(&self, cx: &App) -> bool {
        !self.missing_title(cx)
            && !self.missing_company(cx)
            && !self.missing_city(cx)
            && !self.salary_inverted(cx)
            && !self.missing_skills(cx)
            && !self.missing_description(cx)
    }

    /// none is a valid field, otherwise the caption under it
    fn field_error(&self, missing: bool, value: &str) -> Option<String> {
        if !self.show_missing || !missing {
            return None;
        }
        Some(if trimming_whitespaces_and_newlines(value).is_empty() { tr("form_required") } else { tr("form_invalid_value") })
    }

    fn first_missing(&self, cx: &App) -> FieldID {
        if self.missing_title(cx) {
            return FieldID::Title;
        }
        if self.missing_company(cx) {
            return FieldID::Company;
        }
        if self.missing_city(cx) {
            return FieldID::City;
        }
        if self.salary_inverted(cx) {
            return FieldID::Salary;
        }
        if self.missing_skills(cx) {
            return FieldID::Skills;
        }
        FieldID::Description
    }

    fn prefill(&mut self, v: &VacancyModel, cx: &mut Context<Self>) {
        self.title = v.title.clone();
        self.company = v.company.clone();
        self.city = v.city.clone();
        self.salary_from = if v.salary_from() > 0 { v.salary_from().to_string() } else { String::new() };
        self.salary_to = if v.salary_to() > 0 { v.salary_to().to_string() } else { String::new() };
        self.salary_currency = ResumeCurrency::option(&v.salary_currency).code.to_owned();
        if let Some(service) = VacancyServicesStore::try_read(cx) {
            self.selected_schedule = service.normalize_schedule(&v.schedule);
            self.selected_format = service.normalize_job_format(&v.job_format);
        }
        self.description = v.description.clone();
        self.skills = v.skills.clone();
        self.is_relocation = v.is_relocation_available;
        cx.notify();
    }

    fn skills_with_pending(&self) -> Vec<String> {
        let pending = trimming_whitespaces_and_newlines(&self.skill_input);
        if pending.is_empty() || self.skills.iter().any(|skill| skill == pending) {
            return self.skills.clone();
        }
        let mut skills = self.skills.clone();
        skills.push(pending.to_owned());
        skills
    }

    fn commit_typed_skills(&mut self, value: &str) {
        let Some((last, separator)) = value.char_indices().rev().find(|(_, c)| is_newline(*c)) else {
            return;
        };
        let tail = value[last + separator.len_utf8()..].to_owned();
        for part in value[..last].split(is_newline).filter(|part| !part.is_empty()) {
            self.skill_input = part.to_owned();
            self.add_skill();
        }
        self.skill_input = tail;
    }

    fn add_skill(&mut self) {
        let s = trimming_whitespaces_and_newlines(&self.skill_input).to_owned();
        if s.is_empty() || self.skills.contains(&s) {
            self.skill_input = String::new();
            return;
        }
        self.skills.push(s);
        self.skill_input = String::new();
    }
}

#[derive(IntoElement)]
pub struct CreateVacancy {
    prefill_vacancy: Option<VacancyModel>,
}

impl CreateVacancy {
    pub fn new(prefill_vacancy: Option<VacancyModel>) -> Self {
        Self { prefill_vacancy }
    }

    fn string_binding(
        state: &Entity<CreateVacancyState>,
        get: fn(&CreateVacancyState) -> String,
        set: fn(&mut CreateVacancyState, String),
    ) -> Binding<String> {
        Binding::entity(state.clone(), get, move |state, value, cx| {
            set(state, value);
            cx.notify();
        })
    }

    /// error_text none is valid, empty string gives a red frame without a caption
    fn labeled_field(
        id: &'static str,
        label: String,
        value: Binding<String>,
        placeholder: String,
        error_text: Option<String>,
        theme: &AppTheme,
    ) -> Div {
        let error_color = theme.semantic.error_100;
        let has_error = error_text.is_some();
        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SecondaryText::new(label).px(12.0))
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(keeps_form_focus(InputOutlinedRowUi::new(id, value, placeholder, div())))
                    .when(has_error, |field| {
                        field.child(div().absolute().inset_0().capsule().stroke_border(error_color, 1.0))
                    }),
            )
            .children(error_text.filter(|text| !text.is_empty()).map(|text| Self::field_error_caption(text, theme)))
    }

    fn field_error_caption(text: String, theme: &AppTheme) -> SecondaryText {
        SecondaryText::new(text).px(12.0).color(theme.semantic.error_100)
    }

    /// field id, measured bounds stand in for the scroll lookup by id
    fn field_id(field: impl IntoElement, id: FieldID, field_bounds: &VacancyFieldBounds) -> Div {
        let field_bounds = field_bounds.clone();
        div().relative().w_full().child(field).child(
            measure(SharedString::from(format!("create-vacancy-field-bounds-{}", id.key())), move |bounds, _, _| {
                field_bounds.borrow_mut().insert(id, bounds);
            })
            .absolute()
            .size_full(),
        )
    }

    fn scroll_to_field(id: FieldID, anchor_y: f32, state: &Entity<CreateVacancyState>, reader: &Entity<GiftSheetScrollReader>, cx: &mut App) {
        let bounds = state.read(cx).field_bounds.borrow().get(&id).copied();
        if let Some(bounds) = bounds {
            reader.update(cx, |reader, cx| reader.scroll_to(bounds, anchor_y, 0.25, cx));
        }
    }

    fn reveal_skills(state: &Entity<CreateVacancyState>, reader: &Entity<GiftSheetScrollReader>, cx: &mut App) {
        let bounds = state.read(cx).skills_block_bounds.get();
        if let Some(bounds) = bounds {
            reader.update(cx, |reader, cx| reader.scroll_to(bounds, 1.0, 0.25, cx));
        }
    }

    fn keyed_option(
        id: SharedString,
        key: String,
        label_key: &str,
        selected: &Binding<String>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let is_selected = selected.get(cx) == key;
        let target = if is_selected { 1_f32 } else { 0_f32 };
        let motion =
            window.use_keyed_state(SharedString::from(format!("{id}-motion")), cx, move |_, _| Animated::new(target));
        let now = RiseClock::monotonic();
        let progress = motion.update(cx, |animated, cx| {
            animated.set_in(target, ease_in_out(0.15), now, cx);
            animated.frame_value(now, window)
        });
        let accent = theme.primary._100;
        let selected = selected.clone();
        plain(id)
            .flex_1()
            .min_w(px(0.0))
            .on_press(move |window, cx| selected.set(key.clone(), window, cx))
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .capsule()
                    .swiftui_font(Font::system_weight(13.0, if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }))
                    .text_color(theme.text.secondary.lerp(&accent, progress))
                    .bg(theme.bg._200.lerp(&accent.opacity(0.15), progress))
                    .stroke_border(transparent_black().lerp(&accent, progress), 1.0)
                    .child(tr(label_key)),
            )
    }

    fn keyed_picker(
        id: &'static str,
        title: String,
        options: Vec<(String, String)>,
        selected: Binding<String>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let mut row = hstack(8.0).w_full();
        for (key, label_key) in options {
            let option_id = SharedString::from(format!("{id}-{key}"));
            row = row.child(Self::keyed_option(option_id, key, &label_key, &selected, theme, window, cx));
        }
        vstack(8.0).align(HorizontalAlignment::Leading).w_full().child(SecondaryText::new(title).px(12.0)).child(row)
    }

    fn skills_tag_input(state: &Entity<CreateVacancyState>, theme: &AppTheme, error_text: Option<String>, cx: &App) -> Div {
        let (skills, has_input) = {
            let form = state.read(cx);
            (form.skills.clone(), !form.skill_input.is_empty())
        };
        let accent = theme.primary._100;
        let binding = SKILL_RULE.binding(Binding::entity(
            state.clone(),
            |form: &CreateVacancyState| form.skill_input.clone(),
            |form: &mut CreateVacancyState, value: String, cx| {
                form.skill_input = value.clone();
                form.commit_typed_skills(&value);
                cx.notify();
            },
        ));
        let add_state = state.clone();
        let focus_state = state.clone();
        let stroke = if error_text.is_some() { theme.semantic.error_100 } else { theme.border._100 };
        let field_row = hstack(8.0)
            .w_full()
            .min_h(px(24.0))
            .px(px(14.0))
            .py(px(10.0))
            .bg(theme.bg._200)
            .capsule()
            .stroke_border(stroke, 1.0)
            .child(
                div().flex_1().min_w(px(0.0)).text_color(theme.text.primary).child(
                    VacancyTextEditor::new("create-vacancy-skill-input", binding)
                        .placeholder(tr("vacancy_skill_input_placeholder"), theme.text.secondary)
                        .font_size(14.0)
                        .max_lines(1)
                        .on_focus_change(move |focused, _, cx| {
                            focus_state.update(cx, |form, cx| {
                                form.skill_focused = focused;
                                cx.notify();
                            });
                        }),
                ),
            )
            .when(has_input, |row| {
                row.child(div().capture_any_mouse_down(|_, window, _| window.prevent_default()).child(
                    plain("create-vacancy-skill-add")
                        .on_press(move |_, cx| {
                            add_state.update(cx, |form, cx| {
                                form.add_skill();
                                cx.notify();
                            });
                        })
                        .children(IconUi::sized("plus.circle.fill", px(20.0), accent, false)),
                ))
            });

        let mut column = vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(field_row)
            .children(error_text.map(|text| Self::field_error_caption(text, theme)));
        if !skills.is_empty() {
            let mut flow = ResumeFlowLayout::new().spacing(8.0).line_spacing(8.0);
            for skill in skills {
                let remove_state = state.clone();
                let removed = skill.clone();
                flow = flow.child(
                    hstack(4.0)
                        .px(px(10.0))
                        .py(px(6.0))
                        .bg(accent.opacity(0.12))
                        .capsule()
                        .child(
                            div()
                                .swiftui_font(Font::system_weight(13.0, FontWeight::MEDIUM))
                                .text_color(accent)
                                .child(skill.clone()),
                        )
                        .child(
                            plain(SharedString::from(format!("create-vacancy-skill-remove-{skill}")))
                                .on_press(move |_, cx| {
                                    let removed = removed.clone();
                                    remove_state.update(cx, |form, cx| {
                                        form.skills.retain(|item| *item != removed);
                                        cx.notify();
                                    });
                                })
                                .children(IconUi::sized("xmark", px(10.0), accent, false)),
                        ),
                );
            }
            column = column.child(flow);
        }

        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SecondaryText::new(tr("vacancy_field_skills")).px(12.0))
            .child(column)
    }

    fn currency_picker(state: &Entity<CreateVacancyState>, theme: &AppTheme, cx: &App) -> Div {
        let salary_currency = state.read(cx).salary_currency.clone();
        let is_presented = Binding::entity(
            state.clone(),
            |form: &CreateVacancyState| form.is_currency_menu_presented,
            |form: &mut CreateVacancyState, value: bool, cx| {
                form.is_currency_menu_presented = value;
                cx.notify();
            },
        );
        let actions_state = state.clone();
        let actions: HoldContextMenuPresentationActions = Rc::new(move |cx: &App| {
            let selected = actions_state.read(cx).salary_currency.clone();
            ResumeCurrency::all()
                .iter()
                .map(|currency| {
                    let action = HoldContextMenuAction::new(
                        currency.code,
                        format!("{}  {} ({})", currency.symbol, tr(currency.name_key), currency.code),
                    );
                    if currency.code == selected { action.system_image("checkmark") } else { action }
                })
                .collect()
        });
        let action_state = state.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            action_state.update(cx, |form, cx| {
                form.salary_currency = id;
                cx.notify();
            });
        });
        let source_theme = theme.clone();
        let source_currency = salary_currency.clone();
        let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
            Self::currency_field(&source_currency, &source_theme).into_any_element()
        });
        let opener = state.clone();
        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SecondaryText::new(tr("vacancy_resume_currency")).px(12.0))
            .child(
                plain("create-vacancy-currency")
                    .w_full()
                    .on_press(move |_, cx| {
                        opener.update(cx, |form, cx| {
                            form.is_currency_menu_presented = true;
                            cx.notify();
                        });
                    })
                    .child(Self::currency_field(&salary_currency, theme))
                    .tap_context_menu(
                        "create-vacancy-currency-menu",
                        is_presented,
                        HoldContextMenuAlignment::Leading,
                        false,
                        false,
                        false,
                        None,
                        actions,
                        on_action,
                        source,
                    ),
            )
    }

    fn currency_field(salary_currency: &str, theme: &AppTheme) -> Div {
        let option = ResumeCurrency::option(salary_currency);
        let radius = f32::from(theme.input.radius_300);
        hstack(8.0)
            .w_full()
            .min_h(px(46.0))
            .px(px(14.0))
            .bg(theme.bg._200)
            .continuous_corners(radius)
            .stroke_border(theme.border._100, 1.0)
            .child(
                div()
                    .min_w(px(0.0))
                    .swiftui_font(Font::system_size(15.0))
                    .text_color(theme.text.primary)
                    .line_clamp(1)
                    .child(format!("{}  {} ({})", option.symbol, tr(option.name_key), option.code)),
            )
            .child(spacer_min_w(0.0))
            .children(IconUi::sized("chevron.up.chevron.down", px(13.0), theme.text.secondary, false))
    }

    fn submit(
        state: &Entity<CreateVacancyState>,
        editing_vacancy_id: Option<i64>,
        nav_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let is_submitting = VacancyServicesStore::try_read(cx).is_none_or(|service| service.is_submitting_vacancy());
        if is_submitting {
            return;
        }
        if !state.read(cx).is_form_complete(cx) {
            let page_focus = state.read(cx).focus_handle.clone();
            window.focus(&page_focus, cx);
            state.update(cx, |form, cx| {
                form.show_missing = true;
                form.missing_reveal_tick += 1;
                cx.notify();
            });
            return;
        }
        let sent_skills = state.read(cx).skills_with_pending();
        state.update(cx, |form, cx| {
            form.add_skill();
            cx.notify();
        });
        let (title, company, city, salary_from, salary_to, salary_currency, schedule, job_format, description, is_relocation) = {
            let form = state.read(cx);
            (
                trimmed(&form.title),
                trimmed(&form.company),
                trimmed(&form.city),
                form.salary_from.clone(),
                form.salary_to.clone(),
                form.salary_currency.clone(),
                form.selected_schedule.clone(),
                form.selected_format.clone(),
                trimmed(&form.description),
                form.is_relocation,
            )
        };
        VacancyInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.submit_create_vacancy(
                title,
                company,
                city,
                salary_from,
                salary_to,
                salary_currency,
                schedule,
                job_format,
                description,
                sent_skills,
                is_relocation,
                editing_vacancy_id,
                Box::new(move |saved: VacancyModel, cx: &mut App| {
                    if guide_owns_jobs_scene(cx) {
                        pop_screen(nav_store.as_ref(), cx);
                        return;
                    }
                    let title_key =
                        if editing_vacancy_id.is_none() { "vacancy_create_success_title" } else { "bot_changes_saved" };
                    let message = if saved.is_awaiting_review() { tr("vacancy_create_success_msg") } else { String::new() };
                    NativeConfirmAlert::present_notice(
                        &tr(title_key),
                        &message,
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

    fn salary_filter(id: &'static str, value: String, state: &Entity<CreateVacancyState>, set: fn(&mut CreateVacancyState, String), window: &mut Window, cx: &mut App) {
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
}

impl RenderOnce for CreateVacancy {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("create-vacancy-state", cx, |_, cx| CreateVacancyState::new(cx));
        let reader = window.use_keyed_state("create-vacancy-scroll-reader", cx, |_, _| GiftSheetScrollReader::new());
        reader.update(cx, |reader, _| reader.frame(window));
        let is_submitting = VacancyServicesStore::shared(cx).read(cx).is_submitting_vacancy();
        let editing_vacancy_id = self.prefill_vacancy.as_ref().map(|vacancy| vacancy.id);
        let is_new = self.prefill_vacancy.is_none();
        let page_title = if is_new { tr("vacancy_create_title") } else { tr("vacancy_edit_title") };
        let submit_text = if is_new { tr("vacancy_create_submit") } else { tr("vacancy_edit_submit") };
        let (salary_from, salary_to, is_guide_ready, focus_handle) = {
            let form = state.read(cx);
            (
                form.salary_from.clone(),
                form.salary_to.clone(),
                !form.title.is_empty() && !form.company.is_empty(),
                form.focus_handle.clone(),
            )
        };
        let (title_error, company_error, city_error, salary_error, skills_error, description_error) = {
            let form = state.read(cx);
            (
                form.field_error(form.missing_title(cx), &form.title),
                form.field_error(form.missing_company(cx), &form.company),
                form.field_error(form.missing_city(cx), &form.city),
                form.show_missing && form.salary_inverted(cx),
                form.field_error(form.missing_skills(cx), &form.skill_input),
                form.field_error(form.missing_description(cx), &form.description),
            )
        };
        let (missing_reveal_tick, skills_count, field_bounds, skills_block_bounds) = {
            let form = state.read(cx);
            (form.missing_reveal_tick, form.skills.len(), form.field_bounds.clone(), form.skills_block_bounds.clone())
        };

        Self::salary_filter("create-vacancy-salary-from-change", salary_from, &state, |form, digits| form.salary_from = digits, window, cx);
        Self::salary_filter("create-vacancy-salary-to-change", salary_to, &state, |form, digits| form.salary_to = digits, window, cx);
        on_change("create-vacancy-guide-ready", is_guide_ready, true, window, cx, |_, ready: &bool, _, cx| {
            GuideFacts::set(RiseGuideSignal::VacancyFormReady, *ready, cx);
        });
        let reveal_state = state.clone();
        let reveal_reader = reader.clone();
        on_change("create-vacancy-missing-reveal", missing_reveal_tick, false, window, cx, move |_, _: &u64, _, cx| {
            let first = reveal_state.read(cx).first_missing(cx);
            Self::scroll_to_field(first, MISSING_REVEAL_ANCHOR, &reveal_state, &reveal_reader, cx);
        });
        let skills_state = state.clone();
        let skills_reader = reader.clone();
        on_change("create-vacancy-skills-count", skills_count, false, window, cx, move |old: &usize, new: &usize, _, cx| {
            if skills_state.read(cx).skill_focused && new > old {
                Self::reveal_skills(&skills_state, &skills_reader, cx);
            }
        });

        let schedules: Vec<(String, String)> =
            SCHEDULES.iter().map(|schedule| (schedule.wire().to_owned(), schedule.i18n_key().to_owned())).collect();
        let formats: Vec<(String, String)> =
            FORMATS.iter().map(|(key, label_key)| ((*key).to_owned(), (*label_key).to_owned())).collect();
        let schedule_binding = Self::string_binding(&state, |form| form.selected_schedule.clone(), |form, value| form.selected_schedule = value);
        let format_binding = Self::string_binding(&state, |form| form.selected_format.clone(), |form, value| form.selected_format = value);
        let schedule_picker = Self::keyed_picker("create-vacancy-schedule", tr("vacancy_filter_schedule"), schedules, schedule_binding, &theme, window, cx);
        let format_picker = Self::keyed_picker("create-vacancy-format", tr("vacancy_filter_format"), formats, format_binding, &theme, window, cx);
        let input_radius = f32::from(theme.input.radius_300);

        let press_state = state.clone();
        let press_nav = nav_store.clone();
        let form = vstack(16.0)
            .w_full()
            .px(px(16.0))
            .pt(px(12.0))
            .child(
                vstack(16.0)
                    .w_full()
                    .child(Self::field_id(
                        Self::labeled_field(
                            "create-vacancy-title",
                            tr("vacancy_field_title"),
                            TITLE_RULE.binding(Self::string_binding(&state, |form| form.title.clone(), |form, value| form.title = value)),
                            tr("vacancy_field_title_placeholder"),
                            title_error,
                            &theme,
                        ),
                        FieldID::Title,
                        &field_bounds,
                    ))
                    .child(Self::field_id(
                        Self::labeled_field(
                            "create-vacancy-company",
                            tr("vacancy_field_company"),
                            COMPANY_RULE.binding(Self::string_binding(&state, |form| form.company.clone(), |form, value| form.company = value)),
                            tr("vacancy_field_company_placeholder"),
                            company_error,
                            &theme,
                        ),
                        FieldID::Company,
                        &field_bounds,
                    ))
                    .guide_anchor(RiseGuideAnchor::CreateVacancyBasics),
            )
            .child(Self::field_id(
                Self::labeled_field(
                    "create-vacancy-city",
                    tr("vacancy_field_city"),
                    CITY_RULE.binding(Self::string_binding(&state, |form| form.city.clone(), |form, value| form.city = value)),
                    tr("vacancy_field_city_placeholder"),
                    city_error,
                    &theme,
                ),
                FieldID::City,
                &field_bounds,
            ))
            .child(Self::field_id(
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(
                        hstack(12.0)
                            .w_full()
                            .child(div().flex_1().min_w(px(0.0)).child(Self::labeled_field(
                                "create-vacancy-salary-from",
                                tr("vacancy_salary_from"),
                                Self::string_binding(&state, |form| form.salary_from.clone(), |form, value| form.salary_from = value),
                                SALARY_FROM_PLACEHOLDER.to_owned(),
                                salary_error.then(String::new),
                                &theme,
                            )))
                            .child(div().flex_1().min_w(px(0.0)).child(Self::labeled_field(
                                "create-vacancy-salary-to",
                                tr("vacancy_salary_to"),
                                Self::string_binding(&state, |form| form.salary_to.clone(), |form, value| form.salary_to = value),
                                SALARY_TO_PLACEHOLDER.to_owned(),
                                salary_error.then(String::new),
                                &theme,
                            ))),
                    )
                    .child(
                        SecondaryText::new(if salary_error { tr("form_invalid_value") } else { tr("vacancy_salary_optional_hint") })
                            .px(12.0)
                            .color(salary_error.then_some(theme.semantic.error_100)),
                    ),
                FieldID::Salary,
                &field_bounds,
            ))
            .child(Self::currency_picker(&state, &theme, cx))
            .child(schedule_picker)
            .child(format_picker)
            .child(
                vstack(0.0)
                    .w_full()
                    .mb(px(-SKILLS_KEYBOARD_GAP))
                    .child(Self::field_id(
                        Self::skills_tag_input(&state, &theme, skills_error, cx),
                        FieldID::Skills,
                        &field_bounds,
                    ))
                    .child(div().relative().w_full().h(px(SKILLS_KEYBOARD_GAP)).child(
                        measure("create-vacancy-skills-block", move |bounds, _, _| skills_block_bounds.set(Some(bounds)))
                            .size_full(),
                    )),
            )
            .child(Self::field_id(
                vstack(8.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(SecondaryText::new(tr("vacancy_field_description")).px(12.0))
                    .child(
                        div()
                            .w_full()
                            .p(px(12.0))
                            .bg(theme.bg._200)
                            .text_color(theme.text.primary)
                            .continuous_corners(input_radius)
                            .stroke_border(
                                if description_error.is_some() { theme.semantic.error_100 } else { theme.border._100 },
                                1.0,
                            )
                            .child(
                                VacancyTextEditor::new(
                                    "create-vacancy-description",
                                    DESCRIPTION_RULE.binding(Self::string_binding(&state, |form| form.description.clone(), |form, value| form.description = value)),
                                )
                                .font_size(15.0)
                                .min_height(120.0),
                            ),
                    )
                    .children(description_error.map(|text| Self::field_error_caption(text, &theme))),
                FieldID::Description,
                &field_bounds,
            ))
            .child(
                hstack(0.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(14.0))
                    .bg(theme.bg._200)
                    .continuous_corners(f32::from(theme.radius._300))
                    .child(MainText::new(tr("vacancy_relocation_available")).px(15.0))
                    .child(spacer())
                    .child(SwitchUi::new(
                        "create-vacancy-relocation",
                        Binding::entity(
                            state.clone(),
                            |form: &CreateVacancyState| form.is_relocation,
                            |form: &mut CreateVacancyState, value, cx| {
                                form.is_relocation = value;
                                cx.notify();
                            },
                        ),
                    )),
            )
            .child(
                div()
                    .w_full()
                    .pb(px(40.0))
                    .child(
                        ButtonUi::new("create-vacancy-submit")
                            .disabled(is_submitting)
                            .on_press(move |window, cx| {
                                Self::submit(&press_state, editing_vacancy_id, press_nav.clone(), window, cx)
                            })
                            .child(MainText::new(if is_submitting { tr("vacancy_create_sending") } else { submit_text }))
                            .guide_anchor(RiseGuideAnchor::CreateVacancySubmit),
                    ),
            );

        let key_state = state.clone();
        let key_nav = nav_store.clone();
        let appear_state = state.clone();
        let prefill_vacancy = self.prefill_vacancy;
        let page_focus = PageFocus::keyed_with("create-vacancy-focus", focus_handle.clone(), window, cx);
        let leaving_focus = page_focus.clone();

        SettingsWrapperUi::new(page_title, {
            let nav_store = nav_store.clone();
            move |_, cx| pop_screen(nav_store.as_ref(), cx)
        })
        .content(
            div()
                .size_full()
                .key_context("CreateVacancy")
                .track_focus(&focus_handle)
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let modifiers = &event.keystroke.modifiers;
                    if event.keystroke.key == "enter" && modifiers.secondary() && !modifiers.shift && !modifiers.alt {
                        cx.stop_propagation();
                        Self::submit(&key_state, editing_vacancy_id, key_nav.clone(), window, cx);
                    }
                })
                .child(
                    scroll_view("create-vacancy-scroll")
                        .scroll_handle(reader.read(cx).handle())
                        .list_shell_scroll_under_chrome(cx)
                        .bg(theme.bg._100)
                        .child(form),
                ),
        )
        .on_appear("create-vacancy-appear", move |window, cx| {
            PageFocus::did_appear(&page_focus, window, cx);
            if let Some(vacancy) = prefill_vacancy {
                appear_state.update(cx, |form, cx| form.prefill(&vacancy, cx));
            }
        })
        .on_disappear("create-vacancy-disappear", move |window, cx| {
            if let Some(window) = window {
                PageFocus::will_disappear(&leaving_focus, window, cx);
            }
            GuideFacts::set(RiseGuideSignal::VacancyFormReady, false, cx);
        })
    }
}

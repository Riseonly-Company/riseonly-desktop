use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, Focusable, FontWeight, Hsla, ListAlignment, SharedString, Subscription, Window,
};
use rise_core::foundation::foundation_compare::{CompareOptions, compare_case_insensitive, localized_compare, range_of};
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_i18n::{LanguageStore, tr};
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::icon_ui::IconUi;
use rise_ui::input_ui::input_actions::{MoveDown, MoveUp};
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text::TextAlignment;
use rise_ui::switch_ui::SwitchUi;

use crate::core::lib::canon_field_rule::{RiseFieldRule, RiseFocusedTextInput, RiseTextInputCaret};
use crate::core::ui::input_ui::{InputOutlinedRowUi, InputUiCaret};
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::stepped_highlight;
use crate::modules::vacancy::shared::geo_provider::GeoProvider;
use crate::modules::vacancy::shared::picker_option::PickerOption;

type ResumeRowAction = Rc<dyn Fn(&mut Window, &mut App)>;
type PickerSelectAction = Rc<dyn Fn(PickerOption, &mut Window, &mut App)>;
type PickerTextAction = Rc<dyn Fn(String, &mut Window, &mut App)>;
type PickerRowAction = Rc<dyn Fn(&SearchablePickerRow, &mut Window, &mut App)>;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn system_text(text: impl Into<SharedString>, font: Font, color: Hsla) -> Div {
    let text: SharedString = text.into();
    div().swiftui_font(font).text_color(color).whitespace_nowrap().child(text)
}

fn sf_icon(icon: &str, side: f32, color: Hsla) -> Div {
    div().flex_none().children(IconUi::sized(icon, px(side), color, icon.ends_with(".fill")))
}

#[derive(IntoElement)]
pub struct ResumeFormGroup {
    title: Option<String>,
    footer: Option<String>,
    rows: Vec<AnyElement>,
}

impl ResumeFormGroup {
    pub fn new(rows: Vec<AnyElement>) -> Self {
        Self { title: None, footer: None, rows }
    }

    pub fn title(mut self, title: impl Into<Option<String>>) -> Self {
        self.title = title.into();
        self
    }

    pub fn footer(mut self, footer: impl Into<Option<String>>) -> Self {
        self.footer = footer.into();
        self
    }
}

impl RenderOnce for ResumeFormGroup {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let count = self.rows.len();
        let mut card = vstack(0.0).w_full().bg(theme.bg._200).continuous_corners(18.0).overflow_hidden();
        for (i, row) in self.rows.into_iter().enumerate() {
            card = card.child(div().w_full().child(row));
            if i + 1 < count {
                card = card.child(
                    div()
                        .w_full()
                        .padding_edges(EdgeSet::LEADING, 16.0)
                        .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))),
                );
            }
        }

        vstack(8.0)
            .w_full()
            .items_start()
            .when_some(self.title, |group, title| {
                group.child(
                    div()
                        .padding_edges(EdgeSet::HORIZONTAL, 28.0)
                        .child(MainText::new(title.to_uppercase()).px(12.0).color(theme.text.secondary)),
                )
            })
            .child(div().w_full().padding_edges(EdgeSet::HORIZONTAL, 16.0).child(card))
            .when_some(self.footer, |group, footer| {
                group.child(div().w_full().child(SecondaryText::new(footer).px(12.0).tac(TextAlignT::Left).ml(28.0).mt(2.0)))
            })
    }
}

pub fn resume_icon_tile(icon: &str, bg: Hsla, theme: &AppTheme) -> Div {
    div()
        .flex_none()
        .size(px(29.0))
        .bg(bg)
        .continuous_corners(8.0)
        .flex()
        .items_center()
        .justify_center()
        .child(sf_icon(icon, 14.0, theme.content_on(bg)))
}

pub fn resume_label_color(is_error: bool, theme: &AppTheme) -> Hsla {
    if is_error { theme.semantic.error_100 } else { theme.text.primary }
}

#[derive(IntoElement)]
pub struct ResumeValueRow {
    id: ElementId,
    label: String,
    value: String,
    placeholder: String,
    icon: Option<String>,
    icon_background: Option<Hsla>,
    leading_emoji: Option<String>,
    is_error: bool,
    on_tap: ResumeRowAction,
}

impl ResumeValueRow {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<String>,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value: String::new(),
            placeholder: String::new(),
            icon: None,
            icon_background: None,
            leading_emoji: None,
            is_error: false,
            on_tap: Rc::new(on_tap),
        }
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn icon(mut self, icon: impl Into<Option<String>>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn icon_background(mut self, icon_background: impl Into<Option<Hsla>>) -> Self {
        self.icon_background = icon_background.into();
        self
    }

    pub fn leading_emoji(mut self, leading_emoji: impl Into<Option<String>>) -> Self {
        self.leading_emoji = leading_emoji.into();
        self
    }

    pub fn is_error(mut self, is_error: bool) -> Self {
        self.is_error = is_error;
        self
    }
}

impl RenderOnce for ResumeValueRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let on_tap = self.on_tap;
        let is_empty = self.value.is_empty();
        let shown = if is_empty { self.placeholder } else { self.value };
        let shown_color = if is_empty { theme.text.secondary.opacity(0.7) } else { theme.text.secondary };
        let tile_background = self.icon_background.unwrap_or_else(|| theme.primary._100.opacity(0.88));

        plain(self.id).w_full().bg(theme.bg._200).on_press(move |window, cx| on_tap(window, cx)).child(
            hstack(12.0)
                .w_full()
                .padding_edges(EdgeSet::HORIZONTAL, 16.0)
                .padding_edges(EdgeSet::VERTICAL, 13.0)
                .when_some(self.icon, |row, icon| row.child(resume_icon_tile(&icon, tile_background, &theme)))
                .child(
                    div()
                        .flex_none()
                        .child(MainText::new(self.label).px(15.0).color(resume_label_color(self.is_error, &theme))),
                )
                .child(spacer_min_w(8.0))
                .when_some(self.leading_emoji.filter(|emoji| !emoji.is_empty()), |row, emoji| {
                    row.child(system_text(emoji, Font::system_size(16.0), theme.text.primary))
                })
                .child(div().min_w_0().child(MainText::new(shown).px(15.0).color(shown_color).line_limit(1)))
                .child(sf_icon(rise_ui::IconUi::trailing_chevron(), 12.0, theme.text.secondary.opacity(0.6))),
        )
    }
}

#[derive(IntoElement)]
pub struct ResumeInlineInputRow {
    id: ElementId,
    label: String,
    text: Binding<String>,
    placeholder: String,
    icon: Option<String>,
    icon_background: Option<Hsla>,
    is_error: bool,
}

impl ResumeInlineInputRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<String>, text: Binding<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            text,
            placeholder: String::new(),
            icon: None,
            icon_background: None,
            is_error: false,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn icon(mut self, icon: impl Into<Option<String>>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn icon_background(mut self, icon_background: impl Into<Option<Hsla>>) -> Self {
        self.icon_background = icon_background.into();
        self
    }

    pub fn is_error(mut self, is_error: bool) -> Self {
        self.is_error = is_error;
        self
    }
}

impl RenderOnce for ResumeInlineInputRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let tile_background = self.icon_background.unwrap_or_else(|| theme.primary._100.opacity(0.88));
        let field = ShapedTextField::new(child_id(&self.id, "field"), "", self.text)
            .prompt(Some(self.placeholder))
            .font_size(15.0)
            .text_align(TextAlignment::Trailing.resolve(LayoutDirection::current()))
            .placeholder_color(theme.text.secondary.opacity(0.55));

        hstack(12.0)
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, 16.0)
            .padding_edges(EdgeSet::VERTICAL, 13.0)
            .bg(theme.bg._200)
            .when_some(self.icon, |row, icon| row.child(resume_icon_tile(&icon, tile_background, &theme)))
            .child(
                div().flex_none().child(MainText::new(self.label).px(15.0).color(resume_label_color(self.is_error, &theme))),
            )
            .child(div().flex().flex_1().min_w_0().text_color(theme.text.primary).child(field))
    }
}

#[derive(IntoElement)]
pub struct ResumeToggleRow {
    id: ElementId,
    label: String,
    is_on: Binding<bool>,
    icon: Option<String>,
    icon_background: Option<Hsla>,
}

impl ResumeToggleRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<String>, is_on: Binding<bool>) -> Self {
        Self { id: id.into(), label: label.into(), is_on, icon: None, icon_background: None }
    }

    pub fn icon(mut self, icon: impl Into<Option<String>>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn icon_background(mut self, icon_background: impl Into<Option<Hsla>>) -> Self {
        self.icon_background = icon_background.into();
        self
    }
}

impl RenderOnce for ResumeToggleRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let tile_background = self.icon_background.unwrap_or_else(|| theme.primary._100.opacity(0.88));
        let switch = SwitchUi::new(child_id(&self.id, "switch"), self.is_on);

        hstack(12.0)
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, 16.0)
            .padding_edges(EdgeSet::VERTICAL, 10.0)
            .bg(theme.bg._200)
            .when_some(self.icon, |row, icon| row.child(resume_icon_tile(&icon, tile_background, &theme)))
            .child(MainText::new(self.label).px(15.0).color(theme.text.primary))
            .child(spacer_min_w(8.0))
            .child(switch)
    }
}

#[derive(IntoElement)]
pub struct ResumeChipsField {
    id: ElementId,
    chips: Binding<Vec<String>>,
    placeholder: String,
    rule: RiseFieldRule,
}

impl ResumeChipsField {
    pub fn new(id: impl Into<ElementId>, chips: Binding<Vec<String>>, placeholder: impl Into<String>) -> Self {
        Self { id: id.into(), chips, placeholder: placeholder.into(), rule: RiseFieldRule::NONE }
    }

    /// rule for a list element not the container, the none rule keeps the old behaviour
    pub fn rule(mut self, rule: RiseFieldRule) -> Self {
        self.rule = rule;
        self
    }

    fn is_separator(c: char) -> bool {
        c == ',' || c == '\n'
    }
}

struct ResumeChipsFieldSession {
    field: Entity<InputUiState>,
    chips: Binding<Vec<String>>,
    rule: RiseFieldRule,
    caret: Rc<dyn RiseTextInputCaret>,
    _subscriptions: Vec<Subscription>,
}

impl ResumeChipsFieldSession {
    fn new(chips: Binding<Vec<String>>, rule: RiseFieldRule, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::MultiLine, cx);
            field.set_chromeless(true, cx);
            field.set_max_visible_lines(1, cx);
            field.set_font_size(Some(px(15.0)), cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let caret: Rc<dyn RiseTextInputCaret> = Rc::new(InputUiCaret::new(&field));
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, field, event: &InputUiEvent, window, cx| {
                if *event == InputUiEvent::Changed {
                    let value = field.read(cx).text().to_owned();
                    this.commit_complete_tokens(&value, window, cx);
                    cx.notify();
                }
            }),
            cx.on_focus(&handle, window, |this, _, cx| {
                RiseFocusedTextInput::report(this.caret.clone(), cx);
            }),
            cx.on_blur(&handle, window, |this, window, cx| {
                RiseFocusedTextInput::resign(&this.caret, cx);
                this.commit_draft(window, cx);
            }),
        ];
        Self { field, chips, rule, caret, _subscriptions: subscriptions }
    }

    fn commit_complete_tokens(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = value.rfind(ResumeChipsField::is_separator) else {
            return;
        };
        self.append_chips(&value[..last], window, cx);
        let tail = value[last + 1..].trim_start_matches(char::is_whitespace).to_owned();
        self.field.update(cx, |field, cx| field.reset(tail, cx));
    }

    fn commit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.field.read(cx).text().to_owned();
        self.append_chips(&draft, window, cx);
        if !draft.is_empty() {
            self.field.update(cx, |field, cx| field.reset(String::new(), cx));
        }
    }

    fn append_chips(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut chips = self.chips.get(cx);
        let mut changed = false;
        let parts = text
            .split(ResumeChipsField::is_separator)
            .map(trimming_whitespaces_and_newlines)
            .filter(|part| !part.is_empty());
        for raw in parts {
            let p = self.rule.clamped(raw);
            if p.is_empty() || chips.iter().any(|chip| compare_case_insensitive(chip, &p).is_eq()) {
                continue;
            }
            chips.push(p);
            changed = true;
        }
        if changed {
            self.chips.set(chips, window, cx);
        }
    }
}

impl RenderOnce for ResumeChipsField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let binding = self.chips.clone();
        let rule = self.rule;
        let session = window.use_keyed_state(child_id(&self.id, "chips-session"), cx, move |window, cx| {
            ResumeChipsFieldSession::new(binding, rule, window, cx)
        });
        let chips_binding = self.chips.clone();
        session.update(cx, |session, _| {
            session.chips = chips_binding;
            session.rule = rule;
        });

        let field = session.read(cx).field.clone();
        let placeholder = self.placeholder;
        let placeholder_color = Some(theme.text.secondary.opacity(0.6));
        field.update(cx, |field, cx| {
            if field.placeholder().as_ref() != placeholder.as_str() {
                field.set_placeholder(placeholder, cx);
            }
            if field.placeholder_color() != placeholder_color {
                field.set_placeholder_color(placeholder_color, cx);
            }
        });
        let has_draft = !trimming_whitespaces(field.read(cx).text()).is_empty();
        let chips = self.chips.get(cx);

        let chip_color = theme.primary._100;
        let chip_views = chips.iter().map(|chip| {
            let remove_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(chip.clone()));
            let removed = chip.clone();
            let binding = self.chips.clone();
            hstack(6.0)
                .padding_edges(EdgeSet::HORIZONTAL, 10.0)
                .padding_edges(EdgeSet::VERTICAL, 6.0)
                .bg(chip_color.opacity(0.13))
                .capsule()
                .child(system_text(chip.clone(), Font::system_weight(13.0, FontWeight::MEDIUM), chip_color))
                .child(
                    plain(remove_id)
                        .on_press(move |window, cx| {
                            let mut chips = binding.get(cx);
                            chips.retain(|chip| *chip != removed);
                            binding.set(chips, window, cx);
                        })
                        .child(sf_icon("xmark", 9.0, chip_color.opacity(0.8))),
                )
        });
        let flow = (!chips.is_empty()).then(|| ResumeFlowLayout::new().children(chip_views));

        let plus_session = session.clone();
        let plus = has_draft.then(|| {
            plain(child_id(&self.id, "commit"))
                .on_press(move |window, cx| {
                    plus_session.update(cx, |session, cx| {
                        session.commit_draft(window, cx);
                        let handle = session.field.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    });
                })
                .child(sf_icon("plus.circle.fill", 20.0, chip_color))
        });

        div().w_full().padding_edges(EdgeSet::HORIZONTAL, 16.0).child(
            vstack(10.0)
                .w_full()
                .items_start()
                .p(px(14.0))
                .bg(theme.bg._200)
                .continuous_corners(18.0)
                .overflow_hidden()
                .child(
                    hstack(8.0)
                        .w_full()
                        .child(div().flex_1().min_w_0().text_color(theme.text.primary).child(field))
                        .children(plus),
                )
                .children(flow),
        )
    }
}

/// flow layout of the resume,
/// greedy line breaking is what a wrapping flex row does so taffy does the layout
#[derive(IntoElement)]
pub struct ResumeFlowLayout {
    spacing: f32,
    line_spacing: f32,
    children: Vec<AnyElement>,
}

impl ResumeFlowLayout {
    pub fn new() -> Self {
        Self { spacing: 8.0, line_spacing: 8.0, children: Vec::new() }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn line_spacing(mut self, line_spacing: f32) -> Self {
        self.line_spacing = line_spacing;
        self
    }
}

impl Default for ResumeFlowLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for ResumeFlowLayout {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ResumeFlowLayout {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let row = div().w_full().flex().flex_wrap().gap_x(px(self.spacing)).gap_y(px(self.line_spacing));
        let row = if LayoutDirection::current().is_right_to_left() { row.flex_row_reverse() } else { row };
        row.children(self.children)
    }
}

/// presentation modifiers of the searchable picker sheet, the kit host owns them on desktop
fn searchable_picker_presentation<E: IntoElement>(
    id: impl Into<ElementId>,
    is_presented: Binding<bool>,
    cx: &App,
    content: impl Fn(&mut Window, &mut App) -> E + 'static,
) -> NativeBottomSheetPresentationHost {
    let background = rise_ui::theme(cx).bg._100;
    NativeBottomSheetPresentationHost::new(id, is_presented, content)
        .presentation_detents([PresentationDetent::Fraction(0.7), PresentationDetent::Large])
        .presentation_drag_indicator(true)
        .presentation_background(background)
        .presentation_corner_radius(20.0)
}

struct GeoPickerObservation {
    _subscription: Subscription,
}

fn observe_geo(id: ElementId, window: &mut Window, cx: &mut App) {
    let geo = GeoProvider::shared(cx);
    window.use_keyed_state(id, cx, move |_, cx: &mut Context<GeoPickerObservation>| GeoPickerObservation {
        _subscription: cx.observe(&geo, |_, _, cx| cx.notify()),
    });
}

#[derive(IntoElement, Clone)]
pub struct GeoCountryPickerSheet {
    id: ElementId,
    on_select: PickerTextAction,
}

impl GeoCountryPickerSheet {
    pub fn new(id: impl Into<ElementId>, on_select: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), on_select: Rc::new(on_select) }
    }

    /// presents the country picker sheet
    pub fn presented(
        self,
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        cx: &App,
    ) -> NativeBottomSheetPresentationHost {
        searchable_picker_presentation(id, is_presented, cx, move |_, _| self.clone())
    }
}

/// country picker rows sorted for one locale and catalog size
#[derive(Default)]
struct CountryOptionsCache {
    locale: String,
    country_count: usize,
    options: Vec<PickerOption>,
}

impl RenderOnce for GeoCountryPickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        observe_geo(child_id(&self.id, "geo"), window, cx);
        let locale = LanguageStore::shared(cx).locale();
        let cache = window.use_keyed_state(child_id(&self.id, "country-options"), cx, |_, _| CountryOptionsCache::default());
        let geo = GeoProvider::shared(cx);
        let geo_state = geo.read(cx);
        let is_loading = geo_state.is_loading_countries();
        let country_count = geo_state.countries().len();
        let cached = {
            let cache = cache.read(cx);
            (cache.country_count == country_count && cache.locale == locale.identifier()).then(|| cache.options.clone())
        };
        let options = match cached {
            Some(options) => options,
            None => {
                let mut options: Vec<PickerOption> = geo
                    .read(cx)
                    .countries()
                    .iter()
                    .map(|country| {
                        PickerOption::with_leading(
                            country.name.clone(),
                            country.flag(),
                            locale
                                .localized_string_for_region_code(&country.iso2)
                                .unwrap_or_else(|| country.name.clone()),
                        )
                    })
                    .collect();
                options.sort_by(|a, b| localized_compare(&a.title, &b.title, locale.identifier()));
                let stored = options.clone();
                let locale_identifier = locale.identifier().to_owned();
                cache.update(cx, |cache, _| {
                    *cache = CountryOptionsCache { locale: locale_identifier, country_count, options: stored };
                });
                options
            }
        };

        let select = self.on_select.clone();
        let free_text = self.on_select;
        let sheet = SearchablePickerSheet::new(
            child_id(&self.id, "sheet"),
            tr("vacancy_resume_country"),
            options,
            move |opt, window, cx| {
                select(opt.id.clone(), window, cx);
                GeoProvider::shared(cx).update(cx, |geo, cx| geo.ensure_cities_loaded(&opt.id, cx));
            },
        )
        .allow_free_text(true)
        .free_text_label(tr("vacancy_resume_use"))
        .is_loading(is_loading)
        .on_free_text(move |text, window, cx| free_text(text, window, cx));

        div().size_full().child(sheet).on_appear(child_id(&self.id, "appear"), |_, cx| {
            GeoProvider::shared(cx).update(cx, |geo, cx| geo.ensure_countries_loaded(cx));
        })
    }
}

#[derive(IntoElement, Clone)]
pub struct GeoCityPickerSheet {
    id: ElementId,
    country: String,
    on_select: PickerTextAction,
}

impl GeoCityPickerSheet {
    pub fn new(
        id: impl Into<ElementId>,
        country: impl Into<String>,
        on_select: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), country: country.into(), on_select: Rc::new(on_select) }
    }

    /// presents the city picker sheet
    pub fn presented(
        self,
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        cx: &App,
    ) -> NativeBottomSheetPresentationHost {
        searchable_picker_presentation(id, is_presented, cx, move |_, _| self.clone())
    }
}

impl RenderOnce for GeoCityPickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        observe_geo(child_id(&self.id, "geo"), window, cx);
        let geo = GeoProvider::shared(cx);
        let geo_state = geo.read(cx);
        let options: Vec<PickerOption> = geo_state
            .cached_cities(&self.country)
            .iter()
            .map(|city| PickerOption::new(city.clone(), city.clone()))
            .collect();
        let is_loading = geo_state.is_loading_cities(&self.country);

        let select = self.on_select.clone();
        let free_text = self.on_select;
        let sheet = SearchablePickerSheet::new(
            child_id(&self.id, "sheet"),
            tr("vacancy_field_city"),
            options,
            move |opt, window, cx| select(opt.id, window, cx),
        )
        .allow_free_text(true)
        .free_text_label(tr("vacancy_resume_use"))
        .is_loading(is_loading)
        .on_free_text(move |text, window, cx| free_text(text, window, cx));

        let appear_id = child_id(&self.id, "appear");
        let country = self.country;
        div().size_full().child(sheet).on_appear(appear_id, move |_, cx| {
            GeoProvider::shared(cx).update(cx, |geo, cx| geo.ensure_cities_loaded(&country, cx));
        })
    }
}

#[derive(IntoElement, Clone)]
pub struct SearchablePickerSheet {
    id: ElementId,
    title: String,
    options: Vec<PickerOption>,
    allow_free_text: bool,
    free_text_label: String,
    is_loading: bool,
    on_select: PickerSelectAction,
    on_free_text: Option<PickerTextAction>,
}

impl SearchablePickerSheet {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<String>,
        options: Vec<PickerOption>,
        on_select: impl Fn(PickerOption, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            options,
            allow_free_text: false,
            free_text_label: String::new(),
            is_loading: false,
            on_select: Rc::new(on_select),
            on_free_text: None,
        }
    }

    pub fn allow_free_text(mut self, allow_free_text: bool) -> Self {
        self.allow_free_text = allow_free_text;
        self
    }

    pub fn free_text_label(mut self, free_text_label: impl Into<String>) -> Self {
        self.free_text_label = free_text_label.into();
        self
    }

    pub fn is_loading(mut self, is_loading: bool) -> Self {
        self.is_loading = is_loading;
        self
    }

    pub fn on_free_text(mut self, on_free_text: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        self.on_free_text = Some(Rc::new(on_free_text));
        self
    }

    /// presents the searchable picker sheet with the presentation modifiers of its body
    pub fn presented(
        self,
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        cx: &App,
    ) -> NativeBottomSheetPresentationHost {
        searchable_picker_presentation(id, is_presented, cx, move |_, _| self.clone())
    }

    fn filtered(&self, q: &str) -> Vec<PickerOption> {
        if q.is_empty() {
            return self.options.clone();
        }
        self.options
            .iter()
            .filter(|option| range_of(&option.title, q, CompareOptions::CASE_INSENSITIVE).is_some())
            .cloned()
            .collect()
    }

    fn show_free_text_row(&self, q: &str) -> bool {
        if !self.allow_free_text || q.is_empty() {
            return false;
        }
        !self.options.iter().any(|option| compare_case_insensitive(&option.title, q).is_eq())
    }

    fn rows(&self, search: &str) -> Vec<SearchablePickerRow> {
        let q = trimming_whitespaces(search);
        let filtered = self.filtered(q);
        let mut rows = Vec::with_capacity(filtered.len() + 1);
        if self.show_free_text_row(q) {
            rows.push(SearchablePickerRow::FreeText(q.to_owned()));
        }
        if self.is_loading && filtered.is_empty() {
            rows.push(SearchablePickerRow::Loading);
        }
        rows.extend(filtered.into_iter().map(SearchablePickerRow::Picker));
        rows
    }
}

/// one row of the sheet list,
/// a chosen rows list index is also its keyboard index. loading row exists only when no
/// option does
#[derive(Clone, PartialEq)]
enum SearchablePickerRow {
    FreeText(String),
    Loading,
    Picker(PickerOption),
}

/// search text plus the keyboard highlight and the rows on screen
struct SearchablePickerSheetState {
    search: String,
    highlighted: Option<usize>,
    list: ListUi,
    rows: Vec<SearchablePickerRow>,
}

impl SearchablePickerSheetState {
    fn new(cx: &App) -> Self {
        Self {
            search: String::new(),
            highlighted: None,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            rows: Vec::new(),
        }
    }

    fn sync(&mut self, rows: &[SearchablePickerRow]) {
        if self.rows != rows {
            self.list.replaced(rows.len());
            self.rows = rows.to_vec();
            self.highlighted = None;
        }
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.rows.iter().filter(|row| **row != SearchablePickerRow::Loading).count();
        self.highlighted = stepped_highlight(self.highlighted, delta, count);
        if let Some(index) = self.highlighted {
            self.list.state().scroll_to_reveal_item(index);
        }
        cx.notify();
    }
}

impl RenderOnce for SearchablePickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx: &mut Context<SearchablePickerSheetState>| {
            SearchablePickerSheetState::new(cx)
        });
        let close = dismiss(cx);

        let rows = Rc::new(self.rows(&state.read(cx).search));
        state.update(cx, |state, _| state.sync(&rows));
        let highlighted = state.read(cx).highlighted;

        let activate: PickerRowAction = {
            let on_select = self.on_select.clone();
            let on_free_text = self.on_free_text.clone();
            let close = close.clone();
            Rc::new(move |row: &SearchablePickerRow, window: &mut Window, cx: &mut App| {
                match row {
                    SearchablePickerRow::Loading => return,
                    SearchablePickerRow::FreeText(text) => {
                        if let Some(on_free_text) = &on_free_text {
                            on_free_text(text.clone(), window, cx);
                        }
                    }
                    SearchablePickerRow::Picker(option) => on_select(option.clone(), window, cx),
                }
                close.call_as_function(window, cx);
            })
        };

        let header_close = close.clone();
        let header = hstack(None)
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, 16.0)
            .padding_edges(EdgeSet::TOP, 16.0)
            .padding_edges(EdgeSet::BOTTOM, 10.0)
            .child(MainText::new(self.title.clone()).px(17.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
            .child(spacer())
            .child(
                plain(child_id(&self.id, "close"))
                    .on_press(move |window, cx| header_close.call_as_function(window, cx))
                    .child(sf_icon("xmark.circle.fill", 22.0, theme.text.secondary.opacity(0.5))),
            );

        let search_binding = Binding::entity(
            state.clone(),
            |state: &SearchablePickerSheetState| state.search.clone(),
            |state, search, _| {
                state.search = search;
                state.highlighted = None;
            },
        );
        let submit_state = state.clone();
        let submit_rows = rows.clone();
        let submit_activate = activate.clone();
        let search = div().w_full().padding_edges(EdgeSet::HORIZONTAL, 16.0).padding_edges(EdgeSet::BOTTOM, 8.0).child(
            InputOutlinedRowUi::new(
                child_id(&self.id, "search"),
                search_binding,
                tr("search"),
                sf_icon("magnifyingglass", 14.0, theme.text.secondary),
            )
            .auto_focus(true)
            .on_submit(move |window, cx| {
                let index = submit_state.read(cx).highlighted.unwrap_or(0);
                if let Some(row) = submit_rows.get(index) {
                    submit_activate(row, window, cx);
                }
            })
            .on_cancel(move |window, cx| close.call_as_function(window, cx)),
        );

        let row_id = self.id.clone();
        let row_theme = theme.clone();
        let free_text_label = self.free_text_label.clone();
        let list = state.read(cx).list.element(move |index, _, _| match rows.get(index) {
            Some(row) => {
                let is_highlighted = highlighted == Some(index);
                searchable_picker_row(&row_id, row, is_highlighted, &free_text_label, &activate, &row_theme)
            }
            None => div().into_any_element(),
        });

        let up_state = state.clone();
        let down_state = state;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .on_action(move |_: &MoveUp, _, cx| up_state.update(cx, |state, cx| state.move_highlight(-1, cx)))
            .on_action(move |_: &MoveDown, _, cx| down_state.update(cx, |state, cx| state.move_highlight(1, cx)))
            .child(header)
            .child(search)
            .child(div().w_full().flex_1().min_h_0().child(list))
    }
}

fn searchable_picker_row(
    sheet_id: &ElementId,
    row: &SearchablePickerRow,
    is_highlighted: bool,
    free_text_label: &str,
    activate: &PickerRowAction,
    theme: &AppTheme,
) -> AnyElement {
    let divider = div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.4));
    let highlight = theme.bg._200;
    match row {
        SearchablePickerRow::Loading => {
            div().w_full().pt(px(40.0)).flex().justify_center().child(progress_view()).into_any_element()
        }
        SearchablePickerRow::FreeText(text) => {
            let activate = activate.clone();
            let pressed = row.clone();
            vstack(0.0)
                .w_full()
                .child(
                    plain(child_id(sheet_id, "free-text"))
                        .w_full()
                        .when(is_highlighted, |button| button.bg(highlight))
                        .on_press(move |window, cx| activate(&pressed, window, cx))
                        .child(
                            hstack(10.0)
                                .w_full()
                                .padding_edges(EdgeSet::HORIZONTAL, 16.0)
                                .padding_edges(EdgeSet::VERTICAL, 13.0)
                                .child(sf_icon("plus.circle.fill", 18.0, theme.primary._100))
                                .child(
                                    MainText::new(format!("{free_text_label} \u{ab}{text}\u{bb}"))
                                        .px(15.0)
                                        .color(theme.primary._100),
                                )
                                .child(spacer()),
                        ),
                )
                .child(divider)
                .into_any_element()
        }
        SearchablePickerRow::Picker(option) => {
            let activate = activate.clone();
            let pressed = row.clone();
            let id = ElementId::NamedChild(Arc::new(sheet_id.clone()), SharedString::from(option.id.clone()));
            vstack(0.0)
                .w_full()
                .child(
                    plain(id)
                        .w_full()
                        .when(is_highlighted, |button| button.bg(highlight))
                        .on_press(move |window, cx| activate(&pressed, window, cx))
                        .child(
                            hstack(12.0)
                                .w_full()
                                .padding_edges(EdgeSet::HORIZONTAL, 16.0)
                                .padding_edges(EdgeSet::VERTICAL, 12.0)
                                .when_some(option.leading.clone().filter(|lead| !lead.is_empty()), |line, lead| {
                                    line.child(system_text(lead, Font::system_size(20.0), theme.text.primary))
                                })
                                .child(
                                    vstack(1.0)
                                        .items_start()
                                        .child(MainText::new(option.title.clone()).px(15.0).color(theme.text.primary))
                                        .when_some(option.subtitle.clone(), |column, subtitle| {
                                            column.child(SecondaryText::new(subtitle).px(12.0))
                                        }),
                                )
                                .child(spacer()),
                        ),
                )
                .child(div().w_full().padding_edges(EdgeSet::LEADING, 16.0).child(divider))
                .into_any_element()
        }
    }
}

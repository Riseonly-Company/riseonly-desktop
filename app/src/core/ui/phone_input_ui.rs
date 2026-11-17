use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Context, ElementId, Entity, Focusable, FontWeight, ListAlignment, ListOffset, ListState, MouseButton, Pixels,
    SharedString, Subscription, Task, Window, list,
};
use rise_core::foundation::foundation_character::{Character, characters};
use rise_core::foundation::foundation_compare::{CompareOptions, compare_case_insensitive, folding, localized_case_insensitive_compare};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_format::{PHONE_E164_MAX_DIGITS, PHONE_INTERNATIONAL_PREFIX};
use rise_i18n::{LanguageStore, tr};
use rise_theme::AppTheme;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::error_text_ui::ErrorTextUi;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState, Selection};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::phone_country_catalog::{PhoneCountryCatalog, PhoneInputCountry};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_field_rule::{RiseFocusedTextInput, RiseTextInputCaret};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::input_ui::InputUiCaret;

/// set value callback gets name and value
pub type PhoneInputUiSetValue = Rc<dyn Fn(&str, &str, &mut Window, &mut App)>;
pub type PhoneInputUiSetCallingCode = Rc<dyn Fn(&str, &mut Window, &mut App)>;
pub type PhoneInputUiSetCountry = Rc<dyn Fn(&PhoneInputCountry, &mut Window, &mut App)>;
pub type PhoneInputUiSubmit = Rc<dyn Fn(&mut Window, &mut App)>;

type PickerClose = Rc<dyn Fn(&mut Window, &mut App)>;
type PickerSelect = Rc<dyn Fn(&PhoneInputCountry, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct PhoneInputUi {
    id: ElementId,
    values: HashMap<String, String>,
    errors: HashMap<String, String>,
    name: String,
    title: Option<String>,
    placeholder: String,
    padding_top: f32,
    selected_country: Option<PhoneInputCountry>,
    auto_focus: bool,
    on_submit: PhoneInputUiSubmit,
    on_cancel: Option<PhoneInputUiSubmit>,
    set_value: Option<PhoneInputUiSetValue>,
    set_calling_code: Option<PhoneInputUiSetCallingCode>,
    set_country: Option<PhoneInputUiSetCountry>,
}

impl PhoneInputUi {
    const MINIMUM_FIELD_HEIGHT: f32 = 58.;
    const AUTO_FOCUS_DELAY: Duration = Duration::from_millis(150);
    const REFOCUS_AFTER_PICK_DELAY: Duration = Duration::from_millis(200);

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            values: HashMap::new(),
            errors: HashMap::new(),
            name: String::new(),
            title: None,
            placeholder: tr("phone_number_placeholder"),
            padding_top: 8.,
            selected_country: None,
            auto_focus: false,
            on_submit: Rc::new(|_, _| {}),
            on_cancel: None,
            set_value: None,
            set_calling_code: None,
            set_country: None,
        }
    }

    pub fn values(mut self, values: HashMap<String, String>) -> Self {
        self.values = values;
        self
    }

    pub fn errors(mut self, errors: HashMap<String, String>) -> Self {
        self.errors = errors;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn title(mut self, title: Option<String>) -> Self {
        self.title = title;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn padding_top(mut self, padding_top: f32) -> Self {
        self.padding_top = padding_top;
        self
    }

    pub fn selected_country(mut self, selected_country: Option<PhoneInputCountry>) -> Self {
        self.selected_country = selected_country;
        self
    }

    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Rc::new(on_submit);
        self
    }

    /// desktop only, escape in the number field after a composition has been taken back
    /// the field keeps the action to itself so a screen that steps back on escape has to ask here
    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    pub fn set_value(mut self, set_value: impl Fn(&str, &str, &mut Window, &mut App) + 'static) -> Self {
        self.set_value = Some(Rc::new(set_value));
        self
    }

    pub fn set_calling_code(mut self, set_calling_code: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.set_calling_code = Some(Rc::new(set_calling_code));
        self
    }

    pub fn set_country(
        mut self,
        set_country: impl Fn(&PhoneInputCountry, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.set_country = Some(Rc::new(set_country));
        self
    }

    /// national digits of raw for country, a leading +code is taken off
    /// the whole international number is held to canon phone input rule and the e164 ceiling
    pub fn national_digits(raw: &str, country: &PhoneInputCountry) -> String {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        let calling_code_digits = number_characters(&country.calling_code);
        let calling_code_count = character_count(&calling_code_digits);
        let all_digits = number_characters(trimmed);
        let digits = if trimmed.starts_with(PHONE_INTERNATIONAL_PREFIX) && all_digits.starts_with(&calling_code_digits) {
            drop_first_characters(&all_digits, calling_code_count)
        } else {
            all_digits
        };
        let limit = usize::try_from(PHONE_E164_MAX_DIGITS).unwrap_or(usize::MAX);
        let within_input_limit = prefix_characters(&rise_validate::PHONE.input(&format!("{calling_code_digits}{digits}")), limit);
        drop_first_characters(&within_input_limit, calling_code_count)
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

/// what the field reads from its caller when an edit lands, refreshed every render
#[derive(Clone)]
struct PhoneInputUiProps {
    name: String,
    selected_country: Option<PhoneInputCountry>,
    on_submit: PhoneInputUiSubmit,
    on_cancel: Option<PhoneInputUiSubmit>,
    set_value: Option<PhoneInputUiSetValue>,
    set_calling_code: Option<PhoneInputUiSetCallingCode>,
    set_country: Option<PhoneInputUiSetCountry>,
}

struct PhoneInputUiSession {
    field: Entity<InputUiState>,
    internal_country: PhoneInputCountry,
    is_country_picker_open: bool,
    country_search: String,
    published: String,
    caret: Rc<dyn RiseTextInputCaret>,
    props: PhoneInputUiProps,
    pending_focus: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl PhoneInputUiSession {
    fn new(
        seed: String,
        placeholder: String,
        props: PhoneInputUiProps,
        auto_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder_color = rise_ui::theme(cx).text.secondary.opacity(0.82);
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::SingleLine, cx);
            field.set_chromeless(true, cx);
            field.set_font_size(Some(px(17.)), cx);
            field.set_placeholder(placeholder, cx);
            field.set_placeholder_color(Some(placeholder_color), cx);
            field.reset(seed.clone(), cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let caret: Rc<dyn RiseTextInputCaret> = Rc::new(InputUiCaret::new(&field));

        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, _, event: &InputUiEvent, window, cx| match event {
                InputUiEvent::Changed => this.text_changed(window, cx),
                InputUiEvent::Submitted => {
                    let on_submit = this.props.on_submit.clone();
                    on_submit(window, cx);
                }
                InputUiEvent::Cancelled => match this.props.on_cancel.clone() {
                    Some(on_cancel) => on_cancel(window, cx),
                    None => ScreenFocus::release_text_field(window, cx),
                },
            }),
            cx.on_focus(&handle, window, |this, _, cx| {
                RiseFocusedTextInput::report(this.caret.clone(), cx);
            }),
            cx.on_blur(&handle, window, |this, _, cx| {
                RiseFocusedTextInput::resign(&this.caret, cx);
            }),
        ];

        let internal_country = PhoneCountryCatalog::default_country();
        if props.selected_country.is_none() {
            let set_calling_code = props.set_calling_code.clone();
            let set_country = props.set_country.clone();
            let country = internal_country.clone();
            window.defer(cx, move |window, cx| {
                if let Some(set_calling_code) = set_calling_code {
                    set_calling_code(&country.calling_code, window, cx);
                }
                if let Some(set_country) = set_country {
                    set_country(&country, window, cx);
                }
            });
        }

        let mut session = Self {
            field,
            internal_country,
            is_country_picker_open: false,
            country_search: String::new(),
            published: seed,
            caret,
            props,
            pending_focus: None,
            _subscriptions: subscriptions,
        };
        if auto_focus {
            session.focus_after(PhoneInputUi::AUTO_FOCUS_DELAY, window, cx);
        }
        session
    }

    fn country(&self) -> PhoneInputCountry {
        self.props.selected_country.clone().unwrap_or_else(|| self.internal_country.clone())
    }

    fn focus_after(&mut self, delay: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.field.read(cx).focus_handle(cx);
        self.pending_focus = Some(cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(delay).await;
            cx.update(|window, cx| window.focus(&handle, cx)).ok();
        }));
    }

    fn text_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.field.read(cx).text().to_owned();
        if raw == self.published {
            return;
        }
        let previous = self.published.clone();
        let digits = self.update_phone_value(&raw, &previous, window, cx);
        self.published = digits.clone();
        if digits != raw {
            self.field.update(cx, |field, cx| {
                field.set_text(&digits, cx);
                let end = field.text().len();
                field.set_selection(Selection::cursor(end), cx);
            });
        }
        cx.notify();
    }

    fn take_external(&mut self, external: &str, cx: &mut Context<Self>) {
        if external == self.published {
            return;
        }
        self.published = external.to_owned();
        let external = external.to_owned();
        self.field.update(cx, |field, cx| field.reset(external, cx));
        cx.notify();
    }

    fn update_phone_value(&mut self, raw: &str, previous: &str, window: &mut Window, cx: &mut Context<Self>) -> String {
        let country = self.country();
        let trimmed = trimming_whitespaces_and_newlines(raw);
        let typed_digits = number_characters(trimmed);
        let previous_digits = number_characters(previous);
        let typed_count = character_count(&typed_digits);
        let previous_count = character_count(&previous_digits);
        let is_single_step = typed_digits == previous_digits
            || (typed_count == previous_count + 1 && typed_digits.starts_with(&previous_digits))
            || (typed_count + 1 == previous_count && previous_digits.starts_with(&typed_digits));
        let is_whole_insert = !is_single_step;

        if !trimmed.starts_with(PHONE_INTERNATIONAL_PREFIX) {
            let mut local = number_characters(trimmed);
            let full_length = PhoneCountryCatalog::national_digit_count(&country);
            if is_whole_insert && full_length > 0 && character_count(&local) == full_length + 1 {
                local = drop_first_characters(&local, 1);
            }
            let local_country = if is_whole_insert {
                PhoneCountryCatalog::refined_country(&country.calling_code, &local).unwrap_or_else(|| country.clone())
            } else {
                country.clone()
            };
            if local_country != country {
                self.adopt_country(&local_country, window, cx);
            }
            let digits = PhoneInputUi::national_digits(&local, &local_country);
            self.publish_value(&digits, window, cx);
            return digits;
        }

        let all_digits = number_characters(trimmed);
        let current_calling_code = number_characters(&country.calling_code);
        let pasted_country = if all_digits.starts_with(&current_calling_code) {
            let national = drop_first_characters(&all_digits, character_count(&current_calling_code));
            Some(PhoneCountryCatalog::refined_country(&country.calling_code, &national).unwrap_or_else(|| country.clone()))
        } else {
            PhoneCountryCatalog::country_matching_international_number(trimmed)
        };
        let Some(pasted_country) = pasted_country else {
            let digits = PhoneInputUi::national_digits(raw, &country);
            self.publish_value(&digits, window, cx);
            return digits;
        };

        self.adopt_country(&pasted_country, window, cx);
        let national_digits = PhoneInputUi::national_digits(raw, &pasted_country);
        self.publish_value(&national_digits, window, cx);
        national_digits
    }

    /// sets the internal country, then calls the country and calling code callbacks
    fn adopt_country(&mut self, country: &PhoneInputCountry, window: &mut Window, cx: &mut Context<Self>) {
        self.internal_country = country.clone();
        if let Some(set_country) = self.props.set_country.clone() {
            set_country(country, window, cx);
        }
        if let Some(set_calling_code) = self.props.set_calling_code.clone() {
            set_calling_code(&country.calling_code, window, cx);
        }
    }

    fn publish_value(&self, digits: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(set_value) = self.props.set_value.clone() {
            let name = self.props.name.clone();
            set_value(&name, digits, window, cx);
        }
    }

    fn open_country_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ScreenFocus::release_text_field(window, cx);
        self.country_search.clear();
        self.is_country_picker_open = true;
        cx.notify();
    }

    fn close_country_picker(&mut self, cx: &mut Context<Self>) {
        self.is_country_picker_open = false;
        cx.notify();
    }

    fn select_country(&mut self, country: PhoneInputCountry, window: &mut Window, cx: &mut Context<Self>) {
        self.adopt_country(&country, window, cx);
        self.is_country_picker_open = false;
        self.focus_after(PhoneInputUi::REFOCUS_AFTER_PICK_DELAY, window, cx);
        cx.notify();
    }
}

impl RenderOnce for PhoneInputUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let value = self.values.get(&self.name).cloned().unwrap_or_default();
        let error_text = self.errors.get(&format!("{}Err", self.name)).cloned().unwrap_or_default();
        let field_height = f32::from(theme.input.height_300).max(Self::MINIMUM_FIELD_HEIGHT);
        let props = PhoneInputUiProps {
            name: self.name.clone(),
            selected_country: self.selected_country.clone(),
            on_submit: self.on_submit.clone(),
            on_cancel: self.on_cancel.clone(),
            set_value: self.set_value.clone(),
            set_calling_code: self.set_calling_code.clone(),
            set_country: self.set_country.clone(),
        };

        let seed = value.clone();
        let placeholder = self.placeholder.clone();
        let auto_focus = self.auto_focus;
        let init_props = props.clone();
        let session = window.use_keyed_state(self.child_id("phone-input-ui"), cx, move |window, cx| {
            PhoneInputUiSession::new(seed, placeholder, init_props, auto_focus, window, cx)
        });
        let escape_to_layer = props.on_cancel.is_none();
        session.update(cx, |session, cx| {
            session.field.update(cx, |field, _| field.set_escape_to_layer(escape_to_layer));
            session.props = props;
        });

        let tracked = session.clone();
        on_change(self.child_id("value"), value, false, window, cx, move |_, external, _, cx| {
            tracked.update(cx, |session, cx| session.take_external(external, cx));
        });

        let (field, country) = {
            let session = session.read(cx);
            (session.field.clone(), session.country())
        };
        let placeholder = self.placeholder.clone();
        field.update(cx, |field, cx| {
            if field.placeholder().as_ref() != placeholder.as_str() {
                field.set_placeholder(placeholder, cx);
            }
        });
        let handle = field.read(cx).focus_handle(cx);
        let focused = handle.is_focused(window);

        let opener = session.clone();
        let country_button = plain(self.child_id("country-button"))
            .on_press(move |window, cx| opener.update(cx, |session, cx| session.open_country_picker(window, cx)))
            .h(px(field_height))
            .padding_edges(EdgeSet::LEADING, 18.)
            .padding_edges(EdgeSet::TRAILING, 12.)
            .flex()
            .items_center()
            .child(
                hstack(7.)
                    .child(div().swiftui_font(Font::system_size(23.)).child(country.flag()))
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(16., FontWeight::SEMIBOLD))
                            .text_color(theme.text.primary)
                            .child(country.calling_code.clone()),
                    )
                    .children(IconUi::sized("chevron.down", px(10.), theme.text.secondary, false)),
            );

        let separator = div()
            .flex_none()
            .w(px(0.5))
            .h(px(field_height - 18.))
            .bg(theme.input.border_300.opacity(0.75));

        let focus_on_frame = handle;
        let text_slot = div()
            .flex_1()
            .min_w_0()
            .h(px(field_height))
            .padding_edges(EdgeSet::LEADING, 14.)
            .padding_edges(EdgeSet::TRAILING, 18.)
            .flex()
            .items_center()
            .text_color(theme.text.primary)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&focus_on_frame, cx))
            .child(field);

        let (stroke_color, stroke_width) =
            if focused { (theme.primary._100.opacity(0.8), 1.5) } else { (theme.input.border_300, 1.) };
        let field_row = div()
            .relative()
            .w_full()
            .child(
                hstack(0.)
                    .w_full()
                    .bg(theme.bg._200)
                    .rounded_full()
                    .overflow_hidden()
                    .child(country_button)
                    .child(separator)
                    .child(text_slot),
            )
            .child(centred_stroke(stroke_color, stroke_width, Shape::Capsule));

        let is_open = Binding::entity(
            session.clone(),
            |session: &PhoneInputUiSession| session.is_country_picker_open,
            |session, open, _| session.is_country_picker_open = open,
        );
        let search = Binding::entity(
            session.clone(),
            |session: &PhoneInputUiSession| session.country_search.clone(),
            |session, text, _| session.country_search = text,
        );
        let content_session = session.clone();
        let picker_id = self.child_id("country-picker");
        let footer_session = session.clone();
        let search_id = self.child_id("country-search");
        let close_id = self.child_id("country-close");
        let sheet = BottomSheetUi::new(
            self.child_id("country-sheet"),
            is_open,
            move |_, cx| {
                let search_text = content_session.read(cx).country_search.clone();
                let closer = content_session.clone();
                let selector = content_session.clone();
                PhoneCountryPickerContent {
                    id: picker_id.clone(),
                    countries: PhoneCountryCatalog::all(),
                    search_text,
                    on_close: Rc::new(move |_, cx| closer.update(cx, |session, cx| session.close_country_picker(cx))),
                    on_select: Rc::new(move |country, window, cx| {
                        let country = country.clone();
                        selector.update(cx, |session, cx| session.select_country(country, window, cx));
                    }),
                }
            },
            move |_, cx| {
                let primary = rise_ui::theme(cx).text.primary;
                let closer = footer_session.clone();
                let search_closer = footer_session.clone();
                hstack(10.)
                    .w_full()
                    .padding_edges(EdgeSet::HORIZONTAL, 12.)
                    .padding_edges(EdgeSet::TOP, 8.)
                    .padding_edges(EdgeSet::BOTTOM, 8.)
                    .child(
                        div().flex_1().min_w_0().child(
                            SearchFieldUi::new(search_id.clone(), search.clone())
                                .placeholder(SharedString::from(tr("country_search_placeholder")))
                                .uses_inline_text_field(true)
                                .on_cancel(move |_, cx| {
                                    search_closer.update(cx, |session, cx| session.close_country_picker(cx));
                                }),
                        ),
                    )
                    .child(
                        LiquidGlassButton::new(close_id.clone(), LiquidGlassButtonShape::Circle)
                            .size(42.)
                            .accessibility_label(Some(tr("close")))
                            .on_press(move |_, cx| closer.update(cx, |session, cx| session.close_country_picker(cx)))
                            .children(IconUi::sized("xmark", px(15.), primary, false)),
                    )
            },
        )
        .snap_fractions(vec![1.])
        .full_page_shows_drag_indicator(false)
        .footer_shows_top_border(false)
        .content_scrollable(false);

        vstack(4.)
            .align(HorizontalAlignment::Leading)
            .relative()
            .w_full()
            .padding_edges(EdgeSet::TOP, self.padding_top)
            .when_some(self.title.clone(), |stack, title| {
                stack.child(MainText::new(title).px(12.).m_l(6.).m_b(3.))
            })
            .child(field_row)
            .when(!error_text.is_empty(), |stack| stack.child(ErrorTextUi::new(error_text).px(12.)))
            .child(div().absolute().top_0().left_0().size_0().child(sheet))
    }
}

struct PhoneCountrySection {
    id: String,
    countries: Vec<PhoneInputCountry>,
}

/// one row of the flattened sectioned list
#[derive(Clone, Copy)]
enum PhoneCountryPickerItem {
    Header { section: usize },
    Country { section: usize, row: usize },
    TailInset,
}

struct PhoneCountryPickerState {
    list: ListState,
    built_for: Option<(String, String)>,
    sections: Rc<Vec<PhoneCountrySection>>,
    items: Rc<Vec<PhoneCountryPickerItem>>,
    header_items: Rc<Vec<usize>>,
    tail_inset: Pixels,
}

impl PhoneCountryPickerState {
    fn new(theme: &AppTheme) -> Self {
        Self {
            list: ListState::new(0, ListAlignment::Top, theme.list.overdraw),
            built_for: None,
            sections: Rc::new(Vec::new()),
            items: Rc::new(Vec::new()),
            header_items: Rc::new(Vec::new()),
            tail_inset: Pixels::ZERO,
        }
    }

    /// sections depend on the query and on the app language the localized names come from
    fn refresh(&mut self, countries: &[PhoneInputCountry], search_text: &str, language: &str) {
        let key = (search_text.to_owned(), language.to_owned());
        if self.built_for.as_ref() == Some(&key) {
            return;
        }
        let sections = PhoneCountryPickerContent::sections(countries, search_text);
        let mut items = Vec::new();
        let mut header_items = Vec::with_capacity(sections.len());
        for (section_index, section) in sections.iter().enumerate() {
            header_items.push(items.len());
            items.push(PhoneCountryPickerItem::Header { section: section_index });
            items.extend(
                (0..section.countries.len()).map(|row| PhoneCountryPickerItem::Country { section: section_index, row }),
            );
        }
        items.push(PhoneCountryPickerItem::TailInset);
        self.list.reset(items.len());
        self.sections = Rc::new(sections);
        self.items = Rc::new(items);
        self.header_items = Rc::new(header_items);
        self.tail_inset = Pixels::ZERO;
        self.built_for = Some(key);
    }

    fn update_tail_inset(&mut self, viewport_height: Pixels) {
        let last_rows = self.sections.last().map_or(0, |section| section.countries.len());
        let rows_height = PhoneCountryPickerContent::COUNTRY_ROW_MINIMUM_HEIGHT * last_rows as f32;
        let inset =
            (viewport_height - px(PhoneCountryPickerContent::SECTION_HEADER_HEIGHT) - px(rows_height)).max(Pixels::ZERO);
        if inset == self.tail_inset {
            return;
        }
        self.tail_inset = inset;
        let last = self.items.len().saturating_sub(1);
        self.list.remeasure_items(last..last + 1);
    }

    fn section_of_item(&self, item: usize) -> Option<usize> {
        match self.items.get(item)? {
            PhoneCountryPickerItem::Header { section } | PhoneCountryPickerItem::Country { section, .. } => Some(*section),
            PhoneCountryPickerItem::TailInset => self.sections.len().checked_sub(1),
        }
    }
}

#[derive(IntoElement)]
struct PhoneCountryPickerContent {
    id: ElementId,
    countries: &'static [PhoneInputCountry],
    search_text: String,
    on_close: PickerClose,
    on_select: PickerSelect,
}

impl PhoneCountryPickerContent {
    const SECTION_HEADER_HEIGHT: f32 = 34.;
    const COUNTRY_ROW_MINIMUM_HEIGHT: f32 = 62.;

    fn sections(countries: &[PhoneInputCountry], search_text: &str) -> Vec<PhoneCountrySection> {
        let query = normalized(search_text);
        let mut matching: Vec<(String, &PhoneInputCountry)> = countries
            .iter()
            .map(|country| (country.localized_name(), country))
            .filter(|(localized_name, country)| {
                if query.is_empty() {
                    return true;
                }
                [
                    localized_name.as_str(),
                    country.english_name.as_str(),
                    country.region_code.as_str(),
                    country.calling_code.as_str(),
                ]
                    .iter()
                    .any(|searchable| normalized(searchable).contains(&query))
            })
            .collect();
        let locale = LanguageStore::current_locale();
        matching.sort_by(|left, right| localized_case_insensitive_compare(&left.0, &right.0, locale.identifier()));

        let mut grouped: BTreeMap<String, Vec<PhoneInputCountry>> = BTreeMap::new();
        for (localized_name, country) in matching {
            let letter = characters(&localized_name).next().map_or_else(|| "#".to_owned(), |first| first.uppercased());
            grouped.entry(letter).or_default().push(country.clone());
        }
        grouped.into_iter().map(|(id, countries)| PhoneCountrySection { id, countries }).collect()
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn section_header(section_id: &str, theme: &AppTheme) -> Div {
        hstack(None)
            .w_full()
            .h(px(Self::SECTION_HEADER_HEIGHT))
            .padding_edges(EdgeSet::HORIZONTAL, 18.)
            .bg(theme.bg._200.opacity(0.97))
            .child(
                MainText::new(section_id.to_owned())
                    .px(14.)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            )
            .child(spacer())
    }

    fn country_row(id: ElementId, country: &PhoneInputCountry, theme: &AppTheme, on_select: PickerSelect) -> AnyElement {
        let localized_name = country.localized_name();
        let shows_english_name = compare_case_insensitive(&localized_name, &country.english_name) != Ordering::Equal;
        let selected = country.clone();
        let separator = div().absolute().bottom_0().h(px(0.5)).bg(theme.border._200.opacity(0.5));
        let separator = if LayoutDirection::current().is_right_to_left() {
            separator.left_0().right(px(60.))
        } else {
            separator.left(px(60.)).right_0()
        };
        div()
            .relative()
            .w_full()
            .child(
                plain(id).on_press(move |window, cx| on_select(&selected, window, cx)).w_full().child(
                    hstack(12.)
                        .w_full()
                        .min_h(px(Self::COUNTRY_ROW_MINIMUM_HEIGHT))
                        .padding_edges(EdgeSet::HORIZONTAL, 16.)
                        .child(
                            div()
                                .flex_none()
                                .w(px(32.))
                                .flex()
                                .justify_center()
                                .swiftui_font(Font::system_size(24.))
                                .child(country.flag()),
                        )
                        .child(
                            vstack(2.)
                                .align(HorizontalAlignment::Leading)
                                .min_w_0()
                                .child(
                                    div()
                                        .swiftui_font(Font::system_size(17.))
                                        .text_color(theme.text.primary)
                                        .truncate()
                                        .child(localized_name),
                                )
                                .when(shows_english_name, |names| {
                                    names.child(
                                        div()
                                            .swiftui_font(Font::system_size(13.))
                                            .text_color(theme.text.secondary)
                                            .truncate()
                                            .child(country.english_name.clone()),
                                    )
                                }),
                        )
                        .child(spacer_min_w(8.))
                        .child(
                            div()
                                .flex_none()
                                .swiftui_font(Font::system_size(17.))
                                .text_color(theme.text.secondary)
                                .child(country.calling_code.clone()),
                        ),
                ),
            )
            .child(separator)
            .into_any_element()
    }
}

impl RenderOnce for PhoneCountryPickerContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let language = LanguageStore::active(cx).map(|store| store.code().to_owned()).unwrap_or_default();
        let state = window.use_keyed_state(self.child_id("list"), cx, |_, cx| {
            PhoneCountryPickerState::new(rise_ui::theme(cx))
        });
        let countries = self.countries;
        let search_text = self.search_text.clone();
        state.update(cx, |state, _| state.refresh(countries, &search_text, &language));

        let on_close = self.on_close.clone();
        let header = SheetHeaderBar::new(self.child_id("header"), tr("country_picker_title"), move |window, cx| {
            on_close(window, cx)
        })
        .right_system_image(None);

        let (list_state, sections, items, header_items) = {
            let state = state.read(cx);
            (state.list.clone(), state.sections.clone(), state.items.clone(), state.header_items.clone())
        };

        if sections.is_empty() {
            return vstack(0.)
                .size_full()
                .bg(theme.bg._200)
                .child(header)
                .child(
                    vstack(10.)
                        .flex_1()
                        .w_full()
                        .justify_center()
                        .children(IconUi::sized("magnifyingglass", px(30.), theme.text.secondary, false))
                        .child(SecondaryText::new(tr("country_search_empty")).px(15.).tac(TextAlignT::Center)),
                );
        }

        let viewport = list_state.viewport_bounds();
        state.update(cx, |state, _| state.update_tail_inset(viewport.size.height));
        let tail_inset = state.read(cx).tail_inset;

        let row_theme = theme.clone();
        let row_items = items.clone();
        let row_sections = sections.clone();
        let on_select = self.on_select.clone();
        let row_id = self.id.clone();
        let rows = list(list_state.clone(), move |index, _, _| match row_items.get(index) {
            Some(PhoneCountryPickerItem::Header { section }) => {
                PhoneCountryPickerContent::section_header(&row_sections[*section].id, &row_theme).into_any_element()
            }
            Some(PhoneCountryPickerItem::Country { section, row }) => {
                let country = &row_sections[*section].countries[*row];
                let id = ElementId::NamedChild(Arc::new(row_id.clone()), SharedString::from(country.id.clone()));
                PhoneCountryPickerContent::country_row(id, country, &row_theme, on_select.clone())
            }
            Some(PhoneCountryPickerItem::TailInset) | None => div().w_full().h(tail_inset).into_any_element(),
        })
        .size_full();

        let top_item = list_state.logical_scroll_top().item_ix;
        let pinned = state.read(cx).section_of_item(top_item).map(|section| {
            let push = header_items
                .get(section + 1)
                .and_then(|next| list_state.bounds_for_item(*next))
                .map_or(Pixels::ZERO, |next| {
                    (next.top() - viewport.top() - px(Self::SECTION_HEADER_HEIGHT)).min(Pixels::ZERO)
                });
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(push)
                .child(Self::section_header(&sections[section].id, &theme))
        });

        let list_column = div().relative().flex_1().min_w_0().h_full().overflow_hidden().child(rows).children(pinned);

        let index = self.search_text.is_empty().then(|| {
            let available = f32::from(viewport.size.height.max(Pixels::ZERO));
            let index_height = px((available / sections.len() as f32).min(14.));
            vstack(0.)
                .flex_none()
                .h_full()
                .justify_center()
                .padding_edges(EdgeSet::TRAILING, 2.)
                .children(sections.iter().enumerate().map(|(section_index, section)| {
                    let jump_state = list_state.clone();
                    let target = header_items[section_index];
                    plain(self.child_id(SharedString::from(format!("index-{}", section.id))))
                        .on_press(move |_, _| {
                            jump_state.scroll_to(ListOffset { item_ix: target, offset_in_item: Pixels::ZERO })
                        })
                        .w(px(22.))
                        .h(index_height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .swiftui_font(Font::system_weight(10., FontWeight::SEMIBOLD))
                                .text_color(theme.primary._100)
                                .child(section.id.clone()),
                        )
                }))
        });

        vstack(0.)
            .size_full()
            .bg(theme.bg._200)
            .child(header)
            .child(hstack(0.).flex_1().w_full().min_h_0().items_start().child(list_column).children(index))
    }
}

/// folds diacritics and case, lowercases and trims
fn normalized(value: &str) -> String {
    let folded = folding(value, CompareOptions::DIACRITIC_INSENSITIVE.union(CompareOptions::CASE_INSENSITIVE)).to_lowercase();
    trimming_whitespaces_and_newlines(&folded).to_owned()
}

/// keeps only numeric characters
fn number_characters(text: &str) -> String {
    characters(text).filter(Character::is_number).map(|character| character.as_str()).collect()
}

fn character_count(text: &str) -> usize {
    characters(text).count()
}

fn drop_first_characters(text: &str, count: usize) -> String {
    characters(text).skip(count).map(|character| character.as_str()).collect()
}

/// first count characters
fn prefix_characters(text: &str, count: usize) -> String {
    characters(text).take(count).map(|character| character.as_str()).collect()
}

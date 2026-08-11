use gpui::{
    App, Context, ElementId, EventEmitter, FocusHandle, Focusable, Global, IntoElement, KeyBinding,
    Render, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window, actions,
    div, prelude::*, uniform_list,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState};
use rise_ui::phone_input::PhoneCountry;
use rise_ui::{FlagUi, IconSize, IconUi, MainText, TextTone, theme};

use crate::country_menu::country_menu_filter::{matches, step_highlight};

/// Scoped so an open picker never eats a shortcut the shell owns.
pub const KEY_CONTEXT: &str = "CountryMenu";

const CHECKMARK: &str = "checkmark";

/// Existing keys, already translated in all 41 locales.
const TITLE_KEY: &str = "country_picker_title";
const SEARCH_PLACEHOLDER_KEY: &str = "country_search_placeholder";
const EMPTY_KEY: &str = "country_search_empty";

actions!(
    rise_country_menu,
    [
        ChooseHighlighted,
        DismissCountryMenu,
        HighlightNextCountry,
        HighlightPreviousCountry,
        HighlightFirstCountry,
        HighlightLastCountry,
    ]
);

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

/// Idempotent, and called by [`CountryMenu::new`].
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);

    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("down", HighlightNextCountry, context),
        KeyBinding::new("up", HighlightPreviousCountry, context),
        KeyBinding::new("home", HighlightFirstCountry, context),
        KeyBinding::new("end", HighlightLastCountry, context),
        KeyBinding::new("enter", ChooseHighlighted, context),
        KeyBinding::new("escape", DismissCountryMenu, context),
    ]);
}

/// What leaves the picker. It never decides whether it is on screen — the field
/// that opened it does — so even dismissal is an event.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CountryMenuEvent {
    Chose(PhoneCountry),
    Dismissed,
}

/// The country picker: a floating list with a query field, the current country
/// ticked, and the keyboard working. It owns its copy of the catalogue, since
/// the filtered view is indices into whatever it was given.
pub struct CountryMenu {
    query: Entity<InputUiState>,
    countries: Vec<PhoneCountry>,
    /// Indices into `countries`, in the order they are drawn.
    visible: Vec<usize>,
    highlighted: usize,
    selected: String,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

use gpui::Entity;

impl EventEmitter<CountryMenuEvent> for CountryMenu {}

impl Focusable for CountryMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CountryMenu {
    pub fn new(countries: Vec<PhoneCountry>, selected: &str, cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);

        let query = cx.new(|cx| {
            let mut state = InputUiState::new(InputMode::SingleLine, cx);
            state.set_placeholder(tr(SEARCH_PLACEHOLDER_KEY), cx);
            state
        });

        let subscription = cx.subscribe(&query, |menu, _, event, cx| match event {
            InputUiEvent::Changed => menu.requery(cx),
            InputUiEvent::Submitted => menu.choose_highlighted(cx),
            InputUiEvent::Cancelled => cx.emit(CountryMenuEvent::Dismissed),
        });

        let visible = (0..countries.len()).collect::<Vec<_>>();
        let highlighted = countries
            .iter()
            .position(|country| country.region == selected)
            .unwrap_or(0);

        Self {
            query,
            countries,
            visible,
            highlighted,
            selected: selected.to_owned(),
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    /// The picker opens with the caret in the field.
    pub fn query_focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }

    /// Puts the highlight on screen. Call it once the picker has been laid out —
    /// the scroll handle has no rows before then.
    pub fn reveal_selection(&self) {
        self.scroll
            .scroll_to_item(self.highlighted, ScrollStrategy::Center);
    }

    pub fn highlighted_country(&self) -> Option<&PhoneCountry> {
        self.countries.get(*self.visible.get(self.highlighted)?)
    }

    #[cfg(test)]
    pub fn visible_regions(&self) -> Vec<&str> {
        self.visible
            .iter()
            .filter_map(|index| self.countries.get(*index))
            .map(|country| country.region.as_str())
            .collect()
    }

    #[cfg(test)]
    pub fn highlighted_index(&self) -> usize {
        self.highlighted
    }

    fn requery(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).text().to_owned();
        self.visible = matches(&self.countries, &query);
        // Back to the top: the old row number now points at another country.
        self.highlighted = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn choose_highlighted(&mut self, cx: &mut Context<Self>) {
        if let Some(country) = self.highlighted_country().cloned() {
            cx.emit(CountryMenuEvent::Chose(country));
        }
    }

    fn choose_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(country) = self
            .visible
            .get(row)
            .and_then(|index| self.countries.get(*index))
            .cloned()
        else {
            return;
        };
        cx.emit(CountryMenuEvent::Chose(country));
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let next = step_highlight(self.highlighted, delta, self.visible.len());
        if next == self.highlighted {
            return;
        }
        self.highlighted = next;
        self.scroll.scroll_to_item(next, ScrollStrategy::Top);
        cx.notify();
    }

    fn on_next(&mut self, _: &HighlightNextCountry, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(1, cx);
    }

    fn on_previous(
        &mut self,
        _: &HighlightPreviousCountry,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_highlight(-1, cx);
    }

    fn on_first(&mut self, _: &HighlightFirstCountry, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(isize::MIN, cx);
    }

    fn on_last(&mut self, _: &HighlightLastCountry, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(isize::MAX, cx);
    }

    fn on_choose(&mut self, _: &ChooseHighlighted, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_highlighted(cx);
    }

    fn on_dismiss(&mut self, _: &DismissCountryMenu, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CountryMenuEvent::Dismissed);
    }

    fn row(
        &self,
        theme: &AppTheme,
        row: usize,
        country: &PhoneCountry,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let highlighted = row == self.highlighted;
        let ticked = country.region == self.selected;
        let label = theme.typography.body();

        let mut element = div()
            // The region, never the row number: the list reorders on every
            // keystroke, and an index id would swap two countries' element state.
            .id(ElementId::Name(SharedString::from(country.region.clone())))
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing._300)
            .h(theme.shell.palette_row_height)
            .px(theme.spacing._400)
            .mx(theme.spacing._200)
            .rounded(theme.radius._200)
            .text_size(label.size)
            .line_height(label.line_height)
            .font(label.font)
            .when(highlighted, |row| row.bg(theme.bg._400))
            .cursor_pointer()
            .on_click(cx.listener(move |menu, _event, _window, cx| menu.choose_row(row, cx)))
            .on_hover(cx.listener(move |menu, hovered: &bool, _window, cx| {
                if *hovered && menu.highlighted != row {
                    menu.highlighted = row;
                    cx.notify();
                }
            }))
            .child(FlagUi::render(
                theme,
                &country.region,
                theme.auth.flag_width,
            ))
            .child(
                MainText::body(theme, TextTone::Primary)
                    .flex_1()
                    .truncate()
                    .child(country.english_name.clone()),
            )
            .child(MainText::body(theme, TextTone::Secondary).child(country.display_code()));

        if ticked
            && let Some(mark) =
                IconUi::render(theme, CHECKMARK, IconSize::Small, theme.text.primary)
        {
            element = element.child(mark);
        }

        element
    }
}

impl Render for CountryMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme: AppTheme = theme(cx as &App).clone();
        let metrics = theme.shell;
        let rows = self.visible.clone();
        let entity = cx.entity();

        // Bounded by the window: a picker taller than it has no outside to click.
        let viewport = window.viewport_size();
        let height = (viewport.height * DIALOG_VIEWPORT_SHARE).min(metrics.palette_max_height);
        let width = metrics
            .palette_width
            .min(viewport.width - metrics.overlay_margin * 2.0);

        let list = uniform_list("country.rows", rows.len(), {
            let theme = theme.clone();
            move |range, _window, cx| {
                entity.update(cx, |menu, cx| {
                    range
                        .filter_map(|row| {
                            let country = menu.countries.get(*menu.visible.get(row)?)?.clone();
                            Some(menu.row(&theme, row, &country, cx).into_any_element())
                        })
                        .collect()
                })
            }
        })
        .track_scroll(&self.scroll)
        .flex_1();

        let title = theme.typography.headline();

        let mut panel = div()
            // Without this a click on a row also lands on the dismiss scrim.
            .occlude()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_first))
            .on_action(cx.listener(Self::on_last))
            .on_action(cx.listener(Self::on_choose))
            .on_action(cx.listener(Self::on_dismiss))
            .flex()
            .flex_col()
            .w(width)
            .h(height)
            // Opaque, never a `GlassPanel`: a native material is an AppKit view
            // below the Metal layer, so the page would show through the list.
            .bg(theme.bg._100)
            .border_1()
            .border_color(theme.border._200)
            .rounded(theme.radius._400)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.spacing._300)
                    .p(theme.spacing._400)
                    .child(
                        div()
                            .text_size(title.size)
                            .line_height(title.line_height)
                            .font(title.font)
                            .text_color(theme.text.primary)
                            .child(tr(TITLE_KEY)),
                    )
                    .child(self.query.clone()),
            );

        panel = if rows.is_empty() {
            panel.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(theme.spacing._400)
                    .child(MainText::body(&theme, TextTone::Secondary).child(tr(EMPTY_KEY))),
            )
        } else {
            panel.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .pb(theme.spacing._200)
                    .child(list),
            )
        };

        panel
    }
}

/// How much of the window's height the picker may take.
const DIALOG_VIEWPORT_SHARE: f32 = 0.7;

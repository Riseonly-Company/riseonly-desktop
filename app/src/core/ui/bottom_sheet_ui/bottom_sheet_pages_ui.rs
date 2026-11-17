use std::rc::Rc;

use gpui::{App, ElementId, Empty, Entity, Hsla, Window};
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetNavigationStateKey, BottomSheetScreenItem,
};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_page_deck::BottomSheetPageDeck;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;

type PagesAction = Rc<dyn Fn(&mut Window, &mut App)>;
type ScreenChange = Rc<dyn Fn(&str, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct BottomSheetPagesUi {
    id: ElementId,
    is_bottom_sheet: Binding<bool>,
    pages: Vec<BottomSheetScreenItem>,
    root_page_id: String,
    initial_page_ids: Vec<String>,
    allows_swipe_dismiss: bool,
    dismisses_on_backdrop_tap: bool,
    shows_drag_indicator: bool,
    shows_close_button: bool,
    interaction_locked: bool,
    sheet_background: Option<Hsla>,
    on_present: PagesAction,
    on_dismiss: PagesAction,
    on_did_dismiss: PagesAction,
    on_presentation_refused: PagesAction,
    on_screen_change: Option<ScreenChange>,
    modal_sized_by_detent: bool,
}

/// state of the sheet
struct PagesState {
    navigation: Entity<BottomSheetNavigationState>,
    active_fraction: f32,
    transition_duration: f64,
    presentation_token: u64,
}

#[derive(Clone, PartialEq)]
struct PageFraction {
    page_id: String,
    fraction: f32,
}

impl PagesState {
    /// page fraction plus the docked keyboard fraction, on desktop the keyboard fraction is zero
    fn presented_fraction(&self) -> f32 {
        self.active_fraction.min(0.98)
    }
}

impl BottomSheetPagesUi {
    pub fn new(
        id: impl Into<ElementId>,
        is_bottom_sheet: Binding<bool>,
        pages: Vec<BottomSheetScreenItem>,
        root_page_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            is_bottom_sheet,
            pages,
            root_page_id: root_page_id.into(),
            initial_page_ids: Vec::new(),
            allows_swipe_dismiss: true,
            dismisses_on_backdrop_tap: true,
            shows_drag_indicator: true,
            shows_close_button: true,
            interaction_locked: false,
            sheet_background: None,
            on_present: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_did_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_presentation_refused: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_screen_change: None,
            modal_sized_by_detent: false,
        }
    }

    pub fn initial_page_ids(mut self, initial_page_ids: Vec<String>) -> Self {
        self.initial_page_ids = initial_page_ids;
        self
    }

    pub fn allows_swipe_dismiss(mut self, allows: bool) -> Self {
        self.allows_swipe_dismiss = allows;
        self
    }

    pub fn dismisses_on_backdrop_tap(mut self, dismisses: bool) -> Self {
        self.dismisses_on_backdrop_tap = dismisses;
        self
    }

    pub fn shows_drag_indicator(mut self, shows: bool) -> Self {
        self.shows_drag_indicator = shows;
        self
    }

    pub fn shows_close_button(mut self, shows: bool) -> Self {
        self.shows_close_button = shows;
        self
    }

    pub fn interaction_locked(mut self, locked: bool) -> Self {
        self.interaction_locked = locked;
        self
    }

    pub fn sheet_background(mut self, background: Option<Hsla>) -> Self {
        self.sheet_background = background;
        self
    }

    pub fn modal_sized_by_detent(mut self, sized: bool) -> Self {
        self.modal_sized_by_detent = sized;
        self
    }

    pub fn on_present(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_present = Rc::new(action);
        self
    }

    pub fn on_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(action);
        self
    }

    pub fn on_did_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_did_dismiss = Rc::new(action);
        self
    }

    pub fn on_presentation_refused(
        mut self,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_presentation_refused = Rc::new(action);
        self
    }

    pub fn on_screen_change(
        mut self,
        action: Option<impl Fn(&str, &mut Window, &mut App) + 'static>,
    ) -> Self {
        self.on_screen_change = action.map(|action| Rc::new(action) as ScreenChange);
        self
    }

    /// window bottom safe area, desktop has none
    pub fn window_bottom_inset() -> f32 {
        0.0
    }

    fn fraction(page_id: &str, pages: &[BottomSheetScreenItem]) -> f32 {
        pages
            .iter()
            .find(|page| page.id == page_id)
            .or(pages.first())
            .map_or(0.4, |page| page.fraction)
    }

    fn opening_page_id(&self) -> &str {
        self.initial_page_ids.last().map_or(self.root_page_id.as_str(), String::as_str)
    }

    fn prepare_for_presentation(
        state: &Entity<PagesState>,
        fraction: f32,
        on_present: &PagesAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| {
            state.transition_duration = BottomSheetMetrics::PAGE_TRANSITION_DURATION;
            state.active_fraction = fraction;
            state.presentation_token = state.presentation_token.wrapping_add(1);
            cx.notify();
        });
        on_present(window, cx);
    }
}

impl RenderOnce for BottomSheetPagesUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let opening_fraction = Self::fraction(self.opening_page_id(), &self.pages);
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, move |_, cx| {
            let navigation = BottomSheetNavigationState::new(cx);
            cx.observe(&navigation, |_, _, cx| cx.notify()).detach();
            PagesState {
                navigation,
                active_fraction: opening_fraction,
                transition_duration: BottomSheetMetrics::PAGE_TRANSITION_DURATION,
                presentation_token: 0,
            }
        });

        let background = {
            let theme_store = ThemeStore::shared(cx);
            let theme_bg = theme_store.read(cx).current_theme().bg._100;
            self.sheet_background.unwrap_or(theme_bg)
        };
        let (navigation, sheet_fraction, transition_duration, current_screen_id) = {
            let state = state.read(cx);
            (
                state.navigation.clone(),
                state.presented_fraction(),
                state.transition_duration,
                state.navigation.read(cx).current_screen_id().to_owned(),
            )
        };

        let read_fraction = state.clone();
        let write_fraction = state.clone();
        let selected_snap_fraction = Binding::new(
            move |cx| read_fraction.read(cx).presented_fraction(),
            move |new_value, _, cx| {
                write_fraction.update(cx, |state, cx| {
                    state.active_fraction = new_value;
                    cx.notify();
                });
            },
        );

        let deck_state = state.clone();
        let pages = self.pages.clone();
        let root_page_id = self.root_page_id.clone();
        let initial_page_ids = self.initial_page_ids.clone();
        let deck_id = self.id.clone();
        let is_locked = self.interaction_locked;
        let shows_close_button = self.shows_close_button;
        let close_binding = self.is_bottom_sheet.clone();
        let sheet_content = move |_: &mut Window, cx: &mut App| {
            let token = deck_state.read(cx).presentation_token;
            let transition = deck_state.clone();
            let close = close_binding.clone();
            let deck = BottomSheetPageDeck {
                id: (deck_id.clone(), "deck").into(),
                pages: pages.clone(),
                root_page_id: root_page_id.clone(),
                initial_page_ids: initial_page_ids.clone(),
                navigation: navigation.clone(),
                background,
                bottom_inset: Self::window_bottom_inset(),
                is_locked,
                shows_close_button,
                on_close: Rc::new(move |window: &mut Window, cx: &mut App| close.set(false, window, cx)),
                on_transition: Rc::new(move |fraction: f32, duration: f64, _: &mut Window, cx: &mut App| {
                    transition.update(cx, |state, cx| {
                        state.transition_duration = duration;
                        state.active_fraction = fraction;
                        cx.notify();
                    });
                }),
            };
            div()
                .id((deck_id.clone(), format!("presentation-{token}")))
                .size_full()
                .child(env_scope::<BottomSheetNavigationStateKey>(Some(navigation.clone())).child(deck))
        };

        let on_dismiss = self.on_dismiss.clone();
        let on_did_dismiss = self.on_did_dismiss.clone();
        let on_presentation_refused = self.on_presentation_refused.clone();
        let sheet = BottomSheetUi::new((self.id.clone(), "sheet"), self.is_bottom_sheet.clone(), sheet_content, |_, _| Empty)
            .snap_fractions(vec![sheet_fraction])
            .selected_snap_fraction(Some(selected_snap_fraction))
            .snap_animation_duration(transition_duration)
            .full_page_allows_swipe_dismiss(self.allows_swipe_dismiss)
            .dismisses_on_backdrop_tap(self.dismisses_on_backdrop_tap)
            .dismisses_keyboard_before_backdrop_dismiss(true)
            .full_page_shows_drag_indicator(self.shows_drag_indicator)
            .full_page_hides_footer(true)
            .full_page_avoids_keyboard_when_footer_hidden(true)
            .footer_shows_top_border(false)
            .content_scrollable(false)
            .full_page_interaction_locked(self.interaction_locked)
            .modal_sized_by_detent(self.modal_sized_by_detent)
            .sheet_background(Some(background))
            .footer_background(Some(background))
            .on_dismiss(move |window, cx| on_dismiss(window, cx))
            .on_did_dismiss(move |window, cx| on_did_dismiss(window, cx))
            .on_presentation_refused(move |window, cx| on_presentation_refused(window, cx));

        let current_screen_id_for_fraction = current_screen_id.clone();
        if let Some(on_screen_change) = self.on_screen_change.clone() {
            on_change(
                (self.id.clone(), "current-screen"),
                current_screen_id,
                true,
                window,
                cx,
                move |_, screen_id, window, cx| on_screen_change(screen_id, window, cx),
            );
        }

        let is_open = self.is_bottom_sheet.get(cx);
        let page_fraction = PageFraction {
            page_id: current_screen_id_for_fraction.clone(),
            fraction: Self::fraction(&current_screen_id_for_fraction, &self.pages),
        };
        let fraction_state = state.clone();
        let fraction_binding = self.is_bottom_sheet.clone();
        on_change((self.id.clone(), "page-fraction"), page_fraction, false, window, cx, move |old, new, _, cx| {
            let fraction = new.fraction;
            let is_open = fraction_binding.get(cx);
            fraction_state.update(cx, |state, cx| {
                if old.page_id != new.page_id || !is_open || (state.active_fraction - fraction).abs() <= 0.001 {
                    return;
                }
                state.transition_duration = BottomSheetMetrics::SNAP_DURATION;
                state.active_fraction = fraction;
                cx.notify();
            });
        });

        let present_state = state.clone();
        let present_action = self.on_present.clone();
        on_change((self.id.clone(), "is-bottom-sheet"), is_open, false, window, cx, move |_, is_open, window, cx| {
            if *is_open {
                Self::prepare_for_presentation(&present_state, opening_fraction, &present_action, window, cx);
            }
        });

        let appear_state = state;
        let appear_action = self.on_present.clone();
        let appear_binding = self.is_bottom_sheet.clone();
        sheet.on_appear((self.id, "appear"), move |window, cx| {
            let is_open = appear_binding.get(cx);
            if is_open && appear_state.read(cx).presentation_token == 0 {
                Self::prepare_for_presentation(&appear_state, opening_fraction, &appear_action, window, cx);
            }
        })
    }
}

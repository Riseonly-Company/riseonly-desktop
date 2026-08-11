//! A click on a settings row must reach that row, in both surfaces — a
//! wrapperless row that stopped being hit-testable looks identical.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Context, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext, Window};
use gpui::{div, point, prelude::*, px, size};
use rise_theme::{AppTheme, SettingsIconPalette};
use rise_widgets::{GroupedActivate, GroupedBtn, GroupedBtns, GroupedEndAction};

/// A row's height, read off the theme rather than written down here.
fn row_height() -> f32 {
    f32::from(AppTheme::dark().grouped.row_height())
}

/// The hairline the card draws between two rows.
fn hairline() -> f32 {
    f32::from(AppTheme::dark().grouped.separator_height)
}

/// The end action has no chip, so its height is its label plus its own padding.
fn end_action_height() -> f32 {
    let theme = AppTheme::dark();
    let label = theme
        .typography
        .style(theme.grouped.end_title_size, gpui::FontWeight::SEMIBOLD);

    f32::from(label.line_height) + f32::from(theme.grouped.end_padding_y) * 2.0
}
/// Fixed rather than maximized, so a coordinate means the same on every machine.
const WINDOW: (f32, f32) = (500.0, 700.0);
/// Over the label. Past the group's 12pt inset and the row's own 16pt padding.
const OVER_LABEL: f32 = 120.0;
/// Over the chevron at the far end of the row, inside the card's 12pt inset.
const OVER_CHEVRON: f32 = WINDOW.0 - 20.0;

struct Harness {
    activated: Rc<RefCell<Vec<String>>>,
    bare: bool,
}

impl Harness {
    fn rows() -> Vec<GroupedBtn> {
        vec![
            GroupedBtn::new("profile", "Мой профиль")
                .icon("person.fill", SettingsIconPalette::profile()),
            GroupedBtn::new("sessions", "Активные сессии")
                .icon("iphone", SettingsIconPalette::sessions())
                .trailing("4"),
            GroupedBtn::new("blocked", "Заблокированные")
                .icon("hand.raised.fill", SettingsIconPalette::blocks())
                .disabled(),
        ]
    }

    /// Row `index` in the card, where every row but the last is followed by a
    /// hairline.
    fn card_row_center(index: usize) -> f32 {
        (row_height() + hairline()) * index as f32 + row_height() / 2.0
    }

    /// The same, without the card: no hairlines, so the rows stack flush.
    fn bare_row_center(index: usize) -> f32 {
        row_height() * index as f32 + row_height() / 2.0
    }

    /// Three rows and two hairlines, then the end action's own hairline.
    fn end_action_center() -> f32 {
        row_height() * 3.0 + hairline() * 3.0 + end_action_height() / 2.0
    }
}

impl Render for Harness {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme: AppTheme = rise_ui::theme(cx as &gpui::App).clone();
        let seen = self.activated.clone();

        let activate: GroupedActivate<Self> = Rc::new(move |_harness, id, _window, _cx| {
            seen.borrow_mut().push(id.to_string());
        });

        let group = GroupedBtns::new("settings.test", Self::rows())
            .end_action(GroupedEndAction::new("delete_account", "Удалить аккаунт"));
        let group = if self.bare {
            group.without_wrapper()
        } else {
            group
        };

        div().size_full().child(group.render(&theme, &activate, cx))
    }
}

fn open(bare: bool, cx: &mut TestAppContext) -> (Rc<RefCell<Vec<String>>>, VisualTestContext) {
    cx.update(|cx| {
        if !cx.has_global::<AppTheme>() {
            rise_ui::install_theme(AppTheme::dark(), cx);
        }
    });

    let activated = Rc::new(RefCell::new(Vec::new()));
    let window = cx.open_window(size(px(WINDOW.0), px(WINDOW.1)), |_, _| Harness {
        activated: activated.clone(),
        bare,
    });

    let visual = VisualTestContext::from_window(window.into(), cx);
    (activated, visual)
}

fn click_at(visual: &mut VisualTestContext, x: f32, y: f32) {
    visual.simulate_click(point(px(x), px(y)), Modifiers::default());
}

fn click(visual: &mut VisualTestContext, y: f32) {
    click_at(visual, OVER_LABEL, y);
}

#[gpui::test]
fn a_click_reports_the_row_it_landed_on(cx: &mut TestAppContext) {
    let (activated, mut visual) = open(false, cx);

    click(&mut visual, Harness::card_row_center(0));
    click(&mut visual, Harness::card_row_center(1));

    assert_eq!(*activated.borrow(), vec!["profile", "sessions"]);
}

#[gpui::test]
fn the_whole_row_is_the_target_and_not_just_its_label(cx: &mut TestAppContext) {
    let (activated, mut visual) = open(false, cx);

    // At the far end of the row, over the chevron and past the trailing value.
    click_at(&mut visual, OVER_CHEVRON, Harness::card_row_center(1));

    assert_eq!(*activated.borrow(), vec!["sessions"]);
}

#[gpui::test]
fn a_disabled_row_refuses_the_pointer(cx: &mut TestAppContext) {
    let (activated, mut visual) = open(false, cx);

    click(&mut visual, Harness::card_row_center(2));

    assert!(
        activated.borrow().is_empty(),
        "a dimmed row still had a live click handler behind it"
    );
}

#[gpui::test]
fn the_end_action_leaves_through_the_same_handler(cx: &mut TestAppContext) {
    let (activated, mut visual) = open(false, cx);

    click(&mut visual, Harness::end_action_center());

    assert_eq!(*activated.borrow(), vec!["delete_account"]);
}

#[gpui::test]
fn a_wrapperless_row_is_still_a_target_even_though_it_paints_nothing(cx: &mut TestAppContext) {
    let (activated, mut visual) = open(true, cx);

    click(&mut visual, Harness::bare_row_center(0));
    click(&mut visual, Harness::bare_row_center(1));

    assert_eq!(
        *activated.borrow(),
        vec!["profile", "sessions"],
        "without the card there is no background under the row, and a row that \
         stopped being hit-testable would look exactly the same"
    );
}

#[gpui::test]
fn taking_the_wrapper_away_moves_the_rows_up_by_exactly_the_hairlines(cx: &mut TestAppContext) {
    // The third row is disabled, so its top edge is a boundary a click can read.
    let card_top = Harness::card_row_center(2) - row_height() / 2.0;
    let bare_top = Harness::bare_row_center(2) - row_height() / 2.0;

    assert_eq!(
        card_top - bare_top,
        hairline() * 2.0,
        "two rows above means two hairlines of difference, and nothing else"
    );

    let (card, mut card_visual) = open(false, cx);
    click(&mut card_visual, card_top - 2.0);
    click(&mut card_visual, card_top + 2.0);
    assert_eq!(
        *card.borrow(),
        vec!["sessions"],
        "the boundary is not where the hairlines say it is"
    );

    let (bare, mut bare_visual) = open(true, cx);
    click(&mut bare_visual, bare_top - 2.0);
    click(&mut bare_visual, bare_top + 2.0);
    assert_eq!(
        *bare.borrow(),
        vec!["sessions"],
        "without the card the rows sit flush, and this one is 1pt higher per row above it"
    );
}

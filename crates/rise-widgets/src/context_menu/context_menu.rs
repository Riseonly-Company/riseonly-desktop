use gpui::{
    App, Context, ElementId, EventEmitter, FocusHandle, Focusable, Global, IntoElement, KeyBinding,
    Pixels, Point, Render, SharedString, Size, Window, actions, div, prelude::*, size,
};
use rise_platform::materials::Material;
use rise_theme::AppTheme;
use rise_ui::{IconSize, IconUi, theme};

use crate::context_menu::context_menu_item::{
    ContextMenuEntry, ContextMenuItem, ContextMenuTone, HighlightDirection, next_enabled_index,
};
use crate::glass_panel::GlassPanel;
use crate::overlay::OverlayAnchor;

/// The key context the bindings below are scoped to, so an open menu never eats
/// a shortcut the shell owns and a closed one never sees one at all.
pub const KEY_CONTEXT: &str = "ContextMenu";

/// The reference marks a checked row with `Image(systemName: "checkmark")`.
const CHECKMARK: &str = "checkmark";

actions!(
    rise_context_menu,
    [
        ActivateHighlighted,
        DismissMenu,
        HighlightFirst,
        HighlightLast,
        HighlightNext,
        HighlightPrevious,
    ]
);

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

/// Idempotent, and safe to call from anywhere. [`ContextMenuState::new`] calls
/// it, so a menu works in an app that never heard of it.
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);

    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("down", HighlightNext, context),
        KeyBinding::new("up", HighlightPrevious, context),
        KeyBinding::new("home", HighlightFirst, context),
        KeyBinding::new("end", HighlightLast, context),
        KeyBinding::new("enter", ActivateHighlighted, context),
        KeyBinding::new("escape", DismissMenu, context),
    ]);
}

/// What leaves a menu.
///
/// An id rather than a callback stored in the item: a menu that carries closures
/// is a value that cannot be compared, cannot be asserted on and cannot be sent
/// anywhere. The id is the reference's `key`, so the handler on the other side
/// is the same match the phone runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContextMenuEvent {
    Activated(SharedString),
    Dismissed,
}

pub struct ContextMenuState {
    region_key: &'static str,
    entries: Vec<ContextMenuEntry>,
    anchor: OverlayAnchor,
    highlighted: Option<usize>,
    selected: Option<SharedString>,
    focus_handle: FocusHandle,
}

impl EventEmitter<ContextMenuEvent> for ContextMenuState {}

impl Focusable for ContextMenuState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ContextMenuState {
    /// `region_key` names the menu's surface for the lifetime of the window; two
    /// menus that can be open at once must not share one.
    pub fn new(region_key: &'static str, cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);

        Self {
            region_key,
            entries: Vec::new(),
            anchor: OverlayAnchor::Point(Point::default()),
            highlighted: None,
            selected: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn open(
        &mut self,
        entries: Vec<ContextMenuEntry>,
        anchor: OverlayAnchor,
        cx: &mut Context<Self>,
    ) {
        self.entries = entries;
        self.anchor = anchor;
        self.highlighted = None;
        cx.notify();
    }

    /// The item the reference draws a checkmark beside — `ContextMenuUi`'s
    /// `selected`, matched against the item's stable id.
    pub fn set_selected(&mut self, selected: Option<SharedString>, cx: &mut Context<Self>) {
        if self.selected == selected {
            return;
        }
        self.selected = selected;
        cx.notify();
    }

    pub fn set_anchor(&mut self, anchor: OverlayAnchor, cx: &mut Context<Self>) {
        if self.anchor == anchor {
            return;
        }
        self.anchor = anchor;
        cx.notify();
    }

    pub fn entries(&self) -> &[ContextMenuEntry] {
        &self.entries
    }

    pub fn anchor(&self) -> OverlayAnchor {
        self.anchor
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    pub fn selected(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    pub fn region_key(&self) -> &'static str {
        self.region_key
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    /// The size this menu ASKS for, to hand to [`crate::overlay::place`].
    ///
    /// An ask, not a measurement. The real width is whatever the label runs need
    /// between `menu_min_width` and `menu_max_width`, and nothing can report it
    /// before layout — so placement works from the widest the menu could be, and
    /// `place` clamping to the viewport is what keeps the answer honest in a
    /// window narrower than the ask.
    pub fn asked_size(&self, theme: &AppTheme) -> Size<Pixels> {
        let padding = theme.spacing._200 * 2.0;
        let gaps = theme.spacing._100 * self.entries.len().saturating_sub(1) as f32;

        let content = self.entries.iter().fold(Pixels::ZERO, |total, entry| {
            total
                + match entry {
                    ContextMenuEntry::Item(_) => theme.shell.menu_row_height,
                    ContextMenuEntry::Separator => {
                        theme.shell.overlay_gap / 4.0 + theme.spacing._100 * 2.0
                    }
                }
        });

        size(theme.shell.menu_max_width, padding + gaps + content)
    }

    pub fn highlight(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let index = index.filter(|index| {
            self.entries
                .get(*index)
                .is_some_and(ContextMenuEntry::is_activatable)
        });
        if self.highlighted == index {
            return;
        }
        self.highlighted = index;
        cx.notify();
    }

    pub fn move_highlight(&mut self, direction: HighlightDirection, cx: &mut Context<Self>) {
        let next = next_enabled_index(&self.entries, self.highlighted, direction);
        self.set_highlight(next, cx);
    }

    pub fn highlight_edge(&mut self, direction: HighlightDirection, cx: &mut Context<Self>) {
        let next = next_enabled_index(&self.entries, None, direction);
        self.set_highlight(next, cx);
    }

    /// Emits [`ContextMenuEvent::Activated`] for whatever is highlighted, and
    /// nothing at all when nothing is.
    pub fn activate_highlighted(&mut self, cx: &mut Context<Self>) -> Option<SharedString> {
        self.activate_index(self.highlighted?, cx)
    }

    pub fn activate_index(&mut self, index: usize, cx: &mut Context<Self>) -> Option<SharedString> {
        let item = self.entries.get(index)?.as_item()?;
        if !item.enabled {
            return None;
        }

        let id = item.id.clone();
        cx.emit(ContextMenuEvent::Activated(id.clone()));
        Some(id)
    }

    /// Dismissal is an event too: the menu never decides whether it is on
    /// screen, so the thing that opened it is the thing that takes it away.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(ContextMenuEvent::Dismissed);
    }

    fn set_highlight(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if self.highlighted == index {
            return;
        }
        self.highlighted = index;
        cx.notify();
    }

    fn hover(&mut self, index: usize, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.highlight(Some(index), cx);
        } else if self.highlighted == Some(index) {
            self.highlight(None, cx);
        }
    }

    fn on_highlight_next(&mut self, _: &HighlightNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(HighlightDirection::Forward, cx);
    }

    fn on_highlight_previous(
        &mut self,
        _: &HighlightPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_highlight(HighlightDirection::Backward, cx);
    }

    fn on_highlight_first(&mut self, _: &HighlightFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.highlight_edge(HighlightDirection::Forward, cx);
    }

    fn on_highlight_last(&mut self, _: &HighlightLast, _: &mut Window, cx: &mut Context<Self>) {
        self.highlight_edge(HighlightDirection::Backward, cx);
    }

    fn on_activate(&mut self, _: &ActivateHighlighted, _: &mut Window, cx: &mut Context<Self>) {
        self.activate_highlighted(cx);
    }

    fn on_dismiss(&mut self, _: &DismissMenu, _: &mut Window, cx: &mut Context<Self>) {
        self.close(cx);
    }

    fn separator(theme: &AppTheme) -> impl IntoElement {
        div()
            .h(theme.shell.overlay_gap / 4.0)
            .mx(theme.spacing._300)
            .my(theme.spacing._100)
            .rounded(theme.radius._100)
            .bg(theme.border._200)
    }

    fn row(
        theme: &AppTheme,
        index: usize,
        item: &ContextMenuItem,
        state: RowState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tint = match (item.enabled, item.tone) {
            (false, _) => theme.text.secondary,
            (true, ContextMenuTone::Normal) => theme.text.primary,
            (true, ContextMenuTone::Destructive) => theme.semantic.error_100,
        };
        let label = theme.typography.body_strong();

        let mut row = div()
            // The item's stable id, never the loop index: a menu that reorders
            // would otherwise hand one row's element state to another.
            .id(ElementId::Name(item.id.clone()))
            .flex()
            .items_center()
            .gap(theme.spacing._500)
            .h(theme.shell.menu_row_height)
            .px(theme.spacing._500)
            .rounded(theme.radius._100)
            .text_size(label.size)
            .line_height(label.line_height)
            .font(label.font)
            .text_color(tint)
            .when(state.highlighted, |row| row.bg(theme.bg._400));

        if let Some(symbol) = item.sf_symbol
            && let Some(icon) = IconUi::render(theme, symbol, IconSize::Small, tint)
        {
            row = row.child(icon);
        }

        row = row.child(div().flex_1().truncate().child(item.label()));

        if state.checked
            && let Some(mark) = IconUi::render(theme, CHECKMARK, IconSize::Small, tint)
        {
            row = row.child(mark);
        }

        if item.enabled {
            row = row
                .on_click(cx.listener(move |menu, _event, _window, cx| {
                    menu.activate_index(index, cx);
                }))
                .on_hover(cx.listener(move |menu, hovered: &bool, _window, cx| {
                    menu.hover(index, *hovered, cx);
                }));
        }

        row
    }
}

#[derive(Clone, Copy)]
struct RowState {
    highlighted: bool,
    checked: bool,
}

impl Render for ContextMenuState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme: AppTheme = theme(cx).clone();
        let entries = self.entries.clone();
        let highlighted = self.highlighted;
        let selected = self.selected.clone();

        let mut panel = GlassPanel::surface(self.region_key, Material::Overlay, cx)
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_highlight_next))
            .on_action(cx.listener(Self::on_highlight_previous))
            .on_action(cx.listener(Self::on_highlight_first))
            .on_action(cx.listener(Self::on_highlight_last))
            .on_action(cx.listener(Self::on_activate))
            .on_action(cx.listener(Self::on_dismiss))
            .flex()
            .flex_col()
            .min_w(theme.shell.menu_min_width)
            .max_w(theme.shell.menu_max_width)
            .p(theme.spacing._200)
            .gap(theme.spacing._100);

        for (index, entry) in entries.iter().enumerate() {
            panel = match entry {
                ContextMenuEntry::Separator => panel.child(Self::separator(&theme)),
                ContextMenuEntry::Item(item) => panel.child(Self::row(
                    &theme,
                    index,
                    item,
                    RowState {
                        highlighted: highlighted == Some(index),
                        checked: selected.as_ref() == Some(&item.id),
                    },
                    cx,
                )),
            };
        }

        panel
    }
}

#[cfg(test)]
mod tests {
    use super::HighlightDirection::{Backward, Forward};
    use super::*;
    use crate::context_menu::context_menu_item::ContextMenuItem;
    use crate::overlay::{OverlaySide, place};
    use gpui::{Entity, TestAppContext, bounds, point, px};
    use rise_theme::{Appearance, Density, ThemePalette};
    use std::cell::RefCell;
    use std::rc::Rc;

    type Events = Rc<RefCell<Vec<ContextMenuEvent>>>;

    fn chat_menu() -> Vec<ContextMenuEntry> {
        vec![
            ContextMenuItem::new("open", "ctx_open")
                .icon("arrow.up.right.square")
                .into(),
            ContextMenuItem::new("mark_read", "ctx_mark_read").into(),
            ContextMenuEntry::separator(),
            ContextMenuItem::new("mute", "ctx_mute")
                .icon("bell.slash")
                .into(),
            ContextMenuItem::new("reorder", "ctx_reorder")
                .disabled()
                .into(),
            ContextMenuEntry::separator(),
            ContextMenuItem::new("delete", "ctx_delete")
                .icon("trash")
                .destructive()
                .into(),
        ]
    }

    fn menu(cx: &mut TestAppContext) -> (Entity<ContextMenuState>, Events, gpui::Subscription) {
        let events: Events = Rc::new(RefCell::new(Vec::new()));
        let (menu, subscription) = cx.update(|cx| {
            let menu = cx.new(|cx| ContextMenuState::new("test.context_menu", cx));
            let sink = Rc::clone(&events);
            let subscription = cx.subscribe(&menu, move |_, event: &ContextMenuEvent, _| {
                sink.borrow_mut().push(event.clone());
            });
            (menu, subscription)
        });
        (menu, events, subscription)
    }

    fn opened(
        cx: &mut TestAppContext,
        entries: Vec<ContextMenuEntry>,
    ) -> (Entity<ContextMenuState>, Events, gpui::Subscription) {
        let (menu, events, subscription) = menu(cx);
        menu.update(cx, |state, cx| {
            state.open(entries, OverlayAnchor::Point(Point::default()), cx);
        });
        (menu, events, subscription)
    }

    fn drained(cx: &mut TestAppContext, events: &Events) -> Vec<ContextMenuEvent> {
        cx.run_until_parked();
        events.borrow_mut().drain(..).collect()
    }

    #[gpui::test]
    fn a_freshly_opened_menu_highlights_nothing(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, _cx| {
            assert_eq!(state.highlighted(), None);
            assert_eq!(state.entries().len(), 7);
            assert_eq!(state.region_key(), "test.context_menu");
        });
    }

    #[gpui::test]
    fn reopening_forgets_where_the_highlight_was(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            state.highlight_edge(Backward, cx);
            assert_eq!(state.highlighted(), Some(6));
            state.open(chat_menu(), OverlayAnchor::Point(Point::default()), cx);
            assert_eq!(state.highlighted(), None);
        });
    }

    #[gpui::test]
    fn the_highlight_skips_separators_and_disabled_items(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            state.move_highlight(Forward, cx);
            assert_eq!(state.highlighted(), Some(0));
            state.move_highlight(Forward, cx);
            assert_eq!(state.highlighted(), Some(1));
            state.move_highlight(Forward, cx);
            assert_eq!(state.highlighted(), Some(3), "index 2 is a separator");
            state.move_highlight(Forward, cx);
            assert_eq!(
                state.highlighted(),
                Some(6),
                "4 is disabled and 5 a separator"
            );
            state.move_highlight(Forward, cx);
            assert_eq!(state.highlighted(), Some(0), "and it wraps");
        });
    }

    #[gpui::test]
    fn the_highlight_wraps_backwards_too(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            state.move_highlight(Backward, cx);
            assert_eq!(state.highlighted(), Some(6));
            state.move_highlight(Backward, cx);
            assert_eq!(state.highlighted(), Some(3));
            state.move_highlight(Backward, cx);
            assert_eq!(state.highlighted(), Some(1));
        });
    }

    #[gpui::test]
    fn home_and_end_jump_to_the_first_and_last_enabled_item(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            state.highlight_edge(Backward, cx);
            assert_eq!(state.highlighted(), Some(6));
            state.highlight_edge(Forward, cx);
            assert_eq!(state.highlighted(), Some(0));
        });
    }

    #[gpui::test]
    fn a_menu_with_nothing_enabled_never_highlights_anything(cx: &mut TestAppContext) {
        let entries = vec![
            ContextMenuEntry::separator(),
            ContextMenuItem::new("reorder", "ctx_reorder")
                .disabled()
                .into(),
            ContextMenuEntry::separator(),
        ];
        let (menu, events, _subscription) = opened(cx, entries);

        menu.update(cx, |state, cx| {
            state.move_highlight(Forward, cx);
            assert_eq!(state.highlighted(), None);
            state.highlight_edge(Backward, cx);
            assert_eq!(state.highlighted(), None);
            assert_eq!(state.activate_highlighted(cx), None);
        });

        assert!(drained(cx, &events).is_empty());
    }

    #[gpui::test]
    fn activating_emits_the_stable_action_id(cx: &mut TestAppContext) {
        let (menu, events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            state.move_highlight(Forward, cx);
            assert_eq!(state.activate_highlighted(cx).as_deref(), Some("open"));
        });

        assert_eq!(
            drained(cx, &events),
            vec![ContextMenuEvent::Activated("open".into())]
        );
    }

    #[gpui::test]
    fn a_disabled_row_activates_nothing(cx: &mut TestAppContext) {
        let (menu, events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            assert_eq!(state.activate_index(4, cx), None, "reorder is disabled");
            assert_eq!(state.activate_index(2, cx), None, "index 2 is a separator");
            assert_eq!(state.activate_index(99, cx), None);
        });

        assert!(drained(cx, &events).is_empty());
    }

    #[gpui::test]
    fn dismissal_leaves_as_an_event(cx: &mut TestAppContext) {
        let (menu, events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| state.close(cx));

        assert_eq!(drained(cx, &events), vec![ContextMenuEvent::Dismissed]);
    }

    #[gpui::test]
    fn the_highlight_never_lands_on_a_row_the_keyboard_could_not_reach(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            state.highlight(Some(2), cx);
            assert_eq!(state.highlighted(), None, "a separator");
            state.highlight(Some(4), cx);
            assert_eq!(state.highlighted(), None, "disabled");
            state.highlight(Some(99), cx);
            assert_eq!(state.highlighted(), None, "out of range");
            state.highlight(Some(3), cx);
            assert_eq!(state.highlighted(), Some(3));
        });
    }

    #[gpui::test]
    fn the_pointer_gives_the_highlight_back_when_it_leaves(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            state.hover(3, true, cx);
            assert_eq!(state.highlighted(), Some(3));
            state.hover(0, true, cx);
            assert_eq!(state.highlighted(), Some(0));
            state.hover(3, false, cx);
            assert_eq!(
                state.highlighted(),
                Some(0),
                "leaving another row is not ours"
            );
            state.hover(0, false, cx);
            assert_eq!(state.highlighted(), None);
        });
    }

    #[gpui::test]
    fn the_checkmark_follows_the_selected_action_id(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, cx| {
            assert_eq!(state.selected(), None);
            state.set_selected(Some("mute".into()), cx);
            assert_eq!(state.selected().map(SharedString::as_ref), Some("mute"));

            let checked: Vec<&str> = state
                .entries()
                .iter()
                .filter_map(ContextMenuEntry::as_item)
                .filter(|item| Some(&item.id) == state.selected())
                .map(|item| item.id.as_ref())
                .collect();
            assert_eq!(checked, vec!["mute"], "exactly one row carries the mark");
        });
    }

    #[gpui::test]
    fn the_anchor_is_kept_as_given(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());
        let at = OverlayAnchor::Point(gpui::point(px(320.0), px(180.0)));

        menu.update(cx, |state, cx| {
            state.set_anchor(at, cx);
            assert_eq!(state.anchor(), at);
        });
    }

    #[gpui::test]
    fn every_item_the_menu_ships_resolves_to_a_real_string_key_and_icon(cx: &mut TestAppContext) {
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        menu.update(cx, |state, _cx| {
            for item in state.entries().iter().filter_map(ContextMenuEntry::as_item) {
                assert!(!item.label_key.is_empty());
                assert!(
                    !item.label_key.contains(' '),
                    "{} looks like copy, not a key",
                    item.label_key
                );
                if let Some(symbol) = item.sf_symbol {
                    assert!(
                        IconUi::asset_path(symbol).is_some(),
                        "{symbol} has no Lucide mapping, so the row would draw a gap"
                    );
                }
            }
        });

        assert!(
            IconUi::asset_path(CHECKMARK).is_some(),
            "the checkmark is the one glyph a menu cannot do without"
        );
    }

    #[gpui::test]
    fn the_size_a_menu_asks_for_grows_with_what_is_in_it(cx: &mut TestAppContext) {
        let theme = AppTheme::dark();
        let (menu, _events, _subscription) = opened(cx, chat_menu());

        let full = menu.update(cx, |state, _cx| state.asked_size(&theme));
        assert_eq!(full.width, theme.shell.menu_max_width);
        assert!(full.height > theme.shell.menu_row_height * 5.0);

        let shorter = menu.update(cx, |state, cx| {
            state.open(
                vec![ContextMenuItem::new("open", "ctx_open").into()],
                OverlayAnchor::Point(Point::default()),
                cx,
            );
            state.asked_size(&theme)
        });
        assert!(shorter.height < full.height);
        assert!(shorter.height > Pixels::ZERO);

        let empty = menu.update(cx, |state, cx| {
            state.open(Vec::new(), OverlayAnchor::Point(Point::default()), cx);
            state.asked_size(&theme)
        });
        assert!(
            empty.height > Pixels::ZERO,
            "an empty menu still has its own padding, never a negative height"
        );
    }

    /// `menu_max_width` is what a menu ASKS for. A 400pt window is narrower than
    /// the ask at every density, and the menu still has to land inside it.
    #[test]
    fn a_menu_at_the_width_it_asks_for_still_fits_a_narrow_window() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let theme = AppTheme::new(&ThemePalette::default_dark(), Appearance::Dark, density);
            let metrics = theme.shell;
            let viewport = size(px(400.0), px(700.0));
            let menu = gpui::size(metrics.menu_max_width, metrics.menu_row_height * 8.0);

            let anchors = [
                OverlayAnchor::Point(point(px(390.0), px(690.0))),
                OverlayAnchor::Point(point(px(0.0), px(0.0))),
                OverlayAnchor::Rect(bounds(
                    point(px(360.0), px(660.0)),
                    size(px(36.0), px(36.0)),
                )),
            ];

            for anchor in anchors {
                let placement = place(
                    anchor,
                    menu,
                    viewport,
                    metrics.overlay_gap,
                    metrics.overlay_margin,
                    OverlaySide::Below,
                );

                let x = f32::from(placement.origin.x);
                let y = f32::from(placement.origin.y);
                assert!(x >= 0.0 && y >= 0.0, "{density:?} put the menu off-window");
                assert!(
                    x + f32::from(menu.width) <= f32::from(viewport.width) + 1e-3,
                    "{density:?} let the menu out through the right edge"
                );
                assert!(
                    y + f32::from(menu.height) <= f32::from(viewport.height) + 1e-3,
                    "{density:?} let the menu out through the bottom edge"
                );
            }
        }
    }
}

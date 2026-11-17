use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyWindowHandle, Bounds, ClickEvent, Context, Entity, FocusHandle, IntoElement,
    Keystroke, KeystrokeEvent, Pixels, Render, SharedString, Subscription, Window, anchored, canvas,
    div, point, prelude::*, px,
};
use rise_i18n::tr;
use rise_platform::HostOs;
use rise_platform::gpui_shim::supports_native_menu_bar;
use rise_theme::AppTheme;
use rise_widgets::{
    ContextMenuEntry, ContextMenuEvent, ContextMenuItem, ContextMenuState, HighlightDirection,
    OverlayAnchor, OverlayLayer, OverlayScrim, OverlaySide, place,
};

use super::app_menu::{AppMenuCommand, AppMenuEntry, AppMenuSection, app_menu_sections};

const MENU_REGION_KEY: &str = "shell.app_menu";

/// what a keystroke means to the in-window menu bar, decided before the focused view sees it
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppMenuBarKey {
    Mnemonic,
    Previous,
    Next,
}

/// up and down and enter and escape inside an open menu belong to the menu itself
pub fn menu_bar_key(keystroke: &Keystroke, menu_open: bool) -> Option<AppMenuBarKey> {
    let modifiers = &keystroke.modifiers;
    let alt_alone = modifiers.alt && modifiers.number_of_modifiers() == 1;
    if alt_alone && !mnemonic_candidates(keystroke).is_empty() {
        return Some(AppMenuBarKey::Mnemonic);
    }
    if !menu_open || modifiers.modified() {
        return None;
    }
    match keystroke.key.as_str() {
        "left" => Some(AppMenuBarKey::Previous),
        "right" => Some(AppMenuBarKey::Next),
        _ => None,
    }
}

/// access letter of a menu title, its first letter or digit lowercased
pub fn mnemonic_of(title: &str) -> Option<char> {
    title
        .chars()
        .find(|character| character.is_alphanumeric())
        .and_then(|character| character.to_lowercase().next())
}

/// the key on the layout and the character it types so a title in any script can be reached
pub fn mnemonic_candidates(keystroke: &Keystroke) -> Vec<char> {
    let mut candidates = Vec::new();
    let typed = keystroke.key_char.as_deref();
    for text in std::iter::once(keystroke.key.as_str()).chain(typed) {
        let mut characters = text.chars();
        if let (Some(character), None) = (characters.next(), characters.next())
            && character.is_alphanumeric()
            && let Some(lowered) = character.to_lowercase().next()
            && !candidates.contains(&lowered)
        {
            candidates.push(lowered);
        }
    }
    candidates
}

/// titles sharing a letter take turns, the search starts after the menu already open
pub fn section_for_mnemonic(
    mnemonics: &[Option<char>],
    candidates: &[char],
    current: Option<usize>,
) -> Option<usize> {
    let count = mnemonics.len();
    let start = current.map_or(0, |index| index + 1);
    (0..count)
        .map(|offset| (start + offset) % count)
        .find(|&index| mnemonics[index].is_some_and(|letter| candidates.contains(&letter)))
}

/// wraps at both ends like a native menu bar
pub fn adjacent_section(current: usize, count: usize, key: AppMenuBarKey) -> usize {
    if count == 0 {
        return 0;
    }
    match key {
        AppMenuBarKey::Previous => (current + count - 1) % count,
        AppMenuBarKey::Next => (current + 1) % count,
        AppMenuBarKey::Mnemonic => current % count,
    }
}

pub struct AppMenuBar {
    sections: Vec<AppMenuSection>,
    menu: Option<Entity<ContextMenuState>>,
    open_section: Option<usize>,
    title_bounds: Vec<Rc<Cell<Option<Bounds<Pixels>>>>>,
    return_focus: Option<FocusHandle>,
    window: Option<AnyWindowHandle>,
    _menu_events: Option<Subscription>,
    _keystrokes: Option<Subscription>,
}

impl AppMenuBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let sections = if supports_native_menu_bar().is_supported() {
            Vec::new()
        } else {
            app_menu_sections(HostOs::current())
        };

        let title_bounds = sections.iter().map(|_| Rc::new(Cell::new(None))).collect();

        let keystrokes = (!sections.is_empty()).then(|| {
            let bar = cx.weak_entity();
            cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx| {
                let handled = bar
                    .update(cx, |bar, cx| bar.handle_keystroke(&event.keystroke, window, cx))
                    .unwrap_or(false);
                if handled {
                    cx.stop_propagation();
                }
            })
        });

        Self {
            sections,
            title_bounds,
            menu: None,
            open_section: None,
            return_focus: None,
            window: None,
            _menu_events: None,
            _keystrokes: keystrokes,
        }
    }

    /// false where the os shows the menu, the host strip reserves no room for it then
    pub fn is_drawn(&self) -> bool {
        !self.sections.is_empty()
    }

    pub fn open_section(&self) -> Option<usize> {
        self.open_section
    }

    /// does nothing before the title was laid out once, nothing to anchor to yet
    pub fn open_section_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(section) = self.sections.get(index) else {
            return;
        };
        let Some(title_rect) = self.title_bounds.get(index).and_then(|cell| cell.get()) else {
            return;
        };
        let entries = section_entries(section);

        if self.open_section.is_none() {
            self.return_focus = window.focused(cx);
        }

        let menu = match self.menu.clone() {
            Some(menu) => menu,
            None => {
                let menu = cx.new(|cx| ContextMenuState::new(MENU_REGION_KEY, cx));
                self._menu_events = Some(cx.subscribe_in(
                    &menu,
                    window,
                    |bar, _, event: &ContextMenuEvent, window, cx| match event {
                        ContextMenuEvent::Activated(id) => bar.activate(id.clone(), window, cx),
                        ContextMenuEvent::Dismissed => bar.close(window, cx),
                    },
                ));
                self.menu = Some(menu.clone());
                menu
            }
        };

        menu.update(cx, |menu, cx| {
            menu.open(entries, OverlayAnchor::Rect(title_rect), cx);
            menu.focus(window, cx);
        });

        self.open_section = Some(index);
        cx.notify();
    }

    fn open_section_from_keyboard(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_section_at(index, window, cx);
        if self.open_section != Some(index) {
            return;
        }
        if let Some(menu) = self.menu.clone() {
            menu.update(cx, |menu, cx| menu.highlight_edge(HighlightDirection::Forward, cx));
        }
    }

    fn handle_keystroke(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.window != Some(window.window_handle()) {
            return false;
        }
        let Some(key) = menu_bar_key(keystroke, self.open_section.is_some()) else {
            return false;
        };
        let target = match (key, self.open_section) {
            (AppMenuBarKey::Mnemonic, current) => {
                let mnemonics: Vec<Option<char>> = self
                    .sections
                    .iter()
                    .map(|section| mnemonic_of(&tr(section.title_key)))
                    .collect();
                section_for_mnemonic(&mnemonics, &mnemonic_candidates(keystroke), current)
            }
            (_, Some(current)) => Some(adjacent_section(current, self.sections.len(), key)),
            (_, None) => None,
        };
        let Some(index) = target else {
            return false;
        };
        self.open_section_from_keyboard(index, window, cx);
        true
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_section.take().is_none() {
            return;
        }
        if let Some(focus) = self.return_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// focus goes back first because gpui dispatches to whatever is focused at call time
    fn activate(&mut self, id: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        if let Some(action) = AppMenuCommand::from_id(&id).and_then(AppMenuCommand::action) {
            window.dispatch_action(action, cx);
        }
    }

    fn title(
        &self,
        theme: &AppTheme,
        index: usize,
        section: &AppMenuSection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_open = self.open_section == Some(index);
        let bounds_cell = self.title_bounds.get(index).cloned();
        let body = theme.typography.body();
        let tint = if is_open {
            theme.text.primary
        } else {
            theme.text.secondary
        };

        div()
            .id(SharedString::from(format!("{MENU_REGION_KEY}.{}", section.title_key)))
            .flex()
            .items_center()
            .h_full()
            .px(theme.spacing._300)
            .rounded(theme.radius._200)
            .when(is_open, |title| title.bg(theme.bg._400))
            .hover(|title| title.bg(theme.bg._300))
            .text_size(body.size)
            .font(body.font)
            .text_color(tint)
            .relative()
            .child(tr(section.title_key))
            .children(bounds_cell.map(|cell| {
                canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .inset_0()
            }))
            .on_click(cx.listener(move |bar, _: &ClickEvent, window, cx| {
                if bar.open_section == Some(index) {
                    bar.close(window, cx);
                } else {
                    bar.open_section_at(index, window, cx);
                }
            }))
            .into_any_element()
    }

    /// drawn from the strip so the layer is pinned to the window origin and sized to the viewport
    fn overlay(&self, theme: &AppTheme, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.open_section?;
        let menu = self.menu.clone()?;
        let viewport = window.viewport_size();

        let placement = place(
            menu.read(cx).anchor(),
            menu.read(cx).asked_size(theme),
            viewport,
            theme.shell.overlay_gap,
            theme.shell.overlay_margin,
            OverlaySide::Below,
        );

        let bar = cx.entity();
        let layer = OverlayLayer::new(placement)
            .scrim(OverlayScrim::Invisible)
            .child(menu)
            .on_dismiss(move |window, cx| bar.update(cx, |bar, cx| bar.close(window, cx)))
            .render(cx);

        Some(
            anchored()
                .position(point(px(0.0), px(0.0)))
                .child(div().relative().w(viewport.width).h(viewport.height).child(layer))
                .into_any_element(),
        )
    }
}

fn section_entries(section: &AppMenuSection) -> Vec<ContextMenuEntry> {
    section
        .entries
        .iter()
        .filter_map(|entry| match entry {
            AppMenuEntry::Separator => Some(ContextMenuEntry::separator()),
            AppMenuEntry::Command(command) => command
                .action()
                .map(|_| ContextMenuItem::new(command.id(), command.label_key()).into()),
        })
        .collect()
}

impl Render for AppMenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_drawn() {
            return div().into_any_element();
        }
        self.window = Some(window.window_handle());

        let theme = rise_ui::theme(cx).clone();
        let titles: Vec<AnyElement> = self
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| self.title(&theme, index, section, cx))
            .collect();

        div()
            .flex()
            .flex_row()
            .items_center()
            .h_full()
            .gap(theme.spacing._100)
            .children(titles)
            .children(self.overlay(&theme, window, cx))
            .into_any_element()
    }
}

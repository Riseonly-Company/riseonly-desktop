use std::rc::Rc;

use crate::app::router::navigation_types::{MainTab, RootRoute};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Pixels, Render, SharedString, Subscription, Window, div, prelude::*, px,
};
use rise_i18n::tr;
use rise_platform::materials::Material;
use rise_theme::AppTheme;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState};
use rise_ui::swiftui::button_style::{ButtonAction, button_chrome_hover};
use rise_ui::{IconSize, IconUi};
use rise_widgets::GlassPanel;

use crate::app::shell::shell_actions::{
    ConfirmSelection, PALETTE_KEY_CONTEXT, SelectNextItem, SelectPreviousItem,
};
use crate::core::widgets::navigations::main_tab_bar::{MainTabBarItem, MainTabBarMoreEntry};
use crate::modules::sparks::components::sparks_icon_view::SparksIconView;

#[derive(Clone, PartialEq, Debug)]
pub enum CommandTarget {
    Section(MainTab),
    Route(RootRoute),
}

#[derive(Clone, PartialEq, Debug)]
pub struct PaletteCommand {
    pub id: SharedString,
    pub title: SharedString,
    pub sf_symbol: &'static str,
    pub target: CommandTarget,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum MatchRank {
    Prefix,
    WordStart,
    Anywhere,
}

/// what the left bar offers in order, sections then more rows then settings at the foot
/// built when the palette opens since a more row route reads the signed in profile
pub fn catalogue(cx: &mut App) -> Vec<PaletteCommand> {
    catalogue_from(|entry| (entry.route)(cx))
}

fn catalogue_from(mut route: impl FnMut(&MainTabBarMoreEntry) -> RootRoute) -> Vec<PaletteCommand> {
    let mut commands: Vec<PaletteCommand> = MainTabBarItem::all()
        .iter()
        .map(|item| PaletteCommand {
            id: SharedString::from(format!("section.{}", item.id.raw_value())),
            title: SharedString::from(tr(item.title_key)),
            sf_symbol: item.symbol,
            target: CommandTarget::Section(item.id),
        })
        .collect();

    let more = MainTabBarMoreEntry::all().iter().chain(std::iter::once(MainTabBarMoreEntry::settings()));
    commands.extend(more.map(|entry| PaletteCommand {
        id: SharedString::new_static(entry.id),
        title: SharedString::from(tr(entry.title_key)),
        sf_symbol: entry.symbol,
        target: CommandTarget::Route(route(entry)),
    }));

    commands
}

pub fn filter(query: &str, commands: &[PaletteCommand]) -> Vec<usize> {
    let needle = normalise(query);
    if needle.is_empty() {
        return (0..commands.len()).collect();
    }

    let mut ranked: Vec<(MatchRank, usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            rank(&normalise(&command.title), &needle).map(|rank| (rank, index))
        })
        .collect();

    ranked.sort_by_key(|(rank, index)| (*rank, *index));
    ranked.into_iter().map(|(_, index)| index).collect()
}

fn normalise(raw: &str) -> String {
    raw.trim().to_lowercase()
}

fn rank(haystack: &str, needle: &str) -> Option<MatchRank> {
    let at = haystack.find(needle)?;
    if at == 0 {
        return Some(MatchRank::Prefix);
    }

    let preceding = haystack[..at].chars().next_back();
    match preceding {
        Some(character) if !character.is_alphanumeric() => Some(MatchRank::WordStart),
        _ => Some(MatchRank::Anywhere),
    }
}

pub enum CommandPaletteEvent {
    Dismissed,
    Activated(PaletteCommand),
}

pub struct CommandPalette {
    query: Entity<InputUiState>,
    commands: Vec<PaletteCommand>,
    matches: Vec<usize>,
    highlighted: usize,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CommandPalette {
    pub fn new(commands: Vec<PaletteCommand>, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            let mut state = InputUiState::new(InputMode::SingleLine, cx);
            state.set_placeholder(tr("search_placeholder"), cx);
            state
        });

        let subscription = cx.subscribe(&query, |palette, _, event, cx| match event {
            InputUiEvent::Changed => palette.requery(cx),
            InputUiEvent::Submitted => palette.confirm(&ConfirmSelection, cx),
            InputUiEvent::Cancelled => cx.emit(CommandPaletteEvent::Dismissed),
        });

        let matches = (0..commands.len()).collect();

        Self {
            query,
            commands,
            matches,
            highlighted: 0,
            focus_handle: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    pub fn query_focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }

    #[cfg(test)]
    pub fn query_text(&self, cx: &App) -> String {
        self.query.read(cx).text().to_owned()
    }

    pub fn highlighted_command(&self) -> Option<&PaletteCommand> {
        self.matches
            .get(self.highlighted)
            .and_then(|index| self.commands.get(*index))
    }

    pub fn select_next(
        &mut self,
        _: &SelectNextItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_highlight(1, cx);
    }

    pub fn select_previous(
        &mut self,
        _: &SelectPreviousItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_highlight(-1, cx);
    }

    pub fn confirm_action(
        &mut self,
        action: &ConfirmSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(action, cx);
    }

    fn confirm(&mut self, _: &ConfirmSelection, cx: &mut Context<Self>) {
        let Some(command) = self.highlighted_command().cloned() else {
            return;
        };
        cx.emit(CommandPaletteEvent::Activated(command));
    }

    fn requery(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).text().to_owned();
        self.matches = filter(&query, &self.commands);
        self.highlighted = 0;
        cx.notify();
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }

        let count = self.matches.len() as isize;
        let next = (self.highlighted as isize + delta).rem_euclid(count);
        self.highlighted = next as usize;
        cx.notify();
    }

    fn activate_index(&mut self, position: usize, cx: &mut Context<Self>) {
        if position >= self.matches.len() {
            return;
        }
        self.highlighted = position;
        self.confirm(&ConfirmSelection, cx);
    }

    fn row(
        &self,
        theme: &AppTheme,
        command: PaletteCommand,
        position: usize,
        is_highlighted: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tint = theme.text.primary;
        let body = theme.typography.body();

        let palette = cx.entity();
        let activate: ButtonAction = Rc::new(move |_window: &mut Window, cx: &mut App| {
            palette.update(cx, |palette, cx| palette.activate_index(position, cx));
        });
        let mut row = button_chrome_hover(
            div()
                .id(SharedString::from(format!("palette.{}", command.id)))
                .flex()
                .items_center()
                .gap(theme.spacing._600)
                .w_full()
                .h(theme.shell.palette_row_height)
                .px(theme.spacing._600)
                .rounded(theme.radius._200)
                .when(is_highlighted, |row| row.bg(theme.bg._400)),
            false,
            Some(activate),
            |row| row.bg(theme.bg._300),
        );

        if command.sf_symbol.is_empty() {
            row = row.child(SparksIconView::new().size(f32::from(IconUi::size(theme, IconSize::Regular))));
        } else if let Some(icon) = IconUi::render(theme, command.sf_symbol, IconSize::Regular, tint) {
            row = row.child(icon);
        }

        row.child(
            div()
                .flex_1()
                .text_size(body.size)
                .font(body.font)
                .text_color(tint)
                .truncate()
                .child(command.title.clone()),
        )
        .into_any_element()
    }
}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App).clone();
        let metrics = theme.shell;

        let available = window.viewport_size().width - metrics.overlay_margin * 2.0;
        let width = clamp_width(metrics.palette_width, available);

        let entries: Vec<(usize, PaletteCommand)> = self
            .matches
            .iter()
            .enumerate()
            .filter_map(|(position, index)| {
                self.commands
                    .get(*index)
                    .map(|command| (position, command.clone()))
            })
            .collect();

        let highlighted = self.highlighted;
        let mut rows = Vec::with_capacity(entries.len().max(1));
        for (position, command) in entries {
            rows.push(self.row(&theme, command, position, position == highlighted, cx));
        }
        if rows.is_empty() {
            rows.push(empty_state(&theme));
        }

        GlassPanel::surface("shell.command_palette", Material::Overlay, cx)
            .bg(theme.bg._100)
            .occlude()
            .key_context(PALETTE_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm_action))
            .w(width)
            .max_h(metrics.palette_max_height)
            .flex()
            .flex_col()
            .gap(theme.spacing._300)
            .p(theme.spacing._600)
            .child(div().flex_shrink_0().w_full().child(self.query.clone()))
            .child(
                div()
                    .id("palette.results")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(theme.spacing._100)
                    .overflow_y_scroll()
                    .children(rows),
            )
    }
}

fn empty_state(theme: &AppTheme) -> AnyElement {
    let body = theme.typography.body();
    div()
        .id("palette.empty")
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(theme.shell.palette_row_height)
        .px(theme.spacing._600)
        .text_size(body.size)
        .font(body.font)
        .text_color(theme.text.secondary)
        .child(tr("chat_search_no_results"))
        .into_any_element()
}

fn clamp_width(preferred: Pixels, available: Pixels) -> Pixels {
    if available <= px(0.0) {
        return preferred;
    }
    preferred.min(available)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(title: &str) -> PaletteCommand {
        PaletteCommand {
            id: SharedString::from(title.to_owned()),
            title: SharedString::from(title.to_owned()),
            sf_symbol: "folder",
            target: CommandTarget::Section(MainTab::Posts),
        }
    }

    fn titles(query: &str, commands: &[PaletteCommand]) -> Vec<String> {
        filter(query, commands)
            .into_iter()
            .map(|index| commands[index].title.to_string())
            .collect()
    }

    #[test]
    fn an_empty_query_lists_the_catalogue_in_its_own_order() {
        let commands = vec![command("Posts"), command("Chats")];
        assert_eq!(titles("", &commands), vec!["Posts", "Chats"]);
        assert_eq!(titles("   ", &commands), vec!["Posts", "Chats"]);
    }

    #[test]
    fn a_prefix_outranks_a_word_start_which_outranks_a_match_in_the_middle() {
        let commands = vec![
            command("Unread chats"),
            command("Chats"),
            command("Archived"),
        ];
        assert_eq!(titles("chat", &commands), vec!["Chats", "Unread chats"]);

        let mixed = vec![command("Rechat"), command("New chat")];
        assert_eq!(titles("chat", &mixed), vec!["New chat", "Rechat"]);
    }

    #[test]
    fn matching_ignores_case_and_surrounding_space_in_both_languages() {
        let commands = vec![
            command("\u{41d}\u{430}\u{441}\u{442}\u{440}\u{43e}\u{439}\u{43a}\u{438}"),
            command("Settings"),
        ];
        assert_eq!(
            titles("  \u{41d}\u{410}\u{421}\u{422} ", &commands),
            vec!["\u{41d}\u{430}\u{441}\u{442}\u{440}\u{43e}\u{439}\u{43a}\u{438}"]
        );
        assert_eq!(titles("SET", &commands), vec!["Settings"]);
    }

    #[test]
    fn a_query_nothing_matches_returns_nothing_rather_than_everything() {
        let commands = vec![command("Posts")];
        assert!(titles("zzz", &commands).is_empty());
    }

    fn every_command() -> Vec<PaletteCommand> {
        catalogue_from(|_| RootRoute::Settings)
    }

    #[test]
    fn the_catalogue_offers_every_bar_button_once_and_every_more_row() {
        let commands = every_command();

        assert_eq!(commands.len(), MainTabBarItem::all().len() + MainTabBarMoreEntry::all().len() + 1);
        for item in MainTabBarItem::all() {
            assert_eq!(
                commands.iter().filter(|command| command.target == CommandTarget::Section(item.id)).count(),
                1,
                "{:?} is offered more than once",
                item.id
            );
        }
    }

    #[test]
    fn the_catalogue_keeps_the_bars_order() {
        let commands = every_command();
        for (command, item) in commands.iter().zip(MainTabBarItem::all()) {
            assert_eq!(command.target, CommandTarget::Section(item.id));
        }
        assert_eq!(commands.last().map(|command| command.id.as_ref()), Some(MainTabBarMoreEntry::settings().id));
    }

    #[test]
    fn every_entry_has_an_id_and_an_icon_the_bundle_carries() {
        for command in every_command() {
            assert!(!command.id.is_empty());
            if command.sf_symbol.is_empty() {
                continue;
            }
            assert!(
                IconUi::asset_path(command.sf_symbol).is_some(),
                "{} has no icon, so the row would draw a gap",
                command.sf_symbol
            );
        }
    }

    #[test]
    fn entry_ids_are_unique_so_two_rows_cannot_share_an_element_id() {
        let commands = every_command();
        let mut ids: Vec<&str> = commands.iter().map(|c| c.id.as_ref()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            count,
            "duplicate element ids are dropped in release"
        );
    }

    #[test]
    fn a_narrow_window_shrinks_the_palette_instead_of_pushing_it_off_the_frame() {
        assert_eq!(clamp_width(px(560.0), px(376.0)), px(376.0));
        assert_eq!(clamp_width(px(560.0), px(1156.0)), px(560.0));
        assert_eq!(
            clamp_width(px(560.0), px(-8.0)),
            px(560.0),
            "a window narrower than its own margins must not ask for a negative width"
        );
    }
}

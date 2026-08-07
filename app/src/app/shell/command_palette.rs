use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Pixels, Render, SharedString, Subscription, Window, div, prelude::*, px,
};
use rise_i18n::tr;
use rise_navigation::{RootRoute, RootTab};
use rise_platform::materials::Material;
use rise_theme::AppTheme;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState};
use rise_ui::{IconSize, IconUi};
use rise_widgets::GlassPanel;

use crate::app::shell::rail::{RailFolder, RailSection};
use crate::app::shell::shell_actions::{
    ConfirmSelection, PALETTE_KEY_CONTEXT, SelectNextItem, SelectPreviousItem,
};

/// What running a palette entry does.
///
/// A target rather than a closure, so an entry stays a value: it can be compared,
/// listed in a test, and handed to the shell without the palette knowing what a
/// section or a route means.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CommandTarget {
    Section(RootTab),
    Route(RootRoute),
    Folder(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PaletteCommand {
    pub id: SharedString,
    pub title: SharedString,
    pub sf_symbol: &'static str,
    pub target: CommandTarget,
}

/// How well a query matched, best first.
///
/// Three tiers rather than a fuzzy score: a palette that reorders its list on
/// every keystroke by a similarity number is one the user cannot build muscle
/// memory against. Prefix beats word start beats anywhere, and inside a tier the
/// catalogue's own order wins.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum MatchRank {
    Prefix,
    WordStart,
    Anywhere,
}

pub fn catalogue(folders: &[RailFolder]) -> Vec<PaletteCommand> {
    let mut commands: Vec<PaletteCommand> = RailSection::ALL
        .iter()
        .map(|section| PaletteCommand {
            id: SharedString::from(format!("section.{}", section.tab.screen_id())),
            title: SharedString::from(section.title()),
            sf_symbol: section.sf_symbol,
            target: CommandTarget::Section(section.tab),
        })
        .collect();

    commands.push(PaletteCommand {
        id: SharedString::new_static("open.settings"),
        title: SharedString::from(tr("settings_page_title")),
        sf_symbol: "gearshape",
        target: CommandTarget::Route(RootRoute::Settings),
    });

    commands.extend(folders.iter().map(|folder| PaletteCommand {
        id: SharedString::from(format!("folder.{}", folder.id)),
        title: folder.title.clone(),
        sf_symbol: "folder",
        target: CommandTarget::Folder(folder.id.clone()),
    }));

    commands
}

/// Indices into `commands`, best match first, stable within a tier.
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

    /// The palette opens with the caret already in the field, so the first
    /// keystroke after the shortcut is a query and not a lost one.
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

    /// Runs the entry the pointer landed on.
    ///
    /// The highlight moves first, so that whichever way the entry was chosen —
    /// arrow keys or mouse — the palette reports the same thing.
    fn activate_index(&mut self, position: usize, cx: &mut Context<Self>) {
        if position >= self.matches.len() {
            return;
        }
        self.highlighted = position;
        self.confirm(&ConfirmSelection, cx);
    }

    /// Returns an `AnyElement` rather than `impl IntoElement`: under Rust 2024
    /// the opaque type would capture the `Context` borrow and the loop that
    /// builds the list could only ever run once.
    fn row(
        &self,
        theme: &AppTheme,
        command: PaletteCommand,
        position: usize,
        is_highlighted: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tint = if is_highlighted {
            theme.text.primary
        } else {
            theme.text.secondary
        };
        let body = theme.typography.body();

        let mut row = div()
            .id(SharedString::from(format!("palette.{}", command.id)))
            .flex()
            .items_center()
            .gap(theme.spacing._600)
            .w_full()
            .h(theme.shell.palette_row_height)
            .px(theme.spacing._600)
            .rounded(theme.radius._200)
            .when(is_highlighted, |row| row.bg(theme.bg._400))
            .hover(|row| row.bg(theme.bg._300))
            .on_click(cx.listener(move |palette, _: &ClickEvent, _, cx| {
                palette.activate_index(position, cx);
            }));

        if let Some(icon) = IconUi::render(theme, command.sf_symbol, IconSize::Regular, tint) {
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

        // What the token asks for, clamped to what the window can give: a 400px
        // window must still show the whole palette rather than pushing half of
        // it past the frame, and gpui has no popup that could escape it anyway.
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
        let mut rows = Vec::with_capacity(entries.len());
        for (position, command) in entries {
            rows.push(self.row(&theme, command, position, position == highlighted, cx));
        }

        GlassPanel::surface("shell.command_palette", Material::Overlay, cx)
            // The scrim behind this panel dismisses on a click, and gpui's hit
            // test reports every hitbox under the pointer unless one blocks it.
            // Without this, clicking a result — or the query field — would land
            // on the scrim as well and close the palette instead.
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
            // The ENTITY, not `InputUi::new(..)`: the bare element carries no key
            // context and tracks no focus, so focusing it would put the caret in a
            // node the dispatch tree cannot find — and every shortcut in the frame
            // around it would go to the window root and die there.
            .child(self.query.clone())
            .child(
                div()
                    .id("palette.results")
                    .flex()
                    .flex_col()
                    .gap(theme.spacing._100)
                    .overflow_y_scroll()
                    .children(rows),
            )
    }
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
            target: CommandTarget::Section(RootTab::Feed),
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
        let commands = vec![command("Настройки"), command("Settings")];
        assert_eq!(titles("  НАСТ ", &commands), vec!["Настройки"]);
        assert_eq!(titles("SET", &commands), vec!["Settings"]);
    }

    #[test]
    fn a_query_nothing_matches_returns_nothing_rather_than_everything() {
        let commands = vec![command("Posts")];
        assert!(titles("zzz", &commands).is_empty());
    }

    #[test]
    fn the_catalogue_offers_every_section_and_every_folder_exactly_once() {
        let folders = vec![
            RailFolder {
                id: "f1".into(),
                title: "Work".into(),
                unread: 0,
            },
            RailFolder {
                id: "f2".into(),
                title: "Family".into(),
                unread: 3,
            },
        ];
        let commands = catalogue(&folders);

        assert_eq!(commands.len(), RailSection::ALL.len() + 1 + folders.len());
        for tab in RootTab::ALL {
            assert_eq!(
                commands
                    .iter()
                    .filter(|c| c.target == CommandTarget::Section(tab))
                    .count(),
                1,
                "{tab:?} is offered more than once"
            );
        }
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c.target, CommandTarget::Folder(_)))
                .count(),
            folders.len()
        );
    }

    #[test]
    fn every_entry_has_an_id_and_an_icon_the_bundle_carries() {
        for command in catalogue(&[]) {
            assert!(!command.id.is_empty());
            assert!(
                IconUi::asset_path(command.sf_symbol).is_some(),
                "{} has no icon, so the row would draw a gap",
                command.sf_symbol
            );
        }
    }

    #[test]
    fn entry_ids_are_unique_so_two_rows_cannot_share_an_element_id() {
        let folders = vec![RailFolder {
            id: "Feed".into(),
            title: "Feed".into(),
            unread: 0,
        }];
        let commands = catalogue(&folders);
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

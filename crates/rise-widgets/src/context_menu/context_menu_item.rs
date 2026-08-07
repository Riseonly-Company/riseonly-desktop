use gpui::SharedString;
use rise_i18n::tr;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ContextMenuTone {
    #[default]
    Normal,
    Destructive,
}

/// One row of a context menu.
///
/// `id` is the reference's `key` — `Core/UI/ContextMenuUi.swift` — and it is the
/// STABLE ACTION ID, never a display string and never a position. The handler
/// and the analytics event on the other side of an activation both key off it,
/// so it has to be the same string the phone sends for the same action.
///
/// `label_key` is a string key, resolved at render time. Literal copy here would
/// be untranslatable in all 41 locales and invisible until someone switched one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContextMenuItem {
    pub id: SharedString,
    pub label_key: &'static str,
    pub sf_symbol: Option<&'static str>,
    pub tone: ContextMenuTone,
    pub enabled: bool,
}

impl ContextMenuItem {
    pub fn new(id: impl Into<SharedString>, label_key: &'static str) -> Self {
        Self {
            id: id.into(),
            label_key,
            sf_symbol: None,
            tone: ContextMenuTone::Normal,
            enabled: true,
        }
    }

    pub fn icon(mut self, sf_symbol: &'static str) -> Self {
        self.sf_symbol = Some(sf_symbol);
        self
    }

    pub fn destructive(mut self) -> Self {
        self.tone = ContextMenuTone::Destructive;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn label(&self) -> String {
        tr(self.label_key)
    }
}

/// A menu is a flat list of entries rather than a list of items, because the
/// separator has to occupy an index: keyboard traversal steps over indices, and
/// a separator kept outside the list would make every index mean two things.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContextMenuEntry {
    Item(ContextMenuItem),
    Separator,
}

impl ContextMenuEntry {
    pub fn separator() -> Self {
        Self::Separator
    }

    pub fn as_item(&self) -> Option<&ContextMenuItem> {
        match self {
            Self::Item(item) => Some(item),
            Self::Separator => None,
        }
    }

    /// Whether the keyboard and the pointer are allowed to land here.
    pub fn is_activatable(&self) -> bool {
        self.as_item().is_some_and(|item| item.enabled)
    }
}

impl From<ContextMenuItem> for ContextMenuEntry {
    fn from(item: ContextMenuItem) -> Self {
        Self::Item(item)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HighlightDirection {
    Forward,
    Backward,
}

/// The next index the highlight is allowed to sit on, wrapping.
///
/// `from` is where the highlight is now; `None` means it is nowhere, and the
/// answer is then the first entry in that direction rather than the second.
/// Disabled items and separators are skipped, so a menu whose only entries are
/// either has no answer at all — which is the case that turns into an index out
/// of range if this returns a number regardless.
pub fn next_enabled_index(
    entries: &[ContextMenuEntry],
    from: Option<usize>,
    direction: HighlightDirection,
) -> Option<usize> {
    let len = entries.len();
    if len == 0 {
        return None;
    }

    let step = |index: usize| match direction {
        HighlightDirection::Forward => (index + 1) % len,
        HighlightDirection::Backward => (index + len - 1) % len,
    };

    let mut cursor = match from.filter(|index| *index < len) {
        Some(index) => step(index),
        None => match direction {
            HighlightDirection::Forward => 0,
            HighlightDirection::Backward => len - 1,
        },
    };

    for _ in 0..len {
        if entries[cursor].is_activatable() {
            return Some(cursor);
        }
        cursor = step(cursor);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::HighlightDirection::{Backward, Forward};
    use super::*;

    fn item(id: &str) -> ContextMenuEntry {
        ContextMenuItem::new(id.to_owned(), "ctx_open").into()
    }

    fn disabled(id: &str) -> ContextMenuEntry {
        ContextMenuItem::new(id.to_owned(), "ctx_open")
            .disabled()
            .into()
    }

    #[test]
    fn an_empty_menu_has_nowhere_to_go() {
        assert_eq!(next_enabled_index(&[], None, Forward), None);
        assert_eq!(next_enabled_index(&[], Some(3), Backward), None);
    }

    #[test]
    fn a_menu_of_only_separators_and_disabled_items_has_nowhere_to_go() {
        let entries = [
            ContextMenuEntry::separator(),
            disabled("a"),
            ContextMenuEntry::separator(),
            disabled("b"),
        ];

        for from in [None, Some(0), Some(1), Some(2), Some(3)] {
            assert_eq!(next_enabled_index(&entries, from, Forward), None);
            assert_eq!(next_enabled_index(&entries, from, Backward), None);
        }
    }

    #[test]
    fn a_single_item_wraps_onto_itself() {
        let entries = [item("only")];

        assert_eq!(next_enabled_index(&entries, None, Forward), Some(0));
        assert_eq!(next_enabled_index(&entries, None, Backward), Some(0));
        assert_eq!(next_enabled_index(&entries, Some(0), Forward), Some(0));
        assert_eq!(next_enabled_index(&entries, Some(0), Backward), Some(0));
    }

    #[test]
    fn nothing_highlighted_lands_on_the_first_entry_in_that_direction() {
        let entries = [item("a"), item("b"), item("c")];

        assert_eq!(next_enabled_index(&entries, None, Forward), Some(0));
        assert_eq!(next_enabled_index(&entries, None, Backward), Some(2));
    }

    #[test]
    fn the_highlight_wraps_in_both_directions() {
        let entries = [item("a"), item("b"), item("c")];

        assert_eq!(next_enabled_index(&entries, Some(2), Forward), Some(0));
        assert_eq!(next_enabled_index(&entries, Some(0), Backward), Some(2));
    }

    #[test]
    fn disabled_items_are_stepped_over_rather_than_landed_on() {
        let entries = [item("a"), disabled("b"), disabled("c"), item("d")];

        assert_eq!(next_enabled_index(&entries, Some(0), Forward), Some(3));
        assert_eq!(next_enabled_index(&entries, Some(3), Backward), Some(0));
        assert_eq!(next_enabled_index(&entries, Some(3), Forward), Some(0));
        assert_eq!(next_enabled_index(&entries, Some(0), Backward), Some(3));
    }

    #[test]
    fn a_separator_is_never_a_destination_even_at_either_end() {
        let entries = [
            ContextMenuEntry::separator(),
            item("a"),
            ContextMenuEntry::separator(),
            item("b"),
            ContextMenuEntry::separator(),
        ];

        assert_eq!(next_enabled_index(&entries, None, Forward), Some(1));
        assert_eq!(next_enabled_index(&entries, None, Backward), Some(3));
        assert_eq!(next_enabled_index(&entries, Some(1), Forward), Some(3));
        assert_eq!(next_enabled_index(&entries, Some(3), Forward), Some(1));
        assert_eq!(next_enabled_index(&entries, Some(3), Backward), Some(1));
        assert_eq!(next_enabled_index(&entries, Some(1), Backward), Some(3));
        assert_eq!(
            next_enabled_index(&entries, Some(0), Forward),
            Some(1),
            "a highlight parked on a separator still has to move somewhere"
        );
    }

    #[test]
    fn an_index_past_the_end_is_treated_as_no_highlight_at_all() {
        let entries = [item("a"), item("b")];

        assert_eq!(next_enabled_index(&entries, Some(99), Forward), Some(0));
        assert_eq!(next_enabled_index(&entries, Some(99), Backward), Some(1));
    }

    #[test]
    fn walking_the_whole_menu_forwards_visits_every_enabled_item_once() {
        let entries = [
            item("a"),
            ContextMenuEntry::separator(),
            disabled("b"),
            item("c"),
            item("d"),
        ];

        let mut visited = Vec::new();
        let mut cursor = next_enabled_index(&entries, None, Forward);
        for _ in 0..entries.len() {
            let Some(index) = cursor else { break };
            if visited.contains(&index) {
                break;
            }
            visited.push(index);
            cursor = next_enabled_index(&entries, Some(index), Forward);
        }

        assert_eq!(visited, vec![0, 3, 4]);
    }

    #[test]
    fn an_item_carries_a_key_rather_than_copy() {
        let item = ContextMenuItem::new("delete", "ctx_delete")
            .icon("trash")
            .destructive();

        assert_eq!(item.label_key, "ctx_delete");
        assert_eq!(item.tone, ContextMenuTone::Destructive);
        assert_eq!(item.sf_symbol, Some("trash"));
        assert!(item.enabled);
        assert!(
            !item.label_key.contains(' '),
            "a key with a space in it is copy that will never be translated"
        );
    }

    #[test]
    fn an_entry_knows_whether_the_keyboard_may_land_on_it() {
        assert!(item("a").is_activatable());
        assert!(!disabled("a").is_activatable());
        assert!(!ContextMenuEntry::separator().is_activatable());
        assert!(ContextMenuEntry::separator().as_item().is_none());
    }
}

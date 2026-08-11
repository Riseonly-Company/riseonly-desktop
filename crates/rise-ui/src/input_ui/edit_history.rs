use std::ops::Range;

/// One replacement, stored so it can be run in either direction.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    pub at: usize,
    pub removed: String,
    pub inserted: String,
    pub selection_before: Range<usize>,
    pub selection_after: Range<usize>,
}

impl Edit {
    pub fn undone_range(&self) -> Range<usize> {
        self.at..self.at + self.inserted.len()
    }

    pub fn redone_range(&self) -> Range<usize> {
        self.at..self.at + self.removed.len()
    }
}

/// What produced an edit, which is all the coalescer needs to know.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    /// A character the user typed. A contiguous run of these is one undo step.
    Typing,
    /// A backspace or forward delete. A contiguous run is one undo step.
    Deleting,
    /// A paste, a programmatic set, or a committed IME composition: always its
    /// own step, however small.
    Atomic,
}

#[derive(Default, Debug)]
pub struct EditHistory {
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,
    last_kind: Option<EditKind>,
    coalescing: bool,
}

impl EditHistory {
    pub fn push(&mut self, edit: Edit, kind: EditKind) {
        self.redo_stack.clear();

        if self.coalescing
            && self.last_kind == Some(kind)
            && let Some(previous) = self.undo_stack.last_mut()
            && merge(previous, &edit, kind)
        {
            self.last_kind = Some(kind);
            return;
        }

        self.undo_stack.push(edit);
        self.last_kind = Some(kind);
        self.coalescing = true;
    }

    /// Ends the current run without discarding it.
    ///
    /// Anything that moves the caret without editing must call this, or an edit
    /// either side of the gap coalesces into one step.
    pub fn break_coalescing(&mut self) {
        self.coalescing = false;
    }

    pub fn undo(&mut self) -> Option<Edit> {
        let edit = self.undo_stack.pop()?;
        self.redo_stack.push(edit.clone());
        self.coalescing = false;
        self.last_kind = None;
        Some(edit)
    }

    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.redo_stack.pop()?;
        self.undo_stack.push(edit.clone());
        self.coalescing = false;
        self.last_kind = None;
        Some(edit)
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_kind = None;
        self.coalescing = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn depth(&self) -> usize {
        self.undo_stack.len()
    }
}

fn merge(previous: &mut Edit, edit: &Edit, kind: EditKind) -> bool {
    match kind {
        EditKind::Typing => {
            let contiguous = edit.at == previous.at + previous.inserted.len();
            if !contiguous
                || !edit.removed.is_empty()
                || !previous.removed.is_empty()
                || edit.inserted.contains('\n')
                || previous.inserted.ends_with('\n')
            {
                return false;
            }
            previous.inserted.push_str(&edit.inserted);
            previous.selection_after = edit.selection_after.clone();
            true
        }
        EditKind::Deleting => {
            if !edit.inserted.is_empty() || !previous.inserted.is_empty() {
                return false;
            }
            if edit.at + edit.removed.len() == previous.at {
                let mut removed = edit.removed.clone();
                removed.push_str(&previous.removed);
                previous.removed = removed;
                previous.at = edit.at;
                previous.selection_after = edit.selection_after.clone();
                true
            } else if edit.at == previous.at {
                previous.removed.push_str(&edit.removed);
                previous.selection_after = edit.selection_after.clone();
                true
            } else {
                false
            }
        }
        EditKind::Atomic => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(at: usize, text: &str) -> Edit {
        Edit {
            at,
            removed: String::new(),
            inserted: text.to_owned(),
            selection_before: at..at,
            selection_after: at + text.len()..at + text.len(),
        }
    }

    fn apply(document: &mut String, edit: &Edit, forward: bool) {
        if forward {
            document.replace_range(edit.redone_range(), &edit.inserted);
        } else {
            document.replace_range(edit.undone_range(), &edit.removed);
        }
    }

    #[test]
    fn a_run_of_typed_characters_is_one_undo_step() {
        let mut history = EditHistory::default();
        let mut document = String::new();
        for (index, character) in "hello".char_indices() {
            let edit = typed(index, &character.to_string());
            apply(&mut document, &edit, true);
            history.push(edit, EditKind::Typing);
        }

        assert_eq!(document, "hello");
        assert_eq!(history.depth(), 1);

        let edit = history.undo().unwrap();
        apply(&mut document, &edit, false);
        assert_eq!(document, "");
        assert!(!history.can_undo());
    }

    #[test]
    fn moving_the_caret_ends_the_run() {
        let mut history = EditHistory::default();
        history.push(typed(0, "a"), EditKind::Typing);
        history.break_coalescing();
        history.push(typed(1, "b"), EditKind::Typing);
        assert_eq!(history.depth(), 2);
    }

    #[test]
    fn a_non_contiguous_edit_starts_a_new_step() {
        let mut history = EditHistory::default();
        history.push(typed(0, "a"), EditKind::Typing);
        history.push(typed(9, "b"), EditKind::Typing);
        assert_eq!(history.depth(), 2);
    }

    #[test]
    fn a_newline_closes_the_step_it_ends() {
        let mut history = EditHistory::default();
        history.push(typed(0, "a"), EditKind::Typing);
        history.push(typed(1, "\n"), EditKind::Typing);
        assert_eq!(history.depth(), 2);
        history.push(typed(2, "b"), EditKind::Typing);
        assert_eq!(history.depth(), 3);
    }

    #[test]
    fn a_run_of_backspaces_is_one_undo_step() {
        let mut history = EditHistory::default();
        let mut document = String::from("Привет");
        for at in [10, 8, 6] {
            let removed = document[at..at + 2].to_owned();
            let edit = Edit {
                at,
                removed,
                inserted: String::new(),
                selection_before: at + 2..at + 2,
                selection_after: at..at,
            };
            apply(&mut document, &edit, true);
            history.push(edit, EditKind::Deleting);
        }
        assert_eq!(document, "При");
        assert_eq!(history.depth(), 1);

        let edit = history.undo().unwrap();
        apply(&mut document, &edit, false);
        assert_eq!(document, "Привет");
    }

    #[test]
    fn a_paste_never_merges_with_typing() {
        let mut history = EditHistory::default();
        history.push(typed(0, "a"), EditKind::Typing);
        history.push(typed(1, "pasted"), EditKind::Atomic);
        history.push(typed(7, "b"), EditKind::Typing);
        assert_eq!(history.depth(), 3);
    }

    #[test]
    fn redo_replays_what_undo_took_back() {
        let mut history = EditHistory::default();
        let mut document = String::new();
        let edit = typed(0, "日本語");
        apply(&mut document, &edit, true);
        history.push(edit, EditKind::Atomic);

        let undone = history.undo().unwrap();
        apply(&mut document, &undone, false);
        assert_eq!(document, "");

        let redone = history.redo().unwrap();
        apply(&mut document, &redone, true);
        assert_eq!(document, "日本語");
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn a_new_edit_drops_the_redo_stack() {
        let mut history = EditHistory::default();
        history.push(typed(0, "a"), EditKind::Atomic);
        history.undo();
        assert!(history.can_redo());
        history.push(typed(0, "b"), EditKind::Atomic);
        assert!(!history.can_redo());
    }
}

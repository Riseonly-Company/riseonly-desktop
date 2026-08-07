use gpui::{App, Global, KeyBinding, actions};

/// The key context the bindings below are scoped to.
///
/// Scoped rather than global so a field never eats a shortcut the shell owns.
pub const KEY_CONTEXT: &str = "InputUi";

actions!(
    rise_input,
    [
        Backspace,
        BackspaceWord,
        Cancel,
        Confirm,
        Copy,
        Cut,
        Delete,
        DeleteWord,
        MoveDocumentEnd,
        MoveDocumentStart,
        MoveDown,
        MoveLeft,
        MoveLineEnd,
        MoveLineStart,
        MoveRight,
        MoveUp,
        MoveWordLeft,
        MoveWordRight,
        Paste,
        Redo,
        SelectAll,
        SelectDocumentEnd,
        SelectDocumentStart,
        SelectDown,
        SelectLeft,
        SelectLineEnd,
        SelectLineStart,
        SelectRight,
        SelectUp,
        SelectWordLeft,
        SelectWordRight,
        ShowCharacterPalette,
        Undo,
    ]
);

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

/// Idempotent, and safe to call from anywhere.
///
/// `InputUiState::new` calls this so a field works in an app that never heard of
/// it; an app that wants the bindings present before the first field is built
/// can call it at startup. `secondary-` is gpui's own portable modifier — cmd on
/// macOS, ctrl elsewhere — which is why nothing here needs a `cfg`. The word and
/// document jumps bind both the macOS spelling and the Windows/Linux one, since
/// the other platform's chord is either free or already owned by the window
/// manager, and a keymap that guesses wrong leaves a dead key.
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);

    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("alt-left", MoveWordLeft, context),
        KeyBinding::new("alt-right", MoveWordRight, context),
        KeyBinding::new("ctrl-left", MoveWordLeft, context),
        KeyBinding::new("ctrl-right", MoveWordRight, context),
        KeyBinding::new("alt-shift-left", SelectWordLeft, context),
        KeyBinding::new("alt-shift-right", SelectWordRight, context),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, context),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, context),
        KeyBinding::new("home", MoveLineStart, context),
        KeyBinding::new("end", MoveLineEnd, context),
        KeyBinding::new("cmd-left", MoveLineStart, context),
        KeyBinding::new("cmd-right", MoveLineEnd, context),
        KeyBinding::new("shift-home", SelectLineStart, context),
        KeyBinding::new("shift-end", SelectLineEnd, context),
        KeyBinding::new("cmd-shift-left", SelectLineStart, context),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, context),
        KeyBinding::new("cmd-up", MoveDocumentStart, context),
        KeyBinding::new("cmd-down", MoveDocumentEnd, context),
        KeyBinding::new("ctrl-home", MoveDocumentStart, context),
        KeyBinding::new("ctrl-end", MoveDocumentEnd, context),
        KeyBinding::new("cmd-shift-up", SelectDocumentStart, context),
        KeyBinding::new("cmd-shift-down", SelectDocumentEnd, context),
        KeyBinding::new("ctrl-shift-home", SelectDocumentStart, context),
        KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("alt-backspace", BackspaceWord, context),
        KeyBinding::new("ctrl-backspace", BackspaceWord, context),
        KeyBinding::new("alt-delete", DeleteWord, context),
        KeyBinding::new("ctrl-delete", DeleteWord, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("secondary-z", Undo, context),
        KeyBinding::new("secondary-shift-z", Redo, context),
        KeyBinding::new("ctrl-y", Redo, context),
        KeyBinding::new("enter", Confirm, context),
        KeyBinding::new("escape", Cancel, context),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, context),
    ]);
}

//! The text field: [`InputUi`] paints, [`InputUiState`] remembers.
//!
//! The pieces below it are public because the field is assembled from them, not
//! because a screen is expected to use them directly: [`DisplayText`] is what is
//! shown rather than what was typed, [`ShapedInput`] maps between pixels and
//! offsets, [`EditHistory`] coalesces a run of typing into one undo step,
//! [`CaretBlink`] owns the blink, and `text_boundaries` holds the
//! grapheme- and UTF-16-aware arithmetic the rest of it stands on.

mod caret_blink;
mod display_text;
mod edit_history;
mod input_actions;
mod input_ui;
mod input_ui_state;
mod shaped_input;
mod text_boundaries;

pub use caret_blink::{BLINK_INTERVAL, CaretBlink, PAUSE_AFTER_EDIT};
pub use display_text::{DisplayLine, DisplayText, SECURE_BULLET};
pub use edit_history::{Edit, EditHistory, EditKind};
pub use input_actions::{KEY_CONTEXT, install_key_bindings};
pub use input_ui::InputUi;
pub use input_ui_state::{InputMode, InputUiEvent, InputUiState, SelectGranularity, Selection};
pub use shaped_input::{ShapeKey, ShapedInput};
pub use text_boundaries::{
    char_count, clamp_to_char_boundary, grapheme_count, grapheme_index, grapheme_offset, graphemes,
    line_end, line_range_at, line_start, next_grapheme_boundary, next_word_boundary,
    previous_grapheme_boundary, previous_word_boundary, truncate_to_char_limit,
    utf8_offset_from_utf16, utf8_range_from_utf16, utf16_len, utf16_offset_from_utf8,
    utf16_range_from_utf8, word_range_at,
};

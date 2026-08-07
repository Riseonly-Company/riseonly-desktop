//! The desktop UI kit: the components `riseonly-ios` screens are built from,
//! ported 1:1 on top of gpui.
//!
//! Two shapes live here, and the difference is deliberate.
//!
//! Most components — [`BoxUi`], [`ButtonUi`], [`MainText`], [`IconUi`],
//! [`SkeletonUi`], [`FlagUi`] — are namespaces of associated functions that
//! return a styled [`gpui::Div`]. They contribute theme values and nothing else,
//! so the call site keeps composing with gpui's own builders (`.p_4()`,
//! `.rounded_md()`, `.on_click(..)`) instead of learning a second, narrower API
//! that would have to re-export all of it.
//!
//! [`InputUi`] is the exception: a text field owns a selection, an undo history,
//! IME composition state and a blinking caret, none of which survive being
//! rebuilt every frame. It is a real gpui entity with [`InputUiState`] behind it.
//!
//! Colours, sizes and radii come from [`rise_theme::AppTheme`], installed once
//! per app with [`install_theme`] and read back with [`theme`]. A component
//! takes `&AppTheme` as an argument rather than reaching for the global, which
//! is what keeps its unit tests free of an `App`.

pub mod animation;
pub mod box_ui;
pub mod button_ui;
pub mod flag_ui;
pub mod fonts;
pub mod icon_ui;
pub mod image;
pub mod input_ui;
pub mod list;
pub mod main_text;
pub mod phone_input;
pub mod skeleton;

pub use box_ui::BoxUi;
pub use button_ui::{ButtonSize, ButtonTone, ButtonUi};
pub use flag_ui::FlagUi;
pub use icon_ui::{IconSize, IconUi};
pub use input_ui::{InputUi, InputUiState};
pub use main_text::{MainText, TextTone};
pub use phone_input::{PhoneCountry, PhoneCountryCatalog};
pub use skeleton::{SkeletonShape, SkeletonUi};

use gpui::App;
use rise_theme::AppTheme;

/// The installed theme.
///
/// # Panics
///
/// If [`install_theme`] has not run. That is the intended failure: a screen
/// drawn against a default palette looks almost right and ships, whereas a panic
/// at startup is found on the first run.
pub fn theme(cx: &App) -> &AppTheme {
    cx.global::<AppTheme>()
}

/// Installs the palette every component reads from. Call once, during app
/// setup, before the first window opens.
///
/// Calling it again replaces the theme — that is how a light/dark switch is
/// applied — but gpui does not repaint open windows on its own, so the caller
/// notifies whatever is on screen.
pub fn install_theme(theme: AppTheme, cx: &mut App) {
    cx.set_global(theme);
}

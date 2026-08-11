//! The desktop UI kit: the components `riseonly-ios` screens are built from,
//! ported 1:1 on top of gpui.
//!
//! Most components — [`BoxUi`], [`ButtonUi`], [`MainText`], [`IconUi`],
//! [`SkeletonUi`], [`FlagUi`] — are namespaces of associated functions returning
//! a styled [`gpui::Div`], so the call site keeps composing with gpui's own
//! builders. [`InputUi`] is the exception: a text field owns selection, undo,
//! IME and caret state, so it is an entity with [`InputUiState`] behind it.
//!
//! Colours, sizes and radii come from [`rise_theme::AppTheme`], installed once
//! per app with [`install_theme`] and read back with [`theme`]. A component
//! takes `&AppTheme` as an argument rather than reaching for the global.

pub mod animation;
pub mod avatar_ui;
pub mod badge_ui;
pub mod box_ui;
pub mod button_ui;
pub mod code_field_ui;
pub mod flag_ui;
pub mod fonts;
pub mod icon_ui;
pub mod image_ui;
pub mod input_ui;
pub mod list_ui;
pub mod main_text;
pub mod phone_input;
pub mod skeleton;
pub mod tabs_ui;

pub use avatar_ui::{AvatarSpec, AvatarUi, StoryRing};
pub use badge_ui::BadgeUi;
pub use box_ui::BoxUi;
pub use button_ui::{ButtonSize, ButtonTone, ButtonUi};
pub use code_field_ui::CodeFieldUi;
pub use flag_ui::FlagUi;
pub use icon_ui::{IconSize, IconUi};
pub use image_ui::{ImageLimits, ImageUi, RiseImageCache};
pub use input_ui::{InputUi, InputUiState};
pub use list_ui::{EdgeState, ListUi, PaginationEdge, ScrollProbe};
pub use main_text::{MainText, TextTone};
pub use phone_input::{PhoneCountry, PhoneCountryCatalog, PhoneEdit, PhoneNumberEntry};
pub use skeleton::{SkeletonShape, SkeletonUi};
pub use tabs_ui::{TabItem, TabsEvent, TabsUiState};

use gpui::App;
use rise_theme::AppTheme;

/// The installed theme.
///
/// # Panics
///
/// If [`install_theme`] has not run.
pub fn theme(cx: &App) -> &AppTheme {
    cx.global::<AppTheme>()
}

/// Installs the palette every component reads from. Call once, during app
/// setup, before the first window opens.
///
/// Calling it again replaces the theme, but gpui does not repaint open windows
/// on its own — the caller notifies whatever is on screen.
pub fn install_theme(theme: AppTheme, cx: &mut App) {
    cx.set_global(theme);
}

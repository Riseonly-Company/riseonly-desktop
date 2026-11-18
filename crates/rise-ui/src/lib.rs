// @generated mod list begin: scripts/gen-mod-tree.py from scripts/crate-map.json. Do not edit.
pub mod animated_media_playback_scope;
pub mod animated_tabs_ui;
pub mod badge_ui;
pub mod bg_wrapper_ui;
pub mod bot_icon;
pub mod bottom_sheet_ui;
pub mod button_ui;
#[cfg(test)]
mod button_ui_tests;
pub mod context_menu_ui;
pub mod download_selection_badge;
pub mod error_text_ui;
pub mod expandable_text_ui;
pub mod expandable_text_ui_hooks;
pub mod filter_pill_ui;
pub mod flag_icon_ui;
pub mod fonts;
pub mod frame_request;
pub mod grouped_btns;
pub mod guide_anchor_ui;
pub mod guide_overlay_ui;
pub mod hold_context_menu_ui;
pub mod icon_ui;
pub mod image_ui;
pub mod in_window_prompt;
#[cfg(test)]
mod in_window_prompt_tests;
pub mod input_ui;
pub mod keyboard_dismiss;
#[cfg(test)]
mod keyboard_dismiss_tests;
pub mod layer_keys;
pub mod liquid_chrome;
#[cfg(test)]
mod liquid_chrome_tests;
pub mod liquid_glass_button;
#[cfg(test)]
mod liquid_glass_button_tests;
pub mod list_ui;
pub mod live_voice_bars_ui;
#[cfg(test)]
mod live_voice_bars_ui_tests;
pub mod loader_ui;
pub mod loader_ui_hooks;
#[cfg(test)]
mod loader_ui_tests;
pub mod lottie_view;
pub mod main_text;
#[cfg(test)]
mod main_text_tests;
pub mod media_spoiler_overlay_view;
pub mod message_bubble_geometry;
pub mod message_media_tail_presentation;
pub mod native_confirm_alert;
pub mod official_badge;
pub mod page_header_toolbar_button;
pub mod page_header_ui;
pub mod pager_back_swipe_navigation_hooks;
pub mod phone_country_catalog;
pub mod phone_input;
pub mod pressable_ui;
#[cfg(test)]
mod pressable_ui_tests;
pub mod qr_code;
pub mod reorder_grip;
#[cfg(test)]
mod reorder_grip_tests;
pub mod rich_text;
pub mod rise_frame_rate;
pub mod rise_symbol;
pub mod riseonly_app_logo_view;
pub mod screen_capture_protected_ui;
pub mod scroll_chrome_shadow_ui;
pub mod scroll_edge_blur_ui;
#[cfg(test)]
mod scroll_edge_blur_ui_tests;
pub mod search_field_ui;
pub mod secondary_text;
pub mod section_reveal_ui;
pub mod settings_icon_palette;
pub mod shared;
#[cfg(test)]
mod shared_tests;
pub mod sheet_header_bar;
pub mod simple_button_ui;
#[cfg(test)]
mod simple_button_ui_tests;
pub mod skeleton_ui;
pub mod status_bar_style_anchor;
pub mod stepped_slider_ui;
pub mod swiftui;
pub mod switch_ui;
pub mod text_field_alert;
pub mod text_input_caret;
pub mod top_presenter;
pub mod ui_text_view_content_height;
pub mod unread_count_badge_ui;
pub mod user_logo_story_hooks;
pub mod view_anchor_probe;
pub mod view_screen_snapshot;
pub mod window_cover;
#[cfg(test)]
mod window_cover_tests;
pub mod window_screen_frame_reader;
// @generated mod list end

pub use badge_ui::BadgeUi;
pub use button_ui::{ButtonUi, ButtonUiSurface};
pub use icon_ui::{IconSize, IconUi};
pub use image_ui::{ImageLimits, RiseImageCache};
pub use input_ui::{InputUi, InputUiState};
pub use list_ui::ListUi;
pub use main_text::{MainText, TextAlignT};
pub use pressable_ui::PressableUi;
pub use phone_input::{PhoneCountry, PhoneCountryCatalog};
pub use secondary_text::SecondaryText;
pub use simple_button_ui::{SimpleButtonUi, SimpleButtonVariant};

use gpui::App;
use rise_theme::AppTheme;

/// the installed theme, panics if install_theme hasnt run
pub fn theme(cx: &App) -> &AppTheme {
    cx.global::<AppTheme>()
}

/// installs the palette every component reads, call once before the first window opens
/// calling again replaces it but open windows dont repaint on their own
pub fn install_theme(theme: AppTheme, cx: &mut App) {
    theme.publish_focus_ring_color();
    cx.set_global(theme);
}

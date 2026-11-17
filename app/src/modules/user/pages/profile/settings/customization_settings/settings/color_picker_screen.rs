use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, AppContext, Context, Div, ElementId, Entity, Focusable, FontWeight, Hsla, ScrollHandle, SharedString, Stateful,
    Subscription, TextAlign, Window, canvas, point, px, relative, white,
};
use rise_canon::rise_theme_tokens::HEX_RGB_DIGITS;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::COLOR_PICKER_INPUT_FORMAT;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemePalette, ThemeStore, alpha, color};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::input_ui::{InputUiEvent, InputUiState};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::angular_gradient::{AngularGradient, angular_gradient_circle_stroke};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::folders::chat_folder_edit_sheet::folder_hex_string;
use crate::modules::media_picker::editor::editor_color_picker_bridge::EditorColorSpectrum;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;

const DOT_SIDE: f32 = 36.0;
const PREVIEW_TAB_COUNT: usize = 3;

/// which part of the product a color page previews, drawn with the real components
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPreviewKind {
    Background,
    Button,
    Primary,
    Text,
    SecondaryText,
    Border,
}

/// swatches per appearance so a light theme never gets dark backgrounds or white text
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorPresetSet {
    pub light: &'static [&'static str],
    pub dark: &'static [&'static str],
}

impl ColorPresetSet {
    pub const fn new(light: &'static [&'static str], dark: &'static [&'static str]) -> Self {
        Self { light, dark }
    }

    pub const fn both(both: &'static [&'static str]) -> Self {
        Self::new(both, both)
    }

    pub fn list(&self, appearance: RiseThemeAppearance) -> &'static [&'static str] {
        if appearance == RiseThemeAppearance::Light { self.light } else { self.dark }
    }
}

/// per page palettes, background wants shades and accent wants hues
pub enum ColorPagePresets {}

impl ColorPagePresets {
    pub const fn accents() -> ColorPresetSet {
        ColorPresetSet::both(&[
            "#FF5A5A", "#FF7A00", "#FFC700", "#43A047", "#00BFA5", "#1E88E5", "#3949AB", "#5E35B1", "#8E24AA", "#D81B60",
            "#0088FF", "#34C759", "#5AC8FA", "#E84393", "#6C5CE7",
        ])
    }

    pub const fn surfaces() -> ColorPresetSet {
        ColorPresetSet::new(
            &[
                "#FFFFFF", "#F7F7F7", "#F2F2F7", "#EFEFF4", "#E5E5EA", "#FAF7F2", "#F0F4FA", "#F5F0F7", "#EFF6F1", "#FFF8E7",
            ],
            &[
                "#000000", "#0A0A0A", "#121212", "#1C1C1E", "#242424", "#2C2C2E", "#0F1420", "#141A26", "#1A1420", "#0F1A16",
            ],
        )
    }

    pub const fn inks() -> ColorPresetSet {
        ColorPresetSet::new(
            &[
                "#000000", "#1C1C1E", "#3A3A3C", "#48484A", "#6E6E73", "#8E8E93", "#5C1A1A", "#0B2E5C", "#1B4D26", "#5C4300",
            ],
            &[
                "#FFFFFF", "#F2F2F7", "#E5E5EA", "#C8C8C8", "#AEAEB2", "#8E8E93", "#FFE3E3", "#DCEBFF", "#E2F5E4", "#FFF2CC",
            ],
        )
    }

    pub const fn lines() -> ColorPresetSet {
        ColorPresetSet::new(
            &[
                "#C8C7CC", "#D1D1D6", "#D6D6DC", "#E5E5EA", "#AEAEB2", "#8E8E93", "#6E6E73", "#FF5A5A", "#1E88E5", "#43A047",
            ],
            &[
                "#252525", "#2B2B2B", "#313131", "#3A3A3A", "#48484A", "#5A5A5E", "#6E6E73", "#FF5A5A", "#1E88E5", "#43A047",
            ],
        )
    }
}

type GetCurrent = fn(&ThemePalette) -> String;
type SetColor = fn(&mut ThemePalette, String);

/// page state plus the stores whose changes re-render it
struct ColorPickerScreenState {
    preview_toggle: bool,
    preview_tab: usize,
    preview_tab_fill: [Animated<f32>; PREVIEW_TAB_COUNT],
    palette_scroll: ScrollHandle,
    shows_custom_picker: bool,
    _subscriptions: Vec<Subscription>,
}

impl ColorPickerScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            preview_toggle: true,
            preview_tab: 0,
            preview_tab_fill: [Animated::new(1.0), Animated::new(0.0), Animated::new(0.0)],
            palette_scroll: ScrollHandle::new(),
            shows_custom_picker: false,
            _subscriptions: vec![
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn select_preview_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.preview_tab = index;
        let now = RiseClock::monotonic();
        for (tab, fill) in self.preview_tab_fill.iter_mut().enumerate() {
            let target = if tab == index { 1.0 } else { 0.0 };
            fill.set_in(target, spring(0.28, 0.85), now, cx);
        }
        cx.notify();
    }
}

fn apply(hex: &str, set_color: SetColor, cx: &mut App) {
    ThemeStore::shared(cx).update(cx, |store, cx| {
        let mut palette = store.current_palette().clone();
        set_color(&mut palette, hex.to_owned());
        store.apply(palette, cx);
    });
    GuideSignalBus::emit(RiseGuideSignal::ThemeColorChanged);
}

fn apply_builtin_default_for_this_field(get_current: GetCurrent, set_color: SetColor, cx: &mut App) {
    ThemeStore::shared(cx).update(cx, |store, cx| {
        let reference = store.reference_builtin_palette();
        let mut palette = store.current_palette().clone();
        set_color(&mut palette, get_current(&reference));
        store.apply(palette, cx);
    });
}

/// scrolls the palette to center on the next frame once the dots are laid out
fn scroll_palette_to_center(handle: ScrollHandle, index: usize, window: &mut Window) {
    window.on_next_frame(move |window, _| {
        let Some(item) = handle.bounds_for_item(index) else {
            return;
        };
        let viewport = handle.bounds();
        let center = f32::from(item.center().x) - f32::from(viewport.origin.x);
        let target = f32::from(viewport.size.width) / 2.0 - center;
        let farthest = -f32::from(handle.max_offset().x);
        handle.set_offset(point(px(target.clamp(farthest.min(0.0), 0.0)), px(0.0)));
        window.refresh();
    });
}

/// content ignores hits but the wheel still scrolls the page under it
fn not_hit_testable(content: impl IntoElement) -> Div {
    div()
        .relative()
        .w_full()
        .child(content)
        .child(div().absolute().inset_0().block_mouse_except_scroll())
}

fn separator(color: Hsla, leading: f32) -> impl IntoElement {
    div().w_full().pl(px(leading)).child(div().w_full().h(px(0.5)).bg(color))
}

fn chevron(theme: &AppTheme) -> Option<gpui::Svg> {
    IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(12.0), alpha(theme.text.secondary, 0.75), false)
}

struct PreviewProfile {
    source: Option<SharedString>,
    name: SharedString,
    tag: String,
    user_id: Option<SharedString>,
}

fn preview_profile(cx: &mut App) -> PreviewProfile {
    let auth = AuthServicesStore::shared(cx);
    let profile = auth.read(cx).profile();
    PreviewProfile {
        source: profile
            .and_then(|profile| profile.avatar_url.clone().or_else(|| Some(profile.more.logo.clone())))
            .map(SharedString::from),
        name: SharedString::from(profile.map_or_else(|| "Riseonly".to_owned(), |profile| profile.name.clone())),
        tag: profile.map_or_else(|| "riseonly".to_owned(), |profile| profile.tag.clone()),
        user_id: profile.map(|profile| SharedString::from(profile.id.clone())),
    }
}

#[derive(IntoElement)]
pub struct ColorPickerScreen {
    title_key: &'static str,
    preview: ColorPreviewKind,
    presets: ColorPresetSet,
    get_current: GetCurrent,
    set_color: SetColor,
}

impl ColorPickerScreen {
    pub fn new(
        title_key: &'static str,
        preview: ColorPreviewKind,
        presets: ColorPresetSet,
        get_current: GetCurrent,
        set_color: SetColor,
    ) -> Self {
        Self { title_key, preview, presets, get_current, set_color }
    }

    fn section_header(text: String, theme: &AppTheme) -> impl IntoElement + use<> {
        div()
            .px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING))
            .child(MainText::new(text.to_uppercase()).px(12.0).color(theme.text.secondary))
    }

    fn palette_row(
        &self,
        current: &str,
        state: &Entity<ColorPickerScreenState>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let appearance: RiseThemeAppearance = ThemeStore::shared(cx).read(cx).resolved_appearance();
        let list = self.presets.list(appearance);
        let selected_preset = list.iter().position(|hex| hex.eq_ignore_ascii_case(current));
        let target = selected_preset.unwrap_or(list.len());
        let handle = state.read(cx).palette_scroll.clone();

        let changed_handle = handle.clone();
        on_change("color-palette-appearance", appearance, false, window, cx, move |_, _, window, _| {
            scroll_palette_to_center(changed_handle, target, window);
        });

        let set_color = self.set_color;
        let dots = list.iter().enumerate().map(|(index, hex)| {
            Self::color_dot(index, *hex, selected_preset == Some(index), set_color, theme).into_any_element()
        });
        let appeared_handle = handle.clone();
        div()
            .id("color-palette-row")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(14.0))
            .p(px(16.0))
            .overflow_x_scroll()
            .track_scroll(&handle)
            .children(dots.collect::<Vec<_>>())
            .child(Self::custom_color_dot(current, selected_preset.is_none(), state, theme))
            .on_appear("color-palette-appear", move |window, _| {
                scroll_palette_to_center(appeared_handle, target, window);
            })
    }

    fn color_dot(
        index: usize,
        hex: &'static str,
        selected: bool,
        set_color: SetColor,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = selection_changed();
            apply(hex, set_color, cx);
        });
        let dot = div()
            .id(("color-dot", index))
            .relative()
            .flex_none()
            .size(px(DOT_SIDE))
            .circle()
            .bg(color(hex));
        button_chrome(dot, false, Some(action))
            .tooltip_label(hex.to_uppercase())
            .child(centred_stroke(alpha(theme.border._200, 0.7), 0.5, Shape::Circle))
            .when(selected, |dot| {
                dot.child(centred_stroke(theme.bg._200, 3.0, Shape::Circle)).child(Self::selection_ring(theme))
            })
    }

    /// selection ring, 2 wide and 3 outside
    fn selection_ring(theme: &AppTheme) -> impl IntoElement + use<> {
        div()
            .absolute()
            .inset(px(-3.0))
            .child(centred_stroke(theme.primary._100, 2.0, Shape::Circle))
    }

    fn custom_color_dot(
        current: &str,
        selected: bool,
        state: &Entity<ColorPickerScreenState>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let fill = color(current);
        let state = state.clone();
        let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = selection_changed();
            state.update(cx, |state, cx| {
                state.shows_custom_picker = true;
                cx.notify();
            });
        });
        let rainbow = AngularGradient::new([
            color("#FF3B30"),
            color("#FFCC00"),
            color("#34C759"),
            color("#32ADE6"),
            color("#007AFF"),
            color("#AF52DE"),
            color("#FF3B30"),
        ]);
        let dot = div().id("color-custom-dot").relative().flex_none().size(px(DOT_SIDE)).circle();
        button_chrome(dot, false, Some(action))
            .tooltip_label(tr("settings_color_custom"))
            .when(selected, |dot| dot.bg(fill))
            .child(div().absolute().inset_0().child(angular_gradient_circle_stroke(rainbow, 3.0, DOT_SIDE)))
            .when(selected, |dot| dot.child(Self::selection_ring(theme)))
    }

    fn background_preview(profile: &PreviewProfile, theme: &AppTheme) -> AnyElement {
        vstack(14.0)
            .w_full()
            .p(px(14.0))
            .bg(theme.bg._100)
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(4.0))
                    .child(
                        UserLogoUi::new("color-preview-background-logo")
                            .source(profile.source.clone())
                            .name(profile.name.clone())
                            .user_id(profile.user_id.clone())
                            .size(46.0),
                    )
                    .child(
                        vstack(3.0)
                            .items_start()
                            .child(MainText::new(profile.name.clone()).px(16.0).font_weight(FontWeight::SEMIBOLD))
                            .child(SecondaryText::new(format!("@{}", profile.tag)).px(13.0)),
                    )
                    .child(spacer_min_w(0.0)),
            )
            .child(not_hit_testable(
                GroupedBtns::new(
                    "color-preview-background-rows",
                    vec![
                        GroupedBtn::new("preview_notifications", tr("settings_notifications_title"))
                            .icon("bell.fill")
                            .icon_background(SettingsIconPalette::notifications()),
                        GroupedBtn::new("preview_privacy", tr("settings_privacy_title"))
                            .icon("lock.fill")
                            .icon_background(SettingsIconPalette::privacy()),
                    ],
                )
                .horizontal_padding(0.0),
            ))
            .into_any_element()
    }

    fn button_preview(theme: &AppTheme) -> AnyElement {
        vstack(14.0)
            .w_full()
            .items_start()
            .p(px(14.0))
            .bg(theme.bg._200)
            .child(MainText::new(tr("preview_post_title")).px(15.0).font_weight(FontWeight::SEMIBOLD))
            .child(MainText::new(tr("preview_post_content")).px(13.0).color(theme.text.secondary).number_of_lines(2))
            .child(
                hstack(8.0)
                    .w_full()
                    .child(Self::preview_pill("heart", "128".to_owned(), theme))
                    .child(Self::preview_pill("bubble.right", "14".to_owned(), theme))
                    .child(Self::preview_pill("arrowshape.turn.up.right", tr("ctx_share"), theme))
                    .child(spacer_min_w(0.0)),
            )
            .child(Self::preview_sign_in_button("color-preview-button-signin", theme))
            .into_any_element()
    }

    fn preview_sign_in_button(id: &'static str, theme: &AppTheme) -> impl IntoElement + use<> {
        not_hit_testable(
            ButtonUi::new(id).on_press(|_, _| {}).child(
                MainText::new(tr("signin"))
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.content_on(theme.primary._100)),
            ),
        )
    }

    fn preview_pill(icon: &'static str, text: String, theme: &AppTheme) -> impl IntoElement + use<> {
        hstack(6.0)
            .h(px(34.0))
            .px(px(12.0))
            .bg(theme.button.bg_300)
            .clip_shape(Shape::Capsule)
            .children(IconUi::sized(icon, px(13.0), theme.button.content_300, false))
            .child(MainText::new(text).px(13.0).font_weight(FontWeight::MEDIUM).color(theme.button.content_300))
    }

    fn primary_preview(
        state: &Entity<ColorPickerScreenState>,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let toggled = state.clone();
        let toggle: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            toggled.update(cx, |state, cx| {
                state.preview_toggle = !state.preview_toggle;
                cx.notify();
            });
        });
        let binding = Binding::entity(
            state.clone(),
            |state: &ColorPickerScreenState| state.preview_toggle,
            |state: &mut ColorPickerScreenState, value, cx| {
                state.preview_toggle = value;
                cx.notify();
            },
        );
        let hit = hit_area(Shape::Rectangle).occlude().id("color-preview-toggle-row");
        let toggle_row = hstack(DEFAULT_STACK_SPACING)
            .relative()
            .w_full()
            .child(MainText::new(tr("settings_notifications_title")).px(15.0))
            .child(spacer())
            .child(SwitchUi::new("color-preview-switch", binding))
            .child(button_chrome(hit, false, Some(toggle)));

        let now = RiseClock::monotonic();
        let preview = state.read(cx);
        let titles = [tr("all_chats"), tr("chats_title"), tr("saved_messages")];
        let tabs = titles.into_iter().enumerate().map(|(index, title)| {
            let selected = preview.preview_tab == index;
            let fill = preview.preview_tab_fill[index].frame_value(now, window);
            let select = state.clone();
            plain(("color-preview-tab", index))
                .on_press(move |_, cx| {
                    let _ = selection_changed();
                    select.update(cx, |state, cx| state.select_preview_tab(index, cx));
                })
                .child(
                    div()
                        .relative()
                        .h(px(32.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .bg(theme.bg._300)
                        .clip_shape(Shape::Capsule)
                        .child(div().absolute().inset_0().rounded_full().bg(theme.primary._100).opacity(fill))
                        .child(
                            div().relative().child(
                                MainText::new(title).px(13.0).font_weight(FontWeight::SEMIBOLD).color(if selected {
                                    theme.content_on(theme.primary._100)
                                } else {
                                    theme.text.primary
                                }),
                            ),
                        ),
                )
                .into_any_element()
        });
        let tab_row = hstack(8.0).w_full().children(tabs.collect::<Vec<_>>()).child(spacer_min_w(0.0));

        let tick = div()
            .size(px(21.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .circle()
            .bg(theme.primary._100)
            .children(IconUi::sized("checkmark", px(12.0), theme.content_on(theme.primary._100), false));

        vstack(14.0)
            .w_full()
            .p(px(14.0))
            .bg(theme.bg._200)
            .child(toggle_row)
            .child(tab_row)
            .child(
                hstack(10.0)
                    .w_full()
                    .child(tick)
                    .child(MainText::new(tr("theme_follow_system")).px(15.0))
                    .child(spacer())
                    .child(MainText::new("riseonly.net").px(14.0).primary(true)),
            )
            .child(Self::preview_sign_in_button("color-preview-primary-signin", theme))
            .into_any_element()
    }

    fn text_preview(profile: &PreviewProfile, theme: &AppTheme) -> AnyElement {
        vstack(10.0)
            .w_full()
            .items_start()
            .p(px(14.0))
            .bg(theme.bg._200)
            .child(
                hstack(10.0)
                    .w_full()
                    .child(
                        UserLogoUi::new("color-preview-text-logo")
                            .source(profile.source.clone())
                            .name(profile.name.clone())
                            .user_id(profile.user_id.clone())
                            .size(36.0),
                    )
                    .child(
                        vstack(2.0)
                            .items_start()
                            .child(MainText::new(profile.name.clone()).px(14.0).font_weight(FontWeight::SEMIBOLD))
                            .child(SecondaryText::new(format!("@{}", profile.tag)).px(12.0)),
                    )
                    .child(spacer_min_w(0.0)),
            )
            .child(MainText::new(tr("preview_post_title")).px(17.0).font_weight(FontWeight::BOLD))
            .child(MainText::new(tr("preview_post_content")).px(15.0).line_height(21.0).number_of_lines(4))
            .child(
                hstack(14.0)
                    .w_full()
                    .child(Self::preview_meta("heart", "128", theme))
                    .child(Self::preview_meta("bubble.right", "14", theme))
                    .child(Self::preview_meta("eye", "2.4K", theme))
                    .child(spacer_min_w(0.0)),
            )
            .into_any_element()
    }

    fn preview_meta(icon: &'static str, text: &'static str, theme: &AppTheme) -> impl IntoElement + use<> {
        hstack(5.0)
            .children(IconUi::sized(icon, px(13.0), theme.text.primary, false))
            .child(MainText::new(text).px(13.0).font_weight(FontWeight::MEDIUM))
    }

    fn secondary_text_preview(theme: &AppTheme) -> AnyElement {
        let line = alpha(theme.border._200, 0.55);
        vstack(0.0)
            .w_full()
            .bg(theme.bg._200)
            .child(Self::preview_chat_row(
                "Riseonly".to_owned(),
                tr("settings_quick_reaction_preview_message"),
                "12:45".to_owned(),
                Some("2"),
                theme,
            ))
            .child(separator(line, 72.0))
            .child(Self::preview_chat_row(
                tr("saved_messages"),
                tr("wallpaper_preview_how_are_you"),
                tr("date_yesterday_short"),
                None,
                theme,
            ))
            .child(separator(line, 72.0))
            .child(
                hstack(DEFAULT_STACK_SPACING)
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(MainText::new(tr("settings_customization_title")).px(15.0))
                    .child(spacer())
                    .child(MainText::new(tr("theme_system")).px(14.0).color(theme.text.secondary))
                    .children(chevron(theme)),
            )
            .into_any_element()
    }

    fn preview_chat_row(
        name: String,
        message: String,
        time: String,
        unread: Option<&'static str>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let on_primary = theme.content_on(theme.primary._100);
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .child(
                UserLogoUi::new(format!("color-preview-chat-logo-{name}"))
                    .name(SharedString::from(name.clone()))
                    .user_id(SharedString::from(format!("preview-{name}")))
                    .size(46.0),
            )
            .child(
                vstack(3.0)
                    .flex_1()
                    .min_w_0()
                    .child(
                        hstack(DEFAULT_STACK_SPACING)
                            .w_full()
                            .child(
                                MainText::new(name)
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .number_of_lines(1)
                                    .min_w_0(),
                            )
                            .child(spacer_min_w(6.0))
                            .child(SecondaryText::new(time).px(12.0)),
                    )
                    .child(
                        hstack(DEFAULT_STACK_SPACING)
                            .w_full()
                            .items_start()
                            .child(SecondaryText::new(message).px(14.0).line_limit(1).min_w_0())
                            .child(spacer_min_w(6.0))
                            .children(unread.map(|unread| {
                                div()
                                    .h(px(20.0))
                                    .px(px(7.0))
                                    .flex()
                                    .items_center()
                                    .bg(theme.primary._100)
                                    .clip_shape(Shape::Capsule)
                                    .child(
                                        MainText::new(unread)
                                            .px(12.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(on_primary),
                                    )
                            })),
                    ),
            )
    }

    fn border_preview(theme: &AppTheme) -> AnyElement {
        let line = theme.border._100;
        let titles = [tr("ctx_reply"), tr("ctx_forward"), tr("ctx_copy")];
        let last = titles.len() - 1;
        let rows = titles.into_iter().enumerate().map(|(index, title)| {
            vstack(0.0)
                .w_full()
                .child(
                    hstack(DEFAULT_STACK_SPACING)
                        .w_full()
                        .px(px(14.0))
                        .py(px(11.0))
                        .child(MainText::new(title).px(15.0))
                        .child(spacer())
                        .children(chevron(theme)),
                )
                .when(index < last, |row| row.child(separator(line, 14.0)))
                .into_any_element()
        });
        let shape = Shape::continuous(14.0);
        vstack(14.0)
            .w_full()
            .p(px(14.0))
            .bg(theme.bg._200)
            .child(
                hstack(12.0)
                    .w_full()
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .size(px(44.0))
                            .child(
                                UserLogoUi::new("color-preview-border-logo")
                                    .name(SharedString::from("Riseonly"))
                                    .user_id(SharedString::from("preview-border"))
                                    .size(44.0),
                            )
                            .child(centred_stroke(line, 2.0, Shape::Circle)),
                    )
                    .child(
                        vstack(3.0)
                            .items_start()
                            .child(MainText::new("Riseonly").px(15.0).font_weight(FontWeight::SEMIBOLD))
                            .child(SecondaryText::new(tr("wallpaper_preview_hi")).px(13.0)),
                    )
                    .child(spacer_min_w(0.0))
                    .child(
                        div()
                            .relative()
                            .h(px(32.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .child(MainText::new(tr("ctx_reply")).px(13.0).font_weight(FontWeight::MEDIUM))
                            .child(centred_stroke(line, 1.0, Shape::Capsule)),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(
                        vstack(0.0)
                            .w_full()
                            .bg(alpha(theme.bg._300, 0.5))
                            .clip_shape(shape)
                            .children(rows.collect::<Vec<_>>()),
                    )
                    .child(centred_stroke(line, 1.0, shape)),
            )
            .into_any_element()
    }

    fn custom_picker_sheet(
        state: &Entity<ColorPickerScreenState>,
        get_current: GetCurrent,
        set_color: SetColor,
        theme: &AppTheme,
    ) -> NativeBottomSheetPresentationHost {
        let closing = state.clone();
        let is_presented = Binding::entity(
            state.clone(),
            |state: &ColorPickerScreenState| state.shows_custom_picker,
            |state: &mut ColorPickerScreenState, shows, cx| {
                state.shows_custom_picker = shows;
                cx.notify();
            },
        );
        NativeBottomSheetPresentationHost::new("color-custom-picker-sheet", is_presented, move |_, cx| {
            let current = get_current(ThemeStore::shared(cx).read(cx).current_palette());
            let closed = closing.clone();
            let apply_color: ColorApply = Rc::new(move |hex: &str, cx: &mut App| apply(hex, set_color, cx));
            let close: ColorClose = Rc::new(move |cx: &mut App| {
                closed.update(cx, |state, cx| {
                    state.shows_custom_picker = false;
                    cx.notify();
                });
            });
            CustomColorPickerSheet { current, apply: apply_color, close }
        })
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
        .presentation_background(theme.bg._100)
    }

    fn preview_card(
        &self,
        state: &Entity<ColorPickerScreenState>,
        theme: &AppTheme,
        window: &Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let content = match self.preview {
            ColorPreviewKind::Background => Self::background_preview(&preview_profile(cx), theme),
            ColorPreviewKind::Button => Self::button_preview(theme),
            ColorPreviewKind::Primary => Self::primary_preview(state, theme, window, cx),
            ColorPreviewKind::Text => Self::text_preview(&preview_profile(cx), theme),
            ColorPreviewKind::SecondaryText => Self::secondary_text_preview(theme),
            ColorPreviewKind::Border => Self::border_preview(theme),
        };
        div()
            .relative()
            .w_full()
            .child(div().w_full().clip_shape(GroupedCardMetrics::SHAPE).child(content))
            .child(centred_stroke(alpha(theme.border._200, 0.6), 0.5, GroupedCardMetrics::SHAPE))
    }
}

impl RenderOnce for ColorPickerScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(format!("color-picker-screen-state-{}", self.title_key), cx, |_, cx| {
            ColorPickerScreenState::new(cx)
        });
        let nav_store = env::<NavigationStoreKey>(cx);
        let current = (self.get_current)(ThemeStore::shared(cx).read(cx).current_palette());
        let get_current = self.get_current;
        let set_color = self.set_color;
        let custom_picker = Self::custom_picker_sheet(&state, get_current, set_color, &theme);

        let preview_section = vstack(8.0)
            .w_full()
            .items_start()
            .child(Self::section_header(tr("settings_preview_header"), &theme))
            .child(
                div()
                    .w_full()
                    .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
                    .child(self.preview_card(&state, &theme, window, cx)),
            );

        let palette_row = self.palette_row(&current, &state, &theme, window, cx);
        let palette_section = vstack(8.0)
            .w_full()
            .items_start()
            .child(
                hstack(DEFAULT_STACK_SPACING)
                    .w_full()
                    .child(Self::section_header(tr("settings_color_palette"), &theme))
                    .child(spacer())
                    .child(
                        div().pr(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(
                            MainText::new(current.to_uppercase())
                                .px(12.0)
                                .color(theme.text.secondary)
                                .monospaced_digits(true),
                        ),
                    ),
            )
            .child(
                div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                    div()
                        .w_full()
                        .bg(theme.bg._200)
                        .clip_shape(GroupedCardMetrics::SHAPE)
                        .child(palette_row)
                        .guide_anchor(RiseGuideAnchor::ColorPickerPalette),
                ),
            );

        let reset = GroupedBtns::new(
            "color-reset-group",
            vec![
                GroupedBtn::new("color_reset_default", tr("settings_reset_default"))
                    .icon("arrow.counterclockwise")
                    .icon_background(SettingsIconPalette::reset())
                    .show_chevron(false)
                    .callback(move |_, cx| apply_builtin_default_for_this_field(get_current, set_color, cx)),
            ],
        )
        .end_group_title(tr("settings_color_reset_hint"));

        let sections = vstack(20.0)
            .w_full()
            .pt(px(12.0))
            .child(preview_section)
            .child(palette_section)
            .child(reset)
            .child(spacer_min_h(80.0))
            .music_mini_player_bottom_padding(0.0, 0.0);

        SettingsWrapperUi::new(tr(self.title_key), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            div()
                .relative()
                .size_full()
                .child(
                    scroll_view("color-picker-screen-scroll")
                        .size_full()
                        .bg(theme.bg._100)
                        .list_shell_scroll_under_chrome(cx)
                        .child(sections),
                )
                .child(custom_picker),
        )
    }
}

type ColorApply = Rc<dyn Fn(&str, &mut App)>;
type ColorClose = Rc<dyn Fn(&mut App)>;

/// how a custom color is typed under the spectrum
/// no alpha since theme colors are opaque
/// same in all languages
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorInputFormat {
    Hex,
    Rgb,
    Hsl,
}

impl ColorInputFormat {
    pub const ALL_CASES: [Self; 3] = [Self::Hex, Self::Rgb, Self::Hsl];

    const STORAGE_KEY: &'static str = COLOR_PICKER_INPUT_FORMAT;

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::Rgb => "rgb",
            Self::Hsl => "hsl",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|format| format.raw_value() == raw_value)
    }

    pub fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn title(self) -> String {
        self.raw_value().to_uppercase()
    }

    /// letter and upper bound of each numeric field, hex is one text field instead
    pub fn channels(self) -> Vec<ColorInputChannel> {
        match self {
            Self::Hex => Vec::new(),
            Self::Rgb => vec![
                ColorInputChannel { letter: "R", max: 255 },
                ColorInputChannel { letter: "G", max: 255 },
                ColorInputChannel { letter: "B", max: 255 },
            ],
            Self::Hsl => vec![
                ColorInputChannel { letter: "H", max: 360 },
                ColorInputChannel { letter: "S", max: 100 },
                ColorInputChannel { letter: "L", max: 100 },
            ],
        }
    }

    pub fn remembered() -> Self {
        UserDefaults::standard()
            .string(Self::STORAGE_KEY)
            .and_then(|raw_value| Self::from_raw_value(&raw_value))
            .unwrap_or(Self::Hex)
    }

    pub fn remember(self) {
        UserDefaults::standard().set(Self::STORAGE_KEY, self.raw_value());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorInputChannel {
    pub letter: &'static str,
    pub max: i64,
}

/// integer rgb and hsl of a six digit color as the numeric fields show them
pub enum ColorChannelMath {}

impl ColorChannelMath {
    pub fn rgb_from_digits(digits: &str) -> [i64; 3] {
        if digits.len() != HEX_RGB_DIGITS {
            return [0, 0, 0];
        }
        let Ok(value) = i64::from_str_radix(digits, 16) else {
            return [0, 0, 0];
        };
        [(value >> 16) & 0xFF, (value >> 8) & 0xFF, value & 0xFF]
    }

    pub fn digits_from_rgb(rgb: [i64; 3]) -> String {
        let [r, g, b] = rgb.map(|channel| channel.clamp(0, 255));
        format!("{r:02X}{g:02X}{b:02X}")
    }

    pub fn hsl_from_rgb(rgb: [i64; 3]) -> [i64; 3] {
        let [r, g, b] = rgb.map(|channel| channel as f64 / 255.0);
        let max_value = r.max(g).max(b);
        let min_value = r.min(g).min(b);
        let delta = max_value - min_value;
        let lightness = (max_value + min_value) / 2.0;
        if delta <= 0.0 {
            return [0, 0, (lightness * 100.0).round() as i64];
        }
        let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
        let mut hue = if max_value == r {
            60.0 * (((g - b) / delta) % 6.0)
        } else if max_value == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        if hue < 0.0 {
            hue += 360.0;
        }
        [(hue.round() as i64) % 360, (saturation * 100.0).round() as i64, (lightness * 100.0).round() as i64]
    }

    pub fn rgb_from_hsl(hsl: [i64; 3]) -> [i64; 3] {
        let hue = (hsl[0] % 360) as f64;
        let saturation = hsl[1] as f64 / 100.0;
        let lightness = hsl[2] as f64 / 100.0;
        let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
        let x = chroma * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
        let m = lightness - chroma / 2.0;
        let parts = if hue < 60.0 {
            [chroma, x, 0.0]
        } else if hue < 120.0 {
            [x, chroma, 0.0]
        } else if hue < 180.0 {
            [0.0, chroma, x]
        } else if hue < 240.0 {
            [0.0, x, chroma]
        } else if hue < 300.0 {
            [x, 0.0, chroma]
        } else {
            [chroma, 0.0, x]
        };
        parts.map(|part| ((part + m) * 255.0).round() as i64)
    }
}

const HEX_PLACEHOLDER: &str = "RRGGBB";
const FIELD_HEIGHT: f32 = 40.0;
const FORMAT_CHEVRON: &str = " \u{25BE}";
const HEX_DIGIT_SET: &str = "0123456789ABCDEF";
const CHANNEL_DIGITS: usize = 3;
const CHANNEL_COUNT: usize = 3;

/// state and focus of the custom color sheet, field editors stand for focus states
struct CustomColorPickerSheetState {
    format: ColorInputFormat,
    is_format_menu_presented: bool,
    hex_draft: String,
    hex_field: Entity<InputUiState>,
    channel_texts: Vec<String>,
    channel_fields: Vec<Entity<InputUiState>>,
    synced_texts: Vec<String>,
    typed_hex: Option<String>,
    current: String,
    apply: Option<ColorApply>,
    close: Option<ColorClose>,
    _subscriptions: Vec<Subscription>,
}

impl CustomColorPickerSheetState {
    fn new(current: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hex_field = cx.new(InputUiState::single_line);
        hex_field.update(cx, |field, cx| {
            field.set_chromeless(true, cx);
            field.set_escape_to_layer(false);
            field.set_height(Some(px(FIELD_HEIGHT)), cx);
            field.set_font_size(Some(px(17.0)), cx);
            field.set_placeholder(HEX_PLACEHOLDER, cx);
        });
        let channel_fields: Vec<Entity<InputUiState>> = (0..CHANNEL_COUNT)
            .map(|_| {
                let field = cx.new(InputUiState::single_line);
                field.update(cx, |field, cx| {
                    field.set_chromeless(true, cx);
                    field.set_escape_to_layer(false);
                    field.set_height(Some(px(FIELD_HEIGHT)), cx);
                    field.set_font_size(Some(px(15.0)), cx);
                    field.set_text_align(TextAlign::Right, cx);
                });
                field
            })
            .collect();
        let mut subscriptions = vec![cx.subscribe_in(&hex_field, window, |this, _, event: &InputUiEvent, window, cx| {
            this.hex_field_event(event, window, cx);
        })];
        let hex_focus = hex_field.focus_handle(cx);
        subscriptions.push(cx.on_focus(&hex_focus, window, |_, _, cx| cx.notify()));
        subscriptions.push(cx.on_blur(&hex_focus, window, |_, _, cx| cx.notify()));
        for (index, field) in channel_fields.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(field, window, move |this, _, event: &InputUiEvent, window, cx| {
                this.channel_field_event(index, event, window, cx);
            }));
            let focus = field.focus_handle(cx);
            subscriptions.push(cx.on_focus(&focus, window, |_, _, cx| cx.notify()));
            subscriptions.push(cx.on_blur(&focus, window, |_, _, cx| cx.notify()));
        }
        let mut state = Self {
            format: ColorInputFormat::remembered(),
            is_format_menu_presented: false,
            hex_draft: String::new(),
            hex_field,
            channel_texts: vec![String::new(); CHANNEL_COUNT],
            channel_fields,
            synced_texts: vec![String::new(); CHANNEL_COUNT],
            typed_hex: None,
            current: current.clone(),
            apply: None,
            close: None,
            _subscriptions: subscriptions,
        };
        state.fill_fields(&current, cx);
        state
    }

    fn sync_field_colors(&self, theme: &AppTheme, cx: &mut App) {
        let ink = Some(theme.text.primary);
        let prompt = Some(theme.text.secondary.opacity(0.82));
        for field in std::iter::once(&self.hex_field).chain(self.channel_fields.iter()) {
            let (text_color, placeholder_color) = {
                let field = field.read(cx);
                (field.text_color(), field.placeholder_color())
            };
            if text_color != ink || placeholder_color != prompt {
                field.update(cx, |field, cx| {
                    field.set_text_color(ink, cx);
                    field.set_placeholder_color(prompt, cx);
                    field.set_caret_color(ink, cx);
                });
            }
        }
    }

    fn sync_channel_prompts(&self, cx: &mut App) {
        for (field, channel) in self.channel_fields.iter().zip(self.format.channels()) {
            if &**field.read(cx).placeholder() != channel.letter {
                field.update(cx, |field, cx| field.set_placeholder(channel.letter, cx));
            }
        }
    }

    fn apply_color(&self, hex: &str, cx: &mut App) {
        if let Some(apply) = self.apply.clone() {
            apply(hex, cx);
        }
    }

    fn close_sheet(&self, cx: &mut App) {
        if let Some(close) = self.close.clone() {
            close(cx);
        }
    }

    fn hex_focused(&self, window: &Window, cx: &App) -> bool {
        self.hex_field.focus_handle(cx).is_focused(window)
    }

    fn focused_channel(&self, window: &Window, cx: &App) -> Option<usize> {
        self.channel_fields.iter().position(|field| field.focus_handle(cx).is_focused(window))
    }

    fn current_did_change(&mut self, new_value: &str, cx: &mut Context<Self>) {
        if self.typed_hex.as_deref().is_some_and(|typed| typed.eq_ignore_ascii_case(new_value)) {
            return;
        }
        self.typed_hex = None;
        self.fill_fields(new_value, cx);
        cx.notify();
    }

    fn set_format(&mut self, format: ColorInputFormat, window: &mut Window, cx: &mut Context<Self>) {
        if self.format == format {
            return;
        }
        self.format = format;
        format.remember();
        if self.focused_channel(window, cx).is_some() {
            window.blur();
        }
        let current = self.current.clone();
        self.fill_fields(&current, cx);
        cx.notify();
    }

    fn hex_field_event(&mut self, event: &InputUiEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputUiEvent::Changed => {
                let field = self.hex_field.read(cx);
                if field.marked_range().is_some() || field.text() == self.hex_draft {
                    return;
                }
                let typed = field.text().to_owned();
                self.hex_draft = typed.clone();
                self.hex_edited(&typed, window, cx);
                cx.notify();
            }
            InputUiEvent::Submitted => {
                window.blur();
                cx.notify();
            }
            InputUiEvent::Cancelled => self.close_sheet(cx),
        }
    }

    /// typed or pasted code, only hex digits stay uppercased and max six. a full code applies
    fn hex_edited(&mut self, typed: &str, window: &mut Window, cx: &mut Context<Self>) {
        let digits = Self::hex_digits(typed);
        if digits != typed {
            self.hex_draft = digits.clone();
            self.hex_field.update(cx, |field, cx| field.set_text(&digits, cx));
            self.hex_edited(&digits, window, cx);
            return;
        }
        if !self.hex_focused(window, cx) || digits.len() != HEX_RGB_DIGITS {
            return;
        }
        let hex = format!("#{digits}");
        if hex.eq_ignore_ascii_case(&self.current) {
            return;
        }
        self.typed_hex = Some(hex.clone());
        self.apply_color(&hex, cx);
    }

    fn hex_digits(typed: &str) -> String {
        typed
            .to_uppercase()
            .chars()
            .filter(|character| HEX_DIGIT_SET.contains(*character))
            .take(HEX_RGB_DIGITS)
            .collect()
    }

    fn normalized_digits(hex: &str) -> String {
        folder_hex_string(color(hex)).chars().skip(1).collect()
    }

    fn channel_field_event(&mut self, index: usize, event: &InputUiEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputUiEvent::Changed => {
                if self.channel_fields.iter().any(|field| field.read(cx).marked_range().is_some()) {
                    return;
                }
                let texts: Vec<String> =
                    self.channel_fields.iter().map(|field| field.read(cx).text().to_owned()).collect();
                if texts == self.channel_texts {
                    return;
                }
                self.channel_texts = texts.clone();
                self.channels_edited(texts, window, cx);
                cx.notify();
            }
            InputUiEvent::Submitted => {
                self.focus_next(index, window, cx);
                cx.notify();
            }
            InputUiEvent::Cancelled => self.close_sheet(cx),
        }
    }

    fn focus_next(&self, index: usize, window: &mut Window, cx: &mut App) {
        let next = (index + 1 < self.format.channels().len()).then(|| self.channel_fields.get(index + 1)).flatten();
        match next {
            Some(field) => {
                let focus = field.focus_handle(cx);
                window.focus(&focus, cx);
            }
            None => window.blur(),
        }
    }

    /// typed fields,
    /// digits only and max three,
    /// past the bound is clamped. applies once all fields hold a number
    fn channels_edited(&mut self, texts: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if texts == self.synced_texts {
            return;
        }
        if !self.synced_texts.is_empty() {
            self.synced_texts = Vec::new();
        }
        let channels = self.format.channels();
        if channels.len() != texts.len() {
            return;
        }
        let shaped: Vec<String> =
            texts.iter().zip(&channels).map(|(text, channel)| Self::shaped_channel(text, channel.max)).collect();
        if shaped != texts {
            self.set_channel_texts(shaped.clone(), cx);
            self.channels_edited(shaped, window, cx);
            return;
        }
        let values: Vec<i64> = shaped.iter().filter_map(|text| text.parse::<i64>().ok()).collect();
        let &[first, second, third] = values.as_slice() else {
            return;
        };
        let typed = [first, second, third];
        let rgb = if self.format == ColorInputFormat::Hsl { ColorChannelMath::rgb_from_hsl(typed) } else { typed };
        let hex = format!("#{}", ColorChannelMath::digits_from_rgb(rgb));
        if hex.eq_ignore_ascii_case(&self.current) {
            return;
        }
        self.typed_hex = Some(hex.clone());
        self.apply_color(&hex, cx);
    }

    fn shaped_channel(typed: &str, bound: i64) -> String {
        let digits: String = typed.chars().filter(char::is_ascii_digit).take(CHANNEL_DIGITS).collect();
        match digits.parse::<i64>() {
            Ok(value) => value.min(bound).to_string(),
            Err(_) => digits,
        }
    }

    fn set_channel_texts(&mut self, texts: Vec<String>, cx: &mut App) {
        for (field, text) in self.channel_fields.iter().zip(&texts) {
            if field.read(cx).text() != text {
                field.update(cx, |field, cx| field.set_text(text, cx));
            }
        }
        self.channel_texts = texts;
    }

    /// refills the fields of the current format from a color picked elsewhere
    fn fill_fields(&mut self, hex: &str, cx: &mut App) {
        let digits = Self::normalized_digits(hex);
        if self.hex_draft != digits {
            self.hex_draft = digits.clone();
            self.hex_field.update(cx, |field, cx| field.set_text(&digits, cx));
        }
        if self.format == ColorInputFormat::Hex {
            return;
        }
        let rgb = ColorChannelMath::rgb_from_digits(&digits);
        let values = if self.format == ColorInputFormat::Hsl { ColorChannelMath::hsl_from_rgb(rgb) } else { rgb };
        let texts: Vec<String> = values.iter().map(i64::to_string).collect();
        self.synced_texts = texts.clone();
        self.set_channel_texts(texts, cx);
    }
}

#[derive(IntoElement)]
struct CustomColorPickerSheet {
    current: String,
    apply: ColorApply,
    close: ColorClose,
}

impl CustomColorPickerSheet {
    fn input_bar(
        state: &Entity<CustomColorPickerSheetState>,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let sheet = state.read(cx);
        let preview_hex = if sheet.format == ColorInputFormat::Hex && sheet.hex_draft.len() == HEX_RGB_DIGITS {
            sheet.hex_draft.clone()
        } else {
            sheet.current.clone()
        };
        vstack(0.0)
            .w_full()
            .bg(theme.bg._100)
            .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(12.0))
                    .pb(px(16.0))
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .size(px(GroupedCardMetrics::ICON_SIDE))
                            .circle()
                            .bg(color(&preview_hex))
                            .child(centred_stroke(theme.border._200.opacity(0.7), 0.5, Shape::Circle)),
                    )
                    .child(Self::format_menu(state, sheet.format, theme))
                    .child(Self::fields(state, theme, window, cx)),
            )
    }

    /// one look for every code field,
    /// a pill like the chat composer with a hairline outline that turns red when wrong
    fn code_box(
        id: impl Into<ElementId>,
        wrong: bool,
        field: &Entity<InputUiState>,
        theme: &AppTheme,
        content: impl IntoElement,
    ) -> Stateful<Div> {
        let field = field.clone();
        let outline = if wrong { theme.semantic.error_100 } else { theme.border._200 };
        div()
            .id(id.into())
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h(px(FIELD_HEIGHT))
            .rounded_full()
            .bg(theme.bg._200)
            .child(content)
            .child(centred_stroke(outline, if wrong { 1.5 } else { 0.5 }, Shape::Capsule))
            .on_click(move |_, window, cx| {
                let focus = field.focus_handle(cx);
                window.focus(&focus, cx);
            })
    }

    fn format_menu(
        state: &Entity<CustomColorPickerSheetState>,
        format: ColorInputFormat,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let opener = state.clone();
        let chooser = state.clone();
        let is_presented = Binding::entity(
            state.clone(),
            |state: &CustomColorPickerSheetState| state.is_format_menu_presented,
            |state: &mut CustomColorPickerSheetState, presented, cx| {
                state.is_format_menu_presented = presented;
                cx.notify();
            },
        );
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| {
            ColorInputFormat::ALL_CASES
                .into_iter()
                .map(|option| HoldContextMenuAction::new(option.id(), option.title()).checked(option == format))
                .collect()
        });
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
            if let Some(option) = ColorInputFormat::from_raw_value(&id) {
                chooser.update(cx, |state, cx| state.set_format(option, window, cx));
            }
        });
        let label_theme = theme.clone();
        let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
            Self::format_menu_label(format, &label_theme).into_any_element()
        });
        div().id("color-input-format-hit").flex_none().occlude().child(
            plain("color-input-format")
                .on_press(move |_, cx| {
                    opener.update(cx, |state, cx| {
                        state.is_format_menu_presented = true;
                        cx.notify();
                    });
                })
                .child(Self::format_menu_label(format, theme))
                .tap_context_menu(
                    "color-input-format-menu",
                    is_presented,
                    HoldContextMenuAlignment::Leading,
                    false,
                    false,
                    true,
                    None,
                    actions,
                    on_action,
                    source,
                ),
        )
    }

    fn format_menu_label(format: ColorInputFormat, theme: &AppTheme) -> impl IntoElement + use<> {
        div()
            .flex()
            .items_center()
            .h(px(FIELD_HEIGHT))
            .px(px(14.0))
            .bg(theme.bg._200)
            .clip_shape(Shape::Capsule)
            .child(
                MainText::new(format!("{}{}", format.title(), FORMAT_CHEVRON))
                    .px(13.0)
                    .font_weight(FontWeight::BOLD)
                    .color(theme.text.primary),
            )
    }

    fn fields(
        state: &Entity<CustomColorPickerSheetState>,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let sheet = state.read(cx);
        if sheet.format == ColorInputFormat::Hex {
            return Self::hex_field(sheet, theme, window, cx).into_any_element();
        }
        let focused = sheet.focused_channel(window, cx);
        let channels = sheet.format.channels().into_iter().enumerate().map(|(index, channel)| {
            Self::channel_field(sheet, index, channel, focused, theme).into_any_element()
        });
        hstack(6.0).flex_1().min_w_0().children(channels.collect::<Vec<_>>()).into_any_element()
    }

    /// own color code,
    /// typed or pasted with or without #. full code applies,
    /// incomplete one goes red on blur and isnt applied
    fn hex_field(
        sheet: &CustomColorPickerSheetState,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let draft_complete = sheet.hex_draft.len() == HEX_RGB_DIGITS;
        let shows_error = !sheet.hex_focused(window, cx) && !sheet.hex_draft.is_empty() && !draft_complete;
        hstack(10.0)
            .flex_1()
            .min_w_0()
            .child(MainText::new("#").px(17.0).color(theme.text.secondary).monospaced_digits(true))
            .child(Self::code_box(
                "color-custom-hex-box",
                shows_error,
                &sheet.hex_field,
                theme,
                div().w_full().px(px(16.0)).child(sheet.hex_field.clone()),
            ))
    }

    fn channel_field(
        sheet: &CustomColorPickerSheetState,
        index: usize,
        channel: ColorInputChannel,
        focused: Option<usize>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let empty = sheet.channel_texts.get(index).is_none_or(String::is_empty) && focused != Some(index);
        let field = sheet.channel_fields.get(index).cloned();
        let content = hstack(2.0)
            .w_full()
            .px(px(12.0))
            .child(MainText::new(channel.letter).px(12.0).color(theme.text.secondary))
            .child(div().flex_1().min_w_0().children(field.clone()));
        match field {
            Some(field) => {
                Self::code_box(("color-custom-channel-box", index), empty, &field, theme, content).into_any_element()
            }
            None => content.into_any_element(),
        }
    }
}

impl RenderOnce for CustomColorPickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let seed = self.current.clone();
        let state = window.use_keyed_state("color-custom-picker-sheet-state", cx, move |window, cx| {
            CustomColorPickerSheetState::new(seed, window, cx)
        });
        state.update(cx, |sheet, cx| {
            sheet.current = self.current.clone();
            sheet.apply = Some(self.apply.clone());
            sheet.close = Some(self.close.clone());
            sheet.sync_field_colors(&theme, cx);
            sheet.sync_channel_prompts(cx);
        });
        let changed = state.clone();
        on_change("color-custom-picker-sheet-current", self.current.clone(), false, window, cx, move |_, new_value, _, cx| {
            changed.update(cx, |sheet, cx| sheet.current_did_change(new_value, cx));
        });
        vstack(0.0)
            .w_full()
            .bg(theme.bg._100)
            .child(SystemColorSpectrum { current: self.current.clone(), apply: self.apply.clone(), close: self.close.clone() })
            .child(Self::input_bar(&state, &theme, window, cx))
    }
}

#[derive(IntoElement)]
struct SystemColorSpectrum {
    current: String,
    apply: ColorApply,
    close: ColorClose,
}

impl SystemColorSpectrum {
    const HEIGHT: f32 = 300.0;
    const COLUMNS: usize = 64;
    const ROWS: usize = 40;
    const CLOSE_SIZE: f32 = 32.0;
    const KNOB_SIDE: f32 = 20.0;
}

struct SystemColorSpectrumCoordinator {
    synced_hex: String,
    position: gpui::Point<f32>,
    surface_size: Rc<Cell<(f32, f32)>>,
}

impl SystemColorSpectrumCoordinator {
    fn new(current: &str) -> Self {
        Self {
            synced_hex: current.to_lowercase(),
            position: EditorColorSpectrum::position_of(color(current)),
            surface_size: Rc::new(Cell::new((0.0, 0.0))),
        }
    }

    fn update_ui_view_controller(&mut self, current: &str) -> gpui::Point<f32> {
        let wanted = current.to_lowercase();
        if wanted != self.synced_hex {
            self.synced_hex = wanted;
            self.position = EditorColorSpectrum::position_of(color(current));
        }
        self.position
    }
}

impl RenderOnce for SystemColorSpectrum {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let seed = self.current.clone();
        let coordinator = window.use_keyed_state("color-custom-spectrum-coordinator", cx, move |_, _| {
            SystemColorSpectrumCoordinator::new(&seed)
        });
        let position = coordinator.update(cx, |coordinator, _| coordinator.update_ui_view_controller(&self.current));
        let surface_size = coordinator.read(cx).surface_size.clone();
        let measured = surface_size.clone();
        let selected = color(&self.current);

        let picked = coordinator.clone();
        let owner_current = self.current.clone();
        let apply = self.apply.clone();
        let on_changed = move |gesture: &DragGestureValue, _: &mut Window, cx: &mut App| {
            let (width, height) = surface_size.get();
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            let spot = point(
                (f32::from(gesture.location.x) / width).clamp(0.0, 1.0),
                (f32::from(gesture.location.y) / height).clamp(0.0, 1.0),
            );
            let hex = folder_hex_string(EditorColorSpectrum::color_at(spot));
            picked.update(cx, |coordinator, cx| {
                coordinator.position = spot;
                coordinator.synced_hex = hex.to_lowercase();
                cx.notify();
            });
            if !hex.eq_ignore_ascii_case(&owner_current) {
                apply(&hex, cx);
            }
        };

        let close = self.close.clone();
        let close_button = div().id("color-custom-spectrum-close-hit").flex_none().tooltip_label(tr("close")).child(
            LiquidGlassButton::new("color-custom-spectrum-close", LiquidGlassButtonShape::Circle)
                .size(Self::CLOSE_SIZE)
                .on_press(move |_, cx| close(cx))
                .children(IconUi::sized("xmark", px(14.0), theme.text.primary, false)),
        );

        let knob_offset = px(-Self::KNOB_SIDE / 2.0);
        let surface = div()
            .relative()
            .w_full()
            .h(px(Self::HEIGHT))
            .child(EditorColorSpectrum::surface(Self::COLUMNS, Self::ROWS))
            .child(
                canvas(
                    move |bounds, _, _| measured.set((f32::from(bounds.size.width), f32::from(bounds.size.height))),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(position.x))
                    .top(relative(position.y))
                    .ml(knob_offset)
                    .mt(knob_offset)
                    .size(px(Self::KNOB_SIDE))
                    .circle()
                    .bg(selected)
                    .child(centred_stroke(white(), 3.0, Shape::Circle)),
            )
            .gesture("color-custom-spectrum-drag", DragGesture::new(0.0).on_changed(on_changed));

        vstack(12.0)
            .w_full()
            .p(px(16.0))
            .child(hstack(DEFAULT_STACK_SPACING).w_full().child(spacer()).child(close_button))
            .child(div().w_full().child(surface))
    }
}

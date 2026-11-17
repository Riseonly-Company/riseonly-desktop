use gpui::{App, Context, FocusHandle, FontWeight, KeyDownEvent, Subscription, Window};
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, RiseTextSizePolicy, ThemeStore};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::WallpaperSampleBubbles;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperPreset;

const SIZE_GLYPH: &str = "A";

struct TextSizeSettingsState {
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TextSizeSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self {
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![cx.observe(&theme_store, |_, _, cx| cx.notify())],
        }
    }
}

fn set_text_delta(value: f64, cx: &mut App) {
    ThemeStore::shared(cx).update(cx, |store, cx| store.set_text_delta(value, cx));
}

#[derive(IntoElement)]
pub struct TextSizeSettings;

impl TextSizeSettings {
    pub fn new() -> Self {
        Self
    }

    fn section_header(text: String, theme: &AppTheme) -> impl IntoElement {
        div()
            .px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING))
            .child(MainText::new(text.to_uppercase()).px(12.0).color(theme.text.secondary))
    }

    /// always the shipped wallpaper never the accounts own picture, the picture isnt being judged here
    fn preview(theme: &AppTheme, delta: f64, cx: &mut App) -> impl IntoElement + use<> {
        let appearance = ThemeStore::shared(cx).read(cx).resolved_appearance();
        div()
            .relative()
            .w_full()
            .h(px(200.0))
            .child(
                zstack(Alignment::Bottom)
                    .size_full()
                    .clip_shape(GroupedCardMetrics::SHAPE)
                    .child(
                        ChatDecorativeBackdrop::new()
                            .ignores_safe_area(false)
                            .preview_preset(Some(ChatWallpaperPreset::default_preset().clone()))
                            .animates(false)
                            .appearance_override(Some(appearance)),
                    )
                    .child(
                        div()
                            .id(("text-size-sample-bubbles", delta.to_bits()))
                            .w_full()
                            .py(px(10.0))
                            .child(WallpaperSampleBubbles::new()),
                    ),
            )
            .child(centred_stroke(theme.border._200.opacity(0.6), 0.5, GroupedCardMetrics::SHAPE))
    }

    fn size_glyph(side: f32, theme: &AppTheme) -> impl IntoElement {
        div()
            .text_size(px(side))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text.secondary)
            .child(SIZE_GLYPH)
    }

    fn slider_card(theme: &AppTheme, delta: f64) -> impl IntoElement {
        vstack(10.0)
            .w_full()
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
            .child(
                hstack(None::<f32>)
                    .w_full()
                    .items_baseline()
                    .child(Self::size_glyph(13.0, theme))
                    .child(spacer())
                    .child(
                        MainText::new(RiseTextSizePolicy::label(delta))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .primary(true)
                            .monospaced_digits(true),
                    )
                    .child(spacer())
                    .child(Self::size_glyph(24.0, theme)),
            )
            .child(
                SteppedSliderUi::new(
                    "text-size-slider",
                    delta,
                    RiseTextSizePolicy::RANGE,
                    RiseTextSizePolicy::STEP,
                    |value, _, cx| set_text_delta(value, cx),
                    |value, _, cx| set_text_delta(value, cx),
                )
                .breakpoints(vec![RiseTextSizePolicy::DEFAULT_DELTA]),
            )
    }
}

impl RenderOnce for TextSizeSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("text-size-settings-state", cx, |_, cx| TextSizeSettingsState::new(cx));
        let focus_handle = state.read(cx).focus_handle.clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let delta = f64::from(ThemeStore::shared(cx).read(cx).text_delta());
        let preview = Self::preview(&theme, delta, cx);

        let page = SettingsWrapperUi::new(tr("settings_text_size"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("text-size-settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(
                    vstack(20.0)
                        .w_full()
                        .pt(px(12.0))
                        .child(
                            vstack(8.0)
                                .w_full()
                                .align(HorizontalAlignment::Leading)
                                .child(Self::section_header(tr("settings_preview_header"), &theme))
                                .child(div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(preview)),
                        )
                        .child(
                            GroupedBtns::new(
                                "text-size-slider-group",
                                vec![
                                    GroupedBtn::new("text_size_slider", "")
                                        .content_slot(Self::slider_card(&theme, delta)),
                                ],
                            )
                            .title(tr("settings_text_size").to_uppercase())
                            .end_group_title(tr("settings_text_size_hint")),
                        )
                        .child(GroupedBtns::new(
                            "text-size-reset-group",
                            vec![
                                GroupedBtn::new("text_size_reset", tr("settings_reset_default"))
                                    .icon("arrow.counterclockwise")
                                    .icon_background(SettingsIconPalette::reset())
                                    .disabled(delta == RiseTextSizePolicy::DEFAULT_DELTA)
                                    .show_chevron(false)
                                    .callback(|_, cx| {
                                        selection_changed();
                                        set_text_delta(RiseTextSizePolicy::DEFAULT_DELTA, cx);
                                    }),
                            ],
                        ))
                        .child(div().flex_none().h(px(80.0))),
                ),
        );

        let page_focus = PageFocus::keyed_with("text-size-settings-focus", focus_handle.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        div()
            .size_full()
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if !event.keystroke.modifiers.secondary() {
                    return;
                }
                let direction = match event.keystroke.key.as_str() {
                    "=" | "+" | "add" => 1.0,
                    "-" | "subtract" => -1.0,
                    _ => return,
                };
                cx.stop_propagation();
                let current = f64::from(ThemeStore::shared(cx).read(cx).text_delta());
                set_text_delta(current + direction * RiseTextSizePolicy::STEP, cx);
            })
            .child(page)
            .on_appear("text-size-settings-appear", move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

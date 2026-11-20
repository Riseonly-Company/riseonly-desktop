use std::rc::Rc;

use gpui::{
    AnyElement, App, Background, Context, ElementId, Entity, FontWeight, Render, SharedString, Subscription, Window,
    linear_color_stop, linear_gradient, transparent_black, white,
};
use rise_i18n::tr;
use rise_theme::theme::app_theme::readable_ink;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::{
    BottomSheetMeasuredFraction, BottomSheetMeasuredFractionModifiers,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperPreset;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::theme::components::theme_replace_saved_sheet::theme_replace_saved_sheet::ThemeReplaceSavedSheet;
use crate::modules::theme::stores::theme_presets::theme_presets_actions::theme_presets_actions::ThemePresetsActionsStore;
use crate::modules::theme::stores::theme_presets::theme_presets_services::theme_presets_services::{
    ThemePreset, ThemePresetsServicesStore,
};

const SHEET_ID: &str = "theme-preview-sheet";

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// mounted at the root, shows the theme an addtheme link opened on its own sample only
pub struct ThemePreviewHost {
    services: Entity<ThemePresetsServicesStore>,
    shown: Option<ThemePreset>,
    _services_observation: Subscription,
}

impl ThemePreviewHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let services = ThemePresetsServicesStore::shared(cx);
        let services_observation = cx.observe(&services, |this, services, cx| {
            if let Some(next) = services.read(cx).preview()
                && this.shown.as_ref() != Some(next)
            {
                this.shown = Some(next.clone());
            }
            cx.notify();
        });
        Self { services, shown: None, _services_observation: services_observation }
    }

    fn ask_mode_switch(preset: ThemePreset, cx: &mut App) {
        let mode = preset.mode();
        NativeConfirmAlert::present_confirm(
            &ThemeModeSwitchCopy::title(mode),
            &ThemeModeSwitchCopy::message(mode),
            &tr("chat_mute_apply"),
            move |cx| {
                ThemePresetsActionsStore::shared(cx)
                    .update(cx, |actions, cx| actions.save_preview_action(true, Some(preset), cx));
            },
            cx,
        );
    }
}

impl Render for ThemePreviewHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let services = self.services.read(cx);
        let current = services.preview().cloned().or_else(|| self.shown.clone());
        let sample_theme = services.preview_theme().cloned();
        let is_mutating = services.is_mutating();
        let kept_copy = current.as_ref().and_then(|preview| {
            services
                .presets()
                .iter()
                .find(|kept| kept.id == preview.id || kept.source_theme_id.as_deref() == Some(preview.id.as_str()))
                .map(|kept| kept.id.clone())
        });
        let read = self.services.clone();
        let is_presented = Binding::new(
            move |cx| read.read(cx).preview().is_some(),
            |is_presented, _, cx| {
                if !is_presented {
                    ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.cancel_preview_action(cx));
                }
            },
        );
        let on_save: SheetAction = Rc::new(|_, cx| {
            ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.save_preview_action(false, None, cx));
        });
        let asking = self.services.clone();
        let on_save_and_apply: SheetAction = Rc::new(move |_, cx| {
            let resolved = ThemeStore::shared(cx).read(cx).resolved_appearance();
            match asking.read(cx).preview().cloned() {
                Some(preset) if preset.mode() != resolved => Self::ask_mode_switch(preset, cx),
                _ => {
                    ThemePresetsActionsStore::shared(cx)
                        .update(cx, |actions, cx| actions.save_preview_action(true, None, cx));
                }
            }
        });
        let on_unsave: Option<SheetAction> = kept_copy.map(|kept_id| -> SheetAction {
            Rc::new(move |_, cx| {
                let actions = ThemePresetsActionsStore::shared(cx);
                actions.update(cx, |actions, cx| {
                    actions.cancel_preview_action(cx);
                    actions.delete_action(&kept_id, cx);
                });
            })
        });
        div()
            .size_full()
            .relative()
            .child(
                div().absolute().inset_0().child(ThemePreviewSheet {
                    is_presented,
                    name: current.as_ref().map(|preset| preset.name.clone()).unwrap_or_default(),
                    preset: current,
                    sample_theme,
                    is_mutating,
                    on_save,
                    on_save_and_apply,
                    on_unsave,
                }),
            )
            .child(div().absolute().inset_0().child(ThemeReplaceSavedSheet::new()))
    }
}

/// ask before a theme of the other mode says what will happen,
/// like switch to light theme and that the app goes light
pub enum ThemeModeSwitchCopy {}

impl ThemeModeSwitchCopy {
    pub fn title(mode: RiseThemeAppearance) -> String {
        if mode == RiseThemeAppearance::Light { tr("theme_switch_light_title") } else { tr("theme_switch_dark_title") }
    }

    pub fn message(mode: RiseThemeAppearance) -> String {
        if mode == RiseThemeAppearance::Light {
            tr("theme_switch_light_message")
        } else {
            tr("theme_switch_dark_message")
        }
    }
}

#[derive(IntoElement)]
pub struct ThemePreviewSheet {
    is_presented: Binding<bool>,
    name: String,
    preset: Option<ThemePreset>,
    sample_theme: Option<AppTheme>,
    is_mutating: bool,
    on_save: SheetAction,
    on_save_and_apply: SheetAction,
    on_unsave: Option<SheetAction>,
}

impl ThemePreviewSheet {
    const FRACTION: f32 = 0.82;
    const MINIMUM_FRACTION: f32 = 0.4;

    fn child_id(name: &'static str) -> ElementId {
        (ElementId::from(SharedString::new_static(SHEET_ID)), name).into()
    }

    fn content(&self, theme: &AppTheme, resolved: RiseThemeAppearance) -> Div {
        vstack(14.)
            .w_full()
            .px(px(16.))
            .pt(px(8.))
            .child(
                MainText::new(tr("theme_preview_title"))
                    .px(20.)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(300.))
                    .child(
                        div()
                            .size_full()
                            .continuous_corners(22.)
                            .overflow_hidden()
                            .child(self.sample_chat(theme, resolved)),
                    )
                    .child(centred_stroke(theme.text.secondary.opacity(0.15), 1., Shape::continuous(22.))),
            )
            .when(!self.name.is_empty(), |content| {
                content.child(
                    MainText::new(self.name.clone())
                        .px(17.)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_limit(1_usize),
                )
            })
    }

    fn sample_chat(&self, theme: &AppTheme, resolved: RiseThemeAppearance) -> AnyElement {
        let sample = self.sample_theme.as_ref().unwrap_or(theme);
        let mode = self.preset.as_ref().map(ThemePreset::mode).unwrap_or(resolved);
        div()
            .relative()
            .size_full()
            .child(div().absolute().inset_0().child(self.sample_backdrop(mode)))
            .child(
                vstack(8.)
                    .absolute()
                    .inset_0()
                    .justify_end()
                    .pb(px(10.))
                    .child(Self::sample_bubble(tr("wallpaper_preview_hi"), false, sample))
                    .child(Self::sample_bubble(tr("wallpaper_preview_how_are_you"), true, sample))
                    .child(div().w_full().px(px(10.)).child(Self::composer(sample))),
            )
            .into_any_element()
    }

    fn sample_backdrop(&self, mode: RiseThemeAppearance) -> ChatDecorativeBackdrop {
        let spec = self.preset.as_ref().and_then(|preset| preset.wallpaper.as_ref());
        match spec.and_then(|spec| spec.image_url.clone()).filter(|url| !url.is_empty()) {
            Some(url) => ChatDecorativeBackdrop::new()
                .ignores_safe_area(false)
                .preview_image_url(Some(url))
                .preview_look(spec.map(|spec| spec.look))
                .animates(false)
                .appearance_override(Some(mode)),
            None => {
                let preset_id = spec.map(|spec| spec.preset_id.as_str());
                let preset = ChatWallpaperPreset::all()
                    .iter()
                    .find(|preset| Some(preset.id.as_str()) == preset_id)
                    .unwrap_or(ChatWallpaperPreset::default_preset())
                    .clone();
                ChatDecorativeBackdrop::new()
                    .ignores_safe_area(false)
                    .preview_preset(Some(preset))
                    .animates(false)
                    .appearance_override(Some(mode))
            }
        }
    }

    /// bubble fills of a message, own on the accent gradient and incoming on bg 300
    fn sample_bubble(text: String, is_own: bool, theme: &AppTheme) -> Div {
        let ink = if is_own { readable_ink(theme.primary._100, white()) } else { theme.text.primary };
        let fill: Background = if is_own {
            linear_gradient(
                135.,
                linear_color_stop(theme.primary._100.opacity(0.96), 0.),
                linear_color_stop(theme.primary._200.opacity(0.92), 1.),
            )
        } else {
            theme.bg._300.opacity(0.96).into()
        };
        hstack(0.)
            .w_full()
            .px(px(10.))
            .when(is_own, |row| row.justify_end())
            .child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .continuous_corners(MessageSizes::bubble_corner_radius())
                    .bg(fill)
                    .child(MainText::new(text).px(15.).color(ink)),
            )
    }

    fn composer(theme: &AppTheme) -> Div {
        let mic_ink = readable_ink(theme.primary._100, white());
        hstack(8.)
            .w_full()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .size(px(38.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .circle()
                    .bg(theme.bg._000)
                    .children(IconUi::sized("paperclip", px(18.), theme.text.primary, false)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(38.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .capsule()
                    .bg(theme.bg._000)
                    .child(MainText::new(tr("funny_chat_placeholder")).px(15.).color(theme.text.secondary).line_limit(1_usize)),
            )
            .child(
                div()
                    .flex_none()
                    .size(px(38.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .circle()
                    .bg(theme.primary._100)
                    .children(IconUi::sized("mic.fill", px(16.), mic_ink, true)),
            )
    }

    fn footer(&self, theme: &AppTheme) -> Div {
        let ink = readable_ink(theme.primary._100, white());
        let is_mutating = self.is_mutating;
        let on_save_and_apply = self.on_save_and_apply.clone();
        let on_save = self.on_save.clone();
        if let Some(on_unsave) = self.on_unsave.clone() {
            return vstack(8.).w_full().px(px(16.)).pt(px(12.)).pb(px(4.)).child(
                ButtonUi::new(Self::child_id("unsave"))
                    .background_color(theme.bg._200)
                    .height(52.)
                    .disabled(is_mutating)
                    .on_press(move |window, cx| on_unsave(window, cx))
                    .child(if is_mutating {
                        progress_view().tint(theme.text.primary).into_any_element()
                    } else {
                        MainText::new(tr("theme_unsave"))
                            .px(16.)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.semantic.error_100)
                            .into_any_element()
                    }),
            );
        }
        vstack(8.)
            .w_full()
            .px(px(16.))
            .pt(px(12.))
            .pb(px(4.))
            .child(
                ButtonUi::new(Self::child_id("save-and-apply"))
                    .background_color(theme.primary._100)
                    .height(52.)
                    .disabled(is_mutating)
                    .on_press(move |window, cx| on_save_and_apply(window, cx))
                    .child(if is_mutating {
                        progress_view().tint(ink).into_any_element()
                    } else {
                        MainText::new(tr("theme_save_and_apply"))
                            .px(16.)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(ink)
                            .into_any_element()
                    }),
            )
            .child(
                plain(Self::child_id("save"))
                    .w_full()
                    .disabled(is_mutating)
                    .on_press(move |window, cx| on_save(window, cx))
                    .child(
                        div().w_full().min_h(px(44.)).flex().items_center().justify_center().child(
                            MainText::new(tr("common_save"))
                                .px(15.)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.primary._100),
                        ),
                    ),
            )
    }
}

impl RenderOnce for ThemePreviewSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content_height = window.use_keyed_state(Self::child_id("content-height"), cx, |_, _| 0.0_f32);
        let footer_height = window.use_keyed_state(Self::child_id("footer-height"), cx, |_, _| 0.0_f32);
        let sheet_fraction = BottomSheetMeasuredFraction::attached(
            *content_height.read(cx),
            *footer_height.read(cx),
            Self::MINIMUM_FRACTION,
            window,
        )
        .unwrap_or(Self::FRACTION);
        let sheet_background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let is_presented = self.is_presented.clone();
        let sheet = Rc::new(self);
        let content_sheet = sheet.clone();
        let footer_sheet = sheet;
        BottomSheetUi::new(
            SHEET_ID,
            is_presented,
            move |_, cx| {
                let theme_store = ThemeStore::shared(cx);
                let store = theme_store.read(cx);
                let theme = store.current_theme().clone();
                let resolved = store.resolved_appearance();
                let content_height = content_height.clone();
                div().size_full().child(content_sheet.content(&theme, resolved).on_sheet_part_height(
                    Self::child_id("content-part"),
                    move |height, _, cx| store_height(&content_height, height, cx),
                ))
            },
            move |_, cx| {
                let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
                let footer_height = footer_height.clone();
                footer_sheet.footer(&theme).on_sheet_part_height(
                    Self::child_id("footer-part"),
                    move |height, _, cx| store_height(&footer_height, height, cx),
                )
            },
        )
        .snap_fractions(vec![sheet_fraction])
        .full_page(false)
        .dismisses_on_backdrop_tap(true)
        .full_page_shows_drag_indicator(true)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(false))
        .footer_horizontal_padding(Some(0.))
        .footer_top_padding(Some(0.))
        .content_scrollable(false)
        .sheet_background(Some(sheet_background))
        .footer_background(Some(transparent_black()))
    }
}

fn store_height(height: &Entity<f32>, value: f32, cx: &mut App) {
    height.update(cx, |height, cx| {
        if (*height - value).abs() > f32::EPSILON {
            *height = value;
            cx.notify();
        }
    });
}

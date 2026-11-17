use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, Hsla, SharedString, Subscription, Window};
use rise_i18n::tr;
use rise_platform::system_share::ShareItem;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;
use rise_ui::text_field_alert::TextFieldAlert;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::theme::components::theme_preview_sheet::theme_preview_sheet::ThemeModeSwitchCopy;
use crate::modules::theme::stores::theme_presets::theme_presets_actions::theme_presets_actions::ThemePresetsActionsStore;
use crate::modules::theme::stores::theme_presets::theme_presets_services::theme_presets_services::{
    ThemePreset, ThemePresetsLoadState, ThemePresetsServicesStore,
};

const PAGE_ID: &str = "themes-list-settings";

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    (id.clone(), name.into()).into()
}

/// page state plus the observable reads of its body
struct ThemesListState {
    new_name: String,
    is_naming: bool,
    renaming: Option<ThemePreset>,
    rename_text: String,
    menu_preset_id: Option<String>,
    replaces_oldest: bool,
    _subscriptions: [Subscription; 2],
}

impl ThemesListState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ThemePresetsServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            new_name: String::new(),
            is_naming: false,
            renaming: None,
            rename_text: String::new(),
            menu_preset_id: None,
            replaces_oldest: false,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

/// saved themes of this account, save the current look then apply or share or rename or delete
#[derive(IntoElement)]
pub struct ThemesListSettings {
    id: ElementId,
}

impl Default for ThemesListSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemesListSettings {
    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static(PAGE_ID)) }
    }

    fn naming_alert(&self, state: &Entity<ThemesListState>) -> TextFieldAlert {
        let reading = state.clone();
        let writing = state.clone();
        let is_presented = Binding::new(
            move |cx| reading.read(cx).is_naming,
            move |value, _, cx| {
                writing.update(cx, |state, cx| {
                    state.is_naming = value;
                    cx.notify();
                })
            },
        );
        let text = Self::text_binding(state, |state| state.new_name.clone(), |state, value| state.new_name = value);
        let saving = state.clone();
        TextFieldAlert::new(
            named_child(&self.id, "naming"),
            is_presented,
            tr("theme_save_current"),
            tr("theme_name_placeholder"),
            text,
            tr("cancel"),
            tr("common_save"),
            move |_, cx| {
                let (name, replace_oldest) = {
                    let state = saving.read(cx);
                    (state.new_name.clone(), state.replaces_oldest)
                };
                ThemePresetsActionsStore::shared(cx)
                    .update(cx, |actions, cx| actions.save_current_action(&name, replace_oldest, cx));
            },
        )
    }

    fn rename_alert(&self, state: &Entity<ThemesListState>) -> TextFieldAlert {
        let reading = state.clone();
        let writing = state.clone();
        let is_presented = Binding::new(
            move |cx| reading.read(cx).renaming.is_some(),
            move |value, _, cx| {
                if !value {
                    writing.update(cx, |state, cx| {
                        state.renaming = None;
                        cx.notify();
                    });
                }
            },
        );
        let text = Self::text_binding(state, |state| state.rename_text.clone(), |state, value| state.rename_text = value);
        let renaming = state.clone();
        TextFieldAlert::new(
            named_child(&self.id, "rename"),
            is_presented,
            tr("theme_rename"),
            tr("theme_name_placeholder"),
            text,
            tr("cancel"),
            tr("common_save"),
            move |_, cx| {
                let (preset, name) = renaming.update(cx, |state, cx| {
                    cx.notify();
                    (state.renaming.take(), state.rename_text.clone())
                });
                if let Some(preset) = preset {
                    ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.rename_action(&preset.id, &name, cx));
                }
            },
        )
    }

    fn text_binding(
        state: &Entity<ThemesListState>,
        get: fn(&ThemesListState) -> String,
        set: fn(&mut ThemesListState, String),
    ) -> Binding<String> {
        let reading = state.clone();
        let writing = state.clone();
        Binding::new(
            move |cx| get(reading.read(cx)),
            move |value, _, cx| {
                writing.update(cx, |state, cx| {
                    set(state, value);
                    cx.notify();
                })
            },
        )
    }
}

impl RenderOnce for ThemesListSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| ThemesListState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let naming_alert = self.naming_alert(&state);
        let rename_alert = self.rename_alert(&state);
        div()
            .size_full()
            .relative()
            .child(
                SettingsWrapperUi::new(tr("settings_my_themes"), move |_, cx| {
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(ThemesListBody { id: named_child(&self.id, "body"), state }),
            )
            .child(naming_alert)
            .child(rename_alert)
            .on_appear(named_child(&self.id, "appear"), |_, cx| {
                ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.load_action(cx));
            })
    }
}

#[derive(IntoElement)]
struct ThemesListBody {
    id: ElementId,
    state: Entity<ThemesListState>,
}

impl ThemesListBody {
    fn save_current_group(&self, is_mutating: bool) -> GroupedBtns {
        let state = self.state.clone();
        GroupedBtns::new(
            named_child(&self.id, "save"),
            vec![
                GroupedBtn::new("theme_save_current", tr("theme_save_current"))
                    .icon("plus")
                    .icon_background(color("#00A9BB"))
                    .disabled(is_mutating)
                    .show_chevron(false)
                    .callback(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.new_name.clear();
                            cx.notify();
                        });
                        if ThemePresetsActionsStore::own_at_limit(cx) {
                            Self::confirm_replace_oldest(state.clone(), cx);
                        } else {
                            state.update(cx, |state, cx| {
                                state.replaces_oldest = false;
                                state.is_naming = true;
                                cx.notify();
                            });
                        }
                    }),
            ],
        )
    }

    fn confirm_replace_oldest(state: Entity<ThemesListState>, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("theme_create_replace_oldest_message"),
            "",
            Some(tr("theme_create_replace_oldest_confirm")),
            None,
            false,
            None,
            None,
            move |cx| {
                state.update(cx, |state, cx| {
                    state.replaces_oldest = true;
                    state.is_naming = true;
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn content(
        &self,
        services: &ThemePresetsServicesStore,
        theme: &AppTheme,
        resolved: RiseThemeAppearance,
        cx: &App,
    ) -> Option<AnyElement> {
        let presets = services.presets();
        let applied_id = services.applied_id(cx);
        match services.load_state() {
            ThemePresetsLoadState::Idle => Some(Self::progress()),
            ThemePresetsLoadState::Loading if presets.is_empty() => Some(Self::progress()),
            ThemePresetsLoadState::Failed(message) if presets.is_empty() => Some(
                div()
                    .w_full()
                    .py(px(24.))
                    .child(
                        MainText::new(message.clone())
                            .px(13.)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    )
                    .into_any_element(),
            ),
            _ => {
                let mine: Vec<GroupedBtn> = presets
                    .iter()
                    .filter(|preset| !preset.is_saved())
                    .map(|preset| self.row(preset, applied_id.as_deref(), resolved, theme))
                    .collect();
                let saved: Vec<GroupedBtn> = presets
                    .iter()
                    .filter(|preset| preset.is_saved())
                    .map(|preset| self.row(preset, applied_id.as_deref(), resolved, theme))
                    .collect();
                Some(
                    vstack(20.)
                        .w_full()
                        .when(!mine.is_empty(), |groups| {
                            groups.child(
                                GroupedBtns::new(named_child(&self.id, "mine"), mine)
                                    .title(tr("theme_section_mine").to_uppercase()),
                            )
                        })
                        .when(!saved.is_empty(), |groups| {
                            groups.child(
                                GroupedBtns::new(named_child(&self.id, "saved"), saved)
                                    .title(tr("theme_section_saved").to_uppercase()),
                            )
                        })
                        .into_any_element(),
                )
            }
        }
    }

    fn progress() -> AnyElement {
        div().w_full().py(px(24.)).flex().justify_center().child(progress_view()).into_any_element()
    }

    fn row(
        &self,
        preset: &ThemePreset,
        applied_id: Option<&str>,
        resolved: RiseThemeAppearance,
        theme: &AppTheme,
    ) -> GroupedBtn {
        let applied = applied_id == Some(preset.id.as_str()) && preset.mode() == resolved;
        let picked = preset.clone();
        GroupedBtn::new(format!("theme_{}", preset.id), preset.name.clone())
            .leading_slot(ThemeSwatches { hexes: preset.swatches(resolved), stroke: theme.text.primary })
            .text_line_limit(1)
            .accessibility_selected(applied)
            .show_chevron(false)
            .trailing_slot(self.trailing(preset, applied, theme))
            .callback(move |_, cx| Self::pick(&picked, cx))
    }

    fn pick(preset: &ThemePreset, cx: &mut App) {
        let mode = preset.mode();
        if mode != ThemeStore::shared(cx).read(cx).resolved_appearance() {
            let preset = preset.clone();
            NativeConfirmAlert::present_confirm(
                &ThemeModeSwitchCopy::title(mode),
                &ThemeModeSwitchCopy::message(mode),
                &tr("chat_mute_apply"),
                move |cx| {
                    ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.apply_in_its_mode_action(&preset, cx));
                },
                cx,
            );
            return;
        }
        ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.apply_action(preset, cx));
    }

    fn trailing(&self, preset: &ThemePreset, applied: bool, theme: &AppTheme) -> Div {
        let preset_id = preset.id.clone();
        let reading = self.state.clone();
        let writing = self.state.clone();
        let menu_id = preset_id.clone();
        let is_presented = Binding::new(
            move |cx| reading.read(cx).menu_preset_id.as_deref() == Some(menu_id.as_str()),
            {
                let menu_id = preset_id.clone();
                move |is_presented: bool, _, cx| {
                    writing.update(cx, |state, cx| {
                        if !is_presented && state.menu_preset_id.as_deref() == Some(menu_id.as_str()) {
                            state.menu_preset_id = None;
                            cx.notify();
                        }
                    })
                }
            },
        );
        let actions: HoldContextMenuPresentationActions = Rc::new(|_: &App| {
            vec![
                HoldContextMenuAction::new("share", tr("theme_share")).system_image("square.and.arrow.up"),
                HoldContextMenuAction::new("rename", tr("theme_rename")).system_image("pencil"),
                HoldContextMenuAction::new("delete", tr("theme_delete")).system_image("trash").destructive(true),
            ]
        });
        let acting_state = self.state.clone();
        let acting = preset.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            match id.as_str() {
                "share" => {
                    SystemShare::present(
                        vec![ShareItem::Link(ThemePresetsActionsStore::share_url(&acting.id))],
                        None,
                        cx,
                    );
                }
                "rename" => acting_state.update(cx, |state, cx| {
                    state.rename_text = acting.name.clone();
                    state.renaming = Some(acting.clone());
                    cx.notify();
                }),
                "delete" => Self::confirm_delete(acting.id.clone(), cx),
                _ => {}
            }
        });
        let glyph_color = theme.text.secondary;
        let source: HoldContextMenuTapSource =
            Rc::new(move |_: &mut Window, _: &mut App| preset_menu_glyph(glyph_color).into_any_element());
        let opener = self.state.clone();
        let opened_id = preset_id.clone();
        hstack(12.)
            .items_center()
            .when(applied, |row| {
                row.children(IconUi::sized("checkmark", px(15.), theme.primary._100, false))
            })
            .child(
                plain(named_child(&self.id, format!("menu-{preset_id}")))
                    .on_press(move |_, cx| {
                        let opened_id = opened_id.clone();
                        opener.update(cx, |state, cx| {
                            state.menu_preset_id = Some(opened_id);
                            cx.notify();
                        });
                    })
                    .child(preset_menu_glyph(glyph_color))
                    .tap_context_menu(
                        named_child(&self.id, format!("menu-presentation-{preset_id}")),
                        is_presented,
                        HoldContextMenuAlignment::Trailing,
                        false,
                        false,
                        false,
                        None,
                        actions,
                        on_action,
                        source,
                    ),
            )
    }

    fn confirm_delete(id: String, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("theme_delete_confirm"),
            "",
            Some(tr("theme_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.delete_action(&id, cx));
            },
            cx,
        );
    }
}

impl RenderOnce for ThemesListBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let resolved = ThemeStore::shared(cx).read(cx).resolved_appearance();
        let services = ThemePresetsServicesStore::shared(cx);
        let services = services.read(cx);
        let save_current = self.save_current_group(services.is_mutating());
        let content = self.content(services, &theme, resolved, cx);
        scroll_view(named_child(&self.id, "scroll"))
            .size_full()
            .bg(theme.bg._100)
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(20.)
                    .w_full()
                    .pt(px(12.))
                    .child(save_current)
                    .children(content)
                    .child(spacer_min_h(80.))
                    .music_mini_player_bottom_padding(0., 0.),
            )
    }
}

fn preset_menu_glyph(color: Hsla) -> Div {
    div()
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .children(IconUi::sized("ellipsis.circle", px(18.), color, false))
}

#[derive(IntoElement)]
struct ThemeSwatches {
    hexes: Vec<String>,
    stroke: Hsla,
}

impl RenderOnce for ThemeSwatches {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let stroke = self.stroke;
        hstack(0.).w(px(44.)).h(px(28.)).items_center().justify_center().children(
            self.hexes.into_iter().enumerate().map(move |(index, hex)| {
                div()
                    .relative()
                    .flex_none()
                    .size(px(20.))
                    .circle()
                    .bg(color(&hex))
                    .when(index > 0, |dot| dot.ml(px(-8.)))
                    .child(centred_stroke(stroke.opacity(0.15), 1., Shape::Circle))
            }),
        )
    }
}

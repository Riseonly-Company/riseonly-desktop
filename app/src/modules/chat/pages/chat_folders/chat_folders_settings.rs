use std::collections::HashMap;

use gpui::{App, Entity, FontWeight, SharedString, Window};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::core::ui::formatted_text_ui::FormattedTextStyle;
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_text::InlineCustomEmojiText;
use crate::modules::chat::components::folders::chat_folder_editor_store::ChatFolderEditorStore;
use crate::modules::chat::pages::chats::chats_reorder_toggle_button::ChatsReorderToggleButton;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const ROW_HEIGHT: f32 = 60.0;
const REORDER_SLOT_HEIGHT: f32 = 66.5;
const FIXED_ROWS_HEIGHT: f32 = 112.5;
const MINUS_SLOT_WIDTH: f32 = 34.0;
const HANDLE_WIDTH: f32 = 32.0;
const HANDLE_HEIGHT: f32 = 44.0;
const CHEVRON_WIDTH: f32 = 8.0;
const CHEVRON_ROW_HEIGHT: f32 = 20.0;
const CARD_CORNER_RADIUS: f32 = 14.0;

/// screen state kept as long as the route is drawn
struct ChatFoldersSettingsState {
    is_edit_mode: bool,
    edit_progress: Animated<f32>,
    dragging_id: Option<String>,
    drag_offset_y: f32,
    accumulated_shift: f32,
    working_order: Vec<String>,
    settling: HashMap<String, Animated<f32>>,
}

impl ChatFoldersSettingsState {
    fn new() -> Self {
        Self {
            is_edit_mode: false,
            edit_progress: Animated::new(0.0),
            dragging_id: None,
            drag_offset_y: 0.0,
            accumulated_shift: 0.0,
            working_order: Vec::new(),
            settling: HashMap::new(),
        }
    }

    fn set_edit_mode(&mut self, is_edit_mode: bool, cx: &mut Context<Self>) {
        self.is_edit_mode = is_edit_mode;
        let target = if is_edit_mode { 1.0 } else { 0.0 };
        self.edit_progress.set_in(target, spring(0.32, 0.85), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn displayed_folders(&self, base: Vec<ChatFolder>) -> Vec<ChatFolder> {
        if self.dragging_id.is_none() || self.working_order.is_empty() {
            return base;
        }
        let mut by_id = HashMap::with_capacity(base.len());
        for folder in base {
            by_id.entry(folder.id.clone()).or_insert(folder);
        }
        self.working_order.iter().filter_map(|id| by_id.get(id).cloned()).collect()
    }

    fn settle(&mut self, id: &str, shift: f32, response: f64, damping_fraction: f64, cx: &App) {
        let now = RiseClock::monotonic();
        let settling = self.settling.entry(id.to_owned()).or_insert_with(|| Animated::new(0.0));
        let current = settling.value(now);
        settling.jump(current + shift);
        settling.set_in(0.0, spring(response, damping_fraction), now, cx);
    }

    fn settling_offset(&self, id: &str, window: &Window) -> f32 {
        self.settling.get(id).map_or(0.0, |settling| settling.frame_value(RiseClock::monotonic(), window))
    }

    fn update_drag(&mut self, folder_id: &str, translation_y: f32, in_order: Vec<String>, cx: &mut Context<Self>) {
        if self.dragging_id.as_deref() != Some(folder_id) {
            self.dragging_id = Some(folder_id.to_owned());
            self.working_order = in_order;
            self.accumulated_shift = 0.0;
            let now = RiseClock::monotonic();
            self.settling.retain(|_, settling| settling.is_running(now));
        }
        self.drag_offset_y = translation_y - self.accumulated_shift;
        let Some(current_index) = self.working_order.iter().position(|id| id == folder_id) else {
            cx.notify();
            return;
        };
        if self.drag_offset_y > ROW_HEIGHT * 0.6 && current_index + 1 < self.working_order.len() {
            let neighbour = self.working_order[current_index + 1].clone();
            self.working_order.swap(current_index, current_index + 1);
            self.settle(folder_id, -REORDER_SLOT_HEIGHT, 0.28, 0.86, cx);
            self.settle(&neighbour, REORDER_SLOT_HEIGHT, 0.28, 0.86, cx);
            self.accumulated_shift += ROW_HEIGHT;
            self.drag_offset_y = translation_y - self.accumulated_shift;
        } else if self.drag_offset_y < -ROW_HEIGHT * 0.6 && current_index > 0 {
            let neighbour = self.working_order[current_index - 1].clone();
            self.working_order.swap(current_index, current_index - 1);
            self.settle(folder_id, REORDER_SLOT_HEIGHT, 0.28, 0.86, cx);
            self.settle(&neighbour, -REORDER_SLOT_HEIGHT, 0.28, 0.86, cx);
            self.accumulated_shift -= ROW_HEIGHT;
            self.drag_offset_y = translation_y - self.accumulated_shift;
        }
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) -> Vec<String> {
        if let Some(dragging_id) = self.dragging_id.take() {
            let drag_offset_y = self.drag_offset_y;
            self.settle(&dragging_id, drag_offset_y, 0.3, 0.86, cx);
        }
        self.drag_offset_y = 0.0;
        self.accumulated_shift = 0.0;
        cx.notify();
        std::mem::take(&mut self.working_order)
    }
}

#[derive(IntoElement)]
pub struct ChatFoldersSettings;

impl ChatFoldersSettings {
    pub fn new() -> Self {
        Self
    }

    fn folders_in_order(cx: &mut App) -> Vec<ChatFolder> {
        let service = ChatServicesStore::shared(cx);
        service.read(cx).folders_in_order(cx)
    }

    fn header(theme: &AppTheme) -> impl IntoElement {
        vstack(14.0)
            .w_full()
            .items_center()
            .pb(px(4.0))
            .child(
                div()
                    .size(px(92.0))
                    .circle()
                    .bg(theme.primary._100.opacity(0.14))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("folder.fill.badge.gearshape", px(38.0), theme.primary._100, false)),
            )
            .child(
                div().w_full().px(px(32.0)).child(
                    MainText::new(tr("folder_settings_description"))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(3),
                ),
            )
    }

    fn create_row(theme: &AppTheme) -> impl IntoElement {
        plain("chat-folders-create")
            .w_full()
            .on_press(|_, cx| {
                let _ = selection_changed();
                ChatFolderEditorStore::shared(cx).update(cx, |editor, cx| editor.present_create(cx));
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .items_center()
                    .px(px(14.0))
                    .py(px(11.0))
                    .child(
                        div()
                            .size(px(34.0))
                            .circle()
                            .flex_none()
                            .bg(theme.primary._100.opacity(0.16))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("plus", px(15.0), theme.primary._100, false)),
                    )
                    .child(MainText::new(tr("folder_create")).px(16.0).color(theme.primary._100))
                    .child(spacer()),
            )
            .guide_anchor(RiseGuideAnchor::FolderSettingsCreate)
    }

    fn all_chats_row(theme: &AppTheme) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(11.0))
            .child(
                div()
                    .size(px(34.0))
                    .circle()
                    .flex_none()
                    .bg(theme.text.secondary.opacity(0.18))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("bubble.left.and.bubble.right.fill", px(14.0), theme.text.secondary, true)),
            )
            .child(MainText::new(tr("all_chats")).px(16.0).color(theme.text.primary))
            .child(spacer())
    }

    fn divider(theme: &AppTheme) -> impl IntoElement {
        div().w_full().flex_none().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100))
    }

    fn drag_gesture(state: &Entity<ChatFoldersSettingsState>, folder_id: &str) -> DragGesture {
        let (changed_state, ended_state, changed_id) = (state.clone(), state.clone(), folder_id.to_owned());
        DragGesture::new(2.0)
            .on_changed(move |value, _, cx| {
                let is_tracking = changed_state.read(cx).dragging_id.as_deref() == Some(changed_id.as_str());
                let in_order: Vec<String> = if is_tracking {
                    Vec::new()
                } else {
                    Self::folders_in_order(cx).into_iter().map(|folder| folder.id).collect()
                };
                let translation_y = f32::from(value.translation.height);
                changed_state.update(cx, |state, cx| state.update_drag(&changed_id, translation_y, in_order, cx));
            })
            .on_ended(move |_, _, cx| {
                let order = ended_state.update(cx, |state, cx| state.end_drag(cx));
                if order.is_empty() {
                    return;
                }
                let mut by_id = HashMap::new();
                for folder in Self::folders_in_order(cx) {
                    by_id.entry(folder.id.clone()).or_insert(folder);
                }
                let ordered: Vec<ChatFolder> = order.iter().filter_map(|id| by_id.get(id).cloned()).collect();
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.reorder_folders(ordered, cx));
            })
    }

    fn folder_row(
        folder: &ChatFolder,
        progress: f32,
        is_edit_mode: bool,
        state: &Entity<ChatFoldersSettingsState>,
        theme: &AppTheme,
        corners: (bool, bool),
    ) -> impl IntoElement {
        let folder_id = folder.id.clone();
        let (round_top, round_bottom) = corners;
        let trailing_width = CHEVRON_WIDTH + (HANDLE_WIDTH - CHEVRON_WIDTH) * progress;
        let tap_folder = folder.clone();
        let title_emojis = folder.title_custom_emojis();
        let tint = folder.swift_color().unwrap_or(theme.primary._100);
        let badge = div()
            .size(px(34.0))
            .circle()
            .flex_none()
            .mr(px(12.0))
            .bg(tint.opacity(0.16))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("folder.fill", px(15.0), tint, true));

        let minus = (progress > 0.0).then(|| {
            let minus_id = folder_id.clone();
            div()
                .flex_none()
                .overflow_hidden()
                .w(px(MINUS_SLOT_WIDTH * progress))
                .opacity(progress)
                .occlude()
                .child(
                    div().relative().left(px(-(1.0 - progress) * MINUS_SLOT_WIDTH)).child(
                        plain(SharedString::from(format!("chat-folder-delete-{folder_id}")))
                            .disabled(!is_edit_mode)
                            .on_press(move |_, cx| {
                                let _ = selection_changed();
                                ChatInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.confirm_delete_folder(&minus_id, cx));
                            })
                            .children(IconUi::sized("minus.circle.fill", px(22.0), theme.semantic.error_100, true)),
                    ),
                )
        });

        let handle = (progress > 0.0).then(|| {
            let glyph = div()
                .w(px(HANDLE_WIDTH))
                .h(px(HANDLE_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("line.3.horizontal", px(18.0), theme.text.secondary, false));
            let glyph = if is_edit_mode {
                glyph
                    .gesture(
                        SharedString::from(format!("chat-folder-handle-{folder_id}")),
                        Self::drag_gesture(state, &folder_id),
                    )
                    .into_any_element()
            } else {
                glyph.into_any_element()
            };
            div().absolute().right_0().top_0().opacity(progress).child(glyph)
        });

        let chevron = (progress < 1.0).then(|| {
            div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .opacity(1.0 - progress)
                .flex()
                .items_center()
                .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(13.0), theme.text.secondary.opacity(0.6), false))
        });

        hstack(0.0)
            .id(SharedString::from(format!("chat-folder-row-{folder_id}")))
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(11.0))
            .bg(theme.bg._200)
            .when(round_top, |row| row.rounded_t(px(CARD_CORNER_RADIUS)))
            .when(round_bottom, |row| row.rounded_b(px(CARD_CORNER_RADIUS)))
            .when(!is_edit_mode, |row| row.cursor_pointer())
            .on_click(move |_, _, cx| {
                if is_edit_mode {
                    return;
                }
                let _ = selection_changed();
                ChatFolderEditorStore::shared(cx).update(cx, |editor, cx| editor.present_edit(&tap_folder, cx));
            })
            .children(minus)
            .child(badge)
            .child(
                div()
                    .min_w_0()
                    .mr(px(12.0))
                    .child(if title_emojis.is_empty() {
                        MainText::new(folder.title.clone())
                            .px(16.0)
                            .color(theme.text.primary)
                            .number_of_lines(1)
                            .into_any_element()
                    } else {
                        div()
                            .line_clamp(1)
                            .text_ellipsis()
                            .child(
                                InlineCustomEmojiText::new(
                                    SharedString::from(format!("folder-settings-title-{}", folder.id)),
                                    folder.title.clone(),
                                    FormattedTextStyle::app_standard(theme, 16.0),
                                    title_emojis,
                                )
                                .opens_pack_on_tap(false),
                            )
                            .into_any_element()
                    }),
            )
            .child(spacer_min_w(8.0))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .ml(px(12.0))
                    .w(px(trailing_width))
                    .h(px(CHEVRON_ROW_HEIGHT + (HANDLE_HEIGHT - CHEVRON_ROW_HEIGHT) * progress))
                    .children(handle)
                    .children(chevron),
            )
    }

    fn folders_section(
        folders: &[ChatFolder],
        progress: f32,
        is_edit_mode: bool,
        state: &Entity<ChatFoldersSettingsState>,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let screen = state.read(cx);
        let dragging_id = screen.dragging_id.clone();

        let fixed_rows = (progress < 1.0).then(|| {
            let divider_height = if folders.is_empty() { 0.0 } else { 0.5 };
            let height = (FIXED_ROWS_HEIGHT + divider_height) * (1.0 - progress);
            vstack(0.0)
                .w_full()
                .overflow_hidden()
                .opacity(1.0 - progress)
                .when(progress > 0.0, |rows| rows.h(px(height)))
                .child(Self::create_row(theme))
                .child(Self::divider(theme))
                .child(Self::all_chats_row(theme))
                .when(!folders.is_empty(), |rows| rows.child(Self::divider(theme)))
        });

        let mut card = vstack(0.0)
            .w_full()
            .relative()
            .bg(theme.bg._200)
            .rounded(px(CARD_CORNER_RADIUS))
            .overflow_hidden()
            .children(fixed_rows);
        let mut dragged = None;
        for (index, folder) in folders.iter().enumerate() {
            let corners = (index == 0 && progress >= 1.0, index + 1 == folders.len());
            if dragging_id.as_deref() == Some(folder.id.as_str()) {
                card = card.child(div().w_full().flex_none().h(px(REORDER_SLOT_HEIGHT - 0.5)));
                let top = index as f32 * REORDER_SLOT_HEIGHT
                    + screen.drag_offset_y
                    + screen.settling_offset(&folder.id, window);
                dragged = Some(
                    div().absolute().left_0().right_0().top(px(top)).child(Self::folder_row(
                        folder,
                        progress,
                        is_edit_mode,
                        state,
                        theme,
                        corners,
                    )),
                );
            } else {
                let settling = screen.settling_offset(&folder.id, window);
                card = card.child(div().w_full().relative().top(px(settling)).child(Self::folder_row(
                    folder,
                    progress,
                    is_edit_mode,
                    state,
                    theme,
                    corners,
                )));
            }
            if index + 1 < folders.len() {
                card = card.child(Self::divider(theme));
            }
        }

        vstack(8.0)
            .w_full()
            .child(
                div().w_full().pl(px(30.0)).child(
                    MainText::new(tr("folder_section_header").to_uppercase())
                        .px(12.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary),
                ),
            )
            .child(div().w_full().px(px(16.0)).child(card.children(dragged)))
            .into_any_element()
    }
}

impl Default for ChatFoldersSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for ChatFoldersSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("chat-folders-settings-state", cx, |_, _| ChatFoldersSettingsState::new());

        use_task("chat-folders-settings-task", (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_folders(cx));
            });
        });

        let base = Self::folders_in_order(cx);
        let folders = state.read(cx).displayed_folders(base);
        let (is_edit_mode, is_dragging) = {
            let screen = state.read(cx);
            (screen.is_edit_mode, screen.dragging_id.is_some())
        };
        let progress = state.read(cx).edit_progress.frame_value(RiseClock::monotonic(), window).clamp(0.0, 1.0);

        let empty_state = state.clone();
        on_change("chat-folders-settings-empty", folders.is_empty(), false, window, cx, move |_, is_empty, _, cx| {
            if !*is_empty {
                return;
            }
            empty_state.update(cx, |state, cx| {
                if state.is_edit_mode {
                    state.set_edit_mode(false, cx);
                }
            });
        });

        let section = Self::folders_section(&folders, progress, is_edit_mode, &state, &theme, window, cx);
        let toggle_state = state.clone();
        SettingsWrapperUi::new(tr("settings_chat_folders_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .when(!folders.is_empty(), move |wrapper| {
            wrapper.right_jsx(ChatsReorderToggleButton::new("chat-folders-reorder-toggle", is_edit_mode, move |_, cx| {
                let _ = selection_changed();
                toggle_state.update(cx, |state, cx| {
                    let is_edit_mode = !state.is_edit_mode;
                    state.set_edit_mode(is_edit_mode, cx);
                });
            }))
        })
        .content(
            scroll_view("chat-folders-settings-scroll")
                .list_shell_scroll_under_chrome(cx)
                .scroll_disabled(is_dragging)
                .size_full()
                .bg(theme.bg._100)
                .child(
                    vstack(22.0)
                        .w_full()
                        .pt(px(14.0))
                        .child(Self::header(&theme))
                        .child(section)
                        .child(div().w_full().flex_none().h(px(60.0))),
                ),
        )
    }
}

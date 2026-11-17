use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, MouseButton, MouseDownEvent, Render, SharedString,
    Subscription, Window,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, color};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::layer_keys::LayerKeysExt;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::rich_text::rich_text_markup::RichEditorStyle;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::chat::composer::chat_composer_text_view::ChatComposerTextView;
use crate::modules::chat::components::chat::input_picker::chat_input_emoji_tab::ChatInputEmojiTab;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::components::folders::chat_folder_add_chats_sheet::ChatFolderAddChatsContent;
use crate::modules::chat::components::folders::chat_folder_editor_store::{AddMode, ChatFolderEditorStore};
use crate::modules::chat::components::folders::chat_folder_sheet_presentation_contract::ChatFolderSheetPresentationContract;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::{
    ChatFolderPalette, FolderChatType, FolderChatTypeExt,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_well::ColorPickerWell;

const PANEL_HEIGHT: f32 = 336.0;
const SAVE_REFOCUS_DELAY: Duration = Duration::from_millis(350);
const ESCAPE_KEY: &str = "escape";
const NAME_INPUT_MIN_HEIGHT: f32 = 42.0;
const NAME_INPUT_MAX_HEIGHT: f32 = 92.0;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

#[derive(Clone, PartialEq)]
struct SelectedScope {
    epoch: Option<String>,
    visible: bool,
    ids: HashSet<String>,
}

struct ChatFolderEditSheetState {
    observed_presented: bool,
    emoji_panel_visible: bool,
    name_input_height: f32,
    name_input_width: f32,
    name_selection: Option<NSRange>,
    name_focus_token: i64,
    name_requested_selection: Option<NSRange>,
    name_selection_token: i64,
    emoji_toggle_at: Date,
    emoji_panel: Entity<Presence<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatFolderEditSheetState {
    fn close_emoji_panel(&mut self, cx: &mut Context<Self>) {
        if !self.emoji_panel_visible {
            return;
        }
        self.emoji_panel_visible = false;
        cx.notify();
    }

    fn new(store: &Entity<ChatFolderEditorStore>, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let editor = store.clone();
        let subscriptions = vec![
            cx.observe(store, |state, store, cx| {
                let presented = store.read(cx).is_presented();
                if !presented {
                    state.emoji_panel_visible = false;
                }
                if presented || state.observed_presented {
                    state.observed_presented = presented;
                    cx.notify();
                }
            }),
            cx.observe(&services, move |_, _, cx| {
                if editor.read(cx).is_presented() {
                    cx.notify();
                }
            }),
        ];
        Self {
            observed_presented: store.read(cx).is_presented(),
            emoji_panel_visible: false,
            name_input_height: NAME_INPUT_MIN_HEIGHT,
            name_input_width: 0.0,
            name_selection: None,
            name_focus_token: 0,
            name_requested_selection: None,
            name_selection_token: 0,
            emoji_toggle_at: Date::from_ms(0),
            emoji_panel: cx
                .new(|_| Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom))),
            _subscriptions: subscriptions,
        }
    }
}

#[derive(IntoElement)]
pub struct ChatFolderEditSheet {
    id: ElementId,
}

impl ChatFolderEditSheet {
    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static("chat-folder-edit-sheet")) }
    }
}

impl Default for ChatFolderEditSheet {
    fn default() -> Self {
        Self::new()
    }
}

/// the sheet as its own cached root layer
/// its keyed state and store reads dirty this layer and not the root view and every page under it
pub struct ChatFolderEditSheetLayer;

impl Render for ChatFolderEditSheetLayer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(ChatFolderEditSheet::new())
    }
}

impl RenderOnce for ChatFolderEditSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = ChatFolderEditorStore::shared(cx);
        let observed = store.clone();
        let state = window.use_keyed_state(
            named_child(&self.id, "state"),
            cx,
            move |_, cx: &mut Context<ChatFolderEditSheetState>| ChatFolderEditSheetState::new(&observed, cx),
        );
        let parts = ChatFolderEditSheetParts { id: self.id.clone(), store: store.clone(), state: state.clone() };

        let add_mode = store.read(cx).add_mode;
        let add_mode_state = state.clone();
        on_change(
            named_child(&self.id, "add-mode"),
            add_mode,
            false,
            window,
            cx,
            move |_, mode: &Option<AddMode>, window, cx| {
                if mode.is_some() {
                    add_mode_state.update(cx, |state, cx| {
                        state.emoji_panel_visible = false;
                        cx.notify();
                    });
                    ScreenFocus::release_text_field(window, cx);
                }
            },
        );

        let title = store.read(cx).title.clone();
        let title_store = store.clone();
        on_change(named_child(&self.id, "title"), title, false, window, cx, move |_, title: &String, window, cx| {
            if !title.contains(['\n', '\r']) {
                return;
            }
            let single_line: String = title.chars().filter(|character| !matches!(character, '\n' | '\r')).collect();
            title_store.update(cx, |store, cx| store.set_title(single_line, cx));
            ScreenFocus::release_text_field(window, cx);
        });

        let (visible_now, ids) = {
            let store = store.read(cx);
            let ids: HashSet<String> =
                store.included_chat_ids.iter().chain(store.excluded_chat_ids.iter()).cloned().collect();
            (store.is_presented(), ids)
        };
        let scope = SelectedScope { epoch: SaiAccountCacheScope::active_epoch(), visible: visible_now, ids };
        let resolving = store.clone();
        use_task(named_child(&self.id, "selected-scope"), scope, window, cx, async move |cx| {
            let task = resolving.update(cx, |store, cx| store.resolve_selected_chat_models(cx));
            task.await;
        });

        let reader = store.clone();
        let writer = store.clone();
        let visible = Binding::new(
            move |cx: &App| reader.read(cx).is_presented(),
            move |value: bool, _: &mut Window, cx: &mut App| {
                if !value {
                    writer.update(cx, |store, cx| store.close(cx));
                }
            },
        );

        let is_dirty = store.read(cx).is_dirty();
        let backdrop_parts = parts.clone();
        let escape_parts = parts.clone();
        let sheet = BottomSheetUi::new(
            named_child(&self.id, "sheet"),
            visible,
            move |window, cx| parts.routed_content(window, cx),
            |_, _| div(),
        )
        .snap_fractions(ChatFolderSheetPresentationContract::SNAP_FRACTIONS.to_vec())
        .full_page(ChatFolderSheetPresentationContract::USES_CUSTOM_FULL_PAGE)
        .full_page_allows_swipe_dismiss(!is_dirty)
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .full_page_shows_drag_indicator(false)
        .full_page_hides_footer(true)
        .full_page_avoids_keyboard_when_footer_hidden(true)
        .footer_shows_top_border(false)
        .content_scrollable(ChatFolderSheetPresentationContract::CONTENT_SCROLLABLE)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .backdrop_tap_interceptor(move |_, window, cx| backdrop_parts.intercepts_dirty_close(window, cx));

        div().absolute().inset_0().child(sheet).on_key_down(move |event, window, cx| {
            if event.keystroke.key == ESCAPE_KEY && escape_parts.intercepts_dirty_close(window, cx) {
                cx.stop_propagation();
            }
        })
    }
}

#[derive(Clone)]
struct ChatFolderEditSheetParts {
    id: ElementId,
    store: Entity<ChatFolderEditorStore>,
    state: Entity<ChatFolderEditSheetState>,
}

impl ChatFolderEditSheetParts {
    fn routed_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if self.store.read(cx).add_mode.is_some() {
            ChatFolderAddChatsContent::new(named_child(&self.id, "add-chats")).into_any_element()
        } else {
            self.content(window, cx)
        }
    }

    fn present_discard_alert(&self, cx: &mut App) {
        let discard = self.store.clone();
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("folder_unsaved_title"),
            "",
            Some(tr("folder_unsaved_save")),
            Some(tr("folder_unsaved_discard")),
            true,
            Some(Box::new(move |cx: &mut App| {
                discard.update(cx, |store, cx| store.close(cx));
            })),
            None,
            move |cx| parts.save_from_alert(cx),
            cx,
        );
    }

    fn close_on_done(&self) -> Rc<dyn Fn(&mut App)> {
        let store = self.store.clone();
        Rc::new(move |cx: &mut App| store.update(cx, |store, cx| store.close(cx)))
    }

    fn save_from_alert(&self, cx: &mut App) {
        let (can_save, title_is_empty) = {
            let store = self.store.read(cx);
            (store.can_save(), store.trimmed_title().is_empty())
        };
        if can_save {
            let on_done = self.close_on_done();
            self.store.update(cx, |store, cx| store.save(on_done, cx));
        } else if title_is_empty {
            let store = self.store.clone();
            let state = self.state.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(SAVE_REFOCUS_DELAY).await;
                let can_focus = store.update(cx, |store, _| store.is_presented() && store.add_mode.is_none());
                if can_focus {
                    state.update(cx, |state, cx| {
                        state.name_focus_token = state.name_focus_token.wrapping_add(1);
                        cx.notify();
                    });
                }
            })
            .detach();
        } else {
            self.store.update(cx, |store, cx| store.open_add_chats(AddMode::Included, cx));
        }
    }

    /// escape and a backdrop click on a dirty draft ask the discard question, like the header button
    fn intercepts_dirty_close(&self, window: &Window, cx: &mut App) -> bool {
        let should_ask = {
            let store = self.store.read(cx);
            store.is_presented() && store.add_mode.is_none() && store.is_dirty()
        };
        if !should_ask || ScreenFocus::text_field_is_focused(window, cx) {
            return false;
        }
        self.request_close(cx);
        true
    }

    fn request_close(&self, cx: &mut App) {
        if self.store.read(cx).is_dirty() {
            self.present_discard_alert(cx);
        } else {
            self.store.update(cx, |store, cx| store.close(cx));
        }
    }

    fn content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let (is_editing, can_save) = {
            let store = self.store.read(cx);
            (store.is_editing(), store.can_save())
        };
        let title = if is_editing { tr("folder_edit_title") } else { tr("folder_new_title") };
        let left = self.clone();
        let right = self.clone();
        let header = SheetHeaderBar::new(named_child(&self.id, "header"), title, move |_, cx| left.request_close(cx))
            .right_enabled(can_save)
            .on_right(move |_, cx| {
                let on_done = right.close_on_done();
                right.store.update(cx, |store, cx| store.save(on_done, cx));
            })
            .right_guide_anchor(Some(RiseGuideAnchor::FolderEditorSave));

        let body = scroll_view(named_child(&self.id, "scroll")).shows_indicators(false).child(
            vstack(22.0)
                .w_full()
                .items_start()
                .px(px(16.0))
                .pt(px(14.0))
                .child(self.name_section(&theme, cx))
                .child(self.included_section(&theme, window, cx))
                .child(self.excluded_section(&theme, window, cx))
                .child(self.color_section(&theme, cx))
                .child(div().w_full().h(px(40.0))),
        );
        let swipe_state = self.state.clone();
        let tap_state = self.state.clone();
        let submit = self.clone();
        let body = div()
            .size_full()
            .form_primary(can_save, move |_, cx| {
                let on_done = submit.close_on_done();
                submit.store.update(cx, |store, cx| store.save(on_done, cx));
            })
            .on_scroll_wheel(move |_, _, cx| {
                swipe_state.update(cx, |state, cx| {
                    if RiseClock::now().time_interval_since(&state.emoji_toggle_at) > 0.6 {
                        state.close_emoji_panel(cx);
                    }
                });
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                let state = tap_state.clone();
                cx.defer(move |cx| {
                    state.update(cx, |state, cx| {
                        if RiseClock::now().time_interval_since(&state.emoji_toggle_at) > 0.35 {
                            state.close_emoji_panel(cx);
                        }
                    });
                });
            })
            .child(body);

        let now = RiseClock::monotonic();
        let (visible, presence) = {
            let state = self.state.read(cx);
            (state.emoji_panel_visible, state.emoji_panel.clone())
        };
        let spec = if visible { ease_out(0.22) } else { ease_out(0.2) };
        presence.update(cx, |presence, cx| presence.set_in(visible.then_some(()), spec, now, cx));
        let panel = presence.read(cx).value(now).is_some().then(|| self.emoji_panel());
        let panel = presence.read(cx).render_transitioned(now, window, move |_| {
            div().w_full().flex_none().h(px(PANEL_HEIGHT)).bg(theme.bg._100).children(panel)
        });

        vstack(0.0)
            .size_full()
            .bg(rise_ui::theme(cx).bg._100)
            .child(header)
            .child(div().w_full().flex_1().min_h_0().child(body))
            .children(panel)
            .into_any_element()
    }

    fn emoji_panel(&self) -> AnyElement {
        let (on_emoji, emoji_state) = (self.store.clone(), self.state.clone());
        let (on_custom, custom_state) = (self.store.clone(), self.state.clone());
        let (on_backspace, backspace_state) = (self.store.clone(), self.state.clone());
        ChatInputEmojiTab::new(
            named_child(&self.id, "emoji-tab"),
            move |emoji, _, cx| {
                let selection = emoji_state.read(cx).name_selection;
                let caret = on_emoji.update(cx, |store, cx| store.insert_emoji(&emoji, selection, cx));
                Self::apply_name_caret(&emoji_state, caret, cx);
            },
            move |sticker, _, cx| {
                let selection = custom_state.read(cx).name_selection;
                let caret = on_custom.update(cx, |store, cx| store.append_custom_emoji(sticker, selection, cx));
                Self::apply_name_caret(&custom_state, caret, cx);
            },
            move |_, cx| {
                let selection = backspace_state.read(cx).name_selection;
                let caret = on_backspace.update(cx, |store, cx| store.delete_backward(selection, cx));
                Self::apply_name_caret(&backspace_state, caret, cx);
            },
        )
        .safe_area_bottom(0.0)
        .uses_flat_chrome(true)
        .into_any_element()
    }

    /// puts the caret back where the emoji panel edit left it
    fn apply_name_caret(state: &Entity<ChatFolderEditSheetState>, caret: Option<NSRange>, cx: &mut App) {
        let Some(caret) = caret else {
            return;
        };
        state.update(cx, |state, cx| {
            state.name_selection = Some(caret);
            state.name_requested_selection = Some(caret);
            state.name_selection_token = state.name_selection_token.wrapping_add(1);
            cx.notify();
        });
    }

    fn name_section(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let (visible, input_height, input_width, focus_token, requested_selection, selection_token) = {
            let state = self.state.read(cx);
            (
                state.emoji_panel_visible,
                state.name_input_height,
                state.name_input_width,
                state.name_focus_token,
                state.name_requested_selection,
                state.name_selection_token,
            )
        };
        let (title_is_empty, custom_emoji_runs) = {
            let store = self.store.read(cx);
            (store.title.is_empty(), store.composer_custom_emoji_runs(&store.title))
        };
        let font_size = MessageSizes::composer_text_font_size();
        let title = Binding::entity(
            self.store.clone(),
            |store: &ChatFolderEditorStore| store.title.clone(),
            |store, title, cx| store.set_title(title, cx),
        );
        let selection_state = self.state.clone();
        let runs_store = self.store.clone();
        let field = ChatComposerTextView::new(
            named_child(&self.id, "name-field"),
            ChatFolderEditorStore::title_rule().binding(title),
            input_width,
            NAME_INPUT_MIN_HEIGHT,
            NAME_INPUT_MAX_HEIGHT,
            theme.text.primary,
            RichEditorStyle::make(
                font_size,
                theme.text.primary,
                theme.text.secondary.opacity(0.7),
                theme.text.primary,
                theme.text.primary.opacity(0.12),
            ),
            Binding::entity(
                self.state.clone(),
                |state: &ChatFolderEditSheetState| state.name_input_height,
                |state, height, cx| {
                    state.name_input_height = height;
                    cx.notify();
                },
            ),
        )
        .custom_emoji_runs(custom_emoji_runs)
        .focus_token(focus_token)
        .requested_selection(requested_selection)
        .selection_token(selection_token)
        .on_selection_change(move |range, cx| {
            selection_state.update(cx, |state, _| state.name_selection = Some(range));
        })
        .on_custom_emoji_runs_change(move |runs, cx| {
            let store = runs_store.clone();
            cx.defer(move |cx| store.update(cx, |store, cx| store.sync_custom_emoji_drafts(runs, cx)));
        })
        .holds_custom_emoji_line_height(true)
        .on_send(|window, cx| ScreenFocus::release_text_field(window, cx))
        .on_cancel({
            let parts = self.clone();
            move |_, cx| parts.request_close(cx)
        });

        let press_state = self.state.clone();
        let field = div()
            .id(named_child(&self.id, "name-field-press"))
            .capture_any_mouse_down(move |_: &MouseDownEvent, _, cx| {
                press_state.update(cx, |state, cx| state.close_emoji_panel(cx));
            })
            .child(field);

        let placeholder_top = 10.0 + ChatComposerTextView::held_line_extra(font_size, cx);
        let placeholder = title_is_empty.then(|| {
            pass_through()
                .absolute()
                .top(px(placeholder_top))
                .left(px(12.0))
                .text_size(px(font_size))
                .text_color(theme.text.secondary.opacity(0.82))
                .line_clamp(1)
                .child(SharedString::from(tr("folder_name_placeholder")))
        });

        let width_state = self.state.clone();
        let input = div()
            .relative()
            .w_full()
            .h(px(input_height))
            .child(measure(named_child(&self.id, "name-geometry"), move |bounds, _, cx| {
                width_state.update(cx, |state, cx| {
                    state.name_input_width = f32::from(bounds.size.width);
                    cx.notify();
                });
            }))
            .children(placeholder)
            .child(div().absolute().top_0().left_0().w(px(input_width)).h(px(input_height)).child(field));

        let toggle = self.clone();
        let emoji_button = plain(named_child(&self.id, "emoji-toggle"))
            .w(px(44.0))
            .h(px(42.0))
            .flex()
            .items_center()
            .justify_center()
            .on_press(move |window, cx| toggle.toggle_emoji_panel(window, cx))
            .children(IconUi::sized(
                if visible { "keyboard" } else { "face.smiling" },
                px(19.0),
                theme.text.secondary,
                false,
            ));

        vstack(8.0)
            .w_full()
            .items_start()
            .child(
                div().pl(px(14.0)).child(
                    MainText::new(tr("folder_name_section").to_uppercase())
                        .px(12.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary),
                ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .pl(px(6.0))
                    .pr(px(2.0))
                    .pt(px(4.0))
                    .pb(px(4.0))
                    .bg(theme.bg._200)
                    .capsule()
                    .child(input)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(2.0))
                            .flex()
                            .items_center()
                            .occlude()
                            .child(emoji_button),
                    )
                    .guide_anchor(RiseGuideAnchor::FolderEditorName),
            )
            .into_any_element()
    }

    fn toggle_emoji_panel(&self, window: &mut Window, cx: &mut App) {
        let _ = selection_changed();
        let visible = self.state.read(cx).emoji_panel_visible;
        if !visible {
            ScreenFocus::release_text_field(window, cx);
        }
        self.state.update(cx, |state, cx| {
            state.emoji_toggle_at = RiseClock::now();
            state.emoji_panel_visible = !visible;
            if visible {
                state.name_focus_token = state.name_focus_token.wrapping_add(1);
            }
            cx.notify();
        });
    }

    fn included_section(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let (types, ids) = {
            let store = self.store.read(cx);
            (store.included_chat_types.clone(), store.included_chat_ids.clone())
        };
        let current = folder_presence_rows(types, resolved_chats(&ids, &self.store, cx));
        let now = RiseClock::monotonic();
        let list = window.use_keyed_state(named_child(&self.id, "included-presence"), cx, |_, _| {
            FolderPresenceList::default()
        });
        list.update(cx, |list, cx| list.sync(current, spring(0.3, 0.85), now, cx));
        let slots_id = named_child(&self.id, "included-slots");
        let mut rows = folder_presence_slots(
            &slots_id,
            &list,
            FolderPresenceAxis::Vertical,
            0.0,
            now,
            window,
            cx,
            |value| {
                let store = self.store.clone();
                let row = match value {
                    FolderPresenceValue::Type(raw) => {
                        let removed = raw.clone();
                        self.type_row("included", raw, theme, move |cx| {
                            store.update(cx, |store, cx| {
                                let mut types = store.included_chat_types.clone();
                                types.retain(|value| *value != removed);
                                store.set_included_chat_types(types, cx);
                            });
                        })
                    }
                    FolderPresenceValue::Chat(chat) => {
                        let removed = chat.id.clone();
                        self.chat_row("included", chat, theme, move |cx| {
                            store.update(cx, |store, cx| {
                                let mut ids = store.included_chat_ids.clone();
                                ids.retain(|value| *value != removed);
                                store.set_included_chat_ids(ids, cx);
                            });
                        })
                    }
                };
                vstack(0.0).w_full().child(row).child(Self::divider(theme)).into_any_element()
            },
        );
        let add = self.store.clone();
        rows.push(
            self.action_row("included-add", "plus", tr("folder_add_chats"), theme, move |cx| {
                add.update(cx, |store, cx| store.open_add_chats(AddMode::Included, cx));
            })
            .guide_anchor(RiseGuideAnchor::FolderEditorAddChats)
            .into_any_element(),
        );
        FolderSection::new(tr("folder_included_section"), vstack(0.0).w_full().children(rows).into_any_element())
            .into_any_element()
    }

    fn excluded_section(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let (types, ids) = {
            let store = self.store.read(cx);
            (store.excluded_chat_types.clone(), store.excluded_chat_ids.clone())
        };
        let current = folder_presence_rows(types, resolved_chats(&ids, &self.store, cx));
        let now = RiseClock::monotonic();
        let list = window.use_keyed_state(named_child(&self.id, "excluded-presence"), cx, |_, _| {
            FolderPresenceList::default()
        });
        list.update(cx, |list, cx| list.sync(current, spring(0.3, 0.85), now, cx));
        let slots_id = named_child(&self.id, "excluded-slots");
        let mut rows = folder_presence_slots(
            &slots_id,
            &list,
            FolderPresenceAxis::Vertical,
            0.0,
            now,
            window,
            cx,
            |value| {
                let store = self.store.clone();
                let row = match value {
                    FolderPresenceValue::Type(raw) => {
                        let removed = raw.clone();
                        self.type_row("excluded", raw, theme, move |cx| {
                            store.update(cx, |store, cx| {
                                let mut types = store.excluded_chat_types.clone();
                                types.retain(|value| *value != removed);
                                store.set_excluded_chat_types(types, cx);
                            });
                        })
                    }
                    FolderPresenceValue::Chat(chat) => {
                        let removed = chat.id.clone();
                        self.chat_row("excluded", chat, theme, move |cx| {
                            store.update(cx, |store, cx| {
                                let mut ids = store.excluded_chat_ids.clone();
                                ids.retain(|value| *value != removed);
                                store.set_excluded_chat_ids(ids, cx);
                            });
                        })
                    }
                };
                vstack(0.0).w_full().child(row).child(Self::divider(theme)).into_any_element()
            },
        );
        let add = self.store.clone();
        rows.push(
            self.action_row("excluded-add", "plus", tr("folder_add_exceptions"), theme, move |cx| {
                add.update(cx, |store, cx| store.open_add_chats(AddMode::Excluded, cx));
            })
            .into_any_element(),
        );
        FolderSection::new(tr("folder_excluded_section"), vstack(0.0).w_full().children(rows).into_any_element())
            .into_any_element()
    }

    fn color_section(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let current = self.store.read(cx).color.clone();
        let dots: Vec<AnyElement> =
            ChatFolderPalette::PRESETS.iter().copied().map(|hex| self.color_dot(hex, current.as_deref(), theme)).collect();
        let strip = div()
            .id(named_child(&self.id, "color-strip"))
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.0))
            .px(px(16.0))
            .py(px(16.0))
            .children(dots)
            .child(self.custom_color_dot())
            .child(self.no_color_dot(current.as_deref(), theme))
            .into_any_element();
        FolderSection::new(tr("folder_color_section"), strip).into_any_element()
    }

    fn color_dot(&self, hex: &'static str, current: Option<&str>, theme: &AppTheme) -> AnyElement {
        let is_selected = current.is_some_and(|value| value.to_lowercase() == hex.to_lowercase());
        let store = self.store.clone();
        plain(named_child(&self.id, SharedString::from(format!("color-{hex}"))))
            .relative()
            .flex_none()
            .w(px(34.0))
            .h(px(34.0))
            .circle()
            .bg(color(hex))
            .on_press(move |_, cx| {
                let _ = selection_changed();
                store.update(cx, |store, cx| store.set_color(Some(hex.to_owned()), cx));
            })
            .when(is_selected, |dot| {
                dot.child(centred_stroke(theme.bg._100, 3.0, Shape::Circle)).child(selection_ring(color(hex)))
            })
            .into_any_element()
    }

    fn custom_color_dot(&self) -> AnyElement {
        let reader = self.store.clone();
        let writer = self.store.clone();
        let selection = Binding::new(
            move |cx: &App| {
                let current = reader.read(cx).color.clone().filter(|value| !value.is_empty());
                match current {
                    Some(value) => color(&value),
                    None => rise_ui::theme(cx).primary._100,
                }
            },
            move |picked: Hsla, _: &mut Window, cx: &mut App| {
                writer.update(cx, |store, cx| store.set_color(Some(folder_hex_string(picked)), cx));
            },
        );
        div()
            .flex_none()
            .w(px(34.0))
            .h(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .child(ColorPickerWell::new(named_child(&self.id, "custom-color"), selection).side(34.0))
            .into_any_element()
    }

    fn no_color_dot(&self, current: Option<&str>, theme: &AppTheme) -> AnyElement {
        let is_selected = current.is_none_or(str::is_empty);
        let store = self.store.clone();
        plain(named_child(&self.id, "no-color"))
            .relative()
            .flex_none()
            .w(px(34.0))
            .h(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .on_press(move |_, cx| {
                let _ = selection_changed();
                store.update(cx, |store, cx| store.set_color(None, cx));
            })
            .child(centred_stroke(theme.border._200, 1.5, Shape::Circle))
            .children(IconUi::sized("xmark", px(14.0), theme.text.secondary, false))
            .when(is_selected, |dot| dot.child(selection_ring(theme.primary._100)))
            .into_any_element()
    }

    fn type_row(&self, scope: &str, raw: &str, theme: &AppTheme, on_remove: impl Fn(&mut App) + 'static) -> AnyElement {
        let folder_type = FolderChatType::from_wire(raw);
        let system_image = folder_type.map_or("person.fill", |value| value.system_image());
        let fill = folder_type.map_or(theme.primary._100, |value| value.color());
        let title = tr(folder_type.map_or(raw, |value| value.title_key()));
        hstack(12.0)
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(9.0))
            .child(
                div()
                    .flex_none()
                    .w(px(36.0))
                    .h(px(36.0))
                    .circle()
                    .bg(fill)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(system_image, px(16.0), theme.content_on(fill), system_image.ends_with(".fill"))),
            )
            .child(div().flex_1().min_w_0().child(MainText::new(title).px(16.0).color(theme.text.primary)))
            .child(self.remove_button(SharedString::from(format!("{scope}-type-remove-{raw}")), theme, on_remove))
            .into_any_element()
    }

    fn chat_row(
        &self,
        scope: &str,
        chat: &ChatInfo,
        theme: &AppTheme,
        on_remove: impl Fn(&mut App) + 'static,
    ) -> AnyElement {
        hstack(12.0)
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(9.0))
            .child(
                ChatLogo::new(
                    named_child(&self.id, SharedString::from(format!("{scope}-logo-{}", chat.id))),
                    chat.clone(),
                )
                .size(36.0)
                .icon_size(16.0)
                .show_online_indicator(false),
            )
            .child(
                div().flex_1().min_w_0().child(
                    MainText::new(folder_chat_display_name(chat))
                        .px(16.0)
                        .color(theme.text.primary)
                        .number_of_lines(1),
                ),
            )
            .child(self.remove_button(SharedString::from(format!("{scope}-chat-remove-{}", chat.id)), theme, on_remove))
            .into_any_element()
    }

    fn remove_button(
        &self,
        name: SharedString,
        theme: &AppTheme,
        on_remove: impl Fn(&mut App) + 'static,
    ) -> impl IntoElement {
        plain(named_child(&self.id, name))
            .flex_none()
            .on_press(move |_, cx| {
                let _ = selection_changed();
                on_remove(cx);
            })
            .children(IconUi::sized("minus.circle.fill", px(20.0), theme.semantic.error_100, true))
    }

    fn action_row(
        &self,
        name: &'static str,
        icon: &'static str,
        title: String,
        theme: &AppTheme,
        action: impl Fn(&mut App) + 'static,
    ) -> PlainButton {
        plain(named_child(&self.id, name))
            .w_full()
            .on_press(move |_, cx| {
                let _ = selection_changed();
                action(cx);
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .items_center()
                    .px(px(14.0))
                    .py(px(11.0))
                    .child(
                        div()
                            .flex_none()
                            .w(px(36.0))
                            .h(px(36.0))
                            .circle()
                            .bg(theme.primary._100.opacity(0.16))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(icon, px(16.0), theme.primary._100, false)),
                    )
                    .child(div().flex_1().min_w_0().child(MainText::new(title).px(16.0).color(theme.primary._100))),
            )
    }

    fn divider(theme: &AppTheme) -> AnyElement {
        div().w_full().pl(px(62.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100)).into_any_element()
    }
}

/// a row or chip of a selected list
/// the value outlives its removal so its kept here and not read back from the store
#[derive(Clone)]
pub(crate) enum FolderPresenceValue {
    Type(String),
    Chat(ChatInfo),
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FolderPresenceAxis {
    Vertical,
    Horizontal,
}

struct FolderPresenceEntry {
    key: SharedString,
    value: FolderPresenceValue,
    presence: Animated<f32>,
    natural_extent: Rc<Cell<f32>>,
    is_leaving: bool,
}

/// animated list change over a plain for each
/// a removed entry stays until its slot has collapsed
/// so neighbours travel on the same spec instead of jumping
#[derive(Default)]
pub(crate) struct FolderPresenceList {
    entries: Vec<FolderPresenceEntry>,
    did_sync: bool,
}

impl FolderPresenceList {
    /// runs in render before the slots are read so no frame shows the new list at rest
    pub(crate) fn sync(
        &mut self,
        current: Vec<(SharedString, FolderPresenceValue)>,
        spec: MotionSpec,
        now: Duration,
        cx: &App,
    ) {
        let is_first = !std::mem::replace(&mut self.did_sync, true);
        let current_keys: HashSet<SharedString> = current.iter().map(|(key, _)| key.clone()).collect();
        let positions: HashMap<SharedString, usize> =
            self.entries.iter().enumerate().map(|(index, entry)| (entry.key.clone(), index)).collect();
        let mut previous: Vec<Option<FolderPresenceEntry>> =
            std::mem::take(&mut self.entries).into_iter().map(Some).collect();
        let mut merged: Vec<FolderPresenceEntry> = Vec::with_capacity(previous.len() + current.len());
        let mut cursor = 0;

        for (key, value) in current {
            let Some(position) = positions.get(&key).copied() else {
                let mut presence = Animated::new(if is_first { 1.0_f32 } else { 0.0_f32 });
                if !is_first {
                    presence.set_in(1.0, spec, now, cx);
                }
                merged.push(FolderPresenceEntry {
                    key,
                    value,
                    presence,
                    natural_extent: Rc::new(Cell::new(0.0)),
                    is_leaving: false,
                });
                continue;
            };
            Self::keep_leaving(&mut previous, cursor, position, &current_keys, &mut merged, spec, now, cx);
            cursor = cursor.max(position);
            if let Some(mut entry) = previous[position].take() {
                entry.value = value;
                if entry.is_leaving {
                    entry.is_leaving = false;
                    entry.presence.set_in(1.0, spec, now, cx);
                }
                merged.push(entry);
            }
        }
        let end = previous.len();
        Self::keep_leaving(&mut previous, cursor, end, &current_keys, &mut merged, spec, now, cx);
        self.entries = merged;
    }

    /// entries gone from the list hold their place among neighbours until they have collapsed
    #[allow(clippy::too_many_arguments)]
    fn keep_leaving(
        previous: &mut [Option<FolderPresenceEntry>],
        from: usize,
        to: usize,
        current_keys: &HashSet<SharedString>,
        merged: &mut Vec<FolderPresenceEntry>,
        spec: MotionSpec,
        now: Duration,
        cx: &App,
    ) {
        for slot in previous.iter_mut().take(to).skip(from) {
            if slot.as_ref().is_none_or(|entry| current_keys.contains(&entry.key)) {
                continue;
            }
            let Some(mut entry) = slot.take() else { continue };
            if !entry.is_leaving {
                entry.is_leaving = true;
                entry.presence.set_in(0.0, spec, now, cx);
            }
            if entry.presence.is_running(now) {
                merged.push(entry);
            }
        }
    }
}

pub(crate) fn folder_presence_rows(
    types: Vec<String>,
    chats: Vec<ChatInfo>,
) -> Vec<(SharedString, FolderPresenceValue)> {
    let type_rows = types
        .into_iter()
        .map(|raw| (SharedString::from(format!("type-{raw}")), FolderPresenceValue::Type(raw)));
    let chat_rows = chats
        .into_iter()
        .map(|chat| (SharedString::from(format!("chat-{}", chat.id)), FolderPresenceValue::Chat(chat)));
    type_rows.chain(chat_rows).collect()
}

/// draws each entry in a slot that scales along axis
/// content keeps its natural size inside the clip so the measured extent stays true while collapsing
#[allow(clippy::too_many_arguments)]
pub(crate) fn folder_presence_slots(
    id: &ElementId,
    list: &Entity<FolderPresenceList>,
    axis: FolderPresenceAxis,
    spacing: f32,
    now: Duration,
    window: &mut Window,
    cx: &App,
    mut build: impl FnMut(&FolderPresenceValue) -> AnyElement,
) -> Vec<AnyElement> {
    let is_vertical = axis == FolderPresenceAxis::Vertical;
    let entries = &list.read(cx).entries;
    let mut slots = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let is_running = entry.presence.is_running(now);
        if is_running {
            window.request_animation_frame();
        }
        let progress = entry.presence.value(now).clamp(0.0, 1.0);
        let natural_extent = entry.natural_extent.clone();
        let content = div()
            .relative()
            .flex_none()
            .when(is_vertical, |content| content.w_full())
            .child(build(&entry.value))
            .child(measure(
                named_child(id, SharedString::from(format!("extent-{}", entry.key))),
                move |bounds, _, _| {
                    let size = if is_vertical { bounds.size.height } else { bounds.size.width };
                    natural_extent.set(f32::from(size));
                },
            ));
        let leading = if index > 0 { spacing * progress } else { 0.0 };
        let slot = div()
            .flex_none()
            .when(is_vertical, |slot| slot.w_full())
            .when(leading > 0.0, |slot| slot.ml(px(leading)));
        let slot = if is_running || entry.is_leaving {
            let extent = px(entry.natural_extent.get() * progress);
            slot.flex()
                .overflow_hidden()
                .opacity(progress)
                .when(is_vertical, |slot| slot.flex_col().h(extent))
                .when(!is_vertical, |slot| slot.flex_row().w(extent))
        } else {
            slot
        };
        slots.push(slot.child(content).into_any_element());
    }
    slots
}

/// ring of padding -3 around a selected dot
fn selection_ring(ring: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .top(px(-3.0))
        .left(px(-3.0))
        .right(px(-3.0))
        .bottom(px(-3.0))
        .child(centred_stroke(ring, 2.0, Shape::Circle))
}

fn resolved_chats(ids: &[String], store: &Entity<ChatFolderEditorStore>, cx: &mut App) -> Vec<ChatInfo> {
    let services = ChatServicesStore::shared(cx);
    let cx: &App = cx;
    let all = services.read(cx).current_chat_infos(cx);
    let mut by_id: HashMap<&str, &ChatInfo> = HashMap::with_capacity(all.len());
    for chat in &all {
        by_id.entry(chat.id.as_str()).or_insert(chat);
    }
    ids.iter()
        .filter_map(|id| {
            by_id
                .get(id.as_str())
                .map(|chat| (*chat).clone())
                .or_else(|| store.read(cx).selected_chat_models().get(id).cloned())
                .or_else(|| services.read(cx).chat_info(id, cx))
        })
        .collect()
}

pub fn folder_chat_display_name(chat: &ChatInfo) -> String {
    if chat.r#type == ChatType::Direct {
        return chat.direct_peer_profile_name();
    }
    if chat.r#type == ChatType::Favourites {
        return tr("saved_messages");
    }
    chat.title.clone()
}

/// colour as #RRGGBB in srgb
/// a wide gamut picker can read components below 0 or above 1 so each channel is clamped
pub fn folder_hex_string(color: Hsla) -> String {
    let rgba = color.to_rgb();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", channel(rgba.r), channel(rgba.g), channel(rgba.b))
}

#[derive(IntoElement)]
pub struct FolderSection {
    title: String,
    content: AnyElement,
}

impl FolderSection {
    pub fn new(title: String, content: AnyElement) -> Self {
        Self { title, content }
    }
}

impl RenderOnce for FolderSection {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        vstack(8.0)
            .w_full()
            .items_start()
            .child(
                div().pl(px(14.0)).child(
                    MainText::new(self.title.to_uppercase())
                        .px(12.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary),
                ),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .bg(theme.bg._200)
                    .continuous_corners(22.0)
                    .overflow_hidden()
                    .child(self.content),
            )
    }
}

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ListAlignment, ListState, ParentElement, Render, Styled, Subscription, WeakEntity, Window, div, list, px,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::input_ui::{InputUiEvent, InputUiState};
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::rich_text::document::{
    RiseBlockTextView, RiseDocument, RiseDocumentEditorRow, RiseDocumentEditorRowKind, RiseDocumentFocus,
    RiseDocumentTextStyle,
};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::RichFont;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::rich_text::document::rise_document_editor_hooks::{
    RiseDocumentEditorPickerPanel, rise_document_editor_hooks,
};
use crate::core::ui::rich_text::document::rise_document_editor_model::RiseDocumentEditorModel;
use crate::core::ui::rich_text::document::rise_document_editor_toolbar::RiseDocumentEditorToolbar;
use crate::core::ui::rich_text::document::rise_document_table_editor::RiseDocumentTableEditor;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;
use crate::modules::media_picker::ui::media_picker_header_icon_button::MediaPickerHeaderIconButton;

type ScreenAction = Rc<dyn Fn(&mut Window, &mut App)>;
type SendAction = Rc<dyn Fn(RiseDocument, Vec<MediaPickerItem>, Option<String>, &mut Window, &mut App)>;

const PANEL_HEIGHT: f32 = 300.0;
const ROW_MIN_HEIGHT: f32 = 22.0;
const ROW_SPACING: f32 = 2.0;
const HEADER_ICON_SIZE: f32 = 15.0;
const SEND_SIDE: f32 = 34.0;
const DISABLED_HISTORY_OPACITY: f32 = 0.35;
const DISABLED_SEND_OPACITY: f32 = 0.4;
const DIVIDER_OPACITY: f32 = 0.35;
const SPOILER_OPACITY: f32 = 0.28;
const HTTPS_PREFIX: &str = "https://";
const SCHEME_SEPARATOR: &str = "://";

/// the formatting editor, mounted inside the content of whatever sheet opened it and never as a sibling
/// one presenter per surface keeps a second sheet from opening invisibly over it
/// this screen presents nothing of its own
pub struct RiseDocumentEditorScreen {
    pub model: Entity<RiseDocumentEditorModel>,
    pub on_cancel: ScreenAction,
    pub on_send: SendAction,
    pub on_attach_media: Option<ScreenAction>,
    pub on_emoji: Option<ScreenAction>,

    row_heights: HashMap<Uuid, f32>,
    is_asking_for_link: bool,
    link_url: Entity<InputUiState>,
    list_state: ListState,
    row_ids: Vec<Uuid>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl RiseDocumentEditorScreen {
    pub fn new(
        model: Entity<RiseDocumentEditorModel>,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_send: impl Fn(RiseDocument, Vec<MediaPickerItem>, Option<String>, &mut Window, &mut App) + 'static,
        on_attach_media: Option<ScreenAction>,
        on_emoji: Option<ScreenAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let link_url = cx.new(InputUiState::single_line);
        let placeholder = tr("doc_link_url_placeholder");
        link_url.update(cx, |field, cx| {
            field.set_placeholder(placeholder, cx);
            field.set_chromeless(true, cx);
            field.set_escape_to_layer(false);
        });
        let row_ids: Vec<Uuid> = model.read(cx).rows().iter().map(|row| row.id).collect();
        let overdraw = ThemeStore::shared(cx).read(cx).current_theme().list.overdraw;
        let list_state = ListState::new(row_ids.len(), ListAlignment::Top, overdraw);
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&link_url, window, |this, _, event, window, cx| match event {
                InputUiEvent::Submitted => this.confirm_link(window, cx),
                InputUiEvent::Cancelled => this.cancel_link(cx),
                InputUiEvent::Changed => {}
            }),
        ];
        Self {
            model,
            on_cancel: Rc::new(on_cancel),
            on_send: Rc::new(on_send),
            on_attach_media,
            on_emoji,
            row_heights: HashMap::new(),
            is_asking_for_link: false,
            link_url,
            list_state,
            row_ids,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn normalized_link(raw: &str) -> String {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return String::new();
        }
        let candidate =
            if trimmed.contains(SCHEME_SEPARATOR) { trimmed.to_owned() } else { format!("{HTTPS_PREFIX}{trimmed}") };
        match Url::from_string(&candidate).and_then(|url| url.host()) {
            Some(host) if !host.is_empty() => candidate,
            _ => String::new(),
        }
    }

    fn confirm_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = Self::normalized_link(self.link_url.read(cx).text());
        self.link_url.update(cx, |field, cx| field.clear(cx));
        self.is_asking_for_link = false;
        cx.notify();
        self.focus_handle.focus(window, cx);
        if address.is_empty() {
            return;
        }
        self.model.update(cx, |model, cx| {
            model.toggle_mark(rise_canon::RiseDocumentInlineMark::Link, Some(address), cx);
        });
    }

    fn cancel_link(&mut self, cx: &mut Context<Self>) {
        self.link_url.update(cx, |field, cx| field.clear(cx));
        self.is_asking_for_link = false;
        cx.notify();
    }

    fn toggle_emoji_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).is_keyboard_suspended {
            self.model.update(cx, |model, cx| model.set_is_keyboard_suspended(false, cx));
        } else {
            self.focus_handle.focus(window, cx);
            self.model.update(cx, |model, cx| model.set_is_keyboard_suspended(true, cx));
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" || !self.model.read(cx).is_keyboard_suspended {
            return;
        }
        cx.stop_propagation();
        self.model.update(cx, |model, cx| model.set_is_keyboard_suspended(false, cx));
    }

    fn sync_rows(&mut self, cx: &App) {
        let ids: Vec<Uuid> = self.model.read(cx).rows().iter().map(|row| row.id).collect();
        if ids == self.row_ids {
            return;
        }
        let prefix = self.row_ids.iter().zip(&ids).take_while(|(old, new)| old == new).count();
        let room = self.row_ids.len().min(ids.len()) - prefix;
        let suffix = self.row_ids.iter().rev().zip(ids.iter().rev()).take(room).take_while(|(old, new)| old == new).count();
        self.list_state.splice(prefix..self.row_ids.len() - suffix, ids.len() - prefix - suffix);
        self.row_heights.retain(|id, _| ids.contains(id));
        self.row_ids = ids;
    }

    fn header(&self, theme: &AppTheme, cx: &App) -> AnyElement {
        let model = self.model.read(cx);
        let can_undo = model.can_undo();
        let can_redo = model.can_redo();
        let send_disabled = model.is_empty() || model.projection_overflow() > 0;

        let title = div().absolute().inset_0().flex().items_center().justify_center().child(
            MainText::new(tr("doc_editor_title"))
                .px(17.)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary),
        );

        let on_cancel = self.on_cancel.clone();
        let undo_model = self.model.clone();
        let redo_model = self.model.clone();
        let send_model = self.model.clone();
        let on_send = self.on_send.clone();
        let send = plain("rise-document-editor-send")
            .disabled(send_disabled)
            .on_press(move |window, cx| {
                let model = send_model.read(cx);
                let document = model.document();
                let media = model.attached_media().to_vec();
                let entities = model.projected_entities_json();
                on_send(document, media, entities, window, cx);
            })
            .opacity(if send_disabled { DISABLED_SEND_OPACITY } else { 1. })
            .child(
                div()
                    .size(px(SEND_SIDE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.primary._100)
                    .children(IconUi::sized("paperplane.fill", px(HEADER_ICON_SIZE), theme.bg._100, false)),
            );

        let controls = hstack(4.)
            .child(
                div().opacity(if can_undo { 1. } else { DISABLED_HISTORY_OPACITY }).child(
                    MediaPickerHeaderIconButton::new("rise-document-editor-undo", "arrow.uturn.backward", move |_, cx| {
                        undo_model.update(cx, |model, cx| model.undo(cx));
                    })
                    .font_size(HEADER_ICON_SIZE)
                    .disabled(!can_undo),
                ),
            )
            .child(
                div().opacity(if can_redo { 1. } else { DISABLED_HISTORY_OPACITY }).child(
                    MediaPickerHeaderIconButton::new("rise-document-editor-redo", "arrow.uturn.forward", move |_, cx| {
                        redo_model.update(cx, |model, cx| model.redo(cx));
                    })
                    .font_size(HEADER_ICON_SIZE)
                    .disabled(!can_redo),
                ),
            )
            .child(send);

        let row = hstack(None)
            .w_full()
            .child(
                MediaPickerHeaderIconButton::new("rise-document-editor-cancel", "xmark", move |window, cx| {
                    on_cancel(window, cx)
                })
                .font_size(HEADER_ICON_SIZE),
            )
            .child(spacer())
            .child(controls);

        div()
            .relative()
            .w_full()
            .px(px(14.))
            .py(px(10.))
            .bg(theme.bg._100)
            .child(title)
            .child(row)
            .into_any_element()
    }

    fn overflow_notice(&self, cx: &App) -> Option<AnyElement> {
        let overflow = self.model.read(cx).projection_overflow();
        if overflow == 0 {
            return None;
        }
        Some(
            div()
                .w_full()
                .py(px(4.))
                .flex()
                .justify_center()
                .child(
                    SecondaryText::new(RiseFormat::compact_count(i64::try_from(overflow).unwrap_or(i64::MAX)))
                        .px(12.)
                        .tac(TextAlignT::Center),
                )
                .into_any_element(),
        )
    }

    fn placeholder(model: &RiseDocumentEditorModel, row: &RiseDocumentEditorRow) -> String {
        if model.rows().len() != 1 || !row.is_empty() {
            if matches!(row.kind, RiseDocumentEditorRowKind::CollapsibleTitle { .. }) && row.is_empty() {
                return tr("doc_collapsible_title_placeholder");
            }
            return String::new();
        }
        tr("doc_editor_placeholder")
    }

    fn style(theme: &AppTheme) -> RiseDocumentTextStyle {
        RiseDocumentTextStyle {
            body: RichFont::system(16.),
            mono: RichFont::system_design(14., FontDesign::Monospaced),
            heading_fonts: HashMap::from([
                (1, RichFont::system_weight(22., FontWeight::BOLD)),
                (2, RichFont::system_weight(19., FontWeight::SEMIBOLD)),
                (3, RichFont::system_weight(17., FontWeight::SEMIBOLD)),
            ]),
            text_color: theme.text.primary,
            secondary_color: theme.text.secondary,
            link_color: theme.primary._100,
            code_color: theme.text.primary,
            code_background: theme.bg._300,
            spoiler_background: theme.text.secondary.opacity(SPOILER_OPACITY),
        }
    }

    fn render_row(&self, index: usize, screen: WeakEntity<Self>, theme: &AppTheme, cx: &App) -> AnyElement {
        let model = self.model.read(cx);
        let Some(row) = model.rows().get(index) else {
            return div().into_any_element();
        };
        let is_first = index == 0;
        let is_last = index + 1 == model.rows().len();
        let row_id = row.id;
        let content = RiseDocumentEditorRowView {
            row: row.clone(),
            model: self.model.clone(),
            style: Self::style(theme),
            placeholder: Self::placeholder(model, row),
            height: self.row_heights.get(&row.id).copied().unwrap_or(0.),
            order_in_list: order_in_list(model.rows(), index),
            focus: model.focus,
            is_keyboard_suspended: model.is_keyboard_suspended,
            theme: theme.clone(),
            on_height_change: Rc::new(move |height: f32, cx: &mut App| {
                let screen = screen.clone();
                cx.defer(move |cx| {
                    if let Some(screen) = screen.upgrade() {
                        screen.update(cx, |screen, cx| {
                            screen.row_heights.insert(row_id, height);
                            cx.notify();
                        });
                    }
                });
            }),
        };
        div()
            .w_full()
            .px(px(16.))
            .when(is_first, |row| row.pt(px(12.)))
            .when(!is_last, |row| row.pb(px(ROW_SPACING)))
            .when(is_last, |row| row.pb(px(24.)))
            .child(content)
            .into_any_element()
    }
}

impl Render for RiseDocumentEditorScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_rows(cx);
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let header = self.header(&theme, cx);
        let overflow = self.overflow_notice(cx);

        let screen = cx.weak_entity();
        let list_theme = theme.clone();
        let rows = list(
            self.list_state.clone(),
            cx.processor(move |this: &mut Self, index: usize, _window, cx| {
                this.render_row(index, screen.clone(), &list_theme, cx)
            }),
        )
        .flex_1()
        .w_full();

        let link_screen = cx.weak_entity();
        let emoji_screen = cx.weak_entity();
        let toolbar = RiseDocumentEditorToolbar::new(self.model.clone(), move |window, cx| {
            if let Some(screen) = link_screen.upgrade() {
                screen.update(cx, |screen, cx| {
                    screen.is_asking_for_link = true;
                    cx.notify();
                    screen.link_url.focus_handle(cx).focus(window, cx);
                });
            }
        })
        .on_attach_media(self.on_attach_media.clone())
        .on_emoji(Some(Rc::new(move |window: &mut Window, cx: &mut App| {
            if let Some(screen) = emoji_screen.upgrade() {
                screen.update(cx, |screen, cx| screen.toggle_emoji_panel(window, cx));
            }
        })));

        let panel = if self.model.read(cx).is_keyboard_suspended {
            rise_document_editor_hooks().map(|hooks| {
                let emoji_model = self.model.clone();
                let custom_model = self.model.clone();
                let backspace_model = self.model.clone();
                let panel = (hooks.chat_input_picker_panel)(
                    RiseDocumentEditorPickerPanel {
                        panel_height: PANEL_HEIGHT,
                        on_select_emoji: Rc::new(move |emoji: String, _: &mut Window, cx: &mut App| {
                            emoji_model.update(cx, |model, cx| model.insert_text(&emoji, cx));
                        }),
                        on_select_custom_emoji: Rc::new(move |item: StickerItem, _: &mut Window, cx: &mut App| {
                            custom_model.update(cx, |model, cx| model.insert_custom_emoji(item, cx));
                        }),
                        on_backspace: Rc::new(move |_: &mut Window, cx: &mut App| {
                            backspace_model.update(cx, |model, cx| model.delete_backward(cx));
                        }),
                        uses_flat_chrome: true,
                    },
                    window,
                    cx,
                );
                div()
                    .w_full()
                    .child(div().w_full().h(px(PANEL_HEIGHT)).child(panel).keyboard_dismiss_excluded())
            })
        } else {
            None
        };

        let link_prompt = self.is_asking_for_link.then(|| self.link_prompt(&theme, cx));

        div()
            .id("rise-document-editor-screen")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .child(header)
            .child(rows)
            .children(overflow)
            .child(toolbar)
            .children(panel)
            .children(link_prompt)
    }
}

impl RiseDocumentEditorScreen {
    /// alert with a text field
    /// a dialog over the editor and not another sheet since the editor already lives inside a
    /// presented surface enter confirms and escape cancels
    fn link_prompt(&self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let cancel = cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_link(cx));
        let done = cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_link(window, cx));
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.2))
            .child(
                vstack(12.)
                    .w(px(270.))
                    .p(px(16.))
                    .rounded(px(14.))
                    .bg(theme.bg._200)
                    .child(
                        MainText::new(tr("doc_link_sheet_title"))
                            .px(17.)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary),
                    )
                    .child(
                        div()
                            .w_full()
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .bg(theme.bg._100)
                            .child(self.link_url.clone()),
                    )
                    .child(
                        hstack(12.)
                            .w_full()
                            .child(
                                div()
                                    .id("rise-document-link-cancel")
                                    .flex_1()
                                    .flex()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_click(cancel)
                                    .child(MainText::new(tr("cancel")).px(17.).color(theme.primary._100)),
                            )
                            .child(
                                div()
                                    .id("rise-document-link-done")
                                    .flex_1()
                                    .flex()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_click(done)
                                    .child(
                                        MainText::new(tr("common_done"))
                                            .px(17.)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(theme.primary._100),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// position within the run of list items this row belongs to so a numbered list restarts at one
/// after any other block
fn order_in_list(rows: &[RiseDocumentEditorRow], index: usize) -> usize {
    let Some(RiseDocumentEditorRowKind::ListItem { style, .. }) = rows.get(index).map(|row| &row.kind) else {
        return 1;
    };
    let mut position = 1;
    for previous in rows[..index].iter().rev() {
        match &previous.kind {
            RiseDocumentEditorRowKind::ListItem { style: previous_style, .. } if previous_style == style => {
                position += 1;
            }
            _ => break,
        }
    }
    position
}

#[derive(IntoElement)]
struct RiseDocumentEditorRowView {
    row: RiseDocumentEditorRow,
    model: Entity<RiseDocumentEditorModel>,
    style: RiseDocumentTextStyle,
    placeholder: String,
    height: f32,
    order_in_list: usize,
    focus: Option<RiseDocumentFocus>,
    is_keyboard_suspended: bool,
    theme: AppTheme,
    on_height_change: Rc<dyn Fn(f32, &mut App)>,
}

impl RiseDocumentEditorRowView {
    fn text_view(&self) -> AnyElement {
        let row_id = self.row.id;
        let runs_model = self.model.clone();
        let verbatim_model = self.model.clone();
        let selection_model = self.model.clone();
        let split_model = self.model.clone();
        let backspace_model = self.model.clone();
        let focus_model = self.model.clone();
        let on_height_change = self.on_height_change.clone();
        let view = RiseBlockTextView::new(
            gpui::ElementId::Name(format!("rise-document-row-{}", row_id.uuid_string()).into()),
            row_id.uuid_string(),
            self.row.kind.clone(),
            self.row.runs.clone(),
            self.row.verbatim.clone(),
            self.style.clone(),
            self.placeholder.clone(),
            self.focus == Some(RiseDocumentFocus::Row(row_id)) && !self.is_keyboard_suspended,
        )
        .on_runs_change(move |runs, cx| {
            runs_model.update(cx, |model, cx| model.replace_runs(runs, row_id, cx));
        })
        .on_verbatim_change(move |text, cx| {
            verbatim_model.update(cx, |model, cx| model.replace_verbatim(text, row_id, cx));
        })
        .on_selection_change(move |range, cx| {
            selection_model.update(cx, |model, cx| {
                if model.focus != Some(RiseDocumentFocus::Row(row_id)) {
                    return;
                }
                model.set_selection(range, cx);
            });
        })
        .on_split(move |offset, cx| {
            split_model.update(cx, |model, cx| model.split_row(row_id, offset, cx));
        })
        .on_backspace_at_start(move |cx| {
            backspace_model.update(cx, |model, cx| model.backspace_at_start(row_id, cx));
        })
        .on_focus(move |cx| {
            focus_model.update(cx, |model, cx| model.set_focus(Some(RiseDocumentFocus::Row(row_id)), cx));
        })
        .on_height_change(move |height, cx| on_height_change(height, cx));
        div().flex_1().min_w_0().min_h(px(self.height.max(ROW_MIN_HEIGHT))).child(view).into_any_element()
    }

    fn adornment(&self) -> Option<AnyElement> {
        let theme = &self.theme;
        match &self.row.kind {
            RiseDocumentEditorRowKind::Quote | RiseDocumentEditorRowKind::PullQuote => Some(
                div().w(px(3.)).self_stretch().rounded(px(1.5)).bg(theme.primary._100).into_any_element(),
            ),
            RiseDocumentEditorRowKind::ListItem { style, is_done } => Some(self.list_marker(*style, *is_done)),
            RiseDocumentEditorRowKind::CollapsibleTitle { .. } => Some(
                div()
                    .pt(px(4.))
                    .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(12.), theme.text.secondary, false))
                    .into_any_element(),
            ),
            RiseDocumentEditorRowKind::CollapsibleBody => Some(div().w(px(14.)).into_any_element()),
            _ => None,
        }
    }

    fn list_marker(&self, style: rise_canon::RiseDocumentListStyle, is_done: bool) -> AnyElement {
        let theme = &self.theme;
        match style {
            rise_canon::RiseDocumentListStyle::Bulleted => {
                div().mt(px(9.)).size(px(5.)).rounded_full().bg(theme.text.primary).into_any_element()
            }
            rise_canon::RiseDocumentListStyle::Numbered => div()
                .pt(px(1.))
                .child(SecondaryText::new(format!("{}.", self.order_in_list)).px(15.))
                .into_any_element(),
            rise_canon::RiseDocumentListStyle::Task => {
                let model = self.model.clone();
                let row_id = self.row.id;
                let symbol = if is_done { "checkmark.square.fill" } else { "square" };
                let color = if is_done { theme.primary._100 } else { theme.text.secondary };
                plain(gpui::ElementId::Name(format!("rise-document-task-{}", row_id.uuid_string()).into()))
                    .pt(px(2.))
                    .on_press(move |_, cx| model.update(cx, |model, cx| model.toggle_task(row_id, cx)))
                    .children(IconUi::sized(symbol, px(16.), color, false))
                    .into_any_element()
            }
        }
    }

    fn vertical_padding(&self) -> f32 {
        if matches!(self.row.kind, RiseDocumentEditorRowKind::Heading { .. }) { 6. } else { 2. }
    }
}

impl gpui::RenderOnce for RiseDocumentEditorRowView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        match &self.row.kind {
            RiseDocumentEditorRowKind::Divider => div()
                .w_full()
                .py(px(10.))
                .child(div().w_full().h(px(1.)).bg(theme.text.secondary.opacity(DIVIDER_OPACITY)))
                .into_any_element(),
            RiseDocumentEditorRowKind::Table(table) => RiseDocumentTableEditor::new(
                self.row.id,
                table.clone(),
                self.model.clone(),
                self.style.clone(),
                theme,
            )
            .into_any_element(),
            RiseDocumentEditorRowKind::Media { index } => div()
                .py(px(6.))
                .child(SecondaryText::new(format!("{} #{}", tr("doc_block_media"), index + 1)).px(13.))
                .into_any_element(),
            RiseDocumentEditorRowKind::Unsupported(_) => div()
                .py(px(6.))
                .child(SecondaryText::new(tr("doc_unsupported_block")).px(13.))
                .into_any_element(),
            _ => hstack(8.)
                .items_start()
                .w_full()
                .py(px(self.vertical_padding()))
                .children(self.adornment())
                .child(self.text_view())
                .into_any_element(),
        }
    }
}

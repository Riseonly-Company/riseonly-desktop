use std::rc::Rc;

use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_canon::{RiseDocumentInlineMark, RiseDocumentListStyle};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::rich_text::document::RiseDocumentEditorRowKind;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::rich_text::document::rise_document_editor_model::RiseDocumentEditorModel;

type ToolbarAction = Rc<dyn Fn(&mut Window, &mut App)>;

const ICON_SIDE: f32 = 16.0;
const ICON_FRAME: f32 = 34.0;
const ICON_CORNER_RADIUS: f32 = 10.0;
const ICON_ON_FILL_OPACITY: f32 = 0.16;
const DEFAULT_TABLE_SIDE: usize = 2;

/// row of controls above the text, two states swapped by whether there is a selection
/// a caret offers what to insert and a selection what to apply
#[derive(IntoElement)]
pub struct RiseDocumentEditorToolbar {
    pub model: Entity<RiseDocumentEditorModel>,
    pub on_attach_media: Option<ToolbarAction>,
    pub on_emoji: Option<ToolbarAction>,
    pub on_link: ToolbarAction,
}

impl RiseDocumentEditorToolbar {
    pub fn new(model: Entity<RiseDocumentEditorModel>, on_link: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { model, on_attach_media: None, on_emoji: None, on_link: Rc::new(on_link) }
    }

    pub fn on_attach_media(mut self, action: Option<ToolbarAction>) -> Self {
        self.on_attach_media = action;
        self
    }

    pub fn on_emoji(mut self, action: Option<ToolbarAction>) -> Self {
        self.on_emoji = action;
        self
    }

    fn insertion_controls(&self, theme: &AppTheme, allows_embedded_media: bool) -> Vec<AnyElement> {
        let mut controls: Vec<AnyElement> = Vec::new();

        let blocks = vec![
            kind_item(&self.model, 0, tr("doc_block_paragraph"), "text.alignleft", "paragraph", RiseDocumentEditorRowKind::Paragraph),
            kind_item(
                &self.model,
                1,
                tr("doc_block_heading"),
                "textformat.size.larger",
                "heading",
                RiseDocumentEditorRowKind::Heading { level: 1 },
            ),
            kind_item(&self.model, 2, tr("doc_block_quote"), "text.quote", "quote", RiseDocumentEditorRowKind::Quote),
            kind_item(
                &self.model,
                3,
                tr("doc_block_pull_quote"),
                "quote.opening",
                "pull_quote",
                RiseDocumentEditorRowKind::PullQuote,
            ),
            kind_item(
                &self.model,
                4,
                tr("doc_block_code"),
                "curlybraces",
                "code",
                RiseDocumentEditorRowKind::Code { language: None },
            ),
            kind_item(&self.model, 5, tr("doc_block_formula"), "function", "formula", RiseDocumentEditorRowKind::Formula),
            insert_item(
                &self.model,
                6,
                tr("doc_block_button_row"),
                "rectangle.grid.1x2",
                "button_row",
                RiseDocumentEditorRowKind::Buttons(Vec::new()),
            ),
            insert_item(&self.model, 7, tr("doc_block_divider"), "minus", "divider", RiseDocumentEditorRowKind::Divider)
                .begins_section(true),
        ];
        controls.push(
            ContextMenuUi::new("rise-document-toolbar-blocks", blocks, icon("plus", theme, false))
                .shows_item_icons()
                .into_any_element(),
        );

        let lists = vec![
            kind_item(&self.model, 0, tr("doc_list_none"), "text.alignleft", "list_none", RiseDocumentEditorRowKind::Paragraph),
            kind_item(
                &self.model,
                1,
                tr("doc_list_bulleted"),
                "list.bullet",
                "bulleted",
                RiseDocumentEditorRowKind::ListItem { style: RiseDocumentListStyle::Bulleted, is_done: false },
            ),
            kind_item(
                &self.model,
                2,
                tr("doc_list_numbered"),
                "list.number",
                "numbered",
                RiseDocumentEditorRowKind::ListItem { style: RiseDocumentListStyle::Numbered, is_done: false },
            ),
            kind_item(
                &self.model,
                3,
                tr("doc_list_task"),
                "checklist",
                "task",
                RiseDocumentEditorRowKind::ListItem { style: RiseDocumentListStyle::Task, is_done: false },
            ),
            kind_item(
                &self.model,
                4,
                tr("doc_block_collapsible"),
                "chevron.right.square",
                "collapsible",
                RiseDocumentEditorRowKind::CollapsibleTitle { expanded: false },
            )
            .begins_section(true),
        ];
        controls.push(
            ContextMenuUi::new("rise-document-toolbar-lists", lists, icon("list.bullet", theme, false))
                .shows_item_icons()
                .into_any_element(),
        );

        let model = self.model.clone();
        controls.push(
            plain("rise-document-toolbar-table")
                .on_press(move |_, cx| {
                    model.update(cx, |model, cx| model.insert_table(DEFAULT_TABLE_SIDE, DEFAULT_TABLE_SIDE, cx));
                })
                .child(icon("tablecells", theme, false))
                .into_any_element(),
        );

        if allows_embedded_media && let Some(on_attach_media) = self.on_attach_media.clone() {
            controls.push(
                plain("rise-document-toolbar-media")
                    .on_press(move |window, cx| on_attach_media(window, cx))
                    .child(icon("paperclip", theme, false))
                    .into_any_element(),
            );
        }

        if let Some(on_emoji) = self.on_emoji.clone() {
            controls.push(
                plain("rise-document-toolbar-emoji")
                    .on_press(move |window, cx| on_emoji(window, cx))
                    .child(icon("face.smiling", theme, false))
                    .into_any_element(),
            );
        }
        controls
    }

    fn selection_controls(&self, theme: &AppTheme, cx: &App) -> Vec<AnyElement> {
        let active = self.model.read(cx).active_marks();
        RiseDocumentInlineMark::ALL
            .iter()
            .copied()
            .map(|mark| {
                let id = format!("rise-document-toolbar-mark-{}", mark.wire());
                if mark == RiseDocumentInlineMark::Link {
                    let on_link = self.on_link.clone();
                    plain(id)
                        .on_press(move |window, cx| on_link(window, cx))
                        .child(icon("link", theme, active.contains(&RiseDocumentInlineMark::Link)))
                        .into_any_element()
                } else {
                    let model = self.model.clone();
                    plain(id)
                        .on_press(move |_, cx| {
                            model.update(cx, |model, cx| model.toggle_mark(mark, None, cx));
                        })
                        .child(icon(Self::symbol(mark), theme, active.contains(&mark)))
                        .into_any_element()
                }
            })
            .collect()
    }

    /// symbol names are a platform property of a canon type so they live here and not in canon
    /// android and ios draw the same mark with different assets
    /// the kit maps each name to its desktop glyph
    fn symbol(mark: RiseDocumentInlineMark) -> &'static str {
        match mark {
            RiseDocumentInlineMark::Bold => "bold",
            RiseDocumentInlineMark::Italic => "italic",
            RiseDocumentInlineMark::Underline => "underline",
            RiseDocumentInlineMark::Strikethrough => "strikethrough",
            RiseDocumentInlineMark::Code => "chevron.left.forwardslash.chevron.right",
            RiseDocumentInlineMark::Spoiler => "eye.slash",
            RiseDocumentInlineMark::Link => "link",
        }
    }
}

impl RenderOnce for RiseDocumentEditorToolbar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let model = self.model.read(cx);
        let has_selection = model.has_selection();
        let allows_embedded_media = model.allows_embedded_media;
        let controls = if has_selection {
            self.selection_controls(&theme, cx)
        } else {
            self.insertion_controls(&theme, allows_embedded_media)
        };
        hstack(6.)
            .px(px(10.))
            .py(px(8.))
            .w_full()
            .justify_start()
            .bg(theme.bg._200)
            .children(controls)
    }
}

fn kind_item(
    model: &Entity<RiseDocumentEditorModel>,
    id: i64,
    label: String,
    symbol: &str,
    key: &str,
    kind: RiseDocumentEditorRowKind,
) -> ContextMenuItem {
    let model = model.clone();
    ContextMenuItem::new(id, label, Some(symbol.to_owned()), key, move |_, cx| {
        let kind = kind.clone();
        model.update(cx, |model, cx| model.set_kind(kind, None, cx));
    })
}

fn insert_item(
    model: &Entity<RiseDocumentEditorModel>,
    id: i64,
    label: String,
    symbol: &str,
    key: &str,
    kind: RiseDocumentEditorRowKind,
) -> ContextMenuItem {
    let model = model.clone();
    ContextMenuItem::new(id, label, Some(symbol.to_owned()), key, move |_, cx| {
        let kind = kind.clone();
        model.update(cx, |model, cx| model.insert_row(kind, cx));
    })
}

fn icon(symbol: &str, theme: &AppTheme, is_on: bool) -> AnyElement {
    let color = if is_on { theme.primary._100 } else { theme.text.primary };
    let fill = if is_on { theme.primary._100.opacity(ICON_ON_FILL_OPACITY) } else { gpui::transparent_black() };
    div()
        .size(px(ICON_FRAME))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(ICON_CORNER_RADIUS))
        .bg(fill)
        .children(IconUi::sized(symbol, px(ICON_SIDE), color, false))
        .into_any_element()
}

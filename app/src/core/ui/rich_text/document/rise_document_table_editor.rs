use std::collections::HashMap;

use gpui::{
    AnyElement, App, Entity, InteractiveElement, IntoElement, ParentElement, RenderOnce, StatefulInteractiveElement,
    Styled, Window, div, px,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::rich_text::document::{
    RiseBlockTextView, RiseDocumentEditorRowKind, RiseDocumentTable, RiseDocumentTableCell, RiseDocumentTextStyle,
};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::rich_text::document::rise_document_editor_model::RiseDocumentEditorModel;

const CELL_WIDTH: f32 = 132.0;
const CELL_MIN_HEIGHT: f32 = 22.0;
const GRID_CORNER_RADIUS: f32 = 8.0;
const GRID_STROKE_OPACITY: f32 = 0.22;
const FOCUSED_FILL_OPACITY: f32 = 0.12;
const FOCUSED_STROKE_WIDTH: f32 = 1.5;
const HEADER_LEVEL: i64 = 3;

/// an editable grid, every cell is an ordinary text surface so the selection toolbar
/// inline marks and run algebra work inside it unchanged
/// the shape is changed from a menu under the table and not from drag handles
#[derive(IntoElement)]
pub struct RiseDocumentTableEditor {
    pub row_id: Uuid,
    pub table: RiseDocumentTable,
    pub model: Entity<RiseDocumentEditorModel>,
    pub style: RiseDocumentTextStyle,
    pub theme: AppTheme,
}

/// measured height of every cell text, keyed like the cell identity
#[derive(Default)]
struct RiseDocumentTableEditorState {
    cell_heights: HashMap<String, f32>,
}

impl RiseDocumentTableEditor {
    pub fn new(
        row_id: Uuid,
        table: RiseDocumentTable,
        model: Entity<RiseDocumentEditorModel>,
        style: RiseDocumentTextStyle,
        theme: AppTheme,
    ) -> Self {
        Self { row_id, table, model, style, theme }
    }

    fn focused_cell(&self, cx: &App) -> Option<(usize, usize)> {
        let focus = self.model.read(cx).focus?;
        if focus.row_id() != self.row_id {
            return None;
        }
        focus.table_cell()
    }

    fn grid(&self, state: &Entity<RiseDocumentTableEditorState>, cx: &App) -> AnyElement {
        let rows: Vec<AnyElement> = self
            .table
            .rows
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                hstack(1.)
                    .items_start()
                    .children(
                        row.cells
                            .iter()
                            .enumerate()
                            .map(|(column_index, cell)| self.cell_view(cell, row_index, column_index, state, cx)),
                    )
                    .into_any_element()
            })
            .collect();
        let stroke = centred_stroke(
            self.theme.text.secondary.opacity(GRID_STROKE_OPACITY),
            1.,
            Shape::RoundedRectangle { corner_radius: GRID_CORNER_RADIUS, style: RoundedCornerStyle::Continuous },
        );
        div()
            .relative()
            .child(
                vstack(1.)
                    .items_start()
                    .rounded(px(GRID_CORNER_RADIUS))
                    .overflow_hidden()
                    .children(rows),
            )
            .child(stroke)
            .into_any_element()
    }

    fn cell_view(
        &self,
        cell: &RiseDocumentTableCell,
        row_index: usize,
        column_index: usize,
        state: &Entity<RiseDocumentTableEditorState>,
        cx: &App,
    ) -> AnyElement {
        let key = format!("{}-{}-{}", self.row_id.uuid_string(), row_index, column_index);
        let focused_cell = self.focused_cell(cx);
        let is_focused = focused_cell == Some((row_index, column_index));
        let is_header = self.table.has_header_row && row_index == 0;
        let model = self.model.read(cx);
        let is_keyboard_suspended = model.is_keyboard_suspended;
        let height = state.read(cx).cell_heights.get(&key).copied().unwrap_or(0.);
        let row_id = self.row_id;

        let kind = if is_header {
            RiseDocumentEditorRowKind::Heading { level: HEADER_LEVEL }
        } else {
            RiseDocumentEditorRowKind::Paragraph
        };
        let on_runs_model = self.model.clone();
        let on_selection_model = self.model.clone();
        let on_focus_model = self.model.clone();
        let height_state = state.clone();
        let height_key = key.clone();
        let text_view = RiseBlockTextView::new(
            gpui::ElementId::Name(format!("rise-document-cell-{key}").into()),
            key.clone(),
            kind,
            RiseDocumentEditorModel::cell_runs(cell),
            "",
            self.style.clone(),
            "",
            is_focused && !is_keyboard_suspended,
        )
        .on_runs_change(move |runs, cx| {
            on_runs_model.update(cx, |model, cx| {
                model.focus_table_cell(row_id, row_index, column_index, cx);
                model.set_focused_runs(runs, cx);
            });
        })
        .on_selection_change(move |range, cx| {
            if !is_focused {
                return;
            }
            on_selection_model.update(cx, |model, cx| model.set_selection(range, cx));
        })
        .on_focus(move |cx| {
            on_focus_model.update(cx, |model, cx| model.focus_table_cell(row_id, row_index, column_index, cx));
        })
        .on_height_change(move |height, cx| {
            let key = height_key.clone();
            let height_state = height_state.clone();
            cx.defer(move |cx| {
                height_state.update(cx, |state, cx| {
                    state.cell_heights.insert(key, height);
                    cx.notify();
                });
            });
        });

        let primary = self.theme.primary._100;
        let fill = if is_focused { primary.opacity(FOCUSED_FILL_OPACITY) } else { self.theme.bg._200 };
        let tap_model = self.model.clone();
        div()
            .id(gpui::ElementId::Name(format!("rise-document-cell-box-{key}").into()))
            .relative()
            .px(px(8.))
            .py(px(7.))
            .bg(fill)
            .child(div().w(px(CELL_WIDTH)).min_h(px(height.max(CELL_MIN_HEIGHT))).child(text_view))
            .when(is_focused, |cell| {
                cell.child(centred_stroke(primary, FOCUSED_STROKE_WIDTH, Shape::Rectangle))
            })
            .on_click(move |_, _, cx| {
                tap_model.update(cx, |model, cx| model.focus_table_cell(row_id, row_index, column_index, cx));
            })
            .into_any_element()
    }

    fn controls(&self, cx: &App) -> AnyElement {
        let focused_cell = self.focused_cell(cx);
        let row_id = self.row_id;
        let last_row = self.table.rows.len().saturating_sub(1);
        let last_column = self.table.column_count.saturating_sub(1);
        let target_row = focused_cell.map_or(last_row, |(row, _)| row);
        let target_column = focused_cell.map_or(last_column, |(_, column)| column);
        let row_count = self.table.rows.len();
        let column_count = self.table.column_count;

        let mut items: Vec<ContextMenuItem> = Vec::new();
        let model = self.model.clone();
        items.push(ContextMenuItem::new(
            0,
            tr("doc_table_add_row"),
            Some("plus.rectangle".to_owned()),
            "add_row",
            move |_, cx| model.update(cx, |model, cx| model.insert_table_row(row_id, target_row, cx)),
        ));
        let model = self.model.clone();
        items.push(ContextMenuItem::new(
            1,
            tr("doc_table_add_column"),
            Some("plus.rectangle.portrait".to_owned()),
            "add_column",
            move |_, cx| model.update(cx, |model, cx| model.insert_table_column(row_id, target_column, cx)),
        ));
        let model = self.model.clone();
        items.push(ContextMenuItem::new(
            2,
            tr("doc_table_delete_row"),
            Some("minus.rectangle".to_owned()),
            "delete_row",
            move |_, cx| {
                if row_count <= 1 {
                    return;
                }
                model.update(cx, |model, cx| model.delete_table_row(row_id, target_row, cx));
            },
        ).destructive(true).disabled(row_count <= 1).begins_section(true));
        let model = self.model.clone();
        items.push(ContextMenuItem::new(
            3,
            tr("doc_table_delete_column"),
            Some("minus.rectangle.portrait".to_owned()),
            "delete_column",
            move |_, cx| {
                if column_count <= 1 {
                    return;
                }
                model.update(cx, |model, cx| model.delete_table_column(row_id, target_column, cx));
            },
        ).destructive(true).disabled(column_count <= 1));
        let model = self.model.clone();
        items.push(ContextMenuItem::new(
            4,
            tr("doc_table_header_row"),
            Some("textformat".to_owned()),
            "header_row",
            move |_, cx| model.update(cx, |model, cx| model.toggle_table_header(row_id, cx)),
        ).begins_section(true));
        let model = self.model.clone();
        items.push(ContextMenuItem::new(
            5,
            tr("doc_table_delete"),
            Some("trash".to_owned()),
            "delete_table",
            move |_, cx| model.update(cx, |model, cx| model.delete_row(row_id, cx)),
        ).destructive(true));

        let label = div()
            .w(px(34.))
            .h(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .bg(self.theme.bg._200)
            .children(IconUi::sized("ellipsis", px(15.), self.theme.text.secondary, false));
        hstack(8.)
            .w_full()
            .child(ContextMenuUi::new(
                gpui::ElementId::Name(format!("rise-document-table-menu-{}", row_id.uuid_string()).into()),
                items,
                label,
            )
            .selected(self.table.has_header_row.then(|| "header_row".to_owned()))
            .shows_item_icons())
            .child(spacer_min_w(0.))
            .into_any_element()
    }
}

impl RenderOnce for RiseDocumentTableEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            gpui::ElementId::Name(format!("rise-document-table-{}", self.row_id.uuid_string()).into()),
            cx,
            |_, _| RiseDocumentTableEditorState::default(),
        );
        let grid = self.grid(&state, cx);
        let controls = self.controls(cx);
        vstack(6.)
            .items_start()
            .py(px(6.))
            .w_full()
            .child(
                div()
                    .id(gpui::ElementId::Name(format!("rise-document-table-scroll-{}", self.row_id.uuid_string()).into()))
                    .w_full()
                    .overflow_x_scroll()
                    .child(grid),
            )
            .child(controls)
    }
}

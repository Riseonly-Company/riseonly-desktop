#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, Hsla, ListAlignment, SharedString, Window, rgb};
use rise_core::foundation::foundation_compare::localized_case_insensitive_contains;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use super::bottom_sheet_debug_catalog::BottomSheetDebugCatalog;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationKind, BottomSheetDebugRegistrationMode,
    BottomSheetDebugRegistrationModule,
};
use crate::core::ui::input_ui::InputOutlinedRowUi;

const SYSTEM_ORANGE: u32 = 0xff9500;

pub type SaiBottomSheetGallerySelect = Rc<dyn Fn(&BottomSheetDebugRegistration, &mut Window, &mut App)>;

type GalleryFilter =
    (Option<BottomSheetDebugRegistrationModule>, Option<BottomSheetDebugRegistrationKind>, String);

/// rows of the filtered list, flat so the list only builds what is on screen
#[derive(Clone)]
enum SaiBottomSheetGalleryRow {
    Back,
    ModuleTitle(BottomSheetDebugRegistrationModule),
    Entry(BottomSheetDebugRegistration),
    NoResults,
}

struct SaiBottomSheetGalleryListState {
    list: ListUi,
    filter: Option<GalleryFilter>,
}

#[derive(IntoElement)]
pub struct SaiBottomSheetGalleryPanel {
    theme: AppTheme,
    module: Binding<Option<BottomSheetDebugRegistrationModule>>,
    query: Binding<String>,
    kind: Binding<Option<BottomSheetDebugRegistrationKind>>,
    on_select: SaiBottomSheetGallerySelect,
}

impl SaiBottomSheetGalleryPanel {
    pub fn new(
        theme: AppTheme,
        module: Binding<Option<BottomSheetDebugRegistrationModule>>,
        query: Binding<String>,
        kind: Binding<Option<BottomSheetDebugRegistrationKind>>,
        on_select: impl Fn(&BottomSheetDebugRegistration, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { theme, module, query, kind, on_select: Rc::new(on_select) }
    }

    fn entries(
        module: Option<BottomSheetDebugRegistrationModule>,
        kind: Option<BottomSheetDebugRegistrationKind>,
        query: &str,
    ) -> Vec<BottomSheetDebugRegistration> {
        BottomSheetDebugCatalog::entries()
            .iter()
            .filter(|entry| {
                (module.is_none() || Some(entry.module) == module)
                    && (kind.is_none() || Some(entry.kind) == kind)
                    && (query.is_empty()
                        || localized_case_insensitive_contains(&entry.title(), query)
                        || localized_case_insensitive_contains(&entry.module.title(), query)
                        || entry
                            .variant()
                            .is_some_and(|variant| localized_case_insensitive_contains(&variant, query))
                        || localized_case_insensitive_contains(&entry.source, query))
            })
            .cloned()
            .collect()
    }

    fn search_field(&self, query: &str) -> impl IntoElement + use<> {
        let theme = &self.theme;
        let field = InputOutlinedRowUi::new(
            "sai-bottom-sheet-gallery-search",
            self.query.clone(),
            tr("search"),
            div().children(IconUi::sized("magnifyingglass", px(17.), theme.text.secondary, false)),
        );
        if query.is_empty() {
            return field;
        }
        let binding = self.query.clone();
        field.trailing(
            div().id("sai-bottom-sheet-gallery-clear-tip").tooltip_label(tr("close")).child(
                plain("sai-bottom-sheet-gallery-clear")
                    .on_press(move |window, cx| binding.set(String::new(), window, cx))
                    .children(IconUi::sized("xmark.circle.fill", px(17.), theme.text.secondary, true)),
            ),
        )
    }

    fn kind_picker(&self, kind: Option<BottomSheetDebugRegistrationKind>) -> impl IntoElement + use<> {
        let mut titles = vec![SharedString::from(tr("report_filter_all"))];
        titles.extend(BottomSheetDebugRegistrationKind::ALL_CASES.iter().map(|item| SharedString::from(item.title())));
        let selected = kind
            .and_then(|kind| BottomSheetDebugRegistrationKind::ALL_CASES.iter().position(|item| *item == kind))
            .map_or(0, |index| index + 1);
        let binding = self.kind.clone();
        SegmentedPicker::new("sai-bottom-sheet-gallery-kind", titles, selected, move |index, window, cx| {
            let value = index
                .checked_sub(1)
                .and_then(|index| BottomSheetDebugRegistrationKind::ALL_CASES.get(index).copied());
            binding.set(value, window, cx);
        })
    }

    fn module_cell(&self, item: BottomSheetDebugRegistrationModule) -> impl IntoElement + use<> {
        let theme = &self.theme;
        let binding = self.module.clone();
        let count = BottomSheetDebugCatalog::counts().get(&item).copied().unwrap_or(0);
        plain(ElementId::Name(SharedString::from(format!("sai-bottom-sheet-module-{}", item.id()))))
            .flex_1()
            .min_w(px(0.))
            .on_press(move |window, cx| binding.set(Some(item), window, cx))
            .child(
                hstack(10.)
                    .w_full()
                    .min_h(px(76.))
                    .p(px(14.))
                    .bg(theme.bg._100)
                    .clip_shape(Shape::continuous(18.))
                    .children(IconUi::sized(item.icon(), px(20.), theme.primary._100, false))
                    .child(
                        vstack(4.)
                            .align(HorizontalAlignment::Leading)
                            .child(
                                div()
                                    .child(item.title())
                                    .swiftui_font(Font::subheadline().weight(FontWeight::SEMIBOLD)),
                            )
                            .child(
                                div()
                                    .child(count.to_string())
                                    .swiftui_font(Font::caption())
                                    .text_color(theme.text.secondary),
                            ),
                    )
                    .child(spacer_min_w(0.)),
            )
    }

    fn module_grid(&self) -> Div {
        let modules = BottomSheetDebugCatalog::modules();
        let rows: Vec<Div> = modules
            .chunks(2)
            .map(|pair| {
                let mut row = hstack(10.).w_full();
                for item in pair {
                    row = row.child(self.module_cell(*item));
                }
                if pair.len() == 1 {
                    row = row.child(div().flex_1().min_w(px(0.)));
                }
                row
            })
            .collect();
        vstack(10.).w_full().px(px(12.)).pb(px(24.)).children(rows)
    }

    fn back_button(&self) -> impl IntoElement + use<> {
        let theme = &self.theme;
        let module = self.module.clone();
        let query = self.query.clone();
        let kind = self.kind.clone();
        plain("sai-bottom-sheet-gallery-back")
            .on_press(move |window, cx| {
                module.set(None, window, cx);
                query.set(String::new(), window, cx);
                kind.set(None, window, cx);
            })
            .child(
                hstack(6.)
                    .children(IconUi::sized("chevron.left", px(17.), theme.text.primary, false))
                    .child(div().child(tr("back"))),
            )
    }

    fn entry_row(&self, entry: &BottomSheetDebugRegistration) -> impl IntoElement + use<> {
        let theme = &self.theme;
        let on_select = self.on_select.clone();
        let selected = entry.clone();
        plain(ElementId::Name(SharedString::from(format!("sai-bottom-sheet-entry-{}", entry.id))))
            .w_full()
            .on_press(move |window, cx| on_select(&selected, window, cx))
            .child(
                hstack(12.)
                    .w_full()
                    .p(px(14.))
                    .bg(theme.bg._100)
                    .clip_shape(Shape::continuous(16.))
                    .child(
                        div().flex().justify_center().w(px(24.)).children(IconUi::sized(
                            entry.module.icon(),
                            px(17.),
                            theme.primary._100,
                            false,
                        )),
                    )
                    .child(
                        vstack(5.)
                            .align(HorizontalAlignment::Leading)
                            .min_w(px(0.))
                            .child(
                                div()
                                    .child(entry.title())
                                    .swiftui_font(Font::subheadline().weight(FontWeight::SEMIBOLD)),
                            )
                            .children(entry.variant().map(|variant| {
                                div()
                                    .child(variant)
                                    .swiftui_font(Font::caption())
                                    .text_color(theme.text.secondary)
                            }))
                            .child(
                                hstack(8.)
                                    .child(SaiBottomSheetModeBadge { mode: entry.mode })
                                    .child(
                                        div()
                                            .child(entry.kind.title())
                                            .swiftui_font(Font::caption2())
                                            .text_color(theme.text.secondary),
                                    ),
                            ),
                    )
                    .child(spacer_min_w(4.))
                    .children(IconUi::sized("arrow.up.forward.square", px(17.), theme.text.secondary, false)),
            )
    }

    fn row_element(&self, row: &SaiBottomSheetGalleryRow) -> AnyElement {
        match row {
            SaiBottomSheetGalleryRow::Back => self.back_button().into_any_element(),
            SaiBottomSheetGalleryRow::ModuleTitle(module) => div()
                .child(module.title())
                .swiftui_font(Font::title3().weight(FontWeight::BOLD))
                .into_any_element(),
            SaiBottomSheetGalleryRow::Entry(entry) => self.entry_row(entry).into_any_element(),
            SaiBottomSheetGalleryRow::NoResults => div()
                .child(tr("chat_search_no_results"))
                .text_color(self.theme.text.secondary)
                .into_any_element(),
        }
    }
}

impl RenderOnce for SaiBottomSheetGalleryPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let module = self.module.get(cx);
        let query = self.query.get(cx);
        let kind = self.kind.get(cx);
        let search_field = self.search_field(&query);
        let kind_picker = self.kind_picker(kind);
        let theme = self.theme.clone();

        let content: AnyElement = if module.is_none() && query.is_empty() && kind.is_none() {
            div()
                .id("sai-bottom-sheet-gallery-modules")
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .overflow_y_scroll()
                .child(self.module_grid())
                .into_any_element()
        } else {
            let entries = Self::entries(module, kind, &query);
            let mut rows = vec![SaiBottomSheetGalleryRow::Back];
            rows.extend(module.map(SaiBottomSheetGalleryRow::ModuleTitle));
            if entries.is_empty() {
                rows.push(SaiBottomSheetGalleryRow::NoResults);
            }
            rows.extend(entries.into_iter().map(SaiBottomSheetGalleryRow::Entry));
            let rows = Rc::new(rows);

            let list_theme = theme.clone();
            let list_state = window.use_keyed_state("sai-bottom-sheet-gallery-list", cx, move |_, _| {
                SaiBottomSheetGalleryListState {
                    list: ListUi::new(&list_theme, 0, ListAlignment::Top),
                    filter: None,
                }
            });
            let filter: GalleryFilter = (module, kind, query.clone());
            let count = rows.len();
            list_state.update(cx, |state, _| {
                if state.filter.as_ref() != Some(&filter) || state.list.item_count() != count {
                    state.list.replaced(count);
                    state.filter = Some(filter);
                }
            });

            let panel = Rc::new(self);
            let list = list_state.read(cx).list.element(move |index, _window, _cx| {
                let Some(row) = rows.get(index) else {
                    return div().into_any_element();
                };
                let last = index + 1 == rows.len();
                div()
                    .w_full()
                    .px(px(12.))
                    .pt(px(if index == 0 { 0. } else { 12. }))
                    .pb(px(if last { 24. } else { 0. }))
                    .child(panel.row_element(row))
                    .into_any_element()
            });
            div().flex_1().min_h(px(0.)).w_full().child(list).into_any_element()
        };

        vstack(12.)
            .size_full()
            .pt(px(12.))
            .text_color(theme.text.primary)
            .child(div().w_full().px(px(12.)).child(search_field))
            .child(
                div()
                    .id("sai-bottom-sheet-gallery-kind-label")
                    .w_full()
                    .px(px(12.))
                    .tooltip_label(tr("debug_surface_kind"))
                    .child(kind_picker),
            )
            .child(content)
    }
}

#[derive(IntoElement)]
struct SaiBottomSheetModeBadge {
    mode: BottomSheetDebugRegistrationMode,
}

impl RenderOnce for SaiBottomSheetModeBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let primary = ThemeStore::shared(cx).read(cx).current_theme().primary._100;
        let tint: Hsla =
            if self.mode == BottomSheetDebugRegistrationMode::Mock { primary } else { rgb(SYSTEM_ORANGE).into() };
        div()
            .child(self.mode.title())
            .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
            .px(px(8.))
            .py(px(4.))
            .text_color(tint)
            .bg(tint.opacity(0.12))
            .capsule()
    }
}

/// keeps the selected factory alive for its dismissal animation, no intermediate screen
#[derive(IntoElement)]
pub struct SaiBottomSheetPreview {
    entry: BottomSheetDebugRegistration,
}

impl SaiBottomSheetPreview {
    pub fn new(entry: BottomSheetDebugRegistration) -> Self {
        Self { entry }
    }
}

impl RenderOnce for SaiBottomSheetPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let background = theme_store.read(cx).current_theme().bg._200;
        let text = theme_store.read(cx).current_theme().text.primary;
        let entry_id = self.entry.id.clone();
        let is_presented = window.use_keyed_state(
            ElementId::Name(SharedString::from(format!("sai-bottom-sheet-preview-{entry_id}"))),
            cx,
            |_, _| true,
        );
        let showing = *is_presented.read(cx);
        let binding = Binding::entity(
            is_presented,
            |is_presented: &bool| *is_presented,
            |is_presented: &mut bool, value, _| *is_presented = value,
        );
        let close_binding = binding.clone();
        let view = (self.entry.make_view)(binding, window, cx);

        let escape = (self.entry.kind == BottomSheetDebugRegistrationKind::Modal && showing).then(|| {
            div()
                .id(ElementId::Name(SharedString::from(format!("sai-bottom-sheet-preview-close-tip-{entry_id}"))))
                .absolute()
                .top(px(16.))
                .right(px(16.))
                .tooltip_label(tr("close"))
                .child(
                    plain(ElementId::Name(SharedString::from(format!("sai-bottom-sheet-preview-close-{entry_id}"))))
                        .on_press(move |window, cx| close_binding.set(false, window, cx))
                        .child(
                            div()
                                .p(px(12.))
                                .bg(background)
                                .rounded_full()
                                .children(IconUi::sized("xmark", px(12.), text, false)),
                        ),
                )
        });

        div().relative().size_full().child(view).children(escape)
    }
}

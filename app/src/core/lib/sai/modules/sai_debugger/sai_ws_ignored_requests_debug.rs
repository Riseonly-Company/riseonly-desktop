#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{
    App, Context, Entity, FontWeight, Hsla, ListAlignment, ListState, PromptButton, PromptLevel,
    SharedString, Window, list, rgb,
};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::character_count;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::swiftui::prelude::*;

use super::sai_debug_storage_panel_views::{SaiDebugBodyFormat, SaiDebugExpandableMonospaceBlock};

const ROW_DATE_FORMAT: &str = "yyyy-MM-dd HH:mm:ss.SSS";
const COPY_ALL_SEPARATOR: &str = "\n\n\u{2014} \u{2014} \u{2014}\n\n";
const MAX_ROWS: usize = 200;
const LINK_BLUE: u32 = 0x29b6f6;
const DESTRUCTIVE_RED: u32 = 0xe11d48;
const DEEP_COMPARE_TINT: u32 = 0x7c4dff;
const COALESCING_TINT: u32 = 0x00a896;
const CAPTION_SIZE: f32 = 12.;
const CAPTION2_SIZE: f32 = 11.;
const ROW_ICON_SIDE: f32 = 13.;
const LIST_MAX_HEIGHT: f32 = 520.;
const LIST_OVERDRAW: f32 = 200.;

thread_local! {
    static ROW_FORMATTER: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_date_format(ROW_DATE_FORMAT);
        formatter
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiWsIgnoredRequestKind {
    DeepCompareBody,
    Coalescing,
}

impl SaiWsIgnoredRequestKind {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::DeepCompareBody => "deepCompareBody",
            Self::Coalescing => "coalescing",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsIgnoredRequestRow {
    pub id: Uuid,
    pub at: Date,
    pub kind: SaiWsIgnoredRequestKind,
    pub service: String,
    pub method: String,
    pub body_json: String,
    pub scope_or_key: String,
    pub detail: String,
}

impl SaiWsIgnoredRequestRow {
    pub fn copy_block_text(&self) -> String {
        let at = ROW_FORMATTER.with(|formatter| formatter.string_from(&self.at));
        format!(
            "{} \u{00b7} {}.{}\nat: {}\nscope/key: {}\n{}\n\n{}",
            self.kind.raw_value(),
            self.service,
            self.method,
            at,
            self.scope_or_key,
            self.detail,
            self.body_json,
        )
    }
}

/// ignored requests store, an entity where every published write notifies observers
pub struct SaiWsIgnoredRequestsStore {
    rows: Vec<SaiWsIgnoredRequestRow>,
    max_rows: usize,
}

impl SaiWsIgnoredRequestsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            rows: Vec::new(),
            max_rows: MAX_ROWS,
        }
    }

    pub fn rows(&self) -> &[SaiWsIgnoredRequestRow] {
        &self.rows
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        kind: SaiWsIgnoredRequestKind,
        service: &str,
        method: &str,
        body_json: &str,
        scope_or_key: &str,
        detail: &str,
        cx: &mut Context<Self>,
    ) {
        let row = SaiWsIgnoredRequestRow {
            id: Uuid::new_v4(),
            at: RiseClock::now(),
            kind,
            service: service.to_owned(),
            method: method.to_owned(),
            body_json: body_json.to_owned(),
            scope_or_key: scope_or_key.to_owned(),
            detail: detail.to_owned(),
        };
        self.rows.insert(0, row);
        self.rows.truncate(self.max_rows);
        cx.notify();
    }

    pub fn remove(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.rows.clear();
        cx.notify();
    }

    pub fn copy_all_text(&self) -> String {
        self.rows
            .iter()
            .map(SaiWsIgnoredRequestRow::copy_block_text)
            .collect::<Vec<_>>()
            .join(COPY_ALL_SEPARATOR)
    }
}

/// records ignored requests in debug or debug tools builds only so a release build records nothing
pub enum SaiWsIgnoredRequestsDebugRecorder {}

impl SaiWsIgnoredRequestsDebugRecorder {
    pub fn record_deep_compare_skip(
        service: &str,
        method: &str,
        body_json: &str,
        scope_key: &str,
        detail: &str,
        cx: &mut App,
    ) {
        Self::record(SaiWsIgnoredRequestKind::DeepCompareBody, service, method, body_json, scope_key, detail, cx);
    }

    pub fn record_coalescing_skip(
        service: &str,
        method: &str,
        body_json: &str,
        coalescing_key: &str,
        detail: &str,
        cx: &mut App,
    ) {
        Self::record(SaiWsIgnoredRequestKind::Coalescing, service, method, body_json, coalescing_key, detail, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        kind: SaiWsIgnoredRequestKind,
        service: &str,
        method: &str,
        body_json: &str,
        scope_or_key: &str,
        detail: &str,
        cx: &mut App,
    ) {
        SaiWsIgnoredRequestsStore::shared(cx).update(cx, |store, cx| {
            store.record(kind, service, method, body_json, scope_or_key, detail, cx);
        });
    }
}

/// ignored requests panel, the two confirm dialogs are the system prompt of the window
pub struct SaiWsIgnoredRequestsPanel {
    theme: AppTheme,
    store: Entity<SaiWsIgnoredRequestsStore>,
    delete_id: Option<Uuid>,
    clear_all_confirm: bool,
    list_state: ListState,
}

impl SaiWsIgnoredRequestsPanel {
    pub fn new(theme: AppTheme, cx: &mut Context<Self>) -> Self {
        let store = SaiWsIgnoredRequestsStore::shared(cx);
        let list_state = ListState::new(store.read(cx).rows().len(), ListAlignment::Top, px(LIST_OVERDRAW));
        cx.observe(&store, |panel, store, cx| {
            panel.list_state.reset(store.read(cx).rows().len());
            cx.notify();
        })
        .detach();
        Self {
            theme,
            store,
            delete_id: None,
            clear_all_confirm: false,
            list_state,
        }
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn confirm_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_all_confirm = true;
        cx.notify();
        let answer = window.prompt(
            PromptLevel::Warning,
            "Clear all ignored rows?",
            Some("Removes deepCompare / coalescing skip history."),
            &[PromptButton::Ok("Clear all".into()), PromptButton::Cancel("Cancel".into())],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let choice = answer.await;
            this.update(cx, |panel, cx| {
                if choice == Ok(0) {
                    panel.store.update(cx, |store, cx| store.clear(cx));
                }
                panel.clear_all_confirm = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn confirm_delete(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_id = Some(id);
        cx.notify();
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete this row?",
            Some("Removes one entry from the list."),
            &[PromptButton::Ok("Delete".into()), PromptButton::Cancel("Cancel".into())],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let choice = answer.await;
            this.update(cx, |panel, cx| {
                if choice == Ok(0)
                    && let Some(id) = panel.delete_id
                {
                    panel.store.update(cx, |store, cx| store.remove(id, cx));
                }
                panel.delete_id = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        let theme = &self.theme;
        let rows = self.store.read(cx).rows();
        let count = rows.len();
        let has_rows = !rows.is_empty();
        let store = self.store.clone();
        let this = cx.weak_entity();

        let actions = has_rows.then(|| {
            hstack(None)
                .child(
                    plain("sai-ws-ignored-copy-all")
                        .on_press(move |_, cx| {
                            let text = store.read(cx).copy_all_text();
                            UIPasteboard::general().set_string(&text, cx);
                        })
                        .child(
                            div()
                                .child("Copy all")
                                .text_size(px(CAPTION_SIZE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(LINK_BLUE)),
                        ),
                )
                .child(
                    plain("sai-ws-ignored-clear")
                        .on_press(move |window, cx| {
                            this.update(cx, |panel, cx| panel.confirm_clear_all(window, cx)).ok();
                        })
                        .child(
                            div()
                                .child("Clear")
                                .text_size(px(CAPTION_SIZE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(DESTRUCTIVE_RED)),
                        ),
                )
        });

        hstack(None)
            .w_full()
            .child(
                div()
                    .child("Ignored WS")
                    .text_size(px(CAPTION_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text.secondary),
            )
            .child(
                div()
                    .child(format!("({count})"))
                    .text_size(px(CAPTION2_SIZE))
                    .text_color(theme.text.secondary.opacity(0.75)),
            )
            .child(spacer_min_w(0.))
            .children(actions)
            .px(px(14.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.35))
    }

    fn ignored_row(
        row: &SaiWsIgnoredRequestRow,
        theme: &AppTheme,
        panel: &gpui::WeakEntity<Self>,
    ) -> AnyElement {
        let id_suffix = row.id.uuid_string();
        let copy_text = row.copy_block_text();
        let row_id = row.id;
        let panel = panel.clone();
        let body_json: Rc<str> = Rc::from(row.body_json.as_str());

        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                hstack(8.)
                    .align(VerticalAlignment::FirstTextBaseline)
                    .w_full()
                    .child(
                        div()
                            .child(row.kind.raw_value())
                            .text_size(px(CAPTION2_SIZE))
                            .font_weight(FontWeight::BOLD)
                            .text_color(Hsla::white().opacity(0.95))
                            .px(px(6.))
                            .py(px(2.))
                            .bg(kind_tint(row.kind).opacity(0.85))
                            .clip_shape(Shape::Capsule),
                    )
                    .child(
                        div()
                            .child(format!("{}.{}", row.service, row.method))
                            .text_size(px(CAPTION_SIZE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text.primary),
                    )
                    .child(spacer_min_w(0.))
                    .child(
                        plain(SharedString::from(format!("sai-ws-ignored-copy-{id_suffix}")))
                            .on_press(move |_, cx| UIPasteboard::general().set_string(&copy_text, cx))
                            .children(IconUi::sized("doc.on.doc", px(ROW_ICON_SIDE), theme.text.primary, false)),
                    )
                    .child(
                        plain(SharedString::from(format!("sai-ws-ignored-delete-{id_suffix}")))
                            .on_press(move |window, cx| {
                                panel.update(cx, |panel, cx| panel.confirm_delete(row_id, window, cx)).ok();
                            })
                            .children(IconUi::sized("trash", px(ROW_ICON_SIDE), rgb(DESTRUCTIVE_RED).into(), false)),
                    ),
            )
            .child(
                div()
                    .child(format!("scope/key: {}", row.scope_or_key))
                    .text_size(px(CAPTION2_SIZE))
                    .text_color(theme.text.secondary),
            )
            .child(
                div()
                    .child(row.detail.clone())
                    .text_size(px(CAPTION2_SIZE))
                    .text_color(theme.text.secondary.opacity(0.9)),
            )
            .child(
                SaiDebugExpandableMonospaceBlock::new(
                    SharedString::from(format!("sai-ws-ignored-body-{id_suffix}")),
                    "BODY",
                    SaiDebugBodyFormat::size_label_char_count(
                        i64::try_from(character_count(&row.body_json)).unwrap_or(i64::MAX),
                    ),
                    theme.clone(),
                    Rc::new(move |_: &mut App| body_json.to_string()),
                )
                .background(theme.bg._100.opacity(0.45)),
            )
            .p(px(10.))
            .bg(theme.bg._200.opacity(0.55))
            .clip_shape(Shape::continuous(10.))
            .into_any_element()
    }
}

fn kind_tint(k: SaiWsIgnoredRequestKind) -> Hsla {
    match k {
        SaiWsIgnoredRequestKind::DeepCompareBody => rgb(DEEP_COMPARE_TINT).into(),
        SaiWsIgnoredRequestKind::Coalescing => rgb(COALESCING_TINT).into(),
    }
}

impl Render for SaiWsIgnoredRequestsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let header = self.header(cx);
        let is_empty = self.store.read(cx).rows().is_empty();

        let content: AnyElement = if is_empty {
            div()
                .child("No skipped outbound calls yet.")
                .text_size(px(CAPTION_SIZE))
                .text_color(theme.text.secondary)
                .w_full()
                .flex()
                .justify_center()
                .p(px(14.))
                .pt(px(14. + 28.))
                .into_any_element()
        } else {
            let store = self.store.downgrade();
            let panel = cx.weak_entity();
            let row_theme = theme.clone();
            list(self.list_state.clone(), move |index, _window, cx| {
                let Some(store) = store.upgrade() else {
                    return div().into_any_element();
                };
                let Some(row) = store.read(cx).rows().get(index).cloned() else {
                    return div().into_any_element();
                };
                div()
                    .px(px(14.))
                    .pt(px(if index == 0 { 14. } else { 12. }))
                    .child(Self::ignored_row(&row, &row_theme, &panel))
                    .into_any_element()
            })
            .size_full()
            .into_any_element()
        };

        vstack(0.)
            .w_full()
            .child(header)
            .child(
                div()
                    .child("deepCompare (body only) \u{00b7} coalescing (e.g. toggle parity)")
                    .text_size(px(CAPTION2_SIZE))
                    .text_color(theme.text.secondary.opacity(0.82))
                    .w_full()
                    .px(px(14.))
                    .py(px(6.)),
            )
            .child(
                div()
                    .w_full()
                    .h(px(LIST_MAX_HEIGHT))
                    .bg(theme.bg._100)
                    .child(content),
            )
    }
}

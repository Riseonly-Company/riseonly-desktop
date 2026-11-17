#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::cmp::Ordering;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, ListAlignment, SharedString, Window, rgb, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, content_height_motion};

use super::sai_api_doc_analyzer::SaiApiDocAnalyzer;
use super::sai_api_doc_types::{SaiApiEndpointDoc, SaiApiFieldInfo, SaiApiFieldMap, SaiApiVariant};
use super::sai_api_docs_store::SaiApiDocsStore;
use super::sai_debug_storage_panel_views::{
    SaiDebugBodyFormat, SaiDebugExpandableMonospaceBlock, SaiDebugInspectorMetrics,
};
use super::sai_http_trace_store::SaiHttpTraceStore;
use super::sai_requester_store::SaiRequesterStore;

const DESTRUCTIVE_RED: u32 = 0xe11d48;
const LINK_BLUE: u32 = 0x29b6f6;
const WS_GREEN: u32 = 0x00db0f;
const OPTIONAL_AMBER: u32 = 0xffb020;
const TYPE_STRING: u32 = 0x4caf50;
const TYPE_BOOLEAN: u32 = 0xff9800;
const TYPE_NULL: u32 = 0x9e9e9e;
const TYPE_ARRAY: u32 = 0xce93d8;
const TYPE_OBJECT: u32 = 0x80cbc4;
const TYPE_UNKNOWN: u32 = 0xb0bec5;

pub struct SaiApiDocsPanel {
    theme: AppTheme,
    docs_store: Entity<SaiApiDocsStore>,
    http_trace: Entity<SaiHttpTraceStore>,
    list: ListUi,
    sorted: Rc<Vec<SaiApiEndpointDoc>>,
    entries_count: usize,
}

impl SaiApiDocsPanel {
    pub fn new(theme: AppTheme, cx: &mut Context<Self>) -> Self {
        let docs_store = SaiApiDocsStore::shared(cx);
        let http_trace = SaiHttpTraceStore::shared(cx);
        cx.observe(&docs_store, |panel, _, cx| {
            panel.refresh_sorted(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&http_trace, |panel, http_trace, cx| {
            if http_trace.read(cx).entries().len() != panel.entries_count {
                panel.process_entries(cx);
            }
        })
        .detach();
        let list = ListUi::new(&theme, 0, ListAlignment::Top);
        let mut panel = Self {
            theme,
            docs_store,
            http_trace,
            list,
            sorted: Rc::new(Vec::new()),
            entries_count: 0,
        };
        panel.refresh_sorted(cx);
        panel
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn process_entries(&mut self, cx: &mut Context<Self>) {
        let entries = self.http_trace.read(cx).entries().to_vec();
        self.entries_count = entries.len();
        self.docs_store.update(cx, |store, cx| store.process_entries(&entries, cx));
    }

    fn refresh_sorted(&mut self, cx: &mut Context<Self>) {
        let mut sorted: Vec<SaiApiEndpointDoc> =
            self.docs_store.read(cx).library().endpoints.values().cloned().collect();
        sorted.sort_by(|a, b| b.last_seen_ms.partial_cmp(&a.last_seen_ms).unwrap_or(Ordering::Equal));
        let old_count = self.list.item_count();
        if sorted.len() > old_count {
            self.list.appended(sorted.len() - old_count);
        } else {
            self.list.removed(sorted.len()..old_count);
        }
        self.sorted = Rc::new(sorted);
    }

    fn header_bar(&self) -> Div {
        let theme = &self.theme;
        let docs_store = self.docs_store.clone();
        let count = self.sorted.len();
        hstack(8.)
            .w_full()
            .child(
                div()
                    .child("API Docs")
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary),
            )
            .child(
                div()
                    .child(format!("({count})"))
                    .swiftui_font(Font::caption2().monospaced_digit())
                    .text_color(theme.text.secondary.opacity(0.75)),
            )
            .child(spacer_min_w(0.))
            .children((count > 0).then(|| {
                plain("sai-api-docs-clear-all")
                    .on_press(move |_window, cx| {
                        docs_store.update(cx, |store, cx| store.clear_all(cx));
                    })
                    .child(
                        div()
                            .child("Clear all")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(DESTRUCTIVE_RED)),
                    )
            }))
            .px(px(14.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.35))
    }

    fn endpoint_list(&self) -> Div {
        let theme = self.theme.clone();
        let content: AnyElement = if self.sorted.is_empty() {
            div()
                .w_full()
                .flex()
                .justify_center()
                .px(px(14.))
                .pt(px(14. + 36.))
                .child(
                    div()
                        .child("No requests captured yet.\nMake some API calls and they will appear here.")
                        .swiftui_font(Font::caption())
                        .text_color(theme.text.secondary)
                        .text_center(),
                )
                .into_any_element()
        } else {
            let sorted = self.sorted.clone();
            let row_theme = theme.clone();
            self.list
                .element(move |index, _window, _cx| {
                    let Some(doc) = sorted.get(index) else {
                        return div().into_any_element();
                    };
                    div()
                        .w_full()
                        .px(px(14.))
                        .pt(px(if index == 0 { 14. } else { 10. }))
                        .pb(px(if index + 1 == sorted.len() { 14. } else { 0. }))
                        .child(SaiApiEndpointCard { doc: doc.clone(), theme: row_theme.clone() })
                        .into_any_element()
                })
                .into_any_element()
        };
        div().flex_1().min_h(px(0.)).w_full().bg(theme.bg._100).child(content)
    }
}

impl Render for SaiApiDocsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.weak_entity();
        vstack(0.)
            .size_full()
            .child(self.header_bar())
            .child(self.endpoint_list())
            .on_appear("sai-api-docs-panel-appear", move |_window, cx| {
                let _ = panel.update(cx, |panel, cx| panel.process_entries(cx));
            })
    }
}

fn sub_id(id: &ElementId, part: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), part.into())
}

/// disclosure state of a card plus the branch with the default opacity transition
#[derive(Clone, Debug)]
struct SaiApiDisclosureState {
    shown: bool,
    presence: Presence<()>,
}

impl SaiApiDisclosureState {
    fn new() -> Self {
        Self { shown: false, presence: Presence::new(None) }
    }

    fn toggle(state: &Entity<Self>, duration: f64, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.shown = !state.shown;
            let target = state.shown.then_some(());
            state.presence.set_in(target, ease_in_out(duration), RiseClock::monotonic(), cx);
            cx.notify();
        });
    }
}

fn type_pills(info: &SaiApiFieldInfo) -> Div {
    let pills: Vec<Div> = info
        .types
        .iter()
        .map(|type_name| {
            let color = SaiApiEndpointCard::type_color(type_name);
            div()
                .child(type_name.clone())
                .swiftui_font(Font::system(9., FontWeight::SEMIBOLD, FontDesign::Default))
                .text_color(color)
                .px(px(5.))
                .py(px(2.))
                .bg(color.opacity(0.12))
                .capsule()
        })
        .collect();
    hstack(3.).children(pills).children(info.is_optional.then(|| {
        div()
            .child("?")
            .swiftui_font(Font::system(9., FontWeight::BOLD, FontDesign::Default))
            .text_color(rgb(OPTIONAL_AMBER))
    }))
}

fn schema_rows(schema: &SaiApiFieldMap, theme: &AppTheme, background_opacity: f32) -> Div {
    let mut keys: Vec<&String> = schema.keys().collect();
    keys.sort();
    let rows: Vec<Div> = keys
        .into_iter()
        .filter_map(|key| {
            let info = schema.get(key)?;
            Some(
                hstack(6.)
                    .align(VerticalAlignment::FirstTextBaseline)
                    .w_full()
                    .child(
                        div()
                            .min_w(px(0.))
                            .child(key.clone())
                            .swiftui_font(Font::system(10., FontWeight::MEDIUM, FontDesign::Monospaced))
                            .text_color(theme.text.primary),
                    )
                    .child(spacer_min_w(0.))
                    .child(type_pills(info))
                    .px(px(8.))
                    .py(px(3.))
                    .bg(theme.bg._100.opacity(background_opacity))
                    .clip_shape(Shape::continuous(5.)),
            )
        })
        .collect();
    vstack(3.).align(HorizontalAlignment::Leading).w_full().children(rows)
}

#[derive(IntoElement)]
struct SaiApiEndpointCard {
    doc: SaiApiEndpointDoc,
    theme: AppTheme,
}

impl SaiApiEndpointCard {
    fn element_id(&self) -> ElementId {
        ElementId::Name(SharedString::from(format!("sai-api-endpoint-{}", self.doc.id)))
    }

    fn transport_badge(&self) -> Div {
        let fill: Hsla = if self.doc.transport == "WS" { rgb(WS_GREEN).into() } else { rgb(LINK_BLUE).into() };
        div()
            .child(self.doc.transport.clone())
            .swiftui_font(Font::system(9., FontWeight::EXTRA_BOLD, FontDesign::Default))
            .text_color(white().opacity(0.92))
            .px(px(6.))
            .py(px(3.))
            .bg(fill.opacity(0.7))
            .capsule()
    }

    fn merge_all_request_schemas(doc: &SaiApiEndpointDoc) -> SaiApiFieldMap {
        doc.variants.iter().fold(SaiApiFieldMap::new(), |partial, variant| {
            SaiApiDocAnalyzer::merge_schema(&partial, &variant.request_body_schema)
        })
    }

    fn section_title(title: &'static str, theme: &AppTheme) -> Div {
        div()
            .child(title)
            .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Default))
            .text_color(theme.text.secondary.opacity(0.7))
            .tracking(0.5)
    }

    fn schema_section(title: &'static str, schema: &SaiApiFieldMap, theme: &AppTheme) -> Div {
        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(Self::section_title(title, theme))
            .child(Self::schema_table(schema, theme))
    }

    fn schema_table(schema: &SaiApiFieldMap, theme: &AppTheme) -> Div {
        schema_rows(schema, theme, 0.45)
    }

    fn type_color(type_name: &str) -> Hsla {
        let color = match type_name {
            "string" => TYPE_STRING,
            "number" => LINK_BLUE,
            "boolean" => TYPE_BOOLEAN,
            "null" => TYPE_NULL,
            "array" | "array-item" => TYPE_ARRAY,
            "object" => TYPE_OBJECT,
            _ => TYPE_UNKNOWN,
        };
        rgb(color).into()
    }

    fn play_button(&self, id: ElementId) -> impl IntoElement + use<> {
        let doc = self.doc.clone();
        let tint: Hsla = rgb(LINK_BLUE).into();
        div().occlude().child(
            plain(id)
                .on_press(move |_window, cx| {
                    let Some(variant) = doc.variants.iter().rev().max_by_key(|variant| variant.seen_count) else {
                        return;
                    };
                    SaiRequesterStore::shared(cx).update(cx, |requester, cx| {
                        requester.load_from_variant(variant, &doc, cx);
                    });
                })
                .child(
                    div()
                        .p(px(5.))
                        .bg(tint.opacity(0.12))
                        .clip_shape(Shape::continuous(5.))
                        .children(IconUi::sized("play.fill", px(10.), tint, true)),
                ),
        )
    }
}

impl RenderOnce for SaiApiEndpointCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let id = self.element_id();
        let state = window.use_keyed_state(sub_id(&id, "state"), cx, |_, _| SaiApiDisclosureState::new());
        let current = state.read(cx).clone();
        let count = self.doc.variants.len();
        let variants_label = format!("{count} variant{}", if count == 1 { "" } else { "s" });
        let chevron = if current.shown { "chevron.up" } else { "chevron.down" };

        let toggle = plain(sub_id(&id, "toggle"))
            .w_full()
            .on_press(move |_window, cx| SaiApiDisclosureState::toggle(&state, 0.18, cx))
            .child(
                hstack(8.)
                    .w_full()
                    .px(px(10.))
                    .py(px(9.))
                    .child(self.transport_badge())
                    .child(
                        div()
                            .min_w(px(0.))
                            .child(self.doc.id.clone())
                            .swiftui_font(Font::system(11., FontWeight::SEMIBOLD, FontDesign::Monospaced))
                            .text_color(theme.text.primary)
                            .line_clamp(2),
                    )
                    .child(spacer_min_w(0.))
                    .child(
                        div()
                            .child(variants_label)
                            .swiftui_font(Font::caption2())
                            .text_color(theme.text.secondary.opacity(0.75)),
                    )
                    .child(self.play_button(sub_id(&id, "play")))
                    .children(IconUi::sized(chevron, px(10.), theme.text.secondary, false)),
            );

        let details = current.presence.render_reveal(RiseClock::monotonic(), window, |_| {
            let merged_request = Self::merge_all_request_schemas(&self.doc);
            let mut variants: Vec<&SaiApiVariant> = self.doc.variants.iter().collect();
            variants.sort_by_key(|variant| std::cmp::Reverse(variant.seen_count));
            let cards: Vec<SaiApiVariantCard> = variants
                .into_iter()
                .map(|variant| SaiApiVariantCard {
                    endpoint_id: self.doc.id.clone(),
                    variant: variant.clone(),
                    theme: theme.clone(),
                })
                .collect();
            vstack(0.)
                .w_full()
                .child(div().w_full().h(px(1.)).bg(theme.border._200))
                .child(
                    vstack(12.)
                        .align(HorizontalAlignment::Leading)
                        .w_full()
                        .p(px(10.))
                        .children((!merged_request.is_empty()).then(|| {
                            Self::schema_section("REQUEST BODY", &merged_request, &theme)
                        }))
                        .child(Self::section_title("RESPONSE VARIANTS", &theme))
                        .children(cards),
                )
        });

        div()
            .relative()
            .w_full()
            .child(
                vstack(0.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .bg(theme.bg._200.opacity(0.92))
                    .clip_shape(Shape::continuous(10.))
                    .child(toggle)
                    .child(content_height_motion(
                        sub_id(&id, "details-height"),
                        current.shown,
                        0.,
                        ease_in_out(0.18),
                        details,
                        window,
                        cx,
                    )),
            )
            .child(centred_stroke(theme.border._200, 1., Shape::continuous(10.)))
    }
}

#[derive(IntoElement)]
struct SaiApiVariantCard {
    endpoint_id: String,
    variant: SaiApiVariant,
    theme: AppTheme,
}

impl SaiApiVariantCard {
    fn status_color(&self) -> Hsla {
        let code = self.variant.status_code;
        let color = if code == 0 {
            TYPE_NULL
        } else if code < 300 {
            TYPE_STRING
        } else if code < 500 {
            OPTIONAL_AMBER
        } else {
            DESTRUCTIVE_RED
        };
        rgb(color).into()
    }
}

impl RenderOnce for SaiApiVariantCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let status_color = self.status_color();
        let code = self.variant.status_code;
        let id = ElementId::Name(SharedString::from(format!(
            "sai-api-variant-{}-{}",
            self.endpoint_id, self.variant.id
        )));
        let state = window.use_keyed_state(sub_id(&id, "state"), cx, |_, _| SaiApiDisclosureState::new());
        let current = state.read(cx).clone();

        let example = current.presence.render_reveal(RiseClock::monotonic(), window, |_| {
            let full = self.variant.response_example.clone();
            div().w_full().child(SaiDebugExpandableMonospaceBlock::new(
                sub_id(&id, "example"),
                "EXAMPLE",
                SaiDebugBodyFormat::one_line_preview(
                    &self.variant.response_example,
                    SaiDebugInspectorMetrics::COLLAPSED_PREVIEW_MAX_CHARS,
                ),
                theme.clone(),
                Rc::new(move |_cx: &mut App| full.clone()),
            ))
        });
        let has_example = !self.variant.response_example.is_empty();

        div()
            .relative()
            .w_full()
            .child(
                vstack(6.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .p(px(8.))
                    .bg(theme.bg._300.opacity(0.35))
                    .clip_shape(Shape::continuous(8.))
                    .child(
                        hstack(8.)
                            .w_full()
                            .child(
                                div()
                                    .child(if code > 0 { code.to_string() } else { "ERR".to_owned() })
                                    .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Monospaced))
                                    .text_color(status_color)
                                    .px(px(6.))
                                    .py(px(2.))
                                    .bg(status_color.opacity(0.12))
                                    .capsule(),
                            )
                            .child(
                                div()
                                    .child(format!("{}\u{00d7}", self.variant.seen_count))
                                    .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                                    .text_color(theme.text.secondary),
                            )
                            .child(spacer_min_w(0.))
                            .child(
                                plain(sub_id(&id, "toggle"))
                                    .on_press({
                                        let state = state.clone();
                                        move |_window, cx| SaiApiDisclosureState::toggle(&state, 0.15, cx)
                                    })
                                    .child(
                                        div()
                                            .child(if current.shown { "Hide example" } else { "Show example" })
                                            .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                                            .text_color(rgb(LINK_BLUE)),
                                    ),
                            ),
                    )
                    .children((!self.variant.response_schema.is_empty()).then(|| {
                        schema_rows(&self.variant.response_schema, &theme, 0.35)
                    }))
                    .child(content_height_motion(
                        sub_id(&id, "example-height"),
                        current.shown,
                        6.,
                        ease_in_out(0.15),
                        example.filter(|_| has_example),
                        window,
                        cx,
                    )),
            )
            .child(centred_stroke(status_color.opacity(0.3), 1., Shape::continuous(8.)))
    }
}

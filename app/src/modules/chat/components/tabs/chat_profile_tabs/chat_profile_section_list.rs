use std::ops::RangeInclusive;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, Entity, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    ParentElement, Pixels, RenderOnce, ScrollHandle, Styled, Window, div, point, px,
};

use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfilePinnedTabsBottom, ChatProfileSectionHeader, chat_profile_page_scroll, chat_profile_pinned_tabs_bottom,
    chat_profile_uses_parent_scroll,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSection,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tabs::ChatProfileViewportLayoutPolicy;
use crate::modules::gifts::components::gift_shop_shelf_grid::{GiftScrollWindowProbe, GiftShopShelfRows};

pub(crate) type ChatProfileSectionRow = Rc<dyn Fn(&ChatProfileMediaEntry, &mut Window, &mut App) -> AnyElement>;

const SECTION_HEADER_HEIGHT: f32 = 35.0;

#[derive(Clone, Copy, Default)]
struct ChatProfileSectionListWindow {
    rows: Option<GiftShopShelfRows>,
    band_top: Option<f32>,
}

#[derive(IntoElement)]
pub(crate) struct ChatProfileSectionList {
    id: ElementId,
    sections: Vec<ChatProfileSection<ChatProfileMediaEntry>>,
    row_height: f32,
    row: ChatProfileSectionRow,
}

impl ChatProfileSectionList {
    /// row_height includes the separator
    pub(crate) fn new(
        id: impl Into<ElementId>,
        sections: Vec<ChatProfileSection<ChatProfileMediaEntry>>,
        row_height: f32,
        row: impl Fn(&ChatProfileMediaEntry, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self { id: id.into(), sections, row_height, row: Rc::new(row) }
    }
}

impl RenderOnce for ChatProfileSectionList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, sections, row_height, row } = self;
        let child_id = |name: &'static str| ElementId::from((id.clone(), name));
        let total_height: f32 =
            sections.iter().map(|section| SECTION_HEADER_HEIGHT + section.items.len() as f32 * row_height).sum();

        let row_count = (total_height / row_height).ceil() as i64;
        let initial_rows = (f32::from(window.viewport_size().height) / row_height).ceil() as i64 + 1;
        let state: Entity<ChatProfileSectionListWindow> =
            window.use_keyed_state(child_id("window"), cx, |_, _| ChatProfileSectionListWindow::default());
        let current = *state.read(cx);
        let rows = current.rows.unwrap_or(GiftShopShelfRows::leading(initial_rows)).clamped(row_count);
        let (mounted_top, mounted_bottom) = if rows.is_empty() {
            (0.0, 0.0)
        } else {
            (rows.first as f32 * row_height, (rows.last + 1) as f32 * row_height)
        };

        let top_inset = if chat_profile_uses_parent_scroll(cx) {
            ChatProfileViewportLayoutPolicy::page_content_top_inset()
        } else {
            0.0
        };
        let page_scroll = chat_profile_page_scroll(cx).filter(|_| top_inset > 0.0);
        let tabs_bottom = chat_profile_pinned_tabs_bottom(cx).filter(|_| top_inset > 0.0);
        let measures_pin_line = page_scroll.is_some() || tabs_bottom.is_some();
        let live_pin_line = measures_pin_line.then(|| ChatProfilePinLine { tabs_bottom, page_scroll, top_inset });
        let pin_line = current.band_top.map_or(0.0, |band_top| band_top + top_inset);

        let mut row_elements: Vec<AnyElement> = Vec::new();
        let mut header_elements: Vec<AnyElement> = Vec::new();
        let mut section_top = 0.0_f32;
        for section in &sections {
            let rows_top = section_top + SECTION_HEADER_HEIGHT;
            let section_bottom = rows_top + section.items.len() as f32 * row_height;

            let (header_y, shows_header) = if live_pin_line.is_some() {
                (section_top, section_top < mounted_bottom && section_bottom > mounted_top)
            } else {
                let is_pinned = section_top <= pin_line && pin_line < section_bottom;
                (
                    pinned_header_y(section_top, section_bottom, pin_line),
                    is_pinned || (section_top < mounted_bottom && rows_top > mounted_top),
                )
            };
            if shows_header {
                let header = div()
                    .absolute()
                    .left_0()
                    .w_full()
                    .top(px(header_y))
                    .h(px(SECTION_HEADER_HEIGHT))
                    .child(ChatProfileSectionHeader::new(section.title()));
                header_elements.push(match &live_pin_line {
                    Some(line) => ChatProfilePinnedHeader {
                        child: header.into_any_element(),
                        layout_y: header_y,
                        section_top,
                        section_bottom,
                        line: line.clone(),
                    }
                    .into_any_element(),
                    None => header.into_any_element(),
                });
            }

            if section_bottom > mounted_top && rows_top < mounted_bottom {
                let first = (((mounted_top - rows_top) / row_height).floor().max(0.0)) as usize;
                let last = (((mounted_bottom - rows_top) / row_height).ceil().max(0.0) as usize).min(section.items.len());
                for (offset, entry) in section.items[first.min(last)..last].iter().enumerate() {
                    let y = rows_top + (first + offset) as f32 * row_height;
                    let content = row(entry, window, cx);
                    row_elements.push(
                        div()
                            .absolute()
                            .left_0()
                            .w_full()
                            .top(px(y))
                            .h(px(row_height))
                            .child(content)
                            .into_any_element(),
                    );
                }
            }
            section_top = section_bottom;
        }

        let probe_state = state;
        let probe = GiftScrollWindowProbe::new(child_id("window-probe"), move |band, height, cx| {
            let Some(band) = band else { return };
            if height <= 0.0 {
                return;
            }
            probe_state.update(cx, |list_window, cx| {
                let rows = section_band_rows(
                    &band,
                    height,
                    row_count,
                    list_window.rows.unwrap_or(GiftShopShelfRows::leading(initial_rows)),
                );
                let band_top = *band.start();
                let moved_rows = Some(rows) != list_window.rows;
                let moved_pin = !measures_pin_line && Some(band_top) != list_window.band_top;
                list_window.rows = Some(rows);
                list_window.band_top = Some(band_top);
                if moved_rows || moved_pin {
                    cx.notify();
                }
            });
        });

        div()
            .relative()
            .w_full()
            .h(px(total_height))
            .child(div().absolute().inset_0().child(probe))
            .children(row_elements)
            .children(header_elements)
    }
}

const BUILD_MARGIN_ROWS: f32 = 2.0;
const REBUILD_MARGIN_ROWS: f32 = 8.0;
const KEEP_MARGIN_ROWS: f32 = 14.0;

fn section_band_rows(
    band: &RangeInclusive<f32>,
    height: f32,
    row_count: i64,
    current: GiftShopShelfRows,
) -> GiftShopShelfRows {
    if row_count <= 0 || height <= 0.0 {
        return GiftShopShelfRows::NONE;
    }
    let stride = height / row_count as f32;
    if stride <= 0.0 {
        return GiftShopShelfRows::NONE;
    }
    let rows = |margin_rows: f32| -> GiftShopShelfRows {
        let margin = margin_rows * stride;
        let top = (band.start() - margin).max(0.0);
        let bottom = height.min(band.end() + margin);
        if top >= bottom {
            return GiftShopShelfRows::NONE;
        }
        GiftShopShelfRows {
            first: (row_count - 1).min((top / stride) as i64),
            last: (row_count - 1).min((bottom / stride) as i64),
        }
    };
    let current = current.clamped(row_count);
    let needed = rows(BUILD_MARGIN_ROWS);
    let limit = rows(KEEP_MARGIN_ROWS);
    let covers = needed.is_empty() || (!current.is_empty() && current.first <= needed.first && current.last >= needed.last);
    let within = current.is_empty() || (!limit.is_empty() && current.first >= limit.first && current.last <= limit.last);
    if covers && within {
        return current;
    }
    rows(REBUILD_MARGIN_ROWS)
}

/// header sits at its section top until the pin line passes it
/// then it rides the line until the last row pushes it up
fn pinned_header_y(section_top: f32, section_bottom: f32, line: f32) -> f32 {
    if section_top <= line && line < section_bottom {
        line.min(section_bottom - SECTION_HEADER_HEIGHT).max(section_top)
    } else {
        section_top
    }
}

/// where the pinned header line is read from, the tab bar bottom or else the page scroll top
#[derive(Clone)]
struct ChatProfilePinLine {
    tabs_bottom: Option<ChatProfilePinnedTabsBottom>,
    page_scroll: Option<ScrollHandle>,
    top_inset: f32,
}

impl ChatProfilePinLine {
    /// the line in list coordinates for the given list window top
    fn line(&self, list_top: Pixels) -> Option<f32> {
        match (self.tabs_bottom.as_ref().and_then(|bottom| bottom.get()), self.page_scroll.as_ref()) {
            (Some(bottom), _) => Some(bottom - f32::from(list_top)),
            (None, Some(handle)) => Some(f32::from(handle.bounds().top() - list_top) + self.top_inset),
            (None, None) => None,
        }
    }
}

struct ChatProfilePinnedHeader {
    child: AnyElement,
    layout_y: f32,
    section_top: f32,
    section_bottom: f32,
    line: ChatProfilePinLine,
}

impl IntoElement for ChatProfilePinnedHeader {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ChatProfilePinnedHeader {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let list_top = bounds.top() - px(self.layout_y);
        let y = self
            .line
            .line(list_top)
            .map_or(self.layout_y, |line| pinned_header_y(self.section_top, self.section_bottom, line));
        window.with_element_offset(point(px(0.0), px(y - self.layout_y)), |window| self.child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

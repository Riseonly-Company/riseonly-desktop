use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, BoxShadow, ElementId, FontFeatures, FontWeight, Hsla, SharedString, TextRun, Window,
    black, point, relative,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::main_text::{MainText, system_font};
use crate::swiftui::animated::Animated;
use crate::swiftui::button_style::plain;
use crate::swiftui::motion::spring;
use crate::swiftui::prelude::*;

const CONTROL_HEIGHT: f32 = 32.0;
const TRACK_RADIUS: f32 = 9.0;
const TRACK_INSET: f32 = 2.0;
const THUMB_RADIUS: f32 = 7.0;
const TITLE_SIZE: f32 = 13.0;
const SEGMENT_PADDING_X: f32 = 6.0;
const TITLE_MINIMUM_SCALE: f32 = 0.7;
const TITLE_WIDTH_SLACK: f32 = 2.0;

pub type SegmentedPickerSelect = Rc<dyn Fn(usize, &mut Window, &mut App)>;

struct SegmentedPickerState {
    selected_index: usize,
    thumb_position: Animated<f32>,
}

/// segments are titled in order, selection is the position of one
#[derive(IntoElement)]
pub struct SegmentedPicker {
    id: ElementId,
    titles: Vec<SharedString>,
    selected_index: usize,
    on_select: SegmentedPickerSelect,
}

impl SegmentedPicker {
    pub fn new(
        id: impl Into<ElementId>,
        titles: Vec<SharedString>,
        selected_index: usize,
        on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), titles, selected_index, on_select: Rc::new(on_select) }
    }
}

impl RenderOnce for SegmentedPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = crate::theme(cx).clone();
        let selected_index = self.selected_index;
        let state = window.use_keyed_state(self.id.clone(), cx, move |_, _| SegmentedPickerState {
            selected_index,
            thumb_position: Animated::new(selected_index as f32),
        });
        let now = RiseClock::monotonic();
        if state.read(cx).selected_index != selected_index {
            let reduce_motion = reduce_motion(cx);
            state.update(cx, |state, _| {
                state.selected_index = selected_index;
                state.thumb_position.set(selected_index as f32, spring(0.28, 0.86), now, reduce_motion);
            });
        }
        let thumb_position = state.read(cx).thumb_position.frame_value(now, window);
        let segment_count = self.titles.len();
        let segment_fraction = 1.0 / segment_count.max(1) as f32;
        let content_min_width = content_min_width(&self.titles, window);
        let picker_id = Arc::new(self.id);
        let on_select = self.on_select;

        let segments = self.titles.into_iter().enumerate().map(|(index, title)| {
            let on_select = on_select.clone();
            plain(ElementId::NamedChild(picker_id.clone(), SharedString::from(format!("segment-{index}"))))
                .flex_1()
                .min_w_0()
                .h_full()
                .px(px(SEGMENT_PADDING_X))
                .flex()
                .items_center()
                .justify_center()
                .on_press(move |window, cx| {
                    if index != selected_index {
                        on_select(index, window, cx);
                    }
                })
                .child(
                    MainText::new(title)
                        .px(TITLE_SIZE)
                        .font_weight(if index == selected_index { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                        .number_of_lines(1)
                        .minimum_scale_factor(TITLE_MINIMUM_SCALE),
                )
        });

        let track = div()
            .flex_shrink_0()
            .w_full()
            .min_w(px(content_min_width))
            .h(px(CONTROL_HEIGHT))
            .p(px(TRACK_INSET))
            .rounded(px(TRACK_RADIUS))
            .bg(theme.text.secondary.opacity(0.16))
            .child(
                div()
                    .relative()
                    .size_full()
                    .children((selected_index < segment_count).then(|| {
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(thumb_position * segment_fraction))
                            .w(relative(segment_fraction))
                            .rounded(px(THUMB_RADIUS))
                            .bg(theme.bg._100)
                            .shadow(vec![BoxShadow {
                                color: black().opacity(0.12),
                                offset: point(px(0.0), px(1.0)),
                                blur_radius: px(3.0),
                                spread_radius: px(0.0),
                                inset: false,
                            }])
                    }))
                    .child(hstack(0.0).relative().size_full().children(segments)),
            );

        div()
            .id(ElementId::NamedChild(picker_id, SharedString::new_static("scroll")))
            .flex()
            .w_full()
            .min_w_0()
            .overflow_x_scroll()
            .child(track)
    }
}

/// width where every segment shows its whole title at full size, selected one in semibold
fn content_min_width(titles: &[SharedString], window: &Window) -> f32 {
    let font = system_font(FontWeight::SEMIBOLD, FontFeatures::default());
    let widest = titles
        .iter()
        .filter(|title| !title.is_empty())
        .map(|title| {
            let run = TextRun {
                len: title.len(),
                font: font.clone(),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(window.text_system().shape_line(title.clone(), px(TITLE_SIZE), &[run], None).width()).ceil()
        })
        .fold(0.0_f32, f32::max);
    let segment = widest + 2.0 * SEGMENT_PADDING_X + TITLE_WIDTH_SLACK;
    segment * titles.len() as f32 + 2.0 * TRACK_INSET
}

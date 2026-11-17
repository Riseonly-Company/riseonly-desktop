use gpui::{App, RenderOnce, Window};
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

#[derive(IntoElement)]
pub struct NotifyRowSkeleton {
    alternation_index: usize,
}

impl NotifyRowSkeleton {
    pub fn new() -> Self {
        Self { alternation_index: 0 }
    }

    pub fn alternation_index(mut self, alternation_index: usize) -> Self {
        self.alternation_index = alternation_index;
        self
    }

    fn alt(&self, values: [f32; 4]) -> f32 {
        values[self.alternation_index % values.len()]
    }
}

impl Default for NotifyRowSkeleton {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for NotifyRowSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let width = self.alt([160.0, 220.0, 120.0, 190.0]);
        hstack(12.0)
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(11.0))
            .child(SkeletonCircle::new(46.0))
            .child(
                vstack(6.0)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(SkeletonLine::new(None::<f32>, 9.0))
                    .child(SkeletonLine::new(width, 9.0)),
            )
    }
}

#[derive(IntoElement)]
pub struct NotifyListSkeleton {
    rows: usize,
}

impl NotifyListSkeleton {
    pub fn new() -> Self {
        Self { rows: 8 }
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    fn section_header() -> Div {
        div().px(px(14.0)).pt(px(16.0)).pb(px(6.0)).child(SkeletonLine::new(90.0, 9.0))
    }
}

impl Default for NotifyListSkeleton {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for NotifyListSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let mut items: Vec<AnyElement> = vec![Self::section_header().into_any_element()];
        for index in 0..self.rows {
            if index == 4 {
                items.push(Self::section_header().into_any_element());
            }
            items.push(NotifyRowSkeleton::new().alternation_index(index).into_any_element());
            items.push(
                div()
                    .w_full()
                    .pl(px(72.0))
                    .child(div().w_full().h(px(0.5)).bg(theme.border._100.opacity(0.5)))
                    .into_any_element(),
            );
        }
        div()
            .size_full()
            .overflow_hidden()
            .child(vstack(0.0).w_full().items_start().children(items).skeleton_shimmer(None))
    }
}

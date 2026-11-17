use gpui::{
    AppContext as _, Context, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    Render, Role, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::swiftui::shapes::SwiftUiShapes;

use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowMainContext, PostRowModerationBannerLayout,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::PostRowMetrics;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::{
    PostRowChromeFonts, PostRowTextLineView, PostRowTextLineViewAlignment, post_row_frame,
};

const BANNER_CORNER_RADIUS: f32 = 8.0;
const BANNER_FILL_OPACITY: f32 = 0.1;

pub struct PostRowModerationBannerView {
    label_view: Entity<PostRowTextLineView>,
    second_label_view: Entity<PostRowTextLineView>,
    banner_layout: Option<PostRowModerationBannerLayout>,
    symbol_name: Option<&'static str>,
    color: Hsla,
    accessibility_label: SharedString,
}

impl PostRowModerationBannerView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            label_view: cx.new(|_| PostRowTextLineView::new()),
            second_label_view: cx.new(|_| PostRowTextLineView::new()),
            banner_layout: None,
            symbol_name: None,
            color: gpui::transparent_black(),
            accessibility_label: SharedString::default(),
        }
    }

    pub fn configure(
        &mut self,
        layout: &PostRowModerationBannerLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        self.banner_layout = Some(layout.clone());
        let banner = layout.banner;
        self.symbol_name = Some(banner.symbol_name());
        let color = if banner.is_error() { context.palette.error100 } else { context.palette.primary100 };
        self.color = color;
        let text = tr(banner.text_key());
        let font = PostRowChromeFonts::text(PostRowMetrics::BANNER_FONT_SIZE, FontWeight::MEDIUM, &context.spec);
        let first = layout.lines.first().map(String::as_str).unwrap_or(text.as_str());
        let second = layout.lines.get(1).cloned();
        self.label_view.update(cx, |view, cx| {
            view.set_text(first, font, color, PostRowTextLineViewAlignment::Leading, cx);
        });
        self.second_label_view.update(cx, |view, cx| match second.as_deref() {
            Some(line) => view.set_text(line, font, color, PostRowTextLineViewAlignment::Leading, cx),
            None => view.clear(cx),
        });
        self.accessibility_label = text.into();
        cx.notify();
    }
}

impl Render for PostRowModerationBannerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(layout) = self.banner_layout.as_ref() else {
            return div().into_any_element();
        };
        let color = self.color;
        let line_count = layout.lines.len().max(1);
        let line_height = f32::from(layout.label.size.height) / line_count as f32;
        let lines = [self.label_view.clone(), self.second_label_view.clone()];
        let label = post_row_frame(layout.label).children(lines.into_iter().take(line_count).enumerate().map(
            |(index, view)| div().absolute().left_0().top(px(line_height * index as f32)).w_full().h(px(line_height)).child(view),
        ));
        let icon = self.symbol_name.and_then(|symbol| {
            IconUi::sized(symbol, px(PostRowMetrics::BANNER_ICON_POINT_SIZE), color, symbol.ends_with(".fill"))
        });
        div()
            .id(ElementId::NamedInteger("moderation-banner".into(), cx.entity_id().as_u64()))
            .role(Role::Label)
            .aria_label(self.accessibility_label.clone())
            .relative()
            .size_full()
            .overflow_hidden()
            .continuous_corners(BANNER_CORNER_RADIUS)
            .bg(color.opacity(BANNER_FILL_OPACITY))
            .child(post_row_frame(layout.icon).flex().items_center().justify_center().children(icon))
            .child(label)
            .into_any_element()
    }
}

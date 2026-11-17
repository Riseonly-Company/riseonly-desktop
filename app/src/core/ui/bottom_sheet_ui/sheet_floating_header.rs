use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, IntoElement, RenderOnce, SharedString, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::ThemeStore;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::scroll_chrome_shadow_ui::{
    ScrollChromeShadowStyleAnchor, ScrollChromeShadowStyle, ScrollChromeShadowUi,
};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::modules::media_picker::ui::media_picker_header_icon_button::MediaPickerHeaderIconButton;

/// sizes of the floating header, readable without building one
pub enum SheetFloatingHeaderMetrics {}

impl SheetFloatingHeaderMetrics {
    pub const HEIGHT: f32 = 56.0;
    pub const CHROME_EXTENT: f32 = Self::HEIGHT + 14.0;
}

pub type SheetFloatingHeaderAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SheetFloatingHeader {
    pub id: ElementId,
    pub title: SharedString,
    pub shows_title: bool,
    pub shows_chrome: bool,
    pub leading_system_image: SharedString,
    pub on_leading: SheetFloatingHeaderAction,
    pub trailing: Option<AnyElement>,
}

impl SheetFloatingHeader {
    /// defaults are shows_title and shows_chrome true, leading image xmark and no trailing element
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        on_leading: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            shows_title: true,
            shows_chrome: true,
            leading_system_image: "xmark".into(),
            on_leading: Rc::new(on_leading),
            trailing: None,
        }
    }

    pub fn shows_title(mut self, shows_title: bool) -> Self {
        self.shows_title = shows_title;
        self
    }

    pub fn shows_chrome(mut self, shows_chrome: bool) -> Self {
        self.shows_chrome = shows_chrome;
        self
    }

    pub fn leading_system_image(mut self, leading_system_image: impl Into<SharedString>) -> Self {
        self.leading_system_image = leading_system_image.into();
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

struct SheetFloatingHeaderFades {
    title: Animated<f32>,
    chrome: Animated<f32>,
}

fn visibility(shown: bool) -> f32 {
    if shown { 1.0 } else { 0.0 }
}

impl RenderOnce for SheetFloatingHeader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let text_primary = theme_store.read(cx).current_theme().text.primary;

        let (shows_title, shows_chrome) = (self.shows_title, self.shows_chrome);
        let fades = window.use_keyed_state(self.child_id("fades"), cx, |_, _| SheetFloatingHeaderFades {
            title: Animated::new(visibility(shows_title)),
            chrome: Animated::new(visibility(shows_chrome)),
        });
        let now = RiseClock::monotonic();
        fades.update(cx, |fades, cx| {
            fades.title.set_in(visibility(shows_title), ease_out(0.18), now, cx);
            fades.chrome.set_in(visibility(shows_chrome), ease_out(0.18), now, cx);
        });
        let fades = fades.read(cx);
        let title_opacity = fades.title.frame_value(now, window);
        let chrome_opacity = fades.chrome.frame_value(now, window);

        let title = pass_through()
            .flex_shrink_1()
            .min_w_0()
            .max_w(px(220.0))
            .opacity(title_opacity)
            .child(
                MainText::new(self.title.clone())
                    .px(17.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(text_primary)
                    .number_of_lines(1),
            );

        let on_leading = self.on_leading.clone();
        let leading = hstack(0.0).flex_1().justify_start().child(
            MediaPickerHeaderIconButton::new(
                self.child_id("leading"),
                self.leading_system_image.clone(),
                move |window, cx| on_leading(window, cx),
            )
            .font_size(15.0),
        );
        let trailing = hstack(0.0).flex_1().justify_end().children(self.trailing);

        let chrome = pass_through()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .opacity(chrome_opacity)
            .child(SheetFloatingHeaderChrome::new(SheetFloatingHeaderMetrics::CHROME_EXTENT));

        div()
            .relative()
            .w_full()
            .h(px(SheetFloatingHeaderMetrics::HEIGHT))
            .child(chrome)
            .child(
                hstack(10.0)
                    .size_full()
                    .px(px(14.0))
                    .child(leading)
                    .child(title)
                    .child(trailing),
            )
    }
}

/// color the enclosing sheet paints behind its content so floating chrome can fade into it
pub struct SheetSurfaceColorKey;

impl EnvKey for SheetSurfaceColorKey {
    type Value = Option<Hsla>;

    fn default_value() -> Self::Value {
        None
    }
}

pub fn sheet_surface_color(cx: &App) -> Option<Hsla> {
    env::<SheetSurfaceColorKey>(cx)
}

#[derive(IntoElement)]
pub struct SheetFloatingHeaderChrome {
    pub extent: f32,
}

impl SheetFloatingHeaderChrome {
    pub fn new(extent: f32) -> Self {
        Self { extent }
    }
}

impl RenderOnce for SheetFloatingHeaderChrome {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let surface = theme_store.read(cx).current_theme().bg._100;
        let color = sheet_surface_color(cx).unwrap_or(surface);

        let style = ScrollChromeShadowStyle {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: self.extent,
            overflow_below_header: 0.0,
            peak_opacity: 1.0,
            color,
            ..ScrollChromeShadowStyle::default()
        };
        pass_through()
            .w_full()
            .h(px(self.extent))
            .child(
                ScrollChromeShadowUi::new(style)
                    .solid_fraction(Some(SheetFloatingHeaderMetrics::HEIGHT / self.extent.max(1.0))),
            )
    }
}

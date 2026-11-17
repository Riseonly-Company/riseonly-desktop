use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, Styled,
    Window, div, px,
};
use rise_theme::RiseThemeAppearance;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::IconUi;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::environment_values::{ColorScheme, ColorSchemeKey};

const GLYPH_SIZE: f32 = 16.0;

/// the ellipsis in the chats header menu
/// reads the theme in its own render so it never keeps old palette ink
#[derive(IntoElement, Default)]
pub struct ChatsMenuGlyph;

impl ChatsMenuGlyph {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for ChatsMenuGlyph {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = rise_ui::theme(cx).text.primary;
        div()
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("ellipsis", px(GLYPH_SIZE), color, false))
    }
}

/// pins the glass of the chats menu button to the palette light or dark
/// scheme comes from the palette not the window
#[derive(IntoElement)]
pub struct ChatsMenuGlassSchemePin {
    content: AnyElement,
}

impl ChatsMenuGlassSchemePin {
    pub fn new(content: impl IntoElement) -> Self {
        Self { content: content.into_any_element() }
    }
}

impl ChatsMenuGlassSchemePin {
    /// wraps the content for the modifier
    pub fn body(content: AnyElement, cx: &mut App) -> AnyElement {
        let ink = theme_store(cx).read(cx).window_interface_style();
        let scheme = if ink == RiseThemeAppearance::Light { ColorScheme::Light } else { ColorScheme::Dark };
        let key: ElementId = match scheme {
            ColorScheme::Light => "chats-menu-glass-light".into(),
            ColorScheme::Dark => "chats-menu-glass-dark".into(),
        };
        env_scope::<ColorSchemeKey>(Some(scheme)).child(div().id(key).child(content)).into_any_element()
    }
}

impl RenderOnce for ChatsMenuGlassSchemePin {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        Self::body(self.content, cx)
    }
}

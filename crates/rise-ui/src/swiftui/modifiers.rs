use gpui::{AnyElement, App, IntoElement, RenderOnce, Window};

/// view modifier, body builds the modified view around the content
pub trait ViewModifier: 'static {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement;
}

pub trait ModifierExt: IntoElement + Sized {
    fn modifier<M: ViewModifier>(self, modifier: M) -> ModifiedContent<M> {
        ModifiedContent {
            content: self.into_any_element(),
            modifier,
        }
    }
}

impl<E: IntoElement> ModifierExt for E {}

/// component so the modifier body runs during layout, where the subtree env can be read
#[derive(gpui::IntoElement)]
pub struct ModifiedContent<M: ViewModifier> {
    content: AnyElement,
    modifier: M,
}

impl<M: ViewModifier> RenderOnce for ModifiedContent<M> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.modifier.body(self.content, window, cx)
    }
}

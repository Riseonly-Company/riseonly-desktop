use gpui::{AnyElement, App, Div, IntoElement, SharedString, Window, deferred, div, prelude::*};
use rise_ui::theme;

use crate::overlay::overlay_placement::OverlayPlacement;

/// A stable name for one overlay, not a position in the stack.
pub type OverlayId = SharedString;

type DismissHandler = Box<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OverlayScrim {
    /// A click-catcher that paints nothing, which is what a menu wants.
    #[default]
    Invisible,
    /// A faded backdrop, for a surface that takes the window over.
    Dimmed,
}

/// The in-window layer an overlay is drawn on: it fills the window, holds an
/// optional scrim, and puts its child at the origin [`crate::overlay::place`]
/// computed. It owns no state — what is open is [`OverlayStack`]'s job.
pub struct OverlayLayer {
    placement: OverlayPlacement,
    scrim: OverlayScrim,
    child: Option<AnyElement>,
    dismiss: Option<DismissHandler>,
}

impl OverlayLayer {
    pub fn new(placement: OverlayPlacement) -> Self {
        Self {
            placement,
            scrim: OverlayScrim::default(),
            child: None,
            dismiss: None,
        }
    }

    pub fn scrim(mut self, scrim: OverlayScrim) -> Self {
        self.scrim = scrim;
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }

    /// What a press anywhere on the scrim does. Mouse-down and any button, so a
    /// right-click outside an open menu closes it before opening the next one.
    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.dismiss = Some(Box::new(handler));
        self
    }

    /// The layer, ready to be a child of the window root. Deferred, so it paints
    /// above its siblings; the placement is in window coordinates, so whatever
    /// this is given to must fill the window.
    pub fn render(self, cx: &App) -> AnyElement {
        deferred(self.layer(cx)).into_any_element()
    }

    fn layer(self, cx: &App) -> Div {
        let theme = theme(cx);

        let mut scrim = div().absolute().top_0().left_0().size_full();
        if self.scrim == OverlayScrim::Dimmed {
            scrim = scrim.bg(theme.material.scrim);
        }
        if let Some(dismiss) = self.dismiss {
            scrim = scrim.on_any_mouse_down(move |_, window, cx| dismiss(window, cx));
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(scrim)
            .child(
                // Without `occlude` the scrim stays hovered under the content, and
                // its mouse-down dismiss kills the overlay before the mouse-up.
                div()
                    .occlude()
                    .absolute()
                    .left(self.placement.origin.x)
                    .top(self.placement.origin.y)
                    .children(self.child),
            )
    }
}

/// What is open, innermost last. Bounded at [`OverlayStack::CAPACITY`]: a screen
/// that never dismisses shows a missing overlay rather than leaking memory.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct OverlayStack {
    open: Vec<OverlayId>,
}

impl OverlayStack {
    pub const CAPACITY: usize = 8;

    pub fn new() -> Self {
        Self::default()
    }

    /// Opening what is already open raises it rather than stacking a second copy,
    /// so a repeated open cannot make Esc take two presses.
    pub fn open(&mut self, id: impl Into<OverlayId>) {
        let id = id.into();
        self.open.retain(|existing| existing != &id);
        if self.open.len() >= Self::CAPACITY {
            self.open.remove(0);
        }
        self.open.push(id);
    }

    pub fn dismiss_top(&mut self) -> Option<OverlayId> {
        self.open.pop()
    }

    pub fn dismiss(&mut self, id: &str) -> bool {
        let before = self.open.len();
        self.open.retain(|existing| existing.as_ref() != id);
        self.open.len() != before
    }

    pub fn top(&self) -> Option<&OverlayId> {
        self.open.last()
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.open.iter().any(|existing| existing.as_ref() == id)
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    pub fn clear(&mut self) {
        self.open.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_stack_has_nothing_open() {
        let stack = OverlayStack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
        assert_eq!(stack.top(), None);
        assert!(!stack.is_open("menu"));
    }

    #[test]
    fn the_last_opened_is_the_top() {
        let mut stack = OverlayStack::new();
        stack.open("menu");
        stack.open("submenu");

        assert_eq!(stack.top().map(SharedString::as_ref), Some("submenu"));
        assert_eq!(stack.len(), 2);
        assert!(stack.is_open("menu"));
    }

    #[test]
    fn escape_takes_the_top_one_only() {
        let mut stack = OverlayStack::new();
        stack.open("menu");
        stack.open("submenu");

        assert_eq!(stack.dismiss_top().as_deref(), Some("submenu"));
        assert_eq!(stack.top().map(SharedString::as_ref), Some("menu"));
        assert_eq!(stack.dismiss_top().as_deref(), Some("menu"));
        assert_eq!(stack.dismiss_top(), None);
        assert!(stack.is_empty());
    }

    #[test]
    fn opening_the_same_id_twice_raises_it_instead_of_stacking_it() {
        let mut stack = OverlayStack::new();
        stack.open("menu");
        stack.open("sheet");
        stack.open("menu");

        assert_eq!(stack.len(), 2, "one Esc must close one overlay");
        assert_eq!(stack.top().map(SharedString::as_ref), Some("menu"));

        stack.dismiss_top();
        assert!(!stack.is_open("menu"));
    }

    #[test]
    fn dismissing_by_id_reaches_below_the_top() {
        let mut stack = OverlayStack::new();
        stack.open("menu");
        stack.open("sheet");

        assert!(stack.dismiss("menu"));
        assert!(!stack.is_open("menu"));
        assert_eq!(stack.top().map(SharedString::as_ref), Some("sheet"));
        assert!(
            !stack.dismiss("menu"),
            "dismissing twice is not a second one"
        );
    }

    #[test]
    fn the_stack_is_bounded_and_drops_the_oldest() {
        let mut stack = OverlayStack::new();
        for index in 0..OverlayStack::CAPACITY * 2 {
            stack.open(format!("overlay-{index}"));
        }

        assert_eq!(stack.len(), OverlayStack::CAPACITY);
        assert!(!stack.is_open("overlay-0"), "a leak must not grow forever");
        assert!(stack.is_open("overlay-15"));
        assert_eq!(stack.top().map(SharedString::as_ref), Some("overlay-15"));
    }

    #[test]
    fn clearing_closes_everything() {
        let mut stack = OverlayStack::new();
        stack.open("menu");
        stack.open("sheet");
        stack.clear();

        assert!(stack.is_empty());
        assert_eq!(stack.dismiss_top(), None);
    }
}

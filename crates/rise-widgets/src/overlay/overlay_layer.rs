use gpui::{AnyElement, App, Div, IntoElement, SharedString, Window, deferred, div, prelude::*};
use rise_ui::theme;

use crate::overlay::overlay_placement::OverlayPlacement;

/// A stable name for one overlay, not a position in the stack.
pub type OverlayId = SharedString;

type DismissHandler = Box<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OverlayScrim {
    /// A click-catcher that paints nothing. What a menu wants: a desktop menu
    /// dismisses on a click outside without dimming the window behind it.
    #[default]
    Invisible,
    /// A faded backdrop, for a surface that takes the window over.
    Dimmed,
}

/// The in-window layer an overlay is drawn on.
///
/// gpui has no native popups outside Wayland, so every menu, dropdown and
/// tooltip is a child of this window rather than a window of its own. The layer
/// fills the window, holds an optional scrim, and puts its child at the origin
/// [`crate::overlay::place`] computed.
///
/// It is a rendering helper, not a screen: it owns no state and decides nothing
/// about what is open. That is [`OverlayStack`]'s job.
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

    /// What a press anywhere on the scrim does.
    ///
    /// Mouse-down rather than click, and any button rather than the primary one:
    /// a right-click outside an open menu must close it before it opens the next
    /// one, and a press that starts outside has already left the overlay.
    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.dismiss = Some(Box::new(handler));
        self
    }

    /// The layer, ready to be a child of the window root.
    ///
    /// Deferred, so it paints above everything laid out around it whatever order
    /// the root mounts it in. The placement is in WINDOW coordinates, so the
    /// element this is given to must fill the window.
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
                // `occlude` is load-bearing, not a hint. gpui's hit test collects
                // EVERY hitbox under the pointer and stops only at a blocking
                // one, so without this the window-filling scrim is still hovered
                // while the pointer is over the overlay's own content — and its
                // dismiss handler, which fires on mouse DOWN, tears the overlay
                // out of the tree before the mouse UP that would have completed
                // a click on it. The menu would be reachable only by keyboard.
                div()
                    .occlude()
                    .absolute()
                    .left(self.placement.origin.x)
                    .top(self.placement.origin.y)
                    .children(self.child),
            )
    }
}

/// What is open, innermost last.
///
/// Bounded on purpose. A screen that opens an overlay and never dismisses it
/// leaks one entry per open, and an unbounded stack turns that into a slow leak
/// nothing ever reports; dropping the oldest keeps the failure visible as a
/// missing overlay instead of invisible as growing memory.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct OverlayStack {
    open: Vec<OverlayId>,
}

impl OverlayStack {
    pub const CAPACITY: usize = 8;

    pub fn new() -> Self {
        Self::default()
    }

    /// Opening what is already open raises it rather than stacking a second copy
    /// of it, so a repeated open cannot make Esc take two presses.
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

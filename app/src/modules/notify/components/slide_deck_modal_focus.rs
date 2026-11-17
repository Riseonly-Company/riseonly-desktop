use gpui::{App, Entity, FocusHandle, Window};

pub(crate) struct SlideDeckModalFocus {
    pub(crate) handle: FocusHandle,
    previous: Option<FocusHandle>,
    last_drawn: Option<FocusHandle>,
}

impl SlideDeckModalFocus {
    /// call while rendering the modal before its deck renders
    /// the first frame still sees the focus of the screen underneath
    pub(crate) fn keyed(id: &'static str, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let this = window.use_keyed_state(id, cx, |window, cx| {
            let previous = window.focused(cx);
            Self { handle: cx.focus_handle(), previous, last_drawn: None }
        });
        let focused = window.focused(cx);
        this.update(cx, |focus, _| focus.last_drawn = focused);
        this
    }

    /// runs after the frame without the modal
    /// a screen an action opened has claimed focus by then and keeps it
    /// only unchanged focus is handed back
    pub(crate) fn hand_back(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let (previous, last_drawn) = this.update(cx, |focus, _| (focus.previous.take(), focus.last_drawn.take()));
        if let Some(previous) = previous
            && window.focused(cx) == last_drawn
        {
            window.focus(&previous, cx);
        }
    }
}

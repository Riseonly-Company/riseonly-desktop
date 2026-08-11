use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, Global, IntoElement, KeyBinding,
    SharedString, Window, actions, div, prelude::*,
};
use rise_theme::AppTheme;

use crate::modal_ui::modal_frame::{ModalWidth, resolve_frame};
use rise_ui::{ButtonTone, ButtonUi, IconSize, IconUi, MainText, TextTone};
use smallvec::SmallVec;

/// Scoped so an open modal never eats a shortcut the shell owns.
pub const KEY_CONTEXT: &str = "Modal";

const CLOSE_SYMBOL: &str = "xmark";

actions!(rise_modal, [DismissModal]);

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

/// Idempotent. Call it from the composition root, or from any screen that builds
/// a modal.
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);
    cx.bind_keys([KeyBinding::new("escape", DismissModal, Some(KEY_CONTEXT))]);
}

/// What closes the modal besides its own buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ModalDismiss {
    #[default]
    /// Esc and a click on the scrim, the way a dialog normally behaves.
    Anywhere,
    /// Only an explicit control, for a question that must be answered: a
    /// destructive confirmation, a step the flow cannot continue past.
    Explicit,
}

impl ModalDismiss {
    pub fn is_casual(self) -> bool {
        matches!(self, Self::Anywhere)
    }
}

type Handler = Box<dyn Fn(&mut Window, &mut App) + 'static>;

/// One button in the modal's footer.
pub struct ModalAction {
    label: SharedString,
    tone: ButtonTone,
    id: ElementId,
    enabled: bool,
    handler: Option<Handler>,
}

impl ModalAction {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tone: ButtonTone) -> Self {
        Self {
            label: label.into(),
            tone,
            id: id.into(),
            enabled: true,
            handler: None,
        }
    }

    pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, label, ButtonTone::Primary)
    }

    /// The unaccented one: cancel, "not now", the second of two choices.
    pub fn neutral(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, label, ButtonTone::Neutral)
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.handler = Some(Box::new(handler));
        self
    }
}

/// Every modal in the product — use it for anything that floats over the page.
///
/// It is opaque, never a [`crate::GlassPanel`]: a native material is an AppKit
/// view below the Metal layer, so what gpui already drew stays on top of it. It
/// occludes, so a click inside cannot also land on the dismiss scrim. Its height
/// is bounded by the viewport, and Esc means the same as clicking outside.
///
/// ```ignore
/// ModalUi::new("auth.telegram")
///     .title(tr("tg_redirect_title"))
///     .subtitle(tr("tg_redirect_subtitle"))
///     .width(ModalWidth::Medium)
///     .track_focus(&self.focus_handle)
///     .on_dismiss(cx.listener(..))
///     .child(link_row)
///     .action(ModalAction::primary("open", tr("tg_redirect_open")).on_click(..))
///     .action(ModalAction::secondary("done", tr("tg_redirect_done")).on_click(..))
///     .render(&theme, window, cx)
/// ```
pub struct ModalUi {
    id: ElementId,
    title: Option<SharedString>,
    subtitle: Option<SharedString>,
    width: ModalWidth,
    dismiss: ModalDismiss,
    shows_close: bool,
    scrolls: bool,
    focus: Option<FocusHandle>,
    on_dismiss: Option<Handler>,
    actions: Vec<ModalAction>,
    children: SmallVec<[AnyElement; 2]>,
}

impl ModalUi {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            title: None,
            subtitle: None,
            width: ModalWidth::default(),
            dismiss: ModalDismiss::default(),
            shows_close: true,
            scrolls: true,
            focus: None,
            on_dismiss: None,
            actions: Vec::new(),
            children: SmallVec::new(),
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn width(mut self, width: ModalWidth) -> Self {
        self.width = width;
        self
    }

    pub fn dismiss(mut self, dismiss: ModalDismiss) -> Self {
        self.dismiss = dismiss;
        self
    }

    /// Hides the header's close control. The modal is still dismissible unless
    /// [`ModalDismiss::Explicit`] says otherwise.
    pub fn without_close(mut self) -> Self {
        self.shows_close = false;
        self
    }

    /// Lets the body size itself instead of scrolling inside the ceiling.
    pub fn without_scroll(mut self) -> Self {
        self.scrolls = false;
        self
    }

    /// Required for Esc to work: the action is dispatched up the FOCUS chain,
    /// so a modal nothing focuses never sees the key.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }

    pub fn action(mut self, action: ModalAction) -> Self {
        self.actions.push(action);
        self
    }

    pub fn render(self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        install_key_bindings(cx);

        let metrics = theme.modal;
        let frame = resolve_frame(
            self.width,
            metrics,
            window.viewport_size(),
            theme.shell.overlay_margin,
        );

        let Self {
            id,
            title,
            subtitle,
            width: _,
            dismiss,
            shows_close,
            scrolls,
            focus,
            on_dismiss,
            actions,
            children,
        } = self;

        let casual = dismiss.is_casual();
        let on_dismiss = on_dismiss.map(std::rc::Rc::new);

        let mut panel = div()
            .id(id)
            // Without this a click inside also lands on the scrim and dismisses.
            .occlude()
            .key_context(KEY_CONTEXT)
            .flex()
            .flex_col()
            .w(frame.width)
            .max_h(frame.max_height)
            .p(metrics.padding)
            .gap(metrics.section_gap)
            .bg(theme.bg._100)
            .border_1()
            .border_color(theme.border._200)
            .rounded(theme.radius._400)
            .shadow_lg();

        if let Some(focus) = focus.as_ref() {
            panel = panel.track_focus(focus);
        }

        if let Some(handler) = on_dismiss.clone()
            && casual
        {
            panel = panel.on_action(move |_: &DismissModal, window, cx| handler(window, cx));
        }

        if let Some(header) = Self::header(theme, title, subtitle, shows_close, on_dismiss.clone())
        {
            panel = panel.child(header);
        }

        let mut body = div().flex().flex_col().gap(theme.spacing._400);
        body = body.children(children);
        panel = panel.child(if scrolls {
            div()
                .id("modal.body")
                .flex_1()
                .overflow_y_scroll()
                .child(body)
                .into_any_element()
        } else {
            body.into_any_element()
        });

        if let Some(footer) = Self::footer(theme, actions) {
            panel = panel.child(footer);
        }

        let mut scrim = div()
            .id("modal.scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.material.scrim);

        if let Some(handler) = on_dismiss
            && casual
        {
            scrim = scrim.on_click(move |_: &ClickEvent, window, cx| handler(window, cx));
        }

        scrim.child(panel).into_any_element()
    }

    fn header(
        theme: &AppTheme,
        title: Option<SharedString>,
        subtitle: Option<SharedString>,
        shows_close: bool,
        on_dismiss: Option<std::rc::Rc<Handler>>,
    ) -> Option<Div> {
        let title = title?;
        let heading = theme.typography.headline();

        let mut text = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(theme.spacing._300)
            .child(
                div()
                    .text_size(heading.size)
                    .line_height(heading.line_height)
                    .font(heading.font)
                    .text_color(theme.text.primary)
                    .child(title),
            );

        if let Some(subtitle) = subtitle {
            text = text.child(MainText::body(theme, TextTone::Secondary).child(subtitle));
        }

        let mut header = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(theme.spacing._400)
            .child(text);

        if shows_close && let Some(handler) = on_dismiss {
            let mut close = div()
                .id("modal.close")
                .flex_none()
                .size(theme.modal.close_size)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme.bg._200)
                .cursor_pointer()
                .on_click(move |_: &ClickEvent, window, cx| handler(window, cx));

            if let Some(icon) =
                IconUi::render(theme, CLOSE_SYMBOL, IconSize::Small, theme.text.secondary)
            {
                close = close.child(icon);
            }
            header = header.child(close);
        }

        Some(header)
    }

    fn footer(theme: &AppTheme, actions: Vec<ModalAction>) -> Option<Div> {
        if actions.is_empty() {
            return None;
        }

        let metrics = theme.modal;
        let mut footer = div().flex().flex_col().gap(metrics.action_gap).w_full();

        for action in actions {
            let ModalAction {
                label,
                tone,
                id,
                enabled,
                handler,
            } = action;

            let mut slot = div().id(id).w_full();
            if let Some(handler) = handler
                && enabled
            {
                slot = slot
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, window, cx| handler(window, cx));
            }

            footer = footer.child(
                slot.child(
                    ButtonUi::sized(theme, tone, metrics.action_height)
                        .w_full()
                        .opacity(if enabled { 1.0 } else { 0.5 })
                        .child(label),
                ),
            );
        }

        Some(footer)
    }
}

impl ParentElement for ModalUi {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_modal_is_not_dismissed_by_a_stray_click() {
        assert!(ModalDismiss::Anywhere.is_casual());
        assert!(!ModalDismiss::Explicit.is_casual());
    }

    #[test]
    fn the_default_is_the_ordinary_dialog() {
        assert_eq!(ModalDismiss::default(), ModalDismiss::Anywhere);
    }

    #[test]
    fn a_modal_is_built_without_a_title_an_action_or_a_handler() {
        let modal = ModalUi::new("bare");
        assert!(modal.title.is_none());
        assert!(modal.actions.is_empty());
        assert!(modal.shows_close);
        assert!(modal.scrolls);
    }
}

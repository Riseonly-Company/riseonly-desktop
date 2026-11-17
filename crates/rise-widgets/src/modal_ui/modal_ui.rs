use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, Global, IntoElement, KeyBinding,
    SharedString, Window, actions, div, prelude::*,
};
use rise_theme::AppTheme;

use crate::modal_ui::modal_frame::{ModalWidth, resolve_frame};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::{ButtonUi, IconSize, IconUi, MainText, SecondaryText};
use std::rc::Rc;
use smallvec::SmallVec;

pub const KEY_CONTEXT: &str = "Modal";

const CLOSE_SYMBOL: &str = "xmark";

actions!(rise_modal, [DismissModal]);

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

/// idempotent, call it from the composition root or any screen that builds a modal
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);
    cx.bind_keys([KeyBinding::new("escape", DismissModal, Some(KEY_CONTEXT))]);
}

/// what closes the modal besides its own buttons
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ModalDismiss {
    #[default]
    Anywhere,
    Explicit,
}

impl ModalDismiss {
    pub fn is_casual(self) -> bool {
        matches!(self, Self::Anywhere)
    }
}

type Handler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

/// which fill a footer button takes
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ModalActionTone {
    #[default]
    Primary,
    Neutral,
}

/// one button in the modal footer
pub struct ModalAction {
    label: SharedString,
    tone: ModalActionTone,
    id: ElementId,
    enabled: bool,
    handler: Option<Handler>,
}

impl ModalAction {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tone: ModalActionTone) -> Self {
        Self {
            label: label.into(),
            tone,
            id: id.into(),
            enabled: true,
            handler: None,
        }
    }

    pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, label, ModalActionTone::Primary)
    }

    /// the unaccented one, cancel or not now or the second of two choices
    pub fn neutral(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, label, ModalActionTone::Neutral)
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.handler = Some(Rc::new(handler));
        self
    }
}

/// every modal in the product, opaque (never a GlassPanel)
/// occluding and bounded by the viewport
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

    pub fn without_close(mut self) -> Self {
        self.shows_close = false;
        self
    }

    pub fn without_scroll(mut self) -> Self {
        self.scrolls = false;
        self
    }

    /// needed for esc, the action is dispatched up the focus chain
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
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
            .occlude()
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

        let cancel = match on_dismiss.clone() {
            Some(handler) if casual => LayerCancel::dismiss(move |window, cx| handler(window, cx)),
            _ => LayerCancel::Blocked,
        };
        let primary = actions
            .iter()
            .find(|action| action.tone == ModalActionTone::Primary)
            .and_then(|action| action.handler.clone().map(|handler| (action.enabled, handler)));
        panel = match primary {
            Some((enabled, handler)) => panel.layer_form_keys(cancel, enabled, move |window, cx| handler(window, cx)),
            None => panel.layer_keys(cancel),
        };

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
            text = text.child(SecondaryText::new(subtitle));
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

            let background = match tone {
                ModalActionTone::Primary => theme.primary._100,
                ModalActionTone::Neutral => theme.button.bg_300,
            };
            let mut button = ButtonUi::new(id)
                .disabled(!enabled)
                .background_color(background)
                .height(f32::from(metrics.action_height));
            if let Some(handler) = handler {
                button = button.on_press(move |window, cx| handler(window, cx));
            }
            footer = footer.child(button.child(MainText::new(label)));
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

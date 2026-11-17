use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, SharedString, Window};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

type ChatSearchAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ChatSearchSectionHeader {
    id: ElementId,
    title: String,
    right_title: Option<String>,
    on_right: Option<ChatSearchAction>,
}

impl ChatSearchSectionHeader {
    pub fn new(id: impl Into<ElementId>, title: impl Into<String>) -> Self {
        Self { id: id.into(), title: title.into(), right_title: None, on_right: None }
    }

    pub fn right_title(mut self, right_title: Option<String>) -> Self {
        self.right_title = right_title;
        self
    }

    pub fn on_right(mut self, on_right: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_right = Some(Rc::new(on_right));
        self
    }
}

impl RenderOnce for ChatSearchSectionHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let on_right = self.on_right;
        let right = self.right_title.map(|right_title| {
            plain(self.id.clone())
                .on_press(move |window, cx| {
                    if let Some(on_right) = &on_right {
                        on_right(window, cx);
                    }
                })
                .child(MainText::new(right_title).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.primary._100))
        });
        hstack(None::<f32>)
            .w_full()
            .px(px(16.0))
            .py(px(8.0))
            .child(MainText::new(self.title).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
            .child(div().flex_1())
            .children(right)
    }
}

/// a failed search tab, a plain sentence and a retry
/// the transport error itself is never shown
#[derive(IntoElement)]
pub struct ChatSearchErrorState {
    id: ElementId,
    on_retry: ChatSearchAction,
}

impl ChatSearchErrorState {
    pub fn new(id: impl Into<ElementId>, on_retry: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), on_retry: Rc::new(on_retry) }
    }
}

impl RenderOnce for ChatSearchErrorState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let on_retry = self.on_retry;
        vstack(10.0)
            .w_full()
            .px(px(24.0))
            .pt(px(52.0))
            .children(IconUi::sized("wifi.exclamationmark", px(28.0), theme.text.secondary, false))
            .child(
                MainText::new(tr("error_fetching_data"))
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(3),
            )
            .child(
                system_button(self.id, BorderedStyle::Bordered)
                    .on_press(move |window, cx| on_retry(window, cx))
                    .child(SharedString::from(tr("common_retry"))),
            )
    }
}

/// brings the list to the new ids by the part that differs
/// replacing would throw the person back to the top each time a page or late section lands
pub(crate) fn chat_search_sync_list(list: &ListUi, current: &mut Vec<String>, next: Vec<String>) {
    if *current == next {
        return;
    }
    let prefix = current.iter().zip(&next).take_while(|(old, new)| old == new).count();
    let max_suffix = current.len().min(next.len()) - prefix;
    let suffix = current.iter().rev().zip(next.iter().rev()).take(max_suffix).take_while(|(old, new)| old == new).count();
    list.state().splice(prefix..current.len() - suffix, next.len() - prefix - suffix);
    *current = next;
}

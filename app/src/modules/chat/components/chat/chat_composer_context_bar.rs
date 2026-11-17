use std::rc::Rc;

use gpui::{App, ElementId, IntoElement, RenderOnce, SharedString, Window};
use rise_theme::AppTheme;

use crate::core::widgets::inputs::composer_reply_bar::{ComposerReplyBar, ComposerReplyBarKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChatComposerContextBarKind {
    #[default]
    Reply,
    Edit,
}

#[derive(IntoElement)]
pub struct ChatComposerContextBar {
    id: ElementId,
    pub kind: ChatComposerContextBarKind,
    pub title: SharedString,
    pub subtitle: String,
    pub media_thumbnail_source: Option<String>,
    pub theme: AppTheme,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ChatComposerContextBar {
    pub fn new(
        id: impl Into<ElementId>,
        kind: ChatComposerContextBarKind,
        title: impl Into<SharedString>,
        subtitle: impl Into<String>,
        theme: AppTheme,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            subtitle: subtitle.into(),
            media_thumbnail_source: None,
            theme,
            on_close: Rc::new(on_close),
        }
    }

    pub fn media_thumbnail_source(mut self, source: Option<String>) -> Self {
        self.media_thumbnail_source = source;
        self
    }
}

impl RenderOnce for ChatComposerContextBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let kind = if self.kind == ChatComposerContextBarKind::Edit {
            ComposerReplyBarKind::Edit
        } else {
            ComposerReplyBarKind::Reply
        };
        let on_close = self.on_close;
        ComposerReplyBar::new(self.id, self.title, self.subtitle, move |window, cx| on_close(window, cx))
            .kind(kind)
            .media_thumbnail_source(self.media_thumbnail_source)
            .theme(Some(self.theme))
    }
}

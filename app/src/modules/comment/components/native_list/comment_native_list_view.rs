use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyView, App, ElementId, IntoElement, ParentElement, RenderOnce, Styled, Window, div};
use rise_theme::ThemeStore;
use rise_ui::swiftui::prelude::LifecycleModifiers;

use crate::modules::comment::components::native_list::comment_list_controller::CommentListController;
use crate::modules::comment::components::native_list::comment_list_models::{CommentListInput, CommentListRow};

/// bridge to the comment list controller, hands it one input per update and tears it down on
/// dismantle
#[derive(IntoElement)]
pub struct CommentNativeListView {
    id: ElementId,
    pub identity: String,
    pub rows: Vec<CommentListRow>,
    pub has_more: bool,
    pub is_loading_more: bool,
    pub top_inset: f32,
    pub bottom_inset: f32,
    pub is_interaction_locked: bool,
    pub focus_id: Option<String>,
    pub reveal_id: Option<String>,
    pub load_more: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_focus_resolved: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    pub on_focus_missing: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_revealed: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    pub row_content: Rc<dyn Fn(&CommentListRow, &mut Window, &mut App) -> AnyView>,
}

impl CommentNativeListView {
    #[track_caller]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: String,
        rows: Vec<CommentListRow>,
        has_more: bool,
        is_loading_more: bool,
        top_inset: f32,
        bottom_inset: f32,
        is_interaction_locked: bool,
        load_more: Rc<dyn Fn(&mut Window, &mut App)>,
        row_content: Rc<dyn Fn(&CommentListRow, &mut Window, &mut App) -> AnyView>,
    ) -> Self {
        Self {
            id: ElementId::CodeLocation(*Location::caller()),
            identity,
            rows,
            has_more,
            is_loading_more,
            top_inset,
            bottom_inset,
            is_interaction_locked,
            focus_id: None,
            reveal_id: None,
            load_more,
            on_focus_resolved: Rc::new(|_: &str, _: &mut Window, _: &mut App| {}),
            on_focus_missing: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_revealed: Rc::new(|_: &str, _: &mut Window, _: &mut App| {}),
            row_content,
        }
    }

    pub fn focus_id(mut self, focus_id: Option<String>) -> Self {
        self.focus_id = focus_id;
        self
    }

    pub fn reveal_id(mut self, reveal_id: Option<String>) -> Self {
        self.reveal_id = reveal_id;
        self
    }

    pub fn on_focus_resolved(mut self, on_focus_resolved: Rc<dyn Fn(&str, &mut Window, &mut App)>) -> Self {
        self.on_focus_resolved = on_focus_resolved;
        self
    }

    pub fn on_focus_missing(mut self, on_focus_missing: Rc<dyn Fn(&mut Window, &mut App)>) -> Self {
        self.on_focus_missing = on_focus_missing;
        self
    }

    /// controller identity when one call site builds more than one list
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn on_revealed(mut self, on_revealed: Rc<dyn Fn(&str, &mut Window, &mut App)>) -> Self {
        self.on_revealed = on_revealed;
        self
    }
}

impl RenderOnce for CommentNativeListView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let controller =
            window.use_keyed_state(self.id.clone(), cx, |window, cx| CommentListController::new(window, cx));
        let theme_store = ThemeStore::shared(cx);
        let (background_color, palette) = {
            let theme_store = theme_store.read(cx);
            (theme_store.current_theme().bg._100, theme_store.current_palette().clone())
        };

        let input = CommentListInput {
            identity: self.identity,
            rows: self.rows,
            has_more: self.has_more,
            is_loading_more: self.is_loading_more,
            top_inset: self.top_inset,
            bottom_inset: self.bottom_inset,
            is_interaction_locked: self.is_interaction_locked,
            background_color,
            palette,
            focus_id: self.focus_id,
            reveal_id: self.reveal_id,
            load_more: self.load_more,
            on_focus_resolved: self.on_focus_resolved,
            on_focus_missing: self.on_focus_missing,
            on_revealed: self.on_revealed,
            content: self.row_content,
        };
        controller.update(cx, |controller, cx| controller.submit(input, window, cx));

        let dismantled = controller.clone();
        let dismantle_id = ElementId::NamedChild(Arc::new(self.id), "dismantle".into());
        div().size_full().child(controller).on_disappear(dismantle_id, move |window, cx| {
            if let Some(window) = window {
                dismantled.update(cx, |controller, cx| controller.stop(window, cx));
            }
        })
    }
}

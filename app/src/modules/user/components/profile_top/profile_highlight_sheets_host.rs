use gpui::{App, Context, ElementId, Empty, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div};
use rise_ui::swiftui::binding::Binding;

use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerStack;
use crate::modules::story::stores::story::story_actions::story_wire_types::HighlightDTO;
use crate::modules::user::components::profile_top::highlight_create_sheet::HighlightCreateSheet;
use crate::modules::user::components::profile_top::highlight_edit_sheet::HighlightEditSheet;

/// the rows sheet state, read and written through its own keyed state
#[derive(Clone)]
pub struct ProfileHighlightSheetsMount {
    pub row_id: ElementId,
    pub user_id: String,
    pub is_creating: Binding<bool>,
    pub is_editing: Binding<bool>,
    pub editing_highlight: Binding<Option<HighlightDTO>>,
}

impl ProfileHighlightSheetsMount {
    fn is_presenting(&self, cx: &App) -> bool {
        self.is_creating.get(cx) || self.is_editing.get(cx)
    }
}

pub struct ProfileHighlightSheetsPresenter {
    mount: Option<ProfileHighlightSheetsMount>,
}

impl ProfileHighlightSheetsPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { mount: None }
    }

    /// a row appeared, it takes the sheets unless another row has one open
    pub fn offer(mount: ProfileHighlightSheetsMount, cx: &mut App) {
        let presenter = Self::shared(cx);
        let is_busy = presenter
            .read(cx)
            .mount
            .as_ref()
            .is_some_and(|current| current.row_id != mount.row_id && current.is_presenting(cx));
        if is_busy {
            return;
        }
        presenter.update(cx, |presenter, cx| {
            presenter.mount = Some(mount);
            cx.notify();
        });
    }

    /// a row is about to raise one of the sheets
    pub fn take(mount: ProfileHighlightSheetsMount, cx: &mut App) {
        Self::shared(cx).update(cx, |presenter, cx| {
            presenter.mount = Some(mount);
            cx.notify();
        });
    }
}

pub struct ProfileHighlightSheetsHost {
    presenter: Entity<ProfileHighlightSheetsPresenter>,
    _presenter_observer: Subscription,
}

impl ProfileHighlightSheetsHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let presenter = ProfileHighlightSheetsPresenter::shared(cx);
        let observer = cx.observe(&presenter, |_, _, cx| cx.notify());
        Self { presenter, _presenter_observer: observer }
    }

    /// true when no row flag raises a sheet and no sheet layer is closing,
    /// so the root may draw this host cached
    pub fn is_idle(&self, cx: &App) -> bool {
        match self.presenter.read(cx).mount.as_ref() {
            None => true,
            Some(mount) => !mount.is_presenting(cx) && SheetLayerStack::is_empty(cx),
        }
    }
}

impl Render for ProfileHighlightSheetsHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(mount) = self.presenter.read(cx).mount.clone() else {
            return Empty.into_any_element();
        };
        let editing_highlight = mount.editing_highlight.get(cx);
        let host_id = ElementId::from("profile-highlight-sheets");
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_0()
            .child(HighlightCreateSheet::new((host_id.clone(), "create"), mount.is_creating, mount.user_id.clone()))
            .child(HighlightEditSheet::new((host_id, "edit"), mount.is_editing, editing_highlight, mount.user_id))
            .into_any_element()
    }
}

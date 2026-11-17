use futures::channel::oneshot;
use gpui::{
    App, Context, Empty, Entity, IntoElement, ParentElement, PromptButton, PromptLevel, Render, Styled, Subscription,
    Task, Window, div,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::guide_overlay_ui::guide_overlay_window::GuideOverlayHost;

use crate::modules::post::components::collaborators::post_collaborators_list_sheet::{
    CoauthorsSheet, TaggedPeopleSheet,
};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostCollaboratorSurface {
    Coauthors,
    TaggedPeople,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostCollaboratorConfirmKind {
    AcceptInvite,
    LeaveCoauthorship,
    RemoveOwnTag,
}

impl PostCollaboratorConfirmKind {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::AcceptInvite => "acceptInvite",
            Self::LeaveCoauthorship => "leaveCoauthorship",
            Self::RemoveOwnTag => "removeOwnTag",
        }
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            Self::AcceptInvite => "post_coauthor_accept_title",
            Self::LeaveCoauthorship => "post_coauthor_leave_title",
            Self::RemoveOwnTag => "post_tag_remove_self_title",
        }
    }

    pub const fn message_key(self) -> &'static str {
        match self {
            Self::AcceptInvite => "post_coauthor_accept_message",
            Self::LeaveCoauthorship => "post_coauthor_leave_message",
            Self::RemoveOwnTag => "post_tag_remove_self_message",
        }
    }

    pub const fn confirm_key(self) -> &'static str {
        match self {
            Self::AcceptInvite => "post_coauthor_accept_button",
            Self::LeaveCoauthorship => "post_coauthor_leave_action",
            Self::RemoveOwnTag => "post_tag_remove_self_action",
        }
    }

    pub const fn is_destructive(self) -> bool {
        !matches!(self, Self::AcceptInvite)
    }
}

/// single presentation point for both coauthor surfaces and the three confirmations
/// sheets and alert live in one host not the post view
/// same actions fire from feed, reels and the more sheet
pub struct PostCollaboratorsPresenter {
    surface: Option<PostCollaboratorSurface>,
    post_id: String,
    viewer_is_author: bool,
    confirm: Option<PostCollaboratorConfirmKind>,
    is_closing: bool,
    confirm_post_id: String,
}

impl PostCollaboratorsPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            surface: None,
            post_id: String::new(),
            viewer_is_author: false,
            confirm: None,
            is_closing: false,
            confirm_post_id: String::new(),
        }
    }

    pub fn surface(&self) -> Option<PostCollaboratorSurface> {
        self.surface
    }

    pub fn post_id(&self) -> &str {
        &self.post_id
    }

    pub fn viewer_is_author(&self) -> bool {
        self.viewer_is_author
    }

    pub fn confirm(&self) -> Option<PostCollaboratorConfirmKind> {
        self.confirm
    }

    pub fn is_closing(&self) -> bool {
        self.is_closing
    }

    pub fn mounts_coauthors(&self) -> bool {
        self.surface == Some(PostCollaboratorSurface::Coauthors)
    }

    pub fn mounts_tagged_people(&self) -> bool {
        self.surface == Some(PostCollaboratorSurface::TaggedPeople)
    }

    pub fn has_interactive_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub fn is_sheet_presented(&self) -> bool {
        self.surface.is_some() && !self.is_closing
    }

    pub fn present_coauthors(&mut self, post_id: &str, viewer_is_author: bool, cx: &mut Context<Self>) {
        self.present(PostCollaboratorSurface::Coauthors, post_id, viewer_is_author, cx);
    }

    pub fn present_tagged_people(&mut self, post_id: &str, viewer_is_author: bool, cx: &mut Context<Self>) {
        self.present(PostCollaboratorSurface::TaggedPeople, post_id, viewer_is_author, cx);
    }

    fn present(
        &mut self,
        surface: PostCollaboratorSurface,
        post_id: &str,
        viewer_is_author: bool,
        cx: &mut Context<Self>,
    ) {
        let trimmed = trimming_whitespaces_and_newlines(post_id);
        if trimmed.is_empty() || self.surface.is_some() {
            return;
        }
        self.post_id = trimmed.to_owned();
        self.viewer_is_author = viewer_is_author;
        self.is_closing = false;
        self.surface = Some(surface);
        cx.notify();
    }

    pub fn set_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented {
            return;
        }
        self.dismiss(cx);
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.surface.is_none() || self.is_closing {
            return;
        }
        self.is_closing = true;
        cx.notify();
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        self.surface = None;
        self.is_closing = false;
        self.post_id = String::new();
        self.viewer_is_author = false;
        cx.notify();
    }

    pub fn ask(&mut self, kind: PostCollaboratorConfirmKind, post_id: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(post_id);
        if trimmed.is_empty() {
            return;
        }
        self.confirm_post_id = trimmed.to_owned();
        self.confirm = Some(kind);
        cx.notify();
    }

    pub fn cancel_confirm(&mut self, cx: &mut Context<Self>) {
        self.confirm = None;
        self.confirm_post_id = String::new();
        cx.notify();
    }

    pub fn perform_confirm(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.confirm else {
            return;
        };
        if self.confirm_post_id.is_empty() {
            return;
        }
        let target = std::mem::take(&mut self.confirm_post_id);
        self.confirm = None;
        cx.notify();
        PostInteractionsStore::shared(cx).update(cx, |interactions, cx| match kind {
            PostCollaboratorConfirmKind::AcceptInvite => interactions.accept_coauthor_invite(&target, cx),
            PostCollaboratorConfirmKind::LeaveCoauthorship => interactions.leave_coauthorship(&target, cx),
            PostCollaboratorConfirmKind::RemoveOwnTag => interactions.remove_own_tag(&target, cx),
        });
    }
}

/// holds both coauthor sheets and the shared confirmation
/// with none open it collapses to zero and takes no touches
pub struct PostCollaboratorsHost {
    presenter: Entity<PostCollaboratorsPresenter>,
    alert_task: Option<Task<()>>,
    _presenter_observer: Subscription,
}

impl PostCollaboratorsHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let presenter = PostCollaboratorsPresenter::shared(cx);
        let observer = cx.observe(&presenter, |host, _, cx| {
            host.sync_alert(cx);
            cx.notify();
        });
        Self { presenter, alert_task: None, _presenter_observer: observer }
    }

    fn sync_alert(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.presenter.read(cx).confirm() else {
            return;
        };
        if self.alert_task.is_some() {
            return;
        }
        let guide = GuideOverlayHost::shared(cx);
        let host = if guide.is_presenting() {
            guide.alert_host()
        } else {
            cx.active_window().or_else(|| cx.windows().first().copied())
        };
        let level = if kind.is_destructive() { PromptLevel::Warning } else { PromptLevel::Info };
        let buttons = [PromptButton::ok(tr(kind.confirm_key())), PromptButton::cancel(tr("common_cancel"))];
        let title = tr(kind.title_key());
        let message = tr(kind.message_key());
        let receiver: Option<oneshot::Receiver<usize>> = host.and_then(|host| {
            host.update(cx, |_, window, cx| window.prompt(level, &title, Some(message.as_str()), &buttons, cx)).ok()
        });
        let Some(receiver) = receiver else {
            self.presenter.update(cx, |presenter, cx| presenter.cancel_confirm(cx));
            return;
        };
        self.alert_task = Some(cx.spawn(async move |this, cx| {
            let answer = receiver.await.ok();
            let _ = this.update(cx, |host, cx| {
                host.alert_task = None;
                host.presenter.update(cx, |presenter, cx| {
                    if answer == Some(0) {
                        presenter.perform_confirm(cx);
                    } else {
                        presenter.cancel_confirm(cx);
                    }
                });
            });
        }));
    }
}

impl Render for PostCollaboratorsHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let presenter = self.presenter.read(cx);
        if !presenter.has_interactive_surface() {
            return Empty.into_any_element();
        }
        let mounts_coauthors = presenter.mounts_coauthors();
        let mounts_tagged_people = presenter.mounts_tagged_people();
        let mut host = div().absolute().inset_0();
        if mounts_coauthors {
            host = host.child(CoauthorsSheet);
        }
        if mounts_tagged_people {
            host = host.child(TaggedPeopleSheet);
        }
        host.into_any_element()
    }
}

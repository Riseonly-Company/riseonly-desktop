use std::rc::Rc;

use gpui::{
    AnyView, AnyWindowHandle, App, Context, Entity, EntityId, Render, StyleRefinement, Subscription, Window, anchored, deferred,
};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::frame_request::{opaque_cover_layer, release_opaque_cover};
use rise_ui::swiftui::prelude::*;
use rise_ui::window_cover::WindowCoverFrame;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerFloorKey;
use crate::core::lib::sai::sai::Sai;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::editor::media_editor_view_hooks::media_editor_view_hooks;
use crate::modules::media_picker::editor::media_reedit_launcher::MediaReeditLauncher;
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionEdit;
use crate::modules::media_picker::ui::media_picker_session::{MediaPickerExportProgressPresenter, MediaPickerSession};

#[derive(Clone)]
pub struct MediaEditorOutcome {
    pub edit: MediaSelectionEdit,
    pub immediate_send: bool,
    pub selects_asset: bool,
}

pub type MediaEditorWindowOutcomeHandler = Rc<dyn Fn(MediaEditorOutcome, MediaAsset, &mut Window, &mut App)>;

struct MediaEditorWindowPresenterActiveContext {
    session: Entity<MediaPickerSession>,
    configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
    on_outcome: MediaEditorWindowOutcomeHandler,
}

/// keeps the editor above everything in the window
/// the host is a layer that MediaEditorWindowLayer draws over the window content
pub struct MediaEditorWindowPresenter {
    context: Option<MediaEditorWindowPresenterActiveContext>,
    hosting_controller: Option<Entity<MediaEditorWindowHostView>>,
    hosting_window: Option<AnyWindowHandle>,
    pending_presentation: bool,
    presentation_tracking: Option<Subscription>,
}

impl MediaEditorWindowPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            context: None,
            hosting_controller: None,
            hosting_window: None,
            pending_presentation: false,
            presentation_tracking: None,
        }
    }

    /// the layer to draw over this window, if the editor was presented from it
    fn host(&self, window: &Window) -> Option<Entity<MediaEditorWindowHostView>> {
        let hosting_window = self.hosting_window?;
        if hosting_window.window_id() != window.window_handle().window_id() {
            return None;
        }
        self.hosting_controller.clone()
    }

    fn is_editor_presented(&self, cx: &App) -> bool {
        self.context
            .as_ref()
            .is_some_and(|context| context.session.read(cx).editor_presentation.read(cx).is_presented)
    }

    pub fn sync_presentation(
        &mut self,
        session: Entity<MediaPickerSession>,
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        on_outcome: MediaEditorWindowOutcomeHandler,
        cx: &mut Context<Self>,
    ) {
        let wants_presentation = session.read(cx).editor_presentation.read(cx).is_presented;
        let detail = format!(
            "wants={wants_presentation} session={} ctx={} host={}",
            Self::tag(session.entity_id()),
            self.context_tag(cx),
            self.host_tag()
        );
        MediaEditorWindowLog::event("sync", || detail, cx);

        if wants_presentation {
            self.track_presentation_state(&session, cx);
            self.context =
                Some(MediaEditorWindowPresenterActiveContext { session, configuration, theme_store, on_outcome });
            self.present_host_if_needed(cx);
        } else {
            if let Some(owner) = self.context.as_ref().map(|context| &context.session)
                && *owner != session
                && !owner.read(cx).did_finish()
            {
                return;
            }
            self.dismiss_host("sync-not-presented", cx);
            self.context = None;
        }
    }

    pub fn teardown(&mut self, session: &Entity<MediaPickerSession>, cx: &mut Context<Self>) {
        let matches_owner = self.context.as_ref().is_some_and(|context| context.session == *session);
        let detail = format!(
            "session={} matches={matches_owner} ctx={} host={}",
            Self::tag(session.entity_id()),
            self.context_tag(cx),
            self.host_tag()
        );
        MediaEditorWindowLog::event("teardown", || detail, cx);

        if !matches_owner {
            self.dismiss_orphaned_host("owner-mismatch", cx);
            return;
        }

        self.dismiss_host("teardown", cx);
        self.context = None;
    }

    pub fn dismiss_immediately(
        &mut self,
        then: impl FnOnce(&mut Window, &mut App) + 'static,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.context = None;
        self.dismiss_host("immediate-send", cx);
        window.defer(cx, then);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        let detail = format!("ctx={} host={}", self.context_tag(cx), self.host_tag());
        MediaEditorWindowLog::event("resetAccountState", || detail, cx);
        self.dismiss_host("account-reset", cx);
        self.context = None;
    }

    fn dismiss_orphaned_host(&mut self, reason: &str, cx: &mut Context<Self>) {
        if self.hosting_controller.is_none() {
            return;
        }
        let owner_is_gone = self.context.as_ref().is_none_or(|context| context.session.read(cx).did_finish());
        if !owner_is_gone {
            return;
        }
        let detail = format!("reason={reason} ctx={} host={}", self.context_tag(cx), self.host_tag());
        MediaEditorWindowLog::event("orphan", || detail, cx);
        self.dismiss_host(&format!("orphan-{reason}"), cx);
        self.context = None;
    }

    fn track_presentation_state(&mut self, session: &Entity<MediaPickerSession>, cx: &mut Context<Self>) {
        let state = session.read(cx).editor_presentation.clone();
        let session = session.downgrade();
        self.presentation_tracking = Some(cx.observe(&state, move |this: &mut Self, state, cx| {
            if state.read(cx).is_presented {
                return;
            }
            let Some(session) = session.upgrade() else {
                return;
            };
            if !this.context.as_ref().is_some_and(|context| context.session == session) {
                return;
            }
            let detail = format!("session={} host={}", Self::tag(session.entity_id()), this.host_tag());
            MediaEditorWindowLog::event("observed-close", || detail, cx);
            this.dismiss_host("observed-isPresented-false", cx);
            this.context = None;
        }));
    }

    fn present_host_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.hosting_controller.is_some() || self.pending_presentation || self.context.is_none() {
            let detail = format!(
                "host={} pending={} ctx={}",
                self.host_tag(),
                self.pending_presentation,
                self.context_tag(cx)
            );
            MediaEditorWindowLog::event("present-skip", || detail, cx);
            return;
        }

        self.pending_presentation = true;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.present_host(cx));
        })
        .detach();
    }

    fn present_host(&mut self, cx: &mut Context<Self>) {
        self.pending_presentation = false;
        let context_tag = self.context_tag(cx);

        let Some(context) = self.context.as_ref().filter(|_| self.hosting_controller.is_none()) else {
            let detail = format!("reason=stale host={} ctx={context_tag}", self.host_tag());
            MediaEditorWindowLog::event("present-abort", || detail, cx);
            return;
        };
        let session = context.session.clone();
        let is_presented = session.read(cx).editor_presentation.read(cx).is_presented;
        if !is_presented {
            MediaEditorWindowLog::event("present-abort", || format!("reason=not-presented ctx={context_tag}"), cx);
            self.context = None;
            return;
        }
        if session.read(cx).did_finish() {
            MediaEditorWindowLog::event("present-abort", || format!("reason=session-finished ctx={context_tag}"), cx);
            self.context = None;
            return;
        }
        let Some(presenter) = MediaPickerPresenterResolver::top_presenter(cx) else {
            MediaEditorWindowLog::event("present-abort", || format!("reason=no-presenter ctx={context_tag}"), cx);
            return;
        };

        let configuration = context.configuration.clone();
        let theme_store = context.theme_store.clone();
        let on_outcome = context.on_outcome.clone();
        let host = cx.new(|_| MediaEditorWindowHostView { session, configuration, theme_store, on_outcome });
        let detail = format!("host={} ctx={context_tag}", Self::tag(host.entity_id()));
        self.hosting_controller = Some(host);
        self.hosting_window = Some(presenter);
        MediaEditorWindowLog::event("present", || detail, cx);
        cx.notify();
    }

    fn dismiss_host(&mut self, reason: &str, cx: &mut Context<Self>) {
        let Some(hosting_controller) = self.hosting_controller.take() else {
            let detail = format!("reason={reason} ctx={}", self.context_tag(cx));
            MediaEditorWindowLog::event("dismiss-noop", || detail, cx);
            return;
        };
        let detail = format!("reason={reason} host={}", Self::tag(hosting_controller.entity_id()));
        MediaEditorWindowLog::event("dismiss", || detail, cx);
        self.hosting_window = None;
        cx.notify();
    }

    fn context_tag(&self, cx: &App) -> String {
        let Some(context) = &self.context else {
            return "-".to_owned();
        };
        format!("{}/finished={}", Self::tag(context.session.entity_id()), context.session.read(cx).did_finish())
    }

    fn host_tag(&self) -> String {
        self.hosting_controller.as_ref().map_or_else(|| "-".to_owned(), |host| Self::tag(host.entity_id()))
    }

    fn tag(id: EntityId) -> String {
        format!("{:06X}", id.as_u64() & 0xFF_FFFF)
    }
}

pub enum MediaEditorWindowLog {}

impl MediaEditorWindowLog {
    pub fn event(name: &str, detail: impl FnOnce() -> String, cx: &mut App) {
        let line = format!("{name} | {}", detail());
        tracing::warn!(target: "riseonly::X908", "{line}");
        Sai::log("MediaEditorWindow", move || line, SaiLogKind::Ui, cx);
    }

    /// state event, state lives on the main thread so no off main branch
    pub fn state_event(name: &str, detail: impl FnOnce() -> String, cx: &mut App) {
        Self::event(name, detail, cx);
    }
}

struct MediaEditorWindowHostView {
    session: Entity<MediaPickerSession>,
    configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
    on_outcome: MediaEditorWindowOutcomeHandler,
}

impl Render for MediaEditorWindowHostView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        div().absolute().inset_0().children(media_editor_view_hooks().map(|hooks| {
            (hooks.presentation_host)(
                session.editor_presentation.clone(),
                self.theme_store.clone(),
                self.configuration.clone(),
                session.selection.clone(),
                session.caption.clone(),
                self.on_outcome.clone(),
                session.fetcher.clone(),
            )
        }))
    }
}

/// the windows top layer for the editor and for re-editing
/// root view keeps it as its last child, draws nothing while neither is up
#[derive(IntoElement)]
pub struct MediaEditorWindowLayer;

impl MediaEditorWindowLayer {
    pub const LAYER_PRIORITY: usize = 1500;
}

impl RenderOnce for MediaEditorWindowLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let layer = window.use_keyed_state("media-editor-window-layer", cx, |_, cx| MediaEditorWindowLayerView::new(cx));
        if layer.read(cx).shows {
            AnyView::from(layer).into_any_element()
        } else {
            AnyView::from(layer).cached(StyleRefinement::default().absolute().top_0().left_0()).into_any_element()
        }
    }
}

struct MediaEditorWindowLayerView {
    shows: bool,
    _observations: [Subscription; 2],
}

impl MediaEditorWindowLayerView {
    fn new(cx: &mut Context<Self>) -> Self {
        let presenter = MediaEditorWindowPresenter::shared(cx);
        let reedit = MediaReeditLauncher::shared(cx);
        Self {
            shows: false,
            _observations: [
                cx.observe(&presenter, |_, _, cx| cx.notify()),
                cx.observe(&reedit, |_, _, cx| cx.notify()),
            ],
        }
    }
}

impl Render for MediaEditorWindowLayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = MediaEditorWindowPresenter::shared(cx).read(cx).host(window);
        let reedit = MediaReeditLauncher::shared(cx).read(cx).hosts(window);
        self.shows = editor.is_some() || !reedit.is_empty();
        if !self.shows {
            release_opaque_cover(window, cx);
            return div().into_any_element();
        }
        let export_overlay = if !reedit.is_empty() || MediaEditorWindowPresenter::shared(cx).read(cx).is_editor_presented(cx) {
            MediaPickerExportProgressPresenter::overlay(cx)
        } else {
            None
        };
        let cover = WindowCoverFrame::resolve(window, cx);
        deferred(
            anchored().position(cover.origin).child(
                env_scope::<SheetLayerFloorKey>(MediaEditorWindowLayer::LAYER_PRIORITY).child(
                    div()
                        .relative()
                        .w(cover.size.width)
                        .h(cover.size.height)
                        .child(opaque_cover_layer())
                        .children(editor)
                        .children(reedit)
                        .children(export_overlay),
                ),
            ),
        )
        .with_priority(MediaEditorWindowLayer::LAYER_PRIORITY)
        .into_any_element()
    }
}

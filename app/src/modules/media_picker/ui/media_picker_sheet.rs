use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, ElementId, Empty, Entity, EntityId, ExternalPaths, FontWeight, IntoElement, RenderOnce, SharedString,
    Subscription, Window, anchored, deferred, transparent_black,
};
use rise_ui::window_cover::WindowCoverFrame;
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationContentInteraction, PresentationDetent,
};
use rise_ui::liquid_chrome::LiquidChrome;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition, TransitionModifiers};

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SHEET_LAYER_BASE_PRIORITY;
use crate::core::ui::rich_text::document::RiseDocument;
use crate::core::ui::rich_text::document::rise_document_editor_model::RiseDocumentEditorModel;
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::core::ui::rich_text::document::rise_document_editor_screen::RiseDocumentEditorScreen;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatBottomBarMetrics, ChatDraftCustomEmoji,
};
use crate::modules::chat::stores::message::message_actions::message_poll_codec::MessagePollDraft;
use crate::modules::media_picker::api::media_picker_configuration::{MediaPickerConfiguration, MediaPickerSourceTab};
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerResult, MediaPickerTrackSelection,
};
use crate::modules::media_picker::editor::media_editor_presentation_state::MediaEditorPresentationState;
use crate::modules::media_picker::editor::media_editor_view_hooks::media_editor_view_hooks;
use crate::modules::media_picker::editor::media_editor_window_presenter::{
    MediaEditorOutcome, MediaEditorWindowLog, MediaEditorWindowPresenter,
};
use crate::modules::media_picker::export::media_picker_export_progress_overlay::MediaPickerExportProgressOverlay;
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::media_picker::shared::media_picker_recent_files::{MediaPickerRecentFile, MediaPickerRecentKind};
use crate::modules::media_picker::ui::media_backdrop_hooks::{MEDIA_SEND_BACKDROP_BLUR, media_send_backdrop};
use crate::modules::media_picker::ui::media_picker_attachment_sources::{
    MediaPickerAudioMetadataEditor, MediaPickerDocumentSourcePage,
};
use crate::modules::media_picker::ui::media_picker_caption_bar_hooks::media_picker_caption_bar_hooks;
use crate::modules::media_picker::ui::media_picker_grid_view::MEDIA_PICKER_REFRESH_CELL_FRAMES;
use crate::modules::media_picker::ui::media_picker_location_source_page::MediaPickerLocationSourcePage;
use crate::modules::media_picker::ui::media_picker_poll_composer::MediaPickerPollComposer;
use crate::modules::media_picker::ui::media_picker_screen::{MediaPickerLayout, MediaPickerScreen};
use crate::modules::media_picker::ui::media_picker_session::{MediaPickerExternalVideoDraft, MediaPickerSession};

const REFRESH_CELL_FRAMES_DELAY_SECONDS: f64 = 0.36;
const POLL_COMPOSER_ANIMATION_SECONDS: f64 = 0.25;
const SOURCE_TAB_BAR_HEIGHT: f32 = 49.0;
const EXTERNAL_VIDEO_COVER_PRIORITY: usize = SHEET_LAYER_BASE_PRIORITY + 64;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;
type SheetComplete = Rc<dyn Fn(MediaPickerResult, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct MediaPickerSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
    caption: Option<Binding<String>>,
    caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    initial_track: Option<MediaPickerTrackSelection>,
    experimental_bottom_sheet: bool,
    pending_external_paths: Option<Rc<RefCell<Vec<PathBuf>>>>,
    on_complete: SheetComplete,
    on_dismissed: SheetAction,
}

impl MediaPickerSheet {
    /// defaults are no caption, no custom emojis and no initial track
    /// experimental_bottom_sheet true, on_dismissed empty
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        on_complete: impl Fn(MediaPickerResult, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_presented,
            configuration,
            theme_store,
            caption: None,
            caption_custom_emojis: None,
            initial_track: None,
            experimental_bottom_sheet: true,
            pending_external_paths: None,
            on_complete: Rc::new(on_complete),
            on_dismissed: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn caption(mut self, caption: Option<Binding<String>>) -> Self {
        self.caption = caption;
        self
    }

    pub fn caption_custom_emojis(mut self, caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>) -> Self {
        self.caption_custom_emojis = caption_custom_emojis;
        self
    }

    pub fn initial_track(mut self, initial_track: Option<MediaPickerTrackSelection>) -> Self {
        self.initial_track = initial_track;
        self
    }

    pub fn experimental_bottom_sheet(mut self, experimental_bottom_sheet: bool) -> Self {
        self.experimental_bottom_sheet = experimental_bottom_sheet;
        self
    }

    /// desktop only, files the owner stages before presenting
    /// each presentation takes them once so a stage for one opening cant be sent by a later one
    pub fn pending_external_paths(mut self, pending_external_paths: Rc<RefCell<Vec<PathBuf>>>) -> Self {
        self.pending_external_paths = Some(pending_external_paths);
        self
    }

    pub fn on_dismissed(mut self, on_dismissed: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismissed = Rc::new(on_dismissed);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn native_presentation(&self, wiring: &MediaPickerSheetWiring, cx: &App) -> AnyElement {
        let session = wiring.session(cx);
        let theme = self.theme_store.read(cx).current_theme();
        let surface = theme.bg._100;
        let editor_is_presented = session.as_ref().is_some_and(|session| {
            let presentation = &session.read(cx).editor_presentation;
            gpui::hear_entity_through_events_only(presentation.entity_id());
            presentation.read(cx).is_presented
        });
        let (content_wiring, on_prepare, on_dismiss) = (wiring.clone(), wiring.clone(), wiring.clone());
        let content_id = self.child_id("runtime");
        NativeBottomSheetPresentationHost::new(self.child_id("native-sheet"), self.is_presented.clone(), move |_, cx| {
            match content_wiring.session(cx) {
                Some(session) => content_wiring.runtime(content_id.clone(), session).into_any_element(),
                None => Empty.into_any_element(),
            }
        })
        .is_content_ready(session.is_some())
        .on_prepare(move |_, cx| on_prepare.boot_session(cx))
        .on_dismiss(move |window, cx| {
            on_dismiss.handle_sheet_dismiss(cx);
            (on_dismiss.on_dismissed)(window, cx);
        })
        .presentation_detents([PresentationDetent::Fraction(0.56), PresentationDetent::Fraction(0.92)])
        .presentation_drag_indicator(true)
        .presentation_background(surface)
        .presentation_corner_radius(16.0)
        .presentation_content_interaction(PresentationContentInteraction::Scrolls)
        .interactive_dismiss_disabled(editor_is_presented)
        .into_any_element()
    }

    fn experimental_presentation(&self, wiring: &MediaPickerSheetWiring, window: &mut Window, cx: &mut App) -> AnyElement {
        let boot = wiring.clone();
        on_change(
            self.child_id("is-presented"),
            self.is_presented.get(cx),
            true,
            window,
            cx,
            move |_, open, _, cx| {
                tracing::warn!(target: "riseonly::X808", open = *open, "picker sheet flag changed");
                if *open {
                    boot.boot_session(cx);
                }
            },
        );

        let sheet = wiring.session(cx).map(|session| {
            let surface = LiquidChrome::surface_fill(self.theme_store.read(cx).current_theme(), 0);
            let presentation = &session.read(cx).editor_presentation;
            gpui::hear_entity_through_events_only(presentation.entity_id());
            let editor_is_presented = presentation.read(cx).is_presented;
            let editor_covers_sheet = MediaPickerSheetRuntimeState::editor_covers_sheet(presentation.read(cx));
            let (content_wiring, content_session) = (wiring.clone(), session);
            let content_id = self.child_id("runtime");
            let (on_dismiss, on_did_dismiss, on_refused) = (wiring.clone(), wiring.clone(), wiring.clone());
            let (edge_theme, edge_session) = (self.theme_store.clone(), content_session.clone());
            let edge_handle_clearance =
                if wiring.uses_native_sheet_presentation { 0.0 } else { MediaPickerScreen::SHEET_HANDLE_CLEARANCE };
            BottomSheetUi::new(
                self.child_id("sheet"),
                self.is_presented.clone(),
                move |_, _| content_wiring.runtime(content_id.clone(), content_session.clone()),
                |_, _| Empty,
            )
            .snap_fractions(vec![0.67, 0.92])
            .content_is_edge_to_edge(true)
            .experimental_bottom_sheet(true)
            .experimental_uses_liquid_glass_surface(false)
            .full_page_allows_swipe_dismiss(!editor_is_presented)
            .full_page_shows_drag_indicator(true)
            .full_page_hides_footer(true)
            .full_page_avoids_keyboard_when_footer_hidden(self.configuration.allows_caption)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .full_page_interaction_locked(editor_is_presented)
            .sheet_background(Some(if editor_covers_sheet { transparent_black() } else { surface }))
            .modal_chrome_hidden(editor_covers_sheet)
            .modal_edge_underlay(move |_, cx| {
                if edge_session.read(cx).source_tab != MediaPickerSourceTab::Gallery {
                    return Empty.into_any_element();
                }
                MediaPickerScreen::modal_edge_header_chrome(edge_handle_clearance, &edge_theme, cx)
            })
            .backdrop_color(Some(media_send_backdrop(cx)))
            .backdrop_blur(Some(MEDIA_SEND_BACKDROP_BLUR))
            .backdrop_opacity(Some(1.0))
            .footer_background(Some(transparent_black()))
            .on_dismiss(move |_, cx| on_dismiss.handle_experimental_sheet_dismiss(cx))
            .on_did_dismiss(move |window, cx| on_did_dismiss.handle_experimental_presentation_dismissed(window, cx))
            .on_presentation_refused(move |window, cx| {
                on_refused.is_presented.set(false, window, cx);
                on_refused.handle_sheet_dismiss(cx);
                (on_refused.on_dismissed)(window, cx);
            })
        });

        let disappear = wiring.clone();
        div()
            .absolute()
            .inset_0()
            .children(sheet)
            .on_disappear(self.child_id("presentation"), move |_, cx| {
                if disappear.session(cx).is_some() {
                    disappear.handle_sheet_dismiss(cx);
                }
            })
            .into_any_element()
    }
}

impl RenderOnce for MediaPickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| MediaPickerSheetState { session: None });
        let wiring = MediaPickerSheetWiring {
            state,
            is_presented: self.is_presented.clone(),
            configuration: self.configuration.clone(),
            uses_native_sheet_presentation: !self.experimental_bottom_sheet,
            theme_store: self.theme_store.clone(),
            caption: self.caption.clone(),
            caption_custom_emojis: self.caption_custom_emojis.clone(),
            initial_track: self.initial_track.clone(),
            pending_external_paths: self.pending_external_paths.clone(),
            on_complete: self.on_complete.clone(),
            on_dismissed: self.on_dismissed.clone(),
        };
        if self.experimental_bottom_sheet {
            self.experimental_presentation(&wiring, window, cx)
        } else {
            self.native_presentation(&wiring, cx)
        }
    }
}

struct MediaPickerSheetState {
    session: Option<Entity<MediaPickerSession>>,
}

/// what the sheet closures need from the view, cloned out of it
#[derive(Clone)]
struct MediaPickerSheetWiring {
    state: Entity<MediaPickerSheetState>,
    is_presented: Binding<bool>,
    configuration: MediaPickerConfiguration,
    uses_native_sheet_presentation: bool,
    theme_store: Entity<ThemeStore>,
    caption: Option<Binding<String>>,
    caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    initial_track: Option<MediaPickerTrackSelection>,
    pending_external_paths: Option<Rc<RefCell<Vec<PathBuf>>>>,
    on_complete: SheetComplete,
    on_dismissed: SheetAction,
}

impl MediaPickerSheetWiring {
    fn session(&self, cx: &App) -> Option<Entity<MediaPickerSession>> {
        self.state.read(cx).session.clone()
    }

    fn runtime(&self, id: ElementId, session: Entity<MediaPickerSession>) -> MediaPickerSheetRuntime {
        MediaPickerSheetRuntime {
            id,
            session,
            is_presented: self.is_presented.clone(),
            configuration: self.configuration.clone(),
            uses_native_sheet_presentation: self.uses_native_sheet_presentation,
            theme_store: self.theme_store.clone(),
        }
    }

    fn set_session(&self, session: Option<Entity<MediaPickerSession>>, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.session = session;
            cx.notify();
        });
    }

    fn handle_sheet_dismiss(&self, cx: &mut App) {
        self.tear_down_session(cx);
        self.set_session(None, cx);
    }

    fn handle_experimental_sheet_dismiss(&self, cx: &mut App) {
        self.tear_down_session(cx);
    }

    fn handle_experimental_presentation_dismissed(&self, window: &mut Window, cx: &mut App) {
        tracing::warn!(target: "riseonly::X808", "picker sheet did dismiss");
        self.set_session(None, cx);
        (self.on_dismissed)(window, cx);
    }

    fn tear_down_session(&self, cx: &mut App) {
        let Some(session) = self.session(cx) else {
            return;
        };
        let editor_presentation = session.read(cx).editor_presentation.clone();
        let is_presented = editor_presentation.read(cx).is_presented;
        let did_finish = session.read(cx).did_finish();
        MediaEditorWindowLog::state_event(
            "sheet-teardown",
            move || format!("presented={is_presented} finished={did_finish}"),
            cx,
        );
        if is_presented {
            editor_presentation.update(cx, |presentation, cx| presentation.complete_flyback_handoff(cx));
        }
        MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| presenter.teardown(&session, cx));
        if !did_finish {
            session.update(cx, |session, cx| session.cancel(cx));
        }
    }

    fn boot_session(&self, cx: &mut App) {
        let dropped = self
            .pending_external_paths
            .as_ref()
            .map(|pending| std::mem::take(&mut *pending.borrow_mut()))
            .unwrap_or_default();
        if let Some(session) = self.session(cx) {
            if !dropped.is_empty() && !session.read(cx).did_finish() {
                session.update(cx, |session, cx| session.receive_external_paths(dropped, cx));
            }
            return;
        }
        let (on_complete, is_presented) = (self.on_complete.clone(), self.is_presented.clone());
        let configuration = self.configuration.clone();
        let theme_store = self.theme_store.clone();
        let caption = self.caption.clone();
        let caption_custom_emojis = self.caption_custom_emojis.clone();
        let initial_track = self.initial_track.clone();
        let session = cx.new(|cx| {
            MediaPickerSession::new(
                configuration,
                theme_store,
                caption,
                caption_custom_emojis,
                initial_track,
                Rc::new(move |result: MediaPickerResult, window: &mut Window, cx: &mut App| {
                    on_complete(result, window, cx);
                    is_presented.set(false, window, cx);
                    tracing::warn!(target: "riseonly::X808", still_presented = is_presented.get(cx), "picker session completed");
                }),
                cx,
            )
        });
        if !dropped.is_empty() {
            session.update(cx, |session, cx| session.receive_external_paths(dropped, cx));
        }
        self.set_session(Some(session), cx);

        let is_presented = self.is_presented.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(REFRESH_CELL_FRAMES_DELAY_SECONDS)).await;
            cx.update(|cx| {
                if is_presented.get(cx) {
                    RiseAppEvents::default_center().post(MEDIA_PICKER_REFRESH_CELL_FRAMES);
                }
            });
        })
        .detach();
    }
}

/// formatting editor, its model and the screen built around it
#[derive(Clone)]
struct MediaPickerDocumentEditor {
    model: Entity<RiseDocumentEditorModel>,
    screen: Entity<RiseDocumentEditorScreen>,
}

struct MediaPickerSheetRuntimeState {
    document_editor: Option<MediaPickerDocumentEditor>,
    is_embedding_media: bool,
    is_composing_poll: Presence<()>,
    is_caption_picker_visible: bool,
    caption_bar: Presence<()>,
    has_caption_bar: bool,
    caption_takes_tab_bar: bool,
    source_tab_bar_reveal: Animated<f32>,
    visited_tabs: Vec<MediaPickerSourceTab>,
    host_view: Option<EntityId>,
    seen_editor_presentation: (bool, Option<String>, bool, bool),
    _observers: Vec<Subscription>,
}

impl MediaPickerSheetRuntimeState {
    fn new(session: &Entity<MediaPickerSession>, cx: &mut Context<Self>) -> Self {
        let read = session.read(cx);
        let (selection, editor_presentation) = (read.selection.clone(), read.editor_presentation.clone());
        let fetcher = read.fetcher.clone();
        let has_caption_bar = read.configuration.shows_bottom_bar && read.configuration.allows_caption;
        let has_selection = !selection.read(cx).is_empty();
        let caption_takes_tab_bar = has_caption_bar && has_selection;
        let seen_editor_presentation = Self::editor_presentation_marks(editor_presentation.read(cx));
        Self {
            document_editor: None,
            is_embedding_media: false,
            is_composing_poll: Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom)),
            is_caption_picker_visible: false,
            caption_bar: Presence::new(has_selection.then_some(()))
                .with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity())),
            has_caption_bar,
            caption_takes_tab_bar,
            source_tab_bar_reveal: Animated::new(if caption_takes_tab_bar { 0.0 } else { 1.0 }),
            visited_tabs: Vec::new(),
            host_view: None,
            seen_editor_presentation,
            _observers: vec![
                cx.observe(session, |state, _, cx| state.wake_host(cx)),
                cx.observe(&selection, |state, selection, cx| {
                    let has_selection = !selection.read(cx).is_empty();
                    state.sync_caption_bar(has_selection, cx);
                    state.wake_host(cx);
                }),
                cx.observe(&editor_presentation, |state, presentation, cx| {
                    let marks = Self::editor_presentation_marks(presentation.read(cx));
                    if marks != state.seen_editor_presentation {
                        state.seen_editor_presentation = marks;
                        state.wake_host(cx);
                    }
                }),
                cx.observe(&fetcher, |state, _, cx| state.wake_host(cx)),
            ],
        }
    }

    fn editor_presentation_marks(presentation: &MediaEditorPresentationState) -> (bool, Option<String>, bool, bool) {
        (
            presentation.is_presented,
            presentation.hero_lift_id.clone(),
            presentation.hero_cell_hidden,
            Self::editor_covers_sheet(presentation),
        )
    }

    /// why: editor backdrop is translucent on desktop so the sheet caption bar showed through
    /// grid stays drawn only while the editor flies in, is dragged or flies back
    fn editor_covers_sheet(presentation: &MediaEditorPresentationState) -> bool {
        presentation.is_presented && (presentation.editor_mode != EditorMode::Initial || presentation.covers_picker)
    }

    /// why: a file from the open panel opened the editor but the sheet stayed drawn as before
    /// the wake went to the view the state was born under, not the one drawing the sheet
    fn wake_host(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        if let Some(host_view) = self.host_view {
            App::notify(cx, host_view);
        }
    }

    fn set_composing_poll(&mut self, composing: bool, animated: bool, cx: &mut Context<Self>) {
        let spec = animated.then(|| ease_out(POLL_COMPOSER_ANIMATION_SECONDS));
        self.is_composing_poll.set_in(composing.then_some(()), spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn set_caption_picker_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.is_caption_picker_visible = visible;
        self.update_source_tab_bar_reveal(cx);
        cx.notify();
    }

    fn sync_caption_bar(&mut self, has_selection: bool, cx: &mut Context<Self>) {
        if self.caption_bar.is_present() == has_selection {
            return;
        }
        self.caption_bar.set_in(
            has_selection.then_some(()),
            ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION),
            RiseClock::monotonic(),
            cx,
        );
        self.caption_takes_tab_bar = self.has_caption_bar && has_selection;
        self.update_source_tab_bar_reveal(cx);
    }

    /// hides the tab bar while the caption picker is visible or the caption takes it
    fn update_source_tab_bar_reveal(&mut self, cx: &mut Context<Self>) {
        let hidden = self.is_caption_picker_visible || self.caption_takes_tab_bar;
        self.source_tab_bar_reveal.set_in(
            if hidden { 0.0 } else { 1.0 },
            ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION),
            RiseClock::monotonic(),
            cx,
        );
    }
}

#[derive(IntoElement)]
struct MediaPickerSheetRuntime {
    id: ElementId,
    session: Entity<MediaPickerSession>,
    is_presented: Binding<bool>,
    configuration: MediaPickerConfiguration,
    uses_native_sheet_presentation: bool,
    theme_store: Entity<ThemeStore>,
}

#[derive(Clone)]
struct MediaPickerSheetRuntimeParts {
    id: ElementId,
    state: Entity<MediaPickerSheetRuntimeState>,
    session: Entity<MediaPickerSession>,
    is_presented: Binding<bool>,
    configuration: MediaPickerConfiguration,
    uses_native_sheet_presentation: bool,
    theme_store: Entity<ThemeStore>,
}

impl RenderOnce for MediaPickerSheetRuntime {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("state"));
        let session = self.session.clone();
        let state = window.use_keyed_state(state_id, cx, |_, cx| MediaPickerSheetRuntimeState::new(&session, cx));
        let host_view = window.current_view();
        if state.read(cx).host_view != Some(host_view) {
            tracing::warn!(target: "riseonly::X1409", previous = ?state.read(cx).host_view, now = ?host_view, "picker runtime host view");
            state.update(cx, |state, _| state.host_view = Some(host_view));
        }
        let parts = MediaPickerSheetRuntimeParts {
            id: self.id,
            state,
            session: self.session,
            is_presented: self.is_presented,
            configuration: self.configuration,
            uses_native_sheet_presentation: self.uses_native_sheet_presentation,
            theme_store: self.theme_store,
        };
        parts.body(window, cx)
    }
}

impl MediaPickerSheetRuntimeParts {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn close_if_finished(&self, window: &mut Window, cx: &mut App) {
        if self.session.read(cx).did_finish() {
            self.is_presented.set(false, window, cx);
        }
    }

    fn cancel_and_close(&self, window: &mut Window, cx: &mut App) {
        self.session.update(cx, |session, cx| session.cancel(cx));
        self.is_presented.set(false, window, cx);
    }

    fn body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = self.theme_store.read(cx).current_theme().clone();

        let presentation = &self.session.read(cx).editor_presentation;
        gpui::hear_entity_through_events_only(presentation.entity_id());
        let is_editor_presented = presentation.read(cx).is_presented;
        let editor_covers_sheet = MediaPickerSheetRuntimeState::editor_covers_sheet(presentation.read(cx));
        let sync = self.clone();
        on_change(
            self.child_id("editor-presented"),
            is_editor_presented,
            false,
            window,
            cx,
            move |_, presented, _, cx| {
                let flag = *presented;
                MediaEditorWindowLog::state_event("runtime-flag-change", move || format!("presented={flag}"), cx);
                sync.sync_editor_window(cx);
                GuideFacts::set(RiseGuideSignal::MediaEditorOpen, *presented, cx);
            },
        );

        let source_tab = self.session.read(cx).source_tab;
        let tab_change = self.clone();
        on_change(self.child_id("source-tab"), source_tab, false, window, cx, move |previous, tab, window, cx| {
            tab_change.source_tab_changed(*previous, *tab, window, cx);
        });

        self.error_alert(window, cx);

        let mut content = div().size_full().bg(theme.bg._100).child(self.sheet_content(&theme, window, cx));
        if editor_covers_sheet {
            content = content.invisible();
        }
        let audio_sheet = self.audio_draft_sheet(&theme);
        let external_video_cover = self.external_video_cover(window, cx);

        let (appear, disappear) = (self.clone(), self.clone());
        div()
            .relative()
            .size_full()
            .child(content)
            .child(div().absolute().inset_0().child(audio_sheet))
            .children(external_video_cover)
            .on_appear(self.child_id("runtime-lifecycle"), move |_, cx| {
                let presentation = &appear.session.read(cx).editor_presentation;
                gpui::hear_entity_through_events_only(presentation.entity_id());
                let presented = presentation.read(cx).is_presented;
                MediaEditorWindowLog::state_event("runtime-appear", move || format!("presented={presented}"), cx);
                appear.sync_editor_window(cx);
            })
            .and_on_disappear(move |_, cx| {
                let presentation = &disappear.session.read(cx).editor_presentation;
                gpui::hear_entity_through_events_only(presentation.entity_id());
                let presented = presentation.read(cx).is_presented;
                let finished = disappear.session.read(cx).did_finish();
                MediaEditorWindowLog::state_event(
                    "runtime-disappear",
                    move || format!("presented={presented} finished={finished}"),
                    cx,
                );
                MediaEditorWindowPresenter::shared(cx)
                    .update(cx, |presenter, cx| presenter.teardown(&disappear.session, cx));
                GuideFacts::set(RiseGuideSignal::MediaEditorOpen, false, cx);
                disappear.state.update(cx, |state, _| state.document_editor = None);
            })
            .into_any_element()
    }

    fn error_alert(&self, window: &mut Window, cx: &mut App) {
        MediaPickerSession::error_alert(self.child_id("error-alert"), &self.session, window, cx);
    }

    fn audio_draft_sheet(&self, theme: &AppTheme) -> AnyElement {
        let surface = theme.bg._100;
        let (read_session, write_session) = (self.session.clone(), self.session.clone());
        let is_shown = Binding::new(
            move |cx: &App| read_session.read(cx).pending_audio_draft.is_some(),
            move |shown: bool, _: &mut Window, cx: &mut App| {
                if !shown {
                    write_session.update(cx, |session, cx| session.cancel_audio_draft(cx));
                }
            },
        );
        let parts = self.clone();
        NativeBottomSheetPresentationHost::new(self.child_id("audio-draft-sheet"), is_shown, move |_, cx| {
            let Some(draft) = parts.session.read(cx).pending_audio_draft.clone() else {
                return Empty.into_any_element();
            };
            let (on_cancel, on_complete) = (parts.session.clone(), parts.session.clone());
            MediaPickerAudioMetadataEditor::new(
                SharedString::from(format!("media-picker-audio-draft-{}", draft.id)),
                draft,
                parts.theme_store.clone(),
                move |_, cx| on_cancel.update(cx, |session, cx| session.cancel_audio_draft(cx)),
                move |draft, _, cx| on_complete.update(cx, |session, cx| session.complete_audio_draft(draft, cx)),
            )
            .into_any_element()
        })
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(false)
        .presentation_background(surface)
        .presentation_corner_radius(24.0)
        .into_any_element()
    }

    /// layer over the whole window for the pending external video draft
    fn external_video_cover(&self, window: &Window, cx: &App) -> Option<AnyElement> {
        let draft = self.session.read(cx).pending_external_video_draft.clone()?;
        let cover = WindowCoverFrame::resolve(window, cx);
        let editor = MediaPickerExternalVideoEditor {
            id: SharedString::from(format!("media-picker-external-video-{}", draft.id)).into(),
            draft,
            session: self.session.clone(),
            configuration: self.configuration.clone(),
            theme_store: self.theme_store.clone(),
        };
        Some(
            deferred(anchored().position(cover.origin).child(
                div().id(self.child_id("external-video-cover")).occlude().relative().w(cover.size.width).h(cover.size.height).child(editor),
            ))
            .with_priority(EXTERNAL_VIDEO_COVER_PRIORITY)
            .into_any_element(),
        )
    }

    fn sheet_content(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> Div {
        let tabs = self.sheet_tabs(theme, window, cx);
        let document_editor = self.state.read(cx).document_editor.clone();
        let document_layer = document_editor.map(|editor| {
            let embedder = self.state.read(cx).is_embedding_media.then(|| self.embedding_picker(&editor));
            div().absolute().inset_0().child(editor.screen.clone()).children(embedder)
        });

        let now = RiseClock::monotonic();
        let poll_layer = {
            let state = self.state.read(cx);
            if state.is_composing_poll.is_running(now) {
                window.request_animation_frame();
            }
            state.is_composing_poll.value(now).is_some().then(|| state.is_composing_poll.effect(now))
        }
        .map(|effect| {
            let (on_cancel, on_create) = (self.clone(), self.clone());
            div()
                .absolute()
                .inset_0()
                .child(MediaPickerPollComposer::new(
                    self.child_id("poll-composer"),
                    move |_: &mut Window, cx: &mut App| {
                        on_cancel.state.update(cx, |state, cx| state.set_composing_poll(false, true, cx));
                    },
                    move |poll: MessagePollDraft, window: &mut Window, cx: &mut App| {
                        on_create.state.update(cx, |state, cx| state.set_composing_poll(false, false, cx));
                        on_create.session.update(cx, |session, cx| session.finish_with_poll(poll, cx));
                        on_create.close_if_finished(window, cx);
                    },
                ))
                .transition_effect(effect)
        });

        div().relative().size_full().child(tabs).children(document_layer).children(poll_layer)
    }

    fn source_tab_changed(
        &self,
        previous: MediaPickerSourceTab,
        tab: MediaPickerSourceTab,
        window: &mut Window,
        cx: &mut App,
    ) {
        match tab {
            MediaPickerSourceTab::Formatting => self.open_document_editor(window, cx),
            MediaPickerSourceTab::Poll => {
                let fallback = self.first_source_tab();
                let restored = if previous == MediaPickerSourceTab::Poll { fallback } else { previous };
                self.session.update(cx, |session, cx| {
                    session.source_tab = restored;
                    cx.notify();
                });
                self.state.update(cx, |state, cx| state.set_composing_poll(true, true, cx));
            }
            _ => {}
        }
    }

    fn first_source_tab(&self) -> MediaPickerSourceTab {
        self.configuration.source_tabs.first().copied().unwrap_or(MediaPickerSourceTab::Gallery)
    }

    fn shows_poll_tab(&self) -> bool {
        self.configuration.allows_poll && self.configuration.source_tabs.contains(&MediaPickerSourceTab::Poll)
    }

    fn open_document_editor(&self, window: &mut Window, cx: &mut App) {
        if self.state.read(cx).document_editor.is_some() {
            return;
        }
        let allows_embedded_media = self.configuration.source_tabs.contains(&MediaPickerSourceTab::Gallery);
        let model = cx.new(|_| RiseDocumentEditorModel::new(&RiseDocument::EMPTY, allows_embedded_media));
        let (on_cancel, on_send, on_attach) = (self.clone(), self.clone(), self.clone());
        let on_attach_media: Option<Rc<dyn Fn(&mut Window, &mut App)>> = allows_embedded_media.then(|| {
            Rc::new(move |_: &mut Window, cx: &mut App| {
                on_attach.state.update(cx, |state, cx| {
                    state.is_embedding_media = true;
                    cx.notify();
                });
            }) as Rc<dyn Fn(&mut Window, &mut App)>
        });
        let screen_model = model.clone();
        let screen = cx.new(|cx| {
            RiseDocumentEditorScreen::new(
                screen_model,
                move |_, cx| on_cancel.close_document_editor(cx),
                move |document: RiseDocument,
                      items: Vec<MediaPickerItem>,
                      entities: Option<String>,
                      window: &mut Window,
                      cx: &mut App| {
                    on_send.state.update(cx, |state, cx| {
                        state.document_editor = None;
                        cx.notify();
                    });
                    on_send.session.update(cx, |session, cx| {
                        session.finish_with_document(document, items, entities, cx);
                    });
                    on_send.is_presented.set(false, window, cx);
                },
                on_attach_media,
                None,
                window,
                cx,
            )
        });
        self.state.update(cx, |state, cx| {
            state.document_editor = Some(MediaPickerDocumentEditor { model, screen });
            cx.notify();
        });
    }

    fn embed_configuration(source: &MediaPickerConfiguration) -> MediaPickerConfiguration {
        let mut configuration = source.clone();
        configuration.source_tabs = source
            .source_tabs
            .iter()
            .copied()
            .filter(|tab| *tab != MediaPickerSourceTab::Formatting && *tab != MediaPickerSourceTab::Location)
            .collect();
        configuration.allows_caption = false;
        configuration.allows_poll = false;
        configuration
    }

    fn embedding_picker(&self, editor: &MediaPickerDocumentEditor) -> MediaPickerSheet {
        let is_embedding = Binding::entity(
            self.state.clone(),
            |state: &MediaPickerSheetRuntimeState| state.is_embedding_media,
            |state, is_embedding, _| state.is_embedding_media = is_embedding,
        );
        let (on_complete, model) = (is_embedding.clone(), editor.model.clone());
        MediaPickerSheet::new(
            self.child_id("embedding-picker"),
            is_embedding,
            Self::embed_configuration(&self.configuration),
            self.theme_store.clone(),
            move |result: MediaPickerResult, window: &mut Window, cx: &mut App| {
                on_complete.set(false, window, cx);
                if result.was_cancelled {
                    return;
                }
                model.update(cx, |model, cx| model.attach(result.items, cx));
            },
        )
        .experimental_bottom_sheet(true)
    }

    fn close_document_editor(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.document_editor = None;
            cx.notify();
        });
        let first = self.first_source_tab();
        self.session.update(cx, |session, cx| {
            session.source_tab = first;
            cx.notify();
        });
    }

    fn sheet_tabs(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let source_tabs = &self.configuration.source_tabs;
        if source_tabs.len() > 1 {
            return self.source_tabs(theme, window, cx);
        }
        match source_tabs.first() {
            Some(MediaPickerSourceTab::File) => {
                return self.document_source_page(MediaPickerRecentKind::File).into_any_element();
            }
            Some(MediaPickerSourceTab::Audio) => {
                return self.document_source_page(MediaPickerRecentKind::Audio).into_any_element();
            }
            _ => {}
        }
        let screen = self.picker_screen(0.0, cx);
        if !self.configuration.shows_bottom_bar {
            return screen.into_any_element();
        }
        let footer = if self.configuration.allows_caption {
            self.caption_bar_layer(div().w_full(), None, window, cx)
        } else {
            Some(div().w_full().px(px(16.0)).pb(px(5.0)).child(self.floating_done_button(cx)).into_any_element())
        };
        div()
            .relative()
            .size_full()
            .child(screen)
            .children(footer.map(|footer| div().absolute().left_0().right_0().bottom_0().child(footer)))
            .into_any_element()
    }

    /// caption footer under its rise and fade transition, none once the last unpick has left
    fn caption_bar_layer(
        &self,
        frame: Div,
        on_picker_visible_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
        window: &Window,
        cx: &App,
    ) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        let effect = {
            let state = self.state.read(cx);
            if state.caption_bar.is_running(now) {
                window.request_animation_frame();
            }
            state.caption_bar.value(now).is_some().then(|| state.caption_bar.effect(now))
        }?;
        Some(frame.child(self.sheet_caption_footer(on_picker_visible_change, cx)).transition_effect(effect).into_any_element())
    }

    /// every page once shown stays mounted and only the selected one paints, topmost so its hit targets win
    /// the location page holds a live map that must not run behind the others
    fn source_tabs(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let selected = self.session.read(cx).source_tab;
        self.state.update(cx, |state, _| {
            if !state.visited_tabs.contains(&selected) {
                state.visited_tabs.push(selected);
            }
        });
        let visited = self.state.read(cx).visited_tabs.clone();
        let reveal = self.state.read(cx).source_tab_bar_reveal.frame_value(RiseClock::monotonic(), window);
        let tab_bar_height = SOURCE_TAB_BAR_HEIGHT * reveal;

        let mut pages = Vec::with_capacity(visited.len());
        for tab in visited.iter().copied().filter(|tab| *tab != selected) {
            if tab != MediaPickerSourceTab::Location {
                pages.extend(
                    self.source_page(tab, tab_bar_height, window, cx).map(|page| page.invisible().into_any_element()),
                );
            }
        }
        pages.extend(self.source_page(selected, tab_bar_height, window, cx).map(IntoElement::into_any_element));

        let bar = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(tab_bar_height))
            .overflow_hidden()
            .bg(theme.bg._100)
            .child(self.source_tab_bar(theme, selected));

        div().relative().size_full().children(pages).child(bar).into_any_element()
    }

    fn source_page(&self, tab: MediaPickerSourceTab, tab_bar_height: f32, window: &Window, cx: &mut App) -> Option<Div> {
        let page = div().absolute().inset_0();
        match tab {
            MediaPickerSourceTab::Gallery => Some(page.child(self.gallery_source_page(tab_bar_height, window, cx))),
            MediaPickerSourceTab::File => {
                Some(page.pb(px(tab_bar_height)).child(self.document_source_page(MediaPickerRecentKind::File)))
            }
            MediaPickerSourceTab::Audio => {
                Some(page.pb(px(tab_bar_height)).child(self.document_source_page(MediaPickerRecentKind::Audio)))
            }
            MediaPickerSourceTab::Location => Some(page.pb(px(tab_bar_height)).child(self.location_source_page())),
            MediaPickerSourceTab::Poll | MediaPickerSourceTab::Formatting => None,
        }
    }

    fn source_tab_bar(&self, theme: &AppTheme, selected: MediaPickerSourceTab) -> impl IntoElement {
        let tabs = &self.configuration.source_tabs;
        let items = [
            (MediaPickerSourceTab::Gallery, "media_picker_tab_gallery", "photo.on.rectangle.angled", "tab-gallery"),
            (MediaPickerSourceTab::File, "media_picker_tab_file", "doc.fill", "tab-file"),
            (MediaPickerSourceTab::Audio, "media_picker_tab_audio", "waveform", "tab-audio"),
            (MediaPickerSourceTab::Location, "chat_msg_location", "location.fill", "tab-location"),
            (MediaPickerSourceTab::Poll, "chat_attach_poll", "chart.bar.fill", "tab-poll"),
            (MediaPickerSourceTab::Formatting, "media_picker_tab_formatting", "textformat", "tab-formatting"),
        ];
        let buttons = items.into_iter().filter(|(tab, ..)| match tab {
            MediaPickerSourceTab::Poll => self.shows_poll_tab(),
            tab => tabs.contains(tab),
        });
        hstack(0.0).w_full().h(px(SOURCE_TAB_BAR_HEIGHT)).children(buttons.map(|(tab, title, symbol, name)| {
            let tint = if tab == selected { theme.primary._100 } else { theme.text.secondary };
            let session = self.session.clone();
            div().flex_1().child(
                plain(self.child_id(name))
                    .w_full()
                    .on_press(move |_, cx| {
                        session.update(cx, |session, cx| {
                            session.source_tab = tab;
                            cx.notify();
                        });
                    })
                    .child(
                        vstack(2.0)
                            .w_full()
                            .children(IconUi::sized(symbol, px(22.0), tint, symbol.ends_with(".fill")))
                            .child(MainText::new(tr(title)).px(10.0).font_weight(FontWeight::MEDIUM).color(tint)),
                    ),
            )
        }))
    }

    fn gallery_source_page(&self, tab_bar_height: f32, window: &Window, cx: &mut App) -> Div {
        let is_caption_picker_visible = self.state.read(cx).is_caption_picker_visible;
        let screen = self.picker_screen(if is_caption_picker_visible { 0.0 } else { tab_bar_height }, cx);
        let footer = if !self.configuration.shows_bottom_bar {
            None
        } else if self.configuration.allows_caption {
            let state = self.state.clone();
            let on_visible_change: Rc<dyn Fn(bool, &mut Window, &mut App)> =
                Rc::new(move |visible: bool, _: &mut Window, cx: &mut App| {
                    state.update(cx, |state, cx| state.set_caption_picker_visible(visible, cx));
                });
            let bottom = tab_bar_height + if is_caption_picker_visible { 0.0 } else { 8.0 };
            self.caption_bar_layer(div().w_full().pb(px(bottom)), Some(on_visible_change), window, cx)
        } else {
            Some(
                div()
                    .w_full()
                    .px(px(16.0))
                    .pb(px(5.0 + tab_bar_height))
                    .child(self.floating_done_button(cx))
                    .into_any_element(),
            )
        };
        div()
            .relative()
            .size_full()
            .child(screen)
            .children(footer.map(|footer| div().absolute().left_0().right_0().bottom_0().child(footer)))
    }

    fn document_source_page(&self, kind: MediaPickerRecentKind) -> Div {
        let (on_cancel, on_choose, on_recent, on_drop) = (self.clone(), self.clone(), self.clone(), self.session.clone());
        let (query_session, searching_session) = (self.session.clone(), self.session.clone());
        let name = if kind == MediaPickerRecentKind::Audio { "audio-source-page" } else { "file-source-page" };
        div()
            .size_full()
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                on_drop.update(cx, |session, cx| session.receive_external_paths(paths, cx));
            })
            .child(MediaPickerDocumentSourcePage::new(
                self.child_id(name),
                kind,
                move |window, cx| on_cancel.cancel_and_close(window, cx),
                move |_, cx| {
                    let tab = if kind == MediaPickerRecentKind::Audio {
                        MediaPickerSourceTab::Audio
                    } else {
                        MediaPickerSourceTab::File
                    };
                    on_choose.session.update(cx, |session, cx| session.present_document_picker(tab, cx));
                },
                move |entry: &MediaPickerRecentFile, _: &mut Window, cx: &mut App| {
                    on_recent.session.update(cx, |session, cx| session.select_recent(entry, cx));
                },
                self.configuration.allowed_file_extensions.clone(),
                Binding::entity(
                    query_session,
                    |session: &MediaPickerSession| session.search_query.clone(),
                    |session, query, _| session.search_query = query,
                ),
                Binding::entity(
                    searching_session,
                    |session: &MediaPickerSession| session.is_searching,
                    |session, is_searching, _| session.is_searching = is_searching,
                ),
            ))
    }

    fn location_source_page(&self) -> MediaPickerLocationSourcePage {
        let on_cancel = self.clone();
        MediaPickerLocationSourcePage::new(
            self.child_id("location-source-page"),
            move |window: &mut Window, cx: &mut App| on_cancel.cancel_and_close(window, cx),
        )
    }

    fn picker_screen(&self, bottom_chrome_inset: f32, cx: &App) -> Div {
        let read = self.session.read(cx);
        let (on_cancel, on_outcome, on_send, on_document) = (self.clone(), self.clone(), self.clone(), self.clone());
        let on_open_camera = (self.configuration.shows_camera_tile && read.offers_camera_tile()).then(|| {
            let session = self.session.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                session.update(cx, |session, cx| session.open_camera(true, cx));
            }) as Rc<dyn Fn(&mut Window, &mut App)>
        });
        let handle_clearance =
            if self.uses_native_sheet_presentation { 0.0 } else { MediaPickerScreen::SHEET_HANDLE_CLEARANCE };
        div().size_full().child(
            MediaPickerScreen::new(
                self.configuration.clone(),
                self.theme_store.clone(),
                read.selection.clone(),
                read.caption.clone(),
                read.fetcher.clone(),
                read.editor_presentation.clone(),
                move |window, cx| on_cancel.cancel_and_close(window, cx),
                move |outcome, asset, window, cx| on_outcome.handle_editor_outcome(outcome, asset, window, cx),
                move |send_as_file, window, cx| {
                    on_send.session.update(cx, |session, cx| session.export_selection(send_as_file, cx));
                    on_send.close_if_finished(window, cx);
                },
                move |window, cx| {
                    on_document.session.update(cx, |session, cx| {
                        session.present_document_picker(MediaPickerSourceTab::File, cx);
                    });
                    on_document.close_if_finished(window, cx);
                },
            )
            .caption_custom_emojis(read.caption_custom_emojis.clone())
            .layout(MediaPickerLayout::BottomSheet)
            .bottom_chrome_inset(bottom_chrome_inset)
            .handle_clearance(handle_clearance)
            .on_open_camera(on_open_camera)
            .import_session(self.session.downgrade()),
        )
    }

    fn sheet_caption_footer(
        &self,
        on_picker_visible_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
        cx: &App,
    ) -> MediaPickerSheetCaptionFooter {
        let read = self.session.read(cx);
        let on_send = self.clone();
        MediaPickerSheetCaptionFooter {
            id: self.child_id("caption-footer"),
            selection: read.selection.clone(),
            theme_store: self.theme_store.clone(),
            caption: read.caption.clone(),
            caption_custom_emojis: read.caption_custom_emojis.clone(),
            on_picker_visible_change,
            on_send: Rc::new(move |window: &mut Window, cx: &mut App| {
                on_send.session.update(cx, |session, cx| {
                    let as_file = session.selection.read(cx).send_as_file;
                    session.export_selection(as_file, cx);
                });
                on_send.close_if_finished(window, cx);
            }),
        }
    }

    fn floating_done_button(&self, cx: &App) -> MediaPickerFloatingDoneButton {
        let on_done = self.clone();
        MediaPickerFloatingDoneButton::new(
            self.session.read(cx).selection.clone(),
            self.theme_store.clone(),
            self.configuration.confirm_button_title.clone(),
            move |window: &mut Window, cx: &mut App| {
                on_done.session.update(cx, |session, cx| session.export_selection(false, cx));
                on_done.close_if_finished(window, cx);
            },
        )
    }

    fn handle_editor_outcome(&self, outcome: MediaEditorOutcome, asset: MediaAsset, window: &mut Window, cx: &mut App) {
        self.session.update(cx, |session, cx| session.handle_editor_outcome(outcome, asset, cx));
        self.close_if_finished(window, cx);
    }

    fn sync_editor_window(&self, cx: &mut App) {
        let parts = self.clone();
        let session = self.session.clone();
        let configuration = self.configuration.clone();
        let theme_store = self.theme_store.clone();
        MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.sync_presentation(
                session,
                configuration,
                theme_store,
                Rc::new(move |outcome: MediaEditorOutcome, asset: MediaAsset, window: &mut Window, cx: &mut App| {
                    parts.handle_editor_outcome(outcome, asset, window, cx);
                }),
                cx,
            );
        });
    }
}

struct MediaPickerExternalVideoEditorState {
    selection: Entity<MediaSelectionState>,
    caption: String,
}

#[derive(IntoElement)]
struct MediaPickerExternalVideoEditor {
    id: ElementId,
    draft: MediaPickerExternalVideoDraft,
    session: Entity<MediaPickerSession>,
    configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
}

impl MediaPickerExternalVideoEditor {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

impl RenderOnce for MediaPickerExternalVideoEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| {
            let selection = cx.new(|cx| {
                let mut selection = MediaSelectionState::new(cx);
                selection.max_allowed = 1;
                selection
            });
            MediaPickerExternalVideoEditorState { selection, caption: String::new() }
        });
        let selection = state.read(cx).selection.clone();
        let caption = Binding::entity(
            state,
            |state: &MediaPickerExternalVideoEditorState| state.caption.clone(),
            |state, caption, _| state.caption = caption,
        );
        let (on_cancel, on_done, on_done_draft) = (self.session.clone(), self.session.clone(), self.draft.clone());
        let source_url = self.draft.source_url.clone();
        let export_overlay = {
            let read = self.session.read(cx);
            read.is_exporting_external_video().then(|| {
                MediaPickerExportProgressOverlay::new(
                    read.external_video_export_progress.clone(),
                    self.theme_store.clone(),
                )
            })
        };
        div()
            .relative()
            .size_full()
            .children(media_editor_view_hooks().map(|hooks| {
                (hooks.external_video_screen)(
                    self.child_id("screen"),
                    selection,
                    caption,
                    self.draft.media_asset.clone(),
                    self.theme_store.clone(),
                    self.configuration.clone(),
                    source_url,
                    Box::new(move |_: &mut Window, cx: &mut App| {
                        on_cancel.update(cx, |session, cx| session.cancel_external_video_edit(cx));
                    }),
                    Box::new(move |outcome: MediaEditorOutcome, _: &mut Window, cx: &mut App| {
                        on_done.update(cx, |session, cx| {
                            session.complete_external_video_edit(&on_done_draft, outcome, cx);
                        });
                    }),
                )
            }))
            .children(export_overlay)
    }
}

#[derive(IntoElement)]
struct MediaPickerSheetCaptionFooter {
    id: ElementId,
    selection: Entity<MediaSelectionState>,
    theme_store: Entity<ThemeStore>,
    caption: Binding<String>,
    caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    on_picker_visible_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
    on_send: SheetAction,
}

impl RenderOnce for MediaPickerSheetCaptionFooter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        match media_picker_caption_bar_hooks() {
            Some(hooks) => (hooks.caption_bar)(
                self.id,
                self.selection,
                self.theme_store,
                self.caption,
                self.caption_custom_emojis,
                self.on_picker_visible_change,
                self.on_send,
            ),
            None => Empty.into_any_element(),
        }
    }
}

#[derive(IntoElement)]
pub struct MediaPickerFloatingDoneButton {
    pub selection: Entity<MediaSelectionState>,
    pub theme_store: Entity<ThemeStore>,
    pub title: String,
    pub system_image: SharedString,
    pub on_done: SheetAction,
}

impl MediaPickerFloatingDoneButton {
    /// default system_image is checkmark
    pub fn new(
        selection: Entity<MediaSelectionState>,
        theme_store: Entity<ThemeStore>,
        title: String,
        on_done: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            selection,
            theme_store,
            title,
            system_image: SharedString::new_static("checkmark"),
            on_done: Rc::new(on_done),
        }
    }

    pub fn system_image(mut self, system_image: impl Into<SharedString>) -> Self {
        self.system_image = system_image.into();
        self
    }
}

impl RenderOnce for MediaPickerFloatingDoneButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme_store.read(cx).current_theme();
        let can_confirm = !self.selection.read(cx).is_empty();
        let tint = if can_confirm { theme.primary._100 } else { theme.bg._300 };
        let icon_color = theme.content_on(tint);
        let id = ("media-picker-floating-done", self.selection.entity_id());
        let tooltip_id = ("media-picker-floating-done-tooltip", self.selection.entity_id());
        let on_done = self.on_done;
        div().w_full().flex().justify_end().child(
            div().id(tooltip_id).tooltip_label(self.title.clone()).child(
                LiquidGlassButton::new(id, LiquidGlassButtonShape::Circle)
                    .size(52.0)
                    .tint(Some(tint))
                    .haptic_on_press(can_confirm)
                    .disabled(!can_confirm)
                    .accessibility_label(Some(self.title))
                    .on_press(move |window, cx| {
                        if can_confirm {
                            on_done(window, cx);
                        }
                    })
                    .children(IconUi::sized(&self.system_image, px(19.0), icon_color, self.system_image.ends_with(".fill"))),
            ),
        )
    }
}

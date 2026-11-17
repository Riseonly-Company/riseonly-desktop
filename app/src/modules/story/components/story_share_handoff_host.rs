use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Subscription, Window, black, white};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::media_viewer::media_viewer_presentation_state::media_viewer_presentation;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackSelection;
use crate::modules::media_picker::editor::media_editor_screen::{MediaEditorOutcome, MediaEditorScreen};
use crate::modules::media_picker::library::media_asset::MediaAssetMediaType;
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::story::components::story_create_sheet::StoryCreateSheet;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::story::stores::story::story_services::story_share_handoff_models::{
    StoryShareHandoffComposition, StoryShareHandoffContext, StoryShareHandoffDraft, StoryShareHandoffPhase,
    StoryShareHandoffPresentationStage,
};

fn named_id(name: String) -> ElementId {
    ElementId::Name(name.into())
}

struct StoryShareHandoffHostState {
    service: Entity<StoryServicesStore>,
    key: StoryShareHandoffHostKey,
    drawn: StoryShareHandoffDrawn,
    _observation: Subscription,
}

/// what render draws and reacts to from the story store
#[derive(Clone)]
struct StoryShareHandoffDrawn {
    blocks_interface: bool,
    is_publishing: bool,
    publish_error: Option<String>,
    phase: StoryShareHandoffPhase,
}

impl StoryShareHandoffDrawn {
    fn of(service: &StoryServicesStore) -> Self {
        Self {
            blocks_interface: service.shared_story_handoff_presentation_stage().blocks_interface(),
            is_publishing: service.is_publishing,
            publish_error: service.publish_error.clone(),
            phase: service.story_share_handoff_phase.clone(),
        }
    }
}

/// everything the host draws from the story store
/// store notifies on every publish step
/// this redraws the whole root, so notify on change only
#[derive(Clone, PartialEq)]
struct StoryShareHandoffHostKey {
    stage: StoryShareHandoffPresentationStage,
    is_publishing: bool,
    publish_error: Option<String>,
    phase: StoryShareHandoffPhaseKey,
}

#[derive(Clone, PartialEq)]
enum StoryShareHandoffPhaseKey {
    Idle,
    Preparing { session_id: String, current: usize, total: Option<usize> },
    Editing { draft_id: Uuid, current_index: usize },
    Exporting { draft_id: Uuid, current_index: usize, immediate_send: bool },
    Composing { session_id: String, current_index: usize, immediate_send: bool },
    Recovering { session_id: String },
    Finalizing { session_id: String, has_error: bool },
}

impl StoryShareHandoffHostKey {
    fn of(service: &StoryServicesStore) -> Self {
        let phase = match &service.story_share_handoff_phase {
            StoryShareHandoffPhase::Idle => StoryShareHandoffPhaseKey::Idle,
            StoryShareHandoffPhase::Preparing { session_id, current, total } => StoryShareHandoffPhaseKey::Preparing {
                session_id: session_id.clone(),
                current: *current,
                total: *total,
            },
            StoryShareHandoffPhase::Editing(draft) => StoryShareHandoffPhaseKey::Editing {
                draft_id: draft.id,
                current_index: draft.context.current_index,
            },
            StoryShareHandoffPhase::Exporting(export) => StoryShareHandoffPhaseKey::Exporting {
                draft_id: export.draft.id,
                current_index: export.draft.context.current_index,
                immediate_send: export.immediate_send,
            },
            StoryShareHandoffPhase::Composing(composition) => StoryShareHandoffPhaseKey::Composing {
                session_id: composition.context.session_id.clone(),
                current_index: composition.context.current_index,
                immediate_send: composition.immediate_send,
            },
            StoryShareHandoffPhase::Recovering(recovery) => StoryShareHandoffPhaseKey::Recovering {
                session_id: recovery.context.session_id.clone(),
            },
            StoryShareHandoffPhase::Finalizing(finalization) => StoryShareHandoffPhaseKey::Finalizing {
                session_id: finalization.context.session_id.clone(),
                has_error: finalization.error_message.is_some(),
            },
        };
        Self {
            stage: service.shared_story_handoff_presentation_stage(),
            is_publishing: service.is_publishing,
            publish_error: service.publish_error.clone(),
            phase,
        }
    }
}

#[derive(IntoElement, Default)]
pub struct StoryShareHandoffHost;

impl StoryShareHandoffHost {
    pub fn new() -> Self {
        Self
    }

    fn content(phase: StoryShareHandoffPhase, cx: &mut App) -> Option<AnyElement> {
        match phase {
            StoryShareHandoffPhase::Idle => None,

            StoryShareHandoffPhase::Preparing { current, total, .. } => {
                Some(Self::progress_overlay(tr("story_share_preparing"), current, total).into_any_element())
            }

            StoryShareHandoffPhase::Editing(draft) => {
                let badge = Self::queue_badge(&draft.context);
                let theme_store = ThemeStore::shared(cx);
                let service = StoryServicesStore::shared(cx);
                let (cancel_service, done_service, done_draft) = (service.clone(), service, draft.clone());
                Some(
                    div()
                        .absolute()
                        .inset_0()
                        .child(StoryShareExternalEditor::new(
                            draft,
                            theme_store,
                            move |_, cx| cancel_service.update(cx, |service, cx| service.cancel_shared_story_handoff(cx)),
                            move |outcome, caption, attached_track, _, cx| {
                                done_service.update(cx, |service, cx| {
                                    service.export_shared_story_draft(&done_draft, outcome, caption, attached_track, cx);
                                });
                            },
                        ))
                        .children(badge)
                        .occludes_bottom_chrome()
                        .into_any_element(),
                )
            }

            StoryShareHandoffPhase::Exporting(export) => {
                if export.immediate_send {
                    None
                } else {
                    Some(
                        Self::progress_overlay(
                            tr("story_share_exporting"),
                            export.draft.context.display_index(),
                            Some(export.draft.context.total_count()),
                        )
                        .into_any_element(),
                    )
                }
            }

            StoryShareHandoffPhase::Composing(composition) => {
                if composition.immediate_send {
                    None
                } else {
                    Some(StoryShareComposerPage::new(composition).occludes_bottom_chrome().into_any_element())
                }
            }

            StoryShareHandoffPhase::Recovering(_) => None,

            StoryShareHandoffPhase::Finalizing(_) => None,
        }
    }

    fn progress_overlay(title: String, current: usize, total: Option<usize>) -> impl IntoElement {
        let counter = total.filter(|total| *total > 1).map(|total| {
            MainText::new(format!("{current} / {total}"))
                .px(12.0)
                .font_weight(FontWeight::MEDIUM)
                .color(white().opacity(0.58))
        });
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(black().opacity(0.94))
            .child(
                vstack(14.0)
                    .child(progress_view().control_size(ControlSize::Large).tint(white()))
                    .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).color(white()))
                    .children(counter),
            )
            .occludes_bottom_chrome()
    }

    fn queue_badge(context: &StoryShareHandoffContext) -> Option<impl IntoElement + use<>> {
        (context.total_count() > 1).then(|| {
            pass_through().absolute().top(px(58.0)).right(px(14.0)).child(
                div()
                    .h(px(28.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(black().opacity(0.58))
                    .child(
                        MainText::new(format!("{} / {}", context.display_index(), context.total_count()))
                            .px(12.0)
                            .font_weight(FontWeight::BOLD)
                            .color(white()),
                    ),
            )
        })
    }
}

impl RenderOnce for StoryShareHandoffHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("story-share-handoff-host", cx, |_, cx: &mut Context<StoryShareHandoffHostState>| {
            let service = StoryServicesStore::shared(cx);
            let observation = cx.observe(&service, |state: &mut StoryShareHandoffHostState, service, cx| {
                let service = service.read(cx);
                let key = StoryShareHandoffHostKey::of(service);
                state.drawn = StoryShareHandoffDrawn::of(service);
                if key != state.key {
                    state.key = key;
                    cx.notify();
                }
            });
            let (key, drawn) = {
                let read = service.read(cx);
                (StoryShareHandoffHostKey::of(read), StoryShareHandoffDrawn::of(read))
            };
            StoryShareHandoffHostState { service, key, drawn, _observation: observation }
        });
        let (service, StoryShareHandoffDrawn { blocks_interface, is_publishing, publish_error, phase }) = {
            let read = state.read(cx);
            (read.service.clone(), read.drawn.clone())
        };

        on_change("story-share-handoff-blocks", blocks_interface, true, window, cx, |_, blocks, _, cx| {
            if !*blocks {
                return;
            }
            let viewer = media_viewer_presentation(cx);
            if viewer.read(cx).is_presented() {
                viewer.update(cx, |viewer, cx| viewer.dismiss(cx));
            }
        });
        {
            let service = service.clone();
            on_change("story-share-handoff-publishing", is_publishing, false, window, cx, move |_, publishing, _, cx| {
                if !*publishing {
                    service.update(cx, |service, cx| service.resume_pending_shared_story_handoff_if_possible(cx));
                }
            });
        }
        on_change("story-share-handoff-error", publish_error, false, window, cx, |_, message, _, cx| {
            if message.is_some() {
                StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.present_publish_error(cx));
            }
        });

        let content = Self::content(phase, cx);
        div()
            .relative()
            .size_full()
            .when(blocks_interface, |host| host.occlude())
            .children(content)
    }
}

/// editor state kept by the draft id
struct StoryShareExternalEditorState {
    selection: Entity<MediaSelectionState>,
    caption: String,
}

type StoryShareEditorCancel = Rc<dyn Fn(&mut Window, &mut App)>;
type StoryShareEditorDone = Rc<dyn Fn(MediaEditorOutcome, String, Option<MediaPickerTrackSelection>, &mut Window, &mut App)>;

#[derive(IntoElement)]
struct StoryShareExternalEditor {
    draft: StoryShareHandoffDraft,
    theme_store: Entity<ThemeStore>,
    on_cancel: StoryShareEditorCancel,
    on_done: StoryShareEditorDone,
}

impl StoryShareExternalEditor {
    fn new(
        draft: StoryShareHandoffDraft,
        theme_store: Entity<ThemeStore>,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_done: impl Fn(MediaEditorOutcome, String, Option<MediaPickerTrackSelection>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { draft, theme_store, on_cancel: Rc::new(on_cancel), on_done: Rc::new(on_done) }
    }
}

impl RenderOnce for StoryShareExternalEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let draft = self.draft;
        let key = draft.id.uuid_string();
        let seed = draft.clone();
        let state = window.use_keyed_state(named_id(format!("story-share-editor-{key}")), cx, move |_, cx: &mut Context<StoryShareExternalEditorState>| {
            let track = seed.initial_track.clone();
            StoryShareExternalEditorState {
                selection: cx.new(|cx| {
                    let mut selection = MediaSelectionState::new(cx);
                    selection.max_allowed = 1;
                    selection.attached_track = track;
                    selection
                }),
                caption: seed.initial_caption.clone(),
            }
        });
        let selection = state.read(cx).selection.clone();
        let caption = Binding::entity(
            state.clone(),
            |state: &StoryShareExternalEditorState| state.caption.clone(),
            |state, value, _| state.caption = value,
        );
        let is_video = draft.media_asset.media_type == MediaAssetMediaType::Video;
        let source_video_url = if is_video { Url::from_file_path(&draft.source_url) } else { None };
        let on_cancel = self.on_cancel;
        let on_done = self.on_done;
        let (done_state, done_selection) = (state, selection.clone());

        div().absolute().inset_0().child(
            MediaEditorScreen::new(
                named_id(format!("story-share-editor-screen-{key}")),
                selection,
                caption,
                draft.media_asset.clone(),
                None,
                self.theme_store,
                MediaPickerConfiguration::story(),
                draft.initial_edit.clone(),
                is_video,
                move |window, cx| on_cancel(window, cx),
                move |outcome, window, cx| {
                    let caption = done_state.read(cx).caption.clone();
                    let attached_track = done_selection.read(cx).attached_track.clone();
                    on_done(outcome, caption, attached_track, window, cx);
                },
            )
            .source_image_override(draft.source_image.clone())
            .source_video_url_override(source_video_url)
            .enables_standalone_swipe_dismiss(true)
            .standalone_dismiss_cancels(false)
            .embed_in_hero_host(false),
        )
    }
}

struct StoryShareComposerPageState {
    is_presented: bool,
    did_publish: bool,
}

#[derive(IntoElement)]
struct StoryShareComposerPage {
    composition: StoryShareHandoffComposition,
}

impl StoryShareComposerPage {
    fn new(composition: StoryShareHandoffComposition) -> Self {
        Self { composition }
    }
}

impl RenderOnce for StoryShareComposerPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let composition = self.composition;
        let key = format!("{}-{}", composition.context.session_id, composition.context.current_index);
        let state = window.use_keyed_state(named_id(format!("story-share-composer-{key}")), cx, |_, _| {
            StoryShareComposerPageState { is_presented: true, did_publish: false }
        });

        let is_presented_now = state.read(cx).is_presented;
        {
            let state = state.clone();
            on_change(
                named_id(format!("story-share-composer-presented-{key}")),
                is_presented_now,
                false,
                window,
                cx,
                move |_, presented, window, cx| {
                    if *presented {
                        return;
                    }
                    let state = state.clone();
                    window.defer(cx, move |_, cx| {
                        if !state.read(cx).did_publish {
                            StoryServicesStore::shared(cx).update(cx, |service, cx| service.cancel_shared_story_handoff(cx));
                        }
                    });
                },
            );
        }

        let is_presented = Binding::entity(
            state.clone(),
            |state: &StoryShareComposerPageState| state.is_presented,
            |state, value, _| state.is_presented = value,
        );
        let publish_context = composition.context.publish_context();
        let scrim_presented = is_presented.clone();
        let sheet = StoryCreateSheet::new(
            named_id(format!("story-share-create-{key}")),
            composition.item,
            composition.caption,
            is_presented,
            move |_, cx| {
                state.update(cx, |state, _| state.did_publish = true);
                StoryServicesStore::shared(cx).update(cx, |service, cx| service.shared_story_did_publish_current(cx));
            },
        )
        .initial_track(composition.attached_track)
        .shared_handoff_context(Some(publish_context))
        .in_modal(true);
        sheet.modal_cover(scrim_presented, window, cx)
    }
}

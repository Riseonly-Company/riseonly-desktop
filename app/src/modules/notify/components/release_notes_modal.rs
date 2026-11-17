use std::rc::Rc;

use gpui::{App, Context, Entity, RenderOnce, SharedString, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::shapes::{RoundedCornerStyle, Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::modules::notify::components::slide_deck_modal_focus::SlideDeckModalFocus;
use crate::modules::notify::components::slide_deck_ui::{SlideDeckHeader, SlideDeckUi};
use crate::modules::notify::engine::core::rise_release_notes_wire_models::{RiseReleaseNote, RiseReleaseSlide};
use crate::modules::notify::engine::rise_launch_interruptions::RiseLaunchInterruptions;
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;


struct ReleaseNotesHostState {
    slide_index: i64,
    shown_release_id: Option<i64>,
    note: Option<RiseReleaseNote>,
    presence: Presence<RiseReleaseNote>,
}

/// the whats new modal and the force update wall
/// an overlay not a sheet since sheets dismiss on backdrop tap or drag and a blocking release must not
#[derive(IntoElement, Default)]
pub struct ReleaseNotesHost;

impl ReleaseNotesHost {
    pub fn new() -> Self {
        Self
    }

    fn mark_shown(state: &Entity<ReleaseNotesHostState>, note: &RiseReleaseNote, cx: &mut App) {
        if state.read(cx).shown_release_id == Some(note.id) {
            return;
        }
        state.update(cx, |state, _| state.shown_release_id = Some(note.id));
        NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.release_note_appeared(note, cx));
    }

    fn dismiss(state: &Entity<ReleaseNotesHostState>, note: &RiseReleaseNote, cx: &mut App) {
        let seen_slides = state.read(cx).slide_index + 1;
        NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_release_note(note, seen_slides, cx));
    }

    fn open_action(
        state: &Entity<ReleaseNotesHostState>,
        note: &RiseReleaseNote,
        slide: &RiseReleaseSlide,
        cx: &mut App,
    ) {
        let seen_slides = state.read(cx).slide_index + 1;
        NotifyInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.open_release_note_action(note, slide, seen_slides, cx));
    }
}

impl RenderOnce for ReleaseNotesHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let interruptions = RiseLaunchInterruptions::shared(cx);
        let note = interruptions.read(cx).release_note(cx);
        let state = window.use_keyed_state("release-notes-host", cx, |_, _| ReleaseNotesHostState {
            slide_index: 0,
            shown_release_id: None,
            note: None,
            presence: Presence::new(None).with_transition(Transition::opacity()),
        });

        let next = note.clone();
        state.update(cx, |state: &mut ReleaseNotesHostState, cx: &mut Context<ReleaseNotesHostState>| {
            if state.note != next {
                state.note = next.clone();
                state.presence.set_in(next, ease_out(0.22), now, cx);
                cx.notify();
            }
        });

        let changed_state = state.clone();
        on_change(
            "release-notes-host-note",
            note.as_ref().map(|note| note.id),
            true,
            window,
            cx,
            move |old, new, _, cx| {
                let Some(note) = note.filter(|note| Some(note.id) == *new) else {
                    return;
                };
                if old.is_some() && old != new {
                    changed_state.update(cx, |state, _| state.slide_index = 0);
                }
                Self::mark_shown(&changed_state, &note, cx);
            },
        );

        let shared: &App = cx;
        let overlay = state.read(shared).presence.render_transitioned(now, window, |note| {
            let slide_index = Binding::entity(
                state.clone(),
                |state: &ReleaseNotesHostState| state.slide_index,
                |state: &mut ReleaseNotesHostState, index, _| state.slide_index = index,
            );
            let dismiss_state = state.clone();
            let dismiss_note = note.clone();
            let action_state = state.clone();
            let action_note = note.clone();
            div().absolute().inset_0().child(ReleaseNotesModal::new(
                note.clone(),
                slide_index,
                Rc::new(move |_, cx| Self::dismiss(&dismiss_state, &dismiss_note, cx)),
                Rc::new(move |slide, _, cx| Self::open_action(&action_state, &action_note, slide, cx)),
            ))
        });

        div().absolute().inset_0().children(overlay)
    }
}

#[derive(IntoElement)]
pub struct ReleaseNotesModal {
    note: RiseReleaseNote,
    slide_index: Binding<i64>,
    on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    on_action: Rc<dyn Fn(&RiseReleaseSlide, &mut Window, &mut App)>,
}

impl ReleaseNotesModal {
    pub fn new(
        note: RiseReleaseNote,
        slide_index: Binding<i64>,
        on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
        on_action: Rc<dyn Fn(&RiseReleaseSlide, &mut Window, &mut App)>,
    ) -> Self {
        Self { note, slide_index, on_dismiss, on_action }
    }
}

impl RenderOnce for ReleaseNotesModal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let modal_focus = SlideDeckModalFocus::keyed("release-notes-modal-focus", window, cx);
        let focus_handle = modal_focus.read(cx).handle.clone();
        let is_blocking = self.note.is_blocking;
        let escape_dismiss = self.on_dismiss.clone();

        div()
            .id("release-notes-modal")
            .size_full()
            .relative()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.bg._100.opacity(0.82))
            .track_focus(&focus_handle)
            .layer_keys(if is_blocking {
                LayerCancel::Blocked
            } else {
                LayerCancel::dismiss(move |window, cx| escape_dismiss(window, cx))
            })
            .child(
                div().flex().flex_col().w_full().max_h_full().max_w(px(460.0)).px(px(20.0)).child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .w_full()
                        .min_h_0()
                        .child(
                            vstack(0.0)
                                .w_full()
                                .min_h_0()
                                .pt(px(22.0))
                                .pb(px(16.0))
                                .bg(theme.bg._200)
                                .continuous_corners(22.0)
                                .overflow_hidden()
                                .child(SlideDeckHeader::new(
                                    "release-notes-header",
                                    SharedString::from(self.note.title.clone()),
                                    theme.clone(),
                                    if is_blocking { None } else { Some(self.on_dismiss.clone()) },
                                ))
                                .child(SlideDeckUi::new(
                                    "release-notes-deck",
                                    self.note.slides.clone(),
                                    self.slide_index,
                                    is_blocking,
                                    true,
                                    theme.clone(),
                                    self.on_action,
                                    self.on_dismiss,
                                )),
                        )
                        .child(centred_stroke(
                            theme.bg._400,
                            1.0,
                            Shape::RoundedRectangle { corner_radius: 22.0, style: RoundedCornerStyle::Continuous },
                        )),
                ),
            )
            .on_disappear("release-notes-modal-lifecycle", move |window, cx| {
                if let Some(window) = window {
                    SlideDeckModalFocus::hand_back(&modal_focus, window, cx);
                }
            })
    }
}

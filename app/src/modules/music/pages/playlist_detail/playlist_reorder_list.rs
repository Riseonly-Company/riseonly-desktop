use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, InteractiveElement, IntoElement, ListAlignment, ListState, ParentElement,
    RenderOnce, SharedString, Styled, Window, div, list, px,
};
use rise_ui::IconUi;
use rise_ui::reorder_grip::ReorderGripHandle;

use crate::core::widgets::wrappers::list_shell_ui::ListShellScrollUnderChromeModifiers;
use crate::modules::music::pages::track::track_row::TrackRow;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;

const ROW_HEIGHT: f32 = 64.0;
const HANDLE_SIDE: f32 = 44.0;
const HANDLE_GLYPH_SIDE: f32 = 17.0;

#[derive(IntoElement)]
pub struct PlaylistReorderList {
    playlist_id: String,
    tracks: Vec<TrackModel>,
}

impl PlaylistReorderList {
    pub fn new(playlist_id: impl Into<String>, tracks: Vec<TrackModel>) -> Self {
        Self {
            playlist_id: playlist_id.into(),
            tracks,
        }
    }
}

/// the drag in flight
/// the rows place in order and pointer travel since the press
#[derive(Clone, Copy)]
struct PlaylistReorderDrag {
    from: usize,
    translation: f32,
}

impl PlaylistReorderDrag {
    fn target(&self, count: usize) -> usize {
        let slots = (self.translation / ROW_HEIGHT).round() as i64;
        (self.from as i64 + slots).clamp(0, count.saturating_sub(1) as i64) as usize
    }
}

/// order state plus what the track id change check compares against
struct PlaylistReorderListState {
    order: Vec<TrackModel>,
    observed_ids: Vec<String>,
    drag: Option<PlaylistReorderDrag>,
    list_state: ListState,
}

impl PlaylistReorderListState {
    fn new(tracks: &[TrackModel]) -> Self {
        Self {
            order: tracks.to_vec(),
            observed_ids: Self::ids(tracks),
            drag: None,
            list_state: ListState::new(tracks.len() + 1, ListAlignment::Top, px(ROW_HEIGHT * 4.0)),
        }
    }

    fn ids(tracks: &[TrackModel]) -> Vec<String> {
        tracks.iter().map(|track| track.id.clone()).collect()
    }

    /// why: a read after another members move or a rollback, an own drop is already here
    fn observe(&mut self, tracks: &[TrackModel]) {
        let ids = Self::ids(tracks);
        if ids == self.observed_ids {
            return;
        }
        self.observed_ids = ids;
        if self.observed_ids != Self::ids(&self.order) {
            self.replace_order(tracks.to_vec());
        }
    }

    fn replace_order(&mut self, order: Vec<TrackModel>) {
        if order.len() != self.order.len() {
            self.list_state.reset(order.len() + 1);
        }
        self.order = order;
    }

    /// the order as drawn, the dragged row already sits in the slot it would land in
    fn displayed(&self) -> Vec<(TrackModel, f32)> {
        let mut rows: Vec<(TrackModel, f32)> = self.order.iter().cloned().map(|track| (track, 0.0)).collect();
        if let Some(drag) = self.drag
            && drag.from < rows.len()
        {
            let target = drag.target(rows.len());
            let (track, _) = rows.remove(drag.from);
            let offset = drag.translation - (target as f32 - drag.from as f32) * ROW_HEIGHT;
            rows.insert(target, (track, offset));
        }
        rows
    }
}

impl RenderOnce for PlaylistReorderList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let id = ElementId::Name(SharedString::from(format!("playlist-reorder-{}", self.playlist_id)));
        let tracks = self.tracks;
        let state = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(id.clone()), "state".into()),
            cx,
            |_: &mut Window, _: &mut Context<PlaylistReorderListState>| PlaylistReorderListState::new(&tracks),
        );
        state.update(cx, |state, _| state.observe(&tracks));
        let (rows, list_state) = {
            let state = state.read(cx);
            (Rc::new(state.displayed()), state.list_state.clone())
        };
        let playlist_id: Rc<str> = Rc::from(self.playlist_id.as_str());
        let row_background = theme.bg._100;
        let separator = theme.border._100;
        let handle_color = theme.text.secondary;

        div().size_full().bg(theme.bg._100).child(
            list(list_state, move |index, _, _| {
                let Some((track, offset)) = rows.get(index).cloned() else {
                    return div().w_full().music_mini_player_bottom_padding(40.0, 0.0).into_any_element();
                };
                let track_id = track.id.clone();
                let on_change_state = state.clone();
                let on_change_id = track_id.clone();
                let on_end_state = state.clone();
                let on_end_playlist = playlist_id.clone();
                let grip = ReorderGripHandle::new(
                    ElementId::Name(SharedString::from(format!("playlist-reorder-grip-{track_id}"))),
                    Rc::new(move |translation: f32, _: &mut Window, cx: &mut App| {
                        on_change_state.update(cx, |state, cx| {
                            let from = match state.drag {
                                Some(drag) => drag.from,
                                None => match state.order.iter().position(|track| track.id == on_change_id) {
                                    Some(from) => from,
                                    None => return,
                                },
                            };
                            state.drag = Some(PlaylistReorderDrag { from, translation });
                            cx.notify();
                        });
                    }),
                    Rc::new(move |_: &mut Window, cx: &mut App| {
                        Self::drop_drag(&on_end_state, &on_end_playlist, cx);
                    }),
                );
                div()
                    .id(ElementId::Name(SharedString::from(format!("playlist-reorder-row-{track_id}"))))
                    .relative()
                    .top(px(offset))
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .pl(px(16.0))
                    .pr(px(8.0))
                    .bg(row_background)
                    .border_b_1()
                    .border_color(separator)
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .child(
                                TrackRow::new(ElementId::Name(SharedString::from(track_id.clone())), track)
                                    .is_playing(false)
                                    .shows_like_button(false),
                            )
                            .child(div().absolute().inset_0().block_mouse_except_scroll()),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .size(px(HANDLE_SIDE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("line.3.horizontal", px(HANDLE_GLYPH_SIDE), handle_color, false))
                            .child(div().absolute().inset_0().child(grip)),
                    )
                    .into_any_element()
            })
            .size_full()
            .list_shell_scroll_under_chrome(cx),
        )
    }
}

impl PlaylistReorderList {
    fn drop_drag(state: &Entity<PlaylistReorderListState>, playlist_id: &str, cx: &mut App) {
        let landed = state.update(cx, |state, cx| {
            let drag = state.drag.take()?;
            cx.notify();
            if drag.from >= state.order.len() {
                return None;
            }
            let target = drag.target(state.order.len());
            let previous = state.order.clone();
            let mut next = previous.clone();
            let moved = next.remove(drag.from);
            let moved_id = moved.id.clone();
            next.insert(target, moved);
            if PlaylistReorderListState::ids(&next) == PlaylistReorderListState::ids(&previous) {
                return None;
            }
            let after_track_id = if target > 0 { Some(next[target - 1].id.clone()) } else { None };
            state.order = next;
            Some((moved_id, after_track_id, previous))
        });
        let Some((moved_id, after_track_id, previous)) = landed else {
            return;
        };
        let rollback_state = state.downgrade();
        let previous = Rc::new(previous);
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.move_playlist_track(
                playlist_id,
                &moved_id,
                after_track_id.as_deref(),
                Rc::new(move |cx: &mut App| {
                    if let Some(state) = rollback_state.upgrade() {
                        let previous = previous.as_ref().clone();
                        state.update(cx, |state, cx| {
                            state.replace_order(previous);
                            cx.notify();
                        });
                    }
                }),
                cx,
            );
        });
    }
}

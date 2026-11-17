use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, BoxShadow, Context, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, SharedString, Subscription,
    Task, Window, black, point, rgb, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment_values::ColorScheme;
use rise_ui::swiftui::motion::{ease_out, spring};
use rise_ui::swiftui::page_refresh::{PageFocus, is_page_refresh_keystroke};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition, TransitionModifiers};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::music::pages::track::track_context_menu_row::{TrackContextMenuRow, TrackContextSource};
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::TrackPickerListModel;
use crate::modules::music::widgets::music_mini_player_bottom_padding::{
    MusicMiniPlayerBottomPaddingModifiers, observed_music_player,
};
use crate::modules::music::widgets::music_skeletons::TrackListSkeleton;

const ESCAPE_KEY: &str = "escape";
const TRACK_PREFETCH_THRESHOLD: usize = 8;
const DELETE_DIALOG_FADE: f64 = 0.18;
const DELETE_DIALOG_RADIUS: f32 = 28.0;
const DELETE_DIALOG_MAX_WIDTH: f32 = 300.0;
const DELETE_DIALOG_WARNING_LINE_HEIGHT: f32 = 22.0;
const DELETE_DIALOG_BUTTON_HEIGHT: f32 = 48.0;
const DESTRUCTIVE_COLOR: u32 = 0xFF5752;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn refresh_my_local_tracks(cx: &mut App) {
    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_my_local_tracks(cx));
}

struct LocalMusicTracksState {
    pending_delete_track_id: Option<String>,
    submitted_delete_track_id: Option<String>,
    delete_dialog_presence: Presence<()>,
    list_model: TrackPickerListModel,
    focus: FocusHandle,
    _subscriptions: [Subscription; 2],
}

impl LocalMusicTracksState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        Self {
            pending_delete_track_id: None,
            submitted_delete_track_id: None,
            delete_dialog_presence: Presence::new(None).with_transition(Transition::opacity()),
            list_model: TrackPickerListModel::new(cx),
            focus: cx.focus_handle(),
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// fades the pending delete track in or out, 0.18s ease out
    fn set_pending_delete_track_id(&mut self, track_id: Option<String>, cx: &mut Context<Self>) {
        let presence = track_id.is_some().then_some(());
        self.pending_delete_track_id = track_id;
        self.delete_dialog_presence.set_in(presence, ease_out(DELETE_DIALOG_FADE), RiseClock::monotonic(), cx);
        cx.notify();
    }
}

fn is_delete_pending(cx: &mut App) -> bool {
    MusicActionsStore::slices(cx).delete_local_tracks_sai.is_pending()
}

#[derive(IntoElement)]
pub struct LocalMusicTracks {
    id: ElementId,
}

impl Default for LocalMusicTracks {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalMusicTracks {
    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static("local-music-tracks")) }
    }

    fn handle_back(state: &Entity<LocalMusicTracksState>, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        if state.read(cx).pending_delete_track_id.is_some() {
            if is_delete_pending(cx) {
                return;
            }
            state.update(cx, |state, cx| state.set_pending_delete_track_id(None, cx));
            return;
        }
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn cancel_delete(state: &Entity<LocalMusicTracksState>, cx: &mut App) {
        if is_delete_pending(cx) {
            return;
        }
        state.update(cx, |state, cx| state.set_pending_delete_track_id(None, cx));
    }

    fn delete_pending_track(state: &Entity<LocalMusicTracksState>, cx: &mut App) {
        let Some(track_id) = state.read(cx).pending_delete_track_id.clone() else {
            return;
        };
        state.update(cx, |state, cx| {
            state.submitted_delete_track_id = Some(track_id.clone());
            cx.notify();
        });

        let success_state = state.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            success_state.update(cx, |state, cx| {
                state.submitted_delete_track_id = None;
                state.set_pending_delete_track_id(None, cx);
            });
        });
        let error_state = state.clone();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |_: String, cx: &mut App| {
            error_state.update(cx, |state, cx| {
                state.submitted_delete_track_id = None;
                state.set_pending_delete_track_id(None, cx);
            });
            Notifier::send(
                tr("music_local_tracks_delete_failed_title"),
                NotifierSendOptions {
                    content: Some(tr("music_local_tracks_delete_failed_text")),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.delete_local_track(&track_id, Some(on_success), Some(on_error), cx);
        });
    }
}

impl RenderOnce for LocalMusicTracks {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| LocalMusicTracksState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let tracks = MusicServicesStore::shared(cx).update(cx, |service, cx| service.local_tracks(cx));
        let track_ids: Vec<String> = tracks.iter().map(|track| track.id.clone()).collect();
        let (focus, is_dialog_open) = {
            let local = state.read(cx);
            (local.focus.clone(), local.pending_delete_track_id.is_some())
        };

        let ids_state = state.clone();
        on_change(named_child(&self.id, "track-ids"), track_ids, false, window, cx, move |_, track_ids, _, cx| {
            let (pending, submitted) = {
                let local = ids_state.read(cx);
                (local.pending_delete_track_id.clone(), local.submitted_delete_track_id.is_some())
            };
            let Some(pending) = pending else {
                return;
            };
            if submitted || track_ids.contains(&pending) {
                return;
            }
            ids_state.update(cx, |state, cx| state.set_pending_delete_track_id(None, cx));
        });

        let refocus = focus.clone();
        on_change(named_child(&self.id, "dialog-open"), is_dialog_open, false, window, cx, move |_, is_open, window, cx| {
            if !*is_open {
                window.focus(&refocus, cx);
            }
        });

        let dialog_id = named_child(&self.id, "delete-dialog");
        let dialog_state = state.clone();
        let is_pending = is_delete_pending(cx);
        let now = RiseClock::monotonic();
        let dialog = state.read(cx).delete_dialog_presence.render_transitioned(now, window, |_| {
            let delete_state = dialog_state.clone();
            let cancel_state = dialog_state.clone();
            div().absolute().inset_0().child(LocalMusicTracksDeleteDialog {
                id: dialog_id.clone(),
                is_pending,
                on_delete: Rc::new(move |cx: &mut App| LocalMusicTracks::delete_pending_track(&delete_state, cx)),
                on_cancel: Rc::new(move |cx: &mut App| LocalMusicTracks::cancel_delete(&cancel_state, cx)),
            })
        });

        let back_state = state.clone();
        let back_nav = nav_store;
        let key_state = state.clone();
        let page_focus = PageFocus::keyed_with(named_child(&self.id, "page-focus"), focus.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        div()
            .id(named_child(&self.id, "keys"))
            .relative()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                if is_page_refresh_keystroke(keystroke) {
                    cx.stop_propagation();
                    refresh_my_local_tracks(cx);
                } else if keystroke.key == ESCAPE_KEY && key_state.read(cx).pending_delete_track_id.is_some() {
                    cx.stop_propagation();
                    LocalMusicTracks::cancel_delete(&key_state, cx);
                }
            })
            .child(
                SettingsWrapperUi::new(tr("music_local_tracks_title"), move |_, cx| {
                    Self::handle_back(&back_state, &back_nav, cx);
                })
                .content(LocalMusicTracksBody { id: named_child(&self.id, "body"), state: state.clone(), tracks: Rc::new(tracks) }),
            )
            .children(dialog)
            .on_appear(named_child(&self.id, "appear"), move |window, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_my_local_tracks_if_needed(cx));
                PageFocus::did_appear(&page_focus, window, cx);
            })
            .on_disappear(named_child(&self.id, "disappear"), move |window, cx| {
                HoldContextMenuPresenter::dismiss(cx);
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

#[derive(IntoElement)]
struct LocalMusicTracksBody {
    id: ElementId,
    state: Entity<LocalMusicTracksState>,
    tracks: Rc<Vec<TrackModel>>,
}

impl LocalMusicTracksBody {
    /// placeholder fills the viewport and the pull still refreshes
    fn refreshable_state(id: ElementId, content: Div, cx: &App) -> AnyElement {
        scroll_view(id)
            .shows_indicators(false)
            .size_full()
            .refreshable(|_, cx| {
                refresh_my_local_tracks(cx);
                Task::ready(())
            })
            .list_shell_scroll_under_chrome(cx)
            .child(div().w_full().min_h_full().flex().items_center().justify_center().child(content))
            .into_any_element()
    }

    fn local_tracks_empty_state(theme: &AppTheme) -> Div {
        vstack(12.0)
            .px(px(32.0))
            .children(IconUi::sized("music.note.list", px(34.0), theme.text.secondary, false))
            .child(SecondaryText::new(tr("music_local_tracks_empty")).px(14.0).tac(TextAlignT::Center))
    }

    fn local_tracks_error_state(id: &ElementId, theme: &AppTheme) -> Div {
        vstack(12.0)
            .px(px(32.0))
            .children(IconUi::sized("exclamationmark.triangle.fill", px(30.0), theme.semantic.error_100, false))
            .child(SecondaryText::new(tr("error_fetching_data")).px(14.0).tac(TextAlignT::Center))
            .child(
                ButtonUi::new(named_child(id, "retry"))
                    .background_color(theme.primary._100)
                    .height(42.0)
                    .fit_content(true)
                    .padding_horizontal(18.0)
                    .on_press(|_, cx| refresh_my_local_tracks(cx))
                    .child(MainText::new(tr("music_retry")).px(14.0).font_weight(FontWeight::SEMIBOLD)),
            )
    }

    fn track_list(
        id: ElementId,
        state: Entity<LocalMusicTracksState>,
        tracks: Rc<Vec<TrackModel>>,
        top_inset: f32,
        cx: &mut App,
    ) -> AnyElement {
        let player = MusicPlayerState::shared(cx);
        let (current_track_id, is_playing) = {
            let player = player.read(cx);
            (player.current_track.as_ref().map(|track| track.id.clone()), player.is_playing)
        };
        let theme = rise_ui::theme(cx).clone();
        let count = tracks.len();

        let mut ids: Vec<SharedString> = Vec::with_capacity(count + 2);
        ids.push(SharedString::new_static("top-inset"));
        ids.extend(tracks.iter().map(|track| SharedString::from(track.id.clone())));
        ids.push(SharedString::new_static("foot"));

        let cache_key = MusicServicesStore::my_local_tracks_cache_key();
        let list_id = named_child(&id, "list");
        let row_state = state.clone();
        let render_row = move |index: usize, _: &mut Window, cx: &mut App| -> AnyElement {
                    if index == 0 {
                        return div().w_full().h(px(top_inset + 8.0)).into_any_element();
                    }
                    let Some(track) = tracks.get(index - 1) else {
                        return div().w_full().music_mini_player_bottom_padding(80.0, 0.0).into_any_element();
                    };
                    let position = index - 1;
                    let delete_state = row_state.clone();
                    let delete_track_id = track.id.clone();
                    let play_track = track.clone();
                    let play_queue = Rc::clone(&tracks);
                    let row = TrackContextMenuRow::new(named_child(&id, format!("track-{}", track.id)), track.clone())
                        .context(TrackContextSource::External)
                        .is_playing(current_track_id.as_deref() == Some(track.id.as_str()) && is_playing)
                        .shows_like_button(false)
                        .allows_local_track_editing(true)
                        .on_delete_local_track(move |_, cx| {
                            let track_id = delete_track_id.clone();
                            delete_state.update(cx, |state, cx| state.set_pending_delete_track_id(Some(track_id), cx));
                        })
                        .on_tap(move |_, cx| {
                            let track = play_track.clone();
                            let queue = (*play_queue).clone();
                            let on_playback_accepted: Box<dyn FnOnce(&mut App)> = Box::new(|cx: &mut App| {
                                MusicPlayerState::shared(cx)
                                    .update(cx, |player, cx| player.set_show_now_playing(true, cx));
                            });
                            MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.play_local_track(track, queue, Some(on_playback_accepted), cx);
                            });
                        });
                    vstack(0.0)
                        .w_full()
                        .child(div().w_full().px(px(16.0)).child(row.music_pagination_prefetch(
                            named_child(&id, format!("track-prefetch-{}", track.id)),
                            position,
                            count,
                            &cache_key,
                            true,
                            TRACK_PREFETCH_THRESHOLD,
                            |_, cx| {
                                MusicInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.load_more_my_local_tracks(cx));
                            },
                            cx,
                        )))
                        .when(position + 1 < count, |column| {
                            column.child(
                                div()
                                    .w_full()
                                    .pl(px(76.0))
                                    .pr(px(16.0))
                                    .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))),
                            )
                        })
                        .into_any_element()
        };
        state
            .update(cx, |state, _| {
                state.list_model.sync(ids);
                state.list_model.list().refreshable(
                    list_id,
                    |_, cx| {
                        refresh_my_local_tracks(cx);
                        Task::ready(())
                    },
                    render_row,
                )
            })
            .into_any_element()
    }
}

impl RenderOnce for LocalMusicTracksBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let theme = rise_ui::theme(cx).clone();
        let _player = observed_music_player(named_child(&self.id, "player"), window, cx);
        let actions = MusicActionsStore::shared(cx);
        let slice = Rc::clone(&actions.read(cx).list_my_local_tracks_sai);
        let is_empty = self.tracks.is_empty();

        let (empty_id, empty_theme) = (named_child(&self.id, "empty"), theme.clone());
        let (error_id, error_theme) = (named_child(&self.id, "error"), theme.clone());
        let loading_id = named_child(&self.id, "loading");
        let (list_id, list_state, list_tracks) = (self.id.clone(), self.state, self.tracks);

        div().size_full().bg(theme.bg._100).child(
            SaiDataRender::slice(&*slice, move |_, cx| {
                Self::track_list(list_id, list_state, list_tracks, top_inset, cx)
            })
            .id(named_child(&self.id, "render"))
            .is_empty(is_empty)
            .no_data_text(tr("music_local_tracks_empty"))
            .need_pending(is_empty)
            .sai_data_render_empty(move |_, cx| {
                Self::refreshable_state(empty_id.clone(), Self::local_tracks_empty_state(&empty_theme), cx)
            })
            .sai_data_render_error(move |_, cx| {
                let content = Self::local_tracks_error_state(&error_id, &error_theme);
                Self::refreshable_state(error_id.clone(), content, cx)
            })
            .sai_data_render_loading(move |_, cx| {
                scroll_view(loading_id.clone())
                    .scroll_disabled(true)
                    .shows_indicators(false)
                    .size_full()
                    .list_shell_scroll_under_chrome(cx)
                    .child(div().w_full().pt(px(8.0)).child(TrackListSkeleton::new().rows(8).horizontal_padding(16.0)))
            }),
        )
    }
}

#[derive(IntoElement)]
struct LocalMusicTracksDeleteDialog {
    id: ElementId,
    is_pending: bool,
    on_delete: Rc<dyn Fn(&mut App)>,
    on_cancel: Rc<dyn Fn(&mut App)>,
}

impl LocalMusicTracksDeleteDialog {
    fn dialog_button(
        id: ElementId,
        title: String,
        destructive: bool,
        is_pending: bool,
        disabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let destructive_color: Hsla = rgb(DESTRUCTIVE_COLOR).into();
        let label: AnyElement = if is_pending {
            progress_view().tint(destructive_color).into_any_element()
        } else {
            div()
                .swiftui_font(Font::system_weight(17.0, if destructive { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }))
                .text_color(if destructive { destructive_color } else { white() })
                .whitespace_nowrap()
                .overflow_hidden()
                .child(title)
                .into_any_element()
        };
        let action: ButtonAction = Rc::new(action);
        div()
            .w_full()
            .h(px(DELETE_DIALOG_BUTTON_HEIGHT))
            .px(px(10.0))
            .flex()
            .items_center()
            .justify_center()
            .capsule()
            .bg(white().opacity(0.11))
            .child(label)
            .liquid_press(id, LiquidPressShape::Capsule, true, disabled || is_pending, false, Some(action))
    }
}

impl RenderOnce for LocalMusicTracksDeleteDialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let appeared = window.use_keyed_state(named_child(&self.id, "appeared"), cx, |_, cx: &mut Context<Animated<f32>>| {
            let mut appeared = Animated::new(0.0_f32);
            appeared.set_in(1.0, spring(0.28, 0.86), RiseClock::monotonic(), cx);
            appeared
        });
        let progress = appeared.read(cx).frame_value(RiseClock::monotonic(), window).clamp(0.0, 1.0);
        let is_pending = self.is_pending;
        let on_delete = self.on_delete;
        let on_cancel = self.on_cancel.clone();
        let on_backdrop = self.on_cancel;

        let card = vstack(0.0)
            .relative()
            .w_full()
            .max_w(px(DELETE_DIALOG_MAX_WIDTH))
            .rounded(px(DELETE_DIALOG_RADIUS))
            .overflow_hidden()
            .occlude()
            .border_1()
            .border_color(white().opacity(0.09))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.4),
                offset: point(px(0.0), px(14.0)),
                blur_radius: px(28.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(
                HoldContextMenuGlass::new("local-music-delete-dialog-glass")
                    .tint(black().opacity(0.24))
                    .corner_radius(DELETE_DIALOG_RADIUS)
                    .color_scheme(ColorScheme::Dark),
            )
            .child(
                div()
                    .mt(px(20.0))
                    .size(px(72.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .circle()
                    .bg(white().opacity(0.1))
                    .children(IconUi::sized("trash.fill", px(30.0), Hsla::from(rgb(DESTRUCTIVE_COLOR)), false)),
            )
            .child(
                vstack(6.0)
                    .w_full()
                    .align(HorizontalAlignment::Leading)
                    .px(px(24.0))
                    .pt(px(24.0))
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(17.0, FontWeight::SEMIBOLD))
                            .text_color(white())
                            .child(tr("music_local_tracks_delete_title")),
                    )
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(17.0, FontWeight::NORMAL))
                            .line_height(px(DELETE_DIALOG_WARNING_LINE_HEIGHT))
                            .text_color(white().opacity(0.88))
                            .child(tr("music_local_tracks_delete_warning")),
                    ),
            )
            .child(
                vstack(8.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(20.0))
                    .pb(px(16.0))
                    .child(div().w_full().child(Self::dialog_button(
                        named_child(&self.id, "delete"),
                        tr("ctx_delete"),
                        true,
                        is_pending,
                        false,
                        move |_, cx| on_delete(cx),
                    )))
                    .child(div().w_full().opacity(if is_pending { 0.45 } else { 1.0 }).child(Self::dialog_button(
                        named_child(&self.id, "cancel"),
                        tr("common_cancel"),
                        false,
                        false,
                        is_pending,
                        move |_, cx| on_cancel(cx),
                    ))),
            )
            .opacity(progress)
            .scale_effect(0.94 + 0.06 * progress);

        div()
            .id(named_child(&self.id, "root"))
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .px(px(16.0))
            .child(
                div()
                    .id(named_child(&self.id, "backdrop"))
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(0.52))
                    .on_click(move |_, _, cx| {
                        if !is_pending {
                            on_backdrop(cx);
                        }
                    }),
            )
            .child(card)
    }
}

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Subscription, Window, rgb};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{Locale, tr};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::music::shared::utils::music::music_models::{TrackModel, display_genre};
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{ProtoTrackRecordExt, RiseStoredMusicTrack};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::studio::publish_schedule_sheet::{
    PublishScheduleSheet, PublishScheduleSheetLayout,
};
use crate::modules::music::widgets::music_exclusive_badge::MusicExclusiveBadge;
use crate::modules::music::widgets::music_mini_player_bottom_padding::observed_music_player;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;

const SCHEDULED_DATE_FORMAT: &str = "d MMM yyyy, HH:mm";
const PUBLISHED_COLOR: u32 = 0x00B894;
const PENDING_COLOR: u32 = 0xFDCB6E;
const CARD_RADIUS: f32 = 12.0;

fn named_child(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// state of the screen and the music store
struct StudioTrackDetailState {
    show_publish_sheet: bool,
    publish_sheet_fraction: f32,
    is_publishing: bool,
    _subscriptions: [Subscription; 2],
}

impl StudioTrackDetailState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        Self {
            show_publish_sheet: false,
            publish_sheet_fraction: PublishScheduleSheetLayout::RESTING_FRACTION,
            is_publishing: false,
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct StudioTrackDetail {
    id: ElementId,
    track_id: String,
}

impl StudioTrackDetail {
    pub fn new(track_id: impl Into<String>) -> Self {
        let track_id = track_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("studio-track-detail-{track_id}"))), track_id }
    }

    fn publish_track(
        track_id: &str,
        state: &Entity<StudioTrackDetailState>,
        nav_store: &Option<Entity<NavigationStore>>,
        scheduled_at_ms: Option<i64>,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| {
            state.is_publishing = true;
            cx.notify();
        });
        let success_state = state.clone();
        let success_nav = nav_store.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            success_state.update(cx, |state, cx| {
                state.is_publishing = false;
                cx.notify();
            });
            if let Some(nav_store) = &success_nav {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        });
        let error_state = state.clone();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            error_state.update(cx, |state, cx| {
                state.is_publishing = false;
                cx.notify();
            });
            NativeConfirmAlert::present_notice(
                &tr("music_upload_error_title"),
                &message,
                None,
                None,
                |_: &mut App| {},
                cx,
            );
        });
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.publish_approved_track(track_id, scheduled_at_ms, Some(on_success), Some(on_error), cx);
        });
    }

    fn publish_sheet(
        &self,
        state: &Entity<StudioTrackDetailState>,
        nav_store: &Option<Entity<NavigationStore>>,
        theme: &AppTheme,
    ) -> BottomSheetUi {
        let sheet_id = named_child(&self.id, "publish");
        let track_id = self.track_id.clone();
        let content_state = state.clone();
        let content_nav = nav_store.clone();
        let fraction = Binding::entity(
            state.clone(),
            |state: &StudioTrackDetailState| state.publish_sheet_fraction,
            |state, value, _| state.publish_sheet_fraction = value,
        );
        let sheet_fraction = fraction.clone();
        BottomSheetUi::new(
            named_child(&self.id, "publish-sheet"),
            Binding::entity(
                state.clone(),
                |state: &StudioTrackDetailState| state.show_publish_sheet,
                |state, value, _| state.show_publish_sheet = value,
            ),
            move |_, _| {
                let (now_id, now_state, now_nav) = (track_id.clone(), content_state.clone(), content_nav.clone());
                let (later_id, later_state, later_nav) = (track_id.clone(), content_state.clone(), content_nav.clone());
                let dismiss = content_state.clone();
                PublishScheduleSheet::new(
                    sheet_id.clone(),
                    move |_, cx| Self::publish_track(&now_id, &now_state, &now_nav, None, cx),
                    move |date, _, cx| {
                        let scheduled_at_ms = (date.time_interval_since_1970() * 1000.0) as i64;
                        Self::publish_track(&later_id, &later_state, &later_nav, Some(scheduled_at_ms), cx);
                    },
                )
                .on_dismiss(move |_, cx| {
                    dismiss.update(cx, |state, cx| {
                        state.show_publish_sheet = false;
                        cx.notify();
                    });
                })
                .sheet_fraction(sheet_fraction.clone())
            },
            |_, _| div(),
        )
        .snap_fractions(PublishScheduleSheetLayout::SNAP_FRACTIONS.to_vec())
        .modal_sized_by_detent(true)
        .selected_snap_fraction(Some(fraction))
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
    }
}

impl RenderOnce for StudioTrackDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| StudioTrackDetailState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let theme = rise_ui::theme(cx).clone();
        let back_store = nav_store.clone();
        div()
            .relative()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr("music_studio_track_detail"), move |_, cx| {
                    if let Some(nav_store) = &back_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(StudioTrackDetailBody {
                    id: named_child(&self.id, "body"),
                    track_id: self.track_id.clone(),
                    state: state.clone(),
                }),
            )
            .child(self.publish_sheet(&state, &nav_store, &theme))
            .on_appear(named_child(&self.id, "appear"), |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_my_tracks_if_needed(cx));
            })
    }
}

#[derive(IntoElement)]
struct StudioTrackDetailBody {
    id: ElementId,
    track_id: String,
    state: Entity<StudioTrackDetailState>,
}

impl StudioTrackDetailBody {
    fn wire(&self, store: &MusicActionsStore) -> Option<RiseStoredMusicTrack> {
        store
            .list_my_tracks_sai
            .data()
            .and_then(|data| data.tracks.iter().find(|track| track.id == self.track_id).map(ProtoTrackRecordExt::stored))
    }

    fn unavailable_key(&self, has_wire: bool, cx: &mut App) -> Option<&'static str> {
        if has_wire {
            return None;
        }
        let (status, has_data) = {
            let actions = MusicActionsStore::shared(cx);
            let slice = &actions.read(cx).list_my_tracks_sai;
            (slice.status(), slice.data().is_some())
        };
        match status {
            SaiWsSliceStatus::Rejected if !has_data => Some("music_track_link_load_failed"),
            SaiWsSliceStatus::Fulfilled | SaiWsSliceStatus::Rejected => {
                let cache_key = MusicServicesStore::my_tracks_cache_key();
                let page = MusicServicesStore::shared(cx).update(cx, |service, cx| service.pagination_state(&cache_key, cx));
                if page.has_more { None } else { Some("music_track_link_not_found") }
            }
            SaiWsSliceStatus::Idle | SaiWsSliceStatus::Pending => None,
        }
    }

    fn cover_section(&self, track: &TrackModel, track_status: &str, theme: &AppTheme) -> Div {
        vstack(12.0)
            .w_full()
            .child(
                MusicTrackCoverImage::new(named_child(&self.id, "cover"), track.cover_image_url.clone())
                    .size(160.0)
                    .corner_radius(16.0),
            )
            .child(
                MainText::new(track.title.clone()).px(20.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD),
            )
            .child(MainText::new(track.artist.clone()).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary))
            .children(Self::status_badge(track_status, theme))
    }

    fn info_cards(&self, wire: &RiseStoredMusicTrack, track_status: &str, theme: &AppTheme) -> Div {
        let status_key = if track_status.is_empty() { RiseMusicTrackStatus::Pending.wire() } else { track_status };
        let is_exclusive = wire.is_exclusive == Some(true);
        vstack(10.0)
            .w_full()
            .when_some(wire.genre.as_deref().filter(|genre| !genre.is_empty()), |column, genre| {
                column.child(Self::labeled_card(tr("music_artist_genre"), display_genre(genre), theme))
            })
            .when_some(
                wire.album.as_ref().map(|album| album.title.as_str()).filter(|title| !title.is_empty()),
                |column, album_title| column.child(Self::labeled_card(tr("music_track_album"), album_title.to_owned(), theme)),
            )
            .child(Self::labeled_card(tr("music_status_label"), tr(&format!("music_status_{status_key}")), theme))
            .when_some(wire.rejection_reason.as_deref().filter(|reason| !reason.is_empty()), |column, reason| {
                column.child(
                    Self::labeled_card(tr("music_rejection_reason"), reason.to_owned(), theme)
                        .border_1()
                        .border_color(theme.semantic.error_100.opacity(0.3)),
                )
            })
            .when_some(wire.scheduled_at_ms.filter(|scheduled| *scheduled > 0), |column, scheduled| {
                column.child(Self::labeled_card(
                    tr("music_scheduled_release"),
                    Self::format_scheduled_date(scheduled),
                    theme,
                ))
            })
            .when(is_exclusive, |column| {
                column.child(
                    hstack(0.0)
                        .w_full()
                        .p(px(14.0))
                        .bg(theme.bg._200)
                        .rounded(px(CARD_RADIUS))
                        .child(MusicExclusiveBadge::new())
                        .child(spacer()),
                )
            })
            .when(Self::can_edit_exclusive(track_status), |column| {
                let track_id = self.track_id.clone();
                column.child(
                    hstack(12.0)
                        .w_full()
                        .p(px(14.0))
                        .bg(theme.bg._200)
                        .rounded(px(CARD_RADIUS))
                        .child(
                            vstack(2.0)
                                .flex_1()
                                .min_w_0()
                                .align(HorizontalAlignment::Leading)
                                .child(MainText::new(tr("music_exclusive_toggle_title")).px(15.0).color(theme.text.primary))
                                .child(SecondaryText::new(tr("music_exclusive_toggle_hint")).px(12.0)),
                        )
                        .child(SwitchUi::new(
                            named_child(&self.id, "exclusive"),
                            Binding::new(
                                move |_: &App| is_exclusive,
                                move |value: bool, _, cx| {
                                    MusicInteractionsStore::shared(cx)
                                        .update(cx, |store, cx| store.toggle_track_exclusive(&track_id, value, cx));
                                },
                            ),
                        )),
                )
            })
    }

    fn can_edit_exclusive(track_status: &str) -> bool {
        [RiseMusicTrackStatus::Pending, RiseMusicTrackStatus::Approved, RiseMusicTrackStatus::Rejected]
            .iter()
            .any(|status| status.wire() == track_status)
    }

    fn preview_button(&self, track: &TrackModel, player: &MusicPlayerState, theme: &AppTheme) -> ButtonUi {
        let is_current = player.current_track.as_ref().is_some_and(|current| current.id == track.id);
        let is_preparing = player.is_preparing_playback && is_current;
        let symbol = if player.is_playing && is_current { "pause.fill" } else { "play.fill" };
        let preview_track = track.clone();
        ButtonUi::new(named_child(&self.id, "preview"))
            .disabled(is_preparing)
            .on_press(move |_, cx| {
                let track = preview_track.clone();
                let preview = MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.preview_track(track, cx));
                cx.spawn(async move |cx| {
                    if preview.await {
                        let _ = cx.update(|cx| {
                            MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
                        });
                    }
                })
                .detach();
            })
            .child(
                hstack(8.0)
                    .when(is_preparing, |line| line.child(progress_view().tint(theme.primary._100)))
                    .when(!is_preparing, |line| line.children(IconUi::sized(symbol, px(15.0), theme.text.primary, false)))
                    .child(
                        MainText::new(if is_preparing { tr("music_preview_loading") } else { tr("music_preview_track") })
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD),
                    ),
            )
    }

    fn action_buttons(
        &self,
        wire: &RiseStoredMusicTrack,
        track: &TrackModel,
        track_status: &str,
        is_publishing: bool,
        player: &MusicPlayerState,
        theme: &AppTheme,
    ) -> Div {
        let is_album_track = wire.album.as_ref().is_some_and(|album| !album.id.is_empty());
        let publish_state = self.state.clone();
        vstack(10.0)
            .w_full()
            .when(!track.audio_url.is_empty(), |column| column.child(self.preview_button(track, player, theme)))
            .when(track_status == RiseMusicTrackStatus::Approved.wire(), |column| {
                if is_album_track {
                    column.child(
                        hstack(10.0)
                            .w_full()
                            .p(px(14.0))
                            .bg(theme.primary._100.opacity(0.08))
                            .rounded(theme.radius._300)
                            .children(IconUi::sized("info.circle", px(15.0), theme.primary._100, false))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(SecondaryText::new(tr("music_track_publishes_with_album")).px(13.0)),
                            ),
                    )
                } else {
                    column.child(
                        ButtonUi::new(named_child(&self.id, "publish"))
                            .disabled(is_publishing)
                            .on_press(move |_, cx| {
                                publish_state.update(cx, |state, cx| {
                                    state.show_publish_sheet = true;
                                    cx.notify();
                                });
                            })
                            .child(
                                hstack(8.0)
                                    .children(IconUi::sized("arrow.up.circle.fill", px(15.0), theme.text.primary, true))
                                    .child(MainText::new(tr("music_publish")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
                            ),
                    )
                }
            })
    }

    fn labeled_card(label: String, value: String, theme: &AppTheme) -> Div {
        vstack(6.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .p(px(16.0))
            .bg(theme.bg._200)
            .rounded(px(CARD_RADIUS))
            .child(MainText::new(label).px(12.0).color(theme.text.secondary))
            .child(MainText::new(value).px(14.0))
    }

    fn status_badge(status: &str, theme: &AppTheme) -> Option<Div> {
        if status.is_empty() {
            return None;
        }
        let color = Self::status_color(status, theme);
        Some(
            div()
                .px(px(10.0))
                .py(px(4.0))
                .capsule()
                .bg(color.opacity(0.12))
                .whitespace_nowrap()
                .swiftui_font(Font::system_weight(12.0, FontWeight::SEMIBOLD))
                .text_color(color)
                .child(tr(&format!("music_status_{status}"))),
        )
    }

    fn status_color(status: &str, theme: &AppTheme) -> Hsla {
        if status == RiseMusicTrackStatus::Published.wire() {
            rgb(PUBLISHED_COLOR).into()
        } else if status == RiseMusicTrackStatus::Approved.wire() {
            theme.primary._100
        } else if status == RiseMusicTrackStatus::Rejected.wire() {
            theme.semantic.error_100
        } else {
            rgb(PENDING_COLOR).into()
        }
    }

    fn format_scheduled_date(ms: i64) -> String {
        let date = Date::from_time_interval_since_1970(ms as f64 / 1000.0);
        let mut formatter = DateFormatter::new();
        formatter.set_locale(Locale::app_locale().identifier());
        formatter.set_date_format(SCHEDULED_DATE_FORMAT);
        formatter.string_from(&date)
    }
}

impl RenderOnce for StudioTrackDetailBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let player = observed_music_player(named_child(&self.id, "player"), window, cx);
        let wire = {
            let actions = MusicActionsStore::shared(cx);
            self.wire(actions.read(cx))
        };
        let track = wire.as_ref().and_then(RiseStoredMusicTrack::as_track_model);
        let unavailable_key = self.unavailable_key(wire.is_some(), cx);
        let is_publishing = self.state.read(cx).is_publishing;
        let track_status = wire.as_ref().and_then(|wire| wire.status.clone()).unwrap_or_default();

        let content: AnyElement = if let (Some(wire), Some(track)) = (wire.as_ref(), track.as_ref()) {
            vstack(16.0)
                .w_full()
                .px(px(16.0))
                .pt(px(16.0))
                .child(self.cover_section(track, &track_status, &theme))
                .child(self.info_cards(wire, &track_status, &theme))
                .child(self.action_buttons(wire, track, &track_status, is_publishing, player.read(cx), &theme))
                .child(div().w_full().h(px(80.0)))
                .into_any_element()
        } else if let Some(key) = unavailable_key {
            vstack(16.0)
                .w_full()
                .px(px(16.0))
                .pt(px(80.0))
                .children(IconUi::sized("music.note", px(48.0), theme.text.secondary.opacity(0.4), false))
                .child(MainText::new(tr(key)).px(16.0).tac(TextAlignT::Center).color(theme.text.secondary))
                .into_any_element()
        } else {
            vstack(0.0).w_full().pt(px(80.0)).child(progress_view().tint(theme.primary._100)).into_any_element()
        };

        let cache_key = MusicServicesStore::my_tracks_cache_key();
        let filtered_item_count = usize::from(wire.is_some());
        let drained = content.music_pagination_drain_filtered_empty(
            named_child(&self.id, "drain"),
            filtered_item_count,
            &cache_key,
            true,
            |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_tracks(cx));
            },
            cx,
        );

        scroll_view(named_child(&self.id, "scroll"))
            .size_full()
            .bg(theme.bg._100)
            .list_shell_scroll_under_chrome(cx)
            .child(drained)
    }
}

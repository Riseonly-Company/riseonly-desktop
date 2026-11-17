use std::collections::HashMap;

use gpui::{App, Entity, FontWeight, Hsla, SharedString, Subscription, Window, transparent_black};
use rise_canon::rise_service_text::music_service;
use rise_catalog::MusicGenre;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::{Animated, Lerp};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::input_ui::InputOutlinedRowUi;
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    RequestScrollBody, hex_color, pop_screen,
};
use crate::modules::music::shared::utils::music::music_models::{canon_genre, display_genre};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;

const APPROVED_COLOR: u32 = 0x00B894;
const NOTICE_COLOR: u32 = 0xF19A1A;
const BIO_FIELD: &str = "bio";
const BIO_MIN_LINES: usize = 5;
const BANNER_ICON_SIDE: f32 = 20.0;
const INSTAGRAM_PLACEHOLDER: &str = "@username";

struct BecomeArtistSettingsState {
    stage_name: String,
    selected_genre: String,
    bio: String,
    instagram: String,
    did_prefill_rejected_form: bool,
    _subscriptions: [Subscription; 2],
}

impl BecomeArtistSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            stage_name: String::new(),
            selected_genre: String::new(),
            bio: String::new(),
            instagram: String::new(),
            did_prefill_rejected_form: false,
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn prefill_from_last_request_if_needed(state: &Entity<Self>, cx: &mut App) {
        let actions = MusicActionsStore::shared(cx);
        let store = actions.read(cx);
        let is_rejected = store
            .get_artist_status_sai
            .data()
            .is_some_and(|data| data.status == music_service::ARTIST_STATUS_REJECTED);
        let request = store.get_my_artist_request_sai.data().and_then(|data| data.request);
        let (true, Some(r)) = (is_rejected, request) else {
            return;
        };
        state.update(cx, |state, cx| {
            if state.did_prefill_rejected_form {
                return;
            }
            state.did_prefill_rejected_form = true;
            if state.stage_name.is_empty() && !r.stage_name.is_empty() {
                state.stage_name = r.stage_name;
            }
            if state.selected_genre.is_empty()
                && let Some(genre) = canon_genre(&r.genre)
            {
                state.selected_genre = genre.wire().to_owned();
            }
            if state.bio.is_empty() && !r.bio.is_empty() {
                state.bio = r.bio;
            }
            if state.instagram.is_empty()
                && let Some(instagram) = r.instagram.filter(|value| !value.is_empty())
            {
                state.instagram = instagram;
            }
            cx.notify();
        });
    }
}

#[derive(IntoElement, Default)]
pub struct BecomeArtistSettings;

impl BecomeArtistSettings {
    pub fn new() -> Self {
        Self
    }

    fn banner(alignment_top: bool, symbol: &str, tint: Hsla, content: impl IntoElement, theme: &AppTheme) -> Div {
        hstack(12.0)
            .when(alignment_top, |row| row.items_start())
            .w_full()
            .p(px(14.0))
            .bg(tint.opacity(0.1))
            .continuous_corners(f32::from(theme.radius._300))
            .children(IconUi::sized(symbol, px(BANNER_ICON_SIDE), tint, symbol.ends_with(".fill")))
            .child(div().flex_1().min_w(px(0.0)).child(content))
    }

    fn horizontal_inset(content: impl IntoElement) -> Div {
        div().w_full().px(px(16.0)).child(content)
    }

    fn approved_banner(nav_store: Option<Entity<NavigationStore>>, theme: &AppTheme) -> Vec<AnyElement> {
        vec![
            Self::horizontal_inset(Self::banner(
                false,
                "checkmark.circle.fill",
                hex_color(APPROVED_COLOR),
                SecondaryText::new(tr("music_artist_already_approved")).px(13.0).line_limit(3),
                theme,
            ))
            .into_any_element(),
            Self::horizontal_inset(
                ButtonUi::new("become-artist-open-studio")
                    .on_press(move |_, cx| {
                        if let Some(nav_store) = nav_store.as_ref() {
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.pop(cx);
                                nav_store.push(RootRoute::ArtistStudio, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    })
                    .child(MainText::new(tr("music_artist_studio"))),
            )
            .into_any_element(),
        ]
    }

    fn rejected_banner(rejection_reason: Option<String>, theme: &AppTheme) -> Vec<AnyElement> {
        vec![
            Self::horizontal_inset(Self::banner(
                true,
                "xmark.circle.fill",
                theme.semantic.error_100,
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(tr("music_status_rejected")).px(14.0).font_weight(FontWeight::SEMIBOLD))
                    .when_some(rejection_reason.filter(|reason| !reason.is_empty()), |column, reason| {
                        column.child(SecondaryText::new(reason).px(12.0).line_limit(5))
                    }),
                theme,
            ))
            .into_any_element(),
            div()
                .w_full()
                .px(px(24.0))
                .child(SecondaryText::new(tr("music_artist_reapply_hint")).px(12.0).tac(TextAlignT::Center))
                .into_any_element(),
        ]
    }

    fn pending_banner(theme: &AppTheme) -> AnyElement {
        Self::horizontal_inset(Self::banner(
            false,
            "clock.fill",
            hex_color(NOTICE_COLOR),
            SecondaryText::new(tr("music_artist_review_hint")).px(13.0).line_limit(3),
            theme,
        ))
        .into_any_element()
    }

    fn labeled_field(id: &'static str, label: String, value: Binding<String>, placeholder: String) -> Div {
        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SecondaryText::new(label).px(12.0))
            .child(div().w_full().child(InputOutlinedRowUi::new(id, value, placeholder, div())))
    }

    fn string_binding(
        state: &Entity<BecomeArtistSettingsState>,
        get: fn(&BecomeArtistSettingsState) -> String,
        set: fn(&mut BecomeArtistSettingsState, String),
    ) -> Binding<String> {
        Binding::entity(state.clone(), get, move |state, value, cx| {
            set(state, value);
            cx.notify();
        })
    }

    fn genre_chip(wire: &'static str, selected_genre: &str, state: &Entity<BecomeArtistSettingsState>, theme: &AppTheme, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
        let is_selected = selected_genre == wire;
        let target = if is_selected { 1_f32 } else { 0_f32 };
        let motion = window.use_keyed_state(
            SharedString::from(format!("become-artist-genre-motion-{wire}")),
            cx,
            move |_, _| Animated::new(target),
        );
        let now = RiseClock::monotonic();
        let progress = motion.update(cx, |animated, cx| {
            animated.set_in(target, ease_in_out(0.15), now, cx);
            animated.frame_value(now, window)
        });
        let accent = theme.primary._100;
        let state = state.clone();
        plain(SharedString::from(format!("become-artist-genre-{wire}")))
            .on_press(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.selected_genre = if state.selected_genre == wire { String::new() } else { wire.to_owned() };
                    cx.notify();
                });
            })
            .child(
                div()
                    .px(px(14.0))
                    .py(px(8.0))
                    .capsule()
                    .swiftui_font(Font::system_weight(13.0, if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }))
                    .text_color(theme.text.secondary.lerp(&accent, progress))
                    .bg(theme.bg._200.lerp(&accent.opacity(0.15), progress))
                    .stroke_border(transparent_black().lerp(&accent, progress), 1.0)
                    .child(display_genre(wire)),
            )
    }

    fn form_content(
        state: &Entity<BecomeArtistSettingsState>,
        is_pending: bool,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let (stage_name, selected_genre, bio) = {
            let form = state.read(cx);
            (form.stage_name.clone(), form.selected_genre.clone(), form.bio.clone())
        };
        let can_submit =
            MusicInteractionsStore::shared(cx).read(cx).can_submit_artist_request(&stage_name, &selected_genre);
        let mut chips = hstack(8.0).w_full().flex_wrap();
        for genre in MusicGenre::ALL {
            chips = chips.child(Self::genre_chip(genre.wire(), &selected_genre, state, theme, window, cx));
        }
        let bio_state = state.clone();
        let submit_state = state.clone();

        vec![
            Self::horizontal_inset(Self::banner(
                false,
                "info.circle.fill",
                hex_color(NOTICE_COLOR),
                SecondaryText::new(tr("music_artist_info")).px(13.0).line_limit(5),
                theme,
            ))
            .into_any_element(),
            vstack(14.0)
                .w_full()
                .px(px(16.0))
                .child(Self::labeled_field(
                    "become-artist-stage-name",
                    tr("music_artist_stage_name"),
                    Self::string_binding(state, |state| state.stage_name.clone(), |state, value| state.stage_name = value),
                    tr("music_artist_stage_placeholder"),
                ))
                .child(Self::labeled_field(
                    "become-artist-instagram",
                    tr("music_artist_instagram"),
                    Self::string_binding(state, |state| state.instagram.clone(), |state, value| state.instagram = value),
                    INSTAGRAM_PLACEHOLDER.to_owned(),
                ))
                .child(
                    vstack(8.0)
                        .align(HorizontalAlignment::Leading)
                        .w_full()
                        .child(SecondaryText::new(tr("music_artist_genre")).px(12.0))
                        .child(
                            div().w_full().child(chips),
                        ),
                )
                .child(
                    vstack(8.0)
                        .align(HorizontalAlignment::Leading)
                        .w_full()
                        .child(SecondaryText::new(tr("music_artist_bio")).px(12.0))
                        .child(
                            TextAreaUi::new("become-artist-bio")
                                .values(HashMap::from([(BIO_FIELD.to_owned(), bio)]))
                                .name(BIO_FIELD)
                                .min_lines(BIO_MIN_LINES)
                                .set_value(move |_, value, _, cx| {
                                    bio_state.update(cx, |state, cx| {
                                        state.bio = value.to_owned();
                                        cx.notify();
                                    });
                                }),
                        ),
                )
                .into_any_element(),
            Self::horizontal_inset(
                ButtonUi::new("become-artist-submit")
                    .disabled(!can_submit || is_pending)
                    .on_press(move |_, cx| {
                        let (stage_name, genre, bio, instagram) = {
                            let form = submit_state.read(cx);
                            (form.stage_name.clone(), form.selected_genre.clone(), form.bio.clone(), form.instagram.clone())
                        };
                        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.submit_artist_request(
                                &stage_name,
                                &genre,
                                &bio,
                                (!instagram.is_empty()).then_some(instagram.as_str()),
                                None,
                                cx,
                            );
                        });
                    })
                    .child(MainText::new(if is_pending { tr("music_artist_sending") } else { tr("music_artist_submit") })),
            )
            .into_any_element(),
        ]
    }
}

impl RenderOnce for BecomeArtistSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("become-artist-settings-state", cx, |_, cx| BecomeArtistSettingsState::new(cx));
        let actions = MusicActionsStore::shared(cx);
        let (is_pending, status_data, my_request_id) = {
            let store = actions.read(cx);
            (
                store.request_artist_sai.is_pending(),
                store.get_artist_status_sai.data(),
                store.get_my_artist_request_sai.data().and_then(|data| data.request).map(|request| request.id),
            )
        };
        let artist_status = status_data.as_ref().map(|data| data.status.clone());
        let rejection_reason = status_data.and_then(|data| data.rejection_reason);
        let status = artist_status.as_deref();
        let is_approved = status == Some(music_service::ARTIST_STATUS_APPROVED);
        let is_already_pending = status == Some(music_service::ARTIST_STATUS_PENDING);
        let is_rejected = status == Some(music_service::ARTIST_STATUS_REJECTED);

        let change_state = state.clone();
        let success_store = nav_store.clone();
        on_change("become-artist-status-change", artist_status.clone(), false, window, cx, move |_, status, _, cx| {
            if status.as_deref() == Some(music_service::ARTIST_STATUS_PENDING) {
                NativeConfirmAlert::present_notice(
                    &tr("music_artist_success_title"),
                    &tr("music_artist_success_msg"),
                    None,
                    None,
                    move |cx: &mut App| pop_screen(success_store.as_ref(), cx),
                    cx,
                );
            }
            if status.as_deref() == Some(music_service::ARTIST_STATUS_REJECTED) {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_my_artist_request_if_needed(true, cx));
                BecomeArtistSettingsState::prefill_from_last_request_if_needed(&change_state, cx);
            }
        });
        let prefill_state = state.clone();
        on_change("become-artist-request-change", my_request_id, true, window, cx, move |_, _, _, cx| {
            BecomeArtistSettingsState::prefill_from_last_request_if_needed(&prefill_state, cx);
        });

        let rows: Vec<AnyElement> = if is_approved {
            Self::approved_banner(nav_store.clone(), &theme)
        } else if is_already_pending {
            vec![Self::pending_banner(&theme)]
        } else {
            let mut rows = if is_rejected { Self::rejected_banner(rejection_reason, &theme) } else { Vec::new() };
            rows.extend(Self::form_content(&state, is_pending, &theme, window, cx));
            rows
        };
        let appear_is_rejected = is_rejected;

        SettingsWrapperUi::new(tr("music_become_artist"), move |_, cx| pop_screen(nav_store.as_ref(), cx))
            .content(
                div().size_full().bg(theme.bg._100).child(RequestScrollBody::new(
                    SharedString::new_static("become-artist-scroll"),
                    vstack(20.0).w_full().pt(px(12.0)).children(rows).music_mini_player_bottom_padding(80.0, 0.0),
                )),
            )
            .on_appear("become-artist-appear", move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_artist_status_if_needed(cx);
                    if appear_is_rejected {
                        store.load_my_artist_request_if_needed(true, cx);
                    }
                });
            })
    }
}

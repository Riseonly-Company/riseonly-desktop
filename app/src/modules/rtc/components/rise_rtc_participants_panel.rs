use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, IntoElement, ListAlignment, ParentElement, RenderOnce, Styled, Window,
    div, px,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::MainText;
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::stacks::{hstack, spacer};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::modules::rtc::components::rise_rtc_stage_roster::{
    RiseRTCStageListenerRow, RiseRTCStageMetrics, RiseRTCStageRosterActionHandler, RiseRTCStageRosterTapHandler,
    SEPARATOR_HEIGHT,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCParticipant;
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::RiseRTCListenerVolume;
use crate::modules::rtc::pages::rise_rtc_room_screen::{RiseRTCRoomScreenView, non_empty};
use crate::modules::rtc::shared::rise_rtc_room_menu::{RiseRTCRoomMenuAction, RiseRTCRoomMenuCapabilities};
use crate::modules::rtc::shared::rtc_guide_sandbox_hooks::RtcGuideSandboxHooks;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::vacancy::pages::vacancy_list::vacancy_list::sync_rows;

const PANEL_PADDING: f32 = 12.0;
const SECTION_SPACING: f32 = 14.0;
const TITLE_KEY: &str = "title";
const LOAD_MORE_KEY: &str = "load-more";

struct RiseRTCParticipantsPanelState {
    list: ListUi,
    row_keys: Vec<String>,
    is_loading_more: bool,
}

fn load_more(state: &Entity<RiseRTCParticipantsPanelState>, cx: &mut App) {
    if state.read(cx).is_loading_more {
        return;
    }
    state.update(cx, |state, cx| {
        state.is_loading_more = true;
        cx.notify();
    });
    let interactions = RTCInteractionsStore::shared(cx);
    let task = interactions.update(cx, |interactions, cx| interactions.load_more_participants(cx));
    let state = state.clone();
    cx.spawn(async move |cx| {
        let result = task.await;
        let _ = state.update(cx, |state, cx| {
            state.is_loading_more = false;
            cx.notify();
        });
        if let Err(error) = result {
            cx.update(|cx| RiseRTCRoomScreenView::report_action_failure(LOAD_MORE_KEY.to_owned(), error, cx));
        }
    })
    .detach();
}

#[derive(IntoElement)]
pub struct RiseRTCParticipantsPanel {
    id: ElementId,
    navigation_store: Entity<NavigationStore>,
}

impl RiseRTCParticipantsPanel {
    pub fn new(id: impl Into<ElementId>, navigation_store: Entity<NavigationStore>) -> Self {
        Self { id: id.into(), navigation_store }
    }

    fn title(count: i64, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .px(px(PANEL_PADDING))
            .pt(px(PANEL_PADDING))
            .pb(px(SECTION_SPACING))
            .child(
                hstack(None)
                    .w_full()
                    .items_center()
                    .child(MainText::new(tr("rtc_participants")).px(15.0).font_weight(FontWeight::BOLD))
                    .child(spacer())
                    .child(MainText::new(count.to_string()).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary)),
            )
            .into_any_element()
    }

    fn load_more_button(state: &Entity<RiseRTCParticipantsPanelState>, theme: &AppTheme, cx: &App) -> AnyElement {
        let label = if state.read(cx).is_loading_more {
            progress_view().into_any_element()
        } else {
            MainText::new(tr("common_show_more"))
                .px(14.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.primary._100)
                .into_any_element()
        };
        let press_state = state.clone();
        div()
            .w_full()
            .px(px(PANEL_PADDING))
            .py(px(PANEL_PADDING))
            .child(
                plain("rtc-participants-panel-load-more")
                    .on_press(move |_: &mut Window, cx: &mut App| load_more(&press_state, cx))
                    .w_full()
                    .flex()
                    .justify_center()
                    .py(px(SECTION_SPACING))
                    .child(label)
                    .liquid_chrome_rounded_rectangle_with(f32::from(theme.radius._300), 0.12, None, true),
            )
            .into_any_element()
    }
}

impl RenderOnce for RiseRTCParticipantsPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let rtc = RTCServicesStore::shared(cx);
        let (participants, hands, current_user_id, count, has_more) = match rtc.read(cx).active_room(cx) {
            Some(room) => (
                room.participants.clone(),
                room.raised_hands.iter().map(|hand| hand.user_id.clone()).collect::<HashSet<String>>(),
                room.self_participant.as_ref().map_or_else(String::new, |participant| participant.user_id.clone()),
                room.participant_count,
                room.has_more_participants,
            ),
            None => (Vec::new(), HashSet::new(), String::new(), 0, false),
        };

        let list_theme = theme.clone();
        let state = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "state".into()),
            cx,
            move |_, _| RiseRTCParticipantsPanelState {
                list: ListUi::new(&list_theme, 0, ListAlignment::Top),
                row_keys: Vec::new(),
                is_loading_more: false,
            },
        );

        let mut row_keys = Vec::with_capacity(participants.len() + 2);
        row_keys.push(TITLE_KEY.to_owned());
        row_keys.extend(participants.iter().map(|participant| participant.user_id.clone()));
        if has_more {
            row_keys.push(LOAD_MORE_KEY.to_owned());
        }
        state.update(cx, |state, _| {
            if state.row_keys != row_keys {
                sync_rows(&state.list, &state.row_keys, &row_keys);
                state.row_keys = row_keys;
            }
        });

        let tap_user_id = current_user_id.clone();
        let navigation_store = self.navigation_store;
        let capabilities = RiseRTCRoomMenuCapabilities {
            can_manage_stage: false,
            can_mute: false,
            can_deafen: false,
            can_remove: false,
            chat_id: None,
        };
        let on_action: RiseRTCStageRosterActionHandler =
            Rc::new(|_: RiseRTCRoomMenuAction, _: &RiseRTCParticipant, _: &mut Window, _: &mut App| {});
        let on_tap: RiseRTCStageRosterTapHandler = Rc::new(move |participant: &RiseRTCParticipant, _: &mut Window, cx: &mut App| {
            if participant.user_id == tap_user_id || RtcGuideSandboxHooks::owns_user(&participant.user_id, cx) {
                return;
            }
            let route = RootRoute::UserPage {
                user_id: Some(participant.user_id.clone()),
                tag: participant.profile.as_ref().and_then(|profile| non_empty(&profile.tag)),
            };
            navigation_store.update(cx, |navigation_store, cx| {
                navigation_store.push(route, NavigationStoreSettlingPush::default(), cx);
            });
        });

        let item_state = state.clone();
        let item_theme = theme.clone();
        let list = state.read(cx).list.element(move |index, _, cx| {
            if index == 0 {
                return Self::title(count, &item_theme);
            }
            let Some(participant) = participants.get(index - 1) else {
                return Self::load_more_button(&item_state, &item_theme, cx);
            };
            let is_self = participant.user_id == current_user_id;
            let (listener_volume_percent, has_explicit_listener_volume) = if is_self {
                (RiseRTCListenerVolume::DEFAULT_PERCENT, false)
            } else {
                let services = rtc.read(cx);
                (
                    services.listener_volume_percent(&participant.user_id, cx),
                    services.has_explicit_listener_volume(&participant.user_id, cx),
                )
            };
            let row = RiseRTCStageListenerRow {
                participant: participant.clone(),
                display_name: RiseRTCRoomScreenView::participant_name_user_id(
                    &participant.user_id,
                    participant.profile.as_ref(),
                    &current_user_id,
                ),
                hand_raised: hands.contains(&participant.user_id),
                is_self,
                capabilities: capabilities.clone(),
                listener_volume_percent,
                has_explicit_listener_volume,
                guide_anchor: None,
                on_action: on_action.clone(),
                on_tap: on_tap.clone(),
            };
            let is_first = index == 1;
            let is_last = index == participants.len();
            let radius = px(GroupedCardMetrics::CORNER_RADIUS);
            let separator = item_theme.border._200.opacity(0.55);
            div()
                .w_full()
                .px(px(PANEL_PADDING))
                .when(is_last && !has_more, |cell| cell.pb(px(PANEL_PADDING)))
                .child(
                    div()
                        .w_full()
                        .bg(item_theme.bg._200)
                        .when(is_first, |card| card.rounded_t(radius))
                        .when(is_last, |card| card.rounded_b(radius))
                        .child(row)
                        .when(!is_last, |card| {
                            card.child(
                                div()
                                    .ml(px(RiseRTCStageMetrics::content_leading_inset()))
                                    .h(px(SEPARATOR_HEIGHT))
                                    .bg(separator),
                            )
                        }),
                )
                .into_any_element()
        });

        div().size_full().bg(theme.bg._100).child(list)
    }
}

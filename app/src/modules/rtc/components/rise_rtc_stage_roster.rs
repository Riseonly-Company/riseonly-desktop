use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, Point, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAlignment;
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes};
use rise_ui::swiftui::stacks::{hstack, spacer_min_w};
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_theme::AppTheme;
use rise_ui::{IconUi, MainText};

use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuActionsProvider, HoldContextMenuContainer, HoldContextMenuSourceContent,
};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCParticipant;
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::RiseRTCListenerVolume;
use crate::modules::rtc::shared::rise_rtc_room_menu::{
    RiseRTCRoomMenu, RiseRTCRoomMenuAction, RiseRTCRoomMenuCapabilities,
};
use crate::modules::rtc::shared::rise_rtc_volume_menu_accessory::{
    RiseRTCListenerVolumeContent, RiseRTCVolumeMenuAccessory,
};
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;

pub(crate) const SEPARATOR_HEIGHT: f32 = 0.5;
const OFFSCREEN_ROWS: usize = 8;

pub type RiseRTCStageRosterActionHandler = Rc<dyn Fn(RiseRTCRoomMenuAction, &RiseRTCParticipant, &mut Window, &mut App)>;
pub type RiseRTCStageRosterTapHandler = Rc<dyn Fn(&RiseRTCParticipant, &mut Window, &mut App)>;
pub type RiseRTCStageRosterDisplayName = Rc<dyn Fn(&RiseRTCParticipant, &App) -> String>;

/// stage audience geometry, shared so the row
/// the card and anything reserving space agree
pub enum RiseRTCStageMetrics {}

impl RiseRTCStageMetrics {
    pub const AVATAR_SIZE: f32 = 34.0;
    pub const ROW_HEIGHT: f32 = 54.0;
    pub const HORIZONTAL_PADDING: f32 = GroupedCardMetrics::ROW_HORIZONTAL_PADDING;
    pub const ICON_SPACING: f32 = GroupedCardMetrics::ICON_SPACING;

    /// content leading inset is derived from the avatar since the shared
    /// one assumes the settings icon and the roster leads with an avatar
    pub const fn content_leading_inset() -> f32 {
        Self::HORIZONTAL_PADDING + Self::AVATAR_SIZE + Self::ICON_SPACING
    }

    pub fn card_height(rows: usize) -> f32 {
        if rows == 0 {
            return 0.0;
        }
        rows as f32 * Self::ROW_HEIGHT + (rows - 1) as f32 * SEPARATOR_HEIGHT
    }
}

/// one stage audience member as its own component
/// the row keeps its own state and rebuilds nothing around it
/// every render input is a stored value resolved by the list
#[derive(Clone, IntoElement)]
pub struct RiseRTCStageListenerRow {
    pub participant: RiseRTCParticipant,
    pub display_name: String,
    pub hand_raised: bool,
    pub is_self: bool,
    pub capabilities: RiseRTCRoomMenuCapabilities,
    pub listener_volume_percent: f64,
    pub has_explicit_listener_volume: bool,
    pub guide_anchor: Option<RiseGuideAnchor>,
    pub on_action: RiseRTCStageRosterActionHandler,
    pub on_tap: RiseRTCStageRosterTapHandler,
}

impl RiseRTCStageListenerRow {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(ElementId::Name(self.participant.user_id.clone().into())), name.into())
    }

    fn content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let muted = self.participant.is_server_silenced();
        let deafened = self.participant.deafened || self.participant.server_deafened;
        let profile = self.participant.profile.as_ref();
        let hand_chip = self.hand_chip(&theme, window, cx);
        let secondary = theme.text.secondary;

        hstack(RiseRTCStageMetrics::ICON_SPACING)
            .items_center()
            .child(
                UserLogoUi::new(self.child_id("logo"))
                    .source(profile.map(|profile| SharedString::from(profile.avatar_url.clone())))
                    .name(profile.map(|profile| SharedString::from(profile.name.clone())))
                    .user_id(Some(SharedString::from(self.participant.user_id.clone())))
                    .size(RiseRTCStageMetrics::AVATAR_SIZE)
                    .show_story_ring(false)
                    .image_cache_policy(CleverImageCachePolicy::MemoryOnly),
            )
            .child(
                div().flex_1().min_w_0().child(
                    MainText::new(self.display_name.clone()).px(15.0).font_weight(FontWeight::MEDIUM).number_of_lines(1),
                ),
            )
            .child(spacer_min_w(4.0))
            .when(deafened, |row| row.children(IconUi::sized(RiseSymbol::DEAFENED, px(12.0), secondary, false)))
            .when(muted, |row| row.children(IconUi::sized("mic.slash.fill", px(12.0), secondary, true)))
            .when(!self.is_self, |row| {
                let id = self.child_id("muted-for-me");
                row.child(RiseRTCListenerVolumeContent::new(
                    self.participant.user_id.clone(),
                    move |percent, _, _| {
                        if RiseRTCListenerVolume::is_muted(percent) {
                            div()
                                .id(id)
                                .aria_label(tr("rtc_volume_muted_for_me"))
                                .children(IconUi::sized("speaker.slash.fill", px(12.0), secondary, true))
                                .into_any_element()
                        } else if percent != RiseRTCListenerVolume::DEFAULT_PERCENT {
                            MainText::new(RiseRTCListenerVolume::percent_label(percent))
                                .px(12.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(secondary)
                                .number_of_lines(1)
                                .monospaced_digits(true)
                                .into_any_element()
                        } else {
                            gpui::Empty.into_any_element()
                        }
                    },
                ))
            })
            .children(hand_chip)
            .px(px(RiseRTCStageMetrics::HORIZONTAL_PADDING))
            .h(px(RiseRTCStageMetrics::ROW_HEIGHT))
            .w_full()
            .bg(theme.bg._200)
            .into_any_element()
    }

    /// the queue drops a raised hand on its own deadline so the chip
    /// needs a transition or it pops out of the row like a glitch
    fn hand_chip(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let hand_raised = self.hand_raised;
        let state = window.use_keyed_state(self.child_id("hand-chip"), cx, move |_, _| {
            Presence::new(hand_raised.then_some(()))
                .with_transition(Transition::scale(0.6).combined(Transition::opacity()))
        });
        let now = RiseClock::monotonic();
        state.update(cx, |presence, cx| presence.set_in(hand_raised.then_some(()), ease_in_out(0.2), now, cx));
        let id = self.child_id("hand-chip-icon");
        let color = theme.primary._100;
        state
            .read(cx)
            .render_transitioned(now, window, move |_| {
                div()
                    .id(id)
                    .aria_label(tr("rtc_hand_raised"))
                    .children(IconUi::sized("hand.raised.fill", px(14.0), color, true))
            })
            .map(IntoElement::into_any_element)
    }
}

impl RenderOnce for RiseRTCStageListenerRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let anchor = self.guide_anchor;
        let on_tap = self.on_tap.clone();
        let participant = self.participant.clone();
        if self.is_self {
            let content = self.content(window, cx);
            return div()
                .id(self.child_id("self-row"))
                .w_full()
                .child(content.guide_anchor(anchor))
                .on_click(move |_: &ClickEvent, window, cx| on_tap(&participant, window, cx))
                .into_any_element();
        }

        let accessory = RiseRTCVolumeMenuAccessory::configuration(
            &self.participant,
            RiseRTCVolumeMenuAccessory::shows_blocked_hint(
                self.listener_volume_percent,
                self.has_explicit_listener_volume,
            ),
            cx,
        );
        let outranked = RiseRTCRoomMenu::outranked(&self.participant, &self.capabilities, cx);
        let actions: HoldContextMenuActionsProvider = {
            let participant = self.participant.clone();
            let capabilities = self.capabilities.clone();
            Rc::new(move |cx: &App| RiseRTCRoomMenu::actions(&participant, &capabilities, outranked, cx))
        };
        let on_action = self.on_action.clone();
        let action_participant = self.participant.clone();
        let tap_participant = self.participant.clone();
        let container_id = self.child_id("hold-row");
        let source_identity = self.participant.user_id.clone();
        let content: HoldContextMenuSourceContent = {
            let row = self.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| row.content(window, cx).guide_anchor(anchor).into_any_element())
        };

        HoldContextMenuContainer::new(container_id, HoldContextMenuAlignment::Leading, actions, content)
            .show_emoji_list(false)
            .accessory(accessory)
            .live_action_titles(true)
            .source_identity(Some(source_identity))
            .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                let Some(action) = RiseRTCRoomMenuAction::from_raw_value(&id) else {
                    return;
                };
                on_action(action, &action_participant, window, cx);
            }))
            .on_tap(Rc::new(move |_: Point<gpui::Pixels>, window: &mut Window, cx: &mut App| {
                on_tap(&tap_participant, window, cx);
            }))
            .into_any_element()
    }
}

struct RiseRTCStageListenerListState {
    origin_y: f32,
}

fn visible_rows(origin_y: f32, viewport_height: f32, rows: usize) -> Range<usize> {
    let stride = RiseRTCStageMetrics::ROW_HEIGHT + SEPARATOR_HEIGHT;
    let first = (((0.0 - origin_y) / stride).floor().max(0.0) as usize).saturating_sub(OFFSCREEN_ROWS);
    let last = ((viewport_height - origin_y) / stride).ceil().max(0.0) as usize + OFFSCREEN_ROWS;
    first.min(rows)..last.min(rows)
}

/// stage audience as one grouped card
/// speakers keep the video grid and the audience has none, a
/// grid of identical avatar squares is just noise at stage sizes
#[derive(IntoElement)]
pub struct RiseRTCStageListenerList {
    id: ElementId,
    participants: Vec<RiseRTCParticipant>,
    raised_hand_user_ids: std::collections::HashSet<String>,
    current_user_id: String,
    capabilities: RiseRTCRoomMenuCapabilities,
    display_name: RiseRTCStageRosterDisplayName,
    on_action: RiseRTCStageRosterActionHandler,
    on_tap: RiseRTCStageRosterTapHandler,
}

impl RiseRTCStageListenerList {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        participants: Vec<RiseRTCParticipant>,
        raised_hand_user_ids: std::collections::HashSet<String>,
        current_user_id: impl Into<String>,
        capabilities: RiseRTCRoomMenuCapabilities,
        display_name: impl Fn(&RiseRTCParticipant, &App) -> String + 'static,
        on_action: impl Fn(RiseRTCRoomMenuAction, &RiseRTCParticipant, &mut Window, &mut App) + 'static,
        on_tap: impl Fn(&RiseRTCParticipant, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            participants,
            raised_hand_user_ids,
            current_user_id: current_user_id.into(),
            capabilities,
            display_name: Rc::new(display_name),
            on_action: Rc::new(on_action),
            on_tap: Rc::new(on_tap),
        }
    }
}

impl RenderOnce for RiseRTCStageListenerList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let rtc = RTCServicesStore::shared(cx);
        let observed = rtc.clone();
        let _observation = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "observation".into()),
            cx,
            move |_, cx| RiseRTCStageListenerListObservation { _subscription: cx.observe(&observed, |_, _, cx| cx.notify()) },
        );
        let state = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "viewport".into()),
            cx,
            |_, _| RiseRTCStageListenerListState { origin_y: 0.0 },
        );

        let rows = self.participants.len();
        let viewport_height = f32::from(window.viewport_size().height);
        let range = visible_rows(state.read(cx).origin_y, viewport_height, rows);
        let stride = RiseRTCStageMetrics::ROW_HEIGHT + SEPARATOR_HEIGHT;
        let first_user_id = self.participants.first().map(|participant| participant.user_id.clone());

        let mut card = div()
            .id(self.id.clone())
            .relative()
            .w_full()
            .h(px(RiseRTCStageMetrics::card_height(rows)))
            .bg(theme.bg._200)
            .clip_shape(Shape::continuous(GroupedCardMetrics::CORNER_RADIUS));

        for index in range {
            let participant = &self.participants[index];
            let is_self = participant.user_id == self.current_user_id;
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
                display_name: (self.display_name)(participant, cx),
                hand_raised: self.raised_hand_user_ids.contains(&participant.user_id),
                is_self,
                capabilities: self.capabilities.clone(),
                listener_volume_percent,
                has_explicit_listener_volume,
                guide_anchor: (first_user_id.as_ref() == Some(&participant.user_id))
                    .then_some(RiseGuideAnchor::RtcStageFirstListener),
                on_action: self.on_action.clone(),
                on_tap: self.on_tap.clone(),
            };
            let top = index as f32 * stride;
            card = card.child(div().absolute().top(px(top)).left_0().right_0().child(row));
            if index + 1 < rows {
                card = card.child(
                    div()
                        .absolute()
                        .top(px(top + RiseRTCStageMetrics::ROW_HEIGHT))
                        .left(px(RiseRTCStageMetrics::content_leading_inset()))
                        .right_0()
                        .h(px(SEPARATOR_HEIGHT))
                        .bg(theme.border._200.opacity(0.55)),
                );
            }
        }

        let viewport = state;
        card.child(measure(
            ElementId::NamedChild(Arc::new(self.id), "measure".into()),
            move |bounds, window, cx| {
                let origin_y = f32::from(bounds.origin.y);
                let viewport_height = f32::from(window.viewport_size().height);
                viewport.update(cx, |state, cx| {
                    let previous = visible_rows(state.origin_y, viewport_height, rows);
                    state.origin_y = origin_y;
                    if visible_rows(origin_y, viewport_height, rows) != previous {
                        cx.notify();
                    }
                });
            },
        ))
    }
}

struct RiseRTCStageListenerListObservation {
    _subscription: gpui::Subscription,
}

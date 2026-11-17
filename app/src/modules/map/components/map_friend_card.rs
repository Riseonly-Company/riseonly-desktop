use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Entity, FocusHandle, FontWeight, KeyDownEvent, Pixels, Point, SharedString, WeakEntity, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::error_text_ui::system_red;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::bottom_sheet_ui::bottom_sheet_interaction_policy::{
    BottomSheetInteractionStyle, BottomSheetReleaseDecision, BottomSheetReleasePolicy, BottomSheetSnapAnimationCurve,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::bottom_sheet_motion_host::{BottomSheetMotionController, BottomSheetMotionHost};
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text::TruncationMode;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::emoji_picker_ui::emoji_picker_ui::EmojiPickerUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::map::components::map_friend_profile_sheet::MapFriendProfileContent;
use crate::modules::map::components::reactions::map_reaction_bar::MapReactionBar;
use crate::modules::map::engine::rise_map_profiles_repository::MapProfileTab;
use crate::modules::map::recorder::map_self_location_feed::{
    MapSelfLocation, MapSelfLocationFeed, MapSelfLocationFeedObserver,
};
use crate::modules::map::shared::map_friend_status::MapFriendStatus;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_actions::map_types::MapLiveFriend;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

pub type MapFriendCardAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type MapFriendCardReact = Rc<dyn Fn(String, Point<Pixels>, &mut Window, &mut App) -> bool>;
pub type MapFriendCardCustomReact = Rc<dyn Fn(StickerItem, Point<Pixels>, &mut Window, &mut App) -> bool>;
pub type MapFriendCardPickerVisibility = Rc<dyn Fn(bool, &mut Window, &mut App)>;
pub type MapFriendCardGrid = Rc<dyn Fn(MapPhotoGrid, &mut Window, &mut App)>;

const LOCATION_TIMELINE: Duration = Duration::from_secs(30);
const ACTIONS_HEIGHT: f32 = 46.0;
const EMOJI_PICKER_HEIGHT: f32 = 270.0;

#[derive(IntoElement)]
pub struct MapFriendCard {
    friend: MapLiveFriend,
    on_react: MapFriendCardReact,
    on_custom_react: MapFriendCardCustomReact,
    on_picker_visibility: MapFriendCardPickerVisibility,
    on_write: MapFriendCardAction,
    on_route: MapFriendCardAction,
    on_tracks: MapFriendCardAction,
    on_close: MapFriendCardAction,
    on_expand: Option<MapFriendCardAction>,
    is_expanded: bool,
    is_own: bool,
    active_tab: Binding<MapProfileTab>,
    profile_height: f32,
    on_grid: MapFriendCardGrid,
    on_collapse: MapFriendCardAction,
    docked: bool,
}

struct MapFriendCardState {
    picker_open: bool,
    picker_source: Point<Pixels>,
    picker_presence: Presence<()>,
    actions_presence: Presence<()>,
    branch_height: Animated<f32>,
    own_location: Option<MapSelfLocation>,
    own_location_unavailable: bool,
    location_observer: Option<Uuid>,
}

impl MapFriendCardState {
    fn new() -> Self {
        Self {
            picker_open: false,
            picker_source: Point::default(),
            picker_presence: Presence::new(None)
                .with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity())),
            actions_presence: Presence::new(Some(())).with_transition(Transition::opacity()),
            branch_height: Animated::new(ACTIONS_HEIGHT),
            own_location: None,
            own_location_unavailable: false,
            location_observer: None,
        }
    }
}

impl MapFriendCard {
    pub const QUICK_REACTIONS: [&'static str; 6] = [
        "\u{2764}\u{FE0F}",
        "\u{1F525}",
        "\u{1F602}",
        "\u{1F44B}",
        "\u{1F62E}",
        "\u{1F44D}",
    ];

    const CORNER_RADIUS: f32 = 28.0;
    pub const COMPACT_HEIGHT: f32 = 198.0;
    pub const OWN_COMPACT_HEIGHT: f32 = 96.0;
    pub const PICKER_HEIGHT: f32 = Self::COMPACT_HEIGHT + EMOJI_PICKER_HEIGHT - ACTIONS_HEIGHT;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        friend: MapLiveFriend,
        on_react: impl Fn(String, Point<Pixels>, &mut Window, &mut App) -> bool + 'static,
        on_custom_react: impl Fn(StickerItem, Point<Pixels>, &mut Window, &mut App) -> bool + 'static,
        on_picker_visibility: impl Fn(bool, &mut Window, &mut App) + 'static,
        on_write: impl Fn(&mut Window, &mut App) + 'static,
        on_route: impl Fn(&mut Window, &mut App) + 'static,
        on_tracks: impl Fn(&mut Window, &mut App) + 'static,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        active_tab: Binding<MapProfileTab>,
    ) -> Self {
        Self {
            friend,
            on_react: Rc::new(on_react),
            on_custom_react: Rc::new(on_custom_react),
            on_picker_visibility: Rc::new(on_picker_visibility),
            on_write: Rc::new(on_write),
            on_route: Rc::new(on_route),
            on_tracks: Rc::new(on_tracks),
            on_close: Rc::new(on_close),
            on_expand: None,
            is_expanded: false,
            is_own: false,
            active_tab,
            profile_height: 0.0,
            on_grid: Rc::new(|_, _, _| {}),
            on_collapse: Rc::new(|_, _| {}),
            docked: false,
        }
    }

    pub fn on_expand(mut self, on_expand: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_expand = Some(Rc::new(on_expand));
        self
    }

    pub fn is_expanded(mut self, is_expanded: bool) -> Self {
        self.is_expanded = is_expanded;
        self
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn profile_height(mut self, profile_height: f32) -> Self {
        self.profile_height = profile_height;
        self
    }

    pub fn on_grid(mut self, on_grid: impl Fn(MapPhotoGrid, &mut Window, &mut App) + 'static) -> Self {
        self.on_grid = Rc::new(on_grid);
        self
    }

    pub fn on_collapse(mut self, on_collapse: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_collapse = Rc::new(on_collapse);
        self
    }

    /// when docked in the right panel the panel is the container so the card draws no frame
    pub fn docked(mut self, docked: bool) -> Self {
        self.docked = docked;
        self
    }
}

impl RenderOnce for MapFriendCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("map-friend-card-state", cx, |_, _| MapFriendCardState::new());

        let timeline = state.downgrade();
        use_task("map-friend-card-timeline", (), window, cx, async move |cx| {
            loop {
                cx.background_executor().timer(LOCATION_TIMELINE).await;
                let alive = cx.update(|_, cx| timeline.update(cx, |_, cx| cx.notify()).is_ok()).unwrap_or(false);
                if !alive {
                    return;
                }
            }
        });

        let friend_change_state = state.clone();
        let friend_change_visibility = self.on_picker_visibility.clone();
        on_change("map-friend-card-friend", self.friend.id().to_owned(), false, window, cx, move |_, _, window, cx| {
            set_picker_open(&friend_change_state, false, false, cx);
            friend_change_visibility(false, window, cx);
        });
        let picker_shown = state.read(cx).picker_open && !self.is_own;
        on_change("map-friend-card-picker-fact", picker_shown, true, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapReactionPickerOpen, *open, cx);
        });
        let own_change_state = state.clone();
        on_change("map-friend-card-own", self.is_own, false, window, cx, move |_, is_own, _, cx| {
            observe_own_location(&own_change_state, *is_own, cx);
        });

        let mut top = vstack(6.0).w_full();
        if let Some(on_expand) = self.on_expand.clone() {
            let handle_label = tr(if self.is_expanded { "close" } else { "ctx_open_profile" });
            top = top.child(
                div()
                    .id("map-friend-card-handle-label")
                    .w_full()
                    .aria_label(handle_label.clone())
                    .tooltip_label(handle_label)
                    .child(
                        plain("map-friend-card-handle")
                            .w_full()
                            .h(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_press(move |window, cx| on_expand(window, cx))
                            .child(div().w(px(36.0)).h(px(5.0)).rounded_full().bg(theme.text.secondary.opacity(0.4))),
                    ),
            );
        }
        top = top.child(self.header(&state, &theme, RiseClock::now().ms(), cx));
        if !self.is_own {
            top = top.child(self.reactions(&state)).child(self.picker_or_actions(&state, &theme, window, cx));
        }

        let appear_state = state.clone();
        let is_own = self.is_own;
        let disappear_state = state.clone();
        let disappear_visibility = self.on_picker_visibility.clone();

        let docked = self.docked;
        let column = vstack(0.0)
            .w_full()
            .occlude()
            .child(top.px(px(12.0)).pt(px(6.0)).pb(px(8.0)))
            .child(
                div().w_full().h(px(self.profile_height)).flex_none().child(
                    MapFriendProfileContent::new(
                        self.friend.clone(),
                        self.is_expanded,
                        self.is_own,
                        self.active_tab.clone(),
                        {
                            let on_grid = self.on_grid.clone();
                            move |grid, window, cx| on_grid(grid, window, cx)
                        },
                        {
                            let on_collapse = self.on_collapse.clone();
                            move |window, cx| on_collapse(window, cx)
                        },
                    ),
                ),
            );
        let surface =
            if docked { column.into_any_element() } else { column.liquid_chrome_rounded_rectangle(Self::CORNER_RADIUS) };
        surface
            .on_appear("map-friend-card-lifecycle", move |_, cx| {
                observe_own_location(&appear_state, is_own, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    disappear_visibility(false, window, cx);
                }
                GuideFacts::set(RiseGuideSignal::MapReactionPickerOpen, false, cx);
                let observer = disappear_state.update(cx, |state, _| state.location_observer.take());
                if let Some(observer) = observer {
                    MapSelfLocationFeed::shared(cx).update(cx, |feed, _| feed.cancel(observer));
                }
            })
    }
}

/// sets picker open with its conditional content, animated says if the change is animated
fn set_picker_open(state: &Entity<MapFriendCardState>, open: bool, animated: bool, cx: &mut App) {
    let spec = animated.then(|| spring(0.34, 0.88));
    let now = RiseClock::monotonic();
    state.update(cx, |state, cx| {
        state.picker_open = open;
        state.picker_presence.set_in(open.then_some(()), spec, now, cx);
        state.actions_presence.set_in((!open).then_some(()), spec, now, cx);
        state.branch_height.set_in(if open { EMOJI_PICKER_HEIGHT } else { ACTIONS_HEIGHT }, spec, now, cx);
        cx.notify();
    });
}

fn observe_own_location(state: &Entity<MapFriendCardState>, is_own: bool, cx: &mut App) {
    let feed = MapSelfLocationFeed::shared(cx);
    let observer = state.update(cx, |state, _| state.location_observer.take());
    if let Some(observer) = observer {
        feed.update(cx, |feed, _| feed.cancel(observer));
    }
    let own_location = if is_own { feed.read(cx).location().cloned() } else { None };
    let own_location_unavailable = is_own && feed.read(cx).is_location_available() == Some(false);
    state.update(cx, |state, cx| {
        state.own_location = own_location;
        state.own_location_unavailable = own_location_unavailable;
        cx.notify();
    });
    if !is_own {
        return;
    }
    let weak: WeakEntity<MapFriendCardState> = state.downgrade();
    let reader = feed.downgrade();
    let observer = feed.update(cx, |feed, _| {
        feed.observe(MapSelfLocationFeedObserver {
            on_location: Box::new(move |location, cx: &mut App| {
                let weak = weak.clone();
                let reader = reader.clone();
                cx.defer(move |cx| {
                    let unavailable =
                        reader.read_with(cx, |feed, _| feed.is_location_available() == Some(false)).unwrap_or(false);
                    let _ = weak.update(cx, |state, cx| {
                        state.own_location = location;
                        state.own_location_unavailable = unavailable;
                        cx.notify();
                    });
                });
            }),
            on_heading: Box::new(|_, _| {}),
        })
    });
    state.update(cx, |state, _| state.location_observer = Some(observer));
}

impl MapFriendCard {
    fn header(&self, state: &Entity<MapFriendCardState>, theme: &AppTheme, now_ms: i64, cx: &mut App) -> impl IntoElement {
        let card = state.read(cx);
        let position = self.friend.position.as_ref();
        let updated_at = if self.is_own {
            card.own_location.as_ref().map(|location| location.time_ms)
        } else {
            position.map(|position| position.fix.recorded_at)
        };
        let unavailable = if self.is_own {
            card.own_location_unavailable
                || card.own_location.as_ref().is_some_and(|location| location.unavailable_since_ms.is_some())
        } else {
            position.is_some_and(|position| position.unavailable_since_ms().is_some())
        };
        let stale = updated_at.is_some_and(|updated_at| now_ms - updated_at > MapFriendStatus::live_window_ms());
        let update_text = if unavailable {
            tr("map_location_unavailable")
        } else {
            MapFriendStatus::location_update_text(updated_at, now_ms)
        };

        let mut status = hstack(6.0);
        if unavailable || stale {
            status = status.children(IconUi::sized(
                if unavailable { MapFriendStatus::UNAVAILABLE_SYMBOL } else { MapFriendStatus::STALE_SYMBOL },
                px(16.0),
                theme.text.secondary,
                false,
            ));
        } else {
            status = status.child(div().size(px(9.0)).flex_none().rounded_full().bg(if updated_at.is_some() {
                theme.primary._100
            } else {
                theme.text.secondary.opacity(0.5)
            }));
        }
        status = status.child(
            MainText::new(update_text)
                .px(13.0)
                .color(theme.text.secondary)
                .number_of_lines(1)
                .truncation(TruncationMode::Tail),
        );

        let user = &self.friend.user;
        let mut row = hstack(12.0)
            .w_full()
            .child(
                UserLogoUi::new("map-friend-card-logo")
                    .source(SharedString::from(user.logo.clone()))
                    .name(SharedString::from(user.display_name().to_owned()))
                    .user_id(SharedString::from(user.id.clone()))
                    .size(52.0),
            )
            .child(
                vstack(3.0)
                    .items_start()
                    .min_w_0()
                    .child(
                        MainText::new(user.display_name().to_owned())
                            .px(17.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(1),
                    )
                    .child(status),
            )
            .child(spacer_min_w(8.0));
        if let Some(fix) = position.map(|position| &position.fix).filter(|fix| fix.has_battery()) {
            row = row.child(MapBatteryBadge::new(fix.battery_level, fix.is_charging));
        }
        let on_close = self.on_close.clone();
        row.child(
            div().id("map-friend-card-close-label").aria_label(tr("map_card_close")).tooltip_label(tr("map_card_close")).child(
                plain("map-friend-card-close").on_press(move |window, cx| on_close(window, cx)).child(
                    div()
                        .size(px(30.0))
                        .flex_none()
                        .rounded_full()
                        .bg(theme.text.secondary.opacity(0.16))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("xmark", px(12.0), theme.text.secondary, false)),
                ),
            ),
        )
    }

    fn reactions(&self, state: &Entity<MapFriendCardState>) -> impl IntoElement {
        let on_react = self.on_react.clone();
        let picker_state = state.clone();
        let on_picker_visibility = self.on_picker_visibility.clone();
        MapReactionBar::new(
            Self::QUICK_REACTIONS.iter().map(|emoji| (*emoji).to_owned()).collect(),
            move |emoji, source, window, cx| on_react(emoji, source, window, cx),
            move |point, window, cx| {
                let open = !picker_state.read(cx).picker_open;
                picker_state.update(cx, |state, _| state.picker_source = point);
                set_picker_open(&picker_state, open, true, cx);
                on_picker_visibility(open, window, cx);
            },
        )
        .on_burst(|count, _, cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, _| interactions.note_reaction_burst(count));
        })
        .guide_anchor(RiseGuideAnchor::MapReactionBar)
    }

    fn picker_or_actions(
        &self,
        state: &Entity<MapFriendCardState>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let card = state.read(cx);
        let branch_height = card.branch_height.frame_value(now, window);
        let picker = card.picker_presence.render_transitioned(now, window, |_| {
            let pick_state = state.clone();
            let on_react = self.on_react.clone();
            let custom_state = state.clone();
            let on_custom_react = self.on_custom_react.clone();
            div().absolute().top_0().left_0().right_0().h(px(EMOJI_PICKER_HEIGHT)).child(
                EmojiPickerUi::new("map-friend-card-emoji-picker", move |emoji: String, window: &mut Window, cx: &mut App| {
                    let source = pick_state.read(cx).picker_source;
                    let _ = on_react(emoji, source, window, cx);
                })
                .chrome(false)
                .shows_custom_emoji(true)
                .on_pick_custom(move |item: StickerItem, window: &mut Window, cx: &mut App| {
                    let source = custom_state.read(cx).picker_source;
                    let _ = on_custom_react(item, source, window, cx);
                })
                .guide_anchor(RiseGuideAnchor::MapReactionEmojiPanel),
            )
        });
        let actions = card
            .actions_presence
            .render_transitioned(now, window, |_| div().absolute().top_0().left_0().right_0().child(self.actions(theme)));
        div().relative().w_full().flex_none().h(px(branch_height)).children(picker).children(actions)
    }

    fn actions(&self, theme: &AppTheme) -> impl IntoElement {
        let on_write = self.on_write.clone();
        let on_tracks = self.on_tracks.clone();
        let on_route = self.on_route.clone();
        hstack(10.0)
            .w_full()
            .child(
                div().flex_1().min_w_0().child(
                    ButtonUi::new("map-friend-card-write")
                        .height(46.0)
                        .padding_horizontal(12.0)
                        .gap(8.0)
                        .on_press(move |window, cx| on_write(window, cx))
                        .children(IconUi::sized("bubble.left.fill", px(14.0), theme.content_on(theme.primary._100), true))
                        .child(
                            MainText::new(tr("map_card_write"))
                                .px(15.0)
                                .minimum_scale_factor(0.8)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(theme.primary._100))
                                .number_of_lines(1),
                        ),
                ),
            )
            .child(
                div().flex_1().min_w_0().child(
                    ButtonUi::new("map-friend-card-tracks")
                        .background_color(theme.text.secondary.opacity(0.18))
                        .height(46.0)
                        .padding_horizontal(12.0)
                        .gap(8.0)
                        .on_press(move |window, cx| on_tracks(window, cx))
                        .children(IconUi::sized("chart.bar.fill", px(14.0), theme.text.primary, true))
                        .child(
                            MainText::new(tr("chat_settings_statistics"))
                                .px(15.0)
                                .minimum_scale_factor(0.8)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary)
                                .number_of_lines(1),
                        )
                        .guide_anchor(RiseGuideAnchor::MapCardStatistics),
                ),
            )
            .child(
                div().id("map-friend-card-route-label").flex_none().aria_label(tr("map_card_route")).tooltip_label(tr("map_card_route")).child(
                    ButtonUi::new("map-friend-card-route")
                        .disabled(self.friend.position.is_none())
                        .background_color(theme.text.secondary.opacity(0.18))
                        .height(46.0)
                        .fit_content(true)
                        .padding_horizontal(16.0)
                        .on_press(move |window, cx| on_route(window, cx))
                        .children(IconUi::sized(
                            "arrow.triangle.turn.up.right.diamond.fill",
                            px(14.0),
                            theme.text.primary,
                            true,
                        )),
                ),
            )
    }
}

struct MapFriendPanelState {
    motion: BottomSheetMotionController,
    drag_base_height: Option<Pixels>,
    height: Pixels,
    focus_handle: FocusHandle,
}

/// same surface stays mounted at both heights, the motion host owns the motion
#[derive(IntoElement)]
pub struct MapFriendPanel {
    is_expanded: Binding<bool>,
    collapsed_height: f32,
    docked: bool,
    on_close: MapFriendCardAction,
    content: Rc<dyn Fn(f32, &mut Window, &mut App) -> AnyElement>,
}

impl MapFriendPanel {
    pub fn new<E: IntoElement>(
        is_expanded: Binding<bool>,
        collapsed_height: f32,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        content: impl Fn(f32, &mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            is_expanded,
            collapsed_height,
            docked: false,
            on_close: Rc::new(on_close),
            content: Rc::new(move |maximum, window, cx| content(maximum, window, cx).into_any_element()),
        }
    }

    /// when docked in the right panel the card fills it at full height, no detents to drag
    pub fn docked(mut self, docked: bool) -> Self {
        self.docked = docked;
        self
    }
}

impl RenderOnce for MapFriendPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("map-friend-panel-state", cx, |_, cx| MapFriendPanelState {
            motion: BottomSheetMotionController::new(),
            drag_base_height: None,
            height: px(0.0),
            focus_handle: cx.focus_handle(),
        });
        let (motion, height, focus_handle) = {
            let state = state.read(cx);
            (state.motion.clone(), state.height, state.focus_handle.clone())
        };
        let is_expanded = self.is_expanded.get(cx);
        let maximum = f32::from(height).max(1.0);
        let minimum = maximum.min(self.collapsed_height);
        let resting = if is_expanded { maximum } else { minimum };
        let measured = state.clone();
        if self.docked {
            let content = self.content.clone();
            return div()
                .id("map-friend-panel")
                .relative()
                .size_full()
                .child(measure("map-friend-panel-geometry", move |bounds, _, cx| {
                    measured.update(cx, |state, cx| {
                        state.height = bounds.size.height;
                        cx.notify();
                    });
                }))
                .child(div().w_full().h(px(maximum)).child(content(maximum, window, cx)))
                .into_any_element();
        }

        let expand_focus = focus_handle.clone();
        on_change("map-friend-panel-expanded", is_expanded, true, window, cx, move |_, expanded, window, cx| {
            if *expanded {
                window.focus(&expand_focus, cx);
            }
        });

        let pan_began_state = state.clone();
        let pan_began_motion = motion.clone();
        let pan_ended_state = state.clone();
        let pan_ended_motion = motion.clone();
        let pan_ended_expanded = self.is_expanded.clone();
        let on_close = self.on_close.clone();
        let content = self.content.clone();
        let escape_expanded = self.is_expanded.clone();

        div()
            .id("map-friend-panel")
            .relative()
            .size_full()
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && escape_expanded.get(cx) {
                    escape_expanded.set(false, window, cx);
                    cx.stop_propagation();
                }
            })
            .child(measure("map-friend-panel-geometry", move |bounds, _, cx| {
                measured.update(cx, |state, cx| {
                    state.height = bounds.size.height;
                    cx.notify();
                });
            }))
            .child(
                BottomSheetMotionHost::new("map-friend-panel-motion", motion, move |window, cx| {
                    div().w_full().h(px(maximum)).child(content(maximum, window, cx))
                })
                .resting_height(px(resting))
                .maximum_height(px(maximum))
                .snap_animation_duration(BottomSheetMetrics::SNAP_DURATION)
                .snap_animation_curve(BottomSheetSnapAnimationCurve::EaseOut)
                .allows_dismissal_preview(true)
                .surface_corner_radius(px(28.0))
                .rounds_bottom_corners(true)
                .owns_content_pan(true)
                .content_pan_locked(false)
                .on_content_pan_began(move |_, cx| {
                    let base = pan_began_motion.begin_interaction(px(resting), px(minimum), px(maximum), cx);
                    pan_began_state.update(cx, |state, _| state.drag_base_height = Some(base));
                })
                .on_content_pan_ended(move |translation, velocity, window, cx| {
                    pan_ended_motion.finish_interactive_updates(cx);
                    let base = pan_ended_state
                        .update(cx, |state, _| state.drag_base_height.take())
                        .map(f32::from)
                        .unwrap_or(resting);
                    let decision = BottomSheetReleasePolicy::resolve(
                        BottomSheetInteractionStyle::Standard,
                        &[minimum, maximum],
                        if (base - minimum).abs() < (base - maximum).abs() { 0 } else { 1 },
                        base,
                        f32::from(translation),
                        f32::from(velocity),
                    );
                    match decision {
                        BottomSheetReleaseDecision::Dismiss => on_close(window, cx),
                        BottomSheetReleaseDecision::Snap { index } => {
                            pan_ended_expanded.set(index == 1, window, cx);
                            pan_ended_motion.animate_snap(
                                px(if index == 1 { maximum } else { minimum }),
                                BottomSheetMetrics::SNAP_DURATION,
                                BottomSheetSnapAnimationCurve::EaseOut,
                                |_| {},
                                cx,
                            );
                        }
                    }
                }),
            )
            .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct MapBatteryBadge {
    level: i64,
    is_charging: bool,
}

impl MapBatteryBadge {
    pub fn new(level: i64, is_charging: bool) -> Self {
        Self { level, is_charging }
    }

    fn symbol(&self) -> &'static str {
        if self.is_charging {
            return "battery.100.bolt";
        }
        match self.level {
            ..13 => "battery.0",
            13..38 => "battery.25",
            38..63 => "battery.50",
            63..88 => "battery.75",
            _ => "battery.100",
        }
    }
}

impl RenderOnce for MapBatteryBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let symbol_color = if self.level <= 20 && !self.is_charging { system_red(cx) } else { theme.text.secondary };
        hstack(4.0).children(IconUi::sized(self.symbol(), px(13.0), symbol_color, false)).child(
            MainText::new(RiseFormat::percent(self.level as f64 / 100.0))
                .px(12.0)
                .font_weight(FontWeight::MEDIUM)
                .color(theme.text.secondary)
                .number_of_lines(1)
                .monospaced_digits(true),
        )
    }
}

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Bounds, FontWeight, Pixels, SharedString, Window, canvas};
use rise_canon::RiseGuideAnchor;
use rise_i18n::{tr, tr_plural};
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::map_view::LatLng;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::unread_count_badge_ui::UnreadCountBadgeUi;

use crate::core::ui::premium_badge::UserNameWithBadges;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::modules::map::components::map_place_sheet::MapPlaceGlyph;
use crate::modules::map::components::tracks::map_tracks_panel::MapTracksPanel;
use crate::modules::map::engine::rise_map_friends_repository::RiseMapFriendsRepositoryList;
use crate::modules::map::engine::rise_map_reactions_repository::MapReactionInboxEntry;
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::stores::map_actions::map_tracks_types::{MapPlace, MapPlaceKind};
use crate::modules::map::stores::map_actions::map_types::{MapLiveFriend, MapState};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

/// friends of the nearby panel with their distance, nearest first
pub type MapNearbyFriends = Rc<[(MapLiveFriend, Option<f64>)]>;

/// order of the nearby panel, kept by the page between frames
#[derive(Default)]
pub struct MapNearbyOrder {
    key: Option<(u64, u64, usize, Option<LatLng>)>,
    friends: Option<MapNearbyFriends>,
}

impl MapNearbyOrder {
    /// friends nearest first
    /// the map redraws every camera frame
    /// so distances and sort only run when the live snapshot or own position changes
    pub fn sorted(
        &mut self,
        revision: u64,
        friends_revision: u64,
        friends: &[MapLiveFriend],
        origin: Option<LatLng>,
    ) -> MapNearbyFriends {
        let key = (revision, friends_revision, friends.len(), origin);
        if self.key == Some(key)
            && let Some(sorted) = &self.friends
        {
            return sorted.clone();
        }
        let mut sorted: Vec<(MapLiveFriend, Option<f64>)> = friends
            .iter()
            .map(|friend| (friend.clone(), MapNearbyPanel::friend_meters(origin, friend)))
            .collect();
        sorted.sort_by(|a, b| match (a.1, b.1) {
            (Some(a), Some(b)) => a.total_cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        let sorted: MapNearbyFriends = sorted.into();
        self.key = Some(key);
        self.friends = Some(sorted.clone());
        sorted
    }
}

/// desktop panel floating over the map's trailing edge
/// friends nearest first plus saved places
/// a row focuses the friend and their card takes the panel
#[derive(IntoElement)]
pub struct MapNearbyPanel {
    state: MapState,
    friends: MapNearbyFriends,
    places: Vec<MapPlace>,
    reaction_inbox: Vec<MapReactionInboxEntry>,
    origin: Option<LatLng>,
    on_open_friends: Rc<dyn Fn(&mut Window, &mut App)>,
    on_focus_friend: Rc<dyn Fn(String, &mut Window, &mut App)>,
    on_select_place: Rc<dyn Fn(String, &mut Window, &mut App)>,
    has_more_friends: bool,
}

const MORE_FRIENDS_RETRY: Duration = Duration::from_secs(1);

thread_local! {
    static MORE_FRIENDS_ASKED: Cell<Option<(usize, Duration)>> = const { Cell::new(None) };
}

fn claim_more_friends(count: usize) -> bool {
    let now = RiseClock::monotonic();
    MORE_FRIENDS_ASKED.with(|asked| {
        if asked.get().is_some_and(|(asked_count, at)| asked_count == count && now.saturating_sub(at) < MORE_FRIENDS_RETRY) {
            return false;
        }
        asked.set(Some((count, now)));
        true
    })
}

impl MapNearbyPanel {
    const AVATAR_SIDE: f32 = 40.0;
    const OPEN_SIDE: f32 = 32.0;
    const ROW_SPACING: f32 = 12.0;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: MapState,
        friends: MapNearbyFriends,
        places: Vec<MapPlace>,
        reaction_inbox: Vec<MapReactionInboxEntry>,
        origin: Option<LatLng>,
        on_open_friends: impl Fn(&mut Window, &mut App) + 'static,
        on_focus_friend: impl Fn(String, &mut Window, &mut App) + 'static,
        on_select_place: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            state,
            friends,
            places,
            reaction_inbox,
            origin,
            on_open_friends: Rc::new(on_open_friends),
            on_focus_friend: Rc::new(on_focus_friend),
            on_select_place: Rc::new(on_select_place),
            has_more_friends: false,
        }
    }

    /// friends list has more pages the server hasnt sent
    /// the section ends in a placeholder row that asks for the next page when it scrolls into view
    pub fn has_more_friends(mut self, has_more: bool) -> Self {
        self.has_more_friends = has_more;
        self
    }

    fn more_friends_row(count: usize) -> impl IntoElement {
        hstack(Self::ROW_SPACING)
            .id("map-nearby-more-friends")
            .relative()
            .w_full()
            .items_center()
            .child(SkeletonCircle::new(Self::AVATAR_SIDE))
            .child(
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(120.0, 13.0))
                    .child(SkeletonLine::new(70.0, 11.0)),
            )
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
                        if window.content_mask().bounds.intersects(&bounds) && claim_more_friends(count) {
                            window.defer(cx, |_, cx| {
                                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                    interactions.load_more(RiseMapFriendsRepositoryList::Friends, cx)
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
    }

    fn subtitle(&self) -> String {
        if !self.state.settings.sharing_enabled {
            return tr("map_sharing_paused");
        }
        if self.state.friends_count == 0 {
            tr("map_friends_invite_hint")
        } else {
            tr_plural("map_friends_count", self.state.friends_count)
        }
    }

    fn meters_to(origin: Option<LatLng>, latitude: f64, longitude: f64) -> Option<f64> {
        origin.map(|origin| MapTrackGeometry::haversine_meters(origin.latitude, origin.longitude, latitude, longitude))
    }

    fn friend_meters(origin: Option<LatLng>, friend: &MapLiveFriend) -> Option<f64> {
        let fix = &friend.position.as_ref()?.fix;
        Self::meters_to(origin, fix.latitude, fix.longitude)
    }

    fn distance_text(meters: Option<f64>) -> Option<String> {
        meters.map(|meters| MapTracksPanel::distance(meters.round() as i64))
    }

    fn section_title(text: String, color: gpui::Hsla) -> impl IntoElement {
        div().w_full().pt(px(6.0)).child(
            MainText::new(text).px(13.0).font_weight(FontWeight::SEMIBOLD).color(color).number_of_lines(1),
        )
    }
}

impl RenderOnce for MapNearbyPanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let mut reaction_counts: HashMap<&str, i64> = HashMap::new();
        for entry in &self.reaction_inbox {
            let count = reaction_counts.entry(entry.sender_id.as_str()).or_insert(entry.count);
            *count = (*count).max(entry.count);
        }

        let on_open_friends = self.on_open_friends.clone();
        let open_button = plain("map-nearby-open-friends")
            .on_press(move |window, cx| on_open_friends(window, cx))
            .child(
                div()
                    .id("map-nearby-open-friends-face")
                    .aria_label(tr("map_friends_title"))
                    .tooltip_label(tr("map_friends_title"))
                    .relative()
                    .size(px(Self::OPEN_SIDE))
                    .flex_none()
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .bg(theme.primary._100)
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("plus", px(15.0), theme.content_on(theme.primary._100), false)),
                    )
                    .when(self.state.incoming_requests_count > 0, |button| {
                        button.child(
                            div()
                                .absolute()
                                .top(px(-4.0))
                                .right(px(-5.0))
                                .child(UnreadCountBadgeUi::new(self.state.incoming_requests_count)),
                        )
                    }),
            )
            .guide_anchor(RiseGuideAnchor::MapFriendsOpen);

        let header = hstack(10.0)
            .w_full()
            .items_center()
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(
                        MainText::new(tr("map_friends_title"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(1),
                    )
                    .child(MainText::new(self.subtitle()).px(13.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .child(open_button);

        let origin = self.origin;
        let friends = self.friends;

        let mut rows = div()
            .id("map-nearby-rows")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(Self::ROW_SPACING))
            .overflow_y_scroll()
            .child(Self::section_title(tr("map_friends_section"), theme.text.secondary));

        if friends.is_empty() {
            rows = rows.child(MainText::new(tr("map_friends_empty")).px(13.0).color(theme.text.secondary));
        }
        let first_id = friends.first().map(|(friend, _)| friend.id().to_owned());
        for (friend, meters) in friends.iter() {
            let meters = *meters;
            let unread_count = reaction_counts.get(friend.id()).copied().unwrap_or(0);
            let on_focus = self.on_focus_friend.clone();
            let friend_id = friend.id().to_owned();
            let name = friend.user.display_name().to_owned();
            let anchor = (first_id.as_deref() == Some(friend.id())).then_some(RiseGuideAnchor::MapFriendsStripFirst);
            let avatar = div()
                .relative()
                .flex_none()
                .size(px(Self::AVATAR_SIDE))
                .child(
                    div().absolute().inset_0().opacity(if friend.position.is_none() { 0.45 } else { 1.0 }).child(
                        UserLogoUi::new(SharedString::from(format!("map-nearby-logo-{}", friend.id())))
                            .source(SharedString::from(friend.user.logo.clone()))
                            .name(SharedString::from(name.clone()))
                            .user_id(SharedString::from(friend.user.id.clone()))
                            .size(Self::AVATAR_SIDE)
                            .gift_worn_frame(
                                SharedString::from(format!("map-nearby-frame-{}", friend.id())),
                                (!friend.user.is_deleted).then(|| friend.user.id.clone()),
                                Self::AVATAR_SIDE,
                            ),
                    ),
                )
                .child(div().absolute().bottom_0().right_0().child(UnreadCountBadgeUi::new(unread_count)));
            let mut text = vstack(2.0)
                .flex_1()
                .min_w_0()
                .items_start()
                .child(
                    UserNameWithBadges::new(Some(SharedString::from(friend.user.id.clone())), name.clone())
                        .font(15.0, FontWeight::MEDIUM)
                        .official(friend.user.is_official)
                        .premium(
                            friend.user.is_premium,
                            friend.user.premium_badge_emoji.clone(),
                            friend.user.name_color.clone(),
                        ),
                );
            if let Some(distance) = Self::distance_text(meters) {
                text = text.child(MainText::new(distance).px(13.0).color(theme.text.secondary).number_of_lines(1));
            }
            rows = rows.child(
                plain(SharedString::from(format!("map-nearby-friend-{}", friend.id())))
                    .w_full()
                    .on_press(move |window, cx| on_focus(friend_id.clone(), window, cx))
                    .child(
                        hstack(Self::ROW_SPACING)
                            .id(SharedString::from(format!("map-nearby-friend-row-{}", friend.id())))
                            .aria_label(name)
                            .w_full()
                            .items_center()
                            .child(avatar)
                            .child(text),
                    )
                    .guide_anchor(anchor),
            );
        }
        if self.has_more_friends && !friends.is_empty() {
            rows = rows.child(Self::more_friends_row(friends.len()));
        }

        let places: Vec<MapPlace> = self.places.into_iter().filter(|place| place.kind == MapPlaceKind::Custom).collect();
        if !places.is_empty() {
            rows = rows.child(Self::section_title(tr("map_places_section"), theme.text.secondary));
        }
        for place in places {
            let on_select = self.on_select_place.clone();
            let place_id = place.id.clone();
            let title = if place.name.is_empty() { tr("map_place_location") } else { place.name.clone() };
            let glyph = div()
                .flex_none()
                .size(px(Self::AVATAR_SIDE))
                .rounded_full()
                .bg(theme.bg._100)
                .flex()
                .items_center()
                .justify_center();
            let glyph = if place.emoji.is_empty() && place.custom_emoji.is_none() {
                glyph.children(IconUi::sized("mappin", px(18.0), theme.text.primary, false))
            } else {
                glyph.overflow_hidden().child(MapPlaceGlyph::new(place.clone(), 22.0).is_playing(false))
            };
            let mut text = vstack(2.0)
                .flex_1()
                .min_w_0()
                .items_start()
                .child(MainText::new(title.clone()).px(15.0).font_weight(FontWeight::MEDIUM).number_of_lines(1));
            if let Some(distance) = Self::distance_text(Self::meters_to(origin, place.latitude, place.longitude)) {
                text = text.child(MainText::new(distance).px(13.0).color(theme.text.secondary).number_of_lines(1));
            }
            rows = rows.child(
                plain(SharedString::from(format!("map-nearby-place-{}", place.id)))
                    .w_full()
                    .on_press(move |window, cx| on_select(place_id.clone(), window, cx))
                    .child(
                        hstack(Self::ROW_SPACING)
                            .id(SharedString::from(format!("map-nearby-place-row-{}", place.id)))
                            .aria_label(title)
                            .w_full()
                            .items_center()
                            .child(glyph)
                            .child(text),
                    ),
            );
        }

        vstack(12.0).size_full().items_start().child(header).child(rows)
    }
}

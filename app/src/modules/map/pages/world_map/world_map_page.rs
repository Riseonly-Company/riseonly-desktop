use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, Keystroke, KeystrokeEvent, Pixels, Render,
    SharedString, StyleRefinement, Subscription, WeakEntity, Window,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::language_store::LanguageStore;
use rise_i18n::tr;
use rise_media::map_view::LatLng;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::MainText;
use rise_ui::page_header_toolbar_button::{PageHeaderBackButton, PageHeaderIconButton, PageHeaderMetrics};
use rise_ui::page_header_ui::PanelToggleHeaderReserveKey;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};
use rise_widgets::window_controls_band;

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_aside_hooks::navigation_aside_hooks;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::ui::qr_code::rise_phone_handoff_code::RisePhoneHandoffCode;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::components::globe::map_globe_layer::MapGlobeLayer;
use crate::modules::map::components::globe::map_globe_view::MapGlobePerson;
use crate::modules::map::components::map_compass_button::MapCompassButton;
use crate::modules::map::components::map_zoom_buttons::MapZoomButtons;
use crate::modules::map::components::map_editing_controls::{MapEditingControls, map_control_hint};
use crate::modules::map::components::map_editing_session::{MapEditingMode, MapEditingSession};
use crate::modules::map::components::map_friend_card::{MapFriendCard, MapFriendPanel};
use crate::modules::map::components::map_friends_strip::MapFriendsStrip;
use crate::modules::map::components::map_grid_editor::MapGridEditor;
use crate::modules::map::components::map_nearby_panel::{MapNearbyOrder, MapNearbyPanel};
use crate::modules::map::components::map_photo_card::{MapAddPhotoButton, MapLocateButton};
use crate::modules::map::components::map_photo_grid_card::MapPhotoGridCard;
use crate::modules::map::components::map_place_sheet::MapPlaceSheetHost;
use crate::modules::map::components::map_state_panel::MapStatePanel;
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;
use crate::modules::map::components::pins::map_self_pin_layer::MapSelfProfile;
use crate::modules::map::components::rise_map_view::{MapAction, MapCoordinateAction, RiseMapAttribution, RiseMapView};
use crate::modules::map::components::tracks::map_track_layer::{MapTrackHomeExclusion, MapTrackLayerInput};
use crate::modules::map::components::tracks::map_tracks_panel::{
    MapModeSwitch, MapPanelMode, MapTrackLinesButton, MapTracksPanel,
};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::engine::rise_map_engine_models::{RiseMapLiveSnapshot, RiseMapStateSnapshot};
use crate::modules::map::engine::rise_map_photos_repository::RiseMapOpenGridSnapshot;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::engine::rise_map_profiles_repository::MapProfileTab;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTracksSnapshot;
use crate::modules::map::pages::world_map::map_journey_page::journey_pin_palette;
use crate::modules::map::recorder::map_self_location_feed::MapSelfLocationFeed;
use crate::modules::map::shared::map_gallery_scout::MapGalleryDraft;
use crate::modules::map::shared::rise_map_style::RiseMapStyle;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_actions::map_tracks_types::MapPlaceKind;
use crate::modules::map::stores::map_actions::map_types::{MapFriend, MapLiveFriend, MapSettings};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

fn interact<R>(cx: &mut App, action: impl FnOnce(&mut MapInteractionsStore, &mut Context<MapInteractionsStore>) -> R) -> R {
    MapInteractionsStore::shared(cx).update(cx, action)
}

/// friends the side panel and the card know
/// everyone sharing a live position plus map friends with no fix yet that the live session leaves out
fn known_friends(mut friends: Vec<MapLiveFriend>, graph: &[MapFriend]) -> Vec<MapLiveFriend> {
    for friend in graph {
        if !friends.iter().any(|known| known.id() == friend.id()) {
            friends.push(MapLiveFriend { user: friend.user.clone(), position: None });
        }
    }
    friends
}

fn hash_value<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

fn open_settings(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
    open_beside(nav_store, RootRoute::MapSettings, cx);
}

fn open_friends(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
    open_beside(nav_store, RootRoute::MapFriends, cx);
}

/// settings and friends of the map share the right panel, one takes the place of the other
fn open_beside(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    let beside = nav_store.as_ref().is_some_and(|store| store.read(cx).aside_takes_details());
    if beside {
        if let Some(aside) = navigation_aside_hooks() {
            (aside.open_over_default)(route, cx);
        }
        return;
    }
    push(nav_store, route, cx);
}

/// a view in its slot
/// once removed its drawn over the layout at its last place while it leaves
fn panel_slot(element: impl IntoElement, is_leaving: bool, fills: bool) -> Div {
    if is_leaving {
        div().absolute().left_0().right_0().bottom_0().child(element)
    } else if fills {
        div().size_full().child(element)
    } else {
        div().w_full().child(element)
    }
}

/// full screen map with glass over it
/// friends are pins drawn by the map itself
/// everything depending on the account sits in the bottom glass so the map stays usable in every state
pub struct WorldMapPage {
    edit_on_appear: Option<MapEditingMode>,
    shows_back_button: bool,
    editing: Option<Entity<MapEditingSession>>,
    reaction_picker_open: bool,
    zoom_inset_outside_editing: f32,
    controls_frame: Option<(f32, f32)>,
    profile_friend: Option<MapLiveFriend>,
    profile_is_minimized: bool,
    selected_is_shown: bool,
    grid_was_shown: bool,
    panel_height: Animated<f32>,
    panel_height_is_known: bool,
    panel_height_keys: (Option<String>, Option<String>, bool),
    panel_height_pending_spec: Option<(MotionSpec, std::time::Duration)>,
    profile_expanded: bool,
    profile_tab: MapProfileTab,
    grid_editor_presented: bool,
    phone_handoff_open: bool,
    canvas: Entity<MapCanvas>,
    place_sheet_host: Entity<MapPlaceSheetHost>,
    focus_handle: FocusHandle,
    width: f32,
    chrome_presence: Presence<()>,
    editing_presence: Presence<Entity<MapEditingSession>>,
    grid_presence: Presence<MapPhotoGrid>,
    tracks_presence: Presence<()>,
    friend_presence: Presence<MapLiveFriend>,
    strip_presence: Presence<()>,
    state_presence: Presence<()>,
    presences_placed: bool,
    nearby_order: MapNearbyOrder,
    zoom_keys_host: Option<(WeakEntity<NavigationStore>, AnyWindowHandle)>,
    state_snapshot: RiseMapStateSnapshot,
    _editing_observation: Option<Subscription>,
    _observations: Vec<Subscription>,
}

impl WorldMapPage {
    const PANEL_HEIGHT_TRANSACTION: std::time::Duration = std::time::Duration::from_millis(500);

    const CARD_INSET: f32 = MapFriendCard::COMPACT_HEIGHT + 34.0;
    const STRIP_INSET: f32 = 96.0;
    const TRACKS_INSET: f32 = 154.0;
    const FRIEND_TRACKS_INSET: f32 = 196.0;
    const PHOTO_INSET: f32 = 340.0;
    const NAVIGATION_HEIGHT: f32 = 56.0;
    const HOLD_AREA_SIDE: f32 = 200.0;
    const ATTRIBUTION_CARD_GAP: f32 = 2.0;
    const NAVIGATION_SPACING: f32 = 10.0;
    const FLOATING_INSET: f32 = 12.0;
    const FLOATING_RADIUS: f32 = 16.0;
    const MAP_PANEL_GAP: f32 = 6.0;
    const MAP_MIN_WIDTH_BESIDE_PANEL: f32 = 520.0;
    const ON_PHONE_CODE_SIDE: f32 = 148.0;

    fn state_render_key(snapshot: &RiseMapStateSnapshot) -> RiseMapStateSnapshot {
        let mut key = snapshot.clone();
        key.revision = 0;
        if let Some(state) = key.state.as_mut() {
            state.settings.track_merge_enabled = false;
            state.settings.track_merge_radius_m = 0;
            state.settings.track_merge_lanes = 0;
        }
        key
    }

    fn refresh_state_snapshot(&mut self, cx: &mut Context<Self>) {
        let next = MapServicesStore::state(cx);
        let is_drawn_change = Self::state_render_key(&next) != Self::state_render_key(&self.state_snapshot);
        self.state_snapshot = next;
        if is_drawn_change {
            cx.notify();
        }
    }

    pub fn new(
        edit_on_appear: Option<MapEditingMode>,
        shows_back_button: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let presentation = RiseMapPresentation::shared(cx);
        let interactions = MapInteractionsStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let boxes = presentation.read(cx);
        let (state, live, friends, tracks, photos, drafts, reactions) = (
            boxes.state.clone(),
            boxes.live.clone(),
            boxes.friends.clone(),
            boxes.tracks.clone(),
            boxes.photos.clone(),
            boxes.drafts.clone(),
            boxes.reactions.clone(),
        );
        let sandbox = GuideSandbox::shared(cx);
        let mut observations = vec![
            cx.observe(&presentation, |_, _, cx| cx.notify()),
            cx.observe(&interactions, |_, _, cx| cx.notify()),
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&state, |page, _, cx| page.refresh_state_snapshot(cx)),
            cx.observe(&sandbox, |page, _, cx| page.refresh_state_snapshot(cx)),
            cx.observe(&live, |_, _, cx| cx.notify()),
            cx.observe(&friends, |_, _, cx| cx.notify()),
            cx.observe(&tracks, |_, _, cx| cx.notify()),
            cx.observe(&photos, |_, _, cx| cx.notify()),
            cx.observe(&drafts, |_, _, cx| cx.notify()),
            cx.observe(&reactions, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |page, window, cx| {
                if !window.is_window_active() {
                    page.window_resigned_active(cx);
                }
            }),
        ];
        let weak_page = cx.weak_entity();
        let zoom_keys_owner = weak_page.clone();
        observations.push(cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
            let Some(levels) = Self::zoom_key_levels(&event.keystroke) else {
                return;
            };
            let Some(page) = zoom_keys_owner.upgrade() else {
                return;
            };
            if !page.read(cx).is_zoom_keys_target(window, cx) {
                return;
            }
            MapZoomButtons::zoom(levels, window, cx);
            cx.stop_propagation();
        }));
        let canvas = cx.new(|cx| MapCanvas::new(weak_page, cx));
        let place_sheet_host = cx.new(|cx| MapPlaceSheetHost::new(window, cx));
        let swap = || Transition::move_edge(Edge::Bottom).combined(Transition::opacity()).animation(spring(0.34, 0.86));
        let fade = || Transition::opacity().animation(ease_in_out(BottomSheetMetrics::SNAP_DURATION));
        Self {
            edit_on_appear,
            shows_back_button,
            editing: None,
            reaction_picker_open: false,
            zoom_inset_outside_editing: 0.0,
            controls_frame: None,
            profile_friend: None,
            profile_is_minimized: false,
            selected_is_shown: false,
            grid_was_shown: false,
            panel_height: Animated::new(0.0),
            panel_height_is_known: false,
            panel_height_keys: (None, None, false),
            panel_height_pending_spec: None,
            profile_expanded: false,
            profile_tab: MapProfileTab::Statistics,
            grid_editor_presented: false,
            phone_handoff_open: false,
            state_snapshot: MapServicesStore::state(cx),
            canvas,
            place_sheet_host,
            focus_handle: cx.focus_handle(),
            width: 0.0,
            chrome_presence: Presence::new(Some(())).with_transition(fade()),
            editing_presence: Presence::new(None).with_transition(fade()),
            grid_presence: Presence::new(None).with_transition(swap()),
            tracks_presence: Presence::new(None).with_transition(Transition::opacity().animation(ease_in_out(0.2))),
            friend_presence: Presence::new(None).with_transition(swap()),
            strip_presence: Presence::new(None).with_transition(Transition::opacity()),
            state_presence: Presence::new(None).with_transition(Transition::opacity()),
            presences_placed: false,
            nearby_order: MapNearbyOrder::default(),
            zoom_keys_host: None,
            _editing_observation: None,
            _observations: observations,
        }
    }

    fn window_resigned_active(&mut self, cx: &mut Context<Self>) {
        interact(cx, |interactions, cx| interactions.reaction_selection_changed(None, cx));
        if let Some(editing) = &self.editing {
            editing.update(cx, |session, cx| {
                session.set_stroke_cancellation(session.stroke_cancellation.wrapping_add(1));
                cx.notify();
            });
        }
    }

    fn begin_editing(&mut self, mode: MapEditingMode, cx: &mut Context<Self>) {
        self.close_profile(cx);
        self.reaction_picker_open = false;
        interact(cx, |interactions, cx| interactions.open_journey_composer(cx));
        let session = cx.new(|_| MapEditingSession::new(mode));
        self._editing_observation = Some(cx.observe(&session, |_, _, cx| cx.notify()));
        self.editing = Some(session);
        cx.notify();
    }

    fn end_editing(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self._editing_observation = None;
        cx.notify();
    }

    fn friend_card_height(&self, cx: &mut App) -> f32 {
        let interactions = MapInteractionsStore::shared(cx).read(cx);
        if interactions.selected_friend_id == SaiAccountCacheScope::active_user_id() {
            return MapFriendCard::OWN_COMPACT_HEIGHT;
        }
        if self.reaction_picker_open { MapFriendCard::PICKER_HEIGHT } else { MapFriendCard::COMPACT_HEIGHT }
    }

    fn map_action_buttons(&self, cx: &mut App) -> impl IntoElement {
        let progress = MapInteractionsStore::shared(cx).read(cx).photo_upload_progress;
        hstack(10.0)
            .w_full()
            .px(px(16.0))
            .child(spacer_min_w(0.0))
            .child(MapLocateButton::new(|_, cx| interact(cx, |interactions, cx| interactions.focus_self(cx))))
            .child(
                MapAddPhotoButton::new(progress, |_, cx| {
                    interact(cx, |interactions, cx| interactions.present_photo_picker(None, cx));
                })
                .guide_anchor(RiseGuideAnchor::MapPhotoAdd),
            )
    }

    fn nav_store(&self) -> Option<Entity<NavigationStore>> {
        self.zoom_keys_host.as_ref().and_then(|(store, _)| store.upgrade())
    }

    fn profile_beside(&self, cx: &App) -> bool {
        self.nav_store().is_some_and(|store| store.read(cx).aside_takes_details())
    }

    fn switch_panel_mode(&mut self, mode: MapPanelMode, cx: &mut Context<Self>) {
        if !self.profile_beside(cx) {
            if mode == MapPanelMode::Media {
                self.open_media_library(cx);
            } else {
                self.close_profile(cx);
                interact(cx, |interactions, cx| interactions.set_panel_mode(mode, cx));
            }
            return;
        }
        self.close_profile(cx);
        interact(cx, |interactions, cx| {
            interactions.set_panel_mode(mode, cx);
            if mode == MapPanelMode::Tracks {
                interactions.ensure_own_tracks(cx);
            }
        });
        let route = if mode == MapPanelMode::Friends { RootRoute::MapFriends } else { RootRoute::MapProfile };
        open_beside(&self.nav_store(), route, cx);
    }

    fn open_media_library(&mut self, cx: &mut Context<Self>) {
        interact(cx, |interactions, cx| interactions.open_own_profile(cx));
        self.profile_friend = MapInteractionsStore::own_map_profile(cx);
        self.profile_tab = MapProfileTab::Media;
        self.profile_expanded = true;
        interact(cx, |interactions, cx| {
            interactions.panel_mode = MapPanelMode::Media;
            cx.notify();
        });
    }

    fn restore_media_library_on_appear(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        let (panel_mode, nothing_selected) = {
            let interactions = MapInteractionsStore::shared(cx).read(cx);
            (
                interactions.panel_mode,
                interactions.selected_friend_id.is_none() && interactions.selected_grid_id.is_none(),
            )
        };
        if self.profile_beside(cx) {
            if panel_mode != MapPanelMode::Friends && nothing_selected && MapServicesStore::is_member(cx) {
                if panel_mode == MapPanelMode::Tracks {
                    interact(cx, |interactions, cx| interactions.ensure_own_tracks(cx));
                }
                open_beside(&self.nav_store(), RootRoute::MapProfile, cx);
            }
            return;
        }
        if panel_mode != MapPanelMode::Media || !nothing_selected {
            return;
        }
        if MapServicesStore::is_member(cx) && MapInteractionsStore::own_map_profile(cx).is_some() {
            self.open_media_library(cx);
        } else {
            interact(cx, |interactions, cx| interactions.set_panel_mode(MapPanelMode::Friends, cx));
        }
        cx.notify();
    }

    fn close_profile(&mut self, cx: &mut Context<Self>) {
        self.profile_expanded = false;
        self.profile_friend = None;
        self.profile_is_minimized = false;
        interact(cx, |interactions, cx| {
            interactions.select_friend(None, cx);
            interactions.show_own_routes(cx);
        });
        cx.notify();
    }

    fn bottom_navigation(&self, page: &Entity<Self>, cx: &mut App) -> impl IntoElement {
        let draw_page = page.clone();
        let mode_page = page.clone();
        let panel_mode = MapInteractionsStore::shared(cx).read(cx).panel_mode;
        let shows_lines = MapServicesStore::tracks(cx).shows_lines;
        hstack(10.0)
            .w_full()
            .h(px(Self::NAVIGATION_HEIGHT))
            .child(
                map_control_hint(
                    "map-journey-draw-hint",
                    tr("map_journey_draw"),
                    PageHeaderIconButton::new("map-journey-draw", "pencil.tip.crop.circle", move |_, cx| {
                        draw_page.update(cx, |page, cx| page.begin_editing(MapEditingMode::Journey, cx));
                    })
                    .font_size(20.0),
                )
                .guide_anchor(RiseGuideAnchor::MapJourneyDraw),
            )
            .child(div().flex_1().child(MapModeSwitch::new(panel_mode, move |mode, _, cx| {
                mode_page.update(cx, |page, cx| {
                    page.switch_panel_mode(mode, cx);
                    cx.notify();
                });
            })))
            .child(
                MapTrackLinesButton::new(shows_lines, |_, cx| {
                    interact(cx, |interactions, cx| interactions.toggle_track_lines(cx));
                })
                .guide_anchor(RiseGuideAnchor::MapLinesToggle),
            )
    }

    fn selected_grid_panel(
        selected_grid: &MapPhotoGrid,
        open_grid: Option<RiseMapOpenGridSnapshot>,
        drafts: Vec<MapGalleryDraft>,
        page: &Entity<Self>,
        cx: &mut App,
    ) -> MapPhotoGridCard {
        let publishing_drafts = MapServicesStore::drafts(cx).publishing;
        let interactions = MapInteractionsStore::shared(cx).read(cx);
        let uploads = interactions.grid_uploads.iter().filter(|upload| upload.grid_id == selected_grid.id).cloned().collect();
        let is_own = selected_grid.is_draft || interactions.is_own(selected_grid);
        let upload_progress = interactions.photo_upload_progress;
        let (coordinate, delete_id, edit_id) = (selected_grid.coordinate(), selected_grid.id.clone(), selected_grid.id.clone());
        let edit_page = page.clone();
        MapPhotoGridCard {
            grid: selected_grid.clone(),
            open: open_grid,
            drafts,
            publishing_drafts,
            uploads,
            is_own,
            upload_progress,
            on_add: Rc::new(move |_: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.present_photo_picker(Some(coordinate), cx));
            }),
            on_delete_photo: Rc::new(|id: String, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.delete_photo(&id, cx));
            }),
            on_delete_grid: Rc::new(move |cx: &mut App| {
                interact(cx, |interactions, cx| interactions.delete_grid(&delete_id, cx));
            }),
            on_publish_drafts: Rc::new(|drafts: Vec<MapGalleryDraft>, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.publish_drafts(drafts, cx));
            }),
            on_discard_drafts: Rc::new(|ids: Vec<String>, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.discard_drafts(ids, cx));
            }),
            on_discard_all_drafts: Rc::new(|cx: &mut App| {
                interact(cx, |interactions, cx| interactions.discard_all_drafts(cx));
            }),
            on_cancel_upload: Rc::new(|slot_id: String, _: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.cancel_grid_upload(&slot_id, cx));
            }),
            on_load_more: Rc::new(|_: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.load_more_grid_photos(cx));
            }),
            on_retry: Rc::new(|_: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.reopen_grid(cx));
            }),
            on_close: Rc::new(|_: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.select_grid(None, cx));
            }),
            on_edit: Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.set_grid_to_edit_id(Some(edit_id.clone()), cx));
                edit_page.update(cx, |page, cx| {
                    page.grid_editor_presented = true;
                    cx.notify();
                });
            })),
            docked: false,
        }
    }

    fn tracks_panel(
        tracks: RiseMapTracksSnapshot,
        snapshot: &RiseMapStateSnapshot,
        page: &Entity<Self>,
        cx: &mut App,
    ) -> MapTracksPanel {
        let owner = MapInteractionsStore::shared(cx).read(cx).routes_owner.clone();
        let is_recording = snapshot.state.as_ref().is_some_and(|state| state.settings.tracks_enabled);
        let close_page = page.clone();
        MapTracksPanel::new(
            tracks,
            is_recording,
            |period, _, cx| interact(cx, |interactions, cx| interactions.select_track_period(period, cx)),
            |_, cx| interact(cx, |interactions, cx| interactions.retry_load(cx)),
        )
        .owner(owner)
        .on_close_owner(move |_, cx| close_page.update(cx, |page, cx| page.close_profile(cx)))
    }

    fn friend_panel(
        selected: MapLiveFriend,
        is_own: bool,
        friend_card_height: f32,
        is_expanded: bool,
        docked: bool,
        page: &Entity<Self>,
        nav_store: Option<Entity<NavigationStore>>,
    ) -> impl IntoElement {
        let expanded_binding = Binding::entity(
            page.clone(),
            |page: &Self| page.profile_expanded,
            |page: &mut Self, value: bool, _| page.profile_expanded = value,
        );
        let tab_binding = Binding::entity(
            page.clone(),
            |page: &Self| page.profile_tab,
            |page: &mut Self, value: MapProfileTab, _| page.profile_tab = value,
        );
        let close_page = page.clone();
        let content_page = page.clone();
        let panel_id = SharedString::from(format!("map-friend-panel-{}", selected.id()));
        div().id(panel_id).size_full().child(
            MapFriendPanel::new(
                expanded_binding,
                friend_card_height,
                move |_, cx| close_page.update(cx, |page, cx| page.close_profile(cx)),
                move |maximum_height, _, _| {
                    let friend = selected.clone();
                    let page = content_page.clone();
                    let nav_store = nav_store.clone();
                    let (react_id, custom_id) = (friend.id().to_owned(), friend.id().to_owned());
                    let (write_user, route_friend, tracks_friend) = (friend.user.clone(), friend.clone(), friend.clone());
                    let owner = friend.user.clone();
                    let (visibility_page, tracks_page, close_page, expand_page, grid_page, collapse_page) =
                        (page.clone(), page.clone(), page.clone(), page.clone(), page.clone(), page.clone());
                    let card = MapFriendCard::new(
                        friend,
                        move |emoji, source, _, cx| {
                            interact(cx, |interactions, cx| interactions.send_reaction(&react_id, &emoji, source, None, cx))
                        },
                        move |item: StickerItem, source, _, cx| {
                            let Some(emoji) = EmojiPickerCatalog::custom_emoji_ref(&item.id) else {
                                return false;
                            };
                            interact(cx, |interactions, cx| interactions.send_reaction(&custom_id, &emoji, source, Some(item), cx))
                        },
                        move |is_visible, _, cx| {
                            visibility_page.update(cx, |page, cx| {
                                page.reaction_picker_open = is_visible;
                                cx.notify();
                            });
                        },
                        move |_, cx| {
                            let nav_store = nav_store.clone();
                            interact(cx, |interactions, cx| {
                                interactions.open_chat(
                                    &write_user,
                                    move |chat_id, cx| push(&nav_store, RootRoute::Chat { chat_id: Some(chat_id) }, cx),
                                    cx,
                                );
                            });
                        },
                        move |_, cx| interact(cx, |interactions, cx| interactions.open_route(&route_friend, cx)),
                        move |_, cx| {
                            tracks_page.update(cx, |page, cx| {
                                page.profile_friend = Some(tracks_friend.clone());
                                page.profile_tab = MapProfileTab::Statistics;
                                page.profile_expanded = true;
                                cx.notify();
                            });
                        },
                        move |_, cx| close_page.update(cx, |page, cx| page.close_profile(cx)),
                        tab_binding.clone(),
                    )
                    .is_expanded(is_expanded || docked)
                    .docked(docked)
                    .is_own(is_own)
                    .profile_height((maximum_height - friend_card_height).max(1.0))
                    .on_grid(move |grid, _, cx| {
                        let owner = owner.clone();
                        grid_page.update(cx, |page, cx| {
                            page.profile_expanded = false;
                            cx.notify();
                        });
                        interact(cx, |interactions, cx| interactions.focus_profile_grid(grid, owner, false, cx));
                    })
                    .on_collapse(move |_, cx| {
                        collapse_page.update(cx, |page, cx| {
                            page.profile_expanded = false;
                            cx.notify();
                        });
                    });
                    if docked {
                        card
                    } else {
                        card.on_expand(move |_, cx| {
                            expand_page.update(cx, |page, cx| {
                                page.profile_expanded = !page.profile_expanded;
                                cx.notify();
                            });
                        })
                    }
                },
            )
            .docked(docked),
        )
    }

    fn bottom_inset(has_friend_card: bool, has_photo_card: bool, shows_tracks_panel: bool, shows_friend_routes: bool) -> f32 {
        if has_photo_card {
            return Self::PHOTO_INSET;
        }
        if has_friend_card {
            return Self::CARD_INSET;
        }
        if shows_tracks_panel {
            return if shows_friend_routes { Self::FRIEND_TRACKS_INSET } else { Self::TRACKS_INSET };
        }
        Self::STRIP_INSET
    }

    fn selected_friend_id_changed(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let active_user_id = SaiAccountCacheScope::active_user_id();
        let reacting_to = if id == active_user_id { None } else { id.clone() };
        interact(cx, |interactions, cx| interactions.reaction_selection_changed(reacting_to.as_deref(), cx));
        if id.is_some() {
            self.profile_is_minimized = false;
        } else if !self.profile_is_minimized {
            self.profile_friend = None;
            self.profile_expanded = false;
        }
        if id == active_user_id {
            self.profile_friend = MapInteractionsStore::own_map_profile(cx);
        } else if let Some(id) = id
            && let Some(friend) = known_friends(MapServicesStore::live(cx).friends, &MapServicesStore::friends(cx).friends).into_iter().find(|friend| friend.id() == id)
        {
            let routes_differ = MapInteractionsStore::shared(cx).read(cx).routes_owner.as_ref().map(|owner| owner.id.as_str()) != Some(friend.id());
            if routes_differ {
                interact(cx, |interactions, cx| interactions.show_selected_friend_routes(&friend, cx));
            }
            self.profile_friend = Some(friend);
        }
        cx.notify();
    }

    fn profile_expanded_changed(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let Some(friend) = self.profile_friend.clone() else {
            return;
        };
        let interactions = MapInteractionsStore::shared(cx).read(cx);
        if expanded
            && SaiAccountCacheScope::active_user_id().as_deref() != Some(friend.id())
            && interactions.selected_friend_id.as_deref() == Some(friend.id())
            && interactions.routes_owner.as_ref().map(|owner| owner.id.as_str()) != Some(friend.id())
        {
            interact(cx, |interactions, cx| interactions.show_selected_friend_routes(&friend, cx));
        }
    }

    fn watch_changes(&mut self, page: &Entity<Self>, window: &mut Window, cx: &mut Context<Self>) {
        let is_member = self.state_snapshot.is_member();
        let interactions = MapInteractionsStore::shared(cx).read(cx);
        let grid_to_edit_id = interactions.grid_to_edit_id.clone();
        let selected_friend_id = interactions.selected_friend_id.clone();
        let routes_owner_id = interactions.routes_owner.as_ref().map(|owner| owner.id.clone());
        let is_onboarding_presented = interactions.is_onboarding_presented;
        let profile_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        let reaction_revision = RiseMapPresentation::shared(cx).read(cx).reactions.read(cx).revision();
        let (is_erasing, is_outlined) = match &self.editing {
            Some(editing) => {
                let session = editing.read(cx);
                (session.mode == MapEditingMode::Eraser, session.mode == MapEditingMode::Journey && !session.drawing())
            }
            None => (false, false),
        };
        let friend_profile_open = self.profile_expanded
            && self.profile_friend.as_ref().is_some_and(|friend| {
                SaiAccountCacheScope::active_user_id().as_deref() != Some(friend.id())
            });

        let target = page.downgrade();
        on_change("world-map-grid-to-edit", grid_to_edit_id, false, window, cx, move |_, id, _, cx| {
            let presented = id.is_some();
            let _ = target.update(cx, |page, cx| {
                page.grid_editor_presented = presented;
                cx.notify();
            });
        });
        let target = page.downgrade();
        on_change("world-map-profile-id", profile_id, false, window, cx, move |_, _, _, cx| {
            let _ = target.update(cx, |page, cx| {
                page.end_editing(cx);
                page.profile_friend = None;
                page.profile_is_minimized = false;
                page.profile_expanded = false;
                page.profile_tab = MapProfileTab::Statistics;
                cx.notify();
            });
        });
        let target = page.downgrade();
        on_change("world-map-selected-friend", selected_friend_id, false, window, cx, move |_, id, _, cx| {
            let id = id.clone();
            let _ = target.update(cx, |page, cx| page.selected_friend_id_changed(id, cx));
        });
        let target = page.downgrade();
        on_change("world-map-routes-owner", routes_owner_id, false, window, cx, move |_, owner, _, cx| {
            let no_friend = MapInteractionsStore::shared(cx).read(cx).selected_friend_id.is_none();
            if owner.is_none() && no_friend {
                let _ = target.update(cx, |page, cx| {
                    page.profile_friend = None;
                    page.profile_expanded = false;
                    page.profile_is_minimized = false;
                    cx.notify();
                });
            }
        });
        let target = page.downgrade();
        on_change("world-map-profile-expanded", self.profile_expanded, false, window, cx, move |_, expanded, _, cx| {
            let expanded = *expanded;
            let _ = target.update(cx, |page, cx| page.profile_expanded_changed(expanded, cx));
        });
        on_change("world-map-profile-tab", self.profile_tab, false, window, cx, |_, tab, _, cx| {
            if MapInteractionsStore::shared(cx).read(cx).selected_friend_id != SaiAccountCacheScope::active_user_id() {
                return;
            }
            let mode = match tab {
                MapProfileTab::Statistics => MapPanelMode::Tracks,
                MapProfileTab::Media => MapPanelMode::Media,
                MapProfileTab::Friends => MapPanelMode::Friends,
            };
            interact(cx, |interactions, cx| {
                interactions.panel_mode = mode;
                cx.notify();
            });
        });
        on_change("world-map-reaction-inbox", reaction_revision, false, window, cx, |_, _, _, cx| {
            interact(cx, |interactions, cx| interactions.play_selected_reactions_if_needed(cx));
        });
        on_change("world-map-fact-eraser", is_erasing, true, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapEraserOpen, *open, cx);
        });
        on_change("world-map-fact-member", is_member && !is_onboarding_presented, true, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapOpenedAsMember, *open, cx);
        });
        on_change("world-map-fact-outlined", is_outlined, true, window, cx, |_, outlined, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapJourneyOutlined, *outlined, cx);
        });
        on_change("world-map-fact-profile", friend_profile_open, true, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapFriendProfileOpen, *open, cx);
        });
    }

    fn header(
        &self,
        state_snapshot: &RiseMapStateSnapshot,
        page: &Entity<Self>,
        nav_store: &Option<Entity<NavigationStore>>,
        window_controls_band: Pixels,
        cx: &mut App,
    ) -> Div {
        let is_member = state_snapshot.is_member();
        let mut header =
            hstack(None).absolute().top_0().left_0().right_0().items_start().px(px(16.0)).pt(px(8.0) + window_controls_band);
        if self.shows_back_button {
            let back_nav = nav_store.clone();
            header = header.child(map_control_hint(
                "world-map-back-hint",
                tr("back"),
                PageHeaderBackButton::new("world-map-back", move |_, cx| {
                    if let Some(nav_store) = &back_nav {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                }),
            ));
        }
        header = header.child(spacer_min_w(0.0));
        let has_own_position = MapSelfLocationFeed::shared(cx).read(cx).location().is_some();
        if is_member && !has_own_position {
            let color = rise_ui::theme(cx).text.primary;
            let secondary = rise_ui::theme(cx).text.secondary;
            let toggle_page = page.clone();
            let is_open = self.phone_handoff_open;
            let mut pill = vstack(8.0).items_center().child(div().id("world-map-location-from-phone-hit").occlude().max_w_full().child(
                plain("world-map-location-from-phone")
                    .max_w_full()
                    .on_press(move |_, cx| {
                        toggle_page.update(cx, |page, cx| {
                            page.phone_handoff_open = !page.phone_handoff_open;
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .max_w_full()
                            .min_h(px(44.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .child(
                                hstack(6.0)
                                    .min_w_0()
                                    .max_w_full()
                                    .children(IconUi::sized("iphone", px(16.0), color, false))
                                    .child(
                                        vstack(2.0)
                                            .min_w_0()
                                            .flex_shrink(1.0)
                                            .child(
                                                MainText::new(tr("map_location_from_phone"))
                                                    .px(13.0)
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .number_of_lines(1)
                                                    .max_w_full(),
                                            )
                                            .child(
                                                MainText::new(tr("map_open_on_phone"))
                                                    .px(12.0)
                                                    .color(secondary)
                                                    .number_of_lines(1)
                                                    .max_w_full(),
                                            ),
                                    )
                                    .children(IconUi::sized(
                                        if is_open { "chevron.up" } else { "chevron.down" },
                                        px(11.0),
                                        secondary,
                                        false,
                                    )),
                            )
                            .liquid_chrome_rounded_rectangle(22.0),
                    ),
            ));
            if is_open {
                pill = pill.child(
                    div()
                        .id("world-map-location-phone-code")
                        .occlude()
                        .p(px(12.0))
                        .child(RisePhoneHandoffCode::new(
                            "world-map-location-on-phone",
                            AppEnvironment::current().public_site_url().absolute_string().to_owned(),
                            Self::ON_PHONE_CODE_SIDE,
                        ))
                        .liquid_chrome_rounded_rectangle(22.0),
                );
            }
            header = header.child(div().px(px(8.0)).min_w_0().flex_shrink(1.0).child(pill)).child(spacer_min_w(0.0));
        }
        if is_member {
            let settings_nav = nav_store.clone();
            let eraser_page = page.clone();
            let friends_beside = nav_store.as_ref().is_some_and(|store| store.read(cx).aside_takes_details());
            let controls_page = page.clone();
            header = header.child(
                vstack(10.0)
                    .flex_none()
                    .relative()
                    .child(measure("world-map-controls-frame", move |bounds, _, cx| {
                        let frame = (f32::from(bounds.top()), f32::from(bounds.bottom()));
                        let _ = controls_page.update(cx, |page, cx| {
                            if page.controls_frame != Some(frame) {
                                page.controls_frame = Some(frame);
                                cx.notify();
                            }
                        });
                    }))
                    .child(
                        map_control_hint(
                            "world-map-settings-hint",
                            tr("map_settings_title"),
                            PageHeaderIconButton::new("world-map-settings", "gearshape.fill", move |_, cx| {
                                open_settings(&settings_nav, cx);
                            })
                            .font_size(16.0),
                        )
                        .guide_anchor(RiseGuideAnchor::MapSettingsButton),
                    )
                    .children(friends_beside.then(|| {
                        let friends_nav = nav_store.clone();
                        map_control_hint(
                            "world-map-friends-hint",
                            tr("map_friends_title"),
                            PageHeaderIconButton::new("world-map-friends", "person.2.fill", move |_, cx| {
                                open_friends(&friends_nav, cx);
                            })
                            .font_size(15.0),
                        )
                        .guide_anchor(RiseGuideAnchor::MapFriendsOpen)
                    }))
                    .child(MapCompassButton::new())
                    .child(
                        map_control_hint(
                            "world-map-eraser-hint",
                            tr("map_eraser_title"),
                            PageHeaderIconButton::new("world-map-eraser", "eraser", move |_, cx| {
                                eraser_page.update(cx, |page, cx| page.begin_editing(MapEditingMode::Eraser, cx));
                            })
                            .font_size(18.0),
                        )
                        .guide_anchor(RiseGuideAnchor::MapEraserButton),
                    ),
            );
        }
        header
    }

    fn minimized_profile_banner(friend: &MapLiveFriend, color: Hsla, page: &Entity<Self>) -> impl IntoElement {
        let id = friend.id().to_owned();
        let name = friend.user.display_name().to_owned();
        let open_page = page.clone();
        let close_page = page.clone();
        let logo = UserLogoUi::new(SharedString::from(format!("world-map-minimized-logo-{id}")))
            .source(Some(SharedString::from(friend.user.logo.clone())))
            .name(Some(SharedString::from(name.clone())))
            .user_id(Some(SharedString::from(id.clone())))
            .size(32.0);
        div()
            .relative()
            .w_full()
            .child(
                plain("world-map-minimized-open")
                    .on_press(move |_, cx| {
                        let id = id.clone();
                        interact(cx, |interactions, cx| interactions.select_friend(Some(id), cx));
                        open_page.update(cx, |page, cx| {
                            page.profile_expanded = true;
                            cx.notify();
                        });
                    })
                    .w_full()
                    .child(
                        hstack(10.0)
                            .w_full()
                            .min_h(px(36.0))
                            .p(px(10.0))
                            .pr(px(56.0))
                            .child(logo)
                            .child(MainText::new(name).px(15.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                            .children(IconUi::sized("chevron.up", px(12.0), color, false))
                            .child(spacer_min_w(0.0)),
                    ),
            )
            .child(
                div().absolute().top_0().bottom_0().right(px(10.0)).flex().items_center().child(map_control_hint(
                    "world-map-minimized-close-hint",
                    tr("close"),
                    plain("world-map-minimized-close")
                        .on_press(move |_, cx| close_page.update(cx, |page, cx| page.close_profile(cx)))
                        .child(
                            div()
                                .size(px(36.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("xmark", px(17.0), color, false)),
                        ),
                )),
            )
            .liquid_chrome_rounded_rectangle(22.0)
    }

    fn zoom_key_levels(keystroke: &Keystroke) -> Option<f64> {
        let modifiers = &keystroke.modifiers;
        if !modifiers.secondary() || modifiers.alt || modifiers.function {
            return None;
        }
        match keystroke.key.as_str() {
            "+" | "=" | "add" => Some(1.0),
            "-" | "subtract" => Some(-1.0),
            _ => None,
        }
    }

    fn is_zoom_keys_target(&self, window: &Window, cx: &App) -> bool {
        let Some((nav_store, host_window)) = self.zoom_keys_host.clone() else {
            return false;
        };
        if window.window_handle() != host_window {
            return false;
        }
        let Some(nav_store) = nav_store.upgrade() else {
            return false;
        };
        let nav = nav_store.read(cx);
        if !self.shows_back_button {
            return nav.current_tab == MainTab::Maps && nav.path.is_empty();
        }
        matches!(
            nav.path.last(),
            Some(RootRoute::WorldMap | RootRoute::MapJourneyComposer | RootRoute::MapJourney { .. })
        )
    }

    fn handle_key(&mut self, event: &KeyDownEvent, nav_store: &Option<Entity<NavigationStore>>, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" {
            return;
        }
        let interactions = MapInteractionsStore::shared(cx).read(cx);
        let (has_place, has_friend, has_grid) = (
            interactions.place_sheet.is_some(),
            interactions.selected_friend_id.is_some(),
            interactions.selected_grid_id.is_some(),
        );
        if has_place {
            interact(cx, |interactions, cx| interactions.dismiss_place(cx));
        } else if has_friend {
            self.close_profile(cx);
        } else if has_grid {
            interact(cx, |interactions, cx| interactions.select_grid(None, cx));
        } else if self.shows_back_button {
            if let Some(nav_store) = nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        } else {
            return;
        }
        cx.stop_propagation();
    }
}

impl Render for WorldMapPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = cx.entity();
        let nav_store = env::<NavigationStoreKey>(cx);
        self.zoom_keys_host = nav_store.as_ref().map(|store| (store.downgrade(), window.window_handle()));
        let tab_underlap = main_tab_bar_underlap(cx);
        let theme = rise_ui::theme(cx).clone();
        self.watch_changes(&page, window, cx);

        let state_snapshot = self.state_snapshot.clone();
        let live = MapServicesStore::live(cx);
        let friends_graph = MapServicesStore::friends(cx);
        let friends_revision = friends_graph.revision;
        let panel_friends = known_friends(live.friends.clone(), &friends_graph.friends);
        let is_member = state_snapshot.is_member();
        let active_user_id = SaiAccountCacheScope::active_user_id();
        let (selected_friend_id, selected_grid_id, panel_mode, routes_owner_id, grid_to_edit_id) = {
            let interactions = MapInteractionsStore::shared(cx).read(cx);
            (
                interactions.selected_friend_id.clone(),
                interactions.selected_grid_id.clone(),
                interactions.panel_mode,
                interactions.routes_owner.as_ref().map(|owner| owner.id.clone()),
                interactions.grid_to_edit_id.clone(),
            )
        };
        let selected = if !is_member {
            None
        } else if selected_friend_id == active_user_id {
            MapInteractionsStore::own_map_profile(cx)
        } else {
            panel_friends.iter().find(|friend| Some(friend.id()) == selected_friend_id.as_deref()).cloned()
        };
        let tracks = MapServicesStore::tracks(cx);
        let in_tracks_mode = is_member && panel_mode == MapPanelMode::Tracks;
        let photos = MapServicesStore::photos(cx);
        let draft_layout = if is_member { MapInteractionsStore::draft_layout(cx) } else { Default::default() };
        let selected_grid = if is_member {
            photos
                .grids
                .iter()
                .chain(draft_layout.pins.iter())
                .find(|grid| Some(&grid.id) == selected_grid_id.as_ref())
                .cloned()
        } else {
            None
        };
        let friends_beside = nav_store.as_ref().is_some_and(|store| store.read(cx).aside_takes_details());
        let shows_tracks_panel = in_tracks_mode
            && !friends_beside
            && selected.is_none()
            && selected_grid.is_none()
            && self.profile_friend.is_none()
            && routes_owner_id.is_none();
        let is_own_profile = selected.as_ref().is_some_and(|friend| Some(friend.id()) == active_user_id.as_deref());
        let friend_card_height = self.friend_card_height(cx);

        let aside_width = f32::from(theme.panel.default_width);
        let toggle_reserve = f32::from(rise_ui::swiftui::environment::env::<PanelToggleHeaderReserveKey>(cx));
        let map_min_width = f32::from(theme.shell.content_min_width).max(Self::MAP_MIN_WIDTH_BESIDE_PANEL);
        let friend_in_aside = self.width >= map_min_width + aside_width + Self::FLOATING_INSET * 2.0 + toggle_reserve;
        let inline_friend = selected.is_some() && !friend_in_aside;
        let aside_friend = selected.is_some() && friend_in_aside;
        let strip_wanted = selected_grid.is_none()
            && !shows_tracks_panel
            && selected.is_none()
            && self.profile_friend.is_none()
            && state_snapshot.is_member()
            && state_snapshot.state.is_some();
        let shows_nearby = !friends_beside && friend_in_aside && strip_wanted;
        let grid_in_aside = friend_in_aside && selected_grid.is_some();
        let inline_grid = selected_grid.is_some() && !grid_in_aside;
        let grid_closed = std::mem::replace(&mut self.grid_was_shown, selected_grid.is_some()) && selected_grid.is_none();
        if grid_closed
            && friend_in_aside
            && !friends_beside
            && is_member
            && panel_mode == MapPanelMode::Media
            && selected.is_none()
            && self.editing.is_none()
        {
            cx.defer_in(window, |page, _, cx| {
                let interactions = MapInteractionsStore::shared(cx).read(cx);
                let still_idle = interactions.panel_mode == MapPanelMode::Media && interactions.selected_friend_id.is_none();
                if still_idle {
                    page.open_media_library(cx);
                    cx.notify();
                }
            });
        }
        let floats_panel =
            self.editing.is_none() && ((aside_friend && selected_grid.is_none()) || grid_in_aside || shows_nearby);
        let panel_trailing = if floats_panel { aside_width + Self::FLOATING_INSET * 2.0 + toggle_reserve } else { 0.0 };
        let map_trailing =
            if floats_panel { aside_width + Self::FLOATING_INSET + toggle_reserve + Self::MAP_PANEL_GAP } else { 0.0 };

        let panels_inset = if let Some(editing) = &self.editing {
            let session = editing.read(cx);
            if !session.drawing() && session.mode == MapEditingMode::Journey { MapEditingSession::OUTLINED_JOURNEY_INSET } else { 150.0 }
        } else {
            let measured_tracks = (shows_tracks_panel && !inline_friend && self.panel_height_is_known)
                .then(|| *self.panel_height.target() + 8.0);
            let bottom = measured_tracks.unwrap_or_else(|| {
                Self::bottom_inset(inline_friend, inline_grid, shows_tracks_panel, routes_owner_id.is_some())
            });
            let bottom = if (aside_friend || shows_nearby) && selected_grid.is_none() && !shows_tracks_panel {
                bottom - Self::STRIP_INSET
            } else {
                bottom
            };
            bottom
                + if is_member { Self::NAVIGATION_HEIGHT + Self::NAVIGATION_SPACING } else { 0.0 }
                - if is_own_profile && inline_friend {
                    MapFriendCard::COMPACT_HEIGHT - MapFriendCard::OWN_COMPACT_HEIGHT
                } else {
                    0.0
                }
                + if is_member && (inline_friend || inline_grid) { 58.0 } else { 0.0 }
                + if inline_friend && !is_own_profile && self.reaction_picker_open { 224.0 } else { 0.0 }
                + if self.profile_is_minimized && self.profile_friend.is_some() && selected.is_none() { 64.0 } else { 0.0 }
        };
        let map_bottom_inset = panels_inset + tab_underlap;
        let journey_band = self.editing.as_ref().and_then(|editing| {
            let session = editing.read(cx);
            (!session.drawing() && session.mode == MapEditingMode::Journey).then_some(session.journey_band).flatten()
        });
        let chrome_bottom_inset = journey_band
            .map_or(map_bottom_inset, |band| band + tab_underlap + Self::ATTRIBUTION_CARD_GAP);
        if self.editing.is_none() {
            self.zoom_inset_outside_editing = map_bottom_inset;
        }
        let zoom_bottom_inset = self.zoom_inset_outside_editing;
        let controls_frame = if is_member { self.controls_frame } else { None };

        let selected_friend_shown_id = selected.as_ref().map(|friend| friend.id().to_owned());
        let profile_friend_id = self.profile_friend.as_ref().map(|friend| friend.id().to_owned());
        let selected_grid_shown_id = selected_grid.as_ref().map(|grid| grid.id.clone());
        let editing = self.editing.clone();
        let home_track_exclusion_radius_m = state_snapshot
            .state
            .as_ref()
            .map_or_else(MapSettings::default_home_track_radius, |state| state.settings.home_track_exclusion_radius_m);
        self.canvas.update(cx, |canvas, cx| {
            let selected_friend_id = selected_friend_shown_id.or(profile_friend_id);
            let changed = canvas.editing != editing
                || canvas.is_member != is_member
                || canvas.home_track_exclusion_radius_m != home_track_exclusion_radius_m
                || canvas.selected_friend_id != selected_friend_id
                || canvas.selected_grid_id != selected_grid_shown_id
                || canvas.shows_photos != is_member
                || canvas.bottom_inset != map_bottom_inset
                || canvas.chrome_bottom_inset != chrome_bottom_inset
                || canvas.zoom_bottom_inset != zoom_bottom_inset
                || canvas.controls_frame != controls_frame;
            if !changed {
                return;
            }
            canvas.editing = editing;
            canvas.is_member = is_member;
            canvas.home_track_exclusion_radius_m = home_track_exclusion_radius_m;
            canvas.selected_friend_id = selected_friend_id;
            canvas.selected_grid_id = selected_grid_shown_id;
            canvas.shows_photos = is_member;
            canvas.bottom_inset = map_bottom_inset;
            canvas.chrome_bottom_inset = chrome_bottom_inset;
            canvas.zoom_bottom_inset = zoom_bottom_inset;
            canvas.controls_frame = controls_frame;
            cx.notify();
        });

        let hold_anchor = {
            let open = self.editing.as_ref().is_none_or(|editing| {
                let session = editing.read(cx);
                session.mode == MapEditingMode::Eraser || session.drawing()
            });
            (is_member && open).then_some(RiseGuideAnchor::MapHoldArea)
        };
        let hold_area = pass_through()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .child(spacer_min_h(0.0))
            .child(div().size(px(Self::HOLD_AREA_SIDE)).guide_anchor(hold_anchor))
            .child(spacer_min_h(0.0))
            .child(div().h(px(map_bottom_inset)));

        let now = RiseClock::monotonic();
        let reduces_motion = reduce_motion(cx) || !self.presences_placed;
        let spec = ease_in_out(BottomSheetMetrics::SNAP_DURATION);
        self.chrome_presence.set(self.editing.is_none().then_some(()), spec, now, reduces_motion);
        self.editing_presence.set(self.editing.clone(), spec, now, reduces_motion);

        let drafts = selected_grid
            .as_ref()
            .and_then(|grid| {
                let drafts = if grid.is_draft { &draft_layout.loose } else { &draft_layout.by_grid };
                drafts.get(&grid.id).cloned()
            })
            .unwrap_or_default();
        let shows_grid = selected_grid.is_some();
        let shows_friend = !shows_grid && !shows_tracks_panel && selected.is_some();
        let shows_strip = strip_wanted && !shows_nearby && !friends_beside;
        let shows_state = !shows_grid
            && !shows_tracks_panel
            && selected.is_none()
            && self.profile_friend.is_none()
            && !strip_wanted;
        let panel_spec = spring(0.34, 0.86);
        self.selected_is_shown = selected.is_some();
        let panel_height_keys =
            (selected.as_ref().map(|friend| friend.id().to_owned()), selected_grid_id.clone(), shows_tracks_panel);
        if panel_height_keys != self.panel_height_keys {
            let spec = if panel_height_keys.2 != self.panel_height_keys.2 { ease_in_out(0.2) } else { panel_spec };
            self.panel_height_pending_spec = Some((spec, now));
            self.panel_height_keys = panel_height_keys;
        }
        self.grid_presence.set(selected_grid.clone(), panel_spec, now, reduces_motion);
        self.tracks_presence.set(shows_tracks_panel.then_some(()), panel_spec, now, reduces_motion);
        self.friend_presence.set_keyed(
            selected.clone().filter(|_| shows_friend),
            |friend| friend.id().to_owned(),
            panel_spec,
            now,
            reduces_motion,
        );
        self.strip_presence.set(shows_strip.then_some(()), panel_spec, now, reduces_motion);
        self.state_presence.set(shows_state.then_some(()), panel_spec, now, reduces_motion);
        self.presences_placed = true;

        let chrome = if self.chrome_presence.value(now).is_some() {
            let controls_band = window_controls_band(window, cx);
            let header = self.header(&state_snapshot, &page, &nav_store, controls_band, cx);
            let header = if floats_panel { header } else { header.pr(px(16.0 + toggle_reserve)) };
            let panel = self.bottom_panel(
                &state_snapshot,
                &live,
                photos.open_grid.clone(),
                drafts.clone(),
                tracks.clone(),
                friend_in_aside,
                friend_card_height,
                &page,
                &nav_store,
                now,
                window,
                cx,
            );
            let mut column = div().absolute().inset_0().flex().flex_col().pb(px(tab_underlap));
            if !inline_friend {
                column = column.child(spacer_min_h(0.0));
                if is_member {
                    column = column.child(
                        div()
                            .w_full()
                            .pb(px(10.0))
                            .flex()
                            .justify_center()
                            .child(div().w_full().max_w(theme.shell.feed_max_width).child(self.map_action_buttons(cx))),
                    );
                }
            }
            if let (Some(friend), true, None) = (&self.profile_friend, self.profile_is_minimized, &selected) {
                column = column.child(
                    div().w_full().px(px(12.0)).pb(px(8.0)).child(Self::minimized_profile_banner(friend, theme.text.primary, &page)),
                );
            }
            let overlay_buttons = (inline_friend && !self.profile_expanded).then(|| {
                pass_through()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .pb(px(friend_card_height + 10.0))
                    .child(self.map_action_buttons(cx))
            });
            let mut panel_row = div()
                .w_full()
                .px(px(12.0))
                .pb(px(if is_member { Self::NAVIGATION_SPACING } else { 8.0 }));
            if inline_friend {
                panel_row = panel_row.flex_1().min_h_0();
            }
            let panel_body = if inline_friend {
                div().relative().size_full().child(panel).children(overlay_buttons)
            } else {
                let measure_page = page.clone();
                let panel_probe = measure("world-map-panel-height", move |bounds, _, cx| {
                    measure_page.update(cx, |page, cx| page.panel_height_measured(f32::from(bounds.size.height), cx));
                });
                if self.panel_height_is_known {
                    div()
                        .relative()
                        .w_full()
                        .h(px(self.panel_height.frame_value(now, window)))
                        .child(div().absolute().left_0().right_0().bottom_0().child(panel).child(panel_probe))
                } else {
                    div().relative().w_full().child(panel).child(panel_probe)
                }
            };
            let panel_column = div().w_full().max_w(theme.shell.feed_max_width);
            let panel_column = if inline_friend { panel_column.h_full() } else { panel_column };
            column = column.child(panel_row.flex().justify_center().child(panel_column.child(panel_body)));
            if is_member {
                column = column.child(
                    div()
                        .w_full()
                        .px(px(16.0))
                        .pb(px(8.0))
                        .flex()
                        .justify_center()
                        .child(div().w_full().max_w(theme.shell.feed_max_width).child(self.bottom_navigation(&page, cx))),
                );
            }
            let content = div().absolute().top_0().bottom_0().left_0().right(px((panel_trailing - map_trailing).max(0.0))).child(header).child(column);
            self.chrome_presence.render_transitioned(now, window, |_| content)
        } else {
            None
        };

        let close_page = page.clone();
        let editing_controls = self.editing_presence.render_transitioned(now, window, |session| {
            div().absolute().top_0().left_0().right_0().bottom(px(tab_underlap)).child(MapEditingControls::new(session.clone(), move |window, cx| {
                close_page.update(cx, |page, cx| {
                    page.end_editing(cx);
                    window.focus(&page.focus_handle, cx);
                });
            }))
        });

        let nearby = shows_nearby.then(|| {
            let open_nav = nav_store.clone();
            let origin = MapSelfLocationFeed::shared(cx).read(cx).location().map(|location| location.coordinate());
            let friends = self.nearby_order.sorted(live.revision, friends_revision, &panel_friends, origin);
            state_snapshot.state.clone().map(|state| {
                MapNearbyPanel::new(
                    state,
                    friends,
                    tracks.places.clone(),
                    MapServicesStore::reaction_inbox(cx),
                    origin,
                    move |_, cx| open_friends(&open_nav, cx),
                    |id, _, cx| interact(cx, |interactions, cx| interactions.focus_friend(&id, cx)),
                    |id, _, cx| interact(cx, |interactions, cx| interactions.select_place(&id, cx)),
                )
                .has_more_friends(friends_graph.has_more_friends)
            })
        });
        let aside = if floats_panel && self.chrome_presence.is_present() {
            let is_expanded = self.profile_expanded;
            let aside_page = page.clone();
            let aside_nav = nav_store.clone();
            let is_leaving = self.friend_presence.is_leaving(now);
            let departing = self.friend_presence.render_departing_transitioned(now, window, |friend| {
                let is_own = Some(friend.id()) == active_user_id.as_deref();
                div().size_full().child(Self::friend_panel(
                    friend.clone(),
                    is_own,
                    friend_card_height,
                    is_expanded,
                    true,
                    &aside_page,
                    aside_nav.clone(),
                ))
            });
            let friend = self.friend_presence.render_transitioned(now, window, |friend| {
                let is_own = Some(friend.id()) == active_user_id.as_deref();
                panel_slot(
                    Self::friend_panel(friend.clone(), is_own, friend_card_height, is_expanded, true, &aside_page, aside_nav.clone()),
                    is_leaving,
                    true,
                )
            });
            let grid = if grid_in_aside {
                self.grid_presence.value(now).cloned().and_then(|grid| {
                    let mut card = Self::selected_grid_panel(&grid, photos.open_grid.clone(), drafts.clone(), &aside_page, cx);
                    card.docked = true;
                    let card = card.guide_anchor(RiseGuideAnchor::MapGridCard);
                    self.grid_presence.render_transitioned(now, window, |_| div().size_full().child(card))
                })
            } else {
                None
            };
            let body = match (grid, friend, nearby.flatten()) {
                (Some(grid), _, _) => Some(div().size_full().child(grid).into_any_element()),
                (None, Some(friend), _) => Some(friend.into_any_element()),
                (None, None, Some(nearby)) => Some(div().size_full().child(nearby).into_any_element()),
                (None, None, None) => None,
            };
            let top = window_controls_band(window, cx) + px(Self::FLOATING_INSET);
            body.map(|panel| {
                div()
                    .id("world-map-floating-panel")
                    .absolute()
                    .top(top)
                    .right(px(Self::FLOATING_INSET + toggle_reserve))
                    .bottom(px(Self::FLOATING_INSET + tab_underlap))
                    .w(theme.panel.default_width)
                    .p(px(12.0))
                    .rounded(px(Self::FLOATING_RADIUS))
                    .overflow_hidden()
                    .bg(theme.bg._000)
                    .border_1()
                    .border_color(theme.border._100)
                    .shadow_lg()
                    .occlude()
                    .child(panel)
                    .children(departing.into_iter().map(|departed| div().absolute().inset_0().p(px(12.0)).child(departed)))
            })
        } else {
            None
        };

        let grid_editor = grid_to_edit_id.as_ref().and_then(|id| photos.grids.iter().find(|grid| &grid.id == id)).map(|grid| {
            let dismiss_page = page.clone();
            let is_presented = Binding::entity(
                page.clone(),
                |page: &Self| page.grid_editor_presented,
                |page: &mut Self, value: bool, _| page.grid_editor_presented = value,
            );
            div().id(SharedString::from(format!("world-map-grid-editor-{}", grid.id))).absolute().inset_0().child(MapGridEditor::new(
                grid.clone(),
                is_presented,
                move |_, cx| {
                    if !dismiss_page.read(cx).grid_editor_presented {
                        interact(cx, |interactions, cx| interactions.set_grid_to_edit_id(None, cx));
                    }
                },
            ))
        });

        let edit_on_appear = self.edit_on_appear;
        let page_focus = PageFocus::keyed_with("world-map-page-focus", self.focus_handle.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let appear_page = page.clone();
        let measure_page = page.clone();
        let key_page = page.clone();
        let key_nav = nav_store.clone();
        div()
            .id("world-map-page")
            .relative()
            .size_full()
            .flex()
            .track_focus(&self.focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                key_page.update(cx, |page, cx| page.handle_key(event, &key_nav, cx));
            })
            .child(measure("world-map-page-geometry", move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                measure_page.update(cx, |page, cx| {
                    if page.width != width {
                        page.width = width;
                        cx.notify();
                    }
                });
            }))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .mr(px(map_trailing))
                    .child(
                        div().absolute().inset_0().child(
                            self.canvas
                                .clone()
                                .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
                        ),
                    )
                    .child(hold_area)
                    .children(chrome)
                    .children(editing_controls),
            )
            .children(aside)
            .child(
                div().absolute().inset_0().child(
                    self.place_sheet_host
                        .clone()
                        .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
                ),
            )
            .children(grid_editor)
            .on_appear("world-map-page-lifecycle", move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
                appear_page.update(cx, |page, cx| {
                    interact(cx, |interactions, cx| interactions.map_appeared(cx));
                    page.restore_media_library_on_appear(cx);
                    if page.editing.is_none()
                        && let Some(edit_on_appear) = edit_on_appear
                    {
                        page.begin_editing(edit_on_appear, cx);
                    }
                });
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
                interact(cx, |interactions, cx| {
                    interactions.dismiss_place(cx);
                    interactions.map_disappeared(cx);
                });
                GuideFacts::set(RiseGuideSignal::MapEraserOpen, false, cx);
                GuideFacts::set(RiseGuideSignal::MapFriendProfileOpen, false, cx);
                GuideFacts::set(RiseGuideSignal::MapJourneyOutlined, false, cx);
                GuideFacts::set(RiseGuideSignal::MapOpenedAsMember, false, cx);
            })
    }
}

impl WorldMapPage {
    #[allow(clippy::too_many_arguments)]
    fn panel_height_measured(&mut self, height: f32, cx: &mut Context<Self>) {
        if !self.panel_height_is_known {
            self.panel_height_is_known = true;
            self.panel_height.jump(height);
            cx.notify();
            return;
        }
        let now = RiseClock::monotonic();
        let spec = self
            .panel_height_pending_spec
            .take()
            .filter(|(_, set_at)| now.saturating_sub(*set_at) <= Self::PANEL_HEIGHT_TRANSACTION)
            .map(|(spec, _)| spec);
        self.panel_height.set_in(height, spec, now, cx);
        cx.notify();
    }

    fn bottom_panel(
        &self,
        snapshot: &RiseMapStateSnapshot,
        live: &RiseMapLiveSnapshot,
        open_grid: Option<RiseMapOpenGridSnapshot>,
        drafts: Vec<MapGalleryDraft>,
        tracks: RiseMapTracksSnapshot,
        friend_in_aside: bool,
        friend_card_height: f32,
        page: &Entity<Self>,
        nav_store: &Option<Entity<NavigationStore>>,
        now: std::time::Duration,
        window: &Window,
        cx: &mut App,
    ) -> Div {
        let active_user_id = SaiAccountCacheScope::active_user_id();
        let mut slots = div().size_full().flex().flex_col().justify_end();
        if let Some(grid) = self.grid_presence.value(now).filter(|_| !friend_in_aside) {
            let is_leaving = self.grid_presence.is_leaving(now);
            let card = Self::selected_grid_panel(grid, open_grid, drafts, page, cx).guide_anchor(RiseGuideAnchor::MapGridCard);
            if let Some(panel) = self.grid_presence.render_transitioned(now, window, |_| div().w_full().child(card)) {
                slots = slots.child(panel_slot(panel, is_leaving, false));
            }
        }
        if self.tracks_presence.value(now).is_some() {
            let is_leaving = self.tracks_presence.is_leaving(now);
            let panel = Self::tracks_panel(tracks, snapshot, page, cx);
            if let Some(panel) = self.tracks_presence.render_transitioned(now, window, |_| div().w_full().child(panel)) {
                slots = slots.child(panel_slot(panel, is_leaving, false));
            }
        }
        if !friend_in_aside {
            let is_expanded = self.profile_expanded;
            let is_leaving = self.friend_presence.is_leaving(now);
            let friend_panel = self.friend_presence.render_transitioned(now, window, |friend| {
                let is_own = Some(friend.id()) == active_user_id.as_deref();
                div().size_full().child(Self::friend_panel(
                    friend.clone(),
                    is_own,
                    friend_card_height,
                    is_expanded,
                    false,
                    page,
                    nav_store.clone(),
                ))
            });
            if let Some(panel) = friend_panel {
                slots = slots.child(panel_slot(panel, is_leaving, !is_leaving));
            }
            let departing = self.friend_presence.render_departing_transitioned(now, window, |friend| {
                let is_own = Some(friend.id()) == active_user_id.as_deref();
                div().size_full().child(Self::friend_panel(
                    friend.clone(),
                    is_own,
                    friend_card_height,
                    is_expanded,
                    false,
                    page,
                    nav_store.clone(),
                ))
            });
            slots = slots.children(departing.into_iter().map(|departed| div().absolute().inset_0().child(departed)));
        }
        if self.strip_presence.value(now).is_some() {
            let is_leaving = self.strip_presence.is_leaving(now);
            let reaction_inbox = MapServicesStore::reaction_inbox(cx);
            let strip_nav = nav_store.clone();
            let strip = snapshot.state.clone().map(|state| {
                MapFriendsStrip::new(
                    state,
                    live.friends.clone(),
                    reaction_inbox,
                    move |_, cx| open_friends(&strip_nav, cx),
                    |id, _, cx| interact(cx, |interactions, cx| interactions.focus_friend(&id, cx)),
                )
            });
            if let Some(strip) = strip
                && let Some(panel) = self.strip_presence.render_transitioned(now, window, |_| div().w_full().child(strip))
            {
                slots = slots.child(panel_slot(panel, is_leaving, false));
            }
        }
        if self.state_presence.value(now).is_some() {
            let is_leaving = self.state_presence.is_leaving(now);
            let state_panel = MapStatePanel::new(
                snapshot.clone(),
                |_, cx| interact(cx, |interactions, cx| interactions.present_onboarding(cx)),
                |_, cx| interact(cx, |interactions, cx| interactions.retry_load(cx)),
            );
            if let Some(panel) = self.state_presence.render_transitioned(now, window, |_| div().w_full().child(state_panel)) {
                slots = slots.child(panel_slot(panel, is_leaving, false));
            }
        }
        slots
    }
}

/// separate from the page on purpose
/// landing tiles and moving friends arrive many times a second while panning
/// the tile tick wakes this view alone and not the glass panels
struct MapCanvas {
    editing: Option<Entity<MapEditingSession>>,
    is_member: bool,
    home_track_exclusion_radius_m: i64,
    selected_friend_id: Option<String>,
    selected_grid_id: Option<String>,
    shows_photos: bool,
    bottom_inset: f32,
    chrome_bottom_inset: f32,
    zoom_bottom_inset: f32,
    controls_frame: Option<(f32, f32)>,
    frame: (f32, f32),
    on_background_tap: MapAction,
    globe_opacity: Animated<f32>,
    _observations: Vec<Subscription>,
}

impl MapCanvas {
    fn new(page: WeakEntity<WorldMapPage>, cx: &mut Context<Self>) -> Self {
        let track_tiles = RiseMapPresentation::shared(cx).read(cx).track_tiles.clone();
        let theme_store = ThemeStore::shared(cx);
        let self_location = MapSelfLocationFeed::shared(cx);
        let observations = vec![
            cx.observe(&track_tiles, |_, _, cx| cx.notify()),
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
            cx.observe(&self_location, |_, _, cx| cx.notify()),
        ];
        Self {
            editing: None,
            is_member: false,
            home_track_exclusion_radius_m: MapSettings::default_home_track_radius(),
            selected_friend_id: None,
            selected_grid_id: None,
            shows_photos: false,
            bottom_inset: 0.0,
            chrome_bottom_inset: 0.0,
            zoom_bottom_inset: 0.0,
            controls_frame: None,
            frame: (0.0, 0.0),
            on_background_tap: Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = page.update(cx, |page, cx| {
                    if page.selected_is_shown {
                        page.profile_is_minimized = true;
                    }
                    cx.notify();
                });
                interact(cx, |interactions, cx| interactions.select_friend(None, cx));
            }),
            globe_opacity: Animated::new(0.0),
            _observations: observations,
        }
    }

    fn globe_people(&self, shows_self: bool, cx: &mut App) -> Vec<MapGlobePerson> {
        let mut people: Vec<MapGlobePerson> = MapServicesStore::live(cx)
            .friends
            .iter()
            .filter_map(|friend| {
                let fix = &friend.position.as_ref()?.fix;
                Some(MapGlobePerson {
                    id: friend.id().to_owned(),
                    name: friend.user.display_name().to_owned(),
                    logo: friend.user.logo.clone(),
                    coordinate: fix.coordinate(),
                    is_self: false,
                })
            })
            .collect();
        let here = MapSelfLocationFeed::shared(cx).read(cx).location().map(|location| location.coordinate());
        let auth = AuthServicesStore::shared(cx);
        if shows_self
            && let (Some(profile), Some(coordinate)) = (auth.read(cx).profile(), here)
        {
            people.push(MapGlobePerson {
                id: profile.id.clone(),
                name: profile.name.clone(),
                logo: profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()),
                coordinate,
                is_self: true,
            });
        }
        people
    }

    fn map(&self, palette: MapPinPalette, shows_self: bool, cx: &mut App) -> RiseMapView {
        let live = MapServicesStore::live(cx);
        let tracks = MapServicesStore::tracks(cx);
        let photos = MapServicesStore::photos(cx);
        let drafts_revision = MapServicesStore::drafts(cx).revision;
        let is_member = self.is_member;
        let editing = self.editing.clone();
        let is_editing = editing.is_some();
        let shows_lines = is_member && (tracks.shows_lines || is_editing) && (!is_editing || tracks.owner_id.is_none());
        let (focus_request, focused_profile_grid_id, routes_owner_is_none, is_globe_shown, camera_request) = {
            let interactions = MapInteractionsStore::shared(cx).read(cx);
            (
                interactions.focus_request.clone(),
                interactions.focused_profile_grid_id.clone(),
                interactions.routes_owner.is_none(),
                interactions.globe_entry.is_some(),
                interactions.camera_request,
            )
        };

        let appearance = ThemeStore::shared(cx).read(cx).resolved_appearance();
        let language_code = LanguageStore::shared(cx).code().to_owned();
        let mut view = RiseMapView::new("world-map-view", appearance, language_code);
        view.self_profile = if shows_self {
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| MapSelfProfile {
                id: profile.id.clone(),
                name: profile.name.clone(),
                logo: profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()),
            })
        } else {
            None
        };
        view.friends = if is_member && !is_editing { live.friends.clone() } else { Vec::new() };
        view.friends_revision = if is_member {
            live.revision.wrapping_mul(2).wrapping_add(u64::from(is_editing))
        } else {
            0
        };
        view.selected_friend_id = self.selected_friend_id.clone();
        view.focus_request = if is_member && !is_editing { focus_request } else { None };
        view.pin_palette = Some(palette);
        view.bottom_inset = f64::from(self.bottom_inset);
        view.on_select_friend = Rc::new(|id: Option<String>, _: &mut Window, cx: &mut App| {
            interact(cx, |interactions, cx| interactions.select_friend(id, cx));
        });
        view.on_background_tap = self.on_background_tap.clone();
        view.on_select_self = Rc::new(move |_: &mut Window, cx: &mut App| {
            if !is_editing {
                interact(cx, |interactions, cx| interactions.open_own_profile(cx));
            }
        });
        view.on_focus_handled = Rc::new(|id: Uuid, _: &mut Window, cx: &mut App| {
            interact(cx, |interactions, cx| interactions.focus_handled(id, cx));
        });
        let tail = if is_member && (!is_editing || tracks.owner_id.is_none()) {
            MapInteractionsStore::displayed_track_tail(cx)
        } else {
            Vec::new()
        };
        let since_day = tracks
            .summary
            .as_ref()
            .and_then(|summary| summary.stats(tracks.period))
            .map_or(0, |stats| stats.since_day);
        let mut track_input = MapTrackLayerInput::new(shows_lines, since_day, tail);
        if tracks.owner_id.is_none() {
            track_input.home_exclusion =
                MapTrackHomeExclusion::new(tracks.home.as_ref(), self.home_track_exclusion_radius_m);
            track_input.place_exclusions = tracks
                .places
                .iter()
                .filter(|place| place.kind == MapPlaceKind::Custom && !place.record_tracks)
                .map(MapTrackHomeExclusion::from_place)
                .collect();
        }
        view.track_input = Some(track_input);
        view.track_tick = MapServicesStore::track_tiles_tick(cx);
        view.track_bands = tracks.summary.as_ref().map(|summary| summary.bands.clone()).unwrap_or_default();
        view.track_store = if is_member { MapServicesStore::track_tile_store(cx) } else { None };
        view.grids = match &editing {
            Some(editing) => {
                let session = editing.read(cx);
                if session.mode != MapEditingMode::Journey {
                    Vec::new()
                } else if session.drawing() {
                    session.own_grids.clone()
                } else {
                    session.candidates.clone()
                }
            }
            None if !self.shows_photos => Vec::new(),
            None => match &focused_profile_grid_id {
                Some(selected) => photos.grids.iter().filter(|grid| &grid.id == selected).cloned().collect(),
                None => {
                    let pins = MapInteractionsStore::draft_layout(cx).pins;
                    photos.grids.iter().cloned().chain(pins).collect()
                }
            },
        };
        let (session_id, photo_revision, editing_revision) = match &editing {
            Some(editing) => {
                let session = editing.read(cx);
                (Some(session.id), session.photo_revision, session.render_revision)
            }
            None => (None, 0, 0),
        };
        view.grids_revision = photos
            .revision
            .wrapping_mul(31)
            .wrapping_add(drafts_revision)
            .wrapping_add(hash_value(&focused_profile_grid_id))
            .wrapping_add(hash_value(&session_id))
            .wrapping_add(photo_revision);
        view.shows_photos = self.shows_photos;
        view.selected_grid_id = self.selected_grid_id.clone();
        let grid_session = editing.clone();
        view.on_select_grid = Rc::new(move |id: Option<String>, _: &mut Window, cx: &mut App| {
            if let Some(editing) = &grid_session {
                if editing.read(cx).drawing() {
                    return;
                }
                let Some(id) = id else {
                    return;
                };
                interact(cx, |interactions, cx| interactions.toggle_journey_grid(&id, editing, cx));
            } else {
                interact(cx, |interactions, cx| interactions.select_grid(id, cx));
            }
        });
        view.on_tap_own_track = (!is_editing && shows_lines && routes_owner_is_none).then(|| {
            Rc::new(|coordinate: LatLng, _: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.present_photo_picker(Some(coordinate), cx));
            }) as MapCoordinateAction
        });
        view.is_globe_shown = is_globe_shown;
        view.on_zoomed_out_to_globe = (!is_editing).then(|| {
            Rc::new(|coordinate: LatLng, zoom_level: f64, _: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.enter_globe(coordinate, zoom_level, cx));
            }) as Rc<dyn Fn(LatLng, f64, &mut Window, &mut App)>
        });
        view.camera_request = camera_request;
        view.editing = editing;
        view.editing_revision = editing_revision;
        view.places = if is_member && !is_editing {
            tracks.places.iter().filter(|place| place.kind == MapPlaceKind::Custom).cloned().collect()
        } else {
            Vec::new()
        };
        view.on_select_place = Rc::new(|id: String, _: &mut Window, cx: &mut App| {
            interact(cx, |interactions, cx| interactions.select_place(&id, cx));
        });
        view.on_hold_map = (is_member && !is_editing).then(|| {
            Rc::new(|coordinate: LatLng, _: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.hold_map(coordinate, cx));
            }) as MapCoordinateAction
        });
        view
    }
}

impl MapCanvas {
    /// plus and minus centred in the room under the map controls, beside them when that room is too low
    fn zoom_column(&self, theme: &AppTheme, cx: &App) -> Div {
        let gap = f32::from(theme.spacing._600);
        let edge = f32::from(theme.spacing._600);
        let height = MapZoomButtons::height(cx);
        let (frame_top, frame_height) = self.frame;
        let Some((controls_top, controls_bottom)) = self.controls_frame else {
            return div()
                .absolute()
                .right(px(edge))
                .top_0()
                .bottom(px(self.zoom_bottom_inset))
                .flex()
                .items_center()
                .child(MapZoomButtons::new());
        };
        let top = (controls_bottom - frame_top + gap).max(0.0);
        let room = frame_height - self.zoom_bottom_inset - top;
        if frame_height <= 0.0 || room >= height {
            return div()
                .absolute()
                .right(px(edge))
                .top(px(top))
                .bottom(px(self.zoom_bottom_inset))
                .flex()
                .items_center()
                .child(MapZoomButtons::new());
        }
        div()
            .absolute()
            .right(px(edge + PageHeaderMetrics::TOOLBAR_BUTTON_SIZE + gap))
            .top(px((controls_top - frame_top).max(0.0)))
            .child(MapZoomButtons::new())
    }
}

impl Render for MapCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let shows_self = self.is_member;
        let appearance = ThemeStore::shared(cx).read(cx).resolved_appearance();
        let palette = journey_pin_palette(&theme, appearance);
        let globe_entry = MapInteractionsStore::shared(cx).read(cx).globe_entry;
        let language_code = LanguageStore::shared(cx).code().to_owned();

        on_change("map-canvas-shows-self", shows_self, true, window, cx, |_, shown, _, cx| {
            let shown = *shown;
            interact(cx, |interactions, cx| interactions.self_pin_changed(shown, cx));
        });

        let is_map_unavailable =
            MapGlobeCameraLink::shared(cx).read(cx).map_view().is_some_and(|host| host.unavailable().is_some());
        let is_globe_hidden = globe_entry.is_none() || is_map_unavailable;
        let people = if is_globe_hidden { Vec::new() } else { self.globe_people(shows_self, cx) };
        let now = RiseClock::monotonic();
        self.globe_opacity.set(if is_globe_hidden { 0.0 } else { 1.0 }, ease_in_out(0.36), now, reduce_motion(cx));
        let globe_opacity = self.globe_opacity.frame_value(now, window);
        let map = self.map(palette, shows_self, cx);
        let globe = MapGlobeLayer {
            entry: globe_entry,
            people,
            palette,
            style_json: RiseMapStyle::json(appearance, &language_code),
            style_key: format!("{}-{language_code}", appearance.raw_value()),
            on_dive_in: Rc::new(|coordinate: LatLng, zoom_level: f64, _: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.leave_globe(coordinate, zoom_level, cx));
            }),
            on_settle: Rc::new(|coordinate: LatLng, zoom_level: f64, _: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.globe_settled(coordinate, zoom_level, cx));
            }),
            on_select_person: Rc::new(|id: String, _: &mut Window, cx: &mut App| {
                interact(cx, |interactions, cx| interactions.focus_friend_from_globe(&id, cx));
            }),
            is_hidden: is_globe_hidden && globe_opacity <= 0.0,
        };
        let globe_host = div().absolute().opacity(globe_opacity).overflow_hidden();
        let globe_host = if is_globe_hidden && globe_opacity <= 0.0 { globe_host.size_0() } else { globe_host.inset_0() };
        let is_erasing = self.editing.as_ref().is_some_and(|editing| editing.read(cx).mode == MapEditingMode::Eraser);
        let zoom_buttons = (!is_erasing).then(|| self.zoom_column(&theme, cx));
        let measured = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .child(measure("world-map-canvas-frame", move |bounds, _, cx| {
                let frame = (f32::from(bounds.top()), f32::from(bounds.size.height));
                let _ = measured.update(cx, |canvas, cx| {
                    if canvas.frame != frame {
                        canvas.frame = frame;
                        cx.notify();
                    }
                });
            }))
            .child(map)
            .child(globe_host.child(globe))
            .child(
                div()
                    .absolute()
                    .left(px(8.0))
                    .bottom(px(self.chrome_bottom_inset + 4.0))
                    .child(RiseMapAttribution),
            )
            .children(zoom_buttons)
    }
}

use std::cell::Cell;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    App, AsyncApp, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, ListState, Pixels, Point, SharedString,
    StyleRefinement, Subscription, Window, list,
};
use rise_i18n::{tr, tr_plural};
use rise_theme::AppTheme;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::animated_tabs_ui::animated_tabs_types::TabConfig;
use rise_ui::button_ui::ButtonUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::map::components::map_grid_editor::MapGridThumbnail;
use crate::modules::map::components::map_profile_scroll_anchor::{MapProfileScrollAnchor, MapProfileScrollMemory};
use crate::modules::map::components::tracks::map_tracks_panel::MapTracksPanel;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::engine::rise_map_profiles_repository::{MapProfile, MapProfileSnapshot, MapProfileTab};
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_actions::map_types::{MapFriend, MapLiveFriend, MapUser};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;

const LIST_OVERDRAW: f32 = 300.0;

#[derive(IntoElement)]
pub struct MapFriendProfileContent {
    friend: MapLiveFriend,
    is_expanded: bool,
    is_own: bool,
    active_tab: Binding<MapProfileTab>,
    on_grid: Rc<dyn Fn(MapPhotoGrid, &mut Window, &mut App)>,
    on_collapse: Rc<dyn Fn(&mut Window, &mut App)>,
}

/// state of the view
/// lists keep their measured rows across tab switches and only changed rows are spliced
struct MapFriendProfileContentState {
    snapshot: MapProfileSnapshot,
    loading: Rc<Cell<bool>>,
    mutating_ids: HashSet<String>,
    peer: Option<MapUser>,
    peer_presented: bool,
    media_list: ListState,
    friends_list: ListState,
    media_rows: Vec<u64>,
    friends_rows: Vec<u64>,
    media_anchor: MapProfileScrollAnchor,
    friends_anchor: MapProfileScrollAnchor,
    _tracks_observation: Subscription,
}

impl MapFriendProfileContentState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseMapPresentation::shared(cx);
        let tracks = presentation.read(cx).tracks.clone();
        Self {
            snapshot: MapProfileSnapshot::default(),
            loading: Rc::new(Cell::new(false)),
            mutating_ids: HashSet::new(),
            peer: None,
            peer_presented: false,
            media_list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            friends_list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            media_rows: Vec::new(),
            friends_rows: Vec::new(),
            media_anchor: MapProfileScrollAnchor::new(MapProfileScrollMemory::new()),
            friends_anchor: MapProfileScrollAnchor::new(MapProfileScrollMemory::new()),
            _tracks_observation: cx.observe(&tracks, |_, _, cx| cx.notify()),
        }
    }

    fn sync_rows(&mut self) {
        let snapshot = &self.snapshot;
        let loading = self.loading.get();
        let media = Self::row_hashes(&snapshot.grids, (snapshot.failed, snapshot.has_more_grids, loading));
        let friends = Self::row_hashes(&snapshot.friends, (snapshot.failed, snapshot.has_more_friends, loading));
        Self::splice_changed(&self.media_list, &mut self.media_rows, media);
        Self::splice_changed(&self.friends_list, &mut self.friends_rows, friends);
    }

    fn row_hashes<T: Hash>(items: &[T], pagination: (bool, bool, bool)) -> Vec<u64> {
        let mut rows: Vec<u64> = items.iter().map(Self::hash_of).collect();
        rows.push(Self::hash_of(&(pagination, items.is_empty())));
        rows
    }

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn splice_changed(list: &ListState, rows: &mut Vec<u64>, next: Vec<u64>) {
        if *rows == next {
            return;
        }
        let prefix = rows.iter().zip(next.iter()).take_while(|(a, b)| a == b).count();
        let max_suffix = rows.len().min(next.len()) - prefix;
        let suffix = rows.iter().rev().zip(next.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
        list.splice(prefix..rows.len() - suffix, next.len() - prefix - suffix);
        *rows = next;
    }
}

struct MapProfileLoadingGuard(Rc<Cell<bool>>);

impl Drop for MapProfileLoadingGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

#[derive(Clone)]
struct MapFriendProfileContentParts {
    state: Entity<MapFriendProfileContentState>,
    loading: Rc<Cell<bool>>,
    friend: MapLiveFriend,
    is_expanded: bool,
    is_own: bool,
    active_tab: Binding<MapProfileTab>,
    on_grid: Rc<dyn Fn(MapPhotoGrid, &mut Window, &mut App)>,
    on_collapse: Rc<dyn Fn(&mut Window, &mut App)>,
    nav_store: Option<Entity<NavigationStore>>,
}

struct MapProfileListView {
    parts: MapFriendProfileContentParts,
    tab: MapProfileTab,
    key: u64,
}

impl Render for MapProfileListView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        let list_state = self.parts.state.update(cx, |state, _| {
            let MapFriendProfileContentState { media_anchor, media_list, friends_anchor, friends_list, .. } = state;
            if tab == MapProfileTab::Friends {
                friends_anchor.attach(friends_list);
                friends_list.clone()
            } else {
                media_anchor.attach(media_list);
                media_list.clone()
            }
        });
        let parts = self.parts.clone();
        list(list_state, move |index, window, cx| {
            if tab == MapProfileTab::Friends {
                parts.friend_row(index, window, cx)
            } else {
                parts.media_row(index, window, cx)
            }
        })
        .size_full()
    }
}

impl MapFriendProfileContent {
    pub fn new(
        friend: MapLiveFriend,
        is_expanded: bool,
        is_own: bool,
        active_tab: Binding<MapProfileTab>,
        on_grid: impl Fn(MapPhotoGrid, &mut Window, &mut App) + 'static,
        on_collapse: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { friend, is_expanded, is_own, active_tab, on_grid: Rc::new(on_grid), on_collapse: Rc::new(on_collapse) }
    }
}

impl RenderOnce for MapFriendProfileContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state =
            window.use_keyed_state("map-friend-profile-content", cx, |_, cx| MapFriendProfileContentState::new(cx));
        let parts = MapFriendProfileContentParts {
            loading: state.read(cx).loading.clone(),
            state: state.clone(),
            friend: self.friend,
            is_expanded: self.is_expanded,
            is_own: self.is_own,
            active_tab: self.active_tab,
            on_grid: self.on_grid,
            on_collapse: self.on_collapse,
            nav_store: env::<NavigationStoreKey>(cx),
        };

        let task_parts = parts.clone();
        use_task("map-friend-profile-expanded", parts.is_expanded, window, cx, async move |cx| {
            let cx: &mut AsyncApp = cx;
            let has_profile = task_parts.state.read_with(cx, |state, _| state.snapshot.profile.is_some());
            if !task_parts.is_expanded && has_profile {
                return;
            }
            task_parts.refresh(cx).await;
        });
        let tab_parts = parts.clone();
        let current_tab = parts.active_tab.get(cx);
        on_change("map-friend-profile-tab", current_tab, false, window, cx, move |_, _, _, cx| {
            if !tab_parts.is_expanded {
                return;
            }
            cx.spawn(async move |cx: &mut AsyncApp| tab_parts.load_page(false, cx).await).detach();
        });

        let tabs = MapProfileTab::ALL_CASES
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let page = *page;
                let tab_parts = parts.clone();
                TabConfig::new(index as i64, page.title(), move |window: &mut Window, cx: &mut App| {
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(tab_parts.content(page, window, cx))
                        .into_any_element()
                })
            })
            .collect();

        div()
            .relative()
            .size_full()
            .child(
                AnimatedTabs::new("map-friend-profile-tabs", tabs, parts.tab_index())
                    .horizontal_padding(12.0)
                    .top_padding(0.0)
                    .bottom_padding(2.0)
                    .on_swipe_back_beyond_first(Some(Rc::new(|_: &mut Window, _: &mut App| {}))),
            )
            .children(parts.peer_sheet(cx))
    }
}

impl MapFriendProfileContentParts {
    fn tab_index(&self) -> Binding<i64> {
        let reader = self.active_tab.clone();
        let writer = self.active_tab.clone();
        Binding::new(
            move |cx| {
                let tab = reader.get(cx);
                MapProfileTab::ALL_CASES.iter().position(|page| *page == tab).unwrap_or(0) as i64
            },
            move |index, window, cx| {
                if let Some(page) = usize::try_from(index).ok().and_then(|index| MapProfileTab::ALL_CASES.get(index)) {
                    writer.set(*page, window, cx);
                }
            },
        )
    }

    fn peer_sheet(&self, cx: &mut App) -> Option<AnyElement> {
        let peer = self.state.read(cx).peer.clone()?;
        let theme = rise_ui::theme(cx).clone();
        let is_presented = Binding::entity(
            self.state.clone(),
            |state: &MapFriendProfileContentState| state.peer_presented,
            |state, value, _| state.peer_presented = value,
        );
        let did_dismiss_state = self.state.clone();
        let refused_state = self.state.clone();
        let content_parts = self.clone();
        let content_user = peer.clone();
        Some(
            BottomSheetUi::new(
                SharedString::from(format!("map-friend-profile-peer-{}", peer.id)),
                is_presented,
                move |_, _| {
                    let profile_parts = content_parts.clone();
                    let back_parts = content_parts.clone();
                    MapProfilePeerContent::new(
                        content_user.clone(),
                        move |user, window, cx| {
                            profile_parts.set_peer_presented(false, cx);
                            (profile_parts.on_collapse)(window, cx);
                            if let Some(nav_store) = &profile_parts.nav_store {
                                nav_store.update(cx, |nav_store, cx| {
                                    nav_store.push(
                                        RootRoute::UserPage { user_id: Some(user.id), tag: None },
                                        NavigationStoreSettlingPush::Drop,
                                        cx,
                                    );
                                });
                            }
                        },
                        move |_, cx| back_parts.set_peer_presented(false, cx),
                    )
                },
                |_, _| Empty,
            )
            .snap_fractions(vec![0.45, 0.7])
            .fits_content(true)
            .full_page_hides_footer(true)
            .sheet_background(Some(theme.bg._100))
            .on_did_dismiss(move |_, cx| {
                did_dismiss_state.update(cx, |state, cx| {
                    state.peer = None;
                    cx.notify();
                });
            })
            .on_presentation_refused(move |_, cx| {
                refused_state.update(cx, |state, cx| {
                    state.peer_presented = false;
                    state.peer = None;
                    cx.notify();
                });
            })
            .into_any_element(),
        )
    }

    fn set_peer_presented(&self, presented: bool, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.peer_presented = presented;
            cx.notify();
        });
    }

    fn content(&self, page: MapProfileTab, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let (profile, failed) = {
            let snapshot = &self.state.read(cx).snapshot;
            (snapshot.profile.clone(), snapshot.failed)
        };
        let Some(profile) = profile else {
            if failed {
                return self.retry("map-friend-profile-retry", false, &theme, true);
            }
            return div().p(px(30.0)).child(progress_view()).into_any_element();
        };
        match page {
            MapProfileTab::Statistics => {
                if profile.can_view_tracks {
                    self.statistics(cx)
                } else {
                    Self::unavailable(&theme)
                }
            }
            MapProfileTab::Media => {
                if profile.can_view_media {
                    self.list_view(MapProfileTab::Media, window, cx)
                } else {
                    Self::unavailable(&theme)
                }
            }
            MapProfileTab::Friends => {
                if profile.can_view_friends {
                    self.list_view(MapProfileTab::Friends, window, cx)
                } else {
                    Self::unavailable(&theme)
                }
            }
        }
    }

    fn statistics(&self, cx: &mut App) -> AnyElement {
        let tracks = MapServicesStore::tracks(cx);
        let expected_owner = if self.is_own { None } else { Some(self.friend.id().to_owned()) };
        let body = if tracks.owner_id == expected_owner {
            let is_own = self.is_own;
            let friend = self.friend.clone();
            MapTracksPanel::new(
                tracks,
                false,
                |period, _, cx| {
                    MapInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.select_track_period(period, cx));
                },
                move |_, cx| {
                    MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        if is_own {
                            interactions.retry_load(cx);
                        } else {
                            interactions.show_routes(&friend, true, cx);
                        }
                    });
                },
            )
            .embedded(true)
            .into_any_element()
        } else {
            div().p(px(30.0)).child(progress_view()).into_any_element()
        };
        vstack(0.0).w_full().px(px(12.0)).child(body).into_any_element()
    }

    /// media and friends lists render in their own cached view
    /// a wheel tick would otherwise notify the map page with its canvas
    fn list_view(&self, tab: MapProfileTab, window: &mut Window, cx: &mut App) -> AnyElement {
        let key = MapFriendProfileContentState::hash_of(&(&self.friend, self.is_expanded, self.is_own));
        let element_id = ElementId::Name(SharedString::from(match tab {
            MapProfileTab::Friends => "map-friend-profile-friends-list",
            _ => "map-friend-profile-media-list",
        }));
        let parts = self.clone();
        let view = window.with_global_id(element_id, |global_id, window| {
            window.with_element_state(global_id, |existing: Option<Entity<MapProfileListView>>, _| {
                let view = existing
                    .unwrap_or_else(|| cx.new(|_| MapProfileListView { parts: parts.clone(), tab, key }));
                (view.clone(), view)
            })
        });
        view.update(cx, |view, cx| {
            view.parts = parts;
            if view.key != key {
                view.key = key;
                cx.notify();
            }
        });
        view.cached(StyleRefinement::default().size_full()).into_any_element()
    }

    fn media_row(&self, index: usize, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let (grid, has_more, is_empty) = {
            let snapshot = &self.state.read(cx).snapshot;
            (snapshot.grids.get(index).cloned(), snapshot.has_more_grids, snapshot.grids.is_empty())
        };
        let Some(grid) = grid else {
            return self.pagination_row(MapProfileTab::Media, has_more, is_empty, 14.0, window, cx);
        };
        div()
            .w_full()
            .px(px(12.0))
            .when(index > 0, |row| row.pt(px(14.0)))
            .child(self.grid_row(grid, &theme, cx))
            .into_any_element()
    }

    fn grid_row(&self, grid: MapPhotoGrid, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let actions = self.grid_actions(&grid);
        let disabled = self.state.read(cx).mutating_ids.contains(&grid.id);
        let action_parts = self.clone();
        let action_grid = grid.clone();
        let tap_grid = grid.clone();
        let on_tap = self.on_grid.clone();
        let content_grid = grid.clone();
        let content_theme = theme.clone();
        HoldContextMenuContainer::new(
            SharedString::from(format!("map-friend-profile-grid-{}", grid.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| Self::grid_label(&content_grid, &content_theme)),
        )
        .show_emoji_list(false)
        .disabled(disabled)
        .source_identity(Some(grid.id))
        .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| match id.as_str() {
            "open" => (action_parts.on_grid)(action_grid.clone(), window, cx),
            "edit" => {
                (action_parts.on_collapse)(window, cx);
                let grid = action_grid.clone();
                let owner = action_parts.friend.user.clone();
                MapInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.edit_profile_grid(grid, owner, cx));
            }
            "delete" => action_parts.confirm_delete_grid(action_grid.clone(), cx),
            _ => {}
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
            on_tap(tap_grid.clone(), window, cx);
        }))
        .into_any_element()
    }

    fn grid_label(grid: &MapPhotoGrid, theme: &AppTheme) -> AnyElement {
        let mut text = vstack(5.0).items_start().min_w_0().child(
            MainText::new(grid.display_title()).px(17.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(2),
        );
        if !grid.description.is_empty() {
            text = text.child(
                MainText::new(grid.description.clone()).px(14.0).color(theme.text.secondary).number_of_lines(2),
            );
        }
        text = text.child(
            MainText::new(tr_plural("map_grid_count", grid.photo_count)).px(12.0).color(theme.text.secondary),
        );
        hstack(14.0)
            .w_full()
            .child(MapGridThumbnail::new(grid.cover_url.clone(), 76.0))
            .child(text)
            .child(spacer_min_w(0.0))
            .children(IconUi::sized("arrow.up.right", px(17.0), theme.text.secondary, false))
            .into_any_element()
    }

    fn friend_row(&self, index: usize, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let (entry, has_more, is_empty) = {
            let snapshot = &self.state.read(cx).snapshot;
            (snapshot.friends.get(index).cloned(), snapshot.has_more_friends, snapshot.friends.is_empty())
        };
        let Some(entry) = entry else {
            return self.pagination_row(MapProfileTab::Friends, has_more, is_empty, 0.0, window, cx);
        };

        let peer_state = self.state.clone();
        let peer_user = entry.user.clone();
        let user = &entry.user;
        let mut row = hstack(8.0).w_full().px(px(12.0)).child(
            plain(SharedString::from(format!("map-friend-profile-friend-{}", entry.id())))
                .flex_1()
                .min_w_0()
                .on_press(move |_, cx| {
                    let peer = peer_user.clone();
                    peer_state.update(cx, |state, cx| {
                        state.peer = Some(peer);
                        state.peer_presented = true;
                        cx.notify();
                    });
                })
                .child(
                    hstack(12.0)
                        .w_full()
                        .h(px(68.0))
                        .child(
                            UserLogoUi::new(SharedString::from(format!("map-friend-profile-logo-{}", entry.id())))
                                .source(SharedString::from(user.logo.clone()))
                                .name(SharedString::from(user.display_name().to_owned()))
                                .user_id(SharedString::from(entry.id().to_owned()))
                                .size(46.0),
                        )
                        .child(
                            MainText::new(user.display_name().to_owned())
                                .px(16.0)
                                .font_weight(FontWeight::MEDIUM)
                                .number_of_lines(1),
                        )
                        .child(spacer_min_w(0.0))
                        .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(12.0), theme.text.secondary, false)),
                ),
        );
        if self.is_own {
            let disabled = self.state.read(cx).mutating_ids.contains(entry.id());
            let remove_parts = self.clone();
            let remove_entry = entry.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("map-friend-profile-remove-label-{}", entry.id())))
                    .aria_label(tr("map_friend_remove_action"))
                    .tooltip_label(tr("map_friend_remove_action"))
                    .child(
                        plain(SharedString::from(format!("map-friend-profile-remove-{}", entry.id())))
                            .disabled(disabled)
                            .on_press(move |_, cx| remove_parts.confirm_remove_friend(remove_entry.clone(), cx))
                            .child(
                                div()
                                    .w(px(40.0))
                                    .h(px(44.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .children(IconUi::sized("person.badge.minus", px(17.0), theme.text.secondary, false)),
                            ),
                    ),
            );
        }
        row.into_any_element()
    }

    fn unavailable(theme: &AppTheme) -> AnyElement {
        vstack(14.0)
            .p(px(40.0))
            .children(IconUi::sized("lock.fill", px(28.0), theme.text.secondary, true))
            .child(MainText::new(tr("server_error_forbidden")).px(15.0).color(theme.text.secondary))
            .into_any_element()
    }

    fn grid_actions(&self, grid: &MapPhotoGrid) -> Vec<HoldContextMenuAction> {
        let mut actions = vec![HoldContextMenuAction::new("open", tr("ctx_open")).system_image("map")];
        if self.is_own && Some(&grid.owner_id) == SaiAccountCacheScope::active_user_id().as_ref() {
            actions.push(HoldContextMenuAction::new("edit", tr("edit_action")).system_image("pencil"));
            actions.push(
                HoldContextMenuAction::new("delete", tr("map_grid_delete")).system_image("trash").destructive(true),
            );
        }
        actions
    }

    fn confirm_delete_grid(&self, grid: MapPhotoGrid, cx: &mut App) {
        if !self.is_own || self.state.read(cx).mutating_ids.contains(&grid.id) {
            return;
        }
        let epoch = SaiAccountCacheScope::active_epoch();
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("map_grid_delete_title"),
            &tr("map_grid_delete_text"),
            Some(tr("map_grid_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                if !SaiAccountCacheScope::is_current_account_epoch(Some(parts.friend.id()), epoch.as_deref()) {
                    return;
                }
                let grid_id = grid.id.clone();
                parts.set_mutating(grid_id.clone(), true, cx);
                let task = MapInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.delete_profile_grid(grid, cx));
                cx.spawn(async move |cx: &mut AsyncApp| {
                    if task.await {
                        parts.refresh(cx).await;
                    }
                    cx.update(|cx| parts.set_mutating(grid_id, false, cx));
                })
                .detach();
            },
            cx,
        );
    }

    fn confirm_remove_friend(&self, entry: MapFriend, cx: &mut App) {
        if !self.is_own || self.state.read(cx).mutating_ids.contains(entry.id()) {
            return;
        }
        let epoch = SaiAccountCacheScope::active_epoch();
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("map_friend_remove_title"),
            &tr("map_friend_remove_message").replace("{{name}}", entry.user.display_name()),
            Some(tr("map_friend_remove_action")),
            None,
            false,
            None,
            None,
            move |cx| {
                if !SaiAccountCacheScope::is_current_account_epoch(Some(parts.friend.id()), epoch.as_deref()) {
                    return;
                }
                let entry_id = entry.id().to_owned();
                parts.set_mutating(entry_id.clone(), true, cx);
                let removed_id = entry_id.clone();
                let task = MapInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.remove_profile_friend(removed_id, cx));
                cx.spawn(async move |cx: &mut AsyncApp| {
                    if task.await {
                        parts.refresh(cx).await;
                    }
                    cx.update(|cx| parts.set_mutating(entry_id, false, cx));
                })
                .detach();
            },
            cx,
        );
    }

    fn set_mutating(&self, id: String, mutating: bool, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            if mutating {
                state.mutating_ids.insert(id);
            } else {
                state.mutating_ids.remove(&id);
            }
            cx.notify();
        });
    }

    fn retry(&self, id: &'static str, more: bool, theme: &AppTheme, wide: bool) -> AnyElement {
        let parts = self.clone();
        vstack(10.0)
            .w_full()
            .p(px(if wide { 30.0 } else { 16.0 }))
            .child(
                MainText::new(tr("map_load_failed"))
                    .px(15.0)
                    .color(theme.text.secondary)
                    .tac(TextAlignT::Center),
            )
            .child(
                plain(id)
                    .on_press(move |_, cx| {
                        let parts = parts.clone();
                        cx.spawn(async move |cx: &mut AsyncApp| {
                            if wide {
                                parts.refresh(cx).await;
                            } else {
                                parts.load_page(more, cx).await;
                            }
                        })
                        .detach();
                    })
                    .child(MainText::new(tr("common_retry")).px(17.0).color(theme.primary._100)),
            )
            .into_any_element()
    }

    /// the last row of a list, bottom padding 30 plus the page state
    fn pagination_row(
        &self,
        page: MapProfileTab,
        has_more: bool,
        is_empty: bool,
        spacing: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let content = self.pagination(page, has_more, is_empty, window, cx);
        let has_rows = !is_empty && spacing > 0.0 && content.is_some();
        div()
            .w_full()
            .px(px(12.0))
            .pb(px(30.0))
            .when(has_rows, |row| row.pt(px(spacing)))
            .flex()
            .flex_col()
            .items_center()
            .children(content)
            .into_any_element()
    }

    fn pagination(
        &self,
        page: MapProfileTab,
        has_more: bool,
        is_empty: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let theme = rise_ui::theme(cx).clone();
        let (failed, page_revision) = {
            let snapshot = &self.state.read(cx).snapshot;
            (snapshot.failed, snapshot.page_revision)
        };
        if failed {
            let id = if page == MapProfileTab::Media { "map-friend-profile-media-retry" } else { "map-friend-profile-friends-retry" };
            return Some(self.retry(id, has_more, &theme, false));
        }
        if has_more {
            let is_visible = self.is_expanded && self.active_tab.get(cx) == page;
            let parts = self.clone();
            use_task(
                SharedString::from(format!("map-friend-profile-page-{}-{page_revision}", page.id())),
                is_visible,
                window,
                cx,
                async move |cx| {
                    if !is_visible {
                        return;
                    }
                    let cx: &mut AsyncApp = cx;
                    parts.load_page(true, cx).await;
                },
            );
            return Some(div().p(px(16.0)).child(progress_view()).into_any_element());
        }
        if !is_empty {
            return None;
        }
        if self.loading.get() {
            return Some(div().p(px(32.0)).child(progress_view()).into_any_element());
        }
        let key = if page == MapProfileTab::Media { "map_media_library_empty" } else { "map_friends_empty" };
        Some(div().py(px(32.0)).child(MainText::new(tr(key)).color(theme.text.secondary)).into_any_element())
    }

    /// true while the panel is still shown
    /// a leaving panel still renders with live bindings so its tasks would fire again
    fn is_presented(&self, cx: &mut AsyncApp) -> bool {
        let id = self.friend.id().to_owned();
        cx.update(|cx| {
            MapInteractionsStore::shared(cx)
                .read(cx)
                .selected_friend_id
                .as_deref()
                .is_none_or(|selected| selected == id)
        })
    }

    fn commit(&self, snapshot: Option<MapProfileSnapshot>, cx: &mut AsyncApp) {
        let snapshot = snapshot.filter(|snapshot| {
            snapshot.profile.as_ref().is_none_or(|profile| profile.user.id == self.friend.id())
        });
        self.state.update(cx, |state, cx| {
            if let Some(snapshot) = snapshot {
                state.snapshot = snapshot;
            }
            state.sync_rows();
            cx.notify();
        });
    }

    async fn refresh(&self, cx: &mut AsyncApp) {
        if !self.is_presented(cx) {
            return;
        }
        let owner_id = self.friend.id().to_owned();
        let task = cx.update(|cx| {
            MapInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.load_profile(owner_id, None, false, cx))
        });
        let Some(result) = task.await else {
            return;
        };
        self.commit(Some(result), cx);
        self.load_page(false, cx).await;
    }

    async fn load_page(&self, more: bool, cx: &mut AsyncApp) {
        if !self.is_expanded || self.loading.get() || !self.is_presented(cx) {
            return;
        }
        self.loading.set(true);
        let guard = MapProfileLoadingGuard(self.loading.clone());
        self.commit(None, cx);
        let requested = cx.update(|cx| self.active_tab.get(cx));
        let owner_id = self.friend.id().to_owned();
        let task = cx.update(|cx| {
            MapInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.load_profile(owner_id, Some(requested), more, cx))
        });
        let result = task.await;
        if let Some(result) = result {
            self.commit(Some(result), cx);
            if requested != cx.update(|cx| self.active_tab.get(cx)) {
                self.load_page_after_tab_change(cx).await;
            }
        }
        drop(guard);
        self.commit(None, cx);
    }

    async fn load_page_after_tab_change(&self, cx: &mut AsyncApp) {
        self.loading.set(false);
        Box::pin(self.load_page(false, cx)).await;
    }
}

struct MapProfilePeerContentState {
    profile: Option<MapProfile>,
    loading: bool,
    saving: bool,
    failed: bool,
}

#[derive(IntoElement)]
pub struct MapProfilePeerContent {
    user: MapUser,
    on_profile: Rc<dyn Fn(MapUser, &mut Window, &mut App)>,
    on_back: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl MapProfilePeerContent {
    pub fn new(
        user: MapUser,
        on_profile: impl Fn(MapUser, &mut Window, &mut App) + 'static,
        on_back: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { user, on_profile: Rc::new(on_profile), on_back: Rc::new(on_back) }
    }

    async fn load(state: Entity<MapProfilePeerContentState>, user_id: String, cx: &mut AsyncApp) {
        state.update(cx, |state, cx| {
            state.loading = true;
            cx.notify();
        });
        let task = cx.update(|cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_peer_profile(user_id, cx))
        });
        let profile = task.await;
        state.update(cx, |state, cx| {
            state.failed = profile.is_none();
            state.profile = profile;
            state.loading = false;
            cx.notify();
        });
    }

    fn change_request(state: Entity<MapProfilePeerContentState>, user_id: String, cx: &mut App) {
        let peer = state.read(cx);
        let Some(profile) = peer.profile.clone().filter(|_| !peer.saving) else {
            return;
        };
        state.update(cx, |state, cx| {
            state.saving = true;
            state.failed = false;
            cx.notify();
        });
        let request_user = user_id.clone();
        let task = MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.change_profile_friend_request(request_user, profile.outgoing_request_id, cx)
        });
        cx.spawn(async move |cx: &mut AsyncApp| {
            let succeeded = task.await;
            state.update(cx, |state, cx| {
                state.failed = !succeeded;
                cx.notify();
            });
            if succeeded {
                Self::load(state.clone(), user_id, cx).await;
            }
            state.update(cx, |state, cx| {
                state.saving = false;
                cx.notify();
            });
        })
        .detach();
    }
}

impl RenderOnce for MapProfilePeerContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let user = self.user;
        let state = window.use_keyed_state(
            SharedString::from(format!("map-profile-peer-state-{}", user.id)),
            cx,
            |_, _| MapProfilePeerContentState { profile: None, loading: true, saving: false, failed: false },
        );

        let task_state = state.clone();
        let task_user = user.id.clone();
        use_task(SharedString::from(format!("map-profile-peer-load-{}", user.id)), (), window, cx, async move |cx| {
            let cx: &mut AsyncApp = cx;
            Self::load(task_state, task_user, cx).await;
        });

        let (profile, loading, saving, failed) = {
            let peer = state.read(cx);
            (peer.profile.clone(), peer.loading, peer.saving, peer.failed)
        };
        let logo = profile.as_ref().map(|profile| profile.user.logo.clone()).unwrap_or_else(|| user.logo.clone());
        let on_back = self.on_back;

        let mut column = vstack(18.0)
            .w_full()
            .child(
                hstack(None).w_full().child(spacer_min_w(0.0)).child(
                    div().id("map-profile-peer-close-label").aria_label(tr("close")).tooltip_label(tr("close")).child(
                        plain("map-profile-peer-close").on_press(move |window, cx| on_back(window, cx)).child(
                            div()
                                .size(px(32.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("xmark", px(17.0), theme.text.primary, false)),
                        ),
                    ),
                ),
            )
            .child(
                UserLogoUi::new(SharedString::from(format!("map-profile-peer-logo-{}", user.id)))
                    .source(SharedString::from(logo))
                    .name(SharedString::from(user.display_name().to_owned()))
                    .user_id(SharedString::from(user.id.clone()))
                    .size(70.0),
            )
            .child(MainText::new(user.display_name().to_owned()).px(21.0).font_weight(FontWeight::BOLD));

        if let Some(profile) = profile {
            let on_profile = self.on_profile;
            let profile_user = profile.user.clone();
            column = column
                .child(
                    MainText::new(tr_plural("map_friends_count", profile.friends_count))
                        .px(14.0)
                        .color(theme.text.secondary),
                )
                .child(
                    ButtonUi::new("map-profile-peer-open")
                        .height(46.0)
                        .on_press(move |window, cx| on_profile(profile_user.clone(), window, cx))
                        .child(
                            MainText::new(tr("ctx_open_profile"))
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(theme.primary._100)),
                        ),
                );
            if !profile.is_friend && Some(&user.id) != SaiAccountCacheScope::active_user_id().as_ref() {
                let request_state = state.clone();
                let request_user = user.id.clone();
                let key = if profile.outgoing_request_id.is_none() { "map_add_friend_title" } else { "map_request_cancel" };
                let button = ButtonUi::new("map-profile-peer-request")
                    .disabled(saving)
                    .background_color(theme.bg._200)
                    .height(46.0)
                    .on_press(move |_, cx| Self::change_request(request_state.clone(), request_user.clone(), cx));
                column = column.child(if saving {
                    button.child(progress_view())
                } else {
                    button.child(MainText::new(tr(key)).px(15.0).color(theme.text.primary))
                });
            }
        } else if loading {
            column = column.child(progress_view());
        }
        if failed {
            let retry_state = state.clone();
            let retry_user = user.id.clone();
            column = column.child(
                plain("map-profile-peer-retry")
                    .on_press(move |_, cx| {
                        let state = retry_state.clone();
                        let user_id = retry_user.clone();
                        cx.spawn(async move |cx: &mut AsyncApp| Self::load(state, user_id, cx).await).detach();
                    })
                    .child(MainText::new(tr("common_retry")).px(17.0).color(theme.primary._100)),
            );
        }

        div()
            .id("map-profile-peer-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(column.px(px(16.0)).pb(px(20.0)))
    }
}

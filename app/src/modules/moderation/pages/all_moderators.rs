use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, ListAlignment, Pixels, Point,
    SharedString, StyleRefinement, Subscription, Task, Window,
};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, SettingsWrapperUi, list_shell_floating_top_inset,
    list_shell_safe_area_insets,
};
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationBadge, ModerationFilterPager, ModerationListSkeleton, ModerationMetrics, ModerationPersonRow,
    ModerationRowSeparator, ModerationSegmentedFilter, moderation_child_id,
};
use crate::modules::moderation::pages::moderator_permissions_editor::ModeratorPermissionsEditor;
use crate::modules::moderation::stores::moderation_actions::moderation_actions::{
    ModerationActionDone, ModerationActionsStore,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    ModeratorKind, ModeratorKindExt, ModeratorKindFilter, ModeratorPermissions,
};
use crate::modules::moderation::stores::moderation_actions::moderation_wire_types::{
    ModeratorWithUserWireDTO, ProtoModeratorProfileWithUserExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;

const PAGE_ID: &str = "all-moderators";
const SKELETON_ROWS: usize = 10;
const PREFETCH_THRESHOLD: usize = 8;
const HOLD_PROFILE: &str = "profile";
const HOLD_RIGHTS: &str = "rights";
const HOLD_REMOVE: &str = "remove";
const RETURN_KEY: &str = "enter";
const ROW_HEIGHT: f32 = ModerationMetrics::AVATAR_SIZE + 20.0;

fn page_id(name: impl Into<SharedString>) -> ElementId {
    moderation_child_id(&ElementId::Name(PAGE_ID.into()), name)
}

struct AllModeratorsState {
    pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    edit_rights_for: Option<ModeratorWithUserWireDTO>,
    edit_permissions: ModeratorPermissions,
    list: ListUi,
    list_ids: Vec<String>,
    roster: AllModeratorsRoster,
    _observations: Vec<Subscription>,
}

/// decoded roster page of the selected filter, as the list and title read it
#[derive(Clone, PartialEq)]
struct AllModeratorsRoster {
    moderators: Rc<Vec<ModeratorWithUserWireDTO>>,
    total: i64,
    has_more: bool,
    relative_id: Option<String>,
    cache_key: SharedString,
    kind_filter: ModeratorKindFilter,
    status: SaiWsSliceStatus,
    error: Option<String>,
}

impl AllModeratorsRoster {
    fn read(cx: &mut App) -> Self {
        let kind_filter = ModerationServicesStore::shared(cx).read(cx).kind_filter;
        Self {
            moderators: Rc::new(ModerationServicesStore::moderators(cx)),
            total: ModerationServicesStore::moderators_total(cx),
            has_more: ModerationServicesStore::moderators_has_more(cx),
            relative_id: ModerationServicesStore::moderators_relative_id(cx).map(|id| id.to_string()),
            cache_key: ModerationServicesStore::moderators_cache_key(kind_filter, cx).into(),
            kind_filter,
            status: ModerationActionsStore::shared(cx).read(cx).all_moderators_sai.status(),
            error: ModerationServicesStore::shared(cx).read(cx).moderators_error.clone(),
        }
    }
}

impl AllModeratorsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ModerationServicesStore::shared(cx);
        let actions = ModerationActionsStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            pager_live_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            edit_rights_for: None,
            edit_permissions: ModeratorPermissions::default(),
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            roster: AllModeratorsRoster::read(cx),
            _observations: vec![
                cx.observe(&services, |this, _, cx| this.reread_roster(cx)),
                cx.observe(&actions, |this, _, cx| this.reread_roster(cx)),
                cx.observe(&cache_revisions, |this, _, cx| this.reread_roster(cx)),
            ],
        }
    }

    fn reread_roster(&mut self, cx: &mut Context<Self>) {
        let roster = AllModeratorsRoster::read(cx);
        if roster != self.roster {
            self.roster = roster;
            cx.notify();
        }
    }

    fn sync_list(&mut self, ids: Vec<String>) {
        if ids == self.list_ids {
            return;
        }
        if !self.list_ids.is_empty() && ids.len() > self.list_ids.len() && ids.starts_with(&self.list_ids) {
            self.list.appended(ids.len() - self.list_ids.len());
        } else {
            self.list.replaced(ids.len() + 1);
        }
        self.list_ids = ids;
    }
}

#[derive(IntoElement, Default)]
pub struct AllModerators;

impl AllModerators {
    pub fn new() -> Self {
        Self
    }

    fn filter_index(kind_filter: ModeratorKindFilter) -> i64 {
        ModeratorKindFilter::ALL.iter().position(|filter| *filter == kind_filter).unwrap_or(0) as i64
    }

    fn select_filter(index: i64, cx: &mut App) {
        let filter = usize::try_from(index).ok().and_then(|index| ModeratorKindFilter::ALL.get(index)).copied();
        let Some(filter) = filter else {
            return;
        };
        ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.set_kind_filter(filter, cx));
    }

    fn refresh(cx: &mut App) {
        ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_moderators(cx));
    }

    fn subtitle(total: i64, cx: &mut App) -> Option<SharedString> {
        if !ModerationServicesStore::did_load_moderators(cx) || total <= 0 {
            return None;
        }
        Some(RiseFormat::compact_count(total).into())
    }

    fn open(moderator: &ModeratorWithUserWireDTO, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        let id = moderator.target_user_id().or_else(|| moderator.user.as_ref().map(|user| user.tag.clone()));
        if let (Some(user_id), Some(nav_store)) = (id, nav_store) {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.push(RootRoute::ModeratorDetail { user_id }, NavigationStoreSettlingPush::Drop, cx);
            });
        }
    }

    fn handle_hold(
        id: &str,
        moderator: &ModeratorWithUserWireDTO,
        state: &Entity<AllModeratorsState>,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        match id {
            HOLD_PROFILE => {
                if let Some(nav_store) = nav_store {
                    let route = RootRoute::UserPage {
                        user_id: moderator.target_user_id(),
                        tag: moderator.user.as_ref().map(|user| user.tag.clone()),
                    };
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            }
            HOLD_RIGHTS => {
                let moderator = moderator.clone();
                state.update(cx, |state, cx| {
                    state.edit_permissions = moderator.permissions();
                    state.edit_rights_for = Some(moderator);
                    cx.notify();
                });
            }
            HOLD_REMOVE => {
                if let Some(target) = moderator.target_user_id() {
                    ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.remove_moderator(&target, None, cx);
                    });
                }
            }
            _ => {}
        }
    }

    fn rights_sheet(state: &Entity<AllModeratorsState>) -> NativeBottomSheetPresentationHost {
        let shown_reader = state.clone();
        let shown_writer = state.clone();
        let content_state = state.clone();
        NativeBottomSheetPresentationHost::new(
            page_id("rights-sheet"),
            Binding::new(
                move |cx| shown_reader.read(cx).edit_rights_for.is_some(),
                move |shown, _, cx| {
                    if !shown {
                        shown_writer.update(cx, |state, cx| {
                            state.edit_rights_for = None;
                            cx.notify();
                        });
                    }
                },
            ),
            move |_, cx| {
                let moderator = content_state.read(cx).edit_rights_for.clone();
                let Some((moderator, id)) = moderator.and_then(|moderator| {
                    let id = moderator.target_user_id()?;
                    Some((moderator, id))
                }) else {
                    return div().into_any_element();
                };
                let reader = content_state.clone();
                let writer = content_state.clone();
                let saved = content_state.clone();
                ModeratorRightsSheet::new(
                    id,
                    Binding::new(
                        move |cx| reader.read(cx).edit_permissions.clone(),
                        move |permissions, _, cx| {
                            writer.update(cx, |state, cx| {
                                state.edit_permissions = permissions;
                                cx.notify();
                            });
                        },
                    ),
                    moderator.resolved_kind() == Some(ModeratorKind::SuperModer),
                    move |cx| {
                        saved.update(cx, |state, cx| {
                            state.edit_rights_for = None;
                            cx.notify();
                        });
                    },
                )
                .into_any_element()
            },
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(true)
    }
}

impl RenderOnce for AllModerators {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(page_id("state"), cx, |_, cx| AllModeratorsState::new(cx));
        let filter_index = Self::filter_index(state.read(cx).roster.kind_filter);
        let subtitle = Self::subtitle(state.read(cx).roster.total, cx);
        let titles: Vec<SharedString> =
            ModeratorKindFilter::ALL.iter().map(|filter| tr(filter.title_key()).into()).collect();
        let pager_live_state = state.read(cx).pager_live_state.clone();
        let back_store = nav_store.clone();

        let shell = ListShellUi::new()
            .title(SharedString::from(tr("moderator_all_title")))
            .subtitle(subtitle)
            .shows_back_button(true)
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .on_back(move |_, cx| {
                if let Some(nav_store) = &back_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .below_header(
                ModerationSegmentedFilter::new(page_id("filter"), titles, filter_index, |index, _, cx| {
                    Self::select_filter(index, cx);
                })
                .live_drag_state(Some(pager_live_state.clone())),
            )
            .content(
                ModerationFilterPager::new(
                    page_id("pager"),
                    ModeratorKindFilter::ALL.len() as i64,
                    filter_index,
                    pager_live_state,
                    |index, _, cx| Self::select_filter(index, cx),
                    AllModeratorsList { state: state.clone(), nav_store },
                )
                .skeleton_rows(SKELETON_ROWS),
            );

        div()
            .size_full()
            .relative()
            .bg(theme.bg._100)
            .child(PageRefreshKeys::new(page_id("keys"), |_, cx| Self::refresh(cx), shell))
            .child(Self::rights_sheet(&state))
            .on_appear(page_id("task"), |_, cx| {
                ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_moderators_loaded(cx));
            })
    }
}

/// the selected filters page as its own element so the shells floating inset is in scope
#[derive(IntoElement)]
struct AllModeratorsList {
    state: Entity<AllModeratorsState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl RenderOnce for AllModeratorsList {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let roster = self.state.read(cx).roster.clone();
        let error_text = roster.error.clone().unwrap_or_else(|| tr("error_fetching_data"));
        let status = roster.status;
        let is_empty = roster.moderators.is_empty();

        let state = self.state;
        let nav_store = self.nav_store;
        let top_inset = list_shell_floating_top_inset(cx);
        SaiDataRender::new(status, move |_, cx| {
            AllModeratorsList::rows(roster, top_inset, state, nav_store, cx)
        })
        .id(page_id("data-render"))
        .is_empty(is_empty)
        .no_data_text(tr("moderator_all_empty"))
        .error_text(error_text)
        .need_pending(is_empty)
        .sai_data_render_loading(|_, cx| {
            scroll_view(page_id("skeleton"))
                .scroll_disabled(true)
                .shows_indicators(false)
                .list_shell_scroll_under_chrome(cx)
                .child(ModerationListSkeleton::new().rows(SKELETON_ROWS))
        })
    }
}

impl AllModeratorsList {
    fn rows(
        roster: AllModeratorsRoster,
        top_inset: f32,
        state: Entity<AllModeratorsState>,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> AnyElement {
        let in_sync = {
            let list_ids = &state.read(cx).list_ids;
            list_ids.len() == roster.moderators.len()
                && list_ids.iter().zip(roster.moderators.iter()).all(|(id, moderator)| *id == moderator.id())
        };
        if !in_sync {
            let ids: Vec<String> = roster.moderators.iter().map(|moderator| moderator.id()).collect();
            state.update(cx, |state, _| state.sync_list(ids));
        }

        let AllModeratorsRoster { moderators, cache_key, relative_id, has_more, .. } = roster;
        let row_state = state.clone();
        let item_count = moderators.len();

        state
            .read(cx)
            .list
            .refreshable(
                page_id("list"),
                |_, cx| {
                    AllModerators::refresh(cx);
                    Task::ready(())
                },
                move |index, window, cx| {
                    let Some(moderator) = moderators.get(index) else {
                        return div().h(px(ModerationMetrics::BOTTOM_INSET)).into_any_element();
                    };
                    let tap_moderator = moderator.clone();
                    let tap_store = nav_store.clone();
                    let hold_moderator = moderator.clone();
                    let hold_store = nav_store.clone();
                    let hold_state = row_state.clone();
                    let row = ModeratorRosterRow::new(
                        moderator,
                        window,
                        move |cx| AllModerators::open(&tap_moderator, &tap_store, cx),
                        move |id, cx| AllModerators::handle_hold(id, &hold_moderator, &hold_state, &hold_store, cx),
                        cx,
                    )
                    .sai_pagination_prefetch(
                        page_id(format!("row-{}", moderator.id())),
                        SaiPaginationPrefetchOptions::new(
                            index,
                            item_count,
                            cache_key.clone(),
                            relative_id.clone(),
                            has_more,
                        )
                        .threshold(PREFETCH_THRESHOLD),
                        |_, cx| {
                            ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_moderators(cx));
                        },
                    );
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .when(index == 0, |column| column.pt(px(top_inset)))
                        .child(row)
                        .when(index + 1 < item_count, |column| column.child(ModerationRowSeparator::new()))
                        .into_any_element()
                },
            )
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct ModeratorRosterRow {
    id: String,
    row_view: Entity<ModeratorRosterRowView>,
    hold_actions: Vec<HoldContextMenuAction>,
    on_tap: Rc<dyn Fn(&mut App)>,
    on_hold_action: Rc<dyn Fn(&str, &mut App)>,
}

impl ModeratorRosterRow {
    fn new(
        moderator: &ModeratorWithUserWireDTO,
        window: &mut Window,
        on_tap: impl Fn(&mut App) + 'static,
        on_hold_action: impl Fn(&str, &mut App) + 'static,
        cx: &mut App,
    ) -> Self {
        let id = moderator.id();
        let hold_actions = Self::hold_actions(moderator, cx);
        let row_view = window.use_keyed_state(page_id(format!("row-view-{id}")), cx, |_, cx| {
            ModeratorRosterRowView::new(moderator.clone(), cx)
        });
        row_view.update(cx, |view, cx| view.set_moderator(moderator, cx));
        Self { id, row_view, hold_actions, on_tap: Rc::new(on_tap), on_hold_action: Rc::new(on_hold_action) }
    }

    fn hold_actions(moderator: &ModeratorWithUserWireDTO, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let profile =
            HoldContextMenuAction::new(HOLD_PROFILE, tr("ctx_open_profile")).system_image("person.crop.circle");
        let target_user_id = moderator.target_user_id();
        if ModerationServicesStore::is_caller(target_user_id.as_deref(), cx) {
            return vec![profile];
        }
        vec![
            profile,
            HoldContextMenuAction::new(HOLD_RIGHTS, tr("moderator_edit_rights_button"))
                .system_image("slider.horizontal.3"),
            HoldContextMenuAction::new(HOLD_REMOVE, tr("moderator_remove_action"))
                .system_image("person.crop.circle.badge.minus")
                .destructive(true),
        ]
    }
}

impl RenderOnce for ModeratorRosterRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let lifted_background = rise_ui::theme(cx).bg._100;
        let actions = self.hold_actions;
        let on_tap = self.on_tap;
        let on_hold_action = self.on_hold_action;
        let row_view = self.row_view;

        HoldContextMenuContainer::new(
            page_id(format!("hold-{}", self.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                row_view
                    .clone()
                    .cached(StyleRefinement::default().w_full().h(px(ROW_HEIGHT)))
                    .replay_when_moved()
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .lifted_background(Some(lifted_background))
        .on_action(Rc::new(move |action: String, _: &mut Window, cx: &mut App| on_hold_action(&action, cx)))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| on_tap(cx)))
    }
}

/// roster row as its own cached view
/// each wheel step notifies the host page so every visible row laid out again per frame
/// now a row renders only when its moderator or the theme changes
struct ModeratorRosterRowView {
    moderator: ModeratorWithUserWireDTO,
    _theme: Subscription,
}

impl ModeratorRosterRowView {
    fn new(moderator: ModeratorWithUserWireDTO, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { moderator, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set_moderator(&mut self, moderator: &ModeratorWithUserWireDTO, cx: &mut Context<Self>) {
        if self.moderator != *moderator {
            self.moderator = moderator.clone();
            cx.notify();
        }
    }

    fn subtitle(moderator: &ModeratorWithUserWireDTO) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(user) = moderator.user.as_ref().filter(|user| !user.tag.is_empty()) {
            parts.push(format!("@{}", user.tag));
        }
        if !moderator.is_active() {
            parts.push(tr("moderator_inactive_badge"));
        } else if moderator.reports_resolved > 0 {
            parts.push(format!(
                "{} {}",
                RiseFormat::compact_count(moderator.reports_resolved),
                tr("moderator_metric_resolved")
            ));
        }
        parts.join(" \u{b7} ")
    }
}

impl Render for ModeratorRosterRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let moderator = &self.moderator;
        let user = moderator.user.as_ref();
        let name = user.map(|user| user.name.clone()).unwrap_or_else(|| tr("moderation_unknown_reporter"));
        let mut row = ModerationPersonRow::new(page_id(format!("person-{}", moderator.id())), name)
            .logo_source(user.and_then(|user| user.logo.clone()).map(SharedString::from))
            .user_id(moderator.target_user_id().map(SharedString::from))
            .subtitle(SharedString::from(Self::subtitle(moderator)))
            .is_official(user.is_some_and(|user| user.is_official))
            .is_premium(user.is_some_and(|user| user.is_premium));
        if moderator.resolved_kind() == Some(ModeratorKind::SuperModer) {
            row = row
                .trailing(ModerationBadge::new(tr(ModeratorKind::SuperModer.title_key())).color(theme.primary._100));
        }
        div().w_full().opacity(if moderator.is_active() { 1.0 } else { 0.5 }).child(row)
    }
}

struct ModeratorRightsSheetState {
    focus: FocusHandle,
    _observation: Subscription,
}

/// permission editor shared by the roster and the moderator screen so the two entry points cant drift
#[derive(IntoElement)]
pub struct ModeratorRightsSheet {
    user_id: String,
    permissions: Binding<ModeratorPermissions>,
    show_appeals_toggle: bool,
    on_saved: ModerationActionDone,
}

impl ModeratorRightsSheet {
    pub fn new(
        user_id: impl Into<String>,
        permissions: Binding<ModeratorPermissions>,
        show_appeals_toggle: bool,
        on_saved: impl Fn(&mut App) + 'static,
    ) -> Self {
        Self { user_id: user_id.into(), permissions, show_appeals_toggle, on_saved: Rc::new(on_saved) }
    }

    fn is_pending(cx: &mut App) -> bool {
        ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.update_pending(cx))
    }

    fn element_id(&self, name: &'static str) -> ElementId {
        moderation_child_id(&ElementId::Name(SharedString::from(format!("moderator-rights-{}", self.user_id))), name)
    }
}

impl RenderOnce for ModeratorRightsSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let state = window.use_keyed_state(self.element_id("state"), cx, |_, cx: &mut Context<ModeratorRightsSheetState>| {
            let actions = ModerationActionsStore::shared(cx);
            ModeratorRightsSheetState {
                focus: cx.focus_handle(),
                _observation: cx.observe(&actions, |_, _, cx| cx.notify()),
            }
        });
        let focus = state.read(cx).focus.clone();
        let appearing = focus.clone();
        let is_pending = Self::is_pending(cx);
        let dismiss_action = dismiss(cx);

        let scroll_id = self.element_id("scroll");
        let editor_id = self.element_id("editor");
        let save_id = self.element_id("save");
        let keys_id = self.element_id("keys");
        let appear_id = self.element_id("focus-appear");
        let user_id = self.user_id;
        let permissions = self.permissions;
        let on_saved = self.on_saved;
        let editor_permissions = permissions.clone();
        let save: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            let permissions_json = permissions.get(cx).to_json_string();
            ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.update_rights(&user_id, &permissions_json, Some(on_saved.clone()), cx);
            });
        });
        let key_save = save.clone();

        div()
            .id(keys_id)
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key != RETURN_KEY || ModeratorRightsSheet::is_pending(cx) {
                    return;
                }
                cx.stop_propagation();
                key_save(window, cx);
            })
            .child(
                SettingsWrapperUi::new(tr("moderator_edit_rights_title"), move |window, cx| {
                    dismiss_action.call_as_function(window, cx);
                })
                .content(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .bg(background)
                        .child(div().w_full().flex_1().min_h(px(0.0)).child(ModeratorRightsScroll {
                            scroll_id,
                            editor_id,
                            permissions: editor_permissions,
                            show_appeals_toggle: self.show_appeals_toggle,
                        }))
                        .child(ModeratorRightsSaveBar { id: save_id, is_pending, on_save: save }),
                ),
            )
            .on_appear(appear_id, move |window, cx| window.focus(&appearing, cx))
    }
}

#[derive(IntoElement)]
struct ModeratorRightsScroll {
    scroll_id: ElementId,
    editor_id: ElementId,
    permissions: Binding<ModeratorPermissions>,
    show_appeals_toggle: bool,
}

impl RenderOnce for ModeratorRightsScroll {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let secondary = rise_ui::theme(cx).text.secondary;
        scroll_view(self.scroll_id).list_shell_scroll_under_chrome(cx).child(
            vstack(ModerationMetrics::SECTION_SPACING)
                .w_full()
                .pt(px(12.0))
                .pb(px(16.0))
                .child(
                    div().w_full().px(px(24.0)).child(
                        MainText::new(tr("moderator_edit_rights_subtitle"))
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(secondary),
                    ),
                )
                .child(
                    ModeratorPermissionsEditor::new(self.editor_id, self.permissions)
                        .show_appeals_toggle(self.show_appeals_toggle),
                ),
        )
    }
}

#[derive(IntoElement)]
struct ModeratorRightsSaveBar {
    id: ElementId,
    is_pending: bool,
    on_save: ButtonAction,
}

impl RenderOnce for ModeratorRightsSaveBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let fill = theme.primary._100;
        let content = theme.content_on(fill);
        let bottom = list_shell_safe_area_insets(cx).bottom.max(10.0);
        let on_save = self.on_save;
        div()
            .w_full()
            .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
            .pt(px(10.0))
            .pb(px(bottom))
            .bg(theme.bg._100)
            .child(
                ButtonUi::new(self.id)
                    .disabled(self.is_pending)
                    .is_pending(self.is_pending)
                    .background_color(fill)
                    .loader_color(content)
                    .on_press(move |window, cx| on_save(window, cx))
                    .child(
                        MainText::new(tr("moderator_edit_rights_confirm"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(content),
                    ),
            )
    }
}

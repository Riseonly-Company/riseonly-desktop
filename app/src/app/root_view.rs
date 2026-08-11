use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyView, App, ClickEvent, Context, Entity, ExternalPaths, FocusHandle, Focusable, IntoElement,
    Pixels, Point, Render, Subscription, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_navigation::{NavigationStore, PaneLayout, PanePolicy, PaneSlot, RootRoute, RootTab};
use rise_platform::window_chrome::current_window_corner;
use rise_widgets::{
    BlockUi, ContextMenuEntry, ContextMenuEvent, ContextMenuItem, ContextMenuState, OverlayAnchor,
    OverlayLayer, OverlayScrim, OverlaySide, OverlayStack, glass_host, place,
};

use crate::app::router::root_route_destination::destination;
use crate::app::shell::command_palette::{
    CommandPalette, CommandPaletteEvent, CommandTarget, PaletteCommand, catalogue,
};
use crate::app::shell::file_drop::{self, DropOutcome, DropRejection};
use crate::app::shell::rail::{Rail, RailFolder, RailHandlers, RailState, RailTarget};
use crate::app::shell::shell_actions::{
    self, CloseWindow, DismissOverlay, GoBack, GoForward, KEY_CONTEXT, NewWindow, SelectSlot1,
    SelectSlot2, SelectSlot3, SelectSlot4, SelectSlot5, SelectSlot6, SelectSlot7, SelectSlot8,
    SelectSlot9, ShortcutTarget, ToggleCommandPalette,
};
use crate::app::shell::window_presenter::WindowPresenter;
use crate::core::active_screens::{ActiveScreens, LeaseColumn, ScreenIdentity};
use crate::core::session_activity::{ActivitySignal, SessionActivity};
use crate::modules::auth::pages::sign::sign_in::sign_in::SignInEvent;
use crate::modules::auth::stores::auth_services::try_auth_services;
use crate::modules::onboarding::pages::onboarding::onboarding::OnboardingEvent;

const PALETTE_OVERLAY: &str = "shell.command_palette";
const RAIL_MENU_OVERLAY: &str = "shell.rail_menu";

fn inset_corner_radius(outer_radius: Pixels, inset: Pixels) -> Pixels {
    (outer_radius - inset).max(gpui::px(0.0))
}

pub struct RootView {
    navigation: NavigationStore,
    folders: Vec<RailFolder>,
    selected_folder: Option<String>,
    screens: ActiveScreens,
    activity: SessionActivity,
    overlays: OverlayStack,
    palette: Option<Entity<CommandPalette>>,
    menu: Option<Entity<ContextMenuState>>,
    menu_target: Option<RailTarget>,
    focus_handle: FocusHandle,
    rail_corner_radius: Pixels,
    views: HashMap<String, AnyView>,
    is_authenticated: bool,
    screens_dirty: bool,
    _subscriptions: Vec<Subscription>,
}

impl RootView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = rise_ui::theme(cx as &App);
        let policy = PanePolicy::from_shell_metrics(theme.shell);
        let rail_corner_radius = current_window_corner()
            .map(|corner| {
                inset_corner_radius(gpui::px(corner.radius), theme.shell.rail_outer_inset)
            })
            .unwrap_or(theme.shell.block_radius);
        let layout = policy.layout_for(window.viewport_size().width);
        let mut navigation = NavigationStore::new(policy, layout);

        let is_authenticated = try_auth_services(cx as &App)
            .is_some_and(|services| services.read(cx).is_authenticated());
        navigation.push(if is_authenticated {
            RootRoute::Tab(RootTab::Feed)
        } else {
            RootRoute::Onboarding
        });

        let mut subscriptions = vec![
            cx.observe_window_bounds(window, |view, window, cx| {
                if view.navigation.window_resized(window.viewport_size().width) {
                    view.screens_dirty = true;
                    cx.notify();
                }
            }),
            cx.observe_window_activation(window, |view, window, cx| {
                let mut changed = view
                    .activity
                    .apply(ActivitySignal::WindowFocus(window.is_window_active()));
                changed |= view
                    .activity
                    .apply(ActivitySignal::WindowCount(cx.windows().len()));

                if changed {
                    cx.notify();
                }
            }),
        ];

        if let Some(services) = try_auth_services(cx as &App).cloned() {
            subscriptions.push(cx.observe(&services, |view, services, cx| {
                let authenticated = services.read(cx).is_authenticated();
                view.authentication_changed(authenticated, cx);
            }));
        }

        Self {
            navigation,
            folders: Vec::new(),
            selected_folder: None,
            screens: ActiveScreens::new(),
            activity: SessionActivity::new(),
            overlays: OverlayStack::new(),
            palette: None,
            menu: None,
            menu_target: None,
            focus_handle: cx.focus_handle(),
            rail_corner_radius,
            views: HashMap::new(),
            is_authenticated,
            screens_dirty: true,
            _subscriptions: subscriptions,
        }
    }

    #[cfg(test)]
    pub fn focused_screen_key(&self) -> Option<String> {
        self.navigation.focused_route().map(RootRoute::resource_key)
    }

    fn pop_route(&mut self, cx: &mut Context<Self>) {
        if self.navigation.back().is_some() {
            self.screens_dirty = true;
            cx.notify();
        }
    }

    pub fn open_route(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        self.navigation.push(route);
        self.screens_dirty = true;
        cx.notify();
    }

    fn selected_tab(&self) -> RootTab {
        self.navigation.tab()
    }

    fn shows_rail(&self) -> bool {
        self.navigation
            .active_routes()
            .iter()
            .all(|route| route.requires_authentication())
    }

    fn primary_is_list(&self) -> bool {
        match self.navigation.column(0) {
            Some((_, RootRoute::Tab(tab))) => tab.has_sidebar_list(),
            _ => true,
        }
    }

    fn drawn_pane_count(&self) -> usize {
        if !self.shows_rail() {
            return 1;
        }

        let columns = self.navigation.layout().column_count();
        if self.primary_is_list() {
            return columns;
        }

        (1..columns)
            .filter(|column| self.navigation.column(*column).is_some())
            .count()
            + 1
    }

    fn activate(&mut self, target: RailTarget, cx: &mut Context<Self>) {
        match target {
            RailTarget::Section(tab) => {
                self.selected_folder = None;
                self.navigation.select_tab(tab);
            }
            RailTarget::Folder(id) => {
                self.selected_folder = Some(id);
                self.navigation.select_tab(RootTab::Chats);
            }
            RailTarget::Settings => self.navigation.push(RootRoute::Settings),
        }

        self.screens_dirty = true;
        cx.notify();
    }

    fn select_slot(&mut self, slot: u8, cx: &mut Context<Self>) {
        let Some(target) = shell_actions::shortcut_target(slot, self.folders.len()) else {
            return;
        };

        let target = match target {
            ShortcutTarget::Section(tab) => RailTarget::Section(tab),
            ShortcutTarget::Folder(index) => match self.folders.get(index) {
                Some(folder) => RailTarget::Folder(folder.id.clone()),
                None => return,
            },
        };

        self.activate(target, cx);
    }

    fn go_back(&mut self, _: &GoBack, _window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation.back().is_some() {
            self.screens_dirty = true;
            cx.notify();
        }
    }

    fn go_forward(&mut self, _: &GoForward, _window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation.forward().is_some() {
            self.screens_dirty = true;
            cx.notify();
        }
    }

    fn dismiss_overlay(&mut self, _: &DismissOverlay, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(top) = self.overlays.dismiss_top() {
            self.close_overlay(top.as_ref(), cx);
            self.take_focus(window, cx);
            return;
        }

        if self.navigation.back().is_some() {
            self.screens_dirty = true;
            cx.notify();
        }
    }

    fn take_focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    fn close_overlay(&mut self, id: &str, cx: &mut Context<Self>) {
        self.overlays.dismiss(id);
        match id {
            PALETTE_OVERLAY => self.palette = None,
            RAIL_MENU_OVERLAY => {
                self.menu = None;
                self.menu_target = None;
            }
            _ => {}
        }

        self.screens_dirty = true;
        cx.notify();
    }

    fn open_rail_menu(
        &mut self,
        target: RailTarget,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = self.menu.clone().unwrap_or_else(|| {
            let menu = cx.new(|cx| ContextMenuState::new("shell.rail_menu", cx));

            cx.subscribe(&menu, |view, _, event, cx| match event {
                ContextMenuEvent::Activated(id) => {
                    let target = view.menu_target.clone();
                    view.close_overlay(RAIL_MENU_OVERLAY, cx);
                    if id.as_ref() == RAIL_MENU_OPEN
                        && let Some(target) = target
                    {
                        view.activate(target, cx);
                    }
                }
                ContextMenuEvent::Dismissed => view.close_overlay(RAIL_MENU_OVERLAY, cx),
            })
            .detach();

            menu
        });

        menu.update(cx, |menu, cx| {
            menu.open(rail_menu_entries(), OverlayAnchor::Point(position), cx);
            menu.focus(window, cx);
        });

        self.menu_target = Some(target);
        self.menu = Some(menu);
        self.overlays.open(RAIL_MENU_OVERLAY);
        self.screens_dirty = true;
        cx.notify();
    }

    fn toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() {
            self.close_overlay(PALETTE_OVERLAY, cx);
            return;
        }

        let commands = catalogue(&self.folders);
        let palette = cx.new(|cx| CommandPalette::new(commands, cx));

        cx.subscribe(&palette, |view, _, event, cx| match event {
            CommandPaletteEvent::Dismissed => view.close_overlay(PALETTE_OVERLAY, cx),
            CommandPaletteEvent::Activated(command) => {
                let command = command.clone();
                view.close_overlay(PALETTE_OVERLAY, cx);
                view.run_command(command, cx);
            }
        })
        .detach();

        let query_focus = palette.read(cx).query_focus_handle(cx);
        window.focus(&query_focus, cx);

        self.palette = Some(palette);
        self.overlays.open(PALETTE_OVERLAY);
        self.screens_dirty = true;
        cx.notify();
    }

    fn run_command(&mut self, command: PaletteCommand, cx: &mut Context<Self>) {
        match command.target {
            CommandTarget::Section(tab) => self.activate(RailTarget::Section(tab), cx),
            CommandTarget::Folder(id) => self.activate(RailTarget::Folder(id), cx),
            CommandTarget::Route(route) => self.open_route(route, cx),
        }
    }

    fn new_window(&mut self, _: &NewWindow, _window: &mut Window, cx: &mut Context<Self>) {
        WindowPresenter::open_shell(cx);
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    fn files_dropped(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) -> DropOutcome {
        let target = self.navigation.focused_route().cloned();
        let outcome = file_drop::evaluate(paths.paths(), target.as_ref(), |path| path.is_file());

        match &outcome {
            DropOutcome::Accepted(accepted) => tracing::info!(
                target: "riseonly",
                "{} file(s) dropped on {}; no consumer until the composer lands",
                accepted.paths.len(),
                accepted.screen_id
            ),
            DropOutcome::Rejected(DropRejection::NoAcceptor) => tracing::debug!(
                target: "riseonly",
                "nothing on screen takes files"
            ),
            DropOutcome::Rejected(reason) => tracing::info!(
                target: "riseonly",
                "drop refused: {reason:?}"
            ),
        }

        cx.notify();
        outcome
    }

    // Must run before the element tree is built: gpui flushes entity effects between frames.
    fn sync_screens(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let live: std::collections::HashSet<String> = self
            .navigation
            .all_routes()
            .iter()
            .map(|route| route.resource_key())
            .collect();
        self.views.retain(|key, _| live.contains(key));

        let mounted: Vec<RootRoute> = self
            .navigation
            .active_routes()
            .iter()
            .map(|route| (*route).clone())
            .collect();
        for route in mounted {
            self.mount(&route, window, cx);
        }

        self.sync_active_screens();
    }

    fn sync_active_screens(&mut self) {
        if !self.screens_dirty {
            return;
        }
        self.screens_dirty = false;

        let mut visible: Vec<(ScreenIdentity, LeaseColumn)> = self
            .navigation
            .active_panes()
            .into_iter()
            .map(|(slot, route)| (ScreenIdentity::for_route(route), LeaseColumn::from(slot)))
            .collect();

        if self.palette.is_some() {
            visible.push((
                ScreenIdentity::new("CommandPalette", "CommandPalette"),
                LeaseColumn::Overlay,
            ));
        }

        self.screens.sync_visible(&visible);
    }

    fn mount(&mut self, route: &RootRoute, window: &mut Window, cx: &mut Context<Self>) {
        let key = route.resource_key();
        if self.views.contains_key(&key) {
            return;
        }

        let can_dismiss = self.navigation.can_go_back();
        let view = destination(route, can_dismiss, window, cx);

        if *route == RootRoute::Onboarding
            && let Ok(onboarding) = view.clone().downcast::<crate::modules::onboarding::pages::onboarding::onboarding::Onboarding>()
        {
            cx.subscribe(&onboarding, |view, _, event, cx| match event {
                OnboardingEvent::Finished => view.open_route(RootRoute::SignIn, cx),
            })
            .detach();
        }

        if let Ok(sign_in) = view
            .clone()
            .downcast::<crate::modules::auth::pages::sign::sign_in::sign_in::SignIn>()
        {
            cx.subscribe(&sign_in, |view, _, event, cx| match event {
                SignInEvent::Dismissed => view.pop_route(cx),
            })
            .detach();
        }

        self.views.insert(key, view);
    }

    fn replace_root(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        self.navigation.reset_to_root();
        if !matches!(route, RootRoute::Tab(_)) {
            self.navigation.push(route);
        }
        self.views.clear();
        self.screens_dirty = true;
        cx.notify();
    }

    fn authentication_changed(&mut self, authenticated: bool, cx: &mut Context<Self>) {
        if self.is_authenticated == authenticated {
            return;
        }
        self.is_authenticated = authenticated;

        self.replace_root(
            if authenticated {
                RootRoute::Tab(RootTab::Feed)
            } else {
                RootRoute::Onboarding
            },
            cx,
        );
    }

    fn pane(&mut self, column: usize, cx: &mut Context<Self>) -> gpui::Div {
        let theme = rise_ui::theme(cx as &App).clone();
        let layout = self.navigation.layout();
        let entry = self.navigation.column(column);

        let entry = entry.map(|(slot, route)| (slot, route.clone()));
        let body = match entry
            .as_ref()
            .and_then(|(_, route)| self.views.get(&route.resource_key()))
        {
            Some(view) => view.clone(),
            None => self.empty_pane(cx),
        };

        let slot = entry.map(|(slot, _)| slot);
        let is_sidebar = layout != PaneLayout::Stacked
            && slot == Some(PaneSlot::Primary)
            && self.primary_is_list();
        let is_aside = layout == PaneLayout::ThreeColumn && slot == Some(PaneSlot::Tertiary);

        match (is_sidebar, is_aside) {
            (true, _) => BlockUi::column(&theme)
                .w(theme.shell.sidebar_width)
                .flex_shrink_0()
                .child(body),
            (_, true) => BlockUi::column(&theme)
                .w(theme.shell.aside_width)
                .flex_shrink_0()
                .child(body),
            _ => BlockUi::column(&theme).flex_1().min_w_0().child(body),
        }
    }

    fn full_window_pane(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let route = self.navigation.focused_route().cloned();
        let body = match route
            .as_ref()
            .and_then(|route| self.views.get(&route.resource_key()))
        {
            Some(view) => view.clone(),
            None => self.empty_pane(cx),
        };

        let theme = rise_ui::theme(cx as &App).clone();
        BlockUi::column(&theme).flex_1().min_w_0().child(body)
    }

    fn empty_pane(&self, cx: &mut Context<Self>) -> gpui::AnyView {
        cx.new(|_| EmptyPane).into()
    }

    fn rail_handlers(&self) -> RailHandlers<Self> {
        RailHandlers {
            activate: Rc::new(|view: &mut Self, target, _window, cx| view.activate(target, cx)),
            context_menu: Rc::new(|view: &mut Self, target, position, window, cx| {
                view.open_rail_menu(target, position, window, cx)
            }),
        }
    }
}

impl Focusable for RootView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_screens(window, cx);

        // With nothing focused gpui dispatches from the window root, outside the shell key context.
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }

        let theme = rise_ui::theme(cx as &App).clone();
        let shows_rail = self.shows_rail();
        let selected = self.selected_tab();
        let folders = self.folders.clone();
        let selected_folder = self.selected_folder.clone();
        let handlers = self.rail_handlers();
        let rail_corner_radius = self.rail_corner_radius;

        let mut plate = BlockUi::window(&theme)
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::dismiss_overlay))
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(|view, _: &SelectSlot1, _, cx| view.select_slot(1, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot2, _, cx| view.select_slot(2, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot3, _, cx| view.select_slot(3, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot4, _, cx| view.select_slot(4, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot5, _, cx| view.select_slot(5, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot6, _, cx| view.select_slot(6, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot7, _, cx| view.select_slot(7, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot8, _, cx| view.select_slot(8, cx)))
            .on_action(cx.listener(|view, _: &SelectSlot9, _, cx| view.select_slot(9, cx)))
            .on_drop(cx.listener(|view, paths: &ExternalPaths, _, cx| {
                view.files_dropped(paths, cx);
            }));

        if shows_rail {
            plate = plate.child(
                div()
                    .h_full()
                    .p(theme.shell.rail_outer_inset)
                    .flex_shrink_0()
                    .child(Rail::render(
                        RailState {
                            selected,
                            folders: &folders,
                            selected_folder: selected_folder.as_deref(),
                        },
                        rail_corner_radius,
                        &handlers,
                        cx,
                    )),
            );

            let drawn = self.drawn_pane_count();
            for column in 0..drawn {
                plate = plate.child(self.pane(column, cx));
                if column + 1 < drawn {
                    plate = plate.child(BlockUi::divider(&theme));
                }
            }
        } else {
            plate = plate.child(self.full_window_pane(cx));
        }

        if let Some(palette) = self.palette.clone() {
            plate = plate.child(
                div()
                    .id(PALETTE_OVERLAY)
                    .absolute()
                    .inset_0()
                    .bg(theme.material.scrim)
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(theme.shell.palette_top_inset)
                    .on_click(cx.listener(|view, _: &ClickEvent, window, cx| {
                        view.close_overlay(PALETTE_OVERLAY, cx);
                        view.take_focus(window, cx);
                    }))
                    .child(palette),
            );
        }

        if let Some(menu) = self.menu.clone() {
            let placement = place(
                menu.read(cx).anchor(),
                menu.read(cx).asked_size(&theme),
                window.viewport_size(),
                theme.shell.overlay_gap,
                theme.shell.overlay_margin,
                OverlaySide::Below,
            );

            plate = plate.child(
                OverlayLayer::new(placement)
                    .scrim(OverlayScrim::Invisible)
                    .child(menu)
                    .on_dismiss({
                        let view = cx.entity();
                        move |_window, cx| {
                            view.update(cx, |view, cx| view.close_overlay(RAIL_MENU_OVERLAY, cx));
                        }
                    })
                    .render(cx),
            );
        }

        plate.child(glass_host())
    }
}

const RAIL_MENU_OPEN: &str = "open";

fn rail_menu_entries() -> Vec<ContextMenuEntry> {
    vec![
        ContextMenuItem::new(RAIL_MENU_OPEN, "ctx_open")
            .icon("arrow.up.right")
            .into(),
    ]
}

const EMPTY_PANE_KEY: &str = "chat_preview_empty";

struct EmptyPane;

impl Render for EmptyPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App);

        // No fill: an empty column is a hole in the plate, not a card laid on
        // top of one, and a card at the plate's edge squares its corner.
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                rise_ui::MainText::body(theme, rise_ui::TextTone::Secondary)
                    .child(tr(EMPTY_PANE_KEY)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, WindowHandle, px};
    use rise_theme::AppTheme;

    #[test]
    fn the_inset_rail_keeps_its_corner_parallel_to_the_window() {
        assert_eq!(inset_corner_radius(px(26.5), px(5.0)), px(21.5));
        assert_eq!(inset_corner_radius(px(4.0), px(5.0)), px(0.0));
    }

    fn open(cx: &mut TestAppContext) -> WindowHandle<RootView> {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
            shell_actions::install_key_bindings(cx);
        });
        cx.add_window(RootView::new)
    }

    fn with<R>(
        handle: &WindowHandle<RootView>,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut RootView, &mut Window, &mut Context<RootView>) -> R,
    ) -> R {
        handle.update(cx, f).expect("window is open")
    }

    fn drawn(handle: &WindowHandle<RootView>, cx: &mut TestAppContext) -> Vec<String> {
        with(handle, cx, |view, _, _| {
            view.navigation
                .active_routes()
                .iter()
                .map(|route| route.resource_key())
                .collect()
        })
    }

    #[gpui::test]
    fn the_rail_is_hidden_until_the_user_is_signed_in(cx: &mut TestAppContext) {
        let shell = open(cx);
        assert!(!with(&shell, cx, |view, _, _| view.shows_rail()));

        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.screens_dirty = true;
            cx.notify();
        });
        assert!(with(&shell, cx, |view, _, _| view.shows_rail()));
    }

    #[gpui::test]
    fn a_section_shortcut_selects_the_section_the_rail_draws(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| view.select_slot(3, cx));

        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_tab()),
            RootTab::Chats
        );
        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_folder.clone()),
            None
        );
    }

    #[gpui::test]
    fn a_folder_shortcut_with_no_folder_behind_it_does_nothing(cx: &mut TestAppContext) {
        let shell = open(cx);
        let first_folder = u8::try_from(RootTab::ALL.len() + 1).unwrap();
        with(&shell, cx, |view, _, cx| view.select_slot(first_folder, cx));

        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_tab()),
            RootTab::Feed
        );
    }

    #[gpui::test]
    fn a_folder_shortcut_selects_the_folder_and_the_chats_section(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.folders = vec![RailFolder {
                id: "work".into(),
                title: "Work".into(),
                unread: 2,
            }];
            let first_folder = u8::try_from(RootTab::ALL.len() + 1).unwrap();
            view.select_slot(first_folder, cx);
        });

        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_tab()),
            RootTab::Chats
        );
        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_folder.clone()),
            Some("work".to_owned())
        );
    }

    #[gpui::test]
    fn escape_closes_the_palette_before_it_closes_a_screen(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, window, cx| {
            view.navigation.back();
            view.navigation.push(RootRoute::Settings);
            view.toggle_command_palette(&ToggleCommandPalette, window, cx);
        });
        assert!(with(&shell, cx, |view, _, _| view.palette.is_some()));

        with(&shell, cx, |view, window, cx| {
            view.dismiss_overlay(&DismissOverlay, window, cx)
        });
        assert!(with(&shell, cx, |view, _, _| view.palette.is_none()));
        assert!(drawn(&shell, cx).contains(&"Settings".to_owned()));

        with(&shell, cx, |view, window, cx| {
            view.dismiss_overlay(&DismissOverlay, window, cx)
        });
        assert!(!drawn(&shell, cx).contains(&"Settings".to_owned()));
    }

    #[gpui::test]
    fn the_palette_is_a_lease_above_every_column(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, window, cx| {
            view.navigation.back();
            view.toggle_command_palette(&ToggleCommandPalette, window, cx);
            view.sync_active_screens();

            assert_eq!(
                view.screens.focused().map(ScreenIdentity::screen_id),
                Some("CommandPalette"),
                "an open palette owns the refresh priority, not the feed under it"
            );
        });
    }

    #[gpui::test]
    fn resizing_the_window_never_re_registers_the_screens_it_kept(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.open_route(
                RootRoute::Chat {
                    chat_id: "c1".into(),
                },
                cx,
            );
            view.open_route(
                RootRoute::UserProfile {
                    user_id: "u1".into(),
                },
                cx,
            );
            view.navigation.window_resized(px(2560.0));
            view.screens_dirty = true;
            view.sync_active_screens();

            let key = crate::core::active_screens::CacheKey::from("Chat:c1");
            let owner = ScreenIdentity::new("Chat", "Chat:c1");
            assert!(view.screens.claim_refresh(&owner, &key).is_some());

            view.navigation.window_resized(px(400.0));
            view.screens_dirty = true;
            view.sync_active_screens();
            view.navigation.window_resized(px(2560.0));
            view.screens_dirty = true;
            view.sync_active_screens();

            assert!(
                view.screens.claim_refresh(&owner, &key).is_none(),
                "rearranging the window must not re-fetch what it re-laid-out"
            );
        });
    }

    #[gpui::test]
    fn every_column_the_layout_affords_is_drawn_at_every_width(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.navigation.select_tab(RootTab::Chats);
            view.open_route(
                RootRoute::Chat {
                    chat_id: "c1".into(),
                },
                cx,
            );

            for width in [px(400.0), px(780.0), px(900.0), px(1080.0), px(2560.0)] {
                view.navigation.window_resized(width);
                assert_eq!(
                    view.drawn_pane_count(),
                    view.navigation.layout().column_count(),
                    "at {width:?} the frame drops a column the layout affords"
                );
                assert!(
                    view.navigation.column(0).is_some(),
                    "the sidebar column is empty at {width:?}"
                );
            }
        });
    }

    #[gpui::test]
    fn a_column_with_nothing_open_is_still_drawn_as_an_empty_state(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.navigation.select_tab(RootTab::Chats);
            view.navigation.window_resized(px(2560.0));
            cx.notify();

            assert_eq!(view.drawn_pane_count(), 3);
            assert!(
                view.navigation.column(1).is_none() && view.navigation.column(2).is_none(),
                "nothing is open beside the list, which is the case being drawn"
            );
        });
    }

    #[gpui::test]
    fn a_section_that_is_its_own_content_fills_the_window(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.navigation.window_resized(px(2560.0));
            cx.notify();

            assert_eq!(view.selected_tab(), RootTab::Feed);
            assert!(!view.primary_is_list());
            assert_eq!(
                view.drawn_pane_count(),
                1,
                "a feed in a 320pt sidebar with two empty states beside it is not a feed"
            );
        });
    }

    #[gpui::test]
    fn a_content_section_still_draws_the_columns_that_hold_something(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.navigation.window_resized(px(2560.0));
            view.open_route(
                RootRoute::UserProfile {
                    user_id: "u1".into(),
                },
                cx,
            );

            assert_eq!(view.drawn_pane_count(), 2);
        });
    }

    #[gpui::test]
    fn a_pre_auth_screen_owns_the_whole_window_however_wide_it_is(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, _| {
            view.navigation.window_resized(px(2560.0));

            assert!(!view.shows_rail());
            assert_eq!(
                view.drawn_pane_count(),
                1,
                "sign-in in a 320px sidebar with two empty states beside it is not a screen"
            );
        });
    }

    #[gpui::test]
    fn losing_focus_never_drops_the_socket_or_a_pending_request(cx: &mut TestAppContext) {
        use crate::core::session_activity::{PendingRequestPolicy, SocketPolicy};

        let shell = open(cx);
        with(&shell, cx, |view, _, _| {
            view.activity.apply(ActivitySignal::WindowFocus(false));
            let policy = view.activity.policy();

            assert_eq!(policy.socket, SocketPolicy::KeepConnected);
            assert_eq!(policy.pending_requests, PendingRequestPolicy::Keep);
        });
    }

    #[gpui::test]
    fn an_app_with_its_last_window_closed_is_away_rather_than_present(cx: &mut TestAppContext) {
        use crate::core::session_activity::PresenceState;

        let shell = open(cx);
        with(&shell, cx, |view, _, _| {
            view.activity.apply(ActivitySignal::WindowCount(0));
            assert_eq!(view.activity.presence(), PresenceState::Away);
            assert!(!view.activity.is_user_present());
        });
    }

    #[gpui::test]
    fn the_keyboard_works_on_a_window_nobody_has_clicked_in(cx: &mut TestAppContext) {
        let shell = open(cx);
        let window: gpui::AnyWindowHandle = shell.into();

        let mut visual = gpui::VisualTestContext::from_window(window, cx);

        visual.simulate_keystrokes("cmd-k");
        assert!(
            with(&shell, cx, |view, _, _| view.palette.is_some()),
            "with nothing focused gpui dispatches from the window root, and the shell's key \
             context is not there — every shortcut would be dead until the first click"
        );

        visual.simulate_keystrokes("escape");
        assert!(with(&shell, cx, |view, _, _| view.palette.is_none()));

        visual.simulate_keystrokes("cmd-3");
        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_tab()),
            RootTab::Chats
        );
    }

    #[gpui::test]
    fn a_screen_is_built_once_and_survives_the_frames_after_it(cx: &mut TestAppContext) {
        let shell = open(cx);
        cx.run_until_parked();

        let first = with(&shell, cx, |view, _, _| {
            view.views.get("Onboarding").map(gpui::AnyView::entity_id)
        });
        assert!(first.is_some(), "the pre-auth screen is mounted");

        cx.run_until_parked();
        let second = with(&shell, cx, |view, _, _| {
            view.views.get("Onboarding").map(gpui::AnyView::entity_id)
        });

        assert_eq!(
            first, second,
            "a screen rebuilt every frame loses its caret, its scroll position \
             and the step of the form it was on"
        );
    }

    #[gpui::test]
    fn a_screen_that_is_closed_is_dropped_with_its_state(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.open_route(
                RootRoute::Chat {
                    chat_id: "c1".into(),
                },
                cx,
            );
        });
        cx.run_until_parked();
        assert!(with(&shell, cx, |view, _, _| view
            .views
            .contains_key("Chat:c1")));

        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.screens_dirty = true;
            cx.notify();
        });
        cx.run_until_parked();

        assert!(
            !with(&shell, cx, |view, _, _| view.views.contains_key("Chat:c1")),
            "a closed screen must not keep its subscriptions and its cache alive"
        );
    }

    #[gpui::test]
    fn narrowing_the_window_hides_a_column_without_dropping_what_is_in_it(cx: &mut TestAppContext) {
        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            view.open_route(
                RootRoute::UserProfile {
                    user_id: "u1".into(),
                },
                cx,
            );
            view.navigation.window_resized(px(2560.0));
            view.screens_dirty = true;
            cx.notify();
        });
        cx.run_until_parked();
        let mounted = with(&shell, cx, |view, _, _| {
            view.views
                .get("UserProfile:u1")
                .map(gpui::AnyView::entity_id)
        });
        assert!(mounted.is_some());

        with(&shell, cx, |view, _, cx| {
            view.navigation.window_resized(px(400.0));
            view.screens_dirty = true;
            cx.notify();
        });
        cx.run_until_parked();

        assert_eq!(
            with(&shell, cx, |view, _, _| view
                .views
                .get("UserProfile:u1")
                .map(gpui::AnyView::entity_id)),
            mounted,
            "a hidden column is not a closed screen; rebuilding it on widening \
             would lose its scroll position"
        );
    }

    #[gpui::test]
    fn escape_reaches_the_shell_from_inside_the_palette_field(cx: &mut TestAppContext) {
        let shell = open(cx);
        let window: gpui::AnyWindowHandle = shell.into();

        cx.simulate_keystrokes(window, "cmd-k");
        cx.simulate_keystrokes(window, "a");
        assert_eq!(
            with(&shell, cx, |view, _, cx| view
                .palette
                .as_ref()
                .map(|p| p.read(cx).query_text(cx))),
            Some("a".to_owned()),
            "the field has the caret, which is the state the dismissal has to survive"
        );

        cx.simulate_keystrokes(window, "escape");
        assert!(
            with(&shell, cx, |view, _, _| view.palette.is_none()),
            "the field is rendered as its entity, so its key context has the shell above it"
        );
    }

    #[gpui::test]
    fn a_section_shortcut_still_fires_while_the_palette_holds_the_caret(cx: &mut TestAppContext) {
        let shell = open(cx);
        let window: gpui::AnyWindowHandle = shell.into();

        cx.simulate_keystrokes(window, "cmd-k");
        cx.simulate_keystrokes(window, "cmd-2");
        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_tab()),
            RootTab::Search,
            "a chord the field does not bind must reach the frame around it"
        );
    }

    #[gpui::test]
    fn clicking_a_palette_entry_runs_it_instead_of_dismissing_the_palette(cx: &mut TestAppContext) {
        let shell = open(cx);
        let handle: gpui::AnyWindowHandle = shell.into();
        let mut visual = gpui::VisualTestContext::from_window(handle, cx);

        visual.simulate_keystrokes("cmd-k");
        visual.simulate_keystrokes("chats");
        visual.run_until_parked();

        assert_eq!(
            with(&shell, cx, |view, _, cx| view.palette.as_ref().and_then(
                |p| p.read(cx).highlighted_command().map(|c| c.id.to_string())
            )),
            Some("section.Chats".to_owned()),
            "the query must have narrowed to the row the click is aimed at"
        );

        let point = with(&shell, cx, |view, window, cx| {
            let theme = rise_ui::theme(cx as &App);
            let x = window.viewport_size().width / 2.0;
            let y = theme.shell.palette_top_inset
                + theme.spacing._600
                + theme.input.height_300
                + theme.spacing._300
                + theme.shell.palette_row_height / 2.0;
            let _ = view;
            gpui::point(x, y)
        });
        visual.simulate_click(point, gpui::Modifiers::default());

        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_tab()),
            RootTab::Chats,
            "the click reached the row rather than the scrim behind it"
        );
        assert!(with(&shell, cx, |view, _, _| view.palette.is_none()));
    }

    #[gpui::test]
    fn clicking_away_from_the_palette_closes_it(cx: &mut TestAppContext) {
        let shell = open(cx);
        let handle: gpui::AnyWindowHandle = shell.into();
        let mut visual = gpui::VisualTestContext::from_window(handle, cx);

        visual.simulate_keystrokes("cmd-k");
        visual.simulate_click(gpui::point(px(20.0), px(600.0)), gpui::Modifiers::default());

        assert!(with(&shell, cx, |view, _, _| view.palette.is_none()));
        assert_eq!(
            with(&shell, cx, |view, _, _| view.selected_tab()),
            RootTab::Feed,
            "dismissing must not also run whatever was highlighted"
        );
    }

    #[gpui::test]
    fn a_drop_is_routed_to_whatever_the_window_has_in_front(cx: &mut TestAppContext) {
        use crate::app::shell::file_drop::DropRejection;
        use std::path::PathBuf;

        let mut paths = ExternalPaths::default();
        paths.0.push(PathBuf::from("/tmp/a.png"));

        let shell = open(cx);
        with(&shell, cx, |view, _, cx| {
            view.navigation.back();
            assert_eq!(
                view.files_dropped(&paths, cx),
                DropOutcome::Rejected(DropRejection::NoAcceptor),
                "a feed does not take files"
            );

            view.open_route(
                RootRoute::Chat {
                    chat_id: "c1".into(),
                },
                cx,
            );
            assert!(
                matches!(
                    view.files_dropped(&paths, cx),
                    DropOutcome::Rejected(DropRejection::NotAFile(_))
                ),
                "the path does not exist, so the policy refuses it rather than \
                 queueing an upload of nothing"
            );
        });
    }
}

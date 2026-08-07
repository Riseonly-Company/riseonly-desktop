use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyView, App, ClickEvent, Context, Entity, ExternalPaths, FocusHandle, Focusable, IntoElement,
    Pixels, Point, Render, Subscription, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_navigation::{NavigationStore, PaneLayout, PanePolicy, PaneSlot, RootRoute, RootTab};
use rise_platform::materials::Material;
use rise_widgets::{
    ContextMenuEntry, ContextMenuEvent, ContextMenuItem, ContextMenuState, GlassPanel,
    OverlayAnchor, OverlayLayer, OverlayScrim, OverlaySide, OverlayStack, glass_host, place,
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
use crate::modules::auth::stores::auth_services::try_auth_services;
use crate::modules::onboarding::pages::onboarding::onboarding::OnboardingEvent;

/// What is floating above the columns, newest last.
///
/// Esc closes the topmost and only the topmost, which is why this is a stack and
/// not two booleans.
const PALETTE_OVERLAY: &str = "shell.command_palette";
const RAIL_MENU_OVERLAY: &str = "shell.rail_menu";

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
    /// One live view per resource key, so a screen keeps its caret, its scroll
    /// position and the step of the form it is on across frames. Rebuilding a
    /// destination on every render would reset all three.
    views: HashMap<String, AnyView>,
    /// Whether the last snapshot said somebody is signed in, so the root route
    /// is only swapped when the answer actually changes.
    is_authenticated: bool,
    /// Set by anything that changes what is on screen, cleared once the lease
    /// stack has been told. Rebuilding the set on every frame would allocate on
    /// the frame path for nothing — the routes only move when the user or the
    /// window moves them.
    screens_dirty: bool,
    _subscriptions: Vec<Subscription>,
}

impl RootView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let policy = PanePolicy::from_shell_metrics(rise_ui::theme(cx as &App).shell);
        let layout = policy.layout_for(window.viewport_size().width);
        let mut navigation = NavigationStore::new(policy, layout);

        // The route the app opens on is the auth answer, not a constant: a
        // returning user must not see onboarding for the frame it takes the
        // restore to finish.
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
            // Two of SessionActivity's five signals. Focus and the window count
            // are what this toolkit can answer: gpui can minimise a window but
            // cannot report that it is minimised, and it knows nothing about
            // system idle or a locked screen. Those need a rise-platform seam
            // that does not exist yet — the policy is written and tested, the
            // sensor is not, and the shell says so rather than guessing.
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

        // Signing in and signing out both move the whole window between the
        // pre-auth screens and the shell, so the frame follows the snapshot
        // rather than each screen navigating on its own.
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
            views: HashMap::new(),
            is_authenticated,
            screens_dirty: true,
            _subscriptions: subscriptions,
        }
    }

    /// The resource key of the screen in front.
    #[cfg(test)]
    pub fn focused_screen_key(&self) -> Option<String> {
        self.navigation.focused_route().map(RootRoute::resource_key)
    }

    pub fn open_route(&mut self, route: RootRoute, cx: &mut Context<Self>) {
        self.navigation.push(route);
        self.screens_dirty = true;
        cx.notify();
    }

    fn selected_tab(&self) -> RootTab {
        self.navigation.tab()
    }

    /// Onboarding, sign-in and sign-up own the whole window.
    ///
    /// The rail is navigation between sections of a signed-in app; showing it
    /// to somebody who has not signed in yet offers five destinations that all
    /// refuse them.
    fn shows_rail(&self) -> bool {
        self.navigation
            .active_routes()
            .iter()
            .all(|route| route.requires_authentication())
    }

    /// How many panes the frame draws — every column the layout affords, or one
    /// when a pre-auth screen owns the window.
    ///
    /// A column with no route in it is still drawn, as the shell's empty state:
    /// a window wide enough for a conversation beside the list, with nothing
    /// open, is an empty state and not a blank rectangle.
    fn drawn_pane_count(&self) -> usize {
        if self.shows_rail() {
            self.navigation.layout().column_count()
        } else {
            1
        }
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

    /// Esc closes the topmost thing, and only the topmost thing.
    ///
    /// An overlay is above every column, so it goes first; only once nothing is
    /// floating does Esc reach the navigation stacks.
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

    /// Takes the keyboard back after an overlay that owned it goes away.
    ///
    /// Without this the focused handle is one that has just been dropped, and
    /// gpui falls back to the root — which happens to carry the shell's key
    /// context, so the shortcuts survive by luck rather than by design.
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

    /// The shell decides what a drop means; nothing consumes the files yet.
    ///
    /// The composer that will take them is Phase 7 work. What lands here is the
    /// whole policy — which screen is the target, how many files are too many,
    /// what is not a file — so that wiring a consumer is one call and not a
    /// design.
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

    /// Mounts what is on screen and drops what is not.
    ///
    /// Runs BEFORE the element tree is built, never inside it. Creating an
    /// entity and subscribing to it are effects, and gpui flushes effects
    /// between frames — doing either halfway through a render leaves the
    /// dispatch tree a frame behind, which shows up as a keystroke going
    /// nowhere.
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

    /// Builds a screen once and keeps it.
    ///
    /// Keyed by the resource key rather than by the route, so the same
    /// conversation opened from a folder and from search is one screen — the
    /// same rule the lease stack and the cache use.
    fn mount(&mut self, route: &RootRoute, window: &mut Window, cx: &mut Context<Self>) {
        let key = route.resource_key();
        if self.views.contains_key(&key) {
            return;
        }

        let view = destination(route, window, cx);

        // Onboarding is the one pre-auth screen that hands over rather than
        // navigating: skip and get-started both mean "take me to sign-up", and
        // the frame owns navigation.
        if *route == RootRoute::Onboarding
            && let Ok(onboarding) = view.clone().downcast::<crate::modules::onboarding::pages::onboarding::onboarding::Onboarding>()
        {
            cx.subscribe(&onboarding, |view, _, event, cx| match event {
                OnboardingEvent::Finished => view.replace_root(RootRoute::SignUp, cx),
            })
            .detach();
        }

        self.views.insert(key, view);
    }

    /// Swaps what the whole window is showing.
    ///
    /// Used for the pre-auth screens, which own the window rather than a column:
    /// pushing sign-up on top of onboarding would leave onboarding underneath as
    /// a back destination the user cannot usefully return to.
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
        let is_sidebar = layout != PaneLayout::Stacked && slot == Some(PaneSlot::Primary);
        let is_aside = layout == PaneLayout::ThreeColumn && slot == Some(PaneSlot::Tertiary);

        match (is_sidebar, is_aside) {
            // The list column is the one surface beside the rail that is a
            // material rather than opaque content.
            (true, _) => GlassPanel::surface("shell.sidebar", Material::Panel, cx)
                .w(theme.shell.sidebar_width)
                .h_full()
                .child(body),
            (_, true) => div().w(theme.shell.aside_width).h_full().child(body),
            _ => div().flex_1().h_full().child(body),
        }
    }

    /// Onboarding, sign-in and sign-up get the whole window.
    ///
    /// Not a column: a 2560px window would otherwise draw the sign-in form in a
    /// 320px sidebar with two empty states beside it. Nothing is open behind
    /// these screens, so there is nothing for the other columns to show.
    fn full_window_pane(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let route = self.navigation.focused_route().cloned();
        let body = match route
            .as_ref()
            .and_then(|route| self.views.get(&route.resource_key()))
        {
            Some(view) => view.clone(),
            None => self.empty_pane(cx),
        };

        div().flex_1().h_full().child(body)
    }

    /// A drawn column with nothing in it still says something.
    ///
    /// The three states apply to the frame as much as to a screen: a window wide
    /// enough for a conversation beside the list, with no conversation open, is
    /// an empty state and not a blank rectangle.
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

        // A window nobody has clicked in yet has no focused element, and gpui
        // then dispatches from the window root — where the shell's key context
        // is not, because the shell's context is on the element below it. Every
        // shortcut would be dead until the first click.
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }

        let theme = rise_ui::theme(cx as &App).clone();
        let shows_rail = self.shows_rail();
        let selected = self.selected_tab();
        let folders = self.folders.clone();
        let selected_folder = self.selected_folder.clone();
        let handlers = self.rail_handlers();

        let mut row = rise_ui::BoxUi::screen(&theme)
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
            }))
            .relative()
            .flex()
            .flex_row();

        if shows_rail {
            row = row.child(Rail::render(
                RailState {
                    selected,
                    folders: &folders,
                    selected_folder: selected_folder.as_deref(),
                },
                &handlers,
                cx,
            ));

            for column in 0..self.drawn_pane_count() {
                row = row.child(self.pane(column, cx));
            }
        } else {
            row = row.child(self.full_window_pane(cx));
        }

        // The palette is a modal, not an anchored popup: it is centred against
        // the window rather than placed beside something, so it needs no flip
        // arithmetic — the flex layout cannot put it outside the frame.
        if let Some(palette) = self.palette.clone() {
            row = row.child(
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
            let viewport = window.viewport_size();
            let placement = place(
                menu.read(cx).anchor(),
                menu.read(cx).asked_size(&theme),
                viewport,
                theme.shell.overlay_gap,
                theme.shell.overlay_margin,
                OverlaySide::Below,
            );

            row = row.child(
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

        // Mounted last and once. gpui prepaints the whole tree before it paints
        // any of it, so this commits every glass region the frame reported —
        // from the layout pass, never from a render pass.
        row.child(glass_host())
    }
}

/// The reference's `ContextMenuItem.key`: a stable action id, not a label.
const RAIL_MENU_OPEN: &str = "open";

/// What right-clicking a rail item offers.
///
/// One entry, because one is what the shell can honestly do today: the folder
/// actions the reference has — mark read, mute, reorder — all need chat data
/// that arrives with the module in Phase 7. The ids and the machinery are here
/// so that adding them is a list edit rather than a design.
fn rail_menu_entries() -> Vec<ContextMenuEntry> {
    vec![
        ContextMenuItem::new(RAIL_MENU_OPEN, "ctx_open")
            .icon("arrow.up.right")
            .into(),
    ]
}

/// Borrowed from the reference rather than invented.
///
/// A column that is drawn but holds nothing is a desktop-only state — a phone
/// has one column and it is never empty — so the reference has no string for it,
/// and inventing one would mean writing 41 translations here. This key is the
/// nearest thing it already says, in every locale it ships.
const EMPTY_PANE_KEY: &str = "chat_preview_empty";

/// The placeholder a drawn-but-unoccupied column shows.
struct EmptyPane;

impl Render for EmptyPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App);

        rise_ui::BoxUi::surface(theme)
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
        with(&shell, cx, |view, _, cx| view.select_slot(7, cx));

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
            view.select_slot(6, cx);
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

        // Driven through VisualTestContext because this test is about the
        // DISPATCH TREE, and that only exists once a frame has been drawn.
        //
        // The read between the two chords is load-bearing as well as an
        // assertion: dismissing takes the keyboard back with `window.focus`, and
        // that is applied when the window is next updated. In the app the notify
        // that follows draws a frame; here the read is what supplies it.
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

        // The first result, inside the panel — where the dismissing scrim covers
        // exactly the same point, which is the whole hazard.
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

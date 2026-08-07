use gpui::{AnyView, App, AppContext, Context, Window};
use rise_navigation::{RootRoute, RootTab};

use crate::app::composition;
use crate::modules::auth::pages::sign::auth_step_flow::AuthMode;
use crate::modules::auth::pages::sign::sign_in::sign_in::SignIn;
use crate::modules::auth::pages::sign::sign_up::sign_up::SignUp;
use crate::modules::onboarding::pages::onboarding::onboarding::Onboarding;

/// The single place a route becomes a screen.
///
/// The match is exhaustive on purpose: adding a variant to RootRoute fails the
/// build here until someone decides what it renders. That is the same property
/// the iOS reference gets from its switch with no default arm.
///
/// The caller CACHES what this returns, keyed by the route's resource key. A
/// screen built fresh on every frame would lose its caret, its scroll position
/// and the step of the form it was on — which is exactly what a step flow is.
pub fn destination<T: 'static>(
    route: &RootRoute,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyView {
    match route {
        RootRoute::Onboarding => cx.new(|cx| Onboarding::new(window, cx)).into(),
        RootRoute::SignIn => auth_screen(AuthMode::SignIn, window, cx),
        RootRoute::SignUp => auth_screen(AuthMode::SignUp, window, cx),
        RootRoute::Tab(tab) => placeholder(tab_title(*tab), cx),
        RootRoute::Chat { .. } => placeholder("Chat", cx),
        RootRoute::ChatTopic { .. } => placeholder("Chat topic", cx),
        RootRoute::UserProfile { .. } => placeholder("Profile", cx),
        RootRoute::Post { .. } => placeholder("Post", cx),
        RootRoute::Settings => placeholder("Settings", cx),
    }
}

/// The sign-in and sign-up screens, or a placeholder when the composition root
/// never ran.
///
/// That happens when the transport could not start: the app still draws, and a
/// screen that cannot reach the engine says so rather than panicking on a global
/// that is not there.
fn auth_screen<T: 'static>(mode: AuthMode, window: &mut Window, cx: &mut Context<T>) -> AnyView {
    match composition::interactions(cx as &App) {
        Some(interactions) => {
            let auth = interactions.auth.clone();
            match mode {
                AuthMode::SignIn => cx.new(|cx| SignIn::new(auth, window, cx)).into(),
                AuthMode::SignUp => cx.new(|cx| SignUp::new(auth, window, cx)).into(),
            }
        }
        None => placeholder("Sign in", cx),
    }
}

fn tab_title(tab: RootTab) -> &'static str {
    match tab {
        RootTab::Feed => "Feed",
        RootTab::Search => "Search",
        RootTab::Shorts => "Shorts",
        RootTab::Chats => "Chats",
        RootTab::Profile => "Profile",
    }
}

fn placeholder<T: 'static>(title: &'static str, cx: &mut Context<T>) -> AnyView {
    cx.new(|_| UnbuiltScreen { title }).into()
}

pub struct UnbuiltScreen {
    title: &'static str,
}

impl gpui::Render for UnbuiltScreen {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        let theme = rise_ui::theme(cx as &App);
        rise_ui::BoxUi::screen(theme)
            .flex()
            .items_center()
            .justify_center()
            .child(
                rise_ui::MainText::body(theme, rise_ui::TextTone::Secondary)
                    .child(format!("{} — not built yet", self.title)),
            )
    }
}

use gpui::{AnyView, App, AppContext, Context, Window};
use rise_navigation::{RootRoute, RootTab};

use crate::app::composition;
use crate::modules::auth::pages::sign::auth_step_flow::AuthMode;
use crate::modules::auth::pages::sign::sign_in::sign_in::SignIn;
use crate::modules::auth::pages::sign::sign_up::sign_up::SignUp;
use crate::modules::onboarding::pages::onboarding::onboarding::Onboarding;
use crate::modules::post::pages::posts::Posts;
use crate::modules::user::pages::profile::{Profile, ProfileRouteTarget};

pub fn destination<T: 'static>(
    route: &RootRoute,
    can_dismiss: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyView {
    match route {
        RootRoute::Onboarding => cx.new(|cx| Onboarding::new(window, cx)).into(),
        RootRoute::SignIn | RootRoute::SignUp => {
            auth_screen(AuthMode::SignIn, can_dismiss, window, cx)
        }
        RootRoute::Tab(RootTab::Feed) => feed_screen(window, cx),
        RootRoute::Tab(RootTab::Profile) => profile_screen(ProfileRouteTarget::Own, window, cx),
        RootRoute::Tab(tab) => placeholder(tab_title(*tab), cx),
        RootRoute::Chat { .. } => placeholder("Chat", cx),
        RootRoute::ChatTopic { .. } => placeholder("Chat topic", cx),
        RootRoute::UserProfile { user_id } => {
            profile_screen(ProfileRouteTarget::resolve(Some(user_id), None), window, cx)
        }
        RootRoute::Post { .. } => placeholder("Post", cx),
        RootRoute::Settings => placeholder("Settings", cx),
    }
}

fn auth_screen<T: 'static>(
    mode: AuthMode,
    can_dismiss: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyView {
    match composition::interactions(cx as &App) {
        Some(interactions) => {
            let auth = interactions.auth.clone();
            match mode {
                AuthMode::SignIn => cx
                    .new(|cx| SignIn::new(auth, can_dismiss, window, cx))
                    .into(),
                AuthMode::SignUp => cx
                    .new(|cx| SignUp::new(auth, can_dismiss, window, cx))
                    .into(),
            }
        }
        None => placeholder("Sign in", cx),
    }
}

fn feed_screen<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> AnyView {
    match composition::interactions(cx as &App) {
        Some(interactions) => {
            let post = interactions.post.clone();
            cx.new(|cx| Posts::new(post, window, cx)).into()
        }
        None => placeholder("Feed", cx),
    }
}

fn profile_screen<T: 'static>(
    target: ProfileRouteTarget,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyView {
    match composition::interactions(cx as &App) {
        Some(interactions) => {
            let user = interactions.user.clone();
            let post = interactions.post.clone();
            cx.new(|cx| Profile::new(user, post, target, window, cx))
                .into()
        }
        None => placeholder("Profile", cx),
    }
}

fn tab_title(tab: RootTab) -> &'static str {
    match tab {
        RootTab::Feed => "Feed",
        RootTab::Search => "Search",
        RootTab::Shorts => "Shorts",
        RootTab::Chats => "Chats",
        RootTab::Vacancies => "Jobs",
        RootTab::Music => "Music",
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

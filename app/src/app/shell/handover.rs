use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::App;
use rise_navigation::RootRoute;
use rise_platform::DeepLink;
use rise_platform::single_instance::PrimaryInstance;

use crate::app::root_view::RootView;
use crate::app::router::deep_link_route::route_for;
use crate::app::shell::window_presenter::WindowPresenter;

/// How often the running instance looks for what a later launch handed it.
///
/// The check is one `exists` on a path that is almost always absent, so the cost
/// is a stat per interval and nothing else. It is a poll rather than a watch
/// because `rise-platform` deliberately owns no thread: the file is the IPC, and
/// the app decides when to read it.
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Where `on_open_urls` leaves what the OS handed the running app.
///
/// macOS does NOT pass a link in argv when the app is already running — it
/// delivers an Apple Event, which gpui surfaces as `Application::on_open_urls`,
/// registered before the app has an `App` to talk to. So the callback parks the
/// links here and the same loop that drains the file inbox picks them up.
pub type UrlInbox = Rc<RefCell<Vec<String>>>;

/// Turns what a second launch was asked to do into screens to open.
///
/// argv[0] is the executable and every launch carries it, so it is skipped;
/// anything that is not a link this build understands is ignored rather than
/// guessed at. A launch with nothing routable in it still counts — the running
/// window is raised, which is what the user asked for by launching the app.
pub fn routes_from(handover: &[String], scheme: &str, web_host: &str) -> Vec<RootRoute> {
    routes_from_links(handover.iter().skip(1), scheme, web_host)
}

/// The same mapping over bare links, with no executable path in front of them.
pub fn routes_from_links<'a>(
    links: impl IntoIterator<Item = &'a String>,
    scheme: &str,
    web_host: &str,
) -> Vec<RootRoute> {
    links
        .into_iter()
        .filter_map(|raw| DeepLink::parse(raw, scheme, web_host).ok())
        .filter_map(|link| route_for(&link))
        .collect()
}

/// Drains both inboxes for the life of the app.
///
/// Owns the `PrimaryInstance`, so the advisory lock lives exactly as long as the
/// task does. Dropping it releases the lock, and the next launch becomes the
/// primary rather than handing arguments to nobody. The lock is optional: a
/// filesystem with no advisory locking still gets URL delivery, it just does not
/// get single-instance behaviour.
pub fn serve(
    primary: Option<PrimaryInstance>,
    urls: UrlInbox,
    scheme: String,
    web_host: String,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;

            let handovers = primary
                .as_ref()
                .map(PrimaryInstance::take_pending)
                .unwrap_or_default();
            let opened: Vec<String> = urls.borrow_mut().drain(..).collect();

            if handovers.is_empty() && opened.is_empty() {
                continue;
            }

            let mut routes: Vec<RootRoute> = handovers
                .iter()
                .flat_map(|handover| routes_from(handover, &scheme, &web_host))
                .collect();
            routes.extend(routes_from_links(opened.iter(), &scheme, &web_host));

            cx.update(|cx| deliver(&routes, cx));
        }
    })
    .detach();
}

/// Raises a window and opens whatever the other launch asked for in it.
///
/// The frontmost shell window, not a new one: a link from a chat client should
/// bring the app the user already has forward, not stack another window on it.
/// With no window at all — which on macOS is an ordinary state, since closing
/// the last one does not end the app — one is opened AND the link is still
/// delivered into it. Returning early there would log a handover the user never
/// received.
pub fn deliver(routes: &[RootRoute], cx: &mut App) {
    tracing::info!(
        target: "riseonly",
        "opening {} screen(s) from a link: {:?}",
        routes.len(),
        routes.iter().map(RootRoute::screen_id).collect::<Vec<_>>()
    );

    let handle = match frontmost_shell(cx) {
        Some(handle) => handle,
        None => match WindowPresenter::open_handle(cx, RootView::new) {
            Some(handle) => handle,
            None => return,
        },
    };

    let _ = handle.update(cx, |view, window, cx| {
        for route in routes {
            view.open_route(route.clone(), cx);
        }
        window.activate_window();
    });
}

/// `window_stack()` is the platform's front-to-back order; `windows()` is
/// creation order and would deliver into whichever window was opened first.
///
/// The fallback matters: the stack is `None` on Wayland and on gpui's default
/// platform, and on macOS it omits off-screen windows.
fn frontmost_shell(cx: &App) -> Option<gpui::WindowHandle<RootView>> {
    cx.window_stack()
        .filter(|stack| !stack.is_empty())
        .unwrap_or_else(|| cx.windows())
        .into_iter()
        .find_map(|window| window.downcast::<RootView>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use rise_navigation::RootTab;
    use rise_theme::AppTheme;

    #[gpui::test]
    fn a_link_that_arrives_with_no_window_open_still_reaches_a_screen(cx: &mut TestAppContext) {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });

        let routes = vec![RootRoute::Chat {
            chat_id: "42".into(),
        }];
        cx.update(|cx| {
            assert!(frontmost_shell(cx).is_none(), "the test starts windowless");
            deliver(&routes, cx);
        });

        let opened = cx.update(|cx| {
            frontmost_shell(cx).map(|handle| {
                handle
                    .read_with(cx, |view, _| view.focused_screen_key().unwrap_or_default())
                    .expect("the window is open")
            })
        });

        assert_eq!(
            opened.as_deref(),
            Some("Chat:42"),
            "closing the last window on macOS leaves the app running; a link that \
             arrives then must open a window AND land on the screen it named"
        );
    }

    const SCHEME: &str = "riseonly";
    const HOST: &str = "riseonly.net";

    fn routes(arguments: &[&str]) -> Vec<RootRoute> {
        let owned: Vec<String> = arguments.iter().map(|a| (*a).to_owned()).collect();
        routes_from(&owned, SCHEME, HOST)
    }

    #[test]
    fn the_executable_path_is_not_mistaken_for_a_link() {
        assert!(routes(&["/Applications/Riseonly.app/Contents/MacOS/riseonly"]).is_empty());
    }

    #[test]
    fn a_launch_carrying_a_link_opens_that_screen() {
        assert_eq!(
            routes(&["riseonly", "riseonly://chat/42"]),
            vec![RootRoute::Chat {
                chat_id: "42".into()
            }]
        );
    }

    #[test]
    fn a_launch_carrying_several_links_opens_all_of_them_in_order() {
        assert_eq!(
            routes(&["riseonly", "riseonly://chats", "riseonly://post/9"]),
            vec![
                RootRoute::Tab(RootTab::Chats),
                RootRoute::Post {
                    post_id: "9".into()
                }
            ]
        );
    }

    #[test]
    fn a_link_belonging_to_another_environment_is_ignored() {
        assert!(routes(&["riseonly", "riseonly-staging://chat/42"]).is_empty());
        assert!(routes(&["riseonly", "https://example.com/chat/42"]).is_empty());
    }

    #[test]
    fn a_plain_flag_is_not_a_link_and_does_not_stop_the_ones_after_it() {
        assert_eq!(
            routes(&["riseonly", "-riseStorybook", "riseonly://settings"]),
            vec![RootRoute::Settings]
        );
    }

    #[test]
    fn a_launch_with_nothing_routable_asks_for_no_screens() {
        assert!(routes(&["riseonly"]).is_empty());
    }
}

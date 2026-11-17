use gpui::{AnyWindowHandle, App};
use rise_core::foundation::foundation_url::Url;

use crate::app::root_view::RootView;
use crate::app::shell::handover_hooks::{HandoverHooks, set_handover_hooks};
use crate::app::shell::window_presenter::WindowPresenter;
use crate::modules::system_share::desktop_share_intake;

pub fn install_handover_hooks() {
    set_handover_hooks(HandoverHooks {
        is_shell: |window| window.downcast::<RootView>().is_some(),
        open_shell: |cx| WindowPresenter::open_handle(cx, RootView::new).map(AnyWindowHandle::from),
        open_urls,
        open_url,
        deliver_shared: desktop_share_intake::deliver_shared,
    });
}

fn open_urls(shell: AnyWindowHandle, urls: &[Url], cx: &mut App) {
    let Some(handle) = shell.downcast::<RootView>() else {
        return;
    };
    let _ = handle.update(cx, |view, window, cx| {
        for url in urls {
            view.on_open_url(url.clone(), cx);
        }
        window.activate_window();
    });
}

fn open_url(shell: AnyWindowHandle, url: Url, cx: &mut App) -> Result<(), String> {
    let handle = shell.downcast::<RootView>().ok_or_else(|| "no shell window".to_owned())?;
    handle
        .update(cx, |view, _, cx| view.on_open_url(url, cx))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use rise_platform::system_share::IncomingShare;
    use rise_theme::AppTheme;

    use super::*;
    use crate::app::desktop_share_intake_hooks_install::install_desktop_share_intake_hooks;
    use crate::app::shell::handover::{deliver, deliver_incoming_share, frontmost_shell, raise_app, urls_from};

    #[gpui::test]
    fn raising_the_app_with_no_window_open_opens_one(cx: &mut TestAppContext) {
        install_handover_hooks();
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });

        cx.update(|cx| {
            assert!(frontmost_shell(cx).is_none(), "the test starts windowless");
            raise_app(cx);
            assert!(frontmost_shell(cx).is_some());
        });
    }

    #[gpui::test]
    fn a_link_that_arrives_with_no_window_open_still_opens_a_window(cx: &mut TestAppContext) {
        install_handover_hooks();
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });

        let links = urls(&["riseonly", "riseonly://chat/42"]);
        cx.update(|cx| {
            assert!(frontmost_shell(cx).is_none(), "the test starts windowless");
            deliver(&links, cx);
        });

        assert!(
            cx.update(|cx| frontmost_shell(cx).is_some()),
            "closing the last window on macOS leaves the app running; a link that \
             arrives then must open a window for the queue's consumer"
        );
    }

    #[gpui::test]
    fn a_service_share_with_no_window_open_still_opens_a_window(cx: &mut TestAppContext) {
        install_handover_hooks();
        install_desktop_share_intake_hooks();
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });

        let incoming = IncomingShare { files: Vec::new(), texts: vec!["shared text".to_owned()] };
        cx.update(|cx| {
            assert!(frontmost_shell(cx).is_none(), "the test starts windowless");
            deliver_incoming_share(incoming, cx);
            assert!(frontmost_shell(cx).is_some());
        });
    }

    fn open_shell(cx: &mut TestAppContext) -> gpui::WindowHandle<RootView> {
        cx.update(|cx| WindowPresenter::open_handle(cx, RootView::new)).expect("the shell window opens")
    }

    fn activate(shell: gpui::WindowHandle<RootView>, cx: &mut TestAppContext) {
        cx.update(|cx| shell.update(cx, |_, window, _| window.activate_window())).expect("the window is open");
        cx.run_until_parked();
    }

    fn answers_for_the_process(shell: gpui::WindowHandle<RootView>, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| shell.update(cx, |view, _, cx| view.is_frontmost_shell(cx))).expect("the window is open")
    }

    #[gpui::test]
    fn exactly_the_front_window_takes_the_queue_and_the_next_one_when_it_closes(cx: &mut TestAppContext) {
        install_handover_hooks();
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                rise_ui::install_theme(AppTheme::dark(), cx);
            }
        });
        let first = open_shell(cx);
        let second = open_shell(cx);

        activate(second, cx);
        assert!(answers_for_the_process(second, cx), "the link is raised in the front window");
        assert!(!answers_for_the_process(first, cx), "the older window must not win by subscribing first");

        activate(first, cx);
        assert!(answers_for_the_process(first, cx));
        assert!(!answers_for_the_process(second, cx));

        cx.update(|cx| first.update(cx, |_, window, _| window.remove_window())).expect("the window is open");
        cx.run_until_parked();
        assert!(answers_for_the_process(second, cx), "a closed consumer hands the queue on");
    }

    fn urls(arguments: &[&str]) -> Vec<Url> {
        let owned: Vec<String> = arguments.iter().map(|a| (*a).to_owned()).collect();
        urls_from(&owned)
    }
}

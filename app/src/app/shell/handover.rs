use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::App;
use rise_core::foundation::foundation_file_url::posix_file_url_string_from_bytes;
use rise_core::foundation::foundation_url::Url;
use rise_platform::single_instance::PrimaryInstance;
use rise_platform::system_share::{self, IncomingShare};

use crate::app::shell::desktop_share_arguments_hooks::{
    DesktopShareArguments, split_launch_arguments, split_opened_urls,
};
use crate::app::shell::handover_hooks::handover_hooks;

const POLL_INTERVAL: Duration = Duration::from_millis(400);

pub type UrlInbox = Rc<RefCell<Vec<String>>>;

pub fn urls_from(handover: &[String]) -> Vec<Url> {
    urls_from_links(handover.iter().skip(1))
}

/// anything without a scheme is a path or a flag and not a link
/// the universal link parser decides which links belong to this environment
pub fn urls_from_links<'a>(links: impl IntoIterator<Item = &'a String>) -> Vec<Url> {
    links
        .into_iter()
        .filter_map(|raw| Url::from_string(raw))
        .filter(|url| url.scheme().is_some())
        .collect()
}

/// argv as strings for launch and hand over
/// a non utf8 file name travels as a file url with escapes
/// a non unicode word naming no file is dropped
pub fn launch_arguments_from(
    arguments: impl IntoIterator<Item = OsString>,
    current_directory: Option<&Path>,
    is_file: impl Fn(&Path) -> bool,
) -> Vec<String> {
    arguments
        .into_iter()
        .enumerate()
        .filter_map(|(index, raw)| {
            let raw = match raw.into_string() {
                Ok(argument) => return Some(argument),
                Err(raw) => raw,
            };
            if index == 0 || raw.as_encoded_bytes().first() == Some(&b'-') {
                return Some(raw.to_string_lossy().into_owned());
            }
            let joined = match current_directory {
                Some(current_directory) => current_directory.join(&raw),
                None => PathBuf::from(&raw),
            };
            let encoded = if is_file(&joined) {
                posix_file_url_string_from_bytes(joined.as_os_str().as_encoded_bytes())
            } else {
                None
            };
            if encoded.is_none() {
                tracing::warn!(
                    target: "riseonly",
                    "launch argument {} is not Unicode and names no file; left out",
                    raw.to_string_lossy()
                );
            }
            encoded
        })
        .collect()
}

/// a second instance has another working directory so relative paths are resolved before they leave
/// a word that names no file stays as is
pub fn absolute_launch_arguments(
    arguments: Vec<String>,
    current_directory: Option<&Path>,
    is_file: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let Some(current_directory) = current_directory else {
        return arguments;
    };
    arguments
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            if index == 0 || raw.is_empty() || raw.starts_with('-') || Path::new(&raw).is_absolute() {
                return raw;
            }
            let joined = current_directory.join(&raw);
            match joined.to_str() {
                Some(absolute) if is_file(&joined) => absolute.to_owned(),
                _ => raw,
            }
        })
        .collect()
}

/// links first, since open url claims a share handoff before anything else
/// a launch sharing nothing still reaches deliver and brings a window forward
pub fn deliver_arguments(arguments: DesktopShareArguments, cx: &mut App) {
    if !arguments.links.is_empty() || !arguments.has_shared_content() {
        deliver(&arguments.links, cx);
    }
    if let Some(hooks) = handover_hooks() {
        (hooks.deliver_shared)(&arguments.files, &arguments.texts, cx);
    }
}

/// a service call comes from another app so the app comes forward even if the share is refused
pub fn deliver_incoming_share(incoming: IncomingShare, cx: &mut App) {
    if let Some(hooks) = handover_hooks() {
        (hooks.deliver_shared)(&incoming.files, &incoming.texts, cx);
    }
    raise_app(cx);
}

fn serve_incoming_share(cx: &mut App) {
    let (sender, mut incoming_shares) = mpsc::unbounded::<IncomingShare>();
    let support = system_share::on_incoming_share(move |incoming| {
        let _ = sender.unbounded_send(incoming);
    });
    if !support.is_supported() {
        tracing::debug!(target: "riseonly", "this host has no incoming share service");
        return;
    }
    cx.spawn(async move |cx| {
        while let Some(incoming) = incoming_shares.next().await {
            cx.update(|cx| deliver_incoming_share(incoming, cx));
        }
    })
    .detach();
}

pub fn serve(primary: Option<PrimaryInstance>, urls: UrlInbox, cx: &mut App) {
    serve_incoming_share(cx);
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

            let mut arguments = DesktopShareArguments::default();
            for handover in &handovers {
                arguments.extend(split_launch_arguments(handover));
            }
            arguments.extend(split_opened_urls(&opened));

            cx.update(|cx| deliver_arguments(arguments, cx));
        }
    })
    .detach();
}

/// a link from outside is queued and not pushed
/// on cold launch the restore resets the stack first and only the queue consumer runs after
pub fn deliver(urls: &[Url], cx: &mut App) {
    tracing::info!(target: "riseonly", "received {} link(s) from outside the app", urls.len());

    let Some(handle) = answering_shell(cx) else {
        return;
    };

    if let Some(hooks) = handover_hooks() {
        (hooks.open_urls)(handle, urls, cx);
    }
}

/// the front shell or a new one, closing the last window on macos leaves the app running
pub fn answering_shell(cx: &mut App) -> Option<gpui::AnyWindowHandle> {
    frontmost_shell(cx).or_else(|| handover_hooks().and_then(|hooks| (hooks.open_shell)(cx)))
}

/// a notification tap always brings the app to the screen
/// activate alone only works on macos, windows and linux raise a window and not an app
pub fn raise_app(cx: &mut App) {
    cx.activate(true);
    let Some(handle) = answering_shell(cx) else {
        return;
    };
    let _ = handle.update(cx, |_, window, _| {
        let _ = rise_platform::window_minimise::show_window(&*window);
        window.activate_window();
        #[cfg(feature = "dev-drive")]
        if rise_platform::window_minimise::minimised_for_dev_drive(&*window) == Some(true) {
            let _ = rise_platform::window_minimise::report_for_dev_drive(&*window, false);
        }
    });
}

/// the one window that answers for the process
/// system stack top or the focused window or the first open one
pub fn frontmost_shell(cx: &App) -> Option<gpui::AnyWindowHandle> {
    let hooks = handover_hooks()?;
    let open = cx.windows();
    cx.window_stack()
        .unwrap_or_default()
        .into_iter()
        .chain(cx.active_window())
        .chain(open.iter().copied())
        .filter(|window| open.contains(window))
        .find(|window| (hooks.is_shell)(*window))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_core::foundation::foundation_file_url::PathFlavor;

    fn urls(arguments: &[&str]) -> Vec<Url> {
        let owned: Vec<String> = arguments.iter().map(|a| (*a).to_owned()).collect();
        urls_from(&owned)
    }

    fn strings(links: &[Url]) -> Vec<String> {
        links.iter().map(|link| link.absolute_string().to_owned()).collect()
    }

    #[test]
    fn the_executable_path_is_not_mistaken_for_a_link() {
        assert!(urls(&["/Applications/Riseonly.app/Contents/MacOS/riseonly"]).is_empty());
        assert!(urls(&["riseonly", "/Applications/Riseonly.app"]).is_empty());
    }

    #[test]
    fn a_launch_carrying_several_links_keeps_them_in_order() {
        assert_eq!(
            strings(&urls(&["riseonly", "riseonly://chats", "riseonly://post/9"])),
            vec!["riseonly://chats".to_owned(), "riseonly://post/9".to_owned()]
        );
    }

    #[test]
    fn a_plain_flag_is_not_a_link_and_does_not_stop_the_ones_after_it() {
        assert_eq!(
            strings(&urls(&["riseonly", "-riseStorybook", "riseonly://settings"])),
            vec!["riseonly://settings".to_owned()]
        );
    }

    #[test]
    fn a_relative_file_is_resolved_where_the_launch_happened_and_a_word_is_left_alone() {
        let arguments: Vec<String> = ["riseonly", "a.png", "hello", "-flag", "/tmp/b.png"]
            .iter()
            .map(|a| (*a).to_owned())
            .collect();
        let absolute = absolute_launch_arguments(arguments, Some(Path::new("/home/me")), |path| {
            path == Path::new("/home/me/a.png")
        });
        assert_eq!(absolute, vec!["riseonly", "/home/me/a.png", "hello", "-flag", "/tmp/b.png"]);
    }

    fn name_not_in_utf8() -> OsString {
        let path = Url::from_string("file:///home/me/a%FF.png").and_then(|url| url.to_file_path());
        match path {
            Some(path) => path.into_os_string(),
            None => panic!("a Posix file URL with an escape that is not UTF-8 names a path"),
        }
    }

    #[test]
    fn a_file_name_not_in_utf8_travels_as_a_file_url_and_comes_back_with_its_bytes() {
        if PathFlavor::HOST != PathFlavor::Posix {
            return;
        }
        let arguments = launch_arguments_from(
            [OsString::from("riseonly"), name_not_in_utf8()],
            Some(Path::new("/elsewhere")),
            |_| true,
        );
        assert_eq!(arguments, vec!["riseonly", "file:///home/me/a%FF.png"]);
        let split = split_launch_arguments(&arguments);
        assert_eq!(split.files.len(), 1);
        assert_eq!(split.files[0].as_os_str().as_encoded_bytes(), b"/home/me/a\xFF.png");
    }

    #[test]
    fn a_word_not_in_utf8_that_names_no_file_is_left_out_without_a_panic() {
        if PathFlavor::HOST != PathFlavor::Posix {
            return;
        }
        let arguments = launch_arguments_from(
            [OsString::from("riseonly"), name_not_in_utf8(), OsString::from("hello")],
            None,
            |_| false,
        );
        assert_eq!(arguments, vec!["riseonly", "hello"]);
    }

    #[test]
    fn a_launch_with_nothing_routable_asks_for_nothing() {
        assert!(urls(&["riseonly"]).is_empty());
    }
}

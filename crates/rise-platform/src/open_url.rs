use std::rc::Rc;

use gpui::{App, Global};
use rise_core::foundation::foundation_url::Url;
use rise_deeplink::environment_for_url_scheme;

use crate::deep_link::DeepLink;
use crate::host_os::HostOs;
use crate::url_scheme_handler::can_open_url;

pub const OPEN_SETTINGS_URL_STRING: &str = "app-settings:";

const OPEN_SETTINGS_SCHEME: &str = "app-settings";

const SYSTEM_SCHEMES: [&str; 4] = ["https", "http", "mailto", "tel"];

const APPLICATION_SCHEMES: [&str; 1] = ["tg"];

/// where open_url sends a url
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OpenUrlDestination {
    InApp,
    System(String),
    SystemIfHandled(String),
    Refused,
}

/// settings page closest to the app page, the privacy panes (camera mic file access)
/// linux desktops share no settings url
pub const fn settings_url_on(host: HostOs) -> Option<&'static str> {
    match host {
        HostOs::MacOs => Some("x-apple.systempreferences:com.apple.preference.security?Privacy"),
        HostOs::Windows => Some("ms-settings:privacy"),
        HostOs::Linux => None,
    }
}

/// decides without touching the os
/// url_scheme is this build app scheme, none before the shell installed its route
pub fn destination(url: &Url, host: HostOs, url_scheme: Option<&str>) -> OpenUrlDestination {
    let Some(scheme) = url.scheme() else {
        return OpenUrlDestination::Refused;
    };

    if scheme.eq_ignore_ascii_case(OPEN_SETTINGS_SCHEME) {
        return match settings_url_on(host) {
            Some(settings) => OpenUrlDestination::System(settings.to_owned()),
            None => OpenUrlDestination::Refused,
        };
    }

    let raw = url.absolute_string();
    if let Some(environment) = url_scheme.and_then(environment_for_url_scheme) {
        if environment.url_scheme.eq_ignore_ascii_case(scheme) {
            return OpenUrlDestination::InApp;
        }
        let routed = DeepLink::parse_in(raw, environment).is_ok_and(|link| link.route().is_ok());
        if routed {
            return OpenUrlDestination::InApp;
        }
    }

    if SYSTEM_SCHEMES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(scheme))
    {
        OpenUrlDestination::System(raw.to_owned())
    } else if APPLICATION_SCHEMES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(scheme))
    {
        OpenUrlDestination::SystemIfHandled(raw.to_owned())
    } else {
        OpenUrlDestination::Refused
    }
}

type InAppRoute = Rc<dyn Fn(Url, &mut App)>;

struct InAppUrlRoute {
    url_scheme: String,
    route: InAppRoute,
}

impl Global for InAppUrlRoute {}

/// the shell installs this once at launch with the build app scheme and its router entry
pub fn install_in_app_url_route(
    url_scheme: &str,
    route: impl Fn(Url, &mut App) + 'static,
    cx: &mut App,
) {
    cx.set_global(InAppUrlRoute {
        url_scheme: url_scheme.to_owned(),
        route: Rc::new(route),
    });
}

/// opens the url in the system, result is whether it succeeded
pub fn open_url(url: &Url, cx: &mut App) -> bool {
    let installed = cx
        .try_global::<InAppUrlRoute>()
        .map(|installed| (installed.url_scheme.clone(), installed.route.clone()));
    let url_scheme = installed.as_ref().map(|(scheme, _)| scheme.as_str());

    match destination(url, HostOs::current(), url_scheme) {
        OpenUrlDestination::InApp => {
            let Some((_, route)) = installed else {
                return false;
            };
            route(url.clone(), cx);
            true
        }
        OpenUrlDestination::System(target) => {
            cx.open_url(&target);
            true
        }
        OpenUrlDestination::SystemIfHandled(target) => {
            if !can_open_url(url) {
                return false;
            }
            cx.open_url(&target);
            true
        }
        OpenUrlDestination::Refused => {
            tracing::debug!(target: "riseonly", scheme = ?url.scheme(), "refused to open a URL");
            false
        }
    }
}

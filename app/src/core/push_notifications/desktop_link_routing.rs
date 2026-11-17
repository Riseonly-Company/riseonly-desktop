use gpui::{App, Global};
use rise_core::foundation::foundation_url::Url;
use rise_deeplink::ENVIRONMENTS;
use rise_platform::open_url::install_in_app_url_route;
use rise_platform::system_share::register_open_with;
use rise_platform::url_scheme::register_url_scheme;

use crate::app::shell::handover;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;

struct DesktopLinkRoutingInstalled;

impl Global for DesktopLinkRoutingInstalled {}

pub fn install(cx: &mut App) {
    if cx.has_global::<DesktopLinkRoutingInstalled>() {
        return;
    }
    cx.set_global(DesktopLinkRoutingInstalled);

    let scheme = AppEnvironment::compiled().app_url_scheme();
    let support = register_url_scheme(&scheme, cx);
    tracing::debug!(target: "riseonly", scheme, ?support, "URL scheme claim at launch");
    let open_with_support = register_open_with(cx);
    tracing::debug!(target: "riseonly", ?open_with_support, "Open With claim at launch");
    install_in_app_url_route(&scheme, open_own_link, cx);
}

/// entry for a link from outside the app
/// picks the shell that answers for the process and hands it to the root view
pub fn handle_external_url(url: &Url, cx: &mut App) {
    handover::deliver(std::slice::from_ref(url), cx);
}

fn open_own_link(url: Url, cx: &mut App) {
    match own_link_route(&url, AppEnvironment::compiled()) {
        OwnLinkRoute::Shell => cx.defer(move |cx| handle_external_url(&url, cx)),
        OwnLinkRoute::Browser => cx.open_url(url.absolute_string()),
    }
}

/// parses a universal link, falling back to the malformed music link
pub fn parse_external_link(url: &Url, environment: AppEnvironment) -> Option<PushDeepLink> {
    UniversalLinkParser::parse(url, environment)
        .or_else(|| UniversalLinkParser::malformed_music_link(url, environment))
}

/// what a link from outside the app turned out to be
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExternalUrlOutcome {
    Shared,
    Link(PushDeepLink),
    Ignored,
}

/// share handoffs first and then the deep link parser
pub fn resolve_external_url(
    url: &Url,
    environment: AppEnvironment,
    handle_shared: impl FnOnce(&Url) -> bool,
) -> ExternalUrlOutcome {
    if handle_shared(url) {
        return ExternalUrlOutcome::Shared;
    }
    match parse_external_link(url, environment) {
        Some(link) => ExternalUrlOutcome::Link(link),
        None => ExternalUrlOutcome::Ignored,
    }
}

/// web link on the public host of any canon environment
/// if this build refused it its another build link or a page no screen answers
/// so the browser gets it
pub fn is_canon_web_link(url: &Url) -> bool {
    let (Some(scheme), Some(host)) = (url.scheme(), url.host()) else {
        return false;
    };
    ENVIRONMENTS.iter().any(|environment| {
        environment.web_scheme.eq_ignore_ascii_case(scheme) && environment.accepts_web_host(&host)
    })
}

/// where a link that open_url called the app own goes
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OwnLinkRoute {
    Shell,
    Browser,
}

/// the app scheme has no other reader so it always reaches the shell
/// an own host web link that no screen answers goes to the browser
pub fn own_link_route(url: &Url, environment: AppEnvironment) -> OwnLinkRoute {
    let scheme = url.scheme().map(str::to_lowercase);
    if scheme.as_deref() == Some(environment.app_url_scheme().as_str())
        || parse_external_link(url, environment).is_some()
    {
        OwnLinkRoute::Shell
    } else {
        OwnLinkRoute::Browser
    }
}

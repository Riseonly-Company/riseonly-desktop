use std::cell::RefCell;
use std::rc::Rc;

use gpui::App;
use rise_core::foundation::foundation_url::Url;
use rise_platform::open_url::open_url;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLinkStore;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::modules::bot::shared::bot_ui_validation::BotMiniAppURLPolicy;

const HTTPS_SCHEME: &str = "https";

type InAppLinkBrowserPresenter = Rc<dyn Fn(Url, &mut App)>;

thread_local! {
    static PRESENTER: RefCell<Option<InAppLinkBrowserPresenter>> = const { RefCell::new(None) };
}

/// outside web pages open first in the apps own sheet, same one bot mini apps use
/// its menu hands the page to the browser
pub enum InAppLinkBrowser {}

impl InAppLinkBrowser {
    pub fn install(presenter: impl Fn(Url, &mut App) + 'static) {
        let presenter: InAppLinkBrowserPresenter = Rc::new(presenter);
        PRESENTER.with(|slot| *slot.borrow_mut() = Some(presenter));
    }

    /// true when the sheet took the page
    /// only https pages it can load, http and mail and phone schemes stay with the system
    pub fn open(url: &Url, cx: &mut App) -> bool {
        let Some(presenter) = PRESENTER.with(|slot| slot.borrow().clone()) else {
            return false;
        };
        let Some(page) = Self::page(url) else {
            return false;
        };
        presenter(page, cx);
        true
    }

    /// for callers outside the open url environment
    /// riseonly links route in app, outside pages open in the sheet and anything else goes to the system
    pub fn route(url: &Url, cx: &mut App) {
        if let Some(link) = UniversalLinkParser::parse(url, AppEnvironment::current()) {
            let store = PushDeepLinkStore::shared(cx);
            store.update(cx, |store, cx| {
                store.enqueue(link, cx);
            });
            return;
        }
        if Self::open(url, cx) {
            return;
        }
        open_url(url, cx);
    }

    /// a bare address like example.com/a is read as a site over https
    fn page(url: &Url) -> Option<Url> {
        let resolved = if url.scheme().is_none() {
            Url::from_string(&format!("{HTTPS_SCHEME}://{}", url.absolute_string()))?
        } else {
            url.clone()
        };
        let is_https = resolved.scheme().map(str::to_lowercase).as_deref() == Some(HTTPS_SCHEME);
        if !is_https || !BotMiniAppURLPolicy::allows(&resolved, false) {
            return None;
        }
        Some(resolved)
    }
}

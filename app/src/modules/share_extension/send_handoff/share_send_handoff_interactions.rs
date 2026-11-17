use gpui::{App, Context, Entity};
use rise_canon::rise_key::events;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};

use crate::core::config::app_environment::AppEnvironment;
use crate::modules::share_extension::send_handoff::share_send_handoff_services::ShareSendHandoffServicesStore;

const SHARE_SEND_HOST: &str = "share-send";
const LEGACY_SESSION_QUERY_NAME: &str = "session";

pub struct ShareSendHandoffInteractionsStore {}

impl ShareSendHandoffInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn services(cx: &mut App) -> Entity<ShareSendHandoffServicesStore> {
        ShareSendHandoffServicesStore::shared(cx)
    }

    pub fn is_shared_send_url(&self, url: &Url) -> bool {
        let scheme_matches =
            url.scheme().map(str::to_lowercase) == Some(AppEnvironment::current().app_url_scheme());
        let host_matches = url.host().map(|host| host.to_lowercase()).as_deref() == Some(SHARE_SEND_HOST);
        if !scheme_matches || !host_matches {
            return false;
        }
        let path = url.path();
        path.is_empty() || path == "/"
    }

    pub fn handle_shared_send_url(&mut self, url: &Url, cx: &mut Context<Self>) -> bool {
        if !self.is_shared_send_url(url) {
            return false;
        }
        let session_id = UrlComponents::from_url(url, false)
            .and_then(|components| components.query_items())
            .and_then(|items| {
                items
                    .into_iter()
                    .find(|item| item.name == events::SESSION_ID || item.name == LEGACY_SESSION_QUERY_NAME)
            })
            .and_then(|item| item.value)
            .map(|value| trimming_whitespaces_and_newlines(&value).to_owned())
            .unwrap_or_default();
        Self::services(cx).update(cx, |services, cx| services.enqueue(session_id, true, cx));
        true
    }

    pub fn bootstrap(&mut self, cx: &mut Context<Self>) {
        Self::services(cx).update(cx, |services, cx| services.bootstrap(cx));
    }

    pub fn resume_pending_if_possible(&mut self, cx: &mut Context<Self>) {
        Self::services(cx).update(cx, |services, cx| services.bootstrap(cx));
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        Self::services(cx).update(cx, |services, cx| services.retry(cx));
    }

    pub fn cancel_and_discard(&mut self, cx: &mut Context<Self>) {
        Self::services(cx).update(cx, |services, cx| services.cancel_and_discard(cx));
    }

    pub fn finish_acknowledged_partial(&mut self, cx: &mut Context<Self>) {
        Self::services(cx).update(cx, |services, cx| services.finish_acknowledged_partial(cx));
    }

    pub fn dismiss_committed_until_next_launch(&mut self, cx: &mut Context<Self>) {
        Self::services(cx).update(cx, |services, cx| services.dismiss_committed_until_next_launch(cx));
    }

    pub fn account_did_change(&mut self, cx: &mut Context<Self>) {
        Self::services(cx).update(cx, |services, cx| {
            services.reset_for_account_change(cx);
            services.bootstrap(cx);
        });
    }
}

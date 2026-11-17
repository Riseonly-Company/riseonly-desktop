use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;
use crate::modules::share_extension::shared::rise_share_container::RiseShareContainer;
use crate::modules::share_extension::stores::share_targets::share_targets_actions::ShareTargetsActionsStore;
use crate::modules::share_extension::stores::share_targets::share_targets_services::ShareTargetsServicesStore;

static MAX_EMPTY_PICKER_PAGES_IN_ROW: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("share_picker_empty_page_chain"));

pub struct ShareTargetsInteractionsStore {}

impl ShareTargetsInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn bootstrap(&self, cx: &mut App) {
        RiseShareAppBridge::synchronize_credential(false);
        RiseShareContainer::remove_expired_files();
        self.refresh(cx);
    }

    pub fn refresh(&self, cx: &mut App) {
        ShareTargetsActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_share_targets_action(
                None,
                false,
                false,
                false,
                |response, cx| {
                    ShareTargetsServicesStore::shared(cx).update(cx, |services, cx| services.apply(&response, cx));
                },
                |error, cx| {
                    ShareTargetsServicesStore::shared(cx).update(cx, |services, cx| services.apply_error(error, cx));
                },
                cx,
            );
        });
    }

    pub fn load_picker_targets(
        &self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) {
        let requested_query = query.map(str::to_owned);
        ShareTargetsActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_share_targets_action(
                query,
                has_media,
                has_link,
                has_documents,
                move |response, cx| {
                    if requested_query.is_none() && !has_media && !has_link && !has_documents {
                        ShareTargetsServicesStore::shared(cx)
                            .update(cx, |services, cx| services.apply(&response, cx));
                    }
                    let Some(query) = requested_query.as_deref() else {
                        return;
                    };
                    let has_mapped_target = response
                        .targets
                        .iter()
                        .any(|target| ShareTargetsServicesStore::map_target(target).is_some());
                    if !response.has_more || has_mapped_target {
                        return;
                    }
                    ShareTargetsInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.load_more_picker_targets(Some(query), has_media, has_link, has_documents, 0, cx);
                    });
                },
                |error, cx| {
                    Sai::log(
                        "ShareSheet",
                        || format!("get_share_targets failed: {error}"),
                        SaiLogKind::Network,
                        cx,
                    );
                },
                cx,
            );
        });
    }

    pub fn reset_picker_pages(&self, cx: &mut App) {
        ShareTargetsServicesStore::shared(cx).update(cx, |services, cx| services.reset_picker_pages(cx));
    }

    pub fn load_more_picker_targets(
        &self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        empty_pages_in_row: i64,
        cx: &mut App,
    ) {
        let services = ShareTargetsServicesStore::shared(cx);
        let Some(offset) = services.update(cx, |services, cx| {
            services.next_picker_page_offset(query, has_media, has_link, has_documents, cx)
        }) else {
            return;
        };
        let success_query = query.map(str::to_owned);
        let error_query = success_query.clone();
        ShareTargetsActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_share_targets_page_action(
                offset,
                query,
                has_media,
                has_link,
                has_documents,
                move |response, cx| {
                    let services = ShareTargetsServicesStore::shared(cx);
                    let added = services.update(cx, |services, cx| {
                        services.append_picker_page(
                            &response,
                            offset,
                            success_query.as_deref(),
                            has_media,
                            has_link,
                            has_documents,
                            cx,
                        )
                    });
                    if added != 0 {
                        return;
                    }
                    let search_still_empty = success_query.is_some()
                        && services.update(cx, |services, cx| {
                            services.is_picker_search_scanning(
                                success_query.as_deref(),
                                has_media,
                                has_link,
                                has_documents,
                                cx,
                            )
                        });
                    let continues = search_still_empty || empty_pages_in_row + 1 < *MAX_EMPTY_PICKER_PAGES_IN_ROW;
                    if !continues {
                        return;
                    }
                    let next_query = success_query.clone();
                    cx.defer(move |cx| {
                        ShareTargetsInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.load_more_picker_targets(
                                next_query.as_deref(),
                                has_media,
                                has_link,
                                has_documents,
                                empty_pages_in_row + 1,
                                cx,
                            );
                        });
                    });
                },
                move |error, cx| {
                    if error_query.is_some() {
                        ShareTargetsServicesStore::shared(cx).update(cx, |services, cx| {
                            services.end_picker_pages(error_query.as_deref(), has_media, has_link, has_documents, cx);
                        });
                    }
                    Sai::log(
                        "ShareSheet",
                        || format!("get_share_targets page failed: {error}"),
                        SaiLogKind::Network,
                        cx,
                    );
                },
                cx,
            );
        });
    }

    pub fn load_picker_topics(
        &self,
        chat_id: &str,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) {
        ShareTargetsActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_share_topics_action(
                chat_id,
                has_media,
                has_link,
                has_documents,
                |_, _| {},
                |error, cx| {
                    Sai::log(
                        "ShareSheet",
                        || format!("get_share_topics failed: {error}"),
                        SaiLogKind::Network,
                        cx,
                    );
                },
                cx,
            );
        });
    }

    pub fn account_did_change(&self, cx: &mut App) {
        ShareTargetsServicesStore::shared(cx).update(cx, |services, cx| services.reset_account_state(cx));
        RiseShareAppBridge::clear_donated_interactions();
        self.bootstrap(cx);
    }

    pub fn sign_out(&self, cx: &mut App) {
        ShareTargetsServicesStore::shared(cx).update(cx, |services, cx| services.reset_account_state(cx));
        RiseShareAppBridge::clear_shared_state();
        RiseShareAppBridge::clear_donated_interactions();
    }
}

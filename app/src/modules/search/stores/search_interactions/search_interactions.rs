use gpui::{App, Context, Entity, Task};
use rise_canon::{RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::modules::post::engine::rise_post_engine_models::{RisePostPageStatus, RisePostSearchKind};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::search::engine::rise_search_engine_models::RiseGlobalSearchCategory;
use crate::modules::search::stores::search_actions::search_actions::SearchActionsStore;
use crate::modules::search::stores::search_actions::search_types::{SearchQueryKind, SearchResultsTab};
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;

pub struct SearchInteractionsStore {
    owner_clock: u64,
}

impl SearchInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { owner_clock: 0 }
    }

    fn actions(cx: &mut App) -> Entity<SearchActionsStore> {
        SearchActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<SearchServicesStore> {
        SearchServicesStore::shared(cx)
    }

    fn can_request_trailing(cache_key: &str, cx: &mut App) -> bool {
        SaiWs::pagination_state(cache_key, cx).read(cx).trailing().can_request_automatically()
    }

    pub fn tab_reselected(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.request_field_focus(cx));
    }

    pub fn on_screen_appear(&mut self, cx: &mut Context<Self>) {
        PostInteractionsStore::shared(cx).update(cx, |posts, cx| posts.load_initial_feed(cx));
    }

    pub fn activate_global_search(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &mut Context<Self>,
    ) {
        let page = Self::service(cx).update(cx, |service, cx| service.global_search_page(query, category, cx));
        if page.status != RisePostPageStatus::Idle && page.status != RisePostPageStatus::Failed {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.activate_global_search_action(query, category, cx)).detach();
    }

    pub fn observe_global_search(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        owner_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_owner_generation();
        Self::actions(cx)
            .update(cx, |actions, cx| actions.observe_global_search_action(query, category, owner_id, generation, cx))
            .detach();
    }

    pub fn stop_observing_global_search(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        owner_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_owner_generation();
        Self::actions(cx)
            .update(cx, |actions, cx| {
                actions.stop_observing_global_search_action(query, category, owner_id, generation, cx)
            })
            .detach();
    }

    pub fn refresh_global_search(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        Self::actions(cx).update(cx, |actions, cx| actions.refresh_global_search_action(query, category, cx))
    }

    pub fn load_more_global_search(
        &mut self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &mut Context<Self>,
    ) {
        let page = Self::service(cx).update(cx, |service, cx| service.global_search_page(query, category, cx));
        if !page.has_more || page.is_loading_more || page.is_refreshing {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_global_search_action(query, category, cx)).detach();
    }

    pub fn on_query_changed(&mut self, value: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        service.update(cx, |service, cx| {
            service.query_string = value.to_owned();
            cx.notify();
        });
        let trimmed = trimming_whitespaces_and_newlines(value);

        if trimmed.is_empty() {
            service.update(cx, |service, cx| service.reset_search_results(cx));
            return;
        }

        if trimmed.starts_with('@') {
            service.update(cx, |service, cx| {
                service.results_tab = SearchResultsTab::USER;
                cx.notify();
            });
        } else if trimmed.starts_with('#') {
            service.update(cx, |service, cx| {
                service.results_tab = SearchResultsTab::POST;
                cx.notify();
            });
        }

        SaiDebouncer::debounced_action_delay_milliseconds(
            "globalSearchQuery",
            Box::new(|cx: &mut App| {
                SearchInteractionsStore::shared(cx).update(cx, |store, cx| store.run_debounced_search(cx));
            }),
            RiseTimingCanon::debounce("global_search").client_debounce_ms,
            "globalSearch",
            10,
            None,
            cx,
        );
    }

    pub fn on_results_tab_changed(&mut self, tab: SearchResultsTab, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        service.update(cx, |service, cx| {
            service.results_tab = tab;
            cx.notify();
        });
        if trimming_whitespaces_and_newlines(&service.read(cx).query_string).is_empty() {
            return;
        }

        self.run_debounced_search(cx);
    }

    fn next_owner_generation(&mut self) -> u64 {
        self.owner_clock = self.owner_clock.wrapping_add(1);
        self.owner_clock
    }

    pub fn load_more_posts(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let actions = Self::actions(cx);
        match service.read(cx).active_query_kind() {
            SearchQueryKind::FeedDefault => {
                PostInteractionsStore::shared(cx).update(cx, |posts, cx| posts.load_more_feed(cx));
            }
            SearchQueryKind::Posts => {
                let has_more = service.update(cx, |service, cx| service.post_results_has_more(cx));
                let Some(rel) = service.read(cx).search_post_relative_id().filter(|rel| !rel.is_empty()) else {
                    return;
                };
                let cache_key = service.read(cx).post_results_cache_key();
                if !has_more
                    || actions.read(cx).search_posts_sai.is_pending()
                    || !Self::can_request_trailing(&cache_key, cx)
                {
                    return;
                }
                let query = service.read(cx).query_string.clone();
                actions.update(cx, |actions, cx| actions.search_posts_action(&query, Some(rel), false, true, cx));
            }
            SearchQueryKind::Hashtag => {
                let has_more = service.update(cx, |service, cx| service.hashtag_post_has_more(cx));
                let rel = service.update(cx, |service, cx| service.hashtag_post_relative_id(cx));
                let Some(rel) = rel.filter(|rel| !rel.is_empty()) else {
                    return;
                };
                if !has_more || actions.read(cx).search_posts_by_hashtag_sai.is_pending() {
                    return;
                }
                let tag = {
                    let query = &service.read(cx).query_string;
                    trimming_characters(trimming_whitespaces_and_newlines(query), |c| c == '#').to_owned()
                };
                if !Self::can_request_trailing(&SearchServicesStore::hashtag_posts_cache_key(&tag), cx) {
                    return;
                }
                actions.update(cx, |actions, cx| {
                    actions.search_posts_by_hashtag_action(&tag, Some(rel), false, true, cx);
                });
            }
            _ => {}
        }
    }

    pub fn open_hashtag_search(&mut self, raw_tag: &str, nav_store: &Entity<NavigationStore>, cx: &mut Context<Self>) {
        let tag = trimming_characters(trimming_whitespaces_and_newlines(raw_tag), |c| c == '#');
        if tag.is_empty() {
            return;
        }

        let route = RootRoute::HashtagSearch { tag: tag.to_owned(), kind: RisePostSearchKind::Publications };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }

    pub fn load_more_users(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let actions = Self::actions(cx);
        match service.read(cx).active_query_kind() {
            SearchQueryKind::GlobalUnified => {
                let has_more = service.update(cx, |service, cx| service.global_search_has_more(cx));
                let Some(rel) = service.read(cx).global_search_next_cursor().filter(|rel| !rel.is_empty()) else {
                    return;
                };
                let cache_key = service.update(cx, |service, cx| service.current_global_search_cache_key(cx));
                if !has_more
                    || actions.read(cx).global_search_sai.is_pending()
                    || !Self::can_request_trailing(&cache_key, cx)
                {
                    return;
                }
                let query = service.read(cx).query_string.clone();
                actions.update(cx, |actions, cx| actions.global_search_action(&query, Some(rel), false, true, cx));
            }
            SearchQueryKind::UsersByTag => {
                let has_more = service.update(cx, |service, cx| service.tag_entity_has_more(cx));
                let rel = service.update(cx, |service, cx| service.tag_entity_next_cursor(cx));
                let Some(rel) = rel.filter(|rel| !rel.is_empty()) else {
                    return;
                };
                let cache_key = service.update(cx, |service, cx| service.current_tag_entity_cache_key(cx));
                if !has_more
                    || actions.read(cx).global_search_by_tag_sai.is_pending()
                    || !Self::can_request_trailing(&cache_key, cx)
                {
                    return;
                }
                let query = service.read(cx).query_string.clone();
                actions.update(cx, |actions, cx| actions.global_search_by_tag_action(&query, Some(rel), false, true, cx));
            }
            SearchQueryKind::UsersPlain => {
                let query = service.read(cx).query_string.clone();
                self.load_more_plain_users(&query, cx);
            }
            _ => {}
        }
    }

    pub fn load_more_plain_users(&mut self, query: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(query);
        let Some(page) = Self::service(cx).update(cx, |service, cx| service.search_users_page(trimmed, cx)) else {
            return;
        };
        let Some(rel) = page.relative_id.map(|relative_id| relative_id.to_string()) else {
            return;
        };
        let cache_key = SearchServicesStore::search_users_cache_key(trimmed);
        let can_request =
            SaiWs::pagination_state(&cache_key, cx).read(cx).can_request_automatically(SaiWsPaginationEdge::Trailing);
        tracing::warn!(
            target: "riseonly::X2145",
            query = %trimmed,
            %rel,
            has_more = page.is_have_more,
            can_request,
            "search users load more"
        );
        if !page.is_have_more || rel.is_empty() || !can_request {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.search_users_action(trimmed, Some(rel), false, true, cx));
    }

    pub fn load_more_global_chats(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let actions = Self::actions(cx);
        if service.read(cx).active_query_kind() != SearchQueryKind::GlobalUnified {
            return;
        }
        let has_more = service.update(cx, |service, cx| service.global_search_chats_has_more(cx));
        let Some(relative_id) = service.read(cx).global_search_chats_next_cursor().filter(|rel| !rel.is_empty()) else {
            return;
        };
        let cache_key = service.update(cx, |service, cx| service.current_global_search_chats_cache_key(cx));
        if !has_more
            || actions.read(cx).global_search_chats_sai.is_pending()
            || !Self::can_request_trailing(&cache_key, cx)
        {
            return;
        }
        let query = service.read(cx).query_string.clone();
        actions.update(cx, |actions, cx| actions.global_search_chats_action(&query, Some(relative_id), false, true, cx));
    }

    pub fn load_hashtag_initial(&mut self, tag: &str, cx: &mut Context<Self>) {
        let clean = trimming_characters(trimming_whitespaces_and_newlines(tag), |c| c == '#');
        let service = Self::service(cx);
        let actions = Self::actions(cx);
        let is_same_tag = Self::is_same_tag(&service, clean, cx);
        let is_pending = actions.read(cx).search_posts_by_hashtag_sai.is_pending();
        let did_load = service.update(cx, |service, cx| service.hashtag_post_did_load(cx));
        let needs_refresh = !is_same_tag || (!did_load && !is_pending);

        if !needs_refresh || is_pending {
            return;
        }

        if !is_same_tag {
            service.update(cx, |service, cx| service.reset_hashtag_search(cx));
        }

        actions.update(cx, |actions, cx| actions.search_posts_by_hashtag_action(clean, None, true, false, cx));
    }

    pub fn load_more_hashtag(&mut self, tag: &str, cx: &mut Context<Self>) {
        let clean = trimming_characters(trimming_whitespaces_and_newlines(tag), |c| c == '#');
        let service = Self::service(cx);
        let actions = Self::actions(cx);
        let is_same_tag = Self::is_same_tag(&service, clean, cx);
        let has_more = service.update(cx, |service, cx| service.hashtag_post_has_more(cx));
        let rel = service.update(cx, |service, cx| service.hashtag_post_relative_id(cx));

        let Some(rel) = rel.filter(|rel| !rel.is_empty()) else {
            return;
        };
        if !is_same_tag
            || !has_more
            || actions.read(cx).search_posts_by_hashtag_sai.is_pending()
            || !Self::can_request_trailing(&SearchServicesStore::hashtag_posts_cache_key(clean), cx)
        {
            return;
        }

        actions.update(cx, |actions, cx| actions.search_posts_by_hashtag_action(clean, Some(rel), false, true, cx));
    }

    fn is_same_tag(service: &Entity<SearchServicesStore>, clean: &str, cx: &App) -> bool {
        service.read(cx).hashtag_current_tag.as_deref().map(str::to_lowercase) == Some(clean.to_lowercase())
    }

    fn run_debounced_search(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let actions = Self::actions(cx);
        let trimmed = trimming_whitespaces_and_newlines(&service.read(cx).query_string).to_owned();

        if trimmed.is_empty() {
            return;
        }
        GuideSignalBus::emit(RiseGuideSignal::SearchSubmitted);

        let kind = service.read(cx).active_query_kind();
        actions.update(cx, |actions, cx| match kind {
            SearchQueryKind::FeedDefault => {}
            SearchQueryKind::Posts => actions.search_posts_action(&trimmed, None, true, false, cx),
            SearchQueryKind::Hashtag => actions.search_posts_by_hashtag_action(&trimmed, None, true, false, cx),
            SearchQueryKind::GlobalUnified => {
                actions.global_search_action(&trimmed, None, true, false, cx);
                actions.global_search_chats_action(&trimmed, None, true, false, cx);
            }
            SearchQueryKind::UsersByTag => actions.global_search_by_tag_action(&trimmed, None, true, false, cx),
            SearchQueryKind::UsersPlain => actions.search_users_action(&trimmed, None, true, false, cx),
        });
    }
}

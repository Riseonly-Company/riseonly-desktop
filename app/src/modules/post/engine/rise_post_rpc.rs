use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_key, rise_wire_methods};
use rise_core::foundation::foundation_date::Date;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseJSONValue, RiseWireError, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::modules::auth::services::app_attest_client::AppAttestClient;
use crate::modules::post::engine::rise_post_debug_trace::RisePostDebugTrace;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostFeedKind, RisePostProfileKind, RisePostSearchKind,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RisePostEndpointKind {
    Read,
    Mutation,
    IdempotentMutation { field: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RisePostEndpoint {
    pub service: String,
    pub method: String,
    pub kind: RisePostEndpointKind,
    pub timeout: Duration,
}

impl RisePostEndpoint {
    /// descriptor comes from the canon catalog, kind
    /// timeout and idempotency field all from there
    /// expecting only asserts the declared intent matches
    #[track_caller]
    pub fn new(service: &str, method: &str, expected: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(service, method);
        debug_assert!(
            entry.kind == expected,
            "{service}.{method}: canon declares {}, not {}",
            entry.kind.name(),
            expected.name()
        );
        let kind = match entry.kind {
            RiseEndpointKind::Read => RisePostEndpointKind::Read,
            RiseEndpointKind::Mutation | RiseEndpointKind::RequestIdMutation => RisePostEndpointKind::Mutation,
            RiseEndpointKind::IdempotentMutation => {
                debug_assert!(
                    entry.idempotency_field.is_some(),
                    "{}: idempotent mutation without a dedup field",
                    entry.name
                );
                RisePostEndpointKind::IdempotentMutation {
                    field: entry
                        .idempotency_field
                        .unwrap_or(rise_key::post::CLIENT_REQUEST_ID)
                        .to_owned(),
                }
            }
        };
        Self {
            service: entry.service.to_owned(),
            method: entry.method.to_owned(),
            kind,
            timeout: entry.client_timeout(),
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        let replay_policy = match &self.kind {
            RisePostEndpointKind::Read => RiseWireReplayPolicy::ReadOnly,
            RisePostEndpointKind::Mutation => RiseWireReplayPolicy::Never,
            RisePostEndpointKind::IdempotentMutation { field } => {
                RiseWireReplayPolicy::IdempotencyKey { field: field.clone() }
            }
        };
        RiseWireMethodDescriptor::new(self.service.clone(), self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(replay_policy)
    }
}

static CREATE_POST: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::CREATE_POST,
        RiseEndpointKind::IdempotentMutation,
    )
});

static UPDATE_POST: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::UPDATE_POST,
        RiseEndpointKind::Mutation,
    )
});

static DELETE_POST: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::DELETE_POST,
        RiseEndpointKind::Mutation,
    )
});

static SET_POST_LIKE: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SET_POST_LIKE,
        RiseEndpointKind::IdempotentMutation,
    )
});

static SET_POST_FAVORITE: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SET_POST_FAVORITE,
        RiseEndpointKind::IdempotentMutation,
    )
});

static SEND_POST_SPARKS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SEND_POST_SPARKS,
        RiseEndpointKind::IdempotentMutation,
    )
});

static GET_POST_SPARKS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_POST_SPARKS,
        RiseEndpointKind::Read,
    )
});

static CREATE_REPOST: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::CREATE_REPOST,
        RiseEndpointKind::IdempotentMutation,
    )
});

static DELETE_REPOST: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::DELETE_REPOST,
        RiseEndpointKind::IdempotentMutation,
    )
});

static UPDATE_REPOST: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::UPDATE_REPOST,
        RiseEndpointKind::Mutation,
    )
});

static GET_FEED: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_FEED,
        RiseEndpointKind::Read,
    )
});

static GET_SHORTS_FEED: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_SHORTS_FEED,
        RiseEndpointKind::Read,
    )
});

static GET_PUBLICATIONS_FEED: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_PUBLICATIONS_FEED,
        RiseEndpointKind::Read,
    )
});

static GET_THREADS_FEED: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_THREADS_FEED,
        RiseEndpointKind::Read,
    )
});

static GET_USER_POSTS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_USER_POSTS,
        RiseEndpointKind::Read,
    )
});

static GET_USER_PUBLICATIONS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_USER_PUBLICATIONS,
        RiseEndpointKind::Read,
    )
});

static GET_USER_RIFFS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_USER_RIFFS,
        RiseEndpointKind::Read,
    )
});

static GET_USER_THREADS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_USER_THREADS,
        RiseEndpointKind::Read,
    )
});

static GET_USER_REPOSTS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_USER_REPOSTS,
        RiseEndpointKind::Read,
    )
});

static SEARCH_POSTS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SEARCH_POSTS,
        RiseEndpointKind::Read,
    )
});

static SEARCH_PUBLICATIONS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SEARCH_PUBLICATIONS,
        RiseEndpointKind::Read,
    )
});

static SEARCH_THREADS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SEARCH_THREADS,
        RiseEndpointKind::Read,
    )
});

static SEARCH_POSTS_BY_HASHTAG: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SEARCH_POSTS_BY_HASHTAG,
        RiseEndpointKind::Read,
    )
});

static GET_POST: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_POST,
        RiseEndpointKind::Read,
    )
});

static GET_POST_STATS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_POST_STATS,
        RiseEndpointKind::Read,
    )
});

static RECORD_POST_VIEWS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::RECORD_POST_VIEWS,
        RiseEndpointKind::Mutation,
    )
});

static INVITE_COAUTHORS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::INVITE_COAUTHORS,
        RiseEndpointKind::Mutation,
    )
});

static REMOVE_COAUTHOR: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::REMOVE_COAUTHOR,
        RiseEndpointKind::Mutation,
    )
});

static ACCEPT_COAUTHOR_INVITE: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::ACCEPT_COAUTHOR_INVITE,
        RiseEndpointKind::Mutation,
    )
});

static DECLINE_COAUTHOR_INVITE: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::DECLINE_COAUTHOR_INVITE,
        RiseEndpointKind::Mutation,
    )
});

static SET_COAUTHOR_GRID_VISIBILITY: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SET_COAUTHOR_GRID_VISIBILITY,
        RiseEndpointKind::Mutation,
    )
});

static SET_POST_TAGS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::SET_POST_TAGS,
        RiseEndpointKind::Mutation,
    )
});

static REMOVE_POST_TAG: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::REMOVE_POST_TAG,
        RiseEndpointKind::Mutation,
    )
});

static GET_POST_COLLABORATORS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_POST_COLLABORATORS,
        RiseEndpointKind::Read,
    )
});

static GET_USER_MENTIONS: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_USER_MENTIONS,
        RiseEndpointKind::Read,
    )
});

static GET_USER_FAVORITES: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::GET_USER_FAVORITES,
        RiseEndpointKind::Read,
    )
});

static RECORD_POST_SHARE: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::RECORD_POST_SHARE,
        RiseEndpointKind::Mutation,
    )
});

static RECORD_POST_LINK_OPEN: LazyLock<RisePostEndpoint> = LazyLock::new(|| {
    RisePostEndpoint::new(
        rise_wire_methods::post::SERVICE,
        rise_wire_methods::post::RECORD_POST_LINK_OPEN,
        RiseEndpointKind::Mutation,
    )
});

/// one descriptor per post service method the app calls
pub enum RisePostEndpoints {}

impl RisePostEndpoints {
    pub fn create_post() -> &'static RisePostEndpoint {
        &CREATE_POST
    }

    pub fn update_post() -> &'static RisePostEndpoint {
        &UPDATE_POST
    }

    pub fn delete_post() -> &'static RisePostEndpoint {
        &DELETE_POST
    }

    pub fn set_post_like() -> &'static RisePostEndpoint {
        &SET_POST_LIKE
    }

    pub fn set_post_favorite() -> &'static RisePostEndpoint {
        &SET_POST_FAVORITE
    }

    pub fn send_post_sparks() -> &'static RisePostEndpoint {
        &SEND_POST_SPARKS
    }

    pub fn get_post_sparks() -> &'static RisePostEndpoint {
        &GET_POST_SPARKS
    }

    pub fn create_repost() -> &'static RisePostEndpoint {
        &CREATE_REPOST
    }

    pub fn delete_repost() -> &'static RisePostEndpoint {
        &DELETE_REPOST
    }

    /// editing the opinion overwrites the callers own repost so a replay converges on the same value
    /// no request id needed
    pub fn update_repost() -> &'static RisePostEndpoint {
        &UPDATE_REPOST
    }

    pub fn get_feed() -> &'static RisePostEndpoint {
        &GET_FEED
    }

    pub fn get_shorts_feed() -> &'static RisePostEndpoint {
        &GET_SHORTS_FEED
    }

    pub fn get_publications_feed() -> &'static RisePostEndpoint {
        &GET_PUBLICATIONS_FEED
    }

    pub fn get_threads_feed() -> &'static RisePostEndpoint {
        &GET_THREADS_FEED
    }

    pub fn get_user_posts() -> &'static RisePostEndpoint {
        &GET_USER_POSTS
    }

    pub fn get_user_publications() -> &'static RisePostEndpoint {
        &GET_USER_PUBLICATIONS
    }

    pub fn get_user_riffs() -> &'static RisePostEndpoint {
        &GET_USER_RIFFS
    }

    pub fn get_user_threads() -> &'static RisePostEndpoint {
        &GET_USER_THREADS
    }

    pub fn get_user_reposts() -> &'static RisePostEndpoint {
        &GET_USER_REPOSTS
    }

    pub fn search_posts() -> &'static RisePostEndpoint {
        &SEARCH_POSTS
    }

    pub fn search_publications() -> &'static RisePostEndpoint {
        &SEARCH_PUBLICATIONS
    }

    pub fn search_threads() -> &'static RisePostEndpoint {
        &SEARCH_THREADS
    }

    pub fn search_posts_by_hashtag() -> &'static RisePostEndpoint {
        &SEARCH_POSTS_BY_HASHTAG
    }

    pub fn get_post() -> &'static RisePostEndpoint {
        &GET_POST
    }

    pub fn get_post_stats() -> &'static RisePostEndpoint {
        &GET_POST_STATS
    }

    pub fn record_post_views() -> &'static RisePostEndpoint {
        &RECORD_POST_VIEWS
    }

    pub fn invite_coauthors() -> &'static RisePostEndpoint {
        &INVITE_COAUTHORS
    }

    pub fn remove_coauthor() -> &'static RisePostEndpoint {
        &REMOVE_COAUTHOR
    }

    pub fn accept_coauthor_invite() -> &'static RisePostEndpoint {
        &ACCEPT_COAUTHOR_INVITE
    }

    pub fn decline_coauthor_invite() -> &'static RisePostEndpoint {
        &DECLINE_COAUTHOR_INVITE
    }

    pub fn set_coauthor_grid_visibility() -> &'static RisePostEndpoint {
        &SET_COAUTHOR_GRID_VISIBILITY
    }

    pub fn set_post_tags() -> &'static RisePostEndpoint {
        &SET_POST_TAGS
    }

    pub fn remove_post_tag() -> &'static RisePostEndpoint {
        &REMOVE_POST_TAG
    }

    pub fn get_post_collaborators() -> &'static RisePostEndpoint {
        &GET_POST_COLLABORATORS
    }

    pub fn get_user_mentions() -> &'static RisePostEndpoint {
        &GET_USER_MENTIONS
    }

    pub fn get_user_favorites() -> &'static RisePostEndpoint {
        &GET_USER_FAVORITES
    }

    pub fn record_post_share() -> &'static RisePostEndpoint {
        &RECORD_POST_SHARE
    }

    pub fn record_post_link_open() -> &'static RisePostEndpoint {
        &RECORD_POST_LINK_OPEN
    }

    pub fn all() -> Vec<&'static RisePostEndpoint> {
        vec![
            Self::create_post(),
            Self::update_post(),
            Self::delete_post(),
            Self::set_post_like(),
            Self::set_post_favorite(),
            Self::send_post_sparks(),
            Self::get_post_sparks(),
            Self::create_repost(),
            Self::delete_repost(),
            Self::update_repost(),
            Self::get_feed(),
            Self::get_publications_feed(),
            Self::get_threads_feed(),
            Self::get_user_posts(),
            Self::get_user_publications(),
            Self::get_user_riffs(),
            Self::get_user_threads(),
            Self::get_user_reposts(),
            Self::search_posts(),
            Self::search_publications(),
            Self::search_threads(),
            Self::search_posts_by_hashtag(),
            Self::get_post(),
            Self::get_post_stats(),
            Self::record_post_views(),
            Self::record_post_share(),
            Self::record_post_link_open(),
            Self::invite_coauthors(),
            Self::remove_coauthor(),
            Self::accept_coauthor_invite(),
            Self::decline_coauthor_invite(),
            Self::set_coauthor_grid_visibility(),
            Self::set_post_tags(),
            Self::remove_post_tag(),
            Self::get_post_collaborators(),
            Self::get_user_mentions(),
            Self::get_user_favorites(),
        ]
    }

    pub fn feed(kind: RisePostFeedKind) -> &'static RisePostEndpoint {
        match kind {
            RisePostFeedKind::All => Self::get_feed(),
            RisePostFeedKind::Publications => Self::get_publications_feed(),
            RisePostFeedKind::Threads => Self::get_threads_feed(),
        }
    }

    pub fn profile(kind: RisePostProfileKind) -> &'static RisePostEndpoint {
        match kind {
            RisePostProfileKind::Publications => Self::get_user_publications(),
            RisePostProfileKind::Riffs => Self::get_user_riffs(),
            RisePostProfileKind::Threads => Self::get_user_threads(),
            RisePostProfileKind::Reposts => Self::get_user_reposts(),
            RisePostProfileKind::Mentions => Self::get_user_mentions(),
            RisePostProfileKind::Favorites => Self::get_user_favorites(),
        }
    }

    pub fn search(kind: RisePostSearchKind) -> &'static RisePostEndpoint {
        match kind {
            RisePostSearchKind::Publications => Self::search_publications(),
            RisePostSearchKind::Threads => Self::search_threads(),
        }
    }
}

#[derive(Clone)]
pub struct RisePostRPC {
    pub engine: Arc<RiseEngine>,
}

impl RisePostRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn attested_call<Request, Response>(
        &self,
        endpoint: &RisePostEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        let body = serde_json::to_vec(request)?;
        let descriptor = endpoint.descriptor();
        let headers = AppAttestClient::request_headers(&descriptor.service, &descriptor.method, &body).await;
        if headers.is_empty() {
            return self.call(endpoint, request).await;
        }
        let started_at = Date::now();
        RisePostDebugTrace::network_started(endpoint);
        let result: anyhow::Result<Response> = async {
            let payload = self
                .engine
                .call_descriptor(
                    &descriptor,
                    &RiseJSONValue::String(String::from_utf8_lossy(&body).into_owned()),
                    None,
                    RiseWireRequestMetadata {
                        headers: headers.into_iter().collect::<BTreeMap<_, _>>(),
                        ..RiseWireRequestMetadata::default()
                    },
                )
                .await?;
            payload
                .decode_as::<Response>()
                .map_err(|_| anyhow::Error::from(RiseWireError::ResponseDecodingFailed))
        }
        .await;
        RisePostDebugTrace::network_finished(endpoint, started_at, result.as_ref().err());
        result
    }

    pub async fn call<Request, Response>(
        &self,
        endpoint: &RisePostEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        let started_at = Date::now();
        RisePostDebugTrace::network_started(endpoint);
        let result = self
            .engine
            .call_descriptor_response::<Request, Response>(
                &endpoint.descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await;
        RisePostDebugTrace::network_finished(endpoint, started_at, result.as_ref().err());
        result
    }
}

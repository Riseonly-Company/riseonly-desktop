use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::modules::comment::stores::comment_actions::comment_wire_types::{
    CreateCommentGatewayDTO, GetCommentRepliesGatewayDTO, GetCommentsGatewayDTO, GetUserCommentsGatewayDTO,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseCommentEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub is_read: bool,
}

impl RiseCommentEndpoint {
    /// deadline and replay policy come from the canon catalogue, expected only asserts the
    /// declared intent
    #[track_caller]
    pub fn new(method: &str, expected: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(rise_wire_methods::comment::SERVICE, method);
        debug_assert!(
            entry.kind == expected,
            "comment.{method}: canon declares {}, not {}",
            entry.kind.name(),
            expected.name()
        );
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            is_read: entry.kind == RiseEndpointKind::Read,
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::comment::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(if self.is_read { RiseWireReplayPolicy::ReadOnly } else { RiseWireReplayPolicy::Never })
    }
}

static GET_COMMENTS: LazyLock<RiseCommentEndpoint> =
    LazyLock::new(|| RiseCommentEndpoint::new(rise_wire_methods::comment::GET_COMMENTS, RiseEndpointKind::Read));
static GET_COMMENT_REPLIES: LazyLock<RiseCommentEndpoint> =
    LazyLock::new(|| RiseCommentEndpoint::new(rise_wire_methods::comment::GET_COMMENT_REPLIES, RiseEndpointKind::Read));
static GET_USER_COMMENTS: LazyLock<RiseCommentEndpoint> =
    LazyLock::new(|| RiseCommentEndpoint::new(rise_wire_methods::comment::GET_USER_COMMENTS, RiseEndpointKind::Read));
static CREATE_COMMENT: LazyLock<RiseCommentEndpoint> =
    LazyLock::new(|| RiseCommentEndpoint::new(rise_wire_methods::comment::CREATE_COMMENT, RiseEndpointKind::Mutation));
static DELETE_COMMENT: LazyLock<RiseCommentEndpoint> =
    LazyLock::new(|| RiseCommentEndpoint::new(rise_wire_methods::comment::DELETE_COMMENT, RiseEndpointKind::Mutation));
static LIKE_COMMENT: LazyLock<RiseCommentEndpoint> =
    LazyLock::new(|| RiseCommentEndpoint::new(rise_wire_methods::comment::LIKE_COMMENT, RiseEndpointKind::Mutation));
static DISLIKE_COMMENT: LazyLock<RiseCommentEndpoint> =
    LazyLock::new(|| RiseCommentEndpoint::new(rise_wire_methods::comment::DISLIKE_COMMENT, RiseEndpointKind::Mutation));

pub enum RiseCommentEndpoints {}

impl RiseCommentEndpoints {
    pub fn get_comments() -> &'static RiseCommentEndpoint {
        &GET_COMMENTS
    }

    pub fn get_comment_replies() -> &'static RiseCommentEndpoint {
        &GET_COMMENT_REPLIES
    }

    pub fn get_user_comments() -> &'static RiseCommentEndpoint {
        &GET_USER_COMMENTS
    }

    pub fn create_comment() -> &'static RiseCommentEndpoint {
        &CREATE_COMMENT
    }

    pub fn delete_comment() -> &'static RiseCommentEndpoint {
        &DELETE_COMMENT
    }

    pub fn like_comment() -> &'static RiseCommentEndpoint {
        &LIKE_COMMENT
    }

    pub fn dislike_comment() -> &'static RiseCommentEndpoint {
        &DISLIKE_COMMENT
    }
}

#[derive(Clone)]
pub struct RiseCommentRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseCommentRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn get_comments(
        &self,
        request: &rise_request::comment::GetComments,
    ) -> anyhow::Result<GetCommentsGatewayDTO> {
        self.call(RiseCommentEndpoints::get_comments(), request).await
    }

    pub async fn get_comment_replies(
        &self,
        request: &rise_request::comment::GetCommentReplies,
    ) -> anyhow::Result<GetCommentRepliesGatewayDTO> {
        self.call(RiseCommentEndpoints::get_comment_replies(), request).await
    }

    pub async fn get_user_comments(
        &self,
        request: &rise_request::comment::GetUserComments,
    ) -> anyhow::Result<GetUserCommentsGatewayDTO> {
        self.call(RiseCommentEndpoints::get_user_comments(), request).await
    }

    pub async fn create_comment(
        &self,
        request: &rise_request::comment::CreateComment,
    ) -> anyhow::Result<CreateCommentGatewayDTO> {
        self.call(RiseCommentEndpoints::create_comment(), request).await
    }

    pub async fn delete_comment(
        &self,
        request: &rise_request::comment::DeleteComment,
    ) -> anyhow::Result<rise_response::comment::DeleteComment> {
        self.call(RiseCommentEndpoints::delete_comment(), request).await
    }

    pub async fn like_comment(
        &self,
        request: &rise_request::comment::LikeComment,
    ) -> anyhow::Result<rise_response::comment::LikeComment> {
        self.call(RiseCommentEndpoints::like_comment(), request).await
    }

    pub async fn dislike_comment(
        &self,
        request: &rise_request::comment::DislikeComment,
    ) -> anyhow::Result<rise_response::comment::DislikeComment> {
        self.call(RiseCommentEndpoints::dislike_comment(), request).await
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseCommentEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        self.engine
            .call_descriptor_response::<Request, Response>(
                &endpoint.descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }
}

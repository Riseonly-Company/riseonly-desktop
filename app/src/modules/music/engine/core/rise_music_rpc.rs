use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMusicEndpointReplay {
    ReadOnly,
    Never,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMusicEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseMusicEndpointReplay,
}

impl RiseMusicEndpoint {
    const SERVICE: &'static str = rise_wire_methods::music::SERVICE;

    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

    /// pass DEFAULT_TIMEOUT for the default timeout
    pub fn new(method: &str, timeout: Duration, replay: RiseMusicEndpointReplay) -> Self {
        Self {
            method: method.to_owned(),
            timeout,
            replay,
        }
    }

    /// descriptor from the canon catalog, timeout and replay policy come from there
    /// expecting only checks the declared intent
    #[track_caller]
    pub fn canonical(method: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(Self::SERVICE, method);
        debug_assert!(
            entry.kind == expecting,
            "{}.{method}: the canon declares {}, not {}",
            Self::SERVICE,
            entry.kind.name(),
            expecting.name()
        );
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            replay: if entry.kind == RiseEndpointKind::Read {
                RiseMusicEndpointReplay::ReadOnly
            } else {
                RiseMusicEndpointReplay::Never
            },
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(Self::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(match self.replay {
                RiseMusicEndpointReplay::ReadOnly => RiseWireReplayPolicy::ReadOnly,
                RiseMusicEndpointReplay::Never => RiseWireReplayPolicy::Never,
            })
    }
}

static GET_TRACK: LazyLock<RiseMusicEndpoint> = LazyLock::new(|| {
    RiseMusicEndpoint::canonical(rise_wire_methods::music::GET_TRACK, RiseEndpointKind::Read)
});

static UPDATE_LOCAL_TRACK: LazyLock<RiseMusicEndpoint> = LazyLock::new(|| {
    RiseMusicEndpoint::canonical(rise_wire_methods::music::UPDATE_LOCAL_TRACK, RiseEndpointKind::Mutation)
});

pub enum RiseMusicEndpoints {}

impl RiseMusicEndpoints {
    pub fn get_track() -> &'static RiseMusicEndpoint {
        &GET_TRACK
    }

    pub fn update_local_track() -> &'static RiseMusicEndpoint {
        &UPDATE_LOCAL_TRACK
    }
}

#[derive(Clone)]
pub struct RiseMusicRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseMusicRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn get_track(&self, track_id: &str) -> anyhow::Result<Option<RiseStoredMusicTrack>> {
        let request = rise_request::music::GetTrack {
            track_id: Some(track_id.to_owned()),
        };
        let response: rise_response::music::GetTrack =
            self.call(RiseMusicEndpoints::get_track(), &request).await?;
        Ok(response.track.as_ref().map(RiseStoredMusicTrack::from_canon))
    }

    pub async fn update_local_track(
        &self,
        request: rise_request::music::UpdateLocalTrack,
    ) -> anyhow::Result<Option<RiseStoredMusicTrack>> {
        let response: rise_response::music::UpdateLocalTrack =
            self.call(RiseMusicEndpoints::update_local_track(), &request).await?;
        Ok(response.track.as_ref().map(RiseStoredMusicTrack::from_canon))
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseMusicEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: serde::Serialize + Sync,
        Response: serde::de::DeserializeOwned,
    {
        self.engine
            .call_descriptor_response(
                &endpoint.descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }
}

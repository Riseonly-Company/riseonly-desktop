use std::rc::Rc;
use std::sync::Arc;

use futures::future::{BoxFuture, LocalBoxFuture};
use gpui::{App, AsyncApp};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::share_extension::entry::app_group_share_send_handoff::AppGroupShareSendHandoff;
use crate::modules::share_extension::entry::app_group_share_story_handoff::AppGroupShareStoryHandoff;
use crate::modules::share_extension::entry::rise_share_api_client::RiseShareAPIClient;
use crate::modules::share_extension::entry::share_extension_configuration::ShareExtensionConfiguration;
use crate::modules::share_extension::entry::share_extension_models::{
    ShareAccountSnapshot, ShareDestination, ShareExtensionError, SharePayload, ShareSendProgress,
    ShareSendRequest, ShareTopic,
};
use crate::modules::share_extension::entry::share_extension_snapshot_store::ShareExtensionSnapshotStore;
use crate::modules::share_extension::entry::share_temporary_file_store::ShareTemporaryFileStore;
use crate::modules::share_extension::shared::rise_share_container::RiseShareContainer;
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareCredential, RiseShareSnapshot, RiseShareTopicStructure,
};

pub trait ShareExtensionSnapshotProviding: Send + Sync {
    fn account(&self) -> BoxFuture<'_, anyhow::Result<ShareAccountSnapshot>>;
    fn recent_destinations(&self, limit: i64) -> BoxFuture<'_, anyhow::Result<Vec<ShareDestination>>>;
    fn search_destinations(
        &self,
        query: String,
        limit: i64,
    ) -> BoxFuture<'_, anyhow::Result<Vec<ShareDestination>>>;
}

/// one page of share destinations, most recent chats first
#[derive(Clone, Debug, PartialEq)]
pub struct ShareDestinationPage {
    pub destinations: Vec<ShareDestination>,
    pub next_offset: Option<i64>,
}

pub trait ShareExtensionRemoteProviding {
    fn destinations(
        &self,
        query: Option<String>,
        payload: SharePayload,
        limit: i64,
        offset: i64,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'_, anyhow::Result<ShareDestinationPage>>;

    fn topics(
        &self,
        destination_id: String,
        payload: SharePayload,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'_, anyhow::Result<RiseShareTopicStructure<ShareTopic>>>;
}

pub type ShareSendCommitStarted = Rc<dyn Fn(&mut App)>;

pub type ShareSendProgressHandler = Rc<dyn Fn(ShareSendProgress, &mut App)>;

pub trait ShareContentSending {
    fn send(
        &self,
        request: ShareSendRequest,
        on_commit_started: ShareSendCommitStarted,
        progress: ShareSendProgressHandler,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'_, anyhow::Result<()>>;

    fn discard_cached_uploads(&self, idempotency_key: Uuid) -> LocalBoxFuture<'_, ()>;
}

pub trait ShareStoryHandling {
    fn handoff(&self, payload: SharePayload, cx: AsyncApp) -> LocalBoxFuture<'_, anyhow::Result<()>>;
}

pub trait ShareLargeSendHandling {
    fn handoff(&self, request: ShareSendRequest, cx: AsyncApp) -> LocalBoxFuture<'_, anyhow::Result<()>>;
    fn cancel_pending_handoff(&self, idempotency_key: Uuid) -> LocalBoxFuture<'_, ()>;
}

/// opens the app side, on desktop it wakes the send host or opens the story editor in the same process
pub trait ShareContainingAppOpening {
    fn open_containing_app(&self, url: Url, cx: AsyncApp) -> LocalBoxFuture<'_, bool>;
}

/// bundle of the share layer dependencies
#[derive(Clone)]
pub struct ShareExtensionDependencies {
    pub snapshot: Arc<dyn ShareExtensionSnapshotProviding>,
    pub remote: Rc<dyn ShareExtensionRemoteProviding>,
    pub sender: Rc<dyn ShareContentSending>,
    pub story_handoff: Rc<dyn ShareStoryHandling>,
    pub large_send_handoff: Rc<dyn ShareLargeSendHandling>,
}

pub enum ShareExtensionDependencyResolver {}

impl ShareExtensionDependencyResolver {
    /// builds the dependencies, no extension context on desktop since the layer lives in the app window
    pub fn resolve(
        configuration: ShareExtensionConfiguration,
        app_opener: &Rc<dyn ShareContainingAppOpening>,
        temporary_file_store: Arc<ShareTemporaryFileStore>,
        cx: &mut App,
    ) -> Result<ShareExtensionDependencies, ShareExtensionError> {
        let Some(credential) = Self::session_credential(cx) else {
            return Err(ShareExtensionError::AuthenticationUnavailable);
        };

        let shared_snapshot = RiseShareContainer::load_snapshot()
            .map_err(|_| ShareExtensionError::SnapshotUnavailable)?;

        if shared_snapshot.schema_version != RiseShareSnapshot::CURRENT_SCHEMA_VERSION {
            return Err(ShareExtensionError::SnapshotInvalid);
        }

        Self::validate_account(&shared_snapshot, &credential)?;

        let snapshot_store = ShareExtensionSnapshotStore::new(shared_snapshot.clone());
        let api_client = Rc::new(RiseShareAPIClient::new(
            shared_snapshot.clone(),
            credential,
            configuration.clone(),
        ));
        let story_handoff = AppGroupShareStoryHandoff::new(
            configuration.clone(),
            shared_snapshot.account.id.clone(),
            app_opener,
            temporary_file_store,
        );
        let large_send_handoff = AppGroupShareSendHandoff::new(
            configuration,
            shared_snapshot.account.id.clone(),
            app_opener,
        );

        Ok(ShareExtensionDependencies {
            snapshot: Arc::new(snapshot_store),
            remote: api_client.clone(),
            sender: api_client,
            story_handoff: Rc::new(story_handoff),
            large_send_handoff: Rc::new(large_send_handoff),
        })
    }

    pub fn validate_account(
        snapshot: &RiseShareSnapshot,
        credential: &RiseShareCredential,
    ) -> Result<(), ShareExtensionError> {
        let snapshot_id = trimming_whitespaces_and_newlines(&snapshot.account.id);
        let account_id = trimming_whitespaces_and_newlines(&credential.account_id);
        let user_id = trimming_whitespaces_and_newlines(&credential.user_id);
        if snapshot_id.is_empty() || snapshot_id != account_id || snapshot_id != user_id {
            return Err(ShareExtensionError::AccountChanged);
        }
        Ok(())
    }

    pub fn session_credential(cx: &mut App) -> Option<RiseShareCredential> {
        let store = AuthServicesStore::shared(cx);
        let auth = store.read(cx);
        if !auth.is_authenticated() {
            return None;
        }
        let account_id = auth.active_account_id()?.to_owned();
        let user_id = auth.effective_user_id_for_wire()?;
        let tokens = auth.tokens_and_other_data();
        if account_id.is_empty()
            || user_id.is_empty()
            || tokens.access_token.is_empty()
            || tokens.refresh_token.is_empty()
        {
            return None;
        }
        Some(RiseShareCredential {
            account_id,
            user_id,
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            session_id: tokens.session_id.clone(),
            updated_at_ms: (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64,
        })
    }
}

use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::{FutureExt, LocalBoxFuture};
use gpui::AsyncApp;
use rise_canon::rise_key::events;
use rise_core::foundation::foundation_url::{UrlComponents, UrlQueryItem};
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::modules::share_extension::entry::share_extension_configuration::ShareExtensionConfiguration;
use crate::modules::share_extension::entry::share_extension_dependencies::{
    ShareContainingAppOpening, ShareStoryHandling,
};
use crate::modules::share_extension::entry::share_extension_models::{
    ShareAttachmentKind, ShareExtensionError, SharePayload,
};
use crate::modules::share_extension::entry::share_temporary_file_store::ShareTemporaryFileStore;
use crate::modules::share_extension::shared::rise_share_container::{
    RiseShareContainer, RiseShareContainerError, RiseShareStorySourceItem,
};
use crate::modules::share_extension::shared::rise_share_models::RiseShareStoryMediaKind;

const SHARE_STORY_HOST: &str = "share-story";

pub struct AppGroupShareStoryHandoff {
    configuration: ShareExtensionConfiguration,
    account_id: String,
    app_opener: Weak<dyn ShareContainingAppOpening>,
    temporary_file_store: Arc<ShareTemporaryFileStore>,
}

struct ShareStoryCopyCancellation {
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

impl Drop for ShareStoryCopyCancellation {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

impl AppGroupShareStoryHandoff {
    /// builds the story handoff
    pub fn new(
        configuration: ShareExtensionConfiguration,
        account_id: String,
        app_opener: &Rc<dyn ShareContainingAppOpening>,
        temporary_file_store: Arc<ShareTemporaryFileStore>,
    ) -> Self {
        Self {
            configuration,
            account_id,
            app_opener: Rc::downgrade(app_opener),
            temporary_file_store,
        }
    }

    async fn handoff_payload(&self, payload: SharePayload, cx: AsyncApp) -> anyhow::Result<()> {
        if !payload.can_share_to_story() {
            return Err(ShareExtensionError::StoryUnavailable.into());
        }
        let source_items = payload
            .attachments
            .iter()
            .map(|attachment| {
                let Some(url) = &attachment.local_file_url else {
                    return Err(ShareExtensionError::StoryUnavailable);
                };
                let kind = if attachment.kind == ShareAttachmentKind::Image {
                    RiseShareStoryMediaKind::Image
                } else {
                    RiseShareStoryMediaKind::Video
                };
                let filename = attachment.original_file_name.clone().unwrap_or_else(|| {
                    url.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
                Ok((url.clone(), kind, filename, attachment.mime_type.clone()))
            })
            .collect::<Result<Vec<RiseShareStorySourceItem>, _>>()?;

        let cancelled = Arc::new(AtomicBool::new(false));
        let is_cancelled = cancelled.clone();
        let account_id = self.account_id.clone();
        let copy_task = RiseAccountRuntime::current().spawn_blocking(move || {
            let session = RiseShareContainer::create_story_session(None, &account_id, &source_items, &|| {
                is_cancelled.load(Ordering::SeqCst)
            })?;
            if is_cancelled.load(Ordering::SeqCst) {
                RiseShareContainer::remove_story_session(&session.id);
                return Err(RiseShareContainerError::Cancelled(CancellationError));
            }
            Ok(session)
        });
        let mut cancellation = ShareStoryCopyCancellation { cancelled, armed: true };
        let copied = copy_task.await;
        cancellation.armed = false;
        drop(cancellation);
        let session = copied.map_err(|_| CancellationError)??;

        let mut components = UrlComponents::new();
        components.set_scheme(Some(&self.configuration.story_url_scheme));
        components.set_host(Some(SHARE_STORY_HOST));
        components.set_query_items(Some(vec![UrlQueryItem::new(
            events::SESSION_ID,
            Some(session.id.clone()),
        )]));
        let (Some(url), Some(app_opener)) = (components.url(), self.app_opener.upgrade()) else {
            Self::remove_story_session(session.id);
            return Err(ShareExtensionError::AppOpenFailed.into());
        };

        let opened = app_opener.open_containing_app(url, cx).await;
        if !opened {
            Self::remove_story_session(session.id);
            return Err(ShareExtensionError::AppOpenFailed.into());
        }

        let temporary_file_store = self.temporary_file_store.clone();
        let _ = RiseAccountRuntime::current()
            .spawn_blocking(move || temporary_file_store.remove_session_files())
            .await;
        Ok(())
    }

    fn remove_story_session(id: String) {
        drop(RiseAccountRuntime::current().spawn_blocking(move || RiseShareContainer::remove_story_session(&id)));
    }
}

impl ShareStoryHandling for AppGroupShareStoryHandoff {
    fn handoff(&self, payload: SharePayload, cx: AsyncApp) -> LocalBoxFuture<'_, anyhow::Result<()>> {
        self.handoff_payload(payload, cx).boxed_local()
    }
}

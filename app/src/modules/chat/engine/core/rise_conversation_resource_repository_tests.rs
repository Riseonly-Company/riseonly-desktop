use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_core::rise_secure_key_store::{RiseEngineKeyStore, RiseEngineKeyStoreFuture};
use rise_engine::rise_wire::rise_wire::{RiseWireAccountContext, RiseWireClosureSender, RiseWireSocketSession};
use rise_engine::{RiseEngine, RiseEngineConfiguration, RiseEngineConfigurationArgs};
use rise_platform::file_manager::FileManager;

use crate::modules::chat::engine::core::rise_conversation_resource_repository::{
    RiseConversationResourceKey, RiseConversationResourceReadSource, RiseConversationResourceRepository,
};

#[derive(Default)]
struct RiseResourceTraceTestKeyStore {
    keys: Mutex<HashMap<String, Vec<u8>>>,
}

impl RiseEngineKeyStore for RiseResourceTraceTestKeyStore {
    fn key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(self.keys.lock().unwrap_or_else(PoisonError::into_inner).get(account_id).cloned()) })
    }

    fn create_key_if_absent<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(keys.entry(account_id.to_owned()).or_insert_with(|| vec![0x71; 32]).clone())
        })
    }

    fn delete_key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        Box::pin(async move {
            self.keys.lock().unwrap_or_else(PoisonError::into_inner).remove(account_id);
            Ok(())
        })
    }
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = FileManager::default().remove_item(&self.0);
    }
}

#[tokio::test]
async fn reports_miss_memory_and_persistent_store_without_guessing() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!("rise-resource-source-{}", Uuid::new_v4().uuid_string()));
    let _cleanup = RemoveOnDrop(root.clone());
    let account_id = format!("resource-source-{}", Uuid::new_v4().uuid_string());
    let engine = RiseEngine::open(RiseEngineConfiguration::new(
        root,
        RiseWireAccountContext::new(account_id, "test".to_owned()),
        Arc::new(RiseWireClosureSender::new(Arc::new(
            |_frame: Vec<u8>, _session: RiseWireSocketSession| -> BoxFuture<'static, anyhow::Result<()>> {
                async { Ok(()) }.boxed()
            },
        ))),
        RiseEngineConfigurationArgs {
            key_store: Arc::new(RiseResourceTraceTestKeyStore::default()),
            ..RiseEngineConfigurationArgs::default()
        },
    ))
    .await?;
    let repository = RiseConversationResourceRepository::new(engine.clone());
    let key = RiseConversationResourceKey::new(["tests", "topics"]);

    let missing = repository.cached::<String>(&key).await?;
    assert_eq!(missing.value, None);
    assert_eq!(missing.read_source, RiseConversationResourceReadSource::Miss);

    repository.store(&"cached-topics".to_owned(), &key).await?;
    let memory = repository.cached::<String>(&key).await?;
    assert_eq!(memory.value.as_deref(), Some("cached-topics"));
    assert_eq!(memory.read_source, RiseConversationResourceReadSource::Memory);

    repository.clear_memory().await;
    let persistent = repository.cached::<String>(&key).await?;
    assert_eq!(persistent.value.as_deref(), Some("cached-topics"));
    assert_eq!(persistent.read_source, RiseConversationResourceReadSource::PersistentStore);

    engine.close().await?;
    Ok(())
}

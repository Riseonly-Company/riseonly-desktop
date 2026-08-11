use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use rusqlite::Connection;
use thiserror::Error;
use tokio::sync::oneshot;

use super::rise_sqlite::{StoreOpenError, open_connection, open_in_memory, prepare};
use super::rise_store_cipher::ValueCipher;
use super::rise_store_writes::{WriteBatch, WriteError};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("store is closed")]
    Closed,
    #[error("open: {0}")]
    Open(#[from] StoreOpenError),
    #[error("write: {0}")]
    Write(#[from] WriteError),
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

enum Command {
    Run(Job),
    Close(oneshot::Sender<()>),
}

/// A handle onto the one thread that owns the database connection.
///
/// Every read and write is a message, so work lands in send order. Reads are
/// serialized with writes: a long batch delays reads queued behind it.
#[derive(Clone)]
pub struct StoreHandle {
    commands: Sender<Command>,
    inner: Arc<StoreThread>,
}

struct StoreThread {
    handle: parking_lot::Mutex<Option<JoinHandle<()>>>,
}

pub struct StoreConfig {
    pub account_id: String,
    pub path: Option<PathBuf>,
    pub cipher: Arc<dyn ValueCipher>,
}

impl StoreHandle {
    pub fn open(config: StoreConfig) -> Result<Self, StoreError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (commands, jobs) = channel::<Command>();

        let account_id = config.account_id.clone();
        let cipher = Arc::clone(&config.cipher);
        let path = config.path.clone();

        let thread = std::thread::Builder::new()
            .name(format!("rise-store:{account_id}"))
            .spawn(move || {
                let opened = match path {
                    Some(path) => open_connection(&path),
                    None => open_in_memory(),
                };

                let mut connection = match opened.and_then(|connection| {
                    prepare(&connection, &account_id, cipher.as_ref())?;
                    Ok(connection)
                }) {
                    Ok(connection) => {
                        let _ = ready_tx.send(Ok(()));
                        connection
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };

                run_loop(&mut connection, jobs);
            })
            .expect("failed to spawn the store thread");

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(StoreError::Open(error)),
            Err(_) => return Err(StoreError::Closed),
        }

        Ok(Self {
            commands,
            inner: Arc::new(StoreThread {
                handle: parking_lot::Mutex::new(Some(thread)),
            }),
        })
    }

    /// Runs arbitrary work against the connection on the store thread.
    pub async fn with_connection<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> T + Send + 'static,
    {
        let (respond, response) = oneshot::channel();

        self.commands
            .send(Command::Run(Box::new(move |connection| {
                let _ = respond.send(work(connection));
            })))
            .map_err(|_| StoreError::Closed)?;

        response.await.map_err(|_| StoreError::Closed)
    }

    pub async fn commit(
        &self,
        batch: WriteBatch,
        account_id: String,
        cipher: Arc<dyn ValueCipher>,
        now_ms: i64,
    ) -> Result<usize, StoreError> {
        self.with_connection(move |connection| {
            batch.commit(connection, &account_id, cipher.as_ref(), now_ms)
        })
        .await?
        .map_err(StoreError::from)
    }

    /// Drains outstanding work and joins the thread. Must be awaited before
    /// another store opens the same file; Drop does not do this.
    pub async fn close(&self) -> Result<(), StoreError> {
        let (respond, response) = oneshot::channel();
        if self.commands.send(Command::Close(respond)).is_err() {
            return Ok(());
        }
        let _ = response.await;

        if let Some(handle) = self.inner.handle.lock().take() {
            let _ = handle.join();
        }
        Ok(())
    }
}

fn run_loop(connection: &mut Connection, jobs: Receiver<Command>) {
    while let Ok(command) = jobs.recv() {
        match command {
            Command::Run(job) => job(connection),
            Command::Close(respond) => {
                let _ = respond.send(());
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rise_store::rise_store_cipher::PlaintextCipher;
    use crate::rise_store::rise_store_models::Namespace;

    fn config() -> StoreConfig {
        StoreConfig {
            account_id: "u1".into(),
            path: None,
            cipher: Arc::new(PlaintextCipher),
        }
    }

    fn ns() -> Namespace {
        Namespace::new("test.v1")
    }

    #[tokio::test]
    async fn a_store_opens_and_answers_a_read() {
        let store = StoreHandle::open(config()).unwrap();

        let version: i64 = store
            .with_connection(|connection| {
                connection
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .unwrap()
            })
            .await
            .unwrap();

        assert_eq!(version, crate::rise_store::rise_sqlite::SCHEMA_VERSION);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_committed_batch_is_visible_to_the_next_read() {
        let store = StoreHandle::open(config()).unwrap();
        let cipher: Arc<dyn ValueCipher> = Arc::new(PlaintextCipher);

        let mut batch = WriteBatch::new();
        batch.put_value(&ns(), "k", b"v".to_vec());
        store
            .commit(batch, "u1".into(), Arc::clone(&cipher), 0)
            .await
            .unwrap();

        let count: i64 = store
            .with_connection(|connection| {
                connection
                    .query_row("SELECT count(*) FROM key_values", [], |row| row.get(0))
                    .unwrap()
            })
            .await
            .unwrap();

        assert_eq!(count, 1);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn writes_are_applied_in_the_order_they_were_sent() {
        let store = StoreHandle::open(config()).unwrap();
        let cipher: Arc<dyn ValueCipher> = Arc::new(PlaintextCipher);

        for index in 0..50i64 {
            let mut batch = WriteBatch::new();
            batch.put_value(&ns(), "counter", index.to_string().into_bytes());
            store
                .commit(batch, "u1".into(), Arc::clone(&cipher), index)
                .await
                .unwrap();
        }

        let (value, version): (Vec<u8>, i64) = store
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT value, version FROM key_values WHERE key = 'counter'",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap()
            })
            .await
            .unwrap();

        assert_eq!(value, b"49");
        assert_eq!(
            version, 50,
            "every write must have been applied exactly once"
        );
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_callers_are_serialized_rather_than_interleaved() {
        let store = StoreHandle::open(config()).unwrap();
        let cipher: Arc<dyn ValueCipher> = Arc::new(PlaintextCipher);

        let tasks: Vec<_> = (0..32i64)
            .map(|index| {
                let store = store.clone();
                let cipher = Arc::clone(&cipher);
                tokio::spawn(async move {
                    let mut batch = WriteBatch::new();
                    batch.put_value(&ns(), format!("key-{index}"), vec![index as u8]);
                    store.commit(batch, "u1".into(), cipher, index).await
                })
            })
            .collect();

        for task in tasks {
            task.await.unwrap().unwrap();
        }

        let count: i64 = store
            .with_connection(|connection| {
                connection
                    .query_row("SELECT count(*) FROM key_values", [], |row| row.get(0))
                    .unwrap()
            })
            .await
            .unwrap();

        assert_eq!(count, 32);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_closed_store_refuses_further_work_instead_of_hanging() {
        let store = StoreHandle::open(config()).unwrap();
        store.close().await.unwrap();

        let result = store.with_connection(|_| ()).await;
        assert!(matches!(result, Err(StoreError::Closed)));
    }

    #[tokio::test]
    async fn closing_twice_is_harmless() {
        let store = StoreHandle::open(config()).unwrap();
        store.close().await.unwrap();
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn opening_with_the_wrong_key_fails_at_open_not_on_first_read() {
        use crate::rise_store::rise_store_cipher::AesGcmCipher;

        let dir = std::env::temp_dir().join("rise-store-key-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("engine.sqlite");
        std::fs::remove_file(&path).ok();

        let right: Arc<dyn ValueCipher> = Arc::new(AesGcmCipher::new(&[1u8; 32]).unwrap());
        let store = StoreHandle::open(StoreConfig {
            account_id: "u1".into(),
            path: Some(path.clone()),
            cipher: Arc::clone(&right),
        })
        .unwrap();
        store.close().await.unwrap();

        let wrong: Arc<dyn ValueCipher> = Arc::new(AesGcmCipher::new(&[2u8; 32]).unwrap());
        let result = StoreHandle::open(StoreConfig {
            account_id: "u1".into(),
            path: Some(path.clone()),
            cipher: wrong,
        });

        assert!(matches!(result, Err(StoreError::Open(_))));
        std::fs::remove_file(&path).ok();
    }
}

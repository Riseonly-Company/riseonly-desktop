use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use rise_canon::rise_upload;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_platform::AppPaths;
use rise_platform::file_manager::{CocoaError, DirectoryEnumerationOptions, FileManager};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::config::app_environment::AppEnvironment;
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareSendAttachment, RiseShareSendJSONValue, RiseShareSendSession, RiseShareSnapshot,
    RiseShareStoryItem, RiseShareStoryMediaKind, RiseShareStoryPublishCheckpoint,
    RiseShareStorySession,
};

/// errors of the share container, last four wrap file system and json and cancel errors
#[derive(Debug, thiserror::Error)]
pub enum RiseShareContainerError {
    #[error("The shared RiseOnly container is unavailable.")]
    AppGroupUnavailable,
    #[error("The shared story session identifier is invalid.")]
    InvalidSessionIdentifier,
    #[error("The shared story session is invalid or expired.")]
    InvalidManifest,
    #[error("A shared story item contains an unsafe path.")]
    UnsafeRelativePath,
    #[error("The shared session belongs to another RiseOnly account.")]
    AccountMismatch,
    #[error("A shared session contains an invalid file.")]
    InvalidFile,
    #[error("The shared session lock is unavailable ({0}).")]
    LockUnavailable(i32),
    #[error(transparent)]
    File(#[from] CocoaError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Coding(#[from] serde_json::Error),
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
}

type ContainerResult<T> = Result<T, RiseShareContainerError>;

/// one source file for a story session, path then kind then filename then content type
pub type RiseShareStorySourceItem = (PathBuf, RiseShareStoryMediaKind, String, String);

static ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::compiled().canon())
        .ok()
        .map(|paths| paths.data_dir().to_path_buf())
});

pub enum RiseShareContainer {}

impl RiseShareContainer {
    pub const SNAPSHOT_FILENAME: &'static str = "share-targets-v1.json";
    pub const STORIES_DIRECTORY_NAME: &'static str = "share-story-sessions";
    pub const SEND_SESSIONS_DIRECTORY_NAME: &'static str = "share-send-sessions";
    pub const MANIFEST_FILENAME: &'static str = "manifest.json";
    pub const TEMPORARY_DIRECTORY_NAME: &'static str = "share-extension-tmp";
    pub const AVATAR_CACHE_DIRECTORY_NAME: &'static str = "share-avatar-cache-v1";
    pub const UNBOUND_STORY_ACCOUNT_ID: &'static str = "unbound";
    pub const STORY_SESSION_LIFETIME: TimeInterval = 24.0 * 60.0 * 60.0;
    pub const SEND_SESSION_LIFETIME: TimeInterval = 72.0 * 60.0 * 60.0;
    pub const TEMPORARY_FILE_LIFETIME: TimeInterval = 24.0 * 60.0 * 60.0;

    const MAXIMUM_SEND_MANIFEST_BYTE_COUNT: usize = 2 * 1_024 * 1_024;
    const MAXIMUM_AVATAR_CACHE_FILE_BYTE_COUNT: i64 = 2 * 1_024 * 1_024;
    const MAXIMUM_AVATAR_CACHE_ENTRY_COUNT: usize = 220;

    fn maximum_send_file_byte_count() -> i64 {
        i64::try_from(rise_upload::PIPELINE_MAXIMUM_FILE_BYTES).unwrap_or(i64::MAX)
    }

    pub fn root_url() -> ContainerResult<PathBuf> {
        let Some(root) = ROOT.clone() else {
            return Err(RiseShareContainerError::AppGroupUnavailable);
        };
        FileManager.create_directory(&root, true)?;
        Ok(root)
    }

    pub fn save_snapshot(snapshot: &RiseShareSnapshot) -> ContainerResult<()> {
        let destination = Self::root_url()?.join(Self::SNAPSHOT_FILENAME);
        let data = Self::encode(snapshot)?;
        Self::atomic_write(&data, &destination)
    }

    /// loads the snapshot, another schema version reads as an invalid manifest
    pub fn load_snapshot() -> ContainerResult<RiseShareSnapshot> {
        let url = Self::root_url()?.join(Self::SNAPSHOT_FILENAME);
        let data = std::fs::read(&url)?;
        let snapshot: RiseShareSnapshot = Self::decode(&data)?;
        if snapshot.schema_version != RiseShareSnapshot::CURRENT_SCHEMA_VERSION {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        Ok(snapshot)
    }

    /// cached avatar bytes for this url
    pub fn avatar_image_data(remote_url: &Url) -> Option<Vec<u8>> {
        let file_url = Self::avatar_cache_file_url(remote_url).ok()?;
        let attributes = FileManager.attributes_of_item(&file_url).ok()?;
        let byte_count = attributes.size;
        if byte_count <= 0 || byte_count > Self::MAXIMUM_AVATAR_CACHE_FILE_BYTE_COUNT {
            return None;
        }
        std::fs::read(&file_url).ok()
    }

    pub fn store_avatar_image_data(data: &[u8], remote_url: &Url) -> ContainerResult<()> {
        let byte_count = i64::try_from(data.len()).unwrap_or(i64::MAX);
        if data.is_empty() || byte_count > Self::MAXIMUM_AVATAR_CACHE_FILE_BYTE_COUNT {
            return Err(RiseShareContainerError::InvalidFile);
        }
        let destination = Self::avatar_cache_file_url(remote_url)?;
        Self::atomic_write(data, &destination)?;
        Self::prune_avatar_cache();
        Ok(())
    }

    pub fn clear_avatar_cache() {
        let Ok(root) = Self::root_url() else { return };
        let _ = FileManager.remove_item(&root.join(Self::AVATAR_CACHE_DIRECTORY_NAME));
    }

    pub fn make_temporary_directory() -> ContainerResult<PathBuf> {
        let parent = Self::root_url()?.join(Self::TEMPORARY_DIRECTORY_NAME);
        FileManager.create_directory(&parent, true)?;
        let directory = parent.join(Self::lowercased_uuid());
        FileManager.create_directory(&directory, false)?;
        Ok(directory)
    }

    pub fn story_sessions_directory() -> ContainerResult<PathBuf> {
        let url = Self::root_url()?.join(Self::STORIES_DIRECTORY_NAME);
        FileManager.create_directory(&url, true)?;
        Ok(url)
    }

    pub fn send_sessions_directory() -> ContainerResult<PathBuf> {
        let url = Self::root_url()?.join(Self::SEND_SESSIONS_DIRECTORY_NAME);
        FileManager.create_directory(&url, true)?;
        Ok(url)
    }

    pub fn create_story_session(
        id: Option<String>,
        account_id: &str,
        source_items: &[RiseShareStorySourceItem],
        is_cancelled: &dyn Fn() -> bool,
    ) -> ContainerResult<RiseShareStorySession> {
        let id = id.unwrap_or_else(Self::lowercased_uuid);
        let normalized_account_id = trimming_whitespaces_and_newlines(account_id);
        if !Self::is_valid_session_identifier(&id)
            || normalized_account_id.is_empty()
            || normalized_account_id.chars().count() > 128
            || source_items.is_empty()
        {
            return Err(RiseShareContainerError::InvalidSessionIdentifier);
        }

        let parent = Self::story_sessions_directory()?;
        let staging = parent.join(format!(".{id}-staging"));
        let destination = parent.join(&id);
        let _ = FileManager.remove_item(&staging);
        let _ = FileManager.remove_item(&destination);
        FileManager.create_directory(&staging, false)?;

        let result = (|| -> ContainerResult<RiseShareStorySession> {
            let mut items: Vec<RiseShareStoryItem> = Vec::with_capacity(source_items.len());
            for (index, (url, kind, filename, content_type)) in source_items.iter().enumerate() {
                if is_cancelled() {
                    return Err(CancellationError.into());
                }
                let fallback = if *kind == RiseShareStoryMediaKind::Image { "jpg" } else { "mp4" };
                let ext = Self::safe_path_extension(filename, fallback);
                let relative_path = format!("{index:03}-{}.{ext}", Self::lowercased_uuid());
                let copied_url = staging.join(&relative_path);
                FileManager.copy_item(url, &copied_url)?;
                if is_cancelled() {
                    return Err(CancellationError.into());
                }
                items.push(RiseShareStoryItem {
                    id: Self::lowercased_uuid(),
                    kind: *kind,
                    relative_path,
                    suggested_filename: filename.clone(),
                    content_type: content_type.clone(),
                });
            }

            let session = RiseShareStorySession::new(
                id.clone(),
                normalized_account_id.to_owned(),
                Self::milliseconds(&RiseClock::now()),
                items,
                0,
                None,
            );
            let manifest = Self::encode(&session)?;
            Self::atomic_write(&manifest, &staging.join(Self::MANIFEST_FILENAME))?;
            FileManager.move_item(&staging, &destination)?;
            Ok(session)
        })();
        if result.is_err() {
            let _ = FileManager.remove_item(&staging);
        }
        result
    }

    pub fn create_send_session(
        session: RiseShareSendSession,
        source_directory: &Path,
        is_cancelled: &dyn Fn() -> bool,
    ) -> ContainerResult<(RiseShareSendSession, PathBuf)> {
        let temporary_parent = standardized(&Self::root_url()?.join(Self::TEMPORARY_DIRECTORY_NAME));
        let source = standardized(source_directory);
        if source.parent() != Some(temporary_parent.as_path()) {
            return Err(RiseShareContainerError::UnsafeRelativePath);
        }
        Self::validate_directory(&source)?;
        if is_cancelled() {
            return Err(CancellationError.into());
        }
        Self::validate_send_session(&session, &session.id, &session.account_id, &source, &RiseClock::now(), false)?;

        let parent = Self::send_sessions_directory()?;
        let destination = parent.join(&session.id);
        if FileManager.file_exists(&destination)
            || FileManager.file_exists(&source.join(Self::MANIFEST_FILENAME))
        {
            return Err(RiseShareContainerError::InvalidManifest);
        }

        let manifest_data = Self::encode(&session)?;
        if manifest_data.len() > Self::MAXIMUM_SEND_MANIFEST_BYTE_COUNT {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        Self::atomic_write(&manifest_data, &source.join(Self::MANIFEST_FILENAME))?;
        if is_cancelled() {
            let _ = FileManager.remove_item(&source.join(Self::MANIFEST_FILENAME));
            return Err(CancellationError.into());
        }
        FileManager.move_item(&source, &destination)?;
        Self::validate_directory(&destination)?;
        Self::validate_send_session(&session, &session.id, &session.account_id, &destination, &RiseClock::now(), true)?;
        Ok((session, destination))
    }

    /// loads a send session if it belongs to this account
    pub fn load_send_session(
        id: &str,
        expected_account_id: &str,
    ) -> ContainerResult<(RiseShareSendSession, PathBuf)> {
        Self::load_send_session_at(id, expected_account_id, &RiseClock::now())
    }

    fn load_send_session_at(
        id: &str,
        expected_account_id: &str,
        now: &Date,
    ) -> ContainerResult<(RiseShareSendSession, PathBuf)> {
        if !Self::is_valid_session_identifier(id) {
            return Err(RiseShareContainerError::InvalidSessionIdentifier);
        }
        let directory = Self::send_sessions_directory()?.join(id);
        Self::validate_directory(&directory)?;
        let manifest_url = directory.join(Self::MANIFEST_FILENAME);
        Self::validate_regular_file(&manifest_url, None)?;
        let data = std::fs::read(&manifest_url)?;
        if data.len() > Self::MAXIMUM_SEND_MANIFEST_BYTE_COUNT {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        let session: RiseShareSendSession = Self::decode(&data)?;
        Self::validate_send_session(&session, id, expected_account_id, &directory, now, true)?;
        Ok((session, directory))
    }

    pub fn persist_send_uploaded_media(
        session_id: &str,
        expected_account_id: &str,
        attachment_id: &str,
        media: std::collections::BTreeMap<String, RiseShareSendJSONValue>,
    ) -> ContainerResult<RiseShareSendSession> {
        if media.is_empty() {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        let now = RiseClock::now();
        Self::with_send_session_lock(session_id, || {
            let (mut session, directory) = Self::load_send_session_at(session_id, expected_account_id, &now)?;
            let Some(index) = session.attachments.iter().position(|attachment| attachment.id == attachment_id) else {
                return Err(RiseShareContainerError::InvalidManifest);
            };
            if !session.claimed_by_app || session.commit_started {
                return Err(RiseShareContainerError::InvalidManifest);
            }
            if let Some(existing) = &session.attachments[index].uploaded_media {
                if *existing != media {
                    return Err(RiseShareContainerError::InvalidManifest);
                }
                return Ok(session);
            }
            session.attachments[index].uploaded_media = Some(media);
            session.updated_at_ms = Self::milliseconds(&now);
            Self::write_send_manifest(&session, &directory)?;
            Ok(session)
        })
    }

    pub fn pending_send_session_ids(expected_account_id: &str) -> Vec<String> {
        let now = RiseClock::now();
        let Ok(parent) = Self::send_sessions_directory() else { return Vec::new() };
        let Ok(children) =
            FileManager.contents_of_directory_at(&parent, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
        else {
            return Vec::new();
        };
        let mut pending: Vec<(String, i64)> = children
            .iter()
            .filter_map(|child| {
                let id = last_path_component(child)?;
                if !Self::is_valid_session_identifier(&id) {
                    return None;
                }
                let (session, _) = Self::load_send_session_at(&id, expected_account_id, &now).ok()?;
                session.auto_resume.then_some((id, session.created_at_ms))
            })
            .collect();
        pending.sort_by(|lhs, rhs| lhs.1.cmp(&rhs.1).then_with(|| lhs.0.cmp(&rhs.0)));
        pending.into_iter().map(|(id, _)| id).collect()
    }

    pub fn set_send_session_auto_resume(
        id: &str,
        expected_account_id: &str,
        enabled: bool,
    ) -> ContainerResult<RiseShareSendSession> {
        let now = RiseClock::now();
        Self::with_send_session_lock(id, || {
            let (mut session, directory) = Self::load_send_session_at(id, expected_account_id, &now)?;
            session.auto_resume = enabled;
            session.updated_at_ms = Self::milliseconds(&now);
            Self::write_send_manifest(&session, &directory)?;
            Ok(session)
        })
    }

    pub fn mark_send_session_commit_started(
        id: &str,
        expected_account_id: &str,
    ) -> ContainerResult<RiseShareSendSession> {
        let now = RiseClock::now();
        Self::with_send_session_lock(id, || {
            let (mut session, directory) = Self::load_send_session_at(id, expected_account_id, &now)?;
            if !session.claimed_by_app {
                return Err(RiseShareContainerError::InvalidManifest);
            }
            if session.commit_started {
                return Ok(session);
            }
            session.commit_started = true;
            session.updated_at_ms = Self::milliseconds(&now);
            Self::write_send_manifest(&session, &directory)?;
            Ok(session)
        })
    }

    pub fn claim_send_session_for_app(
        id: &str,
        expected_account_id: &str,
    ) -> ContainerResult<RiseShareSendSession> {
        let now = RiseClock::now();
        Self::with_send_session_lock(id, || {
            let (mut session, directory) = Self::load_send_session_at(id, expected_account_id, &now)?;
            if session.claimed_by_app {
                return Ok(session);
            }
            session.claimed_by_app = true;
            session.claimed_at_ms = Some(Self::milliseconds(&now));
            session.updated_at_ms = Self::milliseconds(&now);
            Self::write_send_manifest(&session, &directory)?;
            Ok(session)
        })
    }

    pub fn resolved_send_attachment_url(
        attachment: &RiseShareSendAttachment,
        in_directory: &Path,
    ) -> ContainerResult<PathBuf> {
        let (Some(relative_path), Some(byte_count)) = (&attachment.relative_path, attachment.byte_count) else {
            return Err(RiseShareContainerError::InvalidFile);
        };
        if !Self::is_safe_leaf_path(relative_path) || byte_count <= 0 {
            return Err(RiseShareContainerError::InvalidFile);
        }
        let url = standardized(&in_directory.join(relative_path));
        if url.parent() != Some(standardized(in_directory).as_path()) {
            return Err(RiseShareContainerError::UnsafeRelativePath);
        }
        Self::validate_regular_file(&url, Some(byte_count))?;
        Ok(url)
    }

    /// removes an acknowledged send session, only a claimed one whose commit started
    pub fn remove_acknowledged_send_session(id: &str, expected_account_id: &str) {
        let _ = Self::with_send_session_lock(id, || {
            let Ok((session, _)) = Self::load_send_session(id, expected_account_id) else {
                return Ok(());
            };
            if !session.claimed_by_app || !session.commit_started {
                return Ok(());
            }
            let Ok(parent) = Self::send_sessions_directory() else { return Ok(()) };
            let _ = FileManager.remove_item(&parent.join(id));
            Ok(())
        });
    }

    pub fn discard_cancelled_send_session(id: &str, expected_account_id: &str) {
        let _ = Self::with_send_session_lock(id, || {
            let Ok((session, directory)) = Self::load_send_session(id, expected_account_id) else {
                return Ok(());
            };
            if session.claimed_by_app || session.commit_started {
                return Ok(());
            }
            let _ = FileManager.remove_item(&directory);
            Ok(())
        });
    }

    pub fn discard_app_claimed_cancelled_send_session(id: &str, expected_account_id: &str) {
        let _ = Self::with_send_session_lock(id, || {
            let Ok((session, directory)) = Self::load_send_session(id, expected_account_id) else {
                return Ok(());
            };
            if !session.claimed_by_app || session.commit_started {
                return Ok(());
            }
            let _ = FileManager.remove_item(&directory);
            Ok(())
        });
    }

    pub fn load_story_session(id: &str) -> ContainerResult<(RiseShareStorySession, PathBuf)> {
        Self::load_story_session_at(id, &RiseClock::now())
    }

    fn load_story_session_at(id: &str, now: &Date) -> ContainerResult<(RiseShareStorySession, PathBuf)> {
        if !Self::is_valid_session_identifier(id) {
            return Err(RiseShareContainerError::InvalidSessionIdentifier);
        }
        let directory = Self::story_sessions_directory()?.join(id);
        let data = std::fs::read(directory.join(Self::MANIFEST_FILENAME))?;
        let session: RiseShareStorySession = Self::decode(&data)?;
        let age = now.time_interval_since_1970() - (session.created_at_ms as f64 / 1_000.0);
        let item_count = i64::try_from(session.items.len()).unwrap_or(i64::MAX);
        if session.schema_version != RiseShareStorySession::CURRENT_SCHEMA_VERSION
            || session.id != id
            || trimming_whitespaces_and_newlines(&session.account_id).is_empty()
            || age < -300.0
            || age > Self::STORY_SESSION_LIFETIME
            || session.items.is_empty()
            || session.next_item_index < 0
            || session.next_item_index > item_count
        {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        for item in &session.items {
            Self::resolved_story_item_url(item, &directory)?;
        }
        Self::validate_story_publish_checkpoint(session.publish_checkpoint.as_ref(), &session)?;
        Ok((session, directory))
    }

    pub fn pending_story_session_ids(expected_account_id: &str) -> Vec<String> {
        let now = RiseClock::now();
        let account_id = trimming_whitespaces_and_newlines(expected_account_id);
        if account_id.is_empty() {
            return Vec::new();
        }
        let Ok(parent) = Self::story_sessions_directory() else { return Vec::new() };
        let Ok(children) =
            FileManager.contents_of_directory_at(&parent, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
        else {
            return Vec::new();
        };
        let mut pending: Vec<(String, i64)> = children
            .iter()
            .filter_map(|child| {
                if !is_real_directory(child) {
                    return None;
                }
                let id = last_path_component(child)?;
                if !Self::is_valid_session_identifier(&id) {
                    return None;
                }
                let (session, _) = Self::load_story_session_at(&id, &now).ok()?;
                let item_count = i64::try_from(session.items.len()).unwrap_or(i64::MAX);
                (session.account_id == account_id && session.next_item_index < item_count)
                    .then_some((id, session.created_at_ms))
            })
            .collect();
        pending.sort_by(|lhs, rhs| lhs.1.cmp(&rhs.1).then_with(|| lhs.0.cmp(&rhs.0)));
        pending.into_iter().map(|(id, _)| id).collect()
    }

    pub fn claim_unbound_story_sessions(account_id: &str) {
        let now = RiseClock::now();
        let owner = trimming_whitespaces_and_newlines(account_id);
        if owner.is_empty() || owner == Self::UNBOUND_STORY_ACCOUNT_ID {
            return;
        }
        let Ok(parent) = Self::story_sessions_directory() else { return };
        let Ok(children) =
            FileManager.contents_of_directory_at(&parent, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
        else {
            return;
        };
        for child in &children {
            if !is_real_directory(child) {
                continue;
            }
            let Some(id) = last_path_component(child) else { continue };
            if !Self::is_valid_session_identifier(&id) {
                continue;
            }
            let _ = Self::with_story_session_lock(&id, || {
                let (session, directory) = Self::load_story_session_at(&id, &now)?;
                if session.account_id != Self::UNBOUND_STORY_ACCOUNT_ID || session.publish_checkpoint.is_some() {
                    return Ok(());
                }
                let claimed = RiseShareStorySession::new(
                    session.id,
                    owner.to_owned(),
                    session.created_at_ms,
                    session.items,
                    session.next_item_index,
                    None,
                );
                Self::atomic_write(&Self::encode(&claimed)?, &directory.join(Self::MANIFEST_FILENAME))
            });
        }
    }

    /// publish checkpoint of a story session for this account
    pub fn story_publish_checkpoint(
        id: &str,
        expected_account_id: &str,
    ) -> ContainerResult<Option<RiseShareStoryPublishCheckpoint>> {
        let (session, _) = Self::load_story_session(id)?;
        if session.account_id != trimming_whitespaces_and_newlines(expected_account_id) {
            return Err(RiseShareContainerError::AccountMismatch);
        }
        Ok(session.publish_checkpoint)
    }

    pub fn store_story_publish_checkpoint(
        id: &str,
        expected_account_id: &str,
        checkpoint: RiseShareStoryPublishCheckpoint,
    ) -> ContainerResult<RiseShareStoryPublishCheckpoint> {
        let account_id = trimming_whitespaces_and_newlines(expected_account_id).to_owned();
        Self::with_story_session_lock(id, || {
            let (mut session, directory) = Self::load_story_session(id)?;
            if account_id.is_empty() || session.account_id != account_id {
                return Err(RiseShareContainerError::AccountMismatch);
            }
            if Self::current_item(&session).map(|item| item.id.as_str()) != Some(checkpoint.item_id.as_str()) {
                return Err(RiseShareContainerError::InvalidManifest);
            }
            Self::validate_story_publish_checkpoint(Some(&checkpoint), &session)?;
            if let Some(existing) = &session.publish_checkpoint {
                if *existing != checkpoint {
                    return Err(RiseShareContainerError::InvalidManifest);
                }
                return Ok(existing.clone());
            }
            session.publish_checkpoint = Some(checkpoint.clone());
            Self::atomic_write(&Self::encode(&session)?, &directory.join(Self::MANIFEST_FILENAME))?;
            Ok(checkpoint)
        })
    }

    pub fn advance_story_session(
        id: &str,
        expected_account_id: &str,
        completed_item_id: &str,
    ) -> ContainerResult<Option<RiseShareStorySession>> {
        let account_id = trimming_whitespaces_and_newlines(expected_account_id).to_owned();
        Self::with_story_session_lock(id, || {
            let (mut session, directory) = Self::load_story_session(id)?;
            if account_id.is_empty()
                || session.account_id != account_id
                || Self::current_item(&session).map(|item| item.id.as_str()) != Some(completed_item_id)
            {
                return Err(RiseShareContainerError::InvalidManifest);
            }
            session.next_item_index += 1;
            session.publish_checkpoint = None;
            Self::atomic_write(&Self::encode(&session)?, &directory.join(Self::MANIFEST_FILENAME))?;
            let item_count = i64::try_from(session.items.len()).unwrap_or(i64::MAX);
            if session.next_item_index >= item_count {
                let _ = FileManager.remove_item(&directory);
                return Ok(None);
            }
            Ok(Some(session))
        })
    }

    fn current_item(session: &RiseShareStorySession) -> Option<&RiseShareStoryItem> {
        usize::try_from(session.next_item_index)
            .ok()
            .and_then(|index| session.items.get(index))
    }

    fn validate_story_publish_checkpoint(
        checkpoint: Option<&RiseShareStoryPublishCheckpoint>,
        session: &RiseShareStorySession,
    ) -> ContainerResult<()> {
        let Some(checkpoint) = checkpoint else { return Ok(()) };
        let mut excluded_sorted = checkpoint.excluded_user_ids.clone();
        excluded_sorted.sort();
        let mut selected_sorted = checkpoint.selected_user_ids.clone();
        selected_sorted.sort();
        let excluded_unique: HashSet<&String> = checkpoint.excluded_user_ids.iter().collect();
        let selected_unique: HashSet<&String> = checkpoint.selected_user_ids.iter().collect();
        let media_type = checkpoint.media_type.as_str();
        let is_valid = Self::current_item(session).map(|item| item.id.as_str()) == Some(checkpoint.item_id.as_str())
            && checkpoint.idempotency_key
                == RiseShareStoryPublishCheckpoint::stable_idempotency_key(
                    &session.account_id,
                    &session.id,
                    &checkpoint.item_id,
                )
            && !trimming_whitespaces_and_newlines(&checkpoint.remote_file_url).is_empty()
            && checkpoint.remote_file_url.len() <= 4_096
            && checkpoint.idempotency_key.len() <= 128
            && (media_type == "image" || media_type == "video")
            && !checkpoint.audience.is_empty()
            && (1..=3).contains(&checkpoint.duration_days)
            && checkpoint.excluded_user_ids == excluded_sorted
            && checkpoint.selected_user_ids == selected_sorted
            && excluded_unique.len() == checkpoint.excluded_user_ids.len()
            && selected_unique.len() == checkpoint.selected_user_ids.len();
        if !is_valid {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        Ok(())
    }

    /// path of a story item, it must be a plain existing file in the session dir
    pub fn resolved_story_item_url(item: &RiseShareStoryItem, in_directory: &Path) -> ContainerResult<PathBuf> {
        let relative_path = item.relative_path.as_str();
        if relative_path.is_empty()
            || relative_path.contains('/')
            || relative_path.contains('\\')
            || relative_path == "."
            || relative_path == ".."
        {
            return Err(RiseShareContainerError::UnsafeRelativePath);
        }
        let standardized_directory = standardized(in_directory);
        let url = standardized(&in_directory.join(relative_path));
        if url.parent() != Some(standardized_directory.as_path()) || !FileManager.file_exists(&url) {
            return Err(RiseShareContainerError::UnsafeRelativePath);
        }
        Ok(url)
    }

    pub fn remove_story_session(id: &str) {
        if !Self::is_valid_session_identifier(id) {
            return;
        }
        let Ok(parent) = Self::story_sessions_directory() else { return };
        let _ = FileManager.remove_item(&parent.join(id));
    }

    /// removes expired files, runs when the share sheet opens
    pub fn remove_expired_files() {
        let now = RiseClock::now();
        if let Ok(stories) = Self::story_sessions_directory() {
            Self::remove_expired_story_sessions(&stories, &now);
        }
        if let Ok(root) = Self::root_url() {
            let temporary = root.join(Self::TEMPORARY_DIRECTORY_NAME);
            Self::remove_expired_children(&temporary, Self::TEMPORARY_FILE_LIFETIME, &now);
        }
        if let Ok(sends) = Self::send_sessions_directory() {
            Self::remove_expired_send_sessions(&sends, &now);
        }
    }

    /// 8 to 64 chars of letters digits and dash, so the id is a safe dir name
    pub fn is_valid_session_identifier(value: &str) -> bool {
        let count = value.chars().count();
        if !(8..=64).contains(&count) {
            return false;
        }
        value.chars().all(|scalar| scalar.is_ascii_alphanumeric() || scalar == '-')
    }

    fn encode<T: Serialize>(value: &T) -> ContainerResult<Vec<u8>> {
        let json = serde_json::to_value(value)?;
        Ok(serde_json::to_vec(&SortedKeys(&json))?)
    }

    fn decode<T: DeserializeOwned>(data: &[u8]) -> ContainerResult<T> {
        Ok(serde_json::from_slice(data)?)
    }

    fn safe_path_extension(filename: &str, fallback: &str) -> String {
        let ext = Path::new(filename)
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext.is_empty() || ext.chars().count() > 10 || !ext.chars().all(char::is_alphanumeric) {
            return fallback.to_owned();
        }
        ext
    }

    fn atomic_write(data: &[u8], destination: &Path) -> ContainerResult<()> {
        let Some(parent) = destination.parent() else {
            return Err(RiseShareContainerError::InvalidFile);
        };
        FileManager.create_directory(parent, true)?;
        let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4().uuid_string()));
        let result = std::fs::write(&temporary, data).and_then(|()| std::fs::rename(&temporary, destination));
        if let Err(error) = result {
            let _ = FileManager.remove_item(&temporary);
            return Err(error.into());
        }
        Ok(())
    }

    fn validate_send_session(
        session: &RiseShareSendSession,
        expected_id: &str,
        expected_account_id: &str,
        directory: &Path,
        now: &Date,
        enforce_lifetime: bool,
    ) -> ContainerResult<()> {
        let normalized_account = trimming_whitespaces_and_newlines(&session.account_id);
        let normalized_expected_account = trimming_whitespaces_and_newlines(expected_account_id);
        let file_attachment_count = session
            .attachments
            .iter()
            .filter(|attachment| attachment.relative_path.is_some())
            .count();
        let is_valid = session.schema_version == RiseShareSendSession::CURRENT_SCHEMA_VERSION
            && session.id == expected_id
            && Self::is_valid_session_identifier(&session.id)
            && !normalized_account.is_empty()
            && normalized_account.chars().count() <= 128
            && normalized_account == normalized_expected_account
            && Uuid::from_uuid_string(&session.idempotency_key).is_some()
            && (1..=20).contains(&session.destinations.len())
            && (1..=64).contains(&session.attachments.len())
            && file_attachment_count <= 10
            && session.comment.chars().count() <= 1_024
            && session.text_content.chars().count() <= 4_096;
        if !is_valid {
            if normalized_account != normalized_expected_account {
                return Err(RiseShareContainerError::AccountMismatch);
            }
            return Err(RiseShareContainerError::InvalidManifest);
        }

        let now_ms = Self::milliseconds(now);
        let has_valid_claim = session.claimed_by_app == session.claimed_at_ms.is_some()
            && session
                .claimed_at_ms
                .is_none_or(|claimed| claimed >= session.created_at_ms && claimed <= now_ms + 300_000);
        if session.created_at_ms > now_ms + 300_000
            || session.updated_at_ms < session.created_at_ms
            || session.updated_at_ms > now_ms + 300_000
            || !has_valid_claim
        {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        if enforce_lifetime
            && now.time_interval_since_1970() - session.updated_at_ms as f64 / 1_000.0 > Self::SEND_SESSION_LIFETIME
        {
            return Err(RiseShareContainerError::InvalidManifest);
        }

        let mut chat_ids: HashSet<&str> = HashSet::new();
        for destination in &session.destinations {
            let chat_id = trimming_whitespaces_and_newlines(&destination.chat_id);
            if chat_id.is_empty()
                || chat_id.chars().count() > 128
                || !chat_ids.insert(chat_id)
                || destination.topic_id.as_ref().map_or(0, |topic| topic.chars().count()) > 128
                || destination.title.chars().count() > 512
            {
                return Err(RiseShareContainerError::InvalidManifest);
            }
        }

        let mut attachment_ids: HashSet<&str> = HashSet::new();
        let mut has_file = false;
        let mut expected_children: HashSet<String> = HashSet::new();
        expected_children.insert(Self::MANIFEST_FILENAME.to_owned());
        let standardized_directory = standardized(directory);
        for attachment in &session.attachments {
            if Uuid::from_uuid_string(&attachment.id).is_none()
                || !attachment_ids.insert(attachment.id.as_str())
                || attachment.kind.is_empty()
                || attachment.kind.chars().count() > 32
                || attachment.mime_type.is_empty()
                || attachment.mime_type.chars().count() > 256
                || attachment.content_type_identifier.chars().count() > 256
                || attachment.original_file_name.as_ref().map_or(0, |name| name.chars().count()) > 512
                || attachment.text.as_ref().map_or(0, |text| text.chars().count()) > 4_096
                || attachment.source_index < 0
            {
                return Err(RiseShareContainerError::InvalidManifest);
            }
            let Some(relative_path) = &attachment.relative_path else {
                if attachment.byte_count.is_some() || attachment.uploaded_media.is_some() {
                    return Err(RiseShareContainerError::InvalidManifest);
                }
                continue;
            };
            has_file = true;
            let Some(byte_count) = attachment.byte_count else {
                return Err(RiseShareContainerError::InvalidFile);
            };
            if !Self::is_safe_leaf_path(relative_path)
                || byte_count <= 0
                || byte_count > Self::maximum_send_file_byte_count()
            {
                return Err(RiseShareContainerError::InvalidFile);
            }
            let file_url = standardized(&directory.join(relative_path));
            if file_url.parent() != Some(standardized_directory.as_path()) {
                return Err(RiseShareContainerError::UnsafeRelativePath);
            }
            Self::validate_regular_file(&file_url, Some(byte_count))?;
            if !expected_children.insert(relative_path.clone()) {
                return Err(RiseShareContainerError::InvalidManifest);
            }
        }
        if !has_file {
            return Err(RiseShareContainerError::InvalidManifest);
        }

        let actual_children = FileManager.contents_of_directory_at(directory, DirectoryEnumerationOptions::default())?;
        for child in &actual_children {
            let Some(name) = last_path_component(child) else {
                return Err(RiseShareContainerError::InvalidFile);
            };
            if !expected_children.contains(&name) {
                return Err(RiseShareContainerError::InvalidFile);
            }
        }
        Ok(())
    }

    fn validate_directory(url: &Path) -> ContainerResult<()> {
        let metadata = std::fs::symlink_metadata(url)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || !FileManager.file_exists(url) {
            return Err(RiseShareContainerError::InvalidFile);
        }
        Ok(())
    }

    fn validate_regular_file(url: &Path, expected_byte_count: Option<i64>) -> ContainerResult<()> {
        let metadata = std::fs::symlink_metadata(url)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || !FileManager.file_exists(url) {
            return Err(RiseShareContainerError::InvalidFile);
        }
        if let Some(expected_byte_count) = expected_byte_count {
            if i64::try_from(metadata.len()).ok() != Some(expected_byte_count) {
                return Err(RiseShareContainerError::InvalidFile);
            }
        }
        Ok(())
    }

    fn is_safe_leaf_path(value: &str) -> bool {
        !value.is_empty()
            && value != "."
            && value != ".."
            && !value.contains('/')
            && !value.contains('\\')
            && value.chars().count() <= 512
    }

    fn write_send_manifest(session: &RiseShareSendSession, directory: &Path) -> ContainerResult<()> {
        let data = Self::encode(session)?;
        if data.len() > Self::MAXIMUM_SEND_MANIFEST_BYTE_COUNT {
            return Err(RiseShareContainerError::InvalidManifest);
        }
        Self::atomic_write(&data, &directory.join(Self::MANIFEST_FILENAME))
    }

    fn remove_expired_send_sessions(directory: &Path, now: &Date) {
        Self::remove_expired_sessions(directory, now, Self::SEND_SESSION_LIFETIME, Self::send_sessions_directory);
    }

    fn remove_expired_story_sessions(directory: &Path, now: &Date) {
        Self::remove_expired_sessions(directory, now, Self::STORY_SESSION_LIFETIME, Self::story_sessions_directory);
    }

    fn remove_expired_sessions(
        directory: &Path,
        now: &Date,
        lifetime: TimeInterval,
        sessions_directory: fn() -> ContainerResult<PathBuf>,
    ) {
        let Ok(children) = FileManager.contents_of_directory_at(directory, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        for child in &children {
            let Some(id) = last_path_component(child) else { continue };
            if now.time_interval_since(&item_date(child)) <= lifetime {
                continue;
            }
            if is_directory(child) && Self::is_valid_session_identifier(&id) {
                let _ = Self::with_session_lock(&id, sessions_directory, || {
                    if now.time_interval_since(&item_date(child)) <= lifetime {
                        return Ok(());
                    }
                    let _ = FileManager.remove_item(child);
                    Ok(())
                });
            } else if id.starts_with('.') && id.ends_with(".lock") {
                Self::remove_lock_file_if_unused(child);
            }
        }
    }

    fn remove_lock_file_if_unused(url: &Path) {
        let Ok(file) = OpenOptions::new().read(true).write(true).open(url) else { return };
        if file.try_lock().is_err() {
            return;
        }
        let _ = FileManager.remove_item(url);
        let _ = file.unlock();
    }

    fn with_send_session_lock<T>(id: &str, operation: impl FnOnce() -> ContainerResult<T>) -> ContainerResult<T> {
        Self::with_session_lock(id, Self::send_sessions_directory, operation)
    }

    fn with_story_session_lock<T>(id: &str, operation: impl FnOnce() -> ContainerResult<T>) -> ContainerResult<T> {
        Self::with_session_lock(id, Self::story_sessions_directory, operation)
    }

    fn with_session_lock<T>(
        id: &str,
        sessions_directory: fn() -> ContainerResult<PathBuf>,
        operation: impl FnOnce() -> ContainerResult<T>,
    ) -> ContainerResult<T> {
        if !Self::is_valid_session_identifier(id) {
            return Err(RiseShareContainerError::InvalidSessionIdentifier);
        }
        let lock_url = sessions_directory()?.join(format!(".{id}.lock"));
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_url)
            .map_err(|error| RiseShareContainerError::LockUnavailable(error.raw_os_error().unwrap_or(-1)))?;
        file.lock()
            .map_err(|error| RiseShareContainerError::LockUnavailable(error.raw_os_error().unwrap_or(-1)))?;
        let result = operation();
        let _ = file.unlock();
        result
    }

    fn remove_expired_children(directory: &Path, lifetime: TimeInterval, now: &Date) {
        let Ok(children) = FileManager.contents_of_directory_at(directory, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        for child in &children {
            if now.time_interval_since(&item_date(child)) > lifetime {
                let _ = FileManager.remove_item(child);
            }
        }
    }

    fn avatar_cache_file_url(remote_url: &Url) -> ContainerResult<PathBuf> {
        let scheme = remote_url.scheme().map(str::to_lowercase);
        if !matches!(scheme.as_deref(), Some("https" | "http")) {
            return Err(RiseShareContainerError::InvalidFile);
        }
        let directory = Self::root_url()?.join(Self::AVATAR_CACHE_DIRECTORY_NAME);
        FileManager.create_directory(&directory, true)?;
        let digest = sha256_hex(remote_url.absolute_string().as_bytes());
        Ok(directory.join(format!("{digest}.jpg")))
    }

    fn prune_avatar_cache() {
        let Ok(root) = Self::root_url() else { return };
        let Ok(entries) = FileManager.contents_of_directory_at(
            &root.join(Self::AVATAR_CACHE_DIRECTORY_NAME),
            DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES,
        ) else {
            return;
        };
        if entries.len() <= Self::MAXIMUM_AVATAR_CACHE_ENTRY_COUNT {
            return;
        }
        let mut dated: Vec<(f64, PathBuf)> = entries
            .into_iter()
            .map(|entry| (modification_interval(&entry), entry))
            .collect();
        dated.sort_by(|lhs, rhs| rhs.0.total_cmp(&lhs.0));
        for (_, stale) in dated.into_iter().skip(Self::MAXIMUM_AVATAR_CACHE_ENTRY_COUNT) {
            let _ = FileManager.remove_item(&stale);
        }
    }

    fn lowercased_uuid() -> String {
        Uuid::new_v4().uuid_string().to_lowercase()
    }

    fn milliseconds(date: &Date) -> i64 {
        (date.time_interval_since_1970() * 1_000.0) as i64
    }
}

/// serializes json with object keys sorted
struct SortedKeys<'a>(&'a serde_json::Value);

impl Serialize for SortedKeys<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self.0 {
            serde_json::Value::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(&SortedKeys(value))?;
                }
                sequence.end()
            }
            serde_json::Value::Object(map) => {
                let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
                entries.sort_by(|lhs, rhs| lhs.0.cmp(rhs.0));
                let mut object = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    object.serialize_entry(key, &SortedKeys(value))?;
                }
                object.end()
            }
            other => other.serialize(serializer),
        }
    }
}

/// cleans a path, drops dots and folds parent parts without touching disk
fn standardized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    result.push(component.as_os_str());
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn last_path_component(path: &Path) -> Option<String> {
    path.file_name().map(|name| name.to_string_lossy().into_owned())
}

fn is_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn is_real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn modification_interval(path: &Path) -> f64 {
    FileManager
        .attributes_of_item(path)
        .ok()
        .and_then(|attributes| attributes.modification_date)
        .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
        .map_or(f64::MIN, |duration| duration.as_secs_f64())
}

/// modified date, else created date else the distant past
fn item_date(path: &Path) -> Date {
    let attributes = FileManager.attributes_of_item(path).ok();
    let seconds = attributes
        .and_then(|attributes| attributes.modification_date.or(attributes.creation_date))
        .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs_f64());
    match seconds {
        Some(seconds) => Date::from_time_interval_since_1970(seconds),
        None => Date::from_time_interval_since_reference_date(-63_114_076_800.0),
    }
}

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, Context, Entity, RenderImage};
use image::Frame;
use rise_canon::RiseLimits;
use rise_canon::rise_key::bot as bot_keys;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::paths::temporary_directory;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::modules::media_picker::editor::editor_state::EditorImageState;
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::share_extension::shared::rise_outgoing_media::RiseOutgoingImageEncoder;

const ROOT_FOLDER: &str = "MediaReedit";
const ACCOUNTS_FOLDER: &str = "accounts";
const ANONYMOUS_ACCOUNT: &str = "anonymous";
const INDEX_FILE_NAME: &str = "index.json";
const ORIGINAL_FILE_NAME: &str = "original.jpg";
const STATE_FILE_NAME: &str = "state.json";
const META_FILE_NAME: &str = "meta.json";
const TTL_SECONDS_KEY: &str = "ttl_seconds";

#[derive(Clone)]
pub struct ReeditDocument {
    pub edit_id: String,
    pub original_image: Arc<RenderImage>,
    pub state: EditorImageState,
}

struct ReeditDocumentEntry {
    dir: PathBuf,
    created: f64,
    bytes: i64,
}

pub struct MediaReeditStore {
    index: HashMap<String, String>,
    loaded_index: bool,
    pending_by_temp_id: HashMap<String, String>,
    loaded_root_path: Option<PathBuf>,
}

impl MediaReeditStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            index: HashMap::new(),
            loaded_index: false,
            pending_by_temp_id: HashMap::new(),
            loaded_root_path: None,
        }
    }

    pub fn chat_ttl_seconds() -> TimeInterval {
        RiseLimits::client_value("media_reedit_chat_retention_ms") as TimeInterval / 1_000.0
    }

    pub fn chat_max_documents() -> i64 {
        RiseLimits::client_value("media_reedit_chat_max_documents")
    }

    pub fn chat_max_bytes() -> i64 {
        RiseLimits::client_value("media_reedit_chat_max_bytes")
    }

    pub fn register_pending_temp(&mut self, temp_id: &str, edit_id: &str) {
        self.activate_account_state();
        if temp_id.is_empty() {
            return;
        }
        self.pending_by_temp_id.insert(temp_id.to_owned(), edit_id.to_owned());
    }

    pub fn consume_pending_temp(&mut self, temp_id: &str) -> Option<String> {
        self.activate_account_state();
        self.pending_by_temp_id.remove(temp_id)
    }

    fn root_url() -> Option<PathBuf> {
        let dir = Self::storage_directory_url(SaiAccountCacheScope::active_user_id().as_deref())?;
        if !FileManager.file_exists(&dir) {
            FileManager.create_directory(&dir, true).ok();
        }
        Some(dir)
    }

    pub fn storage_directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        let base = RiseEngineAppRuntime::application_support_directory(false).ok()?;
        Some(
            base.join(ROOT_FOLDER)
                .join(ACCOUNTS_FOLDER)
                .join(Self::account_directory_name(user_id)),
        )
    }

    fn index_url() -> Option<PathBuf> {
        Self::root_url().map(|root| root.join(INDEX_FILE_NAME))
    }

    /// ttl_seconds is written into the document, not applied by the caller
    /// a chat edit stays reopenable for the canon window, a story as long as it is shown
    pub fn save_document(
        &mut self,
        original_image_url: &Path,
        editor_state_data: &[u8],
        ttl_seconds: TimeInterval,
    ) -> Option<String> {
        self.activate_account_state();
        let root = Self::root_url()?;
        self.purge_expired();
        let edit_id = Uuid::new_v4().uuid_string();
        let dir = root.join(&edit_id);
        Self::write_document(&dir, original_image_url, editor_state_data)?;
        Self::write_created_at(
            &dir,
            Date::now().time_interval_since_1970() * 1_000.0,
            ttl_seconds,
        );
        if ttl_seconds == Self::chat_ttl_seconds() {
            self.trim_chat_documents(&root, &edit_id);
        }
        Some(edit_id)
    }

    fn write_document(dir: &Path, original_image_url: &Path, editor_state_data: &[u8]) -> Option<()> {
        FileManager.create_directory(dir, true).ok()?;
        let original_dest = dir.join(ORIGINAL_FILE_NAME);
        FileManager.remove_item(&original_dest).ok();
        FileManager.copy_item(original_image_url, &original_dest).ok()?;
        if original_image_url.starts_with(temporary_directory()) {
            FileManager.remove_item(original_image_url).ok();
        }
        Self::write_atomically(&dir.join(STATE_FILE_NAME), editor_state_data).ok()?;
        Some(())
    }

    pub fn update_state(&mut self, edit_id: &str, editor_state_data: &[u8]) {
        self.activate_account_state();
        let Some(root) = Self::root_url() else {
            return;
        };
        let dir = root.join(edit_id);
        if !FileManager.file_exists(&dir) {
            return;
        }
        Self::write_atomically(&dir.join(STATE_FILE_NAME), editor_state_data).ok();
    }

    pub fn link(&mut self, identity: &str, to_edit_id: &str) {
        self.activate_account_state();
        if identity.is_empty() {
            return;
        }
        self.load_index_if_needed();
        if self.index.get(identity).map(String::as_str) == Some(to_edit_id) {
            return;
        }
        self.index.insert(identity.to_owned(), to_edit_id.to_owned());
        self.persist_index();
    }

    pub fn edit_id_for_any(&mut self, identities: &[String]) -> Option<String> {
        self.activate_account_state();
        self.load_index_if_needed();
        identities
            .iter()
            .filter(|id| !id.is_empty())
            .find_map(|id| self.index.get(id).cloned())
    }

    pub fn load_document(&mut self, edit_id: &str, ttl_seconds: TimeInterval) -> Option<ReeditDocument> {
        self.activate_account_state();
        let root = Self::root_url()?;
        let dir = root.join(edit_id);
        if Self::is_expired(&dir, ttl_seconds) {
            self.purge(edit_id, &dir);
            return None;
        }
        let image = Self::image_contents_of_file(&dir.join(ORIGINAL_FILE_NAME))?;
        let state_data = std::fs::read(dir.join(STATE_FILE_NAME)).ok()?;
        let state = EditorStateCodec::decode_image(&state_data)?;
        Some(ReeditDocument {
            edit_id: edit_id.to_owned(),
            original_image: image,
            state,
        })
    }

    fn image_contents_of_file(url: &Path) -> Option<Arc<RenderImage>> {
        let mut buffer = RiseOutgoingImageEncoder::source_with_url(url)?.to_rgba8();
        for pixel in buffer.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Some(Arc::new(RenderImage::new([Frame::new(buffer)])))
    }

    fn write_created_at(dir: &Path, ms: f64, ttl_seconds: TimeInterval) {
        let mut object = serde_json::Map::new();
        object.insert(bot_keys::CREATED_AT.to_owned(), serde_json::Value::from(ms));
        object.insert(TTL_SECONDS_KEY.to_owned(), serde_json::Value::from(ttl_seconds));
        let Ok(data) = serde_json::to_vec(&serde_json::Value::Object(object)) else {
            return;
        };
        Self::write_atomically(&dir.join(META_FILE_NAME), &data).ok();
    }

    fn meta(dir: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
        let data = std::fs::read(dir.join(META_FILE_NAME)).ok()?;
        match serde_json::from_slice::<serde_json::Value>(&data).ok()? {
            serde_json::Value::Object(object) => Some(object),
            _ => None,
        }
    }

    /// the documents own window wins over the callers
    /// a sweep from a chat send must not decide how long a storys original lives
    /// no ttl_seconds means the chat window
    fn is_expired(dir: &Path, ttl_seconds: TimeInterval) -> bool {
        let Some(meta) = Self::meta(dir) else {
            return false;
        };
        let Some(created) = meta.get(bot_keys::CREATED_AT).and_then(serde_json::Value::as_f64) else {
            return false;
        };
        let window = meta
            .get(TTL_SECONDS_KEY)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(ttl_seconds);
        (Date::now().time_interval_since_1970() * 1_000.0 - created) > window * 1_000.0
    }

    fn purge(&mut self, edit_id: &str, dir: &Path) {
        FileManager.remove_item(dir).ok();
        self.load_index_if_needed();
        let before = self.index.len();
        self.index.retain(|_, value| value != edit_id);
        if self.index.len() != before {
            self.persist_index();
        }
    }

    fn purge_expired(&mut self) {
        let Some(root) = Self::root_url() else {
            return;
        };
        let Ok(entries) = FileManager.contents_of_directory_at(&root, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        let chat_ttl_seconds = Self::chat_ttl_seconds();
        for dir in entries.iter().filter(|dir| dir.is_dir()) {
            if Self::is_expired(dir, chat_ttl_seconds)
                && let Some(edit_id) = Self::last_path_component(dir)
            {
                self.purge(&edit_id, dir);
            }
        }
    }

    fn trim_chat_documents(&mut self, root: &Path, keeping_edit_id: &str) {
        let Ok(entries) = FileManager.contents_of_directory_at(root, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        let chat_ttl_seconds = Self::chat_ttl_seconds();
        let mut docs = Vec::new();
        for dir in entries.into_iter().filter(|dir| dir.is_dir()) {
            let Some(meta) = Self::meta(&dir) else {
                continue;
            };
            let Some(created) = meta.get(bot_keys::CREATED_AT).and_then(serde_json::Value::as_f64) else {
                continue;
            };
            let window = meta
                .get(TTL_SECONDS_KEY)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(chat_ttl_seconds);
            if window != chat_ttl_seconds {
                continue;
            }
            let bytes = Self::directory_bytes(&dir);
            docs.push(ReeditDocumentEntry { dir, created, bytes });
        }
        docs.sort_by(|lhs, rhs| lhs.created.total_cmp(&rhs.created));
        let max_documents = Self::chat_max_documents();
        let max_bytes = Self::chat_max_bytes();
        let mut count = i64::try_from(docs.len()).unwrap_or(i64::MAX);
        let mut total: i64 = docs.iter().map(|doc| doc.bytes).sum();
        for doc in &docs {
            if count <= max_documents && total <= max_bytes {
                continue;
            }
            let Some(edit_id) = Self::last_path_component(&doc.dir) else {
                continue;
            };
            if edit_id == keeping_edit_id {
                continue;
            }
            self.purge(&edit_id, &doc.dir);
            count -= 1;
            total -= doc.bytes;
        }
    }

    fn directory_bytes(dir: &Path) -> i64 {
        let Ok(files) = FileManager.contents_of_directory_at(dir, DirectoryEnumerationOptions::default()) else {
            return 0;
        };
        files
            .iter()
            .filter_map(|file| FileManager.attributes_of_item(file).ok())
            .map(|attributes| attributes.size)
            .sum()
    }

    fn last_path_component(url: &Path) -> Option<String> {
        url.file_name().map(|name| name.to_string_lossy().into_owned())
    }

    fn load_index_if_needed(&mut self) {
        if self.loaded_index {
            return;
        }
        self.loaded_index = true;
        let Some(decoded) = Self::index_url()
            .and_then(|url| std::fs::read(url).ok())
            .and_then(|data| serde_json::from_slice::<HashMap<String, String>>(&data).ok())
        else {
            return;
        };
        self.index = decoded;
    }

    fn persist_index(&self) {
        let Some(url) = Self::index_url() else {
            return;
        };
        let Ok(data) = serde_json::to_vec(&self.index) else {
            return;
        };
        Self::write_atomically(&url, &data).ok();
    }

    pub fn reset_account_state(&mut self) {
        self.index.clear();
        self.loaded_index = false;
        self.pending_by_temp_id.clear();
        self.loaded_root_path = None;
    }

    pub fn activate_account_state(&mut self) {
        let next_path = Self::root_url();
        if self.loaded_root_path == next_path {
            return;
        }
        self.index.clear();
        self.loaded_index = false;
        self.pending_by_temp_id.clear();
        self.loaded_root_path = next_path;
    }

    pub fn remove_persisted_account(&mut self, user_id: &str) {
        let Some(directory) = Self::storage_directory_url(Some(user_id)) else {
            return;
        };
        FileManager.remove_item(&directory).ok();
    }

    fn account_directory_name(user_id: Option<&str>) -> String {
        let value = user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if value.is_empty() {
            return ANONYMOUS_ACCOUNT.to_owned();
        }
        sha256_hex(value.as_bytes())
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            std::fs::remove_file(&temporary).ok();
        })
    }
}

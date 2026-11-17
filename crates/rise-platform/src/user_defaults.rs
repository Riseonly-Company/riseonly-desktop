use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};

use crate::paths::AppPaths;

const LOG_TARGET: &str = "riseonly";
const PREFERENCES_DIRECTORY: &str = "Preferences";
const STANDARD_FILE: &str = "standard.defaults.json";
const SUITE_FILE_SUFFIX: &str = ".suite.json";
const TEMPORARY_SUFFIX: &str = ".tmp";
const WRITER_THREAD_NAME: &str = "rise-user-defaults";

/// a plist value, the types UserDefaults stores
/// date is never stored, the app writes a double of the unix time
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultsValue {
    Bool(bool),
    Integer(i64),
    Double(f64),
    String(String),
    Data(#[serde(with = "data_as_base64")] Vec<u8>),
    Array(Vec<DefaultsValue>),
    Dictionary(BTreeMap<String, DefaultsValue>),
}

impl DefaultsValue {
    /// bool only for 0 and 1
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Integer(0) => Some(false),
            Self::Integer(1) => Some(true),
            Self::Double(value) if *value == 0.0 => Some(false),
            Self::Double(value) if *value == 1.0 => Some(true),
            _ => None,
        }
    }

    /// int only when a double is integral and in range
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Bool(value) => Some(i64::from(*value)),
            Self::Integer(value) => Some(*value),
            Self::Double(value)
                if value.fract() == 0.0 && *value >= i64::MIN as f64 && *value < i64::MAX as f64 =>
            {
                Some(*value as i64)
            }
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
            Self::Integer(value) => Some(*value as f64),
            Self::Double(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_data(&self) -> Option<&[u8]> {
        match self {
            Self::Data(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[DefaultsValue]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_dictionary(&self) -> Option<&BTreeMap<String, DefaultsValue>> {
        match self {
            Self::Dictionary(value) => Some(value),
            _ => None,
        }
    }
}

impl From<bool> for DefaultsValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for DefaultsValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<i32> for DefaultsValue {
    fn from(value: i32) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<f64> for DefaultsValue {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

impl From<f32> for DefaultsValue {
    fn from(value: f32) -> Self {
        Self::Double(f64::from(value))
    }
}

impl From<String> for DefaultsValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for DefaultsValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<Vec<u8>> for DefaultsValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Data(value)
    }
}

impl From<&[u8]> for DefaultsValue {
    fn from(value: &[u8]) -> Self {
        Self::Data(value.to_vec())
    }
}

impl From<Vec<String>> for DefaultsValue {
    fn from(value: Vec<String>) -> Self {
        Self::Array(value.into_iter().map(Self::String).collect())
    }
}

impl From<Vec<i64>> for DefaultsValue {
    fn from(value: Vec<i64>) -> Self {
        Self::Array(value.into_iter().map(Self::Integer).collect())
    }
}

impl From<Vec<f64>> for DefaultsValue {
    fn from(value: Vec<f64>) -> Self {
        Self::Array(value.into_iter().map(Self::Double).collect())
    }
}

impl From<Vec<DefaultsValue>> for DefaultsValue {
    fn from(value: Vec<DefaultsValue>) -> Self {
        Self::Array(value)
    }
}

impl From<BTreeMap<String, DefaultsValue>> for DefaultsValue {
    fn from(value: BTreeMap<String, DefaultsValue>) -> Self {
        Self::Dictionary(value)
    }
}

impl From<BTreeMap<String, bool>> for DefaultsValue {
    fn from(value: BTreeMap<String, bool>) -> Self {
        Self::Dictionary(value.into_iter().map(|(key, flag)| (key, Self::Bool(flag))).collect())
    }
}

mod data_as_base64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text.as_bytes()).map_err(serde::de::Error::custom)
    }
}

#[derive(Default)]
struct DomainState {
    values: BTreeMap<String, DefaultsValue>,
    registered: BTreeMap<String, DefaultsValue>,
    file: Option<PathBuf>,
    generation: u64,
    written_generation: u64,
    writer_running: bool,
    last_write_ok: bool,
}

#[derive(Default)]
struct Domain {
    state: Mutex<DomainState>,
    changed: Condvar,
}

/// reads come from memory, a write lands in memory at once and hits disk later in one atomic batch
/// a suite is its own file
#[derive(Clone, Default)]
pub struct UserDefaults {
    domain: Arc<Domain>,
}

struct Registry {
    directory: Option<PathBuf>,
    standard: UserDefaults,
    suites: HashMap<String, UserDefaults>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            directory: None,
            standard: UserDefaults::default(),
            suites: HashMap::new(),
        })
    })
}

fn suite_file(directory: &Path, suite_name: &str) -> PathBuf {
    directory.join(format!("{suite_name}{SUITE_FILE_SUFFIX}"))
}

impl UserDefaults {
    /// the standard domain, usable before install and early writes survive the attach
    pub fn standard() -> Self {
        registry().lock().standard.clone()
    }

    /// one shared instance per suite name, None for a name that cant be a file
    pub fn suite(suite_name: &str) -> Option<Self> {
        let invalid = suite_name.is_empty()
            || suite_name.starts_with('.')
            || suite_name.contains(['/', '\\', ':'])
            || suite_name.chars().any(char::is_control);
        if invalid {
            return None;
        }
        let mut registry = registry().lock();
        if let Some(existing) = registry.suites.get(suite_name) {
            return Some(existing.clone());
        }
        let defaults = Self::default();
        if let Some(directory) = registry.directory.clone() {
            defaults.attach(suite_file(&directory, suite_name));
        }
        registry.suites.insert(suite_name.to_owned(), defaults.clone());
        Some(defaults)
    }

    pub fn install(paths: &AppPaths) {
        Self::install_in(&paths.data_dir().join(PREFERENCES_DIRECTORY));
    }

    pub fn install_in(directory: &Path) {
        let mut registry = registry().lock();
        registry.directory = Some(directory.to_path_buf());
        registry.standard.attach(directory.join(STANDARD_FILE));
        for (name, defaults) in &registry.suites {
            defaults.attach(suite_file(directory, name));
        }
    }

    /// domain backed by file, outside the shared registry
    pub fn persistent(file: PathBuf) -> Self {
        let defaults = Self::default();
        defaults.attach(file);
        defaults
    }

    /// domain that never touches disk
    pub fn in_memory() -> Self {
        Self::default()
    }

    fn attach(&self, file: PathBuf) {
        let loaded = load_file(&file);
        let mut state = self.domain.state.lock();
        if state.file.as_ref() == Some(&file) {
            return;
        }
        let early_writes = std::mem::take(&mut state.values);
        let had_early_writes = !early_writes.is_empty();
        state.values = loaded;
        state.values.extend(early_writes);
        state.file = Some(file);
        state.last_write_ok = true;
        if had_early_writes {
            self.mark_dirty(&mut state);
        }
    }

    fn lookup(&self, key: &str) -> Option<DefaultsValue> {
        let state = self.domain.state.lock();
        state.values.get(key).or_else(|| state.registered.get(key)).cloned()
    }

    pub fn object(&self, for_key: &str) -> Option<DefaultsValue> {
        self.lookup(for_key)
    }

    /// returns the string of a number value too
    pub fn string(&self, for_key: &str) -> Option<String> {
        match self.lookup(for_key)? {
            DefaultsValue::String(value) => Some(value),
            DefaultsValue::Bool(value) => Some(i64::from(value).to_string()),
            DefaultsValue::Integer(value) => Some(value.to_string()),
            DefaultsValue::Double(value) => Some(value.to_string()),
            _ => None,
        }
    }

    pub fn bool(&self, for_key: &str) -> bool {
        match self.lookup(for_key) {
            Some(DefaultsValue::Bool(value)) => value,
            Some(DefaultsValue::Integer(value)) => value != 0,
            Some(DefaultsValue::Double(value)) => value != 0.0,
            Some(DefaultsValue::String(text)) => string_bool_value(&text),
            _ => false,
        }
    }

    pub fn integer(&self, for_key: &str) -> i64 {
        match self.lookup(for_key) {
            Some(DefaultsValue::Bool(value)) => i64::from(value),
            Some(DefaultsValue::Integer(value)) => value,
            Some(DefaultsValue::Double(value)) => value as i64,
            Some(DefaultsValue::String(text)) => string_integer_value(&text),
            _ => 0,
        }
    }

    pub fn double(&self, for_key: &str) -> f64 {
        match self.lookup(for_key) {
            Some(DefaultsValue::String(text)) => string_double_value(&text),
            Some(other) => other.as_f64().unwrap_or(0.0),
            None => 0.0,
        }
    }

    pub fn data(&self, for_key: &str) -> Option<Vec<u8>> {
        match self.lookup(for_key)? {
            DefaultsValue::Data(value) => Some(value),
            _ => None,
        }
    }

    pub fn array(&self, for_key: &str) -> Option<Vec<DefaultsValue>> {
        match self.lookup(for_key)? {
            DefaultsValue::Array(value) => Some(value),
            _ => None,
        }
    }

    /// none when any element isnt a string
    pub fn string_array(&self, for_key: &str) -> Option<Vec<String>> {
        let DefaultsValue::Array(items) = self.lookup(for_key)? else {
            return None;
        };
        items
            .into_iter()
            .map(|item| match item {
                DefaultsValue::String(value) => Some(value),
                _ => None,
            })
            .collect()
    }

    pub fn dictionary(&self, for_key: &str) -> Option<BTreeMap<String, DefaultsValue>> {
        match self.lookup(for_key)? {
            DefaultsValue::Dictionary(value) => Some(value),
            _ => None,
        }
    }

    /// registered values first then written values over them
    pub fn dictionary_representation(&self) -> BTreeMap<String, DefaultsValue> {
        let state = self.domain.state.lock();
        let mut merged = state.registered.clone();
        merged.extend(state.values.iter().map(|(key, value)| (key.clone(), value.clone())));
        merged
    }

    /// key goes first so the key literal guard can read the first argument
    pub fn set(&self, for_key: &str, value: impl Into<DefaultsValue>) {
        let value = value.into();
        let mut state = self.domain.state.lock();
        if state.values.get(for_key) == Some(&value) {
            return;
        }
        state.values.insert(for_key.to_owned(), value);
        self.mark_dirty(&mut state);
    }

    pub fn remove_object(&self, for_key: &str) {
        let mut state = self.domain.state.lock();
        if state.values.remove(for_key).is_some() {
            self.mark_dirty(&mut state);
        }
    }

    /// the registration domain, never written to disk
    pub fn register<K: Into<String>>(&self, defaults: impl IntoIterator<Item = (K, DefaultsValue)>) {
        let mut state = self.domain.state.lock();
        state.registered.extend(defaults.into_iter().map(|(key, value)| (key.into(), value)));
    }

    /// blocks until every write so far is on disk, false when the last write failed
    pub fn synchronize(&self) -> bool {
        let mut state = self.domain.state.lock();
        if state.file.is_none() {
            return true;
        }
        if state.written_generation == state.generation {
            return state.last_write_ok;
        }
        if !state.writer_running {
            drop(state);
            return self.write_pending();
        }
        let target = state.generation;
        while state.written_generation < target {
            self.domain.changed.wait(&mut state);
        }
        state.last_write_ok
    }

    fn mark_dirty(&self, state: &mut DomainState) {
        state.generation += 1;
        if state.file.is_none() {
            return;
        }
        if state.writer_running {
            self.domain.changed.notify_all();
            return;
        }
        let domain = Arc::clone(&self.domain);
        let spawned = std::thread::Builder::new()
            .name(WRITER_THREAD_NAME.to_owned())
            .spawn(move || writer_loop(domain));
        match spawned {
            Ok(_) => state.writer_running = true,
            Err(error) => {
                tracing::error!(target: LOG_TARGET, %error, "user defaults writer did not start");
            }
        }
    }

    fn write_pending(&self) -> bool {
        let (generation, snapshot, file) = {
            let state = self.domain.state.lock();
            match state.file.clone() {
                Some(file) => (state.generation, state.values.clone(), file),
                None => return true,
            }
        };
        let ok = write_file(&file, &snapshot);
        let mut state = self.domain.state.lock();
        state.written_generation = state.written_generation.max(generation);
        state.last_write_ok = ok;
        self.domain.changed.notify_all();
        ok
    }
}

fn writer_loop(domain: Arc<Domain>) {
    loop {
        let (generation, snapshot, file) = {
            let mut state = domain.state.lock();
            while state.written_generation == state.generation || state.file.is_none() {
                domain.changed.wait(&mut state);
            }
            let Some(file) = state.file.clone() else {
                continue;
            };
            (state.generation, state.values.clone(), file)
        };
        let ok = write_file(&file, &snapshot);
        let mut state = domain.state.lock();
        state.written_generation = state.written_generation.max(generation);
        state.last_write_ok = ok;
        domain.changed.notify_all();
    }
}

fn load_file(file: &Path) -> BTreeMap<String, DefaultsValue> {
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, %error, "user defaults file is unreadable");
            return BTreeMap::new();
        }
    };
    let raw: BTreeMap<String, serde_json::Value> = match serde_json::from_slice(&bytes) {
        Ok(raw) => raw,
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, %error, "user defaults file is not a dictionary");
            return BTreeMap::new();
        }
    };
    raw.into_iter()
        .filter_map(|(key, value)| match serde_json::from_value::<DefaultsValue>(value) {
            Ok(value) => Some((key, value)),
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, %error, "user defaults entry skipped");
                None
            }
        })
        .collect()
}

fn write_file(file: &Path, values: &BTreeMap<String, DefaultsValue>) -> bool {
    match try_write_file(file, values) {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(target: LOG_TARGET, %error, "user defaults write failed");
            false
        }
    }
}

fn try_write_file(file: &Path, values: &BTreeMap<String, DefaultsValue>) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(values).map_err(std::io::Error::other)?;
    let mut temporary = file.as_os_str().to_owned();
    temporary.push(TEMPORARY_SUFFIX);
    let temporary = PathBuf::from(temporary);
    {
        let mut handle = fs::File::create(&temporary)?;
        handle.write_all(&bytes)?;
        handle.sync_all()?;
    }
    fs::rename(&temporary, file)
}

fn string_bool_value(text: &str) -> bool {
    let start = text.trim_start_matches(char::is_whitespace);
    let word: String = start.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if word.eq_ignore_ascii_case("yes") || word.eq_ignore_ascii_case("true") {
        return true;
    }
    string_integer_value(text) != 0
}

/// leading whitespace then optional sign then digits
/// saturates and gives 0 when nothing parses
fn string_integer_value(text: &str) -> i64 {
    let mut chars = text.trim_start_matches(char::is_whitespace).chars().peekable();
    let negative = match chars.peek() {
        Some('-') => {
            chars.next();
            true
        }
        Some('+') => {
            chars.next();
            false
        }
        _ => false,
    };
    let mut value: i64 = 0;
    for c in chars {
        let Some(digit) = c.to_digit(10) else {
            break;
        };
        let digit = i64::from(digit);
        value = if negative {
            value.saturating_mul(10).saturating_sub(digit)
        } else {
            value.saturating_mul(10).saturating_add(digit)
        };
    }
    value
}

fn string_double_value(text: &str) -> f64 {
    let start = text.trim_start_matches(char::is_whitespace);
    let bytes = start.as_bytes();
    let mut end = 0;
    let mut best = 0.0;
    while end < bytes.len() {
        end += 1;
        if let Some(prefix) = start.get(..end)
            && let Ok(value) = prefix.parse::<f64>()
            && prefix.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
        {
            best = value;
        }
    }
    best
}

use std::any::{Any, TypeId};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, Weak};

use gpui::{
    App, ElementId, FontWeight, Hsla, IntoElement, RenderOnce, Rgba, SharedString, Window, div, px,
    transparent_black, white,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_character::{Character, characters};
use rise_core::foundation::foundation_compare::{CompareOptions, ns_range_of};
use rise_core::foundation::foundation_data_detector::{NSDataDetector, NSTextCheckingType};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_regex::{MENTION, NSMatchingOptions};
use rise_core::foundation::foundation_text::{character_count, character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::ns_cache::NSCache;
use rise_core::foundation::ns_range::{self, NSRange};
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_ui::rich_text::{FontDescriptorSymbolicTraits, RichInlineSpan, RichInlineStyle, RichTextMarkup, RichTextMarkupInlineResult};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::NATURAL_LINE_HEIGHT;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::{AttributedString, FontDesign, LineStyle, RichFont, RichText};
use rise_ui::swiftui::text_selection::SelectableText;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::spoiler_dust::SpoilerDust;
use crate::modules::chat::shared::chat_search_highlight::ChatSearchHighlightPalette;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FormattedTextOptions {
    pub no_url_format: bool,
    pub no_url_subtitle: bool,
    pub no_mention_format: bool,
    pub no_markdown: bool,
    pub literal_code_blocks: bool,
    pub highlight_words: Vec<String>,
    pub secure_urls_only: bool,
    pub link_urls: bool,
}

impl FormattedTextOptions {
    /// post text, an address stays plain text with no color tap or host line
    /// links live only in stories
    /// mentions and hashtags keep their marks
    pub fn post_caption() -> Self {
        Self { no_url_format: true, no_url_subtitle: true, ..Self::default() }
    }
}

fn color_bits(color: Hsla) -> [u32; 4] {
    [color.h.to_bits(), color.s.to_bits(), color.l.to_bits(), color.a.to_bits()]
}

#[derive(Clone, Debug)]
pub struct FormattedTextStyle {
    pub base_color: Hsla,
    pub link_color: Hsla,
    pub subtitle_color: Hsla,
    pub font_size: f32,
    pub subtitle_font_size: f32,
    pub line_spacing: f32,
    pub code_foreground: Option<Hsla>,
    pub code_background: Option<Hsla>,
    pub link_underline: bool,
    pub hashtag_color: Option<Hsla>,
    pub accent: Option<FormattedTextAccent>,
}

type FormattedTextStyleIdentity<'a> = (
    [[u32; 4]; 3],
    [u32; 3],
    Option<[u32; 4]>,
    Option<[u32; 4]>,
    bool,
    Option<[u32; 4]>,
    Option<(&'a str, [u32; 4])>,
);

impl FormattedTextStyle {
    pub fn new(base_color: Hsla, link_color: Hsla, subtitle_color: Hsla) -> Self {
        Self {
            base_color,
            link_color,
            subtitle_color,
            font_size: 15.0,
            subtitle_font_size: 12.0,
            line_spacing: 2.0,
            code_foreground: None,
            code_background: None,
            link_underline: false,
            hashtag_color: None,
            accent: None,
        }
    }

    pub fn standard_link_color() -> Hsla {
        Rgba { r: 0.07, g: 0.45, b: 0.94, a: 1.0 }.into()
    }

    pub fn link_color_on_dark_fill() -> Hsla {
        white()
    }

    fn resolved_code_foreground(&self) -> Hsla {
        self.code_foreground.unwrap_or(self.base_color)
    }

    fn resolved_code_background(&self) -> Hsla {
        self.code_background.unwrap_or_else(|| {
            let mut color = self.base_color;
            color.a *= 0.6 * 0.16;
            color
        })
    }

    fn identity(&self) -> FormattedTextStyleIdentity<'_> {
        (
            [color_bits(self.base_color), color_bits(self.link_color), color_bits(self.subtitle_color)],
            [self.font_size.to_bits(), self.subtitle_font_size.to_bits(), self.line_spacing.to_bits()],
            self.code_foreground.map(color_bits),
            self.code_background.map(color_bits),
            self.link_underline,
            self.hashtag_color.map(color_bits),
            self.accent.as_ref().map(|accent| (accent.word.as_str(), color_bits(accent.color))),
        )
    }
}

impl PartialEq for FormattedTextStyle {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for FormattedTextStyle {}

impl Hash for FormattedTextStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// one word to stand out in the text, like the rarity in a shared achievement post
#[derive(Clone, Debug)]
pub struct FormattedTextAccent {
    pub word: String,
    pub color: Hsla,
}

#[derive(Clone, Debug)]
pub struct RoleMentionSpan {
    pub range: NSRange,
    pub color: Hsla,
    pub link: Option<String>,
}

impl PartialEq for RoleMentionSpan {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range && color_bits(self.color) == color_bits(other.color) && self.link == other.link
    }
}

impl Eq for RoleMentionSpan {}

impl Hash for RoleMentionSpan {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.range.hash(state);
        color_bits(self.color).hash(state);
        self.link.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormattedTextResult {
    pub attributed_body: AttributedString,
    pub display_string: String,
    pub subtitle: Option<String>,
    pub has_url: bool,
    pub has_mention: bool,
    pub has_code_block: bool,
    pub revealed_body: Option<AttributedString>,
    pub spoiler_ranges: Vec<NSRange>,
    pub spoiler_plate: Hsla,
}

impl FormattedTextResult {
    pub fn spoiler_indices(&self) -> HashSet<usize> {
        (0..self.spoiler_ranges.len()).collect()
    }

    /// only the spoilers in revealed are shown, the rest stay under the plate
    /// with dust the closed runs are left clear for a spoiler_dust background to fill
    pub fn body(&self, revealed_spoilers: &HashSet<usize>, dust: bool) -> AttributedString {
        let Some(revealed_body) = self.revealed_body.as_ref() else {
            return self.attributed_body.clone();
        };
        if !dust && revealed_spoilers.is_empty() {
            return self.attributed_body.clone();
        }
        let closed: Vec<NSRange> = self
            .spoiler_ranges
            .iter()
            .enumerate()
            .filter(|(index, _)| !revealed_spoilers.contains(index))
            .map(|(_, range)| *range)
            .collect();
        if closed.is_empty() {
            return revealed_body.clone();
        }
        let plate = if dust { None } else { Some(self.spoiler_plate) };
        FormattedText::hiding_spoilers(revealed_body, &closed, &self.display_string, plate)
            .unwrap_or_else(|| revealed_body.clone())
    }
}

/// exact identity for width independent text geometry
/// keeping this key next to the prepared value lets chat layouts reuse line measurements across
/// remounts
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FormattedTextRenderSeed {
    pub text: String,
    pub style: FormattedTextStyle,
    pub options: FormattedTextOptions,
    pub role_mentions: Vec<RoleMentionSpan>,
}

impl FormattedTextRenderSeed {
    pub fn new(
        text: impl Into<String>,
        style: FormattedTextStyle,
        options: FormattedTextOptions,
        role_mentions: Vec<RoleMentionSpan>,
    ) -> Self {
        Self { text: text.into(), style, options, role_mentions }
    }
}

pub type FormattedTextGeometryCacheKey = FormattedTextRenderSeed;

/// attributed carries the run fonts the line layout measures
/// paragraph line spacing and word wrapping come from cache_key.style since the kit string has no
/// paragraph attributes
#[derive(Clone, Debug)]
pub struct FormattedTextPreparedGeometry {
    pub display: String,
    pub attributed: AttributedString,
    pub cache_key: FormattedTextGeometryCacheKey,
}

#[derive(Clone, Debug)]
pub struct FormattedTextInlinePreparation {
    pub formatted: Arc<FormattedTextResult>,
    pub geometry: Arc<FormattedTextPreparedGeometry>,
}

#[derive(Clone, Debug)]
pub struct FormattedTextInteractionPreparation {
    pub geometry: Arc<FormattedTextPreparedGeometry>,
    pub mentions: Vec<FormattedTextMentionToken>,
    pub urls: Vec<FormattedTextMentionToken>,
    pub spoilers: Vec<FormattedTextMentionToken>,
}

/// presentation window keeps up to 640 rows
/// caches are a bit larger on purpose but have byte budgets and reject huge single entries
pub enum FormattedTextPreparationCachePolicy {}

static ENTRY_LIMIT: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("formatted_text_cache_entries"));
static LINE_MEASUREMENT_ENTRY_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("chat_prepared_text_layout_cache_entries"));
static SOURCE_COST_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("formatted_text_source_cache_bytes"));
static FORMATTED_COST_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("formatted_text_result_cache_bytes"));
static GEOMETRY_COST_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("formatted_text_geometry_cache_bytes"));
static LINE_MEASUREMENT_COST_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("chat_prepared_text_layout_cache_bytes"));
static MAXIMUM_CACHEABLE_UTF16_COUNT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("formatted_text_cache_max_utf16_count"));
static PREPARATION_ENTRY_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("formatted_text_preparation_queue_entries"));
static PREPARATION_INPUT_COST_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("formatted_text_preparation_queue_input_bytes"));

fn utf16_count(text: &str) -> i64 {
    i64::try_from(ns_range::length_utf16(text)).unwrap_or(i64::MAX)
}

fn stride_of<T>() -> i64 {
    i64::try_from(std::mem::size_of::<T>()).unwrap_or(0)
}

impl FormattedTextPreparationCachePolicy {
    pub fn entry_limit() -> i64 {
        *ENTRY_LIMIT
    }

    pub fn line_measurement_entry_limit() -> i64 {
        *LINE_MEASUREMENT_ENTRY_LIMIT
    }

    pub fn source_cost_limit() -> i64 {
        *SOURCE_COST_LIMIT
    }

    pub fn formatted_cost_limit() -> i64 {
        *FORMATTED_COST_LIMIT
    }

    pub fn geometry_cost_limit() -> i64 {
        *GEOMETRY_COST_LIMIT
    }

    pub fn line_measurement_cost_limit() -> i64 {
        *LINE_MEASUREMENT_COST_LIMIT
    }

    pub fn maximum_cacheable_utf16_count() -> i64 {
        *MAXIMUM_CACHEABLE_UTF16_COUNT
    }

    pub fn preparation_entry_limit() -> i64 {
        *PREPARATION_ENTRY_LIMIT
    }

    pub fn preparation_input_cost_limit() -> i64 {
        *PREPARATION_INPUT_COST_LIMIT
    }

    pub fn aggregate_cost_limit() -> i64 {
        Self::source_cost_limit()
            + Self::formatted_cost_limit()
            + Self::geometry_cost_limit()
            + Self::line_measurement_cost_limit()
            + Self::preparation_input_cost_limit()
    }

    pub fn source_cost(text: &str) -> i64 {
        256 + utf16_count(text) * 12
    }

    pub fn formatted_cost(text: &str, role_mention_count: usize) -> i64 {
        512 + utf16_count(text) * 40 + i64::try_from(role_mention_count).unwrap_or(0) * 48
    }

    pub fn geometry_cost(text: &str) -> i64 {
        384 + utf16_count(text) * 28
    }

    pub fn line_measurement_cost(text: &str) -> i64 {
        128 + utf16_count(text) * 2
    }

    pub fn option_cost(options: &FormattedTextOptions) -> i64 {
        options
            .highlight_words
            .iter()
            .fold(0, |total, word| total + stride_of::<String>() + utf16_count(word) * stride_of::<u16>())
    }

    pub fn can_cache(text: &str) -> bool {
        utf16_count(text) <= Self::maximum_cacheable_utf16_count()
    }

    pub fn role_mention_cost(role_mentions: &[RoleMentionSpan]) -> i64 {
        i64::try_from(role_mentions.len()).unwrap_or(0) * stride_of::<RoleMentionSpan>()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn to_cache_units(value: i64) -> usize {
    usize::try_from(value.max(0)).unwrap_or(usize::MAX)
}

struct FormattedTextBoundedCacheShared<Key, Value>
where
    Key: Eq + Hash + Clone + Send + Sync + 'static,
    Value: Clone + Send + Sync + 'static,
{
    storage: NSCache<Key, Value>,
    generation: Mutex<u64>,
}

impl<Key, Value> FormattedTextBoundedCacheShared<Key, Value>
where
    Key: Eq + Hash + Clone + Send + Sync + 'static,
    Value: Clone + Send + Sync + 'static,
{
    fn remove_all(&self) {
        let mut generation = lock(&self.generation);
        *generation = generation.wrapping_add(1);
        self.storage.remove_all_objects();
    }
}

/// pressure aware eviction, the lock covers only lookup and store
/// build runs unlocked so a background warm up never stalls a main thread lookup
/// two threads racing on one key both build and the first stored value wins
pub struct FormattedTextBoundedCache<Key, Value>
where
    Key: Eq + Hash + Clone + Send + Sync + 'static,
    Value: Clone + Send + Sync + 'static,
{
    shared: Arc<FormattedTextBoundedCacheShared<Key, Value>>,
    maximum_entry_cost: i64,
    _memory_warning_observer: RiseSubscription,
}

impl<Key, Value> FormattedTextBoundedCache<Key, Value>
where
    Key: Eq + Hash + Clone + Send + Sync + 'static,
    Value: Clone + Send + Sync + 'static,
{
    pub fn new(count_limit: i64, total_cost_limit: i64) -> Self {
        let storage = NSCache::new();
        storage.set_count_limit(to_cache_units(count_limit));
        storage.set_total_cost_limit(to_cache_units(total_cost_limit));
        let shared = Arc::new(FormattedTextBoundedCacheShared { storage, generation: Mutex::new(0) });
        let weak = Arc::downgrade(&shared);
        let observer = RiseAppEvents::default_center().observe(
            UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
            move |_| {
                if let Some(shared) = weak.upgrade() {
                    shared.remove_all();
                }
            },
        );
        Self {
            shared,
            maximum_entry_cost: (total_cost_limit / 4).max(1),
            _memory_warning_observer: observer,
        }
    }

    pub fn cached_value(&self, for_key: &Key) -> Option<Value> {
        let _generation = lock(&self.shared.generation);
        self.shared.storage.object(for_key)
    }

    pub fn remove_all(&self) {
        self.shared.remove_all();
    }

    pub fn value(
        &self,
        for_key: Key,
        cost: i64,
        cacheable: bool,
        additional_cost: impl FnOnce(&Value) -> i64,
        build: impl FnOnce() -> Value,
    ) -> Value {
        let expected_generation = {
            let generation = lock(&self.shared.generation);
            if let Some(cached) = self.shared.storage.object(&for_key) {
                return cached;
            }
            *generation
        };

        let value = build();
        let resolved_cost = cost + additional_cost(&value);
        if !cacheable || resolved_cost > self.maximum_entry_cost {
            return value;
        }

        let generation = lock(&self.shared.generation);
        if *generation != expected_generation {
            return value;
        }
        if let Some(raced) = self.shared.storage.object(&for_key) {
            return raced;
        }
        self.shared
            .storage
            .set_object_cost(value.clone(), for_key, to_cache_units(resolved_cost.max(1)));
        value
    }
}

trait FormattedTextPreparationJob: Send + Sync {
    fn prepare_if_needed(&self);

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

type FormattedTextPreparationBuild<Value> = Arc<dyn Fn() -> Value + Send + Sync>;

enum FormattedTextPreparationSlot<Value> {
    Ready(Value),
    Pending { build: FormattedTextPreparationBuild<Value>, is_preparing: bool },
}

/// a lease retains only one immutable result
/// dropping every lease cancels queued work
/// a running worker may finish but never captures a view or installs into ui
pub struct FormattedTextPreparation<Value> {
    lock: Mutex<FormattedTextPreparationSlot<Value>>,
}

impl<Value: Clone + Send + Sync + 'static> FormattedTextPreparation<Value> {
    pub fn with_value(value: Value) -> Arc<Self> {
        Arc::new(Self { lock: Mutex::new(FormattedTextPreparationSlot::Ready(value)) })
    }

    pub fn with_build(build: impl Fn() -> Value + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            lock: Mutex::new(FormattedTextPreparationSlot::Pending { build: Arc::new(build), is_preparing: false }),
        })
    }

    /// sync render cant suspend without changing the first frame or measurement
    /// never wait on a worker since it may be behind other rows
    /// if preparation lost the race build this result once on demand, thats the cold fallback
    pub fn value_for_immediate_display(&self) -> Value {
        let build = {
            let mut slot = lock(&self.lock);
            match &mut *slot {
                FormattedTextPreparationSlot::Ready(result) => return result.clone(),
                FormattedTextPreparationSlot::Pending { build, is_preparing } => {
                    *is_preparing = true;
                    build.clone()
                }
            }
        };
        self.finish(build())
    }

    fn finish(&self, value: Value) -> Value {
        let mut slot = lock(&self.lock);
        if let FormattedTextPreparationSlot::Ready(result) = &*slot {
            return result.clone();
        }
        *slot = FormattedTextPreparationSlot::Ready(value.clone());
        value
    }
}

impl<Value: Clone + Send + Sync + 'static> FormattedTextPreparationJob for FormattedTextPreparation<Value> {
    fn prepare_if_needed(&self) {
        let build = {
            let mut slot = lock(&self.lock);
            match &mut *slot {
                FormattedTextPreparationSlot::Pending { build, is_preparing } if !*is_preparing => {
                    *is_preparing = true;
                    build.clone()
                }
                _ => return,
            }
        };
        self.finish(build());
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// queue key with equality and hashing across key types
trait FormattedTextPreparationQueueInput: Send + Sync {
    fn as_any(&self) -> &dyn Any;

    fn input_eq(&self, other: &dyn FormattedTextPreparationQueueInput) -> bool;

    fn input_hash(&self) -> u64;
}

impl<Key: Hash + Eq + Send + Sync + 'static> FormattedTextPreparationQueueInput for Key {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn input_eq(&self, other: &dyn FormattedTextPreparationQueueInput) -> bool {
        other.as_any().downcast_ref::<Key>().is_some_and(|other| other == self)
    }

    fn input_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        TypeId::of::<Key>().hash(&mut hasher);
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone)]
struct FormattedTextPreparationQueueJobKey {
    input: Arc<dyn FormattedTextPreparationQueueInput>,
    value_type: TypeId,
}

impl PartialEq for FormattedTextPreparationQueueJobKey {
    fn eq(&self, other: &Self) -> bool {
        self.value_type == other.value_type && self.input.input_eq(other.input.as_ref())
    }
}

impl Eq for FormattedTextPreparationQueueJobKey {}

impl Hash for FormattedTextPreparationQueueJobKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value_type.hash(state);
        self.input.input_hash().hash(state);
    }
}

struct FormattedTextPreparationQueueEntry {
    key: FormattedTextPreparationQueueJobKey,
    cost: i64,
    job: Weak<dyn FormattedTextPreparationJob>,
}

#[derive(Default)]
struct FormattedTextPreparationQueueState {
    entries: HashMap<FormattedTextPreparationQueueJobKey, Arc<FormattedTextPreparationQueueEntry>>,
    pending: VecDeque<Arc<FormattedTextPreparationQueueEntry>>,
    running: Option<Arc<FormattedTextPreparationQueueEntry>>,
    retained_cost: i64,
    is_draining: bool,
}

/// one background lane with count and retained input limits from the existing text policy
/// the queue holds weak leases and not an unbounded task tail
pub struct FormattedTextPreparationQueue {
    lock: Mutex<FormattedTextPreparationQueueState>,
    memory_warning_observer: Mutex<Option<RiseSubscription>>,
}

static SHARED_PREPARATION_QUEUE: LazyLock<FormattedTextPreparationQueue> = LazyLock::new(|| {
    let queue = FormattedTextPreparationQueue {
        lock: Mutex::new(FormattedTextPreparationQueueState::default()),
        memory_warning_observer: Mutex::new(None),
    };
    let observer = RiseAppEvents::default_center().observe(
        UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
        |_| FormattedTextPreparationQueue::shared().cancel_pending(),
    );
    *lock(&queue.memory_warning_observer) = Some(observer);
    queue
});

impl FormattedTextPreparationQueue {
    pub fn shared() -> &'static FormattedTextPreparationQueue {
        &SHARED_PREPARATION_QUEUE
    }

    fn cancel_pending(&self) {
        let mut state = lock(&self.lock);
        state.pending.clear();
        state.entries.clear();
        state.retained_cost = 0;
        if let Some(running) = state.running.clone() {
            state.retained_cost = running.cost;
            state.entries.insert(running.key.clone(), running);
        }
    }

    pub fn request<Key, Value>(
        &self,
        key: Key,
        cost: i64,
        build: impl Fn() -> Value + Send + Sync + 'static,
    ) -> Arc<FormattedTextPreparation<Value>>
    where
        Key: Hash + Eq + Send + Sync + 'static,
        Value: Clone + Send + Sync + 'static,
    {
        let mut guard = lock(&self.lock);
        let state = &mut *guard;
        let key = FormattedTextPreparationQueueJobKey { input: Arc::new(key), value_type: TypeId::of::<Value>() };
        if let Some(existing) = state
            .entries
            .get(&key)
            .and_then(|entry| entry.job.upgrade())
            .and_then(|job| job.into_any().downcast::<FormattedTextPreparation<Value>>().ok())
        {
            return existing;
        }
        let work = FormattedTextPreparation::with_build(build);
        if let Some(expired) = state.entries.remove(&key) {
            state.retained_cost -= expired.cost;
            state.pending.retain(|entry| !Arc::ptr_eq(entry, &expired));
        }
        let entry_limit = to_cache_units(FormattedTextPreparationCachePolicy::preparation_entry_limit());
        let input_limit = FormattedTextPreparationCachePolicy::preparation_input_cost_limit();
        if state.entries.len() >= entry_limit || cost > input_limit - state.retained_cost {
            let FormattedTextPreparationQueueState { entries, pending, retained_cost, .. } = &mut *state;
            pending.retain(|entry| {
                if entry.job.strong_count() > 0 {
                    return true;
                }
                entries.remove(&entry.key);
                *retained_cost -= entry.cost;
                false
            });
        }
        if state.entries.len() >= entry_limit || cost > input_limit - state.retained_cost {
            return work;
        }
        let job: Arc<dyn FormattedTextPreparationJob> = work.clone();
        let entry = Arc::new(FormattedTextPreparationQueueEntry { key: key.clone(), cost, job: Arc::downgrade(&job) });
        state.entries.insert(key, entry.clone());
        state.retained_cost += cost;
        state.pending.push_back(entry);
        if !state.is_draining {
            state.is_draining = true;
            let spawned = std::thread::Builder::new()
                .name("rise.formatted-text.prepare".to_owned())
                .spawn(|| FormattedTextPreparationQueue::shared().drain());
            if spawned.is_err() {
                state.is_draining = false;
            }
        }
        work
    }

    fn drain(&self) {
        loop {
            let (entry, job) = {
                let mut state = lock(&self.lock);
                let Some(entry) = state.pending.pop_front() else {
                    state.is_draining = false;
                    return;
                };
                let job = entry.job.upgrade();
                state.running = Some(entry.clone());
                (entry, job)
            };
            if let Some(job) = job {
                job.prepare_if_needed();
            }
            let mut state = lock(&self.lock);
            if state.entries.get(&entry.key).is_some_and(|current| Arc::ptr_eq(current, &entry)) {
                state.entries.remove(&entry.key);
                state.retained_cost -= entry.cost;
            }
            state.running = None;
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct FormattedTextSourceKey {
    text: String,
    options: FormattedTextOptions,
}

type FormattedTextResultKey = FormattedTextRenderSeed;

struct FormattedTextSourceAnalysis {
    inline: RichTextMarkupInlineResult,
    links: Vec<Range<usize>>,
    mentions: Vec<FormattedTextMentionToken>,
    urls: Vec<FormattedTextMentionToken>,
    subtitle: Option<String>,
    has_code_block: bool,
}

static SOURCE_CACHE: LazyLock<FormattedTextBoundedCache<FormattedTextSourceKey, Arc<FormattedTextSourceAnalysis>>> =
    LazyLock::new(|| {
        FormattedTextBoundedCache::new(
            FormattedTextPreparationCachePolicy::entry_limit(),
            FormattedTextPreparationCachePolicy::source_cost_limit(),
        )
    });

static FORMATTED_CACHE: LazyLock<FormattedTextBoundedCache<FormattedTextResultKey, Arc<FormattedTextResult>>> =
    LazyLock::new(|| {
        FormattedTextBoundedCache::new(
            FormattedTextPreparationCachePolicy::entry_limit(),
            FormattedTextPreparationCachePolicy::formatted_cost_limit(),
        )
    });

static GEOMETRY_CACHE: LazyLock<
    FormattedTextBoundedCache<FormattedTextGeometryCacheKey, Arc<FormattedTextPreparedGeometry>>,
> = LazyLock::new(|| {
    FormattedTextBoundedCache::new(
        FormattedTextPreparationCachePolicy::entry_limit(),
        FormattedTextPreparationCachePolicy::geometry_cost_limit(),
    )
});

static LINK_DETECTOR: LazyLock<Option<NSDataDetector>> =
    LazyLock::new(|| NSDataDetector::new(NSTextCheckingType::LINK).ok());

static HASHTAG_MAXIMUM_LENGTH: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("max_hashtag_len")).unwrap_or(0));

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattedTextMentionToken {
    pub tag: String,
    pub range: NSRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormattedTextCaptionLinkTokenKind {
    Mention(Url),
    Hashtag(String),
}

/// one tappable run of a plain caption, a mention with its public tag url or a hashtag
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattedTextCaptionLinkToken {
    pub kind: FormattedTextCaptionLinkTokenKind,
    pub range: NSRange,
}

/// one hashtag run, lowercased tag cut to the server length by whole characters
/// plus its range in the text and whether the kept tag has a non digit
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattedTextHashtagRun {
    pub tag: String,
    pub range: NSRange,
    pub has_letter: bool,
}

pub enum FormattedText {}

impl FormattedText {
    pub const HASHTAG_LINK_SCHEME: &'static str = "riseonly-post-hashtag";

    /// can be called directly by the bounded presentation worker before publishing a row
    /// uses the same caches and inputs as the view
    pub fn prepare(seed: &FormattedTextRenderSeed) -> FormattedTextInlinePreparation {
        Self::inline_preparation(&seed.text, &seed.style, &seed.options, &seed.role_mentions)
    }

    pub fn request_formatting(seed: &FormattedTextRenderSeed) -> Arc<FormattedTextPreparation<Arc<FormattedTextResult>>> {
        if let Some(cached) = FORMATTED_CACHE.cached_value(seed) {
            return FormattedTextPreparation::with_value(cached);
        }
        let cost = FormattedTextPreparationCachePolicy::formatted_cost(&seed.text, seed.role_mentions.len())
            + FormattedTextPreparationCachePolicy::option_cost(&seed.options);
        let owned = seed.clone();
        FormattedTextPreparationQueue::shared().request(seed.clone(), cost, move || {
            Self::format(&owned.text, &owned.style, &owned.options, &owned.role_mentions)
        })
    }

    pub fn request_preparation(
        seed: &FormattedTextRenderSeed,
    ) -> Arc<FormattedTextPreparation<FormattedTextInlinePreparation>> {
        if let (Some(formatted), Some(geometry)) = (FORMATTED_CACHE.cached_value(seed), GEOMETRY_CACHE.cached_value(seed)) {
            return FormattedTextPreparation::with_value(FormattedTextInlinePreparation { formatted, geometry });
        }
        let cost = FormattedTextPreparationCachePolicy::formatted_cost(&seed.text, seed.role_mentions.len())
            + FormattedTextPreparationCachePolicy::geometry_cost(&seed.text)
            + FormattedTextPreparationCachePolicy::option_cost(&seed.options);
        let owned = seed.clone();
        FormattedTextPreparationQueue::shared().request(seed.clone(), cost, move || Self::prepare(&owned))
    }

    pub fn format(
        text: &str,
        style: &FormattedTextStyle,
        options: &FormattedTextOptions,
        role_mentions: &[RoleMentionSpan],
    ) -> Arc<FormattedTextResult> {
        let key = FormattedTextResultKey::new(text, style.clone(), options.clone(), role_mentions.to_vec());
        FORMATTED_CACHE.value(
            key,
            FormattedTextPreparationCachePolicy::formatted_cost(text, role_mentions.len())
                + FormattedTextPreparationCachePolicy::option_cost(options),
            FormattedTextPreparationCachePolicy::can_cache(text),
            |_| 0,
            || {
                let source = Self::source_analysis(text, options);
                let body = Self::attributed_body(
                    &source.inline.display,
                    &source.inline.spans,
                    &source.inline.removed_ranges,
                    style,
                    &source.links,
                    &source.mentions,
                    role_mentions,
                    &options.highlight_words,
                    options.link_urls,
                );
                let spoilers = Self::spoiler_ranges(&source.inline.spans);
                let mut plate = style.base_color;
                plate.a *= 0.28;
                let hidden = Self::hiding_spoilers(&body, &spoilers, &source.inline.display, Some(plate));
                let has_spoilers = hidden.is_some();
                Arc::new(FormattedTextResult {
                    attributed_body: hidden.unwrap_or_else(|| body.clone()),
                    display_string: source.inline.display.clone(),
                    subtitle: source.subtitle.clone(),
                    has_url: !source.links.is_empty(),
                    has_mention: !source.mentions.is_empty() || !role_mentions.is_empty(),
                    has_code_block: source.has_code_block,
                    revealed_body: has_spoilers.then_some(body),
                    spoiler_ranges: if has_spoilers { spoilers } else { Vec::new() },
                    spoiler_plate: plate,
                })
            },
        )
    }

    pub fn display_string(text: &str, options: &FormattedTextOptions) -> String {
        Self::source_analysis(text, options).inline.display.clone()
    }

    pub fn has_url(text: &str, options: &FormattedTextOptions) -> bool {
        if options.no_url_format {
            return false;
        }
        !Self::source_analysis(text, options).links.is_empty()
    }

    pub fn geometry_string(
        text: &str,
        style: &FormattedTextStyle,
        options: &FormattedTextOptions,
    ) -> (String, AttributedString) {
        let prepared = Self::prepared_geometry(text, style, options, &[]);
        (prepared.display.clone(), prepared.attributed.clone())
    }

    pub fn prepared_geometry(
        text: &str,
        style: &FormattedTextStyle,
        options: &FormattedTextOptions,
        role_mentions: &[RoleMentionSpan],
    ) -> Arc<FormattedTextPreparedGeometry> {
        let key = FormattedTextGeometryCacheKey::new(text, style.clone(), options.clone(), role_mentions.to_vec());
        let cache_key = key.clone();
        GEOMETRY_CACHE.value(
            key,
            FormattedTextPreparationCachePolicy::geometry_cost(text)
                + FormattedTextPreparationCachePolicy::role_mention_cost(role_mentions)
                + FormattedTextPreparationCachePolicy::option_cost(options),
            FormattedTextPreparationCachePolicy::can_cache(text),
            |_| 0,
            || {
                let source = Self::source_analysis(text, options);
                Arc::new(FormattedTextPreparedGeometry {
                    display: source.inline.display.clone(),
                    attributed: Self::geometry_attributed_string(
                        &source.inline.display,
                        &source.inline.spans,
                        &source.inline.removed_ranges,
                        style,
                        role_mentions,
                    ),
                    cache_key,
                })
            },
        )
    }

    pub fn inline_preparation(
        text: &str,
        style: &FormattedTextStyle,
        options: &FormattedTextOptions,
        role_mentions: &[RoleMentionSpan],
    ) -> FormattedTextInlinePreparation {
        FormattedTextInlinePreparation {
            formatted: Self::format(text, style, options, role_mentions),
            geometry: Self::prepared_geometry(text, style, options, role_mentions),
        }
    }

    pub fn interaction_preparation(
        text: &str,
        style: &FormattedTextStyle,
        options: &FormattedTextOptions,
        role_mentions: &[RoleMentionSpan],
        geometry: Option<Arc<FormattedTextPreparedGeometry>>,
    ) -> FormattedTextInteractionPreparation {
        let source = Self::source_analysis(text, options);
        let key = FormattedTextRenderSeed::new(text, style.clone(), options.clone(), role_mentions.to_vec());
        let resolved_geometry = match geometry {
            Some(geometry) if geometry.cache_key == key => geometry,
            _ => Self::prepared_geometry(text, style, options, role_mentions),
        };
        let span_links = role_mentions.iter().filter_map(|span| {
            let link = span.link.as_ref()?;
            let range = RichTextMarkup::display_range(span.range, &source.inline.removed_ranges)?;
            Some(FormattedTextMentionToken { tag: link.clone(), range })
        });
        FormattedTextInteractionPreparation {
            geometry: resolved_geometry,
            mentions: source.mentions.clone(),
            urls: source.urls.iter().cloned().chain(span_links).collect(),
            spoilers: Self::spoiler_ranges(&source.inline.spans)
                .into_iter()
                .enumerate()
                .map(|(offset, range)| FormattedTextMentionToken { tag: offset.to_string(), range })
                .collect(),
        }
    }

    fn source_analysis(text: &str, options: &FormattedTextOptions) -> Arc<FormattedTextSourceAnalysis> {
        let key = FormattedTextSourceKey { text: text.to_owned(), options: options.clone() };
        SOURCE_CACHE.value(
            key,
            FormattedTextPreparationCachePolicy::source_cost(text) + FormattedTextPreparationCachePolicy::option_cost(options),
            FormattedTextPreparationCachePolicy::can_cache(text),
            |_| 0,
            || {
                let inline = if options.no_markdown {
                    RichTextMarkupInlineResult { display: text.to_owned(), spans: Vec::new(), removed_ranges: Vec::new() }
                } else if options.literal_code_blocks {
                    RichTextMarkup::parse_inline_keeping_code_blocks_literal(text)
                } else {
                    RichTextMarkup::parse_inline(text)
                };
                let links = if options.no_url_format {
                    Vec::new()
                } else {
                    Self::url_ranges(&inline.display, options.secure_urls_only)
                };
                let mentions = if options.no_mention_format { Vec::new() } else { Self::mention_tokens(&inline.display) };
                let subtitle = if options.no_url_format || options.no_url_subtitle {
                    None
                } else {
                    Self::link_subtitle(&inline.display, &links)
                };
                let urls = Self::url_tokens_in_ranges(&inline.display, &links);
                Arc::new(FormattedTextSourceAnalysis {
                    inline,
                    links,
                    mentions,
                    urls,
                    subtitle,
                    has_code_block: !options.no_markdown
                        && !options.literal_code_blocks
                        && RichTextMarkup::has_code_block(text),
                })
            },
        )
    }

    fn geometry_attributed_string(
        display: &str,
        spans: &[RichInlineSpan],
        removed: &[NSRange],
        style: &FormattedTextStyle,
        role_mentions: &[RoleMentionSpan],
    ) -> AttributedString {
        let base_font = RichFont::system(style.font_size);
        let mut attr = AttributedString::new(display);
        attr.update_all(|container| container.font = Some(base_font));
        let length = ns_range::length_utf16(display);
        for (range, bold, italic) in RichTextMarkup::emphasis_runs(spans) {
            if range.location + range.length > length {
                continue;
            }
            let mut traits = FontDescriptorSymbolicTraits::default();
            if bold {
                traits = traits.union(FontDescriptorSymbolicTraits::TRAIT_BOLD);
            }
            if italic {
                traits = traits.union(FontDescriptorSymbolicTraits::TRAIT_ITALIC);
            }
            let font = base_font.with_traits(traits);
            attr.update(range, |container| container.font = Some(font));
        }
        for span in spans {
            if span.range.location + span.range.length > length {
                continue;
            }
            match span.style {
                RichInlineStyle::Bold | RichInlineStyle::Italic | RichInlineStyle::Strikethrough | RichInlineStyle::Spoiler => {}
                RichInlineStyle::Code => {
                    let font = RichFont::system_design(style.font_size * 0.95, FontDesign::Monospaced);
                    attr.update(span.range, |container| container.font = Some(font));
                }
            }
        }
        for span in role_mentions {
            let Some(range) = RichTextMarkup::display_range(span.range, removed) else {
                continue;
            };
            if range.location + range.length > length {
                continue;
            }
            let font = RichFont::system_weight(style.font_size, FontWeight::SEMIBOLD);
            attr.update(range, |container| container.font = Some(font));
        }
        attr
    }

    pub fn contains_link(text: &str) -> bool {
        let Some(detector) = LINK_DETECTOR.as_ref() else {
            return false;
        };
        detector.first_match(text, NSMatchingOptions::NONE, NSRange::full(text)).is_some()
    }

    fn link_subtitle(text: &str, ranges: &[Range<usize>]) -> Option<String> {
        let range = ranges.first()?;
        let raw = trimming_whitespaces_and_newlines(text.get(range.clone())?);
        let url = Url::from_string(raw)?;
        let host = url.host()?;
        let frag = match url.fragment() {
            Some(fragment) if !fragment.is_empty() => format!("#{fragment}"),
            _ => String::new(),
        };
        let query = url.query().map(|query| format!("?{query}")).unwrap_or_default();
        let mut tail = format!("{}{query}{frag}", url.path());
        if tail.is_empty() || tail == "/" {
            return Some(host);
        }
        if character_count(&tail) > 48 {
            tail = format!("{}\u{2026}", character_prefix(&tail, 45));
        }
        Some(format!("{host}{tail}"))
    }

    #[allow(clippy::too_many_arguments)]
    fn attributed_body(
        display: &str,
        spans: &[RichInlineSpan],
        removed: &[NSRange],
        style: &FormattedTextStyle,
        link_ranges: &[Range<usize>],
        mentions: &[FormattedTextMentionToken],
        role_mentions: &[RoleMentionSpan],
        highlight_words: &[String],
        link_urls: bool,
    ) -> AttributedString {
        let mut a = AttributedString::new(display);
        a.update_all(|container| {
            container.font = Some(RichFont::system(style.font_size));
            container.foreground_color = Some(style.base_color);
        });

        for (range, bold, italic) in RichTextMarkup::emphasis_runs(spans) {
            let font = match (bold, italic) {
                (true, true) => RichFont::system_weight(style.font_size, FontWeight::BOLD).italic(),
                (true, false) => RichFont::system_weight(style.font_size, FontWeight::BOLD),
                _ => RichFont::system(style.font_size).italic(),
            };
            a.update(range, |container| container.font = Some(font));
        }
        for span in spans {
            match span.style {
                RichInlineStyle::Bold | RichInlineStyle::Italic | RichInlineStyle::Spoiler => {}
                RichInlineStyle::Strikethrough => {
                    a.update(span.range, |container| container.strikethrough_style = Some(LineStyle::single()));
                }
                RichInlineStyle::Code => {
                    let font = RichFont::system_design(style.font_size * 0.95, FontDesign::Monospaced);
                    let foreground = style.resolved_code_foreground();
                    let background = style.resolved_code_background();
                    a.update(span.range, |container| {
                        container.font = Some(font);
                        container.foreground_color = Some(foreground);
                        container.background_color = Some(background);
                    });
                }
            }
        }

        for range in link_ranges {
            let url = if link_urls { display.get(range.clone()).and_then(Self::tap_url) } else { None };
            a.update_byte_range(range.clone(), |container| {
                container.foreground_color = Some(style.link_color);
                if style.link_underline {
                    container.underline_style = Some(LineStyle::single());
                }
                if let Some(url) = url.as_ref() {
                    container.link = Some(url.clone());
                }
            });
        }
        for token in mentions {
            let url = RiseonlyPublicLink::PublicTag { tag: token.tag.clone() }.web_url(AppEnvironment::current());
            a.update(token.range, |container| {
                container.foreground_color = Some(style.link_color);
                if style.link_underline {
                    container.underline_style = Some(LineStyle::single());
                }
                if let Some(url) = url.as_ref() {
                    container.link = Some(url.clone());
                }
            });
        }
        if let Some(hashtag_color) = style.hashtag_color {
            let mut taken: Vec<NSRange> = link_ranges
                .iter()
                .map(|range| ns_range::byte_to_utf16_range(display, range.clone()))
                .collect();
            taken.extend(mentions.iter().map(|token| token.range));
            taken.extend(spans.iter().filter(|span| span.style == RichInlineStyle::Code).map(|span| span.range));
            for token in Self::hashtag_tokens(display, &taken) {
                let url = Self::hashtag_url(&token.tag);
                a.update(token.range, |container| {
                    container.foreground_color = Some(hashtag_color);
                    container.link = url.clone();
                });
            }
        }
        if let Some(accent) = style.accent.as_ref().filter(|accent| !accent.word.is_empty())
            && let Some(found) = ns_range_of(display, &accent.word, CompareOptions::CASE_INSENSITIVE, NSRange::full(display))
        {
            a.update(found, |container| {
                container.foreground_color = Some(accent.color);
                container.font = Some(RichFont::system_weight(style.font_size, FontWeight::BOLD));
            });
        }
        let display_length = ns_range::length_utf16(display);
        for span in role_mentions {
            let Some(display_range) = RichTextMarkup::display_range(span.range, removed) else {
                continue;
            };
            if display_range.location + display_range.length > display_length {
                continue;
            }
            a.update(display_range, |container| {
                if style.link_underline {
                    container.foreground_color = Some(style.link_color);
                    container.underline_style = Some(LineStyle::single());
                } else {
                    container.foreground_color = Some(if span.link.is_none() { span.color } else { style.link_color });
                }
                container.font = Some(RichFont::system_weight(style.font_size, FontWeight::SEMIBOLD));
                container.link = None;
            });
        }
        if !highlight_words.is_empty() {
            let match_fill = ChatSearchHighlightPalette::match_fill_color();
            for word in highlight_words.iter().filter(|word| !word.is_empty()) {
                let mut search_range = NSRange::full(display);
                while search_range.length > 0 {
                    let Some(found) = ns_range_of(display, word, CompareOptions::LITERAL, search_range) else {
                        break;
                    };
                    a.update(found, |container| container.background_color = Some(match_fill));
                    let next = found.location + found.length;
                    search_range = NSRange::new(next, display_length.saturating_sub(next));
                }
            }
        }
        a
    }

    /// spoiler runs as a solid plate of the text color or clear with no plate
    /// glyphs keep their metrics so measured geometry stays valid
    /// links inside arent tappable until revealed
    pub fn hiding_spoilers(
        body: &AttributedString,
        ranges: &[NSRange],
        display: &str,
        plate: Option<Hsla>,
    ) -> Option<AttributedString> {
        if ranges.is_empty() {
            return None;
        }
        let mut hidden = body.clone();
        for range in ranges {
            hidden.update(*range, |container| {
                container.foreground_color = Some(transparent_black());
                container.background_color = plate;
                container.strikethrough_style = None;
                container.link = None;
            });
        }
        Some(hidden)
    }

    /// spoiler display ranges in source order, the tap layer and the body index them the same way
    pub fn spoiler_ranges(spans: &[RichInlineSpan]) -> Vec<NSRange> {
        spans
            .iter()
            .filter(|span| span.style == RichInlineStyle::Spoiler)
            .map(|span| span.range)
            .collect()
    }

    fn url_ranges(text: &str, secure_only: bool) -> Vec<Range<usize>> {
        let Some(detector) = LINK_DETECTOR.as_ref() else {
            return Vec::new();
        };
        detector
            .matches(text, NSMatchingOptions::NONE, NSRange::full(text))
            .into_iter()
            .filter(|found| found.result_type == NSTextCheckingType::LINK)
            .filter_map(|found| ns_range::utf16_to_byte_range(text, found.range))
            .filter(|range| !(secure_only && text.get(range.clone()).is_some_and(Self::is_plain_http)))
            .collect()
    }

    /// post text rules for a caption drawn without markup, mentions and hashtags
    /// an address stays plain text since links live only in stories, sorted by position
    pub fn caption_link_tokens(text: &str) -> Vec<FormattedTextCaptionLinkToken> {
        if text.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<FormattedTextCaptionLinkToken> = Vec::new();
        let link_ranges: Vec<NSRange> = Self::url_ranges(text, false)
            .into_iter()
            .map(|range| ns_range::byte_to_utf16_range(text, range))
            .collect();
        let mentions: Vec<FormattedTextMentionToken> = Self::mention_tokens(text)
            .into_iter()
            .filter(|token| !link_ranges.iter().any(|range| range.intersection(token.range).is_some_and(|both| both.length > 0)))
            .collect();
        for token in &mentions {
            let Some(url) = RiseonlyPublicLink::PublicTag { tag: token.tag.clone() }.web_url(AppEnvironment::current()) else {
                continue;
            };
            out.push(FormattedTextCaptionLinkToken { kind: FormattedTextCaptionLinkTokenKind::Mention(url), range: token.range });
        }
        let mut taken = link_ranges;
        taken.extend(mentions.iter().map(|token| token.range));
        for token in Self::hashtag_tokens(text, &taken) {
            out.push(FormattedTextCaptionLinkToken { kind: FormattedTextCaptionLinkTokenKind::Hashtag(token.tag), range: token.range });
        }
        out.sort_by_key(|token| token.range.location);
        out
    }

    pub fn is_plain_http(raw: &str) -> bool {
        trimming_whitespaces_and_newlines(raw).to_lowercase().starts_with("http://")
    }

    /// address a detected run opens, as written if it has a scheme
    /// mail gets mailto and a bare address goes over https
    pub fn tap_url(for_link: &str) -> Option<Url> {
        let trimmed = trimming_whitespaces_and_newlines(for_link);
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains("://") {
            return Url::from_string(trimmed);
        }
        if let Some(url) = LINK_DETECTOR
            .as_ref()
            .and_then(|detector| detector.first_match(trimmed, NSMatchingOptions::NONE, NSRange::full(trimmed)))
            .and_then(|found| found.url)
            && url.scheme().is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto"))
        {
            return Some(url);
        }
        Url::from_string(&format!("https://{trimmed}"))
    }

    pub fn url_tokens(text: &str) -> Vec<FormattedTextMentionToken> {
        Self::url_tokens_in_ranges(text, &Self::url_ranges(text, false))
    }

    fn url_tokens_in_ranges(text: &str, ranges: &[Range<usize>]) -> Vec<FormattedTextMentionToken> {
        ranges
            .iter()
            .filter_map(|range| {
                let url = text.get(range.clone())?;
                if url.is_empty() {
                    return None;
                }
                Some(FormattedTextMentionToken {
                    tag: url.to_owned(),
                    range: ns_range::byte_to_utf16_range(text, range.clone()),
                })
            })
            .collect()
    }

    /// hashtags the way telegram finds them, a # not glued to a word or link path
    /// then letters digits and underscore in any script
    /// one rule for post tags and tappable tags in text
    pub fn hashtag_runs(text: &str) -> Vec<FormattedTextHashtagRun> {
        if !text.contains('#') {
            return Vec::new();
        }
        let chars: Vec<Character<'_>> = characters(text).collect();
        let mut offsets = Vec::with_capacity(chars.len() + 1);
        let mut offset = 0;
        for character in &chars {
            offsets.push(offset);
            offset += character.as_str().len();
        }
        offsets.push(offset);

        let maximum_length = *HASHTAG_MAXIMUM_LENGTH;
        let mut out = Vec::new();
        let mut previous: Option<Character<'_>> = None;
        let mut index = 0;
        while index < chars.len() {
            let character = chars[index];
            let body_start = index + 1;
            let glued = previous.is_some_and(|previous| Self::is_hashtag_character(&previous) || previous.as_str() == "/");
            if character.as_str() != "#" || glued {
                previous = Some(character);
                index = body_start;
                continue;
            }
            let mut end = body_start;
            let mut tag = String::new();
            let mut length = 0;
            let mut has_letter = false;
            while end < chars.len() && Self::is_hashtag_character(&chars[end]) {
                let next = chars[end];
                let lowered = next.lowercased();
                let scalars = lowered.chars().count();
                if length + scalars <= maximum_length {
                    tag.push_str(&lowered);
                    length += scalars;
                    has_letter = has_letter || !next.is_number();
                }
                end += 1;
            }
            if !tag.is_empty() {
                out.push(FormattedTextHashtagRun {
                    tag,
                    range: ns_range::byte_to_utf16_range(text, offsets[index]..offsets[end]),
                    has_letter,
                });
            }
            previous = Some(if end > body_start { chars[end - 1] } else { character });
            index = end;
        }
        out
    }

    /// tappable hashtag runs with at least one non digit, skips runs inside excluding
    pub fn hashtag_tokens(text: &str, excluding: &[NSRange]) -> Vec<FormattedTextMentionToken> {
        Self::hashtag_runs(text)
            .into_iter()
            .filter(|run| run.has_letter)
            .filter(|run| {
                !excluding
                    .iter()
                    .any(|taken| taken.intersection(run.range).is_some_and(|common| common.length > 0))
            })
            .map(|run| FormattedTextMentionToken { tag: run.tag, range: run.range })
            .collect()
    }

    pub fn hashtag_url(tag: &str) -> Option<Url> {
        let mut components = UrlComponents::new();
        components.set_scheme(Some(Self::HASHTAG_LINK_SCHEME));
        components.set_path(tag);
        components.url()
    }

    /// tag of a link made by hashtag_url, none for any other link
    pub fn hashtag(from: &Url) -> Option<String> {
        if from.scheme() != Some(Self::HASHTAG_LINK_SCHEME) {
            return None;
        }
        let tag = UrlComponents::from_url(from, false)?.path();
        if tag.is_empty() {
            return None;
        }
        Some(tag)
    }

    fn is_hashtag_character(character: &Character<'_>) -> bool {
        character.is_letter() || character.is_number() || character.as_str() == "_"
    }

    pub fn mention_tokens(text: &str) -> Vec<FormattedTextMentionToken> {
        let Some(rx) = MENTION.as_ref() else {
            return Vec::new();
        };
        if text.is_empty() {
            return Vec::new();
        }
        rx.matches(text, NSMatchingOptions::NONE, NSRange::full(text))
            .into_iter()
            .filter_map(|found| {
                if found.number_of_ranges() < 2 {
                    return None;
                }
                let tag_range = found.range_at(1)?;
                if tag_range.length == 0 || tag_range.location < 1 {
                    return None;
                }
                let tag = ns_range::substring(text, tag_range)?.to_owned();
                let highlight = NSRange::new(tag_range.location - 1, tag_range.length + 1);
                Some(FormattedTextMentionToken { tag, range: highlight })
            })
            .collect()
    }
}

/// mouse selection of text on a surface that owns the press like a chat bubble
/// the highlight and the two questions the surface answers for it
#[derive(Clone)]
pub struct FormattedTextSelection {
    pub color: Hsla,
    pub press_gate: Rc<dyn Fn(&mut App) -> bool>,
    pub single_click_when: Option<Rc<dyn Fn(&mut App) -> bool>>,
}

#[derive(IntoElement)]
pub struct FormattedTextUi {
    id: ElementId,
    style: FormattedTextStyle,
    shows_subtitle: bool,
    preformatted: Option<Arc<FormattedTextResult>>,
    external_spoilers: Option<Binding<HashSet<usize>>>,
    text_selection: Option<FormattedTextSelection>,
    seed: FormattedTextRenderSeed,
}

impl FormattedTextUi {
    pub fn new(id: impl Into<ElementId>, text: impl Into<String>, style: FormattedTextStyle) -> Self {
        let seed = FormattedTextRenderSeed::new(text, style.clone(), FormattedTextOptions::default(), Vec::new());
        Self {
            id: id.into(),
            style,
            shows_subtitle: true,
            preformatted: None,
            external_spoilers: None,
            text_selection: None,
            seed,
        }
    }

    pub fn options(mut self, options: FormattedTextOptions) -> Self {
        self.seed.options = options;
        self
    }

    pub fn shows_subtitle(mut self, shows_subtitle: bool) -> Self {
        self.shows_subtitle = shows_subtitle;
        self
    }

    pub fn role_mentions(mut self, role_mentions: Vec<RoleMentionSpan>) -> Self {
        self.seed.role_mentions = role_mentions;
        self
    }

    pub fn preformatted(mut self, preformatted: Option<Arc<FormattedTextResult>>) -> Self {
        self.preformatted = preformatted;
        self
    }

    pub fn revealed_spoilers(mut self, revealed_spoilers: Option<Binding<HashSet<usize>>>) -> Self {
        self.external_spoilers = revealed_spoilers;
        self
    }

    pub fn text_selection(mut self, text_selection: Option<FormattedTextSelection>) -> Self {
        self.text_selection = text_selection;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    /// same text as the rich one but selectable
    /// link runs keep their look and a tap on them goes to the hit layer since the press surface
    /// owner routes taps through it
    pub fn selectable_body(id: ElementId, text: AttributedString, selection: FormattedTextSelection) -> AnyElement {
        let characters: SharedString = text.characters().to_owned().into();
        let highlights = text.highlights(None);
        let overrides = RichText::new(id.clone(), text).font_family_overrides();
        let press_gate = selection.press_gate;
        let mut body = SelectableText::new(id, characters)
            .with_highlights(highlights)
            .hosted_in_press_surface()
            .selection_color(selection.color)
            .press_gate(move |cx: &mut App| press_gate(cx));
        if !overrides.is_empty() {
            body = body.with_font_family_overrides(overrides);
        }
        if let Some(single_click_when) = selection.single_click_when {
            body = body.single_click_when(move |cx: &mut App| single_click_when(cx));
        }
        body.into_any_element()
    }
}

impl RenderOnce for FormattedTextUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let preparation = match self.preformatted.clone() {
            Some(preformatted) => FormattedTextPreparation::with_value(preformatted),
            None => FormattedText::request_formatting(&self.seed),
        };
        let formatted = preparation.value_for_immediate_display();
        let own_spoilers = window.use_keyed_state(self.child_id("own-spoilers"), cx, |_, _| OwnSpoilers::default());
        let now = RiseClock::monotonic();
        let (revealed, fading) = match self.external_spoilers.as_ref() {
            Some(binding) => (binding.get(cx), None),
            None => {
                let own = own_spoilers.read(cx);
                let progress = own.reveal.frame_value(now, window);
                let fading = (progress < 1.0).then(|| (own.revealing.clone(), progress));
                (own.revealed.clone(), fading)
            }
        };
        let all = formatted.spoiler_indices();
        let mut text = formatted.body(&revealed, true);
        if let Some((revealing, progress)) = fading {
            fade_in_spoilers(&mut text, &formatted.spoiler_ranges, &revealing, progress, self.style.base_color);
        }

        let rich_text = match self.text_selection.clone() {
            Some(selection) => Self::selectable_body(self.child_id("body"), text, selection),
            None => RichText::new(self.child_id("body"), text).into_any_element(),
        };
        let body = div()
            .w_full()
            .swiftui_font(Font::system_size(self.style.font_size))
            .line_height(px(self.style.font_size * NATURAL_LINE_HEIGHT + self.style.line_spacing))
            .text_color(self.style.base_color)
            .child(rich_text)
            .spoiler_dust(&self.seed, None, &all, &revealed, window, cx);

        let reveal_all = if self.external_spoilers.is_none() { all } else { HashSet::new() };
        let body = spoiler_reveal(body, self.child_id("spoiler-reveal"), reveal_all, &revealed, own_spoilers);

        let mut stack = vstack(4.0).items_start().w_full().min_w_0().child(body);
        if self.shows_subtitle
            && let Some(sub) = formatted.subtitle.clone()
        {
            stack = stack.child(
                div()
                    .w_full()
                    .swiftui_font(Font::system_size(self.style.subtitle_font_size))
                    .line_height(px(self.style.subtitle_font_size * NATURAL_LINE_HEIGHT + 2.0))
                    .text_color(self.style.subtitle_color)
                    .child(SharedString::from(sub)),
            );
        }
        stack
    }
}

/// own spoilers with the transaction it was last set in
/// text crossfades the runs that open so runs of the last reveal
/// and its progress are kept beside the set
pub struct OwnSpoilers {
    pub revealed: HashSet<usize>,
    pub revealing: HashSet<usize>,
    pub reveal: Animated<f32>,
}

impl Default for OwnSpoilers {
    fn default() -> Self {
        Self { revealed: HashSet::new(), revealing: HashSet::new(), reveal: Animated::new(1.0) }
    }
}

fn fade_in_spoilers(
    body: &mut AttributedString,
    ranges: &[NSRange],
    revealing: &HashSet<usize>,
    progress: f32,
    base_color: Hsla,
) {
    for (index, range) in ranges.iter().enumerate() {
        if !revealing.contains(&index) {
            continue;
        }
        body.update(*range, |container| {
            let mut color = container.foreground_color.unwrap_or(base_color);
            color.a *= progress;
            container.foreground_color = Some(color);
        });
    }
}

pub fn spoiler_reveal(
    content: AnyElement,
    id: ElementId,
    all: HashSet<usize>,
    revealed: &HashSet<usize>,
    own_spoilers: gpui::Entity<OwnSpoilers>,
) -> AnyElement {
    if all.is_empty() || all.is_subset(revealed) {
        return content;
    }
    div()
        .relative()
        .w_full()
        .child(content)
        .child(
            hit_area(Shape::Rectangle)
                .id(id)
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let _ = SelectionFeedbackGenerator::new().selection_changed();
                    let all = all.clone();
                    own_spoilers.update(cx, |own, cx| {
                        own.revealing = all.difference(&own.revealed).copied().collect();
                        own.revealed = all;
                        own.reveal.jump(0.0);
                        own.reveal.set_in(1.0, ease_out(0.18), RiseClock::monotonic(), cx);
                        cx.notify();
                    });
                }),
        )
        .into_any_element()
}

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use gpui::App;
use rise_core::foundation::ns_cache::NSCache;

use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListCachePurge, PostRowCaptionLayout, PostRowLayout, PostRowLayoutInputs, PreparedPostRow, PreparedPostRowKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_media_aspect_resolver::PostRowAspectMemo;
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_seed_extractor::PostRowSeedExtractor;

fn utf16_count(text: &str) -> usize {
    text.encode_utf16().count()
}

/// prepared native rows by key, estimated and hosted rows are refused
pub struct PostRowPreparedCache {
    pub count_limit: usize,
    pub total_cost_limit: usize,
    storage: NSCache<PreparedPostRowKey, Arc<PreparedPostRow>>,
}

static PREPARED_SHARED: LazyLock<PostRowPreparedCache> = LazyLock::new(|| PostRowPreparedCache::new(384, 8 * 1_024 * 1_024));

impl PostRowPreparedCache {
    pub fn shared() -> &'static PostRowPreparedCache {
        &PREPARED_SHARED
    }

    pub fn new(count_limit: usize, total_cost_limit: usize) -> Self {
        let count_limit = count_limit.max(1);
        let total_cost_limit = total_cost_limit.max(1);
        let storage = NSCache::new();
        storage.set_count_limit(count_limit);
        storage.set_total_cost_limit(total_cost_limit);
        Self {
            count_limit,
            total_cost_limit,
            storage,
        }
    }

    pub fn row(&self, key: &PreparedPostRowKey) -> Option<Arc<PreparedPostRow>> {
        self.storage.object(key)
    }

    /// false for estimated rows (key has no aspects) and hosted rows (measurer owns their height)
    pub fn insert(&self, row: Arc<PreparedPostRow>) -> bool {
        if row.is_height_estimated || row.layout.is_none() || row.route.is_hosted_fallback() {
            return false;
        }
        let key = row.key.clone();
        let cost = Self::estimated_cost(&row);
        self.storage.set_object_cost(row, key, cost);
        true
    }

    pub fn remove_all(&self) {
        self.storage.remove_all_objects();
    }

    /// cost of the row object and its seed, caption lines are costed by the caption cache
    pub fn estimated_cost(row: &PreparedPostRow) -> usize {
        let seed = &row.seed;
        let text_bytes = (utf16_count(&seed.title) + utf16_count(&seed.content) + utf16_count(&seed.author.name)) * 2;
        let slide_bytes = seed.media.slides.len() * 256;
        1_024 + text_bytes + slide_bytes
    }
}

/// native layouts by their full inputs, shared across posts with equal inputs
pub struct PostRowLayoutCache {
    pub count_limit: usize,
    pub total_cost_limit: usize,
    storage: NSCache<PostRowLayoutInputs, Arc<PostRowLayout>>,
}

static LAYOUT_SHARED: LazyLock<PostRowLayoutCache> = LazyLock::new(|| PostRowLayoutCache::new(384, 4 * 1_024 * 1_024));

impl PostRowLayoutCache {
    pub fn shared() -> &'static PostRowLayoutCache {
        &LAYOUT_SHARED
    }

    pub fn new(count_limit: usize, total_cost_limit: usize) -> Self {
        let count_limit = count_limit.max(1);
        let total_cost_limit = total_cost_limit.max(1);
        let storage = NSCache::new();
        storage.set_count_limit(count_limit);
        storage.set_total_cost_limit(total_cost_limit);
        Self {
            count_limit,
            total_cost_limit,
            storage,
        }
    }

    pub fn layout(&self, inputs: &PostRowLayoutInputs) -> Option<Arc<PostRowLayout>> {
        self.storage.object(inputs)
    }

    pub fn insert(&self, layout: Arc<PostRowLayout>) {
        let inputs = layout.inputs.clone();
        let cost = 1_024 + (utf16_count(&inputs.title) + utf16_count(&inputs.content)) * 2;
        self.storage.set_object_cost(layout, inputs, cost);
    }

    pub fn remove_all(&self) {
        self.storage.remove_all_objects();
    }
}

#[derive(Clone, Debug)]
pub struct PostRowCaptionCacheKey {
    pub title: String,
    pub content: String,
    pub hashtags: Vec<String>,
    pub width: f32,
    pub text_delta: f32,
    pub theme_id: String,
    pub is_bold_text_enabled: bool,
    pub display_scale: f32,
    pub locale_identifier: String,
    pub is_expanded: bool,
}

impl PartialEq for PostRowCaptionCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.width.to_bits() == other.width.to_bits()
            && self.text_delta.to_bits() == other.text_delta.to_bits()
            && self.display_scale.to_bits() == other.display_scale.to_bits()
            && self.is_bold_text_enabled == other.is_bold_text_enabled
            && self.theme_id == other.theme_id
            && self.title == other.title
            && self.content == other.content
            && self.hashtags == other.hashtags
            && self.locale_identifier == other.locale_identifier
            && self.is_expanded == other.is_expanded
    }
}

impl Eq for PostRowCaptionCacheKey {}

impl Hash for PostRowCaptionCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.title.hash(state);
        self.content.hash(state);
        self.hashtags.hash(state);
        self.width.to_bits().hash(state);
        self.text_delta.to_bits().hash(state);
        self.theme_id.hash(state);
        self.is_bold_text_enabled.hash(state);
        self.display_scale.to_bits().hash(state);
        self.locale_identifier.hash(state);
        self.is_expanded.hash(state);
    }
}

/// caption text layouts at origin zero, a like or count change reuses them without shaping
pub struct PostRowCaptionCache {
    pub count_limit: usize,
    pub total_cost_limit: usize,
    storage: NSCache<PostRowCaptionCacheKey, Arc<PostRowCaptionLayout>>,
}

static CAPTION_SHARED: LazyLock<PostRowCaptionCache> = LazyLock::new(|| PostRowCaptionCache::new(256, 16 * 1_024 * 1_024));

impl PostRowCaptionCache {
    pub fn shared() -> &'static PostRowCaptionCache {
        &CAPTION_SHARED
    }

    pub fn new(count_limit: usize, total_cost_limit: usize) -> Self {
        let count_limit = count_limit.max(1);
        let total_cost_limit = total_cost_limit.max(1);
        let storage = NSCache::new();
        storage.set_count_limit(count_limit);
        storage.set_total_cost_limit(total_cost_limit);
        Self {
            count_limit,
            total_cost_limit,
            storage,
        }
    }

    pub fn caption(&self, key: &PostRowCaptionCacheKey) -> Option<Arc<PostRowCaptionLayout>> {
        self.storage.object(key)
    }

    pub fn insert(&self, caption: PostRowCaptionLayout, key: PostRowCaptionCacheKey) {
        let cost = Self::estimated_cost(&key, &caption);
        self.storage.set_object_cost(Arc::new(caption), key, cost);
    }

    pub fn remove_all(&self) {
        self.storage.remove_all_objects();
    }

    /// shaped lines have no public byte count so it is text storage plus a guess per line
    pub fn estimated_cost(key: &PostRowCaptionCacheKey, caption: &PostRowCaptionLayout) -> usize {
        let mut lines = caption.title.as_ref().map_or(0, |block| block.text.line_count())
            + caption.hashtags.as_ref().map_or(0, |block| block.text.line_count());
        for paragraph in &caption.paragraphs {
            lines += paragraph.body.text.line_count()
                + paragraph.link_subtitle.as_ref().map_or(0, |block| block.text.line_count());
        }
        let text_bytes = (utf16_count(&key.title) + utf16_count(&key.content)) * 4
            + key.hashtags.iter().map(|tag| utf16_count(tag) * 4).sum::<usize>();
        (text_bytes + (lines + 1) * 1_024).max(2_048)
    }
}

/// registers every model cache with the purge once
pub enum PostRowModelCaches {}

static IS_REGISTERED: AtomicBool = AtomicBool::new(false);

impl PostRowModelCaches {
    pub fn register_purges_if_needed(cx: &mut App) {
        if IS_REGISTERED.swap(true, Ordering::SeqCst) {
            return;
        }
        PostListCachePurge::register(|_: &mut App| PostRowPreparedCache::shared().remove_all(), cx);
        PostListCachePurge::register(|_: &mut App| PostRowLayoutCache::shared().remove_all(), cx);
        PostListCachePurge::register(|_: &mut App| PostRowCaptionCache::shared().remove_all(), cx);
        PostListCachePurge::register(|_: &mut App| PostRowAspectMemo::shared().remove_all(), cx);
        PostListCachePurge::register(|_: &mut App| PostRowSeedExtractor::remove_all(), cx);
    }
}

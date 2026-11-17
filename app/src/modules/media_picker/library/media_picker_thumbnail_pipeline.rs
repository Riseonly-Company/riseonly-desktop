use std::collections::HashMap;
use std::sync::{Arc, Weak};

use futures::StreamExt;
use futures::channel::mpsc::{self, UnboundedSender};
use gpui::{App, Size};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_cache::NSCache;
use rise_core::rise_isolated::RiseIsolated;
use rise_platform::photo_library::{self, ImageContentMode, ImageDeliveryMode, ImageRequestId, LibraryAsset};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::media_picker::library::photos_image_request::{PhotoImageDelivery, PhotosImageRequest};
use crate::modules::media_picker::shared::media_picker_performance_diagnostics::MediaPickerPerformanceDiagnostics;

pub enum MediaPickerThumbnailSizing {}

impl MediaPickerThumbnailSizing {
    pub fn preview_max_pixel_dimension() -> f32 {
        RiseLimits::client_value("media_picker_thumbnail_sizing_preview_max_pixel_dimension") as f32
    }

    pub fn preview_target(sharp_target: Size<f32>) -> Size<f32> {
        let largest = sharp_target.width.max(sharp_target.height);
        let preview_max_pixel_dimension = Self::preview_max_pixel_dimension();
        let exceeds = largest > preview_max_pixel_dimension;
        if !exceeds {
            return sharp_target;
        }
        let factor = preview_max_pixel_dimension / largest;
        Size {
            width: (sharp_target.width * factor).round().max(1.0),
            height: (sharp_target.height * factor).round().max(1.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MediaPickerThumbnailPipelineQuality {
    Preview,
    Sharp,
}

/// grid cell completion, runs on the main thread once per delivered image
pub type MediaPickerThumbnailPipelineResult = Arc<dyn Fn(Option<CleverImage>, &mut App) + Send + Sync>;

type MediaPickerThumbnailPipelineMainJob = Box<dyn FnOnce(&mut App) + Send>;

#[derive(Clone)]
struct MediaPickerThumbnailPipelinePending {
    request_key: String,
    asset: LibraryAsset,
    target_size: Size<f32>,
    quality: MediaPickerThumbnailPipelineQuality,
    allow_network: bool,
    handler: MediaPickerThumbnailPipelineResult,
    generation: Uuid,
}

#[derive(Default)]
struct MediaPickerThumbnailPipelineState {
    inflight: HashMap<String, Option<ImageRequestId>>,
    generation: HashMap<String, Uuid>,
    active_count: usize,
    wait_queue: Vec<MediaPickerThumbnailPipelinePending>,
    inflight_quality: HashMap<String, MediaPickerThumbnailPipelineQuality>,
    allow_network_while_moving: bool,
}

#[derive(Default)]
struct MediaPickerThumbnailPipelineWork {
    cancels: Vec<ImageRequestId>,
    starts: Vec<MediaPickerThumbnailPipelinePending>,
}

pub struct MediaPickerThumbnailPipelineHandle {
    pub request_key: String,
    pub generation: Option<Uuid>,
    pipeline: Weak<MediaPickerThumbnailPipeline>,
}

impl MediaPickerThumbnailPipelineHandle {
    pub fn cancel(&self) {
        let Some(generation) = self.generation else {
            return;
        };
        if let Some(pipeline) = self.pipeline.upgrade() {
            pipeline.cancel(&self.request_key, generation);
        }
    }
}

pub struct MediaPickerThumbnailPipeline {
    memory_cache: NSCache<String, CleverImage>,
    sync_queue: RiseIsolated<MediaPickerThumbnailPipelineState>,
    main_queue: UnboundedSender<MediaPickerThumbnailPipelineMainJob>,
}

impl MediaPickerThumbnailPipeline {
    const MAX_CONCURRENT: usize = 3;
    const MAX_QUEUE_DEPTH: usize = 24;
    const SHARP_VISIBLE_LIMIT: usize = 40;
    const MEMORY_CACHE_COUNT_LIMIT: usize = 240;
    const MEMORY_CACHE_TOTAL_COST_LIMIT: usize = 32 * 1024 * 1024;

    /// the librarys image manager is a platform seam so nothing to pass in
    pub fn new(cx: &mut App) -> Arc<Self> {
        let memory_cache = NSCache::new();
        memory_cache.set_count_limit(Self::MEMORY_CACHE_COUNT_LIMIT);
        memory_cache.set_total_cost_limit(Self::MEMORY_CACHE_TOTAL_COST_LIMIT);
        let (main_queue, mut main_jobs) = mpsc::unbounded::<MediaPickerThumbnailPipelineMainJob>();
        cx.spawn(async move |cx| {
            while let Some(job) = main_jobs.next().await {
                cx.update(job);
            }
        })
        .detach();
        Arc::new(Self {
            memory_cache,
            sync_queue: RiseIsolated::new(MediaPickerThumbnailPipelineState::default()),
            main_queue,
        })
    }

    pub fn set_allow_network_while_moving(&self, allowed: bool) {
        self.sync_queue.with(|state| state.allow_network_while_moving = allowed);
    }

    pub fn cached_image(
        &self,
        asset_id: &str,
        target_size: Size<f32>,
        quality: MediaPickerThumbnailPipelineQuality,
    ) -> Option<CleverImage> {
        self.memory_cache
            .object(&Self::cache_key(asset_id, target_size, quality))
            .or_else(|| {
                self.memory_cache
                    .object(&Self::cache_key(asset_id, target_size, Self::alternate(quality)))
            })
    }

    pub fn request_grid_thumbnail(
        self: &Arc<Self>,
        asset: &LibraryAsset,
        target_size: Size<f32>,
        quality: MediaPickerThumbnailPipelineQuality,
        allow_network: Option<bool>,
        result: impl Fn(Option<CleverImage>, &mut App) + Send + Sync + 'static,
    ) -> MediaPickerThumbnailPipelineHandle {
        let result: MediaPickerThumbnailPipelineResult = Arc::new(result);
        let asset_id = asset.id.as_str();
        let key = Self::cache_key(asset_id, target_size, quality);
        if let Some(hit) = self.memory_cache.object(&key) {
            let result = Arc::clone(&result);
            self.dispatch_main(move |cx| result(Some(hit), cx));
            MediaPickerPerformanceDiagnostics::shared().record_thumbnail_cache_hit(quality);
            return self.handle(key, None);
        }
        let alternate_quality = Self::alternate(quality);
        if let Some(alternate) = self
            .memory_cache
            .object(&Self::cache_key(asset_id, target_size, alternate_quality))
        {
            let alternate_result = Arc::clone(&result);
            self.dispatch_main(move |cx| alternate_result(Some(alternate), cx));
            if quality == MediaPickerThumbnailPipelineQuality::Preview {
                MediaPickerPerformanceDiagnostics::shared().record_thumbnail_cache_hit(alternate_quality);
                return self.handle(key, None);
            }
        }

        let request_key = key;
        let generation = Uuid::new_v4();
        let mut work = MediaPickerThumbnailPipelineWork::default();
        self.sync_queue.with(|state| {
            Self::cancel_inflight_locked(state, &request_key, &mut work);
            state.generation.insert(request_key.clone(), generation);
            let network = allow_network
                .unwrap_or(quality == MediaPickerThumbnailPipelineQuality::Sharp || state.allow_network_while_moving);
            let pending = MediaPickerThumbnailPipelinePending {
                request_key: request_key.clone(),
                asset: asset.clone(),
                target_size,
                quality,
                allow_network: network,
                handler: result,
                generation,
            };
            Self::enqueue(state, pending, &mut work);
        });
        self.perform(work);
        MediaPickerPerformanceDiagnostics::shared().record_thumbnail_requested(quality);
        self.handle(request_key, Some(generation))
    }

    pub fn request_sharp_visible(
        self: &Arc<Self>,
        assets: Vec<(LibraryAsset, Size<f32>)>,
        result: impl Fn(String, Option<CleverImage>, &mut App) + Send + Sync + 'static,
    ) {
        if assets.is_empty() {
            return;
        }
        let result = Arc::new(result);
        let mut work = MediaPickerThumbnailPipelineWork::default();
        self.sync_queue.with(|state| {
            for (asset, target_size) in assets.into_iter().take(Self::SHARP_VISIBLE_LIMIT) {
                let asset_id = asset.id.clone();
                let key = Self::cache_key(&asset_id, target_size, MediaPickerThumbnailPipelineQuality::Sharp);
                if let Some(hit) = self.memory_cache.object(&key) {
                    let result = Arc::clone(&result);
                    self.dispatch_main(move |cx| result(asset_id, Some(hit), cx));
                    continue;
                }
                let request_key = key;
                if state.inflight.contains_key(&request_key)
                    || state.wait_queue.iter().any(|pending| pending.request_key == request_key)
                {
                    continue;
                }
                Self::cancel_inflight_locked(state, &request_key, &mut work);
                let generation = Uuid::new_v4();
                state.generation.insert(request_key.clone(), generation);
                let item_result = Arc::clone(&result);
                let pending = MediaPickerThumbnailPipelinePending {
                    request_key,
                    asset,
                    target_size,
                    quality: MediaPickerThumbnailPipelineQuality::Sharp,
                    allow_network: true,
                    handler: Arc::new(move |image: Option<CleverImage>, cx: &mut App| item_result(asset_id.clone(), image, cx)),
                    generation,
                };
                Self::enqueue(state, pending, &mut work);
            }
        });
        self.perform(work);
    }

    pub fn cancel(self: &Arc<Self>, request_key: &str, generation: Uuid) {
        let mut work = MediaPickerThumbnailPipelineWork::default();
        self.sync_queue.with(|state| {
            if state.generation.get(request_key) != Some(&generation) {
                return;
            }
            Self::cancel_inflight_locked(state, request_key, &mut work);
            state
                .wait_queue
                .retain(|pending| !(pending.request_key == request_key && pending.generation == generation));
            Self::pump_locked(state, &mut work);
        });
        self.perform(work);
    }

    pub fn trim_wait_queue(&self, max_count: usize) {
        self.sync_queue.with(|state| {
            if state.wait_queue.len() > max_count {
                let excess = state.wait_queue.len() - max_count;
                state.wait_queue.drain(..excess);
            }
        });
    }

    pub fn cancel_all_work_for_movement(self: &Arc<Self>) {
        let mut work = MediaPickerThumbnailPipelineWork::default();
        self.sync_queue.with(|state| {
            state.wait_queue.clear();
            let keys: Vec<String> = state.inflight.keys().cloned().collect();
            for key in keys {
                Self::cancel_inflight_locked(state, &key, &mut work);
            }
            Self::pump_locked(state, &mut work);
            Self::report_pipeline_depth(state);
        });
        self.perform(work);
    }

    fn handle(self: &Arc<Self>, request_key: String, generation: Option<Uuid>) -> MediaPickerThumbnailPipelineHandle {
        MediaPickerThumbnailPipelineHandle {
            request_key,
            generation,
            pipeline: Arc::downgrade(self),
        }
    }

    fn alternate(quality: MediaPickerThumbnailPipelineQuality) -> MediaPickerThumbnailPipelineQuality {
        match quality {
            MediaPickerThumbnailPipelineQuality::Sharp => MediaPickerThumbnailPipelineQuality::Preview,
            MediaPickerThumbnailPipelineQuality::Preview => MediaPickerThumbnailPipelineQuality::Sharp,
        }
    }

    fn dispatch_main(&self, job: impl FnOnce(&mut App) + Send + 'static) {
        let _ = self.main_queue.unbounded_send(Box::new(job));
    }

    fn cancel_inflight_locked(
        state: &mut MediaPickerThumbnailPipelineState,
        request_key: &str,
        work: &mut MediaPickerThumbnailPipelineWork,
    ) {
        if let Some(id) = state.inflight.remove(request_key) {
            if let Some(id) = id {
                work.cancels.push(id);
            }
            state.generation.insert(request_key.to_owned(), Uuid::new_v4());
            let quality = state.inflight_quality.remove(request_key);
            state.active_count = state.active_count.saturating_sub(1);
            MediaPickerPerformanceDiagnostics::shared().record_thumbnail_cancelled(quality);
        }
    }

    fn enqueue(
        state: &mut MediaPickerThumbnailPipelineState,
        pending: MediaPickerThumbnailPipelinePending,
        work: &mut MediaPickerThumbnailPipelineWork,
    ) {
        state.wait_queue.retain(|queued| queued.request_key != pending.request_key);
        if pending.quality == MediaPickerThumbnailPipelineQuality::Preview
            && state.wait_queue.len() >= Self::MAX_QUEUE_DEPTH
        {
            let excess = state.wait_queue.len() - Self::MAX_QUEUE_DEPTH + 1;
            state.wait_queue.drain(..excess);
        }
        if pending.quality == MediaPickerThumbnailPipelineQuality::Sharp {
            state.wait_queue.retain(|queued| {
                !(queued.asset.id == pending.asset.id && queued.quality == MediaPickerThumbnailPipelineQuality::Sharp)
            });
        }
        state.wait_queue.push(pending);
        Self::pump_locked(state, work);
        Self::report_pipeline_depth(state);
    }

    fn pump_locked(state: &mut MediaPickerThumbnailPipelineState, work: &mut MediaPickerThumbnailPipelineWork) {
        while state.active_count < Self::MAX_CONCURRENT {
            let Some(next) = Self::dequeue_next(state) else {
                break;
            };
            Self::start_locked(state, next, work);
        }
    }

    fn dequeue_next(state: &mut MediaPickerThumbnailPipelineState) -> Option<MediaPickerThumbnailPipelinePending> {
        if state.wait_queue.is_empty() {
            return None;
        }
        if let Some(preview_index) = state
            .wait_queue
            .iter()
            .position(|pending| pending.quality == MediaPickerThumbnailPipelineQuality::Preview)
        {
            return Some(state.wait_queue.remove(preview_index));
        }
        Some(state.wait_queue.remove(0))
    }

    fn start_locked(
        state: &mut MediaPickerThumbnailPipelineState,
        pending: MediaPickerThumbnailPipelinePending,
        work: &mut MediaPickerThumbnailPipelineWork,
    ) {
        state.active_count += 1;
        state.inflight_quality.insert(pending.request_key.clone(), pending.quality);
        MediaPickerPerformanceDiagnostics::shared().record_thumbnail_started(pending.quality);
        Self::report_pipeline_depth(state);
        state.inflight.insert(pending.request_key.clone(), None);
        work.starts.push(pending);
    }

    fn perform(self: &Arc<Self>, work: MediaPickerThumbnailPipelineWork) {
        for id in work.cancels {
            photo_library::cancel_image(id);
        }
        for pending in work.starts {
            self.start(pending);
        }
    }

    fn start(self: &Arc<Self>, pending: MediaPickerThumbnailPipelinePending) {
        let request_key = pending.request_key.clone();
        let generation = pending.generation;
        let still_reserved = self.sync_queue.with(|state| {
            state.generation.get(&request_key) == Some(&generation)
                && state.inflight.get(&request_key).is_some_and(Option::is_none)
        });
        if !still_reserved {
            return;
        }
        let delivery_mode = if pending.quality == MediaPickerThumbnailPipelineQuality::Sharp {
            ImageDeliveryMode::HighQuality
        } else {
            ImageDeliveryMode::Opportunistic
        };
        let asset_id = pending.asset.id.clone();
        let target_size = pending.target_size;
        let allow_network = pending.allow_network;
        let pipeline = Arc::downgrade(self);
        let id = PhotosImageRequest::request_image(
            &asset_id,
            target_size,
            ImageContentMode::AspectFill,
            delivery_mode,
            allow_network,
            move |delivery| {
                if let Some(pipeline) = pipeline.upgrade() {
                    pipeline.did_deliver(&pending, delivery);
                }
            },
        );
        let Some(id) = id else {
            return;
        };
        let kept = self.sync_queue.with(|state| {
            let current = state.generation.get(&request_key) == Some(&generation);
            match state.inflight.get_mut(&request_key) {
                Some(slot) if current && slot.is_none() => {
                    *slot = Some(id);
                    true
                }
                _ => false,
            }
        });
        if !kept {
            photo_library::cancel_image(id);
        }
    }

    fn did_deliver(self: &Arc<Self>, pending: &MediaPickerThumbnailPipelinePending, delivery: PhotoImageDelivery) {
        let mut work = MediaPickerThumbnailPipelineWork::default();
        self.sync_queue.with(|state| {
            let request_key = &pending.request_key;
            let still_valid = state.generation.get(request_key) == Some(&pending.generation);

            if delivery.is_cancelled {
                if still_valid && state.inflight.remove(request_key).is_some() {
                    state.inflight_quality.remove(request_key);
                    state.active_count = state.active_count.saturating_sub(1);
                    Self::pump_locked(state, &mut work);
                    Self::report_pipeline_depth(state);
                }
                return;
            }

            if still_valid && let Some(image) = delivery.image.as_ref() {
                let key = Self::cache_key(&pending.asset.id, pending.target_size, pending.quality);
                let cost = image.as_bytes(0).map_or(0, <[u8]>::len);
                self.memory_cache.set_object_cost(Arc::clone(image), key, cost);
            }

            if !still_valid {
                return;
            }
            MediaPickerPerformanceDiagnostics::shared().record_thumbnail_delivery(delivery.is_degraded, pending.quality);
            let handler = Arc::clone(&pending.handler);
            let image = delivery.image.clone();
            self.dispatch_main(move |cx| handler(image, cx));

            if delivery.is_degraded {
                return;
            }

            if state.inflight.remove(request_key).is_none() {
                return;
            }
            state.inflight_quality.remove(request_key);
            state.active_count = state.active_count.saturating_sub(1);
            Self::pump_locked(state, &mut work);
            Self::report_pipeline_depth(state);
        });
        self.perform(work);
    }

    fn report_pipeline_depth(state: &MediaPickerThumbnailPipelineState) {
        MediaPickerPerformanceDiagnostics::shared().update_pipeline_depth(state.active_count, state.wait_queue.len());
    }

    fn cache_key(asset_id: &str, size: Size<f32>, quality: MediaPickerThumbnailPipelineQuality) -> String {
        let w = size.width.round() as i64;
        let h = size.height.round() as i64;
        let q = if quality == MediaPickerThumbnailPipelineQuality::Sharp { "hq" } else { "lq" };
        format!("grid|{q}|{asset_id}|{w}x{h}")
    }
}

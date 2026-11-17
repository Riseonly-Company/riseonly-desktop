use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Weak};

use futures::channel::oneshot;
use gpui::{BackgroundExecutor, Hsla, Pixels, Size};
use parking_lot::Mutex;
use rise_core::foundation::ns_cache::NSCache;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_ui::message_bubble_geometry::MessageBubblePath;

use crate::modules::chat::components::chat::async_render::async_text_bubble_node::AsyncTextBubbleNode;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleCorners, AsyncTextBubbleFill, AsyncTextBubblePreparedLayout,
    AsyncTextBubbleVisualStyle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AsyncTextBubblePredisplayVersion {
    pub content: u64,
    pub appearance: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AsyncTextBubblePredisplayKey {
    pub identity: String,
    pub version: AsyncTextBubblePredisplayVersion,
    pub pixel_width: i64,
    pub pixel_height: i64,
    pub scale_bits: u64,
}

impl AsyncTextBubblePredisplayKey {
    pub fn make(
        prepared_layout: &AsyncTextBubblePreparedLayout,
        style: &AsyncTextBubbleVisualStyle,
        corners: &AsyncTextBubbleCorners,
        scale: f32,
    ) -> AsyncTextBubblePredisplayKey {
        let resolved_scale = AsyncTextBubblePredisplayRequest::resolved_scale(scale);
        AsyncTextBubblePredisplayKey {
            identity: prepared_layout.seed.id.clone(),
            version: AsyncTextBubblePredisplayFingerprint::version(prepared_layout, style, corners),
            pixel_width: pixel_extent(prepared_layout.size.width, resolved_scale),
            pixel_height: pixel_extent(prepared_layout.size.height, resolved_scale),
            scale_bits: f64::from(resolved_scale).to_bits(),
        }
    }

    pub fn scale(&self) -> f32 {
        f64::from_bits(self.scale_bits) as f32
    }
}

fn pixel_extent(points: Pixels, scale: f32) -> i64 {
    (f32::from(points) * scale).ceil() as i64
}

#[derive(Clone)]
pub struct AsyncTextBubblePredisplayRequest {
    pub key: AsyncTextBubblePredisplayKey,
    pub prepared_layout: Arc<AsyncTextBubblePreparedLayout>,
    pub style: AsyncTextBubbleVisualStyle,
    pub corners: AsyncTextBubbleCorners,
}

impl AsyncTextBubblePredisplayRequest {
    pub fn new(
        prepared_layout: Arc<AsyncTextBubblePreparedLayout>,
        style: AsyncTextBubbleVisualStyle,
        corners: AsyncTextBubbleCorners,
        scale: f32,
    ) -> Self {
        let key = AsyncTextBubblePredisplayKey::make(&prepared_layout, &style, &corners, scale);
        Self {
            key,
            prepared_layout,
            style,
            corners,
        }
    }

    pub fn resolved_scale(scale: f32) -> f32 {
        if !scale.is_finite() || scale <= 0.0 {
            return 1.0;
        }
        scale.clamp(1.0, 4.0)
    }
}

/// predisplayed bubble
/// gpui has no offscreen bitmap a cell could adopt so the background render resolves the
/// contour once and paint replays it
pub struct AsyncTextBubblePredisplayImage {
    pub key: AsyncTextBubblePredisplayKey,
    pub bubble_path: MessageBubblePath,
    pub size: Size<Pixels>,
    pub scale: f32,
    pub cost: usize,
}

impl AsyncTextBubblePredisplayImage {
    pub fn new(
        key: AsyncTextBubblePredisplayKey,
        bubble_path: MessageBubblePath,
        size: Size<Pixels>,
        scale: f32,
    ) -> Self {
        let cost = (key.pixel_width.max(0) as usize)
            .saturating_mul(key.pixel_height.max(0) as usize)
            .saturating_mul(4);
        Self {
            key,
            bubble_path,
            size,
            scale,
            cost,
        }
    }
}

struct LayoutIndexEntry {
    layout: Weak<AsyncTextBubblePreparedLayout>,
    key: AsyncTextBubblePredisplayKey,
}

impl LayoutIndexEntry {
    fn new(layout: &Arc<AsyncTextBubblePreparedLayout>, key: AsyncTextBubblePredisplayKey) -> Self {
        Self {
            layout: Arc::downgrade(layout),
            key,
        }
    }
}

#[derive(Default)]
struct WaiterCancellationState {
    cancelled: AtomicBool,
}

impl WaiterCancellationState {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

struct PendingWaiter {
    id: u64,
    request: AsyncTextBubblePredisplayRequest,
    cancellation_state: Arc<WaiterCancellationState>,
    continuation: oneshot::Sender<Option<Arc<AsyncTextBubblePredisplayImage>>>,
}

struct PendingRender {
    generation: u64,
    token: u64,
    operation: Arc<BlockOperation>,
    waiters: HashMap<u64, PendingWaiter>,
}

/// one queued render
struct BlockOperation {
    request: AsyncTextBubblePredisplayRequest,
    generation: u64,
    token: u64,
    cancelled: AtomicBool,
}

impl BlockOperation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

#[derive(Default)]
struct OperationQueue {
    running: usize,
    queued: VecDeque<Arc<BlockOperation>>,
}

#[derive(Default)]
struct AsyncTextBubblePredisplayCacheState {
    pending: HashMap<AsyncTextBubblePredisplayKey, PendingRender>,
    layout_index: HashMap<usize, LayoutIndexEntry>,
    render_generation: u64,
    render_queue: OperationQueue,
}

pub struct AsyncTextBubblePredisplayCache {
    pub total_cost_limit: usize,
    pub count_limit: usize,
    pub maximum_concurrent_renders: usize,
    pub maximum_entry_cost: usize,
    images: NSCache<AsyncTextBubblePredisplayKey, Arc<AsyncTextBubblePredisplayImage>>,
    state_lock: Mutex<AsyncTextBubblePredisplayCacheState>,
    next_identifier: AtomicU64,
    memory_warning_observer: Mutex<Option<RiseSubscription>>,
}

static SHARED: LazyLock<Arc<AsyncTextBubblePredisplayCache>> =
    LazyLock::new(AsyncTextBubblePredisplayCache::with_default_limits);

const MAXIMUM_PIXEL_EXTENT: i64 = 8_192;

impl AsyncTextBubblePredisplayCache {
    pub fn shared() -> &'static Arc<AsyncTextBubblePredisplayCache> {
        &SHARED
    }

    pub fn with_default_limits() -> Arc<Self> {
        Self::new(32 * 1_024 * 1_024, 384, 2, 16 * 1_024 * 1_024)
    }

    pub fn new(
        total_cost_limit: usize,
        count_limit: usize,
        maximum_concurrent_renders: usize,
        maximum_entry_cost: usize,
    ) -> Arc<Self> {
        let total_cost_limit = total_cost_limit.max(1);
        let count_limit = count_limit.max(1);
        let images = NSCache::new();
        images.set_name("riseonly.async-text-predisplay");
        images.set_total_cost_limit(total_cost_limit);
        images.set_count_limit(count_limit);

        let cache = Arc::new(Self {
            total_cost_limit,
            count_limit,
            maximum_concurrent_renders: maximum_concurrent_renders.max(1),
            maximum_entry_cost: maximum_entry_cost.min(total_cost_limit).max(1),
            images,
            state_lock: Mutex::new(AsyncTextBubblePredisplayCacheState::default()),
            next_identifier: AtomicU64::new(0),
            memory_warning_observer: Mutex::new(None),
        });

        let weak = Arc::downgrade(&cache);
        let observer = RiseAppEvents::default_center().observe(
            UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
            move |_| {
                if let Some(cache) = weak.upgrade() {
                    cache.remove_all_images();
                }
            },
        );
        *cache.memory_warning_observer.lock() = Some(observer);
        cache
    }

    pub fn image_for(
        &self,
        key: &AsyncTextBubblePredisplayKey,
    ) -> Option<Arc<AsyncTextBubblePredisplayImage>> {
        self.images.object(key)
    }

    pub fn can_predisplay(&self, prepared_layout: &AsyncTextBubblePreparedLayout, scale: f32) -> bool {
        let resolved_scale = AsyncTextBubblePredisplayRequest::resolved_scale(scale);
        let pixel_width = pixel_extent(prepared_layout.size.width, resolved_scale);
        let pixel_height = pixel_extent(prepared_layout.size.height, resolved_scale);
        rgba_cost(pixel_width, pixel_height).is_some_and(|bytes| bytes <= self.maximum_entry_cost)
    }

    pub fn image(
        &self,
        prepared_layout: &Arc<AsyncTextBubblePreparedLayout>,
        style: &AsyncTextBubbleVisualStyle,
        corners: &AsyncTextBubbleCorners,
        scale: f32,
    ) -> Option<Arc<AsyncTextBubblePredisplayImage>> {
        let identity = layout_identity(prepared_layout);
        let key = {
            let mut state = self.state_lock.lock();
            let matches = state.layout_index.get(&identity).is_some_and(|entry| {
                entry
                    .layout
                    .upgrade()
                    .is_some_and(|layout| Arc::ptr_eq(&layout, prepared_layout))
            });
            if !matches {
                state.layout_index.remove(&identity);
                return None;
            }
            state.layout_index.get(&identity).map(|entry| entry.key.clone())?
        };

        let resolved_scale = AsyncTextBubblePredisplayRequest::resolved_scale(scale);
        if key.pixel_width != pixel_extent(prepared_layout.size.width, resolved_scale)
            || key.pixel_height != pixel_extent(prepared_layout.size.height, resolved_scale)
            || key.scale_bits != f64::from(resolved_scale).to_bits()
            || key.version.appearance != AsyncTextBubblePredisplayFingerprint::appearance(style, corners)
        {
            return None;
        }
        let Some(cached) = self.image_for(&key) else {
            self.remove_layout_index(identity, &key);
            return None;
        };
        Some(cached)
    }

    /// resolves to none when the waiting future is dropped, like a cancelled task
    pub async fn image_for_request(
        self: &Arc<Self>,
        request: AsyncTextBubblePredisplayRequest,
        executor: &BackgroundExecutor,
    ) -> Option<Arc<AsyncTextBubblePredisplayImage>> {
        if let Some(cached) = self.cached_image_and_index(&request) {
            return Some(cached);
        }

        let waiter_id = self.next_identifier.fetch_add(1, Ordering::Relaxed);
        let cancellation_state = Arc::new(WaiterCancellationState::default());
        let key = request.key.clone();
        let (continuation, receiver) = oneshot::channel();
        let mut guard = WaiterCancellationGuard {
            cache: Arc::downgrade(self),
            key,
            waiter_id,
            cancellation_state: cancellation_state.clone(),
            armed: true,
        };
        self.enqueue(request, waiter_id, cancellation_state, continuation, executor);
        let image = receiver.await.ok().flatten();
        guard.armed = false;
        image
    }

    pub async fn prewarm(
        self: &Arc<Self>,
        requests: Vec<AsyncTextBubblePredisplayRequest>,
        executor: &BackgroundExecutor,
    ) {
        let mut iterator = uniqued_by_predisplay_key(requests).into_iter();
        let mut group = futures::stream::FuturesUnordered::new();
        for _ in 0..self.maximum_concurrent_renders {
            let Some(request) = iterator.next() else { break };
            group.push(self.image_for_request(request, executor));
        }
        while futures::StreamExt::next(&mut group).await.is_some() {
            if let Some(request) = iterator.next() {
                group.push(self.image_for_request(request, executor));
            }
        }
    }

    pub fn remove_all_images(&self) {
        self.invalidate_pending_and_images();
    }

    fn enqueue(
        self: &Arc<Self>,
        request: AsyncTextBubblePredisplayRequest,
        waiter_id: u64,
        cancellation_state: Arc<WaiterCancellationState>,
        continuation: oneshot::Sender<Option<Arc<AsyncTextBubblePredisplayImage>>>,
        executor: &BackgroundExecutor,
    ) {
        let key = request.key.clone();
        let started = {
            let mut state = self.state_lock.lock();
            if cancellation_state.is_cancelled() {
                drop(state);
                let _ = continuation.send(None);
                return;
            }
            if let Some(cached) = self.image_for(&key) {
                index_locked(&mut state, self.count_limit, &request.prepared_layout, key);
                drop(state);
                let _ = continuation.send(Some(cached));
                return;
            }
            let waiter = PendingWaiter {
                id: waiter_id,
                request: request.clone(),
                cancellation_state,
                continuation,
            };
            if let Some(existing) = state.pending.get_mut(&key) {
                existing.waiters.insert(waiter.id, waiter);
                return;
            }
            if state.pending.len() >= self.count_limit {
                drop(state);
                let _ = waiter.continuation.send(None);
                return;
            }

            let generation = state.render_generation;
            let token = self.next_identifier.fetch_add(1, Ordering::Relaxed);
            let operation = Arc::new(BlockOperation {
                request,
                generation,
                token,
                cancelled: AtomicBool::new(false),
            });
            state.pending.insert(
                key,
                PendingRender {
                    generation,
                    token,
                    operation: operation.clone(),
                    waiters: HashMap::from([(waiter.id, waiter)]),
                },
            );
            state.render_queue.queued.push_back(operation);
            self.dequeue_ready_operations_locked(&mut state)
        };
        self.start(started, executor);
    }

    fn dequeue_ready_operations_locked(
        &self,
        state: &mut AsyncTextBubblePredisplayCacheState,
    ) -> Vec<Arc<BlockOperation>> {
        let mut ready = Vec::new();
        while state.render_queue.running < self.maximum_concurrent_renders {
            let Some(operation) = state.render_queue.queued.pop_front() else { break };
            if operation.is_cancelled() {
                continue;
            }
            state.render_queue.running += 1;
            ready.push(operation);
        }
        ready
    }

    fn start(self: &Arc<Self>, operations: Vec<Arc<BlockOperation>>, executor: &BackgroundExecutor) {
        for operation in operations {
            let weak = Arc::downgrade(self);
            let next_executor = executor.clone();
            executor
                .spawn(async move {
                    let Some(cache) = weak.upgrade() else { return };
                    let image = if operation.is_cancelled() {
                        None
                    } else {
                        AsyncTextBubblePredisplayRenderer::render(
                            &operation.request,
                            cache.maximum_entry_cost,
                            &|| operation.is_cancelled(),
                        )
                    };
                    let next = {
                        let mut state = cache.state_lock.lock();
                        state.render_queue.running = state.render_queue.running.saturating_sub(1);
                        cache.dequeue_ready_operations_locked(&mut state)
                    };
                    cache.finish(
                        &operation.request.key,
                        operation.generation,
                        operation.token,
                        image.map(Arc::new),
                    );
                    cache.start(next, &next_executor);
                })
                .detach();
        }
    }

    fn finish(
        &self,
        key: &AsyncTextBubblePredisplayKey,
        generation: u64,
        token: u64,
        image: Option<Arc<AsyncTextBubblePredisplayImage>>,
    ) {
        let waiters: Vec<PendingWaiter> = {
            let mut state = self.state_lock.lock();
            let is_current = state.render_generation == generation
                && state
                    .pending
                    .get(key)
                    .is_some_and(|completed| completed.generation == generation && completed.token == token);
            if !is_current {
                return;
            }
            let Some(completed) = state.pending.remove(key) else { return };
            let waiters: Vec<PendingWaiter> = completed.waiters.into_values().collect();
            if let Some(image) = &image {
                let active: Vec<&PendingWaiter> = waiters
                    .iter()
                    .filter(|waiter| !waiter.cancellation_state.is_cancelled())
                    .collect();
                if !active.is_empty() {
                    self.images.set_object_cost(image.clone(), key.clone(), image.cost);
                    for waiter in active {
                        index_locked(
                            &mut state,
                            self.count_limit,
                            &waiter.request.prepared_layout,
                            key.clone(),
                        );
                    }
                }
            }
            waiters
        };

        for waiter in waiters {
            let result = if waiter.cancellation_state.is_cancelled() {
                None
            } else {
                image.clone()
            };
            let _ = waiter.continuation.send(result);
        }
    }

    fn cancel_waiter(&self, key: &AsyncTextBubblePredisplayKey, waiter_id: u64) {
        let mut operation_to_cancel = None;
        let removed_waiter = {
            let mut state = self.state_lock.lock();
            let mut removed = None;
            let mut drained = false;
            if let Some(existing) = state.pending.get_mut(key) {
                removed = existing.waiters.remove(&waiter_id);
                drained = removed.is_some() && existing.waiters.is_empty();
            }
            if drained {
                operation_to_cancel = state.pending.remove(key).map(|render| render.operation);
            }
            removed
        };
        if let Some(operation) = operation_to_cancel {
            operation.cancel();
        }
        if let Some(waiter) = removed_waiter {
            let _ = waiter.continuation.send(None);
        }
    }

    fn cached_image_and_index(
        &self,
        request: &AsyncTextBubblePredisplayRequest,
    ) -> Option<Arc<AsyncTextBubblePredisplayImage>> {
        let mut state = self.state_lock.lock();
        let cached = self.image_for(&request.key);
        if cached.is_some() {
            index_locked(&mut state, self.count_limit, &request.prepared_layout, request.key.clone());
        }
        cached
    }

    fn invalidate_pending_and_images(&self) {
        let renders: Vec<PendingRender> = {
            let mut state = self.state_lock.lock();
            state.render_generation = state.render_generation.wrapping_add(1);
            self.images.remove_all_objects();
            state.layout_index = HashMap::new();
            state.render_queue.queued.clear();
            std::mem::take(&mut state.pending).into_values().collect()
        };

        for render in &renders {
            render.operation.cancel();
        }
        for render in renders {
            for waiter in render.waiters.into_values() {
                let _ = waiter.continuation.send(None);
            }
        }
    }

    fn remove_layout_index(&self, identity: usize, key: &AsyncTextBubblePredisplayKey) {
        let mut state = self.state_lock.lock();
        if state.layout_index.get(&identity).is_some_and(|entry| &entry.key == key) {
            state.layout_index.remove(&identity);
        }
    }
}

impl Drop for AsyncTextBubblePredisplayCache {
    fn drop(&mut self) {
        self.memory_warning_observer.lock().take();
        self.invalidate_pending_and_images();
    }
}

struct WaiterCancellationGuard {
    cache: Weak<AsyncTextBubblePredisplayCache>,
    key: AsyncTextBubblePredisplayKey,
    waiter_id: u64,
    cancellation_state: Arc<WaiterCancellationState>,
    armed: bool,
}

impl Drop for WaiterCancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.cancellation_state.cancel();
        if let Some(cache) = self.cache.upgrade() {
            cache.cancel_waiter(&self.key, self.waiter_id);
        }
    }
}

fn layout_identity(layout: &Arc<AsyncTextBubblePreparedLayout>) -> usize {
    Arc::as_ptr(layout) as usize
}

fn index_locked(
    state: &mut AsyncTextBubblePredisplayCacheState,
    count_limit: usize,
    layout: &Arc<AsyncTextBubblePreparedLayout>,
    key: AsyncTextBubblePredisplayKey,
) {
    state
        .layout_index
        .insert(layout_identity(layout), LayoutIndexEntry::new(layout, key));
    let limit = count_limit.saturating_mul(2);
    while state.layout_index.len() > limit {
        let Some(oldest) = state.layout_index.keys().next().copied() else { break };
        state.layout_index.remove(&oldest);
    }
}

fn rgba_cost(pixel_width: i64, pixel_height: i64) -> Option<usize> {
    if pixel_width <= 0
        || pixel_height <= 0
        || pixel_width > MAXIMUM_PIXEL_EXTENT
        || pixel_height > MAXIMUM_PIXEL_EXTENT
    {
        return None;
    }
    (pixel_width as usize)
        .checked_mul(pixel_height as usize)?
        .checked_mul(4)
}

enum AsyncTextBubblePredisplayRenderer {}

impl AsyncTextBubblePredisplayRenderer {
    fn render(
        request: &AsyncTextBubblePredisplayRequest,
        maximum_entry_cost: usize,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<AsyncTextBubblePredisplayImage> {
        let key = &request.key;
        if is_cancelled() {
            return None;
        }
        let rgba = rgba_cost(key.pixel_width, key.pixel_height)?;
        if rgba > maximum_entry_cost {
            return None;
        }
        let size = request.prepared_layout.size;
        let bubble_path = AsyncTextBubbleNode::bubble_path(size, &request.corners);
        if is_cancelled() {
            return None;
        }
        Some(AsyncTextBubblePredisplayImage::new(
            key.clone(),
            bubble_path,
            size,
            key.scale(),
        ))
    }
}

struct Accumulator {
    first: u64,
    second: u64,
}

impl Default for Accumulator {
    fn default() -> Self {
        Self {
            first: 0xcbf2_9ce4_8422_2325,
            second: 0x9e37_79b9_7f4a_7c15,
        }
    }
}

impl Accumulator {
    fn append(&mut self, value: u64) {
        self.first ^= value;
        self.first = self.first.wrapping_mul(0x0000_0100_0000_01b3);
        self.second ^= value.wrapping_add(0x517c_c1b7_2722_0a95);
        self.second = self.second.rotate_left(27);
        self.second = self.second.wrapping_mul(0x94d0_49bb_1331_11eb);
    }

    fn append_pixels(&mut self, value: Pixels) {
        self.append_float(f32::from(value));
    }

    fn append_float(&mut self, value: f32) {
        self.append(f64::from(value).to_bits());
    }

    fn append_string(&mut self, string: &str) {
        self.append(string.len() as u64);
        for byte in string.bytes() {
            self.append(u64::from(byte));
        }
    }

    fn append_color(&mut self, color: Hsla) {
        self.append(4);
        self.append_float(color.h);
        self.append_float(color.s);
        self.append_float(color.l);
        self.append_float(color.a);
    }

    fn value(&self) -> u64 {
        self.first ^ self.second
    }
}

enum AsyncTextBubblePredisplayFingerprint {}

impl AsyncTextBubblePredisplayFingerprint {
    fn version(
        prepared_layout: &AsyncTextBubblePreparedLayout,
        style: &AsyncTextBubbleVisualStyle,
        corners: &AsyncTextBubbleCorners,
    ) -> AsyncTextBubblePredisplayVersion {
        let mut content = Accumulator::default();
        let seed = &prepared_layout.seed;
        let spec = &prepared_layout.spec;
        content.append_string(&seed.text);
        content.append(seed.timestamp.time_interval_since_reference_date().to_bits());
        content.append_string(&seed.time_zone_identifier);
        content.append_string(seed.delivery_state.raw_value());
        content.append(u64::from(seed.view_count_text.is_some()));
        content.append_string(seed.view_count_text.as_deref().unwrap_or(""));
        content.append(seed.highlight_ranges.len() as u64);
        for range in &seed.highlight_ranges {
            content.append(range.location as u64);
            content.append(range.length as u64);
        }
        content.append_pixels(prepared_layout.size.width);
        content.append_pixels(prepared_layout.size.height);
        content.append_pixels(spec.maximum_width);
        content.append_pixels(spec.minimum_width);
        content.append_pixels(spec.minimum_height);
        content.append_pixels(spec.horizontal_padding);
        content.append_pixels(spec.vertical_padding);
        content.append_pixels(spec.top_padding);
        content.append_pixels(spec.bottom_padding);
        content.append(spec.tail_side.raw_value() as u64);
        content.append_pixels(spec.tail_inset);
        content.append_pixels(spec.text_font_size);
        content.append_pixels(spec.time_font_size);
        content.append_pixels(spec.line_spacing);
        content.append_pixels(spec.inline_meta_spacing);
        content.append_pixels(spec.meta_top_spacing);
        content.append_pixels(spec.status_icon_size);
        content.append_pixels(spec.status_icon_spacing);

        AsyncTextBubblePredisplayVersion {
            content: content.value(),
            appearance: Self::appearance(style, corners),
        }
    }

    fn appearance(style: &AsyncTextBubbleVisualStyle, corners: &AsyncTextBubbleCorners) -> u64 {
        let mut appearance = Accumulator::default();
        match &style.fill {
            AsyncTextBubbleFill::Solid(color) => {
                appearance.append(0);
                appearance.append_color(*color);
            }
            AsyncTextBubbleFill::LinearGradient {
                start,
                end,
                start_point,
                end_point,
            } => {
                appearance.append(1);
                appearance.append_color(*start);
                appearance.append_color(*end);
                appearance.append_pixels(start_point.x);
                appearance.append_pixels(start_point.y);
                appearance.append_pixels(end_point.x);
                appearance.append_pixels(end_point.y);
            }
        }
        appearance.append_color(style.text_color);
        appearance.append_color(style.time_color);
        appearance.append_color(style.status_color);
        appearance.append_color(style.read_status_color);
        appearance.append_color(style.failure_color);
        appearance.append_color(style.failure_glyph_color);
        appearance.append_pixels(corners.top_left);
        appearance.append_pixels(corners.top_right);
        appearance.append_pixels(corners.bottom_left);
        appearance.append_pixels(corners.bottom_right);
        appearance.append(corners.tail_side.raw_value() as u64);
        appearance.append(u64::from(corners.draws_tail));

        appearance.value()
    }
}

fn uniqued_by_predisplay_key(
    requests: Vec<AsyncTextBubblePredisplayRequest>,
) -> Vec<AsyncTextBubblePredisplayRequest> {
    let mut keys = HashSet::new();
    let mut result = Vec::with_capacity(requests.len());
    for request in requests {
        if keys.insert(request.key.clone()) {
            result.push(request);
        }
    }
    result
}

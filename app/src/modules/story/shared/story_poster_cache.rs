use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, BoxFuture, Shared};
use futures::FutureExt;
use gpui::{AppContext, AsyncApp, RenderImage};
use image::{Frame, RgbaImage};
use parking_lot::Mutex;
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use smallvec::SmallVec;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions};
use crate::core::ui::clever_image_disk_cache::CleverImage;

static CACHE: LazyLock<NSCache<i64, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(240);
    cache.set_total_cost_limit(48 * 1024 * 1024);
    cache
});

/// one shared generation per story, owned by the cache and not by the tile that started it
#[derive(Clone)]
struct Generation {
    token: u64,
    task: Shared<BoxFuture<'static, Option<CleverImage>>>,
    abort: AbortHandle,
}

struct SlotWaiter {
    token: u64,
    continuation: oneshot::Sender<()>,
}

#[derive(Default)]
struct PosterState {
    generations: HashMap<i64, Generation>,
    next_token: u64,
    generations_running: usize,
    slot_waiters: Vec<SlotWaiter>,
}

impl PosterState {
    fn take_token(&mut self) -> u64 {
        self.next_token = self.next_token.wrapping_add(1);
        self.next_token
    }

    /// hands the slot straight to the next waiter so the running count stays at the limit
    fn release_generation_slot(&mut self) {
        while !self.slot_waiters.is_empty() {
            let waiter = self.slot_waiters.remove(0);
            if waiter.continuation.send(()).is_ok() {
                return;
            }
        }
        self.generations_running = self.generations_running.saturating_sub(1);
    }
}

static STATE: LazyLock<Mutex<PosterState>> = LazyLock::new(|| Mutex::new(PosterState::default()));

struct SlotWaitGuard {
    token: u64,
    settled: bool,
}

impl Drop for SlotWaitGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut state = STATE.lock();
        if let Some(index) = state.slot_waiters.iter().position(|waiter| waiter.token == self.token) {
            state.slot_waiters.remove(index);
        } else {
            state.release_generation_slot();
        }
    }
}

struct GenerationGuard {
    story_id: i64,
    token: u64,
}

impl Drop for GenerationGuard {
    fn drop(&mut self) {
        let mut state = STATE.lock();
        if state.generations.get(&self.story_id).is_some_and(|running| running.token == self.token) {
            state.generations.remove(&self.story_id);
        }
        state.release_generation_slot();
    }
}

/// first frame posters for video stories in a grid
/// a story has only the mp4 so no poster, frame is made once off main
pub enum StoryPosterCache {}

impl StoryPosterCache {
    const POSTER_TIME_SECONDS: f64 = 0.05;

    pub fn cached(story_id: i64) -> Option<CleverImage> {
        CACHE.object(&story_id)
    }

    const GENERATION_LIMIT: usize = 3;

    async fn acquire_generation_slot() -> bool {
        let (token, turn) = {
            let mut state = STATE.lock();
            if state.generations_running < Self::GENERATION_LIMIT {
                state.generations_running += 1;
                return true;
            }
            let token = state.take_token();
            let (continuation, turn) = oneshot::channel();
            state.slot_waiters.push(SlotWaiter { token, continuation });
            (token, turn)
        };
        let mut guard = SlotWaitGuard { token, settled: false };
        let granted = turn.await.is_ok();
        guard.settled = true;
        granted
    }

    /// none for a non video, unusable source
    /// unreadable frame or a dropped caller
    /// a second call for the same story awaits the running generation since cover and tile show together
    pub async fn poster(
        story_id: i64,
        source: &str,
        maximum_pixel_dimension: i64,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        if let Some(hit) = Self::cached(story_id) {
            return Some(hit);
        }
        if story_id <= 0 {
            return None;
        }
        let running = STATE.lock().generations.get(&story_id).cloned();
        if let Some(running) = running {
            return running.task.await;
        }

        if !Self::acquire_generation_slot().await {
            return None;
        }
        if let Some(hit) = Self::cached(story_id) {
            STATE.lock().release_generation_slot();
            return Some(hit);
        }

        let side = u32::try_from(maximum_pixel_dimension.max(1)).unwrap_or(u32::MAX);
        let expected_user_id = SaiAccountCacheScope::active_user_id();
        let expected_epoch = SaiAccountCacheScope::active_epoch();
        let source = source.to_owned();
        let task = {
            let mut state = STATE.lock();
            if let Some(running) = state.generations.get(&story_id).cloned() {
                state.release_generation_slot();
                drop(state);
                return running.task.await;
            }
            let token = state.take_token();
            let (abort, registration) = AbortHandle::new_pair();
            let spawned = cx.spawn(async move |cx| {
                let _guard = GenerationGuard { story_id, token };
                Self::generate(story_id, &source, side, expected_user_id, expected_epoch, cx).await
            });
            let task: BoxFuture<'static, Option<CleverImage>> =
                Abortable::new(spawned, registration).map(|result| result.ok().flatten()).boxed();
            let task = task.shared();
            state.generations.insert(story_id, Generation { token, task: task.clone(), abort });
            task
        };
        task.await
    }

    async fn generate(
        story_id: i64,
        source: &str,
        side: u32,
        expected_user_id: Option<String>,
        expected_epoch: Option<String>,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let Some(local_url) = Self::local_video_url(source, expected_user_id.clone(), expected_epoch.clone(), cx).await else {
            tracing::warn!(target: "riseonly::X1434", story_id, source, "poster: no local file");
            return None;
        };

        let image = cx
            .background_spawn(async move {
                let frame = match rise_media::video_export::poster(&local_url, Self::POSTER_TIME_SECONDS, side) {
                    Ok(frame) => frame,
                    Err(error) => {
                        tracing::warn!(target: "riseonly::X1434", story_id, side, error = %error, path = %local_url.display(), "poster: frame failed");
                        return None;
                    }
                };
                let (width, height) = (frame.width, frame.height);
                let buffer = RgbaImage::from_raw(width, height, frame.bgra)?;
                let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
                let cost = usize::try_from(width).ok()? * 4 * usize::try_from(height).ok()?;
                Some((rise_ui::image_ui::render_image_atlas::RenderImageAtlas::tracked(Arc::new(RenderImage::new(frames))), cost))
            })
            .await;

        let Some((image, cost)) = image else {
            tracing::warn!(target: "riseonly::X1434", story_id, "poster: no image");
            return None;
        };
        if SaiAccountCacheScope::active_user_id() != expected_user_id
            || SaiAccountCacheScope::active_epoch() != expected_epoch
        {
            return None;
        }

        CACHE.set_object_cost(image.clone(), story_id, cost.max(1));
        Some(image)
    }

    pub fn reset_account_state() {
        CACHE.remove_all_objects();
        let generations = std::mem::take(&mut STATE.lock().generations);
        for generation in generations.into_values() {
            generation.abort.abort();
        }
    }

    async fn local_video_url(
        source: &str,
        expected_user_id: Option<String>,
        expected_epoch: Option<String>,
        cx: &AsyncApp,
    ) -> Option<PathBuf> {
        let trimmed = trimming_whitespaces_and_newlines(source);
        if trimmed.is_empty() {
            return None;
        }
        if is_local_absolute_path(trimmed) {
            return Some(Path::new(trimmed).to_path_buf());
        }
        let url = Url::from_string(trimmed)?;
        if url.is_file_url() {
            return url.to_file_path();
        }
        RemoteMediaDiskCache::shared()
            .ensure_cached(
                &url,
                RemoteMediaDiskCacheMaterializeOptions {
                    cache_key: Some(trimmed.to_owned()),
                    expected_account_user_id: expected_user_id,
                    expected_account_epoch: expected_epoch,
                    ..RemoteMediaDiskCacheMaterializeOptions::default()
                },
                cx,
            )
            .await
    }
}

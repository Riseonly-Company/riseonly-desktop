use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use futures::future::{FutureExt, Shared};
use gpui::{AsyncApp, Task};
use parking_lot::Mutex;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapLoader, CleverImageDiskCache, CleverImageRenderMemoryCache, CleverImageWorkLane,
};
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::post_feed_media_prefetch::PostFeedMediaPrefetchPolicy;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowBodyKind, PostRowMediaAspects, PostRowSeed,
};
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchPolicy;

/// what the aspect chain settled for one image url
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PostRowAspectOutcome {
    Aspect(f32),
    Failed,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PostRowAspectMemoKey {
    url: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

/// aspects learned from probes and flight bitmaps, per account
pub struct PostRowAspectMemo {
    entries: Mutex<HashMap<PostRowAspectMemoKey, PostRowAspectOutcome>>,
    entry_limit: usize,
}

static MEMO_SHARED: LazyLock<PostRowAspectMemo> = LazyLock::new(|| PostRowAspectMemo::new(2_048));

impl PostRowAspectMemo {
    pub fn shared() -> &'static PostRowAspectMemo {
        &MEMO_SHARED
    }

    pub fn new(entry_limit: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            entry_limit: entry_limit.max(1),
        }
    }

    pub fn outcome(&self, url: &str, account_user_id: Option<&str>, account_epoch: Option<&str>) -> Option<PostRowAspectOutcome> {
        let key = Self::key(url, account_user_id, account_epoch);
        self.entries.lock().get(&key).copied()
    }

    /// a measured aspect never gets replaced by a failure
    pub fn store(&self, outcome: PostRowAspectOutcome, url: &str, account_user_id: Option<&str>, account_epoch: Option<&str>) {
        if let PostRowAspectOutcome::Aspect(value) = outcome
            && !(value.is_finite() && value > 0.0)
        {
            return;
        }
        let key = Self::key(url, account_user_id, account_epoch);
        let mut entries = self.entries.lock();
        let existing = entries.get(&key).copied();
        if outcome == PostRowAspectOutcome::Failed && matches!(existing, Some(PostRowAspectOutcome::Aspect(_))) {
            return;
        }
        if existing.is_none() && entries.len() >= self.entry_limit {
            entries.clear();
        }
        entries.insert(key, outcome);
    }

    pub fn remove_all(&self) {
        self.entries.lock().clear();
    }

    fn key(url: &str, account_user_id: Option<&str>, account_epoch: Option<&str>) -> PostRowAspectMemoKey {
        PostRowAspectMemoKey {
            url: CleverImageDiskCache::normalized_cache_key(url),
            account_user_id: account_user_id.map(str::to_owned),
            account_epoch: account_epoch.map(str::to_owned),
        }
    }
}

/// slide bucket load shared with the row and the list prefetch, only its aspect is kept
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostRowAspectProbeRequest {
    pub url: String,
    pub pixel_dimension: Option<i64>,
    pub is_own: bool,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

/// aspects of one seed and whether they can still change
#[derive(Clone)]
pub struct PostRowAspectResolution {
    pub aspects: PostRowMediaAspects,
    pub is_settled: bool,
    pub probe: Option<PostRowAspectProbeRequest>,
}

/// natural aspect per slide before insert
/// chain is server size, any bitmap of the url in memory
/// the memo, then a probe that never blocks prepare
pub enum PostRowMediaAspectResolver {}

impl PostRowMediaAspectResolver {
    /// sync and memory only, safe off main
    pub fn resolve(seed: &PostRowSeed, account_user_id: Option<&str>, account_epoch: Option<&str>) -> PostRowAspectResolution {
        let slides = &seed.media.slides;
        if slides.is_empty() {
            return PostRowAspectResolution {
                aspects: PostRowMediaAspects::empty(),
                is_settled: true,
                probe: None,
            };
        }
        match seed.body {
            PostRowBodyKind::Standard => Self::resolve_gallery(seed, slides, account_user_id, account_epoch),
            PostRowBodyKind::Reel => Self::resolve_riff_card(seed, slides, account_user_id, account_epoch),
        }
    }

    /// still of page 0, the url an anchor bitmap aspect is remembered under
    pub fn cover_source(seed: &PostRowSeed) -> Option<String> {
        seed.media.slides.first().and_then(Self::poster_source)
    }

    /// aspect of any decoded bitmap of the url in memory,
    /// tries requested bucket then full size then any
    pub fn memory_aspect(
        url: &str,
        _is_own: bool,
        bucket: Option<i64>,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Option<f32> {
        if let Some(image) = CleverImageBitmapLoader::memory_image(
            url,
            PostMediaCachePolicy::FEED_IMAGES_PERSIST_TO_DISK,
            account_user_id,
            account_epoch,
            bucket,
            PostMediaCachePolicy::FEED_IMAGE_POLICY,
            None,
            false,
        ) && let Some(aspect) = Self::aspect(&image)
        {
            return Some(aspect);
        }
        if let Some(image) = MediaViewerImageMemoryCache::memory_only(&RemoteMediaURL::display_source(url, None), account_user_id)
            && let Some(aspect) = Self::aspect(&image)
        {
            return Some(aspect);
        }
        if let Some(image) = CleverImageRenderMemoryCache::latest_image(
            &CleverImageDiskCache::normalized_cache_key(url),
            account_user_id,
            account_epoch,
        ) && let Some(aspect) = Self::aspect(&image)
        {
            return Some(aspect);
        }
        None
    }

    pub fn aspect(image: &CleverImage) -> Option<f32> {
        let size = image.size(0);
        if size.width.0 <= 0 || size.height.0 <= 0 {
            return None;
        }
        let value = size.width.0 as f32 / size.height.0 as f32;
        (value.is_finite() && value > 0.0).then_some(value)
    }

    fn resolve_gallery(
        seed: &PostRowSeed,
        slides: &[PostMediaSlide],
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> PostRowAspectResolution {
        let mut natural: Vec<Option<f32>> = Vec::with_capacity(slides.len());
        let mut probe = None;
        for (index, slide) in slides.iter().enumerate() {
            if let PostMediaSlide::Image { size, .. } = slide
                && let Some(server) = PostMediaBox::aspect(size.as_ref())
            {
                natural.push(Some(server));
                continue;
            }
            let Some(source) = Self::poster_source(slide) else {
                natural.push(None);
                continue;
            };
            if let Some(aspect) = Self::memory_aspect(
                &source,
                seed.is_own,
                PostFeedMediaPrefetchPolicy::slide_pixel_dimension(slide.is_video()),
                account_user_id,
                account_epoch,
            ) {
                natural.push(Some(aspect));
                continue;
            }
            match PostRowAspectMemo::shared().outcome(&source, account_user_id, account_epoch) {
                Some(PostRowAspectOutcome::Aspect(aspect)) => natural.push(Some(aspect)),
                Some(PostRowAspectOutcome::Failed) => natural.push(None),
                None => {
                    natural.push(None);
                    if index == 0 {
                        probe = Some(Self::probe_request(
                            source,
                            PostFeedMediaPrefetchPolicy::slide_pixel_dimension(slide.is_video()),
                            seed,
                            account_user_id,
                            account_epoch,
                        ));
                    }
                }
            }
        }
        PostRowAspectResolution {
            aspects: PostRowMediaAspects {
                natural,
                is_provisional: probe.is_some(),
            },
            is_settled: probe.is_none(),
            probe,
        }
    }

    fn resolve_riff_card(
        seed: &PostRowSeed,
        slides: &[PostMediaSlide],
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> PostRowAspectResolution {
        let slide = &slides[0];
        let server = PostMediaBox::aspect(slide.size());
        let resolution = |first: Option<f32>, probe: Option<PostRowAspectProbeRequest>| {
            let mut natural = vec![None; slides.len()];
            natural[0] = first;
            PostRowAspectResolution {
                aspects: PostRowMediaAspects {
                    natural,
                    is_provisional: first.is_none() && probe.is_some(),
                },
                is_settled: probe.is_none(),
                probe,
            }
        };

        let Some(source) = Self::poster_source(slide) else {
            return resolution(server, None);
        };
        if let Some(poster) = Self::memory_aspect(
            &source,
            seed.is_own,
            Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION),
            account_user_id,
            account_epoch,
        ) {
            return resolution(Some(poster), None);
        }
        match PostRowAspectMemo::shared().outcome(&source, account_user_id, account_epoch) {
            Some(PostRowAspectOutcome::Aspect(aspect)) => resolution(Some(aspect), None),
            Some(PostRowAspectOutcome::Failed) => resolution(server, None),
            None => resolution(
                server,
                Some(Self::probe_request(
                    source,
                    Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION),
                    seed,
                    account_user_id,
                    account_epoch,
                )),
            ),
        }
    }

    fn probe_request(
        url: String,
        pixel_dimension: Option<i64>,
        seed: &PostRowSeed,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> PostRowAspectProbeRequest {
        PostRowAspectProbeRequest {
            url,
            pixel_dimension,
            is_own: seed.is_own,
            account_user_id: account_user_id.map(str::to_owned),
            account_epoch: account_epoch.map(str::to_owned),
        }
    }

    fn poster_source(slide: &PostMediaSlide) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(slide.poster_url()?);
        (!value.is_empty()).then(|| value.to_owned())
    }
}

/// owns in flight aspect probes, the bitmap work runs in the image loader bounded lanes
pub struct PostRowAspectProbe {
    in_flight: RefCell<HashMap<PostRowAspectProbeRequest, Shared<Task<PostRowAspectOutcome>>>>,
}

thread_local! {
    static PROBE_SHARED: Rc<PostRowAspectProbe> = Rc::new(PostRowAspectProbe {
        in_flight: RefCell::new(HashMap::new()),
    });
}

struct PostRowAspectProbeInFlightGuard<'a> {
    probe: &'a PostRowAspectProbe,
    request: &'a PostRowAspectProbeRequest,
}

impl Drop for PostRowAspectProbeInFlightGuard<'_> {
    fn drop(&mut self) {
        self.probe.in_flight.borrow_mut().remove(self.request);
    }
}

impl PostRowAspectProbe {
    pub fn shared() -> Rc<PostRowAspectProbe> {
        PROBE_SHARED.with(Rc::clone)
    }

    /// loads at the slide bucket so it joins the row and prefetch load instead of decoding twice
    pub async fn resolve(&self, request: PostRowAspectProbeRequest, cx: &AsyncApp) -> PostRowAspectOutcome {
        let account_user_id = request.account_user_id.as_deref();
        let account_epoch = request.account_epoch.as_deref();
        if let Some(known) = PostRowAspectMemo::shared().outcome(&request.url, account_user_id, account_epoch) {
            return known;
        }
        let running = self.in_flight.borrow().get(&request).cloned();
        if let Some(running) = running {
            return running.await;
        }
        let load = request.clone();
        let task = cx
            .spawn(async move |cx: &mut AsyncApp| {
                let image = CleverImageBitmapLoader::load(
                    &load.url,
                    PostMediaCachePolicy::FEED_IMAGES_PERSIST_TO_DISK,
                    true,
                    load.account_user_id.clone(),
                    load.account_epoch.clone(),
                    load.pixel_dimension,
                    PostMediaCachePolicy::FEED_IMAGE_POLICY,
                    CleverImageWorkLane::Speculative,
                    cx,
                )
                .await;
                match image.as_ref().and_then(PostRowMediaAspectResolver::aspect) {
                    Some(aspect) => PostRowAspectOutcome::Aspect(aspect),
                    None => PostRowAspectOutcome::Failed,
                }
            })
            .shared();
        self.in_flight.borrow_mut().insert(request.clone(), task.clone());
        let in_flight = PostRowAspectProbeInFlightGuard {
            probe: self,
            request: &request,
        };
        let outcome = task.await;
        drop(in_flight);
        if SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            PostRowAspectMemo::shared().store(outcome, &request.url, account_user_id, account_epoch);
        }
        outcome
    }
}

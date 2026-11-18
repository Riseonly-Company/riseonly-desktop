use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use gpui::{App, AsyncApp, AtlasKey, EntityId, ImageId, RenderImage, RenderImageParams, Window};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

/// gives the gpui sprite atlas back the textures of images nobody holds anymore
/// gpui frees a tile only on explicit drop_image so cost evicting caches would leak without this
pub enum RenderImageAtlas {}

struct TrackedImage {
    image: Weak<RenderImage>,
    id: ImageId,
    frame_count: usize,
}

static TRACKED: LazyLock<Mutex<Vec<TrackedImage>>> = LazyLock::new(|| Mutex::new(Vec::new()));

thread_local! {
    static PENDING_UPLOADS: RefCell<HashMap<ImageId, Vec<EntityId>>> = RefCell::new(HashMap::new());
    static REFUSED_UPLOADS: RefCell<HashSet<ImageId>> = RefCell::new(HashSet::new());
}

impl RenderImageAtlas {
    const SWEEP_INTERVAL: Duration = Duration::from_secs(2);
    const PAINT_UPLOAD_BYTES: usize = 512 * 1024;

    /// true when it paints now with no big gpu copy on the main thread, else a worker uploads it
    pub fn is_ready(image: &Arc<RenderImage>, window: &mut Window, cx: &mut App) -> bool {
        if image.frame_count() != 1
            || image.as_bytes(0).is_none_or(|bytes| bytes.len() <= Self::PAINT_UPLOAD_BYTES)
            || REFUSED_UPLOADS.with_borrow(|refused| refused.contains(&image.id))
        {
            return true;
        }
        let Some(uploader) = window.image_uploader() else {
            return true;
        };
        let key: AtlasKey = RenderImageParams { image_id: image.id, frame_index: 0 }.into();
        if uploader.is_resident(&key) {
            return true;
        }
        let view = window.current_view();
        let id = image.id;
        let starts = PENDING_UPLOADS.with_borrow_mut(|pending| match pending.get_mut(&id) {
            Some(views) => {
                if !views.contains(&view) {
                    views.push(view);
                }
                false
            }
            None => {
                pending.insert(id, vec![view]);
                true
            }
        });
        if starts {
            let image = image.clone();
            let upload = cx.background_executor().spawn(async move {
                if let Some(bytes) = image.as_bytes(0) {
                    uploader.upload(&key, image.size(0), bytes);
                }
                uploader.is_resident(&key)
            });
            cx.spawn(async move |cx: &mut AsyncApp| {
                if !upload.await {
                    REFUSED_UPLOADS.with_borrow_mut(|refused| refused.insert(id));
                }
                let views = PENDING_UPLOADS.with_borrow_mut(|pending| pending.remove(&id)).unwrap_or_default();
                cx.update(|cx| {
                    for view in views {
                        cx.notify(view);
                    }
                });
            })
            .detach();
        }
        false
    }

    /// registers the image for reclamation and hands it back, safe from any thread
    pub fn tracked(image: Arc<RenderImage>) -> Arc<RenderImage> {
        Self::track(&image);
        image
    }

    /// registers the image for reclamation, safe from any thread
    pub fn track(image: &Arc<RenderImage>) {
        let entry = TrackedImage {
            image: Arc::downgrade(image),
            id: image.id,
            frame_count: image.frame_count(),
        };
        if let Ok(mut tracked) = TRACKED.lock() {
            tracked.push(entry);
        }
    }

    pub fn install(cx: &mut App) {
        cx.spawn(async move |cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Self::SWEEP_INTERVAL).await;
                cx.update(Self::sweep);
            }
        })
        .detach();
    }

    /// removes atlas tiles of every tracked image whose last Arc is gone
    pub fn sweep(cx: &mut App) {
        let released: Vec<(ImageId, usize)> = {
            let Ok(mut tracked) = TRACKED.lock() else {
                return;
            };
            let mut released = Vec::new();
            tracked.retain(|entry| {
                if entry.image.strong_count() > 0 {
                    return true;
                }
                released.push((entry.id, entry.frame_count));
                false
            });
            released
        };
        for (id, frame_count) in released {
            if let Some(ghost) = Self::ghost(id, frame_count) {
                cx.drop_image(ghost, None);
            }
        }
    }

    pub fn tracked_count() -> usize {
        TRACKED.lock().map(|tracked| tracked.len()).unwrap_or(0)
    }

    fn ghost(id: ImageId, frame_count: usize) -> Option<Arc<RenderImage>> {
        if frame_count == 0 {
            return None;
        }
        let frames: SmallVec<[Frame; 1]> = (0..frame_count)
            .filter_map(|_| RgbaImage::from_raw(1, 1, vec![0; 4]).map(Frame::new))
            .collect();
        let mut ghost = RenderImage::new(frames);
        ghost.id = id;
        Some(Arc::new(ghost))
    }
}

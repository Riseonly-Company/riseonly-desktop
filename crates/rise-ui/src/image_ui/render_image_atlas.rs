use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use gpui::{App, AsyncApp, ImageId, RenderImage};
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

impl RenderImageAtlas {
    const SWEEP_INTERVAL: Duration = Duration::from_secs(2);

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

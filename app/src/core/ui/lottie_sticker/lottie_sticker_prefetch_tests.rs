use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;
use gpui::{DevicePixels, TestAppContext, px, size};
use image::{Rgba, RgbaImage};
use parking_lot::Mutex;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use super::*;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder, CleverImageRenderMemoryCache};
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{LottieStickerContainer, LottieStickerLoaderError};
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieImageVisibility;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatComposerCustomEmojiPlaceholder;

const TGS_BASE64: &str = "H4sIAAAAAAAAA6tWKlOyUjLVM9czUdJRSitSsjI20FHKLFCyAlL5QMpQR6kcTGaAyZzEytSiYiWr6NhaADZypI07AAAA";

#[derive(Clone, Default)]
struct LottieStickerPrefetchProbeSnapshot {
    active: i64,
    maximum_active: i64,
    start_order: Vec<String>,
    completed: HashMap<String, i64>,
    cancelled: HashMap<String, i64>,
}

struct LottieStickerPrefetchProbe {
    state: Mutex<LottieStickerPrefetchProbeSnapshot>,
    release: Mutex<Option<oneshot::Sender<()>>>,
    gate: Shared<oneshot::Receiver<()>>,
}

impl LottieStickerPrefetchProbe {
    fn new() -> Arc<Self> {
        let (release, gate) = oneshot::channel();
        Arc::new(Self {
            state: Mutex::new(LottieStickerPrefetchProbeSnapshot::default()),
            release: Mutex::new(Some(release)),
            gate: gate.shared(),
        })
    }

    fn begin(&self, url: &str) {
        let mut state = self.state.lock();
        state.active += 1;
        state.maximum_active = state.maximum_active.max(state.active);
        state.start_order.push(url.to_owned());
    }

    fn finish(&self, url: &str) {
        let mut state = self.state.lock();
        state.active -= 1;
        *state.completed.entry(url.to_owned()).or_insert(0) += 1;
    }

    fn cancel(&self, url: &str) {
        let mut state = self.state.lock();
        state.active -= 1;
        *state.cancelled.entry(url.to_owned()).or_insert(0) += 1;
    }

    fn release(&self) {
        if let Some(release) = self.release.lock().take() {
            let _ = release.send(());
        }
    }

    fn snapshot(&self) -> LottieStickerPrefetchProbeSnapshot {
        self.state.lock().clone()
    }
}

struct LottieStickerPrefetchProbeRun {
    probe: Arc<LottieStickerPrefetchProbe>,
    url: String,
    finished: bool,
}

impl Drop for LottieStickerPrefetchProbeRun {
    fn drop(&mut self) {
        if !self.finished {
            self.probe.cancel(&self.url);
        }
    }
}

fn make_queue(
    maximum_concurrent: i64,
    maximum_pending: i64,
    probe: &Arc<LottieStickerPrefetchProbe>,
) -> Arc<LottieStickerPrefetchQueue> {
    let probe = Arc::clone(probe);
    LottieStickerPrefetchQueue::new(
        maximum_concurrent,
        Some(maximum_pending),
        None,
        Arc::new(move |request: LottieStickerPrefetchQueueRequest, _cx: AsyncApp| {
            let probe = Arc::clone(&probe);
            async move {
                probe.begin(&request.url_string);
                let mut run = LottieStickerPrefetchProbeRun {
                    probe: Arc::clone(&probe),
                    url: request.url_string.clone(),
                    finished: false,
                };
                let _ = probe.gate.clone().await;
                run.finished = true;
                probe.finish(&request.url_string);
            }
            .boxed_local()
        }),
    )
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn filled_image(side: u32, fill: Option<(u32, u32, u32, u32, [u8; 4])>) -> CleverImage {
    let mut rgba = RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
    if let Some((x, y, width, height, color)) = fill {
        for row in y..y + height {
            for column in x..x + width {
                rgba.put_pixel(column, row, Rgba(color));
            }
        }
    }
    CleverImageBitmapDecoder::render_image(rgba)
}

fn total(counts: &HashMap<String, i64>) -> i64 {
    counts.values().sum()
}

#[test]
fn composer_and_prefetch_share_first_frame_cache_size() {
    assert_eq!(LottieStickerPrefetch::FIRST_FRAME_POINT_SIZE, size(px(180.0), px(180.0)));
}

#[test]
fn composer_can_reuse_visible_lottie_frame_at_any_cached_size() {
    let account_user_id = SaiAccountCacheScope::active_user_id();
    let account_epoch = SaiAccountCacheScope::active_epoch();
    let url = format!("https://example.test/{}.lottie", Uuid::new_v4());
    let image = filled_image(44, Some((0, 0, 44, 44, [255, 45, 85, 255])));

    RiseLottieFirstFrameCache::store(
        Arc::clone(&image),
        &url,
        size(px(44.0), px(44.0)),
        account_user_id.as_deref(),
        account_epoch.as_deref(),
    );

    let latest = RiseLottieFirstFrameCache::latest_memory_image(&url, account_user_id.as_deref())
        .expect("first frame is cached");
    assert!(Arc::ptr_eq(&latest, &image));
}

#[test]
fn rejects_transparent_lottie_poster_but_accepts_visible_content() {
    let transparent = filled_image(32, None);
    let visible = filled_image(32, Some((8, 8, 16, 16, [175, 82, 222, 255])));

    assert!(!RiseLottieImageVisibility::has_visible_alpha(&transparent));
    assert!(RiseLottieImageVisibility::has_visible_alpha(&visible));
}

#[test]
fn composer_can_reuse_visible_raster_at_any_decoded_size() {
    let account_user_id = format!("composer-preview-{}", Uuid::new_v4());
    let account_epoch = format!("epoch-{}", Uuid::new_v4());
    let cache_key = format!("https://example.test/{}.webp", Uuid::new_v4());
    let image = filled_image(96, Some((0, 0, 96, 96, [0, 122, 255, 255])));

    CleverImageRenderMemoryCache::store(
        Arc::clone(&image),
        &cache_key,
        144,
        Some(&account_user_id),
        Some(&account_epoch),
    );

    let latest = CleverImageRenderMemoryCache::latest_image(&cache_key, Some(&account_user_id), Some(&account_epoch))
        .expect("raster is cached");
    assert!(Arc::ptr_eq(&latest, &image));
}

#[test]
fn missing_custom_emoji_uses_transparent_attachment_image() {
    let image = ChatComposerCustomEmojiPlaceholder::transparent_image();

    assert_eq!(image.size(0), size(DevicePixels(1), DevicePixels(1)));
    let bytes = image.as_bytes(0).expect("one frame");
    assert_eq!(bytes.len(), 4);
    assert_eq!(bytes[3], 0);
}

#[test]
fn detects_payload_container_from_bytes_instead_of_url_suffix() {
    let json = br#"{"v":"5.7.4","fr":30,"ip":0,"op":1,"w":1,"h":1,"layers":[]}"#;
    let tgs = STANDARD.decode(TGS_BASE64).expect("valid base64");
    let dot_lottie = [0x50, 0x4B, 0x03, 0x04];

    assert_eq!(LottieStickerLoader::detected_container(json), Some(LottieStickerContainer::Json));
    assert_eq!(LottieStickerLoader::detected_container(&tgs), Some(LottieStickerContainer::Ros));
    assert_eq!(
        LottieStickerLoader::detected_container(&dot_lottie),
        Some(LottieStickerContainer::DotLottie)
    );
    assert_eq!(LottieStickerLoader::detected_container(b"<html></html>"), None);
}

#[test]
fn normalizes_gzip_tgs_into_validated_lottie_json() {
    let tgs = STANDARD.decode(TGS_BASE64).expect("valid base64");

    let json = LottieStickerLoader::normalized_json_data(&tgs, LottieStickerContainer::Ros).expect("normalized");
    let root: Map<String, Value> = serde_json::from_slice(&json).expect("json object");

    assert_eq!(root.get("v").and_then(Value::as_str), Some("5.7.4"));
    assert!(root.get("layers").is_some_and(Value::is_array));
}

#[test]
fn rejects_non_lottie_json_even_when_extension_would_claim_json() {
    let invalid = br#"{"message":"not an animation"}"#;

    let result: Result<Vec<u8>, LottieStickerLoaderError> =
        LottieStickerLoader::normalized_json_data(invalid, LottieStickerContainer::Json);
    assert!(result.is_err());
}

#[gpui::test]
async fn bounds_concurrency_and_keeps_shared_persistent_owner_alive(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let probe = LottieStickerPrefetchProbe::new();
    let queue = make_queue(2, 8, &probe);
    let urls: Vec<String> = (0..5).map(|index| format!("https://example.test/sticker-{index}.lottie")).collect();
    let mut input = urls.clone();
    input.push(urls[0].clone());
    input.push(format!(" {} ", urls[1]));

    queue
        .enqueue(&input, Some("account-a".into()), Some("epoch-a".into()), None, &async_cx)
        .await;
    queue
        .replace_directional(
            &urls[..1],
            "chat-a",
            1,
            Some("account-a".into()),
            Some("epoch-a".into()),
            None,
            &async_cx,
        )
        .await;
    cx.run_until_parked();

    assert_eq!(probe.snapshot().active, 2);
    assert_eq!(queue.pending_count().await, 3);

    queue.cancel_directional("chat-a", 2, &async_cx).await;
    probe.release();
    cx.run_until_parked();

    let last = probe.snapshot();
    assert_eq!(total(&last.completed), i64::try_from(urls.len()).unwrap_or(i64::MAX));
    assert_eq!(last.maximum_active, 2);
    assert_eq!(last.start_order.len(), urls.len());
    assert!(last.cancelled.is_empty());
}

#[gpui::test]
async fn replaces_direction_and_rejects_stale_generation(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let probe = LottieStickerPrefetchProbe::new();
    let queue = make_queue(1, 8, &probe);
    let old: Vec<String> = (0..3).map(|index| format!("https://example.test/old-{index}.lottie")).collect();
    let new: Vec<String> = (0..2).map(|index| format!("https://example.test/new-{index}.lottie")).collect();
    let account = || (Some("account-a".to_owned()), Some("epoch-a".to_owned()));

    let (user, epoch) = account();
    queue
        .replace_directional(&old, "chat-a", 1, user, epoch, None, &async_cx)
        .await;
    cx.run_until_parked();
    assert_eq!(probe.snapshot().start_order.first(), Some(&old[0]));

    let (user, epoch) = account();
    queue
        .replace_directional(&new, "chat-a", 2, user, epoch, None, &async_cx)
        .await;
    probe.release();
    cx.run_until_parked();
    assert_eq!(total(&probe.snapshot().completed), i64::try_from(new.len()).unwrap_or(i64::MAX));

    let stale = strings(&["https://example.test/stale.lottie"]);
    let (user, epoch) = account();
    queue
        .replace_directional(&stale, "chat-a", 1, user, epoch, None, &async_cx)
        .await;
    cx.run_until_parked();

    let last = probe.snapshot();
    assert_eq!(last.cancelled.get(&old[0]), Some(&1));
    assert!(!last.start_order.contains(&old[1]));
    assert!(!last.start_order.contains(&old[2]));
    assert_eq!(last.completed.get(&new[0]), Some(&1));
    assert_eq!(last.completed.get(&new[1]), Some(&1));
    assert!(!last.start_order.contains(&stale[0]));
}

#[gpui::test]
async fn prioritizes_directional_work_within_bounded_pending_queue(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let probe = LottieStickerPrefetchProbe::new();
    let queue = make_queue(1, 2, &probe);
    let first = "https://example.test/persistent-first.lottie".to_owned();
    let second = "https://example.test/persistent-second.lottie".to_owned();
    let directional = "https://example.test/directional.lottie".to_owned();

    queue
        .enqueue(
            &[first.clone(), second.clone()],
            Some("account-a".into()),
            Some("epoch-a".into()),
            None,
            &async_cx,
        )
        .await;
    cx.run_until_parked();
    assert_eq!(probe.snapshot().start_order, vec![first.clone()]);

    queue
        .replace_directional(
            std::slice::from_ref(&directional),
            "chat-a",
            1,
            Some("account-a".into()),
            Some("epoch-a".into()),
            None,
            &async_cx,
        )
        .await;
    assert_eq!(queue.pending_count().await, 2);
    probe.release();
    cx.run_until_parked();

    let last = probe.snapshot();
    assert_eq!(total(&last.completed), 3);
    assert_eq!(last.start_order, vec![first, directional, second]);
    assert_eq!(last.maximum_active, 1);
}

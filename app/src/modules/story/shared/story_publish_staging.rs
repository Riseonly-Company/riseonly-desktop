use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;

use futures::channel::{mpsc, oneshot};
use futures::{FutureExt, StreamExt};
use gpui::{App, Global, Task};
use futures::future::{BoxFuture, Shared};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_engine::rise_upload::rise_upload_progress::{
    RiseUploadKey, RiseUploadPhase, RiseUploadProgress, RiseUploadProgressHub,
};
use rise_platform::AppPaths;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::paths::temporary_directory;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::widgets::notifier::notifier_upload_progress::NotifierUploadProgress;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

/// local spelling of the publish phases so the story stores dont name the engine phase
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoryUploadPhase {
    Preparing,
    Uploading,
    Processing,
    Completed,
    Failed,
}

static HUB: LazyLock<Arc<RiseUploadProgressHub>> = LazyLock::new(|| Arc::new(RiseUploadProgressHub::default()));

static KEY: LazyLock<RiseUploadKey> = LazyLock::new(|| RiseUploadKey::new("story.publish", 0));

static TAIL: LazyLock<Mutex<Option<Shared<oneshot::Receiver<()>>>>> = LazyLock::new(|| Mutex::new(None));

/// progress channel of the one story publish in flight
/// an observable fraction would redraw profile header and stories tray per chunk
pub enum StoryUploadProgressCenter {}

impl StoryUploadProgressCenter {
    pub fn hub() -> Arc<RiseUploadProgressHub> {
        Arc::clone(&HUB)
    }

    pub fn key() -> RiseUploadKey {
        KEY.clone()
    }

    fn enqueue(work: impl FnOnce(Arc<RiseUploadProgressHub>) -> BoxFuture<'static, ()> + Send + 'static) {
        let (done, finished) = oneshot::channel::<()>();
        let previous = TAIL.lock().replace(finished.shared());
        let hub = Self::hub();
        RiseAccountRuntime::current().spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            work(hub).await;
            let _ = done.send(());
        });
    }

    pub fn begin(cx: &mut App) {
        let owner_id = KEY.owner_id.clone();
        Self::enqueue(move |hub| async move { hub.begin(&owner_id).await }.boxed());
        if let Some(banner) = cx.default_global::<StoryUploadBannerState>().banner.take() {
            banner.finish(cx);
        }
        let banner = NotifierUploadProgress::new(
            tr("story_upload_background"),
            Some(tr("cancel")),
            Some(std::rc::Rc::new(|cx: &mut App| {
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.cancel_story_publish(cx));
            })),
            cx,
        );
        let (sender, mut receiver) = mpsc::unbounded::<(f64, StoryUploadPhase)>();
        *BANNER_TICKS.lock() = Some(sender);
        let drain = cx.spawn(async move |cx| {
            while let Some((fraction, phase)) = receiver.next().await {
                cx.update(|cx| Self::update_banner(fraction, phase, cx));
            }
        });
        let state = cx.default_global::<StoryUploadBannerState>();
        state.banner = Some(std::rc::Rc::new(banner));
        state.compresses = false;
        state.drain = Some(drain);
    }

    pub fn publish(fraction: f64, phase: StoryUploadPhase) {
        let engine_phase = match phase {
            StoryUploadPhase::Preparing => RiseUploadPhase::Preparing,
            StoryUploadPhase::Uploading => RiseUploadPhase::Uploading,
            StoryUploadPhase::Processing => RiseUploadPhase::Processing,
            StoryUploadPhase::Completed => RiseUploadPhase::Completed,
            StoryUploadPhase::Failed => RiseUploadPhase::Failed {
                code: "story_publish".to_owned(),
            },
        };
        let progress = RiseUploadProgress::new(Self::key(), fraction, 0, 0, engine_phase);
        Self::enqueue(move |hub| async move { hub.publish(progress).await }.boxed());
        if let Some(sender) = BANNER_TICKS.lock().as_ref() {
            let _ = sender.unbounded_send((fraction, phase));
        }
    }

    pub fn clear(cx: &mut App) {
        let owner_id = KEY.owner_id.clone();
        Self::enqueue(move |hub| async move { hub.remove_owner_id(&owner_id).await }.boxed());
        BANNER_TICKS.lock().take();
        let state = cx.default_global::<StoryUploadBannerState>();
        state.drain = None;
        if let Some(banner) = state.banner.take() {
            banner.finish(cx);
        }
    }

    fn update_banner(fraction: f64, phase: StoryUploadPhase, cx: &mut App) {
        let state = cx.default_global::<StoryUploadBannerState>();
        let Some(banner) = state.banner.as_ref() else {
            return;
        };
        let banner = banner.clone();
        match phase {
            StoryUploadPhase::Preparing => {
                state.compresses = true;
                banner.update(fraction * COMPRESSION_SHARE, cx);
            }
            StoryUploadPhase::Uploading => {
                let value = if state.compresses {
                    COMPRESSION_SHARE + fraction * (1.0 - COMPRESSION_SHARE)
                } else {
                    fraction
                };
                banner.update(value, cx);
            }
            StoryUploadPhase::Processing => banner.update(fraction, cx),
            StoryUploadPhase::Completed => {
                state.banner = None;
                banner.succeed(tr("story_publish_success"), None, cx);
            }
            StoryUploadPhase::Failed => {
                state.banner = None;
                banner.finish(cx);
            }
        }
    }
}

/// publish also runs under a bottom banner with the real percent
/// video compression fills the first 35 and transfer the rest up to 99
/// photo compresses in the first slice
#[derive(Default)]
struct StoryUploadBannerState {
    banner: Option<std::rc::Rc<NotifierUploadProgress>>,
    compresses: bool,
    drain: Option<Task<()>>,
}

impl Global for StoryUploadBannerState {}

const COMPRESSION_SHARE: f64 = 0.35;

static BANNER_TICKS: LazyLock<Mutex<Option<mpsc::UnboundedSender<(f64, StoryUploadPhase)>>>> =
    LazyLock::new(|| Mutex::new(None));

static DID_SWEEP: AtomicBool = AtomicBool::new(false);

/// app owned copy of the media a story publishes
/// picker exports are unowned temp files, composer closes before upload ends
/// bytes move to a dir this publish owns
pub enum StoryPublishStaging {}

impl StoryPublishStaging {
    const DIRECTORY_NAME: &'static str = "story-publish";
    const STALE_AGE_SECONDS: f64 = 24.0 * 60.0 * 60.0;

    pub fn stage(source: &Path) -> Option<PathBuf> {
        if !FileManager.file_exists(source) {
            return None;
        }
        let directory = Self::ensure_directory()?;

        let file_extension = source
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .filter(|extension| !extension.is_empty())
            .unwrap_or_else(|| "dat".to_owned());
        let destination = directory.join(format!("{}.{file_extension}", Uuid::new_v4().uuid_string()));
        match FileManager.copy_item(source, &destination) {
            Ok(()) => Some(destination),
            Err(_) => None,
        }
    }

    /// muxer own output, once this publish has taken its bytes
    pub fn discard_temporary(url: &Path) {
        if !url.starts_with(temporary_directory()) {
            return;
        }
        let url = url.to_path_buf();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = FileManager.remove_item(&url);
        }));
    }

    pub fn discard(url: Option<&Path>) {
        let Some(url) = url else {
            return;
        };
        if !Self::is_staged(url) {
            return;
        }
        let url = url.to_path_buf();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = FileManager.remove_item(&url);
        }));
    }

    pub fn discard_stale_files() {
        if DID_SWEEP.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(directory) = Self::ensure_directory() else {
            return;
        };
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let cutoff = Date::now().adding_time_interval(-Self::STALE_AGE_SECONDS);
            let contents = FileManager
                .contents_of_directory_at(&directory, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
                .unwrap_or_default();
            for url in contents {
                let modified = FileManager
                    .attributes_of_item(&url)
                    .ok()
                    .and_then(|attributes| attributes.modification_date)
                    .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
                    .map(|since| Date::from_time_interval_since_1970(since.as_secs_f64()))
                    .unwrap_or_else(Date::distant_past);
                if modified >= cutoff {
                    continue;
                }
                let _ = FileManager.remove_item(&url);
            }
        }));
    }

    fn is_staged(url: &Path) -> bool {
        url.parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == Self::DIRECTORY_NAME)
    }

    fn ensure_directory() -> Option<PathBuf> {
        let base = AppPaths::resolve(AppEnvironment::current().canon())
            .ok()
            .map(|paths| paths.cache_dir().to_path_buf())?;

        let directory = base.join(Self::DIRECTORY_NAME);
        if !FileManager.file_exists(&directory) {
            let _ = FileManager.create_directory(&directory, true);
        }
        Some(directory)
    }
}

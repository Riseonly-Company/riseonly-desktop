use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, Context, Entity, Size, Task, WeakEntity, Window};
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::photo_library::picked_files_library;
use rise_platform::photo_library::{
    self, ChangeWatch, ImageContentMode, ImageDeliveryMode, ImageRequestId, LibraryAsset, LibraryAssetCollection,
    LibraryAssetKind, LibraryImage, LibraryMediaKind,
};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerMediaKind;
use crate::modules::media_picker::library::photo_library_albums::{PhotoLibraryAlbum, PhotoLibraryAlbums};
use crate::modules::media_picker::library::media_picker_thumbnail_pipeline::MediaPickerThumbnailPipelineQuality;
use crate::modules::media_picker::library::photos_image_request::{PhotoImageDelivery, PhotosImageRequest};
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::media_picker::shared::media_picker_performance_diagnostics::MediaPickerPerformanceDiagnostics;

pub trait PhotoLibraryFetcherDelegate: Sized + 'static {
    fn photo_library_did_change(&mut self, fetcher: &Entity<PhotoLibraryFetcher>, cx: &mut Context<Self>);
    fn photo_library_filter_did_change(&mut self, fetcher: &Entity<PhotoLibraryFetcher>, cx: &mut Context<Self>) {
        self.photo_library_did_change(fetcher, cx);
    }
}

pub enum MediaPickerGridThumbnailPolicy {}

impl MediaPickerGridThumbnailPolicy {
    pub const TARGET_SIZE: Size<f32> = Size {
        width: 256.0,
        height: 256.0,
    };
}

static GUIDE_GALLERY_PHOTO: Mutex<Option<PathBuf>> = Mutex::new(None);

type PhotoLibraryFetcherDelegateCall = Rc<dyn Fn(&Entity<PhotoLibraryFetcher>, &mut App)>;

#[derive(Clone)]
struct PhotoLibraryFetcherDelegateCalls {
    did_change: PhotoLibraryFetcherDelegateCall,
    filter_did_change: PhotoLibraryFetcherDelegateCall,
}

/// why a fetch ran
/// only a library change may be held, a filter change reads a new list
#[derive(Clone, Copy, PartialEq, Eq)]
enum PhotoLibraryFetchReason {
    Initial,
    LibraryChange,
    FilterChange,
}

/// pickers view of the photo library
/// newest first assets of one media kind, kept in step with library changes
/// plus grid and hero image requests
pub struct PhotoLibraryFetcher {
    delegate: Option<PhotoLibraryFetcherDelegateCalls>,
    media_kind: MediaPickerMediaKind,
    shown_kind: MediaPickerMediaKind,
    album_id: String,
    album_collection: Option<LibraryAssetCollection>,
    fetch_result: Arc<Vec<LibraryAsset>>,
    pub holds_insertions: bool,
    pub should_hold_insertions: Option<Rc<dyn Fn(&App) -> bool>>,
    held_result: Option<Arc<Vec<LibraryAsset>>>,
    reads_picked_files: bool,
    picked_assets: Vec<LibraryAsset>,
    _fetch_task: Option<Task<()>>,
    _change_watch: Option<ChangeWatch>,
    _change_task: Task<()>,
}

impl PhotoLibraryFetcher {
    pub fn new(media_kind: MediaPickerMediaKind, cx: &mut Context<Self>) -> Self {
        let reads_picked_files = true;
        let _ = picked_files_library::install_picked_video_poster(Self::picked_video_poster);
        let (changes, mut change_events) = mpsc::unbounded::<()>();
        let change_watch = if reads_picked_files {
            None
        } else {
            match photo_library::watch_changes(move || {
                let _ = changes.unbounded_send(());
            }) {
                Ok(watch) => Some(watch),
                Err(error) => {
                    tracing::debug!(target: "riseonly", %error, "photo library change observer not registered");
                    None
                }
            }
        };
        let change_task = cx.spawn(async move |this, cx| {
            while change_events.next().await.is_some() {
                if this.update(cx, |this, cx| this.photo_library_did_change(cx)).is_err() {
                    break;
                }
            }
        });
        let mut fetcher = Self {
            delegate: None,
            media_kind,
            shown_kind: media_kind,
            album_id: PhotoLibraryAlbum::RECENTS_ID.to_string(),
            album_collection: None,
            fetch_result: Arc::new(Vec::new()),
            holds_insertions: false,
            should_hold_insertions: None,
            held_result: None,
            reads_picked_files,
            picked_assets: Vec::new(),
            _fetch_task: None,
            _change_watch: change_watch,
            _change_task: change_task,
        };
        fetcher.run_fetch(PhotoLibraryFetchReason::Initial, cx);
        fetcher.list_guide_gallery_photo(cx);
        fetcher
    }

    /// set by the guide sandbox while a lesson anchors on the first gallery photo, cleared when it ends
    pub fn set_guide_gallery_photo(path: Option<PathBuf>) {
        if let Ok(mut slot) = GUIDE_GALLERY_PHOTO.lock() {
            *slot = path;
        }
    }

    fn list_guide_gallery_photo(&mut self, cx: &mut Context<Self>) {
        let path = GUIDE_GALLERY_PHOTO.lock().ok().and_then(|slot| slot.clone());
        let Some(path) = path.filter(|_| self.reads_picked_files) else {
            return;
        };
        let register = RiseAccountRuntime::current()
            .spawn_blocking(move || picked_files_library::register_picked_files(&[path]));
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let Ok(registered) = register.await else {
                return;
            };
            let _ = this.update(cx, |fetcher, cx| fetcher.append_picked(registered, cx));
        })
        .detach();
    }

    pub fn set_delegate<D: PhotoLibraryFetcherDelegate>(&mut self, delegate: Option<WeakEntity<D>>) {
        self.delegate = delegate.map(|delegate| {
            let filter_delegate = delegate.clone();
            PhotoLibraryFetcherDelegateCalls {
                did_change: Rc::new(move |fetcher: &Entity<PhotoLibraryFetcher>, cx: &mut App| {
                    let _ = delegate.update(cx, |delegate, cx| delegate.photo_library_did_change(fetcher, cx));
                }) as PhotoLibraryFetcherDelegateCall,
                filter_did_change: Rc::new(move |fetcher: &Entity<PhotoLibraryFetcher>, cx: &mut App| {
                    let _ = filter_delegate
                        .update(cx, |delegate, cx| delegate.photo_library_filter_did_change(fetcher, cx));
                }) as PhotoLibraryFetcherDelegateCall,
            }
        });
    }

    /// true when the grid lists brought files instead of the system gallery
    pub fn reads_picked_files(&self) -> bool {
        self.reads_picked_files
    }

    pub fn has_picked_files(&self) -> bool {
        !self.picked_assets.is_empty()
    }

    pub fn picked_asset(&self, id: &str) -> Option<&LibraryAsset> {
        self.picked_assets.iter().find(|asset| asset.id == id)
    }

    pub fn add_picked_files(
        fetcher: &Entity<Self>,
        selection: &Entity<MediaSelectionState>,
        paths: Vec<PathBuf>,
        cx: &mut App,
    ) {
        Self::add_picked_files_then(fetcher, selection, paths, true, cx, |_, _| {});
    }

    /// selects false leaves the selection alone, eg a one photo story pick opens the editor instead
    /// then gets the newly listed files once they are in the grid
    pub fn add_picked_files_then(
        fetcher: &Entity<Self>,
        selection: &Entity<MediaSelectionState>,
        paths: Vec<PathBuf>,
        selects: bool,
        cx: &mut App,
        then: impl FnOnce(Vec<LibraryAsset>, &mut App) + 'static,
    ) {
        if paths.is_empty() {
            return;
        }
        let register = RiseAccountRuntime::current().spawn_blocking(move || picked_files_library::register_picked_files(&paths));
        let fetcher = fetcher.downgrade();
        let selection = selection.downgrade();
        cx.spawn(async move |cx| {
            let Ok(registered) = register.await else {
                return;
            };
            let added = fetcher
                .update(cx, |fetcher, cx| fetcher.append_picked(registered, cx))
                .unwrap_or_default();
            if selects {
                let _ = selection.update(cx, |selection, cx| {
                    for asset in &added {
                        if !selection.contains(&asset.id) && selection.can_select(asset) {
                            let _ = selection.toggle(asset, cx);
                        }
                    }
                });
            }
            if !added.is_empty() {
                cx.update(|cx| then(added, cx));
            }
        })
        .detach();
    }

    /// keeps files of the kinds this picker takes, drops repeats and returns the new ones
    fn append_picked(&mut self, registered: Vec<LibraryAsset>, cx: &mut Context<Self>) -> Vec<LibraryAsset> {
        let takes = |kind: LibraryAssetKind| match self.media_kind {
            MediaPickerMediaKind::All => true,
            MediaPickerMediaKind::Images => kind == LibraryAssetKind::Image,
            MediaPickerMediaKind::Videos => kind == LibraryAssetKind::Video,
        };
        let added: Vec<LibraryAsset> = registered
            .into_iter()
            .filter(|asset| takes(asset.kind))
            .filter(|asset| !self.picked_assets.iter().any(|known| known.id == asset.id))
            .collect();
        if added.is_empty() {
            tracing::warn!(target: "riseonly::X644", "picked files: nothing new to list");
            return added;
        }
        self.picked_assets.extend(added.iter().cloned());
        self.run_fetch(PhotoLibraryFetchReason::FilterChange, cx);
        added
    }

    fn picked_video_poster(path: &Path, max_side: u32) -> Option<LibraryImage> {
        let poster = rise_media::video_export::poster(path, 0.0, max_side.max(1)).ok()?;
        Some(LibraryImage {
            width: poster.width,
            height: poster.height,
            bgra: poster.bgra,
        })
    }

    fn picked_of_kind(&self, kind: LibraryMediaKind) -> Vec<LibraryAsset> {
        self.picked_assets
            .iter()
            .filter(|asset| match kind {
                LibraryMediaKind::All => true,
                LibraryMediaKind::Images => asset.kind == LibraryAssetKind::Image,
                LibraryMediaKind::Videos => asset.kind == LibraryAssetKind::Video,
            })
            .cloned()
            .collect()
    }

    pub fn shown_kind(&self) -> MediaPickerMediaKind {
        self.shown_kind
    }

    pub fn album_id(&self) -> &str {
        &self.album_id
    }

    pub fn has_held_insertions(&self) -> bool {
        self.held_result.is_some()
    }

    pub fn release_held_insertions(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.held_result.take() else {
            return;
        };
        self.fetch_result = result;
        cx.notify();
        self.notify_delegate(PhotoLibraryFetchReason::LibraryChange, cx);
    }

    pub fn set_shown_kind(&mut self, kind: MediaPickerMediaKind, cx: &mut Context<Self>) {
        if self.media_kind != MediaPickerMediaKind::All || kind == self.shown_kind {
            return;
        }
        self.shown_kind = kind;
        self.held_result = None;
        cx.notify();
        self.run_fetch(PhotoLibraryFetchReason::FilterChange, cx);
    }

    pub fn set_album(&mut self, id: &str, cx: &mut Context<Self>) {
        if id == self.album_id {
            return;
        }
        self.album_id = id.to_string();
        self.held_result = None;
        cx.notify();
        self.run_fetch(PhotoLibraryFetchReason::FilterChange, cx);
    }

    pub fn fetch_result(&self) -> &[LibraryAsset] {
        &self.fetch_result
    }

    pub fn asset_count(&self) -> usize {
        self.fetch_result.len()
    }

    pub fn asset(&self, at: i64) -> Option<&LibraryAsset> {
        let index = usize::try_from(at).ok()?;
        self.fetch_result.get(index)
    }

    pub fn creation_date(&self, at: i64) -> Option<Date> {
        self.asset(at)?.created_at_ms.map(Date::from_ms)
    }

    pub fn request_grid_thumbnail(
        &mut self,
        asset: &LibraryAsset,
        mut result: impl FnMut(PhotoImageDelivery, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Option<ImageRequestId> {
        let diagnostics = MediaPickerPerformanceDiagnostics::shared();
        diagnostics.record_thumbnail_requested(MediaPickerThumbnailPipelineQuality::Preview);
        diagnostics.record_thumbnail_started(MediaPickerThumbnailPipelineQuality::Preview);
        let (deliveries, mut delivered) = mpsc::unbounded::<PhotoImageDelivery>();
        let request_id = PhotosImageRequest::request_image(
            &asset.id,
            MediaPickerGridThumbnailPolicy::TARGET_SIZE,
            ImageContentMode::AspectFill,
            ImageDeliveryMode::Opportunistic,
            true,
            move |delivery| {
                if !delivery.is_cancelled && delivery.image.is_some() {
                    MediaPickerPerformanceDiagnostics::shared()
                        .record_thumbnail_delivery(delivery.is_degraded, MediaPickerThumbnailPipelineQuality::Preview);
                }
                let _ = deliveries.unbounded_send(delivery);
            },
        );
        cx.spawn(async move |_, cx| {
            while let Some(delivery) = delivered.next().await {
                cx.update(|cx| result(delivery, cx));
            }
        })
        .detach();
        request_id
    }

    pub fn cancel_grid_thumbnail_request(&mut self, id: Option<ImageRequestId>) {
        let Some(id) = id else {
            return;
        };
        photo_library::cancel_image(id);
        MediaPickerPerformanceDiagnostics::shared()
            .record_thumbnail_cancelled(Some(MediaPickerThumbnailPipelineQuality::Preview));
    }

    /// the window is the screen here, its viewport in device pixels is the target size
    pub fn request_hero_thumbnail(
        &mut self,
        asset: &LibraryAsset,
        window: &Window,
        result: impl FnOnce(Option<CleverImage>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Option<ImageRequestId> {
        let scale = window.scale_factor();
        let screen = window.viewport_size();
        let target_size = Size {
            width: f32::from(screen.width) * scale,
            height: f32::from(screen.height) * scale,
        };
        let cache_key = format!("ph://{}", asset.id);
        if let Some(cached) = MediaViewerImageMemoryCache::image(
            &cache_key,
            SaiAccountCacheScope::active_user_id().as_deref(),
            CleverImageCachePolicy::Standard,
        ) {
            cx.defer(move |cx| result(Some(cached), cx));
            return None;
        }
        let (image_sender, image_receiver) = oneshot::channel::<CleverImage>();
        let mut image_sender = Some(image_sender);
        let request_id = PhotosImageRequest::request_image(
            &asset.id,
            target_size,
            ImageContentMode::AspectFill,
            ImageDeliveryMode::HighQuality,
            true,
            move |delivery| {
                if delivery.is_cancelled {
                    return;
                }
                let Some(image) = delivery.image else {
                    return;
                };
                MediaViewerImageMemoryCache::store(
                    Arc::clone(&image),
                    &cache_key,
                    SaiAccountCacheScope::active_user_id().as_deref(),
                );
                if let Some(sender) = image_sender.take() {
                    let _ = sender.send(image);
                }
            },
        );
        cx.spawn(async move |_, cx| {
            if let Ok(image) = image_receiver.await {
                cx.update(|cx| result(Some(image), cx));
            }
        })
        .detach();
        request_id
    }

    pub fn cancel_image_request(&mut self, id: Option<ImageRequestId>) {
        let Some(id) = id else {
            return;
        };
        photo_library::cancel_image(id);
    }

    fn library_kind(kind: MediaPickerMediaKind) -> LibraryMediaKind {
        match kind {
            MediaPickerMediaKind::Images => LibraryMediaKind::Images,
            MediaPickerMediaKind::Videos => LibraryMediaKind::Videos,
            MediaPickerMediaKind::All => LibraryMediaKind::All,
        }
    }

    fn run_fetch(&mut self, reason: PhotoLibraryFetchReason, cx: &mut Context<Self>) {
        let kind = Self::library_kind(self.shown_kind);
        if self.reads_picked_files {
            self._fetch_task = None;
            let assets = self.picked_of_kind(kind);
            self.apply_change(assets, reason, cx);
            return;
        }
        let album_id = self.album_id.clone();
        let known_collection = self.album_collection.clone();
        let resolves_album = reason == PhotoLibraryFetchReason::FilterChange;
        let fetch = RiseAccountRuntime::current().spawn_blocking(move || {
            let collection =
                if resolves_album { PhotoLibraryAlbums::collection(&album_id) } else { known_collection };
            let assets = Self::fetch_assets(kind, collection.as_ref());
            (collection, assets)
        });
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let Ok((collection, assets)) = fetch.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if resolves_album {
                    if collection.is_none() {
                        this.album_id = PhotoLibraryAlbum::RECENTS_ID.to_string();
                    }
                    this.album_collection = collection;
                }
                this.apply_change(assets, reason, cx);
            });
        }));
    }

    fn fetch_assets(kind: LibraryMediaKind, collection: Option<&LibraryAssetCollection>) -> Vec<LibraryAsset> {
        let page = match collection {
            Some(collection) => photo_library::collection_page(collection, kind, 0, usize::MAX),
            None => photo_library::page(kind, 0, usize::MAX),
        };
        match page {
            Ok(page) => page.assets,
            Err(error) => {
                tracing::debug!(target: "riseonly", %error, "photo library fetch answered empty");
                Vec::new()
            }
        }
    }

    fn apply_change(&mut self, fetch_result: Vec<LibraryAsset>, reason: PhotoLibraryFetchReason, cx: &mut Context<Self>) {
        if reason == PhotoLibraryFetchReason::LibraryChange {
            let holds = match self.should_hold_insertions.as_ref() {
                Some(should_hold) => should_hold(cx),
                None => self.holds_insertions,
            };
            let is_holding = self.held_result.is_some();
            let latest = self.held_result.clone().unwrap_or_else(|| Arc::clone(&self.fetch_result));
            if is_holding && *latest == fetch_result {
                return;
            }
            if holds && Self::keeps_shown_indexes(&latest, &fetch_result, is_holding) {
                self.held_result = Some(Arc::new(fetch_result));
                return;
            }
            self.held_result = None;
        }
        self.fetch_result = Arc::new(fetch_result);
        cx.notify();
        self.notify_delegate(reason, cx);
    }

    fn notify_delegate(&self, reason: PhotoLibraryFetchReason, cx: &mut Context<Self>) {
        let Some(delegate) = self.delegate.clone() else {
            return;
        };
        let call = match reason {
            PhotoLibraryFetchReason::FilterChange => delegate.filter_did_change,
            PhotoLibraryFetchReason::Initial | PhotoLibraryFetchReason::LibraryChange => delegate.did_change,
        };
        let fetcher = cx.entity();
        cx.defer(move |cx| call(&fetcher, cx));
    }

    fn photo_library_did_change(&mut self, cx: &mut Context<Self>) {
        self.run_fetch(PhotoLibraryFetchReason::LibraryChange, cx);
    }

    fn keeps_shown_indexes(latest: &[LibraryAsset], after: &[LibraryAsset], while_holding: bool) -> bool {
        let after_ids: HashSet<&str> = after.iter().map(|asset| asset.id.as_str()).collect();
        if latest.iter().any(|asset| !after_ids.contains(asset.id.as_str())) {
            return false;
        }
        let latest_ids: HashSet<&str> = latest.iter().map(|asset| asset.id.as_str()).collect();
        let kept_in_order = after.iter().filter(|asset| latest_ids.contains(asset.id.as_str()));
        if !kept_in_order.map(|asset| asset.id.as_str()).eq(latest.iter().map(|asset| asset.id.as_str())) {
            return false;
        }
        while_holding || after.len() > latest.len()
    }
}

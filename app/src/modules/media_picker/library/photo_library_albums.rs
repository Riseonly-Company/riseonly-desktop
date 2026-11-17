use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, size};
use rise_i18n::tr;
use rise_platform::photo_library::{
    self, ImageContentMode, ImageDeliveryMode, LibraryAsset, LibraryAssetCollection, LibraryMediaKind,
    LibrarySmartAlbum,
};

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerMediaKind;
use crate::modules::media_picker::library::photos_image_request::PhotosImageRequest;

/// one album of the picker album menu
/// recents first then system smart albums then the users own
/// system albums use the app names and user albums keep theirs
#[derive(Clone, Debug)]
pub struct PhotoLibraryAlbum {
    pub id: String,
    pub title: String,
    pub count: i64,
    pub cover: Option<LibraryAsset>,
}

impl PhotoLibraryAlbum {
    pub const RECENTS_ID: &'static str = "recents";
}

impl PartialEq for PhotoLibraryAlbum {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.title == other.title
            && self.count == other.count
            && self.cover.as_ref().map(|cover| cover.id.as_str()) == other.cover.as_ref().map(|cover| cover.id.as_str())
    }
}

/// names of recents and the smart albums in the apps language, read on the main thread
/// why: system title follows the device language
/// so a russian app showed Recents and Videos
#[derive(Clone, Debug)]
pub struct PhotoLibraryAlbumsTitles {
    pub recents: String,
    smart: Vec<String>,
}

impl PhotoLibraryAlbumsTitles {
    pub fn new() -> Self {
        Self {
            recents: tr("media_picker_album_recents"),
            smart: PhotoLibraryAlbums::SMART_ALBUMS.iter().map(|(_, key)| tr(key)).collect(),
        }
    }
}

impl Default for PhotoLibraryAlbumsTitles {
    fn default() -> Self {
        Self::new()
    }
}

pub enum PhotoLibraryAlbums {}

impl PhotoLibraryAlbums {
    const SMART_ALBUMS: [(LibrarySmartAlbum, &'static str); 10] = [
        (LibrarySmartAlbum::Favorites, "media_picker_album_favorites"),
        (LibrarySmartAlbum::Videos, "media_picker_album_videos"),
        (LibrarySmartAlbum::SelfPortraits, "media_picker_album_selfies"),
        (LibrarySmartAlbum::LivePhotos, "media_picker_album_live_photos"),
        (LibrarySmartAlbum::DepthEffect, "media_picker_album_portraits"),
        (LibrarySmartAlbum::Panoramas, "media_picker_album_panoramas"),
        (LibrarySmartAlbum::Timelapses, "media_picker_album_timelapses"),
        (LibrarySmartAlbum::SlomoVideos, "media_picker_album_slomo"),
        (LibrarySmartAlbum::Bursts, "media_picker_album_bursts"),
        (LibrarySmartAlbum::Screenshots, "media_picker_album_screenshots"),
    ];

    pub fn fetch_options(kind: MediaPickerMediaKind) -> LibraryMediaKind {
        match kind {
            MediaPickerMediaKind::Images => LibraryMediaKind::Images,
            MediaPickerMediaKind::Videos => LibraryMediaKind::Videos,
            MediaPickerMediaKind::All => LibraryMediaKind::All,
        }
    }

    pub fn collection(id: &str) -> Option<LibraryAssetCollection> {
        if id == PhotoLibraryAlbum::RECENTS_ID {
            return None;
        }
        photo_library::asset_collection(id)
    }

    /// off the main thread, counts every album under the pickers kind
    /// empty albums are left out, an image only picker has no videos row
    pub fn load(kind: MediaPickerMediaKind, titles: &PhotoLibraryAlbumsTitles) -> Vec<PhotoLibraryAlbum> {
        let options = Self::fetch_options(kind);
        let mut albums: Vec<PhotoLibraryAlbum> = Vec::new();
        let (count, cover) = match photo_library::page(options, 0, 1) {
            Ok(page) => (page.total, page.assets.into_iter().next()),
            Err(_) => (0, None),
        };
        albums.push(PhotoLibraryAlbum {
            id: PhotoLibraryAlbum::RECENTS_ID.to_string(),
            title: titles.recents.clone(),
            count: i64::try_from(count).unwrap_or(i64::MAX),
            cover,
        });
        for (index, (subtype, _)) in Self::SMART_ALBUMS.iter().enumerate() {
            if matches!(subtype, LibrarySmartAlbum::Videos) && kind != MediaPickerMediaKind::All {
                continue;
            }
            let title = titles.smart.get(index).map(String::as_str);
            for collection in photo_library::smart_albums(*subtype) {
                if let Some(album) = Self::album(&collection, options, title) {
                    albums.push(album);
                }
            }
        }
        for collection in photo_library::user_albums() {
            if let Some(album) = Self::album(&collection, options, None) {
                albums.push(album);
            }
        }
        albums
    }

    fn album(
        collection: &LibraryAssetCollection,
        options: LibraryMediaKind,
        app_title: Option<&str>,
    ) -> Option<PhotoLibraryAlbum> {
        let page = photo_library::collection_page(collection, options, 0, 1).ok()?;
        if page.total == 0 {
            return None;
        }
        let title = app_title.map(str::to_string).or_else(|| collection.localized_title.clone())?;
        if title.is_empty() {
            return None;
        }
        Some(PhotoLibraryAlbum {
            id: collection.id.clone(),
            title,
            count: i64::try_from(page.total).unwrap_or(i64::MAX),
            cover: page.assets.into_iter().next(),
        })
    }

    const COVER_SIDE: f32 = 28.0;

    pub fn request_cover(
        asset: &LibraryAsset,
        scale: f32,
        mut result: impl FnMut(Option<CleverImage>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let side = Self::COVER_SIDE * scale;
        let (deliveries, mut delivered) = mpsc::unbounded::<Option<CleverImage>>();
        PhotosImageRequest::request_image(
            &asset.id,
            size(side, side),
            ImageContentMode::AspectFill,
            ImageDeliveryMode::Opportunistic,
            true,
            move |delivery| {
                let _ = deliveries.unbounded_send(delivery.image);
            },
        );
        cx.spawn(async move |cx| {
            while let Some(image) = delivered.next().await {
                cx.update(|cx| result(image, cx));
            }
        })
        .detach();
    }
}

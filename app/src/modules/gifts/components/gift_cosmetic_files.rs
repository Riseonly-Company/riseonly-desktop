use gpui::{App, AsyncApp};
use rise_core::foundation::foundation_url::Url;

use crate::core::lib::sai::sai::Sai;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions,
};
use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAppearanceDTO, RiseGiftAppearanceExt, RiseGiftItemKind, RiseGiftMediaDTO, RiseGiftMediaExt,
    RiseGiftMediaKind,
};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;

/// a still a surface decodes and the size it asks for
#[derive(Clone, Debug, PartialEq, Eq)]
struct GiftCosmeticPoster {
    url: String,
    pixels: i64,
}

pub enum GiftCosmeticFiles {}

impl GiftCosmeticFiles {
    /// the movie on disk, downloaded first when missing none if it couldnt be fetched
    pub async fn local(movie: &Url, cx: &AsyncApp) -> Option<Url> {
        if movie.is_file_url() {
            return Some(movie.clone());
        }
        let local = RemoteMediaDiskCache::shared()
            .ensure_cached(
                movie,
                RemoteMediaDiskCacheMaterializeOptions {
                    cache_key: Some(RiseMediaCacheIdentity::key(movie)),
                    ..RemoteMediaDiskCacheMaterializeOptions::default()
                },
                cx,
            )
            .await?;
        Url::from_file_path(&local)
    }

    /// puts everything one look plays and shows on disk before its needed
    /// movie of each worn model and rolled effect, and posters at the sizes surfaces decode
    pub fn warm(
        appearance: &RiseGiftAppearanceDTO,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut App,
    ) {
        let media: Vec<RiseGiftMediaDTO> = RiseGiftItemKind::cosmetics()
            .into_iter()
            .filter_map(|kind| appearance.worn(kind))
            .flat_map(|worn| {
                [
                    worn.model.as_ref().and_then(|model| model.media.clone()),
                    worn.effect.as_ref().and_then(|effect| effect.media.clone()),
                ]
            })
            .flatten()
            .collect();
        if media.is_empty() {
            return;
        }
        let movies: Vec<Url> = media.iter().filter_map(|media| media.cosmetic_movie()).collect();
        let posters: Vec<GiftCosmeticPoster> = media.iter().flat_map(Self::poster_sizes).collect();
        cx.spawn(async move |cx: &mut AsyncApp| {
            for movie in movies {
                let local = RemoteMediaDiskCache::shared()
                    .ensure_cached(
                        &movie,
                        RemoteMediaDiskCacheMaterializeOptions {
                            cache_key: Some(RiseMediaCacheIdentity::key(&movie)),
                            expected_account_user_id: account_user_id.clone(),
                            expected_account_epoch: account_epoch.clone(),
                            ..RemoteMediaDiskCacheMaterializeOptions::default()
                        },
                        cx,
                    )
                    .await;
                if local.is_none() {
                    let message = format!("could not fetch ahead: {}", movie.last_path_component());
                    cx.update(|cx| Self::note(message, cx));
                }
            }
            for poster in posters {
                let _ = CleverImageBitmapLoader::load(
                    &poster.url,
                    true,
                    true,
                    account_user_id.clone(),
                    account_epoch.clone(),
                    Some(poster.pixels),
                    CleverImageCachePolicy::Standard,
                    CleverImageWorkLane::Speculative,
                    cx,
                )
                .await;
            }
        })
        .detach();
    }

    fn poster_sizes(media: &RiseGiftMediaDTO) -> Vec<GiftCosmeticPoster> {
        let poster = |small: bool, pixels: i64| {
            media
                .cosmetic_poster(small)
                .map(|url| GiftCosmeticPoster { url, pixels })
        };
        match media.media_kind() {
            Some(RiseGiftMediaKind::FrameVideo) => {
                [poster(false, 768), poster(true, 360)].into_iter().flatten().collect()
            }
            Some(RiseGiftMediaKind::PlateVideo) => poster(false, 784).into_iter().collect(),
            Some(RiseGiftMediaKind::BannerVideo) => poster(false, 786).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    pub fn note(message: String, cx: &mut App) {
        Sai::log_channel("GiftCosmetic", move || message, "cosmetics", cx);
    }
}

use std::collections::HashMap;
use std::path::Path;

use gpui::{AppContext, Context};
use image::{Rgba, RgbaImage};
use rise_canon::RiseIdentityCanon;
use rise_canon::{RiseGuideSignal, RiseUploadFormat};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::emoji_raster::{EmojiRaster, render_emoji};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::stickers::engine::rise_sticker_models::{
    RiseStickerPackContent, RiseStickerPage, RiseStickerPresentationSnapshot,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    PackKind, StickerAssetType, StickerItem, StickerPack,
};

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

/// stickers for the own pack and favourites lessons
/// panda's pack of three is drawn to the temp folder on arm and sits among saved packs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxStickersScene {
    pub panda_pack: StickerPack,
    pub panda_stickers: Vec<StickerItem>,
    pub own_pack: Option<StickerPack>,
    pub own_stickers: Vec<StickerItem>,
    pub favourites: HashMap<String, bool>,
    pub starred: Vec<StickerItem>,
    pub files: Vec<Url>,
}

impl GuideSandboxStickersScene {
    pub fn new(panda_pack: StickerPack) -> Self {
        Self {
            panda_pack,
            panda_stickers: Vec::new(),
            own_pack: None,
            own_stickers: Vec::new(),
            favourites: HashMap::new(),
            starred: Vec::new(),
            files: Vec::new(),
        }
    }
}

impl GuideSandbox {
    const STICKER_SIDE: i64 = 512;
    const FALLBACK_STICKER_EMOJI: &'static str = "\u{1F642}";
    const FALLBACK_STICKER_EXTENSION: &'static str = "png";

    pub fn own_sticker_pack_id() -> String {
        format!("{OPTIMISTIC_TEMPORARY_PREFIX}guide_sticker_pack_own")
    }

    pub fn panda_first_sticker_id() -> String {
        format!("{}_1", Self::sticker_pack_id())
    }

    pub fn owns_stickers_scene(&self) -> bool {
        self.is_armed && self.stickers_scene.is_some()
    }

    pub fn owns_sticker_pack(&self, id: &str) -> bool {
        if !self.owns_stickers_scene() {
            return false;
        }
        id == &*Self::sticker_pack_id() || id == Self::own_sticker_pack_id()
    }

    pub fn arm_stickers(&mut self, cx: &mut Context<Self>) {
        let stamp = (Self::scene_stamp() * 1000.0) as i64;
        self.stickers_scene = Some(GuideSandboxStickersScene::new(StickerPack::new(
            Self::sticker_pack_id().to_owned(),
            tr("guide_scene_sender"),
            String::new(),
            false,
            false,
            stamp,
            stamp,
            false,
            true,
            PackKind::Sticker,
            String::new(),
            StickerAssetType::StaticImage,
            false,
            0,
            None,
        )));
        cx.notify();
        let render = cx.background_spawn(async move { GuideSandboxStickerArt::render() });
        let task = cx.spawn(async move |this, cx| {
            let files = render.await;
            this.update(cx, |sandbox, cx| {
                if !sandbox.is_armed {
                    return;
                }
                let Some(scene) = sandbox.stickers_scene.as_mut() else {
                    return;
                };
                scene.panda_stickers = files
                    .iter()
                    .enumerate()
                    .map(|(index, file)| {
                        Self::sticker_item(
                            format!("{}_{}", Self::sticker_pack_id(), index + 1),
                            Self::sticker_pack_id().to_owned(),
                            &file.url,
                            file.emoji.clone(),
                            index as i64,
                            stamp - index as i64,
                        )
                    })
                    .collect();
                scene.files.extend(files.into_iter().map(|file| file.url));
                cx.notify();
            })
            .ok();
        });
        self.scene_tasks.push(task);
    }

    pub fn withdraw_stickers(&mut self) {
        let Some(scene) = self.stickers_scene.as_ref() else {
            return;
        };
        for file in &scene.files {
            if let Some(path) = file.to_file_path() {
                FileManager.remove_item(&path).ok();
            }
        }
    }

    /// reader's sticker state with the lesson's packs and favourites laid on top
    pub fn stickers_snapshot(&self, real: RiseStickerPresentationSnapshot) -> RiseStickerPresentationSnapshot {
        let Some(scene) = self.stickers_scene.as_ref().filter(|_| self.owns_stickers_scene()) else {
            return real;
        };
        let mut snapshot = real;
        let own: Vec<StickerPack> = scene.own_pack.iter().cloned().collect();

        let mut saved = snapshot.saved_packs.get(&PackKind::Sticker).cloned().unwrap_or_default();
        saved.items = own
            .iter()
            .cloned()
            .chain(std::iter::once(scene.panda_pack.clone()))
            .chain(saved.items)
            .collect();
        snapshot.saved_packs.insert(PackKind::Sticker, saved);

        if !own.is_empty() {
            let mut mine: RiseStickerPage<StickerPack> =
                snapshot.my_packs.get(&PackKind::Sticker).cloned().unwrap_or_default();
            mine.items = own.iter().cloned().chain(mine.items).collect();
            mine.total = mine.total.map(|total| total + 1);
            snapshot.my_packs.insert(PackKind::Sticker, mine);
        }

        snapshot.packs.insert(
            scene.panda_pack.id.clone(),
            RiseStickerPackContent {
                pack: Some(scene.panda_pack.clone()),
                stickers: scene.panda_stickers.clone(),
            },
        );
        if let Some(own_pack) = scene.own_pack.as_ref() {
            snapshot.packs.insert(
                own_pack.id.clone(),
                RiseStickerPackContent {
                    pack: Some(own_pack.clone()),
                    stickers: scene.own_stickers.clone(),
                },
            );
        }

        let kept: Vec<StickerItem> = snapshot
            .favourites
            .iter()
            .filter(|sticker| scene.favourites.get(&sticker.id) != Some(&false))
            .cloned()
            .collect();
        let added: Vec<StickerItem> = scene
            .starred
            .iter()
            .filter(|sticker| scene.favourites.get(&sticker.id) == Some(&true) && !kept.contains(*sticker))
            .cloned()
            .collect();
        snapshot.favourites = added.into_iter().chain(kept).collect();
        snapshot
    }

    pub fn toggle_practice_favourite(&mut self, sticker: &StickerItem, is_favourite: bool, cx: &mut Context<Self>) {
        if !self.owns_stickers_scene() {
            return;
        }
        let Some(scene) = self.stickers_scene.as_mut() else {
            return;
        };
        scene.favourites.insert(sticker.id.clone(), !is_favourite);
        if !is_favourite {
            scene.starred.retain(|starred| starred.id != sticker.id);
            scene.starred.insert(0, sticker.clone());
        }
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::StickerFavouriteChanged);
    }

    pub fn create_practice_pack(&mut self, title: String, cx: &mut Context<Self>) -> Option<StickerPack> {
        if !self.owns_stickers_scene() {
            return None;
        }
        let stamp = (RiseClock::now().time_interval_since_1970() * 1000.0) as i64;
        let pack = StickerPack::new(
            Self::own_sticker_pack_id(),
            title,
            String::new(),
            true,
            true,
            stamp,
            stamp,
            false,
            true,
            PackKind::Sticker,
            String::new(),
            StickerAssetType::StaticImage,
            false,
            0,
            None,
        );
        let scene = self.stickers_scene.as_mut()?;
        scene.own_pack = Some(pack.clone());
        scene.own_stickers = Vec::new();
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::StickerPackCreated);
        Some(pack)
    }

    /// gallery picture becomes a sticker of the practice pack
    /// copied so picker cleanup cant remove it mid lesson
    pub fn add_practice_sticker(&mut self, source: &Url, emojis: &[String], cx: &mut Context<Self>) {
        if !self.owns_stickers_scene() {
            return;
        }
        let Some(scene) = self.stickers_scene.as_mut() else {
            return;
        };
        let Some(pack_id) = scene.own_pack.as_ref().map(|pack| pack.id.clone()) else {
            return;
        };
        let index = scene.own_stickers.len() as i64 + 1;
        let extension = source.path_extension();
        let extension = if extension.is_empty() { Self::FALLBACK_STICKER_EXTENSION.to_owned() } else { extension };
        let target_path = temporary_directory()
            .join(format!("guide_sticker_own_{index}_{}", Uuid::new_v4().uuid_string()))
            .with_extension(extension);
        let copied = source
            .to_file_path()
            .is_some_and(|source_path| FileManager.copy_item(&source_path, &target_path).is_ok());
        let target = if copied { Url::from_file_path(&target_path) } else { None };
        let file = match target {
            Some(target) => {
                scene.files.push(target.clone());
                target
            }
            None => source.clone(),
        };
        let sticker = Self::sticker_item(
            format!("{pack_id}_{index}"),
            pack_id,
            &file,
            emojis.first().cloned().unwrap_or_else(|| Self::FALLBACK_STICKER_EMOJI.to_owned()),
            index - 1,
            (RiseClock::now().time_interval_since_1970() * 1000.0) as i64,
        );
        scene.own_stickers.push(sticker);
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::StickerAdded);
    }

    fn sticker_item(id: String, pack_id: String, file: &Url, emoji: String, position: i64, stamp: i64) -> StickerItem {
        StickerItem::new(
            id,
            pack_id,
            file.absolute_string().to_owned(),
            None,
            Self::STICKER_SIDE,
            Self::STICKER_SIDE,
            None,
            position,
            stamp,
            vec![emoji],
            StickerAssetType::StaticImage,
            Some(RiseUploadFormat::IMAGE_PNG.mime.to_owned()),
            None,
        )
    }
}

/// one drawn sticker, its png file and the emoji it shows
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxStickerArtFile {
    pub url: Url,
    pub emoji: String,
}

pub enum GuideSandboxStickerArt {}

impl GuideSandboxStickerArt {
    pub const EMOJIS: [&'static str; 3] = ["\u{1F43C}", "\u{1F389}", "\u{1F44D}"];

    const SIDE: u32 = 512;
    const PLATE_INSET: f64 = 24.0;
    const PLATE_ALPHA: f64 = 0.92;
    const EMOJI_POINT_SIZE: f32 = 300.0;

    pub fn render() -> Vec<GuideSandboxStickerArtFile> {
        Self::EMOJIS
            .iter()
            .enumerate()
            .filter_map(|(index, emoji)| {
                let mut image = Self::plate();
                if let Ok(raster) = render_emoji(emoji, Self::EMOJI_POINT_SIZE) {
                    Self::draw_centered(&mut image, &raster);
                }
                let path = temporary_directory()
                    .join(format!("guide_sticker_{}_{}.png", index + 1, Uuid::new_v4().uuid_string()));
                if !Self::write_png(&image, &path) {
                    return None;
                }
                Some(GuideSandboxStickerArtFile {
                    url: Url::from_file_path(&path)?,
                    emoji: (*emoji).to_owned(),
                })
            })
            .collect()
    }

    /// white circle plate filled at 0.92 alpha, inset 24 from the bounds
    fn plate() -> RgbaImage {
        let side = f64::from(Self::SIDE);
        let center = side / 2.0;
        let radius = center - Self::PLATE_INSET;
        let alpha = (Self::PLATE_ALPHA * 255.0).round() as u8;
        RgbaImage::from_fn(Self::SIDE, Self::SIDE, |x, y| {
            let dx = f64::from(x) + 0.5 - center;
            let dy = f64::from(y) + 0.5 - center;
            if dx * dx + dy * dy <= radius * radius {
                Rgba([255, 255, 255, alpha])
            } else {
                Rgba([0, 0, 0, 0])
            }
        })
    }

    /// draws the emoji pixels over the plate with source over, centred in the measured box
    fn draw_centered(image: &mut RgbaImage, raster: &EmojiRaster) {
        let origin_x = (i64::from(Self::SIDE) - i64::from(raster.width)) / 2;
        let origin_y = (i64::from(Self::SIDE) - i64::from(raster.height)) / 2;
        for y in 0..raster.height {
            for x in 0..raster.width {
                let target_x = origin_x + i64::from(x);
                let target_y = origin_y + i64::from(y);
                let (Ok(target_x), Ok(target_y)) = (u32::try_from(target_x), u32::try_from(target_y)) else {
                    continue;
                };
                if target_x >= Self::SIDE || target_y >= Self::SIDE {
                    continue;
                }
                let offset = ((y * raster.width + x) * 4) as usize;
                let Some(source) = raster.rgba.get(offset..offset + 4) else {
                    continue;
                };
                let source_alpha = f64::from(source[3]) / 255.0;
                if source_alpha <= 0.0 {
                    continue;
                }
                let destination = image.get_pixel_mut(target_x, target_y);
                let destination_alpha = f64::from(destination[3]) / 255.0;
                let out_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
                for channel in 0..3 {
                    let source_value = f64::from(source[channel]) / 255.0;
                    let destination_value = f64::from(destination[channel]) / 255.0;
                    let blended = (source_value * source_alpha
                        + destination_value * destination_alpha * (1.0 - source_alpha))
                        / out_alpha;
                    destination[channel] = (blended.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                destination[3] = (out_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }

    fn write_png(image: &RgbaImage, path: &Path) -> bool {
        image.save_with_format(path, image::ImageFormat::Png).is_ok()
    }
}

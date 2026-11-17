use std::io::BufWriter;
use std::path::PathBuf;

use gpui::{App, AppContext, Context};
use image::codecs::jpeg::JpegEncoder;
use image::{Rgb, RgbImage};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatWallpaperCatalogEntry, ChatWallpaperCatalogState, ChatWallpaperPlacement,
};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{RiseChatWallpaperDraft, RiseChatWallpaperUpload};

/// fake chat wallpaper, three drawn pictures plus the pinned one and the show wallpaper switch
/// no server has a catalogue for the fake chat
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxWallpaperScene {
    pub entries: Vec<ChatWallpaperCatalogEntry>,
    pub is_ready: bool,
    pub placement: Option<ChatWallpaperPlacement>,
    pub shown: bool,
    pub adopted_files: Vec<Url>,
}

impl Default for GuideSandboxWallpaperScene {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            is_ready: false,
            placement: None,
            shown: true,
            adopted_files: Vec::new(),
        }
    }
}

impl GuideSandbox {
    /// true if this is the fake chat of an armed wallpaper lesson
    pub fn owns_wallpaper(&self, id: Option<&str>) -> bool {
        self.owns_chat_id(id) && self.wallpaper.is_some()
    }

    pub fn arm_wallpaper(&mut self, cx: &mut Context<Self>) {
        self.wallpaper = Some(GuideSandboxWallpaperScene::default());
        cx.notify();
        let render = cx.background_spawn(async move { GuideSandboxWallpaperArt::render() });
        let task = cx.spawn(async move |this, cx| {
            let files = render.await;
            this.update(cx, |sandbox, cx| {
                if !sandbox.is_armed || sandbox.wallpaper.is_none() {
                    return;
                }
                let stamp = (Self::scene_stamp() * 1000.0) as i64;
                let entries = files
                    .iter()
                    .enumerate()
                    .map(|(index, url)| ChatWallpaperCatalogEntry {
                        id: format!("{}_{}", Self::wallpaper_scene_id(), index + 1),
                        chat_id: Self::chat_id().to_owned(),
                        topic_id: String::new(),
                        image_url: url.absolute_string().to_owned(),
                        preview_url: url.absolute_string().to_owned(),
                        look: ChatWallpaperLook::DEFAULT_LOOK,
                        added_by_user_id: Self::panda_user_id().to_owned(),
                        created_at_ms: stamp - index as i64,
                    })
                    .collect();
                if let Some(scene) = sandbox.wallpaper.as_mut() {
                    scene.entries = entries;
                    scene.is_ready = true;
                }
                cx.notify();
            })
            .ok();
        });
        self.scene_tasks.push(task);
    }

    /// catalogue the picker page shows, none for any other chat
    pub fn wallpaper_catalog(&self, chat_id: &str) -> Option<ChatWallpaperCatalogState> {
        if !self.owns_wallpaper(Some(chat_id)) {
            return None;
        }
        let scene = self.wallpaper.as_ref()?;
        Some(ChatWallpaperCatalogState {
            entries: scene.entries.clone(),
            is_loading: !scene.is_ready,
            is_loaded: scene.is_ready,
            ..ChatWallpaperCatalogState::default()
        })
    }

    /// none for any chat but the lesson's, the account's own rule decides those
    pub fn shows_chat_wallpaper(&self, chat_id: &str) -> Option<bool> {
        if !self.owns_wallpaper(Some(chat_id)) {
            return None;
        }
        self.wallpaper.as_ref().map(|scene| scene.shown)
    }

    pub fn set_shows_chat_wallpaper(&mut self, shows: bool, cx: &mut Context<Self>) {
        if let Some(scene) = self.wallpaper.as_mut() {
            scene.shown = shows;
            cx.notify();
        }
    }

    pub fn pin_wallpaper(&mut self, entry: &ChatWallpaperCatalogEntry, cx: &mut Context<Self>) {
        if self.wallpaper.is_none() {
            return;
        }
        let set_by_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let updated_at_ms = (RiseClock::now().time_interval_since_1970() * 1000.0) as i64;
        if let Some(scene) = self.wallpaper.as_mut() {
            scene.placement = Some(ChatWallpaperPlacement {
                id: entry.id.clone(),
                image_url: entry.image_url.clone(),
                preview_url: entry.preview_url.clone(),
                look: entry.look,
                set_by_user_id,
                updated_at_ms,
            });
            scene.shown = true;
        }
        cx.notify();
        self.republish_conversation_detail(cx);
        GuideSignalBus::emit(RiseGuideSignal::ChatWallpaperSet);
    }

    pub fn unpin_wallpaper(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.wallpaper.as_mut() else {
            return;
        };
        scene.placement = None;
        cx.notify();
        self.republish_conversation_detail(cx);
    }

    pub fn adopt_wallpaper(&mut self, draft: RiseChatWallpaperDraft, look: ChatWallpaperLook, cx: &mut Context<Self>) {
        let Some(adopted) = self.wallpaper.as_ref().map(|scene| scene.adopted_files.len()) else {
            return;
        };
        let added_by_user_id =
            AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let entry = ChatWallpaperCatalogEntry {
            id: format!("{}_own_{}", Self::wallpaper_scene_id(), adopted + 1),
            chat_id: Self::chat_id().to_owned(),
            topic_id: String::new(),
            image_url: draft.local_url.absolute_string().to_owned(),
            preview_url: draft.local_url.absolute_string().to_owned(),
            look,
            added_by_user_id,
            created_at_ms: (RiseClock::now().time_interval_since_1970() * 1000.0) as i64,
        };
        if let Some(scene) = self.wallpaper.as_mut() {
            scene.adopted_files.push(draft.local_url);
            scene.entries.insert(0, entry.clone());
        }
        self.pin_wallpaper(&entry, cx);
    }

    pub fn withdraw_wallpaper(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.wallpaper.as_ref() else {
            return;
        };
        let adopted_files = scene.adopted_files.clone();
        cx.background_spawn(async move {
            for file in &adopted_files {
                RiseChatWallpaperUpload::discard_staged_file(file);
            }
            GuideSandboxWallpaperArt::discard();
        })
        .detach();
    }

    /// the chat row again with the pinned picture
    /// so profile and chat screen read it like a real chat's wallpaper
    fn republish_conversation_detail(&self, cx: &mut App) {
        if !self.is_armed {
            return;
        }
        let chat = self.chat().to_wire_dto(None);
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            let Some(account_id) = presentation.account_id().filter(|id| !id.is_empty()).map(str::to_owned) else {
                return;
            };
            presentation.publish_chat_detail(chat, &account_id, Some(false), cx);
        });
    }
}

/// srgb color with components in 0 to 1
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideSandboxArtColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

impl GuideSandboxArtColor {
    pub const fn new(red: f64, green: f64, blue: f64) -> Self {
        Self { red, green, blue }
    }
}

/// three portrait pictures drawn once per lesson
/// warm evening, sea and forest as soft circles over a gradient
pub enum GuideSandboxWallpaperArt {}

impl GuideSandboxWallpaperArt {
    const WIDTH: u32 = 720;
    const HEIGHT: u32 = 1280;
    const JPEG_QUALITY: u8 = 86;

    const PALETTES: [[GuideSandboxArtColor; 3]; 3] = [
        [
            GuideSandboxArtColor::new(0.99, 0.60, 0.47),
            GuideSandboxArtColor::new(0.93, 0.36, 0.53),
            GuideSandboxArtColor::new(0.42, 0.25, 0.55),
        ],
        [
            GuideSandboxArtColor::new(0.16, 0.72, 0.86),
            GuideSandboxArtColor::new(0.13, 0.40, 0.75),
            GuideSandboxArtColor::new(0.10, 0.16, 0.42),
        ],
        [
            GuideSandboxArtColor::new(0.62, 0.84, 0.47),
            GuideSandboxArtColor::new(0.24, 0.58, 0.42),
            GuideSandboxArtColor::new(0.07, 0.27, 0.29),
        ],
    ];

    fn folder() -> PathBuf {
        temporary_directory().join("guide-sandbox-wallpapers")
    }

    pub fn render() -> Vec<Url> {
        let file_manager = FileManager;
        let folder = Self::folder();
        file_manager.create_directory(&folder, true).ok();
        let mut urls = Vec::new();
        for (index, palette) in Self::PALETTES.iter().enumerate() {
            let path = folder.join(format!("wallpaper-{}.jpg", index + 1));
            if !file_manager.file_exists(&path) {
                let image = Self::draw(palette, index as u64, Self::WIDTH, Self::HEIGHT);
                if !Self::write_jpeg(&image, &path) {
                    continue;
                }
            }
            if let Some(url) = Url::from_file_path(&path) {
                urls.push(url);
            }
        }
        urls
    }

    pub fn discard() {
        FileManager.remove_item(&Self::folder()).ok();
    }

    pub fn write_jpeg(image: &RgbImage, path: &std::path::Path) -> bool {
        let staging = path.with_extension("jpg.partial");
        let written = std::fs::File::create(&staging)
            .ok()
            .map(BufWriter::new)
            .and_then(|mut writer| {
                JpegEncoder::new_with_quality(&mut writer, Self::JPEG_QUALITY).encode_image(image).ok()
            })
            .is_some();
        if !written || std::fs::rename(&staging, path).is_err() {
            FileManager.remove_item(&staging).ok();
            return false;
        }
        true
    }

    /// gradient under soft circles, the stories scene draws its pictures the same way
    pub fn draw(palette: &[GuideSandboxArtColor; 3], seed: u64, width: u32, height: u32) -> RgbImage {
        let size_width = f64::from(width);
        let size_height = f64::from(height);
        let mut pixels = vec![[0.0_f64; 3]; (width as usize) * (height as usize)];

        let start = (size_width * 0.1, 0.0);
        let end = (size_width * 0.9, size_height);
        let axis = (end.0 - start.0, end.1 - start.1);
        let axis_length_squared = axis.0 * axis.0 + axis.1 * axis.1;
        for y in 0..height {
            for x in 0..width {
                let point = (f64::from(x) + 0.5 - start.0, f64::from(y) + 0.5 - start.1);
                let t = ((point.0 * axis.0 + point.1 * axis.1) / axis_length_squared).clamp(0.0, 1.0);
                let (from, to, local) = if t <= 0.55 {
                    (palette[0], palette[1], t / 0.55)
                } else {
                    (palette[1], palette[2], (t - 0.55) / 0.45)
                };
                pixels[(y as usize) * (width as usize) + x as usize] = [
                    from.red + (to.red - from.red) * local,
                    from.green + (to.green - from.green) * local,
                    from.blue + (to.blue - from.blue) * local,
                ];
            }
        }

        let mut state = (seed + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut next = || -> f64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 10_000) as f64 / 10_000.0
        };
        for _ in 0..14 {
            let radius = 40.0 + next() * 150.0;
            let center = (next() * size_width, next() * size_height);
            let alpha = 0.06 + next() * 0.12;
            let min_x = (center.0 - radius).floor().max(0.0) as u32;
            let max_x = (center.0 + radius).ceil().min(size_width) as u32;
            let min_y = (center.1 - radius).floor().max(0.0) as u32;
            let max_y = (center.1 + radius).ceil().min(size_height) as u32;
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let dx = f64::from(x) + 0.5 - center.0;
                    let dy = f64::from(y) + 0.5 - center.1;
                    if dx * dx + dy * dy > radius * radius {
                        continue;
                    }
                    let pixel = &mut pixels[(y as usize) * (width as usize) + x as usize];
                    for channel in pixel.iter_mut() {
                        *channel = *channel * (1.0 - alpha) + alpha;
                    }
                }
            }
        }

        RgbImage::from_fn(width, height, |x, y| {
            let pixel = pixels[(y as usize) * (width as usize) + x as usize];
            Rgb(pixel.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8))
        })
    }
}

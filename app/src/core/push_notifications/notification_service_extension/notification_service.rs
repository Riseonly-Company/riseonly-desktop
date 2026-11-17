use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use futures::future::{Either, select};
use gpui::{App, AppContext, AsyncApp};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use rise_canon::RisePushType;
use rise_canon::rise_key::{push, push_route};
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_notify::PUSH_CATEGORIES;
use rise_platform::AppPaths;
use rise_platform::emoji_raster::{EmojiRaster, TextWeight, render_text};
use rise_platform::notifications::{LocalNotification, NotificationTag, attachment_fallback_support};
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::rich_text::RichTextMarkup;
use serde_json::{Map, Value};

use super::api_client::APIClient;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;

const LOG_NAME: &str = "NotificationService";

const SERVICE_EXTENSION_TIME_LIMIT: Duration = Duration::from_secs(30);

const AVATAR_CACHE_FOLDER: &str = "NotificationAvatars";
const MAXIMUM_CACHED_AVATAR_FILES: usize = 256;

const LETTER_AVATAR_SIDE: u32 = 256;
const LETTER_FONT_FRACTION: f32 = 0.46;
const NOTIFICATION_AVATAR_MAXIMUM_SIDE: u32 = 512;
const AVATAR_JPEG_QUALITY: u8 = 85;

const COMPOSITE_SIDE: u32 = 1024;
const COMPOSITE_BADGE_FRACTION: f32 = 0.28;
const COMPOSITE_PADDING_FRACTION: f32 = 0.05;
const COMPOSITE_ICON_INSET_FRACTION: f32 = 0.15;
const COMPOSITE_SHADOW_OFFSET_Y: f32 = 2.;
const COMPOSITE_SHADOW_BLUR: f32 = 6.;
const COMPOSITE_SHADOW_ALPHA: f32 = 0.25;
const COMPOSITE_JPEG_QUALITY: u8 = 90;

const APP_ICON_PNG: &[u8] = include_bytes!("../../../../resources/macos/AppIcon.png");

static AVATAR_CACHE_DIRECTORY: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::compiled().canon())
        .ok()
        .map(|paths| paths.cache_dir().join(AVATAR_CACHE_FOLDER))
});

/// notification request with the tag the manager groups banners by
#[derive(Clone, Debug)]
pub struct NotificationRequest {
    pub tag: NotificationTag,
    pub title: String,
    pub body: String,
    pub user_info: Map<String, Value>,
}

/// notification content as deliver takes it
#[derive(Clone, Debug)]
pub struct NotificationContent {
    pub notification: LocalNotification,
    pub category: Option<String>,
    pub user_info: Map<String, Value>,
}

/// arguments of the communication notification that dont change after the payload is read
#[derive(Clone, Debug, PartialEq, Eq)]
struct MessageNotification {
    chat_id: String,
    sender_id: String,
    sender_name: String,
    avatar_url: Option<String>,
    author: Option<NotificationAuthor>,
}

/// member who wrote a group message
#[derive(Clone, Debug, PartialEq, Eq)]
struct NotificationAuthor {
    id: String,
    name: String,
    avatar_url: Option<String>,
}

/// files of one sender in the cache
#[derive(Clone, Debug, PartialEq, Eq)]
struct AvatarFiles {
    avatar: PathBuf,
    attachment: Option<PathBuf>,
}

fn string_in<'a>(user_info: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    user_info.get(key).and_then(Value::as_str)
}

pub struct NotificationService;

impl NotificationService {
    /// a category missing from canon must crash here
    /// better than shipping a button less notification nobody notices
    fn message_category_identifier() -> &'static str {
        PUSH_CATEGORIES
            .iter()
            .find(|spec| spec.name == "message")
            .map(|spec| spec.identifier)
            .expect("Canon has no push category `message`")
    }

    pub fn did_receive(
        request: NotificationRequest,
        content_handler: impl FnOnce(NotificationContent, &mut App) + 'static,
        cx: &mut App,
    ) {
        let notification_type = string_in(&request.user_info, push_route::TYPE).unwrap_or_default();
        if notification_type == RisePushType::NEW_MESSAGE {
            let message = MessageNotification::read(&request, None);
            Self::handle_message_notification(request, message, content_handler, cx);
        } else if notification_type == RisePushType::REACTION_ADDED {
            let message = MessageNotification::read(&request, Some(&request.title));
            Self::handle_message_notification(request, message, content_handler, cx);
        } else {
            content_handler(Self::plain_content(request), cx);
        }
    }

    fn plain_content(request: NotificationRequest) -> NotificationContent {
        NotificationContent {
            notification: LocalNotification::new(request.tag, request.title, request.body),
            category: None,
            user_info: request.user_info,
        }
    }

    fn handle_message_notification(
        request: NotificationRequest,
        message: MessageNotification,
        content_handler: impl FnOnce(NotificationContent, &mut App) + 'static,
        cx: &mut App,
    ) {
        cx.spawn(async move |cx| {
            let timeout = cx.background_executor().timer(SERVICE_EXTENSION_TIME_LIMIT);
            let mut work_cx = cx.clone();
            let work = Self::prepared_avatars(&message, &mut work_cx);
            let (avatar, author_avatar) = match select(Box::pin(work), timeout).await {
                Either::Left((avatars, _)) => avatars,
                Either::Right(_) if message.needs_any_download() => {
                    let owner = message.sender_id.clone();
                    let name = message.sender_name.clone();
                    let letter = cx
                        .background_spawn(async move { Self::letter_avatar_files(&owner, &name) })
                        .await;
                    (letter, None)
                }
                Either::Right(_) => (None, None),
            };
            let author_avatar = match (&message.author, author_avatar) {
                (Some(author), None) => {
                    let (owner, name) = (author.id.clone(), author.name.clone());
                    cx.background_spawn(async move { Self::letter_avatar_files(&owner, &name) }).await
                }
                (_, files) => files,
            };
            let content = Self::build_communication_notification(request, &message, avatar, author_avatar);
            cx.update(|cx| content_handler(content, cx));
        })
        .detach();
    }

    async fn prepared_avatars(
        message: &MessageNotification,
        cx: &mut AsyncApp,
    ) -> (Option<AvatarFiles>, Option<AvatarFiles>) {
        let avatar = Self::load_avatar(message.avatar_url.as_deref(), &message.sender_id, &message.sender_name, cx).await;
        let Some(author) = &message.author else {
            return (avatar, None);
        };
        let author_avatar = Self::load_avatar(author.avatar_url.as_deref(), &author.id, &author.name, cx).await;
        (avatar, author_avatar)
    }

    /// true for a photo url, a missing url and a server letter picture are drawn locally
    fn needs_download(url: Option<&str>) -> bool {
        url.filter(|url| !url.is_empty())
            .is_some_and(|url| AvatarPalette::generated_avatar_owner_id(Some(url)).is_none())
    }

    /// loads the avatar, a server letter picture drawn locally or else the photo or the owner letter
    /// hidden photo has no url and a failed download gets the letter too
    async fn load_avatar(url: Option<&str>, owner: &str, name: &str, cx: &mut AsyncApp) -> Option<AvatarFiles> {
        if let Some(letter_owner) = Self::letter_owner(url, owner) {
            return Self::letter_files_in_background(letter_owner, name.to_owned(), cx).await;
        }
        let avatar_url = url.filter(|url| !url.is_empty())?.to_owned();
        let cache_key = sha256_hex(avatar_url.as_bytes());
        let Some(data) = APIClient::download_image(&avatar_url, cx).await else {
            Self::log("avatar download failed", cx);
            if owner.is_empty() {
                return None;
            }
            return Self::letter_files_in_background(owner.to_owned(), name.to_owned(), cx).await;
        };
        let files = cx
            .background_spawn(async move { Self::photo_avatar_files(&cache_key, &data) })
            .await;
        if files.is_none() {
            Self::log("could not decode the avatar image data", cx);
        }
        files
    }

    async fn letter_files_in_background(owner: String, name: String, cx: &mut AsyncApp) -> Option<AvatarFiles> {
        let files = cx
            .background_spawn(async move { Self::letter_avatar_files(&owner, &name) })
            .await;
        if files.is_none() {
            Self::log("could not store the letter avatar", cx);
        }
        files
    }

    /// whose letter to draw, none for a photo to download or no picture
    /// an old server letter has a random color and gets redrawn in the canon one
    fn letter_owner(url: Option<&str>, owner: &str) -> Option<String> {
        match url.filter(|url| !url.is_empty()) {
            Some(url) => AvatarPalette::generated_avatar_owner_id(Some(url)),
            None => (!owner.is_empty()).then(|| owner.to_owned()),
        }
    }

    fn log(text: &'static str, cx: &mut AsyncApp) {
        cx.update(|cx| Sai::log(LOG_NAME, move || text, SaiLogKind::Default, cx));
    }

    /// builds the communication notification
    /// the seam makes the system conversation look and falls back to the attachment where refused
    fn build_communication_notification(
        request: NotificationRequest,
        message: &MessageNotification,
        avatar: Option<AvatarFiles>,
        author_avatar: Option<AvatarFiles>,
    ) -> NotificationContent {
        let target_title = message.target_title(&request.title);
        let body = RichTextMarkup::preview_text(&request.body);
        let (sender, picture) = match &message.author {
            Some(author) => {
                let name = if author.name.is_empty() { target_title.clone() } else { author.name.clone() };
                (name, author_avatar)
            }
            None => {
                let name =
                    if message.sender_name.is_empty() { target_title.clone() } else { message.sender_name.clone() };
                (name, avatar)
            }
        };
        let mut notification = LocalNotification::new(request.tag, target_title, body)
            .with_thread(message.conversation_id())
            .with_sender(sender);
        if let Some(files) = picture {
            notification = notification.with_avatar(files.avatar);
            if let Some(attachment) = files.attachment {
                notification = notification.with_attachment(attachment);
            }
        }
        NotificationContent {
            notification,
            category: Some(Self::message_category_identifier().to_owned()),
            user_info: request.user_info,
        }
    }

    fn string_value(value: Option<&Value>) -> Option<String> {
        let raw = match value? {
            Value::String(string) => string.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => (if *flag { "1" } else { "0" }).to_owned(),
            _ => return None,
        };
        let normalized = trimming_whitespaces_and_newlines(&raw);
        (!normalized.is_empty()).then(|| normalized.to_owned())
    }

    fn letter_avatar_files(owner: &str, name: &str) -> Option<AvatarFiles> {
        let cache_key = sha256_hex(format!("{owner}\n{name}").as_bytes());
        let directory = (*AVATAR_CACHE_DIRECTORY).clone()?;
        let image = Self::letter_avatar_image(owner, name);
        let mut png = Vec::new();
        image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).ok()?;
        let avatar = Self::store(&directory, &format!("{cache_key}.png"), &png)?;
        let attachment = Self::attachment_file(&directory, &cache_key, &DynamicImage::ImageRgb8(image));
        Some(AvatarFiles { avatar, attachment })
    }

    fn photo_avatar_files(cache_key: &str, data: &[u8]) -> Option<AvatarFiles> {
        let directory = (*AVATAR_CACHE_DIRECTORY).clone()?;
        let image = image::load_from_memory(data).ok()?;
        let jpeg = Self::encode_jpeg(&Self::resize_for_notification_avatar(&image).to_rgb8(), AVATAR_JPEG_QUALITY)?;
        let avatar = Self::store(&directory, &format!("{cache_key}.jpg"), &jpeg)?;
        let attachment = Self::attachment_file(&directory, cache_key, &image);
        Some(AvatarFiles { avatar, attachment })
    }

    /// attaches the composited avatar
    /// only where a route can show it, no icon just the avatar
    fn attachment_file(directory: &Path, cache_key: &str, avatar: &DynamicImage) -> Option<PathBuf> {
        if !attachment_fallback_support().is_supported() {
            return None;
        }
        let composited = Self::composite_avatar_with_app_icon(avatar).unwrap_or_else(|| avatar.to_rgb8());
        let jpeg = Self::encode_jpeg(&composited, COMPOSITE_JPEG_QUALITY)?;
        Self::store(directory, &format!("{cache_key}-attachment.jpg"), &jpeg)
    }

    /// the same letter the app shows for that person or chat in the canon color of the owner id
    /// so one person has one color in app and notification
    fn letter_avatar_image(owner: &str, name: &str) -> RgbImage {
        let (background, letter_color, letter) = AvatarPalette::letter_avatar(owner, name);
        let mut image = RgbImage::from_pixel(
            LETTER_AVATAR_SIDE,
            LETTER_AVATAR_SIDE,
            Rgb([background.red, background.green, background.blue]),
        );
        let point_size = LETTER_AVATAR_SIDE as f32 * LETTER_FONT_FRACTION;
        if let Ok(raster) = render_text(&letter, point_size, TextWeight::Semibold) {
            Self::draw_centered(
                &mut image,
                &raster,
                [letter_color.red, letter_color.green, letter_color.blue],
            );
        }
        image
    }

    /// draws the text centered
    /// the seam draws in one neutral ink so a letter takes
    /// the canon color but a color emoji keeps its own
    fn draw_centered(image: &mut RgbImage, raster: &EmojiRaster, ink: [u8; 3]) {
        let is_neutral = raster.rgba.chunks_exact(4).all(|pixel| pixel[0] == pixel[1] && pixel[1] == pixel[2]);
        let left = (i64::from(image.width()) - i64::from(raster.width)) / 2;
        let top = (i64::from(image.height()) - i64::from(raster.height)) / 2;
        for (index, pixel) in raster.rgba.chunks_exact(4).enumerate() {
            let column = (index as u32 % raster.width.max(1)) as i64 + left;
            let row = (index as u32 / raster.width.max(1)) as i64 + top;
            let (Ok(column), Ok(row)) = (u32::try_from(column), u32::try_from(row)) else {
                continue;
            };
            if column >= image.width() || row >= image.height() {
                continue;
            }
            let color = if is_neutral { ink } else { [pixel[0], pixel[1], pixel[2]] };
            let alpha = f32::from(pixel[3]) / 255.;
            let target = image.get_pixel_mut(column, row);
            for (under, over) in target.0.iter_mut().zip(color) {
                *under = Self::mix(*under, over, alpha);
            }
        }
    }

    fn mix(under: u8, over: u8, alpha: f32) -> u8 {
        (f32::from(under) * (1. - alpha) + f32::from(over) * alpha).round() as u8
    }

    fn resize_for_notification_avatar(image: &DynamicImage) -> DynamicImage {
        let maximum = NOTIFICATION_AVATAR_MAXIMUM_SIDE;
        if image.width() > maximum || image.height() > maximum {
            image.resize(maximum, maximum, FilterType::Lanczos3)
        } else {
            image.clone()
        }
    }

    fn encode_jpeg(image: &RgbImage, quality: u8) -> Option<Vec<u8>> {
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, quality).encode_image(image).ok()?;
        Some(jpeg)
    }

    fn composite_avatar_with_app_icon(avatar: &DynamicImage) -> Option<RgbImage> {
        let app_icon = image::load_from_memory(APP_ICON_PNG).ok()?;
        Some(Self::composite(avatar, &app_icon))
    }

    fn composite(avatar: &DynamicImage, app_icon: &DynamicImage) -> RgbImage {
        let side = COMPOSITE_SIDE;
        let mut canvas: RgbaImage = imageops::resize(&avatar.to_rgba8(), side, side, FilterType::Lanczos3);
        let side_length = side as f32;
        let badge_size = side_length * COMPOSITE_BADGE_FRACTION;
        let badge_padding = side_length * COMPOSITE_PADDING_FRACTION;
        let badge_left = side_length - badge_size - badge_padding;
        let badge_top = side_length - badge_size - badge_padding;
        let radius = badge_size / 2.;
        let center_x = badge_left + radius;
        let center_y = badge_top + radius;

        let reach = radius + COMPOSITE_SHADOW_OFFSET_Y + COMPOSITE_SHADOW_BLUR;
        let first_column = (center_x - reach).floor().max(0.) as u32;
        let first_row = (center_y - reach).floor().max(0.) as u32;
        let last_column = (center_x + reach).ceil().min(side_length) as u32;
        let last_row = (center_y + reach).ceil().min(side_length) as u32;
        for row in first_row..last_row {
            for column in first_column..last_column {
                let x = column as f32 + 0.5 - center_x;
                let y = row as f32 + 0.5 - center_y;
                let shadow_edge = (x * x + (y - COMPOSITE_SHADOW_OFFSET_Y).powi(2)).sqrt() - radius;
                let shadow = COMPOSITE_SHADOW_ALPHA
                    * (0.5 - shadow_edge / (2. * COMPOSITE_SHADOW_BLUR)).clamp(0., 1.);
                let circle = (0.5 - ((x * x + y * y).sqrt() - radius)).clamp(0., 1.);
                let pixel: &mut Rgba<u8> = canvas.get_pixel_mut(column, row);
                for value in pixel.0.iter_mut().take(3) {
                    *value = Self::mix(Self::mix(*value, 0, shadow), 255, circle);
                }
                pixel.0[3] = 255;
            }
        }

        let icon_padding = badge_size * COMPOSITE_ICON_INSET_FRACTION;
        let icon_side = (badge_size - 2. * icon_padding).round() as u32;
        let icon = imageops::resize(&app_icon.to_rgba8(), icon_side, icon_side, FilterType::Lanczos3);
        imageops::overlay(
            &mut canvas,
            &icon,
            i64::from((badge_left + icon_padding).round() as u32),
            i64::from((badge_top + icon_padding).round() as u32),
        );
        DynamicImage::ImageRgba8(canvas).to_rgb8()
    }

    /// writes through a temp name so a banner never reads half a file
    /// a file with that name is the same picture since names come from the url or owner
    fn store(directory: &Path, file_name: &str, data: &[u8]) -> Option<PathBuf> {
        let target = directory.join(file_name);
        if target.exists() {
            return Some(target);
        }
        std::fs::create_dir_all(directory).ok()?;
        let temporary = directory.join(format!("{}.tmp", Uuid::new_v4().uuid_string()));
        std::fs::write(&temporary, data).ok()?;
        if std::fs::rename(&temporary, &target).is_err() {
            let _ = std::fs::remove_file(&temporary);
            return None;
        }
        Self::trim(directory, MAXIMUM_CACHED_AVATAR_FILES);
        Some(target)
    }

    fn trim(directory: &Path, maximum: usize) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        if files.len() <= maximum {
            return;
        }
        files.sort();
        let excess = files.len() - maximum;
        for (_, path) in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(path);
        }
    }

    pub fn clear_avatar_cache() {
        if let Some(directory) = AVATAR_CACHE_DIRECTORY.as_ref() {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

impl MessageNotification {
    /// reads the message notification payload
    /// display_name is the name a reaction takes from the alert title
    fn read(request: &NotificationRequest, display_name: Option<&str>) -> Self {
        let user_info = &request.user_info;
        let chat_id = string_in(user_info, push_route::CHAT_ID).unwrap_or_default().to_owned();
        let chat_type = string_in(user_info, push::CHAT_TYPE).unwrap_or_default().to_uppercase();
        let is_group_or_channel = chat_type == "GROUP" || chat_type == "CHANNEL";
        let sender_id = if is_group_or_channel {
            chat_id.clone()
        } else {
            string_in(user_info, push_route::SENDER_ID).unwrap_or_default().to_owned()
        };
        let actor_name = display_name
            .map(trimming_whitespaces_and_newlines)
            .filter(|name| !name.is_empty())
            .or_else(|| string_in(user_info, push::SENDER_NAME))
            .unwrap_or_default()
            .to_owned();
        let sender_name = if is_group_or_channel {
            NotificationService::string_value(user_info.get(push::CHAT_TITLE))
                .unwrap_or_else(|| request.title.clone())
        } else {
            actor_name.clone()
        };
        let avatar_url = string_in(user_info, push::AVATAR_URL).map(str::to_owned);
        let author = Self::group_author(user_info, &chat_type, display_name, &actor_name, &sender_name);
        Self { chat_id, sender_id, sender_name, avatar_url, author }
    }

    fn group_author(
        user_info: &Map<String, Value>,
        chat_type: &str,
        display_name: Option<&str>,
        actor_name: &str,
        sender_name: &str,
    ) -> Option<NotificationAuthor> {
        if chat_type != "GROUP" || display_name.is_some() {
            return None;
        }
        let id = NotificationService::string_value(user_info.get(push_route::SENDER_ID))?;
        let name = trimming_whitespaces_and_newlines(actor_name);
        Some(NotificationAuthor {
            id,
            name: if name.is_empty() { sender_name.to_owned() } else { name.to_owned() },
            avatar_url: NotificationService::string_value(user_info.get(push::SENDER_AVATAR)),
        })
    }

    /// the time limit falls back to letters only while a photo is still on its way
    fn needs_any_download(&self) -> bool {
        NotificationService::needs_download(self.avatar_url.as_deref())
            || NotificationService::needs_download(self.author.as_ref().and_then(|author| author.avatar_url.as_deref()))
    }

    /// chat id, or the sender id if the chat id is empty
    fn conversation_id(&self) -> String {
        if self.chat_id.is_empty() { self.sender_id.clone() } else { self.chat_id.clone() }
    }

    fn target_title(&self, original_title: &str) -> String {
        if !trimming_whitespaces_and_newlines(original_title).is_empty() {
            original_title.to_owned()
        } else if self.sender_name.is_empty() {
            "Riseonly".to_owned()
        } else {
            self.sender_name.clone()
        }
    }
}

#[cfg(test)]
#[path = "notification_service_tests.rs"]
mod tests;

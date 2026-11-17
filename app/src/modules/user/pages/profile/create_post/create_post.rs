use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use gpui::{
    AnyView, App, Context, Entity, ExternalPaths, FocusHandle, FontWeight, Hsla, Image, KeyDownEvent, ObjectFit,
    Pixels, RenderImage, SharedString, StyleRefinement, StyledImage, Subscription, Task, WeakEntity, Window, black, canvas,
    hsla, img, relative, size, white,
};
use image::{Delay, Frame, RgbaImage};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseLimits};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::video_export::{poster as video_poster, probe as video_probe};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::haptics::selection_changed;
use rise_platform::uniform_type::UTType;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::input_ui::{Cancel, Paste};
use rise_ui::keyboard_dismiss::{KeyboardDismiss, KeyboardDismissModifiers};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::SkeletonUi;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::motion::{ease_in_out, ease_out};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::RichText;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition, Transitioned};
use rise_ui::switch_ui::SwitchUi;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use smallvec::SmallVec;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::CreatePostIntent;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFocusedTextInput;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::restriction::restriction_store::RestrictionStore;
use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsHeader;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::input_ui::InputUi;
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::people_picker_sheet::{PeoplePickerHostContext, PeoplePickerMode, PeoplePickerStore};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_under_chrome_box,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatBottomBarMetrics, ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatDraftCustomEmoji,
};
use crate::modules::chat::components::chat::input_picker::chat_input_emoji_tab::ChatInputEmojiTab;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerItemKind, MediaPickerResult, MediaPickerTrackPlate, MediaPickerTrackSelection,
};
use crate::modules::media_picker::api::short_video_limits::ShortVideoLimits;
use crate::modules::media_picker::editor::editor_state::{EditorPersonTagEntity, EditorVideoState};
use crate::modules::media_picker::ui::media_picker_session::MediaPickerSession;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::widgets::bottomsheets::track_picker::editor_track_picker_sheet::EditorTrackPickerSheet;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;
use crate::modules::music::widgets::track_trim_overlay::TrackTrimOverlay;
use crate::modules::post::components::post::post_achievement_media::{
    PostAchievementArtworkView, PostAchievementCatalog, PostAchievementCopy, PostAchievementMediaMetrics,
};
use crate::modules::post::engine::rise_post_engine_models::{RisePostContentKind, RisePostContentKindResolve};
use crate::modules::post::shared::post_field_rules::PostFieldRules;
use crate::modules::post::shared::post_media_slides::{PostMediaSlide, PostMediaSlides};
use crate::modules::post::shared::post_media_tag_remap::PostMediaTagRemap;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostCollaboratorUser, PostCountVisibility, PostEditMediaItem, PostHashtagInputFormatting,
    PostTagInputDraft, PostTagPlacement, PostTaggedUser,
};
use crate::modules::post::stores::post_actions::rise_post_publish_failure::RisePostPublishFailure;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::sparks::components::sparks_icon_view::SparksSettingsTile;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::pages::profile::create_post::create_post_media_page::CreatePostMediaHandoff;

const FIELD_TITLE: &str = "title";
const FIELD_CONTENT: &str = "content";
const FIELD_HASHTAGS: &str = "hashtags";

/// text of a new post the server refused on a moderation restriction
/// the next composer of same kind and account opens with it
struct CreatePostRefusedDraft {
    user_id: String,
    content_kind: RisePostContentKind,
    values: HashMap<String, String>,
    disable_comments: bool,
}

thread_local! {
    static REFUSED_DRAFT: RefCell<Option<CreatePostRefusedDraft>> = const { RefCell::new(None) };
}
const DEFAULT_EMOJI_PANEL_HEIGHT: f32 = 336.0;
const EMOJI_PANEL_RADIUS: f32 = 28.0;
const EMOJI_TOGGLE_SIDE: f32 = 34.0;
const TILE_SIDE: f32 = 108.0;
const HERO_MAX_WIDTH: f32 = 300.0;
const HERO_ASPECT: f32 = 4.0 / 5.0;
const PASTE_KEY: &str = "v";
const SHEET_MIN_WINDOW_WIDTH: f32 = 900.0;
const SHEET_HEIGHT_FRACTION: f32 = 0.92;
const SPLIT_SHEET_MAX_WIDTH: f32 = 1240.0;
const SPLIT_SHEET_WIDTH_FRACTION: f32 = 0.94;
const SPLIT_FORM_WIDTH: f32 = 460.0;
const SPLIT_HERO_INSET: f32 = 24.0;
const VIDEO_PREVIEW_SECONDS: f64 = 0.1;

#[derive(Clone)]
pub struct CreatePostMediaAttachment {
    pub id: String,
    pub kind: MediaPickerItemKind,
    pub local_file: Option<SaiWsOptimisticLocalFile>,
    pub remote_slide: Option<PostMediaSlide>,
    pub preview: Option<CleverImage>,
    pub is_preview_loading: bool,
    pub picker_item: Option<MediaPickerItem>,
    pub person_tags: Vec<EditorPersonTagEntity>,
}

impl CreatePostMediaAttachment {
    pub fn existing(slide: &PostMediaSlide) -> Self {
        Self {
            id: format!("existing:{}", slide.id()),
            kind: if slide.is_video() { MediaPickerItemKind::Video } else { MediaPickerItemKind::Image },
            local_file: None,
            remote_slide: Some(slide.clone()),
            preview: None,
            is_preview_loading: false,
            picker_item: None,
            person_tags: Vec::new(),
        }
    }

    /// url a slide is known by, same identity tag placements follow
    pub fn remote_key(&self) -> String {
        match &self.remote_slide {
            None => String::new(),
            Some(PostMediaSlide::Image { url, .. } | PostMediaSlide::Video { url, .. }) => {
                trimming_whitespaces_and_newlines(url).to_owned()
            }
        }
    }

    pub fn duration_text(&self) -> Option<String> {
        if let Some(payload) = self.picker_item.as_ref().and_then(|item| item.video_payload.as_ref()) {
            return Some(RiseFormat::duration_time_interval(payload.duration));
        }
        let milliseconds = self.remote_slide.as_ref()?.size()?.duration_ms;
        (milliseconds > 0).then(|| RiseFormat::duration_milliseconds(milliseconds))
    }
}

struct CreatePostImagePreviewInput {
    id: String,
    url: Url,
    data: Option<Vec<u8>>,
    maximum_pixel_dimension: i64,
}

type CreatePostPreviewCompletion = Rc<dyn Fn(String, Option<CleverImage>, &mut App)>;

struct CreatePostPreviewCoordinatorImageJob {
    token: u64,
    input: CreatePostImagePreviewInput,
    completion: CreatePostPreviewCompletion,
}

struct CreatePostPreviewActiveTask {
    id: String,
    token: u64,
    cancelled: Arc<AtomicBool>,
    _task: Task<()>,
}

impl CreatePostPreviewActiveTask {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

struct CreatePostPreviewCoordinator {
    next_token: u64,
    pending_image_jobs: Vec<CreatePostPreviewCoordinatorImageJob>,
    active_image_tasks: HashMap<String, CreatePostPreviewActiveTask>,
    video_task: Option<CreatePostPreviewActiveTask>,
}

impl CreatePostPreviewCoordinator {
    const MAXIMUM_ACTIVE_IMAGE_TASKS: usize = 2;

    fn new() -> Self {
        Self {
            next_token: 0,
            pending_image_jobs: Vec::new(),
            active_image_tasks: HashMap::new(),
            video_task: None,
        }
    }

    fn maximum_image_jobs() -> usize {
        usize::try_from(MediaPickerConfiguration::post_media_ceiling(true)).unwrap_or(0)
    }

    fn take_token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn prepare_images(
        &mut self,
        inputs: Vec<CreatePostImagePreviewInput>,
        completion: CreatePostPreviewCompletion,
        cx: &mut Context<Self>,
    ) {
        let mut rejected_ids = Vec::new();
        for input in inputs {
            self.cancel_image(&input.id, cx);
            if self.pending_image_jobs.len() + self.active_image_tasks.len() >= Self::maximum_image_jobs() {
                rejected_ids.push(input.id);
                continue;
            }
            let token = self.take_token();
            self.pending_image_jobs.push(CreatePostPreviewCoordinatorImageJob { token, input, completion: completion.clone() });
        }
        self.start_pending_image_jobs(cx);
        for id in rejected_ids {
            let completion = completion.clone();
            cx.defer(move |cx| completion(id, None, cx));
        }
    }

    fn prepare_video(
        &mut self,
        id: String,
        url: Url,
        maximum_pixel_dimension: i64,
        completion: CreatePostPreviewCompletion,
        cx: &mut Context<Self>,
    ) {
        self.cancel_video();
        let token = self.take_token();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let task_id = id.clone();
        tracing::warn!(target: "riseonly::X2239", %id, %url, maximum_pixel_dimension, "video preview start");
        let task = cx.spawn(async move |this, cx| {
            let image = cx
                .background_executor()
                .spawn(async move {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        CreatePostPreviewDecoder::video(&url, maximum_pixel_dimension, &flag)
                    }))
                    .unwrap_or_else(|_| {
                        tracing::warn!(target: "riseonly::X2239", "video preview decoder panicked");
                        None
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| this.finish_video(&task_id, token, image, completion, cx));
        });
        self.video_task = Some(CreatePostPreviewActiveTask { id, token, cancelled, _task: task });
    }

    /// cancels one attachment only, video and photos share the composer
    fn cancel(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.video_task.as_ref().is_some_and(|task| task.id == id) {
            self.cancel_video();
        }
        self.cancel_image(id, cx);
    }

    fn cancel_image(&mut self, id: &str, cx: &mut Context<Self>) {
        self.pending_image_jobs.retain(|job| job.input.id != id);
        if let Some(active) = self.active_image_tasks.remove(id) {
            active.cancel();
            self.start_pending_image_jobs(cx);
        }
    }

    fn cancel_all(&mut self) {
        self.pending_image_jobs.clear();
        for task in self.active_image_tasks.values() {
            task.cancel();
        }
        self.active_image_tasks.clear();
        self.cancel_video();
    }

    fn start_pending_image_jobs(&mut self, cx: &mut Context<Self>) {
        while self.active_image_tasks.len() < Self::MAXIMUM_ACTIVE_IMAGE_TASKS && !self.pending_image_jobs.is_empty() {
            let job = self.pending_image_jobs.remove(0);
            let CreatePostPreviewCoordinatorImageJob { token, input, completion } = job;
            let id = input.id.clone();
            let cancelled = Arc::new(AtomicBool::new(false));
            let flag = cancelled.clone();
            let task_id = id.clone();
            let task = cx.spawn(async move |this, cx| {
                let image = cx
                    .background_executor()
                    .spawn(async move {
                        CreatePostPreviewDecoder::image(
                            input.data.as_deref(),
                            &input.url,
                            input.maximum_pixel_dimension,
                            &flag,
                        )
                    })
                    .await;
                let _ = this.update(cx, |this, cx| this.finish_image(&task_id, token, image, completion, cx));
            });
            self.active_image_tasks
                .insert(id.clone(), CreatePostPreviewActiveTask { id, token, cancelled, _task: task });
        }
    }

    fn finish_image(
        &mut self,
        id: &str,
        token: u64,
        image: Option<CleverImage>,
        completion: CreatePostPreviewCompletion,
        cx: &mut Context<Self>,
    ) {
        if self.active_image_tasks.get(id).map(|task| task.token) != Some(token) {
            return;
        }
        self.active_image_tasks.remove(id);
        self.start_pending_image_jobs(cx);
        completion(id.to_owned(), image, cx);
    }

    fn finish_video(
        &mut self,
        id: &str,
        token: u64,
        image: Option<CleverImage>,
        completion: CreatePostPreviewCompletion,
        cx: &mut Context<Self>,
    ) {
        if self.video_task.as_ref().is_none_or(|task| task.id != id || task.token != token) {
            tracing::warn!(target: "riseonly::X2239", %id, token, "video preview result dropped: task replaced or cancelled");
            return;
        }
        tracing::warn!(target: "riseonly::X2239", %id, has_image = image.is_some(), "video preview finished");
        self.video_task = None;
        completion(id.to_owned(), image, cx);
    }

    fn cancel_video(&mut self) {
        if let Some(task) = self.video_task.take() {
            task.cancel();
        }
    }
}

enum CreatePostPreviewDecoder {}

static TILE_PIXEL_DIMENSION: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("create_post_preview_decoder_maximum_pixel_dimension"));

impl CreatePostPreviewDecoder {
    const PREVIEW_PIXEL_BUDGET: i64 = 16_000_000;

    fn tile_pixel_dimension() -> i64 {
        *TILE_PIXEL_DIMENSION
    }

    /// hero decode size, the window pixel width since the hero renders edge to edge
    fn hero_pixel_dimension(window: &Window) -> i64 {
        let viewport = window.viewport_size();
        let side = f32::from(viewport.width).min(f32::from(viewport.height));
        (side * window.scale_factor()).ceil() as i64
    }

    fn hero_pixel_dimension_for_attachment_count(count: usize, window: &Window) -> i64 {
        let count = (count as i64).max(1);
        let hero = Self::hero_pixel_dimension(window);
        if count * hero * hero <= Self::PREVIEW_PIXEL_BUDGET {
            return hero;
        }
        let fitted = (Self::PREVIEW_PIXEL_BUDGET as f64 / count as f64).sqrt() as i64;
        Self::tile_pixel_dimension().max(hero.min(fitted))
    }

    fn image(
        data: Option<&[u8]>,
        url: &Url,
        maximum_pixel_dimension: i64,
        cancelled: &AtomicBool,
    ) -> Option<CleverImage> {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let image = match data {
            Some(data) if !data.is_empty() => CleverImageBitmapDecoder::decode_frame(data, 0, maximum_pixel_dimension),
            _ => {
                let read = std::fs::read(url.to_file_path()?).ok()?;
                if cancelled.load(Ordering::Relaxed) {
                    return None;
                }
                CleverImageBitmapDecoder::decode_frame(&read, 0, maximum_pixel_dimension)
            }
        };
        if cancelled.load(Ordering::Relaxed) { None } else { image }
    }

    fn video(url: &Url, maximum_pixel_dimension: i64, cancelled: &AtomicBool) -> Option<CleverImage> {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let Some(path) = url.to_file_path() else {
            tracing::warn!(target: "riseonly::X2239", %url, "video preview: not a file url");
            return None;
        };
        let side = u32::try_from(maximum_pixel_dimension).ok()?;
        let frame = match video_poster(&path, VIDEO_PREVIEW_SECONDS, side) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(target: "riseonly::X2239", path = %path.display(), ?error, "video preview poster failed");
                return None;
            }
        };
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let bitmap = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
        let frame = Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1));
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
        Some(Arc::new(RenderImage::new(frames)))
    }
}

pub struct CreatePostCollaboratorLimits {
    pub coauthors: i64,
    pub tagged_users: i64,
    pub tag_placements: i64,
}

static COLLABORATOR_LIMITS: LazyLock<CreatePostCollaboratorLimits> = LazyLock::new(|| CreatePostCollaboratorLimits {
    coauthors: RiseLimits::service_value("post_service_max_coauthors"),
    tagged_users: RiseLimits::service_value("post_service_max_tagged_users"),
    tag_placements: RiseLimits::service_value("post_service_max_tag_placements"),
});

static MAX_VIDEO_ATTACHMENTS: LazyLock<i64> =
    LazyLock::new(|| MediaPickerConfiguration::post_media().max_video_count.unwrap_or(1).clamp(0, 1));

pub struct CreatePostScreenState {
    pub values: HashMap<String, String>,
    pub content_kind: RisePostContentKind,
    pub opens_as_riff: bool,
    pub has_existing_publication_media: bool,
    pub disable_comments: bool,
    pub count_visibility: PostCountVisibility,
    pub map_journey_show_heat: bool,
    pub requests_riff: bool,
    pub requests_publications: bool,
    pub attached_track: Option<RiseStoredMusicTrack>,
    pub attached_track_start_ms: Option<i64>,
    pub attached_track_end_ms: Option<i64>,
    pub attached_track_plate: Option<MediaPickerTrackPlate>,
    pub shared_achievement_id: Option<i64>,
    pub shared_achievement: Option<RiseAchievement>,
    pub coauthors: Vec<PostCollaboratorUser>,
    pub tagged_people: Vec<PostCollaboratorUser>,
    pub media: Vec<CreatePostMediaAttachment>,
    pub edits_media: bool,
    pub seeded_media_ids: Vec<String>,
}

impl Default for CreatePostScreenState {
    fn default() -> Self {
        Self {
            values: HashMap::from([
                (FIELD_TITLE.to_owned(), String::new()),
                (FIELD_CONTENT.to_owned(), String::new()),
                (FIELD_HASHTAGS.to_owned(), String::new()),
            ]),
            content_kind: RisePostContentKind::Publication,
            opens_as_riff: false,
            has_existing_publication_media: false,
            disable_comments: false,
            count_visibility: PostCountVisibility::SHOWN,
            map_journey_show_heat: true,
            requests_riff: false,
            requests_publications: true,
            attached_track: None,
            attached_track_start_ms: None,
            attached_track_end_ms: None,
            attached_track_plate: None,
            shared_achievement_id: None,
            shared_achievement: None,
            coauthors: Vec::new(),
            tagged_people: Vec::new(),
            media: Vec::new(),
            edits_media: false,
            seeded_media_ids: Vec::new(),
        }
    }
}

impl CreatePostScreenState {
    fn value(&self, name: &str) -> &str {
        self.values.get(name).map_or("", String::as_str)
    }

    /// true if the media list changed, only a changed list is sent so a text edit keeps the media
    pub fn media_changed(&self) -> bool {
        self.edits_media && !self.media.iter().map(|item| &item.id).eq(self.seeded_media_ids.iter())
    }

    /// max attachments for this author tier from canon post_images, photos and video together
    /// read on demand so a subscription starting or lapsing mid compose moves it
    pub fn max_attachment_count(cx: &App) -> i64 {
        MediaPickerConfiguration::post_media_ceiling(Self::author_is_premium(cx))
    }

    pub fn author_is_premium(cx: &App) -> bool {
        AuthServicesStore::try_read(cx)
            .and_then(|auth| auth.profile())
            .is_some_and(|profile| profile.more.is_premium)
    }

    pub fn max_video_attachments() -> i64 {
        *MAX_VIDEO_ATTACHMENTS
    }

    /// publication needs media,
    /// thread needs a description,
    /// title and hashtags are optional. a riff needs a photo or video
    pub fn disabled(&self, cx: &App) -> bool {
        if self.content_kind == RisePostContentKind::Publication {
            if self.media.len() as i64 > Self::max_attachment_count(cx) && (!self.edits_media || self.media_changed()) {
                return true;
            }
            if self.is_riff_composer() {
                return self.media.is_empty();
            }
            return !self.has_publication_media();
        }
        PostFieldRules::is_description_missing(self.content_kind, self.value(FIELD_CONTENT))
    }

    /// checked against the kind not cleared on switch,
    /// so returning to publication doesnt turn a riff screen plain
    pub fn is_riff_composer(&self) -> bool {
        self.content_kind == RisePostContentKind::Publication && self.opens_as_riff
    }

    /// only the share flow makes these, server refuses achievement_id with any media key
    pub fn is_achievement_share(&self) -> bool {
        self.shared_achievement_id.unwrap_or(0) > 0
    }

    pub fn has_publication_media(&self) -> bool {
        self.has_existing_publication_media
            || !self.media.is_empty()
            || self.attached_track.is_some()
            || self.is_achievement_share()
    }

    pub fn has_video(&self) -> bool {
        self.media.iter().any(|item| item.kind == MediaPickerItemKind::Video)
    }

    pub fn can_add_more_media(&self, cx: &App) -> bool {
        (self.media.len() as i64) < Self::max_attachment_count(cx)
    }

    pub fn exceeds_media_limit(&self, cx: &App) -> bool {
        self.content_kind == RisePostContentKind::Publication
            && self.media.len() as i64 > Self::max_attachment_count(cx)
    }

    /// count against the ceiling plus why it binds, premium line only for a standard author
    pub fn media_limit_message(&self, cx: &App) -> String {
        let mut message = format!(
            "{}/{} {}",
            self.media.len(),
            Self::max_attachment_count(cx),
            tr("premium_upsell_reason_generic")
        );
        if !Self::author_is_premium(cx) {
            message.push(' ');
            message.push_str(&tr("premium_upsell_subtitle"));
        }
        message
    }

    pub fn video_attachment(&self) -> Option<&CreatePostMediaAttachment> {
        self.media.iter().find(|item| item.kind == MediaPickerItemKind::Video)
    }

    pub fn photo_files(&self) -> Vec<SaiWsOptimisticLocalFile> {
        self.media
            .iter()
            .filter(|item| item.kind == MediaPickerItemKind::Image)
            .filter_map(|item| item.local_file.clone())
            .collect()
    }

    /// video index among all slides, equals the photos picked before it
    pub fn video_position(&self) -> Option<i64> {
        self.media.iter().position(|item| item.kind == MediaPickerItemKind::Video).map(|index| index as i64)
    }

    pub fn shows_create_placement_row(&self) -> bool {
        if self.content_kind != RisePostContentKind::Publication || self.is_achievement_share() {
            return false;
        }
        self.opens_as_riff || !self.media.is_empty()
    }

    pub fn publishes_riff_only(&self) -> bool {
        self.content_kind == RisePostContentKind::Publication
            && !self.is_achievement_share()
            && self.requests_riff
            && !self.requests_publications
    }

    pub fn collaborator_limits() -> &'static CreatePostCollaboratorLimits {
        &COLLABORATOR_LIMITS
    }

    pub fn has_taggable_media(&self) -> bool {
        self.content_kind == RisePostContentKind::Publication && !self.is_achievement_share() && !self.media.is_empty()
    }

    pub fn placed_tag_user_ids(&self) -> HashSet<String> {
        self.media.iter().flat_map(|attachment| attachment.person_tags.iter().map(|tag| tag.user_id.clone())).collect()
    }

    /// everyone tagged, placed on a frame plus picked from the list
    pub fn tagged_people_count(&self) -> usize {
        let mut ids = self.placed_tag_user_ids();
        ids.extend(self.tagged_people.iter().map(|user| user.id.clone()));
        ids.len()
    }

    /// tags for the wire, frame placements first then list picks without a point
    /// one entry per person, clamped to canon ceilings
    pub fn merged_tag_drafts(&self) -> Vec<PostTagInputDraft> {
        self.tag_drafts(&[])
    }

    pub fn edited_media_items(&self) -> Option<Vec<PostEditMediaItem>> {
        if !self.media_changed() {
            return None;
        }
        Some(
            self.media
                .iter()
                .filter_map(|attachment| {
                    if let Some(slide) = &attachment.remote_slide {
                        return Some(PostEditMediaItem::Existing(slide.clone()));
                    }
                    match attachment.kind {
                        MediaPickerItemKind::Video => attachment.picker_item.clone().map(PostEditMediaItem::Video),
                        MediaPickerItemKind::Image => attachment.local_file.clone().map(PostEditMediaItem::Photo),
                        MediaPickerItemKind::File => None,
                    }
                })
                .collect(),
        )
    }

    pub fn edited_tag_drafts(&self, post: &GetPostFeedResponse) -> Option<Vec<PostTagInputDraft>> {
        if !self.media_changed()
            || !self.media.iter().any(|item| item.remote_slide.is_none() && !item.person_tags.is_empty())
        {
            return None;
        }
        let keys: Vec<String> = self.media.iter().map(CreatePostMediaAttachment::remote_key).collect();
        let carried = PostMediaTagRemap::remap(&post.tagged_users, &post.media_slide_keys(), &keys);
        Some(self.tag_drafts(&carried.tagged_users))
    }

    fn tag_drafts(&self, seed: &[PostTaggedUser]) -> Vec<PostTagInputDraft> {
        let mut order: Vec<String> = Vec::new();
        let mut placements: HashMap<String, Vec<PostTagPlacement>> = HashMap::new();
        let mut users: HashMap<String, PostCollaboratorUser> = HashMap::new();

        for entry in seed {
            let Some(user) = &entry.user else {
                continue;
            };
            if placements.contains_key(&user.id) {
                continue;
            }
            placements.insert(user.id.clone(), entry.placements.clone());
            users.insert(user.id.clone(), user.clone());
            order.push(user.id.clone());
        }

        for (slide_index, attachment) in self.media.iter().enumerate() {
            for tag in &attachment.person_tags {
                if !placements.contains_key(&tag.user_id) {
                    placements.insert(tag.user_id.clone(), Vec::new());
                    order.push(tag.user_id.clone());
                    users.insert(
                        tag.user_id.clone(),
                        PostCollaboratorUser {
                            id: tag.user_id.clone(),
                            logo: tag.logo.clone().unwrap_or_default(),
                            name: tag.name.clone(),
                            tag: tag.tag.clone(),
                            ..PostCollaboratorUser::default()
                        },
                    );
                }
                if let Some(own) = placements.get_mut(&tag.user_id) {
                    own.push(PostTagPlacement {
                        is_dark: tag.uses_dark_style(),
                        slide_index: slide_index as i64,
                        x: f64::from(tag.center.x),
                        y: f64::from(tag.center.y),
                    });
                }
            }
        }
        for user in &self.tagged_people {
            users.insert(user.id.clone(), user.clone());
            if !placements.contains_key(&user.id) {
                placements.insert(user.id.clone(), Vec::new());
                order.push(user.id.clone());
            }
        }

        let limits = Self::collaborator_limits();
        let placement_ceiling = usize::try_from(limits.tag_placements).unwrap_or(0);
        let people_ceiling = usize::try_from(limits.tagged_users).unwrap_or(0);
        order
            .into_iter()
            .take(people_ceiling)
            .map(|user_id| {
                let own = placements.get(&user_id).map_or_else(Vec::new, |all| PostMediaSlides::first_placement_per_slide(all));
                let user = users.get(&user_id).cloned();
                PostTagInputDraft {
                    user_id,
                    placements: own.into_iter().take(placement_ceiling).collect(),
                    user,
                }
            })
            .collect()
    }

    pub fn all_tagged_people(&self) -> Vec<PostCollaboratorUser> {
        self.merged_tag_drafts().into_iter().filter_map(|draft| draft.user).collect()
    }

    pub fn remove_tagged_people(&mut self, ids: &HashSet<String>) {
        if ids.is_empty() {
            return;
        }
        self.tagged_people.retain(|user| !ids.contains(&user.id));
        for attachment in &mut self.media {
            attachment.person_tags.retain(|tag| !ids.contains(&tag.user_id));
        }
    }

    pub fn set_value(&mut self, name: &str, value: &str) {
        self.values.insert(name.to_owned(), value.to_owned());
    }

    pub fn apply_content_kind(&mut self, kind: RisePostContentKind) {
        self.content_kind = kind;
        if kind == RisePostContentKind::Thread {
            self.media.clear();
            self.tagged_people.clear();
            self.attached_track = None;
            self.attached_track_start_ms = None;
            self.attached_track_end_ms = None;
            self.attached_track_plate = None;
            self.has_existing_publication_media = false;
            self.reset_optional_riff_if_media_is_gone();
        }
    }

    pub fn remove_media(&mut self, id: &str) {
        self.media.retain(|item| item.id != id);
        self.reset_optional_riff_if_media_is_gone();
    }

    /// at least one placement stays on, switching off the last one does nothing
    pub fn set_requests_riff(&mut self, value: bool) {
        if value || self.requests_publications {
            self.requests_riff = value;
        }
    }

    pub fn set_requests_publications(&mut self, value: bool) {
        if value || self.requests_riff {
            self.requests_publications = value;
        }
    }

    /// hidden also-riffs switch must not come back on when media is picked again
    fn reset_optional_riff_if_media_is_gone(&mut self) {
        if !self.media.is_empty() || self.opens_as_riff {
            return;
        }
        self.requests_riff = false;
        self.requests_publications = true;
    }
}

/// row of a counter or sparks switch, path to its flag
#[derive(Clone, Copy)]
struct CountVisibilityPath {
    get: fn(&PostCountVisibility) -> bool,
    set: fn(&mut PostCountVisibility, bool),
}

impl CountVisibilityPath {
    const LIKES: Self = Self { get: |v| v.likes, set: |v, value| v.likes = value };
    const COMMENTS: Self = Self { get: |v| v.comments, set: |v, value| v.comments = value };
    const REPOSTS: Self = Self { get: |v| v.reposts, set: |v, value| v.reposts = value };
    const SHARES: Self = Self { get: |v| v.shares, set: |v, value| v.shares = value };
    const COMMENT_DISLIKES: Self = Self { get: |v| v.comment_dislikes, set: |v, value| v.comment_dislikes = value };
    const SPARKS: Self = Self { get: |v| v.sparks, set: |v, value| v.sparks = value };
    const ALLOWS_SPARKS: Self = Self { get: |v| v.allows_sparks, set: |v, value| v.allows_sparks = value };
    const ALLOWS_DOWNLOAD: Self = Self { get: |v| v.allows_download, set: |v, value| v.allows_download = value };
    const ALLOWS_DOWNLOAD_WITHOUT_WATERMARK: Self = Self {
        get: |v| v.allows_download_without_watermark,
        set: |v, value| v.allows_download_without_watermark = value,
    };
}

#[derive(IntoElement)]
struct CreatePostCommentsDisabledGlyph {
    background: Hsla,
}

impl RenderOnce for CreatePostCommentsDisabledGlyph {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bar = |width: f32, fill: Hsla| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(div().w(px(width)).h(px(22.0)).rounded(px(width / 2.0)).bg(fill).rotation_effect(-45.0))
        };
        div()
            .relative()
            .size(px(GroupedCardMetrics::ICON_SIDE))
            .rounded(px(8.0))
            .bg(self.background)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("bubble.left.fill", px(14.0), white(), true))
            .child(bar(5.0, self.background))
            .child(bar(2.0, white()))
    }
}

fn external_item(path: &Path) -> Option<MediaPickerItem> {
    if !path.is_file() {
        return None;
    }
    let url = Url::from_file_path(path)?;
    let content_type = path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(UTType::from_filename_extension)?;
    let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let id = Uuid::new_v4().uuid_string();
    if content_type.conforms_to(&UTType::image()) {
        return Some(MediaPickerItem::new(
            id,
            MediaPickerItemKind::Image,
            url,
            file_name,
            None,
            None,
            None,
            None,
            false,
        ));
    }
    if !content_type.conforms_to(&UTType::movie()) {
        return None;
    }
    let probe = video_probe(path).ok()?;
    let pixel_size = size(probe.natural_size.width as f32, probe.natural_size.height as f32);
    let edit_state = EditorVideoState::empty(Some(id), probe.duration_seconds);
    Some(MediaPickerItem::deferred_external_video(url, file_name, pixel_size, edit_state, None))
}

pub struct CreatePost {
    map_journey_id: Option<String>,
    editing_post: Option<GetPostFeedResponse>,
    intent: CreatePostIntent,
    state: Entity<CreatePostScreenState>,
    came_from_media_step: bool,
    hero_page_id: Option<String>,
    hero_pane: Rc<Cell<Option<(f32, f32)>>>,
    hero_beside: bool,
    page_width: Rc<Cell<Option<f32>>>,
    show_media_picker: bool,
    show_track_picker: bool,
    show_track_trim: bool,
    track_trim_presence: Presence<()>,
    pending_content_kind: Option<RisePostContentKind>,
    is_submitting: bool,
    preview_coordinator: Entity<CreatePostPreviewCoordinator>,
    preview_generation: u64,
    emoji_field: Option<&'static str>,
    focused_field: Option<&'static str>,
    emoji_caret: Option<NSRange>,
    emoji_caret_text: String,
    title_focus_token: i64,
    content_focus_token: i64,
    last_keyboard_height: f32,
    emoji_presence: Presence<()>,
    hero_pixels: i64,
    nav_store: Option<Entity<NavigationStore>>,
    focus: FocusHandle,
    initial_values: HashMap<String, String>,
    initial_track_id: Option<String>,
    form_view: Option<Entity<CreatePostFormView>>,
    layers_view: Option<Entity<CreatePostLayersView>>,
    section_views: HashMap<CreatePostFormSection, Entity<CreatePostSectionView>>,
    _observations: Vec<Subscription>,
}

impl CreatePost {
    pub fn new(
        intent: CreatePostIntent,
        editing_post: Option<GetPostFeedResponse>,
        shared_achievement_id: Option<i64>,
        map_journey_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let handoff = CreatePostMediaHandoff::shared(cx);
        let arrives_from_media_step = editing_post.is_none()
            && shared_achievement_id.unwrap_or(0) <= 0
            && handoff.read(cx).has_pending(intent);
        let mut seeded = CreatePostScreenState::default();
        if editing_post.is_none() {
            match intent {
                CreatePostIntent::Publication => {}
                CreatePostIntent::Reel => {
                    seeded.opens_as_riff = true;
                    seeded.requests_riff = true;
                    seeded.requests_publications = false;
                }
                CreatePostIntent::Thread => seeded.content_kind = RisePostContentKind::Thread,
            }
        }
        if let Some(post) = &editing_post {
            Self::seed_from_post(&mut seeded, post, cx);
        }
        if let Some(achievement_id) = shared_achievement_id
            && achievement_id > 0
            && editing_post.is_none()
        {
            seeded.shared_achievement_id = Some(achievement_id);
            seeded.content_kind = RisePostContentKind::Thread;
            seeded.attached_track = None;
            seeded.attached_track_start_ms = None;
            seeded.attached_track_end_ms = None;
            seeded.attached_track_plate = None;
            if let Some(achievement) = PostAchievementCatalog::achievement(achievement_id, cx) {
                seeded.values.insert(FIELD_TITLE.to_owned(), PostAchievementCopy::suggested_title(&achievement));
                seeded
                    .values
                    .insert(FIELD_CONTENT.to_owned(), PostAchievementCopy::suggested_description(&achievement));
                seeded.shared_achievement = Some(achievement);
            }
        }
        let initial_values = seeded.values.clone();
        if editing_post.is_none() && seeded.shared_achievement_id.is_none() {
            Self::restore_refused_draft(&mut seeded, cx);
        }
        let initial_track_id = seeded.attached_track.as_ref().and_then(|track| track.id.clone());
        let state = cx.new(|_| seeded);
        let observation = cx.observe(&state, |_, state, cx| {
            let facts = CreatePostGuideFacts::facts(state.read(cx));
            CreatePostGuideFacts::report(facts, cx);
            cx.notify();
        });
        CreatePostGuideFacts::report(CreatePostGuideFacts::facts(state.read(cx)), cx);
        let mut page = Self {
            map_journey_id,
            editing_post,
            intent,
            state,
            came_from_media_step: arrives_from_media_step,
            hero_page_id: None,
            hero_pane: Rc::new(Cell::new(None)),
            hero_beside: false,
            page_width: Rc::new(Cell::new(None)),
            show_media_picker: false,
            show_track_picker: false,
            show_track_trim: false,
            track_trim_presence: Presence::new(None)
                .with_transition(Transition::opacity().combined(Transition::scale(1.02))),
            pending_content_kind: None,
            is_submitting: false,
            preview_coordinator: cx.new(|_| CreatePostPreviewCoordinator::new()),
            preview_generation: 0,
            emoji_field: None,
            focused_field: None,
            emoji_caret: None,
            emoji_caret_text: String::new(),
            title_focus_token: 0,
            content_focus_token: 0,
            last_keyboard_height: DEFAULT_EMOJI_PANEL_HEIGHT,
            emoji_presence: Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom)),
            hero_pixels: CreatePostPreviewDecoder::hero_pixel_dimension(window),
            nav_store: None,
            focus: cx.focus_handle(),
            initial_values,
            initial_track_id,
            form_view: None,
            layers_view: None,
            section_views: HashMap::new(),
            _observations: vec![observation],
        };
        page.consume_picked_media_handoff_if_needed(window, cx);
        page
    }

    fn seed_from_post(seeded: &mut CreatePostScreenState, post: &GetPostFeedResponse, cx: &mut App) {
        seeded.content_kind = RisePostContentKind::resolve(
            Some(post.content_kind.as_str()),
            &post.images,
            post.video.as_deref(),
            post.is_short,
            post.is_image_reel,
            post.track_id.as_deref(),
        );
        if seeded.content_kind == RisePostContentKind::Publication && post.shared_achievement_id().is_none() {
            let existing: Vec<CreatePostMediaAttachment> =
                post.media_slides().iter().map(CreatePostMediaAttachment::existing).collect();
            seeded.seeded_media_ids = existing.iter().map(|item| item.id.clone()).collect();
            seeded.media = existing;
            seeded.edits_media = true;
        } else {
            seeded.has_existing_publication_media =
                !post.images.is_empty() || post.video.is_some() || post.track_id.is_some();
        }
        let hashtags: Vec<String> = post.hashtags.iter().map(|tag| format!("#{tag}")).collect();
        seeded.values = HashMap::from([
            (FIELD_TITLE.to_owned(), PostFieldRules::title().clamped(&post.title)),
            (FIELD_CONTENT.to_owned(), post.content.clone()),
            (FIELD_HASHTAGS.to_owned(), hashtags.join(" ")),
        ]);
        seeded.disable_comments = !post.can_comment;
        seeded.count_visibility = post.count_visibility;
        seeded.requests_riff = seeded.content_kind == RisePostContentKind::Publication && post.is_riff;
        seeded.requests_publications = !seeded.requests_riff || post.show_in_publications;
        seeded.attached_track = post.track.clone();
        seeded.attached_track_start_ms = post.track_start_ms;
        seeded.attached_track_end_ms = post.track_end_ms;
        seeded.attached_track_plate = post.track_plate.as_ref().map(|plate| MediaPickerTrackPlate {
            is_dark: plate.is_dark,
            center_x: plate.center_x,
            center_y: plate.center_y,
            rotation: plate.rotation,
            scale: plate.scale,
        });
        seeded.shared_achievement_id = post.shared_achievement_id();
        seeded.shared_achievement =
            post.shared_achievement_id().and_then(|id| PostAchievementCatalog::achievement(id, cx));
    }

    /// store publishes concurrently and never drops a second publish,
    /// so an uploading post doesnt lock the composer
    fn is_publishing(&self) -> bool {
        self.is_submitting
    }

    fn media_picker_configuration(&self, cx: &App) -> MediaPickerConfiguration {
        let state = self.state.read(cx);
        let videos_used = i64::from(state.has_video());
        let mut configuration = MediaPickerConfiguration::post_media_appending(
            CreatePostScreenState::max_attachment_count(cx) - state.media.len() as i64,
            CreatePostScreenState::max_video_attachments() - videos_used,
            CreatePostScreenState::author_is_premium(cx),
        );
        if self.editing_post.is_some() {
            configuration.allows_music_selection = false;
        }
        configuration
    }

    fn composer_track_selection(&self, cx: &App) -> Option<MediaPickerTrackSelection> {
        let state = self.state.read(cx);
        let track = state.attached_track.clone()?;
        let end_ms = state.attached_track_end_ms.unwrap_or_else(|| Self::default_segment_end(&track));
        Some(MediaPickerTrackSelection {
            track,
            start_ms: state.attached_track_start_ms.unwrap_or(0),
            end_ms,
            plate: state.attached_track_plate.clone(),
        })
    }

    fn update_state(&self, cx: &mut Context<Self>, update: impl FnOnce(&mut CreatePostScreenState)) {
        self.state.update(cx, |state, cx| {
            update(state);
            cx.notify();
        });
    }

    fn set_show_track_trim(&mut self, shows: bool, cx: &mut Context<Self>) {
        self.show_track_trim = shows;
        self.track_trim_presence.set_in(shows.then_some(()), ease_in_out(0.22), RiseClock::monotonic(), cx);
        cx.notify();
    }

    /// callback of a row or button,
    /// runs on the page by weak handle since sheet hosts may keep closures while presented
    fn action<F: Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static>(
        cx: &Context<Self>,
        action: F,
    ) -> impl Fn(&mut Window, &mut App) + 'static + use<F> {
        let weak = cx.weak_entity();
        move |window, cx| {
            let _ = weak.update(cx, |page, cx| action(page, window, cx));
        }
    }

    fn consume_picked_media_handoff_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.came_from_media_step {
            return;
        }
        let intent = self.intent;
        let result = CreatePostMediaHandoff::shared(cx).update(cx, |handoff, _| handoff.take(intent));
        if let Some(result) = result {
            self.handle_media_picker_result(result, window, cx);
        }
    }

    /// an edited post and an achievement share keep the kind they had
    fn is_content_kind_locked(&self, cx: &App) -> bool {
        self.editing_post.is_some() || self.state.read(cx).is_achievement_share()
    }

    fn content_kind_group(&self, theme: &AppTheme, cx: &Context<Self>) -> GroupedBtns {
        let locked = self.is_content_kind_locked(cx);
        let state = self.state.read(cx);
        let reader = self.state.clone();
        let setter = cx.weak_entity();
        let tapper = cx.weak_entity();
        let active_tab = Binding::new(
            move |cx| i64::from(reader.read(cx).content_kind != RisePostContentKind::Publication),
            move |index, _, cx| {
                let _ = setter.update(cx, |page, cx| page.request_content_kind(index, cx));
            },
        );
        let header = AnimatedTabsHeader::new(
            "create_post_kind_tabs",
            vec![
                TabHeaderConfig::new(0, tr("create_post_kind_publication")),
                TabHeaderConfig::new(1, tr("create_post_kind_thread")),
            ],
            active_tab,
            Rc::new(|_: &App| 0.0_f32),
            0.0,
            Rc::new(move |index: i64, _: &mut Window, cx: &mut App| {
                let _ = tapper.update(cx, |page, cx| page.request_content_kind(index, cx));
            }),
        );
        let hint = (!locked && (state.content_kind == RisePostContentKind::Thread || !state.has_publication_media()))
            .then(|| {
                let key = if state.content_kind == RisePostContentKind::Thread {
                    "create_post_kind_thread_hint"
                } else {
                    "create_post_kind_publication_hint"
                };
                MainText::new(tr(key)).px(12.0).color(theme.text.secondary)
            });
        let content = vstack(10.0)
            .items_start()
            .w_full()
            .px(px(12.0))
            .py(px(14.0))
            .child(
                hstack(None)
                    .w_full()
                    .child(MainText::new(tr("create_post_kind_title")).px(15.0).font_weight(FontWeight::SEMIBOLD))
                    .when(locked, |row| {
                        row.children(IconUi::sized("lock.fill", px(12.0), theme.text.secondary, true))
                    })
                    .child(spacer()),
            )
            .child(
                div()
                    .w_full()
                    .h(AnimatedTabsMetrics::BAR_HEIGHT)
                    .when(locked, |tabs| tabs.opacity(0.5))
                    .child(header),
            )
            .children(hint);
        GroupedBtns::new(
            "create_post_kind_group",
            vec![GroupedBtn::new("createPostKind", "").content_slot(content)],
        )
    }

    fn request_content_kind(&mut self, index: i64, cx: &mut Context<Self>) {
        if self.is_content_kind_locked(cx) {
            return;
        }
        let kind = if index == 1 { RisePostContentKind::Thread } else { RisePostContentKind::Publication };
        let state = self.state.read(cx);
        if kind == state.content_kind {
            return;
        }
        if kind == RisePostContentKind::Thread && state.has_publication_media() {
            self.pending_content_kind = Some(kind);
            let confirm = cx.weak_entity();
            let dismiss = cx.weak_entity();
            NativeConfirmAlert::present_delete(
                &tr("create_post_switch_thread_title"),
                &tr("create_post_switch_thread_message"),
                Some(tr("create_post_switch_thread_confirm")),
                None,
                false,
                None,
                Some(Box::new(move |cx: &mut App| {
                    let _ = dismiss.update(cx, |page, _| page.pending_content_kind = None);
                })),
                move |cx| {
                    let _ = confirm.update(cx, |page, cx| {
                        if let Some(kind) = page.pending_content_kind.take() {
                            page.apply_content_kind(kind, cx);
                        }
                    });
                },
                cx,
            );
            return;
        }
        self.apply_content_kind(kind, cx);
    }

    fn achievement_composer(&self, theme: &AppTheme, cx: &Context<Self>) -> GroupedBtns {
        GroupedBtns::new(
            "create_post_achievement_group",
            vec![GroupedBtn::new("createPostAchievement", "").content_slot(self.achievement_composer_content(theme, cx))],
        )
    }

    fn achievement_composer_content(&self, theme: &AppTheme, cx: &Context<Self>) -> Div {
        let state = self.state.read(cx);
        let side = PostAchievementMediaMetrics::COMPOSER_ARTWORK_SIDE;
        let body = vstack(10.0).w_full().py(px(16.0));
        match &state.shared_achievement {
            Some(achievement) => body
                .child(PostAchievementArtworkView::new("create_post_achievement_art", achievement.clone(), side))
                .child(
                    div().w_full().px(px(16.0)).text_size(px(13.0)).text_color(theme.text.secondary).text_center().child(
                        RichText::new(
                            "create_post_achievement_text",
                            PostAchievementCopy::highlighting_rarity(
                                state.value(FIELD_CONTENT),
                                achievement.rarity_tier(),
                            ),
                        ),
                    ),
                ),
            None => body.child(
                div()
                    .size(px(side))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("trophy.fill", px(side * 0.34), theme.text.secondary.opacity(0.55), true)),
            ),
        }
    }

    fn media_composer(&self, theme: &AppTheme, cx: &Context<Self>) -> GroupedBtns {
        GroupedBtns::new(
            "create_post_media_group",
            vec![
                GroupedBtn::new("createPostMedia", "")
                    .disabled(self.is_publishing())
                    .content_slot(self.media_composer_content(theme, cx)),
            ],
        )
    }

    fn media_composer_content(&self, theme: &AppTheme, cx: &Context<Self>) -> Div {
        let state = self.state.read(cx);
        let title = if state.is_riff_composer() { tr("riffs") } else { tr("create_post_media_section") };
        vstack(10.0)
            .items_start()
            .w_full()
            .py(px(14.0))
            .child(
                hstack(None)
                    .w_full()
                    .px(px(12.0))
                    .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD))
                    .when(!state.media.is_empty(), |row| {
                        row.child(MainText::new(state.media.len().to_string()).px(13.0).color(theme.text.secondary))
                    })
                    .when(!state.is_riff_composer() && state.requests_riff && !state.media.is_empty(), |row| {
                        row.child(Self::riffs_badge(theme))
                    })
                    .child(spacer()),
            )
            .when(state.exceeds_media_limit(cx), |column| {
                column.child(
                    div().w_full().px(px(12.0)).child(
                        MainText::new(state.media_limit_message(cx)).px(13.0).color(theme.semantic.error_100),
                    ),
                )
            })
            .child(
                div()
                    .id("create_post_media_strip")
                    .flex()
                    .w_full()
                    .overflow_x_scroll()
                    .gap(px(10.0))
                    .px(px(12.0))
                    .child(self.add_media_tile(theme, cx))
                    .children(state.media.iter().map(|item| {
                        if item.kind == MediaPickerItemKind::Video {
                            self.video_tile(item, theme, cx)
                        } else {
                            self.media_tile(item, theme, cx)
                        }
                    })),
            )
    }

    fn media_hero_group(&self, theme: &AppTheme, cx: &Context<Self>) -> Div {
        vstack(10.0)
            .items_stretch()
            .w_full()
            .px(px(12.0))
            .when(!self.hero_beside, |group| group.child(self.hero_media_pager(theme, cx)))
            .children(self.hero_add_more_row(theme, cx))
    }

    /// paged hero media, dots switch the page and chevrons replace the swipe a pointer lacks
    fn hero_media_pager(&self, theme: &AppTheme, cx: &Context<Self>) -> Div {
        let state = self.state.read(cx);
        let selected = self
            .hero_page_id
            .as_ref()
            .and_then(|id| state.media.iter().position(|item| item.id == *id))
            .unwrap_or(0);
        let count = state.media.len();
        let (frame_width, frame_height) = self.hero_frame(state.media.get(selected));
        let pager = div()
            .relative()
            .w(px(frame_width))
            .h(px(frame_height))
            .rounded(px(20.0))
            .overflow_hidden()
            .children(state.media.get(selected).map(|item| {
                if item.kind == MediaPickerItemKind::Video {
                    self.hero_video_page(item, theme, cx)
                } else {
                    self.hero_image_page(item, theme, cx)
                }
            }))
            .when(count > 1, |pager| {
                let ids: Vec<String> = state.media.iter().map(|item| item.id.clone()).collect();
                let previous = selected.checked_sub(1).and_then(|index| ids.get(index).cloned());
                let next = ids.get(selected + 1).cloned();
                pager
                    .children(previous.map(|id| Self::hero_page_arrow("create_post_hero_previous", "chevron.left", id, cx).left(px(8.0))))
                    .children(next.map(|id| Self::hero_page_arrow("create_post_hero_next", "chevron.right", id, cx).right(px(8.0))))
                    .child(
                        div()
                            .absolute()
                            .bottom(px(10.0))
                            .left_0()
                            .right_0()
                            .flex()
                            .justify_center()
                            .child(
                                hstack(6.0).px(px(8.0)).py(px(5.0)).rounded(px(10.0)).bg(black().opacity(0.35)).children(
                                    ids.iter().enumerate().map(|(index, id)| {
                                        let target = id.clone();
                                        plain(SharedString::from(format!("create_post_hero_dot_{id}")))
                                            .size(px(7.0))
                                            .rounded(px(3.5))
                                            .bg(white().opacity(if index == selected { 1.0 } else { 0.4 }))
                                            .on_press(Self::action(cx, move |page, _, cx| {
                                                page.hero_page_id = Some(target.clone());
                                                cx.notify();
                                            }))
                                    }),
                                ),
                            ),
                    )
            });
        div().w_full().flex().justify_center().child(pager)
    }

    fn hero_page_arrow(
        id: &'static str,
        icon: &'static str,
        target: String,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        div().id(id).absolute().top_0().bottom_0().flex().items_center().child(
            plain(SharedString::from(format!("{id}_button")))
                .size(px(30.0))
                .rounded(px(15.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(black().opacity(0.35))
                .children(IconUi::sized(icon, px(14.0), white(), false))
                .on_press(Self::action(cx, move |page, _, cx| {
                    page.hero_page_id = Some(target.clone());
                    cx.notify();
                })),
        )
    }

    fn hero_still_page(
        &self,
        item: &CreatePostMediaAttachment,
        placeholder_icon: &'static str,
        theme: &AppTheme,
        window_hero_pixels: i64,
    ) -> Div {
        if let Some(slide) = &item.remote_slide {
            return div().absolute().inset_0().bg(black()).child(self.remote_still(
                &item.id,
                slide,
                ContentMode::Fit,
                window_hero_pixels,
                theme,
            ));
        }
        if let Some(preview) = &item.preview {
            return div()
                .absolute()
                .inset_0()
                .bg(black())
                .child(img(preview.clone()).size_full().object_fit(ObjectFit::Contain));
        }
        div()
            .absolute()
            .inset_0()
            .bg(theme.bg._200)
            .flex()
            .items_center()
            .justify_center()
            .when(item.is_preview_loading, |page| page.child(progress_view().tint(theme.text.secondary)))
            .when(!item.is_preview_loading, |page| {
                page.children(IconUi::sized(placeholder_icon, px(26.0), theme.text.secondary, true))
            })
    }

    fn hero_image_page(&self, item: &CreatePostMediaAttachment, theme: &AppTheme, cx: &Context<Self>) -> Div {
        div()
            .absolute()
            .inset_0()
            .child(self.hero_still_page(item, "photo.fill", theme, self.hero_pixels))
            .child(self.hero_remove_button(item, cx))
    }

    fn hero_video_page(&self, item: &CreatePostMediaAttachment, theme: &AppTheme, cx: &Context<Self>) -> Div {
        div()
            .absolute()
            .inset_0()
            .child(self.hero_still_page(item, "video.fill", theme, self.hero_pixels))
            .when(item.preview.is_some() || item.remote_slide.is_some(), |page| {
                page.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Self::play_glyph(34.0)),
                )
            })
            .children(item.duration_text().map(|duration| {
                div()
                    .absolute()
                    .bottom(px(10.0))
                    .right(px(10.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .bg(black().opacity(0.55))
                    .rounded(px(8.0))
                    .child(MainText::new(duration).px(12.0).font_weight(FontWeight::SEMIBOLD).color(white()))
            }))
            .child(self.hero_remove_button(item, cx))
    }

    /// play icon with a drop shadow, a dimmed copy sits under it since icons have no shadow
    fn play_glyph(side: f32) -> Div {
        div()
            .relative()
            .size(px(side))
            .child(div().absolute().top(px(2.0)).left_0().children(IconUi::sized(
                "play.fill",
                px(side),
                black().opacity(0.35),
                true,
            )))
            .child(div().absolute().top_0().left_0().children(IconUi::sized("play.fill", px(side), white(), true)))
    }

    fn hero_add_more_row(&self, theme: &AppTheme, cx: &Context<Self>) -> Option<PlainButton> {
        let state = self.state.read(cx);
        if !state.can_add_more_media(cx) {
            return None;
        }
        let maximum = CreatePostScreenState::max_attachment_count(cx);
        Some(
            plain("create_post_hero_add_more")
                .w_full()
                .h(px(44.0))
                .px(px(14.0))
                .bg(theme.bg._200)
                .rounded(px(14.0))
                .disabled(self.is_publishing())
                .on_press(Self::action(cx, |page, _, cx| {
                    page.show_media_picker = true;
                    cx.notify();
                }))
                .child(
                    hstack(8.0)
                        .size_full()
                        .children(IconUi::sized("plus", px(13.0), theme.text.primary, false))
                        .child(MainText::new(tr(self.add_media_key(cx))).px(13.0).font_weight(FontWeight::SEMIBOLD))
                        .child(spacer())
                        .child(
                            MainText::new(format!("{}/{maximum}", state.media.len()))
                                .px(13.0)
                                .color(theme.text.secondary),
                        ),
                ),
        )
    }

    /// hero frame size,
    /// 4:5 card in single column or the pictures own shape fitted into the preview pane in the
    /// split sheet
    fn hero_frame(&self, item: Option<&CreatePostMediaAttachment>) -> (f32, f32) {
        let column = (HERO_MAX_WIDTH, HERO_MAX_WIDTH / HERO_ASPECT);
        if !self.hero_beside {
            return column;
        }
        let Some((pane_width, pane_height)) = self.hero_pane.get() else {
            return column;
        };
        let box_width = (pane_width - SPLIT_HERO_INSET * 2.0).max(HERO_MAX_WIDTH);
        let box_height = (pane_height - SPLIT_HERO_INSET * 2.0).max(HERO_MAX_WIDTH / HERO_ASPECT);
        let aspect = item.and_then(Self::hero_fitted_aspect).unwrap_or(HERO_ASPECT);
        if box_width / box_height > aspect {
            (box_height * aspect, box_height)
        } else {
            (box_width, box_width / aspect)
        }
    }

    fn hero_split_pane(&self, theme: &AppTheme, cx: &Context<Self>) -> Div {
        let cell = self.hero_pane.clone();
        let weak = cx.weak_entity();
        div()
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .bg(black())
            .flex()
            .items_center()
            .justify_center()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let measured = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                        if cell.get() != Some(measured) {
                            cell.set(Some(measured));
                            window.refresh();
                            let _ = weak.update(cx, |_, cx| cx.notify());
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .child(self.hero_media_pager(theme, cx))
    }

    /// width over height of the picture fitted into the 4:5 frame, none if unknown
    fn hero_fitted_aspect(item: &CreatePostMediaAttachment) -> Option<f32> {
        if let Some(preview) = &item.preview {
            let bitmap = preview.size(0);
            if bitmap.width.0 > 0 && bitmap.height.0 > 0 {
                return Some(bitmap.width.0 as f32 / bitmap.height.0 as f32);
            }
        }
        let size = item.remote_slide.as_ref()?.size()?;
        (size.width > 0 && size.height > 0).then(|| size.width as f32 / size.height as f32)
    }

    /// pinned to the corner of the fitted picture not the frame, black bars would leave the button hanging
    fn hero_remove_button(&self, item: &CreatePostMediaAttachment, cx: &Context<Self>) -> Div {
        let (frame_width, frame_height) = self.hero_frame(Some(item));
        let frame_aspect = frame_width / frame_height;
        let (bar_x, bar_y) = match Self::hero_fitted_aspect(item) {
            Some(aspect) if aspect >= frame_aspect => (0.0, ((frame_height - frame_width / aspect) / 2.0).max(0.0)),
            Some(aspect) => (((frame_width - frame_height * aspect) / 2.0).max(0.0), 0.0),
            None => (0.0, 0.0),
        };
        let id = item.id.clone();
        let target = item.id.clone();
        div()
            .absolute()
            .top(px(bar_y + 10.0))
            .right(px(bar_x + 10.0))
            .child(
                plain(SharedString::from(format!("create_post_hero_remove_{id}")))
                    .size(px(30.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(black().opacity(0.55))
                    .rounded(px(15.0))
                    .disabled(self.is_publishing())
                    .children(IconUi::sized("xmark", px(13.0), white(), false))
                    .on_press(Self::action(cx, move |page, _, cx| page.remove_media(&target, cx))),
            )
    }

    fn post_fields_group(&self, cx: &Context<Self>) -> GroupedBtns {
        GroupedBtns::new(
            "create_post_fields_group",
            vec![
                GroupedBtn::new("createPostTitle", "").content_slot(self.title_field_row(cx)),
                GroupedBtn::new("createPostContent", "").content_slot(self.content_field_row(cx)),
                GroupedBtn::new("createPostHashtags", "").content_slot(self.hashtags_field_row(cx)),
            ],
        )
        .separator_leading_padding(57.0)
    }

    fn field_setter(&self) -> impl Fn(&str, &str, &mut Window, &mut App) + 'static + use<> {
        let state = self.state.clone();
        move |name, value, _, cx| {
            state.update(cx, |state, cx| {
                state.set_value(name, value);
                cx.notify();
            });
        }
    }

    fn title_field_row(&self, cx: &Context<Self>) -> Div {
        hstack(12.0)
            .w_full()
            .pl(px(16.0))
            .pr(px(8.0))
            .py(px(6.0))
            .child(Self::grouped_field_icon("textformat", color("#008DE5")))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    InputUi::new("create_post_title")
                        .values(self.state.read(cx).values.clone())
                        .errors(HashMap::new())
                        .name(FIELD_TITLE)
                        .placeholder(tr("create_post_title_placeholder"))
                        .rule(PostFieldRules::title())
                        .focus_token(self.title_focus_token)
                        .on_focus_change(Self::focus_reporter(FIELD_TITLE, cx))
                        .set_value(self.field_setter()),
                ),
            )
            .children(self.emoji_toggle(FIELD_TITLE, cx))
    }

    fn content_field_row(&self, cx: &Context<Self>) -> Div {
        hstack(12.0)
            .items_start()
            .w_full()
            .pl(px(16.0))
            .pr(px(8.0))
            .py(px(6.0))
            .child(div().pt(px(10.0)).child(Self::grouped_field_icon("text.alignleft", color("#7E57F7"))))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    TextAreaUi::new("create_post_content")
                        .values(self.state.read(cx).values.clone())
                        .errors(HashMap::new())
                        .name(FIELD_CONTENT)
                        .placeholder(tr("create_post_content_placeholder"))
                        .min_lines(4)
                        .max_lines(10)
                        .formatting(true)
                        .rule(PostFieldRules::source())
                        .focus_token(self.content_focus_token)
                        .on_focus_change(Self::focus_reporter(FIELD_CONTENT, cx))
                        .set_value(self.field_setter()),
                ),
            )
            .children(self.emoji_toggle(FIELD_CONTENT, cx).map(|toggle| div().pt(px(6.0)).child(toggle)))
    }

    fn hashtags_field_row(&self, cx: &Context<Self>) -> Div {
        hstack(12.0)
            .w_full()
            .pl(px(16.0))
            .pr(px(8.0))
            .py(px(6.0))
            .child(Self::grouped_field_icon("number", color("#00A9BB")))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    InputUi::new("create_post_hashtags")
                        .values(self.state.read(cx).values.clone())
                        .errors(HashMap::new())
                        .name(FIELD_HASHTAGS)
                        .placeholder(tr("create_post_hashtags_placeholder"))
                        .format(|raw, previous| PostHashtagInputFormatting::format(raw, previous))
                        .set_value(self.field_setter()),
                ),
            )
    }

    /// circle key beside a focused field,
    /// swaps keyboard for emoji and back. hidden while neither field nor panel is active
    fn emoji_toggle(&self, field: &'static str, cx: &Context<Self>) -> Option<AnyElement> {
        if self.focused_field != Some(field) && self.emoji_field != Some(field) {
            return None;
        }
        let theme = rise_ui::theme(cx);
        let symbol = if self.emoji_field == Some(field) { "keyboard" } else { "face.smiling" };
        let key = div()
            .id(SharedString::from(format!("create_post_emoji_toggle_{field}")))
            .size(px(EMOJI_TOGGLE_SIDE))
            .flex_none()
            .rounded_full()
            .overflow_hidden()
            .bg(theme.bg._200)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(symbol, px(18.0), theme.text.secondary, false));
        let action: ButtonAction = Rc::new(Self::action(cx, move |page, _, cx| page.toggle_emoji_panel(field, cx)));
        Some(
            button_chrome(key, false, Some(action))
                .tooltip_label(tr("picker_tab_emoji"))
                .keyboard_dismiss_excluded()
                .into_any_element(),
        )
    }

    fn emoji_panel_height(&self) -> f32 {
        if self.last_keyboard_height > 100.0 { self.last_keyboard_height } else { DEFAULT_EMOJI_PANEL_HEIGHT }
    }

    fn post_emoji_panel(&self, cx: &mut Context<Self>) -> Div {
        let (flat_chrome, bg_100) = {
            let theme_store = ThemeStore::shared(cx);
            let theme_store = theme_store.read(cx);
            (theme_store.resolved_appearance() == RiseThemeAppearance::Light, theme_store.current_theme().bg._100)
        };
        let insert = cx.weak_entity();
        let backspace = cx.weak_entity();
        let tab = ChatInputEmojiTab::new(
            "create_post_emoji_tab",
            move |emoji: String, _: &mut Window, cx: &mut App| {
                let _ = insert.update(cx, |page, cx| page.insert_post_emoji(&emoji, cx));
            },
            |_: StickerItem, _: &mut Window, _: &mut App| {},
            move |_: &mut Window, cx: &mut App| {
                let _ = backspace.update(cx, |page, cx| page.delete_post_emoji_backward(cx));
            },
        )
        .safe_area_bottom(0.0)
        .is_active(self.emoji_field.is_some())
        .reserves_picker_bottom_bar(false)
        .uses_flat_chrome(flat_chrome)
        .system_emoji_only(true);
        let background = if flat_chrome {
            div().absolute().inset_0().bg(bg_100).into_any_element()
        } else {
            div()
                .absolute()
                .inset_0()
                .child(HoldContextMenuGlass::new("create_post_emoji_glass").tint(black().opacity(0.34)))
                .into_any_element()
        };
        div().w_full().h(px(self.emoji_panel_height())).child(
            div()
                .relative()
                .size_full()
                .occlude()
                .rounded_tl(px(EMOJI_PANEL_RADIUS))
                .rounded_tr(px(EMOJI_PANEL_RADIUS))
                .overflow_hidden()
                .child(background)
                .child(tab)
                .keyboard_dismiss_excluded(),
        )
    }

    fn set_emoji_field(&mut self, field: Option<&'static str>, cx: &mut Context<Self>) {
        self.emoji_field = field;
        self.emoji_presence.set_in(
            field.map(|_| ()),
            ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
    }

    fn toggle_emoji_panel(&mut self, field: &'static str, cx: &mut Context<Self>) {
        let _ = selection_changed();
        if self.emoji_field == Some(field) {
            self.focused_field = Some(field);
            self.set_emoji_field(None, cx);
            self.focus_field(field, cx);
            return;
        }
        let text = self.state.read(cx).value(field).to_owned();
        self.emoji_caret = RiseFocusedTextInput::caret(&text, cx);
        self.emoji_caret_text = text;
        self.set_emoji_field(Some(field), cx);
        KeyboardDismiss::perform(cx);
    }

    fn focus_field(&mut self, field: &'static str, cx: &mut Context<Self>) {
        if field == FIELD_TITLE {
            self.title_focus_token = self.title_focus_token.wrapping_add(1);
        } else {
            self.content_focus_token = self.content_focus_token.wrapping_add(1);
        }
        cx.notify();
    }

    fn focus_reporter(field: &'static str, cx: &Context<Self>) -> impl Fn(bool, &mut Window, &mut App) + 'static {
        let weak = cx.weak_entity();
        move |is_focused, _, cx| {
            let weak = weak.clone();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |page, cx| page.field_focus_changed(field, is_focused, cx));
            });
        }
    }

    fn field_focus_changed(&mut self, field: &'static str, is_focused: bool, cx: &mut Context<Self>) {
        if is_focused {
            self.focused_field = Some(field);
            if self.emoji_field.is_some() {
                self.set_emoji_field(None, cx);
            }
        } else if self.focused_field == Some(field) {
            self.focused_field = None;
        }
        cx.notify();
    }

    fn insert_post_emoji(&mut self, emoji: &str, cx: &mut Context<Self>) {
        self.edit_emoji_field(
            |text, caret| {
                let mut drafts: Vec<ChatDraftCustomEmoji> = Vec::new();
                Some(ChatComposerCaretEdit::replace(text, caret, emoji, &mut drafts, None))
            },
            cx,
        );
    }

    fn delete_post_emoji_backward(&mut self, cx: &mut Context<Self>) {
        self.edit_emoji_field(
            |text, caret| {
                let mut drafts: Vec<ChatDraftCustomEmoji> = Vec::new();
                ChatComposerCaretEdit::delete_backward(text, caret, &mut drafts)
            },
            cx,
        );
    }

    /// one panel edit at the remembered caret,
    /// an insert past the canon limit is dropped whole not cut into half an emoji
    fn edit_emoji_field(
        &mut self,
        edit: impl FnOnce(&str, Option<NSRange>) -> Option<ChatComposerCaretEditResult>,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.emoji_field else {
            return;
        };
        let text = self.state.read(cx).value(field).to_owned();
        let caret = if self.emoji_caret_text == text { self.emoji_caret } else { None };
        let Some(result) = edit(&text, caret) else {
            return;
        };
        let rule = if field == FIELD_TITLE { PostFieldRules::title() } else { PostFieldRules::source() };
        if rule.clamped(&result.text) != result.text {
            return;
        }
        self.update_state(cx, |state| state.set_value(field, &result.text));
        self.emoji_caret = Some(result.caret);
        self.emoji_caret_text = result.text;
    }

    fn grouped_field_icon(system_name: &'static str, fill: Hsla) -> Div {
        div()
            .size(px(29.0))
            .flex_none()
            .rounded(px(8.0))
            .bg(fill)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(system_name, px(14.0), white(), false))
    }

    fn switch(&self, id: impl Into<gpui::ElementId>, value: Binding<bool>) -> SwitchUi {
        SwitchUi::new(id, value)
    }

    fn post_options_group(&self, theme: &AppTheme, cx: &Context<Self>) -> GroupedBtns {
        GroupedBtns::new("create_post_options_group", self.post_option_items(theme, cx))
    }

    fn post_option_items(&self, theme: &AppTheme, cx: &Context<Self>) -> Vec<GroupedBtn> {
        let state = self.state.read(cx);
        let mut items = vec![
            GroupedBtn::new("createPostComments", tr("create_post_can_comment"))
                .leading_slot(CreatePostCommentsDisabledGlyph { background: SettingsIconPalette::privacy_messages() })
                .disabled(self.is_publishing())
                .guide_anchor(RiseGuideAnchor::CreatePostComments)
                .show_chevron(false)
                .trailing_slot(self.switch(
                    "create_post_comments_switch",
                    Binding::entity(self.state.clone(), |state| state.disable_comments, |state, value, _| {
                        state.disable_comments = value;
                    }),
                ))
                .callback(Self::action(cx, |page, _, cx| {
                    page.update_state(cx, |state| state.disable_comments = !state.disable_comments);
                })),
        ];

        if let Some(post) = &self.editing_post {
            if self.shows_edit_placement_rows(post, cx) {
                items.push(self.publications_placement_item(
                    "editPostPlacementPublications",
                    tr("post_placement_publications"),
                    true,
                    cx,
                ));
                items.push(self.riffs_placement_item(
                    "editPostPlacementRiffs",
                    tr_args("post_placement_riffs", &[FormatArg::from(tr("riffs"))]),
                    true,
                    cx,
                ));
            }
        } else if state.shows_create_placement_row() {
            if state.is_riff_composer() {
                items.push(self.publications_placement_item(
                    "createPostAlsoPublication",
                    tr("create_post_also_publication"),
                    false,
                    cx,
                ));
            } else {
                items.push(self.riffs_placement_item(
                    "createPostAlsoRiff",
                    tr_args("create_post_also_riff", &[FormatArg::from(tr("riffs"))]),
                    false,
                    cx,
                ));
            }
        }

        if self.editing_post.is_none() {
            items.push(self.coauthors_grouped_item(cx));
        }

        if self.editing_post.is_none() && state.has_taggable_media() {
            items.push(self.tag_people_grouped_item(cx));
        }

        if state.content_kind == RisePostContentKind::Publication && !state.is_achievement_share() {
            items.push(self.music_grouped_item(theme, cx));
        }
        items
    }

    /// placement is editable only on a publication with media,
    /// a thread is never a riff and music alone cant be placed
    fn shows_edit_placement_rows(&self, post: &GetPostFeedResponse, cx: &App) -> bool {
        let state = self.state.read(cx);
        state.content_kind == RisePostContentKind::Publication
            && !state.is_achievement_share()
            && if state.edits_media { !state.media.is_empty() } else { post.media_slide_count() > 0 }
    }

    /// locks_last_placement greys the row while its the only placement on,
    /// so the switch shows why it cant turn off
    fn publications_placement_item(
        &self,
        id: &'static str,
        text: String,
        locks_last_placement: bool,
        cx: &Context<Self>,
    ) -> GroupedBtn {
        let state = self.state.read(cx);
        let is_last_placement = state.requests_publications && !state.requests_riff;
        GroupedBtn::new(id, text)
            .icon("square.grid.3x3.fill")
            .icon_background(color("#2F8FE5"))
            .disabled(self.is_publishing() || (locks_last_placement && is_last_placement))
            .show_chevron(false)
            .trailing_slot(self.switch(
                SharedString::from(format!("{id}_switch")),
                Binding::entity(self.state.clone(), |state| state.requests_publications, |state, value, _| {
                    state.set_requests_publications(value);
                }),
            ))
            .callback(Self::action(cx, |page, _, cx| {
                page.update_state(cx, |state| state.set_requests_publications(!state.requests_publications));
            }))
    }

    fn riffs_placement_item(
        &self,
        id: &'static str,
        text: String,
        locks_last_placement: bool,
        cx: &Context<Self>,
    ) -> GroupedBtn {
        let state = self.state.read(cx);
        let is_last_placement = state.requests_riff && !state.requests_publications;
        GroupedBtn::new(id, text)
            .icon("photo.stack")
            .icon_background(color("#7E57F7"))
            .disabled(self.is_publishing() || (locks_last_placement && is_last_placement))
            .guide_anchor((id == "createPostAlsoRiff").then_some(RiseGuideAnchor::CreatePostAlsoRiff))
            .show_chevron(false)
            .trailing_slot(self.switch(
                SharedString::from(format!("{id}_switch")),
                Binding::entity(self.state.clone(), |state| state.requests_riff, |state, value, _| {
                    state.set_requests_riff(value);
                }),
            ))
            .callback(Self::action(cx, |page, _, cx| {
                page.update_state(cx, |state| state.set_requests_riff(!state.requests_riff));
            }))
    }

    fn coauthors_grouped_item(&self, cx: &Context<Self>) -> GroupedBtn {
        let count = self.state.read(cx).coauthors.len();
        let item = GroupedBtn::new("createPostCoauthors", tr("create_post_coauthors_row"))
            .icon("person.2.fill")
            .icon_background(SettingsIconPalette::privacy_friends())
            .disabled(self.is_publishing())
            .guide_anchor(RiseGuideAnchor::CreatePostCoauthors)
            .callback(Self::action(cx, |page, _, cx| page.present_coauthor_picker(cx)));
        if count == 0 {
            item
        } else {
            item.trailing(tr_args("create_post_people_selected", &[FormatArg::from(count)]))
        }
    }

    fn tag_people_grouped_item(&self, cx: &Context<Self>) -> GroupedBtn {
        let count = self.state.read(cx).tagged_people_count();
        let item = GroupedBtn::new("createPostTagPeople", tr("create_post_tag_people_row"))
            .icon("person.crop.circle.badge.plus")
            .icon_background(SettingsIconPalette::profile())
            .disabled(self.is_publishing())
            .guide_anchor(RiseGuideAnchor::CreatePostTagPeople)
            .callback(Self::action(cx, |page, _, cx| page.present_tag_people_picker(cx)));
        if count == 0 {
            item
        } else {
            item.trailing(tr_args("create_post_people_selected", &[FormatArg::from(count)]))
        }
    }

    fn present_coauthor_picker(&mut self, cx: &mut Context<Self>) {
        let me = AuthServicesStore::try_read(cx)
            .and_then(|auth| auth.profile())
            .map(|profile| profile.id.clone())
            .unwrap_or_default();
        let excluded: HashSet<String> = if me.is_empty() { HashSet::new() } else { HashSet::from([me]) };
        let preselected = self.state.read(cx).coauthors.clone();
        let state = self.state.clone();
        PeoplePickerStore::shared(cx).update(cx, |store, cx| {
            store.present(
                PeoplePickerMode::SearchAll,
                CreatePostScreenState::collaborator_limits().coauthors,
                preselected,
                excluded,
                tr("post_coauthor_invite_action"),
                None,
                PeoplePickerHostContext::Root,
                move |picked, cx| {
                    state.update(cx, |state, cx| {
                        state.coauthors = picked;
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }

    fn present_tag_people_picker(&mut self, cx: &mut Context<Self>) {
        let preselected = self.state.read(cx).all_tagged_people();
        let preselected_ids: HashSet<String> = preselected.iter().map(|user| user.id.clone()).collect();
        let state = self.state.clone();
        PeoplePickerStore::shared(cx).update(cx, |store, cx| {
            store.present(
                PeoplePickerMode::Friends,
                CreatePostScreenState::collaborator_limits().tagged_users,
                preselected,
                HashSet::new(),
                tr("post_tag_people_action"),
                Some(tr("people_picker_friends_only_hint")),
                PeoplePickerHostContext::Root,
                move |picked, cx| {
                    state.update(cx, |state, cx| {
                        let picked_ids: HashSet<String> = picked.iter().map(|user| user.id.clone()).collect();
                        let removed: HashSet<String> = preselected_ids.difference(&picked_ids).cloned().collect();
                        state.remove_tagged_people(&removed);
                        let placed = state.placed_tag_user_ids();
                        let mut known: HashSet<String> = state.tagged_people.iter().map(|user| user.id.clone()).collect();
                        for user in picked {
                            if !placed.contains(&user.id) && known.insert(user.id.clone()) {
                                state.tagged_people.push(user);
                            }
                        }
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }

    fn post_counters_group(&self, cx: &Context<Self>) -> GroupedBtns {
        GroupedBtns::new("create_post_counters_group", self.post_counter_items(cx))
    }

    fn post_counter_items(&self, cx: &Context<Self>) -> Vec<GroupedBtn> {
        let rows: [(&'static str, &'static str, Hsla, &'static str, CountVisibilityPath); 5] = [
            (
                "createPostShowLikes",
                "heart.fill",
                SettingsIconPalette::subscription(),
                "create_post_show_likes_count",
                CountVisibilityPath::LIKES,
            ),
            (
                "createPostShowComments",
                "bubble.left.fill",
                SettingsIconPalette::privacy_messages(),
                "create_post_show_comments_count",
                CountVisibilityPath::COMMENTS,
            ),
            (
                "createPostShowReposts",
                "arrow.2.squarepath",
                color("#7E57F7"),
                "create_post_show_reposts_count",
                CountVisibilityPath::REPOSTS,
            ),
            (
                "createPostShowShares",
                "paperplane.fill",
                color("#2F8FE5"),
                "create_post_show_shares_count",
                CountVisibilityPath::SHARES,
            ),
            (
                "createPostShowCommentDislikes",
                "hand.thumbsdown.fill",
                hsla(0.0, 0.0, 0.42, 1.0),
                "create_post_show_comment_dislikes",
                CountVisibilityPath::COMMENT_DISLIKES,
            ),
        ];
        rows.into_iter()
            .map(|(id, icon, tint, key, path)| {
                self.sparks_switch_item(id, key, path, cx).icon(icon).icon_background(tint)
            })
            .collect()
    }

    fn post_sparks_group(&self, cx: &Context<Self>) -> GroupedBtns {
        let allow = self
            .sparks_switch_item("createPostAllowSparks", "create_post_allow_sparks", CountVisibilityPath::ALLOWS_SPARKS, cx)
            .leading_slot(SparksSettingsTile::new().solid(true));
        let show = self
            .sparks_switch_item("createPostShowSparks", "create_post_show_sparks", CountVisibilityPath::SPARKS, cx)
            .icon("sparkles")
            .icon_background(SettingsIconPalette::sparks());
        GroupedBtns::new("create_post_sparks_group", vec![allow, show])
    }

    fn post_download_group(&self, cx: &Context<Self>) -> GroupedBtns {
        let allow = self
            .sparks_switch_item(
                "createPostAllowDownload",
                "create_post_allow_download",
                CountVisibilityPath::ALLOWS_DOWNLOAD,
                cx,
            )
            .icon("arrow.down.circle.fill")
            .icon_background(SettingsIconPalette::memory_videos())
            .trailing_slot(self.switch(
                "createPostAllowDownload_switch",
                Binding::entity(
                    self.state.clone(),
                    |state| state.count_visibility.allows_download,
                    |state, value, _| Self::set_allows_download(state, value),
                ),
            ))
            .callback(Self::action(cx, |page, _, cx| {
                page.update_state(cx, |state| {
                    let on = !state.count_visibility.allows_download;
                    Self::set_allows_download(state, on);
                });
            }));
        if !self.state.read(cx).count_visibility.allows_download {
            return GroupedBtns::new("create_post_download_group", vec![allow]);
        }
        let bare = self
            .sparks_switch_item(
                "createPostAllowDownloadWithoutWatermark",
                "create_post_allow_download_without_watermark",
                CountVisibilityPath::ALLOWS_DOWNLOAD_WITHOUT_WATERMARK,
                cx,
            )
            .icon("drop.fill")
            .icon_background(SettingsIconPalette::memory_images());
        GroupedBtns::new("create_post_download_group", vec![allow, bare])
    }

    fn set_allows_download(state: &mut CreatePostScreenState, on: bool) {
        state.count_visibility.allows_download = on;
        if !on {
            state.count_visibility.allows_download_without_watermark = false;
        }
    }

    fn sparks_switch_item(
        &self,
        id: &'static str,
        key: &'static str,
        path: CountVisibilityPath,
        cx: &Context<Self>,
    ) -> GroupedBtn {
        GroupedBtn::new(id, tr(key))
            .disabled(self.is_publishing())
            .show_chevron(false)
            .trailing_slot(self.switch(
                SharedString::from(format!("{id}_switch")),
                Binding::entity(
                    self.state.clone(),
                    move |state| (path.get)(&state.count_visibility),
                    move |state, value, _| (path.set)(&mut state.count_visibility, value),
                ),
            ))
            .callback(Self::action(cx, move |page, _, cx| {
                page.update_state(cx, |state| {
                    let value = (path.get)(&state.count_visibility);
                    (path.set)(&mut state.count_visibility, !value);
                });
            }))
    }

    fn music_grouped_item(&self, theme: &AppTheme, cx: &Context<Self>) -> GroupedBtn {
        if let Some(track) = &self.state.read(cx).attached_track {
            return GroupedBtn::new("createPostMusic", "")
                .disabled(self.is_publishing())
                .content_slot(self.attached_music_row(track, theme, cx));
        }
        GroupedBtn::new("createPostMusic", tr("post_attach_music"))
            .icon("music.note")
            .icon_background(SettingsIconPalette::memory_audio())
            .disabled(self.is_publishing())
            .guide_anchor(RiseGuideAnchor::CreatePostMusic)
            .callback(Self::action(cx, |page, _, cx| {
                page.show_track_picker = true;
                cx.notify();
            }))
    }

    fn attached_music_row(&self, track: &RiseStoredMusicTrack, theme: &AppTheme, cx: &Context<Self>) -> Div {
        hstack(10.0)
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(
                plain("create_post_attached_music")
                    .flex_1()
                    .min_w(px(0.0))
                    .on_press(Self::action(cx, |page, _, cx| {
                        page.show_track_picker = true;
                        cx.notify();
                    }))
                    .child(
                        hstack(12.0)
                            .w_full()
                            .child(
                                MusicTrackCoverImage::new(
                                    "create_post_attached_music_cover",
                                    RemoteMediaURL::optional_normalized(track.cover_url.as_deref()).unwrap_or_default(),
                                )
                                .size(40.0)
                                .corner_radius(8.0),
                            )
                            .child(
                                div().flex_1().min_w(px(0.0)).child(
                                    MainText::new(self.attached_track_chip_text(track, cx))
                                        .px(14.0)
                                        .font_weight(FontWeight::MEDIUM)
                                        .number_of_lines(1),
                                ),
                            ),
                    ),
            )
            .child(
                plain("create_post_attached_music_remove")
                    .size(px(28.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg._300)
                    .rounded(px(14.0))
                    .children(IconUi::sized("xmark", px(12.0), theme.text.secondary, false))
                    .on_press(Self::action(cx, |page, _, cx| page.remove_attached_track(cx))),
            )
    }

    fn remove_attached_track(&mut self, cx: &mut Context<Self>) {
        self.update_state(cx, |state| {
            state.attached_track = None;
            state.attached_track_start_ms = None;
            state.attached_track_end_ms = None;
            state.attached_track_plate = None;
        });
    }

    fn publish_group(&self, theme: &AppTheme, cx: &Context<Self>) -> GroupedBtns {
        let item = GroupedBtn::new("createPostPublish", self.publish_button_title(cx))
            .icon("paperplane.fill")
            .icon_background(theme.primary._100)
            .disabled(self.state.read(cx).disabled(cx) || self.is_publishing())
            .guide_anchor(RiseGuideAnchor::CreatePostPublish)
            .show_chevron(false)
            .callback(Self::action(cx, |page, _, cx| page.publish_post(cx)));
        let item = if self.is_publishing() {
            item.trailing_slot(progress_view().tint(theme.text.secondary))
        } else {
            item
        };
        GroupedBtns::new("create_post_publish_group", vec![item])
    }

    fn screen_title_key(&self, cx: &App) -> &'static str {
        if self.editing_post.is_some() {
            return "edit_action";
        }
        let state = self.state.read(cx);
        if state.is_riff_composer() && state.requests_riff { "create_riff_title" } else { "create_post_text" }
    }

    fn publish_button_title(&self, cx: &App) -> String {
        if self.editing_post.is_some() {
            return tr("common_save");
        }
        if self.state.read(cx).publishes_riff_only() {
            return tr_args("create_post_publish_short", &[FormatArg::from(tr("riffs"))]);
        }
        tr("create_post_btn_text")
    }

    fn riffs_badge(theme: &AppTheme) -> Div {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .bg(theme.bg._200)
            .rounded(px(100.0))
            .child(MainText::new(tr("riffs")).px(11.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
    }

    fn add_media_tile(&self, theme: &AppTheme, cx: &Context<Self>) -> impl IntoElement {
        let can_add_more = self.state.read(cx).can_add_more_media(cx);
        plain("create_post_add_media")
            .size(px(TILE_SIDE))
            .flex_none()
            .when(!can_add_more, |tile| tile.opacity(0.45))
            .disabled(self.is_publishing())
            .on_press(Self::action(cx, |page, _, cx| {
                let state = page.state.read(cx);
                if state.can_add_more_media(cx) {
                    page.show_media_picker = true;
                    cx.notify();
                    return;
                }
                let message = state.media_limit_message(cx);
                Notifier::send(
                    tr("premium_upsell_title"),
                    NotifierSendOptions {
                        content: Some(message),
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }))
            .child(
                div()
                    .size_full()
                    .rounded(px(14.0))
                    .bg(theme.bg._200)
                    .border(px(1.5))
                    .border_dashed()
                    .border_color(theme.input.border_300)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .children(IconUi::sized("photo.on.rectangle.angled", px(22.0), theme.text.secondary, false))
                    .child(
                        div().px(px(6.0)).child(
                            MainText::new(tr(self.add_media_key(cx)))
                                .px(11.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.secondary),
                        ),
                    ),
            )
            .guide_anchor(RiseGuideAnchor::CreatePostAddMedia)
    }

    /// a post holds one video so once attached the picker only adds photos
    fn add_media_key(&self, cx: &App) -> &'static str {
        if self.state.read(cx).has_video() { "create_post_add_photos" } else { "create_post_add_media" }
    }

    fn tile_remove_button(&self, item: &CreatePostMediaAttachment, cx: &Context<Self>) -> Div {
        let target = item.id.clone();
        div().absolute().top(px(6.0)).right(px(6.0)).child(
            plain(SharedString::from(format!("create_post_tile_remove_{}", item.id)))
                .size(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(black().opacity(0.55))
                .rounded(px(12.0))
                .disabled(self.is_publishing())
                .children(IconUi::sized("xmark", px(11.0), white(), false))
                .on_press(Self::action(cx, move |page, _, cx| page.remove_media(&target, cx))),
        )
    }

    fn tile_still(
        &self,
        item: &CreatePostMediaAttachment,
        placeholder_icon: &'static str,
        theme: &AppTheme,
    ) -> gpui::AnyElement {
        if let Some(slide) = &item.remote_slide {
            return self.remote_tile(&item.id, slide, theme).into_any_element();
        }
        if let Some(preview) = &item.preview {
            return div()
                .size(px(TILE_SIDE))
                .rounded(px(14.0))
                .overflow_hidden()
                .child(img(preview.clone()).size_full().object_fit(ObjectFit::Cover))
                .into_any_element();
        }
        Self::preview_placeholder(placeholder_icon, item.is_preview_loading, theme).into_any_element()
    }

    fn video_tile(&self, item: &CreatePostMediaAttachment, theme: &AppTheme, cx: &Context<Self>) -> Div {
        div()
            .relative()
            .size(px(TILE_SIDE))
            .flex_none()
            .child(self.tile_still(item, "video.fill", theme))
            .when(item.preview.is_some() || item.remote_slide.is_some(), |tile| {
                tile.child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(Self::play_glyph(24.0)),
                )
            })
            .children(item.duration_text().map(|duration| {
                div()
                    .absolute()
                    .bottom(px(6.0))
                    .right(px(6.0))
                    .px(px(6.0))
                    .py(px(3.0))
                    .bg(black().opacity(0.55))
                    .rounded(px(6.0))
                    .child(MainText::new(duration).px(11.0).font_weight(FontWeight::SEMIBOLD).color(white()))
            }))
            .child(self.tile_remove_button(item, cx))
    }

    fn media_tile(&self, item: &CreatePostMediaAttachment, theme: &AppTheme, cx: &Context<Self>) -> Div {
        div()
            .relative()
            .size(px(TILE_SIDE))
            .flex_none()
            .child(self.tile_still(item, "photo.fill", theme))
            .child(self.tile_remove_button(item, cx))
    }

    fn preview_placeholder(icon: &'static str, is_loading: bool, theme: &AppTheme) -> Div {
        div()
            .size(px(TILE_SIDE))
            .rounded(px(14.0))
            .bg(theme.bg._200)
            .flex()
            .items_center()
            .justify_center()
            .when(is_loading, |tile| tile.child(progress_view().tint(theme.text.secondary)))
            .when(!is_loading, |tile| tile.children(IconUi::sized(icon, px(22.0), theme.text.secondary, true)))
    }

    /// a slide the post already has,
    /// photo or video poster. the box keeps its geometry while loading,
    /// filled with skeleton color
    fn remote_still(
        &self,
        id: &str,
        slide: &PostMediaSlide,
        content_mode: ContentMode,
        pixel_dimension: i64,
        theme: &AppTheme,
    ) -> gpui::AnyElement {
        if let Some(url) = slide.poster_url().filter(|url| !url.is_empty()) {
            return CleverImageUi::new(url.to_owned())
                .id(SharedString::from(format!("create_post_remote_{id}_{pixel_dimension}")))
                .content_mode(content_mode)
                .corner_radius(0.0)
                .persist_to_disk(true)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(pixel_dimension)
                .into_any_element();
        }
        div()
            .size_full()
            .bg(SkeletonUi::shape_color(theme))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(
                if slide.is_video() { "video.fill" } else { "photo.fill" },
                px(22.0),
                theme.text.secondary,
                true,
            ))
            .into_any_element()
    }

    fn remote_tile(&self, id: &str, slide: &PostMediaSlide, theme: &AppTheme) -> Div {
        div().size(px(TILE_SIDE)).rounded(px(14.0)).overflow_hidden().child(self.remote_still(
            id,
            slide,
            ContentMode::Fill,
            CreatePostPreviewDecoder::tile_pixel_dimension(),
            theme,
        ))
    }

    fn publish_post(&mut self, cx: &mut Context<Self>) {
        if self.state.read(cx).disabled(cx) || self.is_publishing() {
            return;
        }
        self.is_submitting = true;
        cx.notify();

        let state = self.state.read(cx);
        let title = state.value(FIELD_TITLE).to_owned();
        let content = state.value(FIELD_CONTENT).to_owned();
        let hashtags = state.value(FIELD_HASHTAGS).to_owned();
        let can_comment = !state.disable_comments;
        let track = state.attached_track.clone();
        let track_id = track.as_ref().and_then(|track| track.id.clone());
        let track_start_ms = state.attached_track_start_ms;
        let track_end_ms = state.attached_track_end_ms;
        let track_plate = state.attached_track_plate.clone();
        let count_visibility = state.count_visibility;

        if let Some(post) = self.editing_post.clone() {
            let placement_changed = self.shows_edit_placement_rows(&post, cx)
                && (state.requests_riff != post.is_riff || state.requests_publications != post.show_in_publications);
            let requests_riff = state.requests_riff;
            let requests_publications = state.requests_publications;
            let edited_media = state.edited_media_items();
            let edited_tags = state.edited_tag_drafts(&post);
            self.pop_after_submit(cx);
            PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.update_post(
                    &post,
                    &title,
                    &content,
                    &hashtags,
                    can_comment,
                    track_id,
                    track,
                    track_start_ms,
                    track_end_ms,
                    track_plate,
                    Some(count_visibility),
                    placement_changed.then_some(requests_riff),
                    placement_changed.then_some(requests_publications),
                    edited_media,
                    edited_tags,
                    None,
                    Some(Box::new(|message: String, cx: &mut App| {
                        Self::report_publish_error("edit_post_error_text", &message, cx);
                    })),
                    cx,
                );
            });
            return;
        }

        let content_kind = state.content_kind;
        let photos = state.photo_files();
        let video = state.video_attachment().and_then(|attachment| attachment.picker_item.clone());
        let video_position = if video.is_none() { None } else { state.video_position() };
        let is_riff = content_kind == RisePostContentKind::Publication && state.requests_riff;
        let show_in_publications = !is_riff || state.requests_publications;
        let achievement_id = state.shared_achievement_id;
        let map_journey_id = self.map_journey_id.clone();
        let map_journey_show_heat = state.map_journey_show_heat;
        let coauthors = state.coauthors.clone();
        let tagged_users = state.merged_tag_drafts();
        let refused_values = state.values.clone();
        let disable_comments = state.disable_comments;
        self.pop_after_submit(cx);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.publish(
                content_kind,
                &title,
                &content,
                &hashtags,
                can_comment,
                photos,
                video,
                video_position,
                is_riff,
                show_in_publications,
                achievement_id,
                map_journey_id,
                map_journey_show_heat,
                track_id,
                track,
                track_start_ms,
                track_end_ms,
                track_plate,
                count_visibility,
                coauthors,
                tagged_users,
                None,
                Some(Rc::new(move |message: String, cx: &mut App| {
                    Self::keep_refused_draft(content_kind, &refused_values, disable_comments, cx);
                    Self::report_publish_error("create_post_error_text", &message, cx);
                })),
                cx,
            );
        });
    }

    fn keep_refused_draft(
        content_kind: RisePostContentKind,
        values: &HashMap<String, String>,
        disable_comments: bool,
        cx: &mut App,
    ) {
        if !RestrictionStore::shared(cx).read(cx).is_presented() {
            return;
        }
        let Some(user_id) = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire() else {
            return;
        };
        let draft = CreatePostRefusedDraft { user_id, content_kind, values: values.clone(), disable_comments };
        REFUSED_DRAFT.with(|slot| *slot.borrow_mut() = Some(draft));
    }

    fn restore_refused_draft(seeded: &mut CreatePostScreenState, cx: &mut App) {
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let Some(draft) = REFUSED_DRAFT.with(|slot| slot.borrow_mut().take()) else {
            return;
        };
        if user_id.as_deref() != Some(draft.user_id.as_str()) || draft.content_kind != seeded.content_kind {
            return;
        }
        seeded.values = draft.values;
        seeded.disable_comments = draft.disable_comments;
    }

    fn report_publish_error(title_key: &str, message: &str, cx: &mut App) {
        if RestrictionStore::shared(cx).read(cx).is_presented() {
            return;
        }
        Notifier::send(
            tr(title_key),
            NotifierSendOptions {
                content: Some(Self::localized_publish_error(message)),
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    /// media step leaves the picker page underneath so publishing must land on the origin screen
    /// not the spent picker
    fn pop_after_submit(&self, cx: &mut Context<Self>) {
        let Some(nav_store) = self.nav_store.clone() else {
            return;
        };
        let came_from_media_step = self.came_from_media_step;
        nav_store.update(cx, |nav_store, cx| {
            nav_store.pop(cx);
            if came_from_media_step {
                nav_store.pop(cx);
            }
        });
    }

    fn handle_media_picker_result(&mut self, result: MediaPickerResult, window: &Window, cx: &mut Context<Self>) {
        self.show_media_picker = false;
        cx.notify();
        if self.state.read(cx).content_kind != RisePostContentKind::Publication {
            return;
        }
        if result.was_cancelled || result.items.is_empty() {
            return;
        }

        if let Some(attached) = result.attached_track.clone() {
            self.update_state(cx, |state| {
                state.attached_track = Some(attached.track);
                state.attached_track_start_ms = Some(attached.start_ms);
                state.attached_track_end_ms = Some(attached.end_ms);
                state.attached_track_plate = attached.plate;
            });
        }

        let state = self.state.read(cx);
        let mut capacity = (CreatePostScreenState::max_attachment_count(cx) - state.media.len() as i64).max(0);
        let mut video_slots =
            (CreatePostScreenState::max_video_attachments() - i64::from(state.has_video())).max(0);
        let mut known_ids: HashSet<String> = state.media.iter().map(|item| item.id.clone()).collect();
        let mut added: Vec<CreatePostMediaAttachment> = Vec::new();
        let mut image_data: HashMap<String, Vec<u8>> = HashMap::new();
        let mut picked_video: Option<MediaPickerItem> = None;
        let mut refused_long_video = false;
        for item in &result.items {
            if capacity <= 0 {
                break;
            }
            if !known_ids.insert(item.id.clone()) {
                continue;
            }
            match item.kind {
                MediaPickerItemKind::Image => {
                    added.push(Self::attachment(item));
                    if let Some(payload) = &item.image_payload {
                        image_data.insert(item.id.clone(), payload.data.clone());
                    }
                }
                MediaPickerItemKind::Video => {
                    if video_slots <= 0 {
                        continue;
                    }
                    if let Some(payload) = &item.video_payload
                        && payload.duration > ShortVideoLimits::MAX_DURATION_SECONDS as f64 + 0.05
                    {
                        refused_long_video = true;
                        continue;
                    }
                    video_slots -= 1;
                    added.push(Self::attachment(item));
                    picked_video = Some(item.clone());
                }
                MediaPickerItemKind::File => continue,
            }
            capacity -= 1;
        }

        if refused_long_video {
            NativeConfirmAlert::present_notice(
                &tr("create_post_error_text"),
                &tr("create_post_short_too_long"),
                None,
                None,
                |_| {},
                cx,
            );
        }
        if added.is_empty() {
            return;
        }
        let image_inputs_source = added.clone();
        self.update_state(cx, |state| state.media.extend(added));
        let state = self.state.read(cx);
        if self.hero_page_id.is_none() {
            self.hero_page_id = state.media.first().map(|item| item.id.clone());
        }

        let dimension = if self.came_from_media_step
            || state.edits_media
            || (self.editing_post.is_none() && !state.is_riff_composer())
        {
            CreatePostPreviewDecoder::hero_pixel_dimension_for_attachment_count(state.media.len(), window)
        } else {
            CreatePostPreviewDecoder::tile_pixel_dimension()
        };
        let image_inputs: Vec<CreatePostImagePreviewInput> = image_inputs_source
            .into_iter()
            .filter(|attachment| attachment.kind == MediaPickerItemKind::Image)
            .filter_map(|attachment| {
                let file = attachment.local_file?;
                Some(CreatePostImagePreviewInput {
                    data: image_data.remove(&attachment.id),
                    id: attachment.id,
                    url: file.url,
                    maximum_pixel_dimension: dimension,
                })
            })
            .collect();
        let generation = self.preview_generation;
        let weak = cx.weak_entity();
        let completion: CreatePostPreviewCompletion = Rc::new(move |id: String, image: Option<CleverImage>, cx: &mut App| {
            let _ = weak.update(cx, |page, cx| page.apply_preview(generation, &id, image, cx));
        });
        if let Some(picked_video) = picked_video {
            let completion = completion.clone();
            self.preview_coordinator.update(cx, |coordinator, cx| {
                coordinator.prepare_video(picked_video.id, picked_video.url, dimension, completion, cx);
            });
        }
        if !image_inputs.is_empty() {
            self.preview_coordinator.update(cx, |coordinator, cx| {
                coordinator.prepare_images(image_inputs, completion, cx);
            });
        }
    }

    fn apply_preview(&mut self, generation: u64, id: &str, image: Option<CleverImage>, cx: &mut Context<Self>) {
        if self.preview_generation != generation {
            tracing::warn!(target: "riseonly::X2239", %id, generation, current = self.preview_generation, "preview dropped: generation");
            return;
        }
        self.state.update(cx, |state, cx| {
            if state.content_kind != RisePostContentKind::Publication {
                return;
            }
            let Some(attachment) = state.media.iter_mut().find(|item| item.id == id) else {
                tracing::warn!(target: "riseonly::X2239", %id, "preview dropped: attachment gone");
                return;
            };
            attachment.preview = image;
            attachment.is_preview_loading = false;
            cx.notify();
        });
    }

    /// photos drop the picker item since upload needs only the local file. video keeps it for
    /// duration and baked in track
    fn attachment(item: &MediaPickerItem) -> CreatePostMediaAttachment {
        let is_video = item.kind == MediaPickerItemKind::Video;
        let person_tags = if is_video {
            item.video_payload
                .as_ref()
                .and_then(|payload| payload.spatial_edit.as_ref())
                .and_then(|edit| edit.person_tags.clone())
                .unwrap_or_default()
        } else {
            item.image_payload.as_ref().map(|payload| payload.person_tags.clone()).unwrap_or_default()
        };
        CreatePostMediaAttachment {
            id: item.id.clone(),
            kind: if is_video { MediaPickerItemKind::Video } else { MediaPickerItemKind::Image },
            local_file: Some(SaiWsOptimisticLocalFile::new(
                item.url.clone(),
                item.suggested_file_name.clone(),
                Self::mime_type(item).to_owned(),
            )),
            remote_slide: None,
            preview: None,
            is_preview_loading: true,
            picker_item: is_video.then(|| item.clone()),
            person_tags,
        }
    }

    fn remove_media(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.hero_page_id.as_deref() == Some(id) {
            self.hero_page_id =
                self.state.read(cx).media.iter().find(|item| item.id != id).map(|item| item.id.clone());
        }
        self.update_state(cx, |state| state.remove_media(id));
        self.preview_coordinator.update(cx, |coordinator, cx| coordinator.cancel(id, cx));
    }

    fn apply_content_kind(&mut self, kind: RisePostContentKind, cx: &mut Context<Self>) {
        if kind == RisePostContentKind::Thread {
            self.invalidate_preview_preparation(cx);
        }
        self.update_state(cx, |state| state.apply_content_kind(kind));
    }

    fn invalidate_preview_preparation(&mut self, cx: &mut Context<Self>) {
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.preview_coordinator.update(cx, |coordinator, _| coordinator.cancel_all());
    }

    fn default_segment_end(track: &RiseStoredMusicTrack) -> i64 {
        TrackTrimOverlay::normalized_segment(None, None, track.duration_ms).1
    }

    fn attached_track_chip_text(&self, track: &RiseStoredMusicTrack, cx: &App) -> String {
        let state = self.state.read(cx);
        let base = track.artist_title_line();
        match (state.attached_track_start_ms, state.attached_track_end_ms) {
            (Some(start), Some(end)) if end > start => {
                format!("{base} \u{b7} {}\u{2013}{}", Self::format_ms(start), Self::format_ms(end))
            }
            _ => base,
        }
    }

    fn format_ms(ms: i64) -> String {
        RiseFormat::duration_milliseconds(ms)
    }

    fn mime_type(item: &MediaPickerItem) -> &'static str {
        match item.url.path_extension().to_lowercase().as_str() {
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "heic" | "heif" => "image/heic",
            "mp4" | "m4v" | "mov" => "video/mp4",
            _ => "image/jpeg",
        }
    }

    fn localized_publish_error(message: &str) -> String {
        RisePostPublishFailure::display_text(message)
    }

    /// files and images only go where the media list is open,
    /// a thread,
    /// achievement share and edit without media have no place
    fn accepts_external_media(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        state.content_kind == RisePostContentKind::Publication
            && !state.is_achievement_share()
            && (self.editing_post.is_none() || state.edits_media)
            && !self.is_publishing()
    }

    fn attach_external_files(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_external_media(cx) {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let items: Vec<MediaPickerItem> = cx
                .background_executor()
                .spawn(async move { paths.iter().filter_map(|path| external_item(path)).collect::<Vec<_>>() })
                .await;
            if items.is_empty() {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.handle_media_picker_result(MediaPickerResult::new(items, false), window, cx);
            });
        })
        .detach();
    }

    fn attach_pasted_image(&mut self, image: Image, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_external_media(cx) {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let written =
                cx.background_executor().spawn(async move { MediaPickerSession::write_pasted_image(&image) }).await;
            let _ = this.update_in(cx, |this, window, cx| match written {
                Ok(path) => this.attach_external_files(vec![path], window, cx),
                Err(error) => {
                    tracing::warn!(target: "riseonly", %error, "a pasted image was not staged");
                    Notifier::send(tr("sticker_pack_media_unsupported"), NotifierSendOptions::default(), cx);
                }
            });
        })
        .detach();
    }

    /// true when the clipboard held a file or image, plain text stays with the field
    fn attach_pasteboard(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.accepts_external_media(cx) {
            return false;
        }
        let paths = UIPasteboard::general().file_paths(cx);
        if !paths.is_empty() {
            self.attach_external_files(paths, window, cx);
            return true;
        }
        let Some(image) = UIPasteboard::general().image(cx) else {
            return false;
        };
        self.attach_pasted_image(image, window, cx);
        true
    }

    fn has_open_layer(&self) -> bool {
        self.show_media_picker || self.show_track_picker || self.show_track_trim
    }

    fn has_unsaved_changes(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        state.values != self.initial_values
            || !state.media.iter().map(|item| &item.id).eq(state.seeded_media_ids.iter())
            || state.attached_track.as_ref().and_then(|track| track.id.clone()) != self.initial_track_id
            || !state.coauthors.is_empty()
            || !state.tagged_people.is_empty()
    }

    fn close(&self, cx: &mut Context<Self>) {
        if let Some(nav_store) = self.nav_store.clone() {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    /// esc, the composer keeps no draft so filled in work goes only after a question
    fn request_close(&mut self, cx: &mut Context<Self>) {
        if !self.has_unsaved_changes(cx) {
            self.close(cx);
            return;
        }
        let weak = cx.weak_entity();
        NativeConfirmAlert::present_delete(
            &tr("profile_unsaved_title"),
            "",
            Some(tr("profile_unsaved_discard")),
            None,
            false,
            None,
            None,
            move |cx| {
                let _ = weak.update(cx, |page, cx| page.close(cx));
            },
            cx,
        );
    }

    fn shows_media_hero(&self, cx: &Context<Self>) -> bool {
        let state = self.state.read(cx);
        if state.media.is_empty() || state.is_achievement_share() {
            return false;
        }
        let is_publication = state.content_kind == RisePostContentKind::Publication;
        self.came_from_media_step
            || (self.editing_post.is_none() && is_publication && !state.is_riff_composer())
            || (self.editing_post.is_some() && state.edits_media && is_publication)
    }

    fn composer_body(&self, cx: &Context<Self>) -> Div {
        let state = self.state.read(cx);
        let shows_download = state.content_kind != RisePostContentKind::Thread && !state.is_achievement_share();
        let sections = [
            CreatePostFormSection::Lead,
            CreatePostFormSection::Fields,
            CreatePostFormSection::Options,
            CreatePostFormSection::Counters,
            CreatePostFormSection::Sparks,
        ]
        .into_iter()
        .chain(shows_download.then_some(CreatePostFormSection::Download))
        .chain(self.map_journey_id.is_some().then_some(CreatePostFormSection::Journey))
        .chain([CreatePostFormSection::Publish]);
        vstack(14.0)
            .items_stretch()
            .w_full()
            .pt(px(10.0))
            .children(sections.filter_map(|section| self.section_element(section, cx)))
            .child(div().h(px(if self.emoji_field.is_none() { 80.0 } else { self.emoji_panel_height() + 24.0 })))
    }

    fn section_element(&self, section: CreatePostFormSection, cx: &App) -> Option<AnyElement> {
        let view = self.section_views.get(&section)?.clone();
        let height = view.read(cx).height;
        let view = AnyView::from(view);
        Some(match height {
            Some(height) => {
                view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved().into_any_element()
            }
            None => view.into_any_element(),
        })
    }

    fn form_section_content(&self, section: CreatePostFormSection, cx: &Context<Self>) -> Option<AnyElement> {
        let theme = rise_ui::theme(cx).clone();
        Some(match section {
            CreatePostFormSection::Lead => self.lead_section(&theme, cx).into_any_element(),
            CreatePostFormSection::Fields => self.post_fields_group(cx).into_any_element(),
            CreatePostFormSection::Options => self.post_options_group(&theme, cx).into_any_element(),
            CreatePostFormSection::Counters => self.post_counters_group(cx).into_any_element(),
            CreatePostFormSection::Sparks => self.post_sparks_group(cx).into_any_element(),
            CreatePostFormSection::Download => self.post_download_group(cx).into_any_element(),
            CreatePostFormSection::Journey => {
                self.map_journey_id.as_ref()?;
                self.journey_group(&theme, cx).into_any_element()
            }
            CreatePostFormSection::Publish => self.publish_group(&theme, cx).into_any_element(),
        })
    }

    fn lead_section(&self, theme: &AppTheme, cx: &Context<Self>) -> Div {
        let state = self.state.read(cx);
        let is_publication = state.content_kind == RisePostContentKind::Publication;
        let body = vstack(14.0)
            .items_stretch()
            .w_full()
            .when(self.map_journey_id.is_some(), |body| {
                body.child(
                    hstack(6.0)
                        .px(px(16.0))
                        .children(IconUi::sized("map", px(15.0), theme.text.secondary, false))
                        .child(MainText::new(tr("map_journey_attached")).px(15.0).color(theme.text.secondary)),
                )
            });
        if self.came_from_media_step && !state.is_achievement_share() {
            if state.media.is_empty() {
                body.child(self.media_composer(theme, cx))
            } else {
                body.child(self.media_hero_group(theme, cx))
            }
        } else {
            let body = body.child(self.content_kind_group(theme, cx));
            if state.is_achievement_share() {
                body.child(self.achievement_composer(theme, cx))
            } else if self.editing_post.is_none() && is_publication {
                if !state.media.is_empty() && !state.is_riff_composer() {
                    body.child(self.media_hero_group(theme, cx))
                } else {
                    body.child(self.media_composer(theme, cx))
                }
            } else if state.edits_media && is_publication {
                body.when(!state.media.is_empty() && !self.hero_beside, |body| {
                    body.child(div().px(px(12.0)).child(self.hero_media_pager(theme, cx)))
                })
                .child(self.media_composer(theme, cx))
            } else {
                body
            }
        }
    }

    fn journey_group(&self, theme: &AppTheme, cx: &Context<Self>) -> GroupedBtns {
        GroupedBtns::new(
            "create_post_journey_group",
            vec![
                GroupedBtn::new("createPostJourneyHeat", tr("map_journey_show_heat"))
                    .icon("flame.fill")
                    .icon_background(theme.primary._100)
                    .disabled(self.is_publishing())
                    .show_chevron(false)
                    .trailing_slot(self.switch(
                        "create_post_journey_switch",
                        Binding::entity(self.state.clone(), |state| state.map_journey_show_heat, |state, value, _| {
                            state.map_journey_show_heat = value;
                        }),
                    ))
                    .callback(Self::action(cx, |page, _, cx| {
                        page.update_state(cx, |state| state.map_journey_show_heat = !state.map_journey_show_heat);
                    })),
            ],
        )
    }

    fn track_picker(&self, theme: &AppTheme, cx: &Context<Self>) -> BottomSheetUi {
        let weak = cx.weak_entity();
        let binding = Binding::entity(cx.entity(), |page: &CreatePost| page.show_track_picker, |page, value, _| {
            page.show_track_picker = value;
        });
        BottomSheetUi::new(
            "create_post_track_picker",
            binding,
            move |_, cx| {
                let attached = weak.upgrade().and_then(|page| page.read(cx).composer_track_selection(cx));
                let (pick, edit, remove, dismiss) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                EditorTrackPickerSheet::new(
                    "create_post_track_picker_sheet",
                    attached,
                    move |track, cx| {
                        let _ = pick.update(cx, |page, cx| {
                            let end_ms = Self::default_segment_end(&track);
                            page.update_state(cx, |state| {
                                state.attached_track = Some(track);
                                state.attached_track_start_ms = Some(0);
                                state.attached_track_end_ms = Some(end_ms);
                            });
                            page.show_track_picker = false;
                            page.set_show_track_trim(true, cx);
                        });
                    },
                    move |_, cx| {
                        let _ = edit.update(cx, |page, cx| {
                            page.show_track_picker = false;
                            page.set_show_track_trim(true, cx);
                        });
                    },
                    move |_, cx| {
                        let _ = remove.update(cx, |page, cx| page.remove_attached_track(cx));
                    },
                    move |_, cx| {
                        let _ = dismiss.update(cx, |page, cx| {
                            page.show_track_picker = false;
                            cx.notify();
                        });
                    },
                )
            },
            |_, _| div(),
        )
        .snap_fractions(vec![0.88, 0.93])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
    }

    fn track_trim(&self, window: &Window, cx: &Context<Self>) -> Option<Transitioned> {
        let state = self.state.read(cx);
        let track = state.attached_track.clone()?;
        let start_ms = state.attached_track_start_ms.unwrap_or(0);
        let end_ms = state.attached_track_end_ms.unwrap_or_else(|| Self::default_segment_end(&track));
        let confirm = cx.weak_entity();
        let cancel = cx.weak_entity();
        let overlay = TrackTrimOverlay::new(
            "create_post_track_trim",
            track,
            start_ms,
            end_ms,
            move |start, end, _, _, cx| {
                let _ = confirm.update(cx, |page, cx| {
                    page.update_state(cx, |state| {
                        state.attached_track_start_ms = Some(start);
                        state.attached_track_end_ms = Some(end);
                    });
                    page.set_show_track_trim(false, cx);
                });
            },
            move |_, cx| {
                let _ = cancel.update(cx, |page, cx| page.set_show_track_trim(false, cx));
            },
        )
        .opaque_backdrop(true);
        self.track_trim_presence.render_transitioned(RiseClock::monotonic(), window, move |_| {
            div().absolute().inset_0().child(overlay)
        })
    }
}

impl Render for CreatePost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.nav_store = env::<NavigationStoreKey>(cx);
        self.hero_pixels = CreatePostPreviewDecoder::hero_pixel_dimension_for_attachment_count(
            self.state.read(cx).media.len(),
            window,
        );
        self.hero_beside = self.is_sheet(window) && self.shows_media_hero(cx);
        let form = self
            .form_view
            .get_or_insert_with(|| {
                let page = cx.weak_entity();
                cx.new(|_| CreatePostFormView { page })
            })
            .clone();
        let layers = self
            .layers_view
            .get_or_insert_with(|| {
                let page = cx.weak_entity();
                cx.new(|_| CreatePostLayersView { page })
            })
            .clone();
        let cancel_page = cx.weak_entity();
        let paste_page = cx.weak_entity();
        let key_page = cx.weak_entity();
        let escape_page = cx.weak_entity();
        let submit_page = cx.weak_entity();
        let drop_page = cx.weak_entity();
        let page_focus = PageFocus::keyed_with("create-post-page-focus", self.focus.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let disappear_page = cx.weak_entity();
        div()
            .id("create_post")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .capture_action(move |_: &Cancel, _, cx| {
                let _ = cancel_page.update(cx, |page, cx| {
                    if !page.has_open_layer() {
                        cx.stop_propagation();
                        page.request_close(cx);
                    }
                });
            })
            .capture_action(move |_: &Paste, window, cx| {
                let _ = paste_page.update(cx, |page, cx| {
                    if !page.has_open_layer() && page.attach_pasteboard(window, cx) {
                        cx.stop_propagation();
                    }
                });
            })
            .layer_form_keys(
                LayerCancel::dismiss(move |_, cx| {
                    let _ = escape_page.update(cx, |page, cx| {
                        if !page.has_open_layer() {
                            page.request_close(cx);
                        }
                    });
                }),
                true,
                move |_, cx| {
                    let _ = submit_page.update(cx, |page, cx| {
                        if !page.has_open_layer() {
                            page.publish_post(cx);
                        }
                    });
                },
            )
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                let _ = key_page.update(cx, |page, cx| {
                    if !page.has_open_layer()
                        && keystroke.key == PASTE_KEY
                        && keystroke.modifiers.secondary()
                        && page.attach_pasteboard(window, cx)
                    {
                        cx.stop_propagation();
                    }
                });
            })
            .on_drop(move |paths: &ExternalPaths, window, cx| {
                let paths = paths.paths().to_vec();
                let _ = drop_page.update(cx, |page, cx| page.attach_external_files(paths, window, cx));
            })
            .child({
                let cell = self.page_width.clone();
                let weak = cx.weak_entity();
                canvas(
                    move |bounds, window, cx| {
                        let measured = f32::from(bounds.size.width);
                        if cell.get() != Some(measured) {
                            cell.set(Some(measured));
                            window.refresh();
                            let _ = weak.update(cx, |_, cx| cx.notify());
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
                .size_full()
            })
            .child(form.cached(StyleRefinement::default().size_full()))
            .child(layers)
            .children(self.track_trim(window, cx))
            .on_appear("create_post_appear", move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
            })
            .on_disappear("create_post_disappear", move |window, cx| {
                CreatePostGuideFacts::clear(cx);
                let _ = disappear_page.update(cx, |page, cx| page.invalidate_preview_preparation(cx));
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

/// composer form under any layer of the page, as its own cached view
struct CreatePostFormView {
    page: WeakEntity<CreatePost>,
}

impl Render for CreatePostFormView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.page
            .update(cx, |page, cx| page.surface(window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// which group of the composer form a section view draws
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum CreatePostFormSection {
    Lead,
    Fields,
    Options,
    Counters,
    Sparks,
    Download,
    Journey,
    Publish,
}

impl CreatePostFormSection {
    const ALL: [Self; 8] = [
        Self::Lead,
        Self::Fields,
        Self::Options,
        Self::Counters,
        Self::Sparks,
        Self::Download,
        Self::Journey,
        Self::Publish,
    ];

    fn measure_id(self) -> &'static str {
        match self {
            Self::Lead => "create_post_lead_size",
            Self::Fields => "create_post_fields_size",
            Self::Options => "create_post_options_size",
            Self::Counters => "create_post_counters_size",
            Self::Sparks => "create_post_sparks_size",
            Self::Download => "create_post_download_size",
            Self::Journey => "create_post_journey_size",
            Self::Publish => "create_post_publish_size",
        }
    }
}

/// one composer form group as its own cached view, wheel steps dont relayout fields
/// gpui sizes it from style alone, measured once
struct CreatePostSectionView {
    page: WeakEntity<CreatePost>,
    section: CreatePostFormSection,
    height: Option<Pixels>,
}

impl CreatePostSectionView {
    fn set_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        let prior = self.height.replace(height);
        if prior.is_some_and(|prior| prior != height) {
            let form = self.page.upgrade().and_then(|page| page.read(cx).form_view.clone());
            if let Some(form) = form {
                form.update(cx, |_, cx| cx.notify());
            }
        }
    }
}

impl Render for CreatePostSectionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.section;
        let content = self.page.update(cx, |page, cx| page.form_section_content(section, cx)).ok().flatten();
        let this = cx.weak_entity();
        div().relative().w_full().flex().flex_col().items_stretch().children(content).child(measure(
            section.measure_id(),
            move |bounds, _, cx| {
                this.update(cx, |view, cx| view.set_height(bounds.size.height, cx)).ok();
            },
        ))
    }
}

/// media and track picker sheets of the page as their own view
struct CreatePostLayersView {
    page: WeakEntity<CreatePost>,
}

impl Render for CreatePostLayersView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.page
            .update(cx, |page, cx| page.layers(window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl CreatePost {
    /// in the right panel window width made a form beside a squeezed hero,
    /// so sheet and split read the measured page width
    fn is_sheet(&self, window: &Window) -> bool {
        let page_width = self.page_width.get().unwrap_or_else(|| f32::from(window.viewport_size().width));
        page_width >= SHEET_MIN_WINDOW_WIDTH
    }

    fn surface(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        for section in CreatePostFormSection::ALL {
            if !self.section_views.contains_key(&section) {
                let page = cx.weak_entity();
                let view = cx.new(|_| CreatePostSectionView { page, section, height: None });
                self.section_views.insert(section, view);
            }
        }
        let theme = rise_ui::theme(cx).clone();
        let is_sheet = self.is_sheet(window);
        let emoji_panel = self.post_emoji_panel(cx);
        let emoji_overlay = self.emoji_presence.render_transitioned(RiseClock::monotonic(), window, move |_| {
            div().absolute().left_0().right_0().bottom_0().child(emoji_panel)
        });
        let wrapper = SettingsWrapperUi::new(tr(self.screen_title_key(cx)), {
            let nav_store = self.nav_store.clone();
            move |_, cx| {
                if let Some(nav_store) = nav_store.clone() {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            }
        })
        .content({
            let form = div()
                .relative()
                .size_full()
                .child(
                    scroll_view("create_post_scroll")
                        .size_full()
                        .bg(theme.bg._100)
                        .list_shell_scroll_under_chrome(cx)
                        .child(self.composer_body(cx)),
                )
                .children(emoji_overlay);
            if self.hero_beside {
                hstack(0.0)
                    .items_stretch()
                    .size_full()
                    .child(
                        list_shell_under_chrome_box()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .flex()
                            .bg(black())
                            .child(self.hero_split_pane(&theme, cx)),
                    )
                    .child(div().w(px(SPLIT_FORM_WIDTH)).flex_none().h_full().child(form))
            } else {
                form
            }
        });
        let surface: gpui::AnyElement = if is_sheet {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(black().opacity(0.4))
                .child(
                    div()
                        .w(relative(SPLIT_SHEET_WIDTH_FRACTION))
                        .max_w(px(SPLIT_SHEET_MAX_WIDTH))
                        .h(relative(SHEET_HEIGHT_FRACTION))
                        .rounded(px(20.0))
                        .overflow_hidden()
                        .bg(theme.bg._100)
                        .child(wrapper),
                )
                .into_any_element()
        } else {
            wrapper.into_any_element()
        };
        surface
    }

    fn layers(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let picker_weak = cx.weak_entity();
        div()
            .size_0()
            .child(
                MediaPickerSheet::new(
                    "create_post_media_picker",
                    Binding::entity(cx.entity(), |page: &CreatePost| page.show_media_picker, |page, value, _| {
                        page.show_media_picker = value;
                    }),
                    self.media_picker_configuration(cx),
                    ThemeStore::shared(cx),
                    move |result, window, cx| {
                        let _ = picker_weak.update(cx, |page, cx| page.handle_media_picker_result(result, window, cx));
                    },
                )
                .experimental_bottom_sheet(true),
            )
            .child(self.track_picker(&theme, cx))
            .into_any_element()
    }
}

/// what a composer guide waits on, reported from the composers own state
enum CreatePostGuideFacts {}

impl CreatePostGuideFacts {
    fn facts(state: &CreatePostScreenState) -> [(RiseGuideSignal, bool); 6] {
        [
            (RiseGuideSignal::ComposerHasMedia, !state.media.is_empty()),
            (RiseGuideSignal::ComposerTrackAttached, state.attached_track.is_some()),
            (RiseGuideSignal::ComposerCoauthorAdded, !state.coauthors.is_empty()),
            (RiseGuideSignal::ComposerPeopleTagged, state.tagged_people_count() > 0),
            (RiseGuideSignal::ComposerAlsoRiff, state.requests_riff),
            (RiseGuideSignal::ComposerCommentsOff, state.disable_comments),
        ]
    }

    fn report(facts: [(RiseGuideSignal, bool); 6], cx: &mut App) {
        for (fact, value) in facts {
            GuideFacts::set(fact, value, cx);
        }
    }

    fn clear(cx: &mut App) {
        for fact in [
            RiseGuideSignal::ComposerHasMedia,
            RiseGuideSignal::ComposerTrackAttached,
            RiseGuideSignal::ComposerCoauthorAdded,
            RiseGuideSignal::ComposerPeopleTagged,
            RiseGuideSignal::ComposerAlsoRiff,
            RiseGuideSignal::ComposerCommentsOff,
        ] {
            GuideFacts::set(fact, false, cx);
        }
    }
}

use std::cell::Cell;
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::LazyLock;

use gpui::{
    App, AppContext, AsyncApp, Bounds, ClickEvent, Context, Entity, EntityId, FocusHandle, FontWeight, Hsla,
    InteractiveElement, IntoElement, ObjectFit, ParentElement, Pixels, Render, Role, SharedString, StatefulInteractiveElement, Styled, StyledImage, Task, Window,
    black, div, img, point, px, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_cache::NSCache;
use rise_i18n::tr;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::window_screen_frame_reader::{
    MediaCellScreenFrameRegistry, WindowScreenFrameReader, WindowScreenFrameReaderFrameView,
};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::music::shared::utils::music::music_cover_letterbox::MusicCoverLetterbox;
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListCachePurge, PostRowActionRouting, PostRowMainContext, PostRowMusicStripLayout, PostRowSeedTrack,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::PostRowMetrics;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::{
    PostRowChromeFonts, PostRowChromeSymbols, PostRowTextLineView, PostRowTextLineViewAlignment,
    post_row_frame,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_header_view::{
    PostRowLocalImageDecoder, PostRowMusicLineText,
};

/// music strip with cover
/// title and duration button and the feed sound button on grey 0.6, radius 12
/// the row registers anchor_view as the media anchor
pub struct PostRowMusicStripView {
    title_view: Entity<PostRowTextLineView>,
    duration_view: Entity<PostRowTextLineView>,
    sound_control: PostRowStripSoundControl,
    title_focus: FocusHandle,
    sound_focus: FocusHandle,
    title_label: SharedString,
    cover_image: Option<CleverImage>,

    strip_layout: Option<PostRowMusicStripLayout>,
    post_id: Option<String>,
    router: Option<Weak<dyn PostRowActionRouting>>,
    cover_key: Option<String>,
    cover_task: Option<Task<()>>,
    cover_generation: u64,
    background: Hsla,
    reader_id: String,
    is_anchor_lifted: bool,
}

impl PostRowMusicStripView {
    pub fn new(row_view: EntityId, cx: &mut Context<Self>) -> Self {
        PostRowMusicCoverCache::register_purge_if_needed(cx);
        Self {
            title_view: cx.new(|_| PostRowTextLineView::new()),
            duration_view: cx.new(|_| PostRowTextLineView::new()),
            sound_control: PostRowStripSoundControl::new(),
            title_focus: cx.focus_handle().tab_stop(true),
            sound_focus: cx.focus_handle().tab_stop(true),
            title_label: SharedString::default(),
            cover_image: None,
            strip_layout: None,
            post_id: None,
            router: None,
            cover_key: None,
            cover_task: None,
            cover_generation: 0,
            background: gpui::transparent_black(),
            reader_id: format!("post-row-strip-{}", row_view.as_u64()),
            is_anchor_lifted: false,
        }
    }

    /// the Media anchor, none until the strip was drawn once
    pub fn anchor_view(&self) -> Option<WindowScreenFrameReaderFrameView> {
        MediaCellScreenFrameRegistry::attached_view(&self.reader_id)
    }

    pub fn set_anchor_lifted(&mut self, is_lifted: bool, cx: &mut Context<Self>) {
        self.is_anchor_lifted = is_lifted;
        cx.notify();
    }

    pub fn configure(
        &mut self,
        post_id: &str,
        track: &PostRowSeedTrack,
        layout: &PostRowMusicStripLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        self.post_id = Some(post_id.to_owned());
        self.router = Some(Rc::downgrade(&context.router));
        self.strip_layout = Some(layout.clone());

        let palette = &context.palette;
        let spec = &context.spec;
        self.background = palette.background300.opacity(0.6);

        let title = PostRowMusicLineText::text(track, &spec.locale_identifier);
        let title_font = PostRowChromeFonts::text(PostRowMetrics::STRIP_TITLE_FONT_SIZE, FontWeight::MEDIUM, spec);
        let title_color = palette.text_primary;
        self.title_view.update(cx, |view, cx| {
            view.set_text(&title, title_font, title_color, PostRowTextLineViewAlignment::Leading, cx)
        });
        let duration_font = PostRowChromeFonts::text(PostRowMetrics::SECONDARY_FONT_SIZE, FontWeight::NORMAL, spec);
        let duration_color = palette.text_secondary;
        self.duration_view.update(cx, |view, cx| {
            view.set_text(&track.formatted_duration, duration_font, duration_color, PostRowTextLineViewAlignment::Leading, cx)
        });
        self.title_label = [title.as_str(), track.formatted_duration.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
            .into();
        self.sound_control.set_muted(context.feed_audio_muted);

        self.configure_cover(&track.cover_url, context, cx);
        cx.notify();
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.cancel_cover();
        self.cover_key = None;
        self.cover_image = None;
        self.post_id = None;
        self.strip_layout = None;
        self.is_anchor_lifted = false;
        cx.notify();
    }

    fn configure_cover(&mut self, cover_url: &str, context: &PostRowMainContext, cx: &mut Context<Self>) {
        let account_user_id = context.account_user_id.clone();
        let account_epoch = context.account_epoch.clone();
        let key = format!(
            "{cover_url}|{}|{}",
            account_user_id.as_deref().unwrap_or("anonymous"),
            account_epoch.as_deref().unwrap_or("none"),
        );
        if self.cover_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.cancel_cover();
        self.cover_key = Some(key);

        let sources = MusicMediaURL::cover_url_strings_for_fetch(Some(cover_url));
        for source in &sources {
            if let Some(image) = PostRowMusicCoverCache::shared().image(source, account_user_id.as_deref()) {
                self.cover_image = Some(image);
                return;
            }
        }
        self.cover_image = None;
        if sources.is_empty() {
            return;
        }

        self.cover_generation = self.cover_generation.wrapping_add(1);
        let generation = self.cover_generation;
        let pixels = (PostRowMetrics::STRIP_COVER_SIZE * context.spec.display_scale.max(1.0)).ceil() as i64;
        self.cover_task = Some(cx.spawn(async move |this, cx| {
            let image =
                PostRowMusicCoverCache::load(sources, pixels, account_user_id, account_epoch, &*cx).await;
            let Some(image) = image else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.cover_generation == generation {
                    this.cover_image = Some(image);
                    cx.notify();
                }
            });
        }));
    }

    fn cancel_cover(&mut self) {
        self.cover_generation = self.cover_generation.wrapping_add(1);
        self.cover_task = None;
    }

    fn handle_title_tap(&mut self, cx: &mut Context<Self>) {
        let (Some(router), Some(post_id)) = (self.router.as_ref().and_then(Weak::upgrade), self.post_id.clone()) else {
            return;
        };
        router.open_audio_page(&post_id, cx);
    }

    fn handle_sound_tap(&mut self, cx: &mut Context<Self>) {
        let Some(router) = self.router.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        router.toggle_feed_sound(cx);
        self.sound_control.set_muted(!self.sound_control.is_muted());
        cx.notify();
    }
}

impl Render for PostRowMusicStripView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(layout) = self.strip_layout.clone() else {
            return div().into_any_element();
        };
        if self.is_anchor_lifted {
            return div()
                .relative()
                .size_full()
                .child(WindowScreenFrameReader::new(self.reader_id.clone()))
                .into_any_element();
        }
        let title_frame = layout.title_line.union(&layout.duration_line);
        let title_line = local_frame(layout.title_line, title_frame);
        let duration_line = local_frame(layout.duration_line, title_frame);

        let cover = post_row_frame(layout.cover)
            .overflow_hidden()
            .continuous_corners(8.0)
            .child(MusicCoverPlaceholder::new().corner_radius(8.0))
            .children(self.cover_image.clone().map(|image| {
                div()
                    .absolute()
                    .inset_0()
                    .child(img(image).size_full().object_fit(ObjectFit::Cover))
            }));

        let title = post_row_frame(title_frame)
            .id("post-row-strip-title")
            .track_focus(&self.title_focus)
            .role(Role::Button)
            .aria_label(self.title_label.clone())
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.handle_title_tap(cx);
            }))
            .child(post_row_frame(title_line).child(self.title_view.clone()))
            .child(post_row_frame(duration_line).child(self.duration_view.clone()));

        let sound = post_row_frame(layout.sound_button)
            .id("post-row-strip-sound")
            .track_focus(&self.sound_focus)
            .role(Role::Button)
            .aria_label(tr(self.sound_control.label_key()))
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .tooltip_label(tr(self.sound_control.label_key()))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.handle_sound_tap(cx);
            }))
            .child(self.sound_control.view());

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .continuous_corners(12.0)
            .bg(self.background)
            .child(WindowScreenFrameReader::new(self.reader_id.clone()))
            .child(cover)
            .child(title)
            .child(sound)
            .into_any_element()
    }
}

fn local_frame(frame: Bounds<Pixels>, parent: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        point(frame.origin.x - parent.origin.x, frame.origin.y - parent.origin.y),
        frame.size,
    )
}

/// sound button, 34 pt black 0.45 circle with a 14 pt white speaker in a 48 pt target
struct PostRowStripSoundControl {
    is_muted: bool,
}

impl PostRowStripSoundControl {
    const CIRCLE_SIDE: f32 = 34.0;

    fn new() -> Self {
        Self { is_muted: false }
    }

    fn is_muted(&self) -> bool {
        self.is_muted
    }

    fn set_muted(&mut self, muted: bool) {
        self.is_muted = muted;
    }

    fn label_key(&self) -> &'static str {
        if self.is_muted { "feed_sound_unmute" } else { "feed_sound_mute" }
    }

    fn view(&self) -> impl IntoElement {
        let glyph = if self.is_muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" };
        div()
            .size(px(Self::CIRCLE_SIDE))
            .rounded_full()
            .bg(black().opacity(0.45))
            .flex()
            .items_center()
            .justify_center()
            .children(
                PostRowChromeSymbols::image(glyph, 14.0, FontWeight::SEMIBOLD).map(|icon| icon.text_color(white())),
            )
    }
}

/// trimmed track covers per account so a revisited strip draws its cover in the configure pass
pub struct PostRowMusicCoverCache {
    cache: NSCache<String, CleverImage>,
}

static POST_ROW_MUSIC_COVER_CACHE: LazyLock<PostRowMusicCoverCache> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(128);
    cache.set_total_cost_limit(16 * 1_024 * 1_024);
    PostRowMusicCoverCache { cache }
});

thread_local! {
    static IS_PURGE_REGISTERED: Cell<bool> = const { Cell::new(false) };
}

impl PostRowMusicCoverCache {
    pub fn shared() -> &'static Self {
        &POST_ROW_MUSIC_COVER_CACHE
    }

    pub fn image(&self, source: &str, account_user_id: Option<&str>) -> Option<CleverImage> {
        self.cache.object(&Self::key(source, account_user_id))
    }

    pub fn insert(&self, image: CleverImage, source: &str, account_user_id: Option<&str>) {
        let dimensions = image.size(0);
        let pixels = dimensions.width.0 as usize * dimensions.height.0 as usize * 4;
        self.cache.set_object_cost(image, Self::key(source, account_user_id), pixels.max(1));
    }

    pub fn remove_all(&self) {
        self.cache.remove_all_objects();
    }

    /// first source that loads, in cover order
    /// none when every source fails or the account changed
    /// dropping the future cancels it
    pub async fn load(
        sources: Vec<String>,
        local_pixel_dimension: i64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        for source in sources {
            let trimmed = trimming_whitespaces_and_newlines(&source).to_owned();
            let image = if trimmed.starts_with("file://") || Path::new(&trimmed).is_absolute() {
                cx.background_executor()
                    .spawn(async move { PostRowLocalImageDecoder::decode(&trimmed, local_pixel_dimension) })
                    .await
            } else {
                CleverImageBitmapLoader::load(
                    &source,
                    true,
                    true,
                    account_user_id.clone(),
                    account_epoch.clone(),
                    None,
                    CleverImageCachePolicy::Standard,
                    CleverImageWorkLane::Visible,
                    cx,
                )
                .await
                .map(|loaded| MusicCoverLetterbox::trimmed_cache_key(&loaded, &source))
            };
            let Some(image) = image else {
                continue;
            };
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                return None;
            }
            Self::shared().insert(image.clone(), &source, account_user_id.as_deref());
            return Some(image);
        }
        None
    }

    pub fn register_purge_if_needed(cx: &mut App) {
        if IS_PURGE_REGISTERED.replace(true) {
            return;
        }
        let _ = PostListCachePurge::register(|_| PostRowMusicCoverCache::shared().remove_all(), cx);
    }

    fn key(source: &str, account_user_id: Option<&str>) -> String {
        format!("{}|{source}", account_user_id.unwrap_or("anonymous"))
    }
}

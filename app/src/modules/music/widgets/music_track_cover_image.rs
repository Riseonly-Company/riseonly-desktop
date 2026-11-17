use std::sync::Arc;

use gpui::{
    AnyElement, App, AsyncWindowContext, ElementId, Entity, ImageSource, IntoElement, ObjectFit, RenderOnce, SharedString, Subscription, Window, div, img,
    px, white,
};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::swiftui::blur::blur_aware_image_source;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::modifiers::{ModifiedContent, ModifierExt, ViewModifier};
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use super::music_cover_placeholder::{MusicCoverMedia, MusicCoverPlaceholder};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, MediaViewerPresentationState, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerGalleryItemKind, MediaViewerHeroSkin,
};
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::modules::music::shared::utils::music::music_cover_letterbox::MusicCoverLetterbox;
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;

struct MusicCoverImageState {
    cover_url: String,
    sources: Vec<String>,
    source_index: usize,
    ui_image: Option<CleverImage>,
    did_fail_all: bool,
}

impl MusicCoverImageState {
    fn new(cover_url: String) -> Self {
        let sources = MusicCoverMedia::fetch_sources(&cover_url);
        let mut state = Self { cover_url, sources, source_index: 0, ui_image: None, did_fail_all: false };
        state.resolve_from_cache();
        state
    }

    fn set_cover_url(&mut self, cover_url: String) {
        self.sources = MusicCoverMedia::fetch_sources(&cover_url);
        self.cover_url = cover_url;
        self.source_index = 0;
        self.ui_image = None;
        self.did_fail_all = false;
        self.resolve_from_cache();
    }

    fn active_source(&self) -> &str {
        self.sources.get(self.source_index).map_or("", String::as_str)
    }

    fn active_local_source(&self) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(self.active_source());
        (value.starts_with("file://") || is_local_absolute_path(value)).then(|| value.to_owned())
    }

    fn resolve_from_cache(&mut self) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        for source in &self.sources {
            if let Some(cached) = CleverImageBitmapLoader::memory_image(
                source,
                true,
                account_user_id.as_deref(),
                account_epoch.as_deref(),
                None,
                CleverImageCachePolicy::Standard,
                None,
                false,
            ) {
                self.ui_image = Some(MusicCoverLetterbox::trimmed_cache_key(&cached, source));
                self.did_fail_all = false;
                return;
            }
        }
    }

    fn cached_cover(sources: &[String], account_user_id: Option<&str>, account_epoch: Option<&str>) -> Option<CleverImage> {
        sources.iter().find_map(|source| {
            CleverImageBitmapLoader::image_from_memory_or_disk(
                source,
                true,
                account_user_id,
                account_epoch,
                CleverImageCachePolicy::Standard,
                None,
            )
            .map(|image| MusicCoverLetterbox::trimmed_cache_key(&image, source))
        })
    }

    async fn load_cover(state: Entity<Self>, cx: &mut AsyncWindowContext) {
        let (sources, start) = state.update(cx, |state, _| {
            let done = state.ui_image.is_some() || state.did_fail_all;
            (if done { Vec::new() } else { state.sources.clone() }, state.source_index)
        });
        if start >= sources.len() {
            state.update(cx, |state, cx| {
                if state.ui_image.is_none() && !state.did_fail_all {
                    state.did_fail_all = true;
                    cx.notify();
                }
            });
            return;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let executor = cx.background_executor().clone();
        for (index, source) in sources.iter().enumerate().skip(start) {
            let source = source.clone();
            let cached_sources = if index == start { sources[start..].to_vec() } else { vec![source.clone()] };
            let disk_user_id = account_user_id.clone();
            let disk_epoch = account_epoch.clone();
            let cached = executor
                .spawn(async move { Self::cached_cover(&cached_sources, disk_user_id.as_deref(), disk_epoch.as_deref()) })
                .await;
            let image = match cached {
                Some(image) => Some(image),
                None => {
                    let loaded = CleverImageBitmapLoader::load(
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
                    .await;
                    match loaded {
                        Some(image) => {
                            let trim_source = source.clone();
                            Some(
                                executor
                                    .spawn(async move { MusicCoverLetterbox::trimmed_cache_key(&image, &trim_source) })
                                    .await,
                            )
                        }
                        None => None,
                    }
                }
            };
            if let Some(image) = image {
                state.update(cx, |state, cx| {
                    state.ui_image = Some(image);
                    state.did_fail_all = false;
                    cx.notify();
                });
                return;
            }
            if index + 1 < sources.len() {
                state.update(cx, |state, cx| {
                    state.source_index = index + 1;
                    cx.notify();
                });
            }
        }
        state.update(cx, |state, cx| {
            state.did_fail_all = true;
            cx.notify();
        });
    }
}

#[derive(IntoElement)]
pub struct MusicCoverImage {
    id: ElementId,
    cover_url: String,
    fallback_colors: Vec<String>,
    size: Option<f32>,
    corner_radius: f32,
    fills_container: bool,
    shows_loading_indicator: bool,
    shows_placeholder_note: bool,
}

pub type MusicTrackCoverImage = MusicCoverImage;

impl MusicCoverImage {
    pub fn new(id: impl Into<ElementId>, cover_url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            cover_url: cover_url.into(),
            fallback_colors: Vec::new(),
            size: Some(48.0),
            corner_radius: 8.0,
            fills_container: false,
            shows_loading_indicator: true,
            shows_placeholder_note: true,
        }
    }

    pub fn fallback_colors(mut self, fallback_colors: Vec<String>) -> Self {
        self.fallback_colors = fallback_colors;
        self
    }

    pub fn size(mut self, size: impl Into<Option<f32>>) -> Self {
        self.size = size.into();
        self
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn fills_container(mut self, fills_container: bool) -> Self {
        self.fills_container = fills_container;
        self
    }

    pub fn shows_loading_indicator(mut self, shows_loading_indicator: bool) -> Self {
        self.shows_loading_indicator = shows_loading_indicator;
        self
    }

    pub fn shows_placeholder_note(mut self, shows_placeholder_note: bool) -> Self {
        self.shows_placeholder_note = shows_placeholder_note;
        self
    }

    fn fallback_gradient(&self) -> MusicCoverPlaceholder {
        MusicCoverPlaceholder::new()
            .icon_scale(0.34)
            .corner_radius(self.corner_radius)
            .colors(self.fallback_colors.clone())
            .shows_note(self.shows_placeholder_note)
    }
}

impl RenderOnce for MusicCoverImage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let cover_url = self.cover_url.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, move |_, _| MusicCoverImageState::new(cover_url));
        if state.read(cx).cover_url != self.cover_url {
            let cover_url = self.cover_url.clone();
            state.update(cx, |state, cx| {
                state.set_cover_url(cover_url);
                cx.notify();
            });
        }
        let load_state = state.clone();
        use_task(
            ElementId::NamedChild(Arc::new(self.id.clone()), "music-cover-load".into()),
            self.cover_url.clone(),
            window,
            cx,
            async move |cx| MusicCoverImageState::load_cover(load_state, cx).await,
        );

        let corner_radius = self.corner_radius;
        let (active_local_source, ui_image, is_empty_or_failed) = {
            let state = state.read(cx);
            (state.active_local_source(), state.ui_image.clone(), state.did_fail_all || state.active_source().is_empty())
        };

        let content: AnyElement = if let Some(active_local_source) = active_local_source {
            let scale = window.scale_factor();
            CleverImageUi::new(active_local_source)
                .id(SharedString::from(format!("{}-local", self.id)))
                .content_mode(ContentMode::Fill)
                .corner_radius(corner_radius)
                .persist_to_disk(false)
                .maximum_decode_pixel_dimension(self.size.map(|size| ((size * scale) as i64).max(1)))
                .into_any_element()
        } else if let Some(ui_image) = ui_image {
            img(blur_aware_image_source(ImageSource::Render(ui_image)))
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .clip_shape(Shape::continuous(corner_radius))
                .into_any_element()
        } else if is_empty_or_failed {
            self.fallback_gradient().into_any_element()
        } else if self.shows_loading_indicator {
            zstack(Alignment::Center)
                .relative()
                .size_full()
                .child(
                    div().absolute().inset_0().child(
                        MusicCoverPlaceholder::new()
                            .icon_scale(0.34)
                            .corner_radius(corner_radius)
                            .colors(self.fallback_colors.clone())
                            .shows_note(false),
                    ),
                )
                .child(progress_view().tint(white()).control_size(ControlSize::Small))
                .into_any_element()
        } else {
            div().size_full().into_any_element()
        };

        let frame = div().relative().flex().overflow_hidden().clip_shape(Shape::continuous(corner_radius)).child(content);
        if self.fills_container {
            frame.size_full()
        } else if let Some(size) = self.size {
            frame.size(px(size)).flex_none()
        } else {
            frame.size_full()
        }
    }
}

pub trait MusicTrackCoverImageModifiers: IntoElement + Sized {
    fn music_cover_media_viewer(
        self,
        id: &str,
        source: &str,
        title: &str,
        corner_radius: f32,
    ) -> ModifiedContent<MusicCoverMediaViewerModifier> {
        self.modifier(MusicCoverMediaViewerModifier {
            id: id.to_owned(),
            source: source.to_owned(),
            title: title.to_owned(),
            corner_radius,
        })
    }
}

impl<E: IntoElement> MusicTrackCoverImageModifiers for E {}

struct MusicCoverMediaViewerObservation {
    seen: (bool, bool),
    _subscription: Subscription,
}

impl MusicCoverMediaViewerObservation {
    fn seen(presenter: &MediaViewerPresentationState, registry_id: &str) -> (bool, bool) {
        let is_lifted = presenter.hero_lift_id() == Some(registry_id) && presenter.bubble_cell_hidden();
        (is_lifted, presenter.is_presented())
    }
}

pub struct MusicCoverMediaViewerModifier {
    id: String,
    source: String,
    title: String,
    corner_radius: f32,
}

impl MusicCoverMediaViewerModifier {
    fn resolved_source(&self) -> Option<String> {
        MusicMediaURL::resolved_url_string(Some(&self.source))
    }

    fn registry_id(&self) -> String {
        MediaViewerFrameKeys::music_cover(&self.id)
    }

    fn open(source: String, registry_id: String, title: String, corner_radius: f32, cx: &mut App) {
        let is_local = source.to_lowercase().starts_with("file://");
        let item = MediaViewerGalleryItem {
            id: registry_id.clone(),
            kind: MediaViewerGalleryItemKind::Image,
            url: source.clone(),
            thumbnail_url: None,
            width: 1.0,
            height: 1.0,
            duration: None,
            sender_id: String::new(),
            sender_name: title,
            sender_logo: Some(source.clone()),
            created_at: 0.0,
            caption_title: String::new(),
            caption: String::new(),
            message_id: String::new(),
            index_in_message: 0,
            media_item_id: registry_id.clone(),
            uses_fit_letterbox: false,
            frame_registry_id: registry_id.clone(),
            is_own: false,
            can_delete: false,
            can_save: true,
            can_forward: false,
            can_reply: false,
            can_show_in_context: false,
            can_delete_for_me: false,
            can_delete_for_everyone: false,
            can_set_as_main: false,
            is_album_row: false,
            display_source_override: Some(source.clone()),
            persist_media_to_disk: !is_local,
            deletes_from_favourites: false,
            post_download: None,
            image_cache_policy: if is_local {
                CleverImageCachePolicy::MemoryOnly
            } else {
                CleverImageCachePolicy::Standard
            },
        };
        let radii = MessageBubbleRadii::uniform(px(corner_radius));
        let hero_skin = MediaViewerHeroSkin {
            display_source: source,
            uses_fit_foreground: false,
            media_tail: None,
            source_media_radii: Some(radii),
        };
        let source_frame = MediaCellScreenFrameRegistry::frame(&registry_id).unwrap_or_default();
        let presenter = media_viewer_presentation(cx);

        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                presenter,
                vec![item],
                0,
                source_frame,
                registry_id,
                hero_skin,
                MediaViewerChrome { shows_bottom_bar: false, can_report: false, ..Default::default() },
                MediaViewerGalleryScope::Complete,
                MediaViewerPresentationHandlers::default(),
                cx,
            )
            .await;
        })
        .detach();
    }
}

impl ViewModifier for MusicCoverMediaViewerModifier {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(resolved_source) = self.resolved_source() else {
            return content;
        };
        let registry_id = self.registry_id();
        let corner_radius = self.corner_radius;

        let presenter = media_viewer_presentation(cx);
        let observed = presenter.clone();
        let observed_id = registry_id.clone();
        let _observation = window.use_keyed_state(
            SharedString::from(format!("{registry_id}-observation")),
            cx,
            move |_, cx| MusicCoverMediaViewerObservation {
                seen: MusicCoverMediaViewerObservation::seen(observed.read(cx), &observed_id),
                _subscription: cx.observe(&observed, move |this: &mut MusicCoverMediaViewerObservation, presenter, cx| {
                    let seen = MusicCoverMediaViewerObservation::seen(presenter.read(cx), &observed_id);
                    if seen != this.seen {
                        this.seen = seen;
                        cx.notify();
                    }
                }),
            },
        );
        let is_lifted = MusicCoverMediaViewerObservation::seen(presenter.read(cx), &registry_id).0;

        let title = self.title;
        let open_registry_id = registry_id.clone();
        div()
            .id(SharedString::from(format!("{registry_id}-open")))
            .relative()
            .flex_none()
            .cursor_pointer()
            .opacity(if is_lifted { 0.0 } else { 1.0 })
            .on_click(move |_, _, cx| {
                Self::open(resolved_source.clone(), open_registry_id.clone(), title.clone(), corner_radius, cx);
            })
            .child(content)
            .child(
                WindowScreenFrameReader::new(registry_id)
                    .source_media_radii(Some(MessageBubbleRadii::uniform(px(corner_radius)))),
            )
            .into_any_element()
    }
}

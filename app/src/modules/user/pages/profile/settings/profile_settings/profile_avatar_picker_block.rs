use std::path::{Path, PathBuf};

use gpui::{
    App, AsyncApp, BoxShadow, Context, Entity, ExternalPaths, ImageFormat, Subscription, Window, black, point, white,
};
use rise_canon::RiseUploadFormat;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::window_screen_frame_reader::WindowScreenFrameReader;

use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerPresentationState, media_viewer_presentation,
};
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerResult};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions};
use crate::modules::media_picker::ui::media_picker_session::MediaPickerSession;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::user::components::profile_top::profile_avatar_album::{
    ProfileAlbumAvatarImage, ProfileAvatarAlbum, ProfileAvatarAlbumSync,
};
use crate::modules::user::engine::rise_user_profile_photo_upload::RiseProfilePhotoDraft;
use crate::modules::user::engine::rise_user_profile_photos_presentation::RiseUserProfilePhotosPresentation;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_interactions::user_profile_photos_interactions::UserProfilePhotosInteractionsStore;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_services::user_profile_photos_services::UserProfilePhotosServicesStore;
use crate::modules::vacancy::pages::create_resume::local_image_file_hooks::{
    format_of_extension, local_image_file, named_file,
};

const PNG_SIGNATURE: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];
const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// what the editors avatar circle draws from the album and media viewer
#[derive(Clone, PartialEq)]
struct ProfileEditorAvatarFacts {
    source: String,
    pending_count: usize,
    is_hero_hidden: bool,
}

struct ProfileEditorAvatarObserver {
    user_id: String,
    avatar_source: Option<String>,
    media_presenter: Entity<MediaViewerPresentationState>,
    facts: ProfileEditorAvatarFacts,
    _subscriptions: Vec<Subscription>,
}

impl ProfileEditorAvatarObserver {
    fn new(
        user_id: String,
        avatar_source: Option<String>,
        media_presenter: Entity<MediaViewerPresentationState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let albums = RiseUserProfilePhotosPresentation::shared(cx);
        let facts = ProfileEditorAvatar::facts(&user_id, avatar_source.as_deref(), &media_presenter, cx);
        Self {
            _subscriptions: vec![
                cx.observe(&albums, |this, _, cx| this.refresh(cx)),
                Self::observe_presenter(&media_presenter, cx),
            ],
            user_id,
            avatar_source,
            media_presenter,
            facts,
        }
    }

    fn observe_presenter(media_presenter: &Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) -> Subscription {
        cx.observe(media_presenter, |this: &mut Self, _, cx| this.refresh(cx))
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let next = ProfileEditorAvatar::facts(&self.user_id, self.avatar_source.as_deref(), &self.media_presenter, cx);
        if next != self.facts {
            self.facts = next;
            cx.notify();
        }
    }

    fn set_inputs(
        &mut self,
        user_id: &str,
        avatar_source: Option<&str>,
        media_presenter: &Entity<MediaViewerPresentationState>,
        cx: &mut Context<Self>,
    ) {
        if self.media_presenter != *media_presenter {
            self._subscriptions[1] = Self::observe_presenter(media_presenter, cx);
            self.media_presenter = media_presenter.clone();
        } else if self.user_id == user_id && self.avatar_source.as_deref() == avatar_source {
            return;
        }
        self.user_id = user_id.to_owned();
        self.avatar_source = avatar_source.map(str::to_owned);
        self.facts = ProfileEditorAvatar::facts(&self.user_id, self.avatar_source.as_deref(), &self.media_presenter, cx);
    }
}

/// avatar circle on the profile editing screen,
/// a tap opens the album viewer from this circle never the picker
#[derive(IntoElement)]
pub struct ProfileEditorAvatar {
    user_id: String,
    user_name: String,
    avatar_source: Option<String>,
    profile_photos_count: i64,
}

impl ProfileEditorAvatar {
    const AVATAR_SIZE: f32 = 88.0;

    pub fn new(
        user_id: impl Into<String>,
        user_name: impl Into<String>,
        avatar_source: Option<String>,
        profile_photos_count: i64,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            user_name: user_name.into(),
            avatar_source,
            profile_photos_count,
        }
    }

    fn facts(
        user_id: &str,
        avatar_source: Option<&str>,
        media_presenter: &Entity<MediaViewerPresentationState>,
        cx: &mut App,
    ) -> ProfileEditorAvatarFacts {
        let registry_id = ProfileAvatarAlbum::editor_registry_id(user_id);
        let is_hero_hidden = {
            let presenter = media_presenter.read(cx);
            presenter.hero_lift_id() == Some(registry_id.as_str()) && presenter.bubble_cell_hidden()
        };
        ProfileEditorAvatarFacts {
            source: Self::displayed_avatar_source(user_id, avatar_source, cx),
            pending_count: Self::pending_count(user_id, cx),
            is_hero_hidden,
        }
    }

    fn displayed_avatar_source(user_id: &str, avatar_source: Option<&str>, cx: &mut App) -> String {
        let album = UserProfilePhotosServicesStore::shared(cx).update(cx, |albums, cx| albums.album(user_id, cx));
        if let Some(pending) = album.as_ref().and_then(|album| album.main_photo()).filter(|photo| photo.is_pending) {
            return pending.file_url.clone();
        }
        trimming_whitespaces_and_newlines(avatar_source.unwrap_or_default()).to_owned()
    }

    fn pending_count(user_id: &str, cx: &mut App) -> usize {
        UserProfilePhotosServicesStore::shared(cx)
            .update(cx, |albums, cx| albums.photos(user_id, cx))
            .iter()
            .filter(|photo| photo.is_pending)
            .count()
    }
}

impl RenderOnce for ProfileEditorAvatar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let media_presenter = media_viewer_presentation(cx);
        let observed_presenter = media_presenter.clone();
        let initial_user_id = self.user_id.clone();
        let initial_avatar_source = self.avatar_source.clone();
        let observer = window.use_keyed_state("profile-editor-avatar-observer", cx, move |_, cx| {
            ProfileEditorAvatarObserver::new(initial_user_id, initial_avatar_source, observed_presenter, cx)
        });
        let facts = observer.update(cx, |observer, cx| {
            observer.set_inputs(&self.user_id, self.avatar_source.as_deref(), &media_presenter, cx);
            observer.facts.clone()
        });
        let side = Self::AVATAR_SIZE;
        let registry_id = ProfileAvatarAlbum::editor_registry_id(&self.user_id);
        let ProfileEditorAvatarFacts { source, pending_count, is_hero_hidden } = facts;

        let Self { user_id, user_name, avatar_source, profile_photos_count } = self;
        let drop_user_id = user_id.clone();
        let tap_user_id = user_id.clone();
        let tap_user_name = user_name.clone();
        let tap_registry_id = registry_id.clone();

        div()
            .id("profile-editor-avatar")
            .relative()
            .flex_none()
            .size(px(side))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .opacity(if is_hero_hidden { 0.0 } else { 1.0 })
            .shadow(vec![BoxShadow {
                color: black().opacity(0.22),
                offset: point(px(0.0), px(3.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .cursor_pointer()
            .occlude()
            .tooltip_label(tr("profile_photos_open"))
            .child(
                pass_through().absolute().inset_0().child(
                    WindowScreenFrameReader::new(registry_id.clone())
                        .source_media_radii(Some(MessageBubbleRadii::uniform(px(side / 2.0)))),
                ),
            )
            .child(
                ProfileAvatarAlbumSync::new("profile-editor-avatar-sync", user_id.clone(), user_name.clone())
                    .registry_id(registry_id),
            )
            .child(ProfileAlbumAvatarImage::new(
                "profile-editor-avatar-image",
                user_id,
                user_name,
                false,
                side,
                true,
                avatar_source.unwrap_or_default(),
            ))
            .when(pending_count > 0, |avatar| {
                avatar
                    .child(div().absolute().inset_0().rounded_full().bg(black().opacity(0.40)))
                    .child(div().absolute().child(progress_view().tint(white())))
            })
            .child(centred_stroke(theme.bg._100, 3.0, Shape::rounded_rect(side / 2.0)))
            .on_click(move |_, _, cx| {
                if !ProfileAvatarAlbum::can_open(&source) {
                    return;
                }
                let _ = selection_changed();
                ProfileAvatarAlbum::open(
                    &media_presenter,
                    &tap_user_id,
                    &tap_user_name,
                    &source,
                    true,
                    profile_photos_count,
                    &tap_registry_id,
                    side,
                    cx,
                );
            })
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                ProfileAvatarPickerBlock::import_external_paths(paths.paths().to_vec(), &drop_user_id, cx);
            })
    }
}

struct ProfileAvatarPickerBlockState {
    picker_open: bool,
    user_id: String,
    remaining_slots: i64,
    mutation_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProfileAvatarPickerBlockState {
    fn new(user_id: String, cx: &mut Context<Self>) -> Self {
        let albums = RiseUserProfilePhotosPresentation::shared(cx);
        let (remaining_slots, mutation_error) = Self::album_facts(&user_id, cx);
        Self {
            picker_open: false,
            user_id,
            remaining_slots,
            mutation_error,
            _subscriptions: vec![cx.observe(&albums, |this, _, cx| this.refresh(cx))],
        }
    }

    fn album_facts(user_id: &str, cx: &mut App) -> (i64, Option<String>) {
        let remaining_slots = ProfileAvatarPickerBlock::remaining_slots(user_id, cx);
        let mutation_error =
            UserProfilePhotosServicesStore::shared(cx).update(cx, |albums, cx| albums.mutation_error(user_id, cx));
        (remaining_slots, mutation_error)
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let (remaining_slots, mutation_error) = Self::album_facts(&self.user_id, cx);
        if remaining_slots != self.remaining_slots || mutation_error != self.mutation_error {
            self.remaining_slots = remaining_slots;
            self.mutation_error = mutation_error;
            cx.notify();
        }
    }

    fn set_user_id(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if self.user_id == user_id {
            return;
        }
        self.user_id = user_id.to_owned();
        let (remaining_slots, mutation_error) = Self::album_facts(&self.user_id, cx);
        self.remaining_slots = remaining_slots;
        self.mutation_error = mutation_error;
    }

    fn set_picker_open(&mut self, picker_open: bool, cx: &mut Context<Self>) {
        self.picker_open = picker_open;
        cx.notify();
    }
}

/// the only control that opens the album picker,
/// adding photos isnt tied to save and uploads run in background
#[derive(IntoElement)]
pub struct ProfileAvatarPickerBlock {
    user_id: String,
}

impl ProfileAvatarPickerBlock {
    pub fn new(user_id: impl Into<String>) -> Self {
        Self { user_id: user_id.into() }
    }

    fn remaining_slots(user_id: &str, cx: &mut App) -> i64 {
        UserProfilePhotosInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.remaining_slots(user_id, cx))
    }

    /// photos only, the preset refuses video at the fetch so anything else came from the document source
    pub fn drafts(result: &MediaPickerResult) -> Vec<RiseProfilePhotoDraft> {
        result
            .items
            .iter()
            .filter(|item| item.kind == MediaPickerItemKind::Image && !item.is_file)
            .map(|item| {
                let content_type = Self::image_content_type(item);
                RiseProfilePhotoDraft {
                    local_url: item.url.clone(),
                    filename: Self::filename(item, content_type),
                    content_type: content_type.to_owned(),
                }
            })
            .collect()
    }

    /// named from the bytes, not the picker suggestion
    /// upload policy refuses an extension that disagrees with bytes
    /// stem is exporter temp name
    fn filename(item: &MediaPickerItem, content_type: &str) -> String {
        let stem = item.url.deleting_path_extension().last_path_component();
        named_file(&stem, content_type)
    }

    fn image_content_type(item: &MediaPickerItem) -> &'static str {
        if let Some(payload) = &item.image_payload {
            if payload.data.starts_with(&PNG_SIGNATURE) {
                return RiseUploadFormat::IMAGE_PNG.mime;
            }
            if payload.data.starts_with(&JPEG_SIGNATURE) {
                return RiseUploadFormat::IMAGE_JPEG.mime;
            }
        }
        format_of_extension(&item.url.path_extension()).map_or(RiseUploadFormat::IMAGE_JPEG.mime, |format| format.mime)
    }

    /// upload fields of a dropped or pasted image file,
    /// none for anything else. stem stays,
    /// extension comes from the format
    pub fn local_image_file(path: &Path) -> Option<(Url, String, String)> {
        local_image_file(path)
    }

    /// desktop only,
    /// files dropped on the block or circle go where the sheet result goes. capped at the albums
    /// free slots
    pub fn import_external_paths(paths: Vec<PathBuf>, user_id: &str, cx: &mut App) {
        let remaining_slots = usize::try_from(Self::remaining_slots(user_id, cx)).unwrap_or(0);
        let drafts: Vec<RiseProfilePhotoDraft> = paths
            .iter()
            .filter_map(|path| Self::local_image_file(path))
            .take(remaining_slots)
            .map(|(local_url, filename, content_type)| RiseProfilePhotoDraft { local_url, filename, content_type })
            .collect();
        if drafts.is_empty() {
            return;
        }
        UserProfilePhotosInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.add_photos(drafts, cx));
    }

    pub fn import_pasteboard(user_id: String, cx: &mut App) {
        let pasteboard = UIPasteboard::general();
        let paths = pasteboard.file_paths(cx);
        if !paths.is_empty() {
            Self::import_external_paths(paths, &user_id, cx);
            return;
        }
        let Some(image) = pasteboard.image(cx) else {
            return;
        };
        if !Self::accepts_pasted_image(image.format) {
            return;
        }
        cx.spawn(async move |cx: &mut AsyncApp| {
            let written =
                cx.background_executor().spawn(async move { MediaPickerSession::write_pasted_image(&image) }).await;
            cx.update(|cx| match written {
                Ok(path) => Self::import_external_paths(vec![path], &user_id, cx),
                Err(error) => {
                    tracing::warn!(target: "riseonly", %error, "a pasted image was not staged");
                    Notifier::send(tr("sticker_pack_media_unsupported"), NotifierSendOptions::default(), cx);
                }
            });
        })
        .detach();
    }

    fn accepts_pasted_image(format: ImageFormat) -> bool {
        matches!(
            format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Webp | ImageFormat::Tiff | ImageFormat::Bmp
        )
    }

    /// mounted always so the shell can animate out
    fn picker(state: &Entity<ProfileAvatarPickerBlockState>, remaining_slots: i64, cx: &mut App) -> impl IntoElement + use<> {
        let complete_state = state.clone();
        pass_through().absolute().inset_0().child(
            MediaPickerSheet::new(
                "profile-avatar-picker-sheet",
                Binding::entity(
                    state.clone(),
                    |state: &ProfileAvatarPickerBlockState| state.picker_open,
                    |state: &mut ProfileAvatarPickerBlockState, picker_open, cx| state.set_picker_open(picker_open, cx),
                ),
                MediaPickerConfiguration::profile_avatar_album(remaining_slots),
                theme_store(cx),
                move |result, _, cx| {
                    complete_state.update(cx, |state, cx| state.set_picker_open(false, cx));
                    if result.was_cancelled {
                        return;
                    }
                    let drafts = Self::drafts(&result);
                    if drafts.is_empty() {
                        return;
                    }
                    cx.defer(move |cx| {
                        UserProfilePhotosInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.add_photos(drafts, cx));
                    });
                },
            )
            .experimental_bottom_sheet(true),
        )
    }
}

impl RenderOnce for ProfileAvatarPickerBlock {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let initial_user_id = self.user_id.clone();
        let state = window.use_keyed_state("profile-avatar-picker-block-state", cx, move |_, cx| {
            ProfileAvatarPickerBlockState::new(initial_user_id, cx)
        });
        on_change("profile-avatar-picker-block-album", self.user_id.clone(), true, window, cx, |_, user_id, _, cx| {
            if user_id.is_empty() {
                return;
            }
            UserProfilePhotosInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.open_album(user_id, cx));
        });

        let user_id = self.user_id;
        let (remaining_slots, mutation_error) = state.update(cx, |state, cx| {
            state.set_user_id(&user_id, cx);
            (state.remaining_slots, state.mutation_error.clone())
        });
        let press_state = state.clone();
        let picker = Self::picker(&state, remaining_slots, cx);

        vstack(9.0)
            .id("profile-avatar-picker-block")
            .relative()
            .w_full()
            .items_center()
            .child(
                plain("profile-avatar-picker-open")
                    .disabled(remaining_slots == 0)
                    .on_press(move |_, cx| press_state.update(cx, |state, cx| state.set_picker_open(true, cx)))
                    .opacity(if remaining_slots == 0 { 0.5 } else { 1.0 })
                    .child(
                        div().py(px(4.0)).pl(px(12.0)).pr(px(12.0)).child(
                            MainText::new(tr("select_avatar_text")).px(14.0).color(theme.primary._100),
                        ),
                    ),
            )
            .when(remaining_slots == 0, |block| {
                block.child(
                    div().w_full().pl(px(18.0)).pr(px(18.0)).child(
                        MainText::new(tr("profile_photos_limit_reached"))
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
                )
            })
            .when_some(mutation_error, |block, mutation_error| {
                block.child(
                    div().w_full().pl(px(18.0)).pr(px(18.0)).child(
                        MainText::new(mutation_error)
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .color(theme.semantic.error_100),
                    ),
                )
            })
            .child(picker)
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                Self::import_external_paths(paths.paths().to_vec(), &user_id, cx);
            })
    }
}

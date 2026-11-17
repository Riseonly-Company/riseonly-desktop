use std::rc::Rc;

use gpui::{
    AnyView, App, Bounds, Context, ElementId, Entity, IntoElement, Render, RenderOnce, SharedString,
    StyleRefinement, Subscription, Window, div, prelude::*, px,
};
use rise_i18n::tr;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, MediaViewerPresentationState, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_share_draft::MediaViewerShareDraft;
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerGalleryItemKind, MediaViewerHeroSkin,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::user::engine::rise_user_profile_photos_presentation::{
    RiseProfilePhotoAlbum, RiseUserProfilePhotosLoadState, RiseUserProfilePhotosPresentation,
};
use crate::modules::user::stores::user_profile_photos::user_profile_photos_interactions::user_profile_photos_interactions::UserProfilePhotosInteractionsStore;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_services::user_profile_photos_services::UserProfilePhotosServicesStore;

/// bridges a profile photo album into the media viewer. profile gives only current photo and
/// count so the album loads on first look
pub enum ProfileAvatarAlbum {}

/// what differs between two album rows
struct ProfileAvatarAlbumItemRow<'a> {
    id: &'a str,
    url: &'a str,
    created_at_ms: i64,
    can_set_as_main: bool,
    is_album_row: bool,
}

impl ProfileAvatarAlbum {
    /// all pages share one registry id,
    /// the avatar circle,
    /// since only the circle is registered
    pub fn registry_id(user_id: &str) -> String {
        MediaViewerFrameKeys::user_avatar(user_id)
    }

    /// separate id for the editing screen circle,
    /// both circles live at once and the registry keeps one frame per id
    pub fn editor_registry_id(user_id: &str) -> String {
        MediaViewerFrameKeys::user_avatar(user_id) + "-editor"
    }

    /// false for a letter avatar, its drawn locally and has no photo behind it
    pub fn can_open(avatar_source: &str) -> bool {
        AvatarPalette::resolved_logo_url(Some(avatar_source)).is_some()
    }

    pub fn seed_item(user_id: &str, user_name: &str, avatar_url: &str, registry_id: &str) -> MediaViewerGalleryItem {
        Self::item(
            ProfileAvatarAlbumItemRow {
                id: registry_id,
                url: avatar_url,
                created_at_ms: 0,
                can_set_as_main: false,
                is_album_row: false,
            },
            user_id,
            user_name,
            registry_id,
        )
    }

    pub fn items(
        album: &RiseProfilePhotoAlbum,
        user_id: &str,
        user_name: &str,
        registry_id: &str,
    ) -> Vec<MediaViewerGalleryItem> {
        album
            .photos
            .iter()
            .map(|photo| {
                Self::item(
                    ProfileAvatarAlbumItemRow {
                        id: &photo.id,
                        url: &photo.file_url,
                        created_at_ms: photo.created_at_ms,
                        can_set_as_main: !photo.is_main && !photo.is_pending,
                        is_album_row: !photo.is_pending,
                    },
                    user_id,
                    user_name,
                    registry_id,
                )
            })
            .collect()
    }

    /// opens the viewer on the seed, no round trip
    /// then swaps in the album once pager is mounted
    /// used by profile header and editor
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        state: &Entity<MediaViewerPresentationState>,
        user_id: &str,
        user_name: &str,
        avatar_source: &str,
        is_owner: bool,
        profile_photos_count: i64,
        registry_id: &str,
        avatar_size: f32,
        cx: &mut App,
    ) {
        if !Self::can_open(avatar_source) {
            return;
        }
        let seed = Self::seed_item(user_id, user_name, avatar_source, registry_id);
        let albums = UserProfilePhotosInteractionsStore::shared(cx);
        let source_frame = MediaCellScreenFrameRegistry::frame(registry_id).unwrap_or_else(Bounds::default);
        let hero_skin = MediaViewerHeroSkin {
            display_source: avatar_source.to_owned(),
            uses_fit_foreground: false,
            media_tail: None,
            source_media_radii: Some(MessageBubbleRadii::uniform(px(avatar_size / 2.0))),
        };
        let main_albums = albums.clone();
        let delete_albums = albums.clone();
        let handlers = MediaViewerPresentationHandlers {
            on_forward: Some(Rc::new(|item: &MediaViewerGalleryItem, cx: &mut App| Self::forward(item.clone(), cx))),
            on_set_as_main: Some(Rc::new(move |item: &MediaViewerGalleryItem, cx: &mut App| {
                main_albums.update(cx, |albums, cx| albums.set_as_main(&item.id, cx));
            })),
            on_delete_photo: Some(Rc::new(move |item: &MediaViewerGalleryItem, cx: &mut App| {
                delete_albums.update(cx, |albums, cx| albums.delete_photo(&item.id, cx));
            })),
            ..MediaViewerPresentationHandlers::default()
        };
        let chrome = if is_owner { MediaViewerChrome::OWN_PROFILE_ALBUM } else { MediaViewerChrome::PROFILE_ALBUM };
        let frame_registry_id = registry_id.to_owned();
        let user_id = user_id.to_owned();
        let state = state.clone();

        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                state,
                vec![seed],
                0,
                source_frame,
                frame_registry_id,
                hero_skin,
                chrome,
                MediaViewerGalleryScope::Complete,
                handlers,
                cx,
            )
            .await;

            if !is_owner && profile_photos_count == 1 {
                return;
            }

            let seeding = albums.update(cx, |albums, cx| albums.seed_album(&user_id, cx));
            let _ = seeding.await;
            albums.update(cx, |albums, cx| albums.open_album(&user_id, cx));
        })
        .detach();
    }

    /// staged as a local copy so the message survives the photo leaving the album
    fn forward(item: MediaViewerGalleryItem, cx: &mut App) {
        cx.spawn(async move |cx| {
            let draft = MediaViewerShareDraft::make(&item, cx).await;
            cx.update(|cx| {
                let Some(draft) = draft else {
                    Notifier::send(
                        tr("error"),
                        NotifierSendOptions {
                            r#type: NotifierType::Warning,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                    return;
                };
                ShareSheetStore::shared(cx).update(cx, |sheet, cx| sheet.present_photo(draft, cx));
            });
        })
        .detach();
    }

    fn item(
        row: ProfileAvatarAlbumItemRow<'_>,
        user_id: &str,
        user_name: &str,
        registry_id: &str,
    ) -> MediaViewerGalleryItem {
        MediaViewerGalleryItem {
            id: row.id.to_owned(),
            kind: MediaViewerGalleryItemKind::Image,
            url: row.url.to_owned(),
            thumbnail_url: None,
            width: 1.0,
            height: 1.0,
            duration: None,
            sender_id: user_id.to_owned(),
            sender_name: user_name.to_owned(),
            sender_logo: Some(row.url.to_owned()),
            created_at: if row.created_at_ms > 0 { row.created_at_ms as f64 / 1_000.0 } else { 0.0 },
            caption_title: String::new(),
            caption: String::new(),
            message_id: String::new(),
            index_in_message: 0,
            media_item_id: row.id.to_owned(),
            uses_fit_letterbox: false,
            frame_registry_id: registry_id.to_owned(),
            is_own: false,
            can_delete: false,
            can_save: true,
            can_forward: row.is_album_row,
            can_reply: false,
            can_show_in_context: false,
            can_delete_for_me: false,
            can_delete_for_everyone: false,
            can_set_as_main: row.can_set_as_main,
            is_album_row: row.is_album_row,
            display_source_override: Some(row.url.to_owned()),
            persist_media_to_disk: true,
            deletes_from_favourites: false,
            post_download: None,
            image_cache_policy: CleverImageCachePolicy::Standard,
        }
    }
}

#[derive(Clone, PartialEq)]
struct ProfileAlbumAvatarImageProps {
    user_id: String,
    user_name: String,
    is_deleted: bool,
    size: f32,
    is_owner: bool,
    remote_source: String,
    hidden: bool,
}

struct ProfileAlbumAvatarImageView {
    props: ProfileAlbumAvatarImageProps,
    pending_source: Option<String>,
    _album: Subscription,
}

impl ProfileAlbumAvatarImageView {
    fn new(props: ProfileAlbumAvatarImageProps, cx: &mut Context<Self>) -> Self {
        let presentation = RiseUserProfilePhotosPresentation::shared(cx);
        let pending_source = Self::pending_source(&props, cx);
        Self {
            props,
            pending_source,
            _album: cx.observe(&presentation, |this, _, cx| {
                let next = Self::pending_source(&this.props, cx);
                if next != this.pending_source {
                    this.pending_source = next;
                    cx.notify();
                }
            }),
        }
    }

    /// pending upload is drawn from the file on disk
    fn pending_source(props: &ProfileAlbumAvatarImageProps, cx: &mut App) -> Option<String> {
        if !props.is_owner {
            return None;
        }
        let album = UserProfilePhotosServicesStore::shared(cx).update(cx, |albums, cx| albums.album(&props.user_id, cx));
        album
            .as_ref()
            .and_then(RiseProfilePhotoAlbum::main_photo)
            .filter(|photo| photo.is_pending)
            .map(|photo| photo.file_url.clone())
    }

    fn source(&self) -> String {
        self.pending_source.clone().unwrap_or_else(|| self.props.remote_source.clone())
    }

    fn set_props(&mut self, props: ProfileAlbumAvatarImageProps, cx: &mut Context<Self>) {
        if self.props == props {
            return;
        }
        if self.props.user_id != props.user_id || self.props.is_owner != props.is_owner {
            self.pending_source = Self::pending_source(&props, cx);
        }
        self.props = props;
        cx.notify();
    }
}

impl Render for ProfileAlbumAvatarImageView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let source = self.source();
        let logo = UserLogoUi::new("profile-album-avatar")
            .source(SharedString::from(source))
            .name(SharedString::from(self.props.user_name.clone()))
            .user_id(SharedString::from(self.props.user_id.clone()))
            .is_deleted(self.props.is_deleted)
            .size(self.props.size)
            .show_border(false)
            .show_story_ring(false);
        div().size_full().opacity(if self.props.hidden { 0.0 } else { 1.0 }).child(logo)
    }
}

/// avatar circle of a profile header, own view so album changes redraw only the circle
#[derive(IntoElement)]
pub struct ProfileAlbumAvatarImage {
    id: ElementId,
    props: ProfileAlbumAvatarImageProps,
}

impl ProfileAlbumAvatarImage {
    pub fn new(
        id: impl Into<ElementId>,
        user_id: impl Into<String>,
        user_name: impl Into<String>,
        is_deleted: bool,
        size: f32,
        is_owner: bool,
        remote_source: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            props: ProfileAlbumAvatarImageProps {
                user_id: user_id.into(),
                user_name: user_name.into(),
                is_deleted,
                size,
                is_owner,
                remote_source: remote_source.into(),
                hidden: false,
            },
        }
    }

    /// hidden while the media viewer flies this avatar, part of props so the cached circle repaints
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.props.hidden = hidden;
        self
    }
}

impl RenderOnce for ProfileAlbumAvatarImage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = self.props.size;
        let initial = self.props.clone();
        let view = window.use_keyed_state(self.id, cx, move |_, cx| ProfileAlbumAvatarImageView::new(initial, cx));
        let props = self.props;
        view.update(cx, |view, cx| view.set_props(props, cx));
        AnyView::from(view).cached(StyleRefinement::default().w(px(size)).h(px(size))).replay_when_moved()
    }
}

#[derive(Clone, PartialEq)]
struct ProfileAvatarAlbumSyncProps {
    user_id: String,
    user_name: String,
    registry_id: Option<String>,
}

impl ProfileAvatarAlbumSyncProps {
    fn resolved_registry_id(&self) -> String {
        self.registry_id.clone().unwrap_or_else(|| ProfileAvatarAlbum::registry_id(&self.user_id))
    }
}

struct ProfileAvatarAlbumSyncView {
    props: ProfileAvatarAlbumSyncProps,
    media_presenter: Entity<MediaViewerPresentationState>,
    viewer_on_avatar: bool,
    fingerprint: String,
    _subscriptions: [Subscription; 2],
}

impl ProfileAvatarAlbumSyncView {
    fn new(
        props: ProfileAvatarAlbumSyncProps,
        media_presenter: Entity<MediaViewerPresentationState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let presentation = RiseUserProfilePhotosPresentation::shared(cx);
        let viewer_on_avatar = Self::viewer_on(&props, &media_presenter, cx);
        let fingerprint = Self::current_fingerprint(&props, cx);
        Self {
            _subscriptions: [
                cx.observe(&presentation, |this, _, cx| {
                    let next = Self::current_fingerprint(&this.props, cx);
                    if next != this.fingerprint {
                        this.fingerprint = next;
                        cx.notify();
                    }
                }),
                Self::observe_presenter(&media_presenter, cx),
            ],
            props,
            media_presenter,
            viewer_on_avatar,
            fingerprint,
        }
    }

    fn current_fingerprint(props: &ProfileAvatarAlbumSyncProps, cx: &mut App) -> String {
        let album = UserProfilePhotosServicesStore::shared(cx).update(cx, |albums, cx| albums.album(&props.user_id, cx));
        Self::album_fingerprint(album.as_ref())
    }

    fn viewer_on(
        props: &ProfileAvatarAlbumSyncProps,
        media_presenter: &Entity<MediaViewerPresentationState>,
        cx: &App,
    ) -> bool {
        let presenter = media_presenter.read(cx);
        presenter.is_presented() && presenter.hero_lift_id() == Some(props.resolved_registry_id().as_str())
    }

    fn observe_presenter(
        media_presenter: &Entity<MediaViewerPresentationState>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.observe(media_presenter, |this: &mut Self, presenter, cx| {
            let next = Self::viewer_on(&this.props, &presenter, cx);
            if next != this.viewer_on_avatar {
                this.viewer_on_avatar = next;
                cx.notify();
            }
        })
    }

    fn set_props(
        &mut self,
        props: ProfileAvatarAlbumSyncProps,
        media_presenter: Entity<MediaViewerPresentationState>,
        cx: &mut Context<Self>,
    ) {
        if self.media_presenter != media_presenter {
            self._subscriptions[1] = Self::observe_presenter(&media_presenter, cx);
            self.media_presenter = media_presenter;
            self.viewer_on_avatar = Self::viewer_on(&self.props, &self.media_presenter, cx);
            cx.notify();
        }
        if self.props != props {
            if self.props.user_id != props.user_id {
                self.fingerprint = Self::current_fingerprint(&props, cx);
            }
            self.props = props;
            self.viewer_on_avatar = Self::viewer_on(&self.props, &self.media_presenter, cx);
            cx.notify();
        }
    }

    fn album_fingerprint(album: Option<&RiseProfilePhotoAlbum>) -> String {
        let Some(album) = album else {
            return String::new();
        };
        let rows = album
            .photos
            .iter()
            .map(|photo| format!("{}:{}:{}", photo.id, photo.is_main, photo.file_url))
            .collect::<Vec<_>>()
            .join(",");
        let mut mutating: Vec<&str> = album.mutating_ids.iter().map(String::as_str).collect();
        mutating.sort_unstable();
        format!("{rows}|{}|{}", mutating.join(","), album.mutation_error.as_deref().unwrap_or(""))
    }

    fn sync_into_viewer(
        props: &ProfileAvatarAlbumSyncProps,
        media_presenter: &Entity<MediaViewerPresentationState>,
        cx: &mut App,
    ) {
        let registry_id = props.resolved_registry_id();
        let presented = {
            let presenter = media_presenter.read(cx);
            presenter.is_presented() && presenter.hero_lift_id() == Some(registry_id.as_str())
        };
        if !presented {
            return;
        }
        let album =
            UserProfilePhotosServicesStore::shared(cx).update(cx, |albums, cx| albums.album(&props.user_id, cx));
        let Some(album) = album else {
            return;
        };
        if album.photos.is_empty() {
            if album.load_state == RiseUserProfilePhotosLoadState::Loaded
                && album.is_visible
                && album.mutating_ids.is_empty()
                && album.mutation_error.is_none()
            {
                media_presenter.update(cx, |presenter, cx| presenter.request_animated_dismissal(cx));
            }
            return;
        }
        let items = ProfileAvatarAlbum::items(&album, &props.user_id, &props.user_name, &registry_id);
        let selecting = Self::selected_photo_id(&album, media_presenter, cx);
        media_presenter.update(cx, |presenter, cx| presenter.replace_items(items, selecting.as_deref(), cx));
    }

    fn selected_photo_id(
        album: &RiseProfilePhotoAlbum,
        media_presenter: &Entity<MediaViewerPresentationState>,
        cx: &App,
    ) -> Option<String> {
        let presenter = media_presenter.read(cx);
        let current_id = presenter.items().get(presenter.current_index()).map(|item| item.id.as_str());
        if let Some(current_id) = current_id
            && album.photos.iter().any(|photo| photo.id == current_id)
        {
            return Some(current_id.to_owned());
        }
        album.main_photo().map(|photo| photo.id.clone())
    }
}

impl Render for ProfileAvatarAlbumSyncView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fingerprint = self.fingerprint.clone();
        let viewer_is_on_this_avatar = self.viewer_on_avatar;

        let props = self.props.clone();
        let presenter = self.media_presenter.clone();
        on_change("album-fingerprint", fingerprint, false, window, cx, move |_, _, _, cx| {
            Self::sync_into_viewer(&props, &presenter, cx);
        });
        let props = self.props.clone();
        let presenter = self.media_presenter.clone();
        on_change("viewer-on-avatar", viewer_is_on_this_avatar, false, window, cx, move |_, is_open, _, cx| {
            if *is_open {
                Self::sync_into_viewer(&props, &presenter, cx);
            }
        });
        div().size_0()
    }
}

/// zero size observer that keeps the open viewer in step with the album
/// own view so every promote or delete doesnt repaint the header
#[derive(IntoElement)]
pub struct ProfileAvatarAlbumSync {
    id: ElementId,
    props: ProfileAvatarAlbumSyncProps,
}

impl ProfileAvatarAlbumSync {
    pub fn new(id: impl Into<ElementId>, user_id: impl Into<String>, user_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            props: ProfileAvatarAlbumSyncProps {
                user_id: user_id.into(),
                user_name: user_name.into(),
                registry_id: None,
            },
        }
    }

    pub fn registry_id(mut self, registry_id: impl Into<String>) -> Self {
        self.props.registry_id = Some(registry_id.into());
        self
    }
}

impl RenderOnce for ProfileAvatarAlbumSync {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let media_presenter = media_viewer_presentation(cx);
        let initial_props = self.props.clone();
        let initial_presenter = media_presenter.clone();
        let view = window.use_keyed_state(self.id, cx, move |_, cx| {
            ProfileAvatarAlbumSyncView::new(initial_props, initial_presenter, cx)
        });
        let props = self.props;
        view.update(cx, |view, cx| view.set_props(props, media_presenter, cx));
        AnyView::from(view).cached(StyleRefinement::default().absolute().size_0()).replay_when_moved()
    }
}

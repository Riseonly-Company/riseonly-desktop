use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, IntoElement, RenderOnce, SharedString, Subscription, Window, black,
    div, px, white,
};
use rise_canon::RiseGuideAnchor;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::{tr, tr_plural};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUiSurface;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use super::audio_posts_grid::{AudioPageNavigate, AudioPostsGrid};
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{CreatePostIntent, RootRoute};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackSelection;
use crate::modules::music::engine::rise_music_presentation::RiseMusicPresentation;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_listening_interruption_gate::MusicListeningInterruptionGate;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::AudioPageHeaderSkeleton;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;
use crate::modules::music::widgets::track_trim_overlay::TrackTrimOverlay;
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::story::components::story_composer_host::StoryComposerHost;
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryDTO;

type AudioPageOnClose = Rc<dyn Fn(&mut Window, &mut App)>;
type AudioPageOnCreateStory = Rc<dyn Fn(MediaPickerTrackSelection, &mut Window, &mut App)>;

struct AudioPageState {
    external_audio_owner: MusicExternalAudioOwner,
    show_story_composer: bool,
    is_use_sound_menu_presented: bool,
    story_track_seed: Option<MediaPickerTrackSelection>,
    page: Option<Rc<AudioPage>>,
    _subscriptions: [Subscription; 6],
}

impl AudioPageState {
    fn new(cx: &mut Context<Self>) -> Self {
        let music_services = MusicServicesStore::shared(cx);
        let presentation = RiseMusicPresentation::shared(cx);
        let post_services = PostServicesStore::shared(cx);
        let revisions = SaiCacheRevisionTracker::shared(cx);
        let sound_player = PostMusicPlaybackController::shared(cx);
        let foreground = RiseMediaForegroundGate::shared(cx);
        Self {
            external_audio_owner: MusicExternalAudioOwner::unique("audio_page"),
            show_story_composer: false,
            is_use_sound_menu_presented: false,
            story_track_seed: None,
            page: None,
            _subscriptions: [
                cx.observe(&music_services, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&post_services, |_, _, cx| cx.notify()),
                cx.observe(&revisions, |_, _, cx| cx.notify()),
                cx.observe(&sound_player, |_, _, cx| cx.notify()),
                cx.observe(&foreground, |this, foreground, cx| {
                    if foreground.read(cx).is_foreground() {
                        return;
                    }
                    let Some(page) = this.page.clone() else { return };
                    let owner = this.external_audio_owner.clone();
                    cx.defer(move |cx| page.stop_sound(&owner, cx));
                }),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct AudioPage {
    id: ElementId,
    track_id: String,
    source_selection: Option<MediaPickerTrackSelection>,
    source_story: Option<StoryDTO>,
    is_sheet: bool,
    on_close: AudioPageOnClose,
    on_navigate: Option<AudioPageNavigate>,
    on_create_story: Option<AudioPageOnCreateStory>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl AudioPage {
    const UNKNOWN_DURATION_CEILING_MS: i64 = 24 * 60 * 60 * 1000;

    pub fn new(id: impl Into<ElementId>, track_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            track_id: track_id.into(),
            source_selection: None,
            source_story: None,
            is_sheet: false,
            on_close: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_navigate: None,
            on_create_story: None,
            nav_store: None,
        }
    }

    pub fn source_selection(mut self, source_selection: Option<MediaPickerTrackSelection>) -> Self {
        self.source_selection = source_selection;
        self
    }

    pub fn source_story(mut self, source_story: Option<StoryDTO>) -> Self {
        self.source_story = source_story;
        self
    }

    pub fn is_sheet(mut self, is_sheet: bool) -> Self {
        self.is_sheet = is_sheet;
        self
    }

    pub fn on_close(mut self, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Rc::new(on_close);
        self
    }

    pub fn on_navigate(mut self, on_navigate: impl Fn(RootRoute, &mut Window, &mut App) + 'static) -> Self {
        self.on_navigate = Some(Rc::new(on_navigate));
        self
    }

    pub fn on_create_story(
        mut self,
        on_create_story: impl Fn(MediaPickerTrackSelection, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_create_story = Some(Rc::new(on_create_story));
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    /// fresh response first then the copy from the source post then the list
    /// screen opens ready and the network only refines it
    fn wire(&self, cx: &mut App) -> Option<RiseStoredMusicTrack> {
        let services = MusicServicesStore::shared(cx);
        if let Some(track) = services.update(cx, |services, cx| services.sound_track(&self.track_id, cx)) {
            return Some(track);
        }
        if self.is_unavailable(cx) {
            return None;
        }
        if let Some(source) = self.source_selection.as_ref()
            && source.track.id.as_deref().unwrap_or("") == self.track_id
        {
            return Some(source.track.clone());
        }
        PostServicesStore::shared(cx).update(cx, |services, cx| services.audio_posts_first_track_wire(&self.track_id, cx))
    }

    fn track(wire: Option<&RiseStoredMusicTrack>) -> Option<TrackModel> {
        wire.and_then(RiseStoredMusicTrack::as_track_model)
    }

    fn is_original_audio(wire: Option<&RiseStoredMusicTrack>) -> bool {
        wire.and_then(|wire| wire.status.as_deref()) == Some(RiseMusicTrackStatus::OriginalAudio.wire())
    }

    fn is_unavailable(&self, cx: &mut App) -> bool {
        MusicServicesStore::shared(cx).update(cx, |services, cx| services.sound_track_unavailable(&self.track_id, cx))
    }

    fn sound_failed(&self, cx: &mut App) -> bool {
        MusicServicesStore::shared(cx).update(cx, |services, cx| services.sound_track_failed(&self.track_id, cx))
    }

    /// play and use sound need a playable copy, a fresh response or
    /// the source copy while its refresh hasnt failed
    /// offline, a track never loaded keeps both inactive
    fn can_use_sound(&self, cx: &mut App) -> bool {
        let services = MusicServicesStore::shared(cx);
        if services.update(cx, |services, cx| services.sound_track(&self.track_id, cx)).is_some() {
            return true;
        }
        self.wire(cx).is_some() && !self.sound_failed(cx)
    }

    fn header_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        let has_plate = self.source_story.as_ref().is_some_and(|story| story.track_plate.is_some());
        if self.wire(cx).is_some() || has_plate || self.is_unavailable(cx) {
            return SaiWsSliceStatus::Fulfilled;
        }
        if self.sound_failed(cx) {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Pending
    }

    fn title_text(&self, wire: Option<&RiseStoredMusicTrack>, track: Option<&TrackModel>) -> String {
        let Some(track) = track else {
            return self
                .source_story
                .as_ref()
                .and_then(|story| story.track_plate.as_ref())
                .map(|plate| plate.title.clone())
                .unwrap_or_default();
        };
        if Self::is_original_audio(wire) {
            let owner = wire
                .and_then(|wire| wire.artist.as_ref())
                .map(|artist| artist.stage_name.as_str())
                .unwrap_or(track.artist.as_str());
            return format!("{} \u{b7} {owner}", tr("original_audio_title"));
        }
        track.title.clone()
    }

    fn header(self: &Rc<Self>, state: &Entity<AudioPageState>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let wire = self.wire(cx);
        let can_use_sound = self.can_use_sound(cx);
        let track = Self::track(wire.as_ref());
        let is_unavailable = self.is_unavailable(cx);
        let plate = self.source_story.as_ref().and_then(|story| story.track_plate.as_ref());
        let artist = track.as_ref().map(|track| track.artist.clone()).or_else(|| plate.map(|plate| plate.artist.clone()));
        let posts = PostServicesStore::shared(cx).update(cx, |services, cx| services.audio_posts(&self.track_id, cx));
        let secondary = theme.text.secondary;

        vstack(12.0)
            .w_full()
            .child(
                div()
                    .relative()
                    .size(px(160.0))
                    .flex_none()
                    .continuous_corners(16.0)
                    .overflow_hidden()
                    .child(self.cover(track.as_ref()))
                    .when(!is_unavailable, |cover| {
                        cover.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(self.playback_button(can_use_sound, state, cx)),
                        )
                    }),
            )
            .child(
                vstack(4.0)
                    .w_full()
                    .px(px(16.0))
                    .child(
                        MainText::new(self.title_text(wire.as_ref(), track.as_ref()))
                            .px(20.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD)
                            .number_of_lines(2),
                    )
                    .when_some(
                        artist.filter(|artist| !Self::is_original_audio(wire.as_ref()) && !artist.is_empty()),
                        |text, artist| {
                            text.child(
                                MainText::new(artist)
                                    .px(14.0)
                                    .tac(TextAlignT::Center)
                                    .color(secondary)
                                    .number_of_lines(1),
                            )
                        },
                    )
                    .when(is_unavailable, |text| {
                        text.child(
                            MainText::new(tr("post_music_unavailable"))
                                .px(14.0)
                                .tac(TextAlignT::Center)
                                .color(secondary)
                                .number_of_lines(1),
                        )
                    })
                    .when(posts.did_load, |text| {
                        text.child(
                            hstack(5.0)
                                .pt(px(2.0))
                                .children(IconUi::sized("play.rectangle.fill", px(12.0), secondary, false))
                                .child(
                                    MainText::new(tr_plural("audio_page_videos_count", posts.total_count))
                                        .px(13.0)
                                        .tac(TextAlignT::Center)
                                        .color(secondary)
                                        .number_of_lines(1),
                                ),
                        )
                    }),
            )
            .child(div().w_full().px(px(16.0)).child(self.use_sound_menu(can_use_sound, state, theme)))
            .into_any_element()
    }

    fn use_sound_menu(self: &Rc<Self>, has_wire: bool, state: &Entity<AudioPageState>, theme: &AppTheme) -> AnyElement {
        if !has_wire {
            return Self::use_sound_label(has_wire, theme).guide_anchor(RiseGuideAnchor::AudioUseSound).into_any_element();
        }
        let is_presented = Binding::entity(
            state.clone(),
            |state: &AudioPageState| state.is_use_sound_menu_presented,
            |state: &mut AudioPageState, value: bool, cx| {
                state.is_use_sound_menu_presented = value;
                cx.notify();
            },
        );
        let actions: HoldContextMenuPresentationActions = Rc::new(|_: &App| {
            vec![
                HoldContextMenuAction::new("publication", tr("create_post_kind_publication"))
                    .system_image("photo.on.rectangle.angled"),
                HoldContextMenuAction::new("reel", tr("riffs")).system_image("play.rectangle"),
                HoldContextMenuAction::new("story", tr("create_menu_story")).system_image("plus.circle"),
            ]
        });
        let (page, action_state) = (self.clone(), state.clone());
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
            page.handle_use_sound_action(&id, &action_state, window, cx);
        });
        let source_theme = theme.clone();
        let source: HoldContextMenuTapSource =
            Rc::new(move |_: &mut Window, _: &mut App| Self::use_sound_label(true, &source_theme).into_any_element());
        let opener = state.clone();
        let menu = plain(self.child_id("use-sound"))
            .on_press(move |_, cx| {
                opener.update(cx, |state, cx| {
                    state.is_use_sound_menu_presented = true;
                    cx.notify();
                });
            })
            .child(Self::use_sound_label(has_wire, theme))
            .tap_context_menu(
                self.child_id("use-sound-menu"),
                is_presented,
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            );
        div()
            .id(self.child_id("use-sound-label"))
            .aria_label(tr("audio_use_sound"))
            .child(menu)
            .guide_anchor(RiseGuideAnchor::AudioUseSound)
            .into_any_element()
    }

    fn use_sound_label(has_wire: bool, theme: &AppTheme) -> ButtonUiSurface {
        let surface = ButtonUiSurface::new();
        let ink = surface.resolved_colors(theme).fill_content;
        surface.disabled(!has_wire).child(
            hstack(8.0)
                .children(IconUi::sized("music.note", px(17.0), ink, false))
                .child(MainText::new(tr("audio_use_sound")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
        )
    }

    fn handle_use_sound_action(
        self: &Rc<Self>,
        id: &str,
        state: &Entity<AudioPageState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let owner = state.read(cx).external_audio_owner.clone();
        match id {
            "publication" => self.open_composer(CreatePostIntent::Publication, &owner, window, cx),
            "reel" => self.open_composer(CreatePostIntent::Reel, &owner, window, cx),
            "story" => {
                let Some(selection) = self.composer_selection(cx) else { return };
                self.stop_sound(&owner, cx);
                if let Some(on_create_story) = self.on_create_story.as_ref() {
                    on_create_story(selection, window, cx);
                } else {
                    state.update(cx, |state, cx| {
                        state.story_track_seed = Some(selection);
                        state.show_story_composer = true;
                        cx.notify();
                    });
                }
            }
            _ => {}
        }
    }

    fn cover(&self, track: Option<&TrackModel>) -> AnyElement {
        let cover_url = match track {
            Some(track) => Some(track.cover_image_url.clone()),
            None => self
                .source_story
                .as_ref()
                .and_then(|story| story.track_plate.as_ref())
                .map(|plate| plate.cover_url.clone()),
        };
        match cover_url.filter(|url| !url.is_empty()) {
            Some(cover_url) => MusicTrackCoverImage::new(self.child_id("cover"), cover_url)
                .size(160.0)
                .corner_radius(16.0)
                .into_any_element(),
            None => MusicCoverPlaceholder::new().corner_radius(16.0).shows_note(false).into_any_element(),
        }
    }

    fn playback_id(&self) -> String {
        format!("audio_page:{}", self.track_id)
    }

    fn playback_token(&self) -> MusicExternalAudioOwner {
        MusicExternalAudioOwner { raw_value: Cow::Owned(self.playback_id()) }
    }

    /// a plain listen, not a player session
    /// goes through the same ambient player the reels use so no mini player and the queue stays
    /// leaving the page stops it
    fn playback_button(self: &Rc<Self>, has_wire: bool, state: &Entity<AudioPageState>, cx: &mut App) -> AnyElement {
        let playback_id = self.playback_id();
        let shows_pause =
            PostMusicPlaybackController::shared(cx).read(cx).active_post_id() == Some(playback_id.as_str());
        let label = if shows_pause { tr("media_viewer_pause") } else { tr("music_play") };
        let page = self.clone();
        let state = state.clone();

        div()
            .id(self.child_id("playback-label"))
            .aria_label(label.clone())
            .tooltip_label(label)
            .size(px(64.0))
            .child(
                plain(self.child_id("playback"))
                    .disabled(!has_wire)
                    .size(px(64.0))
                    .on_press(move |_, cx| {
                        let owner = state.read(cx).external_audio_owner.clone();
                        if shows_pause {
                            page.stop_sound(&owner, cx);
                        } else {
                            page.start_sound(&owner, cx);
                        }
                    })
                    .child(
                        div()
                            .relative()
                            .size(px(64.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .circle()
                            .bg(black().opacity(0.55))
                            .child(centred_stroke(white().opacity(0.25), 1.0, Shape::Circle))
                            .child(
                                div()
                                    .relative()
                                    .left(px(if shows_pause { 0.0 } else { 2.0 }))
                                    .children(IconUi::sized(
                                        if shows_pause { "pause.fill" } else { "play.fill" },
                                        px(26.0),
                                        white(),
                                        false,
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// same rule as Riffs
    /// in a co-play session the listener first confirms that this
    /// sound pauses the room or takes them out of it
    fn start_sound(self: &Rc<Self>, owner: &MusicExternalAudioOwner, cx: &mut App) {
        if !self.can_use_sound(cx) {
            return;
        }
        let Some(wire) = self.wire(cx) else { return };
        let Some(url) = RemoteMediaURL::optional_normalized(wire.audio_url.as_deref())
            .and_then(|normalized| Url::from_string(&normalized))
        else {
            return;
        };
        let retry = {
            let page = self.clone();
            let owner = owner.clone();
            Rc::new(move |cx: &mut App| page.start_sound(&owner, cx))
        };
        let token = self.playback_token();
        let allowed = MusicListeningInterruptionGate::shared(cx)
            .update(cx, |gate, cx| gate.request_external_playback(Some(token), false, retry, None, cx));
        if !allowed {
            return;
        }
        let player = MusicPlayerState::shared(cx);
        let over_session = player.read(cx).listening_session().is_some();
        if over_session {
            let owner = owner.clone();
            player.update(cx, |player, cx| player.suspend_for_external_audio(owner, false, cx));
        }
        let duration_ms = wire.duration_ms.unwrap_or(0);
        let playback_id = self.playback_id();
        let track_id = self.track_id.clone();
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| {
            let _ = controller.play_ambient(
                &playback_id,
                &url,
                Some(track_id.as_str()),
                0,
                if duration_ms > 0 { duration_ms } else { Self::UNKNOWN_DURATION_CEILING_MS },
                false,
                true,
                over_session,
                cx,
            );
        });
    }

    fn stop_sound(&self, owner: &MusicExternalAudioOwner, cx: &mut App) {
        let token = self.playback_token();
        MusicListeningInterruptionGate::shared(cx).update(cx, |gate, cx| gate.invalidate_prompt(token, cx));
        let playback_id = self.playback_id();
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&playback_id, cx));
        let owner = owner.clone();
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.resume_after_external_audio(owner, cx));
    }

    fn composer_selection(&self, cx: &mut App) -> Option<MediaPickerTrackSelection> {
        if !self.can_use_sound(cx) {
            return None;
        }
        let wire = self.wire(cx)?;
        let story = self.source_story.as_ref();
        let start_ms =
            self.source_selection.as_ref().map(|source| source.start_ms).or_else(|| story.and_then(|story| story.track_start_ms));
        let end_ms =
            self.source_selection.as_ref().map(|source| source.end_ms).or_else(|| story.and_then(|story| story.track_end_ms));
        let (start_ms, end_ms) = TrackTrimOverlay::normalized_segment(start_ms, end_ms, wire.duration_ms);
        Some(MediaPickerTrackSelection { track: wire, start_ms, end_ms, plate: None })
    }

    fn open_composer(
        &self,
        intent: CreatePostIntent,
        owner: &MusicExternalAudioOwner,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(selection) = self.composer_selection(cx) else { return };
        self.stop_sound(owner, cx);
        let route = RootRoute::CreatePostMedia { intent, track_seed: Some(selection), journey_id: None };
        if let Some(on_navigate) = self.on_navigate.as_ref() {
            on_navigate(route, window, cx);
        } else if let Some(nav_store) = self.nav_store.as_ref() {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

#[derive(IntoElement)]
struct AudioPageContent {
    page: Rc<AudioPage>,
    state: Entity<AudioPageState>,
}

impl AudioPageContent {
    fn retry(track_id: &str, posts_failed: bool, sound_failed: bool, cx: &mut App) {
        if posts_failed {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_audio_posts_if_needed(track_id, cx));
        }
        if sound_failed {
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_sound_track(track_id, true, cx)).detach();
        }
    }
}

impl RenderOnce for AudioPageContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let page = self.page;
        let state = self.state;
        let track_id = page.track_id.clone();
        let sound_failed = page.sound_failed(cx);
        let posts_failed = PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.audio_posts_status(&track_id, cx))
            == SaiWsSliceStatus::Rejected;
        let header_status = page.header_status(cx);
        let header_rejected = header_status == SaiWsSliceStatus::Rejected;
        let shows_page_retry = header_rejected || (sound_failed && !posts_failed);
        let grid_retry: Option<Rc<dyn Fn(&mut App)>> = (posts_failed && !header_rejected).then(|| {
            let retry_track_id = track_id.clone();
            Rc::new(move |cx: &mut App| Self::retry(&retry_track_id, true, sound_failed, cx)) as Rc<dyn Fn(&mut App)>
        });

        let header_page = page.clone();
        let header_theme = theme.clone();
        let retry_track_id = track_id.clone();
        scroll_view(page.child_id("scroll"))
            .when(!page.is_sheet, |scroller| scroller.list_shell_scroll_under_chrome(cx))
            .bg(theme.bg._100)
            .child(
                vstack(18.0)
                    .w_full()
                    .pt(px(16.0))
                    .child(
                        SaiDataRender::new(header_status, move |_, cx| header_page.header(&state, &header_theme, cx))
                            .id(page.child_id("header"))
                            .error_text(tr("feed_load_error"))
                            .message_height_percent(30.0)
                            .sai_data_render_loading(|_, _| AudioPageHeaderSkeleton::new()),
                    )
                    .when(shows_page_retry, |stack| {
                        stack.child(
                            plain(page.child_id("retry"))
                                .on_press(move |_, cx| Self::retry(&retry_track_id, posts_failed, sound_failed, cx))
                                .child(MainText::new(tr("retry")).px(17.0).color(theme.primary._100)),
                        )
                    })
                    .child(
                        AudioPostsGrid::new(page.child_id("grid"), track_id)
                            .on_navigate(page.on_navigate.clone())
                            .on_retry(grid_retry),
                    )
                    .music_mini_player_bottom_padding(if page.is_sheet { 20.0 } else { 80.0 }, 0.0),
            )
    }
}

impl RenderOnce for AudioPage {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.nav_store = env::<NavigationStoreKey>(cx);
        let page = Rc::new(self);
        let state = window.use_keyed_state(page.child_id("state"), cx, |_, cx| AudioPageState::new(cx));
        let current = page.clone();
        state.update(cx, |state, _| state.page = Some(current));

        let task_track_id = page.track_id.clone();
        use_task(page.child_id("task"), page.track_id.clone(), window, cx, async move |cx| {
            if task_track_id.is_empty() {
                return;
            }
            let load = cx.update(|_, cx| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_audio_posts_if_needed(&task_track_id, cx));
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_sound_track(&task_track_id, false, cx))
            });
            if let Ok(load) = load {
                load.await;
            }
        });

        let content = AudioPageContent { page: page.clone(), state: state.clone() };
        let body: AnyElement = if page.is_sheet {
            let on_close = page.on_close.clone();
            let close_label = tr("story_action_close");
            let ink = rise_ui::theme(cx).text.primary;
            vstack(0.0)
                .size_full()
                .child(
                    hstack(None)
                        .w_full()
                        .pl(px(16.0))
                        .child(MainText::new(tr("audio_page_title")).px(17.0).font_weight(FontWeight::SEMIBOLD))
                        .child(spacer())
                        .child(
                            div()
                                .id(page.child_id("close-label"))
                                .aria_label(close_label.clone())
                                .tooltip_label(close_label)
                                .size(px(44.0))
                                .child(
                                    plain(page.child_id("close"))
                                        .size(px(44.0))
                                        .on_press(move |window, cx| on_close(window, cx))
                                        .child(
                                            div()
                                                .size(px(44.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .children(IconUi::sized("xmark", px(17.0), ink, false)),
                                        ),
                                ),
                        ),
                )
                .child(div().w_full().flex_1().min_h_0().child(content))
                .into_any_element()
        } else {
            let nav_store = page.nav_store.clone();
            SettingsWrapperUi::new(tr("audio_page_title"), move |_, cx| {
                if let Some(nav_store) = nav_store.as_ref() {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .content(content)
            .into_any_element()
        };

        let story_track_seed = state.read(cx).story_track_seed.clone();
        let disappear_page = page.clone();
        let disappear_state = state.clone();
        div()
            .relative()
            .size_full()
            .child(
                div().absolute().child(
                    StoryComposerHost::new(
                        page.child_id("story-composer"),
                        Binding::entity(
                            state,
                            |state: &AudioPageState| state.show_story_composer,
                            |state, value, _| state.show_story_composer = value,
                        ),
                    )
                    .initial_track(story_track_seed),
                ),
            )
            .child(body)
            .on_disappear(page.child_id("disappear"), move |_: Option<&mut Window>, cx: &mut App| {
                let owner = disappear_state.read(cx).external_audio_owner.clone();
                disappear_page.stop_sound(&owner, cx);
            })
    }
}

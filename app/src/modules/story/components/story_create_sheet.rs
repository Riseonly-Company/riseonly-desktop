use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton, SharedString, Subscription,
    Window, anchored, black, deferred, white,
};
use rise_ui::bottom_sheet_ui::modal_sheet_surface::{
    ModalSheetFrame, modal_sheet_centering, modal_sheet_chrome, modal_sheet_outline,
};
use rise_ui::error_text_ui::system_red;
use rise_ui::window_cover::WindowCoverFrame;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_limits::service_limit::STORY_DURATION_DAYS;
use rise_platform::haptics::{NotificationFeedbackGenerator, NotificationKind, SelectionFeedbackGenerator};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, Appearance, RiseThemeAppearance, ThemeStore};
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::stepped_slider_ui::{KNOB_SIZE, SteppedSliderUi};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::switch_ui::SwitchUi;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerFloorKey;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerItemKind, MediaPickerTrackPlate, MediaPickerTrackSelection,
};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::widgets::bottomsheets::track_picker::editor_track_picker_sheet::EditorTrackPickerSheet;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;
use crate::modules::music::widgets::track_trim_overlay::{TrackTrimOverlay, TrackTrimPlateChoice};
use crate::modules::post::components::post::post_music_plate_overlay::MusicPlateOverlay;
use crate::modules::story::stores::story::story_actions::story_actions::{StoryActionDoneHandler, StoryActionTextHandler};
use crate::modules::story::stores::story::story_actions::story_types::{StoryAudience, StoryAudienceExt};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::story::stores::story::story_services::story_share_handoff_models::StorySharePublishContext;

const DURATION_RANGE_UPPER: i64 = {
    let days = rise_limits::signed(STORY_DURATION_DAYS);
    if days > 1 { days } else { 1 }
};
const DEFAULT_SEGMENT_MS: i64 = 30_000;
const ESCAPE_KEY: &str = "escape";
const ROW_SEPARATOR_INSET: f32 = 56.0;
const TRACK_PICKER_SNAP_FRACTIONS: [f32; 2] = [0.88, 0.93];

/// called when the create sheet publishes or submits
pub type StoryCreateSheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// state of the sheet, alive as long as its drawn
struct StoryCreateSheetState {
    service: Entity<StoryServicesStore>,
    audience: StoryAudience,
    allow_screenshots: bool,
    post_to_profile: bool,
    duration_days: i64,
    attached_track: Option<RiseStoredMusicTrack>,
    attached_track_start_ms: Option<i64>,
    attached_track_end_ms: Option<i64>,
    attached_track_plate: Option<MediaPickerTrackPlate>,
    did_seed_initial_track: bool,
    show_track_picker: bool,
    show_track_trim: bool,
    track_trim_presence: Entity<Presence<()>>,
    focus: FocusHandle,
    _observations: Vec<Subscription>,
}

impl StoryCreateSheetState {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = StoryServicesStore::shared(cx);
        let observations = vec![cx.observe(&service, |_, _, cx| cx.notify())];
        let transition = Transition::opacity().combined(Transition::scale(1.02));
        Self {
            service,
            audience: StoryAudience::Everyone,
            allow_screenshots: true,
            post_to_profile: true,
            duration_days: 1,
            attached_track: None,
            attached_track_start_ms: None,
            attached_track_end_ms: None,
            attached_track_plate: None,
            did_seed_initial_track: false,
            show_track_picker: false,
            show_track_trim: false,
            track_trim_presence: cx.new(|_| Presence::new(None).with_transition(transition)),
            focus: cx.focus_handle(),
            _observations: observations,
        }
    }

    fn composer_track_selection(&self) -> Option<MediaPickerTrackSelection> {
        let attached_track = self.attached_track.as_ref()?;
        Some(MediaPickerTrackSelection {
            track: attached_track.clone(),
            start_ms: self.attached_track_start_ms.unwrap_or(0),
            end_ms: self.attached_track_end_ms.unwrap_or_else(|| Self::default_segment_end(attached_track)),
            plate: self.attached_track_plate,
        })
    }

    fn apply_plate_choice(&mut self, choice: Option<TrackTrimPlateChoice>) {
        let Some(choice) = choice else {
            self.attached_track_plate = None;
            return;
        };
        if let Some(existing) = self.attached_track_plate.as_mut() {
            existing.is_dark = choice.is_dark;
            return;
        }
        self.attached_track_plate = Some(MediaPickerTrackPlate {
            is_dark: choice.is_dark,
            center_x: 0.5,
            center_y: 0.3,
            rotation: 0.0,
            scale: 1.0,
        });
    }

    fn remove_attached_track(&mut self) {
        self.show_track_picker = false;
        self.attached_track = None;
        self.attached_track_start_ms = None;
        self.attached_track_end_ms = None;
        self.attached_track_plate = None;
    }

    fn default_segment_end(track: &RiseStoredMusicTrack) -> i64 {
        let duration = track.duration_ms.unwrap_or(0);
        if duration > 0 { DEFAULT_SEGMENT_MS.min(duration) } else { DEFAULT_SEGMENT_MS }
    }

    fn attached_track_segment(&self) -> Option<String> {
        let (start, end) = (self.attached_track_start_ms?, self.attached_track_end_ms?);
        if end <= start {
            return None;
        }
        Some(format!(
            "{}\u{2013}{}",
            RiseFormat::duration_milliseconds(start),
            RiseFormat::duration_milliseconds(end)
        ))
    }
}

#[derive(IntoElement)]
pub struct StoryCreateSheet {
    id: ElementId,
    item: MediaPickerItem,
    caption: String,
    initial_track: Option<MediaPickerTrackSelection>,
    is_presented: Binding<bool>,
    shared_handoff_context: Option<StorySharePublishContext>,
    on_published: StoryCreateSheetAction,
    on_submit: Option<StoryCreateSheetAction>,
    in_modal: bool,
}

impl StoryCreateSheet {
    pub const COVER_LEVEL: usize = 1000;

    /// create sheet as a centered modal over the dimmed window, for the tray composer and the share handoff
    pub fn modal_cover(self, presented: Binding<bool>, window: &mut Window, cx: &mut App) -> AnyElement {
        let cover = WindowCoverFrame::resolve(window, cx);
        let frame = ModalSheetFrame::for_window(window, cx);
        let theme = rise_ui::theme(cx).clone();
        let sheet = env_scope::<SheetLayerFloorKey>(Self::COVER_LEVEL).child(self);
        let surface = div().relative().w(px(frame.width)).max_w_full().h(px(frame.max_height)).occlude().child(sheet);
        let surface = modal_sheet_chrome(surface, theme.bg._100, modal_sheet_outline(&theme), 1.0);
        deferred(
            anchored().position(cover.origin).child(
                div()
                    .relative()
                    .w(cover.size.width)
                    .h(cover.size.height)
                    .child(div().absolute().inset_0().bg(black().opacity(0.5)).occlude().on_mouse_down(
                        MouseButton::Left,
                        move |_, window, cx| {
                            if !StoryServicesStore::shared(cx).read(cx).is_publishing {
                                presented.set(false, window, cx);
                            }
                        },
                    ))
                    .child(modal_sheet_centering().child(surface)),
            ),
        )
        .with_priority(Self::COVER_LEVEL)
        .into_any_element()
    }

    fn appearance(cx: &mut App) -> Appearance {
        match ThemeStore::shared(cx).read(cx).resolved_appearance() {
            RiseThemeAppearance::Light => Appearance::Light,
            RiseThemeAppearance::Dark => Appearance::Dark,
        }
    }

    pub fn new(
        id: impl Into<ElementId>,
        item: MediaPickerItem,
        caption: impl Into<String>,
        is_presented: Binding<bool>,
        on_published: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            item,
            caption: caption.into(),
            initial_track: None,
            is_presented,
            shared_handoff_context: None,
            on_published: Rc::new(on_published),
            on_submit: None,
            in_modal: false,
        }
    }

    /// the sheet sits in a centered modal surface instead of filling the window
    pub fn in_modal(mut self, in_modal: bool) -> Self {
        self.in_modal = in_modal;
        self
    }

    pub fn initial_track(mut self, initial_track: Option<MediaPickerTrackSelection>) -> Self {
        self.initial_track = initial_track;
        self
    }

    pub fn shared_handoff_context(mut self, shared_handoff_context: Option<StorySharePublishContext>) -> Self {
        self.shared_handoff_context = shared_handoff_context;
        self
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }
}

impl RenderOnce for StoryCreateSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(self.id.clone(), cx, StoryCreateSheetState::new);
        let initial_track = self.initial_track.clone();
        state.update(cx, |state, _| {
            if state.did_seed_initial_track {
                return;
            }
            state.did_seed_initial_track = true;
            let Some(initial_track) = initial_track else {
                return;
            };
            state.attached_track = Some(initial_track.track);
            state.attached_track_start_ms = Some(initial_track.start_ms);
            state.attached_track_end_ms = Some(initial_track.end_ms);
            state.attached_track_plate = initial_track.plate;
        });

        let in_modal = self.in_modal;
        let parts = StoryCreateSheetParts {
            id: self.id.clone(),
            state: state.clone(),
            item: Rc::new(self.item),
            caption: Rc::new(self.caption),
            is_presented: self.is_presented,
            shared_handoff_context: self.shared_handoff_context,
            on_published: self.on_published,
            on_submit: self.on_submit,
        };
        let is_busy = parts.is_busy(cx);
        let publish_error = state.read(cx).service.read(cx).publish_error.clone();
        let focus = state.read(cx).focus.clone();
        let appear_focus = focus.clone();
        let return_focus = focus.clone();
        let has_overlay = {
            let state = state.read(cx);
            state.show_track_picker || state.show_track_trim
        };
        on_change(parts.child_id("return-focus"), has_overlay, false, window, cx, move |_, has_overlay, window, cx| {
            if !*has_overlay {
                window.focus(&return_focus, cx);
            }
        });

        let (key_parts, disappear_service) = (parts.clone(), state.read(cx).service.clone());
        let track_trim = parts.track_trim_overlay(&theme, window, cx);

        div()
            .id(parts.child_id("root"))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .when(!in_modal, |root| root.pt(WindowCoverFrame::resolve(window, cx).safe_top))
            .bg(theme.bg._100)
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key != ESCAPE_KEY {
                    return;
                }
                cx.stop_propagation();
                if key_parts.state.read(cx).show_track_picker {
                    key_parts.update_state(cx, |state| state.show_track_picker = false);
                    return;
                }
                if !key_parts.is_busy(cx) {
                    key_parts.is_presented.set(false, window, cx);
                }
            })
            .child(parts.header(&theme, is_busy))
            .child(
                div().flex_1().min_h_0().child(
                    scroll_view(parts.child_id("scroll")).shows_indicators(false).child(
                        vstack(16.0)
                            .w_full()
                            .pt(px(8.0))
                            .child(parts.preview_card(&theme, cx))
                            .child(parts.music_section(&theme, is_busy, cx))
                            .child(parts.audience_section(&theme, cx))
                            .child(parts.toggles_section(&theme, cx))
                            .child(parts.duration_section(&theme, is_busy, cx))
                            .children(publish_error.map(|err| {
                                div().w_full().px(px(16.0)).child(
                                    MainText::new(err).px(13.0).tac(TextAlignT::Center).color(system_red(cx)),
                                )
                            }))
                            .child(parts.publish_button(&theme, is_busy))
                            .child(spacer_min_h(24.0)),
                    ),
                ),
            )
            .child(div().absolute().inset_0().child(parts.track_picker(&theme)))
            .children(track_trim)
            .on_appear(parts.child_id("appear"), move |window, cx| window.focus(&appear_focus, cx))
            .on_disappear(parts.child_id("disappear"), move |_, cx| {
                let is_wait_error = disappear_service.read(cx).publish_error.as_deref()
                    == Some(tr("story_publish_wait_previous").as_str());
                if is_wait_error {
                    StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.dismiss_publish_error(cx));
                }
            })
    }
}

#[derive(Clone)]
struct StoryCreateSheetParts {
    id: ElementId,
    state: Entity<StoryCreateSheetState>,
    item: Rc<MediaPickerItem>,
    caption: Rc<String>,
    is_presented: Binding<bool>,
    shared_handoff_context: Option<StorySharePublishContext>,
    on_published: StoryCreateSheetAction,
    on_submit: Option<StoryCreateSheetAction>,
}

impl StoryCreateSheetParts {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn keyed_id(&self, name: String) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    /// only the share queue waits here, it publishes items one by one and advances when this sheet closes
    fn is_busy(&self, cx: &App) -> bool {
        self.shared_handoff_context.is_some() && self.state.read(cx).service.read(cx).is_publishing
    }

    fn update_state(&self, cx: &mut App, action: impl FnOnce(&mut StoryCreateSheetState)) {
        self.state.update(cx, |state, cx| {
            action(state);
            cx.notify();
        });
    }

    fn header(&self, theme: &AppTheme, is_busy: bool) -> impl IntoElement {
        let is_presented = self.is_presented.clone();
        zstack(Alignment::Center)
            .w_full()
            .flex_none()
            .px(px(16.0))
            .py(px(14.0))
            .child(
                MainText::new(tr("story_create_title"))
                    .px(17.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1),
            )
            .child(
                hstack(0.0)
                    .w_full()
                    .child(
                        plain(self.child_id("cancel"))
                            .on_press(move |window, cx| is_presented.set(false, window, cx))
                            .disabled(is_busy)
                            .child(MainText::new(tr("common_cancel")).px(16.0).color(theme.primary._100)),
                    )
                    .child(spacer_min_w(0.0)),
            )
    }

    fn preview_card(&self, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let preview = self.preview_source();
        let is_video = self.item.kind == MediaPickerItemKind::Video;
        let duration = self.preview_duration_text();
        let trailing_inset = if duration.is_none() { 0.0 } else { 64.0 };
        let caption = trimming_whitespaces_and_newlines(&self.caption).to_owned();

        let play = (is_video && preview.is_some()).then(|| {
            pass_through()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .relative()
                        .size(px(34.0))
                        .child(
                            div()
                                .absolute()
                                .top(px(2.0))
                                .children(IconUi::sized("play.fill", px(34.0), black().opacity(0.35), false)),
                        )
                        .child(div().absolute().children(IconUi::sized("play.fill", px(34.0), white(), false))),
                )
        });
        let duration_badge = duration.map(|duration| {
            pass_through().absolute().right(px(10.0)).bottom(px(10.0)).child(
                div().px(px(8.0)).py(px(4.0)).rounded_full().bg(black().opacity(0.55)).child(
                    MainText::new(duration)
                        .px(12.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(white())
                        .number_of_lines(1),
                ),
            )
        });
        let caption_badge = (!caption.is_empty()).then(|| {
            div()
                .absolute()
                .left(px(10.0))
                .right(px(10.0 + trailing_inset))
                .bottom(px(10.0))
                .flex()
                .flex_col()
                .items_start()
                .child(
                    div()
                        .max_w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .px(px(10.0))
                        .py(px(6.0))
                        .continuous_corners(8.0)
                        .bg(black().opacity(0.45))
                        .child(
                            MainText::new(caption)
                                .px(13.0)
                                .tac(TextAlignT::Left)
                                .color(white())
                                .number_of_lines(2),
                        ),
                )
        });

        div().w_full().flex().justify_center().px(px(16.0)).child(
            div()
                .relative()
                .w_full()
                .max_w(px(300.0))
                .aspect_ratio(4.0 / 5.0)
                .continuous_corners(20.0)
                .overflow_hidden()
                .child(self.media_with_plate(theme, "preview", cx))
                .children(play)
                .children(duration_badge)
                .children(caption_badge),
        )
    }

    /// story media fitted on black with the plate where it sat in the editor
    fn media_with_plate(&self, theme: &AppTheme, slot: &'static str, cx: &App) -> AnyElement {
        let is_video = self.item.kind == MediaPickerItemKind::Video;
        let media: AnyElement = match self.preview_source() {
            Some(preview) => CleverImageUi::new(preview)
                .id(self.child_id(slot))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .shows_progress_indicator(false)
                .into_any_element(),
            None => div()
                .children(IconUi::sized(
                    if is_video { "video.fill" } else { "photo.fill" },
                    px(26.0),
                    theme.text.secondary,
                    false,
                ))
                .into_any_element(),
        };
        let state = self.state.read(cx);
        let plate = state.attached_track_plate.zip(state.attached_track.as_ref()).map(|(plate, track)| {
            MusicPlateOverlay::new(
                "story-create-music-plate",
                track.title.clone().unwrap_or_default(),
                track.artist.as_ref().map(|artist| artist.stage_name.clone()).unwrap_or_default(),
                RemoteMediaURL::optional_normalized(track.cover_url.as_deref()).unwrap_or_default(),
                track.id.clone().unwrap_or_default(),
                plate.is_dark,
                plate.center_x,
                plate.center_y,
                plate.rotation,
                plate.scale,
            )
            .media_aspect(self.preview_media_aspect())
            .media_fills(false)
        });

        div()
            .absolute()
            .inset_0()
            .bg(black())
            .child(div().absolute().inset_0().flex().items_center().justify_center().child(media))
            .children(plate.map(|plate| div().absolute().inset_0().child(plate)))
            .into_any_element()
    }

    fn preview_duration_text(&self) -> Option<String> {
        if self.item.kind != MediaPickerItemKind::Video {
            return None;
        }
        let duration = self.item.video_payload.as_ref()?.duration;
        (duration > 0.0).then(|| RiseFormat::duration_time_interval(duration))
    }

    fn preview_media_aspect(&self) -> Option<f32> {
        let size = self
            .item
            .video_payload
            .as_ref()
            .map(|video| video.pixel_size)
            .or_else(|| self.item.image_payload.as_ref().map(|image| image.pixel_size))?;
        (size.width > 0.0 && size.height > 0.0).then(|| size.width / size.height)
    }

    fn preview_source(&self) -> Option<String> {
        if let Some(poster_url) = self.item.video_payload.as_ref().and_then(|video| video.poster_url.as_ref()) {
            return Some(poster_url.absolute_string().to_owned());
        }
        Some(self.item.url.absolute_string().to_owned())
    }

    fn audience_options() -> [StoryAudience; 3] {
        [StoryAudience::Everyone, StoryAudience::Contacts, StoryAudience::CloseFriends]
    }

    fn music_section(&self, theme: &AppTheme, is_busy: bool, cx: &App) -> AnyElement {
        let state = self.state.read(cx);
        let Some(attached_track) = state.attached_track.as_ref() else {
            let open = self.clone();
            return div()
                .w_full()
                .px(px(16.0))
                .child(
                    plain(self.child_id("attach-music"))
                        .on_press(move |_, cx| open.update_state(cx, |state| state.show_track_picker = true))
                        .disabled(is_busy)
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .px(px(12.0))
                        .py(px(11.0))
                        .bg(theme.bg._200)
                        .continuous_corners(14.0)
                        .child(Self::tinted_icon(SettingsIconPalette::memory_audio(), "music.note"))
                        .child(
                            MainText::new(tr("post_attach_music"))
                                .px(15.0)
                                .tac(TextAlignT::Left)
                                .font_weight(FontWeight::MEDIUM),
                        )
                        .child(spacer_min_w(0.0))
                        .children(IconUi::sized("chevron.right", px(13.0), theme.text.secondary, false)),
                )
                .into_any_element();
        };

        let stage_name = attached_track.artist.as_ref().map(|artist| artist.stage_name.clone()).unwrap_or_default();
        let segment = state.attached_track_segment().map(|segment| {
            MainText::new(if stage_name.is_empty() { segment } else { format!(" \u{b7} {segment}") })
                .px(12.0)
                .color(theme.text.secondary)
                .number_of_lines(1)
                .flex_none()
        });
        let (open_trim, remove) = (self.clone(), self.clone());
        let cover_url = RemoteMediaURL::optional_normalized(attached_track.cover_url.as_deref()).unwrap_or_default();

        div()
            .w_full()
            .px(px(16.0))
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(12.0))
                    .py(px(10.0))
                    .bg(theme.bg._200)
                    .continuous_corners(14.0)
                    .child(
                        plain(self.child_id("edit-track"))
                            .on_press(move |_, cx| open_trim.update_state(cx, |state| state.show_track_trim = true))
                            .disabled(is_busy)
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                MusicTrackCoverImage::new(self.child_id("track-cover"), cover_url)
                                    .size(40.0)
                                    .corner_radius(8.0),
                            )
                            .child(
                                vstack(2.0)
                                    .flex_1()
                                    .min_w_0()
                                    .align(HorizontalAlignment::Leading)
                                    .child(
                                        MainText::new(Self::attached_track_title(attached_track))
                                            .px(14.0)
                                            .font_weight(FontWeight::MEDIUM)
                                            .number_of_lines(1),
                                    )
                                    .child(
                                        hstack(0.0)
                                            .w_full()
                                            .child(
                                                MainText::new(stage_name)
                                                    .px(12.0)
                                                    .color(theme.text.secondary)
                                                    .number_of_lines(1),
                                            )
                                            .children(segment),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id(self.child_id("remove-track-label"))
                            .flex_none()
                            .aria_label(tr("delete_action"))
                            .tooltip_label(tr("delete_action"))
                            .child(
                                plain(self.child_id("remove-track"))
                                    .on_press(move |_, cx| remove.update_state(cx, |state| state.remove_attached_track()))
                                    .disabled(is_busy)
                                    .size(px(28.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(theme.bg._300)
                                    .rounded_full()
                                    .children(IconUi::sized("xmark", px(12.0), theme.text.secondary, false)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn tinted_icon(tint: Hsla, icon: &str) -> impl IntoElement {
        div()
            .flex_none()
            .size(px(32.0))
            .rounded_full()
            .bg(tint)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(icon, px(14.0), white(), false))
    }

    fn row_separator(theme: &AppTheme) -> impl IntoElement {
        div()
            .w_full()
            .pl(px(ROW_SEPARATOR_INSET))
            .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.4)))
    }

    fn audience_section(&self, theme: &AppTheme, cx: &mut App) -> impl IntoElement {
        let rows: Vec<AnyElement> = Self::audience_options()
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let row = self.audience_row(*option, theme, cx);
                div().w_full().when(index > 0, |rows| rows.child(Self::row_separator(theme))).child(row).into_any_element()
            })
            .collect();
        div()
            .w_full()
            .px(px(16.0))
            .child(
                vstack(8.0)
                    .w_full()
                    .align(HorizontalAlignment::Leading)
                    .child(
                        MainText::new(tr("story_audience_title"))
                            .px(13.0)
                            .tac(TextAlignT::Left)
                            .color(theme.text.secondary)
                            .m_l(4.0),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .bg(theme.bg._200)
                            .continuous_corners(14.0)
                            .overflow_hidden()
                            .children(rows),
                    ),
            )
    }

    fn audience_row(&self, option: StoryAudience, theme: &AppTheme, cx: &mut App) -> impl IntoElement {
        let selected = self.state.read(cx).audience == option;
        let tint = Self::audience_color(option, cx);
        let press = self.clone();
        plain(self.keyed_id(format!("audience-{}", option.wire())))
            .on_press(move |_, cx| {
                let _ = SelectionFeedbackGenerator::new().selection_changed();
                press.update_state(cx, |state| state.audience = option);
            })
            .w_full()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(12.0))
            .py(px(11.0))
            .child(Self::tinted_icon(tint, option.icon()))
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(
                        MainText::new(tr(option.i18n_key()))
                            .px(15.0)
                            .tac(TextAlignT::Left)
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(
                        MainText::new(tr(option.hint_i18n_key()))
                            .px(11.5)
                            .tac(TextAlignT::Left)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
            )
            .children(selected.then(|| IconUi::sized("checkmark", px(14.0), theme.primary._100, false)).flatten())
    }

    fn audience_color(option: StoryAudience, cx: &mut App) -> Hsla {
        let appearance = StoryCreateSheet::appearance(cx);
        match option {
            StoryAudience::Everyone => UIColor::system_blue(),
            StoryAudience::Contacts => UIColor::system_green(),
            StoryAudience::CloseFriends => UIColor::system_orange(),
            StoryAudience::Selected => UIColor::system_purple(),
        }
        .resolved_color(appearance)
    }

    fn toggles_section(&self, theme: &AppTheme, cx: &mut App) -> impl IntoElement {
        let appearance = StoryCreateSheet::appearance(cx);
        let allow_screenshots = Binding::entity(
            self.state.clone(),
            |state: &StoryCreateSheetState| state.allow_screenshots,
            |state, value, _| state.allow_screenshots = value,
        );
        let post_to_profile = Binding::entity(
            self.state.clone(),
            |state: &StoryCreateSheetState| state.post_to_profile,
            |state, value, _| state.post_to_profile = value,
        );
        div().w_full().px(px(16.0)).child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .bg(theme.bg._200)
                .continuous_corners(14.0)
                .overflow_hidden()
                .child(self.toggle_row(
                    "allow-screenshots",
                    "camera.viewfinder",
                    UIColor::system_indigo().resolved_color(appearance),
                    tr("story_allow_screenshots"),
                    tr("story_allow_screenshots_hint"),
                    allow_screenshots,
                    theme,
                ))
                .child(Self::row_separator(theme))
                .child(self.toggle_row(
                    "post-to-profile",
                    "person.crop.square.fill",
                    UIColor::system_teal().resolved_color(appearance),
                    tr("story_post_to_profile"),
                    tr("story_post_to_profile_hint"),
                    post_to_profile,
                    theme,
                )),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn toggle_row(
        &self,
        name: &'static str,
        icon: &str,
        tint: Hsla,
        title: String,
        hint: String,
        value: Binding<bool>,
        theme: &AppTheme,
    ) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .px(px(12.0))
            .py(px(11.0))
            .child(Self::tinted_icon(tint, icon))
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(title).px(15.0).tac(TextAlignT::Left).font_weight(FontWeight::MEDIUM))
                    .child(
                        MainText::new(hint)
                            .px(11.5)
                            .tac(TextAlignT::Left)
                            .color(theme.text.secondary)
                            .number_of_lines(2),
                    ),
            )
            .child(SwitchUi::new(self.child_id(name), value))
    }

    fn duration_section(&self, theme: &AppTheme, is_busy: bool, cx: &App) -> impl IntoElement {
        let duration_days = self.state.read(cx).duration_days;
        let (change, commit) = (self.clone(), self.clone());
        let days = 1..=DURATION_RANGE_UPPER;
        let labels: Vec<AnyElement> = days
            .clone()
            .map(|day| {
                let is_current = duration_days == day;
                div()
                    .w(px(KNOB_SIZE))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(
                        MainText::new(day.to_string())
                            .px(12.0)
                            .tac(TextAlignT::Center)
                            .font_weight(if is_current { FontWeight::BOLD } else { FontWeight::NORMAL })
                            .color(if is_current { theme.text.primary } else { theme.text.secondary }),
                    )
                    .into_any_element()
            })
            .collect();
        div().w_full().px(px(16.0)).child(
            vstack(8.0)
                .w_full()
                .align(HorizontalAlignment::Leading)
                .child(
                    hstack(8.0)
                        .w_full()
                        .px(px(4.0))
                        .child(
                            MainText::new(tr("story_duration_title"))
                                .px(13.0)
                                .tac(TextAlignT::Left)
                                .color(theme.text.secondary),
                        )
                        .child(spacer_min_w(0.0))
                        .child(
                            MainText::new(format!("{duration_days} {}", Self::duration_label(duration_days)))
                                .px(13.0)
                                .tac(TextAlignT::Right)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary),
                        ),
                )
                .child(
                    vstack(4.0)
                        .w_full()
                        .px(px(12.0))
                        .py(px(12.0))
                        .bg(theme.bg._200)
                        .continuous_corners(14.0)
                        .child(
                            SteppedSliderUi::new(
                                self.child_id("duration"),
                                duration_days as f64,
                                1.0..=DURATION_RANGE_UPPER as f64,
                                1.0,
                                move |value, _, cx| {
                                    change.update_state(cx, |state| state.duration_days = value.round() as i64);
                                },
                                move |value, _, cx| {
                                    commit.update_state(cx, |state| state.duration_days = value.round() as i64);
                                },
                            )
                            .breakpoints(days.map(|day| day as f64).collect())
                            .is_enabled(!is_busy),
                        )
                        .child(hstack(0.0).w_full().justify_between().children(labels)),
                ),
        )
    }

    fn duration_label(day: i64) -> String {
        if day == 1 { tr("story_duration_day") } else { tr("story_duration_days") }
    }

    fn publish_button(&self, theme: &AppTheme, is_busy: bool) -> impl IntoElement {
        let press = self.clone();
        let mut label = hstack(8.0).w_full().min_h(px(38.0)).justify_center();
        label = if is_busy {
            label.child(progress_view().tint(theme.primary._100)).child(
                MainText::new(tr("story_publishing"))
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.primary._100),
            )
        } else {
            label.children(IconUi::sized("paperplane.fill", px(14.0), theme.primary._100, false)).child(
                MainText::new(tr("story_publish"))
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.primary._100),
            )
        };
        div().w_full().px(px(16.0)).child(
            LiquidGlassButton::new(self.child_id("publish"), LiquidGlassButtonShape::Capsule)
                .disabled(is_busy)
                .on_press(move |window, cx| press.publish(window, cx))
                .child(label),
        )
    }

    fn track_picker(&self, theme: &AppTheme) -> impl IntoElement {
        let is_bottom_sheet = Binding::entity(
            self.state.clone(),
            |state: &StoryCreateSheetState| state.show_track_picker,
            |state, value, _| state.show_track_picker = value,
        );
        let parts = self.clone();
        BottomSheetUi::new(
            self.child_id("track-picker"),
            is_bottom_sheet,
            move |_, cx| {
                let (pick, edit, remove, dismiss) = (parts.clone(), parts.clone(), parts.clone(), parts.clone());
                EditorTrackPickerSheet::new(
                    parts.child_id("track-picker-content"),
                    parts.state.read(cx).composer_track_selection(),
                    move |track: RiseStoredMusicTrack, cx: &mut App| {
                        pick.update_state(cx, |state| {
                            if state.attached_track.as_ref().map(|attached| &attached.id) != Some(&track.id) {
                                state.attached_track_start_ms = Some(0);
                                state.attached_track_end_ms = Some(StoryCreateSheetState::default_segment_end(&track));
                                state.attached_track = Some(track);
                            }
                            state.show_track_picker = false;
                            state.show_track_trim = true;
                        });
                    },
                    move |_, cx| {
                        edit.update_state(cx, |state| {
                            state.show_track_picker = false;
                            state.show_track_trim = true;
                        });
                    },
                    move |_, cx| remove.update_state(cx, |state| state.remove_attached_track()),
                    move |_, cx| dismiss.update_state(cx, |state| state.show_track_picker = false),
                )
            },
            |_, _| gpui::Empty,
        )
        .snap_fractions(TRACK_PICKER_SNAP_FRACTIONS.to_vec())
        .footer_shows_top_border(false)
        .sheet_background(Some(theme.bg._100))
    }

    fn track_trim_overlay(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        let (presence, is_shown, attached) = {
            let state = self.state.read(cx);
            (state.track_trim_presence.clone(), state.show_track_trim && state.attached_track.is_some(), state.attached_track.clone())
        };
        presence.update(cx, |presence, cx| {
            presence.set_in(is_shown.then_some(()), ease_in_out(0.22), now, cx);
        });
        presence.read(cx).value(now)?;
        let track = attached?;
        let (start_ms, end_ms, plate) = {
            let state = self.state.read(cx);
            (
                state.attached_track_start_ms.unwrap_or(0),
                state.attached_track_end_ms.unwrap_or_else(|| StoryCreateSheetState::default_segment_end(&track)),
                state.attached_track_plate,
            )
        };
        let backdrop = self.media_with_plate(theme, "trim-backdrop", cx);
        let (confirm, cancel) = (self.clone(), self.clone());
        let overlay = TrackTrimOverlay::new(
            SharedString::new_static("story-create-track-trim"),
            track,
            start_ms,
            end_ms,
            move |start, end, plate, _, cx| {
                confirm.update_state(cx, |state| {
                    state.attached_track_start_ms = Some(start);
                    state.attached_track_end_ms = Some(end);
                    state.apply_plate_choice(plate);
                    state.show_track_trim = false;
                });
            },
            move |_, cx| cancel.update_state(cx, |state| state.show_track_trim = false),
        )
        .allows_plate(true)
        .initial_plate(plate.map(|plate| TrackTrimPlateChoice { is_dark: plate.is_dark }))
        .opaque_backdrop(true)
        .backdrop(Some(backdrop));

        presence
            .read(cx)
            .render_transitioned(now, window, move |_| div().absolute().inset_0().child(overlay))
            .map(IntoElement::into_any_element)
    }

    fn publish(&self, window: &mut Window, cx: &mut App) {
        if let Some(on_submit) = self.on_submit.as_ref() {
            on_submit(window, cx);
            return;
        }
        if self.is_busy(cx) {
            return;
        }
        self.submit(window, cx);
    }

    fn submit(&self, window: &mut Window, cx: &mut App) {
        let is_shared_handoff = self.shared_handoff_context.is_some();
        let refused = Rc::new(Cell::new(false));
        let (audience, allow_screenshots, post_to_profile, duration_days, attached_track, service) = {
            let state = self.state.read(cx);
            (
                state.audience,
                state.allow_screenshots,
                state.post_to_profile,
                state.duration_days,
                state.composer_track_selection(),
                state.service.clone(),
            )
        };

        let on_success: StoryActionDoneHandler = {
            let is_presented = self.is_presented.clone();
            let on_published = self.on_published.clone();
            let handle = window.window_handle();
            Rc::new(move |cx: &mut App| {
                if !is_shared_handoff {
                    return;
                }
                let _ = NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Success);
                let (is_presented, on_published) = (is_presented.clone(), on_published.clone());
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        is_presented.set(false, window, cx);
                        on_published(window, cx);
                    });
                });
            })
        };
        let on_error: StoryActionTextHandler = {
            let refused = refused.clone();
            Rc::new(move |_: String, _: &mut App| {
                refused.set(true);
                let _ = NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Error);
            })
        };

        let item = (*self.item).clone();
        let caption = (*self.caption).clone();
        let idempotency_key = self.shared_handoff_context.as_ref().map(StorySharePublishContext::idempotency_key);
        let context = self.shared_handoff_context.clone();
        StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.publish_story(
                item,
                caption,
                audience,
                allow_screenshots,
                post_to_profile,
                duration_days,
                Vec::new(),
                attached_track,
                idempotency_key,
                context,
                Some(on_success),
                Some(on_error),
                cx,
            );
        });

        let (is_presented, on_published) = (self.is_presented.clone(), self.on_published.clone());
        window.defer(cx, move |window, cx| {
            if is_shared_handoff || refused.get() || !service.read(cx).is_publishing {
                return;
            }
            is_presented.set(false, window, cx);
            on_published(window, cx);
        });
    }

    fn attached_track_title(track: &RiseStoredMusicTrack) -> String {
        if track.status.as_deref() == Some(RiseMusicTrackStatus::OriginalAudio.wire()) {
            tr("original_audio_title")
        } else {
            track.title.clone().unwrap_or_default()
        }
    }
}

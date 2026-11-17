use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui::{
    AnyView, AnyWindowHandle, App, Bounds, Context, Edges, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent,
    MouseDownEvent, Pixels, Point, RenderImage, SharedString, Size, StyleRefinement, Task, WeakEntity, Window, black, point, size,
    white,
};
use image::{Frame, RgbaImage};
use rise_canon::{
    RiseGuideSignal, RiseLimits, RiseMediaLinkPreviewMode, RiseMediaLinkStyle, RiseStoryCanvasPhotoShape, RiseTimingCanon,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args, tr_plural};
use rise_media::video::av_asset::AVURLAsset;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::paths::temporary_directory;
use rise_platform::photo_library::{ExportOutcome, ImageContentMode, ImageDeliveryMode, LibraryAsset};
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::liquid_chrome::{LiquidChrome, LiquidChromeModifiers, LiquidGlassContrast};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::modifiers::ModifierExt;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::swiftui::transition::{Edge, Presence, Transition, TransitionModifiers};
use rise_ui::window_cover::WindowCoverFrame;
use smallvec::SmallVec;

use crate::app::router::navigation_fullscreen_pop::NavigationInteractivePopGuard;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIResponder};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::media_viewer::media_viewer_types::MediaViewerTransition;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::input_ui::InputUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::people_picker_sheet::{
    PeoplePickerHost, PeoplePickerHostContext, PeoplePickerMode, PeoplePickerStore,
};
use crate::modules::chat::components::chat::bubble::web_m_sticker_playback::WebMStickerFile;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_panel::ChatInputPickerPanel;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionsStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItemKind, MediaPickerResult, MediaPickerTrackPlate, MediaPickerTrackSelection,
};
use crate::modules::media_picker::editor::editor_adjustments_panel::EditorAdjustmentsPanel;
use crate::modules::media_picker::editor::editor_adjustments_renderer::EditorAdjustmentsRenderer;
use crate::modules::media_picker::editor::editor_bottom_bar::{EditorBottomBar, EditorTagPeoplePanel};
use crate::modules::media_picker::editor::editor_brush_panel::EditorBrushPanel;
use crate::modules::media_picker::editor::media_editor_presentation_host::{MEDIA_EDITOR_BACKDROP, media_edge_color};
use crate::modules::media_picker::editor::editor_color_picker_bridge::{
    EditorColorPickerBridge, EditorColorPickerHost, EditorColorPickerLauncher,
};
use crate::modules::media_picker::editor::editor_crop_auto_fit::EditorCropAutoFitPlanner;
use crate::modules::media_picker::editor::editor_crop_overlay::EditorCropPanel;
use crate::modules::media_picker::editor::editor_edit_preview_cache::EditorEditPreviewCache;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_image_layout::{
    EditorImageLayout, editor_main_chrome_height_reader,
};
use crate::modules::media_picker::editor::editor_image_rect::EditorImageRect;
use crate::modules::media_picker::editor::editor_image_stage::{
    EditorImageStage, EditorImageStageDragChanged, EditorImageStageDragEnded,
};
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::modules::media_picker::editor::editor_music_plate::EditorMusicPlateCoverStore;
use crate::modules::media_picker::editor::editor_music_track_lane::{
    EditorMusicLaneRepresentable, EditorMusicPreviewObservers, EditorSoundSheet,
};
use crate::modules::media_picker::editor::editor_link_entity::{
    EDITOR_LINK_CUSTOMIZE, EditorLinkCustomizeContent, EditorLinkDraft, EditorLinkEntity, EditorLinkLiveSample,
    EditorLinkStyleSwatches, MediaLinkAddress, MediaLinkAddressOutcome,
};
use crate::modules::media_picker::editor::editor_state::{
    EditorDrawStroke, EditorDrawStrokeTool, EditorImageAdjustments, EditorImageState, EditorMusicPlateEntity, EditorPersonTagEntity,
    EditorTextEntity, EditorTextEntityStyleMode, EditorTextEntityTextAlignment, EditorVideoState,
};
use crate::modules::media_picker::editor::editor_sticker_entity::{EditorStickerArtStore, EditorStickerEntity};
use crate::modules::media_picker::editor::editor_text_editor_overlay::EditorTextEditorOverlay;
use crate::modules::media_picker::editor::editor_thickness_track::{EditorBrushPreviewCircle, EditorThicknessTrack};
use crate::modules::media_picker::editor::editor_video_playback_controller::{
    EditorVideoPlaybackController, EditorVideoPlaybackControllerMusicPreview,
};
use crate::modules::media_picker::editor::editor_video_trim_view::{
    EditorVideoTimelineRepresentable, EditorVideoTrimView, EditorVideoVolumeButton,
};
use crate::modules::media_picker::editor::media_editor_pager::{MediaEditorPageOffset, MediaEditorPager};
use crate::modules::media_picker::editor::media_editor_picker_top_bar::MediaEditorPickerTopBar;
use crate::modules::media_picker::editor::media_editor_presentation_state::MediaEditorPresentationState;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowLog;
use crate::modules::media_picker::editor::story_canvas_model::{
    EditorCanvasPhotoEntity, EditorStoryCanvas, StoryCanvasBackground, StoryCanvasGeometry, StoryCanvasPhotoStore,
};
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::library::photos_image_request::{PhotoImageDelivery, PhotosImageRequest};
use crate::modules::media_picker::selection::media_selection_state::{
    MediaSelectionEdit, MediaSelectionEditPayload, MediaSelectionState,
};
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::music::shared::utils::music::music_track_waveform::MusicTrackWaveform;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::widgets::bottomsheets::track_picker::editor_track_picker_sheet::EditorTrackPickerSheetView;
use crate::modules::music::widgets::track_trim_overlay::{TrackTrimOverlay, TrackTrimPlateChoice};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

#[derive(Clone)]
pub struct MediaEditorOutcome {
    pub edit: MediaSelectionEdit,
    pub immediate_send: bool,
    pub selects_asset: bool,
}

pub type MediaEditorScreenCancel = Rc<dyn Fn(&mut Window, &mut App)>;
pub type MediaEditorScreenDone = Rc<dyn Fn(MediaEditorOutcome, &mut Window, &mut App)>;

static MAX_PERSON_TAGS_PER_SLIDE: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("post_service_max_tagged_users"));

const MUSIC_PREVIEW_SESSION_ID: &str = "editor_music_preview";

static MAX_STICKERS_PER_SLIDE: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("media_editor_stickers_per_slide_max"));

const STICKER_PICKER_HEIGHT: f32 = 400.0;
const STICKER_PICKER_WIDTH: f32 = 360.0;
const STICKER_PICKER_CORNER_RADIUS: f32 = 16.0;
const STICKER_PICKER_GAP: f32 = 8.0;
const STICKER_PICKER_TOP_MARGIN: f32 = 24.0;
const MEDIA_EDITOR_PANEL_MAX_WIDTH: f32 = 560.0;

/// the screens stored properties, handed over on every render
#[derive(Clone)]
struct MediaEditorScreenProps {
    selection: Entity<MediaSelectionState>,
    caption: Binding<String>,
    media_asset: MediaAsset,
    ph_asset: Option<LibraryAsset>,
    theme_store: Entity<ThemeStore>,
    configuration: MediaPickerConfiguration,
    initial_edit: Option<MediaSelectionEdit>,
    source_image_override: Option<CleverImage>,
    source_video_url_override: Option<Url>,
    enables_standalone_swipe_dismiss: bool,
    standalone_dismiss_cancels: bool,
    editor_presentation: Option<Entity<MediaEditorPresentationState>>,
    is_video: bool,
    embed_in_hero_host: bool,
    seed_hero_image: Option<CleverImage>,
    image_stage_hidden: bool,
    chrome_opacity: f32,
    image_dismiss_offset: Size<f32>,
    image_dismiss_scale: f32,
    on_host_dismiss_drag_changed: Option<EditorImageStageDragChanged>,
    on_host_dismiss_drag_ended: Option<EditorImageStageDragEnded>,
    pager: Option<Entity<MediaEditorPager>>,
    on_page_drag_changed: Option<EditorImageStageDragChanged>,
    on_page_drag_ended: Option<EditorImageStageDragEnded>,
    shows_picker_top_bar: bool,
    defers_source_load: bool,
    on_cancel: MediaEditorScreenCancel,
    on_done: MediaEditorScreenDone,
}

struct EditorTrackPickerSheetHolder {
    view: Entity<EditorTrackPickerSheetView>,
}

/// the editors stage as its own view
/// so a sheet over it renders the screen without rebuilding and laying out the stage
struct MediaEditorStageView {
    state: WeakEntity<MediaEditorScreenState>,
    _observe_state: gpui::Subscription,
}

impl MediaEditorStageView {
    fn new(state: &Entity<MediaEditorScreenState>, cx: &mut Context<Self>) -> Self {
        let observe_state = cx.observe(state, |_, _, cx| cx.notify());
        Self { state: state.downgrade(), _observe_state: observe_state }
    }
}

impl Render for MediaEditorStageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(state) = self.state.upgrade() else {
            return div().into_any_element();
        };
        let stage =
            state.update(cx, |state, cx| state.stage_layer(window, cx).liquid_glass_contrast(LiquidGlassContrast::Media));
        div().relative().size_full().child(stage).into_any_element()
    }
}

#[derive(Clone, IntoElement)]
pub struct MediaEditorScreen {
    id: ElementId,
    props: MediaEditorScreenProps,
}

impl MediaEditorScreen {
    pub(crate) fn stage_safe_insets_for(reported: Edges<f32>, shows_picker_top_bar: bool) -> Edges<f32> {
        MediaEditorScreenState::stage_safe_insets_for(reported, shows_picker_top_bar)
    }

    /// the required args of the view in order, the rest start at defaults and have setters below
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        selection: Entity<MediaSelectionState>,
        caption: Binding<String>,
        media_asset: MediaAsset,
        ph_asset: Option<LibraryAsset>,
        theme_store: Entity<ThemeStore>,
        configuration: MediaPickerConfiguration,
        initial_edit: Option<MediaSelectionEdit>,
        is_video: bool,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_done: impl Fn(MediaEditorOutcome, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            props: MediaEditorScreenProps {
                selection,
                caption,
                media_asset,
                ph_asset,
                theme_store,
                configuration,
                initial_edit,
                source_image_override: None,
                source_video_url_override: None,
                enables_standalone_swipe_dismiss: true,
                standalone_dismiss_cancels: false,
                editor_presentation: None,
                is_video,
                embed_in_hero_host: false,
                seed_hero_image: None,
                image_stage_hidden: false,
                chrome_opacity: 1.0,
                image_dismiss_offset: size(0.0, 0.0),
                image_dismiss_scale: 1.0,
                on_host_dismiss_drag_changed: None,
                on_host_dismiss_drag_ended: None,
                pager: None,
                on_page_drag_changed: None,
                on_page_drag_ended: None,
                shows_picker_top_bar: false,
                defers_source_load: true,
                on_cancel: Rc::new(on_cancel),
                on_done: Rc::new(on_done),
            },
        }
    }

    pub fn source_image_override(mut self, source_image_override: Option<CleverImage>) -> Self {
        self.props.source_image_override = source_image_override;
        self
    }

    pub fn source_video_url_override(mut self, source_video_url_override: Option<Url>) -> Self {
        self.props.source_video_url_override = source_video_url_override;
        self
    }

    pub fn enables_standalone_swipe_dismiss(mut self, enables_standalone_swipe_dismiss: bool) -> Self {
        self.props.enables_standalone_swipe_dismiss = enables_standalone_swipe_dismiss;
        self
    }

    pub fn standalone_dismiss_cancels(mut self, standalone_dismiss_cancels: bool) -> Self {
        self.props.standalone_dismiss_cancels = standalone_dismiss_cancels;
        self
    }

    pub fn editor_presentation(mut self, editor_presentation: Option<Entity<MediaEditorPresentationState>>) -> Self {
        self.props.editor_presentation = editor_presentation;
        self
    }

    pub fn embed_in_hero_host(mut self, embed_in_hero_host: bool) -> Self {
        self.props.embed_in_hero_host = embed_in_hero_host;
        self
    }

    pub fn seed_hero_image(mut self, seed_hero_image: Option<CleverImage>) -> Self {
        self.props.seed_hero_image = seed_hero_image;
        self
    }

    pub fn image_stage_hidden(mut self, image_stage_hidden: bool) -> Self {
        self.props.image_stage_hidden = image_stage_hidden;
        self
    }

    pub fn chrome_opacity(mut self, chrome_opacity: f32) -> Self {
        self.props.chrome_opacity = chrome_opacity;
        self
    }

    pub fn image_dismiss_offset(mut self, image_dismiss_offset: Size<f32>) -> Self {
        self.props.image_dismiss_offset = image_dismiss_offset;
        self
    }

    pub fn image_dismiss_scale(mut self, image_dismiss_scale: f32) -> Self {
        self.props.image_dismiss_scale = image_dismiss_scale;
        self
    }

    pub fn on_host_dismiss_drag_changed(mut self, callback: Option<EditorImageStageDragChanged>) -> Self {
        self.props.on_host_dismiss_drag_changed = callback;
        self
    }

    pub fn on_host_dismiss_drag_ended(mut self, callback: Option<EditorImageStageDragEnded>) -> Self {
        self.props.on_host_dismiss_drag_ended = callback;
        self
    }

    pub fn pager(mut self, pager: Option<Entity<MediaEditorPager>>) -> Self {
        self.props.pager = pager;
        self
    }

    pub fn on_page_drag_changed(mut self, callback: Option<EditorImageStageDragChanged>) -> Self {
        self.props.on_page_drag_changed = callback;
        self
    }

    pub fn on_page_drag_ended(mut self, callback: Option<EditorImageStageDragEnded>) -> Self {
        self.props.on_page_drag_ended = callback;
        self
    }

    pub fn shows_picker_top_bar(mut self, shows_picker_top_bar: bool) -> Self {
        self.props.shows_picker_top_bar = shows_picker_top_bar;
        self
    }

    pub fn defers_source_load(mut self, defers_source_load: bool) -> Self {
        self.props.defers_source_load = defers_source_load;
        self
    }
}

impl RenderOnce for MediaEditorScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let props = self.props;
        let seed = props.clone();
        let state = window.use_keyed_state(self.id, cx, move |window, cx: &mut Context<MediaEditorScreenState>| {
            MediaEditorScreenState::new(seed, window, cx)
        });
        state.update(cx, |state, _| state.props = props);
        state
    }
}

/// conditional views of the screen that fade in and out
/// each is a bare flag, what they show is read from the screen state when they draw
struct EditorChromePresences {
    picker_top_bar: Presence<()>,
    tag_hint: Presence<()>,
    crop_panel: Presence<()>,
    draw_history: Presence<()>,
    draw_slider: Presence<()>,
    video_playback_chrome: Presence<()>,
    initial_panel: Presence<()>,
    draw_panel: Presence<()>,
    adjust_panel: Presence<()>,
    tag_people_panel: Presence<()>,
}

impl EditorChromePresences {
    fn new(shows_picker_top_bar: bool) -> Self {
        let move_bottom = || Transition::move_edge(Edge::Bottom).combined(Transition::opacity());
        Self {
            picker_top_bar: Presence::new(shows_picker_top_bar.then_some(())).with_transition(Transition::opacity()),
            tag_hint: Presence::new(None).with_transition(Transition::opacity()),
            crop_panel: Presence::new(None).with_transition(Transition::opacity()),
            draw_history: Presence::new(None).with_transition(Transition::opacity()),
            draw_slider: Presence::new(None)
                .with_transition(Transition::move_edge(Edge::Trailing).combined(Transition::opacity())),
            video_playback_chrome: Presence::new(Some(())).with_transition(move_bottom()),
            initial_panel: Presence::new(Some(())).with_transition(move_bottom()),
            draw_panel: Presence::new(None).with_transition(move_bottom()),
            adjust_panel: Presence::new(None).with_transition(move_bottom()),
            tag_people_panel: Presence::new(None).with_transition(move_bottom()),
        }
    }
}

/// everything chrome_layer differs on between the image and video body
struct EditorChromeInput {
    safe_insets: Edges<f32>,
    brush_rect: Bounds<f32>,
    text_rect: Bounds<f32>,
    asset: Option<AVURLAsset>,
}

struct MediaEditorScreenState {
    props: MediaEditorScreenProps,

    is_loading: bool,

    image_state: Option<EditorImageState>,
    source_image: Option<CleverImage>,
    original_source_image: Option<CleverImage>,
    rendered_preview: Option<CleverImage>,
    crop_stage_image: Option<CleverImage>,
    crop_stage_generation: u64,
    image_rect: EditorImageRect,
    mode: EditorMode,
    text_opened_from_draw: bool,
    video_player_ready: bool,
    video_canvas_cover: Animated<f32>,
    video_ready_fallback: Option<Task<()>>,
    swipe_down_offset: Animated<f32>,

    show_track_picker: bool,
    stage_view: Option<Entity<MediaEditorStageView>>,
    show_track_trim: bool,
    trim_track: Option<RiseStoredMusicTrack>,
    show_sound_sheet: bool,
    music_preview_url: Option<Url>,
    music_segment_pinned: bool,
    show_sticker_picker: bool,
    sticker_picker_presence: Presence<()>,
    canvas_stage_image: Option<CleverImage>,
    show_canvas_gallery: bool,
    is_importing_canvas_photos: bool,
    show_link_input: bool,
    link_draft: String,
    link_draft_rejected: bool,
    link_draft_card: EditorLinkDraft,
    link_draft_card_presence: Presence<EditorLinkEntity>,
    link_draft_fetch: Option<Task<()>>,
    link_input_session: i64,
    customizing_link_id: Option<Uuid>,

    brush_tool: EditorDrawStrokeTool,
    last_painting_tool: EditorDrawStrokeTool,
    brush_color: Hsla,
    draw_undo: Vec<Vec<EditorDrawStroke>>,
    draw_redo: Vec<Vec<EditorDrawStroke>>,
    draw_history_applying: Option<Vec<EditorDrawStroke>>,
    draw_undo_fade: Animated<f32>,
    draw_redo_fade: Animated<f32>,
    brush_width_fraction: f32,
    thickness_is_dragging: bool,

    text_draft: Option<EditorTextEntity>,

    main_chrome_height: f32,
    video_playback_chrome_top: f32,
    crop_chrome_height: f32,
    crop_coordination_id: Uuid,

    video_state: Option<EditorVideoState>,
    loaded_av_asset: Option<AVURLAsset>,
    video_is_playing: bool,
    video_resumes_after_trim: bool,
    video_trim_auto_clamped: bool,
    video_playback: Rc<EditorVideoPlaybackController>,
    video_poster_original: Option<CleverImage>,
    video_poster_rendered: Option<CleverImage>,
    video_image_rect: EditorImageRect,
    is_loading_video_poster: bool,

    local_user_deselected: bool,
    keyboard_visible: bool,
    video_page_drag_activation_x: Option<f32>,
    video_dismiss_drag_active: bool,
    video_plate_pressed: bool,

    stage_bounds: Bounds<f32>,
    root_focus: FocusHandle,
    focus_before_editor: Option<FocusHandle>,
    window_handle: AnyWindowHandle,
    keyboard_visible_at_press: bool,

    chrome: EditorChromePresences,
    crop_panel: Option<Entity<EditorCropPanel>>,
    text_draft_presence: Presence<EditorTextEntity>,
    track_trim_presence: Presence<RiseStoredMusicTrack>,
    trim_chrome_fade: Animated<f32>,

    preview_generation: u64,
    preview_task: Option<Task<()>>,
    canvas_generation: u64,
    canvas_task: Option<Task<()>>,
    crop_commit_task: Option<Task<()>>,
    poster_generation: u64,
    poster_task: Option<Task<()>>,
    crop_stage_task: Option<Task<()>>,
    image_request_task: Option<Task<()>>,
    video_request_task: Option<Task<()>>,

    _selection_observation: gpui::Subscription,
    _keyboard_observers: Vec<RiseSubscription>,
    _keyboard_inbox: Task<()>,
    _link_customize_observer: RiseSubscription,
    _link_customize_inbox: Task<()>,
}

fn mode_animation() -> MotionSpec {
    spring(0.34, 0.84)
}

fn image_pixel_size(image: &RenderImage) -> Size<f32> {
    let pixels = image.size(0);
    size(pixels.width.0 as f32, pixels.height.0 as f32)
}

fn pixels_bounds(bounds: Bounds<Pixels>) -> Bounds<f32> {
    Bounds {
        origin: point(f32::from(bounds.origin.x), f32::from(bounds.origin.y)),
        size: size(f32::from(bounds.size.width), f32::from(bounds.size.height)),
    }
}

fn copy_poster_image(source: &Path, at_seconds: f64, max_side: u32) -> Option<CleverImage> {
    let frame = rise_media::video_export::poster(source, at_seconds, max_side).ok()?;
    let buffer = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
    Some(Arc::new(RenderImage::new(frames)))
}

/// link preview text as the card shows it, trimmed and empty when the service sent none
fn trimmed_preview_text(value: Option<String>) -> String {
    value.map(|value| trimming_whitespaces_and_newlines(&value).to_string()).unwrap_or_default()
}

impl MediaEditorScreenState {
    fn new(props: MediaEditorScreenProps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let seeded_stage_bounds = props
            .editor_presentation
            .as_ref()
            .map(|presentation| presentation.read(cx).last_stage_bounds)
            .unwrap_or_default();
        if seeded_stage_bounds.size.width > 0.0 {
            MediaEditorWindowLog::event(
                "riseonly::X649 stage-seeded",
                || format!("w={} h={}", seeded_stage_bounds.size.width, seeded_stage_bounds.size.height),
                cx,
            );
        }
        let selection_observation = cx.observe(&props.selection, |_, _, cx| cx.notify());

        let (sender, mut inbox) = mpsc::unbounded::<bool>();
        let center = RiseAppEvents::default_center();
        let observe = |name, visible: bool| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(visible);
            })
        };
        let keyboard_observers = vec![
            observe(UIResponder::KEYBOARD_WILL_SHOW_NOTIFICATION, true),
            observe(UIResponder::KEYBOARD_WILL_HIDE_NOTIFICATION, false),
        ];
        let keyboard_inbox = cx.spawn(async move |this, cx| {
            while let Some(visible) = inbox.next().await {
                if this.update(cx, |state, _| state.keyboard_visible = visible).is_err() {
                    break;
                }
            }
        });

        let (link_sender, mut link_inbox) = mpsc::unbounded::<Uuid>();
        let link_customize_observer = center.observe(EDITOR_LINK_CUSTOMIZE, move |note| {
            if let Some(id) = note.payload::<Uuid>() {
                let _ = link_sender.unbounded_send(*id);
            }
        });
        let link_customize_inbox = cx.spawn(async move |this, cx| {
            while let Some(id) = link_inbox.next().await {
                let delivered = this.update(cx, |state, cx| {
                    if state.active_links().iter().any(|link| link.id == id) {
                        state.customizing_link_id = Some(id);
                        cx.notify();
                    }
                });
                if delivered.is_err() {
                    break;
                }
            }
        });

        let empty_rect = EditorImageRect { zoom: 1.0, ..EditorImageRect::default() };
        let chrome = EditorChromePresences::new(props.shows_picker_top_bar);
        Self {
            props,
            is_loading: true,
            image_state: None,
            source_image: None,
            original_source_image: None,
            rendered_preview: None,
            crop_stage_image: None,
            crop_stage_generation: 0,
            image_rect: empty_rect,
            mode: EditorMode::Initial,
            text_opened_from_draw: false,
            video_player_ready: false,
            video_canvas_cover: Animated::new(1.0),
            video_ready_fallback: None,
            swipe_down_offset: Animated::new(0.0),
            show_track_picker: false,
            stage_view: None,
            show_track_trim: false,
            trim_track: None,
            show_sound_sheet: false,
            music_preview_url: None,
            music_segment_pinned: false,
            show_sticker_picker: false,
            sticker_picker_presence: Presence::new(None).with_transition(Transition::opacity()),
            canvas_stage_image: None,
            show_canvas_gallery: false,
            is_importing_canvas_photos: false,
            show_link_input: false,
            link_draft: String::new(),
            link_draft_rejected: false,
            link_draft_card: EditorLinkDraft::default(),
            link_draft_card_presence: Presence::new(None).with_transition(Transition::opacity()),
            link_draft_fetch: None,
            link_input_session: 0,
            customizing_link_id: None,
            brush_tool: EditorDrawStrokeTool::Pen,
            last_painting_tool: EditorDrawStrokeTool::Pen,
            brush_color: white(),
            draw_undo: Vec::new(),
            draw_redo: Vec::new(),
            draw_history_applying: None,
            draw_undo_fade: Animated::new(0.35),
            draw_redo_fade: Animated::new(0.35),
            brush_width_fraction: 0.04,
            thickness_is_dragging: false,
            text_draft: None,
            main_chrome_height: 0.0,
            video_playback_chrome_top: 0.0,
            crop_chrome_height: 0.0,
            crop_coordination_id: Uuid::new_v4(),
            video_state: None,
            loaded_av_asset: None,
            video_is_playing: false,
            video_resumes_after_trim: false,
            video_trim_auto_clamped: false,
            video_playback: EditorVideoPlaybackController::new(),
            video_poster_original: None,
            video_poster_rendered: None,
            video_image_rect: empty_rect,
            is_loading_video_poster: false,
            local_user_deselected: false,
            keyboard_visible: false,
            video_page_drag_activation_x: None,
            video_dismiss_drag_active: false,
            video_plate_pressed: false,
            stage_bounds: seeded_stage_bounds,
            root_focus: cx.focus_handle(),
            focus_before_editor: None,
            window_handle: window.window_handle(),
            keyboard_visible_at_press: false,
            chrome,
            crop_panel: None,
            text_draft_presence: Presence::new(None).with_transition(Transition::opacity()),
            track_trim_presence: Presence::new(None)
                .with_transition(Transition::opacity().combined(Transition::scale(1.02))),
            trim_chrome_fade: Animated::new(1.0),
            preview_generation: 0,
            preview_task: None,
            canvas_generation: 0,
            canvas_task: None,
            crop_commit_task: None,
            poster_generation: 0,
            poster_task: None,
            crop_stage_task: None,
            image_request_task: None,
            video_request_task: None,
            _selection_observation: selection_observation,
            _keyboard_observers: keyboard_observers,
            _keyboard_inbox: keyboard_inbox,
            _link_customize_observer: link_customize_observer,
            _link_customize_inbox: link_customize_inbox,
        }
    }

    fn did_appear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(presentation) = self.props.editor_presentation.clone() {
            let mode = self.mode;
            let this = cx.weak_entity();
            presentation.update(cx, |presentation, cx| {
                presentation.dismiss_persist_handler = Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |state, cx| state.persist_current_edit_to_selection(cx));
                }));
                presentation.editor_mode = mode;
                cx.notify();
            });
        }
        let root_focus = self.root_focus.clone();
        self.focus_before_editor = window.focused(cx).filter(|focused| *focused != root_focus);
        self.take_keyboard_focus(window, cx);
        self.sync_navigation_pop_guard(cx);
        self.load_if_needed(window, cx);
    }

    fn did_disappear(&mut self, cx: &mut Context<Self>) {
        self.prepare_crop_stage(EditorMode::Initial, cx);
        self.video_is_playing = false;
        self.video_playback.stop_music_preview(cx);
        let Some(presentation) = self.props.editor_presentation.clone() else {
            NavigationInteractivePopGuard::set_media_editor_editing(false, cx);
            return;
        };
        {
            let presentation = presentation.read(cx);
            if presentation.is_presented
                && presentation.media_asset.as_ref().is_some_and(|current| current.id != self.props.media_asset.id)
            {
                return;
            }
        }
        presentation.update(cx, |presentation, cx| {
            presentation.dismiss_persist_handler = None;
            presentation.editor_mode = EditorMode::Initial;
            cx.notify();
        });
        NavigationInteractivePopGuard::set_media_editor_editing(false, cx);
    }

    fn hand_keyboard_focus_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous = self.focus_before_editor.take();
        let turned_page = self.props.editor_presentation.as_ref().is_some_and(|presentation| {
            let presentation = presentation.read(cx);
            presentation.is_presented
                && presentation.media_asset.as_ref().is_some_and(|current| current.id != self.props.media_asset.id)
        });
        if turned_page {
            return;
        }
        let holds_focus = window.focused(cx).is_some_and(|focused| {
            focused == self.root_focus
                || self.root_focus.contains(&focused, window)
                || !focused.is_in_rendered_frame(window)
        });
        if !holds_focus {
            return;
        }
        match previous.filter(|previous| previous.is_in_rendered_frame(window)) {
            Some(previous) => window.focus(&previous, cx),
            None => window.blur(),
        }
    }

    /// how keys get into the screen on desktop
    /// esc and secondary enter reach the root only from a focus inside it
    /// a video leaves the slot empty so its player takes it, it owns space
    fn take_keyboard_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ScreenFocus::register_sheet(&self.root_focus, cx);
        if self.props.is_video {
            window.blur();
        } else {
            window.focus(&self.root_focus, cx);
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_track_picker || self.show_sound_sheet || self.show_track_trim || self.text_draft.is_some() {
            return;
        }
        if PeoplePickerStore::shared(cx).read(cx).mounts()
            || EditorColorPickerBridge::shared(cx).read(cx).picker_in_flight().is_some()
        {
            return;
        }
        let keystroke = &event.keystroke;
        if keystroke.key == "escape" {
            if self.mode == EditorMode::Initial {
                cx.stop_propagation();
                self.dismiss_to_grid(window, cx);
            }
            return;
        }
        if keystroke.key == "enter" && keystroke.modifiers.secondary() {
            cx.stop_propagation();
            match self.mode {
                EditorMode::Initial => self.commit(true, window, cx),
                EditorMode::Crop if self.props.is_video => self.commit_video_spatial_crop(cx),
                EditorMode::Crop => self.commit_crop_to_canvas(cx),
                EditorMode::Draw | EditorMode::Adjust | EditorMode::TagPeople => {
                    self.set_mode(EditorMode::Initial, cx);
                }
                EditorMode::Text { .. } => {}
            }
        }
    }

    fn user_deselected(&self, cx: &App) -> bool {
        match &self.props.editor_presentation {
            Some(presentation) => presentation.read(cx).user_deselected_asset_ids.contains(&self.asset_local_id()),
            None => self.local_user_deselected,
        }
    }

    fn set_user_deselected(&mut self, deselected: bool, cx: &mut Context<Self>) {
        self.local_user_deselected = deselected;
        if let Some(presentation) = self.props.editor_presentation.clone() {
            let asset_id = self.asset_local_id();
            presentation.update(cx, |presentation, cx| presentation.set_user_deselected(deselected, &asset_id, cx));
        }
    }

    fn set_mode(&mut self, mode: EditorMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    fn set_image_state(&mut self, mut new: Option<EditorImageState>, cx: &mut Context<Self>) {
        if let (Some(new), Some(old)) = (new.as_mut(), self.image_state.as_ref()) {
            new.raise_added_entities(old);
        }
        if self.image_state == new {
            return;
        }
        let old = std::mem::replace(&mut self.image_state, new);
        if old.is_some() {
            self.auto_select_if_edited(cx);
        }
        let new = self.image_state.as_ref();
        let plate_changed = old.as_ref().map(|state| &state.music_plate) != new.map(|state| &state.music_plate);
        let strokes_changed = !self.props.is_video
            && old.as_ref().map_or(&[][..], |state| state.strokes.as_slice())
                != new.map_or(&[][..], |state| state.strokes.as_slice());
        let old_strokes =
            strokes_changed.then(|| old.as_ref().map(|state| state.strokes.clone()).unwrap_or_default());
        if plate_changed {
            self.sync_plate_to_selection(cx);
        }
        if let Some(old_strokes) = old_strokes {
            self.on_active_strokes_changed(old_strokes, cx);
        }
        let old_adjustments = old.map(|state| state.adjustments).unwrap_or_default();
        let new_adjustments = self.image_state.as_ref().map(|state| state.adjustments).unwrap_or_default();
        if old_adjustments != new_adjustments {
            self.recompute_preview(cx);
        }
        cx.notify();
    }

    fn set_video_state(&mut self, mut new: Option<EditorVideoState>, cx: &mut Context<Self>) {
        if let (Some(new), Some(old)) = (
            new.as_mut().and_then(|state| state.spatial_edit.as_mut()),
            self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref()),
        ) {
            new.raise_added_entities(old);
        }
        if self.video_state == new {
            return;
        }
        let old = std::mem::replace(&mut self.video_state, new);
        if old.is_some() {
            self.auto_select_if_edited(cx);
        }
        let old_spatial = old.as_ref().and_then(|state| state.spatial_edit.as_ref());
        let new_spatial = self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref());
        let plate_changed = old_spatial.and_then(|state| state.music_plate.as_ref())
            != new_spatial.and_then(|state| state.music_plate.as_ref());
        let old_adjustments = old_spatial.map(|state| state.adjustments).unwrap_or_default();
        let new_adjustments = new_spatial.map(|state| state.adjustments).unwrap_or_default();
        let strokes_changed = old_spatial.map_or(&[][..], |state| state.strokes.as_slice())
            != new_spatial.map_or(&[][..], |state| state.strokes.as_slice());
        let old_strokes =
            strokes_changed.then(|| old_spatial.map(|state| state.strokes.clone()).unwrap_or_default());
        if plate_changed {
            self.sync_plate_to_selection(cx);
        }
        if let Some(old_strokes) = old_strokes {
            self.on_active_strokes_changed(old_strokes, cx);
        }
        if old_adjustments != new_adjustments {
            self.recompute_video_poster_preview(cx);
        }
        cx.notify();
    }

    fn set_attached_track(&self, attached: Option<MediaPickerTrackSelection>, cx: &mut Context<Self>) {
        self.props.selection.update(cx, |selection, cx| {
            selection.attached_track = attached;
            cx.notify();
        });
    }

    fn attached_track(&self, cx: &App) -> Option<MediaPickerTrackSelection> {
        self.props.selection.read(cx).attached_track.clone()
    }

    fn on_mode_changed(&mut self, new_mode: EditorMode, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_crop_stage(new_mode, cx);
        if new_mode != EditorMode::Initial {
            self.video_player_ready = false;
            self.video_canvas_cover.set_in(1.0, None::<MotionSpec>, RiseClock::monotonic(), cx);
            self.video_ready_fallback = None;
        } else if self.props.is_video && self.loaded_av_asset.is_some() {
            self.start_video_ready_fallback(cx);
        }
        GuideFacts::set(RiseGuideSignal::MediaEditorAdjusting, new_mode == EditorMode::Adjust, cx);
        GuideFacts::set(RiseGuideSignal::MediaEditorIdle, new_mode == EditorMode::Initial, cx);
        if let Some(presentation) = self.props.editor_presentation.clone() {
            presentation.update(cx, |presentation, cx| {
                presentation.editor_mode = new_mode;
                if let EditorMode::Text { .. } = new_mode {
                    presentation.focused_text_entity_id = None;
                }
                cx.notify();
            });
        }
        self.sync_navigation_pop_guard(cx);
        if new_mode != EditorMode::Initial && *self.swipe_down_offset.target() != 0.0 {
            self.swipe_down_offset.set_in(0.0, spring(0.30, 0.85), RiseClock::monotonic(), cx);
        }
        if self.props.is_video && new_mode != EditorMode::Initial {
            self.video_is_playing = false;
            self.ensure_video_poster_for_edit_mode(cx);
        }
        if new_mode == EditorMode::Draw {
            self.draw_undo.clear();
            self.draw_redo.clear();
            self.draw_history_applying = None;
            self.sync_draw_history_fades(cx);
        }
        if new_mode == EditorMode::Initial {
            self.take_keyboard_focus(window, cx);
        }
        self.sync_chrome_presences(cx);
        cx.notify();
    }

    fn make_crop_panel(this: Entity<Self>, window: &mut Window, cx: &mut App) -> Entity<EditorCropPanel> {
        let is_video = this.read(cx).props.is_video;
        let coordination_id = this.read(cx).crop_coordination_id;
        let state = if is_video { Self::video_spatial_binding(this.clone()) } else { Self::image_state_binding(this.clone()) };
        let close = this.clone();
        let confirm = this.clone();
        let height = this;
        cx.new(|cx| {
            EditorCropPanel::new(
                coordination_id,
                state,
                Rc::new(|_: &mut Window, _: &mut App| {}),
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    close.update(cx, |state, cx| state.set_mode(EditorMode::Initial, cx));
                }),
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    confirm.update(cx, |state, cx| {
                        if is_video {
                            state.commit_video_spatial_crop(cx);
                        } else {
                            state.commit_crop_to_canvas(cx);
                        }
                    });
                }),
                Rc::new(move |value: f32, _: &mut Window, cx: &mut App| {
                    if value > 1.0 {
                        height.update(cx, |state, cx| {
                            if (state.crop_chrome_height - value).abs() > 0.5 {
                                state.crop_chrome_height = value;
                                cx.notify();
                            }
                        });
                    }
                }),
                window,
                cx,
            )
        })
    }

    fn sync_chrome_presences(&mut self, cx: &mut Context<Self>) {
        let mode = self.mode;
        let now = RiseClock::monotonic();
        let top_bar = self.props.shows_picker_top_bar && mode == EditorMode::Initial && self.text_draft.is_none();
        let chrome = &mut self.chrome;
        let wanted: [(&mut Presence<()>, bool); 10] = [
            (&mut chrome.picker_top_bar, top_bar),
            (&mut chrome.tag_hint, mode == EditorMode::TagPeople),
            (&mut chrome.crop_panel, mode == EditorMode::Crop),
            (&mut chrome.draw_history, mode == EditorMode::Draw),
            (&mut chrome.draw_slider, mode == EditorMode::Draw),
            (&mut chrome.video_playback_chrome, mode == EditorMode::Initial),
            (&mut chrome.initial_panel, mode == EditorMode::Initial),
            (&mut chrome.draw_panel, mode == EditorMode::Draw),
            (&mut chrome.adjust_panel, mode == EditorMode::Adjust),
            (&mut chrome.tag_people_panel, mode == EditorMode::TagPeople),
        ];
        for (presence, visible) in wanted {
            if presence.is_present() != visible {
                presence.set_in(visible.then_some(()), mode_animation(), now, cx);
            }
        }
    }

    fn on_text_draft_changed(&mut self, cx: &mut Context<Self>) {
        let draft = self.text_draft.clone();
        self.text_draft_presence.set_in(draft, ease_in_out(0.18), RiseClock::monotonic(), cx);
        self.sync_chrome_presences(cx);
        cx.notify();
    }

    fn on_show_track_trim_changed(&mut self, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        let track = if self.show_track_trim { self.trim_track.clone() } else { None };
        self.track_trim_presence.set_in(track, ease_in_out(0.22), now, cx);
        let fade = if self.show_track_trim { 0.0 } else { 1.0 };
        self.trim_chrome_fade.set_in(fade, ease_in_out(0.22), now, cx);
        cx.notify();
    }

    fn stage_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let offset = self.swipe_down_offset.frame_value(RiseClock::monotonic(), window);
        let drag_progress = (offset / 600.0).clamp(0.0, 1.0);
        let embed = self.props.embed_in_hero_host;
        let swipes = self.props.enables_standalone_swipe_dismiss;
        let (width, height) = (self.stage_bounds.size.width, self.stage_bounds.size.height);
        let mask = self.dismiss_gesture_mask();
        let gesture = self.dismiss_drag_gesture(cx.entity());
        let content = self.content(window, cx);
        let group = if embed || !swipes {
            content
        } else {
            div()
                .absolute()
                .left_0()
                .top(px(offset))
                .size_full()
                .child(
                    div()
                        .w(px(width))
                        .h(px(height))
                        .relative()
                        .child(content)
                        .simultaneous_gesture("media-editor-dismiss-drag", gesture, mask),
                )
                .into_any_element()
        };
        div()
            .absolute()
            .inset_0()
            .when(!embed, |layer| {
                layer.bg(MEDIA_EDITOR_BACKDROP.opacity(1.0 - drag_progress * 0.85))
            })
            .child(group)
            .into_any_element()
    }

    fn music_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let this = cx.entity();
        let mut overlays = Vec::new();
        if self.props.configuration.allows_music_selection {
            overlays.push(self.track_picker_overlay(this.clone(), cx));
        }
        if self.props.is_video {
            overlays.push(self.sound_sheet_overlay(this.clone(), cx));
        }
        if self.props.configuration.allows_music_selection
            && let Some(overlay) = self.track_trim_overlay(this, window, cx)
        {
            overlays.push(overlay);
        }
        overlays
    }

    fn person_tagging_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.props.configuration.allows_person_tagging {
            return None;
        }
        let host = window.use_keyed_state(
            "media-editor-people-picker-host",
            cx,
            |window, cx: &mut Context<PeoplePickerHost>| {
                PeoplePickerHost::new(PeoplePickerHostContext::MediaEditor, window, cx)
            },
        );
        Some(div().absolute().inset_0().child(host.cached(StyleRefinement::default().size_full())).into_any_element())
    }

    fn layered_root(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        let stage = self.stage_element(&this, cx);
        let people = self.person_tagging_overlay(window, cx);
        let canvas_and_links = self.canvas_and_link_overlays(this.clone(), window, cx);
        let music = self.music_overlays(window, cx);
        let stickers = self.sticker_picker_overlay(this, window, cx);
        div()
            .relative()
            .size_full()
            .child(stage)
            .children(people)
            .children(canvas_and_links)
            .children(music)
            .children(stickers)
            .into_any_element()
    }

    fn stage_element(&mut self, this: &Entity<Self>, cx: &mut Context<Self>) -> AnyElement {
        let stage = self.stage_view.get_or_insert_with(|| cx.new(|cx| MediaEditorStageView::new(this, cx))).clone();
        if self.show_track_picker {
            AnyView::from(stage)
                .cached(StyleRefinement::default().absolute().top_0().left_0().size_full())
                .into_any_element()
        } else {
            div().absolute().inset_0().child(AnyView::from(stage)).into_any_element()
        }
    }

    fn allows_editor_dismiss_swipe(&self) -> bool {
        self.mode == EditorMode::Initial
    }

    fn allows_page_drag(&self) -> bool {
        self.props.pager.is_some() && self.allows_editor_dismiss_swipe() && self.text_draft.is_none()
    }

    /// in the hero host a drag down over the video closes the viewer, same as over a photo
    fn allows_video_dismiss_drag(&self) -> bool {
        self.props.embed_in_hero_host
            && self.props.on_host_dismiss_drag_changed.is_some()
            && self.allows_editor_dismiss_swipe()
            && self.text_draft.is_none()
    }

    fn video_page_drag_gesture(this: Entity<Self>) -> DragGesture {
        let ended = this.clone();
        DragGesture::new(16.0)
            .on_changed(move |value, _, cx| {
                this.update(cx, |state, cx| state.video_page_drag_changed(value, cx));
            })
            .on_ended(move |value, _, cx| {
                ended.update(cx, |state, cx| state.video_page_drag_ended(value, cx));
            })
    }

    fn video_page_drag_changed(&mut self, value: &DragGestureValue, cx: &mut Context<Self>) {
        let dx = f32::from(value.translation.width);
        let dy = f32::from(value.translation.height);
        if self.video_dismiss_drag_active {
            if let Some(changed) = self.props.on_host_dismiss_drag_changed.clone() {
                changed(dy.max(0.0), cx);
            }
            return;
        }
        if self.video_page_drag_activation_x.is_none() {
            if self.video_plate_pressed {
                return;
            }
            if self.allows_video_dismiss_drag() && dy > 12.0 && dy > dx.abs() * 0.72 {
                self.video_dismiss_drag_active = true;
                if let Some(changed) = self.props.on_host_dismiss_drag_changed.clone() {
                    changed(dy.max(0.0), cx);
                }
                return;
            }
            if !self.allows_page_drag() || dx.abs() <= dy.abs() * 1.2 {
                return;
            }
            self.video_page_drag_activation_x = Some(dx);
        }
        if let Some(changed) = self.props.on_page_drag_changed.clone() {
            changed(dx - self.video_page_drag_activation_x.unwrap_or(0.0), cx);
        }
    }

    fn video_page_drag_ended(&mut self, value: &DragGestureValue, cx: &mut Context<Self>) {
        if self.video_dismiss_drag_active {
            self.video_dismiss_drag_active = false;
            if let Some(ended) = self.props.on_host_dismiss_drag_ended.clone() {
                ended(f32::from(value.translation.height).max(0.0), f32::from(value.velocity.height), cx);
            }
            return;
        }
        let Some(activation) = self.video_page_drag_activation_x.take() else {
            return;
        };
        if let Some(ended) = self.props.on_page_drag_ended.clone() {
            ended(f32::from(value.translation.width) - activation, f32::from(value.velocity.width), cx);
        }
    }

    fn picker_top_bar(&self, this: Entity<Self>, width: f32, cx: &App) -> MediaEditorPickerTopBar {
        let selection = self.props.selection.read(cx);
        MediaEditorPickerTopBar::new(
            self.props.configuration.destination_title.clone(),
            width / 2.0,
            self.props.configuration.shows_selection_controls && self.props.ph_asset.is_some(),
            selection.index(&self.asset_local_id()).map(|index| index as i64 + 1),
            self.props.ph_asset.as_ref().is_some_and(|asset| selection.can_select(asset)),
            move |_, cx| this.update(cx, |state, cx| state.toggle_selection(cx)),
        )
    }

    /// with the caption keyboard up a tap only lowers it, otherwise it works as the circle
    fn handle_canvas_tap(&mut self, cx: &mut Context<Self>) {
        if self.keyboard_visible || self.keyboard_visible_at_press {
            self.keyboard_visible_at_press = false;
            let handle = self.window_handle;
            let focus = self.root_focus.clone();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| window.focus(&focus, cx));
            });
            return;
        }
        if !self.props.configuration.shows_selection_controls {
            return;
        }
        self.toggle_selection(cx);
    }

    fn toggle_selection(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.props.ph_asset.clone() else {
            return;
        };
        let selection = self.props.selection.clone();
        if selection.read(cx).contains(&asset.id) {
            selection.update(cx, |selection, cx| selection.remove(&asset.id, cx));
            self.set_user_deselected(true, cx);
        } else if selection.update(cx, |selection, cx| selection.toggle(&asset, cx)) {
            self.set_user_deselected(false, cx);
        } else {
            self.props.configuration.offer_more_if_count_refused(&asset, &selection, cx);
        }
    }

    fn dismiss_gesture_mask(&self) -> GestureMask {
        if self.allows_editor_dismiss_swipe() { GestureMask::All } else { GestureMask::None }
    }

    fn dismiss_drag_gesture(&self, this: Entity<Self>) -> DragGesture {
        let ended = this.clone();
        DragGesture::new(14.0)
            .on_changed(move |value, _, cx| {
                this.update(cx, |state, cx| {
                    if !state.allows_editor_dismiss_swipe() {
                        return;
                    }
                    let dy = f32::from(value.translation.height);
                    let dx = f32::from(value.translation.width);
                    if dy > 0.0 && dy.abs() > dx.abs() * 1.4 {
                        state.swipe_down_offset.jump(dy);
                        cx.notify();
                    }
                });
            })
            .on_ended(move |value, window, cx| {
                ended.update(cx, |state, cx| state.dismiss_drag_ended(value, window, cx));
            })
    }

    fn dismiss_drag_ended(&mut self, value: &DragGestureValue, window: &mut Window, cx: &mut Context<Self>) {
        if !self.allows_editor_dismiss_swipe() {
            return;
        }
        let dy = f32::from(value.translation.height);
        let velocity = f32::from(value.predicted_end_translation.height) - dy;
        let now = RiseClock::monotonic();
        if dy > 140.0 || velocity > 180.0 {
            let height = f32::from(window.viewport_size().height);
            self.swipe_down_offset.set_in(height, ease_out(0.22), now, cx);
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs_f64(0.22)).await;
                let _ = this.update_in(cx, |state, window, cx| state.dismiss_to_grid(window, cx));
            })
            .detach();
        } else {
            self.swipe_down_offset.set_in(0.0, spring(0.32, 0.84), now, cx);
        }
        cx.notify();
    }

    fn content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.stage_bounds.size.width <= 0.0 || self.stage_bounds.size.height <= 0.0 {
            return div().into_any_element();
        }
        if self.props.is_video { self.video_body(window, cx) } else { self.image_body(window, cx) }
    }

    fn chrome_layer(&self, content: Vec<AnyElement>, window: &Window) -> AnyElement {
        let opacity = self.props.chrome_opacity * self.trim_chrome_fade.frame_value(RiseClock::monotonic(), window);
        div()
            .absolute()
            .inset_0()
            .opacity(opacity)
            .when(opacity <= 0.05, |layer| layer.invisible())
            .children(content)
            .into_any_element()
    }

    /// chrome both bodies share, in z order
    fn chrome_children(&mut self, input: EditorChromeInput, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let this = cx.entity();
        let now = RiseClock::monotonic();
        let safe_top = input.safe_insets.top;
        let width = self.stage_bounds.size.width;
        let mut children: Vec<AnyElement> = Vec::new();

        if let Some(top) = self.props.configuration.custom_top_overlay.clone() {
            let overlay = top(window, cx);
            children.push(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .px(px(14.0))
                    .pt(px(safe_top + 6.0))
                    .child(overlay)
                    .into_any_element(),
            );
        }

        let bar = self.picker_top_bar(this.clone(), width, cx);
        if let Some(element) = self.chrome.picker_top_bar.render_transitioned(now, window, |_| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .px(px(14.0))
                .pt(px(Self::picker_top_bar_top(safe_top)))
                .child(bar)
        }) {
            children.push(element.into_any_element());
        }

        if let Some(element) =
            self.chrome.tag_hint.render_transitioned(now, window, |_| self.tag_hint_banner(safe_top))
        {
            children.push(element.into_any_element());
        }

        if let Some(panel) = self.crop_panel.clone()
            && let Some(element) = self.chrome.crop_panel.render_transitioned(now, window, |_| {
                div().absolute().inset_0().child(panel)
            })
        {
            children.push(element.into_any_element());
        }

        let history_bar = self.draw_history_bar(this.clone(), safe_top, window);
        if let Some(element) = self.chrome.draw_history.render_transitioned(now, window, |_| history_bar) {
            children.push(element.into_any_element());
        }

        if let Some(element) = self.chrome.draw_slider.render_transitioned(now, window, |_| {
            self.draw_mode_right_slider(this.clone())
        }) {
            children.push(element.into_any_element());
        }
        if self.mode == EditorMode::Draw && self.thickness_is_dragging {
            let rect = input.brush_rect;
            if !self.props.is_video || rect.size.width > 1.0 {
                let diameter = (self.brush_width_fraction * rect.size.width.min(rect.size.height)).max(4.0);
                let mid_x = rect.origin.x + rect.size.width / 2.0;
                let mid_y = rect.origin.y + rect.size.height / 2.0;
                children.push(
                    div()
                        .absolute()
                        .left(px(mid_x - diameter / 2.0))
                        .top(px(mid_y - diameter / 2.0))
                        .child(EditorBrushPreviewCircle::new(diameter, self.brush_color))
                        .into_any_element(),
                );
            }
        }

        if let Some(asset) = input.asset {
            let bottom = self.resolved_main_chrome_height();
            if let Some(element) = self.chrome.video_playback_chrome.render_transitioned(now, window, |_| {
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(bottom))
                    .child(self.video_playback_chrome(this.clone(), asset, cx))
            }) {
                children.push(element.into_any_element());
            }
        }

        children.extend(self.bottom_panel(this.clone(), window, cx));

        if let Some(draft) = self.text_draft_presence.value(now).cloned() {
            let rect = input.text_rect;
            let on_cancel = this.clone();
            let on_commit = this.clone();
            let binding = this;
            let overlay = EditorTextEditorOverlay::new(
                Binding::entity(
                    binding,
                    {
                        let draft = draft.clone();
                        move |state: &Self| state.text_draft.clone().unwrap_or_else(|| draft.clone())
                    },
                    |state, value, _| state.text_draft = Some(value),
                ),
                move |_, cx| on_cancel.update(cx, |state, cx| state.cancel_text_draft(cx)),
                move |committed, _, cx| on_commit.update(cx, |state, cx| state.commit_text_draft(committed, cx)),
                rect,
                safe_top,
            );
            if let Some(element) = self.text_draft_presence.render_transitioned(now, window, |_| {
                div().absolute().inset_0().child(overlay)
            }) {
                children.push(element.into_any_element());
            }
        }

        children
    }

    fn image_state_binding(this: Entity<Self>) -> Binding<EditorImageState> {
        Binding::entity(
            this,
            |state: &Self| state.image_state.clone().unwrap_or_else(|| EditorImageState::empty(None, size(0.0, 0.0))),
            |state, value, cx| state.set_image_state(Some(value), cx),
        )
    }

    /// everything that moves the stage as one
    /// dismiss drag, the pager and the hero hiding the picture while it flies
    /// pages is off for the crop and draw stage of a video
    fn stage_transform(&self, stage: impl IntoElement, pages: bool) -> AnyElement {
        let embed = self.props.embed_in_hero_host;
        let scale = if embed { self.props.image_dismiss_scale } else { 1.0 };
        let offset = if embed { self.props.image_dismiss_offset } else { size(0.0, 0.0) };
        let hidden = pages && self.props.image_stage_hidden;
        let moved = div()
            .absolute()
            .left(px(offset.width))
            .top(px(offset.height))
            .w(px(self.stage_bounds.size.width))
            .h(px(self.stage_bounds.size.height))
            .child(div().size_full().child(stage).scale_effect(scale));
        let paged = moved.modifier(MediaEditorPageOffset { pager: if pages { self.props.pager.clone() } else { None } });
        div().absolute().inset_0().when(hidden, |layer| layer.invisible()).child(paged).into_any_element()
    }

    fn image_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (Some(preview), Some(state)) = (self.rendered_preview.clone(), self.image_state.clone()) else {
            if self.props.seed_hero_image.is_none() {
                return Self::spinner();
            }
            let safe_insets = self.stage_safe_insets(window, cx);
            let chrome = self.chrome_children(
                EditorChromeInput {
                    safe_insets,
                    brush_rect: self.image_rect.rect,
                    text_rect: self.image_rect.rect,
                    asset: None,
                },
                window,
                cx,
            );
            return div().absolute().inset_0().child(self.chrome_layer(chrome, window)).into_any_element();
        };
        let this = cx.entity();
        let geo = self.stage_bounds.size;
        let safe_insets = self.stage_safe_insets(window, cx);
        let stage_image = self.image_for_stage(self.mode, &preview, &state);
        let stage_display_size = EditorImageLayout::oriented_display_size(
            image_pixel_size(&stage_image),
            if self.mode == EditorMode::Crop { state.rotation_steps } else { 0 },
            0.0,
        );
        let insets = EditorImageLayout::compute_insets(
            geo,
            stage_display_size,
            safe_insets.top,
            safe_insets.bottom,
            safe_insets.left,
            safe_insets.right,
            self.mode,
            self.resolved_toolbar_height(),
        );
        let stage = self
            .editor_stage(this.clone(), stage_image, Self::image_state_binding(this.clone()), insets, false)
            .allows_horizontal_page_drag(self.allows_page_drag())
            .on_horizontal_page_drag_changed(self.props.on_page_drag_changed.clone())
            .on_horizontal_page_drag_ended(self.props.on_page_drag_ended.clone())
            .on_single_tap(self.props.shows_picker_top_bar.then(|| {
                let tapped = this.clone();
                Rc::new(move |cx: &mut App| tapped.update(cx, |state, cx| state.handle_canvas_tap(cx)))
                    as Rc<dyn Fn(&mut App)>
            }));
        let frame = self.media_edge_frame();
        let stage = self.stage_transform(div().relative().size_full().child(stage).children(frame), true);
        let chrome = self.chrome_children(
            EditorChromeInput {
                safe_insets,
                brush_rect: self.image_rect.rect,
                text_rect: self.image_rect.rect,
                asset: None,
            },
            window,
            cx,
        );
        div().absolute().inset_0().child(stage).child(self.chrome_layer(chrome, window)).into_any_element()
    }

    fn media_edge_frame(&self) -> Option<AnyElement> {
        let rect = self.image_rect.rect;
        if self.mode == EditorMode::Crop || rect.size.width <= 1.0 || rect.size.height <= 1.0 {
            return None;
        }
        Some(
            div()
                .absolute()
                .left(px(rect.origin.x))
                .top(px(rect.origin.y))
                .w(px(rect.size.width))
                .h(px(rect.size.height))
                .border_1()
                .border_color(white().opacity(0.18))
                .into_any_element(),
        )
    }

    fn editor_stage(
        &self,
        this: Entity<Self>,
        rendered_image: Arc<RenderImage>,
        state: Binding<EditorImageState>,
        insets: Edges<f32>,
        reports_video_rect: bool,
    ) -> EditorImageStage {
        let on_rect = this.clone();
        let on_edit = this.clone();
        let on_create = this.clone();
        let on_tag = this.clone();
        let on_focus = this;
        EditorImageStage::new(
            if reports_video_rect { "media-editor-video-stage" } else { "media-editor-image-stage" },
            self.crop_coordination_id,
            rendered_image,
            state,
            self.mode,
            self.brush_tool,
            self.brush_color,
            self.brush_width_fraction,
            self.allows_image_zoom(),
            insets,
            Rc::new(move |rect: EditorImageRect, cx: &mut App| {
                on_rect.update(cx, |state, cx| {
                    let slot = if reports_video_rect { &mut state.video_image_rect } else { &mut state.image_rect };
                    if *slot != rect {
                        *slot = rect;
                        cx.notify();
                    }
                });
            }),
            Rc::new(move |id: Uuid, cx: &mut App| {
                on_edit.update(cx, |state, cx| state.begin_editing_text(id, cx));
            }),
            Rc::new(move |_: Point<f32>, cx: &mut App| {
                on_create.update(cx, |state, cx| state.begin_creating_text(cx));
            }),
        )
        .on_request_create_tag(Rc::new(move |point: Point<f32>, cx: &mut App| {
            on_tag.update(cx, |state, cx| state.present_person_tag_picker(point, cx));
        }))
        .on_text_focus_changed(Rc::new(move |id: Option<Uuid>, cx: &mut App| {
            on_focus.update(cx, |state, cx| {
                if let Some(presentation) = state.props.editor_presentation.clone() {
                    presentation.update(cx, |presentation, cx| {
                        presentation.focused_text_entity_id = id;
                        cx.notify();
                    });
                }
                state.sync_navigation_pop_guard(cx);
            });
        }))
        .allows_vertical_dismiss_drag(self.props.embed_in_hero_host && self.allows_editor_dismiss_swipe())
        .on_vertical_dismiss_drag_changed(if self.props.embed_in_hero_host {
            self.props.on_host_dismiss_drag_changed.clone()
        } else {
            None
        })
        .on_vertical_dismiss_drag_ended(if self.props.embed_in_hero_host {
            self.props.on_host_dismiss_drag_ended.clone()
        } else {
            None
        })
    }

    fn picker_top_bar_top(safe_top: f32) -> f32 {
        if safe_top > 30.0 { safe_top - 12.0 } else { safe_top }
    }

    /// safe insets the image stage is fitted into
    fn stage_safe_insets(&self, window: &Window, cx: &App) -> Edges<f32> {
        Self::stage_safe_insets_for(Self::reported_safe_insets(window, cx), self.props.shows_picker_top_bar)
    }

    pub(crate) fn stage_safe_insets_for(reported: Edges<f32>, shows_picker_top_bar: bool) -> Edges<f32> {
        let mut insets = reported;
        if shows_picker_top_bar {
            let bar_bottom = Self::picker_top_bar_top(insets.top) + MediaEditorPickerTopBar::HEIGHT + 4.0;
            insets.top = insets.top.max(bar_bottom);
        }
        insets
    }

    /// the only unsafe band on desktop is the top band of a window cover
    fn reported_safe_insets(window: &Window, cx: &App) -> Edges<f32> {
        let top = f32::from(WindowCoverFrame::resolve(window, cx).safe_top);
        EditorImageLayout::resolved_safe_area_insets(EdgeInsets::new(top, 0.0, 0.0, 0.0))
    }

    fn spinner() -> AnyElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(progress_view().tint(white()))
            .into_any_element()
    }

    fn resolved_toolbar_height(&self) -> f32 {
        match self.mode {
            EditorMode::Crop => {
                if self.crop_chrome_height > 1.0 {
                    self.crop_chrome_height
                } else {
                    EditorImageLayout::fallback_toolbar_height(EditorMode::Crop)
                }
            }
            mode => {
                if self.main_chrome_height > 1.0 {
                    self.main_chrome_height
                } else {
                    EditorImageLayout::fallback_toolbar_height(mode)
                }
            }
        }
    }

    fn resolved_main_chrome_height(&self) -> f32 {
        if self.main_chrome_height > 1.0 {
            self.main_chrome_height
        } else {
            EditorImageLayout::fallback_toolbar_height(EditorMode::Initial)
        }
    }

    fn allows_image_zoom(&self) -> bool {
        if self.canvas_active() {
            return false;
        }
        matches!(self.mode, EditorMode::Initial | EditorMode::Adjust | EditorMode::Crop)
    }

    fn music_tool_action(&self, this: Entity<Self>) -> Option<ButtonAction> {
        if !self.props.configuration.allows_music_selection {
            return None;
        }
        Some(Rc::new(move |_: &mut Window, cx: &mut App| {
            this.update(cx, |state, cx| {
                state.show_track_picker = true;
                cx.notify();
            });
        }))
    }

    fn tag_people_tool_action(&self, this: Entity<Self>) -> Option<ButtonAction> {
        if !self.props.configuration.allows_person_tagging {
            return None;
        }
        Some(Rc::new(Self::mode_action(this, EditorMode::TagPeople)))
    }

    fn mode_action(this: Entity<Self>, mode: EditorMode) -> impl Fn(&mut Window, &mut App) + 'static {
        move |_, cx| this.update(cx, |state, cx| state.set_mode(mode, cx))
    }

    fn main_chrome_height_changed(&mut self, height: f32, cx: &mut Context<Self>) {
        if height <= 1.0 {
            return;
        }
        if self.main_chrome_height != height {
            self.main_chrome_height = height;
            cx.notify();
        }
        if self.mode == EditorMode::Initial
            && let Some(pager) = self.props.pager.clone()
            && pager.read(cx).chrome_height != height
        {
            pager.update(cx, |pager, cx| {
                pager.chrome_height = height;
                cx.notify();
            });
        }
    }

    /// one absolute slot per panel
    /// a leaving and an arriving panel overlap while they cross, a shared column cant do that
    fn panel_slot(this: Entity<Self>, reports_height: bool, panel: impl IntoElement) -> gpui::Div {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .child(div().flex().justify_center().child(div().flex_1().max_w(px(MEDIA_EDITOR_PANEL_MAX_WIDTH)).child(panel)))
            .when(reports_height, |slot| {
                slot.child(editor_main_chrome_height_reader("media-editor-main-chrome-height", move |height, _, cx| {
                    this.update(cx, |state, cx| state.main_chrome_height_changed(height, cx));
                }))
            })
    }

    fn bottom_panel(&mut self, this: Entity<Self>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let now = RiseClock::monotonic();
        let mode = self.mode;
        let canvas_active = self.canvas_active();
        let mut panels = Vec::new();

        if let Some(element) = self.chrome.initial_panel.render_transitioned(now, window, |_| {
            let back = this.clone();
            let send = this.clone();
            let panel = EditorBottomBar::new(
                self.props.caption.clone(),
                move |window, cx| back.update(cx, |state, cx| state.dismiss_to_grid(window, cx)),
                move |window, cx| send.update(cx, |state, cx| state.commit(true, window, cx)),
                Self::mode_action(this.clone(), EditorMode::Crop),
                Self::mode_action(this.clone(), EditorMode::Draw),
                Self::mode_action(this.clone(), EditorMode::Adjust),
            )
            .shows_caption(self.props.configuration.allows_caption)
            .has_attached_track(self.props.selection.read(cx).attached_track.is_some())
            .on_music(self.music_tool_action(this.clone()))
            .has_person_tags(!self.active_person_tags().is_empty())
            .on_tag_people(self.tag_people_tool_action(this.clone()))
            .shows_crop_and_adjust(!canvas_active)
            .on_add_photo(canvas_active.then(|| {
                let add = this.clone();
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    add.update(cx, |state, cx| state.present_canvas_gallery(cx));
                }) as ButtonAction
            }))
            .add_photo_enabled(self.canvas_photo_count() < StoryCanvasGeometry::photos_max());
            Self::panel_slot(this.clone(), mode == EditorMode::Initial, panel)
        }) {
            panels.push(element.into_any_element());
        }

        if let Some(element) = self.chrome.draw_panel.render_transitioned(now, window, |_| {
            let select_text = this.clone();
            let select_stickers = this.clone();
            let pick_color = this.clone();
            let picker = this.clone();
            let panel = EditorBrushPanel::new(
                Binding::entity(
                    this.clone(),
                    |state: &Self| state.brush_tool,
                    |state, value, _| {
                        state.brush_tool = value;
                        if EditorBrushPanel::paints_with_color(value) {
                            state.last_painting_tool = value;
                        }
                    },
                ),
                self.brush_color,
                Binding::entity(
                    this.clone(),
                    |state: &Self| state.brush_width_fraction,
                    |state, value, _| state.brush_width_fraction = value,
                ),
                move |color: Hsla, _: &mut Window, cx: &mut App| {
                    pick_color.update(cx, |state, cx| state.apply_brush_color(color, cx));
                },
                Self::mode_action(this.clone(), EditorMode::Initial),
                Self::mode_action(this.clone(), EditorMode::Initial),
                move |_, cx| select_text.update(cx, |state, cx| state.begin_creating_text(cx)),
                move |_, cx| select_stickers.update(cx, |state, cx| state.present_sticker_picker(cx)),
                move |window, cx| {
                    let initial = picker.read(cx).brush_color;
                    let done = picker.clone();
                    EditorColorPickerLauncher::present(
                        initial,
                        true,
                        |_, _, _| {},
                        move |color, _, cx| {
                            done.update(cx, |state, cx| state.apply_brush_color(color, cx));
                        },
                        window,
                        cx,
                    );
                },
            )
            .on_select_link(self.link_tool_action(this.clone()))
            .allows_blur(!self.props.is_video && !canvas_active);
            Self::panel_slot(this.clone(), mode == EditorMode::Draw, panel)
        }) {
            panels.push(element.into_any_element());
        }

        if let Some(element) = self.chrome.adjust_panel.render_transitioned(now, window, |_| {
            let panel = EditorAdjustmentsPanel::new(
                Self::active_adjustments_binding(this.clone()),
                Self::mode_action(this.clone(), EditorMode::Initial),
                Self::mode_action(this.clone(), EditorMode::Initial),
            );
            Self::panel_slot(this.clone(), mode == EditorMode::Adjust, panel)
        }) {
            panels.push(element.into_any_element());
        }

        if let Some(element) = self.chrome.tag_people_panel.render_transitioned(now, window, |_| {
            let panel = EditorTagPeoplePanel::new(
                self.active_person_tags().len() as i64,
                Self::mode_action(this.clone(), EditorMode::Initial),
                Self::mode_action(this.clone(), EditorMode::Initial),
            );
            Self::panel_slot(this.clone(), mode == EditorMode::TagPeople, panel)
        }) {
            panels.push(element.into_any_element());
        }

        panels
    }

    fn tag_hint_banner(&self, top_inset: f32) -> gpui::Div {
        let limit = *MAX_PERSON_TAGS_PER_SLIDE;
        let at_limit = self.active_person_tags().len() as i64 >= limit;
        let text = if at_limit {
            tr_plural("media_editor_tag_limit_reached", limit)
        } else {
            tr("media_editor_tag_hint")
        };
        div().absolute().top_0().left_0().right_0().flex().flex_col().items_center().child(
            div().mx(px(24.0)).mt(px(top_inset + 12.0)).child(
                div()
                    .px(px(16.0))
                    .py(px(10.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .text_center()
                    .child(text)
                    .liquid_chrome_capsule_with(0.1, true, None, false),
            ),
        )
    }

    fn active_adjustments(&self) -> EditorImageAdjustments {
        if self.props.is_video {
            return self
                .video_state
                .as_ref()
                .and_then(|state| state.spatial_edit.as_ref())
                .map(|state| state.adjustments)
                .unwrap_or_default();
        }
        self.image_state.as_ref().map(|state| state.adjustments).unwrap_or_default()
    }

    fn active_adjustments_binding(this: Entity<Self>) -> Binding<EditorImageAdjustments> {
        Binding::entity(
            this,
            |state: &Self| state.active_adjustments(),
            |state, new_value, cx| {
                if state.props.is_video
                    && let Some(mut video_state) = state.video_state.clone()
                {
                    let mut spatial = video_state.spatial_edit.clone().unwrap_or_else(|| {
                        EditorImageState::empty(Some(state.asset_local_id()), state.asset_pixel_size())
                    });
                    spatial.adjustments = new_value;
                    spatial.was_edited = true;
                    video_state.spatial_edit = Some(spatial);
                    video_state.was_edited = true;
                    state.set_video_state(Some(video_state), cx);
                    return;
                }
                if let Some(mut image_state) = state.image_state.clone() {
                    image_state.adjustments = new_value;
                    image_state.was_edited = true;
                    state.set_image_state(Some(image_state), cx);
                }
            },
        )
    }

    fn sound_sheet_overlay(&self, this: Entity<Self>, cx: &App) -> AnyElement {
        let background = self.props.theme_store.read(cx).current_theme().bg._100;
        let content = this.clone();
        let selection = self.props.selection.clone();
        let close = this.clone();
        let sheet = BottomSheetUi::new(
            "media-editor-sound-sheet",
            Binding::entity(this, |state: &Self| state.show_sound_sheet, |state, value, _| state.show_sound_sheet = value),
            move |_, cx| {
                let attached = selection.read(cx).attached_track.clone();
                EditorSoundSheet::new(
                    Self::video_volume_binding(content.clone()),
                    Self::music_volume_binding(content.clone()),
                    attached.as_ref().map(|attached| attached.track.artist_title_line()),
                    attached.is_some(),
                )
                .on_close({
                    let close = close.clone();
                    move |_, cx| close.update(cx, |state, cx| {
                        state.show_sound_sheet = false;
                        cx.notify();
                    })
                })
            },
            |_, _| div(),
        )
        .snap_fractions(vec![0.42])
        .fits_content(true)
        .footer_shows_top_border(false)
        .content_scrollable(false)
        .sheet_background(Some(background));
        div().absolute().inset_0().child(sheet).into_any_element()
    }

    fn track_picker_overlay(&self, this: Entity<Self>, cx: &App) -> AnyElement {
        let background = self.props.theme_store.read(cx).current_theme().bg._100;
        let content = this.clone();
        let sheet = BottomSheetUi::new(
            "media-editor-track-picker",
            Binding::entity(this, |state: &Self| state.show_track_picker, |state, value, _| state.show_track_picker = value),
            move |window, cx| {
                let attached = content.read(cx).attached_track(cx);
                let (pick, edit, remove, dismiss) = (content.clone(), content.clone(), content.clone(), content.clone());
                let initial = attached.clone();
                let view = window.use_keyed_state("media-editor-track-picker-view", cx, move |_, cx| {
                    EditorTrackPickerSheetHolder { view: cx.new(|_| EditorTrackPickerSheetView::new(
                        initial,
                        move |track, cx| pick.update(cx, |state, cx| state.handle_picked_track(track, cx)),
                        move |_, cx| {
                            edit.update(cx, |state, cx| {
                                if let Some(attached) = state.attached_track(cx) {
                                    state.present_trim(attached.track, cx);
                                }
                            });
                        },
                        move |_, cx| {
                            remove.update(cx, |state, cx| {
                                state.set_attached_track(None, cx);
                                state.set_music_plate(None, cx);
                            });
                        },
                        move |_, cx| {
                            dismiss.update(cx, |state, cx| {
                                state.show_track_picker = false;
                                cx.notify();
                            });
                        },
                    )),
                    }
                })
                .read(cx)
                .view
                .clone();
                view.update(cx, |view, cx| view.set_attached_track(attached, cx));
                AnyView::from(view).cached(StyleRefinement::default().size_full())
            },
            |_, _| div(),
        )
        .snap_fractions(vec![0.88, 0.93])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(background));
        div().absolute().inset_0().child(sheet).into_any_element()
    }

    /// the clip picker is a full screen overlay that dims the editor below, not a bottom sheet
    fn track_trim_overlay(&self, this: Entity<Self>, window: &mut Window, cx: &App) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        self.track_trim_presence
            .render_transitioned(now, window, |track| {
                let (start_ms, end_ms) = self.current_segment(track, cx);
                let confirm = this.clone();
                let cancel = this.clone();
                let confirmed_track = track.clone();
                div().absolute().inset_0().child(
                    TrackTrimOverlay::new(
                        "media-editor-track-trim",
                        track.clone(),
                        start_ms,
                        end_ms,
                        move |start, end, plate, _, cx| {
                            let track = confirmed_track.clone();
                            confirm.update(cx, |state, cx| state.track_trim_confirmed(track, start, end, plate, cx));
                        },
                        move |_, cx| {
                            cancel.update(cx, |state, cx| {
                                state.show_track_trim = false;
                                cx.notify();
                            });
                        },
                    )
                    .allows_plate(true)
                    .initial_plate(self.current_plate_choice()),
                )
            })
            .map(IntoElement::into_any_element)
    }

    fn track_trim_confirmed(
        &mut self,
        track: RiseStoredMusicTrack,
        start: i64,
        end: i64,
        plate: Option<TrackTrimPlateChoice>,
        cx: &mut Context<Self>,
    ) {
        self.set_attached_track(
            Some(MediaPickerTrackSelection { track: track.clone(), start_ms: start, end_ms: end, plate: None }),
            cx,
        );
        self.apply_plate_choice(plate, &track, cx);
        self.music_segment_pinned = true;
        self.apply_music_preview(cx);
        self.show_track_trim = false;
        cx.notify();
    }

    fn current_segment(&self, track: &RiseStoredMusicTrack, cx: &App) -> (i64, i64) {
        if let Some(attached) = self.attached_track(cx)
            && attached.track.id == track.id
        {
            return (attached.start_ms, attached.end_ms);
        }
        (0, 0)
    }

    fn active_person_tags(&self) -> Vec<EditorPersonTagEntity> {
        let tags = if self.props.is_video {
            self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref()).and_then(|state| state.person_tags.clone())
        } else {
            self.image_state.as_ref().and_then(|state| state.person_tags.clone())
        };
        tags.unwrap_or_default()
    }

    fn set_person_tags(&mut self, tags: Vec<EditorPersonTagEntity>, cx: &mut Context<Self>) {
        if self.props.is_video {
            let Some(mut video_state) = self.video_state.clone() else {
                return;
            };
            if tags.is_empty() && video_state.spatial_edit.is_none() {
                return;
            }
            let mut spatial = video_state
                .spatial_edit
                .clone()
                .unwrap_or_else(|| EditorImageState::empty(Some(self.asset_local_id()), self.asset_pixel_size()));
            spatial.person_tags = Some(tags);
            spatial.synchronize_edit_flag();
            video_state.spatial_edit = Some(spatial);
            video_state.was_edited = video_state.has_meaningful_edits();
            self.set_video_state(Some(video_state), cx);
            return;
        }
        let Some(mut state) = self.image_state.clone() else {
            return;
        };
        state.person_tags = Some(tags);
        state.synchronize_edit_flag();
        self.set_image_state(Some(state), cx);
    }

    fn present_person_tag_picker(&mut self, normalized_point: Point<f32>, cx: &mut Context<Self>) {
        let tags = self.active_person_tags();
        if tags.len() as i64 >= *MAX_PERSON_TAGS_PER_SLIDE {
            let _ = notification(NotificationKind::Warning);
            return;
        }
        let excluded: HashSet<String> = tags.iter().map(|tag| tag.user_id.clone()).collect();
        let this = cx.weak_entity();
        PeoplePickerStore::shared(cx).update(cx, |store, cx| {
            let _ = store.present(
                PeoplePickerMode::Friends,
                1,
                Vec::new(),
                excluded,
                tr("post_tag_people_action"),
                Some(tr("people_picker_friends_only_hint")),
                PeoplePickerHostContext::MediaEditor,
                move |picked, cx| {
                    let Some(user) = picked.first() else {
                        return;
                    };
                    if tags.iter().any(|tag| tag.user_id == user.id) {
                        return;
                    }
                    let mut tags = tags;
                    tags.push(EditorPersonTagEntity {
                        id: Uuid::new_v4(),
                        user_id: user.id.clone(),
                        name: user.name.clone(),
                        tag: user.tag.clone(),
                        logo: Some(user.logo.clone()),
                        center: point(normalized_point.x.clamp(0.0, 1.0), normalized_point.y.clamp(0.0, 1.0)),
                        is_dark: None,
                    });
                    let _ = this.update(cx, |state, cx| state.set_person_tags(tags, cx));
                },
                cx,
            );
        });
    }

    fn active_music_plate(&self) -> Option<EditorMusicPlateEntity> {
        if self.props.is_video {
            self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref()).and_then(|state| state.music_plate.clone())
        } else {
            self.image_state.as_ref().and_then(|state| state.music_plate.clone())
        }
    }

    fn current_plate_choice(&self) -> Option<TrackTrimPlateChoice> {
        self.active_music_plate().map(|plate| TrackTrimPlateChoice { is_dark: plate.is_dark })
    }

    fn apply_plate_choice(
        &mut self,
        choice: Option<TrackTrimPlateChoice>,
        track: &RiseStoredMusicTrack,
        cx: &mut Context<Self>,
    ) {
        let updated = self.updated_plate(self.active_music_plate(), choice, track);
        self.set_music_plate(updated.clone(), cx);
        self.sync_plate_to_selection(cx);
        if let Some(updated) = updated {
            EditorMusicPlateCoverStore::shared().prefetch(&updated.track_id, &updated.cover_url, None, cx);
        }
    }

    fn updated_plate(
        &self,
        existing: Option<EditorMusicPlateEntity>,
        choice: Option<TrackTrimPlateChoice>,
        track: &RiseStoredMusicTrack,
    ) -> Option<EditorMusicPlateEntity> {
        let choice = choice?;
        let track_id = track.id.clone().unwrap_or_default();
        let title = track.title.clone().unwrap_or_default();
        let artist = track.artist.as_ref().map(|artist| artist.stage_name.clone()).unwrap_or_default();
        let cover_url = RemoteMediaURL::optional_normalized(track.cover_url.as_deref()).unwrap_or_default();
        if let Some(mut plate) = existing {
            plate.track_id = track_id;
            plate.title = title;
            plate.artist = artist;
            plate.cover_url = cover_url;
            plate.is_dark = choice.is_dark;
            return Some(plate);
        }
        Some(EditorMusicPlateEntity {
            id: Uuid::new_v4(),
            track_id,
            title,
            artist,
            cover_url,
            is_dark: choice.is_dark,
            center: point(0.5, 0.3),
            rotation: 0.0,
            scale: 1.0,
        })
    }

    /// plate ships with the post, not the baked media
    /// so every transform tweak on the canvas is mirrored into the session selection that publish reads
    fn sync_plate_to_selection(&mut self, cx: &mut Context<Self>) {
        let Some(mut attached) = self.attached_track(cx) else {
            return;
        };
        let mirrored = self.active_music_plate().map(|plate| MediaPickerTrackPlate {
            is_dark: plate.is_dark,
            center_x: f64::from(plate.center.x),
            center_y: f64::from(plate.center.y),
            rotation: f64::from(plate.rotation),
            scale: f64::from(plate.scale),
        });
        if attached.plate != mirrored {
            attached.plate = mirrored;
            self.set_attached_track(Some(attached), cx);
        }
    }

    fn set_music_plate(&mut self, plate: Option<EditorMusicPlateEntity>, cx: &mut Context<Self>) {
        if self.props.is_video {
            let Some(mut video_state) = self.video_state.clone() else {
                return;
            };
            if plate.is_none() && video_state.spatial_edit.is_none() {
                return;
            }
            let mut spatial = video_state
                .spatial_edit
                .clone()
                .unwrap_or_else(|| EditorImageState::empty(Some(self.asset_local_id()), self.asset_pixel_size()));
            spatial.music_plate = plate;
            spatial.synchronize_edit_flag();
            video_state.spatial_edit = Some(spatial);
            video_state.was_edited = video_state.has_meaningful_edits();
            self.set_video_state(Some(video_state), cx);
            self.recompute_video_poster_preview(cx);
            return;
        }
        let Some(mut state) = self.image_state.clone() else {
            return;
        };
        state.music_plate = plate;
        state.synchronize_edit_flag();
        self.set_image_state(Some(state), cx);
    }

    fn handle_picked_track(&mut self, track: RiseStoredMusicTrack, cx: &mut Context<Self>) {
        let attached_id = self.attached_track(cx).and_then(|attached| attached.track.id);
        if attached_id != track.id {
            let duration = track.duration_ms.unwrap_or(0);
            let length = if duration > 0 {
                TrackTrimOverlay::MAX_LENGTH_MS.min(duration)
            } else {
                TrackTrimOverlay::MAX_LENGTH_MS
            };
            self.set_attached_track(
                Some(MediaPickerTrackSelection { track: track.clone(), start_ms: 0, end_ms: length, plate: None }),
                cx,
            );
            if self.active_music_plate().is_some() {
                self.apply_plate_choice(self.current_plate_choice(), &track, cx);
            }
        }
        self.present_trim(track, cx);
    }

    fn present_trim(&mut self, track: RiseStoredMusicTrack, cx: &mut Context<Self>) {
        self.trim_track = Some(track);
        self.show_track_picker = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(0.35)).await;
            let _ = this.update(cx, |state, cx| {
                state.show_track_trim = true;
                cx.notify();
            });
        })
        .detach();
    }

    fn draw_mode_right_slider(&self, this: Entity<Self>) -> gpui::Div {
        let dragging = this.clone();
        let fraction = Binding::entity(
            this,
            |state: &Self| state.brush_width_fraction,
            |state, value, _| state.brush_width_fraction = value,
        );
        let bottom = if self.main_chrome_height > 1.0 {
            self.main_chrome_height
        } else {
            EditorImageLayout::fallback_toolbar_height(EditorMode::Draw)
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .child(spacer_min_h(80.0))
            .child(div().flex().flex_row().child(spacer()).child(div().pr(px(12.0)).child(EditorThicknessTrack::new(
                "media-editor-thickness-track",
                fraction,
                200.0,
                22.0,
                move |active, _, cx| {
                    dragging.update(cx, |state, cx| {
                        state.thickness_is_dragging = active;
                        cx.notify();
                    });
                },
            ))))
            .child(spacer_min_h(bottom))
    }

    fn active_strokes(&self) -> Vec<EditorDrawStroke> {
        if self.props.is_video {
            self.video_state
                .as_ref()
                .and_then(|state| state.spatial_edit.as_ref())
                .map(|state| state.strokes.clone())
                .unwrap_or_default()
        } else {
            self.image_state.as_ref().map(|state| state.strokes.clone()).unwrap_or_default()
        }
    }

    fn on_active_strokes_changed(&mut self, old: Vec<EditorDrawStroke>, cx: &mut Context<Self>) {
        if let Some(applying) = self.draw_history_applying.take()
            && applying == self.active_strokes()
        {
            return;
        }
        if self.mode != EditorMode::Draw {
            return;
        }
        self.draw_undo.push(old);
        self.draw_redo.clear();
        self.sync_draw_history_fades(cx);
    }

    fn set_active_strokes(&mut self, strokes: Vec<EditorDrawStroke>, cx: &mut Context<Self>) {
        self.draw_history_applying = Some(strokes.clone());
        if self.props.is_video {
            let Some(mut video_state) = self.video_state.clone() else {
                return;
            };
            let Some(mut spatial) = video_state.spatial_edit.clone() else {
                return;
            };
            spatial.strokes = strokes;
            spatial.was_edited = true;
            video_state.spatial_edit = Some(spatial);
            video_state.was_edited = true;
            self.set_video_state(Some(video_state), cx);
            return;
        }
        let Some(mut state) = self.image_state.clone() else {
            return;
        };
        state.strokes = strokes;
        state.was_edited = true;
        self.set_image_state(Some(state), cx);
    }

    fn undo_draw_stroke(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.draw_undo.pop() else {
            return;
        };
        let current = self.active_strokes();
        self.draw_redo.push(current);
        self.set_active_strokes(previous, cx);
        self.sync_draw_history_fades(cx);
    }

    fn redo_draw_stroke(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self.draw_redo.pop() else {
            return;
        };
        let current = self.active_strokes();
        self.draw_undo.push(current);
        self.set_active_strokes(next, cx);
        self.sync_draw_history_fades(cx);
    }

    /// fades the history buttons in and out, 0.15s ease out
    fn sync_draw_history_fades(&mut self, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        let undo = if self.draw_undo.is_empty() { 0.35 } else { 1.0 };
        let redo = if self.draw_redo.is_empty() { 0.35 } else { 1.0 };
        if *self.draw_undo_fade.target() != undo {
            self.draw_undo_fade.set_in(undo, ease_out(0.15), now, cx);
        }
        if *self.draw_redo_fade.target() != redo {
            self.draw_redo_fade.set_in(redo, ease_out(0.15), now, cx);
        }
        cx.notify();
    }

    fn draw_history_bar(&self, this: Entity<Self>, safe_top: f32, window: &Window) -> gpui::Div {
        let now = RiseClock::monotonic();
        let redo_this = this.clone();
        let undo = Self::draw_history_button(
            "media-editor-draw-undo",
            "arrow.uturn.backward",
            tr("desktop_menu_undo"),
            !self.draw_undo.is_empty(),
            self.draw_undo_fade.frame_value(now, window),
            move |_: &mut Window, cx: &mut App| this.update(cx, |state, cx| state.undo_draw_stroke(cx)),
        );
        let redo = Self::draw_history_button(
            "media-editor-draw-redo",
            "arrow.uturn.forward",
            tr("desktop_menu_redo"),
            !self.draw_redo.is_empty(),
            self.draw_redo_fade.frame_value(now, window),
            move |_: &mut Window, cx: &mut App| redo_this.update(cx, |state, cx| state.redo_draw_stroke(cx)),
        );
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .flex_row()
            .gap(px(10.0))
            .pl(px(14.0))
            .pt(px(safe_top + 8.0))
            .child(undo)
            .child(redo)
    }

    fn draw_history_button(
        id: &'static str,
        icon: &'static str,
        label: String,
        enabled: bool,
        icon_opacity: f32,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> LiquidGlassButton {
        LiquidGlassButton::new(id, LiquidGlassButtonShape::Circle)
            .size(44.0)
            .disabled(!enabled)
            .accessibility_label(Some(label))
            .on_press(action)
            .children(IconUi::sized(icon, px(17.0), white().opacity(icon_opacity), false))
    }

    /// picking a color from a tool that ignores it goes back to the last painting tool
    /// eg eraser or blur
    fn apply_brush_color(&mut self, color: Hsla, cx: &mut Context<Self>) {
        self.brush_color = color;
        if !EditorBrushPanel::paints_with_color(self.brush_tool) {
            self.brush_tool = self.last_painting_tool;
        }
        cx.notify();
    }

    fn active_stickers(&self) -> Vec<EditorStickerEntity> {
        let stickers = if self.props.is_video {
            self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref()).and_then(|state| state.stickers.clone())
        } else {
            self.image_state.as_ref().and_then(|state| state.stickers.clone())
        };
        stickers.unwrap_or_default()
    }

    fn present_sticker_picker(&mut self, cx: &mut Context<Self>) {
        if self.active_stickers().len() as i64 >= *MAX_STICKERS_PER_SLIDE {
            let _ = notification(NotificationKind::Warning);
            return;
        }
        self.show_sticker_picker = true;
        self.sticker_picker_presence.set_in(Some(()), ease_out(0.25), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn dismiss_sticker_picker(&mut self, cx: &mut Context<Self>) {
        self.show_sticker_picker = false;
        self.sticker_picker_presence.set_in(None, ease_out(0.22), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn add_emoji(&mut self, emoji: String, cx: &mut Context<Self>) {
        if emoji.is_empty() {
            return;
        }
        self.insert_sticker(EditorStickerEntity::from_emoji(&emoji, point(0.5, 0.5)), cx);
    }

    fn add_sticker(&mut self, sticker: StickerItem, cx: &mut Context<Self>) {
        let loops = self.props.is_video || self.canvas_active();
        self.insert_sticker(EditorStickerEntity::from_sticker(&sticker, point(0.5, 0.5), loops), cx);
    }

    fn insert_sticker(&mut self, entity: EditorStickerEntity, cx: &mut Context<Self>) {
        if self.active_stickers().len() as i64 >= *MAX_STICKERS_PER_SLIDE {
            return;
        }
        EditorStickerArtStore::shared().load(&entity, |_, _| {}, cx);
        if self.props.is_video {
            let Some(mut video_state) = self.video_state.clone() else {
                return;
            };
            let mut spatial = video_state
                .spatial_edit
                .clone()
                .unwrap_or_else(|| EditorImageState::empty(Some(self.asset_local_id()), self.asset_pixel_size()));
            spatial.stickers.get_or_insert_with(Vec::new).push(entity);
            spatial.synchronize_edit_flag();
            video_state.spatial_edit = Some(spatial);
            video_state.was_edited = video_state.has_meaningful_edits();
            self.set_video_state(Some(video_state), cx);
            return;
        }
        let Some(mut state) = self.image_state.clone() else {
            return;
        };
        state.stickers.get_or_insert_with(Vec::new).push(entity);
        state.synchronize_edit_flag();
        self.set_image_state(Some(state), cx);
    }

    fn sticker_picker_overlay(
        &self,
        this: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        if !self.sticker_picker_presence.is_present() && !self.sticker_picker_presence.is_leaving(now) {
            return None;
        }
        let safe_bottom = Self::reported_safe_insets(window, cx).bottom;
        let bottom = self.resolved_toolbar_height() + safe_bottom + STICKER_PICKER_GAP;
        let available = f32::from(window.viewport_size().height) - bottom - STICKER_PICKER_TOP_MARGIN;
        let height = STICKER_PICKER_HEIGHT.min(available).max(160.0);
        let (fill, edge) = {
            let theme = ThemeStore::shared(cx).read(cx).current_theme();
            (LiquidChrome::surface_fill(theme, 0), media_edge_color(theme))
        };
        let backdrop = this.clone();
        let emoji = this.clone();
        let custom = this.clone();
        let sticker = this;
        let panel = ChatInputPickerPanel::new(
            "media-editor-sticker-picker",
            height,
            move |picked: String, _: &mut Window, cx: &mut App| {
                emoji.update(cx, |state, cx| {
                    state.dismiss_sticker_picker(cx);
                    state.add_emoji(picked, cx);
                });
            },
            move |picked: StickerItem, _: &mut Window, cx: &mut App| {
                custom.update(cx, |state, cx| {
                    state.dismiss_sticker_picker(cx);
                    state.add_sticker(picked, cx);
                });
            },
            |_: &mut Window, _: &mut App| {},
            move |picked: StickerItem, _: &mut Window, cx: &mut App| {
                sticker.update(cx, |state, cx| {
                    state.dismiss_sticker_picker(cx);
                    state.add_sticker(picked, cx);
                });
            },
            |_, _: &mut Window, _: &mut App| {},
            |_: &mut Window, _: &mut App| {},
            |_, _: &mut Window, _: &mut App| {},
        )
        .places_on_media(true)
        .surface_fill(Some(fill));
        self.sticker_picker_presence
            .render_transitioned(now, window, |_| {
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        div()
                            .id("media-editor-sticker-backdrop")
                            .absolute()
                            .inset_0()
                            .bg(black().opacity(0.001))
                            .on_click(move |_, _, cx| {
                                backdrop.update(cx, |state, cx| state.dismiss_sticker_picker(cx));
                            }),
                    )
                    .child(
                        div().absolute().left_0().right_0().bottom(px(bottom)).flex().justify_center().child(
                            div()
                                .id("media-editor-sticker-card")
                                .occlude()
                                .w(px(STICKER_PICKER_WIDTH))
                                .h(px(height))
                                .rounded(px(STICKER_PICKER_CORNER_RADIUS))
                                .overflow_hidden()
                                .bg(fill)
                                .border_1()
                                .border_color(edge)
                                .shadow_lg()
                                .child(panel),
                        ),
                    )
            })
            .map(IntoElement::into_any_element)
    }

    fn canvas_active(&self) -> bool {
        !self.props.is_video && self.image_state.as_ref().is_some_and(|state| state.story_canvas.is_some())
    }

    fn canvas_photo_count(&self) -> i64 {
        self.image_state
            .as_ref()
            .and_then(|state| state.story_canvas.as_ref())
            .map_or(0, |canvas| canvas.photos.len() as i64)
    }

    fn present_canvas_gallery(&mut self, cx: &mut Context<Self>) {
        if !self.canvas_active() || self.is_importing_canvas_photos {
            return;
        }
        let photos_max = StoryCanvasGeometry::photos_max();
        if self.canvas_photo_count() >= photos_max {
            let _ = notification(NotificationKind::Warning);
            Notifier::send(
                tr_args("story_canvas_photos_limit_reached", &[FormatArg::Int(photos_max)]),
                NotifierSendOptions { r#type: NotifierType::Warning, ..NotifierSendOptions::default() },
                cx,
            );
            return;
        }
        self.show_canvas_gallery = true;
        cx.notify();
    }

    fn handle_canvas_gallery_result(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
        self.show_canvas_gallery = false;
        cx.notify();
        if result.was_cancelled {
            return;
        }
        let free = (StoryCanvasGeometry::photos_max() - self.canvas_photo_count()).max(0) as usize;
        let sources: Vec<(Option<std::path::PathBuf>, Option<Vec<u8>>)> = result
            .items
            .into_iter()
            .filter(|item| matches!(item.kind, MediaPickerItemKind::Image))
            .take(free)
            .map(|item| (item.url.to_file_path(), item.image_payload.map(|payload| payload.data)))
            .collect();
        if sources.is_empty() {
            return;
        }
        self.is_importing_canvas_photos = true;
        cx.spawn(async move |this, cx| {
            let imported = cx
                .background_spawn(async move {
                    let store = StoryCanvasPhotoStore::shared();
                    sources
                        .into_iter()
                        .filter_map(|(path, data)| {
                            data.and_then(|data| store.import_data(&data))
                                .or_else(|| path.and_then(|path| store.import_file(&path)))
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |state, cx| state.add_canvas_photos(imported, cx));
        })
        .detach();
    }

    fn add_canvas_photos(&mut self, imported: Vec<(String, Size<f32>)>, cx: &mut Context<Self>) {
        self.is_importing_canvas_photos = false;
        cx.notify();
        let Some(mut state) = self.image_state.clone() else {
            return;
        };
        let Some(mut canvas) = state.story_canvas.clone() else {
            return;
        };
        if imported.is_empty() {
            return;
        }
        let photos_max = StoryCanvasGeometry::photos_max();
        for (index, (key, pixel_size)) in imported.into_iter().enumerate() {
            if canvas.photos.len() as i64 >= photos_max {
                continue;
            }
            let shift = index as f32 * 0.04;
            canvas.photos.push(EditorCanvasPhotoEntity {
                id: Uuid::new_v4(),
                photo_key: key,
                file_id: None,
                pixel_width: pixel_size.width as i64,
                pixel_height: pixel_size.height as i64,
                center: point((0.5 + shift).min(0.85), (0.5 + shift).min(0.85)),
                scale: StoryCanvasGeometry::ADDED_PHOTO_SCALE,
                rotation: 0.0,
                shape: RiseStoryCanvasPhotoShape::Square.wire().to_string(),
            });
        }
        state.story_canvas = Some(canvas);
        state.synchronize_edit_flag();
        self.set_image_state(Some(state), cx);
    }

    fn active_links(&self) -> Vec<EditorLinkEntity> {
        let links = if self.props.is_video {
            self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref()).and_then(|state| state.links.clone())
        } else {
            self.image_state.as_ref().and_then(|state| state.links.clone())
        };
        links.unwrap_or_default()
    }

    fn set_active_links(&mut self, links: Vec<EditorLinkEntity>, cx: &mut Context<Self>) {
        let value = if links.is_empty() { None } else { Some(links) };
        if self.props.is_video {
            let Some(mut video_state) = self.video_state.clone() else {
                return;
            };
            let mut spatial = video_state
                .spatial_edit
                .clone()
                .unwrap_or_else(|| EditorImageState::empty(Some(self.asset_local_id()), self.asset_pixel_size()));
            spatial.links = value;
            spatial.synchronize_edit_flag();
            video_state.spatial_edit = Some(spatial);
            video_state.was_edited = video_state.has_meaningful_edits();
            self.set_video_state(Some(video_state), cx);
            return;
        }
        let Some(mut state) = self.image_state.clone() else {
            return;
        };
        state.links = value;
        state.synchronize_edit_flag();
        self.set_image_state(Some(state), cx);
    }

    fn link_tool_action(&self, this: Entity<Self>) -> Option<ButtonAction> {
        if !self.props.configuration.allows_media_links {
            return None;
        }
        Some(Rc::new(move |_: &mut Window, cx: &mut App| {
            this.update(cx, |state, cx| state.present_link_input(cx));
        }))
    }

    fn links_placed_elsewhere(&self, cx: &App) -> i64 {
        if !self.props.configuration.media_links_shared_across_selection {
            return 0;
        }
        let mut own = HashSet::new();
        own.insert(self.asset_local_id());
        if let Some(asset) = &self.props.ph_asset {
            own.insert(asset.id.clone());
        }
        let selection = self.props.selection.read(cx);
        let mut count = self.props.configuration.media_links_elsewhere;
        for (id, edit) in selection.edits_by_identifier() {
            if own.contains(id) || !selection.contains(id) {
                continue;
            }
            let placed = match &edit.payload {
                MediaSelectionEditPayload::Image(state) => state.links.as_ref().map_or(0, Vec::len),
                MediaSelectionEditPayload::Video(state) => {
                    state.spatial_edit.as_ref().and_then(|spatial| spatial.links.as_ref()).map_or(0, Vec::len)
                }
            };
            count += placed as i64;
        }
        count
    }

    fn can_add_link(&self, cx: &App) -> bool {
        self.active_links().len() as i64 + self.links_placed_elsewhere(cx) < self.props.configuration.media_links_max
    }

    fn present_link_input(&mut self, cx: &mut Context<Self>) {
        if !self.can_add_link(cx) {
            let _ = notification(NotificationKind::Warning);
            Notifier::send(
                tr_args("media_link_limit_reached", &[FormatArg::Int(self.props.configuration.media_links_max)]),
                NotifierSendOptions { r#type: NotifierType::Warning, ..NotifierSendOptions::default() },
                cx,
            );
            return;
        }
        self.link_draft = String::new();
        self.link_draft_rejected = false;
        self.link_draft_fetch = None;
        self.link_draft_card = EditorLinkDraft::default();
        self.sync_link_draft_card_presence(cx);
        self.link_input_session += 1;
        self.show_link_input = true;
        cx.notify();
    }

    fn commit_link_draft(&mut self, cx: &mut Context<Self>) {
        let MediaLinkAddressOutcome::Valid(url) = MediaLinkAddress::normalize(&self.link_draft) else {
            self.link_draft_rejected = true;
            let _ = notification(NotificationKind::Warning);
            cx.notify();
            return;
        };
        if !self.can_add_link(cx) {
            self.show_link_input = false;
            cx.notify();
            return;
        }
        self.show_link_input = false;
        self.link_draft_fetch = None;
        cx.notify();
        let mut draft = self.link_draft_card.clone();
        draft.url = url.clone();
        let Some(entity) = draft.entity(self.system_is_dark(cx)) else {
            return;
        };
        let id = entity.id;
        let mut links = self.active_links();
        links.push(entity);
        self.set_active_links(links, cx);
        self.set_mode(EditorMode::Initial, cx);
        if !draft.has_snapshot() {
            self.load_link_preview(id, url, cx);
        }
    }

    fn system_is_dark(&self, cx: &App) -> bool {
        matches!(self.props.theme_store.read(cx).resolved_appearance(), RiseThemeAppearance::Dark)
    }

    /// fades the live link card in and out, 0.2s ease out
    fn sync_link_draft_card_presence(&mut self, cx: &mut Context<Self>) {
        let card = self.link_draft_card.entity(self.system_is_dark(cx));
        if card.is_some() != self.link_draft_card_presence.is_present() {
            self.link_draft_card_presence.set_in(card, ease_out(0.2), RiseClock::monotonic(), cx);
        }
    }

    fn update_link_draft(&mut self, value: String, cx: &mut Context<Self>) {
        self.link_draft_rejected = false;
        let url = match MediaLinkAddress::normalize(&value) {
            MediaLinkAddressOutcome::Valid(valid) => valid,
            MediaLinkAddressOutcome::Rejected => String::new(),
        };
        self.link_draft = value;
        cx.notify();
        if url == self.link_draft_card.url {
            return;
        }
        self.link_draft_fetch = None;
        self.link_draft_card.url = url.clone();
        self.sync_link_draft_card_presence(cx);
        if url.is_empty() {
            return;
        }
        let debounce = RiseTimingCanon::debounce("chat_link_preview").client_debounce_ms.max(0) as u64;
        self.link_draft_fetch = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(debounce)).await;
            let Ok(Some(load)) = this.update(cx, |state, cx| {
                (state.link_draft_card.url == url).then(|| {
                    MessageActionsStore::shared(cx).update(cx, |actions, cx| actions.load_link_preview_action(&url, cx))
                })
            }) else {
                return;
            };
            let Ok(preview) = load.await else {
                return;
            };
            let _ = this.update(cx, |state, cx| {
                if state.link_draft_card.url != url {
                    return;
                }
                state.link_draft_card.title = trimmed_preview_text(preview.title);
                state.link_draft_card.summary = trimmed_preview_text(preview.description);
                state.link_draft_card.image_url = trimmed_preview_text(preview.image_url);
                state.link_draft_card.fetched_url = url.clone();
                cx.notify();
            });
        }));
    }

    /// preview snapshot with title, description and picture comes from link-preview-service once
    /// card is drawn without it until then
    fn load_link_preview(&mut self, id: Uuid, url: String, cx: &mut Context<Self>) {
        let load = MessageActionsStore::shared(cx).update(cx, |actions, cx| actions.load_link_preview_action(&url, cx));
        cx.spawn(async move |this, cx| {
            let Ok(preview) = load.await else {
                return;
            };
            let _ = this.update(cx, |state, cx| {
                let mut links = state.active_links();
                let Some(index) = links.iter().position(|link| link.id == id) else {
                    return;
                };
                links[index].title = trimmed_preview_text(preview.title);
                links[index].summary = trimmed_preview_text(preview.description);
                links[index].image_url = trimmed_preview_text(preview.image_url);
                state.set_active_links(links, cx);
            });
        })
        .detach();
    }

    fn set_link_preview_mode(&mut self, mode: RiseMediaLinkPreviewMode, id: Uuid, cx: &mut Context<Self>) {
        let mut links = self.active_links();
        let Some(index) = links.iter().position(|link| link.id == id) else {
            return;
        };
        links[index].preview = mode.wire().to_string();
        self.set_active_links(links, cx);
    }

    fn set_link_style(&mut self, style: RiseMediaLinkStyle, id: Uuid, cx: &mut Context<Self>) {
        let mut links = self.active_links();
        let Some(index) = links.iter().position(|link| link.id == id) else {
            return;
        };
        links[index].style = style.wire().to_string();
        self.set_active_links(links, cx);
    }

    fn canvas_and_link_overlays(&self, this: Entity<Self>, window: &Window, cx: &App) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if self.canvas_active() {
            let remaining = (StoryCanvasGeometry::photos_max() - self.canvas_photo_count()).max(1);
            let done = this.clone();
            let sheet = MediaPickerSheet::new(
                "media-editor-canvas-gallery",
                Binding::entity(
                    this.clone(),
                    |state: &Self| state.show_canvas_gallery,
                    |state, value, cx| {
                        state.show_canvas_gallery = value;
                        cx.notify();
                    },
                ),
                MediaPickerConfiguration::story_canvas_add(remaining),
                self.props.theme_store.clone(),
                move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                    done.update(cx, |state, cx| state.handle_canvas_gallery_result(result, cx));
                },
            )
            .experimental_bottom_sheet(true);
            overlays.push(div().absolute().inset_0().child(sheet).into_any_element());
        }
        if self.props.configuration.allows_media_links {
            overlays.push(self.link_input_sheet(this.clone(), window, cx));
            overlays.push(self.link_customize_sheet(this, cx));
        }
        overlays
    }

    fn link_input_sheet(&self, this: Entity<Self>, _window: &Window, cx: &App) -> AnyElement {
        let background = self.props.theme_store.read(cx).current_theme().bg._100;
        let content = this.clone();
        let footer = this.clone();
        let sheet = BottomSheetUi::new(
            "media-editor-link-input",
            Binding::entity(
                this,
                |state: &Self| state.show_link_input,
                |state, value, cx| {
                    state.show_link_input = value;
                    cx.notify();
                },
            ),
            move |window: &mut Window, cx: &mut App| Self::link_input_content(content.clone(), window, cx),
            move |_: &mut Window, cx: &mut App| Self::link_input_footer(footer.clone(), cx),
        )
        .snap_fractions(vec![0.3])
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .footer_shows_top_border(false)
        .content_scrollable(false)
        .fits_content(true)
        .fits_content_with_keyboard(true)
        .sheet_background(Some(background))
        .footer_background(Some(background));
        div().absolute().inset_0().child(sheet).into_any_element()
    }

    fn link_input_content(this: Entity<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        let now = RiseClock::monotonic();
        let state = this.read(cx);
        let theme = state.props.theme_store.read(cx).current_theme().clone();
        let rejected = state.link_draft_rejected;
        let session = state.link_input_session;
        let draft = state.link_draft_card.clone();
        let card = draft
            .entity(state.system_is_dark(cx))
            .or_else(|| state.link_draft_card_presence.value(now).cloned());
        let controls = card.map(|card| Self::link_draft_controls(this.clone(), card, &draft, &theme));
        let controls = controls.and_then(|controls| {
            this.read(cx).link_draft_card_presence.render_transitioned(now, window, |_| controls)
        });
        let submit = this.clone();
        let typed = this;
        let input = InputUi::new(ElementId::Name(SharedString::from(format!("media-link-input-{session}"))))
            .error_message(rejected.then(|| tr("media_link_https_only")))
            .name("url")
            .placeholder(tr("media_link_placeholder"))
            .auto_focus(true)
            .on_submit(move |_: &mut Window, cx: &mut App| submit.update(cx, |state, cx| state.commit_link_draft(cx)))
            .set_value(move |_: &str, value: &str, _: &mut Window, cx: &mut App| {
                typed.update(cx, |state, cx| state.update_link_draft(value.to_string(), cx));
            });
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .px(px(16.0))
            .pt(px(6.0))
            .pb(px(4.0))
            .child(
                MainText::new(tr("media_link_add_title"))
                    .px(17.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary),
            )
            .child(input)
            .children(controls)
            .into_any_element()
    }

    fn link_draft_controls(this: Entity<Self>, card: EditorLinkEntity, draft: &EditorLinkDraft, theme: &AppTheme) -> gpui::Div {
        let style_this = this.clone();
        let mode_this = this;
        let modes = RiseMediaLinkPreviewMode::ALL;
        let selected = modes.iter().position(|mode| *mode == draft.preview).unwrap_or(0);
        let titles = modes.iter().map(|mode| SharedString::from(tr(mode.i18n_key()))).collect();
        let is_dark_theme = card.is_dark_theme;
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(EditorLinkLiveSample::new(card))
            .child(EditorLinkStyleSwatches::new(
                draft.style,
                is_dark_theme,
                theme.primary._100,
                move |style: RiseMediaLinkStyle, _: &mut Window, cx: &mut App| {
                    style_this.update(cx, |state, cx| {
                        state.link_draft_card.style = style;
                        cx.notify();
                    });
                },
            ))
            .child(SegmentedPicker::new(
                "media-link-preview-mode",
                titles,
                selected,
                move |index: usize, _: &mut Window, cx: &mut App| {
                    mode_this.update(cx, |state, cx| {
                        if let Some(mode) = RiseMediaLinkPreviewMode::ALL.get(index) {
                            state.link_draft_card.preview = *mode;
                            cx.notify();
                        }
                    });
                },
            ))
    }

    fn link_input_footer(this: Entity<Self>, cx: &mut App) -> AnyElement {
        let state = this.read(cx);
        let disabled = trimming_whitespaces_and_newlines(&state.link_draft).is_empty();
        let color = state.props.theme_store.read(cx).current_theme().text.primary;
        div()
            .px(px(16.0))
            .child(
                ButtonUi::new("media-link-done")
                    .disabled(disabled)
                    .on_press(move |_: &mut Window, cx: &mut App| {
                        this.update(cx, |state, cx| state.commit_link_draft(cx));
                    })
                    .child(MainText::new(tr("done")).px(16.0).color(color)),
            )
            .into_any_element()
    }

    fn link_customize_sheet(&self, this: Entity<Self>, cx: &App) -> AnyElement {
        let background = self.props.theme_store.read(cx).current_theme().bg._100;
        let content = this.clone();
        let sheet = BottomSheetUi::new(
            "media-editor-link-customize",
            Binding::entity(
                this,
                |state: &Self| state.customizing_link_id.is_some(),
                |state, value, cx| {
                    if !value {
                        state.customizing_link_id = None;
                        cx.notify();
                    }
                },
            ),
            move |_: &mut Window, cx: &mut App| Self::link_customize_content(content.clone(), cx),
            |_: &mut Window, _: &mut App| div(),
        )
        .snap_fractions(vec![0.6])
        .footer_shows_top_border(false)
        .content_scrollable(false)
        .fits_content(true)
        .sheet_background(Some(background));
        div().absolute().inset_0().child(sheet).into_any_element()
    }

    fn link_customize_content(this: Entity<Self>, cx: &mut App) -> AnyElement {
        let state = this.read(cx);
        let Some(id) = state.customizing_link_id else {
            return div().into_any_element();
        };
        let Some(link) = state.active_links().into_iter().find(|link| link.id == id) else {
            return div().into_any_element();
        };
        let theme = state.props.theme_store.read(cx).current_theme().clone();
        let mode_this = this.clone();
        let style_this = this;
        EditorLinkCustomizeContent::new(
            link,
            theme,
            move |mode: RiseMediaLinkPreviewMode, _: &mut Window, cx: &mut App| {
                mode_this.update(cx, |state, cx| state.set_link_preview_mode(mode, id, cx));
            },
            move |style: RiseMediaLinkStyle, _: &mut Window, cx: &mut App| {
                style_this.update(cx, |state, cx| state.set_link_style(style, id, cx));
            },
        )
        .into_any_element()
    }

    fn begin_creating_text(&mut self, cx: &mut Context<Self>) {
        self.text_draft = Some(EditorTextEntity {
            id: Uuid::new_v4(),
            text: String::new(),
            font_size: 0.06,
            color: self.brush_color,
            center: point(0.5, 0.5),
            rotation: 0.0,
            scale: 1.0,
            style_mode: EditorTextEntityStyleMode::Background,
            alignment: EditorTextEntityTextAlignment::Center,
        });
        if self.props.is_video {
            self.ensure_video_poster_for_edit_mode(cx);
        }
        self.text_opened_from_draw = self.mode == EditorMode::Draw;
        self.set_mode(EditorMode::Text { entity_id: None }, cx);
    }

    fn begin_editing_text(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.text_opened_from_draw = self.mode == EditorMode::Draw;
        let texts = if self.props.is_video {
            self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref()).map(|state| &state.texts)
        } else {
            self.image_state.as_ref().map(|state| &state.texts)
        };
        let Some(existing) = texts.and_then(|texts| texts.iter().find(|text| text.id == id)).cloned() else {
            return;
        };
        self.text_draft = Some(existing);
        self.set_mode(EditorMode::Text { entity_id: Some(id) }, cx);
    }

    fn cancel_text_draft(&mut self, cx: &mut Context<Self>) {
        self.text_draft = None;
        self.set_mode(self.text_return_mode(), cx);
    }

    /// close and done return to where the text was opened from
    fn text_return_mode(&self) -> EditorMode {
        if self.text_opened_from_draw { EditorMode::Draw } else { EditorMode::Initial }
    }

    /// edit shared by the image and video branch of commit_text_draft
    /// an edited text keeps its placement, a new one lands at the center
    fn apply_committed_text(
        texts: &mut Vec<EditorTextEntity>,
        edited_existing_id: Option<Uuid>,
        committed: Option<EditorTextEntity>,
    ) {
        if let Some(id) = edited_existing_id {
            if let Some(index) = texts.iter().position(|text| text.id == id) {
                match committed {
                    Some(mut updated) => {
                        updated.center = texts[index].center;
                        updated.rotation = texts[index].rotation;
                        updated.scale = texts[index].scale;
                        texts[index] = updated;
                    }
                    None => {
                        texts.remove(index);
                    }
                }
            }
        } else if let Some(mut added) = committed {
            added.center = point(0.5, 0.5);
            texts.push(added);
        }
    }

    fn commit_text_draft(&mut self, committed: Option<EditorTextEntity>, cx: &mut Context<Self>) {
        let edited_existing_id = match self.mode {
            EditorMode::Text { entity_id } => entity_id,
            _ => None,
        };
        if self.props.is_video {
            let spatial = self.video_state.as_ref().and_then(|state| state.spatial_edit.clone());
            let (Some(mut video_state), Some(mut spatial)) = (self.video_state.clone(), spatial) else {
                self.text_draft = None;
                self.set_mode(EditorMode::Initial, cx);
                return;
            };
            Self::apply_committed_text(&mut spatial.texts, edited_existing_id, committed);
            spatial.was_edited = true;
            video_state.spatial_edit = Some(spatial);
            video_state.was_edited = true;
            self.set_video_state(Some(video_state), cx);
            self.recompute_video_poster_preview(cx);
            self.text_draft = None;
            self.set_mode(self.text_return_mode(), cx);
            return;
        }
        let Some(mut state) = self.image_state.clone() else {
            self.text_draft = None;
            self.set_mode(EditorMode::Initial, cx);
            return;
        };
        Self::apply_committed_text(&mut state.texts, edited_existing_id, committed);
        state.was_edited = true;
        self.set_image_state(Some(state), cx);
        self.text_draft = None;
        self.set_mode(self.text_return_mode(), cx);
    }

    fn load_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let initial_payload = self.props.initial_edit.as_ref().map(|edit| edit.payload.clone());
        if self.props.is_video {
            if let Some(MediaSelectionEditPayload::Video(saved)) = initial_payload {
                let clamped = self.clamp_video_trim_state(saved.clone());
                self.video_trim_auto_clamped =
                    clamped.end_seconds - clamped.start_seconds < saved.end_seconds - saved.start_seconds;
                self.video_state = Some(clamped);
                if self.active_music_plate().is_some() {
                    self.sync_plate_to_selection(cx);
                }
            }
            if self.props.embed_in_hero_host && self.props.defers_source_load {
                cx.spawn_in(window, async move |this, cx| {
                    let delay = MediaViewerTransition::MOVE_DURATION + 0.12;
                    cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
                    let _ = this.update_in(cx, |state, window, cx| state.load_video_asset(window, cx));
                })
                .detach();
            } else {
                self.load_video_asset(window, cx);
            }
        } else {
            if let Some(MediaSelectionEditPayload::Image(saved)) = initial_payload {
                self.image_state = Some(saved);
                if self.active_music_plate().is_some() {
                    self.sync_plate_to_selection(cx);
                }
            }
            self.load_image(window, cx);
        }
        if let Some(plate) = self.active_music_plate() {
            EditorMusicPlateCoverStore::shared().prefetch(&plate.track_id, &plate.cover_url, None, cx);
        }
        cx.notify();
    }

    fn asset_local_id(&self) -> String {
        self.props.ph_asset.as_ref().map(|asset| asset.id.clone()).unwrap_or_else(|| self.props.media_asset.id.clone())
    }

    fn asset_pixel_size(&self) -> Size<f32> {
        if let Some(asset) = &self.props.ph_asset {
            return size(asset.pixel_width as f32, asset.pixel_height as f32);
        }
        if let Some(image) = &self.props.source_image_override {
            return image_pixel_size(image);
        }
        size(self.props.media_asset.pixel_width.max(1) as f32, self.props.media_asset.pixel_height.max(1) as f32)
    }

    fn load_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(image) = self.props.source_image_override.clone() {
            self.apply_loaded_image(image, true, cx);
            return;
        }
        if let Some(seed) = self.props.seed_hero_image.clone() {
            self.apply_loaded_image(seed, false, cx);
        }
        if self.props.embed_in_hero_host && self.props.defers_source_load {
            cx.spawn_in(window, async move |this, cx| {
                let delay = MediaViewerTransition::MOVE_DURATION + 0.08;
                cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
                let _ = this.update_in(cx, |state, window, cx| state.request_high_quality_image(window, cx));
            })
            .detach();
        } else {
            self.request_high_quality_image(window, cx);
        }
    }

    fn image_for_stage(&self, mode: EditorMode, preview: &CleverImage, state: &EditorImageState) -> CleverImage {
        if let Some(canvas) = &state.story_canvas {
            return self.canvas_stage_image.clone().unwrap_or_else(|| {
                StoryCanvasBackground::stage_image(&canvas.background_top, &canvas.background_bottom)
            });
        }
        let Some(original) = self.original_source_image.as_ref().filter(|_| mode == EditorMode::Crop) else {
            return preview.clone();
        };
        if let Some(image) = &self.crop_stage_image {
            return image.clone();
        }
        EditorAdjustmentsRenderer::shared().render(original, &state.adjustments)
    }

    /// renders the marked crop stage off the main thread, leaving crop drops it with its memory
    fn prepare_crop_stage(&mut self, new_mode: EditorMode, cx: &mut Context<Self>) {
        self.crop_stage_generation = self.crop_stage_generation.wrapping_add(1);
        self.crop_stage_image = None;
        self.crop_stage_task = None;
        if new_mode != EditorMode::Crop {
            return;
        }
        let (Some(state), Some(original)) = (self.image_state.clone(), self.original_source_image.clone()) else {
            return;
        };
        if !EditorImageBake::crop_stage_needs_marks(&state) {
            return;
        }
        let generation = self.crop_stage_generation;
        self.crop_stage_task = Some(cx.spawn(async move |this, cx| {
            let image = cx
                .background_spawn(async move { EditorImageBake::crop_stage_image(&original, &state) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.crop_stage_generation && this.mode == EditorMode::Crop {
                    this.crop_stage_image = Some(image);
                    cx.notify();
                }
            });
        }));
    }

    /// runs off the main thread, bake takes as long as the photo is large
    /// mode leaves crop once the baked canvas is in so the stage never shows the old one
    fn commit_crop_to_canvas(&mut self, cx: &mut Context<Self>) {
        let original = self.original_source_image.clone().or_else(|| self.source_image.clone());
        let (Some(state), Some(original)) = (self.image_state.clone(), original) else {
            self.set_mode(EditorMode::Initial, cx);
            return;
        };
        tracing::warn!(target: "riseonly::X2339", steps = state.rotation_steps, free = state.free_rotation_degrees, mirrored = state.is_mirrored, crop = ?state.crop_rect_relative, "crop commit photo");
        self.canvas_generation = self.canvas_generation.wrapping_add(1);
        self.canvas_task = None;
        self.crop_commit_task = Some(cx.spawn(async move |this, cx| {
            let committed = cx
                .background_spawn(async move { EditorImageBake::commit_canvas(&original, &state) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match committed {
                    Some((baked, new_state)) => {
                        tracing::warn!(target: "riseonly::X2339", steps = new_state.rotation_steps, free = new_state.free_rotation_degrees, w = baked.size(0).width.0, h = baked.size(0).height.0, "crop commit photo baked");
                        this.source_image = Some(baked);
                        this.set_image_state(Some(new_state), cx);
                        this.recompute_preview(cx);
                    }
                    None => this.recompute_preview(cx),
                }
                this.set_mode(EditorMode::Initial, cx);
            });
        }));
    }

    fn apply_loaded_image(&mut self, image: CleverImage, is_final_source: bool, cx: &mut Context<Self>) {
        if is_final_source || self.original_source_image.is_none() {
            self.original_source_image = Some(image.clone());
        }
        if self.image_state.is_none() {
            self.image_state = Some(EditorImageState::empty(Some(self.asset_local_id()), self.asset_pixel_size()));
        }
        self.restore_canvas_from_original(image.clone(), cx);
        if is_final_source {
            self.set_up_story_canvas_if_needed(&image, cx);
        }
        self.is_loading = false;
        cx.notify();
    }

    fn set_up_story_canvas_if_needed(&mut self, first_photo: &CleverImage, cx: &mut Context<Self>) {
        if self.props.is_video {
            return;
        }
        let Some(mut state) = self.image_state.clone() else {
            return;
        };
        if state.story_canvas.is_none() {
            if !self.props.configuration.uses_story_canvas {
                return;
            }
            state.story_canvas = Some(EditorStoryCanvas::make(first_photo));
            state.synchronize_edit_flag();
            self.set_image_state(Some(state.clone()), cx);
        }
        if let Some(canvas) = &state.story_canvas {
            self.canvas_stage_image =
                Some(StoryCanvasBackground::stage_image(&canvas.background_top, &canvas.background_bottom));
            cx.notify();
        }
    }

    fn restore_canvas_from_original(&mut self, ph_image: CleverImage, cx: &mut Context<Self>) {
        let needs_canvas = self.image_state.clone().filter(Self::state_needs_rendered_canvas);
        let Some(state) = needs_canvas else {
            self.source_image = Some(ph_image.clone());
            self.rendered_preview = Some(ph_image);
            self.recompute_preview(cx);
            return;
        };
        let mut background_state = state;
        background_state.strokes = Vec::new();
        background_state.texts = Vec::new();
        background_state.stickers = None;
        background_state.links = None;
        background_state.story_canvas = None;
        background_state.adjustments = EditorImageAdjustments::default();
        self.canvas_generation = self.canvas_generation.wrapping_add(1);
        let generation = self.canvas_generation;
        self.canvas_task = Some(cx.spawn(async move |this, cx| {
            let canvas = cx
                .background_spawn(async move { EditorImageBake::render_edited_image(&ph_image, &background_state) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.canvas_generation {
                    return;
                }
                this.source_image = Some(canvas.clone());
                this.rendered_preview = Some(canvas);
                this.recompute_preview(cx);
                cx.notify();
            });
        }));
    }

    fn state_needs_rendered_canvas(state: &EditorImageState) -> bool {
        state.crop_baked_into_canvas
            || !state.strokes.is_empty()
            || !state.texts.is_empty()
            || !state.adjustments.is_identity()
            || !EditorCropAutoFitPlanner::is_effectively_full(state.crop_rect_relative)
            || state.rotation_steps % 4 != 0
            || state.is_mirrored
            || state.free_rotation_degrees.abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES
    }

    fn sync_navigation_pop_guard(&self, cx: &mut App) {
        let Some(presentation) = &self.props.editor_presentation else {
            return;
        };
        let blocks = presentation.read(cx).blocks_navigation_pop();
        NavigationInteractivePopGuard::set_media_editor_editing(blocks, cx);
    }

    fn auto_select_if_edited(&mut self, cx: &mut Context<Self>) {
        if self.user_deselected(cx) {
            return;
        }
        if self.props.is_video {
            if !self.video_state.as_ref().is_some_and(EditorVideoState::has_meaningful_edits) {
                return;
            }
        } else if !self.image_state.as_ref().is_some_and(EditorImageState::has_user_edits) {
            return;
        }
        if let Some(asset) = self.props.ph_asset.clone() {
            self.props.selection.update(cx, |selection, cx| {
                let _ = selection.ensure_selected(&asset, cx);
            });
        }
    }

    fn persist_current_edit_to_selection(&mut self, cx: &mut Context<Self>) {
        let user_deselected = self.user_deselected(cx);
        let asset = self.props.ph_asset.clone().filter(|_| !user_deselected);
        let asset_id = self.asset_local_id();
        let selection = self.props.selection.clone();
        let payload = if self.props.is_video {
            let Some(mut video_state) = self.video_state.clone() else {
                return;
            };
            if video_state.has_meaningful_edits() {
                video_state.was_edited = true;
            }
            MediaSelectionEditPayload::Video(video_state)
        } else {
            let Some(mut state) = self.image_state.clone() else {
                return;
            };
            Self::sync_was_edited(&mut state);
            self.image_state = Some(state.clone());
            if !state.has_user_edits() && state.music_plate.is_none() {
                selection.update(cx, |selection, cx| selection.remove_edit(&asset_id, cx));
                return;
            }
            MediaSelectionEditPayload::Image(state)
        };
        let edit = MediaSelectionEdit { payload };
        selection.update(cx, |selection, cx| match &asset {
            Some(asset) => selection.set_edit_for_asset(edit, asset, cx),
            None => selection.set_edit(edit, &asset_id, cx),
        });
        if !self.props.is_video {
            self.refresh_edit_preview_thumbnail(cx);
        }
    }

    fn sync_was_edited(state: &mut EditorImageState) {
        state.synchronize_edit_flag();
    }

    fn refresh_edit_preview_thumbnail(&self, cx: &mut Context<Self>) {
        let asset_id = self.props.media_asset.id.clone();
        let edited = self.image_state.clone().filter(|state| state.was_edited && state.has_user_edits());
        let (Some(snapshot), Some(original)) = (edited, self.original_source_image.clone()) else {
            EditorEditPreviewCache::remove(&asset_id, cx);
            return;
        };
        cx.spawn(async move |_, cx| {
            let thumb = cx
                .background_spawn(async move {
                    EditorImageBake::thumbnail_preview(
                        &original,
                        &snapshot,
                        EditorImageBake::THUMBNAIL_PREVIEW_MAX_PIXEL_SIDE,
                    )
                })
                .await;
            let _ = cx.update(|cx| EditorEditPreviewCache::store(Some(thumb), &asset_id, cx));
        })
        .detach();
    }

    fn dismiss_to_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_current_edit_to_selection(cx);
        if self.props.embed_in_hero_host || self.props.standalone_dismiss_cancels {
            (self.props.on_cancel.clone())(window, cx);
        } else {
            self.commit(false, window, cx);
        }
    }

    fn request_high_quality_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self.props.ph_asset.clone() else {
            self.is_loading = false;
            cx.notify();
            return;
        };
        let target = size(2400.0_f32.min(asset.pixel_width as f32), 2400.0_f32.min(asset.pixel_height as f32));
        let (sender, mut deliveries) = mpsc::unbounded::<PhotoImageDelivery>();
        let _ = PhotosImageRequest::request_image(
            &asset.id,
            target,
            ImageContentMode::AspectFit,
            ImageDeliveryMode::HighQuality,
            true,
            move |delivery| {
                let _ = sender.unbounded_send(delivery);
            },
        );
        self.image_request_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(delivery) = deliveries.next().await {
                if this.update_in(cx, |state, window, cx| state.handle_image_delivery(delivery, window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn handle_image_delivery(&mut self, delivery: PhotoImageDelivery, window: &mut Window, cx: &mut Context<Self>) {
        let Some(image) = delivery.image else {
            if delivery.is_degraded || self.source_image.is_some() {
                return;
            }
            self.is_loading = false;
            cx.notify();
            if !delivery.is_cancelled {
                Notifier::send(
                    tr("media_picker_photo_load_failed"),
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
            (self.props.on_cancel.clone())(window, cx);
            return;
        };
        if delivery.is_degraded {
            if self.source_image.is_none() {
                self.apply_loaded_image(image, false, cx);
            }
            return;
        }
        self.apply_loaded_image(image, true, cx);
    }

    /// off the main thread because a slider drag asks for this on every step
    /// only the newest request may land
    fn recompute_preview(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.image_state.clone() else {
            return;
        };
        let original = self.original_source_image.clone();
        let source = self.source_image.clone();
        if original.is_none() && source.is_none() {
            return;
        }
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_spawn(async move {
                    if let Some(original) = original {
                        let mut background = state;
                        background.strokes = Vec::new();
                        background.texts = Vec::new();
                        background.stickers = None;
                        background.links = None;
                        background.story_canvas = None;
                        Some(EditorImageBake::render_edited_image(&original, &background))
                    } else {
                        source.map(|source| EditorAdjustmentsRenderer::shared().render(&source, &state.adjustments))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.preview_generation
                    && let Some(rendered) = rendered
                {
                    this.rendered_preview = Some(rendered);
                    cx.notify();
                }
            });
        }));
    }

    fn commit(&mut self, immediate: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.video_playback.stop_music_preview(cx);
        let selects_asset = !self.user_deselected(cx) || self.props.selection.read(cx).is_empty();
        tracing::warn!(target: "riseonly::X908", immediate, selects_asset, video = self.props.is_video, has_image = self.image_state.is_some(), has_video = self.video_state.is_some(), "editor commit");
        let on_done = self.props.on_done.clone();
        if self.props.is_video
            && let Some(mut video_state) = self.video_state.clone()
        {
            if let Some(spatial) = video_state.spatial_edit.as_mut() {
                spatial.synchronize_edit_flag();
            }
            if self.video_trim_auto_clamped && self.props.configuration.untrimmed_long_video_message_key.is_some() {
                video_state.start_seconds = 0.0;
                video_state.end_seconds = video_state.duration;
            }
            video_state.was_edited = video_state.has_meaningful_edits();
            on_done(
                MediaEditorOutcome {
                    edit: MediaSelectionEdit { payload: MediaSelectionEditPayload::Video(video_state) },
                    immediate_send: immediate,
                    selects_asset,
                },
                window,
                cx,
            );
        } else if !self.props.is_video
            && let Some(mut image_state) = self.image_state.clone()
        {
            image_state.synchronize_edit_flag();
            on_done(
                MediaEditorOutcome {
                    edit: MediaSelectionEdit { payload: MediaSelectionEditPayload::Image(image_state) },
                    immediate_send: immediate,
                    selects_asset,
                },
                window,
                cx,
            );
        } else {
            (self.props.on_cancel.clone())(window, cx);
        }
    }

    fn video_state_binding(this: Entity<Self>) -> Binding<EditorVideoState> {
        Binding::entity(
            this,
            |state: &Self| state.video_state.clone().unwrap_or_else(|| EditorVideoState::empty(None, 0.0)),
            |state, value, cx| state.set_video_state(Some(value), cx),
        )
    }

    fn video_spatial_binding(this: Entity<Self>) -> Binding<EditorImageState> {
        Binding::entity(
            this,
            |state: &Self| {
                state
                    .video_state
                    .as_ref()
                    .and_then(|video_state| video_state.spatial_edit.clone())
                    .unwrap_or_else(|| EditorImageState::empty(Some(state.asset_local_id()), state.asset_pixel_size()))
            },
            |state, new_value, cx| {
                let Some(mut video_state) = state.video_state.clone() else {
                    return;
                };
                video_state.spatial_edit = Some(new_value);
                state.set_video_state(Some(video_state), cx);
                state.recompute_video_poster_preview(cx);
            },
        )
    }

    fn video_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(asset) = self.loaded_av_asset.clone().filter(|_| self.video_state.is_some()) else {
            return if self.is_loading { Self::spinner() } else { div().into_any_element() };
        };
        let this = cx.entity();
        let geo = self.stage_bounds.size;
        let safe_insets = self.stage_safe_insets(window, cx);
        let spatial = self.video_state.as_ref().and_then(|state| state.spatial_edit.clone());
        let preview = self.video_poster_rendered.clone().or_else(|| self.video_poster_original.clone());
        let stage_image = self.video_stage_image(self.mode, preview.as_ref(), spatial.as_ref());
        let stage_display_size = stage_image.as_ref().map_or(geo, |image| {
            EditorImageLayout::oriented_display_size(
                image_pixel_size(image),
                if self.mode == EditorMode::Crop { spatial.as_ref().map_or(0, |state| state.rotation_steps) } else { 0 },
                0.0,
            )
        });
        let insets = EditorImageLayout::compute_insets(
            geo,
            stage_display_size,
            safe_insets.top,
            safe_insets.bottom,
            safe_insets.left,
            safe_insets.right,
            self.mode,
            self.resolved_toolbar_height(),
        );

        let initial = self.mode == EditorMode::Initial;
        let player = if initial {
            let plate = this.clone();
            let plate_delete = this.clone();
            let plate_press = this.clone();
            let ready = this.clone();
            let tags = this.clone();
            let is_playing = Binding::entity(
                this.clone(),
                |state: &Self| state.video_is_playing,
                |state, value, _| state.video_is_playing = value,
            );
            let center_bottom = if self.video_playback_chrome_top > 1.0 {
                (geo.height - self.video_playback_chrome_top).max(0.0)
            } else {
                self.resolved_toolbar_height()
            };
            let background_tap: Option<ButtonAction> = self.props.shows_picker_top_bar.then(|| {
                let tapped = this.clone();
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    tapped.update(cx, |state, cx| state.handle_canvas_tap(cx));
                }) as ButtonAction
            });
            let mask = if self.allows_page_drag() || self.allows_video_dismiss_drag() {
                GestureMask::All
            } else {
                GestureMask::Subviews
            };
            let player = EditorVideoTrimView::new(
                "media-editor-video-trim",
                asset.clone(),
                Self::video_state_binding(this.clone()),
                is_playing,
                self.video_playback.clone(),
                true,
            )
            .center_play_insets(EdgeInsets::new(safe_insets.top, 0.0, center_bottom, 0.0))
            .plate(if self.show_track_trim { None } else { self.active_music_plate() })
            .on_plate_update(move |updated, cx| plate.update(cx, |state, cx| state.set_music_plate(Some(updated), cx)))
            .on_plate_delete(move |cx| plate_delete.update(cx, |state, cx| state.set_music_plate(None, cx)))
            .on_plate_press_changed(move |pressed, cx| {
                plate_press.update(cx, |state, _| state.video_plate_pressed = pressed);
            })
            .person_tags(self.active_person_tags())
            .on_person_tags_update(move |updated, cx| tags.update(cx, |state, cx| state.set_person_tags(updated, cx)))
            .on_background_tap(background_tap)
            .on_ready_for_display(move |cx: &mut App| ready.update(cx, |state, cx| state.set_video_player_ready(cx)));
            let player = div().w(px(geo.width)).h(px(geo.height)).child(player).simultaneous_gesture(
                "media-editor-video-page-drag",
                Self::video_page_drag_gesture(this.clone()),
                mask,
            );
            Some(self.stage_transform(player, true))
        } else {
            None
        };
        let cover = if initial { self.video_canvas_cover.frame_value(RiseClock::monotonic(), window) } else { 1.0 };
        let canvas = if (!initial || cover > 0.001)
            && let Some(stage_image) = stage_image.filter(|_| preview.is_some())
        {
            let stage = self.editor_stage(
                this.clone(),
                stage_image,
                Self::video_spatial_binding(this.clone()),
                insets,
                true,
            );
            Some(div().absolute().inset_0().opacity(cover).child(self.stage_transform(stage, false)).into_any_element())
        } else if !initial && self.is_loading_video_poster {
            Some(Self::spinner())
        } else {
            None
        };

        let text_rect =
            if self.video_image_rect.rect.size.width > 1.0 { self.video_image_rect.rect } else { self.image_rect.rect };
        let chrome = self.chrome_children(
            EditorChromeInput {
                safe_insets,
                brush_rect: self.video_image_rect.rect,
                text_rect,
                asset: Some(asset),
            },
            window,
            cx,
        );
        div()
            .absolute()
            .inset_0()
            .children(player)
            .children(canvas)
            .child(self.chrome_layer(chrome, window))
            .into_any_element()
    }

    fn set_video_player_ready(&mut self, cx: &mut Context<Self>) {
        if self.video_player_ready {
            return;
        }
        self.video_player_ready = true;
        self.video_canvas_cover.set_in(0.0, spring(0.34, 0.84), RiseClock::monotonic(), cx);
        cx.notify();
    }

    /// a player that never reports a frame must not stay under the canvas
    fn start_video_ready_fallback(&mut self, cx: &mut Context<Self>) {
        self.video_ready_fallback = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = this.update(cx, |state, cx| state.set_video_player_ready(cx));
        }));
    }

    fn video_playback_chrome(&self, this: Entity<Self>, asset: AVURLAsset, cx: &App) -> gpui::Div {
        let open_volume = this.clone();
        let started = this.clone();
        let committed = this.clone();
        let request_volume = this.clone();
        let measured = this.clone();
        let is_muted = self.video_state.as_ref().is_some_and(|state| state.is_muted);
        let attached = self.attached_track(cx).filter(|_| self.video_state.is_some());
        let lane = attached.map(|attached| {
            let duration = self.video_state.as_ref().map_or(0.0, |state| state.duration);
            let started = this.clone();
            let committed = this.clone();
            let request_volume = this.clone();
            div().w_full().h(px(40.0)).occlude().child(EditorMusicLaneRepresentable::new(
                "media-editor-music-lane",
                attached.track,
                duration.max(0.1),
                Self::music_offset_binding(this.clone()),
                Self::music_start_binding(self.props.selection.clone()),
                Self::music_end_binding(self.props.selection.clone()),
                self.props.theme_store.read(cx).current_theme().primary._100,
                self.video_playback.clone(),
                1_000,
                move |_, cx| {
                    started.update(cx, |state, cx| {
                        state.video_is_playing = false;
                        cx.notify();
                    });
                },
                move |_, cx| {
                    committed.update(cx, |state, cx| {
                        state.music_segment_pinned = true;
                        state.apply_music_preview(cx);
                    });
                },
                move |_, cx| {
                    request_volume.update(cx, |state, cx| {
                        state.show_sound_sheet = true;
                        cx.notify();
                    });
                },
            ))
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.0))
            .px(px(14.0))
            .pb(px(8.0))
            .child(EditorVideoVolumeButton::new(is_muted, move |_, cx| {
                open_volume.update(cx, |state, cx| {
                    state.show_sound_sheet = true;
                    cx.notify();
                });
            }))
            .child(
                div().w_full().h(px(64.0)).occlude().child(
                    EditorVideoTimelineRepresentable::new(
                        "media-editor-video-timeline",
                        asset,
                        Self::video_state_binding(this),
                        self.video_playback.clone(),
                        move |cx| {
                            started.update(cx, |state, cx| {
                                state.video_resumes_after_trim = state.video_is_playing;
                                state.video_is_playing = false;
                                cx.notify();
                            });
                        },
                        move |new_state, cx| {
                            committed.update(cx, |state, cx| state.video_trim_committed(new_state, cx));
                        },
                    )
                    .max_trim_duration_seconds(self.max_video_trim_duration())
                    .on_request_volume(move |cx| {
                        request_volume.update(cx, |state, cx| {
                            state.show_sound_sheet = true;
                            cx.notify();
                        });
                    }),
                ),
            )
            .children(lane)
            .child(measure("media-editor-video-chrome-frame", move |bounds, _, cx| {
                measured.update(cx, |state, cx| {
                    let top = pixels_bounds(bounds).origin.y
                        - state.stage_bounds.origin.y
                        - *state.swipe_down_offset.target();
                    if state.video_playback_chrome_top != top {
                        state.video_playback_chrome_top = top;
                        cx.notify();
                    }
                });
            }))
    }

    fn video_trim_committed(&mut self, new_state: EditorVideoState, cx: &mut Context<Self>) {
        let resumes = self.video_resumes_after_trim || self.video_is_playing;
        self.video_resumes_after_trim = false;
        self.video_trim_auto_clamped = false;
        let clamped = self.clamp_video_trim_state(new_state);
        self.set_video_state(Some(clamped.clone()), cx);
        self.video_playback.video_window.set((clamped.start_seconds, clamped.end_seconds));
        self.video_playback.source_duration_seconds.set(clamped.duration);
        self.sync_music_segment_to_video_window(cx);
        self.apply_music_preview(cx);
        self.video_playback.request_seek(clamped.start_seconds, !resumes, cx);
        if resumes {
            self.video_is_playing = true;
        }
        cx.notify();
    }

    fn music_preview_signature(&self, cx: &App) -> String {
        if !self.props.is_video {
            return String::new();
        }
        self.props
            .selection
            .read(cx)
            .attached_track
            .as_ref()
            .and_then(|attached| attached.track.id.clone())
            .unwrap_or_default()
    }

    fn refresh_music_preview(&mut self, cx: &mut Context<Self>) {
        let attached = self.attached_track(cx).filter(|_| self.props.is_video);
        let Some(attached) = attached else {
            self.music_preview_url = None;
            self.video_playback.set_music_preview(None, cx);
            return;
        };
        self.music_preview_url = MusicTrackWaveform::local_file_url(&attached.track).or_else(|| {
            RemoteMediaURL::optional_normalized(attached.track.audio_url.as_deref())
                .and_then(|url| Url::from_string(&url))
        });
        self.apply_music_preview(cx);
        self.update_music_preview_playback(cx);
    }

    fn apply_music_preview(&mut self, cx: &mut Context<Self>) {
        let attached = self.attached_track(cx).filter(|_| self.props.is_video);
        let (Some(url), Some(attached)) = (self.music_preview_url.clone(), attached) else {
            self.video_playback.set_music_preview(None, cx);
            return;
        };
        let volume = self.video_state.as_ref().map_or(1.0, EditorVideoState::effective_music_volume);
        let offset_seconds = self.video_state.as_ref().map_or(0.0, |state| state.music_offset_seconds);
        self.video_playback.set_music_preview(
            Some(EditorVideoPlaybackControllerMusicPreview {
                session_id: MUSIC_PREVIEW_SESSION_ID.to_owned(),
                url,
                track_id: attached.track.id.clone(),
                start_ms: attached.start_ms,
                end_ms: attached.end_ms,
                volume: volume as f32,
                offset_seconds,
            }),
            cx,
        );
    }

    fn update_music_preview_playback(&self, cx: &mut Context<Self>) {
        self.video_playback.set_preview_playing(self.video_is_playing && !self.show_track_trim, cx);
    }

    fn music_start_binding(selection: Entity<MediaSelectionState>) -> Binding<i64> {
        let reader = selection.clone();
        Binding::new(
            move |cx| reader.read(cx).attached_track.as_ref().map_or(0, |attached| attached.start_ms),
            move |new_value, _, cx| {
                selection.update(cx, |selection, cx| {
                    if let Some(attached) = selection.attached_track.as_mut() {
                        attached.start_ms = new_value;
                        cx.notify();
                    }
                });
            },
        )
    }

    fn music_end_binding(selection: Entity<MediaSelectionState>) -> Binding<i64> {
        let reader = selection.clone();
        Binding::new(
            move |cx| reader.read(cx).attached_track.as_ref().map_or(0, |attached| attached.end_ms),
            move |new_value, _, cx| {
                selection.update(cx, |selection, cx| {
                    if let Some(attached) = selection.attached_track.as_mut() {
                        attached.end_ms = new_value;
                        cx.notify();
                    }
                });
            },
        )
    }

    fn video_volume_binding(this: Entity<Self>) -> Binding<f64> {
        Binding::entity(
            this,
            |state: &Self| state.video_state.as_ref().map_or(1.0, EditorVideoState::effective_video_volume),
            |state, new_value, cx| {
                let Some(mut video_state) = state.video_state.clone() else {
                    return;
                };
                video_state.video_volume = new_value;
                video_state.is_muted = new_value <= 0.0001;
                video_state.was_edited = true;
                state.set_video_state(Some(video_state), cx);
            },
        )
    }

    fn music_volume_binding(this: Entity<Self>) -> Binding<f64> {
        Binding::entity(
            this,
            |state: &Self| state.video_state.as_ref().map_or(1.0, |video_state| video_state.music_volume),
            |state, new_value, cx| {
                let Some(mut video_state) = state.video_state.clone() else {
                    return;
                };
                video_state.music_volume = new_value;
                video_state.was_edited = true;
                state.set_video_state(Some(video_state), cx);
            },
        )
    }

    fn music_offset_binding(this: Entity<Self>) -> Binding<f64> {
        Binding::entity(
            this,
            |state: &Self| state.video_state.as_ref().map_or(0.0, |video_state| video_state.music_offset_seconds),
            |state, new_value, cx| {
                let Some(mut video_state) = state.video_state.clone() else {
                    return;
                };
                video_state.music_offset_seconds = new_value;
                video_state.was_edited = true;
                state.set_video_state(Some(video_state), cx);
            },
        )
    }

    fn sync_music_segment_to_video_window(&mut self, cx: &mut Context<Self>) {
        if self.music_segment_pinned {
            return;
        }
        let (Some(mut attached), Some(video_state)) = (self.attached_track(cx), self.video_state.clone()) else {
            return;
        };
        let window_ms = ((video_state.end_seconds - video_state.start_seconds).max(0.0) * 1000.0).round() as i64;
        if window_ms <= 0 {
            return;
        }
        let duration = attached.track.duration_ms.unwrap_or(0);
        let mut length = window_ms.max(1_000);
        let mut start = attached.start_ms;
        if duration > 0 {
            length = length.min(duration);
            start = start.min(duration - length).max(0);
        }
        attached.start_ms = start;
        attached.end_ms = start + length;
        self.set_attached_track(Some(attached), cx);
        if (video_state.music_offset_seconds - video_state.start_seconds).abs() > 0.01 {
            let mut video_state = video_state;
            video_state.music_offset_seconds = video_state.start_seconds;
            self.set_video_state(Some(video_state), cx);
        }
    }

    fn video_stage_image(
        &self,
        mode: EditorMode,
        preview: Option<&CleverImage>,
        spatial: Option<&EditorImageState>,
    ) -> Option<CleverImage> {
        let preview = preview?;
        let Some(original) = self.video_poster_original.as_ref().filter(|_| mode == EditorMode::Crop) else {
            return Some(preview.clone());
        };
        let adjustments = spatial.map(|state| state.adjustments).unwrap_or_default();
        Some(EditorAdjustmentsRenderer::shared().render(original, &adjustments))
    }

    fn ensure_video_poster_for_edit_mode(&mut self, cx: &mut Context<Self>) {
        if self.video_poster_original.is_some() || self.is_loading_video_poster {
            return;
        }
        let (Some(asset), Some(video_state)) = (self.loaded_av_asset.clone(), self.video_state.clone()) else {
            return;
        };
        let Some(source) = asset.url().to_file_path() else {
            return;
        };
        self.is_loading_video_poster = true;
        cx.notify();
        let midpoint = (video_state.start_seconds + video_state.end_seconds) * 0.5;
        cx.spawn(async move |this, cx| {
            let image = cx.background_spawn(async move { copy_poster_image(&source, midpoint, 1920) }).await;
            let _ = this.update(cx, |state, cx| {
                state.is_loading_video_poster = false;
                cx.notify();
                let Some(image) = image else {
                    return;
                };
                state.video_poster_original = Some(image.clone());
                if let Some(mut current) = state.video_state.clone()
                    && current.spatial_edit.is_none()
                {
                    current.spatial_edit =
                        Some(EditorImageState::empty(Some(state.asset_local_id()), state.asset_pixel_size()));
                    state.set_video_state(Some(current), cx);
                }
                state.restore_video_poster_canvas(image, cx);
            });
        })
        .detach();
    }

    fn restore_video_poster_canvas(&mut self, base: CleverImage, cx: &mut Context<Self>) {
        let spatial = self.video_state.as_ref().and_then(|state| state.spatial_edit.as_ref());
        if !spatial.is_some_and(Self::state_needs_rendered_canvas) {
            self.video_poster_rendered = Some(base);
            cx.notify();
            return;
        }
        self.recompute_video_poster_preview(cx);
    }

    fn recompute_video_poster_preview(&mut self, cx: &mut Context<Self>) {
        let spatial = self.video_state.as_ref().and_then(|state| state.spatial_edit.clone());
        let (Some(original), Some(spatial)) = (self.video_poster_original.clone(), spatial) else {
            return;
        };
        self.poster_generation = self.poster_generation.wrapping_add(1);
        let generation = self.poster_generation;
        self.poster_task = Some(cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_spawn(async move {
                    let mut background = spatial;
                    background.strokes = Vec::new();
                    background.texts = Vec::new();
                    background.stickers = None;
                    EditorImageBake::render_edited_image(&original, &background)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation == this.poster_generation {
                    this.video_poster_rendered = Some(rendered);
                    cx.notify();
                }
            });
        }));
    }

    /// off the main thread, same reason as commit_crop_to_canvas
    fn commit_video_spatial_crop(&mut self, cx: &mut Context<Self>) {
        let spatial = self.video_state.as_ref().and_then(|state| state.spatial_edit.clone());
        let (Some(spatial), Some(original)) = (spatial, self.video_poster_original.clone()) else {
            self.set_mode(EditorMode::Initial, cx);
            return;
        };
        tracing::warn!(target: "riseonly::X2339", steps = spatial.rotation_steps, free = spatial.free_rotation_degrees, mirrored = spatial.is_mirrored, crop = ?spatial.crop_rect_relative, "crop commit video");
        self.poster_generation = self.poster_generation.wrapping_add(1);
        self.crop_commit_task = Some(cx.spawn(async move |this, cx| {
            let committed = cx
                .background_spawn(async move { EditorImageBake::commit_canvas(&original, &spatial) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some((baked, mut new_state)) = committed {
                    tracing::warn!(target: "riseonly::X2339", steps = new_state.rotation_steps, free = new_state.free_rotation_degrees, "crop commit video baked");
                    this.video_poster_rendered = Some(baked);
                    new_state.was_edited = true;
                    if let Some(mut video_state) = this.video_state.clone() {
                        video_state.spatial_edit = Some(new_state);
                        video_state.was_edited = true;
                        this.set_video_state(Some(video_state), cx);
                    }
                }
                this.recompute_video_poster_preview(cx);
                this.set_mode(EditorMode::Initial, cx);
            });
        }));
    }

    fn load_video_asset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.props.source_video_url_override.clone() {
            let is_regular_file = url.is_file_url()
                && url
                    .to_file_path()
                    .and_then(|path| std::fs::symlink_metadata(path).ok())
                    .is_some_and(|metadata| metadata.is_file());
            if !is_regular_file {
                self.is_loading = false;
                cx.notify();
                return;
            }
            let source_asset_id = self.props.media_asset.id.clone();
            let fallback_duration = self.props.media_asset.duration;
            self.apply_loaded_video_asset(AVURLAsset::new(url), source_asset_id, fallback_duration, cx);
            return;
        }

        let Some(asset) = self.props.ph_asset.clone() else {
            return;
        };
        let (sender, receiver) = oneshot::channel::<ExportOutcome>();
        let _ = PhotosImageRequest::request_av_asset(&asset.id, &temporary_directory(), move |outcome| {
            let _ = sender.send(outcome);
        });
        self.video_request_task = Some(cx.spawn_in(window, async move |this, cx| {
            let Ok(outcome) = receiver.await else {
                return;
            };
            let _ = this.update(cx, |state, cx| {
                if let ExportOutcome::Exported(path) = outcome
                    && let Some(url) = Url::from_file_path(&path)
                {
                    state.apply_loaded_video_asset(AVURLAsset::new(url), asset.id.clone(), asset.duration_secs, cx);
                }
                state.is_loading = state.loaded_av_asset.is_none();
                cx.notify();
            });
        }));
    }

    fn apply_loaded_video_asset(
        &mut self,
        asset: AVURLAsset,
        source_asset_id: String,
        fallback_duration: f64,
        cx: &mut Context<Self>,
    ) {
        if self.loaded_av_asset.is_none() && self.mode == EditorMode::Initial {
            self.video_is_playing = true;
        }
        self.loaded_av_asset = Some(asset.clone());
        self.is_loading = false;
        cx.notify();
        if self.video_state.is_some() {
            self.video_state_did_load(cx);
            return;
        }
        let source = asset.url().to_file_path();
        cx.spawn(async move |this, cx| {
            let loaded_duration = cx
                .background_spawn(async move {
                    let source = source?;
                    if WebMStickerFile::is_web_m_path(&source) {
                        return WebMStickerFile::track(&source).map(|track| WebMStickerFile::duration_seconds(&track));
                    }
                    let probe = rise_media::video_export::probe(&source).ok()?;
                    Some(probe.duration_ms() as f64 / 1000.0)
                })
                .await
                .filter(|duration: &f64| duration.is_finite() && *duration > 0.0);
            let _ = this.update(cx, |state, cx| {
                if state.video_state.is_none() {
                    let duration = loaded_duration.unwrap_or_else(|| fallback_duration.max(0.0));
                    let fresh = EditorVideoState::empty(Some(source_asset_id), duration);
                    let clamped = state.clamp_video_trim_state(fresh.clone());
                    state.video_trim_auto_clamped =
                        clamped.end_seconds - clamped.start_seconds < fresh.end_seconds - fresh.start_seconds;
                    state.video_state = Some(clamped);
                }
                state.video_state_did_load(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn video_state_did_load(&mut self, cx: &mut Context<Self>) {
        if self.mode == EditorMode::Initial && !self.video_player_ready && self.video_ready_fallback.is_none() {
            self.start_video_ready_fallback(cx);
        }
        if self.video_state.as_ref().is_some_and(|state| state.spatial_edit.is_some()) {
            self.ensure_video_poster_for_edit_mode(cx);
        }
    }

    fn max_video_trim_duration(&self) -> Option<f64> {
        self.props.configuration.max_video_duration_seconds.filter(|seconds| *seconds > 0).map(|seconds| seconds as f64)
    }

    fn clamp_video_trim_state(&self, state: EditorVideoState) -> EditorVideoState {
        let Some(max_trim) = self.max_video_trim_duration().filter(|max_trim| *max_trim > 0.0) else {
            return state;
        };
        let mut clamped = state;
        if clamped.end_seconds - clamped.start_seconds > max_trim {
            clamped.end_seconds = clamped.duration.min(clamped.start_seconds + max_trim);
        }
        if clamped.end_seconds - clamped.start_seconds > max_trim {
            clamped.start_seconds = (clamped.end_seconds - max_trim).max(0.0);
        }
        clamped
    }
}

impl Render for MediaEditorScreenState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let now = RiseClock::monotonic();

        if self.crop_panel.is_some() && !self.chrome.crop_panel.is_present() && !self.chrome.crop_panel.is_leaving(now) {
            self.crop_panel = None;
        }

        let changed = this.clone();
        on_change("media-editor-mode", self.mode, false, window, cx, move |_, new_mode, window, cx| {
            let new_mode = *new_mode;
            changed.update(cx, |state, cx| state.on_mode_changed(new_mode, window, cx));
            if new_mode == EditorMode::Crop && changed.read(cx).crop_panel.is_none() {
                let panel = MediaEditorScreenState::make_crop_panel(changed.clone(), window, cx);
                changed.update(cx, |state, cx| {
                    state.crop_panel = Some(panel);
                    cx.notify();
                });
            }
        });
        let changed = this.clone();
        on_change(
            "media-editor-text-draft",
            self.text_draft.is_some(),
            false,
            window,
            cx,
            move |_, _, _, cx| changed.update(cx, |state, cx| state.on_text_draft_changed(cx)),
        );
        let changed = this.clone();
        on_change("media-editor-track-trim", self.show_track_trim, false, window, cx, move |_, _, _, cx| {
            changed.update(cx, |state, cx| state.on_show_track_trim_changed(cx));
        });

        let signature = self.music_preview_signature(cx);
        let volume = self.video_state.as_ref().map_or(1.0, |state| state.music_volume);
        let (track_changed, volume_changed, playback_changed, background) =
            (this.downgrade(), this.clone(), this.clone(), this.clone());
        let layered_root = self.layered_root(window, cx);
        let observed = EditorMusicPreviewObservers::new(
            "media-editor-music-preview",
            signature,
            volume,
            self.video_is_playing,
            self.show_track_trim,
            move |mut cx| {
                let this = track_changed.clone();
                async move {
                    let _ = this.update(&mut cx, |state, cx| state.refresh_music_preview(cx));
                }
                .boxed_local()
            },
            move |_, cx| volume_changed.update(cx, |state, cx| state.apply_music_preview(cx)),
            move |_, cx| playback_changed.update(cx, |state, cx| state.update_music_preview_playback(cx)),
            move |_, cx| {
                background.update(cx, |state, cx| {
                    state.video_is_playing = false;
                    state.video_playback.set_preview_playing(false, cx);
                    cx.notify();
                });
            },
            layered_root,
        );

        let color_host = window.use_keyed_state(
            "editor-color-picker-host",
            cx,
            |_, cx: &mut Context<EditorColorPickerHost>| EditorColorPickerHost::new(cx),
        );

        let measured = this.clone();
        let appeared = this.clone();
        let disappeared = this;
        div()
            .id("media-editor-screen")
            .relative()
            .size_full()
            .track_focus(&self.root_focus)
            .capture_any_mouse_down(cx.listener(|state, _: &MouseDownEvent, _, _| {
                state.keyboard_visible_at_press = state.keyboard_visible;
            }))
            .on_key_down(cx.listener(|state, event: &KeyDownEvent, window, cx| {
                state.handle_key_down(event, window, cx);
            }))
            .child(measure("media-editor-stage-bounds", move |bounds, _, cx| {
                measured.update(cx, |state, cx| {
                    let bounds = pixels_bounds(bounds);
                    if state.stage_bounds != bounds {
                        state.stage_bounds = bounds;
                        if let Some(presentation) = state.props.editor_presentation.clone() {
                            presentation.update(cx, |presentation, _| presentation.last_stage_bounds = bounds);
                        }
                        cx.notify();
                    }
                });
            }))
            .child(div().absolute().inset_0().child(observed))
            .child(color_host.cached(StyleRefinement::default().absolute().top_0().left_0().size_full()))
            .on_appear("media-editor-screen-appear", move |window, cx| {
                appeared.update(cx, |state, cx| state.did_appear(window, cx));
            })
            .and_on_disappear(move |window, cx| {
                disappeared.update(cx, |state, cx| {
                    if let Some(window) = window {
                        state.hand_keyboard_focus_back(window, cx);
                    }
                    state.did_disappear(cx);
                });
            })
    }
}

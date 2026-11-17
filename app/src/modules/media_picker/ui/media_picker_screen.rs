use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Context, ElementId, Entity, ExternalPaths, FocusHandle,
    FontWeight, Hsla, IntoElement, KeyDownEvent, Pixels, RenderImage, RenderOnce, Task, WeakEntity, Window,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::file_dialog::{self, FileFilter, FilterLabel, OpenRequest, Selection};
use rise_platform::photo_library::folder_library::{FOLDER_IMAGE_EXTENSIONS, FOLDER_VIDEO_EXTENSIONS};
use rise_platform::photo_library::LibraryAsset;
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::ModalSheetMetrics;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::{LiquidChrome, LiquidChromeModifiers};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::page_header_ui::PageHeaderBarLayout;
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowUi};
use rise_ui::swiftui::linear_gradient::{GradientStop, LinearGradient};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::swiftui::rounded_gradient::rounded_gradient_fill;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::shapes::pass_through;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::media_viewer::media_viewer_types::{MediaViewerFrameKeys, MediaViewerHeroSkin};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatDraftCustomEmoji;
use crate::modules::media_picker::api::media_picker_configuration::{
    MediaPickerConfiguration, MediaPickerMediaKind, MediaPickerSendMode,
};
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::api::media_picker_result::MediaPickerViewMode;
use crate::modules::media_picker::editor::media_editor_presentation_state::MediaEditorPresentationState;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorOutcome;
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::library::photo_library_albums::{
    PhotoLibraryAlbum, PhotoLibraryAlbums, PhotoLibraryAlbumsTitles,
};
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;
use crate::modules::media_picker::permissions::photo_library_permissions::{
    PhotoLibraryAccessLevel, PhotoLibraryPermissions,
};
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::media_picker::ui::media_picker_caption_bar_hooks::media_picker_caption_bar_hooks;
use crate::modules::media_picker::ui::media_picker_grid_view::MediaPickerGridView;
use crate::modules::media_picker::ui::media_picker_header_icon_button::MediaPickerHeaderIconButton;
use crate::modules::media_picker::ui::media_picker_permission_denied_view::MediaPickerPermissionDeniedView;
use crate::modules::media_picker::ui::media_picker_session::{MediaPickerExportProgressPresenter, MediaPickerSession};
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerFloatingDoneButton;

const ENTER_KEY: &str = "enter";
const PASTE_KEY: &str = "v";
const COUNT_SPRING_RESPONSE: f64 = 0.34;
const COUNT_SPRING_DAMPING_FRACTION: f64 = 0.85;
const HEADER_CHROME_MATERIAL_OPACITY: f32 = 0.84;
const GUIDE_ANCHOR_MORE: RiseGuideAnchor = RiseGuideAnchor::MediaPickerMore;
const HEADER_CHROME_MASK: [(f32, f32); 4] = [(0.0, 1.0), (0.56, 1.0), (0.76, 0.62), (1.0, 0.0)];
const HEADER_BAR_HEIGHT: f32 = 56.0;
const HEADER_CHROME_OVERHANG: f32 = 8.0;
const ALBUM_ACTION_PREFIX: &str = "media_picker_album:";
const SEND_UNCOMPRESSED_ACTION_ID: &str = "media_picker_send_uncompressed";
const MANAGE_ACCESS_ACTION_ID: &str = "media_picker_manage_access";
const SPOILER_ACTION_ID: &str = "media_picker_spoiler";
const VIEW_ONCE_ACTION_ID: &str = "media_picker_view_once";
const VIEW_TIMER_ACTION_ID: &str = "media_picker_view_timer";
const VIEW_TIMER_OFF_ACTION_ID: &str = "media_picker_view_timer_off";
const VIEW_TIMER_PRESET_PREFIX: &str = "media_picker_view_timer_";
const ALBUM_MENU_MAX_VISIBLE_ROWS: i64 = 4;
const ALBUM_COVER_CORNER_RADIUS: f32 = 6.0;
const TITLE_MAX_WIDTH: f32 = 220.0;
const TITLE_CHEVRON_SIZE: f32 = 11.0;
const TITLE_MIN_HEIGHT: f32 = 44.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaPickerLayout {
    #[default]
    FullScreen,
    BottomSheet,
}

struct MediaPickerScreenState {
    access: PhotoLibraryAccessLevel,
    bump_refresh: Uuid,
    is_more_menu_presented: bool,
    is_album_menu_presented: bool,
    album_menu: Entity<MediaPickerAlbumMenuModel>,
    album_id: String,
    offered_files: bool,
    open_panel: Option<Task<()>>,
}

impl MediaPickerScreenState {
    fn new(fetcher: &Entity<PhotoLibraryFetcher>, kind: MediaPickerMediaKind, scale: f32, cx: &mut Context<Self>) -> Self {
        let album_id = fetcher.read(cx).album_id().to_string();
        let album_menu = cx.new(|_| MediaPickerAlbumMenuModel { scale, ..MediaPickerAlbumMenuModel::default() });
        let reads_picked_files = fetcher.read(cx).reads_picked_files();
        let current = if reads_picked_files {
            PhotoLibraryAccessLevel::Authorized
        } else {
            PhotoLibraryPermissions::current()
        };
        if reads_picked_files {
        } else if current == PhotoLibraryAccessLevel::NotDetermined {
            Self::request_access(album_menu.clone(), kind, cx);
        } else if Self::reads_library(current) {
            MediaPickerScreen::reload_albums(&album_menu, kind, cx);
        }
        Self {
            access: current,
            bump_refresh: Uuid::new_v4(),
            is_more_menu_presented: false,
            is_album_menu_presented: false,
            album_menu,
            album_id,
            offered_files: false,
            open_panel: None,
        }
    }

    fn reads_library(access: PhotoLibraryAccessLevel) -> bool {
        access == PhotoLibraryAccessLevel::Authorized || access == PhotoLibraryAccessLevel::Limited
    }

    fn request_access(album_menu: Entity<MediaPickerAlbumMenuModel>, kind: MediaPickerMediaKind, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let granted = PhotoLibraryPermissions::request().await;
            let _ = this.update(cx, |this, cx| {
                this.access = granted;
                if Self::reads_library(granted) {
                    MediaPickerScreen::reload_albums(&album_menu, kind, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// rows and covers of the album menu
/// an entity so the open menu in its own overlay follows a cover or count that arrives late
#[derive(Default)]
pub struct MediaPickerAlbumMenuModel {
    pub albums: Vec<PhotoLibraryAlbum>,
    pub covers: HashMap<String, Arc<RenderImage>>,
    pub scale: f32,
}

#[derive(Clone, IntoElement)]
pub struct MediaPickerScreen {
    configuration: Rc<MediaPickerConfiguration>,
    theme_store: Entity<ThemeStore>,
    selection: Entity<MediaSelectionState>,
    caption: Binding<String>,
    caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    fetcher: Entity<PhotoLibraryFetcher>,
    editor_presentation: Entity<MediaEditorPresentationState>,
    layout: MediaPickerLayout,
    bottom_chrome_inset: f32,
    handle_clearance: f32,
    on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
    #[allow(dead_code)]
    on_editor_outcome: Rc<dyn Fn(MediaEditorOutcome, MediaAsset, &mut Window, &mut App)>,
    on_send_selection: Rc<dyn Fn(bool, &mut Window, &mut App)>,
    on_present_document_picker: Rc<dyn Fn(&mut Window, &mut App)>,
    on_open_camera: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    import_session: Option<WeakEntity<MediaPickerSession>>,
}

impl MediaPickerScreen {
    /// defaults are caption custom emojis none, full screen layout
    /// bottom chrome inset and handle clearance zero, on_open_camera none
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        selection: Entity<MediaSelectionState>,
        caption: Binding<String>,
        fetcher: Entity<PhotoLibraryFetcher>,
        editor_presentation: Entity<MediaEditorPresentationState>,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_editor_outcome: impl Fn(MediaEditorOutcome, MediaAsset, &mut Window, &mut App) + 'static,
        on_send_selection: impl Fn(bool, &mut Window, &mut App) + 'static,
        on_present_document_picker: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            configuration: Rc::new(configuration),
            theme_store,
            selection,
            caption,
            caption_custom_emojis: None,
            fetcher,
            editor_presentation,
            layout: MediaPickerLayout::FullScreen,
            bottom_chrome_inset: 0.0,
            handle_clearance: 0.0,
            on_cancel: Rc::new(on_cancel),
            on_editor_outcome: Rc::new(on_editor_outcome),
            on_send_selection: Rc::new(on_send_selection),
            on_present_document_picker: Rc::new(on_present_document_picker),
            on_open_camera: None,
            import_session: None,
        }
    }

    pub fn caption_custom_emojis(mut self, caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>) -> Self {
        self.caption_custom_emojis = caption_custom_emojis;
        self
    }

    pub fn layout(mut self, layout: MediaPickerLayout) -> Self {
        self.layout = layout;
        self
    }

    pub fn bottom_chrome_inset(mut self, bottom_chrome_inset: f32) -> Self {
        self.bottom_chrome_inset = bottom_chrome_inset;
        self
    }

    pub fn handle_clearance(mut self, handle_clearance: f32) -> Self {
        self.handle_clearance = handle_clearance;
        self
    }

    pub fn on_open_camera(mut self, on_open_camera: Option<Rc<dyn Fn(&mut Window, &mut App)>>) -> Self {
        self.on_open_camera = on_open_camera;
        self
    }

    /// desktop only, session that takes files dropped on the picker or pasted into it
    /// without it the picker takes neither
    pub fn import_session(mut self, import_session: WeakEntity<MediaPickerSession>) -> Self {
        self.import_session = Some(import_session);
        self
    }

    fn grid_bottom_inset(&self, cx: &App) -> f32 {
        if self.layout == MediaPickerLayout::BottomSheet {
            if !self.configuration.shows_bottom_bar {
                return self.bottom_chrome_inset;
            }
            if !self.configuration.allows_caption {
                return 72.0 + self.bottom_chrome_inset;
            }
            let caption_bar = if self.selection.read(cx).is_empty() { 0.0 } else { 86.0 };
            return caption_bar + self.bottom_chrome_inset;
        }
        120.0
    }

    fn shows_floating_caption(&self) -> bool {
        self.layout != MediaPickerLayout::BottomSheet && self.configuration.allows_caption
    }

    fn shows_floating_done_button(&self) -> bool {
        self.layout != MediaPickerLayout::BottomSheet
            && self.configuration.shows_bottom_bar
            && !self.configuration.allows_caption
    }

    pub const SHEET_HANDLE_CLEARANCE: f32 = BottomSheetMetrics::HANDLE_TOP_PADDING
        + BottomSheetMetrics::HANDLE_HEIGHT
        + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;

    fn floating_header_height(&self) -> f32 {
        HEADER_BAR_HEIGHT + self.handle_clearance
    }

    /// only a preset that can deliver files offers sending without compression
    fn offers_uncompressed_send(&self) -> bool {
        self.configuration.send_mode != MediaPickerSendMode::Media
    }

    fn floating_header_chrome_extent(&self) -> f32 {
        self.floating_header_height() + HEADER_CHROME_OVERHANG
    }

    fn offers_view_once(&self, cx: &App) -> bool {
        self.configuration.allows_view_once && !self.selection.read(cx).is_empty()
    }

    fn offers_spoiler(&self, cx: &App) -> bool {
        self.configuration.allows_media_spoiler && !self.selection.read(cx).is_empty()
    }

    fn more_menu_actions(&self, access: PhotoLibraryAccessLevel) -> HoldContextMenuPresentationActions {
        let configuration = self.configuration.clone();
        let selection = self.selection.clone();
        let offers_uncompressed_send = self.offers_uncompressed_send();
        Rc::new(move |cx: &App| {
            let selection = selection.read(cx);
            let has_selection = !selection.is_empty();
            let mut actions: Vec<HoldContextMenuAction> = Vec::new();
            if offers_uncompressed_send {
                actions.push(
                    HoldContextMenuAction::new(SEND_UNCOMPRESSED_ACTION_ID, tr("media_picker_send_uncompressed"))
                        .system_image("doc"),
                );
            }
            if configuration.allows_media_spoiler && has_selection {
                let forced = selection.view_mode.is_some();
                let hidden = selection.hides_under_spoiler || forced;
                actions.push(
                    HoldContextMenuAction::new(
                        SPOILER_ACTION_ID,
                        tr(if hidden { "media_picker_show_without_spoiler" } else { "media_picker_hide_under_spoiler" }),
                    )
                    .system_image(if hidden { "eye" } else { "eye.slash" })
                    .disabled(forced),
                );
            }
            if configuration.allows_view_once && has_selection {
                let is_once = matches!(selection.view_mode, Some(MediaPickerViewMode::Once));
                let seconds = selection.view_mode.as_ref().and_then(MediaPickerViewMode::seconds);
                actions.push(
                    HoldContextMenuAction::new(VIEW_ONCE_ACTION_ID, tr("media_view_once"))
                        .system_image(if is_once { "checkmark" } else { "1.circle" }),
                );
                let mut presets: Vec<HoldContextMenuAction> = MediaPickerViewMode::timer_presets()
                    .into_iter()
                    .map(|preset| {
                        HoldContextMenuAction::new(
                            format!("{VIEW_TIMER_PRESET_PREFIX}{preset}"),
                            tr("media_view_seconds").to_string().replace("{count}", &preset.to_string()),
                        )
                        .checked(seconds == Some(preset))
                    })
                    .collect();
                presets.push(
                    HoldContextMenuAction::new(VIEW_TIMER_OFF_ACTION_ID, tr("media_view_off")).checked(seconds.is_none()),
                );
                if !selection.contains_video() {
                    actions.push(
                        HoldContextMenuAction::new(VIEW_TIMER_ACTION_ID, tr("media_view_timer"))
                            .system_image("timer")
                            .submenu(presets),
                    );
                }
            }
            if access == PhotoLibraryAccessLevel::Limited {
                actions.push(
                    HoldContextMenuAction::new(MANAGE_ACCESS_ACTION_ID, tr("media_picker_manage_access"))
                        .system_image("photo.on.rectangle.angled"),
                );
            }
            actions
        })
    }

    fn handle_more_menu_action(&self) -> HoldContextMenuPresentationOnAction {
        let selection = self.selection.clone();
        let on_send_selection = self.on_send_selection.clone();
        Rc::new(move |id: String, window: &mut Window, cx: &mut App| match id.as_str() {
            SEND_UNCOMPRESSED_ACTION_ID => {
                selection.update(cx, |selection, cx| {
                    selection.send_as_file = true;
                    cx.notify();
                });
                on_send_selection(true, window, cx);
            }
            MANAGE_ACCESS_ACTION_ID => Self::present_limited_selector(cx),
            SPOILER_ACTION_ID => selection.update(cx, |selection, cx| {
                selection.hides_under_spoiler = !selection.hides_under_spoiler;
                cx.notify();
            }),
            VIEW_ONCE_ACTION_ID => selection.update(cx, |selection, cx| {
                let is_once = matches!(selection.view_mode, Some(MediaPickerViewMode::Once));
                selection.view_mode = if is_once { None } else { Some(MediaPickerViewMode::Once) };
                cx.notify();
            }),
            VIEW_TIMER_OFF_ACTION_ID => selection.update(cx, |selection, cx| {
                if selection.view_mode.as_ref().and_then(MediaPickerViewMode::seconds).is_some() {
                    selection.view_mode = None;
                    cx.notify();
                }
            }),
            other => {
                let Some(seconds) =
                    other.strip_prefix(VIEW_TIMER_PRESET_PREFIX).and_then(|rest| rest.parse::<i64>().ok())
                else {
                    return;
                };
                selection.update(cx, |selection, cx| {
                    selection.view_mode = Some(MediaPickerViewMode::Timed { seconds });
                    cx.notify();
                });
            }
        })
    }

    fn base_id(&self) -> ElementId {
        ElementId::View(self.selection.entity_id())
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.base_id()), name.into())
    }

    fn content(
        &self,
        state: &Entity<MediaPickerScreenState>,
        access: PhotoLibraryAccessLevel,
        bump_refresh: Uuid,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        match access {
            PhotoLibraryAccessLevel::Authorized | PhotoLibraryAccessLevel::Limited => {
                self.authorized_content(state, access, bump_refresh, window, cx)
            }
            PhotoLibraryAccessLevel::NotDetermined => {
                let tint = self.theme_store.read(cx).current_theme().text.primary;
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(progress_view().tint(tint))
                    .into_any_element()
            }
            PhotoLibraryAccessLevel::Denied | PhotoLibraryAccessLevel::Restricted => {
                let on_cancel = self.on_cancel.clone();
                let on_present_document_picker = self.on_present_document_picker.clone();
                let bottom = if self.layout == MediaPickerLayout::BottomSheet { self.grid_bottom_inset(cx) } else { 0.0 };
                div()
                    .size_full()
                    .pb(px(bottom))
                    .child(MediaPickerPermissionDeniedView::new(
                        self.theme_store.clone(),
                        |_, cx| PhotoLibraryPermissions::open_system_settings(cx),
                        move |window, cx| on_cancel(window, cx),
                    )
                    .on_choose_file(
                        self.configuration.offers_file_chooser.then_some(on_present_document_picker),
                    ))
                    .into_any_element()
            }
        }
    }

    fn authorized_content(
        &self,
        state: &Entity<MediaPickerScreenState>,
        access: PhotoLibraryAccessLevel,
        bump_refresh: Uuid,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let (hero_lift_id, hero_cell_hidden) = {
            let presentation = self.editor_presentation.read(cx);
            (presentation.hero_lift_id.clone(), presentation.hero_cell_hidden)
        };
        let on_pick = self.clone();
        let on_toggle = self.clone();
        let on_preview = self.clone();
        let grid = MediaPickerGridView::new(
            self.child_id("grid"),
            self.fetcher.clone(),
            self.selection.clone(),
            self.theme_store.clone(),
            move |asset: &LibraryAsset, _: &mut Window, cx: &mut App| on_pick.toggle_or_offer_more(asset, cx),
            move |asset: &LibraryAsset, _: &mut Window, cx: &mut App| on_toggle.toggle_or_offer_more(asset, cx),
            move |asset: &LibraryAsset, index: i64, frame: Option<Bounds<Pixels>>, window: &mut Window, cx: &mut App| {
                on_preview.quick_select_or_open_editor(asset, index, frame, window, cx);
            },
            bump_refresh,
            hero_lift_id,
            hero_cell_hidden,
        )
        .shows_selection_controls(self.configuration.shows_selection_controls)
        .keeps_stable_maximum_viewport(self.layout == MediaPickerLayout::BottomSheet)
        .top_content_inset(self.floating_header_height())
        .bottom_content_inset(self.grid_bottom_inset(cx))
        .on_camera_tile(if self.fetcher.read(cx).reads_picked_files() { None } else { self.on_open_camera.clone() });

        let mut content = div().relative().size_full().child(grid).children(self.empty_files_offer(state, cx));

        if self.shows_floating_caption() {
            let selection = self.selection.clone();
            let on_send_selection = self.on_send_selection.clone();
            let caption_bar = media_picker_caption_bar_hooks().map(|hooks| {
                (hooks.caption_bar)(
                    self.child_id("caption-bar"),
                    self.selection.clone(),
                    self.theme_store.clone(),
                    self.caption.clone(),
                    self.caption_custom_emojis.clone(),
                    None,
                    Rc::new(move |window: &mut Window, cx: &mut App| {
                        let send_as_file = selection.read(cx).send_as_file;
                        on_send_selection(send_as_file, window, cx);
                    }),
                )
            });
            content = content.child(div().absolute().left_0().right_0().bottom_0().children(caption_bar));
        }

        if self.shows_floating_done_button() {
            let on_send_selection = self.on_send_selection.clone();
            content = content.child(
                div().absolute().left_0().right_0().bottom(px(5.0)).px(px(16.0)).child(
                    MediaPickerFloatingDoneButton::new(
                        self.selection.clone(),
                        self.theme_store.clone(),
                        self.configuration.confirm_button_title.clone(),
                        move |window, cx| on_send_selection(false, window, cx),
                    )
                    .system_image(self.configuration.confirm_button_system_image.clone()),
                ),
            );
        }

        let header = self.header(state, access, window, cx);
        content
            .child(self.picker_header_chrome(cx))
            .child(div().absolute().top_0().left_0().right_0().child(header))
            .into_any_element()
    }

    fn quick_select_or_open_editor(
        &self,
        asset: &LibraryAsset,
        index: i64,
        source_screen_frame: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let kind_locked = self.selection.read(cx).is_kind_locked(asset);
        if kind_locked {
            return;
        }
        if !self.configuration.allows_editing {
            if !self.selection.read(cx).can_select(asset) {
                self.offer_more_if_count_refused(asset, cx);
                return;
            }
            if !self.selection.read(cx).contains(&asset.id)
                && !self.selection.update(cx, |selection, cx| selection.toggle(asset, cx))
            {
                return;
            }
            if self.configuration.auto_confirm_on_select && !self.selection.read(cx).is_empty() {
                (self.on_send_selection)(false, window, cx);
            }
            return;
        }
        self.open_editor(asset, index, source_screen_frame, cx);
    }

    fn toggle_or_offer_more(&self, asset: &LibraryAsset, cx: &mut App) {
        let selection = self.selection.read(cx);
        if !selection.contains(&asset.id) && !selection.can_select(asset) {
            self.offer_more_if_count_refused(asset, cx);
            return;
        }
        let _ = self.selection.update(cx, |selection, cx| selection.toggle(asset, cx));
    }

    fn offer_more_if_count_refused(&self, asset: &LibraryAsset, cx: &mut App) {
        self.configuration.offer_more_if_count_refused(asset, &self.selection, cx);
    }

    fn open_editor(
        &self,
        asset: &LibraryAsset,
        index: i64,
        source_screen_frame: Option<Bounds<Pixels>>,
        cx: &mut App,
    ) {
        let media_asset = MediaAsset::from_asset(asset);
        let registry_id = MediaViewerFrameKeys::picker_asset(&asset.id);
        let skin = MediaViewerHeroSkin::new(format!("ph://{}", asset.id), false);
        let source_frame = source_screen_frame.or_else(|| MediaCellScreenFrameRegistry::frame(&registry_id));
        let editor_presentation = self.editor_presentation.clone();
        let asset = asset.clone();
        cx.spawn(async move |cx| {
            MediaEditorPresentationState::prepare_hero_warm_image(
                editor_presentation.clone(),
                skin.display_source.clone(),
                cx,
            )
            .await;
            editor_presentation.update(cx, |presentation, cx| {
                presentation.present(media_asset, asset, source_frame, registry_id, skin, Some(index), cx);
            });
        })
        .detach();
    }

    fn present_limited_selector(cx: &App) {
        if let Some(host) = MediaPickerPresenterResolver::top_presenter(cx) {
            let _ = PhotoLibraryPermissions::present_limited_selector(host);
        }
    }

    fn count_badge(&self, window: &mut Window, cx: &mut App) -> Option<impl IntoElement + use<>> {
        let (count, primary, on_primary) = {
            let selection = self.selection.read(cx);
            let visible = self.configuration.shows_selection_controls && !selection.is_empty();
            let theme = self.theme_store.read(cx).current_theme();
            (visible.then(|| selection.count()), theme.primary._100, theme.content_on(theme.primary._100))
        };
        let now = RiseClock::monotonic();
        let motion = spring(COUNT_SPRING_RESPONSE, COUNT_SPRING_DAMPING_FRACTION);
        let presence = window.use_keyed_state(self.child_id("count"), cx, |_, _| {
            Presence::new(count).with_transition(Transition::scale_default().combined(Transition::opacity()))
        });
        presence.update(cx, |presence, cx| presence.set_in(count, motion, now, cx));
        presence.read(cx).render_transitioned(now, window, |count| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(26.0))
                .min_h(px(26.0))
                .px(px(7.0))
                .rounded_full()
                .bg(primary)
                .child(MainText::new(count.to_string()).px(13.0).font_weight(FontWeight::BOLD).color(on_primary))
        })
    }

    fn header(
        &self,
        state: &Entity<MediaPickerScreenState>,
        access: PhotoLibraryAccessLevel,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let text_primary = self.theme_store.read(cx).current_theme().text.primary;
        let on_cancel = self.on_cancel.clone();
        let leading = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                MediaPickerHeaderIconButton::new(self.child_id("close"), "xmark", move |window, cx| {
                    on_cancel(window, cx);
                })
                .font_size(15.0),
            )
            .children(self.count_badge(window, cx));

        let reads_picked_files = self.fetcher.read(cx).reads_picked_files();
        let trailing = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .children(reads_picked_files.then(|| self.add_files_button(state)))
            .children((!reads_picked_files).then(|| self.file_chooser_button()).flatten())
            .child(self.header_trailing(state, access, cx));

        let title = if reads_picked_files {
            Self::plain_title(self.configuration.title.clone(), text_primary)
        } else {
            self.album_title_button(state, text_primary, cx)
        };

        div()
            .pt(px(self.handle_clearance))
            .child(
                div().px(px(14.0)).h(px(HEADER_BAR_HEIGHT)).child(
                    PageHeaderBarLayout::new()
                        .edge_padding(0.0)
                        .center_gap(8.0)
                        .child(leading)
                        .child(trailing)
                        .child(title),
                ),
            )
            .into_any_element()
    }

    fn current_album_title(state: &Entity<MediaPickerScreenState>, cx: &App) -> String {
        let state = state.read(cx);
        state
            .album_menu
            .read(cx)
            .albums
            .iter()
            .find(|album| album.id == state.album_id)
            .map(|album| album.title.clone())
            .unwrap_or_else(|| tr("media_picker_album_recents").to_string())
    }

    /// the shown album with a chevron in the titles place and color
    /// a tap opens the album menu
    /// why: no Gallery line above it, name and chevron both white
    fn title_label(title: String, text_primary: Hsla) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .max_w(px(TITLE_MAX_WIDTH))
            .px(px(8.0))
            .min_h(px(TITLE_MIN_HEIGHT))
            .child(
                MainText::new(title)
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .color(text_primary)
                    .number_of_lines(1)
                    .minimum_scale_factor(0.8),
            )
            .children(IconUi::sized("chevron.down", px(TITLE_CHEVRON_SIZE), text_primary, false))
            .into_any_element()
    }

    fn album_title_button(&self, state: &Entity<MediaPickerScreenState>, text_primary: Hsla, cx: &App) -> AnyElement {
        let title = Self::current_album_title(state, cx);
        let album_menu = state.read(cx).album_menu.clone();
        let is_album_menu_presented = Binding::entity(
            state.clone(),
            |state: &MediaPickerScreenState| state.is_album_menu_presented,
            |state: &mut MediaPickerScreenState, value: bool, cx| {
                state.is_album_menu_presented = value;
                cx.notify();
            },
        );
        let source_title = title.clone();
        let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
            Self::title_label(source_title.clone(), text_primary)
        });
        let actions_menu = album_menu.clone();
        let actions: HoldContextMenuPresentationActions = Rc::new(move |cx: &App| Self::album_menu_actions(&actions_menu, cx));
        let fetcher = self.fetcher.clone();
        let selected_state = state.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::select_album(&fetcher, &selected_state, &id, cx);
        });
        let opener = state.clone();
        let kind = self.configuration.kind;
        div()
            .id(self.child_id("album-title"))
            .cursor_pointer()
            .aria_label(title.clone())
            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                Self::reload_albums(&album_menu, kind, cx);
                opener.update(cx, |state, cx| {
                    state.is_album_menu_presented = true;
                    cx.notify();
                });
            })
            .child(Self::title_label(title, text_primary))
            .tap_context_menu(
                self.child_id("album-menu"),
                is_album_menu_presented,
                HoldContextMenuAlignment::Leading,
                false,
                false,
                false,
                Some(ALBUM_MENU_MAX_VISIBLE_ROWS),
                actions,
                on_action,
                source,
            )
            .into_any_element()
    }

    fn album_menu_actions(album_menu: &Entity<MediaPickerAlbumMenuModel>, cx: &App) -> Vec<HoldContextMenuAction> {
        let menu = album_menu.read(cx);
        menu.albums
            .iter()
            .map(|album| {
                let cover = album.cover.as_ref().and_then(|cover| menu.covers.get(&cover.id).cloned());
                HoldContextMenuAction::new(format!("{ALBUM_ACTION_PREFIX}{}", album.id), album.title.clone())
                    .system_image("photo.on.rectangle")
                    .leading_image(cover)
                    .subtitle(album.count.to_string())
                    .leading_image_corner_radius(ALBUM_COVER_CORNER_RADIUS)
            })
            .collect()
    }

    fn select_album(
        fetcher: &Entity<PhotoLibraryFetcher>,
        state: &Entity<MediaPickerScreenState>,
        action_id: &str,
        cx: &mut App,
    ) {
        let Some(id) = action_id.strip_prefix(ALBUM_ACTION_PREFIX) else {
            return;
        };
        fetcher.update(cx, |fetcher, cx| fetcher.set_album(id, cx));
        let album_id = fetcher.read(cx).album_id().to_string();
        state.update(cx, |state, cx| {
            state.album_id = album_id;
            cx.notify();
        });
    }

    /// counts the albums off the main thread then pulls covers it doesnt hold yet
    /// runs when the picker gets access so covers are usually there before the menu opens
    fn reload_albums(album_menu: &Entity<MediaPickerAlbumMenuModel>, kind: MediaPickerMediaKind, cx: &mut App) {
        let titles = PhotoLibraryAlbumsTitles::new();
        let menu = album_menu.clone();
        let load = RiseAccountRuntime::current().spawn_blocking(move || PhotoLibraryAlbums::load(kind, &titles));
        cx.spawn(async move |cx| {
            let Ok(list) = load.await else {
                return;
            };
            let _ = menu.update(cx, |menu, cx| {
                let missing: Vec<_> = list
                    .iter()
                    .filter_map(|album| album.cover.clone())
                    .filter(|cover| !menu.covers.contains_key(&cover.id))
                    .collect();
                menu.albums = list;
                cx.notify();
                let scale = if menu.scale > 0.0 { menu.scale } else { 1.0 };
                for cover in missing {
                    let model = cx.entity();
                    let cover_id = cover.id.clone();
                    PhotoLibraryAlbums::request_cover(
                        &cover,
                        scale,
                        move |image, cx| {
                            let Some(image) = image else {
                                return;
                            };
                            model.update(cx, |menu, cx| {
                                menu.covers.insert(cover_id.clone(), image);
                                cx.notify();
                            });
                        },
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// title of a picker that lists brought files, no album to choose
    /// why: no width cap, the header already offers the room between the buttons
    /// a column stretches the text to that width and truncates there
    fn plain_title(title: String, text_primary: Hsla) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .justify_center()
            .max_w_full()
            .px(px(8.0))
            .min_h(px(TITLE_MIN_HEIGHT))
            .child(
                MainText::new(title)
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .color(text_primary)
                    .number_of_lines(1)
                    .minimum_scale_factor(0.8),
            )
            .into_any_element()
    }

    fn add_files_button(&self, state: &Entity<MediaPickerScreenState>) -> AnyElement {
        let screen = self.clone();
        let state = state.clone();
        div()
            .id(self.child_id("add-files-hint"))
            .tooltip_label(tr("media_picker_choose_file"))
            .child(
                MediaPickerHeaderIconButton::new(self.child_id("add-files"), "plus", move |window, cx| {
                    screen.present_file_dialog(&state, false, window, cx);
                })
                .font_size(15.0),
            )
            .into_any_element()
    }

    fn picked_files_filter(&self) -> FileFilter {
        let (label, extensions): (FilterLabel, Vec<&str>) = match self.configuration.kind {
            MediaPickerMediaKind::Images => (FilterLabel::IMAGES, FOLDER_IMAGE_EXTENSIONS.to_vec()),
            MediaPickerMediaKind::Videos => (FilterLabel::VIDEO, FOLDER_VIDEO_EXTENSIONS.to_vec()),
            MediaPickerMediaKind::All => (
                FilterLabel::MEDIA,
                FOLDER_IMAGE_EXTENSIONS.iter().chain(FOLDER_VIDEO_EXTENSIONS).copied().collect(),
            ),
        };
        let extensions = extensions.into_iter().filter(|extension| {
            self.configuration
                .allowed_file_extensions
                .as_ref()
                .is_none_or(|allowed| allowed.iter().any(|allowed| allowed.eq_ignore_ascii_case(extension)))
        });
        FileFilter::new(label, extensions)
    }

    /// desktop picker opens the native open panel instead of a gallery
    /// closes_when_empty is the first offer on appear, cancelling with nothing brought closes the picker
    fn present_file_dialog(
        &self,
        state: &Entity<MediaPickerScreenState>,
        closes_when_empty: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let closes_when_empty = closes_when_empty && self.configuration.source_tabs.len() <= 1;
        let request = OpenRequest::files(self.picked_files_filter())
            .allowing_multiple(self.configuration.max_selection_count > 1);
        let answer = file_dialog::open(cx, &request);
        let fetcher = self.fetcher.clone();
        let selection = self.selection.clone();
        let on_cancel = self.on_cancel.clone();
        let opener = self.clone();
        let document_session = self.import_session.clone();
        let open_panel = window.spawn(cx, async move |cx| {
            let picked = answer.await;
            let _ = cx.update(|window, cx| {
                let mut sent_documents = false;
                let accepted = match picked {
                    Selection::Picked(chosen) => {
                        if chosen.has_rejections() {
                            tracing::info!(target: "riseonly::X764", rejected = chosen.rejected.len(), "picked files outside the media filter go as documents");
                            if let Some(session) = document_session.as_ref().and_then(WeakEntity::upgrade) {
                                session.update(cx, |session, cx| session.receive_external_paths(chosen.rejected, cx));
                                sent_documents = true;
                            }
                        }
                        chosen.accepted
                    }
                    Selection::Cancelled => Vec::new(),
                    Selection::Failed(error) => {
                        tracing::warn!(target: "riseonly::X644", %error, "open panel failed");
                        Vec::new()
                    }
                };
                if accepted.is_empty() {
                    if closes_when_empty && !sent_documents && !fetcher.read(cx).has_picked_files() {
                        on_cancel(window, cx);
                    }
                    return;
                }
                opener.add_picked_files(&fetcher, &selection, accepted, cx);
            });
        });
        state.update(cx, |state, _| state.open_panel = Some(open_panel));
    }

    fn add_picked_files(
        &self,
        fetcher: &Entity<PhotoLibraryFetcher>,
        selection: &Entity<MediaSelectionState>,
        paths: Vec<std::path::PathBuf>,
        cx: &mut App,
    ) {
        if self.configuration.shows_selection_controls {
            let configuration = self.configuration.clone();
            let chosen = selection.clone();
            PhotoLibraryFetcher::add_picked_files_then(fetcher, selection, paths, true, cx, move |added, cx| {
                let refused = {
                    let state = chosen.read(cx);
                    if !state.max_reached() {
                        return;
                    }
                    added.into_iter().find(|asset| !state.contains(&asset.id))
                };
                if let Some(refused) = refused {
                    configuration.announce_picked_overflow(&refused, &chosen, cx);
                }
            });
            return;
        }
        let screen = self.clone();
        let listed = fetcher.clone();
        PhotoLibraryFetcher::add_picked_files_then(fetcher, selection, paths, false, cx, move |added, cx| {
            let Some(first) = added.first() else {
                return;
            };
            let index = {
                let fetcher = listed.read(cx);
                (0..fetcher.asset_count() as i64).find(|at| fetcher.asset(*at).is_some_and(|asset| asset.id == first.id))
            };
            if !screen.selection.read(cx).is_kind_locked(first) {
                screen.open_editor(first, index.unwrap_or(0), None, cx);
            }
        });
    }

    fn empty_files_offer(&self, state: &Entity<MediaPickerScreenState>, cx: &App) -> Option<AnyElement> {
        let fetcher = self.fetcher.read(cx);
        if !fetcher.reads_picked_files() || fetcher.has_picked_files() {
            return None;
        }
        let theme = self.theme_store.read(cx).current_theme().clone();
        let label = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(18.0))
            .h(px(38.0))
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text.primary)
            .children(IconUi::sized("folder", px(15.0), theme.text.primary, false))
            .child(tr("media_picker_choose_file"))
            .liquid_chrome_capsule_with(0.18, true, None, false);
        let screen = self.clone();
        let state = state.clone();
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    plain(self.child_id("empty-choose-file"))
                        .on_press(move |window, cx| screen.present_file_dialog(&state, false, window, cx))
                        .child(label),
                )
                .into_any_element(),
        )
    }

    fn offer_files_on_appear(&self, state: &Entity<MediaPickerScreenState>, window: &mut Window, cx: &mut App) {
        let fetcher = self.fetcher.read(cx);
        if !fetcher.reads_picked_files() || fetcher.has_picked_files() || state.read(cx).offered_files {
            return;
        }
        state.update(cx, |state, _| state.offered_files = true);
        self.present_file_dialog(state, true, window, cx);
    }

    fn file_chooser_button(&self) -> Option<impl IntoElement + use<>> {
        self.configuration.offers_file_chooser.then(|| {
            let on_present_document_picker = self.on_present_document_picker.clone();
            div().id(self.child_id("choose-file-hint")).tooltip_label(tr("media_picker_choose_file")).child(
                MediaPickerHeaderIconButton::new(self.child_id("choose-file"), "folder", move |window, cx| {
                    on_present_document_picker(window, cx);
                })
                .font_size(15.0),
            )
        })
    }

    fn header_trailing(&self, state: &Entity<MediaPickerScreenState>, access: PhotoLibraryAccessLevel, cx: &App) -> AnyElement {
        if self.configuration.header_confirms_selection {
            let is_empty = self.selection.read(cx).is_empty();
            let on_send_selection = self.on_send_selection.clone();
            let selection = self.selection.clone();
            let confirm = div()
                .id(self.child_id("confirm-hint"))
                .tooltip_label(self.configuration.confirm_button_title.clone())
                .child(
                    MediaPickerHeaderIconButton::new(
                        self.child_id("confirm"),
                        self.configuration.confirm_button_system_image.clone(),
                        move |window, cx| {
                            if selection.read(cx).is_empty() {
                                return;
                            }
                            on_send_selection(false, window, cx);
                        },
                    )
                    .font_size(15.0)
                    .disabled(is_empty),
                );
            let limited = (access == PhotoLibraryAccessLevel::Limited).then(|| {
                div().id(self.child_id("manage-access-hint")).tooltip_label(tr("media_picker_manage_access")).child(
                    MediaPickerHeaderIconButton::new(
                        self.child_id("manage-access"),
                        "photo.on.rectangle.angled",
                        |_, cx| Self::present_limited_selector(cx),
                    )
                    .font_size(15.0),
                )
            });
            return div().flex().items_center().gap(px(10.0)).children(limited).child(confirm).into_any_element();
        }
        let side = px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE);
        if self.configuration.shows_selection_controls
            && (self.offers_uncompressed_send()
                || access == PhotoLibraryAccessLevel::Limited
                || self.offers_spoiler(cx)
                || self.offers_view_once(cx))
        {
            let is_more_menu_presented = Binding::entity(
                state.clone(),
                |state: &MediaPickerScreenState| state.is_more_menu_presented,
                |state: &mut MediaPickerScreenState, value: bool, cx| {
                    state.is_more_menu_presented = value;
                    cx.notify();
                },
            );
            let source_id = self.child_id("more-source");
            let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
                MediaPickerHeaderIconButton::new(source_id.clone(), "ellipsis", |_, _| {}).font_size(17.0).into_any_element()
            });
            let opener = state.clone();
            return div()
                .id(self.child_id("more-hint"))
                .aria_label(tr("share_action_more"))
                .tooltip_label(tr("share_action_more"))
                .child(
                    MediaPickerHeaderIconButton::new(self.child_id("more-icon"), "ellipsis", move |_, cx| {
                        opener.update(cx, |state, cx| {
                            state.is_more_menu_presented = true;
                            cx.notify();
                        });
                    })
                    .font_size(17.0)
                    .tap_context_menu(
                        self.child_id("more"),
                        is_more_menu_presented,
                        HoldContextMenuAlignment::Trailing,
                        false,
                        false,
                        false,
                        None,
                        self.more_menu_actions(access),
                        self.handle_more_menu_action(),
                        source,
                    ),
                )
                .guide_anchor(GUIDE_ANCHOR_MORE)
                .into_any_element();
        }
        if self.configuration.offers_file_chooser {
            return div().hidden().into_any_element();
        }
        div().size(side).into_any_element()
    }

    fn picker_header_chrome(&self, cx: &App) -> impl IntoElement + use<> {
        let extent = self.floating_header_chrome_extent();
        let bg_100 = self.theme_store.read(cx).current_theme().bg._100;
        let (material_stops, shadow_stops) = Self::header_chrome_stops(extent, bg_100, cx);
        pass_through()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(extent))
            .child(pass_through().absolute().inset_0().child(LinearGradient::new(
                material_stops,
                UnitPoint::TOP,
                UnitPoint::BOTTOM,
            )))
            .child(pass_through().absolute().inset_0().child(LinearGradient::new(
                shadow_stops,
                UnitPoint::TOP,
                UnitPoint::BOTTOM,
            )))
    }

    pub(crate) fn modal_edge_header_chrome(handle_clearance: f32, theme_store: &Entity<ThemeStore>, cx: &App) -> AnyElement {
        let inset = ModalSheetMetrics::CONTENT_INSET;
        let radius = ModalSheetMetrics::CORNER_RADIUS - ModalSheetMetrics::OUTLINE_WIDTH;
        let extent = HEADER_BAR_HEIGHT + handle_clearance + HEADER_CHROME_OVERHANG;
        let height = inset + extent + radius;
        let bg_100 = theme_store.read(cx).current_theme().bg._100;
        let (material_stops, shadow_stops) = Self::header_chrome_stops(extent, bg_100, cx);
        let layer = |stops: Vec<GradientStop>| {
            let stops = stops
                .into_iter()
                .map(|stop| GradientStop::new(stop.color, (inset + stop.location * extent) / height))
                .collect();
            pass_through()
                .absolute()
                .inset_0()
                .child(rounded_gradient_fill(LinearGradient::new(stops, UnitPoint::TOP, UnitPoint::BOTTOM), radius))
        };
        pass_through()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(height))
            .child(layer(material_stops))
            .child(layer(shadow_stops))
            .into_any_element()
    }

    /// bar material and shadow of the header chrome over a band extent tall
    fn header_chrome_stops(extent: f32, bg_100: Hsla, cx: &App) -> (Vec<GradientStop>, Vec<GradientStop>) {
        let material = bg_100.opacity(LiquidChrome::bar_material().painted_opacity() * HEADER_CHROME_MATERIAL_OPACITY);
        let material_stops: Vec<GradientStop> = HEADER_CHROME_MASK
            .iter()
            .map(|(location, opacity)| GradientStop::new(material.opacity(*opacity), *location))
            .collect();
        let shadow_stops: Vec<GradientStop> = ScrollChromeShadowUi::new(ScrollChromeShadowStyle {
            extent,
            overflow_below_header: 0.0,
            peak_opacity: 0.88,
            color: bg_100,
            mid_location: 0.64,
            mid_opacity_factor: 0.58,
            top_plateau_end: 0.17,
            top_plateau_opacity_factor: 1.0,
            ..ScrollChromeShadowStyle::default()
        })
        .stops(cx)
        .into_iter()
        .map(|stop| Self::masked_stop(stop.color, stop.location))
        .collect();
        (material_stops, shadow_stops)
    }

    fn masked_stop(color: Hsla, location: f32) -> GradientStop {
        GradientStop::new(color.opacity(Self::header_chrome_mask_opacity(location)), location)
    }

    fn header_chrome_mask_opacity(location: f32) -> f32 {
        let mut previous = HEADER_CHROME_MASK[0];
        for current in HEADER_CHROME_MASK {
            if location <= current.0 {
                let span = current.0 - previous.0;
                if span <= 0.0 {
                    return current.1;
                }
                return previous.1 + (current.1 - previous.1) * (location - previous.0) / span;
            }
            previous = current;
        }
        previous.1
    }

    fn handle_key_down(
        &self,
        event: &KeyDownEvent,
        focus_handle: &FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.editor_presentation.read(cx).is_presented {
            return;
        }
        let key = event.keystroke.key.as_str();
        if !focus_handle.is_focused(window) {
            return;
        }
        if key == ENTER_KEY {
            let selection = self.selection.read(cx);
            if selection.is_empty() {
                return;
            }
            let send_as_file = self.configuration.allows_caption && selection.send_as_file;
            cx.stop_propagation();
            (self.on_send_selection)(send_as_file, window, cx);
        } else if key == PASTE_KEY
            && event.keystroke.modifiers.secondary()
            && let Some(session) = &self.import_session
        {
            cx.stop_propagation();
            let _ = session.update(cx, |session, cx| session.import_pasteboard(cx));
        }
    }
}

impl RenderOnce for MediaPickerScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let sheet_bg = {
            let theme = self.theme_store.read(cx).current_theme();
            if self.layout == MediaPickerLayout::BottomSheet { LiquidChrome::surface_fill(theme, 0) } else { theme.bg._000 }
        };
        let (fetcher, kind) = (self.fetcher.clone(), self.configuration.kind);
        let scale = window.scale_factor();
        let state = window.use_keyed_state(self.child_id("state"), cx, move |_, cx| {
            MediaPickerScreenState::new(&fetcher, kind, scale, cx)
        });
        let album_menu = state.read(cx).album_menu.clone();
        if album_menu.read(cx).scale != scale {
            album_menu.update(cx, |menu, _| menu.scale = scale);
        }
        let (access, bump_refresh) = {
            let state = state.read(cx);
            (state.access, state.bump_refresh)
        };
        let focus_handle: Entity<FocusHandle> =
            window.use_keyed_state(self.child_id("focus"), cx, |_, cx| cx.focus_handle());
        let focus_handle = focus_handle.read(cx).clone();

        let content = self.content(&state, access, bump_refresh, window, cx);
        let export_overlay = if self.editor_presentation.read(cx).is_presented {
            None
        } else {
            MediaPickerExportProgressPresenter::overlay(cx)
        };

        let on_key = self.clone();
        let cancel_screen = self.clone();
        let key_focus = focus_handle.clone();
        let page_focus = PageFocus::keyed_with(self.child_id("page-focus"), focus_handle.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let drop_session = self.import_session.clone();
        let drop_fetcher = self.fetcher.clone();
        let drop_selection = self.selection.clone();
        let appear_screen = self.clone();
        let drop_screen = self.clone();
        let appear_state = state.clone();
        div()
            .id(self.child_id("root"))
            .relative()
            .size_full()
            .bg(sheet_bg)
            .track_focus(&focus_handle)
            .layer_keys(LayerCancel::dismiss(move |window, cx| {
                if !cancel_screen.editor_presentation.read(cx).is_presented {
                    (cancel_screen.on_cancel)(window, cx);
                }
            }))
            .on_key_down(move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                on_key.handle_key_down(event, &key_focus, window, cx);
            })
            .on_drop(move |paths: &ExternalPaths, _: &mut Window, cx: &mut App| {
                let urls = paths.paths().to_vec();
                if let Some(session) = &drop_session {
                    let _ = session.update(cx, |session, cx| session.receive_external_paths(urls, cx));
                } else if drop_fetcher.read(cx).reads_picked_files() {
                    drop_screen.add_picked_files(&drop_fetcher, &drop_selection, urls, cx);
                }
            })
            .child(content)
            .children(export_overlay)
            .on_appear(self.child_id("appear"), move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
                appear_screen.offer_files_on_appear(&appear_state, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

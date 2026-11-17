use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, Bounds, BoxShadow, Context, Edges, ElementId, Entity, FocusHandle, ImageId,
    KeyDownEvent, ObjectFit, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, Size,
    StyleRefinement, Subscription, Task, Window, black, hsla, img, point, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::RiseAppEvents;
use rise_platform::host_os::HostOs;
use rise_platform::photo_library::LibraryAsset;
use rise_platform::wheel_gesture::{
    LineBurst, LineBurstVerdict, WHEEL_SETTLE_DELAY, WheelGestureTracker, WheelStep, line_burst_step,
    line_burst_turn_lines,
};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::top_presenter::TopPresenter;
use rise_ui::window_cover::WindowCoverFrame;
use rise_ui::window_screen_frame_reader::WindowScreenFrameReader;

use crate::app::router::navigation_fullscreen_pop::NavigationInteractivePopGuard;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::media_viewer::media_viewer_types::{MediaViewerFrameKeys, MediaViewerHeroSkin, MediaViewerTransition};
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi};
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::chat::components::chat::bubble::media_letterbox_photo::MediaAspectFit;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::editor::editor_image_stage::{EditorImageStageDragChanged, EditorImageStageDragEnded};
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::modules::media_picker::editor::media_editor_pager::{
    MediaEditorPager, MediaEditorPagerNeighbours, MediaEditorPagerPage,
};
use crate::modules::media_picker::editor::media_editor_presentation_state::MediaEditorPresentationState;
use crate::modules::media_picker::editor::media_editor_screen::{MediaEditorOutcome, MediaEditorScreen};
use crate::modules::media_picker::editor::media_editor_window_presenter::{
    MediaEditorWindowLog, MediaEditorWindowOutcomeHandler, MediaEditorWindowPresenter,
};
use crate::modules::media_picker::library::media_asset::{MediaAsset, MediaAssetMediaType};
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;
use crate::modules::media_picker::selection::media_selection_state::{MediaSelectionEdit, MediaSelectionState};
use crate::modules::media_picker::ui::media_picker_grid_view::{
    MEDIA_PICKER_REFRESH_CELL_FRAMES, MEDIA_PICKER_REVEAL_ASSET, MediaPickerRevealAssetPayload,
};

const WHEEL_PAGE_TRAVEL: f32 = 48.0;

enum MediaEditorPresentationHostHeroMetrics {}

impl MediaEditorPresentationHostHeroMetrics {
    const MOVE_DURATION: f64 = MediaViewerTransition::MOVE_DURATION;
    const EDITOR_BOTTOM_INSET: f32 = 150.0;

    fn move_curve() -> MotionSpec {
        MediaViewerTransition::move_animation()
    }
}

/// the editor over the picker
/// hero flight out of the grid cell and back, dismiss drag and page turns
/// asset, frame and mode live in state so this only holds motion
#[derive(IntoElement)]
pub struct MediaEditorPresentationHost {
    state: Entity<MediaEditorPresentationState>,
    theme_store: Entity<ThemeStore>,
    configuration: MediaPickerConfiguration,
    selection: Entity<MediaSelectionState>,
    caption: Binding<String>,
    fetcher: Option<Entity<PhotoLibraryFetcher>>,
    on_outcome: MediaEditorWindowOutcomeHandler,
}

impl MediaEditorPresentationHost {
    pub fn new(
        state: Entity<MediaEditorPresentationState>,
        theme_store: Entity<ThemeStore>,
        configuration: MediaPickerConfiguration,
        selection: Entity<MediaSelectionState>,
        caption: Binding<String>,
        on_outcome: MediaEditorWindowOutcomeHandler,
    ) -> Self {
        Self { state, theme_store, configuration, selection, caption, fetcher: None, on_outcome }
    }

    pub fn fetcher(mut self, fetcher: Option<Entity<PhotoLibraryFetcher>>) -> Self {
        self.fetcher = fetcher;
        self
    }
}

impl RenderOnce for MediaEditorPresentationHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = ElementId::Name(SharedString::from(format!(
            "media-editor-presentation-host-{}",
            self.state.entity_id().as_u64()
        )));
        let view = window.use_keyed_state(key, cx, move |window, cx| MediaEditorPresentationHostView::new(self, window, cx));
        div().absolute().inset_0().child(view)
    }
}

struct MediaEditorPresentationHostView {
    state: Entity<MediaEditorPresentationState>,
    theme_store: Entity<ThemeStore>,
    configuration: MediaPickerConfiguration,
    selection: Entity<MediaSelectionState>,
    caption: Binding<String>,
    fetcher: Option<Entity<PhotoLibraryFetcher>>,
    on_outcome: MediaEditorWindowOutcomeHandler,

    open_progress: Animated<f32>,
    image_handoff_complete: bool,
    editor_chrome_opacity: Animated<f32>,
    backdrop_opacity: Animated<f32>,
    overlay_screen_frame: Bounds<Pixels>,
    is_closing_hero: bool,
    is_closing: bool,
    drag_translation: Animated<Size<Pixels>>,
    content_scale: Animated<f32>,
    vertical_drag_active: bool,
    host_drag_claimed: bool,
    pending_hero_open: bool,
    pager: Entity<MediaEditorPager>,
    screen_slot: Entity<MediaEditorScreenSlot>,
    is_page_settling: bool,
    page_offset: Animated<f32>,
    page_settle_generation: u64,
    wheel: WheelGestureTracker,
    wheel_line_burst: Option<LineBurst>,
    wheel_paged: bool,
    wheel_coasting: bool,
    wheel_settle: Option<Task<()>>,

    focus_handle: FocusHandle,
    focus_before_presentation: Option<FocusHandle>,
    last_is_presented: bool,
    last_editor_mode: EditorMode,
    last_focused_text_entity_id: Option<Uuid>,
    open_work: Option<Task<()>>,
    close_work: Option<Task<()>>,
    _observations: [Subscription; 3],
}

fn smoothstep(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn interpolate_rect(source: Bounds<Pixels>, target: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    let lerp = |a: Pixels, b: Pixels| a + (b - a) * t;
    Bounds::new(
        point(lerp(source.origin.x, target.origin.x), lerp(source.origin.y, target.origin.y)),
        size(lerp(source.size.width, target.size.width), lerp(source.size.height, target.size.height)),
    )
}

fn screen_rect_to_overlay_local(screen: Bounds<Pixels>, overlay_screen: Bounds<Pixels>) -> Bounds<Pixels> {
    if overlay_screen.size.width <= px(1.0) || overlay_screen.size.height <= px(1.0) {
        return screen;
    }
    Bounds::new(
        point(screen.origin.x - overlay_screen.origin.x, screen.origin.y - overlay_screen.origin.y),
        screen.size,
    )
}

fn element_id(prefix: &str, id: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("{prefix}-{id}")))
}

pub(crate) const MEDIA_EDITOR_BACKDROP: gpui::Hsla = hsla(0.0, 0.0, 0.11, 0.9);

pub(crate) const MEDIA_SEND_BACKDROP_BLUR: f32 = 24.0;

/// dim over the blur, the editors dark grey but lighter so the blurred app shows through
/// no blur means the editors own dim
pub(crate) fn media_send_backdrop(cx: &gpui::App) -> gpui::Hsla {
    if gpui::BACKDROP_BLUR_SUPPORTED && !rise_ui::swiftui::environment_values::accessibility_reduce_transparency(cx) {
        hsla(0.0, 0.0, 0.11, 0.72)
    } else {
        MEDIA_EDITOR_BACKDROP
    }
}

/// hairline around a media preview on a themed surface
/// so a black photo on the dark sheet or a white one on the light sheet still shows where it ends
pub(crate) fn media_edge_color(theme: &rise_theme::AppTheme) -> gpui::Hsla {
    theme.content_on(theme.bg._100).opacity(0.16)
}

impl MediaEditorPresentationHostView {
    fn new(host: MediaEditorPresentationHost, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let MediaEditorPresentationHost { state, theme_store, configuration, selection, caption, fetcher, on_outcome } =
            host;
        let pager = cx.new(|_| MediaEditorPager::new());
        let observations = [
            cx.observe_in(&state, window, |this: &mut Self, _, window, cx| this.state_did_change(window, cx)),
            cx.observe(&pager, |_, _, cx| cx.notify()),
            cx.on_release(|_, cx| NavigationInteractivePopGuard::set_media_editor_editing(false, cx)),
        ];
        let mut view = Self {
            state,
            theme_store,
            configuration,
            selection,
            caption,
            fetcher,
            on_outcome,
            open_progress: Animated::new(0.0),
            image_handoff_complete: false,
            editor_chrome_opacity: Animated::new(0.0),
            backdrop_opacity: Animated::new(0.0),
            overlay_screen_frame: Bounds::default(),
            is_closing_hero: false,
            is_closing: false,
            drag_translation: Animated::new(size(px(0.0), px(0.0))),
            content_scale: Animated::new(1.0),
            vertical_drag_active: false,
            host_drag_claimed: false,
            pending_hero_open: false,
            pager,
            screen_slot: cx.new(|_| MediaEditorScreenSlot { screen: None, props: None }),
            is_page_settling: false,
            page_offset: Animated::new(0.0),
            page_settle_generation: 0,
            wheel: WheelGestureTracker::new(),
            wheel_line_burst: None,
            wheel_paged: false,
            wheel_coasting: false,
            wheel_settle: None,
            focus_handle: cx.focus_handle(),
            focus_before_presentation: None,
            last_is_presented: false,
            last_editor_mode: EditorMode::Initial,
            last_focused_text_entity_id: None,
            open_work: None,
            close_work: None,
            _observations: observations,
        };
        view.state_did_change(window, cx);
        view
    }

    fn now() -> Duration {
        RiseClock::monotonic()
    }

    fn state_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (is_presented, editor_mode, focused_text_entity_id) = {
            let state = self.state.read(cx);
            (state.is_presented, state.editor_mode, state.focused_text_entity_id)
        };
        if is_presented != self.last_is_presented {
            self.last_is_presented = is_presented;
            self.reset_session_state(cx);
            if is_presented {
                self.focus_before_presentation = window.focused(cx);
                window.focus(&self.focus_handle, cx);
                self.pending_hero_open = true;
                self.try_start_hero_open(window, cx);
            } else if let Some(focus) = self.focus_before_presentation.take() {
                window.focus(&focus, cx);
            }
            self.sync_navigation_pop_guard(cx);
        }
        if editor_mode != self.last_editor_mode {
            self.last_editor_mode = editor_mode;
            if editor_mode != EditorMode::Initial {
                self.cancel_host_dismiss_drag(cx);
                self.pager.update(cx, |pager, cx| pager.clear_arrival(cx));
            }
            self.sync_navigation_pop_guard(cx);
        }
        if focused_text_entity_id != self.last_focused_text_entity_id {
            self.last_focused_text_entity_id = focused_text_entity_id;
            self.sync_navigation_pop_guard(cx);
        }
        cx.notify();
    }

    fn should_show_hero(&self, cx: &App) -> bool {
        if self.state.read(cx).source_frame.is_none() {
            return false;
        }
        if self.is_closing_hero {
            return true;
        }
        !self.image_handoff_complete
    }

    /// safe area insets plus the picker top bar band
    /// same as the editor screen stage insets so the hero lands where the editor draws
    fn safe_insets(cover: &WindowCoverFrame, shows_picker_top_bar: bool) -> Edges<f32> {
        let reported =
            EditorImageLayout::resolved_safe_area_insets(EdgeInsets::new(f32::from(cover.safe_top), 0.0, 0.0, 0.0));
        MediaEditorScreen::stage_safe_insets_for(reported, shows_picker_top_bar)
    }

    fn editor_insets(cover: &WindowCoverFrame, shows_picker_top_bar: bool) -> Edges<Pixels> {
        let safe = Self::safe_insets(cover, shows_picker_top_bar);
        Edges {
            top: px(safe.top + 12.0),
            left: px(0.0),
            bottom: px(safe.bottom + MediaEditorPresentationHostHeroMetrics::EDITOR_BOTTOM_INSET),
            right: px(0.0),
        }
    }

    fn editor_target_rect(
        cover: &WindowCoverFrame,
        image_size: Size<Pixels>,
        shows_picker_top_bar: bool,
    ) -> Bounds<Pixels> {
        MediaAspectFit::centered_fit_rect(image_size, cover.size, Self::editor_insets(cover, shows_picker_top_bar))
    }

    fn resolved_hero_image(&self, cx: &App) -> Option<CleverImage> {
        let state = self.state.read(cx);
        if let Some(warm) = &state.hero_warm_ui_image {
            return Some(warm.clone());
        }
        let url = &state.hero_skin.as_ref()?.display_source;
        Self::memory_image(url)
    }

    fn memory_image(url: &str) -> Option<CleverImage> {
        MediaViewerImageMemoryCache::image(
            url,
            SaiAccountCacheScope::active_user_id().as_deref(),
            CleverImageCachePolicy::Standard,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn editor_layer(
        &self,
        media_asset: &MediaAsset,
        ph_asset: &LibraryAsset,
        seed_image: Option<CleverImage>,
        image_stage_hidden: bool,
        chrome_opacity: f32,
        image_dismiss_offset: Size<Pixels>,
        image_dismiss_scale: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let existing = self.selection.read(cx).edit(&media_asset.id).cloned();
        let has_fetcher = self.fetcher.is_some();
        let arrived_by_page_turn = self.state.read(cx).arrived_by_page_turn;
        let slot_props = MediaEditorScreenSlotProps {
            asset_id: media_asset.id.clone(),
            initial_edit: existing.clone(),
            seed_image: seed_image.as_ref().map(|image| image.id),
            image_stage_hidden,
            chrome_opacity,
            image_dismiss_offset,
            image_dismiss_scale,
            has_fetcher,
            arrived_by_page_turn,
        };

        let this = cx.weak_entity();
        let on_drag_changed = this.clone();
        let on_drag_ended = this.clone();
        let on_page_changed = this.clone();
        let on_page_ended = this.clone();
        let on_cancel = this.clone();
        let on_done = this;
        let on_outcome = self.on_outcome.clone();
        let done_asset = media_asset.clone();

        let window_handle = window.window_handle();
        let dismiss_drag_changed: EditorImageStageDragChanged = Rc::new(move |dy: f32, cx: &mut App| {
            let _ = on_drag_changed.update(cx, |this, cx| this.update_dismiss_drag(dy, cx));
        });
        let dismiss_drag_ended: EditorImageStageDragEnded =
            Rc::new(move |dy: f32, velocity_y: f32, cx: &mut App| {
                let on_drag_ended = on_drag_ended.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = on_drag_ended.update(cx, |this, cx| this.finish_dismiss_drag(dy, velocity_y, window, cx));
                    });
                });
            });
        let page_drag_changed: Option<EditorImageStageDragChanged> = has_fetcher.then(|| {
            Rc::new(move |dx: f32, cx: &mut App| {
                let _ = on_page_changed.update(cx, |this, cx| this.update_page_drag(dx, cx));
            }) as EditorImageStageDragChanged
        });
        let page_drag_ended: Option<EditorImageStageDragEnded> = has_fetcher.then(|| {
            Rc::new(move |dx: f32, velocity_x: f32, cx: &mut App| {
                let on_page_ended = on_page_ended.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = on_page_ended.update(cx, |this, cx| this.finish_page_drag(dx, velocity_x, window, cx));
                    });
                });
            }) as EditorImageStageDragEnded
        });

        let screen = MediaEditorScreen::new(
            element_id("media-editor-screen", &media_asset.id),
            self.selection.clone(),
            self.caption.clone(),
            media_asset.clone(),
            Some(ph_asset.clone()),
            self.theme_store.clone(),
            self.configuration.clone(),
            existing,
            media_asset.media_type == MediaAssetMediaType::Video,
            move |window: &mut Window, cx: &mut App| {
                let _ = on_cancel.update(cx, |this, cx| this.animate_close_without_save(window, cx));
            },
            move |outcome: MediaEditorOutcome, window: &mut Window, cx: &mut App| {
                let immediate_send = outcome.immediate_send;
                MediaEditorWindowLog::event("editor-done", || format!("immediateSend={immediate_send}"), cx);
                if immediate_send {
                    let _ = on_done.update(cx, |this, cx| this.complete_flyback_handoff(cx));
                    let on_outcome = on_outcome.clone();
                    let asset = done_asset.clone();
                    MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| {
                        presenter.dismiss_immediately(
                            move |window, cx| on_outcome(outcome, asset, window, cx),
                            window,
                            cx,
                        );
                    });
                } else {
                    on_outcome(outcome, done_asset.clone(), window, cx);
                    let _ = on_done.update(cx, |this, cx| this.animate_close(window, cx));
                }
            },
        )
        .editor_presentation(Some(self.state.clone()))
        .embed_in_hero_host(true)
        .seed_hero_image(seed_image)
        .image_stage_hidden(image_stage_hidden)
        .chrome_opacity(chrome_opacity)
        .image_dismiss_offset(size(f32::from(image_dismiss_offset.width), f32::from(image_dismiss_offset.height)))
        .image_dismiss_scale(image_dismiss_scale)
        .on_host_dismiss_drag_changed(Some(dismiss_drag_changed))
        .on_host_dismiss_drag_ended(Some(dismiss_drag_ended))
        .pager(has_fetcher.then(|| self.pager.clone()))
        .on_page_drag_changed(page_drag_changed)
        .on_page_drag_ended(page_drag_ended)
        .shows_picker_top_bar(has_fetcher)
        .defers_source_load(!arrived_by_page_turn);

        let stale = self.screen_slot.update(cx, |slot, _| slot.set_props(screen, slot_props));
        let style = StyleRefinement::default().absolute().top_0().left_0().size_full();
        if stale {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(AnyView::from(self.screen_slot.clone()))
                .into_any_element()
        } else {
            AnyView::from(self.screen_slot.clone()).cached(style).into_any_element()
        }
    }

    fn page_width(&self, window: &Window) -> f32 {
        let width = f32::from(self.overlay_screen_frame.size.width);
        if width > 1.0 { width } else { f32::from(window.viewport_size().width) }
    }

    fn can_turn_pages(&self, cx: &App) -> bool {
        self.fetcher.is_some()
            && self.image_handoff_complete
            && !self.is_closing
            && !self.is_page_settling
            && !self.vertical_drag_active
            && self.state.read(cx).allows_dismiss_swipe()
    }

    fn set_pager_offset(&mut self, offset: f32, cx: &mut Context<Self>) {
        self.pager.update(cx, |pager, cx| {
            pager.offset = offset;
            cx.notify();
        });
    }

    fn update_page_drag(&mut self, dx: f32, cx: &mut Context<Self>) {
        if !self.can_turn_pages(cx) {
            return;
        }
        let has_target = {
            let pager = self.pager.read(cx);
            if dx > 0.0 { pager.previous().is_some() } else { pager.next().is_some() }
        };
        let offset = if has_target { dx } else { dx * 0.25 };
        self.page_offset.jump(offset);
        self.set_pager_offset(offset, cx);
    }

    fn finish_page_drag(&mut self, dx: f32, velocity_x: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_page_settling {
            return;
        }
        if !self.can_turn_pages(cx) {
            self.settle_page_back(window, cx);
            return;
        }
        let width = self.page_width(window);
        let (target, direction) = {
            let pager = self.pager.read(cx);
            if dx < 0.0 && (dx < -width * 0.5 || velocity_x < -500.0) && pager.next().is_some() {
                (pager.next().cloned(), -1.0)
            } else if dx > 0.0 && (dx > width * 0.5 || velocity_x > 500.0) && pager.previous().is_some() {
                (pager.previous().cloned(), 1.0)
            } else {
                (None, 0.0)
            }
        };
        let Some(target) = target else {
            self.settle_page_back(window, cx);
            return;
        };
        self.is_page_settling = true;
        self.page_offset.set_in(
            direction * (width + MediaEditorPager::PAGE_GAP),
            spring(0.3, 0.92),
            Self::now(),
            cx,
        );
        self.drive_page_offset(Some(target), window, cx);
    }

    fn settle_page_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pager.read(cx).offset == 0.0 {
            return;
        }
        self.page_offset.set_in(0.0, spring(0.3, 0.86), Self::now(), cx);
        self.drive_page_offset(None, window, cx);
    }

    /// runs the page settle, target is what the animation completion commits
    fn drive_page_offset(
        &mut self,
        target: Option<MediaEditorPagerPage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page_settle_generation = self.page_settle_generation.wrapping_add(1);
        self.schedule_page_offset_frame(self.page_settle_generation, target, window, cx);
    }

    fn schedule_page_offset_frame(
        &self,
        generation: u64,
        target: Option<MediaEditorPagerPage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, move |this, window, cx| {
            if this.page_settle_generation != generation {
                return;
            }
            let now = Self::now();
            let offset = this.page_offset.value(now);
            this.set_pager_offset(offset, cx);
            if this.page_offset.is_running(now) {
                this.schedule_page_offset_frame(generation, target, window, cx);
                return;
            }
            if let Some(target) = target {
                this.commit_page_turn(target, window, cx);
            }
        });
    }

    fn commit_page_turn(&mut self, target: MediaEditorPagerPage, window: &mut Window, cx: &mut Context<Self>) {
        let is_presented = self.state.read(cx).is_presented;
        let Some(fetcher) = self.fetcher.clone().filter(|_| is_presented && !self.is_closing) else {
            self.is_page_settling = false;
            return;
        };
        self.persist_current_edit(window, cx);
        let local_id = target.asset.id.clone();
        let registry_id = MediaViewerFrameKeys::picker_asset(&local_id);
        RiseAppEvents::default_center().post_with(
            MEDIA_PICKER_REVEAL_ASSET,
            MediaPickerRevealAssetPayload { local_id: local_id.clone(), index: target.index },
        );
        self.state.update(cx, |state, cx| {
            state.turn_page(
                MediaAsset::from_asset(&target.asset),
                target.asset.clone(),
                target.index,
                registry_id,
                MediaViewerHeroSkin::new(format!("ph://{local_id}"), false),
                target.image.clone(),
                cx,
            );
        });
        self.page_offset.jump(0.0);
        self.pager.update(cx, |pager, cx| {
            pager.offset = 0.0;
            pager.show_arrival(target.image.clone(), cx);
            pager.prepare_neighbours(&target.asset, Some(target.index), &fetcher, window, cx);
        });
        self.is_page_settling = false;
    }

    fn prepare_pager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fetcher) = self.fetcher.clone() else {
            return;
        };
        let (ph_asset, page_index) = {
            let state = self.state.read(cx);
            (state.ph_asset.clone(), state.page_index)
        };
        let Some(ph_asset) = ph_asset else {
            return;
        };
        self.pager.update(cx, |pager, cx| pager.prepare_neighbours(&ph_asset, page_index, &fetcher, window, cx));
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" if self.state.read(cx).allows_dismiss_swipe() => self.animate_close_without_save(window, cx),
            "left" if self.can_turn_pages(cx) => self.finish_page_drag(1.0, 501.0, window, cx),
            "right" if self.can_turn_pages(cx) => self.finish_page_drag(-1.0, -501.0, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_turn_pages(cx) && !self.is_page_settling {
            return;
        }
        if let ScrollDelta::Lines(lines) = event.delta {
            let along = if lines.x.abs() > lines.y.abs() { lines.x } else { lines.y };
            let turn_lines = line_burst_turn_lines(HostOs::current());
            let (burst, verdict) = line_burst_step(self.wheel_line_burst, along, 0.0, true, turn_lines, Self::now());
            self.wheel_line_burst = burst;
            match verdict {
                LineBurstVerdict::NotMine => return,
                LineBurstVerdict::Claimed => {}
                LineBurstVerdict::TurnBack => self.turn_page_from_wheel(1.0, window, cx),
                LineBurstVerdict::TurnForward => self.turn_page_from_wheel(-1.0, window, cx),
            }
            cx.stop_propagation();
            return;
        }
        let feed = self.wheel.feed(event, window.line_height());
        if self.apply_wheel_step(feed.step, window, cx) {
            cx.stop_propagation();
        }
        if let Some(token) = feed.settle {
            self.wheel_settle = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(WHEEL_SETTLE_DELAY).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(step) = this.wheel.settle(token) {
                        this.apply_wheel_step(step, window, cx);
                    }
                });
            }));
        }
    }

    /// says whether the event belongs to the pager and not to whatever lies under it
    fn apply_wheel_step(&mut self, step: WheelStep, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match step {
            WheelStep::Ignored => self.wheel_coasting,
            WheelStep::Moved(translation) => {
                self.wheel_coasting = false;
                if translation == Point::default() {
                    return false;
                }
                let (dx, dy) = (f32::from(translation.x), f32::from(translation.y));
                let along = if dx.abs() > dy.abs() { dx } else { dy };
                if !self.wheel_paged && along.abs() >= WHEEL_PAGE_TRAVEL {
                    self.wheel_paged = true;
                    self.turn_page_from_wheel(along.signum(), window, cx);
                }
                true
            }
            WheelStep::Ended(_) => {
                self.wheel_paged = false;
                self.wheel_coasting = true;
                true
            }
            WheelStep::Cancelled => {
                self.wheel_paged = false;
                true
            }
        }
    }

    /// positive direction turns back and negative forward
    /// the sign of a flick
    fn turn_page_from_wheel(&mut self, direction: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_turn_pages(cx) {
            self.finish_page_drag(direction, direction * 501.0, window, cx);
        }
    }

    fn dismiss_drag_allowed(&self, cx: &App) -> bool {
        self.image_handoff_complete && !self.is_closing && self.state.read(cx).allows_dismiss_swipe()
    }

    fn update_dismiss_drag(&mut self, dy: f32, cx: &mut Context<Self>) {
        if !self.dismiss_drag_allowed(cx) {
            return;
        }
        self.vertical_drag_active = true;
        self.set_covers_picker(false, cx);
        let down = dy.max(0.0);
        let progress = (down / 240.0).clamp(0.0, 1.0);
        self.drag_translation.jump(size(px(0.0), px(down)));
        self.content_scale.jump(1.0 - progress * 0.18);
        self.editor_chrome_opacity.jump(1.0 - progress * 0.95);
        self.backdrop_opacity.jump(1.0 - progress * 0.55);
        cx.notify();
    }

    fn finish_dismiss_drag(&mut self, dy: f32, velocity_y: f32, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dismiss_drag_allowed(cx) {
            self.cancel_host_dismiss_drag(cx);
            return;
        }
        self.vertical_drag_active = false;
        if dy > 130.0 || velocity_y > 720.0 {
            self.close_after_drag(true, window, cx);
        } else {
            self.animate_drag_back(spring(0.34, 0.86), cx);
        }
    }

    fn host_dismiss_gesture(&self, stage_rect: Bounds<Pixels>, cx: &Context<Self>) -> DragGesture {
        let changed = cx.weak_entity();
        let ended = changed.clone();
        DragGesture::new(22.0)
            .on_changed(move |value: &DragGestureValue, _, cx| {
                let dx = f32::from(value.translation.width).abs();
                let dy = f32::from(value.translation.height);
                let _ = changed.update(cx, |this, cx| {
                    if !this.dismiss_drag_allowed(cx) {
                        return;
                    }
                    if !this.host_drag_claimed {
                        if stage_rect.contains(&value.start_location) || dy <= 12.0 || dy <= dx * 0.72 {
                            return;
                        }
                        this.host_drag_claimed = true;
                    }
                    this.update_dismiss_drag(dy, cx);
                });
            })
            .on_ended(move |value: &DragGestureValue, window, cx| {
                let dy = f32::from(value.translation.height);
                let velocity_y = f32::from(value.predicted_end_translation.height) - dy;
                let _ = ended.update(cx, |this, cx| {
                    if !std::mem::take(&mut this.host_drag_claimed) {
                        return;
                    }
                    this.finish_dismiss_drag(dy, velocity_y, window, cx);
                });
            })
    }

    fn cancel_host_dismiss_drag(&mut self, cx: &mut Context<Self>) {
        self.vertical_drag_active = false;
        let now = Self::now();
        if *self.drag_translation.target() == size(px(0.0), px(0.0))
            && self.content_scale.value(now) == 1.0
            && self.editor_chrome_opacity.value(now) == 1.0
        {
            self.recover_picker_if_settled(cx);
            return;
        }
        self.animate_drag_back(spring(0.30, 0.85), cx);
    }

    fn animate_drag_back(&mut self, curve: MotionSpec, cx: &mut Context<Self>) {
        let now = Self::now();
        self.drag_translation.set_in(size(px(0.0), px(0.0)), curve, now, cx);
        self.content_scale.set_in(1.0, curve, now, cx);
        self.editor_chrome_opacity.set_in(1.0, curve, now, cx);
        self.backdrop_opacity.set_in(1.0, curve, now, cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(0.4)).await;
            let _ = this.update(cx, |this, cx| this.recover_picker_if_settled(cx));
        })
        .detach();
    }

    fn set_covers_picker(&self, covers: bool, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_covers_picker(covers, cx));
    }

    fn recover_picker_if_settled(&self, cx: &mut Context<Self>) {
        if self.image_handoff_complete && !self.is_closing && !self.vertical_drag_active {
            self.set_covers_picker(true, cx);
        }
    }

    fn sync_navigation_pop_guard(&self, cx: &mut App) {
        let blocks = self.state.read(cx).blocks_navigation_pop();
        NavigationInteractivePopGuard::set_media_editor_editing(blocks, cx);
    }

    fn hero_flight_layer(
        &self,
        source_screen: Bounds<Pixels>,
        target: Bounds<Pixels>,
        hero_image: Option<CleverImage>,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let now = Self::now();
        let source_local = screen_rect_to_overlay_local(source_screen, self.overlay_screen_frame);
        let url = {
            let state = self.state.read(cx);
            state.hero_skin.as_ref().map(|skin| skin.display_source.clone()).unwrap_or_else(|| {
                format!("ph://{}", state.media_asset.as_ref().map_or("", |asset| asset.id.as_str()))
            })
        };

        let t = self.open_progress.frame_value(now, window).clamp(0.0, 1.0);
        let u = smoothstep(t);
        let rect = interpolate_rect(source_local, target, u);
        let w = f32::from(rect.size.width).max(1.0);
        let h = f32::from(rect.size.height).max(1.0);
        let corner = 4.0 * (1.0 - u);
        let scale = self.content_scale.frame_value(now, window);
        let drag = self.drag_translation.frame_value(now, window);

        let flight = div()
            .absolute()
            .inset_0()
            .rounded(px(corner))
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1.0 - u)
                    .child(Self::hero_fill_content(hero_image.clone(), &url)),
            )
            .child(div().absolute().inset_0().opacity(u).child(Self::hero_fit_content(hero_image, &url)));

        let scaled_w = w * scale;
        let scaled_h = h * scale;
        let center_x = f32::from(rect.origin.x) + w / 2.0 + f32::from(drag.width);
        let center_y = f32::from(rect.origin.y) + h / 2.0 + f32::from(drag.height);
        let fade = 1.0 - u;
        let shadow = BoxShadow {
            color: black().opacity(0.28 * fade),
            offset: point(px(0.0), px(6.0 * fade)),
            blur_radius: px(14.0 * fade),
            spread_radius: px(0.0),
            inset: false,
        };
        div()
            .absolute()
            .left(px(center_x - scaled_w / 2.0))
            .top(px(center_y - scaled_h / 2.0))
            .w(px(scaled_w))
            .h(px(scaled_h))
            .shadow(vec![shadow])
            .child(flight)
            .into_any_element()
    }

    fn hero_fit_content(cached: Option<CleverImage>, url: &str) -> AnyElement {
        match cached {
            Some(cached) => img(cached).size_full().object_fit(ObjectFit::Contain).into_any_element(),
            None => CleverImageUi::new(url.to_owned())
                .id(element_id("media-editor-hero-fit", url))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .into_any_element(),
        }
    }

    fn hero_fill_content(cached: Option<CleverImage>, url: &str) -> AnyElement {
        let content = match cached {
            Some(cached) => img(cached).size_full().object_fit(ObjectFit::Cover).into_any_element(),
            None => CleverImageUi::new(url.to_owned())
                .id(element_id("media-editor-hero-fill", url))
                .content_mode(ContentMode::Fill)
                .corner_radius(0.0)
                .into_any_element(),
        };
        div().absolute().inset_0().bg(black()).overflow_hidden().child(content).into_any_element()
    }

    fn reset_session_state(&mut self, cx: &mut Context<Self>) {
        self.open_progress.jump(0.0);
        self.image_handoff_complete = false;
        self.editor_chrome_opacity.jump(0.0);
        self.backdrop_opacity.jump(0.0);
        self.is_closing_hero = false;
        self.is_closing = false;
        self.drag_translation.jump(size(px(0.0), px(0.0)));
        self.content_scale.jump(1.0);
        self.vertical_drag_active = false;
        self.pending_hero_open = false;
        self.is_page_settling = false;
        self.page_offset.jump(0.0);
        self.page_settle_generation = self.page_settle_generation.wrapping_add(1);
        self.open_work = None;
        self.close_work = None;
        self.pager.update(cx, |pager, cx| pager.reset(cx));
    }

    fn sync_source_frame_from_registry(&mut self, cx: &mut Context<Self>) {
        RiseAppEvents::default_center().post(MEDIA_PICKER_REFRESH_CELL_FRAMES);
        let fresh = {
            let state = self.state.read(cx);
            let Some(id) = &state.hero_lift_id else {
                return;
            };
            state.resolved_source_frame(id)
        };
        if let Some(fresh) = fresh
            && fresh.size.width > px(1.0)
            && fresh.size.height > px(1.0)
        {
            self.state.update(cx, |state, cx| state.update_source_frame(fresh, cx));
        }
    }

    fn overlay_frame_is_ready(&self) -> bool {
        self.overlay_screen_frame.size.width > px(1.0) && self.overlay_screen_frame.size.height > px(1.0)
    }

    fn try_start_hero_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending_hero_open
            || !self.state.read(cx).is_presented
            || !self.overlay_frame_is_ready()
            || *self.open_progress.target() != 0.0
            || self.image_handoff_complete
        {
            return;
        }
        self.pending_hero_open = false;
        self.sync_source_frame_from_registry(cx);
        self.animate_open(window, cx);
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.state.update(cx, |state, cx| state.set_hero_cell_hidden(true, cx));
            });
        })
        .detach();
    }

    fn animate_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_closing_hero = false;
        self.is_closing = false;
        self.image_handoff_complete = false;
        if self.state.read(cx).source_frame.is_none() {
            self.open_progress.jump(1.0);
            self.backdrop_opacity.jump(1.0);
            self.image_handoff_complete = true;
            self.set_covers_picker(true, cx);
            self.editor_chrome_opacity.jump(1.0);
            self.prepare_pager(window, cx);
            cx.notify();
            return;
        }
        self.editor_chrome_opacity.jump(0.0);
        self.open_progress.jump(0.0);
        self.backdrop_opacity.jump(0.0);
        cx.on_next_frame(window, |this, _, cx| {
            if !this.state.read(cx).is_presented {
                return;
            }
            let now = Self::now();
            let curve = MediaEditorPresentationHostHeroMetrics::move_curve();
            this.open_progress.set_in(1.0, curve, now, cx);
            this.backdrop_opacity.set_in(1.0, curve, now, cx);
            cx.notify();
        });
        self.open_work = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(MediaEditorPresentationHostHeroMetrics::MOVE_DURATION))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.read(cx).is_presented || this.is_closing {
                    return;
                }
                this.image_handoff_complete = true;
                if *this.backdrop_opacity.target() != 1.0 {
                    this.open_progress.jump(1.0);
                    this.backdrop_opacity.set_in(1.0, ease_out(0.2), Self::now(), cx);
                }
                this.editor_chrome_opacity.set_in(1.0, ease_out(0.2), Self::now(), cx);
                this.set_covers_picker(true, cx);
                this.prepare_pager(window, cx);
                cx.notify();
            });
        }));
    }

    fn can_fly_back_to_cell(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        state.hero_lift_id.as_ref().is_some_and(|id| state.resolved_source_frame(id).is_some())
    }

    fn refresh_flyback_source_frame(&mut self, cx: &mut Context<Self>) {
        self.sync_source_frame_from_registry(cx);
    }

    fn prepare_flyback_hero_bitmap(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.state.read(cx).hero_skin.as_ref().map(|skin| skin.display_source.clone()) else {
            return;
        };
        if let Some(cached) = Self::memory_image(&url) {
            self.state.update(cx, |state, cx| state.set_hero_warm_ui_image(Some(cached), cx));
        }
    }

    fn begin_flyback_close(&mut self, cx: &mut Context<Self>) {
        self.refresh_flyback_source_frame(cx);
        self.prepare_flyback_hero_bitmap(cx);
        self.is_closing_hero = true;
        self.image_handoff_complete = false;
        self.open_progress.jump(1.0);
        self.editor_chrome_opacity.set_in(0.0, ease_out(0.14), Self::now(), cx);
    }

    fn complete_flyback_handoff(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.complete_flyback_handoff(cx));
        self.reset_session_state(cx);
        cx.notify();
    }

    fn animate_close_without_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_after_drag(false, window, cx);
    }

    fn animate_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_after_drag(true, window, cx);
    }

    fn persist_current_edit(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handler) = self.state.read(cx).dismiss_persist_handler.clone() {
            let app: &mut App = cx;
            handler(window, app);
        }
    }

    fn close_after_drag(&mut self, persist_first: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (is_presented, allows_dismiss_swipe, editor_mode, has_focused_text) = {
            let state = self.state.read(cx);
            (
                state.is_presented,
                state.allows_dismiss_swipe(),
                state.editor_mode,
                state.focused_text_entity_id.is_some(),
            )
        };
        if self.is_closing || !is_presented || !allows_dismiss_swipe {
            let is_closing = self.is_closing;
            MediaEditorWindowLog::event(
                "close-blocked",
                || {
                    format!(
                        "isClosing={is_closing} presented={is_presented} allowsSwipe={allows_dismiss_swipe} \
                         mode={editor_mode:?} focusedText={has_focused_text}"
                    )
                },
                cx,
            );
            return;
        }
        if persist_first {
            self.persist_current_edit(window, cx);
        }
        self.is_closing = true;
        self.set_covers_picker(false, cx);

        let now = Self::now();
        let zero = size(px(0.0), px(0.0));
        let delay = if self.can_fly_back_to_cell(cx) {
            self.begin_flyback_close(cx);
            let curve = MediaEditorPresentationHostHeroMetrics::move_curve();
            self.open_progress.set_in(0.0, curve, now, cx);
            self.drag_translation.set_in(zero, curve, now, cx);
            self.content_scale.set_in(1.0, curve, now, cx);
            self.backdrop_opacity.set_in(0.0, curve, now, cx);
            MediaEditorPresentationHostHeroMetrics::MOVE_DURATION * 0.97
        } else {
            let curve = ease_out(0.24);
            self.drag_translation.set_in(zero, curve, now, cx);
            self.content_scale.set_in(1.0, curve, now, cx);
            self.editor_chrome_opacity.set_in(0.0, curve, now, cx);
            self.backdrop_opacity.set_in(0.0, curve, now, cx);
            0.26
        };
        self.close_work = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = this.update(cx, |this, cx| this.complete_flyback_handoff(cx));
        }));
        cx.notify();
    }
}

/// inputs of the editor screen that change between host renders
/// callbacks, bindings and entities are the same on every render of one presentation
#[derive(PartialEq)]
struct MediaEditorScreenSlotProps {
    asset_id: String,
    initial_edit: Option<MediaSelectionEdit>,
    seed_image: Option<ImageId>,
    image_stage_hidden: bool,
    chrome_opacity: f32,
    image_dismiss_offset: Size<Pixels>,
    image_dismiss_scale: f32,
    has_fetcher: bool,
    arrived_by_page_turn: bool,
}

/// holds the editor screen as a cached view so the hosts motion frames replay it
struct MediaEditorScreenSlot {
    screen: Option<MediaEditorScreen>,
    props: Option<MediaEditorScreenSlotProps>,
}

impl MediaEditorScreenSlot {
    fn set_props(&mut self, screen: MediaEditorScreen, props: MediaEditorScreenSlotProps) -> bool {
        self.screen = Some(screen);
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        self.props = Some(props);
        true
    }
}

impl Render for MediaEditorScreenSlot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.screen.clone())
    }
}

impl Render for MediaEditorPresentationHostView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (media_asset, ph_asset, source_frame) = {
            let state = self.state.read(cx);
            if !state.is_presented {
                return div().into_any_element();
            }
            (state.media_asset.clone(), state.ph_asset.clone(), state.source_frame)
        };
        let (Some(media_asset), Some(ph_asset)) = (media_asset, ph_asset) else {
            return div().into_any_element();
        };
        TopPresenter::note_presented("media-editor-top-presenter", window, cx);

        let now = Self::now();
        let cover = WindowCoverFrame::resolve(window, cx);
        let viewport = cover.size;
        let hero_image = self.resolved_hero_image(cx);
        let image_size = match &hero_image {
            Some(image) => {
                let pixels = image.size(0);
                size(px(pixels.width.0 as f32), px(pixels.height.0 as f32))
            }
            None => size(px(media_asset.pixel_width.max(1) as f32), px(media_asset.pixel_height.max(1) as f32)),
        };
        let target = Self::editor_target_rect(&cover, image_size, self.fetcher.is_some());
        let show_hero = self.should_show_hero(cx);
        let interactive = self.image_handoff_complete && !self.is_closing;

        let backdrop = div().absolute().inset_0().child(
            div()
                .w(viewport.width)
                .h(viewport.height)
                .bg(MEDIA_EDITOR_BACKDROP)
                .opacity(self.backdrop_opacity.frame_value(now, window))
                .gesture("media-editor-host-dismiss-drag", self.host_dismiss_gesture(target, cx)),
        );

        let neighbours = (self.fetcher.is_some() && self.image_handoff_complete && !self.is_closing).then(|| {
            div().absolute().inset_0().child(MediaEditorPagerNeighbours {
                pager: self.pager.clone(),
                size: size(f32::from(viewport.width), f32::from(viewport.height)),
                safe_insets: Self::safe_insets(&cover, true),
            })
        });

        let chrome_opacity = self.editor_chrome_opacity.frame_value(now, window);
        let image_dismiss_offset = self.drag_translation.frame_value(now, window);
        let image_dismiss_scale = self.content_scale.frame_value(now, window);
        let editor = self.editor_layer(
            &media_asset,
            &ph_asset,
            hero_image.clone(),
            show_hero,
            chrome_opacity,
            image_dismiss_offset,
            image_dismiss_scale,
            window,
            cx,
        );
        let hit_blocker = (!interactive).then(|| div().id("media-editor-hit-blocker").absolute().inset_0().occlude());

        let hero = source_frame
            .filter(|_| show_hero)
            .map(|source_screen| self.hero_flight_layer(source_screen, target, hero_image, window, cx));

        let frame_reader = cx.weak_entity();
        div()
            .id("media-editor-overlay")
            .track_focus(&self.focus_handle)
            .absolute()
            .inset_0()
            .occlude()
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .child(backdrop)
            .children(neighbours)
            .child(editor)
            .children(hit_blocker)
            .children(hero)
            .child(WindowScreenFrameReader::new("media-editor-root").on_change(move |screen, window, cx| {
                let _ = frame_reader.update(cx, |this, cx| {
                    if this.overlay_screen_frame == screen {
                        return;
                    }
                    this.overlay_screen_frame = screen;
                    this.try_start_hero_open(window, cx);
                    cx.notify();
                });
            }))
            .occludes_bottom_chrome()
            .into_any_element()
    }
}

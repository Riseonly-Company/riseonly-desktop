use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Background, Bounds, BoxShadow, ClickEvent, Corners, Edges, ElementId, Entity, FontWeight, Hsla, ImageSource,
    KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, Path, Pixels, Point, RenderImage, SharedString, Size, StyleRefinement, Subscription,
    Task, WeakEntity, Window, black, canvas, hsla, img, point, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_media::video::av_asset::AVURLAsset;
use rise_media::video::av_player::{AVLayerVideoGravity, AVPlayer, AVPlayerItem, AVPlayerLayer, AVPlayerTimeObserver};
use rise_media::video_export::probe as video_probe;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_ui::IconUi;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::core::media_viewer::media_viewer_video_scrubber::rounded_rect_path;
use crate::modules::chat::components::chat::bubble::web_m_sticker_playback::{
    WebMStickerFile, WebMStickerLayer, WebMStickerPlayback, WebMStickerTrackStore,
};
use crate::modules::chat::components::chat::bubble::media_letterbox_photo::MediaAspectFit;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_music_plate::EditorMusicPlateEntityView;
use crate::modules::media_picker::editor::editor_person_tag_entity_view::EditorPersonTagEntityView;
use crate::modules::media_picker::editor::editor_state::{
    EditorImageState, EditorMusicPlateEntity, EditorPersonTagEntity, EditorVideoState,
};
use crate::modules::media_picker::editor::editor_video_filmstrip_cache::EditorVideoFilmstripCache;
use crate::modules::media_picker::editor::editor_video_playback_controller::EditorVideoPlaybackController;

fn child_id(parent: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(parent.clone()), SharedString::new_static(name))
}

/// a native callback has no window but a binding write needs one, so use the hosts window
fn write_binding<T: 'static>(binding: &Binding<T>, value: T, handle: AnyWindowHandle, cx: &mut App) {
    let _ = handle.update(cx, |_, window, cx| binding.set(value, window, cx));
}

fn trim_shadow(color: Hsla, blur: f32, y: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color,
        offset: point(px(0.0), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.0),
        inset: false,
    }]
}

#[derive(Default)]
struct EditorVideoTrimViewLocalState {
    video_rect: Bounds<Pixels>,
    plate_selected: bool,
}

#[derive(IntoElement)]
pub struct EditorVideoTrimView {
    id: ElementId,
    asset: AVURLAsset,
    state: Binding<EditorVideoState>,
    is_playing: Binding<bool>,
    playback: Rc<EditorVideoPlaybackController>,
    shows_center_play_chrome: bool,
    center_play_insets: EdgeInsets,
    plate: Option<EditorMusicPlateEntity>,
    on_plate_update: Option<Rc<dyn Fn(EditorMusicPlateEntity, &mut App)>>,
    on_plate_delete: Option<Rc<dyn Fn(&mut App)>>,
    on_plate_press_changed: Option<Rc<dyn Fn(bool, &mut App)>>,
    person_tags: Vec<EditorPersonTagEntity>,
    on_person_tags_update: Option<Rc<dyn Fn(Vec<EditorPersonTagEntity>, &mut App)>>,
    on_background_tap: Option<ButtonAction>,
    on_ready_for_display: Option<Rc<dyn Fn(&mut App)>>,
}

impl EditorVideoTrimView {
    const SPACE_KEY: &'static str = "space";

    pub fn new(
        id: impl Into<ElementId>,
        asset: AVURLAsset,
        state: Binding<EditorVideoState>,
        is_playing: Binding<bool>,
        playback: Rc<EditorVideoPlaybackController>,
        shows_center_play_chrome: bool,
    ) -> Self {
        Self {
            id: id.into(),
            asset,
            state,
            is_playing,
            playback,
            shows_center_play_chrome,
            center_play_insets: EdgeInsets::new(0.0, 0.0, 0.0, 0.0),
            plate: None,
            on_plate_update: None,
            on_plate_delete: None,
            on_plate_press_changed: None,
            person_tags: Vec::new(),
            on_person_tags_update: None,
            on_background_tap: None,
            on_ready_for_display: None,
        }
    }

    pub fn on_ready_for_display(mut self, on_ready_for_display: impl Fn(&mut App) + 'static) -> Self {
        self.on_ready_for_display = Some(Rc::new(on_ready_for_display));
        self
    }

    pub fn center_play_insets(mut self, center_play_insets: EdgeInsets) -> Self {
        self.center_play_insets = center_play_insets;
        self
    }

    pub fn plate(mut self, plate: Option<EditorMusicPlateEntity>) -> Self {
        self.plate = plate;
        self
    }

    pub fn on_plate_update(mut self, on_plate_update: impl Fn(EditorMusicPlateEntity, &mut App) + 'static) -> Self {
        self.on_plate_update = Some(Rc::new(on_plate_update));
        self
    }

    pub fn on_plate_delete(mut self, on_plate_delete: impl Fn(&mut App) + 'static) -> Self {
        self.on_plate_delete = Some(Rc::new(on_plate_delete));
        self
    }

    /// press held on the card
    /// the screens drag over the player must not close or page meanwhile
    pub fn on_plate_press_changed(mut self, on_plate_press_changed: impl Fn(bool, &mut App) + 'static) -> Self {
        self.on_plate_press_changed = Some(Rc::new(on_plate_press_changed));
        self
    }

    pub fn person_tags(mut self, person_tags: Vec<EditorPersonTagEntity>) -> Self {
        self.person_tags = person_tags;
        self
    }

    pub fn on_person_tags_update(
        mut self,
        on_person_tags_update: impl Fn(Vec<EditorPersonTagEntity>, &mut App) + 'static,
    ) -> Self {
        self.on_person_tags_update = Some(Rc::new(on_person_tags_update));
        self
    }

    pub fn on_background_tap(mut self, on_background_tap: Option<ButtonAction>) -> Self {
        self.on_background_tap = on_background_tap;
        self
    }

    fn center_play_chrome(
        id: &ElementId,
        is_playing: bool,
        binding: Binding<bool>,
        insets: EdgeInsets,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let presence = window.use_keyed_state(child_id(id, "play-presence"), cx, |_, _| {
            Presence::new((!is_playing).then_some(()))
                .with_transition(Transition::opacity().combined(Transition::scale(0.92)))
        });
        let now = RiseClock::monotonic();
        presence.update(cx, |presence, cx| {
            presence.set_in((!is_playing).then_some(()), ease_out(0.18), now, cx);
        });

        let play_binding = binding.clone();
        let button = LiquidGlassButton::new(child_id(id, "play"), LiquidGlassButtonShape::Circle)
            .size(68.0)
            .on_press(move |window, cx| play_binding.set(true, window, cx))
            .child(
                div().relative().left(px(2.0)).children(IconUi::sized("play.fill", px(26.0), white(), true)),
            );
        let revealed = presence.read(cx).render_transitioned(now, window, |_| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .pt(px(insets.top))
                .pb(px(insets.bottom))
                .pl(px(insets.leading))
                .pr(px(insets.trailing))
                .child(button)
        });

        div()
            .absolute()
            .inset_0()
            .children(revealed)
            .when(is_playing, |this| {
                this.child(hit_area(Shape::Rectangle).id(child_id(id, "pause")).on_click(
                    move |_: &ClickEvent, window, cx| binding.set(false, window, cx),
                ))
            })
            .into_any_element()
    }
}

impl RenderOnce for EditorVideoTrimView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let local = window.use_keyed_state(child_id(&id, "local"), cx, |_, _| EditorVideoTrimViewLocalState::default());
        let focus = window.use_keyed_state(child_id(&id, "focus"), cx, |_, cx| cx.focus_handle()).read(cx).clone();
        let is_playing = self.is_playing.get(cx);
        let state = self.state.get(cx);
        let (video_rect, plate_selected) = {
            let local = local.read(cx);
            (local.video_rect, local.plate_selected)
        };
        let rect_is_usable = f32::from(video_rect.size.width) > 1.0 && f32::from(video_rect.size.height) > 1.0;
        let canvas_size = size(f32::from(video_rect.size.width), f32::from(video_rect.size.height));

        let rect_local = local.clone();
        let player = EditorVideoPlayerView::new(
            child_id(&id, "player"),
            self.asset,
            state,
            self.is_playing.clone(),
            self.playback,
        )
        .on_ready_for_display(self.on_ready_for_display)
        .on_video_rect_change(move |rect, cx| {
            rect_local.update(cx, |local, cx| {
                if local.video_rect == rect {
                    return;
                }
                local.video_rect = rect;
                cx.notify();
            });
        });

        let background_tap = self.on_background_tap.filter(|_| !is_playing && !plate_selected);
        let plate_layer = self.plate.filter(|_| rect_is_usable).map(|plate| {
            let binding_local = local.clone();
            let writing_local = local.clone();
            let is_selected = Binding::new(
                move |cx| binding_local.read(cx).plate_selected,
                move |value, _, cx| {
                    writing_local.update(cx, |local, cx| {
                        local.plate_selected = value;
                        cx.notify();
                    });
                },
            );
            let on_plate_update = self.on_plate_update.clone();
            let on_plate_delete = self.on_plate_delete.clone();
            let deleting_local = local.clone();
            EditorVideoPlateHost::new(child_id(&id, "plate-host"), plate, canvas_size, is_selected, move |plate, cx| {
                if let Some(on_plate_update) = &on_plate_update {
                    on_plate_update(plate, cx);
                }
            })
            .on_delete(move |cx| {
                deleting_local.update(cx, |local, cx| {
                    local.plate_selected = false;
                    cx.notify();
                });
                if let Some(on_plate_delete) = &on_plate_delete {
                    on_plate_delete(cx);
                }
            })
            .on_press_changed(self.on_plate_press_changed.clone())
        });
        let tags_layer = (!self.person_tags.is_empty() && rect_is_usable).then(|| {
            let on_person_tags_update = self.on_person_tags_update.clone();
            EditorVideoPersonTagsHost::new(child_id(&id, "tags-host"), self.person_tags, canvas_size, move |tags, cx| {
                if let Some(on_person_tags_update) = &on_person_tags_update {
                    on_person_tags_update(tags, cx);
                }
            })
        });
        let chrome = self.shows_center_play_chrome.then(|| {
            EditorVideoTrimView::center_play_chrome(
                &id,
                is_playing,
                self.is_playing.clone(),
                self.center_play_insets,
                window,
                cx,
            )
        });

        let placed = |content: AnyElement| {
            div()
                .absolute()
                .left(video_rect.origin.x)
                .top(video_rect.origin.y)
                .w(video_rect.size.width)
                .h(video_rect.size.height)
                .child(content)
        };
        let space_binding = self.is_playing;
        let deselect_local = local.clone();
        let page_focus = PageFocus::keyed_with(child_id(&id, "page-focus"), focus.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let appear_id = child_id(&id, "appear");
        div()
            .id(child_id(&id, "root"))
            .relative()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == EditorVideoTrimView::SPACE_KEY && !event.keystroke.modifiers.modified() {
                    space_binding.toggle(window, cx);
                }
            })
            .child(div().absolute().inset_0().child(player))
            .when_some(background_tap, |this, on_background_tap| {
                this.child(
                    hit_area(Shape::Rectangle)
                        .id(child_id(&id, "background-tap"))
                        .on_click(move |_: &ClickEvent, window, cx| on_background_tap(window, cx)),
                )
            })
            .when(plate_layer.is_some() && plate_selected, |this| {
                this.child(hit_area(Shape::Rectangle).id(child_id(&id, "plate-deselect")).on_click(
                    move |_: &ClickEvent, _, cx| {
                        deselect_local.update(cx, |local, cx| {
                            local.plate_selected = false;
                            cx.notify();
                        });
                    },
                ))
            })
            .children(chrome)
            .when_some(plate_layer, |this, plate_layer| this.child(placed(plate_layer.into_any_element())))
            .when_some(tags_layer, |this, tags_layer| this.child(placed(tags_layer.into_any_element())))
            .on_appear(appear_id, move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

#[derive(IntoElement)]
pub struct EditorVideoVolumeButton {
    pub is_muted: bool,
    on_open: ButtonAction,
}

impl EditorVideoVolumeButton {
    pub fn new(is_muted: bool, on_open: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { is_muted, on_open: Rc::new(on_open) }
    }
}

impl RenderOnce for EditorVideoVolumeButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_open = self.on_open;
        let symbol = if self.is_muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" };
        div().rounded_full().shadow(trim_shadow(black().opacity(0.25), 8.0, 2.0)).child(
            LiquidGlassButton::new("editor-video-volume-button", LiquidGlassButtonShape::Capsule)
                .horizontal_padding(14.0)
                .vertical_padding(10.0)
                .on_press(move |window, cx| on_open(window, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_color(white())
                        .children(IconUi::sized(symbol, px(15.0), white(), true))
                        .child(
                            div()
                                .swiftui_font(Font::system(13.0, FontWeight::SEMIBOLD, FontDesign::Default))
                                .child(tr("editor_sound_title")),
                        ),
                ),
        )
    }
}

#[derive(IntoElement)]
pub struct EditorVideoPlateHost {
    id: ElementId,
    pub plate: EditorMusicPlateEntity,
    pub canvas_size: Size<f32>,
    is_selected: Binding<bool>,
    on_update: Rc<dyn Fn(EditorMusicPlateEntity, &mut App)>,
    on_delete: Option<Rc<dyn Fn(&mut App)>>,
    on_press_changed: Option<Rc<dyn Fn(bool, &mut App)>>,
}

impl EditorVideoPlateHost {
    pub fn new(
        id: impl Into<ElementId>,
        plate: EditorMusicPlateEntity,
        canvas_size: Size<f32>,
        is_selected: Binding<bool>,
        on_update: impl Fn(EditorMusicPlateEntity, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            plate,
            canvas_size,
            is_selected,
            on_update: Rc::new(on_update),
            on_delete: None,
            on_press_changed: None,
        }
    }

    pub fn on_press_changed(mut self, on_press_changed: Option<Rc<dyn Fn(bool, &mut App)>>) -> Self {
        self.on_press_changed = on_press_changed;
        self
    }

    pub fn on_delete(mut self, on_delete: impl Fn(&mut App) + 'static) -> Self {
        self.on_delete = Some(Rc::new(on_delete));
        self
    }
}

impl RenderOnce for EditorVideoPlateHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let entity_id = self.plate.id;
        let view = window.use_keyed_state(self.id, cx, |_, cx| EditorMusicPlateEntityView::new(entity_id, cx));
        let handle = window.window_handle();
        let selected = self.is_selected.get(cx);
        let is_selected = self.is_selected;
        let on_update = self.on_update;
        let on_delete = self.on_delete;
        let on_press_changed = self.on_press_changed;
        let plate = self.plate;
        let canvas_size = self.canvas_size;
        view.update(cx, |view, cx| {
            view.on_update = Some(on_update);
            view.on_request_delete = on_delete.map(|on_delete| -> Rc<dyn Fn(Uuid, &mut App)> {
                Rc::new(move |_, cx: &mut App| on_delete(cx))
            });
            view.on_press_changed = on_press_changed;
            view.on_select = Some(Rc::new(move |_, cx: &mut App| {
                if is_selected.get(cx) {
                    return;
                }
                write_binding(&is_selected, true, handle, cx);
            }));
            view.set_interaction_state(selected, true, cx);
            view.apply(&plate, canvas_size, selected, cx);
        });
        div().relative().size_full().child(view)
    }
}

#[derive(Default)]
pub struct EditorVideoPersonTagsHostCoordinator {
    views: Vec<Entity<EditorPersonTagEntityView>>,
}

#[derive(IntoElement)]
pub struct EditorVideoPersonTagsHost {
    id: ElementId,
    pub tags: Vec<EditorPersonTagEntity>,
    pub canvas_size: Size<f32>,
    on_update: Rc<dyn Fn(Vec<EditorPersonTagEntity>, &mut App)>,
}

impl EditorVideoPersonTagsHost {
    pub fn new(
        id: impl Into<ElementId>,
        tags: Vec<EditorPersonTagEntity>,
        canvas_size: Size<f32>,
        on_update: impl Fn(Vec<EditorPersonTagEntity>, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), tags, canvas_size, on_update: Rc::new(on_update) }
    }

    fn rebuild(&self, coordinator: &Entity<EditorVideoPersonTagsHostCoordinator>, cx: &mut App) {
        let views: Vec<_> = self
            .tags
            .iter()
            .map(|entity| {
                let entity_id = entity.id;
                cx.new(|cx| EditorPersonTagEntityView::new(entity_id, cx))
            })
            .collect();
        coordinator.update(cx, |coordinator, _| coordinator.views = views);
    }

    fn apply(&self, views: &[Entity<EditorPersonTagEntityView>], cx: &mut App) {
        let tags = Rc::new(self.tags.clone());
        for view in views {
            let view_entity_id = view.read(cx).entity_id;
            let Some(entity) = self.tags.iter().find(|tag| tag.id == view_entity_id) else {
                continue;
            };
            let update_tags = Rc::clone(&tags);
            let update = Rc::clone(&self.on_update);
            let delete_tags = Rc::clone(&tags);
            let delete = Rc::clone(&self.on_update);
            let canvas_size = self.canvas_size;
            view.update(cx, |view, cx| {
                view.on_update = Some(Rc::new(move |updated: EditorPersonTagEntity, cx: &mut App| {
                    let mut next = (*update_tags).clone();
                    let Some(index) = next.iter().position(|tag| tag.id == updated.id) else {
                        return;
                    };
                    next[index] = updated;
                    update(next, cx);
                }));
                view.on_request_delete = Some(Rc::new(move |id, cx: &mut App| {
                    delete(delete_tags.iter().filter(|tag| tag.id != id).cloned().collect(), cx);
                }));
                view.set_interaction_state(true);
                view.apply(entity, canvas_size, false, cx);
            });
        }
    }
}

impl RenderOnce for EditorVideoPersonTagsHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator =
            window.use_keyed_state(self.id.clone(), cx, |_, _| EditorVideoPersonTagsHostCoordinator::default());
        let existing: Vec<_> = coordinator.read(cx).views.iter().map(|view| view.read(cx).entity_id).collect();
        let wanted: Vec<_> = self.tags.iter().map(|tag| tag.id).collect();
        if existing != wanted {
            self.rebuild(&coordinator, cx);
        }
        let views = coordinator.read(cx).views.clone();
        self.apply(&views, cx);
        div().relative().size_full().children(views)
    }
}

#[derive(Default)]
pub struct EditorVideoPlayerViewCoordinator {
    pub did_make_view: bool,
    pub did_configure_asset: bool,
}

#[derive(IntoElement)]
pub struct EditorVideoPlayerView {
    id: ElementId,
    pub asset: AVURLAsset,
    pub state: EditorVideoState,
    is_playing: Binding<bool>,
    pub playback: Rc<EditorVideoPlaybackController>,
    on_video_rect_change: Option<Rc<dyn Fn(Bounds<Pixels>, &mut App)>>,
    on_ready_for_display: Option<Rc<dyn Fn(&mut App)>>,
}

impl EditorVideoPlayerView {
    pub fn new(
        id: impl Into<ElementId>,
        asset: AVURLAsset,
        state: EditorVideoState,
        is_playing: Binding<bool>,
        playback: Rc<EditorVideoPlaybackController>,
    ) -> Self {
        Self {
            id: id.into(),
            asset,
            state,
            is_playing,
            playback,
            on_video_rect_change: None,
            on_ready_for_display: None,
        }
    }

    pub fn on_ready_for_display(mut self, on_ready_for_display: Option<Rc<dyn Fn(&mut App)>>) -> Self {
        self.on_ready_for_display = on_ready_for_display;
        self
    }

    pub fn on_video_rect_change(mut self, on_video_rect_change: impl Fn(Bounds<Pixels>, &mut App) + 'static) -> Self {
        self.on_video_rect_change = Some(Rc::new(on_video_rect_change));
        self
    }

    fn reached_end_callback(&self, handle: AnyWindowHandle) -> Rc<dyn Fn(&mut App)> {
        let is_playing = self.is_playing.clone();
        Rc::new(move |cx: &mut App| write_binding(&is_playing, false, handle, cx))
    }

    fn make_ui_view(&self, view: &Entity<EditorVideoPlayerUIView>, handle: AnyWindowHandle, cx: &mut App) {
        let on_reached_end = self.reached_end_callback(handle);
        let on_video_rect_change = self.on_video_rect_change.clone();
        let on_ready_for_display = self.on_ready_for_display.clone();
        view.update(cx, |view, _| {
            view.on_video_rect_change = on_video_rect_change;
            view.on_ready_for_display = on_ready_for_display;
            view.on_reached_end = Some(on_reached_end);
        });
        self.playback.player_view.replace(Some(view.downgrade()));
        let timeline = self.playback.timeline_view.borrow().as_ref().and_then(WeakEntity::upgrade);
        if let Some(timeline) = timeline {
            self.playback.wire(view, &timeline, cx);
        }
        let is_playing = self.is_playing.get(cx);
        view.update(cx, |view, cx| {
            view.configure(
                &self.asset,
                self.state.start_seconds,
                self.state.end_seconds,
                self.state.spatial_edit.as_ref(),
                cx,
            );
            view.set_video_volume(self.state.effective_video_volume(), cx);
            view.set_playing(is_playing, cx);
            view.seek(self.state.start_seconds, !is_playing, cx);
        });
    }

    fn update_ui_view(
        &self,
        view: &Entity<EditorVideoPlayerUIView>,
        coordinator: &Entity<EditorVideoPlayerViewCoordinator>,
        handle: AnyWindowHandle,
        cx: &mut App,
    ) {
        let is_playing = self.is_playing.get(cx);
        let on_reached_end = self.reached_end_callback(handle);
        let on_video_rect_change = self.on_video_rect_change.clone();
        view.update(cx, |view, cx| {
            view.on_reached_end = Some(on_reached_end);
            view.on_video_rect_change = on_video_rect_change;
            view.set_video_volume(self.state.effective_video_volume(), cx);
            view.set_loop_range(self.state.start_seconds, self.state.end_seconds);
            view.set_spatial_edit(self.state.spatial_edit.as_ref(), cx);
            view.set_playing(is_playing, cx);
        });

        if let Some(seek) = self.playback.pending_seek_seconds.get() {
            view.update(cx, |view, cx| view.seek(seek, !is_playing, cx));
            self.playback.pending_seek_seconds.set(None);
            return;
        }

        if !coordinator.read(cx).did_configure_asset {
            view.update(cx, |view, cx| {
                view.configure(
                    &self.asset,
                    self.state.start_seconds,
                    self.state.end_seconds,
                    self.state.spatial_edit.as_ref(),
                    cx,
                );
            });
            coordinator.update(cx, |coordinator, _| coordinator.did_configure_asset = true);
        }
    }
}

impl RenderOnce for EditorVideoPlayerView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = window
            .use_keyed_state(child_id(&self.id, "coordinator"), cx, |_, _| EditorVideoPlayerViewCoordinator::default());
        let view = window.use_keyed_state(self.id.clone(), cx, |_, cx| EditorVideoPlayerUIView::new(cx));
        let handle = window.window_handle();
        if coordinator.read(cx).did_make_view {
            self.update_ui_view(&view, &coordinator, handle, cx);
        } else {
            self.make_ui_view(&view, handle, cx);
            coordinator.update(cx, |coordinator, _| {
                coordinator.did_make_view = true;
                coordinator.did_configure_asset = true;
            });
        }
        div().size_full().child(view.cached(StyleRefinement::default().size_full()))
    }
}

pub struct EditorVideoPlayerUIView {
    player: Option<Entity<AVPlayer>>,
    player_item: Option<AVPlayerItem>,
    _player_subscription: Option<Subscription>,
    observer: Option<AVPlayerTimeObserver>,
    start_seconds: f64,
    end_seconds: f64,
    configured_asset_id: Option<AVURLAsset>,
    spatial_edit: Option<EditorImageState>,
    current_seconds: f64,
    pub on_time_update: Option<Rc<dyn Fn(f64, &mut App)>>,
    pub on_video_rect_change: Option<Rc<dyn Fn(Bounds<Pixels>, &mut App)>>,
    pub on_reached_end: Option<Rc<dyn Fn(&mut App)>>,
    pub on_ready_for_display: Option<Rc<dyn Fn(&mut App)>>,
    ready_observation: bool,
    last_reported_video_rect: Bounds<Pixels>,
    bounds_size: Size<Pixels>,
    composition_source: Option<Arc<RenderImage>>,
    composition_frame: Option<Arc<RenderImage>>,
    composition_generation: u64,
    composition_rendered_generation: u64,
    is_composition_rendering: bool,
    _composition_task: Option<Task<()>>,
    web_m: Option<EditorVideoWebMPreview>,
    _web_m_task: Option<Task<()>>,
}

/// looping sticker playback that stands in for the player on a webm source
struct EditorVideoWebMPreview {
    layer: Entity<WebMStickerLayer>,
    playback: WebMStickerPlayback,
    video_size: Size<Pixels>,
}

impl EditorVideoPlayerUIView {
    const PERIODIC_OBSERVER_SECONDS: f64 = 0.12;
    const FORWARD_BUFFER_SECONDS: f64 = 2.0;
    const SEEK_TOLERANCE_SECONDS: f64 = 0.03;
    const END_TOLERANCE_SECONDS: f64 = 0.05;
    const MINIMUM_WINDOW_SECONDS: f64 = 0.1;
    const MUTE_THRESHOLD: f64 = 0.0001;

    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|view, cx| view.teardown(cx)).detach();
        Self {
            player: None,
            player_item: None,
            _player_subscription: None,
            observer: None,
            start_seconds: 0.0,
            end_seconds: 0.0,
            configured_asset_id: None,
            spatial_edit: None,
            current_seconds: 0.0,
            on_time_update: None,
            on_video_rect_change: None,
            on_reached_end: None,
            on_ready_for_display: None,
            ready_observation: false,
            last_reported_video_rect: Bounds::default(),
            bounds_size: Size::default(),
            composition_source: None,
            composition_frame: None,
            composition_generation: 0,
            composition_rendered_generation: 0,
            is_composition_rendering: false,
            _composition_task: None,
            web_m: None,
            _web_m_task: None,
        }
    }

    pub fn current_seconds(&self) -> f64 {
        self.current_seconds
    }

    fn layout_subviews(&mut self, bounds_size: Size<Pixels>, cx: &mut Context<Self>) {
        self.bounds_size = bounds_size;
        let Some(video_size) = self.displayed_video_size(cx) else {
            return;
        };
        let rect = MediaAspectFit::centered_fit_rect(video_size, bounds_size, Edges::default());
        if f32::from(rect.size.width) <= 1.0
            || f32::from(rect.size.height) <= 1.0
            || rect == self.last_reported_video_rect
        {
            return;
        }
        self.last_reported_video_rect = rect;
        if let Some(on_video_rect_change) = self.on_video_rect_change.clone() {
            cx.defer(move |cx| on_video_rect_change(rect, cx));
        }
    }

    fn displayed_video_size(&self, cx: &App) -> Option<Size<Pixels>> {
        if self.composition_edit().is_some()
            && let Some(frame) = &self.composition_frame
        {
            let frame_size = frame.size(0);
            return Some(size(px(frame_size.width.0 as f32), px(frame_size.height.0 as f32)));
        }
        if let Some(web_m) = &self.web_m {
            return Some(web_m.video_size);
        }
        let presentation = self.player.as_ref()?.read(cx).presentation_size();
        (presentation.width > 0.0 && presentation.height > 0.0)
            .then(|| size(px(presentation.width), px(presentation.height)))
    }

    pub fn teardown(&mut self, cx: &mut App) {
        if let (Some(observer), Some(player)) = (self.observer.take(), self.player.as_ref()) {
            player.update(cx, |player, _| player.remove_time_observer(observer));
        }
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| {
                player.pause(cx);
                player.replace_current_item(None, cx);
            });
        }
        self._player_subscription = None;
        self.player_item = None;
        self._web_m_task = None;
        if let Some(mut web_m) = self.web_m.take() {
            web_m.playback.stop(cx);
        }
        self.configured_asset_id = None;
        self.spatial_edit = None;
        self._composition_task = None;
        self.composition_source = None;
        self.composition_frame = None;
        self.is_composition_rendering = false;
    }

    pub fn configure(
        &mut self,
        asset: &AVURLAsset,
        start: f64,
        end: f64,
        spatial_edit: Option<&EditorImageState>,
        cx: &mut Context<Self>,
    ) {
        if self.configured_asset_id.as_ref() == Some(asset) {
            self.set_loop_range(start, end);
            self.set_spatial_edit(spatial_edit, cx);
            return;
        }
        self.configured_asset_id = Some(asset.clone());
        if let (Some(observer), Some(player)) = (self.observer.take(), self.player.as_ref()) {
            player.update(cx, |player, _| player.remove_time_observer(observer));
        }

        if let Some(source) = asset.url().to_file_path().filter(|path| WebMStickerFile::is_web_m_path(path)) {
            self.configure_web_m(source, start, end, cx);
            return;
        }

        let mut item = AVPlayerItem::new(asset.url().clone());
        item.set_preferred_forward_buffer_duration(Duration::from_secs_f64(Self::FORWARD_BUFFER_SECONDS));
        self.spatial_edit = spatial_edit.cloned();
        self.composition_did_change(cx);
        self.composition_source = None;
        self.composition_frame = None;
        let player_item = item.clone();
        let player = cx.new(|cx| {
            let mut player = AVPlayer::new(cx);
            player.replace_current_item(Some(player_item), cx);
            player.set_automatically_waits_to_minimize_stalling(true, cx);
            player
        });
        self.ready_observation = false;
        self._player_subscription = Some(cx.observe(&player, |view, player, cx| {
            let is_ready = player.read(cx).is_ready_for_display();
            if is_ready && !view.ready_observation
                && let Some(on_ready_for_display) = view.on_ready_for_display.clone()
            {
                cx.defer(move |cx| on_ready_for_display(cx));
            }
            view.ready_observation = is_ready;
            cx.notify();
        }));
        self.player_item = Some(item);
        self.player = Some(player);
        self.set_loop_range(start, end);
        self.add_periodic_time_observer(cx);
        cx.notify();
    }

    fn configure_web_m(&mut self, source: std::path::PathBuf, start: f64, end: f64, cx: &mut Context<Self>) {
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| {
                player.pause(cx);
                player.replace_current_item(None, cx);
            });
        }
        self.player_item = None;
        self._player_subscription = None;
        if let Some(mut web_m) = self.web_m.take() {
            web_m.playback.stop(cx);
        }
        self.ready_observation = false;
        self.set_loop_range(start, end);
        let key = source.to_string_lossy().into_owned();
        self._web_m_task = Some(cx.spawn(async move |this, cx| {
            let Some(track) = WebMStickerTrackStore::track(source, key).await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                let video_size = size(px(track.width as f32), px(track.height as f32));
                let layer = cx.new(|_| WebMStickerLayer::new());
                let mut playback = WebMStickerPlayback::new(track, layer.downgrade());
                let weak = cx.weak_entity();
                playback.set_on_first_frame(Some(Rc::new(move |cx: &mut App| {
                    let _ = weak.update(cx, |view, cx| {
                        if view.ready_observation {
                            return;
                        }
                        view.ready_observation = true;
                        if let Some(on_ready_for_display) = view.on_ready_for_display.clone() {
                            cx.defer(move |cx| on_ready_for_display(cx));
                        }
                    });
                })));
                playback.play(cx);
                view.web_m = Some(EditorVideoWebMPreview { layer, playback, video_size });
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn set_spatial_edit(&mut self, spatial_edit: Option<&EditorImageState>, cx: &mut Context<Self>) {
        if self.spatial_edit.as_ref() == spatial_edit {
            return;
        }
        self.spatial_edit = spatial_edit.cloned();
        self.composition_did_change(cx);
        if self.player_item.is_none() {
            return;
        }
        if let Some(player) = self.player.clone() {
            player.update(cx, |player, cx| {
                let current_time = player.current_time().unwrap_or(0.0);
                player.seek(current_time, 0.0, 0.0, None, cx);
            });
        }
        cx.notify();
    }

    /// the edit a composition would render
    /// none for an edit with no pixel changes, then the player shows the plain track
    fn composition_edit(&self) -> Option<&EditorImageState> {
        self.spatial_edit.as_ref().filter(|edit| edit.has_baked_edits())
    }

    fn composition_did_change(&mut self, cx: &mut Context<Self>) {
        self.composition_generation = self.composition_generation.wrapping_add(1);
        if self.composition_edit().is_none() {
            self.composition_frame = None;
        }
        cx.notify();
    }

    fn pull_composition_frame(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.composition_edit().cloned() else {
            return;
        };
        if self.is_composition_rendering {
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        let buffer = player.update(cx, |player, _| player.copy_pixel_buffer_for_current_time());
        let source = self.composition_source.clone();
        let generation = self.composition_generation;
        if buffer.is_none() && (source.is_none() || self.composition_rendered_generation == generation) {
            return;
        }
        self.is_composition_rendering = true;
        self._composition_task = Some(cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_spawn(async move {
                    let source = match buffer {
                        Some(buffer) => buffer.oriented_image().map(Arc::new),
                        None => source,
                    }?;
                    let frame = EditorImageBake::render_edited_image(&source, &edit);
                    Some((source, frame))
                })
                .await;
            let _ = this.update(cx, |view, cx| view.composition_did_render(generation, rendered, cx));
        }));
    }

    fn composition_did_render(
        &mut self,
        generation: u64,
        rendered: Option<(Arc<RenderImage>, Arc<RenderImage>)>,
        cx: &mut Context<Self>,
    ) {
        self.is_composition_rendering = false;
        if let Some((source, frame)) = rendered {
            self.composition_source = Some(source);
            if self.composition_edit().is_some() {
                self.composition_frame = Some(frame);
                self.composition_rendered_generation = generation;
            }
        }
        cx.notify();
    }

    /// mute button silences only the video track
    /// editor music has its own player and isnt controlled here
    pub fn set_video_volume(&mut self, volume: f64, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        let clamped = volume.clamp(0.0, 1.0) as f32;
        let muted = volume <= Self::MUTE_THRESHOLD;
        player.update(cx, |player, cx| {
            if player.volume() != clamped {
                player.set_volume(clamped, cx);
            }
            if player.is_muted() != muted {
                player.set_is_muted(muted, cx);
            }
        });
    }

    /// live player time
    /// the time observer ticks every 120 ms which is jerky for the playhead
    /// so lanes poll the clock every frame
    pub fn live_playback_seconds(&self, cx: &App) -> Option<f64> {
        let seconds = self.player.as_ref()?.read(cx).current_time()?;
        seconds.is_finite().then_some(seconds)
    }

    pub fn is_actually_playing(&self, cx: &App) -> bool {
        self.player.as_ref().is_some_and(|player| player.read(cx).rate() > 0.0)
    }

    pub fn set_loop_range(&mut self, start: f64, end: f64) {
        self.start_seconds = start;
        self.end_seconds = (start + Self::MINIMUM_WINDOW_SECONDS).max(end);
    }

    pub fn set_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        if let Some(web_m) = self.web_m.as_mut() {
            if playing {
                web_m.playback.play(cx);
            } else {
                web_m.playback.pause();
            }
            return;
        }
        if !playing {
            if self.is_actually_playing(cx)
                && let Some(player) = self.player.clone()
            {
                player.update(cx, |player, cx| player.pause(cx));
            }
            return;
        }
        if self.current_seconds >= self.end_seconds - Self::END_TOLERANCE_SECONDS {
            self.seek(self.start_seconds, false, cx);
            return;
        }
        if !self.is_actually_playing(cx)
            && let Some(player) = self.player.clone()
        {
            player.update(cx, |player, cx| player.play_immediately(1.0, cx));
        }
    }

    pub fn seek(&mut self, to: f64, pause: bool, cx: &mut Context<Self>) {
        let clamped = to.max(self.start_seconds).min(self.end_seconds);
        self.current_seconds = clamped;
        if let Some(player) = self.player.clone() {
            player.update(cx, |player, cx| {
                player.seek(clamped, Self::SEEK_TOLERANCE_SECONDS, Self::SEEK_TOLERANCE_SECONDS, None, cx);
                if pause {
                    player.pause(cx);
                } else {
                    player.play_immediately(1.0, cx);
                }
            });
        }
        self.report_time(clamped, cx);
    }

    /// always outside this entitys update since the listener reads this view back
    fn report_time(&self, seconds: f64, cx: &mut Context<Self>) {
        if let Some(on_time_update) = self.on_time_update.clone() {
            cx.defer(move |cx| on_time_update(seconds, cx));
        }
    }

    fn add_periodic_time_observer(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        let view = cx.weak_entity();
        let block: Box<dyn Fn(f64, &mut App)> = Box::new(move |seconds, cx| {
            let _ = view.update(cx, |view, cx| view.periodic_time_did_fire(seconds, cx));
        });
        let interval = Duration::from_secs_f64(Self::PERIODIC_OBSERVER_SECONDS);
        self.observer = Some(player.update(cx, |player, cx| player.add_periodic_time_observer(interval, block, cx)));
    }

    fn periodic_time_did_fire(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if !seconds.is_finite() {
            return;
        }
        let Some(player) = self.player.clone() else {
            return;
        };
        self.current_seconds = seconds;
        self.report_time(seconds, cx);
        if seconds < self.end_seconds - Self::END_TOLERANCE_SECONDS {
            return;
        }
        let end_seconds = self.end_seconds;
        player.update(cx, |player, cx| {
            player.pause(cx);
            player.seek(end_seconds, Self::END_TOLERANCE_SECONDS, 0.0, None, cx);
        });
        self.current_seconds = end_seconds;
        self.report_time(end_seconds, cx);
        if let Some(on_reached_end) = self.on_reached_end.clone() {
            cx.defer(move |cx| on_reached_end(cx));
        }
    }
}

impl Render for EditorVideoPlayerUIView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pull_composition_frame(cx);
        let edited_frame = self.composition_frame.clone().filter(|_| self.composition_edit().is_some());
        let view = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .bg(black())
            .map(|this| match &self.web_m {
                Some(web_m) => this.child(div().absolute().inset_0().child(web_m.layer.clone())),
                None => this.child(
                    AVPlayerLayer::for_player(self.player.clone()).video_gravity(AVLayerVideoGravity::ResizeAspect),
                ),
            })
            .when_some(edited_frame, |this, frame| {
                this.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            let fitted = ObjectFit::Contain.get_bounds(bounds, frame.size(0));
                            let _ = window.paint_image(bounds, fitted, Corners::default(), frame, 0, false);
                        },
                    )
                    .absolute()
                    .inset_0()
                    .bg(black()),
                )
            })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = view.update(cx, |view, cx| view.layout_subviews(bounds.size, cx));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[derive(Default)]
pub struct EditorVideoTimelineRepresentableCoordinator {
    pub did_make_view: bool,
    pub last_width: f32,
}

#[derive(IntoElement)]
pub struct EditorVideoTimelineRepresentable {
    id: ElementId,
    pub asset: AVURLAsset,
    state: Binding<EditorVideoState>,
    pub playback: Rc<EditorVideoPlaybackController>,
    pub max_trim_duration_seconds: Option<f64>,
    on_interaction_started: Rc<dyn Fn(&mut App)>,
    on_trim_committed: Rc<dyn Fn(EditorVideoState, &mut App)>,
    on_request_volume: Rc<dyn Fn(&mut App)>,
}

impl EditorVideoTimelineRepresentable {
    pub fn new(
        id: impl Into<ElementId>,
        asset: AVURLAsset,
        state: Binding<EditorVideoState>,
        playback: Rc<EditorVideoPlaybackController>,
        on_interaction_started: impl Fn(&mut App) + 'static,
        on_trim_committed: impl Fn(EditorVideoState, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            asset,
            state,
            playback,
            max_trim_duration_seconds: None,
            on_interaction_started: Rc::new(on_interaction_started),
            on_trim_committed: Rc::new(on_trim_committed),
            on_request_volume: Rc::new(|_: &mut App| {}),
        }
    }

    pub fn max_trim_duration_seconds(mut self, max_trim_duration_seconds: Option<f64>) -> Self {
        self.max_trim_duration_seconds = max_trim_duration_seconds;
        self
    }

    pub fn on_request_volume(mut self, on_request_volume: impl Fn(&mut App) + 'static) -> Self {
        self.on_request_volume = Rc::new(on_request_volume);
        self
    }

    fn scrub_callback(&self) -> Rc<dyn Fn(f64, bool, &mut App)> {
        let playback = Rc::clone(&self.playback);
        Rc::new(move |seconds: f64, _: bool, cx: &mut App| {
            playback.pending_seek_seconds.set(Some(seconds));
            let player = playback.player_view.borrow().as_ref().and_then(WeakEntity::upgrade);
            if let Some(player) = player {
                player.update(cx, |player, cx| player.seek(seconds, true, cx));
            }
        })
    }

    fn make_ui_view(&self, view: &Entity<EditorVideoTimelineUIView>, handle: AnyWindowHandle, cx: &mut App) {
        self.playback.timeline_view.replace(Some(view.downgrade()));
        let player = self.playback.player_view.borrow().as_ref().and_then(WeakEntity::upgrade);
        if let Some(player) = player {
            self.playback.wire(&player, view, cx);
        }
        let state = self.state.get(cx);
        let on_trim_committed = Rc::clone(&self.on_trim_committed);
        let state_binding = self.state.clone();
        let on_interaction_started = Rc::clone(&self.on_interaction_started);
        let on_scrub = self.scrub_callback();
        let on_long_press = Rc::clone(&self.on_request_volume);
        view.update(cx, |view, cx| {
            view.configure(&self.asset, &state, self.max_trim_duration_seconds, cx);
            view.on_trim_committed = Some(on_trim_committed);
            view.on_trim_changed = Some(Rc::new(move |new_state: EditorVideoState, cx: &mut App| {
                write_binding(&state_binding, new_state, handle, cx);
            }));
            view.on_interaction_started = Some(on_interaction_started);
            view.on_scrub = Some(on_scrub);
            view.on_long_press = Some(on_long_press);
        });
    }

    fn update_ui_view(
        &self,
        view: &Entity<EditorVideoTimelineUIView>,
        coordinator: &Entity<EditorVideoTimelineRepresentableCoordinator>,
        cx: &mut App,
    ) {
        let state = self.state.get(cx);
        let on_interaction_started = Rc::clone(&self.on_interaction_started);
        let on_scrub = self.scrub_callback();
        let on_long_press = Rc::clone(&self.on_request_volume);
        view.update(cx, |view, _| {
            view.on_interaction_started = Some(on_interaction_started);
            view.on_scrub = Some(on_scrub);
            view.on_long_press = Some(on_long_press);
        });
        self.playback.video_window.set((state.start_seconds, state.end_seconds));
        self.playback.source_duration_seconds.set(state.duration);
        let width = view.read(cx).bounds_size.width;
        if (width - coordinator.read(cx).last_width).abs() > 1.0 {
            coordinator.update(cx, |coordinator, _| coordinator.last_width = width);
            view.update(cx, |view, cx| {
                view.configure(&self.asset, &state, self.max_trim_duration_seconds, cx);
            });
        } else {
            view.update(cx, |view, cx| view.apply_trim_state(&state, cx));
        }
    }
}

impl RenderOnce for EditorVideoTimelineRepresentable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = window.use_keyed_state(child_id(&self.id, "coordinator"), cx, |_, _| {
            EditorVideoTimelineRepresentableCoordinator::default()
        });
        let view = window.use_keyed_state(self.id.clone(), cx, |_, cx| EditorVideoTimelineUIView::new(cx));
        let handle = window.window_handle();
        if coordinator.read(cx).did_make_view {
            self.update_ui_view(&view, &coordinator, cx);
        } else {
            self.make_ui_view(&view, handle, cx);
            coordinator.update(cx, |coordinator, _| coordinator.did_make_view = true);
        }
        div().size_full().child(view.cached(StyleRefinement::default().size_full()))
    }
}

/// which pan a press on the lane belongs to, handles or the strip
#[derive(Clone, Copy, PartialEq, Debug)]
enum EditorVideoTimelinePan {
    LeftHandle,
    RightHandle,
    Scrub,
    Nothing,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct EditorVideoTimelineActivePan {
    target: EditorVideoTimelinePan,
    last_translation_x: f32,
}

/// chrome frames in view coordinates except the strips dimming
#[derive(Clone, Copy, PartialEq, Debug)]
struct EditorVideoTimelineChrome {
    lx: f32,
    rx: f32,
    window_width: f32,
    handle_width: f32,
    strip_height: f32,
    strip_width: f32,
}

pub struct EditorVideoTimelineUIView {
    asset: Option<AVURLAsset>,
    state: EditorVideoState,
    playhead_seconds: f64,
    filmstrip_token: u64,
    filmstrip_scheduled: bool,
    filmstrip_installed: bool,
    filmstrip_frames: Vec<Arc<RenderImage>>,

    max_trim_duration_seconds: Option<f64>,
    frame_step_seconds: f64,
    _frame_rate_task: Option<Task<()>>,

    pub on_trim_committed: Option<Rc<dyn Fn(EditorVideoState, &mut App)>>,
    pub on_trim_changed: Option<Rc<dyn Fn(EditorVideoState, &mut App)>>,
    pub on_interaction_started: Option<Rc<dyn Fn(&mut App)>>,
    pub on_scrub: Option<Rc<dyn Fn(f64, bool, &mut App)>>,
    pub on_long_press: Option<Rc<dyn Fn(&mut App)>>,

    selection_haptic: SelectionFeedbackGenerator,
    playhead_provider: Option<Rc<dyn Fn(&App) -> Option<f64>>>,
    playhead_painted: Rc<Cell<bool>>,
    playhead_step_armed: bool,
    bounds_origin: Point<Pixels>,
    bounds_size: Size<f32>,
    window_scale: f32,
    active_pan: Option<EditorVideoTimelineActivePan>,
}

impl EditorVideoTimelineUIView {
    const HANDLE_WIDTH: f32 = 11.0;
    const STRIP_CORNER_RADIUS: f32 = 8.0;
    const STRIP_VERTICAL_INSET: f32 = 4.0;
    const TIMELINE_INSET: f32 = 9.0;
    const FRAME_CORNER_RADIUS: f32 = 2.0;
    const SELECTION_BORDER_WIDTH: f32 = 2.0;
    const DIM_ALPHA: f32 = 0.62;
    const STRIP_WHITE: f32 = 0.12;
    const GRIP_ALPHA: f32 = 0.9;
    const GRIP_WIDTH: f32 = 2.0;
    const PLAYHEAD_WIDTH: f32 = 2.0;
    const PLAYHEAD_FOLLOW_EPSILON: f64 = 0.0005;
    const PLAYHEAD_SHADOW_SPREAD: f32 = 1.0;
    const PLAYHEAD_SHADOW_ALPHA: f32 = 0.2;
    const RIGHT_PREVIEW_INSET_SECONDS: f64 = 0.08;
    const DEFAULT_FRAME_STEP_SECONDS: f64 = 1.0 / 30.0;
    const PAN_MINIMUM_DISTANCE: f32 = 2.0;

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            asset: None,
            state: EditorVideoState::empty(None, 0.0),
            playhead_seconds: 0.0,
            filmstrip_token: 0,
            filmstrip_scheduled: false,
            filmstrip_installed: false,
            filmstrip_frames: Vec::new(),
            max_trim_duration_seconds: None,
            frame_step_seconds: Self::DEFAULT_FRAME_STEP_SECONDS,
            _frame_rate_task: None,
            on_trim_committed: None,
            on_trim_changed: None,
            on_interaction_started: None,
            on_scrub: None,
            on_long_press: None,
            selection_haptic: SelectionFeedbackGenerator::new(),
            playhead_provider: None,
            playhead_painted: Rc::new(Cell::new(false)),
            playhead_step_armed: false,
            bounds_origin: Point::default(),
            bounds_size: Size::default(),
            window_scale: 1.0,
            active_pan: None,
        }
    }

    fn handle_hold(&mut self, location: Point<f32>, cx: &mut Context<Self>) {
        if matches!(
            self.pan_target(location),
            EditorVideoTimelinePan::LeftHandle | EditorVideoTimelinePan::RightHandle
        ) {
            return;
        }
        self.play_selection_haptic();
        if let Some(on_long_press) = self.on_long_press.clone() {
            cx.defer(move |cx| on_long_press(cx));
        }
    }

    fn play_selection_haptic(&self) {
        let _ = self.selection_haptic.prepare();
        let _ = self.selection_haptic.selection_changed();
    }

    pub fn configure(
        &mut self,
        asset: &AVURLAsset,
        state: &EditorVideoState,
        max_trim_duration_seconds: Option<f64>,
        cx: &mut Context<Self>,
    ) {
        if self.asset.as_ref() != Some(asset) {
            self.filmstrip_installed = false;
            self.filmstrip_scheduled = false;
            self.filmstrip_frames.clear();
            self.load_frame_step(asset, cx);
        }
        self.asset = Some(asset.clone());
        self.max_trim_duration_seconds = max_trim_duration_seconds;
        self.state = self.clamped_trim_state(state);
        self.playhead_seconds = self.state.start_seconds;
        cx.notify();
        self.schedule_filmstrip_if_needed(cx);
    }

    fn load_frame_step(&mut self, asset: &AVURLAsset, cx: &mut Context<Self>) {
        let Some(source) = asset.url().to_file_path() else {
            return;
        };
        self._frame_rate_task = Some(cx.spawn(async move |this, cx| {
            let probed = RiseAccountRuntime::current().spawn_blocking(move || video_probe(&source)).await;
            let Ok(Ok(probe)) = probed else {
                return;
            };
            if probe.nominal_frame_rate <= 0.0 {
                return;
            }
            let _ = this.update(cx, |view, _| {
                view.frame_step_seconds = 1.0 / f64::from(probe.nominal_frame_rate);
            });
        }));
    }

    pub fn apply_trim_state(&mut self, state: &EditorVideoState, cx: &mut Context<Self>) {
        let next = self.clamped_trim_state(state);
        if next == self.state {
            return;
        }
        self.state = next;
        cx.notify();
    }

    fn clamped_trim_state(&self, state: &EditorVideoState) -> EditorVideoState {
        let mut s = state.clone();
        s.start_seconds = s.start_seconds.min(s.duration).max(0.0);
        s.end_seconds = s.end_seconds.min(s.duration).max(s.start_seconds);
        let minimum = self.minimum_trim_duration(s.duration);
        if s.end_seconds - s.start_seconds < minimum {
            s.end_seconds = s.duration.min(s.start_seconds + minimum);
        }
        if s.end_seconds - s.start_seconds < minimum {
            s.start_seconds = (s.end_seconds - minimum).max(0.0);
        }
        if let Some(maximum) = self.max_trim_duration_seconds.filter(|maximum| *maximum > 0.0) {
            if s.end_seconds - s.start_seconds > maximum {
                s.end_seconds = s.duration.min(s.start_seconds + maximum);
            }
            if s.end_seconds - s.start_seconds > maximum {
                s.start_seconds = (s.end_seconds - maximum).max(0.0);
            }
        }
        s
    }

    pub fn update_playhead(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if self.playhead_provider.is_some() && self.playhead_step_armed && self.playhead_painted.get() {
            return;
        }
        self.playhead_seconds = seconds;
        cx.notify();
    }

    pub fn follow_playhead(&mut self, provider: Rc<dyn Fn(&App) -> Option<f64>>, cx: &mut Context<Self>) {
        self.playhead_provider = Some(provider);
        cx.notify();
    }

    pub fn stop_following_playhead(&mut self) {
        self.playhead_provider = None;
    }

    fn step_playhead(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.playhead_provider.clone() else {
            return;
        };
        if let Some(seconds) = provider(cx) {
            self.playhead_seconds = seconds;
        }
        if !self.playhead_step_armed {
            self.playhead_step_armed = true;
            let weak = cx.weak_entity();
            window.on_next_frame(move |window, cx| Self::playhead_frame(weak, window, cx));
        }
    }

    fn playhead_frame(weak: WeakEntity<Self>, window: &mut Window, cx: &mut App) {
        let Some(view) = weak.upgrade() else {
            return;
        };
        let step = view.update(cx, |view, cx| {
            let seconds = view.playhead_provider.clone().and_then(|provider| provider(cx));
            let Some(seconds) = seconds else {
                view.playhead_step_armed = false;
                return None;
            };
            if (seconds - view.playhead_seconds).abs() <= Self::PLAYHEAD_FOLLOW_EPSILON {
                view.playhead_step_armed = false;
                return None;
            }
            view.playhead_seconds = seconds;
            let strip_height = view.playhead_strip_height();
            let shapes = view.layout_playhead_only().map(|x| Self::playhead_shapes(x, strip_height, Point::default()));
            Some((shapes, view.painted_key_base(cx)))
        });
        let Some((shapes, base)) = step else {
            return;
        };
        let swapped = view.read(cx).playhead_painted.get()
            && shapes.and_then(|shapes| Self::playhead_swaps(base, shapes)).is_some_and(|swaps| window.swap_paths(swaps));
        if swapped {
            window.on_next_frame(move |window, cx| Self::playhead_frame(weak, window, cx));
            return;
        }
        view.update(cx, |view, cx| {
            view.playhead_painted.set(false);
            view.playhead_step_armed = false;
            cx.notify();
        });
    }

    fn painted_key_base(&self, cx: &Context<Self>) -> u64 {
        cx.entity_id().as_u64()
    }

    fn playhead_strip_height(&self) -> f32 {
        (self.bounds_size.height - Self::STRIP_VERTICAL_INSET * 2.0).max(0.0)
    }

    fn playhead_swap_key(base: u64, shape: u64) -> u64 {
        base ^ (shape << 56)
    }

    /// shadow and body of the playhead at x, laid out from origin
    fn playhead_shapes(x: f32, strip_height: f32, origin: Point<Pixels>) -> (Option<Path<Pixels>>, Option<Path<Pixels>>) {
        let half = Self::PLAYHEAD_WIDTH / 2.0;
        let shadow = Self::PLAYHEAD_SHADOW_SPREAD;
        (
            rounded_rect_path(
                x - half - shadow,
                Self::STRIP_VERTICAL_INSET - shadow,
                Self::PLAYHEAD_WIDTH + shadow * 2.0,
                strip_height + shadow * 2.0,
                origin,
            ),
            rounded_rect_path(x - half, Self::STRIP_VERTICAL_INSET, Self::PLAYHEAD_WIDTH, strip_height, origin),
        )
    }

    fn playhead_swaps(
        base: u64,
        shapes: (Option<Path<Pixels>>, Option<Path<Pixels>>),
    ) -> Option<Vec<(u64, Path<Pixels>, Background)>> {
        Some(vec![
            (Self::playhead_swap_key(base, 1), shapes.0?, black().opacity(Self::PLAYHEAD_SHADOW_ALPHA).into()),
            (Self::playhead_swap_key(base, 2), shapes.1?, white().into()),
        ])
    }

    fn layout_subviews(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.bounds_origin = bounds.origin;
        let next = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
        if next == self.bounds_size {
            return;
        }
        self.bounds_size = next;
        self.schedule_filmstrip_if_needed(cx);
        let view = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = view.update(cx, |_, cx| cx.notify());
        });
    }

    fn layout_chrome(&self) -> Option<EditorVideoTimelineChrome> {
        let w = self.bounds_size.width;
        if self.state.duration <= 0.0 || w <= 1.0 {
            return None;
        }
        let strip_height = (self.bounds_size.height - Self::STRIP_VERTICAL_INSET * 2.0).max(0.0);
        let strip_width = (w - Self::TIMELINE_INSET * 2.0).max(1.0);
        let lx = Self::TIMELINE_INSET + (self.state.start_seconds / self.state.duration) as f32 * strip_width;
        let rx = Self::TIMELINE_INSET + (self.state.end_seconds / self.state.duration) as f32 * strip_width;
        let window_width = (rx - lx).max(0.0);
        let handle_width = Self::HANDLE_WIDTH.min((window_width / 2.0).max(1.0));
        Some(EditorVideoTimelineChrome { lx, rx, window_width, handle_width, strip_height, strip_width })
    }

    /// x of the playhead, none while hidden
    fn layout_playhead_only(&self) -> Option<f32> {
        if self.state.duration <= 0.0 || self.bounds_size.width <= 1.0 {
            return None;
        }
        if self.playhead_seconds < self.state.start_seconds || self.playhead_seconds > self.state.end_seconds {
            return None;
        }
        let w = (self.bounds_size.width - Self::TIMELINE_INSET * 2.0).max(1.0);
        Some(Self::TIMELINE_INSET + (self.playhead_seconds / self.state.duration) as f32 * w)
    }

    /// hit test
    /// right handle lies over the left, both over the strip
    /// only handles take a press inside the selection frame
    fn pan_target(&self, location: Point<f32>) -> EditorVideoTimelinePan {
        let Some(chrome) = self.layout_chrome() else {
            return EditorVideoTimelinePan::Nothing;
        };
        let left = Bounds::new(
            point(chrome.lx, Self::STRIP_VERTICAL_INSET),
            size(chrome.handle_width, chrome.strip_height),
        );
        let right = Bounds::new(
            point(chrome.rx - chrome.handle_width, Self::STRIP_VERTICAL_INSET),
            size(chrome.handle_width, chrome.strip_height),
        );
        if EditorVideoTrimHandleView::point_inside(right, location) {
            return EditorVideoTimelinePan::RightHandle;
        }
        if EditorVideoTrimHandleView::point_inside(left, location) {
            return EditorVideoTimelinePan::LeftHandle;
        }
        let strip = Bounds::new(
            point(Self::TIMELINE_INSET, Self::STRIP_VERTICAL_INSET),
            size(chrome.strip_width, chrome.strip_height),
        );
        if strip.contains(&location) {
            return EditorVideoTimelinePan::Scrub;
        }
        EditorVideoTimelinePan::Nothing
    }

    fn handle_pan(&mut self, value: &DragGestureValue, ended: bool, cx: &mut Context<Self>) {
        let began = self.active_pan.is_none();
        let active = self.active_pan.unwrap_or_else(|| EditorVideoTimelineActivePan {
            target: self.pan_target(point(
                f32::from(value.start_location.x),
                f32::from(value.start_location.y),
            )),
            last_translation_x: 0.0,
        });
        let translation_x = f32::from(value.translation.width);
        let delta_x = translation_x - active.last_translation_x;
        self.active_pan = (!ended).then_some(EditorVideoTimelineActivePan {
            target: active.target,
            last_translation_x: translation_x,
        });
        match active.target {
            EditorVideoTimelinePan::LeftHandle => self.pan_left(began, delta_x, ended, cx),
            EditorVideoTimelinePan::RightHandle => self.pan_right(began, delta_x, ended, cx),
            EditorVideoTimelinePan::Scrub => self.pan_scrub(began, f32::from(value.location.x), ended, cx),
            EditorVideoTimelinePan::Nothing => {}
        }
    }

    fn begin_pan(&self, cx: &mut Context<Self>) {
        self.play_selection_haptic();
        if let Some(on_interaction_started) = self.on_interaction_started.clone() {
            cx.defer(move |cx| on_interaction_started(cx));
        }
    }

    fn per_second(&self) -> f64 {
        f64::from((self.bounds_size.width - Self::TIMELINE_INSET * 2.0).max(1.0)) / self.state.duration
    }

    fn publish_trim(&mut self, scrub_seconds: f64, ended: bool, cx: &mut Context<Self>) {
        cx.notify();
        if let Some(on_trim_changed) = self.on_trim_changed.clone() {
            let state = self.state.clone();
            cx.defer(move |cx| on_trim_changed(state, cx));
        }
        self.publish_scrub(scrub_seconds, ended, cx);
        if ended {
            self.state.was_edited = true;
            if let Some(on_trim_committed) = self.on_trim_committed.clone() {
                let state = self.state.clone();
                cx.defer(move |cx| on_trim_committed(state, cx));
            }
        }
    }

    fn publish_scrub(&self, seconds: f64, ended: bool, cx: &mut Context<Self>) {
        if let Some(on_scrub) = self.on_scrub.clone() {
            cx.defer(move |cx| on_scrub(seconds, ended, cx));
        }
    }

    fn pan_left(&mut self, began: bool, delta_x: f32, ended: bool, cx: &mut Context<Self>) {
        if self.state.duration <= 0.0 {
            return;
        }
        if began {
            self.begin_pan(cx);
        }
        let minimum = self.minimum_trim_duration(self.state.duration);
        let maximum = self.max_trim_duration_seconds.filter(|maximum| *maximum > 0.0);
        let mut new_start = self.state.start_seconds + f64::from(delta_x) / self.per_second();
        new_start = new_start.min(self.state.end_seconds - minimum).max(0.0);
        if let Some(maximum) = maximum {
            new_start = new_start.max(self.state.end_seconds - maximum);
        }
        let snapped_start = self.snapped_to_frame(new_start);
        self.state.start_seconds = snapped_start.min(self.state.end_seconds - minimum).max(0.0);
        if let Some(maximum) = maximum {
            self.state.start_seconds = self.state.start_seconds.max(self.state.end_seconds - maximum);
        }
        if self.playhead_seconds < self.state.start_seconds {
            self.playhead_seconds = self.state.start_seconds;
        }
        self.publish_trim(self.state.start_seconds, ended, cx);
    }

    fn pan_right(&mut self, began: bool, delta_x: f32, ended: bool, cx: &mut Context<Self>) {
        if self.state.duration <= 0.0 {
            return;
        }
        if began {
            self.begin_pan(cx);
        }
        let minimum = self.minimum_trim_duration(self.state.duration);
        let maximum = self.max_trim_duration_seconds.filter(|maximum| *maximum > 0.0);
        let mut new_end = self.state.end_seconds + f64::from(delta_x) / self.per_second();
        new_end = new_end.min(self.state.duration).max(self.state.start_seconds + minimum);
        if let Some(maximum) = maximum {
            new_end = new_end.min(self.state.start_seconds + maximum);
        }
        let snapped_end = self.snapped_to_frame(new_end);
        self.state.end_seconds = snapped_end.min(self.state.duration).max(self.state.start_seconds + minimum);
        if let Some(maximum) = maximum {
            self.state.end_seconds = self.state.end_seconds.min(self.state.start_seconds + maximum);
        }
        if self.playhead_seconds > self.state.end_seconds {
            self.playhead_seconds = self.state.end_seconds;
        }
        let preview_seconds = self
            .state
            .start_seconds
            .max(self.state.end_seconds - Self::RIGHT_PREVIEW_INSET_SECONDS.min(minimum / 2.0));
        self.publish_trim(preview_seconds, ended, cx);
    }

    fn pan_scrub(&mut self, began: bool, location_x: f32, ended: bool, cx: &mut Context<Self>) {
        if self.state.duration <= 0.0 {
            return;
        }
        if began {
            self.begin_pan(cx);
        }
        let x = f64::from(location_x - Self::TIMELINE_INSET);
        let seconds = (x / self.per_second()).max(self.state.start_seconds).min(self.state.end_seconds);
        self.playhead_seconds = seconds;
        cx.notify();
        self.publish_scrub(seconds, ended, cx);
    }

    fn schedule_filmstrip_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.filmstrip_scheduled
            || self.filmstrip_installed
            || self.bounds_size.width <= 1.0
            || self.asset.is_none()
        {
            return;
        }
        if let Some(asset) = self.asset.clone()
            && let Some(cached) = EditorVideoFilmstripCache::cached_frames(&asset, EditorVideoFilmstripCache::FRAME_COUNT)
            && !cached.is_empty()
        {
            self.apply_filmstrip(cached, cx);
            return;
        }
        self.filmstrip_scheduled = true;
        self.filmstrip_token = self.filmstrip_token.wrapping_add(1);
        let token = self.filmstrip_token;
        let view = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = view.update(cx, |view, cx| {
                if view.filmstrip_token == token {
                    view.load_filmstrip(token, cx);
                }
            });
        });
    }

    fn load_filmstrip(&mut self, token: u64, cx: &mut Context<Self>) {
        let asset = self.asset.clone().filter(|_| self.bounds_size.width > 1.0);
        let Some(asset) = asset else {
            self.filmstrip_scheduled = false;
            return;
        };
        let count = EditorVideoFilmstripCache::FRAME_COUNT;
        if let Some(cached) = EditorVideoFilmstripCache::cached_frames(&asset, count) {
            self.apply_filmstrip(cached, cx);
            return;
        }
        let view = cx.weak_entity();
        EditorVideoFilmstripCache::load(
            &asset,
            count,
            self.window_scale,
            move |frames, cx| {
                let _ = view.update(cx, |view, cx| {
                    if view.filmstrip_token == token {
                        view.apply_filmstrip(frames, cx);
                    }
                });
            },
            cx,
        );
    }

    fn apply_filmstrip(&mut self, frames: Vec<Arc<RenderImage>>, cx: &mut Context<Self>) {
        self.filmstrip_scheduled = !frames.is_empty();
        self.filmstrip_installed = !frames.is_empty();
        self.filmstrip_frames = frames;
        cx.notify();
    }

    fn snapped_to_frame(&self, seconds: f64) -> f64 {
        if self.frame_step_seconds <= 0.0 {
            return seconds;
        }
        (seconds / self.frame_step_seconds).round() * self.frame_step_seconds
    }

    fn minimum_trim_duration(&self, duration: f64) -> f64 {
        if duration <= 0.0 {
            return 0.0;
        }
        let mut minimum = duration.min(1.0);
        if let Some(maximum) = self.max_trim_duration_seconds.filter(|maximum| *maximum > 0.0) {
            minimum = minimum.min(maximum);
        }
        duration.min(self.frame_step_seconds.max(minimum))
    }

    fn filmstrip_element(&self, chrome: Option<EditorVideoTimelineChrome>) -> impl IntoElement {
        let radius = px(Self::STRIP_CORNER_RADIUS);
        let strip_width = (self.bounds_size.width - Self::TIMELINE_INSET * 2.0).max(0.0);
        let strip_height = (self.bounds_size.height - Self::STRIP_VERTICAL_INSET * 2.0).max(0.0);
        let count = self.filmstrip_frames.len();
        let cell_width = if count > 0 { strip_width / count as f32 } else { 0.0 };
        let frames = self.filmstrip_frames.iter().enumerate().map(|(index, frame)| {
            img(ImageSource::Render(Arc::clone(frame)))
                .absolute()
                .left(px(index as f32 * cell_width))
                .top(px(0.0))
                .w(px(cell_width))
                .h(px(strip_height))
                .object_fit(ObjectFit::Cover)
                .rounded(px(Self::FRAME_CORNER_RADIUS))
                .when(index == 0, |this| this.rounded_l(radius))
                .when(index + 1 == count, |this| this.rounded_r(radius))
        });
        let dim = black().opacity(Self::DIM_ALPHA);
        let dims = chrome.map(|chrome| {
            let left_width = (chrome.lx - Self::TIMELINE_INSET).max(0.0);
            let right_x = (chrome.rx - Self::TIMELINE_INSET).max(0.0);
            let right_width = ((self.bounds_size.width - Self::TIMELINE_INSET) - chrome.rx).max(0.0);
            [
                div().absolute().left(px(0.0)).top(px(0.0)).w(px(left_width)).h(px(strip_height)).bg(dim).rounded_l(radius),
                div().absolute().left(px(right_x)).top(px(0.0)).w(px(right_width)).h(px(strip_height)).bg(dim).rounded_r(radius),
            ]
        });
        div()
            .absolute()
            .left(px(Self::TIMELINE_INSET))
            .top(px(Self::STRIP_VERTICAL_INSET))
            .w(px(strip_width))
            .h(px(strip_height))
            .bg(hsla(0.0, 0.0, Self::STRIP_WHITE, 1.0))
            .rounded(radius)
            .overflow_hidden()
            .children(frames)
            .children(dims.into_iter().flatten())
    }

    fn selection_frame_element(chrome: EditorVideoTimelineChrome) -> impl IntoElement {
        let radius = px(Self::STRIP_CORNER_RADIUS);
        let grip_height = (chrome.strip_height * 0.34).clamp(8.0, 16.0);
        let grip = || {
            div()
                .absolute()
                .left(px((chrome.handle_width - Self::GRIP_WIDTH) / 2.0))
                .top(px((chrome.strip_height - grip_height) / 2.0))
                .w(px(Self::GRIP_WIDTH))
                .h(px(grip_height))
                .rounded(px(1.0))
                .bg(hsla(0.0, 0.0, Self::STRIP_WHITE, Self::GRIP_ALPHA))
        };
        let handle = |x: f32| {
            div()
                .absolute()
                .left(px(x))
                .top(px(0.0))
                .w(px(chrome.handle_width))
                .h(px(chrome.strip_height))
                .bg(white())
                .cursor_col_resize()
                .child(grip())
        };
        div()
            .absolute()
            .left(px(chrome.lx))
            .top(px(Self::STRIP_VERTICAL_INSET))
            .w(px(chrome.window_width))
            .h(px(chrome.strip_height))
            .child(handle(0.0).rounded_l(radius))
            .child(handle(chrome.window_width - chrome.handle_width).rounded_r(radius))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .border(px(Self::SELECTION_BORDER_WIDTH))
                    .border_color(white())
                    .rounded(radius),
            )
    }
}

impl Render for EditorVideoTimelineUIView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window_scale = window.scale_factor();
        self.step_playhead(window, cx);
        let chrome = self.layout_chrome();
        let playhead_x = self.layout_playhead_only();
        if playhead_x.is_none() {
            self.playhead_painted.set(false);
        }
        let playhead_strip_height = self.playhead_strip_height();
        let playhead_base = self.painted_key_base(cx);
        let playhead_painted = self.playhead_painted.clone();

        let changed = cx.weak_entity();
        let ended = cx.weak_entity();
        let gesture = DragGesture::new(Self::PAN_MINIMUM_DISTANCE)
            .on_changed(move |value, _, cx| {
                let _ = changed.update(cx, |view, cx| view.handle_pan(value, false, cx));
            })
            .on_ended(move |value, _, cx| {
                let _ = ended.update(cx, |view, cx| view.handle_pan(value, true, cx));
            });
        let lane = div()
            .relative()
            .w(px(self.bounds_size.width))
            .h(px(self.bounds_size.height))
            .child(self.filmstrip_element(chrome))
            .when_some(chrome, |this, chrome| this.child(Self::selection_frame_element(chrome)))
            .when_some(playhead_x, |this, playhead_x| {
                this.child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _| {
                            let shapes = Self::playhead_shapes(playhead_x, playhead_strip_height, bounds.origin);
                            let Some(swaps) = Self::playhead_swaps(playhead_base, shapes) else {
                                playhead_painted.set(false);
                                return;
                            };
                            for (key, path, color) in swaps {
                                window.paint_swappable_path(path, color, key, bounds.origin);
                            }
                            playhead_painted.set(true);
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            });

        let measured = cx.weak_entity();
        let hold = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                let _ = hold.update(cx, |view, cx| {
                    let location = point(
                        f32::from(event.position.x - view.bounds_origin.x),
                        f32::from(event.position.y - view.bounds_origin.y),
                    );
                    view.handle_hold(location, cx);
                });
            })
            .child(lane.gesture(("editor-video-timeline-pan", cx.entity_id()), gesture))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = measured.update(cx, |view, cx| view.layout_subviews(bounds, cx));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

pub enum EditorTrimSelectionFrameView {}

impl EditorTrimSelectionFrameView {
    pub fn point_inside(left_handle: Bounds<f32>, right_handle: Bounds<f32>, location: Point<f32>) -> bool {
        EditorVideoTrimHandleView::point_inside(left_handle, location)
            || EditorVideoTrimHandleView::point_inside(right_handle, location)
    }
}

pub enum EditorVideoTrimHandleView {}

impl EditorVideoTrimHandleView {
    const HIT_OUTSET_X: f32 = 12.0;
    const HIT_OUTSET_Y: f32 = 8.0;

    pub fn point_inside(bounds: Bounds<f32>, location: Point<f32>) -> bool {
        let outset = Bounds::new(
            point(bounds.origin.x - Self::HIT_OUTSET_X, bounds.origin.y - Self::HIT_OUTSET_Y),
            size(bounds.size.width + Self::HIT_OUTSET_X * 2.0, bounds.size.height + Self::HIT_OUTSET_Y * 2.0),
        );
        outset.contains(&location)
    }
}

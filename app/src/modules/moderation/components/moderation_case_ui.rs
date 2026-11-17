use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, SharedString, Subscription, Window, anchored,
    black, deferred, white,
};
use rise_ui::window_cover::WindowCoverFrame;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerLayer, AVPlayerTimeControlStatus,
    AVPlayerTimeObserver,
};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::keyboard_dismiss::LayerFocus;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::slider::slider;

use super::moderation_kit_ui::{moderation_child_id, moderation_symbol};
use crate::core::lib::canon_format::RiseFormat;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions, RemoteMediaFileLease,
};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::modules::moderation::engine::core::report_evidence_item::ReportEvidenceItem;

const EVIDENCE_TILE_RADIUS: f32 = 14.0;
const EVIDENCE_COVER_LEVEL: usize = 1000;
const VIDEO_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const VIDEO_SEEK_TOLERANCE: f64 = 0.1;
const VIDEO_END_SLACK: f64 = 0.1;

/// block of someone elses words, small grey caption with the text inset on the page background
#[derive(IntoElement)]
pub struct ModerationQuoteBlock {
    title: SharedString,
    text: SharedString,
    background: Hsla,
    theme: AppTheme,
    title_color: Option<Hsla>,
}

impl ModerationQuoteBlock {
    pub fn new(
        title: impl Into<SharedString>,
        text: impl Into<SharedString>,
        background: Hsla,
        theme: AppTheme,
    ) -> Self {
        Self { title: title.into(), text: text.into(), background, theme, title_color: None }
    }

    pub fn title_color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.title_color = color.into();
        self
    }
}

impl RenderOnce for ModerationQuoteBlock {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(MainText::new(self.title).px(12.0).color(self.title_color.unwrap_or(self.theme.text.secondary)))
            .child(
                Shape::continuous(EVIDENCE_TILE_RADIUS)
                    .round(div().w_full().p(px(12.0)).bg(self.background).overflow_hidden())
                    .child(MainText::new(self.text).px(14.0).color(self.theme.text.primary)),
            )
    }
}

struct RemoteEvidenceStripState {
    opened: Option<ReportEvidenceItem>,
}

/// uploaded evidence as a row of thumbnails that open full screen
#[derive(IntoElement)]
pub struct RemoteEvidenceStrip {
    id: ElementId,
    items: Vec<ReportEvidenceItem>,
    size: f32,
}

impl RemoteEvidenceStrip {
    pub fn new(id: impl Into<ElementId>, items: Vec<ReportEvidenceItem>) -> Self {
        Self { id: id.into(), items, size: 92.0 }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    fn tile(&self, item: &ReportEvidenceItem, theme: &AppTheme) -> AnyElement {
        let tile = Shape::continuous(EVIDENCE_TILE_RADIUS)
            .round(div().relative().flex_none().overflow_hidden())
            .frame_size(self.size, self.size);
        if item.is_video {
            return tile
                .bg(theme.bg._300)
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("play.circle.fill", px(24.0), theme.text.secondary, true))
                .into_any_element();
        }
        if let Some(source) = &item.absolute_url {
            return tile
                .child(
                    CleverImageUi::new(source.clone())
                        .id(moderation_child_id(&self.id, item.id.clone()))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(EVIDENCE_TILE_RADIUS)
                        .persist_to_disk(false)
                        .maximum_decode_pixel_dimension(320_i64)
                        .cache_policy(CleverImageCachePolicy::MemoryOnly),
                )
                .into_any_element();
        }
        tile.bg(theme.bg._300)
            .flex()
            .items_center()
            .justify_center()
            .children(moderation_symbol("photo", 17.0, theme.text.secondary))
            .into_any_element()
    }
}

impl RenderOnce for RemoteEvidenceStrip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(moderation_child_id(&self.id, "state"), cx, |_, _| {
            RemoteEvidenceStripState { opened: None }
        });
        let opened = state.read(cx).opened.clone();

        let tiles: Vec<AnyElement> = self
            .items
            .iter()
            .map(|item| {
                let press_state = state.clone();
                let press_item = item.clone();
                plain(ElementId::NamedChild(Arc::new(self.id.clone()), format!("tile-{}", item.id).into()))
                    .flex_none()
                    .on_press(move |_, cx| {
                        let item = press_item.clone();
                        press_state.update(cx, |state, cx| {
                            state.opened = Some(item);
                            cx.notify();
                        });
                    })
                    .child(self.tile(item, &theme))
                    .into_any_element()
            })
            .collect();

        let cover = opened.map(|item| {
            let cover = WindowCoverFrame::resolve(window, cx);
            let select_state = state.clone();
            let dismiss_state = state.clone();
            deferred(
                anchored().position(cover.origin).child(
                    div().relative().w(cover.size.width).h(cover.size.height).occlude().child(
                        ModerationEvidenceViewer::new(moderation_child_id(&self.id, "viewer"), item, self.items.clone())
                            .on_select(move |item, _, cx| {
                                select_state.update(cx, |state, cx| {
                                    state.opened = Some(item);
                                    cx.notify();
                                });
                            })
                            .on_dismiss(move |_, cx| {
                                dismiss_state.update(cx, |state, cx| {
                                    state.opened = None;
                                    cx.notify();
                                });
                            }),
                    ),
                ),
            )
            .with_priority(EVIDENCE_COVER_LEVEL)
        });

        div()
            .id(moderation_child_id(&self.id, "scroll"))
            .self_stretch()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(hstack(8.0).w_full().flex_wrap().children(tiles))
            .children(cover)
    }
}

pub type ModerationEvidenceSelect = Rc<dyn Fn(ReportEvidenceItem, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ModerationEvidenceViewer {
    id: ElementId,
    item: ReportEvidenceItem,
    items: Vec<ReportEvidenceItem>,
    on_select: ModerationEvidenceSelect,
    on_dismiss: ButtonAction,
}

impl ModerationEvidenceViewer {
    pub fn new(id: impl Into<ElementId>, item: ReportEvidenceItem, items: Vec<ReportEvidenceItem>) -> Self {
        Self { id: id.into(), item, items, on_select: Rc::new(|_, _, _| {}), on_dismiss: Rc::new(|_, _| {}) }
    }

    pub fn on_select(mut self, action: impl Fn(ReportEvidenceItem, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Rc::new(action);
        self
    }

    pub fn on_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(action);
        self
    }

    fn neighbour(items: &[ReportEvidenceItem], item_id: &str, forward: bool) -> Option<ReportEvidenceItem> {
        let position = items.iter().position(|candidate| candidate.id == item_id)?;
        let next = if forward { position.checked_add(1)? } else { position.checked_sub(1)? };
        items.get(next).cloned()
    }
}

impl RenderOnce for ModerationEvidenceViewer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state::<FocusHandle>(moderation_child_id(&self.id, "focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let layer_focus = LayerFocus::keyed(moderation_child_id(&self.id, "focus-return"), window, cx);
        let (layer_appearing, layer_leaving) = (layer_focus.clone(), layer_focus);
        let appearing = focus.clone();
        let leaving = focus.clone();
        let key_focus = focus.clone();

        let content = if self.item.is_video {
            ModerationEvidenceVideo { id: moderation_child_id(&self.id, self.item.id.clone()), item: self.item.clone() }
                .into_any_element()
        } else if let Some(source) = &self.item.absolute_url {
            CleverImageUi::new(source.clone())
                .id(moderation_child_id(&self.id, self.item.id.clone()))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .persist_to_disk(false)
                .cache_policy(CleverImageCachePolicy::MemoryOnly)
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let close_action = self.on_dismiss.clone();
        let key_dismiss = self.on_dismiss;
        let on_select = self.on_select;
        let items = self.items;
        let item_id = self.item.id;

        div()
            .id(moderation_child_id(&self.id, "cover"))
            .relative()
            .size_full()
            .bg(black())
            .track_focus(&focus)
            .layer_keys(LayerCancel::Dismiss(key_dismiss))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let forward = match event.keystroke.key.as_str() {
                    "right" => true,
                    "left" => false,
                    _ => return,
                };
                cx.stop_propagation();
                if let Some(next) = Self::neighbour(&items, &item_id, forward) {
                    window.focus(&key_focus, cx);
                    on_select(next, window, cx);
                }
            })
            .child(div().absolute().inset_0().flex().items_center().justify_center().child(content))
            .child(
                plain(moderation_child_id(&self.id, "close"))
                    .absolute()
                    .top(px(16.0))
                    .right(px(16.0))
                    .size(px(36.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(black().opacity(0.45))
                    .rounded_full()
                    .on_press(move |window, cx| close_action(window, cx))
                    .children(moderation_symbol("xmark", 15.0, white())),
            )
            .on_appear(moderation_child_id(&self.id, "focus-appear"), move |window, cx| {
                LayerFocus::did_appear(&layer_appearing, &appearing, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    LayerFocus::will_disappear(&layer_leaving, &leaving, window, cx);
                }
            })
    }
}

/// player and failed state of the viewer, kept per clip
#[derive(Default)]
struct ModerationEvidenceVideoState {
    player: Option<Entity<AVPlayer>>,
    failed: bool,
    _lease: Option<RemoteMediaFileLease>,
    _time_observer: Option<AVPlayerTimeObserver>,
    _player_observation: Option<Subscription>,
    progress: f64,
    duration: f64,
    is_scrubbing: bool,
}

impl ModerationEvidenceVideoState {
    fn install(&mut self, local: PathBuf, cx: &mut Context<Self>) {
        let Some(url) = Url::from_file_path(&local) else {
            self.failed = true;
            cx.notify();
            return;
        };
        self._lease = RemoteMediaDiskCache::lease(Some(local.as_path()));
        let player = cx.new(AVPlayer::new);
        let weak = cx.weak_entity();
        let time_observer = player.update(cx, |player, cx| {
            player.replace_current_item(Some(AVPlayerItem::new(url)), cx);
            player.add_periodic_time_observer(
                VIDEO_PROGRESS_INTERVAL,
                Box::new(move |seconds, cx| {
                    let _ = weak.update(cx, |state, cx| state.time_did_change(seconds, cx));
                }),
                cx,
            )
        });
        self._time_observer = Some(time_observer);
        self._player_observation =
            Some(cx.observe(&player, |state, player, cx| state.player_did_change(&player, cx)));
        self.player_did_change(&player, cx);
        self.player = Some(player);
    }

    fn player_did_change(&mut self, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        let (status, duration) = {
            let player = player.read(cx);
            (player.current_item_status(), player.current_item_duration())
        };
        if status == AVPlayerItemStatus::Failed {
            self.failed = true;
        }
        if let Some(duration) = duration.filter(|seconds| seconds.is_finite()) {
            self.duration = duration.max(0.0);
        }
        cx.notify();
    }

    fn time_did_change(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if self.is_scrubbing || !seconds.is_finite() {
            return;
        }
        self.progress = seconds.max(0.0);
        cx.notify();
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else {
            return;
        };
        let is_paused = player.read(cx).time_control_status() == AVPlayerTimeControlStatus::Paused;
        let restart = is_paused && self.duration > 0.0 && self.progress >= self.duration - VIDEO_END_SLACK;
        if restart {
            self.progress = 0.0;
        }
        player.update(cx, |player, cx| {
            if restart {
                player.seek(0.0, VIDEO_SEEK_TOLERANCE, VIDEO_SEEK_TOLERANCE, None, cx);
            }
            if is_paused {
                player.play(cx);
            } else {
                player.pause(cx);
            }
        });
        cx.notify();
    }

    fn scrub(&mut self, seconds: f64, cx: &mut Context<Self>) {
        self.progress = seconds;
        if let Some(player) = self.player.clone() {
            player.update(cx, |player, cx| {
                player.seek(seconds, VIDEO_SEEK_TOLERANCE, VIDEO_SEEK_TOLERANCE, None, cx);
            });
        }
        cx.notify();
    }
}

#[derive(IntoElement)]
struct ModerationEvidenceVideo {
    id: ElementId,
    item: ReportEvidenceItem,
}

impl RenderOnce for ModerationEvidenceVideo {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(moderation_child_id(&self.id, "state"), cx, |_, _| {
            ModerationEvidenceVideoState::default()
        });

        let source = self.item.absolute_url.clone();
        let task_state = state.clone();
        use_task(moderation_child_id(&self.id, "materialize"), self.item.id.clone(), window, cx, async move |cx| {
            let remote = source.as_deref().and_then(Url::from_string);
            let local = match remote {
                Some(remote) => {
                    let cache = RemoteMediaDiskCache::shared();
                    cache.materialize(&remote, RemoteMediaDiskCacheMaterializeOptions::default(), &*cx).await.ok()
                }
                None => None,
            };
            let _ = cx.update(|_, cx| {
                task_state.update(cx, |state, cx| match local {
                    Some(local) => state.install(local, cx),
                    None => {
                        state.failed = true;
                        cx.notify();
                    }
                });
            });
        });

        let (player, failed) = {
            let state = state.read(cx);
            (state.player.clone(), state.failed)
        };
        match player {
            _ if failed => div()
                .w_full()
                .child(
                    MainText::new(tr("error_fetching_data"))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(white().opacity(0.8)),
                )
                .into_any_element(),
            Some(player) => ModerationEvidenceVideoSurface { id: self.id, state, player }.into_any_element(),
            None => progress_view().tint(white()).into_any_element(),
        }
    }
}

#[derive(IntoElement)]
struct ModerationEvidenceVideoSurface {
    id: ElementId,
    state: Entity<ModerationEvidenceVideoState>,
    player: Entity<AVPlayer>,
}

impl ModerationEvidenceVideoSurface {
    fn transport(&self, cx: &App) -> AnyElement {
        let (progress, duration) = {
            let state = self.state.read(cx);
            (state.progress, state.duration)
        };
        let is_paused = self.player.read(cx).time_control_status() == AVPlayerTimeControlStatus::Paused;
        let symbol = if is_paused { "play.fill" } else { "pause.fill" };

        let toggle_state = self.state.clone();
        let value_state = self.state.clone();
        let scrub_state = self.state.clone();
        let editing_state = self.state.clone();
        let value = Binding::new(
            move |cx: &App| value_state.read(cx).progress,
            move |seconds, _window, cx| scrub_state.update(cx, |state, cx| state.scrub(seconds, cx)),
        );

        hstack(12.0)
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .px(px(16.0))
            .py(px(12.0))
            .bg(black().opacity(0.45))
            .child(
                plain(moderation_child_id(&self.id, "toggle"))
                    .size(px(28.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .on_press(move |_, cx| toggle_state.update(cx, |state, cx| state.toggle_playback(cx)))
                    .children(moderation_symbol(symbol, 17.0, white())),
            )
            .child(Self::time_label(progress))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    slider(moderation_child_id(&self.id, "scrubber"), value, 0.0..=duration.max(0.0))
                        .tint(white())
                        .on_editing_changed(move |editing, _, cx| {
                            editing_state.update(cx, |state, _| state.is_scrubbing = editing);
                        }),
                ),
            )
            .child(Self::time_label(duration))
            .into_any_element()
    }

    fn time_label(seconds: f64) -> impl IntoElement {
        div().flex_none().whitespace_nowrap().child(
            MainText::new(RiseFormat::duration(seconds as i64))
                .px(13.0)
                .font_weight(FontWeight::MEDIUM)
                .monospaced_digits(true)
                .color(white()),
        )
    }
}

impl RenderOnce for ModerationEvidenceVideoSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let transport = self.transport(cx);
        let toggle_state = self.state.clone();
        let appearing = self.player.clone();
        let disappearing = self.player.clone();
        let focus = window
            .use_keyed_state::<FocusHandle>(moderation_child_id(&self.id, "focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let focus_appearing = focus.clone();
        div()
            .id(moderation_child_id(&self.id, "surface"))
            .relative()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "space" {
                    cx.stop_propagation();
                    toggle_state.update(cx, |state, cx| state.toggle_playback(cx));
                }
            })
            .child(
                AVPlayerLayer::for_player(Some(self.player.clone())).video_gravity(AVLayerVideoGravity::ResizeAspect),
            )
            .child(transport)
            .on_appear(moderation_child_id(&self.id, "playback"), move |window, cx| {
                window.focus(&focus_appearing, cx);
                appearing.update(cx, |player, cx| player.play(cx));
            })
            .and_on_disappear(move |_, cx| {
                disappearing.update(cx, |player, cx| player.pause(cx));
            })
    }
}

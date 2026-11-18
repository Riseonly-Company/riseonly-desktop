use std::cell::Cell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyWindowHandle, App, Bounds, Context, ElementId, EntityId, FontWeight, Global, Hsla, InteractiveElement, IntoElement,
    ParentElement, Path, PathBuilder, Pixels, Point, RenderOnce, SharedString, Size, Styled, Subscription, Window,
    canvas, div, fill, point, px, size,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::font::Font;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::shapes::{Shape, centred_stroke};
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, vstack};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::modules::music::shared::utils::music::music_player_state::{MusicAudioDownloadProgress, MusicPlayerState};
use crate::modules::music::widgets::music_marquee_title::MusicMarqueeTitle;

const DOWNLOAD_RING_LINE_WIDTH: f32 = 2.0;

fn paint_download_ring(bounds: Bounds<Pixels>, fraction: f32, color: Hsla, window: &mut Window) {
    if fraction <= 0.0 {
        return;
    }
    let center = bounds.center();
    let radius = f32::from(bounds.size.width) / 2.0;
    let steps = (fraction.min(1.0) * 96.0).ceil().max(2.0) as usize;
    let sweep = 2.0 * PI * fraction.min(1.0);
    let points: Vec<Point<Pixels>> = (0..=steps)
        .map(|step| {
            let angle = -PI / 2.0 + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect();
    let mut builder = gpui::PathBuilder::stroke(px(DOWNLOAD_RING_LINE_WIDTH));
    builder.add_polygon(&points, false);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
    let half = DOWNLOAD_RING_LINE_WIDTH / 2.0;
    for end in [points.first(), points.last()].into_iter().flatten() {
        let dot = Bounds::new(
            point(end.x - px(half), end.y - px(half)),
            size(px(DOWNLOAD_RING_LINE_WIDTH), px(DOWNLOAD_RING_LINE_WIDTH)),
        );
        window.paint_quad(fill(dot, color).corner_radii(px(half)));
    }
}

const STRIP_HEIGHT: f32 = 1.5;
const STRIP_CLIP_ROWS: usize = 6;

/// horizontal inset of a rounded corner of radius at rise above the card bottom edge
fn corner_inset(radius: f32, rise: f32) -> f32 {
    if radius <= 0.0 || rise >= radius {
        return 0.0;
    }
    let dy = radius - rise;
    radius - (radius * radius - dy * dy).max(0.0).sqrt()
}

/// filled part of the bottom strip from origin
/// an empty fill stays as a transparent sliver so the swap key keeps a path to replace
fn progress_strip(
    origin: Point<Pixels>,
    strip: Size<Pixels>,
    fraction: f32,
    tint: Hsla,
    corner_radius: f32,
) -> (Option<Path<Pixels>>, Hsla) {
    let full_width = f32::from(strip.width);
    let width = full_width * fraction.clamp(0.0, 1.0);
    let height = f32::from(strip.height);
    let at = |x: f32, y: f32| point(origin.x + px(x), origin.y + px(y));
    let sliver = || (vec![at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0)], tint.opacity(0.0));
    let (points, color) = if width <= 0.0 || height <= 0.0 {
        sliver()
    } else if corner_radius <= 0.0 {
        (vec![at(0.0, 0.0), at(width, 0.0), at(width, height), at(0.0, height)], tint)
    } else {
        let rows: Vec<(f32, f32, f32)> = (0..=STRIP_CLIP_ROWS)
            .map(|row| {
                let y = height * row as f32 / STRIP_CLIP_ROWS as f32;
                let inset = corner_inset(corner_radius, height - y);
                let leading = inset.min(full_width / 2.0);
                let trailing = (full_width - inset).min(width).max(leading);
                (y, leading, trailing)
            })
            .collect();
        if rows.iter().all(|(_, leading, trailing)| trailing - leading <= 0.0) {
            sliver()
        } else {
            let mut points: Vec<Point<Pixels>> = rows.iter().map(|(y, leading, _)| at(*leading, *y)).collect();
            points.extend(rows.iter().rev().map(|(y, _, trailing)| at(*trailing, *y)));
            (points, tint)
        }
    };
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&points, true);
    (builder.build().ok(), color)
}

/// desktop only, the strip follows the position tick by a path swap
/// not a notify of the chat screen
/// that rendered chat, header and route each tick
struct ChatAudioProgressSwap {
    painted: Rc<Cell<Option<(Size<Pixels>, Hsla, f32)>>>,
    awaits_paint: bool,
    _subscription: Subscription,
}

impl ChatAudioProgressSwap {
    fn new(window_handle: AnyWindowHandle, host: EntityId, cx: &mut Context<Self>) -> Self {
        let publication = MusicPlayerState::shared(cx).read(cx).position_publication().clone();
        let _subscription = cx.observe(&publication, move |swap: &mut Self, _, cx| {
            let player = MusicPlayerState::shared(cx);
            let (fraction, is_downloading) = {
                let player = player.read(cx);
                (player.progress, player.is_preparing_playback && player.current_track_download_progress().is_some())
            };
            let painted = swap.painted.get();
            if swap.awaits_paint && painted.is_none() {
                return;
            }
            let key = Self::swap_key(cx.entity_id());
            let swapped = !is_downloading
                && painted.is_some_and(|(strip_size, tint, corner_radius)| {
                    let (path, color) =
                        progress_strip(Point::default(), strip_size, fraction.max(0.0) as f32, tint, corner_radius);
                    path.is_some_and(|path| {
                        window_handle
                            .update(cx, |_, window, _| window.swap_paths(vec![(key, path, color.into())]))
                            .unwrap_or(false)
                    })
                });
            if !swapped {
                swap.painted.set(None);
                swap.awaits_paint = true;
                App::notify(cx, host);
            }
        });
        Self { painted: Rc::new(Cell::new(None)), awaits_paint: false, _subscription }
    }

    fn swap_key(entity: EntityId) -> u64 {
        entity.as_u64() ^ (0x43 << 56)
    }
}

struct ChatAudioNowPlayingBarDismissal(Option<String>);

impl Global for ChatAudioNowPlayingBarDismissal {}

#[derive(IntoElement)]
pub struct ChatAudioNowPlayingBar {
    id: ElementId,
    on_open: Rc<dyn Fn(&mut Window, &mut App)>,
    card_bottom: Option<(f32, bool)>,
}

impl ChatAudioNowPlayingBar {
    pub fn new(id: impl Into<ElementId>, on_open: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), on_open: Rc::new(on_open), card_bottom: None }
    }

    pub fn clipped_by_card_bottom(mut self, corner_radius: f32, spans_card: bool) -> Self {
        self.card_bottom = Some((corner_radius, spans_card));
        self
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn playback_identity(player: &MusicPlayerState) -> Option<String> {
        let track = player.current_track.as_ref()?;
        Some(player.playback_scope_id().map_or_else(|| track.id.clone(), str::to_owned))
    }

    pub fn is_hidden(cx: &mut App) -> bool {
        let Some(hidden) = cx.try_global::<ChatAudioNowPlayingBarDismissal>().and_then(|dismissal| dismissal.0.clone())
        else {
            return false;
        };
        Self::playback_identity(MusicPlayerState::shared(cx).read(cx)) == Some(hidden)
    }

    pub fn reveal(cx: &mut App) {
        if cx.try_global::<ChatAudioNowPlayingBarDismissal>().is_some_and(|dismissal| dismissal.0.is_some()) {
            cx.set_global(ChatAudioNowPlayingBarDismissal(None));
        }
    }

    fn hide(host: EntityId, cx: &mut App) {
        let identity = Self::playback_identity(MusicPlayerState::shared(cx).read(cx));
        cx.set_global(ChatAudioNowPlayingBarDismissal(identity));
        cx.notify(host);
    }

    fn play_pause_label(
        &self,
        theme: &AppTheme,
        is_playing: bool,
        is_downloading: bool,
        download: Option<&MusicAudioDownloadProgress>,
    ) -> gpui::AnyElement {
        let body = div().relative().size(px(30.0)).flex().items_center().justify_center();
        if !is_downloading {
            let symbol = if is_playing { "pause.fill" } else { "play.fill" };
            return body.children(IconUi::sized(symbol, px(14.0), theme.text.primary, true)).into_any_element();
        }
        let track_ring = centred_stroke(theme.border._200.opacity(0.45), DOWNLOAD_RING_LINE_WIDTH, Shape::Circle);
        let Some(fraction) = download.and_then(|download| download.fraction()) else {
            return body
                .child(track_ring)
                .child(div().size(px(20.0)).child(progress_view().tint(theme.text.primary).scale_effect(0.68)))
                .into_any_element();
        };
        let ring_color = theme.primary._100;
        body.child(track_ring)
            .child(
                canvas(|_, _, _| {}, move |bounds, _, window, _| {
                    paint_download_ring(bounds, fraction as f32, ring_color, window);
                })
                .absolute()
                .inset_0(),
            )
            .children(IconUi::sized("arrow.down", px(9.0), theme.text.primary, false))
            .into_any_element()
    }
}

impl RenderOnce for ChatAudioNowPlayingBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.current_view();
        let player = MusicPlayerState::shared(cx);
        let observed = player.clone();
        let _observer = window.use_keyed_state(self.child_id("player-observer"), cx, move |_, cx| {
            App::observe(cx, &observed, move |_, cx: &mut App| cx.notify(host))
        });
        let window_handle = window.window_handle();
        let strip_swap = window.use_keyed_state(self.child_id("progress-swap"), cx, move |_, cx| {
            ChatAudioProgressSwap::new(window_handle, host, cx)
        });
        let strip_key = ChatAudioProgressSwap::swap_key(strip_swap.entity_id());
        let strip_painted = strip_swap.read(cx).painted.clone();

        let state = player.read(cx);
        let Some(track) = state.current_track.clone() else {
            return div().into_any_element();
        };
        let is_playing = state.is_playing;
        let progress = state.progress;
        let download = state.current_track_download_progress();
        let is_downloading = state.is_preparing_playback && download.is_some();

        let theme = rise_ui::theme(cx);
        let play_pause_tooltip = tr(if is_playing { "media_viewer_pause" } else { "media_viewer_play" });
        let subtitle = match download.as_ref() {
            Some(download) => format!("{} \u{b7} \u{2193} {}", track.artist, download.byte_description()),
            None => track.artist.clone(),
        };
        let filled_fraction = if is_downloading {
            download.as_ref().and_then(|download| download.fraction()).unwrap_or(0.0)
        } else {
            progress
        }
        .max(0.0) as f32;
        let filled_color = if is_downloading { theme.primary._100 } else { theme.text.primary };
        let track_color = theme.border._200.opacity(0.34);
        let card_bottom = self.card_bottom;
        let on_open = self.on_open.clone();

        vstack(0.0)
            .relative()
            .w_full()
            .child(
                hstack(8.0)
                    .w_full()
                    .px(px(8.0))
                    .py(px(6.5))
                    .child(
                        div().id(self.child_id("play-pause-tooltip")).flex_none().tooltip_label(play_pause_tooltip).child(
                            plain(self.child_id("play-pause"))
                                .on_press(|_, cx| {
                                    MusicPlayerState::shared(cx).update(cx, |player, cx| player.toggle_play_pause(cx));
                                })
                                .child(self.play_pause_label(theme, is_playing, is_downloading, download.as_ref())),
                        ),
                    )
                    .child(
                        plain(self.child_id("open"))
                            .flex_1()
                            .min_w_0()
                            .on_press(move |window, cx| on_open(window, cx))
                            .child(
                                vstack(1.0)
                                    .align(HorizontalAlignment::Leading)
                                    .w_full()
                                    .child(
                                        MusicMarqueeTitle::new(self.child_id("title"), track.title.clone())
                                            .font(Font::system_weight(13.0, FontWeight::SEMIBOLD))
                                            .foreground(theme.text.primary)
                                            .line_height(16.0),
                                    )
                                    .child(
                                        MusicMarqueeTitle::new(self.child_id("subtitle"), subtitle)
                                            .font(Font::system_size(11.0))
                                            .foreground(theme.text.secondary)
                                            .line_height(14.0),
                                    ),
                            ),
                    )
                    .child(
                        div().id(self.child_id("close-tooltip")).flex_none().tooltip_label(tr("close")).child(
                            plain(self.child_id("close"))
                                .on_press(move |_, cx| Self::hide(host, cx))
                                .w(px(44.0))
                                .h(px(30.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("xmark", px(12.0), theme.text.secondary, false)),
                        ),
                    ),
            )
            .child(div().w_full().h(px(STRIP_HEIGHT)).flex_none())
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let bar_width = f32::from(bounds.size.width);
                        let bar_height = f32::from(bounds.size.height);
                        let corner_radius = card_bottom.map_or(0.0, |(radius, spans_card)| {
                            let cap = if spans_card { bar_width.min(bar_height) } else { bar_width };
                            radius.min(cap / 2.0)
                        });
                        let strip_origin =
                            point(bounds.origin.x, bounds.origin.y + bounds.size.height - px(STRIP_HEIGHT));
                        let strip_size = size(bounds.size.width, px(STRIP_HEIGHT));
                        let (track, track_tint) =
                            progress_strip(strip_origin, strip_size, 1.0, track_color, corner_radius);
                        if let Some(track) = track {
                            window.paint_path(track, track_tint);
                        }
                        let (path, color) =
                            progress_strip(strip_origin, strip_size, filled_fraction, filled_color, corner_radius);
                        if let Some(path) = path {
                            window.paint_swappable_path(path, color, strip_key, strip_origin);
                            strip_painted.set(Some((strip_size, filled_color, corner_radius)));
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }
}

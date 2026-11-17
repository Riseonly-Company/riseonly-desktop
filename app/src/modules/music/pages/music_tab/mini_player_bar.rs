use std::cell::Cell;
use std::f32::consts::PI;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, Bounds, ElementId, EntityId, FontWeight, Hsla, MouseButton, MouseDownEvent, Path, PathBuilder,
    Pixels, Point, SharedString, Size, Stateful, Subscription, TransformationMatrix, Window, black, canvas, fill, point,
    size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, alpha, color};
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::{IgnoresTextSizeDeltaKey, MainText};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::font::Font;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rounded_gradient::rounded_gradient_fill;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::UnitPoint;

use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::{MusicAudioDownloadProgress, MusicPlayerState};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverMedia;
use crate::modules::music::widgets::music_marquee_title::MusicMarqueeTitle;
use crate::modules::music::widgets::music_mini_player_bottom_padding::{
    MusicPlayerHiddenUnderFullPlayerKey, is_hidden_under_full_player, observed_music_player,
};
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const DISMISS_THRESHOLD: f32 = 44.0;
const DRAG_MINIMUM_DISTANCE: f32 = 12.0;
const COVER_SIDE: f32 = 38.0;
const COVER_RADIUS: f32 = 8.0;
const BUTTON_SIDE: f32 = 28.0;
const PROGRESS_HEIGHT: f32 = 2.0;
const RING_LINE_WIDTH: f32 = 2.0;
const LIKED_COLOR: &str = "#E84393";
const COVER_GRADIENT: [&str; 2] = ["#FF6268", "#D62D3C"];

fn settle_animation() -> MotionSpec {
    spring(0.3, 0.82)
}

fn glyph(name: &str, side: f32, tint: Hsla) -> Option<gpui::Svg> {
    IconUi::sized(name, px(side), tint, name.ends_with(".fill"))
}

/// drag offset
/// on desktop also the pointer, whether a drag ate the coming click and whether the close button shows
struct MiniPlayerBarState {
    drag_offset: Animated<f32>,
    dragged: bool,
    is_hovered: bool,
    last_track_id: Option<String>,
}

impl MiniPlayerBarState {
    fn new() -> Self {
        Self { drag_offset: Animated::new(0.0), dragged: false, is_hovered: false, last_track_id: None }
    }

    fn settle(&mut self, cx: &mut Context<Self>) {
        self.drag_offset.set_in(0.0, settle_animation(), RiseClock::monotonic(), cx);
        cx.notify();
    }
}

/// desktop only, playhead line moves by a path swap on the position tick instead of a notify
/// bar is drawn inside the route destinations so a notify per tick laid out the root twice a second
/// about 9 ms a frame
struct MiniPlayerProgressSwap {
    painted: Rc<Cell<Option<(Size<Pixels>, Hsla)>>>,
    awaits_paint: bool,
    _subscription: Subscription,
}

impl MiniPlayerProgressSwap {
    fn new(window_handle: AnyWindowHandle, host: EntityId, hides_under_full_player: bool, cx: &mut Context<Self>) -> Self {
        let player = MusicPlayerState::shared(cx);
        let publication = player.read(cx).position_publication().clone();
        let painted: Rc<Cell<Option<(Size<Pixels>, Hsla)>>> = Rc::new(Cell::new(None));
        let _subscription = cx.observe(&publication, move |swap: &mut Self, _, cx| {
            let player = MusicPlayerState::shared(cx);
            let (fraction, is_downloading) = {
                let player = player.read(cx);
                if is_hidden_under_full_player(player, hides_under_full_player) {
                    return;
                }
                (player.progress, player.is_preparing_playback && player.current_track_download_progress().is_some())
            };
            if swap.awaits_paint && swap.painted.get().is_none() {
                return;
            }
            let key = Self::swap_key(cx.entity_id());
            let swapped = !is_downloading
                && swap.painted.get().is_some_and(|(line_size, tint)| {
                    let (path, color) = capsule_progress_line(Bounds::new(Point::default(), line_size), fraction as f32, tint);
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
        Self { painted, awaits_paint: false, _subscription }
    }

    fn swap_key(entity: EntityId) -> u64 {
        entity.as_u64() ^ (0x4D << 56)
    }
}

#[derive(IntoElement, Default)]
pub struct MiniPlayerBar {
    on_tap: Option<ButtonAction>,
}

impl MiniPlayerBar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_tap = Some(Rc::new(on_tap));
        self
    }

    /// for the host this ends the session for everyone
    /// so it asks the same confirmation as the session screen
    pub fn dismiss(cx: &mut App) {
        let player = MusicPlayerState::shared(cx);
        let needs_confirmation = {
            let player = player.read(cx);
            player.is_listening_session_host() && player.is_listening_together()
        };
        if needs_confirmation {
            NativeConfirmAlert::present_delete(
                &tr("music_live_close_confirmation_title"),
                &tr("music_live_close_confirmation_message"),
                Some(tr("music_live_leave")),
                None,
                false,
                None,
                None,
                |cx: &mut App| {
                    MusicPlayerState::shared(cx).update(cx, |player, cx| player.stop_and_clear(cx));
                },
                cx,
            );
        } else {
            player.update(cx, |player, cx| player.stop_and_clear(cx));
        }
    }

    fn dismiss_drag(state: &gpui::Entity<MiniPlayerBarState>) -> DragGesture {
        let changed = state.clone();
        let ended = state.clone();
        DragGesture::new(DRAG_MINIMUM_DISTANCE)
            .on_changed(move |value, _, cx| {
                let offset = f32::from(value.translation.height).max(0.0);
                changed.update(cx, |state, cx| {
                    state.dragged = true;
                    state.drag_offset.set_in(offset, None::<MotionSpec>, RiseClock::monotonic(), cx);
                    cx.notify();
                });
            })
            .on_ended(move |value, _, cx| {
                let needs_settle = f32::from(value.translation.height) <= DISMISS_THRESHOLD || {
                    let player = MusicPlayerState::shared(cx);
                    let player = player.read(cx);
                    player.is_listening_session_host() && player.is_listening_together()
                };
                if needs_settle {
                    ended.update(cx, |state, cx| state.settle(cx));
                }
                if f32::from(value.translation.height) > DISMISS_THRESHOLD {
                    Self::dismiss(cx);
                }
            })
    }

    fn mini_cover(track: &TrackModel) -> AnyElement {
        if MusicCoverMedia::has_remote_cover(&track.cover_image_url) {
            return MusicCoverImage::new("mini-player-cover", track.cover_image_url.clone())
                .fallback_colors(track.cover_colors.clone())
                .size(COVER_SIDE)
                .corner_radius(COVER_RADIUS)
                .into_any_element();
        }
        let gradient = LinearGradient::colors(
            COVER_GRADIENT.iter().map(|hex| color(hex)),
            UnitPoint::TOP_LEADING,
            UnitPoint::BOTTOM_TRAILING,
        );
        div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(COVER_SIDE))
            .child(div().absolute().inset_0().child(rounded_gradient_fill(gradient, COVER_RADIUS)))
            .child(div().relative().children(glyph("music.note", 16.0, white())))
            .into_any_element()
    }

    fn download_overlay(download: Option<&MusicAudioDownloadProgress>) -> Div {
        let content: AnyElement = match download.and_then(MusicAudioDownloadProgress::fraction) {
            Some(fraction) => div()
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .child(
                    canvas(|_, _, _| (), move |bounds, _, window, _| paint_download_ring(bounds, fraction as f32, window))
                        .absolute()
                        .inset_0(),
                )
                .child(div().relative().children(glyph("arrow.down", 8.0, white())))
                .into_any_element(),
            None => progress_view().tint(white()).control_size(ControlSize::Small).into_any_element(),
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(black().opacity(0.48))
            .clip_shape(Shape::continuous(COVER_RADIUS))
            .child(content)
    }

    fn session_badge(has_permission_request: bool, theme: &AppTheme) -> Stateful<Div> {
        let mut badge = div()
            .id("mini-player-session-badge")
            .tooltip_label(tr("music_live_session_title"))
            .aria_label(tr("music_live_session_title"))
            .flex()
            .items_center()
            .justify_center()
            .absolute()
            .right(px(-4.0))
            .bottom(px(-4.0))
            .size(px(17.0))
            .rounded_full()
            .bg(UIColor::system_green().resolved_color(theme.appearance))
            .stroke_border(theme.bg._100, 2.0)
            .children(glyph("shareplay", 8.0, black()));
        if has_permission_request {
            badge = badge.child(
                div()
                    .absolute()
                    .top(px(-2.0))
                    .right(px(-2.0))
                    .size(px(7.0))
                    .rounded_full()
                    .bg(UIColor::system_red().resolved_color(theme.appearance))
                    .stroke_border(theme.bg._100, 1.0),
            );
        }
        badge
    }

    fn icon_button(
        id: &'static str,
        label: String,
        content: impl IntoElement,
        action: impl Fn(&mut App) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(SharedString::new_static(id))
            .flex_none()
            .tooltip_label(label.clone())
            .aria_label(label)
            .child(
                plain(ElementId::Name(SharedString::new_static(id)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(BUTTON_SIDE))
                    .on_press(move |_, cx| {
                        cx.stop_propagation();
                        action(cx);
                    })
                    .child(content),
            )
    }
}

impl RenderOnce for MiniPlayerBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player_entity = observed_music_player("mini-player-observed", window, cx);
        let host = window.current_view();
        let hides_under_full_player = env::<MusicPlayerHiddenUnderFullPlayerKey>(cx);
        let window_handle = window.window_handle();
        let progress_swap = window.use_keyed_state("mini-player-progress-swap", cx, move |_, cx| {
            MiniPlayerProgressSwap::new(window_handle, host, hides_under_full_player, cx)
        });
        let _services_observer = window.use_keyed_state("mini-player-services", cx, move |_, cx| -> Subscription {
            let services = MusicServicesStore::shared(cx);
            App::observe(cx, &services, move |_, cx: &mut App| cx.notify(host))
        });
        let state = window.use_keyed_state("mini-player-state", cx, |_, _| MiniPlayerBarState::new());

        let (track, download, is_preparing, is_playing, progress, shows_session, has_request) = {
            let player = player_entity.read(cx);
            let Some(track) = player.current_track.clone() else {
                return div().into_any_element();
            };
            (
                track,
                player.current_track_download_progress(),
                player.is_preparing_playback,
                player.is_playing,
                player.progress,
                player.is_listening_together() && !player.is_listening_playback_displaced_by_ad_hoc(),
                player.listening_permission_request_notice.is_some(),
            )
        };
        let is_downloading = is_preparing && download.is_some();

        state.update(cx, |state, _| {
            if state.last_track_id.as_deref() != Some(track.id.as_str()) {
                state.last_track_id = Some(track.id.clone());
                state.drag_offset = Animated::new(0.0);
                state.dragged = false;
            }
        });

        let theme = rise_ui::theme(cx).clone();
        let like_track = track.clone();
        let is_liked = MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.is_liked(&like_track, cx));
        let pins_to_profile = track.source_chat_id.is_some() && track.transfer_phase.is_none();

        let mut cover = div().relative().flex_none().size(px(COVER_SIDE)).child(Self::mini_cover(&track));
        if is_downloading {
            cover = cover.child(Self::download_overlay(download.as_ref()));
        }
        if shows_session {
            cover = cover.child(Self::session_badge(has_request, &theme));
        }

        let subtitle = match &download {
            Some(download) => format!("{} \u{b7} \u{2193} {}", track.artist, download.byte_description()),
            None => track.artist.clone(),
        };
        let titles = vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .min_w_0()
            .child(
                MusicMarqueeTitle::new("mini-player-title", track.title.clone())
                    .font(Font::system_weight(14.0, FontWeight::SEMIBOLD))
                    .foreground(theme.text.primary)
                    .line_height(18.0)
                    .shows_copy_menu(false)
                    .is_active(is_playing),
            )
            .child(
                env_scope::<IgnoresTextSizeDeltaKey>(true).child(
                    MainText::new(subtitle)
                        .px(12.0)
                        .font_weight(FontWeight::NORMAL)
                        .color(theme.text.secondary)
                        .monospaced_digits(true)
                        .line_limit(1),
                ),
            );

        let like_label = if pins_to_profile {
            tr("music_add_to_profile")
        } else {
            tr(if is_liked { "ctx_unlike" } else { "ctx_like" })
        };
        let like_icon = if pins_to_profile {
            "person.crop.circle.badge.plus"
        } else if is_liked {
            "heart.fill"
        } else {
            "heart"
        };
        let like_tint = if is_liked && !pins_to_profile { color(LIKED_COLOR) } else { theme.text.secondary };
        let like_target = track.clone();
        let like = Self::icon_button("mini-player-like", like_label, div().children(glyph(like_icon, 16.0, like_tint)), move |cx| {
            let target = like_target.clone();
            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                if pins_to_profile {
                    interactions.add_track_to_profile(target, cx);
                } else {
                    interactions.toggle_like(&target.id, cx);
                }
            });
        });

        let play_content: AnyElement = if is_downloading {
            progress_view().tint(theme.text.primary).control_size(ControlSize::Small).into_any_element()
        } else {
            div()
                .children(glyph(if is_playing { "pause.fill" } else { "play.fill" }, 18.0, theme.text.primary))
                .into_any_element()
        };
        let play = Self::icon_button(
            "mini-player-play",
            tr(if is_playing { "media_viewer_pause" } else { "media_viewer_play" }),
            play_content,
            |cx| MusicPlayerState::shared(cx).update(cx, |player, cx| player.toggle_play_pause(cx)),
        );
        let next = Self::icon_button(
            "mini-player-next",
            tr("music_player_next"),
            div().children(glyph("forward.fill", 16.0, theme.text.primary)),
            |cx| MusicPlayerState::shared(cx).update(cx, |player, cx| player.next(cx)),
        );

        let (is_hovered, drag_offset) = {
            let bar = state.read(cx);
            (bar.is_hovered, bar.drag_offset.frame_value(RiseClock::monotonic(), window))
        };
        let close = is_hovered.then(|| {
            Self::icon_button(
                "mini-player-close",
                tr("close"),
                div().children(glyph("xmark", 14.0, theme.text.secondary)),
                Self::dismiss,
            )
        });

        let fraction = if is_downloading {
            download.as_ref().and_then(MusicAudioDownloadProgress::fraction).unwrap_or(0.0)
        } else {
            progress
        };
        let progress_tint = theme.primary._100;
        let progress_key = MiniPlayerProgressSwap::swap_key(progress_swap.entity_id());
        let progress_painted = progress_swap.read(cx).painted.clone();
        let body = div()
            .relative()
            .w_full()
            .rounded_full()
            .bg(alpha(theme.bg._100, 1.0))
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(14.0))
                    .pt(px(8.0))
                    .pb(px(9.0))
                    .child(cover)
                    .child(titles)
                    .child(like)
                    .child(play)
                    .child(next)
                    .child(div().flex_none().size(px(BUTTON_SIDE)).children(close)),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let (path, color) = capsule_progress_line(bounds, fraction as f32, progress_tint);
                        if let Some(path) = path {
                            let swappable = window.transformation() == TransformationMatrix::unit()
                                && !path.bounds.intersect(&window.content_mask().bounds).is_empty();
                            window.paint_swappable_path(path, color, progress_key, bounds.origin);
                            if swappable {
                                progress_painted.set(Some((bounds.size, progress_tint)));
                            }
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .liquid_chrome_capsule();

        let mouse_state = state.clone();
        let hover_state = state.clone();
        let tap_state = state.clone();
        let on_tap = self.on_tap;
        div()
            .id("mini-player-bar")
            .relative()
            .w_full()
            .top(px(drag_offset))
            .opacity(if drag_offset > 0.0 { (1.0 - drag_offset / 180.0).max(0.3) } else { 1.0 })
            .rounded_full()
            .overflow_hidden()
            .cursor_pointer()
            .block_mouse_except_scroll()
            .on_hover(move |hovered, _, cx| {
                let hovered = *hovered;
                hover_state.update(cx, |state, cx| {
                    state.is_hovered = hovered;
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, cx| {
                mouse_state.update(cx, |state, _| state.dragged = false);
            })
            .on_click(move |_, window, cx| {
                let dragged = tap_state.read(cx).dragged;
                if dragged {
                    tap_state.update(cx, |state, _| state.dragged = false);
                    return;
                }
                if let Some(on_tap) = &on_tap {
                    on_tap(window, cx);
                }
            })
            .child(body)
            .gesture(ElementId::Name(SharedString::new_static("mini-player-drag")), Self::dismiss_drag(&state))
            .into_any_element()
    }
}

fn paint_download_ring(bounds: Bounds<Pixels>, fraction: f32, window: &mut Window) {
    let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
    let radius = (side - RING_LINE_WIDTH) / 2.0;
    let center = bounds.center();
    let start = -PI / 2.0;
    stroke_arc(&arc_points(center, radius, start, 2.0 * PI), true, white().opacity(0.28), window);
    let sweep = 2.0 * PI * fraction.clamp(0.0, 1.0);
    if sweep <= 0.0 {
        return;
    }
    let arc = arc_points(center, radius, start, sweep);
    stroke_arc(&arc, false, white(), window);
    for end in [arc.first(), arc.last()].into_iter().flatten() {
        let half = RING_LINE_WIDTH / 2.0;
        let dot = Bounds::new(point(end.x - px(half), end.y - px(half)), size(px(RING_LINE_WIDTH), px(RING_LINE_WIDTH)));
        window.paint_quad(fill(dot, white()).corner_radii(px(half)));
    }
}

fn arc_points(center: Point<Pixels>, radius: f32, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
    let steps = ((sweep.abs() / (2.0 * PI)) * 48.0).ceil().max(2.0) as usize;
    (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect()
}

fn stroke_arc(points: &[Point<Pixels>], closed: bool, tint: Hsla, window: &mut Window) {
    let mut builder = PathBuilder::stroke(px(RING_LINE_WIDTH));
    builder.add_polygon(points, closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, tint);
    }
}

/// 2pt progress line along the bottom of the capsule
/// gpui content mask is a rectangle so the line is a trapezoid whose slanted ends follow the corners
/// an empty line is a clear sliver at the leading corner so the next tick swap finds it
fn capsule_progress_line(bounds: Bounds<Pixels>, fraction: f32, tint: Hsla) -> (Option<Path<Pixels>>, Hsla) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let end = width * fraction.clamp(0.0, 1.0);
    let radius = height / 2.0;
    let inner = radius - PROGRESS_HEIGHT;
    let top_inset = radius - (radius * radius - inner * inner).max(0.0).sqrt();
    let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    let top = height - PROGRESS_HEIGHT;
    let (points, color) = if end <= top_inset || width <= 0.0 {
        (vec![at(top_inset, top), at(top_inset + 1.0, top), at(top_inset, height)], tint.opacity(0.0))
    } else {
        let mut points = vec![at(top_inset, top), at(end.min(width - top_inset), top)];
        if end < radius {
            let reach = (end - top_inset) / (radius - top_inset);
            points.push(at(end, top + PROGRESS_HEIGHT * reach));
        } else {
            points.push(at(end.min(width - radius), height));
            points.push(at(radius, height));
        }
        (points, tint)
    };
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&points, true);
    (builder.build().ok(), color)
}

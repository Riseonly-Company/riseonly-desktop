use std::cell::Cell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gpui::{
    App, AsyncWindowContext, Bounds, Element, ElementId, Entity, FontWeight, GlobalElementId, Hsla, InspectorElementId,
    LayoutId, PathBuilder, Pixels, Point, SVG_FADE_KEY_BIT, SharedString, Subscription, WeakEntity, Window, black,
    canvas, fill, point, rgb, size, white,
};
use rise_i18n::{FormatArg, format_printf, tr};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::AppTheme;
use rise_theme::ui_color_system::UIColor;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::byte_format::ByteFormat;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::music::pages::playlist_detail::music_playlist_listener_cluster::{
    MusicPlaylistListenerCluster, MusicPlaylistLiveScopeIdKey,
};
use crate::modules::music::shared::utils::music::music_models::{MusicLocalTrackTransferPhase, TrackModel};
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistListener;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_exclusive_badge::MusicExclusiveBadge;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;

const COVER_SIDE: f32 = 48.;
const COVER_RADIUS: f32 = 8.;
const LIKED_COLOR: u32 = 0xE84393;
const WAVEFORM_STEP: Duration = Duration::from_millis(250);
const WAVEFORM_OPACITY: [f32; 4] = [1., 0.72, 0.45, 0.72];

type TrackRowAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// a row of a track list
/// cover with transfer or playback state, title artist duration and the like button
#[derive(IntoElement)]
pub struct TrackRow {
    id: ElementId,
    track: TrackModel,
    is_playing: bool,
    shows_like_button: bool,
    on_tap: TrackRowAction,
}

impl TrackRow {
    pub fn new(id: impl Into<ElementId>, track: TrackModel) -> Self {
        Self {
            id: id.into(),
            track,
            is_playing: false,
            shows_like_button: true,
            on_tap: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn shows_like_button(mut self, shows_like_button: bool) -> Self {
        self.shows_like_button = shows_like_button;
        self
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_tap = Rc::new(on_tap);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn live_listeners(&self, cx: &mut App) -> Vec<MusicPlaylistListener> {
        let Some(live_playlist_id) = env::<MusicPlaylistLiveScopeIdKey>(cx) else {
            return Vec::new();
        };
        let live_snapshot = MusicServicesStore::shared(cx)
            .update(cx, |services, cx| services.playlist_listening_snapshot(&live_playlist_id, None, cx));
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        live_snapshot
            .map(|snapshot| snapshot.listeners)
            .unwrap_or_default()
            .into_iter()
            .filter(|listener| {
                listener.track_id == self.track.id && Some(listener.user_id.as_str()) != current_user_id.as_deref()
            })
            .collect()
    }

    fn shows_cover_status_glyph(&self) -> bool {
        self.is_playing
            || self.track.is_local_transfer_active()
            || self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::WaitingForConnection)
            || self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::Failed)
    }

    fn title_text(&self, theme: &AppTheme) -> MainText {
        MainText::new(self.track.title.clone())
            .px(15.)
            .font_weight(if self.is_playing { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
            .color(if self.is_playing { theme.primary._100 } else { theme.text.primary })
            .line_limit(1)
    }

    fn transfer_subtitle(&self) -> Option<String> {
        if self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::WaitingForConnection) {
            return Some(tr("music_local_upload_waiting_for_connection"));
        }
        if self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::Failed) {
            return Some(self.track.transfer_error.clone().unwrap_or_else(|| tr("music_local_upload_error_title")));
        }
        if !self.track.is_local_transfer_active() {
            return None;
        }
        if self.track.transfer_total_bytes <= 0 {
            return Some(if self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::Preparing) {
                tr("chat_media_upload_processing")
            } else {
                tr("chat_media_upload_loading")
            });
        }
        Some(format_printf(
            &tr("chat_media_upload_progress"),
            &[
                FormatArg::Text(ByteFormat::telegram(self.track.transferred_bytes).into()),
                FormatArg::Text(ByteFormat::telegram(self.track.transfer_total_bytes).into()),
            ],
        ))
    }

    fn cover(&self) -> AnyElement {
        let mut cover = zstack(Alignment::Center).flex_none().child(
            MusicTrackCoverImage::new(self.child_id("cover"), self.track.cover_image_url.clone())
                .fallback_colors(self.track.cover_colors.clone())
                .size(COVER_SIDE)
                .corner_radius(COVER_RADIUS)
                .shows_placeholder_note(!self.shows_cover_status_glyph()),
        );
        if self.is_playing {
            cover = cover.child(cover_scrim(0.35)).child(PlayingWaveform::new(self.child_id("waveform")));
        } else if self.track.is_local_transfer_active() {
            cover = cover.child(cover_scrim(0.42));
            if self.track.transfer_total_bytes > 0 {
                cover = cover.child(transfer_ring(self.track.normalized_transfer_progress() as f32));
            } else {
                cover = cover.child(div().child(progress_view().tint(white()).scale_effect(0.8)));
            }
        } else if self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::WaitingForConnection) {
            cover = cover.child(cover_scrim(0.42)).child(glyph("wifi.slash", 16., white()));
        } else if self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::Failed) {
            cover = cover.child(cover_scrim(0.42)).child(glyph("exclamationmark", 16., white()));
        }
        cover.into_any_element()
    }

    fn exclusive_title(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let fit = window.use_keyed_state(self.child_id("title-fit"), cx, |_, _| TitleFit::default());
        let measured = *fit.read(cx);
        let full_badge_fits = measured.full_width <= measured.available_width;
        let available = fit.clone();
        let full = fit;
        div()
            .relative()
            .w_full()
            .child(measure(self.child_id("title-fit-available"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                available.update(cx, |fit, cx| {
                    if fit.available_width != width {
                        fit.available_width = width;
                        cx.notify();
                    }
                });
            }))
            .child(
                hstack(6.)
                    .max_w_full()
                    .child(div().min_w_0().flex_shrink(1.0).child(self.title_text(theme)))
                    .child(
                        MusicExclusiveBadge::new().compact(true).icon_only(!full_badge_fits),
                    ),
            )
            .child(
                hstack(6.)
                    .absolute()
                    .top_0()
                    .left_0()
                    .opacity(0.)
                    .child(self.title_text(theme))
                    .child(MusicExclusiveBadge::new().compact(true))
                    .child(measure(self.child_id("title-fit-full"), move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        full.update(cx, |fit, cx| {
                            if fit.full_width != width {
                                fit.full_width = width;
                                cx.notify();
                            }
                        });
                    })),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy, Default)]
struct TitleFit {
    available_width: f32,
    full_width: f32,
}

fn glyph(name: &str, side: f32, color: Hsla) -> AnyElement {
    match IconUi::sized(name, px(side), color, name.ends_with(".fill")) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

fn cover_scrim(opacity: f32) -> Div {
    div().size(px(COVER_SIDE)).bg(black().opacity(opacity)).continuous_corners(COVER_RADIUS)
}

fn transfer_ring(progress: f32) -> impl IntoElement {
    const SIDE: f32 = 28.;
    const LINE_WIDTH: f32 = 2.4;
    const STEPS_PER_TURN: f32 = 64.;

    fn arc(center: Point<Pixels>, radius: f32, sweep: f32) -> Vec<Point<Pixels>> {
        let steps = ((STEPS_PER_TURN * sweep / (2. * PI)).ceil() as usize).max(1);
        (0..=steps)
            .map(|step| {
                let angle = -PI / 2. + sweep * step as f32 / steps as f32;
                point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
            })
            .collect()
    }

    fn stroke(points: &[Point<Pixels>], closed: bool, color: Hsla, window: &mut Window) {
        let mut builder = PathBuilder::stroke(px(LINE_WIDTH));
        builder.add_polygon(points, closed);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }

    fn round_cap(at: Point<Pixels>, color: Hsla, window: &mut Window) {
        let half = LINE_WIDTH / 2.;
        let bounds = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(LINE_WIDTH), px(LINE_WIDTH)));
        window.paint_quad(fill(bounds, color).corner_radii(px(half)));
    }

    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let center = bounds.center();
            let radius = (SIDE - LINE_WIDTH) / 2.;
            stroke(&arc(center, radius, 2. * PI), true, white().opacity(0.3), window);
            if progress <= 0. {
                return;
            }
            let points = arc(center, radius, 2. * PI * progress.min(1.));
            stroke(&points, false, white(), window);
            if let (Some(first), Some(last)) = (points.first(), points.last()) {
                round_cap(*first, white(), window);
                round_cap(*last, white(), window);
            }
        },
    )
    .size(px(SIDE))
}

/// playing waveform glyph
/// the glyph is one path here so the pass over layers becomes a stepped dim of the whole glyph
/// steps fade the painted icon in the frame instead of repainting the window at display rate
#[derive(IntoElement)]
pub struct PlayingWaveform {
    id: ElementId,
}

impl PlayingWaveform {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for PlayingWaveform {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let still = cx.reduce_motion();
        let opacity = if still { 1. } else { WAVEFORM_OPACITY[waveform_step(RiseClock::monotonic())] };
        let timeline_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("timeline"));
        let timeline = window.use_keyed_state(timeline_id, cx, |_, _| PlayingWaveformTimeline { newest_key: 0 });
        PlayingWaveformPulse {
            child: div().id(self.id).relative().opacity(opacity).child(glyph("waveform", 14., white())).into_any_element(),
            timeline,
            shown: opacity,
            still,
        }
    }
}

/// swap key of the newest paint of one waveform, a step loop over an older paint stops
struct PlayingWaveformTimeline {
    newest_key: u32,
}

static NEXT_WAVEFORM_SWAP_KEY: AtomicU32 = AtomicU32::new(1);

fn next_waveform_swap_key() -> u32 {
    (NEXT_WAVEFORM_SWAP_KEY.fetch_add(1, Ordering::Relaxed) & (SVG_FADE_KEY_BIT - 1)).max(1) | SVG_FADE_KEY_BIT
}

struct PlayingWaveformPulse {
    child: AnyElement,
    timeline: Entity<PlayingWaveformTimeline>,
    shown: f32,
    still: bool,
}

impl IntoElement for PlayingWaveformPulse {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PlayingWaveformPulse {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.still {
            self.child.paint(window, cx);
            return;
        }
        let key = next_waveform_swap_key();
        window.with_glyph_swap_key(key, |window| self.child.paint(window, cx));
        self.timeline.update(cx, |timeline, _| timeline.newest_key = key);
        let visible = bounds.intersect(&window.content_mask().bounds);
        if visible.size.width <= Pixels::ZERO || visible.size.height <= Pixels::ZERO {
            return;
        }
        PlayingWaveformSteps { key, shown: self.shown, timeline: self.timeline.downgrade() }.schedule(window, cx);
    }
}

struct PlayingWaveformSteps {
    key: u32,
    shown: f32,
    timeline: WeakEntity<PlayingWaveformTimeline>,
}

impl PlayingWaveformSteps {
    /// waits for the next step boundary off the frame loop then fades in a frame callback
    fn schedule(self, window: &Window, cx: &App) {
        let step = WAVEFORM_STEP.as_nanos().max(1);
        let into_step = RiseClock::monotonic().as_nanos() % step;
        let wait = Duration::from_nanos((step - into_step) as u64 + 1_000_000);
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(wait).await;
                let _ = cx.update(move |window, _| window.on_next_frame(move |window, cx| self.tick(window, cx)));
            })
            .detach();
    }

    fn tick(mut self, window: &mut Window, cx: &mut App) {
        let Some(timeline) = self.timeline.upgrade() else {
            return;
        };
        if timeline.read(cx).newest_key != self.key {
            return;
        }
        let target = WAVEFORM_OPACITY[waveform_step(RiseClock::monotonic())];
        if target == self.shown || window.fade_swapped_glyphs(self.key, target / self.shown) {
            self.shown = target;
            self.schedule(window, cx);
            return;
        }
        let timeline = self.timeline.clone();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(WAVEFORM_STEP).await;
                let _ = cx.update(|_, cx| timeline.update(cx, |_, cx| cx.notify()).ok());
            })
            .detach();
    }
}

fn waveform_step(now: Duration) -> usize {
    let step = WAVEFORM_STEP.as_nanos().max(1);
    ((now.as_nanos() / step) % WAVEFORM_OPACITY.len() as u128) as usize
}

impl TrackRow {
    /// one toggle per click, the flag clears once the click dispatch is over
    fn toggle_like_once(track_id: &str, handled: &Rc<Cell<bool>>, cx: &mut App) {
        if handled.replace(true) {
            return;
        }
        let reset = handled.clone();
        cx.defer(move |_| reset.set(false));
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.toggle_like(track_id, cx));
    }
}

impl RenderOnce for TrackRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.current_view();
        let _store_observer = window.use_keyed_state(self.child_id("store-observer"), cx, move |_, cx| -> Subscription {
            let services = MusicServicesStore::shared(cx);
            App::observe(cx, &services, move |_, cx: &mut App| cx.notify(host))
        });

        let theme = rise_ui::theme(cx).clone();
        let track_for_like = self.track.clone();
        let is_liked = MusicInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.is_liked(&track_for_like, cx));
        let live_listeners = self.live_listeners(cx);

        let mut cover = div().relative().flex_none().child(self.cover());
        if !live_listeners.is_empty() {
            let total_count = live_listeners.len() as i64;
            cover = cover.child(
                div().absolute().top(px(-7.)).right(px(-7.)).child(
                    MusicPlaylistListenerCluster::new(self.child_id("listeners"), live_listeners, total_count).size(25.),
                ),
            );
        }

        let title: AnyElement = if self.track.is_exclusive {
            self.exclusive_title(&theme, window, cx)
        } else {
            div().w_full().min_w_0().child(self.title_text(&theme)).into_any_element()
        };
        let subtitle_color = if self.track.transfer_phase == Some(MusicLocalTrackTransferPhase::Failed) {
            UIColor::system_red().resolved_color(theme.appearance)
        } else {
            theme.text.secondary
        };
        let details = vstack(2.)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .min_w_0()
            .child(title)
            .child(
                div().w_full().min_w_0().child(
                    SecondaryText::new(self.transfer_subtitle().unwrap_or_else(|| self.track.artist.clone()))
                        .px(13.)
                        .color(subtitle_color)
                        .line_limit(1),
                ),
            );

        let hover_color = theme.text.primary.opacity(0.05);
        let mut row = hstack(12.)
            .w_full()
            .py(px(8.))
            .rounded_rect(COVER_RADIUS)
            .hover(move |style| style.bg(hover_color))
            .child(cover)
            .child(details);

        if !self.track.is_local_transfer_active() {
            row = row.child(SecondaryText::new(self.track.formatted_duration()).px(12.));
        }

        if self.shows_like_button && self.track.transfer_phase.is_none() {
            let press_track_id = self.track.id.clone();
            let plate_track_id = self.track.id.clone();
            let press_handled = Rc::new(Cell::new(false));
            let plate_handled = press_handled.clone();
            let like_color = if is_liked { Hsla::from(rgb(LIKED_COLOR)) } else { theme.text.secondary };
            row = row.child(
                div()
                    .relative()
                    .flex_none()
                    .child(
                        div().absolute().top(px(-14.5)).left(px(-14.5)).child(div().size(px(44.)).bubble_tap_target(
                            self.child_id("like-plate"),
                            move |_, cx| Self::toggle_like_once(&plate_track_id, &plate_handled, cx),
                        )),
                    )
                    .child(
                        plain(self.child_id("like"))
                            .on_press(move |_, cx| {
                                cx.stop_propagation();
                                Self::toggle_like_once(&press_track_id, &press_handled, cx);
                            })
                            .child(glyph(if is_liked { "heart.fill" } else { "heart" }, 15., like_color)),
                    ),
            );
        }

        let on_tap = self.on_tap.clone();
        plain(self.id.clone())
            .w_full()
            .hover_fill(row_hover_fill(&theme))
            .on_press(move |window, cx| on_tap(window, cx))
            .child(row)
    }
}

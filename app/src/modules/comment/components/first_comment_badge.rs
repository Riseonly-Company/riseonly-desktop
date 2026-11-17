use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, App, AsyncWindowContext, Bounds, Element, ElementId, Entity, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, RenderOnce, SharedString, WeakEntity, Window, hsla,
};
use rise_core::foundation::foundation_date::Date;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::SkeletonWindowHeightReader;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::prelude::*;

use crate::modules::user::components::achievements::achievement_rarity_chip::{sampled_color_runs, strip_color};

/// crowned label for the earliest comment under a post
/// drawn only when the server marks the row since the feed sort isnt chronological
#[derive(IntoElement)]
pub struct FirstCommentBadge {
    id: ElementId,
}

struct FirstCommentBadgeEnergyObserver {
    _subscription: gpui::Subscription,
}

/// whether the badge may sweep and the swap key of its newest paint
struct FirstCommentBadgeTimeline {
    is_on_screen: bool,
    newest_key: u32,
}

static NEXT_BADGE_SWAP_KEY: AtomicU32 = AtomicU32::new(1 << 31);

fn next_badge_swap_key() -> u32 {
    NEXT_BADGE_SWAP_KEY.fetch_add(1, Ordering::Relaxed) | (1 << 31)
}

struct FirstCommentBadgeSweep {
    child: AnyElement,
    timeline: Entity<FirstCommentBadgeTimeline>,
    period: Arc<Vec<Hsla>>,
    paused: bool,
}

impl IntoElement for FirstCommentBadgeSweep {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FirstCommentBadgeSweep {
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
        let key = next_badge_swap_key();
        window.with_glyph_swap_key(key, |window| self.child.paint(window, cx));
        self.timeline.update(cx, |timeline, _| timeline.newest_key = key);
        let clip = window.content_mask().bounds;
        let visible = bounds.intersect(&clip);
        if self.paused || visible.size.width <= Pixels::ZERO || visible.size.height <= Pixels::ZERO {
            return;
        }
        FirstCommentBadgeSwaps { key, period: self.period.clone(), timeline: self.timeline.downgrade() }.schedule(window, cx);
    }
}

struct FirstCommentBadgeSwaps {
    key: u32,
    period: Arc<Vec<Hsla>>,
    timeline: WeakEntity<FirstCommentBadgeTimeline>,
}

impl FirstCommentBadgeSwaps {
    fn schedule(self, window: &Window, cx: &App) {
        let interval = Duration::from_secs_f64(FirstCommentBadge::FRAME_INTERVAL);
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(interval).await;
                let _ = cx.update(move |window, _| window.on_next_frame(move |window, cx| self.tick(window, cx)));
            })
            .detach();
    }

    fn tick(self, window: &mut Window, cx: &mut App) {
        let Some(timeline) = self.timeline.upgrade() else {
            return;
        };
        if timeline.read(cx).newest_key != self.key {
            return;
        }
        let shift = (1.0 - FirstCommentBadge::progress_now()).fract();
        let period = self.period.clone();
        if window.swap_glyph_colours(self.key, &|place: f32| strip_color(&period, (place + shift).fract())) {
            self.schedule(window, cx);
            return;
        }
        let timeline = self.timeline.clone();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = cx.update(|_, cx| timeline.update(cx, |_, cx| cx.notify()).ok());
            })
            .detach();
    }
}

impl FirstCommentBadge {
    const CYCLE: f64 = 4.2;
    const CROWN_SIZE: f32 = 11.0;
    const FRAME_INTERVAL: f64 = 1.0 / 30.0;
    /// seconds from the unix epoch to 2001-01-01

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    /// position in the badge cycle from 0 to 1 based on the clock
    fn progress_now() -> f32 {
        let since_reference = Date::now().time_interval_since_reference_date();
        (since_reference % Self::CYCLE / Self::CYCLE) as f32
    }
}

impl RenderOnce for FirstCommentBadge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let energy = RiseEnergySavingStore::shared(cx);
        let observed = energy.clone();
        let _ = window.use_keyed_state(self.child_id("energy-observer"), cx, move |_, cx| FirstCommentBadgeEnergyObserver {
            _subscription: cx.observe(&observed, |_, _, cx| cx.notify()),
        });
        let timeline = window.use_keyed_state(self.child_id("timeline"), cx, |_, _| FirstCommentBadgeTimeline {
            is_on_screen: true,
            newest_key: 0,
        });
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let paused = !timeline.read(cx).is_on_screen
            || reduce_motion(cx)
            || energy.read(cx).reduces(RiseEnergySavingFeature::InterfaceEffects);

        let text = SharedString::from(tr("comment_first_badge"));
        let period = FirstCommentGradient::period(&theme);
        let progress = if paused { 0.0 } else { Self::progress_now() };
        let swept = MainText::new(text.clone())
            .px(11.0)
            .font_weight(FontWeight::SEMIBOLD)
            .number_of_lines(1)
            .minimum_scale_factor(0.7)
            .color_runs(sampled_color_runs(&text, &period, (1.0 - progress).fract()));

        let visibility = timeline.clone();
        let reader = SkeletonWindowHeightReader::new(self.child_id("visibility"), move |visible, _, cx| {
            visibility.update(cx, |timeline, cx| {
                if timeline.is_on_screen != visible {
                    timeline.is_on_screen = visible;
                    cx.notify();
                }
            });
        });

        let crown = IconUi::sized("crown.fill", px(Self::CROWN_SIZE), SettingsIconPalette::subscription(), true)
            .map(|icon| icon.rotation_effect(-38.0));

        div()
            .relative()
            .min_w_0()
            .child(reader)
            .child(FirstCommentBadgeSweep { child: swept.into_any_element(), timeline, period: Arc::new(period), paused })
            .child(pass_through().absolute().left(px(-12.0)).top(px(-10.0)).children(crown))
    }
}

enum FirstCommentGradient {}

impl FirstCommentGradient {
    fn period(theme: &AppTheme) -> Vec<Hsla> {
        let base = theme.primary._100;
        let deep = theme.primary._300;
        vec![deep, base, Self::vivid(base), base, deep]
    }

    fn vivid(color: Hsla) -> Hsla {
        let value = color.l + color.s * color.l.min(1.0 - color.l);
        let saturation = if value > 0.0 { 2.0 * (1.0 - color.l / value) } else { 0.0 };
        let saturation = (saturation * 1.2).min(1.0);
        let lightness = 1.0 - saturation / 2.0;
        let hsl_saturation = if lightness >= 1.0 { 0.0 } else { (1.0 - lightness) / lightness.min(1.0 - lightness) };
        hsla(color.h, hsl_saturation, lightness, 1.0)
    }
}

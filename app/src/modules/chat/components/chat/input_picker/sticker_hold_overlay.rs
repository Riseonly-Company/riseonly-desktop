use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Bounds, BoxShadow, Context, DispatchPhase, Entity, EntityId, IntoElement, MouseDownEvent,
    MouseUpEvent, Pixels, Anchor, Render, SharedString, Size, StyleRefinement, Subscription, Window, anchored, black,
    canvas, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::hold_context_menu_ui::hold_context_menu_action_list::{HoldContextMenuActionList, HoldContextMenuInkKey};
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::{HoldContextMenuGlass, HoldContextMenuShadow};
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};

use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaPlaybackKind;
use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::core::ui::lottie_sticker::sticker_premium_effect::{StickerPremiumEffectGeometry, StickerPremiumEffectRegistry};
use crate::modules::chat::components::chat::bubble::media_letterbox_photo::MediaAspectFit;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::ChatInputStickerAsset;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::{
    StickerHoldPreviewPresenter, StickerHoldPreviewPresenterPhase, StickerPreviewAsset,
};

const ASSET_SIZE: f32 = 196.0;
const MEDIA_HEIGHT: f32 = ASSET_SIZE + 56.0;
const CURSOR_MENU_MARGIN: f32 = 8.0;
const CURSOR_PREVIEW_GAP: f32 = 8.0;
const SLOT_SCALE_INACTIVE: f32 = 0.82;
const FLAT_BACKDROP_OPACITY: f32 = 0.82;

fn presence_animation() -> MotionSpec {
    ease_out(0.2)
}

fn phase_animation() -> MotionSpec {
    spring(0.34, 0.82)
}

fn slot_animation() -> MotionSpec {
    spring(0.28, 0.78)
}

/// what a press that began on the open menu landed on, its release acts on the same part
#[derive(Clone, Debug, PartialEq)]
enum OwnedPress {
    Row(String),
    Media,
    Backdrop,
}

/// window layer showing the held sticker or gif and its menu after the hold escalates
/// mounted over root and every open sheet modal, only the newest one draws
pub struct StickerHoldOverlay {
    presenter: Entity<StickerHoldPreviewPresenter>,
    host_id: EntityId,
    slot_asset: [Option<StickerPreviewAsset>; 2],
    slot_key: [Option<String>; 2],
    slot_main: [Option<String>; 2],
    active_slot: usize,
    last_key: Option<String>,
    was_active: bool,
    presence: Animated<f32>,
    menu_progress: Animated<f32>,
    slot_progress: Animated<f32>,
    owned_press: Rc<RefCell<Option<OwnedPress>>>,
    media_frame: Rc<Cell<Option<Bounds<Pixels>>>>,
    cursor_panel: Entity<StickerHoldCursorPanel>,
    _subscriptions: Vec<Subscription>,
}

impl StickerHoldOverlay {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let presenter = StickerHoldPreviewPresenter::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let host_id = cx.entity_id();
        StickerHoldPreviewPresenter::register_overlay_host(host_id, cx);
        let cursor_panel = {
            let presenter = presenter.clone();
            cx.new(|cx| StickerHoldCursorPanel::new(presenter, cx))
        };
        let subscriptions = vec![
            cx.observe(&presenter, |this, _, cx| this.presenter_did_change(cx)),
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
            cx.on_release(move |_, cx| StickerHoldPreviewPresenter::unregister_overlay_host(host_id, cx)),
        ];
        Self {
            presenter,
            host_id,
            slot_asset: [None, None],
            slot_key: [None, None],
            slot_main: [None, None],
            active_slot: 0,
            last_key: None,
            was_active: false,
            presence: Animated::new(0.0),
            menu_progress: Animated::new(0.0),
            slot_progress: Animated::new(0.0),
            owned_press: Rc::new(RefCell::new(None)),
            media_frame: Rc::new(Cell::new(None)),
            cursor_panel,
            _subscriptions: subscriptions,
        }
    }

    fn presenter_did_change(&mut self, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        let (is_active, current_key, is_menu) = {
            let presenter = self.presenter.read(cx);
            (
                presenter.is_active(),
                presenter.current_key().map(str::to_owned),
                presenter.phase() == StickerHoldPreviewPresenterPhase::Menu,
            )
        };

        if is_active != self.was_active {
            self.was_active = is_active;
            self.presence.set_in(if is_active { 1.0 } else { 0.0 }, presence_animation(), now, cx);
            if is_active {
                self.sync_slots(current_key.clone(), now, cx);
            } else {
                self.slot_asset = [None, None];
                self.slot_key = [None, None];
                self.slot_main = [None, None];
            }
        } else if current_key != self.last_key {
            self.sync_slots(current_key.clone(), now, cx);
        }
        self.last_key = current_key;
        self.menu_progress.set_in(if is_menu { 1.0 } else { 0.0 }, phase_animation(), now, cx);
        cx.notify();
    }

    fn sync_slots(&mut self, new_key: Option<String>, now: std::time::Duration, cx: &mut Context<Self>) {
        let Some(new_key) = new_key else {
            return;
        };
        let (asset, main_emoji) = {
            let presenter = self.presenter.read(cx);
            (presenter.asset().cloned(), presenter.main_emoji().map(str::to_owned))
        };
        let Some(asset) = asset else {
            return;
        };
        if self.slot_key[self.active_slot].as_deref() == Some(new_key.as_str()) {
            return;
        }
        let next = 1 - self.active_slot;
        self.slot_asset[next] = Some(asset);
        self.slot_key[next] = Some(new_key);
        self.slot_main[next] = main_emoji;
        self.active_slot = next;
        self.slot_progress.set_in(next as f32, slot_animation(), now, cx);
    }

    /// nothing to show, presenter not active and fade out played
    /// root keeps an idle overlay cached and draws an active one uncached
    pub fn is_idle(&self, cx: &App) -> bool {
        !self.presenter.read(cx).is_active() && !self.presence.is_running(RiseClock::monotonic())
    }

    /// true when the presenter is active and this is the newest mounted overlay
    fn draws(&self, cx: &App) -> bool {
        let presenter = self.presenter.read(cx);
        presenter.is_active() && presenter.draws_overlay(self.host_id)
    }

    fn premium_effect(&self, asset: &StickerPreviewAsset, key: &str, playing: bool) -> Option<AnyElement> {
        let StickerPreviewAsset::Sticker(item) = asset else {
            return None;
        };
        let effect = StickerPremiumEffectRegistry::shared().effect_url(&item.file_url)?;
        let side = ASSET_SIZE * StickerPremiumEffectGeometry::CANVAS_SCALE;
        let inset = (side - ASSET_SIZE) / 2.0;
        Some(
            div()
                .absolute()
                .left(px(-inset))
                .top(px(-inset))
                .frame_size(side, side)
                .child(
                    LottieStickerView::new(SharedString::from(format!("sticker-hold-overlay/premium/{key}")), effect)
                        .size(side)
                        .is_playing(playing)
                        .playback_id(Some(format!("premium-effect|{key}"))),
                )
                .into_any_element(),
        )
    }

    fn slot_view(&self, slot: usize) -> AnyElement {
        let is_active = slot == self.active_slot;
        let mut column = vstack(14.0);
        if let Some(main) = self.slot_main[slot].as_ref().filter(|main| !main.is_empty()) {
            column = column.child(div().swiftui_font(Font::system_size(38.0)).child(main.clone()));
        }
        if let Some(asset) = &self.slot_asset[slot] {
            let key = self.slot_key[slot].clone().unwrap_or_else(|| format!("slot{slot}"));
            column = column.child(
                div()
                    .id(SharedString::from(format!("sticker-hold-overlay/{key}")))
                    .relative()
                    .frame_size(ASSET_SIZE, ASSET_SIZE)
                    .flex()
                    .items_center()
                    .justify_center()
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.45),
                        offset: point(px(0.0), px(10.0)),
                        blur_radius: px(22.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .children(self.premium_effect(asset, &key, is_active))
                    .child(asset_view(slot, asset, is_active)),
            );
        }
        column.into_any_element()
    }

    /// one view for both phases
    /// a branch per phase would swap identity under the spring and the chat would show through for a moment
    fn backdrop(&self, menu_progress: f32, uses_flat_chrome: bool, theme: &AppTheme) -> AnyElement {
        if uses_flat_chrome {
            return div().size_full().bg(theme.bg._100.opacity(FLAT_BACKDROP_OPACITY)).into_any_element();
        }
        let mut stack = zstack(Alignment::Center).size_full().child(div().absolute().inset_0().bg(black().opacity(0.42)));
        if menu_progress > 0.0 {
            stack = stack.child(
                zstack(Alignment::Center)
                    .absolute()
                    .inset_0()
                    .opacity(menu_progress.clamp(0.0, 1.0))
                    .child(HoldContextMenuGlass::new("sticker-hold-overlay-glass").tint(black().opacity(0.58)))
                    .child(div().absolute().inset_0().bg(black().opacity(0.24))),
            );
        }
        stack.into_any_element()
    }

    fn menu_top(&self, size: Size<f32>, media_y: f32, row_count: usize) -> f32 {
        let under_asset =
            size.height / 2.0 + media_y + MEDIA_HEIGHT / 2.0 + HoldContextMenuMetrics::MENU_TO_SOURCE_GAP * 2.0;
        let rows_height = row_count as f32 * HoldContextMenuMetrics::MENU_ROW_HEIGHT + 10.0;
        under_asset.min(size.height - rows_height - 34.0).max(0.0)
    }

    /// overlay owns every touch while its menu is up
    /// other trees could steal the click a menu row was drawn over so presses are caught in capture phase
    fn pointer_owner(&self) -> impl IntoElement {
        let owned = self.owned_press.clone();
        let media_frame = self.media_frame.clone();
        let presenter = self.presenter.clone();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (down_owned, down_media, down_presenter) = (owned.clone(), media_frame.clone(), presenter.clone());
                window.on_mouse_event(move |event: &MouseDownEvent, phase, _: &mut Window, cx: &mut App| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let press = match down_presenter.read(cx).menu_row_at(event.position) {
                        Some(id) => OwnedPress::Row(id),
                        None if down_media.get().is_some_and(|frame| frame.contains(&event.position)) => {
                            OwnedPress::Media
                        }
                        None => OwnedPress::Backdrop,
                    };
                    tracing::info!(target: "riseonly::X1627", press = ?press, point = ?event.position, "hold menu owns press");
                    *down_owned.borrow_mut() = Some(press);
                    cx.stop_propagation();
                });
                let (up_owned, up_media, up_presenter) = (owned.clone(), media_frame.clone(), presenter.clone());
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window: &mut Window, cx: &mut App| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let Some(press) = up_owned.borrow_mut().take() else {
                        return;
                    };
                    cx.stop_propagation();
                    match press {
                        OwnedPress::Row(id) => {
                            let still_on_row = up_presenter.read(cx).menu_row_at(event.position).as_deref() == Some(id.as_str());
                            if still_on_row {
                                StickerHoldPreviewPresenter::select_menu(&id, window, cx);
                            }
                        }
                        OwnedPress::Media => {
                            if up_media.get().is_some_and(|frame| frame.contains(&event.position)) {
                                StickerHoldPreviewPresenter::trigger_primary(window, cx);
                            }
                        }
                        OwnedPress::Backdrop => StickerHoldPreviewPresenter::dismiss(cx),
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    #[allow(clippy::too_many_arguments)]
    fn cursor_menu(
        &self,
        anchor: gpui::Point<Pixels>,
        is_menu: bool,
        menu_row_count: usize,
        menu_progress: f32,
        presence: f32,
        hit_testing: bool,
        viewport_height: f32,
        uses_flat_chrome: bool,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.media_frame.set(None);
        let shows_menu = (is_menu || menu_progress > 0.0) && menu_row_count > 0;
        let gif = if shows_menu {
            self.slot_asset[self.active_slot]
                .clone()
                .filter(|asset| matches!(asset, StickerPreviewAsset::Gif(..)))
                .map(|asset| (self.active_slot, asset))
        } else {
            None
        };
        let mut backdrop = div().id("sticker-hold-overlay-backdrop").absolute().inset_0();
        if gif.is_some() {
            backdrop = backdrop.child(if uses_flat_chrome {
                self.backdrop(menu_progress, true, theme)
            } else {
                div().size_full().bg(black().opacity(0.42)).into_any_element()
            });
        }
        if hit_testing {
            backdrop = backdrop.occlude().on_click(|_, _window, cx| StickerHoldPreviewPresenter::dismiss(cx));
        }
        let mut content = div().absolute().inset_0().child(backdrop);
        if shows_menu {
            let props = CursorPanelProps { anchor, viewport_height, gif };
            self.cursor_panel.update(cx, |panel, cx| panel.set_props(props, cx));
            content = content.child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(menu_progress.clamp(0.0, 1.0))
                    .child(AnyView::from(self.cursor_panel.clone()).cached(StyleRefinement::default().size_full())),
            );
        }
        if hit_testing {
            content = content.child(self.pointer_owner());
        } else {
            self.owned_press.borrow_mut().take();
        }
        div()
            .absolute()
            .inset_0()
            .size_full()
            .opacity(presence.clamp(0.0, 1.0))
            .child(content.keyboard_dismiss_excluded())
            .into_any_element()
    }
}

impl Render for StickerHoldOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let presence = self.presence.frame_value(now, window);
        let draws = self.draws(cx);
        if self.presenter.read(cx).is_active() {
            tracing::warn!(target: "riseonly::X1620", host = ?self.host_id, draws, presence, window = ?window.window_handle().window_id(), "overlay render");
        }
        if presence <= 0.0 && !draws {
            return div().into_any_element();
        }
        let (is_menu, is_pinned, menu_row_count, uses_flat_chrome, cursor_anchor) = {
            let presenter = self.presenter.read(cx);
            (
                presenter.phase() == StickerHoldPreviewPresenterPhase::Menu,
                presenter.is_pinned(),
                presenter.menu_actions.len(),
                presenter.uses_flat_chrome(),
                presenter.menu_anchor(),
            )
        };
        let menu_progress = self.menu_progress.frame_value(now, window);
        let slot_progress = self.slot_progress.frame_value(now, window);
        if self.presence.is_running(now) || self.menu_progress.is_running(now) || self.slot_progress.is_running(now) {
            window.request_animation_frame();
        }

        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let viewport = window.viewport_size();
        let geo = size(f32::from(viewport.width), f32::from(viewport.height));
        let media_y = -geo.height * 0.16;
        let hit_testing = draws && (is_menu || is_pinned);

        if let Some(anchor) = cursor_anchor {
            return self.cursor_menu(
                anchor,
                is_menu,
                menu_row_count,
                menu_progress,
                presence,
                hit_testing,
                geo.height,
                uses_flat_chrome,
                &theme,
                cx,
            );
        }

        let mut backdrop = div().id("sticker-hold-overlay-backdrop").absolute().inset_0().child(self.backdrop(
            menu_progress,
            uses_flat_chrome,
            &theme,
        ));
        if hit_testing {
            backdrop = backdrop.occlude().on_click(|_, _window, cx| StickerHoldPreviewPresenter::dismiss(cx));
        }

        let mut media = zstack(Alignment::Bottom).frame_size(ASSET_SIZE, MEDIA_HEIGHT);
        for slot in 0..2 {
            let weight = if slot == 1 { slot_progress } else { 1.0 - slot_progress };
            let scale = SLOT_SCALE_INACTIVE + (1.0 - SLOT_SCALE_INACTIVE) * weight;
            media = media.child(
                div()
                    .size_full()
                    .flex()
                    .items_end()
                    .justify_center()
                    .opacity(weight.clamp(0.0, 1.0))
                    .child(self.slot_view(slot))
                    .scale_effect(scale),
            );
        }
        let frame_sink = self.media_frame.clone();
        let mut media_layer = div().id("sticker-hold-overlay-media").child(media).child(
            canvas(move |bounds, _, _| frame_sink.set(Some(bounds)), |_, _, _, _| {}).absolute().inset_0(),
        );
        if hit_testing {
            media_layer =
                media_layer.occlude().on_click(|_, window, cx| StickerHoldPreviewPresenter::trigger_primary(window, cx));
        }

        let mut content = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(backdrop)
            .child(div().relative().top(px(media_y)).child(media_layer));

        if (is_menu || menu_progress > 0.0) && menu_row_count > 0 {
            let scale = HoldContextMenuMetrics::INITIAL_PANEL_SCALE
                + (1.0 - HoldContextMenuMetrics::INITIAL_PANEL_SCALE) * menu_progress.clamp(0.0, 1.0);
            content = content.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(px(self.menu_top(geo, media_y, menu_row_count)))
                    .child(
                        div()
                            .flex_none()
                            .opacity(menu_progress.clamp(0.0, 1.0))
                            .child(hold_menu(self.presenter.read(cx), uses_flat_chrome, &theme))
                            .scale_effect_anchored(scale, UnitPoint::TOP),
                    ),
            );
        }

        if hit_testing {
            content = content.child(self.pointer_owner());
        } else {
            self.owned_press.borrow_mut().take();
        }

        div()
            .absolute()
            .inset_0()
            .size_full()
            .opacity(presence.clamp(0.0, 1.0))
            .child(content.keyboard_dismiss_excluded())
            .into_any_element()
    }
}

fn asset_view(slot: usize, asset: &StickerPreviewAsset, playing: bool) -> AnyElement {
    match asset {
        StickerPreviewAsset::Sticker(item) => {
            ChatInputStickerAsset::new(item.clone(), ASSET_SIZE).is_playing(playing).into_any_element()
        }
        StickerPreviewAsset::UnicodeEmoji(text) => div()
            .frame_size(ASSET_SIZE, ASSET_SIZE)
            .flex()
            .items_center()
            .justify_center()
            .swiftui_font(Font::system_size(ASSET_SIZE * 0.74))
            .child(text.clone())
            .into_any_element(),
        StickerPreviewAsset::Gif(url, aspect) => {
            let fitted = MediaAspectFit::aspect_fit_size(
                size(px(aspect.max(0.05)), px(1.0)),
                size(px(ASSET_SIZE), px(ASSET_SIZE)),
            );
            div()
                .w(fitted.width)
                .h(fitted.height)
                .clip_shape(Shape::continuous(18.0))
                .child(
                    AnimatedGifView::new(("sticker-hold-overlay-gif", slot), url.clone())
                        .content_mode(AnimatedGifContentMode::ScaleAspectFill)
                        .is_playing(playing)
                        .playback_kind(AnimatedMediaPlaybackKind::OpenedAnimation),
                )
                .into_any_element()
        }
    }
}

fn hold_menu(presenter: &StickerHoldPreviewPresenter, uses_flat_chrome: bool, theme: &AppTheme) -> AnyElement {
    let rows: Vec<HoldContextMenuAction> = presenter
        .menu_actions
        .iter()
        .map(|action| {
            HoldContextMenuAction::new(action.id.clone(), action.title.clone())
                .system_image(action.system_image.clone())
                .destructive(action.destructive)
                .guide_anchor(action.guide_anchor)
        })
        .collect();
    let on_select = Rc::new(|id: String, window: &mut Window, cx: &mut App| {
        StickerHoldPreviewPresenter::select_menu(&id, window, cx)
    }) as Rc<dyn Fn(String, &mut Window, &mut App)>;
    let list = HoldContextMenuActionList::new(
        "sticker-hold-overlay-menu",
        rows,
        presenter.highlighted_menu_id().map(str::to_owned),
        on_select,
    )
    .hit_targets(Some(presenter.menu_hit_targets.clone()))
    .chrome(!uses_flat_chrome)
    .row_line_limit(2);
    if !uses_flat_chrome {
        return list.into_any_element();
    }
    let radius = HoldContextMenuMetrics::MENU_CORNER_RADIUS;
    let shape = Shape::continuous(radius);
    div()
        .relative()
        .flex_none()
        .child(div().absolute().inset_0().child(HoldContextMenuShadow::new(radius)))
        .child(
            div()
                .relative()
                .bg(theme.bg._100)
                .clip_shape(shape)
                .child(env_scope::<HoldContextMenuInkKey>(theme.text.primary).child(list))
                .child(centred_stroke(theme.border._100, 0.66, shape).absolute().inset_0()),
        )
        .into_any_element()
}

#[derive(Clone, PartialEq)]
struct CursorPanelProps {
    anchor: gpui::Point<Pixels>,
    viewport_height: f32,
    gif: Option<(usize, StickerPreviewAsset)>,
}

/// context click gif preview and menu
/// renders again only on new props, presenter change or theme change
/// the overlay fades it without rendering
struct StickerHoldCursorPanel {
    presenter: Entity<StickerHoldPreviewPresenter>,
    props: Option<CursorPanelProps>,
    _subscriptions: Vec<Subscription>,
}

impl StickerHoldCursorPanel {
    fn new(presenter: Entity<StickerHoldPreviewPresenter>, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&presenter, |_, _, cx| cx.notify()),
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
        ];
        Self { presenter, props: None, _subscriptions: subscriptions }
    }

    fn set_props(&mut self, props: CursorPanelProps, cx: &mut Context<Self>) {
        if self.props.as_ref() == Some(&props) {
            return;
        }
        self.props = Some(props);
        cx.notify();
    }
}

impl Render for StickerHoldCursorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(props) = self.props.clone() else {
            return div().into_any_element();
        };
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let presenter = self.presenter.read(cx);
        let menu_row_count = presenter.menu_actions.len();
        let uses_flat_chrome = presenter.uses_flat_chrome();
        let anchor = props.anchor;
        let viewport_height = props.viewport_height;
        let mut content = div().absolute().inset_0().size_full();
        if props.gif.is_some() && !uses_flat_chrome {
            content = content
                .child(HoldContextMenuGlass::new("sticker-hold-overlay-glass").tint(black().opacity(0.58)))
                .child(div().absolute().inset_0().bg(black().opacity(0.24)));
        }
        let gif_preview = props.gif.as_ref().map(|(slot, asset)| {
            div()
                .flex_none()
                .shadow(vec![BoxShadow {
                    color: black().opacity(0.45),
                    offset: point(px(0.0), px(10.0)),
                    blur_radius: px(22.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
                .child(asset_view(*slot, asset, true))
        });
        let menu_height = menu_row_count as f32 * HoldContextMenuMetrics::MENU_ROW_HEIGHT + 10.0;
        let opens_up = f32::from(anchor.y) + menu_height > viewport_height - CURSOR_MENU_MARGIN;
        if let Some(gif_preview) = gif_preview {
            let y = f32::from(anchor.y);
            let below_menu = y + menu_height + CURSOR_PREVIEW_GAP;
            let (preview_anchor, preview_y) = if opens_up {
                (Anchor::BottomLeft, y - menu_height - CURSOR_PREVIEW_GAP)
            } else if below_menu + ASSET_SIZE <= viewport_height - CURSOR_MENU_MARGIN {
                (Anchor::TopLeft, below_menu)
            } else {
                (Anchor::BottomLeft, y - CURSOR_PREVIEW_GAP)
            };
            content = content.child(
                anchored()
                    .position(point(anchor.x, px(preview_y)))
                    .anchor(preview_anchor)
                    .snap_to_window_with_margin(px(CURSOR_MENU_MARGIN))
                    .child(gif_preview),
            );
        }
        content
            .child(
                anchored()
                    .position(anchor)
                    .anchor(if opens_up { Anchor::BottomLeft } else { Anchor::TopLeft })
                    .child(div().flex_none().child(hold_menu(presenter, uses_flat_chrome, &theme))),
            )
            .into_any_element()
    }
}

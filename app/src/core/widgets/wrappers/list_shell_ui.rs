use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Div, Edges, Element, ElementId, Entity, EntityId, GlobalElementId, Hsla, InspectorElementId, LayoutId, Length, Position, Style, InteractiveElement, IntoElement, List, ParentElement, Pixels,
    RenderOnce, SharedString, Stateful, StyleRefinement, Styled, Window, div, px,
};
use rise_theme::{RiseThemeAppearance, ThemeStore};
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::guide_anchor_ui::guide_anchor_registry::{GuideAnchorCoverKind, GuideAnchorCoverModifiers};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::page_header_ui::PageHeaderUi;
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowStyleColorRole, ScrollChromeShadowUi};
use rise_ui::scroll_edge_blur_ui::{ScrollEdgeBlurFalloff, ScrollEdgeBlurUi};
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::swiftui::environment_values::accessibility_reduce_transparency;
use rise_ui::swiftui::linear_gradient::GradientStop;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::scroll_view::ScrollView;
use rise_ui::swiftui::shapes::pass_through;

use crate::core::widgets::wrappers::list_shell_header_collapse::ListShellHeaderCollapse;

type ListShellAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListShellCoordinateSpace {
    Content,
}

impl ListShellCoordinateSpace {
    /// content frame of the shell in window coordinates, known one frame after layout
    /// none outside a shell or before its first layout
    pub fn frame(self, cx: &App) -> Option<Bounds<Pixels>> {
        match self {
            Self::Content => env::<ListShellContentCoordinateSpaceKey>(cx).and_then(|frame| frame.read(cx).0),
        }
    }
}

pub struct ListShellFloatingTopInsetKey;

impl EnvKey for ListShellFloatingTopInsetKey {
    type Value = f32;

    fn default_value() -> f32 {
        0.0
    }
}

pub struct ListShellSafeAreaInsetsKey;

impl EnvKey for ListShellSafeAreaInsetsKey {
    type Value = EdgeInsets;

    fn default_value() -> EdgeInsets {
        EdgeInsets::new(0.0, 0.0, 0.0, 0.0)
    }
}

/// what a center page paints beside the column it is held to
/// a page with its own background (grouped settings) hands the color here so the slot fills the window
#[derive(Clone, Default)]
pub struct PageColumnBackdrop {
    color: Rc<Cell<Option<Hsla>>>,
    top_inset: Rc<Cell<f32>>,
    header_fade: Rc<Cell<Option<PageColumnHeaderFade>>>,
    blurred_header: Rc<RefCell<Option<PageColumnBlurredHeader>>>,
}

/// header of a page whose scrim blurs the rows under it, for the stack slot to repeat beside a
/// narrow page
/// page color, blur tint and scrim top to bottom over height
#[derive(Clone, Debug, PartialEq)]
pub struct PageColumnBlurredHeader {
    pub background: Hsla,
    pub height: f32,
    pub tint: Vec<GradientStop>,
    pub shadow: Vec<GradientStop>,
}

/// solid header scrim and its fade in points from the column top, for the stack slot to repeat
/// beside a narrow page
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageColumnHeaderFade {
    pub color: Hsla,
    pub solid_end: f32,
    pub end: f32,
}

impl PageColumnBackdrop {
    pub fn color(&self) -> Option<Hsla> {
        self.color.get()
    }

    /// where the fill starts, the floating chrome of the shell stays window color above it
    pub fn top_inset(&self) -> f32 {
        self.top_inset.get()
    }

    /// header fade the page paints over its own column, if its an opaque one
    pub fn header_fade(&self) -> Option<PageColumnHeaderFade> {
        self.header_fade.get()
    }

    /// blurred header the page paints over its own column, if it has one
    pub fn blurred_header(&self) -> Option<PageColumnBlurredHeader> {
        self.blurred_header.borrow().clone()
    }
}

pub struct PageColumnBackdropKey;

impl EnvKey for PageColumnBackdropKey {
    type Value = Option<PageColumnBackdrop>;

    fn default_value() -> Option<PageColumnBackdrop> {
        None
    }
}

/// called from a page render with the color its scroller paints
/// outside a capped stack slot nothing happens
pub fn page_column_backdrop(color: Hsla, cx: &App) {
    if let Some(backdrop) = env::<PageColumnBackdropKey>(cx) {
        backdrop.color.set(Some(color));
    }
}

fn note_page_column_top_inset(top_inset: f32, cx: &App) {
    if let Some(backdrop) = env::<PageColumnBackdropKey>(cx) {
        backdrop.top_inset.set(top_inset);
    }
}

fn note_page_column_blurred_header(blurred_header: Option<PageColumnBlurredHeader>, cx: &App) {
    if let Some(backdrop) = env::<PageColumnBackdropKey>(cx)
        && *backdrop.blurred_header.borrow() != blurred_header
    {
        *backdrop.blurred_header.borrow_mut() = blurred_header;
    }
}

fn note_page_column_header_fade(header_fade: Option<PageColumnHeaderFade>, cx: &App) {
    if let Some(backdrop) = env::<PageColumnBackdropKey>(cx) {
        backdrop.header_fade.set(header_fade);
    }
}

/// carries the frame behind the shell content coordinate space
pub struct ListShellContentCoordinateSpaceKey;

impl EnvKey for ListShellContentCoordinateSpaceKey {
    type Value = Option<Entity<ListShellContentFrame>>;

    fn default_value() -> Option<Entity<ListShellContentFrame>> {
        None
    }

    fn fingerprint(_: &Self::Value) -> Option<u64> {
        None
    }
}

/// room the shell content keeps under the floating chrome, reading it marks the content as keeping it
pub fn list_shell_floating_top_inset(cx: &App) -> f32 {
    if let Some(read) = env::<ListShellInsetReadKey>(cx) {
        read.set(true);
    }
    env::<ListShellFloatingTopInsetKey>(cx)
}

struct ListShellInsetReadKey;

impl EnvKey for ListShellInsetReadKey {
    type Value = Option<Rc<Cell<bool>>>;

    fn default_value() -> Option<Rc<Cell<bool>>> {
        None
    }

    fn fingerprint(_: &Self::Value) -> Option<u64> {
        None
    }
}

/// right panel strip over a page and the panel record of pages floating under it
#[derive(Clone)]
pub struct PanelStripInset {
    height: f32,
    page: EntityId,
    floats: Rc<RefCell<HashMap<EntityId, bool>>>,
}

impl PanelStripInset {
    pub fn new(height: f32, page: EntityId, floats: Rc<RefCell<HashMap<EntityId, bool>>>) -> Self {
        Self { height, page, floats }
    }
}

/// set by the right panel for each page it shows, none elsewhere
pub struct PanelStripInsetKey;

impl EnvKey for PanelStripInsetKey {
    type Value = Option<PanelStripInset>;

    fn default_value() -> Option<PanelStripInset> {
        None
    }

    fn fingerprint(value: &Option<PanelStripInset>) -> Option<u64> {
        value.as_ref().map(|inset| u64::from(inset.height.to_bits()))
    }
}

struct ListShellInsetHonored(Rc<Cell<bool>>);

struct PanelStripChrome {
    strip: PanelStripInset,
    honored: Rc<Cell<bool>>,
    child: AnyElement,
}

impl IntoElement for PanelStripChrome {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for PanelStripChrome {
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
    ) -> (LayoutId, ()) {
        let child = self.child.request_layout(window, cx);
        let floats = self.honored.get();
        self.strip.floats.borrow_mut().insert(self.strip.page, floats);
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.inset = Edges {
            top: px(if floats { self.strip.height } else { 0.0 }).into(),
            right: px(0.0).into(),
            bottom: Length::Auto,
            left: px(0.0).into(),
        };
        (window.request_layout(style, [child], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

pub fn list_shell_safe_area_insets(cx: &App) -> EdgeInsets {
    env::<ListShellSafeAreaInsetsKey>(cx)
}

/// scroller of the shell content, its rows start under the floating chrome and scroll up beneath it
/// the top margin is padding inside the scroller so it scrolls with the content
pub trait ListShellScrollUnderChromeModifiers: Sized {
    fn list_shell_scroll_under_chrome(self, cx: &App) -> Self;

    /// same as the plain one but for screens whose content is ready on the first frame
    /// the flag changes nothing here since the margin is read on every render
    fn list_shell_scroll_under_chrome_resetting(self, resets_on_safe_area_change: bool, cx: &App) -> Self {
        let _ = resets_on_safe_area_change;
        self.list_shell_scroll_under_chrome(cx)
    }
}

impl ListShellScrollUnderChromeModifiers for ScrollView {
    fn list_shell_scroll_under_chrome(self, _cx: &App) -> Self {
        self.content_top_margin(list_shell_floating_top_inset)
    }
}

fn pad_under_chrome<E: Styled>(element: E, cx: &App) -> E {
    let inset = list_shell_floating_top_inset(cx);
    if inset > 0.0 { element.pt(px(inset)) } else { element }
}

impl ListShellScrollUnderChromeModifiers for Div {
    fn list_shell_scroll_under_chrome(self, cx: &App) -> Self {
        pad_under_chrome(self, cx)
    }
}

impl ListShellScrollUnderChromeModifiers for Stateful<Div> {
    fn list_shell_scroll_under_chrome(self, cx: &App) -> Self {
        pad_under_chrome(self, cx)
    }
}

impl ListShellScrollUnderChromeModifiers for List {
    fn list_shell_scroll_under_chrome(self, cx: &App) -> Self {
        pad_under_chrome(self, cx)
    }
}

/// for a container that isnt itself the scroller and is built as the shell argument
/// it reads the inset when it renders inside the shell scope
pub fn list_shell_under_chrome_box() -> ListShellUnderChromeBox {
    ListShellUnderChromeBox { base: div() }
}

/// the element list_shell_under_chrome_box returns
#[derive(IntoElement)]
pub struct ListShellUnderChromeBox {
    base: Div,
}

impl Styled for ListShellUnderChromeBox {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for ListShellUnderChromeBox {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for ListShellUnderChromeBox {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        pad_under_chrome(self.base, cx)
    }
}

const LIST_SHELL_HEADER_BODY_HEIGHT_DEFAULT: f32 = PageHeaderMetrics::HEIGHT;

struct ListShellHeaderBodyHeight(f32);

pub struct ListShellContentFrame(Option<Bounds<Pixels>>);

#[derive(IntoElement)]
pub struct ListShellUi {
    title: Option<SharedString>,
    subtitle: Option<SharedString>,
    center_slot: Option<AnyElement>,
    center_slot_self_styled: bool,
    center_slot_fills_width: bool,
    use_safe_zone: bool,
    shows_back_button: bool,
    on_back: ListShellAction,
    right_jsx: Option<AnyElement>,
    left_jsx: Option<AnyElement>,
    fills_viewport_background: bool,
    respects_keyboard_safe_area: bool,
    header_chrome_shadow: Option<ScrollChromeShadowStyle>,
    header_collapse: Option<Entity<ListShellHeaderCollapse>>,
    below_header: Option<AnyElement>,
    content: Option<AnyElement>,
}

impl Default for ListShellUi {
    fn default() -> Self {
        Self {
            title: None,
            subtitle: None,
            center_slot: None,
            center_slot_self_styled: false,
            center_slot_fills_width: false,
            use_safe_zone: true,
            shows_back_button: true,
            on_back: Rc::new(|_, _| {}),
            right_jsx: None,
            left_jsx: None,
            fills_viewport_background: true,
            respects_keyboard_safe_area: false,
            header_chrome_shadow: Some(ScrollChromeShadowStyle::list_shell_header()),
            header_collapse: None,
            below_header: None,
            content: None,
        }
    }
}

impl ListShellUi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<Option<SharedString>>) -> Self {
        self.title = title.into();
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<Option<SharedString>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn center_slot(mut self, center_slot: impl IntoElement) -> Self {
        self.center_slot = Some(center_slot.into_any_element());
        self
    }

    pub fn center_slot_self_styled(mut self, center_slot_self_styled: bool) -> Self {
        self.center_slot_self_styled = center_slot_self_styled;
        self
    }

    pub fn center_slot_fills_width(mut self, center_slot_fills_width: bool) -> Self {
        self.center_slot_fills_width = center_slot_fills_width;
        self
    }

    pub fn use_safe_zone(mut self, use_safe_zone: bool) -> Self {
        self.use_safe_zone = use_safe_zone;
        self
    }

    pub fn shows_back_button(mut self, shows_back_button: bool) -> Self {
        self.shows_back_button = shows_back_button;
        self
    }

    pub fn on_back(mut self, on_back: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_back = Rc::new(on_back);
        self
    }

    pub fn right_jsx(mut self, right_jsx: impl IntoElement) -> Self {
        self.right_jsx = Some(right_jsx.into_any_element());
        self
    }

    pub fn left_jsx(mut self, left_jsx: impl IntoElement) -> Self {
        self.left_jsx = Some(left_jsx.into_any_element());
        self
    }

    pub fn fills_viewport_background(mut self, fills_viewport_background: bool) -> Self {
        self.fills_viewport_background = fills_viewport_background;
        self
    }

    pub fn respects_keyboard_safe_area(mut self, respects_keyboard_safe_area: bool) -> Self {
        self.respects_keyboard_safe_area = respects_keyboard_safe_area;
        self
    }

    pub fn header_chrome_shadow(mut self, header_chrome_shadow: Option<ScrollChromeShadowStyle>) -> Self {
        self.header_chrome_shadow = header_chrome_shadow;
        self
    }

    pub fn header_collapse(mut self, header_collapse: Entity<ListShellHeaderCollapse>) -> Self {
        self.header_collapse = Some(header_collapse);
        self
    }

    pub fn below_header(mut self, below_header: impl IntoElement) -> Self {
        self.below_header = Some(below_header.into_any_element());
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    fn header_chrome_shadow_height(&self, header_body_height: f32, safe_top: f32) -> f32 {
        let Some(s) = self.header_chrome_shadow else {
            return 0.0;
        };
        let core = s.extent.max(header_body_height + s.overflow_below_header);
        core + safe_top
    }

    /// bottom inset only counts when its the window own, not the keyboard
    /// desktop has no safe area or keyboard so nothing below the content is reserved
    fn container_bottom_inset(&self, geometry_inset: f32) -> f32 {
        if geometry_inset <= 0.0 {
            return 0.0;
        }
        let window_inset = 0.0_f32;
        if geometry_inset > window_inset + 1.0 {
            return 0.0;
        }
        geometry_inset.min(window_inset)
    }
}

impl RenderOnce for ListShellUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_bg = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let header_body_height_state =
            window.use_state(cx, |_, _| ListShellHeaderBodyHeight(LIST_SHELL_HEADER_BODY_HEIGHT_DEFAULT));
        let content_frame_state = window.use_state(cx, |_, _| ListShellContentFrame(None));
        let header_body_height = header_body_height_state.read(cx).0;

        let safe_top = 0.0_f32;
        let strip = env::<PanelStripInsetKey>(cx).filter(|_| self.use_safe_zone);
        let strip_height = strip.as_ref().map_or(0.0, |strip| strip.height);
        let honored = window.use_state(cx, |_, _| ListShellInsetHonored(Rc::new(Cell::new(false)))).read(cx).0.clone();
        let header_top_inset = if self.use_safe_zone { safe_top + header_body_height } else { 0.0 };
        let content_top_inset = header_top_inset + strip_height;
        note_page_column_top_inset(header_top_inset, cx);
        let geometry_bottom_inset = 0.0_f32;
        let safe_bottom_inset = if self.respects_keyboard_safe_area {
            self.container_bottom_inset(geometry_bottom_inset)
        } else {
            geometry_bottom_inset
        };
        let shell_insets = EdgeInsets::new(
            if self.use_safe_zone { safe_top } else { 0.0 },
            0.0,
            safe_bottom_inset,
            0.0,
        );
        let shadow_height = self.header_chrome_shadow_height(header_body_height, if self.use_safe_zone { safe_top } else { 0.0 });
        let header_fade = self
            .header_chrome_shadow
            .map(ScrollChromeShadowStyle::over_backdrop_blur)
            .filter(|style| {
                style.opaque_through_header
                    && style.color_role == ScrollChromeShadowStyleColorRole::Surface
                    && self.header_collapse.is_none()
            })
            .filter(|_| shadow_height > 0.0)
            .map(|style| PageColumnHeaderFade {
                color: theme_bg.opacity(style.peak_opacity.clamp(0.0, 1.0) as f32),
                solid_end: (shadow_height - style.overflow_below_header).max(0.0),
                end: shadow_height,
            });
        let blurred_header = self
            .header_chrome_shadow
            .filter(|_| {
                header_fade.is_none()
                    && self.fills_viewport_background
                    && self.header_collapse.is_none()
                    && shadow_height > 0.0
            })
            .map(ScrollChromeShadowStyle::over_backdrop_blur)
            .filter(|style| style.blurs_backdrop)
            .map(|style| {
                let shadow = ListShellHeaderShadow {
                    style,
                    full_height: shadow_height,
                    collapse: None,
                };
                let blur_solid_end = (shadow_height - style.overflow_below_header).max(0.0) / shadow_height;
                let tint_peak = ScrollEdgeBlurFalloff::header_tint_peak(
                    ThemeStore::shared(cx).read(cx).resolved_appearance() == RiseThemeAppearance::Dark,
                );
                PageColumnBlurredHeader {
                    background: theme_bg,
                    height: shadow_height,
                    tint: ScrollEdgeBlurFalloff::tint_stops_with_peak(
                        rise_ui::theme(cx).bg._100,
                        blur_solid_end,
                        tint_peak,
                        accessibility_reduce_transparency(cx),
                    ),
                    shadow: ScrollChromeShadowUi::new(style)
                        .solid_fraction(shadow.solid_fraction(shadow_height))
                        .stops(cx),
                }
            });
        note_page_column_header_fade(header_fade, cx);
        note_page_column_blurred_header(blurred_header, cx);
        let blocks_touches = self.use_safe_zone || self.header_collapse.is_some();

        let mut page_header = PageHeaderUi::new()
            .title(self.title)
            .subtitle(self.subtitle)
            .shows_back_button(self.shows_back_button)
            .on_back({
                let on_back = self.on_back.clone();
                move |window, cx| on_back(window, cx)
            })
            .center_slot_self_styled(self.center_slot_self_styled)
            .center_slot_fills_width(self.center_slot_fills_width);
        if let Some(right_jsx) = self.right_jsx {
            page_header = page_header.right_jsx(right_jsx);
        }
        if let Some(left_jsx) = self.left_jsx {
            page_header = page_header.left_jsx(left_jsx);
        }
        if let Some(center_slot) = self.center_slot {
            page_header = page_header.center_slot(center_slot);
        }

        let mut root = div().relative().size_full().overflow_hidden();

        if self.fills_viewport_background {
            root = root.child(div().absolute().inset_0().bg(theme_bg));
        }

        let content = self.content.unwrap_or_else(|| div().into_any_element());
        root = root.child(
            div()
                .absolute()
                .inset_0()
                .child(
                    env_scope::<ListShellSafeAreaInsetsKey>(shell_insets).child(
                        env_scope::<ListShellFloatingTopInsetKey>(content_top_inset).child(
                            env_scope::<ListShellInsetReadKey>(Some(honored.clone())).child(
                                env_scope::<PanelStripInsetKey>(None).child(
                                    env_scope::<ListShellContentCoordinateSpaceKey>(Some(content_frame_state.clone()))
                                        .child(div().size_full().child(content)),
                                ),
                            ),
                        ),
                    ),
                )
                .child(measure("list-shell-content-frame", move |bounds, _, cx| {
                    content_frame_state.update(cx, |frame, cx| {
                        if frame.0 != Some(bounds) {
                            let resized = frame.0.is_some_and(|frame| frame.size != bounds.size);
                            frame.0 = Some(bounds);
                            if resized {
                                cx.notify();
                            }
                        }
                    });
                })),
        );

        let mut chrome = div().absolute().top_0().left_0().right_0();
        if let Some(style) = self.header_chrome_shadow {
            chrome = chrome.child(
                pass_through()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .child(ListShellHeaderShadow {
                        style: style.over_backdrop_blur(),
                        full_height: shadow_height,
                        collapse: self.header_collapse.clone(),
                    }),
            );
        }

        let title_bar = div().w_full().child(page_header);
        let title_bar = if blocks_touches { title_bar.block_mouse_except_scroll() } else { title_bar };

        let mut header_body = div().relative().w_full().flex().flex_col().flex_none().child(title_bar);
        if let Some(below_header) = self.below_header {
            let below_header = match (&self.header_collapse, blocks_touches) {
                (Some(collapse), true) => ListShellBelowHeaderBlocker {
                    collapse: collapse.clone(),
                    content: below_header,
                }
                .into_any_element(),
                (None, true) => div().w_full().child(below_header).block_mouse_except_scroll().into_any_element(),
                (_, false) => div().w_full().child(below_header).into_any_element(),
            };
            header_body = header_body.child(below_header);
        }
        header_body = header_body.child(measure("list-shell-header-body-height", move |bounds, _, cx| {
            let height = f32::from(bounds.size.height);
            header_body_height_state.update(cx, |state, cx| {
                if state.0 != height {
                    state.0 = height;
                    cx.notify();
                }
            });
        }));

        let header_body = header_body.guide_anchor_cover("list-shell-guide-chrome", GuideAnchorCoverKind::Chrome);
        let chrome = chrome.child(header_body);
        match strip {
            Some(strip) => root.child(PanelStripChrome { strip, honored, child: chrome.into_any_element() }),
            None => root.child(chrome),
        }
    }
}

#[derive(IntoElement)]
struct ListShellBelowHeaderBlocker {
    collapse: Entity<ListShellHeaderCollapse>,
    content: AnyElement,
}

impl RenderOnce for ListShellBelowHeaderBlocker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let folded = self.collapse.read(cx).presented_offset(window).max(0.0);
        div()
            .relative()
            .w_full()
            .child(div().absolute().top_0().left_0().right_0().bottom(px(folded)).block_mouse_except_scroll())
            .child(self.content)
    }
}

/// scrim under the chrome, shortened by the folded away amount
/// reads the collapse here so a scroll frame redraws one gradient and not the whole shell
#[derive(IntoElement)]
struct ListShellHeaderShadow {
    style: ScrollChromeShadowStyle,
    full_height: f32,
    collapse: Option<Entity<ListShellHeaderCollapse>>,
}

impl ListShellHeaderShadow {
    fn solid_fraction(&self, height: f32) -> Option<f32> {
        if !self.style.opaque_through_header || height <= 0.0 {
            return None;
        }
        Some((height - self.style.overflow_below_header).max(0.0) / height)
    }
}

impl RenderOnce for ListShellHeaderShadow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let folded = self
            .collapse
            .as_ref()
            .map_or(0.0, |collapse| collapse.read(cx).presented_offset(window));
        let height = (self.full_height - folded).max(0.0);
        let solid_fraction = self.solid_fraction(height);
        let blur_solid_end = if height > 0.0 {
            (height - self.style.overflow_below_header).max(0.0) / height
        } else {
            0.0
        };
        let tint_peak = ScrollEdgeBlurFalloff::header_tint_peak(
            ThemeStore::shared(cx).read(cx).resolved_appearance() == RiseThemeAppearance::Dark,
        );
        let blur = self.style.blurs_backdrop.then(|| {
            div().absolute().top_0().left_0().size_full().child(
                ScrollEdgeBlurUi::new().solid_end(blur_solid_end).tint_peak(tint_peak),
            )
        });
        div()
            .relative()
            .w_full()
            .h(px(height))
            .children(blur)
            .child(ScrollChromeShadowUi::new(self.style).solid_fraction(solid_fraction))
    }
}

#[derive(IntoElement)]
pub struct SettingsWrapperUi {
    title: SharedString,
    subtitle: Option<SharedString>,
    use_safe_zone: bool,
    shows_back_button: bool,
    right_jsx: Option<AnyElement>,
    respects_keyboard_safe_area: bool,
    on_back: ListShellAction,
    content: Option<AnyElement>,
}

impl SettingsWrapperUi {
    pub fn new(title: impl Into<SharedString>, on_back: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            use_safe_zone: true,
            shows_back_button: true,
            right_jsx: None,
            respects_keyboard_safe_area: true,
            on_back: Rc::new(on_back),
            content: None,
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<Option<SharedString>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn use_safe_zone(mut self, use_safe_zone: bool) -> Self {
        self.use_safe_zone = use_safe_zone;
        self
    }

    pub fn shows_back_button(mut self, shows_back_button: bool) -> Self {
        self.shows_back_button = shows_back_button;
        self
    }

    pub fn right_jsx(mut self, right_jsx: impl IntoElement) -> Self {
        self.right_jsx = Some(right_jsx.into_any_element());
        self
    }

    pub fn respects_keyboard_safe_area(mut self, respects_keyboard_safe_area: bool) -> Self {
        self.respects_keyboard_safe_area = respects_keyboard_safe_area;
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }
}

impl RenderOnce for SettingsWrapperUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_back = self.on_back.clone();
        let mut shell = ListShellUi::new()
            .title(self.title)
            .subtitle(self.subtitle)
            .use_safe_zone(self.use_safe_zone)
            .shows_back_button(self.shows_back_button)
            .on_back(move |window, cx| on_back(window, cx))
            .fills_viewport_background(true)
            .respects_keyboard_safe_area(self.respects_keyboard_safe_area)
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()));
        if let Some(right_jsx) = self.right_jsx {
            shell = shell.right_jsx(right_jsx);
        }
        if let Some(content) = self.content {
            shell = shell.content(content);
        }
        shell
    }
}

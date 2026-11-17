use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, Div, Entity, FontWeight, HighlightStyle, Hsla,
    InteractiveElement, InteractiveText, IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement, Pixels,
    Point, Render, SharedString, Size, StatefulInteractiveElement, Styled, StyledImage, StyledText, Task, TextRun,
    WeakEntity, Window, black, div, img, point, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::theme::app_theme::{AppTheme, alpha};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::{LiquidChrome, LiquidChromeModifiers};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT, SwiftUiFont};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::minimum_scale_factor::visual_line_count;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::transition::TransitionModifiers;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::view_screen_snapshot::ViewScreenSource;

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{
    HoldContextMenuActionHandler, HoldContextMenuPresenter, HoldContextMenuRequest, HoldContextMenuRequestID,
    HoldContextMenuSourceLift,
};
use crate::core::ui::hold_context_menu_ui::hold_gesture_plate::HoldGesturePlate;
use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntry, ChatMessageRenderEntryKind,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListReusableView, RiseManualListSelectionPresentable, RiseManualListTopStickyHeaderFading,
    RiseManualListTopStickyHeaderInteractive,
};
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_image_layer::ChatWallpaperBitmapCache;
use crate::modules::chat::components::chat::message_system_pill::{
    MessageSystemGiftCard, MessageSystemPersonLink, MessageSystemPill, MessageTopicDivider,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageSystemGiftItem, MessageSystemKind, MessageSystemWallpaperPreview,
};
use crate::modules::gifts::components::gift_message_card::GiftMessageCard;

pub type RiseNativeSystemRowSelectTopic = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;
/// is_pinned tells the chip floating at the top of the viewport from the same separator at its
/// own place in the timeline
pub type RiseNativeSystemRowSelectDate = Rc<dyn Fn(i64, bool, &mut Window, &mut App)>;
/// gets the user id of the person whose name was clicked in a system pill
pub type RiseNativeSystemRowOpenPerson = Rc<dyn Fn(String, &mut Window, &mut App)>;
type RiseNativeSystemRowPersonClick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// configuration of the system row view, new carries the defaults
pub struct RiseNativeSystemRowViewConfiguration {
    pub theme: AppTheme,
    pub on_select_topic: Option<RiseNativeSystemRowSelectTopic>,
    pub on_select_date: Option<RiseNativeSystemRowSelectDate>,
    pub hold_actions: Vec<HoldContextMenuAction>,
    pub on_hold_action: Option<HoldContextMenuActionHandler>,
    pub person_links: Vec<MessageSystemPersonLink>,
    pub on_open_person: Option<RiseNativeSystemRowOpenPerson>,
    pub row_width: Option<f32>,
}

impl RiseNativeSystemRowViewConfiguration {
    pub fn new(theme: AppTheme) -> Self {
        Self {
            theme,
            on_select_topic: None,
            on_select_date: None,
            hold_actions: Vec::new(),
            on_hold_action: None,
            person_links: Vec::new(),
            on_open_person: None,
            row_width: None,
        }
    }

    pub fn person_links(mut self, person_links: Vec<MessageSystemPersonLink>) -> Self {
        self.person_links = person_links;
        self
    }

    pub fn on_open_person(mut self, on_open_person: Option<RiseNativeSystemRowOpenPerson>) -> Self {
        self.on_open_person = on_open_person;
        self
    }

    pub fn on_select_topic(mut self, on_select_topic: Option<RiseNativeSystemRowSelectTopic>) -> Self {
        self.on_select_topic = on_select_topic;
        self
    }

    pub fn on_select_date(mut self, on_select_date: Option<RiseNativeSystemRowSelectDate>) -> Self {
        self.on_select_date = on_select_date;
        self
    }

    pub fn hold_actions(mut self, hold_actions: Vec<HoldContextMenuAction>) -> Self {
        self.hold_actions = hold_actions;
        self
    }

    pub fn on_hold_action(mut self, on_hold_action: Option<HoldContextMenuActionHandler>) -> Self {
        self.on_hold_action = on_hold_action;
        self
    }

    pub fn row_width(mut self, row_width: f32) -> Self {
        self.row_width = Some(row_width);
        self
    }
}

struct Insets {
    top: f32,
    bottom: f32,
}

/// what the hold menu lifts, it fills whatever slot draws it (the row or the overlay host)
struct RiseNativeSystemRowPillControl {
    label: SharedString,
    is_topic: bool,
    line_limit: usize,
    text_size: Size<f32>,
    pill_width: f32,
    corner_radius: f32,
    text_color: Hsla,
    fill_color: Hsla,
    border_color: Hsla,
    person_ranges: Vec<Range<usize>>,
    person_color: Hsla,
    on_person_click: Option<RiseNativeSystemRowPersonClick>,
}

impl RiseNativeSystemRowPillControl {
    fn new() -> Self {
        Self {
            label: SharedString::default(),
            is_topic: false,
            line_limit: MessageSystemPill::LINE_LIMIT,
            text_size: Size::default(),
            pill_width: 0.0,
            corner_radius: 0.0,
            text_color: Hsla::default(),
            fill_color: Hsla::default(),
            border_color: Hsla::default(),
            person_ranges: Vec::new(),
            person_color: Hsla::default(),
            on_person_click: None,
        }
    }

    fn label_element(&self, left: f32, width: f32) -> Div {
        let label = div()
            .absolute()
            .left(px(left))
            .top(px(RiseNativeSystemRowView::PILL_VERTICAL_PADDING))
            .w(px(width))
            .h(px(self.text_size.height))
            .swiftui_font(RiseNativeSystemRowView::text_font())
            .text_color(self.text_color)
            .text_center()
            .text_ellipsis()
            .line_clamp(self.line_limit);
        let Some(on_person_click) = self.on_person_click.clone().filter(|_| !self.person_ranges.is_empty()) else {
            return label.child(self.label.clone());
        };
        let person_color = self.person_color;
        let highlights = self
            .person_ranges
            .iter()
            .map(|range| (range.clone(), HighlightStyle { color: Some(person_color), ..HighlightStyle::default() }));
        label.child(
            InteractiveText::new(
                "rise-native-system-row-person-text",
                StyledText::new(self.label.clone()).with_highlights(highlights),
            )
            .on_click(self.person_ranges.clone(), move |index, window, cx| on_person_click(index, window, cx)),
        )
    }

    fn topic_glyph(&self, sf_symbol: &str, left: f32, width: f32, side: f32, opacity: f32, is_filled: bool) -> Div {
        div()
            .absolute()
            .left(px(left))
            .top(px(RiseNativeSystemRowView::PILL_VERTICAL_PADDING))
            .w(px(width))
            .h(px(self.text_size.height))
            .flex()
            .items_center()
            .justify_center()
            .opacity(opacity)
            .children(IconUi::sized(sf_symbol, px(side), self.text_color, is_filled))
    }
}

impl Render for RiseNativeSystemRowPillControl {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let surface = div()
            .relative()
            .size_full()
            .rounded(px(self.corner_radius))
            .bg(self.fill_color)
            .border(px(LiquidChrome::nav_glass_border_width()))
            .border_color(self.border_color);
        if self.is_topic {
            let icon_left = RiseNativeSystemRowView::PILL_HORIZONTAL_PADDING;
            let label_left =
                icon_left + RiseNativeSystemRowView::TOPIC_ICON_WIDTH + RiseNativeSystemRowView::TOPIC_SPACING;
            let chevron_left = label_left + self.text_size.width + RiseNativeSystemRowView::TOPIC_SPACING;
            surface
                .child(self.topic_glyph(
                    "bubble.left.fill",
                    icon_left,
                    RiseNativeSystemRowView::TOPIC_ICON_WIDTH,
                    RiseNativeSystemRowView::TOPIC_ICON_FONT_SIZE,
                    1.0,
                    true,
                ))
                .child(self.label_element(label_left, self.text_size.width))
                .child(self.topic_glyph(
                    "chevron.right",
                    chevron_left,
                    RiseNativeSystemRowView::TOPIC_CHEVRON_WIDTH,
                    RiseNativeSystemRowView::TOPIC_CHEVRON_FONT_SIZE,
                    0.55,
                    false,
                ))
        } else {
            let width = (self.pill_width - RiseNativeSystemRowView::PILL_HORIZONTAL_PADDING * 2.0).max(0.0);
            surface.child(self.label_element(RiseNativeSystemRowView::PILL_HORIZONTAL_PADDING, width))
        }
    }
}

pub struct RiseNativeSystemRowView {
    pill_control: Entity<RiseNativeSystemRowPillControl>,
    pill_probe: ViewAnchorProbe,
    kind: Option<ChatMessageRenderEntryKind>,
    entry_id: String,
    label: String,
    topic_id: Option<String>,
    day_start_ms: Option<i64>,
    on_select_topic: Option<RiseNativeSystemRowSelectTopic>,
    on_select_date: Option<RiseNativeSystemRowSelectDate>,
    hold_actions: Vec<HoldContextMenuAction>,
    on_hold_action: Option<HoldContextMenuActionHandler>,
    person_links: Vec<MessageSystemPersonLink>,
    on_open_person: Option<RiseNativeSystemRowOpenPerson>,
    person_color: Hsla,
    hold_began_in_touch: bool,
    is_pill_holdable: bool,
    is_pill_button: bool,
    text_color: Hsla,
    fill_color: Hsla,
    border_color: Hsla,
    wallpaper_preview: Option<MessageSystemWallpaperPreview>,
    preview_image: Option<CleverImage>,
    preview_load_token: u64,
    preview_load_task: Option<Task<()>>,
    shows_gift_card: bool,
    gift_item: Option<MessageSystemGiftItem>,
    active_hold_request_id: Option<HoldContextMenuRequestID>,
    selection_lane_width: f32,
    is_sticky_chip_faded: Option<bool>,
    pill_alpha: Animated<f32>,
    configured_width: Option<f32>,
    bounds_width: Option<f32>,
    pill_frame: Bounds<f32>,
    drawn: Option<RiseNativeSystemRowDrawn>,
    text_size_memo: Option<(String, ChatMessageRenderEntryKind, f32, f32, Size<f32>)>,
}

/// configured data the render reads
/// callbacks are left out, clicks reach them through the row at click time so a fresh closure
/// alone redraws nothing
#[derive(Clone, PartialEq)]
struct RiseNativeSystemRowDrawn {
    kind: Option<ChatMessageRenderEntryKind>,
    entry_id: String,
    label: String,
    topic_id: Option<String>,
    day_start_ms: Option<i64>,
    is_pill_holdable: bool,
    is_pill_button: bool,
    person_links: Vec<MessageSystemPersonLink>,
    opens_person: bool,
    selects_topic: bool,
    selects_date: bool,
    configured_width: Option<f32>,
    text_color: Hsla,
    person_color: Hsla,
    fill_color: Hsla,
    border_color: Hsla,
    wallpaper_preview: Option<MessageSystemWallpaperPreview>,
    shows_gift_card: bool,
    gift_item: Option<MessageSystemGiftItem>,
}

impl RiseNativeSystemRowView {
    pub const REUSE_IDENTIFIER: &'static str = "RiseNativeSystemRowView";

    pub const WALLPAPER_PREVIEW_SIZE: Size<f32> = Size { width: 112.0, height: 150.0 };
    const WALLPAPER_PREVIEW_SPACING: f32 = 8.0;
    const WALLPAPER_PREVIEW_RADIUS: f32 = 14.0;

    const SIDE_INSET: f32 = 10.0;
    const PILL_HORIZONTAL_PADDING: f32 = 12.0;
    const PILL_VERTICAL_PADDING: f32 = 5.0;
    const TOPIC_SPACING: f32 = 5.0;
    const TOPIC_ICON_WIDTH: f32 = 12.0;
    const TOPIC_CHEVRON_WIDTH: f32 = 8.0;
    const TOPIC_ICON_FONT_SIZE: f32 = 10.0;
    const TOPIC_CHEVRON_FONT_SIZE: f32 = 9.0;

    pub fn wallpaper_preview(entry: &ChatMessageRenderEntry) -> Option<&MessageSystemWallpaperPreview> {
        if entry.kind != ChatMessageRenderEntryKind::System {
            return None;
        }
        entry.message.as_ref()?.system.as_ref()?.wallpaper_preview.as_ref()
    }

    pub fn shows_gift_card(entry: &ChatMessageRenderEntry) -> bool {
        entry.kind == ChatMessageRenderEntryKind::System
            && entry
                .message
                .as_ref()
                .and_then(|message| message.system.as_ref())
                .is_some_and(|system| matches!(system.kind, MessageSystemKind::PremiumGifted))
    }

    pub fn gift_item(entry: &ChatMessageRenderEntry) -> Option<&MessageSystemGiftItem> {
        if entry.kind != ChatMessageRenderEntryKind::System {
            return None;
        }
        let system = entry.message.as_ref()?.system.as_ref()?;
        if !matches!(system.kind, MessageSystemKind::GiftSent) {
            return None;
        }
        system.gift_item.as_ref()
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_selection_presentable::<Self>(cx);
        RiseManualListConformances::register_top_sticky_header_interactive::<Self>(cx);
        RiseManualListConformances::register_top_sticky_header_fading::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        Self {
            pill_control: cx.new(|_| RiseNativeSystemRowPillControl::new()),
            pill_probe: ViewAnchorProbe::new(),
            kind: None,
            entry_id: String::new(),
            label: String::new(),
            topic_id: None,
            day_start_ms: None,
            on_select_topic: None,
            on_select_date: None,
            hold_actions: Vec::new(),
            on_hold_action: None,
            person_links: Vec::new(),
            on_open_person: None,
            person_color: Hsla::default(),
            hold_began_in_touch: false,
            is_pill_holdable: false,
            is_pill_button: false,
            text_color: Hsla::default(),
            fill_color: Hsla::default(),
            border_color: Hsla::default(),
            wallpaper_preview: None,
            preview_image: None,
            preview_load_token: 0,
            preview_load_task: None,
            shows_gift_card: false,
            gift_item: None,
            active_hold_request_id: None,
            selection_lane_width: 0.0,
            is_sticky_chip_faded: None,
            pill_alpha: Animated::new(1.0),
            configured_width: None,
            bounds_width: None,
            pill_frame: Bounds::default(),
            drawn: None,
            text_size_memo: None,
        }
    }

    pub fn measure(entry: &ChatMessageRenderEntry, width: f32, window: &Window) -> f32 {
        if entry.kind == ChatMessageRenderEntryKind::Message {
            return 0.0;
        }
        let insets = Self::row_insets(entry.kind);
        let text_height = Self::measured_text_size(&entry.label, entry.kind, width, window).height;
        let preview_height = if Self::wallpaper_preview(entry).is_none() {
            0.0
        } else {
            Self::WALLPAPER_PREVIEW_SPACING + Self::WALLPAPER_PREVIEW_SIZE.height
        };
        let gift_height = if Self::shows_gift_card(entry) {
            Self::WALLPAPER_PREVIEW_SPACING + MessageSystemGiftCard::SIZE.height
        } else {
            0.0
        };
        let gift_item_height = Self::gift_item(entry).map_or(0.0, |item| {
            Self::WALLPAPER_PREVIEW_SPACING + GiftMessageCard::size(item, window).height
        });
        (insets.top
            + text_height
            + Self::PILL_VERTICAL_PADDING * 2.0
            + preview_height
            + gift_height
            + gift_item_height
            + insets.bottom)
            .ceil()
    }

    pub fn configure(
        &mut self,
        entry: &ChatMessageRenderEntry,
        configuration: RiseNativeSystemRowViewConfiguration,
        cx: &mut Context<Self>,
    ) {
        assert!(
            entry.kind != ChatMessageRenderEntryKind::Message,
            "Native system rows cannot render message entries"
        );

        self.kind = Some(entry.kind);
        self.entry_id = entry.id.clone();
        self.topic_id = entry.topic_id.clone();
        self.day_start_ms = entry.day_start_ms;
        let is_date = entry.kind == ChatMessageRenderEntryKind::DateSeparator
            && entry.day_start_ms.is_some()
            && configuration.on_select_date.is_some();
        self.on_select_topic = configuration.on_select_topic;
        self.on_select_date = configuration.on_select_date;
        self.hold_actions = if entry.kind == ChatMessageRenderEntryKind::System {
            configuration.hold_actions
        } else {
            Vec::new()
        };
        self.on_hold_action = configuration.on_hold_action;
        self.person_links = if entry.kind == ChatMessageRenderEntryKind::System && configuration.on_open_person.is_some() {
            configuration.person_links
        } else {
            Vec::new()
        };
        self.on_open_person = configuration.on_open_person;
        self.label = entry.label.clone();
        self.configured_width = configuration.row_width;
        let wallpaper_preview = Self::wallpaper_preview(entry).cloned();
        if wallpaper_preview != self.wallpaper_preview {
            self.apply_wallpaper_preview(wallpaper_preview, cx);
        }
        self.shows_gift_card = Self::shows_gift_card(entry);
        self.gift_item = Self::gift_item(entry).cloned();

        let is_topic = entry.kind == ChatMessageRenderEntryKind::TopicSeparator;
        let is_holdable = !self.hold_actions.is_empty();
        self.is_pill_holdable = is_holdable;
        self.is_pill_button = is_topic || is_date;

        let theme = &configuration.theme;
        self.text_color = alpha(theme.text.primary, 0.95);
        self.person_color = theme.primary._100;
        let is_dark = Self::is_dark(theme.bg._100);
        self.fill_color = LiquidChrome::surface_fill(theme, 0);
        self.border_color = if is_dark { alpha(white(), 0.14) } else { alpha(black(), 0.10) };
        let drawn = self.drawn_state();
        if self.drawn.as_ref() != Some(&drawn) {
            self.drawn = Some(drawn);
            cx.notify();
        }
    }

    fn drawn_state(&self) -> RiseNativeSystemRowDrawn {
        RiseNativeSystemRowDrawn {
            kind: self.kind,
            entry_id: self.entry_id.clone(),
            label: self.label.clone(),
            topic_id: self.topic_id.clone(),
            day_start_ms: self.day_start_ms,
            is_pill_holdable: self.is_pill_holdable,
            is_pill_button: self.is_pill_button,
            person_links: self.person_links.clone(),
            opens_person: self.on_open_person.is_some(),
            selects_topic: self.on_select_topic.is_some(),
            selects_date: self.on_select_date.is_some(),
            configured_width: self.configured_width,
            text_color: self.text_color,
            person_color: self.person_color,
            fill_color: self.fill_color,
            border_color: self.border_color,
            wallpaper_preview: self.wallpaper_preview.clone(),
            shows_gift_card: self.shows_gift_card,
            gift_item: self.gift_item.clone(),
        }
    }

    fn memoized_text_size(&mut self, kind: ChatMessageRenderEntryKind, width: f32, window: &Window) -> Size<f32> {
        let font_size = MessageSizes::system_pill_font_size();
        if let Some((label, memo_kind, memo_width, memo_font_size, size)) = self.text_size_memo.as_ref()
            && *memo_kind == kind
            && *memo_width == width
            && *memo_font_size == font_size
            && *label == self.label
        {
            return *size;
        }
        let size = Self::measured_text_size(&self.label, kind, width, window);
        self.text_size_memo = Some((self.label.clone(), kind, width, font_size, size));
        size
    }

    fn pill_selection(&self) -> Option<Rc<dyn Fn(&mut Window, &mut App)>> {
        match self.kind {
            Some(ChatMessageRenderEntryKind::TopicSeparator) => {
                let on_select_topic = self.on_select_topic.clone()?;
                let topic_id = self.topic_id.clone();
                Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                    on_select_topic(topic_id.clone(), window, cx);
                }))
            }
            Some(ChatMessageRenderEntryKind::DateSeparator) => {
                let day_start_ms = self.day_start_ms?;
                let on_select_date = self.on_select_date.clone()?;
                Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                    on_select_date(day_start_ms, false, window, cx);
                }))
            }
            _ => None,
        }
    }

    fn person_name_ranges(text: &str, links: &[MessageSystemPersonLink]) -> Vec<(Range<usize>, String)> {
        let mut result: Vec<(Range<usize>, String)> = Vec::new();
        for link in links.iter().filter(|link| !link.name.is_empty()) {
            for (start, found) in text.match_indices(link.name.as_str()) {
                let range = start..start + found.len();
                if result.iter().any(|(taken, _)| taken.start < range.end && range.start < taken.end) {
                    continue;
                }
                result.push((range, link.user_id.clone()));
            }
        }
        result
    }

    fn person_click(&self, cx: &mut Context<Self>) -> Option<(Vec<Range<usize>>, RiseNativeSystemRowPersonClick)> {
        self.on_open_person.as_ref()?;
        let ranges = Self::person_name_ranges(&self.label, &self.person_links);
        if ranges.is_empty() {
            return None;
        }
        let row = cx.weak_entity();
        let user_ids: Vec<String> = ranges.iter().map(|(_, user_id)| user_id.clone()).collect();
        let click: RiseNativeSystemRowPersonClick = Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
            let Some((held, on_open_person)) =
                row.upgrade().map(|row| (row.read(cx).hold_began_in_touch, row.read(cx).on_open_person.clone()))
            else {
                return;
            };
            if held || HoldContextMenuPresenter::has_active_request(cx) {
                return;
            }
            if let (Some(user_id), Some(on_open_person)) = (user_ids.get(index), on_open_person) {
                on_open_person(user_id.clone(), window, cx);
            }
        });
        Some((ranges.into_iter().map(|(range, _)| range).collect(), click))
    }

    /// same baked bitmap the chat backdrop draws so the card shows exactly the pinned look, dim is
    /// an overlay
    fn apply_wallpaper_preview(&mut self, preview: Option<MessageSystemWallpaperPreview>, cx: &mut Context<Self>) {
        self.preview_load_token = self.preview_load_token.wrapping_add(1);
        self.preview_load_task = None;
        self.preview_image = None;
        let source = preview.as_ref().map(|preview| (preview.image_url.clone(), preview.look.blur_percent));
        self.wallpaper_preview = preview;
        let Some((source, blur_percent)) = source else {
            return;
        };
        let token = self.preview_load_token;
        self.preview_load_task = Some(cx.spawn(async move |this, cx| {
            let image = ChatWallpaperBitmapCache::image(&source, blur_percent, cx).await;
            let _ = this.update(cx, |this, cx| {
                if this.preview_load_token != token {
                    return;
                }
                this.preview_image = image;
                cx.notify();
            });
        }));
    }

    fn handle_hold_began(
        row: &WeakEntity<Self>,
        plate: &ViewAnchorProbe,
        location: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let _ = row.update(cx, |row, _| row.hold_began_in_touch = true);
        if HoldContextMenuPresenter::has_active_request(cx) {
            return;
        }
        let Some(source_frame) = plate.window_frame() else {
            return;
        };
        if f32::from(source_frame.size.width) <= 1.0 || f32::from(source_frame.size.height) <= 1.0 {
            return;
        }
        let request_id = HoldContextMenuRequestID::new();
        let parts = row
            .update(cx, |row, _| {
                if row.hold_actions.is_empty() {
                    return None;
                }
                row.active_hold_request_id = Some(request_id);
                Some((
                    row.hold_actions.clone(),
                    row.on_hold_action.clone(),
                    SharedString::from(row.label.clone()),
                    ViewScreenSource::new(row.pill_control.clone(), row.pill_probe.clone()),
                ))
            })
            .ok()
            .flatten();
        let Some((actions, on_action, text, source)) = parts else {
            return;
        };
        let restored_row = row.clone();
        HoldContextMenuPresenter::present(
            HoldContextMenuRequest {
                id: request_id,
                source_frame,
                initial_source_center: source_frame.center(),
                alignment: HoldContextMenuAlignment::Leading,
                show_emoji_list: false,
                emojis: Vec::new(),
                allows_custom_emoji: false,
                expanded_emojis: None,
                actions,
                accessory: None,
                on_emoji: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
                on_action: Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
                    if let Some(on_action) = on_action.as_ref() {
                        on_action(action, window, cx);
                    }
                }),
                on_lift_source: Rc::new(|_: &mut App| {}),
                on_restore_source: Rc::new(move |cx: &mut App| {
                    let _ = restored_row.update(cx, |row, cx| {
                        if row.active_hold_request_id != Some(request_id) {
                            return;
                        }
                        row.active_hold_request_id = None;
                        cx.notify();
                    });
                }),
                source_lift: None,
                snapshot: None,
                source_bubble_clip: None,
                initial_lift_scale: 1.0,
                uses_flat_chrome: false,
                content: Rc::new(move |_: &mut Window, _: &mut App| {
                    Self::lifted_content(text.clone(), source_frame.size)
                }),
                is_centered: false,
                live_actions: None,
                on_dismiss: Rc::new(|_: &mut App| {}),
                controller_preview: None,
                prefers_dark_chrome: false,
                keeps_source_editing: false,
                max_visible_rows: None,
            },
            Some(&source),
            Some(location),
            window,
            cx,
        );
    }

    fn lifted_content(text: SharedString, frame: Size<Pixels>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(frame.width)
            .h(frame.height)
            .child(
                div()
                    .swiftui_font(Self::text_font())
                    .text_color(alpha(white(), 0.95))
                    .text_center()
                    .line_clamp(MessageSystemPill::LINE_LIMIT)
                    .child(text)
                    .liquid_glass_nav_pill(12.0, Some(1.0)),
            )
            .into_any_element()
    }

    fn hold_plate(&self, cx: &mut Context<Self>) -> HoldGesturePlate {
        let held = cx.weak_entity();
        let tapped = cx.weak_entity();
        HoldGesturePlate::new("rise-native-system-row-plate")
            .on_confirm(Rc::new(
                move |plate: &ViewAnchorProbe, location: Point<Pixels>, window: &mut Window, cx: &mut App| {
                    Self::handle_hold_began(&held, plate, location, window, cx);
                },
            ))
            .on_update(Rc::new(|location: Point<Pixels>, _: &mut Window, cx: &mut App| {
                if HoldContextMenuPresenter::has_active_request(cx) {
                    HoldContextMenuPresenter::update_drag(location, cx);
                }
            }))
            .on_release(Rc::new(|location: Option<Point<Pixels>>, _: &mut Window, cx: &mut App| {
                if HoldContextMenuPresenter::has_active_request(cx) {
                    HoldContextMenuPresenter::release_at(location, cx);
                }
            }))
            .on_cancel(Rc::new(|_: &mut Window, cx: &mut App| {
                if HoldContextMenuPresenter::has_active_request(cx) {
                    HoldContextMenuPresenter::release_at(None, cx);
                }
            }))
            .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
                let selection = tapped.read_with(cx, |row, _| row.pill_selection()).ok().flatten();
                if let Some(selection) = selection {
                    selection(window, cx);
                }
            }))
    }

    fn selection_transforms(&self, width: f32, pill_width: f32) -> (f32, f32) {
        if self.selection_lane_width <= 0.0 {
            return (0.0, 1.0);
        }
        let leading = MessageSizes::row_side_inset() + self.selection_lane_width;
        let trailing = width - Self::SIDE_INSET;
        let shift = (leading + trailing - width) * 0.5;
        let scale = if pill_width > 1.0 { ((trailing - leading).max(1.0) / pill_width).min(1.0) } else { 1.0 };
        (shift, scale)
    }

    fn topic_fixed_content_width() -> f32 {
        Self::TOPIC_ICON_WIDTH + Self::TOPIC_SPACING + Self::TOPIC_SPACING + Self::TOPIC_CHEVRON_WIDTH
    }

    /// height of a one line pill row of this kind, before any text is laid out
    pub fn single_line_row_height(kind: ChatMessageRenderEntryKind) -> f32 {
        if kind == ChatMessageRenderEntryKind::Message {
            return 0.0;
        }
        let insets = Self::row_insets(kind);
        (insets.top + Self::text_line_height() + Self::PILL_VERTICAL_PADDING * 2.0 + insets.bottom).ceil()
    }

    fn row_insets(kind: ChatMessageRenderEntryKind) -> Insets {
        match kind {
            ChatMessageRenderEntryKind::DateSeparator
            | ChatMessageRenderEntryKind::TopicSeparator
            | ChatMessageRenderEntryKind::UnreadSeparator => Insets { top: 10.0, bottom: 6.0 },
            ChatMessageRenderEntryKind::System => Insets { top: 6.0, bottom: 6.0 },
            ChatMessageRenderEntryKind::Message => Insets { top: 0.0, bottom: 0.0 },
        }
    }

    fn text_font() -> Font {
        Font::system_weight(MessageSizes::system_pill_font_size(), FontWeight::SEMIBOLD)
    }

    fn text_line_height() -> f32 {
        MessageSizes::system_pill_font_size() * NATURAL_LINE_HEIGHT
    }

    fn measured_text_size(text: &str, kind: ChatMessageRenderEntryKind, width: f32, window: &Window) -> Size<f32> {
        let line_height = Self::text_line_height();
        let maximum_pill_width = (width - Self::SIDE_INSET * 2.0).max(0.0);
        let topic_chrome_width =
            if kind == ChatMessageRenderEntryKind::TopicSeparator { Self::topic_fixed_content_width() } else { 0.0 };
        let maximum_text_width =
            (maximum_pill_width - Self::PILL_HORIZONTAL_PADDING * 2.0 - topic_chrome_width).max(0.0);
        if maximum_text_width <= 0.0 {
            return size(0.0, line_height);
        }

        let measured_text = if text.is_empty() { " " } else { text };
        let font = Self::text_font();
        let run = TextRun {
            len: measured_text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: font.style(),
            },
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let maximum_height = (line_height * Self::line_limit(kind) as f32).ceil();
        let Ok(lines) = window.text_system().shape_text(
            SharedString::from(measured_text.to_owned()),
            px(font.size),
            &[run],
            Some(px(maximum_text_width)),
            None,
        ) else {
            return size(maximum_text_width, line_height.ceil());
        };

        let mut measured_width = 0.0_f32;
        for line in lines.iter() {
            let layout = &line.unwrapped_layout;
            let mut row_start = 0.0_f32;
            for boundary in line.wrap_boundaries.iter() {
                let Some(glyph) = layout.runs.get(boundary.run_ix).and_then(|run| run.glyphs.get(boundary.glyph_ix))
                else {
                    continue;
                };
                let row_end = f32::from(glyph.position.x);
                measured_width = measured_width.max(row_end - row_start);
                row_start = row_end;
            }
            measured_width = measured_width.max(f32::from(layout.width) - row_start);
        }
        let measured_height = visual_line_count(&lines) as f32 * line_height;
        size(maximum_text_width.min(measured_width.ceil()), measured_height.ceil().min(maximum_height))
    }

    fn line_limit(kind: ChatMessageRenderEntryKind) -> usize {
        if kind == ChatMessageRenderEntryKind::TopicSeparator {
            MessageTopicDivider::LINE_LIMIT
        } else {
            MessageSystemPill::LINE_LIMIT
        }
    }

    fn maximum_corner_radius() -> f32 {
        ((Self::text_line_height() * 2.0).ceil() + Self::PILL_VERTICAL_PADDING * 2.0) * 0.5
    }

    fn is_dark(color: Hsla) -> bool {
        let rgb = color.to_rgb();
        rgb.r * 0.2126 + rgb.g * 0.7152 + rgb.b * 0.0722 < 0.55
    }

    fn wallpaper_preview_element(&self, left: f32, top: f32, dim: f32) -> Div {
        let side = Self::WALLPAPER_PREVIEW_SIZE;
        let radius = px(Self::WALLPAPER_PREVIEW_RADIUS);
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(side.width))
            .h(px(side.height))
            .rounded(radius)
            .bg(black())
            .children(self.preview_image.clone().map(|image| {
                img(image).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).rounded(radius)
            }))
            .child(div().absolute().inset_0().rounded(radius).bg(black()).opacity(dim))
            .child(div().absolute().inset_0().rounded(radius).border_1().border_color(alpha(white(), 0.18)))
    }
}

impl Render for RiseNativeSystemRowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let measured = cx.weak_entity();
        let bounds_reader = measure("rise-native-system-row-bounds", move |bounds, _, cx| {
            let width = f32::from(bounds.size.width);
            let _ = measured.update(cx, |row, cx| {
                if row.bounds_width != Some(width) {
                    let drawn_width = row.bounds_width.or(row.configured_width);
                    row.bounds_width = Some(width);
                    if drawn_width != Some(width) {
                        cx.notify();
                    }
                }
            });
        });
        let root = div().relative().size_full().child(bounds_reader);
        let (Some(kind), Some(width)) = (self.kind, self.bounds_width.or(self.configured_width)) else {
            self.pill_frame = Bounds::default();
            return root;
        };

        let insets = Self::row_insets(kind);
        let text_size = self.memoized_text_size(kind, width, window);
        let is_topic = kind == ChatMessageRenderEntryKind::TopicSeparator;
        let topic_chrome_width = if is_topic { Self::topic_fixed_content_width() } else { 0.0 };
        let content_width = text_size.width + topic_chrome_width;
        let maximum_pill_width = (width - Self::SIDE_INSET * 2.0).max(0.0);
        let pill_width = maximum_pill_width.min((content_width + Self::PILL_HORIZONTAL_PADDING * 2.0).ceil());
        let pill_height = (text_size.height + Self::PILL_VERTICAL_PADDING * 2.0).ceil();
        let pill_left = ((width - pill_width) * 0.5).floor();
        let pill_top = insets.top;
        let (shift, pill_scale) = self.selection_transforms(width, pill_width);
        let scaled = size(pill_width * pill_scale, pill_height * pill_scale);
        self.pill_frame = Bounds::new(
            point(
                pill_left + shift + (pill_width - scaled.width) * 0.5,
                pill_top + (pill_height - scaled.height) * 0.5,
            ),
            scaled,
        );

        let label = SharedString::from(self.label.clone());
        let (text_color, fill_color, border_color) = (self.text_color, self.fill_color, self.border_color);
        let person_color = self.person_color;
        let (person_ranges, on_person_click) = match self.person_click(cx) {
            Some((ranges, click)) => (ranges, Some(click)),
            None => (Vec::new(), None),
        };
        self.pill_control.update(cx, |pill, _| {
            pill.person_ranges = person_ranges;
            pill.person_color = person_color;
            pill.on_person_click = on_person_click;
            pill.label = label.clone();
            pill.is_topic = is_topic;
            pill.line_limit = Self::line_limit(kind);
            pill.text_size = text_size;
            pill.pill_width = pill_width;
            pill.corner_radius = (pill_height * 0.5).min(Self::maximum_corner_radius());
            pill.text_color = text_color;
            pill.fill_color = fill_color;
            pill.border_color = border_color;
        });

        let pill_view: AnyView = self.pill_control.clone().into();
        let pill_content = HoldContextMenuSourceLift::lifted_placeholder(&pill_view)
            .unwrap_or_else(|| pill_view.into_any_element());
        let plate = self.is_pill_holdable.then(|| self.hold_plate(cx));
        let pill_click = (self.is_pill_button && !self.is_pill_holdable && self.pill_selection().is_some())
            .then(|| cx.weak_entity());
        let now = RiseClock::monotonic();
        let pill = div()
            .id("rise-native-system-row-pill")
            .absolute()
            .left(px(pill_left + shift))
            .top(px(pill_top))
            .w(px(pill_width))
            .h(px(pill_height))
            .aria_label(label)
            .opacity(self.pill_alpha.frame_value(now, window))
            .when(self.is_pill_button, |pill| pill.cursor_pointer())
            .when_some(pill_click, |pill, row| {
                pill.on_click(move |_, window, cx| {
                    let selection = row.read_with(cx, |row, _| row.pill_selection()).ok().flatten();
                    if let Some(selection) = selection {
                        selection(window, cx);
                    }
                })
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|row, _: &MouseDownEvent, _, _| {
                    row.hold_began_in_touch = false;
                }),
            )
            .child(pill_content)
            .view_anchor_probe(&self.pill_probe)
            .children(plate);
        let pill = if pill_scale < 1.0 { pill.scale_effect(pill_scale).into_any_element() } else { pill.into_any_element() };

        let attachment_top = pill_top + pill_height + Self::WALLPAPER_PREVIEW_SPACING;
        let centered_left = |attachment_width: f32| ((width - attachment_width) * 0.5).floor() + shift;
        let mut root = root.child(pill);
        if let Some(preview) = self.wallpaper_preview.as_ref() {
            let left = centered_left(Self::WALLPAPER_PREVIEW_SIZE.width);
            root = root.child(self.wallpaper_preview_element(left, attachment_top, preview.look.dim_percent as f32 / 100.0));
        }
        if self.shows_gift_card {
            let side = MessageSystemGiftCard::SIZE;
            root = root.child(
                div()
                    .absolute()
                    .left(px(centered_left(side.width)))
                    .top(px(attachment_top))
                    .w(px(side.width))
                    .h(px(side.height))
                    .child(MessageSystemGiftCard::new(SharedString::from(format!(
                        "rise-native-system-row-gift-{}",
                        self.entry_id
                    )))),
            );
        }
        if let Some(item) = self.gift_item.as_ref() {
            let side = GiftMessageCard::size(item, window);
            root = root.child(
                div()
                    .absolute()
                    .left(px(centered_left(side.width)))
                    .top(px(attachment_top))
                    .w(px(side.width))
                    .h(px(side.height))
                    .child(GiftMessageCard::new(
                        SharedString::from(format!("rise-native-system-row-gift-item-{}", self.entry_id)),
                        item.clone(),
                    )),
            );
        }
        root
    }
}

impl RiseManualListReusableView for RiseNativeSystemRowView {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        self.kind = None;
        self.entry_id.clear();
        self.label.clear();
        self.topic_id = None;
        self.day_start_ms = None;
        self.on_select_topic = None;
        self.on_select_date = None;
        self.hold_actions = Vec::new();
        self.on_hold_action = None;
        self.person_links = Vec::new();
        self.on_open_person = None;
        self.hold_began_in_touch = false;
        self.apply_wallpaper_preview(None, cx);
        self.shows_gift_card = false;
        self.gift_item = None;
        if let Some(active_hold_request_id) = self.active_hold_request_id.take() {
            cx.defer(move |cx| HoldContextMenuPresenter::dismiss_if_active(active_hold_request_id, cx));
        }
        self.selection_lane_width = 0.0;
        self.is_sticky_chip_faded = None;
        self.pill_alpha.jump(1.0);
        self.is_pill_holdable = false;
        self.is_pill_button = false;
        self.drawn = None;
        cx.notify();
    }
}

impl RiseManualListSelectionPresentable for RiseNativeSystemRowView {
    fn apply_manual_list_selection_presentation(
        &mut self,
        is_active: bool,
        content_translation_x: f32,
        cx: &mut Context<Self>,
    ) {
        let lane_width = if is_active { content_translation_x } else { 0.0 };
        if lane_width == self.selection_lane_width {
            return;
        }
        self.selection_lane_width = lane_width;
        cx.notify();
    }
}

impl RiseManualListTopStickyHeaderInteractive for RiseNativeSystemRowView {
    fn top_sticky_header_contains(&self, point: Point<Pixels>) -> bool {
        if self.kind != Some(ChatMessageRenderEntryKind::DateSeparator)
            || self.day_start_ms.is_none()
            || self.on_select_date.is_none()
        {
            return false;
        }
        let (x, y) = (f32::from(point.x), f32::from(point.y));
        let frame = self.pill_frame;
        x >= frame.origin.x - 8.0
            && x < frame.origin.x + frame.size.width + 8.0
            && y >= frame.origin.y - 6.0
            && y < frame.origin.y + frame.size.height + 6.0
    }

    fn activate_top_sticky_header(&mut self, is_pinned: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind != Some(ChatMessageRenderEntryKind::DateSeparator) {
            return;
        }
        let Some(day_start_ms) = self.day_start_ms else {
            return;
        };
        if let Some(on_select_date) = self.on_select_date.clone() {
            on_select_date(day_start_ms, is_pinned, window, cx);
        }
    }
}

impl RiseManualListTopStickyHeaderFading for RiseNativeSystemRowView {
    fn set_top_sticky_header_faded(&mut self, faded: bool, animated: bool, cx: &mut Context<Self>) {
        let previous = self.is_sticky_chip_faded;
        if previous == Some(faded) {
            return;
        }
        self.is_sticky_chip_faded = Some(faded);
        let target = if faded { 0.0 } else { 1.0 };
        if animated && previous.is_some() && self.pill_probe.window_frame().is_some() {
            let duration = if faded { 0.4 } else { 0.2 };
            self.pill_alpha.set_in(target, ease_in_out(duration), RiseClock::monotonic(), cx);
        } else {
            self.pill_alpha.jump(target);
        }
        cx.notify();
    }

    fn is_top_sticky_header_on_screen(&self) -> bool {
        self.pill_alpha.value(RiseClock::monotonic()) >= 0.3
    }
}

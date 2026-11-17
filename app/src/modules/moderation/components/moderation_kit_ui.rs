use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, Render, SharedString, StyleRefinement,
    Subscription, Svg, Task, Window, white,
};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::grouped_btns::{GroupedBtn, GroupedCardMetrics};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsHeader;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerLiveState};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AdminSanctionKind, AppealStatus, BlockTimeFormatting, ModerationDateFormatting, ReportReason, ReportStatus,
    ReportTargetType, ResolveAction, UserBlockType,
};

const FILLED_SYMBOL_SUFFIX: &str = ".fill";

pub fn moderation_child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(std::sync::Arc::new(id.clone()), name.into())
}

/// system image at a size and color
/// lucide has no filled variants so the fill intent is read off the sf name
pub fn moderation_symbol(symbol: &str, side: f32, color: Hsla) -> Option<Svg> {
    IconUi::sized(symbol, px(side), color, symbol.ends_with(FILLED_SYMBOL_SUFFIX))
}

/// moderation reuses the app settings palette
/// reasons are banded and not tinted one by one, the queue was unreadable with a hue each
/// reasons share nine colors by topic, object kind and action weight keep theirs
pub enum ModerationTint {}

impl ModerationTint {
    /// bands not a color per reason, same band never side by side
    /// red gravest harm then pink person harm then orange abuse then indigo identity
    /// purple privacy then teal truthfulness then blue noise then green money
    pub fn reason(reason: ReportReason) -> Hsla {
        match reason {
            ReportReason::ChildSafety | ReportReason::Terrorism | ReportReason::Violence => ChatTileColor::red(),
            ReportReason::ExplicitContent | ReportReason::Suicide => ChatTileColor::pink(),
            ReportReason::Harassment | ReportReason::Inadequacy | ReportReason::Copyright => ChatTileColor::orange(),
            ReportReason::Impersonation => ChatTileColor::indigo(),
            ReportReason::Discrimination | ReportReason::Political | ReportReason::PersonalData => {
                ChatTileColor::purple()
            }
            ReportReason::UntruthInformation | ReportReason::AiGenerated => ChatTileColor::teal(),
            ReportReason::Scam => ChatTileColor::green(),
            ReportReason::Spam => ChatTileColor::blue(),
            ReportReason::Other => ChatTileColor::gray(),
        }
    }

    pub fn target(target: ReportTargetType) -> Hsla {
        match target {
            ReportTargetType::Post => ChatTileColor::blue(),
            ReportTargetType::Comment => ChatTileColor::teal(),
            ReportTargetType::User => ChatTileColor::indigo(),
            ReportTargetType::Chat => ChatTileColor::green(),
            ReportTargetType::Message => ChatTileColor::blue(),
            ReportTargetType::MusicTrack | ReportTargetType::Artist => ChatTileColor::pink(),
            ReportTargetType::Story => ChatTileColor::orange(),
            ReportTargetType::Bot => ChatTileColor::purple(),
            ReportTargetType::Vacancy | ReportTargetType::Other => ChatTileColor::gray(),
        }
    }

    pub fn action(action: ResolveAction) -> Hsla {
        match action {
            ResolveAction::Dismiss => ChatTileColor::green(),
            ResolveAction::Warn => ChatTileColor::orange(),
            ResolveAction::Mute => ChatTileColor::orange(),
            ResolveAction::DeleteContent | ResolveAction::BlockUser | ResolveAction::Both => ChatTileColor::red(),
        }
    }

    pub fn sanction(kind: AdminSanctionKind) -> Hsla {
        if kind.is_destructive() { ChatTileColor::red() } else { ChatTileColor::green() }
    }

    pub fn status(status: ReportStatus, theme: &AppTheme) -> Hsla {
        match status {
            ReportStatus::Active => theme.primary._100,
            ReportStatus::Resolved => ChatTileColor::green(),
            ReportStatus::Appealed => ChatTileColor::orange(),
            ReportStatus::Rejected => theme.text.secondary,
        }
    }

    pub fn appeal(status: AppealStatus, theme: &AppTheme) -> Hsla {
        match status {
            AppealStatus::Pending => theme.primary._100,
            AppealStatus::Accepted => ChatTileColor::green(),
            AppealStatus::Rejected => theme.text.secondary,
        }
    }

    pub fn block(block_type: UserBlockType) -> Hsla {
        if block_type == UserBlockType::Full { ChatTileColor::red() } else { ChatTileColor::orange() }
    }
}

/// artwork of a moderation screen, the reported picture or a solid tile with a white glyph at hero size
#[derive(IntoElement)]
pub struct ModerationArtwork {
    id: ElementId,
    icon: SharedString,
    tint: Hsla,
    image_source: Option<SharedString>,
    side: f32,
}

impl ModerationArtwork {
    pub fn new(id: impl Into<ElementId>, icon: impl Into<SharedString>, tint: Hsla) -> Self {
        Self { id: id.into(), icon: icon.into(), tint, image_source: None, side: ModerationMetrics::HERO_SIDE }
    }

    pub fn image_source(mut self, image_source: impl Into<Option<SharedString>>) -> Self {
        self.image_source = image_source.into();
        self
    }

    pub fn side(mut self, side: f32) -> Self {
        self.side = side;
        self
    }
}

impl RenderOnce for ModerationArtwork {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let side = self.side;
        let radius = side * 0.28;
        let frame = Shape::continuous(radius).round(div().flex_none().overflow_hidden()).frame_size(side, side);
        match self.image_source.filter(|source| !source.is_empty()) {
            Some(source) => frame.child(
                CleverImageUi::new(source)
                    .id(self.id)
                    .content_mode(ContentMode::Fill)
                    .corner_radius(radius)
                    .persist_to_disk(false)
                    .maximum_decode_pixel_dimension(480_i64)
                    .cache_policy(CleverImageCachePolicy::MemoryOnly),
            ),
            None => frame
                .bg(self.tint)
                .flex()
                .items_center()
                .justify_center()
                .children(moderation_symbol(&self.icon, side * 0.42, white())),
        }
    }
}

/// title block of every moderation screen like the playlist and achievement screens
/// artwork plus one bold line, one quiet line and one badge
#[derive(IntoElement)]
pub struct ModerationHero {
    id: ElementId,
    title: SharedString,
    subtitle: Option<SharedString>,
    badge_text: Option<SharedString>,
    badge_color: Option<Hsla>,
    on_title_press: Option<ButtonAction>,
    artwork: AnyElement,
}

impl ModerationHero {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>, artwork: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            badge_text: None,
            badge_color: None,
            on_title_press: None,
            artwork: artwork.into_any_element(),
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<Option<SharedString>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn badge_text(mut self, badge_text: impl Into<Option<SharedString>>) -> Self {
        self.badge_text = badge_text.into();
        self
    }

    pub fn badge_color(mut self, badge_color: impl Into<Option<Hsla>>) -> Self {
        self.badge_color = badge_color.into();
        self
    }

    pub fn on_title_press(mut self, on_title_press: Option<ButtonAction>) -> Self {
        self.on_title_press = on_title_press;
        self
    }
}

impl RenderOnce for ModerationHero {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let title_block = vstack(5.0)
            .w_full()
            .child(
                MainText::new(self.title)
                    .px(22.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(2),
            )
            .children(self.subtitle.filter(|subtitle| !subtitle.is_empty()).map(|subtitle| {
                MainText::new(subtitle)
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(3)
            }));

        let title = match self.on_title_press {
            Some(on_title_press) => plain(moderation_child_id(&self.id, "title"))
                .w_full()
                .on_press(move |window, cx| on_title_press(window, cx))
                .child(title_block)
                .into_any_element(),
            None => title_block.into_any_element(),
        };
        let badge_color = self.badge_color;

        vstack(12.0)
            .w_full()
            .px(px(18.0))
            .pb(px(4.0))
            .child(self.artwork)
            .child(title)
            .children(self.badge_text.map(|text| ModerationBadge::new(text).color(badge_color)))
    }
}

/// one quiet capsule, neutral unless the state is worth a color
#[derive(IntoElement)]
pub struct ModerationBadge {
    text: SharedString,
    color: Option<Hsla>,
    icon: Option<SharedString>,
}

impl ModerationBadge {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), color: None, icon: None }
    }

    pub fn color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.color = color.into();
        self
    }

    pub fn icon(mut self, icon: impl Into<Option<SharedString>>) -> Self {
        self.icon = icon.into();
        self
    }
}

impl RenderOnce for ModerationBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let resolved = self.color.unwrap_or(theme.text.secondary);
        hstack(5.0)
            .flex_none()
            .px(px(10.0))
            .py(px(5.0))
            .bg(if self.color.is_none() { theme.bg._200 } else { resolved.opacity(0.14) })
            .rounded_full()
            .children(self.icon.and_then(|icon| moderation_symbol(&icon, 10.0, resolved)))
            .child(MainText::new(self.text).px(12.0).font_weight(FontWeight::SEMIBOLD).color(resolved))
    }
}

/// a fact inside a settings card, small grey label and plain value
/// no chevron, same as the chat profile and achievement sheet
pub enum ModerationFact {}

impl ModerationFact {
    pub fn row(
        id: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        tint: Hsla,
        label: impl Into<SharedString>,
        value: impl Into<SharedString>,
        multiline: bool,
        on_tap: Option<ButtonAction>,
    ) -> GroupedBtn {
        let row = GroupedBtn::new(id, value)
            .icon(icon)
            .icon_background(tint)
            .pretitle(label)
            .text_line_limit(if multiline { 0 } else { 2 })
            .show_chevron(on_tap.is_some());
        match on_tap {
            Some(on_tap) => row.callback(move |window, cx| on_tap(window, cx)),
            None => row,
        }
    }
}

/// one row of any moderation list, two lines like other people lists
/// who and one line about them
#[derive(IntoElement)]
pub struct ModerationPersonRow {
    id: ElementId,
    avatar: Option<AnyElement>,
    logo_source: Option<SharedString>,
    name: SharedString,
    user_id: Option<SharedString>,
    subtitle: Option<SharedString>,
    subtitle_color: Option<Hsla>,
    is_official: bool,
    is_premium: bool,
    trailing_text: Option<SharedString>,
    trailing_age_ms: Option<i64>,
    trailing: Option<AnyElement>,
}

impl ModerationPersonRow {
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            avatar: None,
            logo_source: None,
            name: name.into(),
            user_id: None,
            subtitle: None,
            subtitle_color: None,
            is_official: false,
            is_premium: false,
            trailing_text: None,
            trailing_age_ms: None,
            trailing: None,
        }
    }

    pub fn avatar(mut self, avatar: impl IntoElement) -> Self {
        self.avatar = Some(avatar.into_any_element());
        self
    }

    pub fn logo_source(mut self, logo_source: impl Into<Option<SharedString>>) -> Self {
        self.logo_source = logo_source.into();
        self
    }

    pub fn user_id(mut self, user_id: impl Into<Option<SharedString>>) -> Self {
        self.user_id = user_id.into();
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<Option<SharedString>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn subtitle_color(mut self, subtitle_color: impl Into<Option<Hsla>>) -> Self {
        self.subtitle_color = subtitle_color.into();
        self
    }

    pub fn is_official(mut self, is_official: bool) -> Self {
        self.is_official = is_official;
        self
    }

    pub fn is_premium(mut self, is_premium: bool) -> Self {
        self.is_premium = is_premium;
        self
    }

    pub fn trailing_text(mut self, trailing_text: impl Into<Option<SharedString>>) -> Self {
        self.trailing_text = trailing_text.into();
        self
    }

    pub fn trailing_age_ms(mut self, trailing_age_ms: impl Into<Option<i64>>) -> Self {
        self.trailing_age_ms = trailing_age_ms.into();
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }
}

impl RenderOnce for ModerationPersonRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let avatar = self.avatar.unwrap_or_else(|| {
            UserLogoUi::new(moderation_child_id(&self.id, "logo"))
                .source(self.logo_source)
                .name(Some(self.name.clone()))
                .user_id(self.user_id)
                .size(ModerationMetrics::AVATAR_SIZE)
                .into_any_element()
        });

        let name_line = hstack(5.0)
            .min_w(px(0.0))
            .child(MainText::new(self.name).px(16.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
            .children(self.is_official.then(|| OfficialBadge::new().size(px(14.0))))
            .children(self.is_premium.then(|| PremiumDiamond::new().size(14.0)));
        let subtitle = self.subtitle.filter(|subtitle| !subtitle.is_empty()).map(|subtitle| {
            MainText::new(subtitle)
                .px(13.0)
                .color(self.subtitle_color.unwrap_or(theme.text.secondary))
                .number_of_lines(1)
        });

        let trailing_label = match self.trailing_age_ms.filter(|ms| *ms > 0) {
            Some(ms) => Some(
                ModerationLiveAge::new(moderation_child_id(&self.id, "age"), ms, theme.text.secondary)
                    .into_any_element(),
            ),
            None => self.trailing_text.map(|text| {
                MainText::new(text).px(12.0).color(theme.text.secondary).number_of_lines(1).into_any_element()
            }),
        };

        hstack(12.0)
            .w_full()
            .px(px(ModerationMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(10.0))
            .child(div().flex_none().child(avatar))
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w(px(0.0))
                    .child(name_line)
                    .children(subtitle),
            )
            .children(trailing_label)
            .children(self.trailing)
    }
}

#[derive(IntoElement)]
pub struct ModerationLiveAge {
    id: ElementId,
    ms: i64,
    color: Hsla,
}

struct ModerationLiveAgeTick;

impl ModerationLiveAge {
    pub fn new(id: impl Into<ElementId>, ms: i64, color: Hsla) -> Self {
        Self { id: id.into(), ms, color }
    }

    fn refresh_dates(since_ms: i64) -> Vec<Date> {
        let start = BlockTimeFormatting::date(since_ms);
        let now = Date::now();
        let mut dates = Vec::new();
        let mut next = start.adding_time_interval(60.0);
        while next <= start.adding_time_interval(3600.0) {
            if next > now {
                dates.push(next);
            }
            next = next.adding_time_interval(60.0);
        }
        next = start.adding_time_interval(7200.0);
        while next <= start.adding_time_interval(86_400.0) {
            if next > now {
                dates.push(next);
            }
            next = next.adding_time_interval(3600.0);
        }
        dates
    }
}

impl RenderOnce for ModerationLiveAge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ms = self.ms;
        let tick = window.use_keyed_state(moderation_child_id(&self.id, "tick"), cx, |_, _| ModerationLiveAgeTick);

        let timeline = tick.downgrade();
        use_task(moderation_child_id(&self.id, "timeline"), ms, window, cx, async move |cx| {
            for date in Self::refresh_dates(ms) {
                let delay = date.time_interval_since(&Date::now());
                if delay > 0.0 {
                    cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
                }
                let alive = cx.update(|_, cx| timeline.update(cx, |_, cx| cx.notify()).is_ok()).unwrap_or(false);
                if !alive {
                    return;
                }
            }
        });

        div().flex_none().whitespace_nowrap().child(
            MainText::new(ModerationDateFormatting::age(ms, Date::now())).px(12.0).color(self.color).number_of_lines(1),
        )
    }
}

pub enum ModerationMetrics {}

impl ModerationMetrics {
    pub const AVATAR_SIZE: f32 = 50.0;
    pub const ROW_HORIZONTAL_PADDING: f32 = 16.0;
    pub const LIST_SEPARATOR_INSET: f32 = 78.0;
    pub const SECTION_SPACING: f32 = 18.0;
    pub const BOTTOM_INSET: f32 = 90.0;
    pub const HERO_SIDE: f32 = 104.0;
}

#[derive(IntoElement)]
pub struct ModerationRowSeparator {
    inset: f32,
}

impl Default for ModerationRowSeparator {
    fn default() -> Self {
        Self { inset: ModerationMetrics::LIST_SEPARATOR_INSET }
    }
}

impl ModerationRowSeparator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }
}

impl RenderOnce for ModerationRowSeparator {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div().w_full().pl(px(self.inset)).child(div().w_full().h(px(0.5)).bg(rise_ui::theme(cx).border._100))
    }
}

pub type ModerationFilterSelect = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// status axis of a list, same bar the chats and feed use
#[derive(IntoElement)]
pub struct ModerationSegmentedFilter {
    id: ElementId,
    titles: Vec<SharedString>,
    selected_index: i64,
    live_drag_state: Option<Entity<AnimatedTabsPagerLiveState>>,
    on_select: ModerationFilterSelect,
}

impl ModerationSegmentedFilter {
    pub fn new(
        id: impl Into<ElementId>,
        titles: Vec<SharedString>,
        selected_index: i64,
        on_select: impl Fn(i64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), titles, selected_index, live_drag_state: None, on_select: Rc::new(on_select) }
    }

    pub fn live_drag_state(mut self, live_drag_state: Option<Entity<AnimatedTabsPagerLiveState>>) -> Self {
        self.live_drag_state = live_drag_state;
        self
    }
}

impl RenderOnce for ModerationSegmentedFilter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tabs = self
            .titles
            .into_iter()
            .enumerate()
            .map(|(offset, title)| TabHeaderConfig::new(offset as i64, title))
            .collect();
        let progress_state = self.live_drag_state.clone();
        let base_state = self.live_drag_state.clone();
        let tap_state = self.live_drag_state;
        let on_select = self.on_select;
        div().w_full().pt(px(2.0)).pb(px(8.0)).child(
            AnimatedTabsHeader::new(
                self.id,
                tabs,
                Binding::constant(self.selected_index),
                Rc::new(move |cx: &App| progress_state.as_ref().map_or(0.0, |state| state.read(cx).drag_progress())),
                12.0,
                on_select,
            )
            .drag_base_index(Rc::new(move |cx: &App| {
                base_state.as_ref().and_then(|state| state.read(cx).drag_base_index())
            }))
            .suppresses_tap(Rc::new(move |cx: &App| {
                tap_state.as_ref().is_some_and(|state| state.read(cx).suppresses_tab_taps())
            }))
            .force_scrollable(false),
        )
    }
}

struct ModerationFilterPagerState {
    page_width: f32,
}

/// swipeable body under the segmented filter
/// store holds one list for the selected filter so only that page is live
/// neighbours show the loading skeleton, ctrl+tab walks the filters like a swipe
#[derive(IntoElement)]
pub struct ModerationFilterPager {
    id: ElementId,
    count: i64,
    selected_index: i64,
    live_drag_state: Entity<AnimatedTabsPagerLiveState>,
    skeleton_rows: usize,
    on_select: ModerationFilterSelect,
    selected_page: AnyElement,
}

impl ModerationFilterPager {
    pub fn new(
        id: impl Into<ElementId>,
        count: i64,
        selected_index: i64,
        live_drag_state: Entity<AnimatedTabsPagerLiveState>,
        on_select: impl Fn(i64, &mut Window, &mut App) + 'static,
        selected_page: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            count,
            selected_index,
            live_drag_state,
            skeleton_rows: 8,
            on_select: Rc::new(on_select),
            selected_page: selected_page.into_any_element(),
        }
    }

    pub fn skeleton_rows(mut self, skeleton_rows: usize) -> Self {
        self.skeleton_rows = skeleton_rows;
        self
    }
}

impl RenderOnce for ModerationFilterPager {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let count = self.count;
        let selected_index = self.selected_index;
        let state = window.use_keyed_state(moderation_child_id(&id, "state"), cx, |_, _| {
            ModerationFilterPagerState { page_width: 0.0 }
        });
        let focus = window
            .use_keyed_state::<FocusHandle>(moderation_child_id(&id, "focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let page_focus = PageFocus::keyed_with(moderation_child_id(&id, "page-focus"), focus.clone(), window, cx);
        let leaving_focus = page_focus.clone();

        let binding_select = self.on_select.clone();
        let active_tab = Binding::new(
            move |_: &App| selected_index,
            move |index, window, cx| {
                if index != selected_index {
                    binding_select(index, window, cx);
                }
            },
        );
        let width_reader = state.clone();
        let page_width = Binding::new(
            move |cx: &App| width_reader.read(cx).page_width,
            move |width, _window, cx| {
                state.update(cx, |state, _| state.page_width = width);
            },
        );

        let mut selected_page = Some(self.selected_page);
        let skeleton_rows = self.skeleton_rows;
        let pages: Vec<AnyElement> = (0..count)
            .map(|index| {
                let page_id = moderation_child_id(&id, format!("page-{index}"));
                let content = if index == selected_index { selected_page.take() } else { None };
                let content = content.unwrap_or_else(|| {
                    let top_inset = list_shell_floating_top_inset(cx);
                    let view = window.use_keyed_state(moderation_child_id(&page_id, "skeleton-view"), cx, |_, cx| {
                        ModerationSkeletonPage::new(skeleton_rows, top_inset, cx)
                    });
                    view.update(cx, |view, cx| view.set(skeleton_rows, top_inset, cx));
                    view.cached(StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
                });
                div().id(page_id).size_full().child(content).into_any_element()
            })
            .collect();

        let suppressed_state = self.live_drag_state.clone();
        let pager = AnimatedTabsPager::new(
            moderation_child_id(&id, "pager"),
            count,
            active_tab,
            Binding::constant(0.0),
            page_width,
        )
        .live_drag_state(Some(self.live_drag_state))
        .yields_first_tab_back_swipe_to_navigation(true)
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed, _window, cx| {
            suppressed_state.update(cx, |live, cx| {
                live.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
            });
        })))
        .children(pages);

        let key_select = self.on_select;
        div()
            .id(moderation_child_id(&id, "keys"))
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key != "tab" || !keystroke.modifiers.control || count < 2 {
                    return;
                }
                cx.stop_propagation();
                let step = if keystroke.modifiers.shift { count - 1 } else { 1 };
                key_select((selected_index + step).rem_euclid(count), window, cx);
            })
            .child(pager)
            .on_appear(moderation_child_id(&id, "focus-appear"), move |window, cx| {
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
pub struct ModerationFailureState {
    id: ElementId,
    message: SharedString,
    theme: AppTheme,
    on_retry: ButtonAction,
}

impl ModerationFailureState {
    pub fn new(
        id: impl Into<ElementId>,
        message: impl Into<SharedString>,
        theme: AppTheme,
        on_retry: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), message: message.into(), theme, on_retry: Rc::new(on_retry) }
    }
}

impl RenderOnce for ModerationFailureState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let press_retry = self.on_retry.clone();
        let refresh_retry = self.on_retry;
        scroll_view(moderation_child_id(&self.id, "scroll"))
            .refreshable(move |window, cx| {
                refresh_retry(window, cx);
                Task::ready(())
            })
            .child(
                vstack(14.0)
                    .w_full()
                    .pt(px(80.0))
                    .child(moderation_state_circle("wifi.exclamationmark", &theme))
                    .child(
                        div().w_full().px(px(32.0)).child(
                            MainText::new(self.message).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary),
                        ),
                    )
                    .child(
                        plain(moderation_child_id(&self.id, "retry"))
                            .on_press(move |window, cx| press_retry(window, cx))
                            .child(
                                MainText::new(tr("retry"))
                                    .px(14.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.primary._100),
                            ),
                    ),
            )
    }
}

#[derive(IntoElement)]
pub struct ModerationEmptyState {
    icon: SharedString,
    text: SharedString,
    theme: AppTheme,
}

impl ModerationEmptyState {
    pub fn new(icon: impl Into<SharedString>, text: impl Into<SharedString>, theme: AppTheme) -> Self {
        Self { icon: icon.into(), text: text.into(), theme }
    }
}

impl RenderOnce for ModerationEmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        vstack(14.0)
            .w_full()
            .pt(px(80.0))
            .child(moderation_state_circle(&self.icon, &theme))
            .child(
                div()
                    .w_full()
                    .px(px(32.0))
                    .child(MainText::new(self.text).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary)),
            )
    }
}

fn moderation_state_circle(icon: &str, theme: &AppTheme) -> Div {
    div()
        .size(px(76.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.bg._200)
        .children(moderation_symbol(icon, 28.0, theme.text.secondary))
}

struct ModerationSkeletonPage {
    rows: usize,
    top_inset: f32,
    _theme: Subscription,
}

impl ModerationSkeletonPage {
    fn new(rows: usize, top_inset: f32, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { rows, top_inset, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set(&mut self, rows: usize, top_inset: f32, cx: &mut Context<Self>) {
        if self.rows != rows || self.top_inset != top_inset {
            self.rows = rows;
            self.top_inset = top_inset;
            cx.notify();
        }
    }
}

impl Render for ModerationSkeletonPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .overflow_hidden()
            .when(self.top_inset > 0.0, |page| page.pt(px(self.top_inset)))
            .child(ModerationListSkeleton::new().rows(self.rows))
    }
}

#[derive(IntoElement)]
pub struct ModerationListSkeleton {
    rows: usize,
}

impl Default for ModerationListSkeleton {
    fn default() -> Self {
        Self { rows: 10 }
    }
}

impl ModerationListSkeleton {
    const TITLE_WIDTHS: [f32; 4] = [150.0, 112.0, 178.0, 132.0];
    const SUBTITLE_WIDTHS: [f32; 4] = [92.0, 124.0, 74.0, 104.0];

    pub fn new() -> Self {
        Self::default()
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }
}

impl RenderOnce for ModerationListSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let separator = rise_ui::theme(cx).border._100;
        let rows = self.rows;
        let items = (0..rows).map(move |index| {
            let title_width = Self::TITLE_WIDTHS[index % Self::TITLE_WIDTHS.len()];
            let subtitle_width = Self::SUBTITLE_WIDTHS[index % Self::SUBTITLE_WIDTHS.len()];
            vstack(0.0)
                .w_full()
                .child(
                    hstack(12.0)
                        .w_full()
                        .px(px(ModerationMetrics::ROW_HORIZONTAL_PADDING))
                        .py(px(10.0))
                        .child(SkeletonCircle::new(ModerationMetrics::AVATAR_SIZE))
                        .child(
                            vstack(8.0)
                                .align(HorizontalAlignment::Leading)
                                .child(SkeletonLine::new(title_width, 10.0))
                                .child(SkeletonLine::new(subtitle_width, 8.0)),
                        )
                        .child(spacer_min_w(0.0)),
                )
                .children((index + 1 < rows).then(|| {
                    div()
                        .w_full()
                        .pl(px(ModerationMetrics::LIST_SEPARATOR_INSET))
                        .child(div().w_full().h(px(0.5)).bg(separator))
                }))
        });
        pass_through()
            .size_full()
            .overflow_hidden()
            .child(vstack(0.0).w_full().children(items).skeleton_shimmer(None))
    }
}

/// shape of a case screen while loading, hero then two cards
#[derive(IntoElement, Default)]
pub struct ModerationCaseSkeleton;

impl ModerationCaseSkeleton {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for ModerationCaseSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let card = |height: f32| {
            div()
                .w_full()
                .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
                .child(SkeletonBlock::new(None::<f32>, height, GroupedCardMetrics::CORNER_RADIUS))
        };
        pass_through().w_full().child(
            vstack(ModerationMetrics::SECTION_SPACING)
                .w_full()
                .pt(px(12.0))
                .child(
                    vstack(12.0)
                        .w_full()
                        .child(SkeletonBlock::new(
                            ModerationMetrics::HERO_SIDE,
                            ModerationMetrics::HERO_SIDE,
                            ModerationMetrics::HERO_SIDE * 0.28,
                        ))
                        .child(SkeletonLine::new(190.0, 14.0))
                        .child(SkeletonLine::new(130.0, 10.0)),
                )
                .child(card(168.0))
                .child(card(224.0))
                .skeleton_shimmer(None),
        )
    }
}

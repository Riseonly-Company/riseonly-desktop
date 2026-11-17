use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AsyncWindowContext, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, SharedString, Window, transparent_black};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPager;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi};
use crate::modules::notify::components::release_slide_video::ReleaseSlideVideo;
use crate::modules::notify::engine::core::rise_announcements_wire_models::RiseAnnouncementSlide;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::RiseReleaseSlide;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// what the deck needs from a slide, whichever feature authored it
pub trait SlideDeckSlide: Clone + 'static {
    fn id(&self) -> i64;
    fn media_url(&self) -> &str;
    fn media_kind(&self) -> &str;
    fn title(&self) -> &str;
    fn body(&self) -> &str;
    fn action_title(&self) -> &str;
    fn has_action(&self) -> bool;
}

impl SlideDeckSlide for RiseReleaseSlide {
    fn id(&self) -> i64 {
        self.id
    }

    fn media_url(&self) -> &str {
        &self.media_url
    }

    fn media_kind(&self) -> &str {
        &self.media_kind
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn body(&self) -> &str {
        &self.body
    }

    fn action_title(&self) -> &str {
        &self.action_title
    }

    fn has_action(&self) -> bool {
        RiseReleaseSlide::has_action(self)
    }
}

impl SlideDeckSlide for RiseAnnouncementSlide {
    fn id(&self) -> i64 {
        self.id
    }

    fn media_url(&self) -> &str {
        &self.media_url
    }

    fn media_kind(&self) -> &str {
        &self.media_kind
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn body(&self) -> &str {
        &self.body
    }

    fn action_title(&self) -> &str {
        &self.action_title
    }

    fn has_action(&self) -> bool {
        RiseAnnouncementSlide::has_action(self)
    }
}

/// media box metrics
/// the admin dashboard preview mirrors them by hand so changing one side makes the preview lie
pub enum SlideDeckMetrics {}

impl SlideDeckMetrics {
    pub const COPY_ALLOWANCE: f32 = 170.0;
    pub const FALLBACK_MEDIA_HEIGHT: f32 = 230.0;
    pub const MINIMUM_MEDIA_HEIGHT: f32 = 140.0;
    pub const MAXIMUM_MEDIA_HEIGHT: f32 = 280.0;
    pub const CLOSE_BUTTON_ROOM: f32 = 36.0;
    pub const TEXT_ONLY_PAGE_HEIGHT: f32 = 400.0;
    pub const PAGE_HORIZONTAL_PADDING: f32 = 18.0;

    /// width is always spent in full so height follows the aspect
    /// only the clamped cases crop
    pub fn media_height(aspect: Option<f32>, width: f32) -> f32 {
        let Some(aspect) = aspect.filter(|aspect| *aspect > 0.0 && width > 0.0) else {
            return Self::FALLBACK_MEDIA_HEIGHT;
        };
        let ceiling = width.min(Self::MAXIMUM_MEDIA_HEIGHT);
        (width / aspect).max(Self::MINIMUM_MEDIA_HEIGHT).min(ceiling)
    }
}

struct SlideDeckUiState {
    media_aspects: HashMap<i64, f32>,
    deck_width: f32,
    displayed_media_height: Animated<f32>,
    focus: FocusHandle,
}

/// paged deck with slides, dots and footer button
/// one media height for every page, the tallest any slide needs
/// so a page height change mid swipe doesnt resize the modal
#[derive(IntoElement)]
pub struct SlideDeckUi<Slide: SlideDeckSlide> {
    id: ElementId,
    slides: Rc<Vec<Slide>>,
    slide_index: Binding<i64>,
    is_blocking: bool,
    shows_media_box: bool,
    theme: AppTheme,
    on_action: Rc<dyn Fn(&Slide, &mut Window, &mut App)>,
    on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl<Slide: SlideDeckSlide> SlideDeckUi<Slide> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        slides: Vec<Slide>,
        slide_index: Binding<i64>,
        is_blocking: bool,
        shows_media_box: bool,
        theme: AppTheme,
        on_action: Rc<dyn Fn(&Slide, &mut Window, &mut App)>,
        on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    ) -> Self {
        Self {
            id: id.into(),
            slides: Rc::new(slides),
            slide_index,
            is_blocking,
            shows_media_box,
            theme,
            on_action,
            on_dismiss,
        }
    }

    fn active_slide(&self, slide_index: i64) -> Option<&Slide> {
        usize::try_from(slide_index).ok().and_then(|index| self.slides.get(index)).or_else(|| self.slides.first())
    }

    fn is_last_slide(&self, slide_index: i64) -> bool {
        slide_index >= self.slide_count() - 1
    }

    fn deck_has_action(&self) -> bool {
        self.slides.iter().any(|slide| slide.has_action())
    }

    fn slide_count(&self) -> i64 {
        i64::try_from(self.slides.len()).unwrap_or(i64::MAX)
    }

    fn content_width(deck_width: f32) -> f32 {
        (deck_width - SlideDeckMetrics::PAGE_HORIZONTAL_PADDING * 2.0).max(0.0)
    }

    fn media_height(&self, media_aspects: &HashMap<i64, f32>, deck_width: f32) -> f32 {
        let content_width = Self::content_width(deck_width);
        if content_width <= 0.0 {
            return SlideDeckMetrics::FALLBACK_MEDIA_HEIGHT;
        }
        self.slides
            .iter()
            .map(|slide| SlideDeckMetrics::media_height(media_aspects.get(&slide.id()).copied(), content_width))
            .reduce(f32::max)
            .unwrap_or(SlideDeckMetrics::FALLBACK_MEDIA_HEIGHT)
    }

    fn page_ceiling(&self, media_height: f32) -> f32 {
        if self.shows_media_box {
            media_height + SlideDeckMetrics::COPY_ALLOWANCE
        } else {
            SlideDeckMetrics::TEXT_ONLY_PAGE_HEIGHT
        }
    }

    fn page_sizer(&self, media_height: f32) -> impl IntoElement {
        vstack(16.0)
            .when(self.shows_media_box, |sizer| sizer.child(div().flex_none().w_full().h(px(media_height))))
            .child(div().flex().flex_row().items_start().w_full().children(self.slides.iter().enumerate().map(
                |(index, slide)| {
                    div()
                        .flex_none()
                        .w_full()
                        .when(index > 0, |copy| copy.ml(gpui::relative(-1.0)))
                        .child(SlideDeckCopy { slide: slide.clone() })
                },
            )))
            .child(div().flex_none().h(px(0.0)))
            .px(px(SlideDeckMetrics::PAGE_HORIZONTAL_PADDING))
            .pt(px(14.0))
            .w_full()
            .max_h(px(self.page_ceiling(media_height)))
            .overflow_hidden()
            .invisible()
    }

    fn pager(&self, media_height: f32, slide_index: i64, state: &Entity<SlideDeckUiState>) -> impl IntoElement {
        SlideDeckPagerHeight {
            sizer: self.page_sizer(media_height).into_any_element(),
            overlay: self.pager_tabs(media_height, slide_index, state).into_any_element(),
        }
    }

    fn pager_tabs(&self, media_height: f32, slide_index: i64, state: &Entity<SlideDeckUiState>) -> impl IntoElement {
        let pages = self.slides.iter().enumerate().map(|(index, slide)| {
            let index = i64::try_from(index).unwrap_or(i64::MAX);
            let slide_id = slide.id();
            let aspect_state = state.clone();
            SlideDeckPage {
                id: named_child(&self.id, SharedString::from(format!("page-{slide_id}"))),
                slide: slide.clone(),
                renders_media: (index - slide_index).abs() <= 1,
                is_current: index == slide_index,
                shows_media_box: self.shows_media_box,
                media_height,
                theme: self.theme.clone(),
                on_natural_aspect: Rc::new(move |aspect: f32, cx: &mut App| {
                    if aspect <= 0.0 {
                        return;
                    }
                    aspect_state.update(cx, |state, cx| {
                        if let Some(known) = state.media_aspects.get(&slide_id)
                            && (known - aspect).abs() <= known * 0.01
                        {
                            return;
                        }
                        state.media_aspects.insert(slide_id, aspect);
                        cx.notify();
                    });
                }),
            }
        });

        AnimatedTabsPager::new(
            named_child(&self.id, "pager"),
            self.slide_count(),
            self.slide_index.clone(),
            Binding::constant(0.0),
            Binding::constant(0.0),
        )
        .children(pages)
    }

    fn dots(&self, slide_index: i64) -> AnyElement {
        if self.slides.len() > 1 {
            self.page_dots(slide_index).into_any_element()
        } else {
            div().flex_none().h(px(4.0)).into_any_element()
        }
    }

    fn page_dots(&self, slide_index: i64) -> impl IntoElement {
        hstack(6.0)
            .flex_none()
            .pt(px(4.0))
            .pb(px(16.0))
            .children((0..self.slide_count()).map(|index| SlideDeckDot {
                id: named_child(&self.id, SharedString::from(format!("dot-{index}"))),
                is_current: index == slide_index,
                theme: self.theme.clone(),
            }))
    }

    fn footer(&self, slide_index: i64) -> impl IntoElement {
        let theme = &self.theme;
        let active_slide = self.active_slide(slide_index);
        let active_has_action = active_slide.is_some_and(|slide| slide.has_action());
        let deck_has_action = self.deck_has_action();
        let primary_fill = theme.primary._100;
        let closing_fill = if active_has_action { theme.bg._300 } else { primary_fill };
        let closing_text = if active_has_action { theme.text.primary } else { theme.content_on(primary_fill) };

        let top: Option<AnyElement> = match active_slide {
            Some(slide) if slide.has_action() => {
                let slide = slide.clone();
                let title = slide.action_title().to_owned();
                let on_action = self.on_action.clone();
                Some(
                    ButtonUi::new(named_child(&self.id, "action"))
                        .background_color(primary_fill)
                        .height(50.0)
                        .on_press(move |window, cx| on_action(&slide, window, cx))
                        .child(
                            MainText::new(title)
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(primary_fill)),
                        )
                        .into_any_element(),
                )
            }
            _ if !self.is_blocking && deck_has_action => {
                Some(div().flex_none().h(px(50.0)).into_any_element())
            }
            _ => None,
        };

        let bottom: Option<AnyElement> = if self.is_blocking {
            (!active_has_action).then(|| {
                div()
                    .w_full()
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(deck_has_action, |hint| hint.min_h(px(50.0)))
                    .child(SecondaryText::new(tr("release_notes_update_hint")).px(13.0).tac(TextAlignT::Center).w_full())
                    .into_any_element()
            })
        } else if self.is_last_slide(slide_index) || self.slides.len() == 1 {
            let on_dismiss = self.on_dismiss.clone();
            Some(
                ButtonUi::new(named_child(&self.id, "done"))
                    .background_color(closing_fill)
                    .height(50.0)
                    .on_press(move |window, cx| on_dismiss(window, cx))
                    .child(
                        MainText::new(tr("release_notes_done"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(closing_text),
                    )
                    .into_any_element(),
            )
        } else {
            let slide_binding = self.slide_index.clone();
            let slide_count = self.slide_count();
            Some(
                ButtonUi::new(named_child(&self.id, "continue"))
                    .background_color(closing_fill)
                    .height(50.0)
                    .on_press(move |window, cx| advance(&slide_binding, slide_count, window, cx))
                    .child(
                        MainText::new(tr("release_notes_continue"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(closing_text),
                    )
                    .into_any_element(),
            )
        };

        vstack(8.0).flex_none().w_full().px(px(18.0)).children(top).children(bottom)
    }
}

fn advance(slide_index: &Binding<i64>, slide_count: i64, window: &mut Window, cx: &mut App) {
    let current = slide_index.get(cx);
    if current < slide_count - 1 {
        slide_index.set(current + 1, window, cx);
    }
}

async fn resolve_media_aspects<Slide: SlideDeckSlide>(
    slides: Rc<Vec<Slide>>,
    shows_media_box: bool,
    state: Entity<SlideDeckUiState>,
    cx: &mut AsyncWindowContext,
) {
    if !shows_media_box {
        return;
    }
    for (index, slide) in slides.iter().enumerate() {
        let slide_id = slide.id();
        if state.read_with(&*cx, |state, _| state.media_aspects.contains_key(&slide_id)) {
            continue;
        }
        if !loads_remote_image(slide) {
            continue;
        }
        let image = CleverImageBitmapLoader::load(
            slide.media_url(),
            true,
            true,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            Some(256),
            CleverImageCachePolicy::Standard,
            if index <= 1 { CleverImageWorkLane::Visible } else { CleverImageWorkLane::Speculative },
            &*cx,
        )
        .await;
        let Some(image) = image else { continue };
        let size = image.size(0);
        if size.width.0 <= 0 || size.height.0 <= 0 {
            continue;
        }
        let aspect = size.width.0 as f32 / size.height.0 as f32;
        state.update(cx, |state, cx| {
            if !state.media_aspects.contains_key(&slide_id) {
                state.media_aspects.insert(slide_id, aspect);
                cx.notify();
            }
        });
    }
}

fn loads_remote_image<Slide: SlideDeckSlide>(slide: &Slide) -> bool {
    !slide.media_url().is_empty()
        && slide.media_kind() != "emoji"
        && slide.media_kind() != "video"
        && slide.media_kind() != "none"
        && slide.media_url().to_lowercase().starts_with("http")
}

impl<Slide: SlideDeckSlide> RenderOnce for SlideDeckUi<Slide> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| SlideDeckUiState {
            media_aspects: HashMap::new(),
            deck_width: 0.0,
            displayed_media_height: Animated::new(SlideDeckMetrics::FALLBACK_MEDIA_HEIGHT),
            focus: cx.focus_handle(),
        });
        let slide_index = self.slide_index.get(cx);

        let focus = state.read(cx).focus.clone();
        let appear_focus = focus.clone();
        on_change(named_child(&self.id, "focus"), (), true, window, cx, move |_, _, window, cx| {
            window.focus(&appear_focus, cx);
        });

        let slide_ids: Vec<i64> = self.slides.iter().map(|slide| slide.id()).collect();
        let task_slides = self.slides.clone();
        let task_state = state.clone();
        let shows_media_box = self.shows_media_box;
        use_task(
            named_child(&self.id, "resolve-aspects"),
            slide_ids,
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                resolve_media_aspects(task_slides, shows_media_box, task_state, cx).await;
            },
        );

        let target_height = {
            let state = state.read(cx);
            self.media_height(&state.media_aspects, state.deck_width)
        };
        let height_state = state.clone();
        on_change(
            named_child(&self.id, "media-height"),
            target_height,
            false,
            window,
            cx,
            move |_, new_height, _, cx| {
                let new_height = *new_height;
                height_state.update(cx, |state, cx| {
                    state.displayed_media_height.set_in(new_height, ease_out(0.18), RiseClock::monotonic(), cx);
                    cx.notify();
                });
            },
        );
        let media_height = state.read(cx).displayed_media_height.frame_value(RiseClock::monotonic(), window);

        let width_state = state.clone();
        let key_binding = self.slide_index.clone();
        let slide_count = self.slide_count();
        let right_to_left = layout_direction(cx) == LayoutDirection::RightToLeft;

        vstack(0.0)
            .id(named_child(&self.id, "deck"))
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let forward = match event.keystroke.key.as_str() {
                    "right" => !right_to_left,
                    "left" => right_to_left,
                    _ => return,
                };
                let current = key_binding.get(cx);
                if forward {
                    advance(&key_binding, slide_count, window, cx);
                } else if current > 0 {
                    key_binding.set(current - 1, window, cx);
                }
                cx.stop_propagation();
            })
            .relative()
            .w_full()
            .min_h_0()
            .child(self.pager(media_height, slide_index, &state))
            .child(self.dots(slide_index))
            .child(self.footer(slide_index))
            .child(measure(named_child(&self.id, "width"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                width_state.update(cx, |state, cx| {
                    if (width - state.deck_width).abs() > 0.5 {
                        state.deck_width = width;
                        cx.notify();
                    }
                });
            }))
    }
}

#[derive(IntoElement)]
struct SlideDeckDot {
    id: ElementId,
    is_current: bool,
    theme: AppTheme,
}

impl RenderOnce for SlideDeckDot {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_current = self.is_current;
        let target = if is_current { 1.0_f32 } else { 0.0_f32 };
        let progress = window.use_keyed_state(named_child(&self.id, "progress"), cx, |_, _| Animated::new(target));
        let progress_writer = progress.clone();
        on_change(named_child(&self.id, "current"), is_current, false, window, cx, move |_, _, _, cx| {
            progress_writer.update(cx, |progress, cx| {
                progress.set_in(target, ease_out(0.18), RiseClock::monotonic(), cx);
                cx.notify();
            });
        });
        let value = progress.read(cx).frame_value(RiseClock::monotonic(), window).clamp(0.0, 1.0);
        div()
            .flex_none()
            .w(px(6.0 + 12.0 * value))
            .h(px(6.0))
            .rounded_full()
            .bg(self.theme.bg._400.blend(self.theme.primary._100.opacity(value)))
    }
}

#[derive(IntoElement)]
pub struct SlideDeckCloseButton {
    id: ElementId,
    theme: AppTheme,
    on_press: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl SlideDeckCloseButton {
    pub fn new(
        id: impl Into<ElementId>,
        theme: AppTheme,
        on_press: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), theme, on_press: Rc::new(on_press) }
    }
}

impl RenderOnce for SlideDeckCloseButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_press = self.on_press;
        div().id(named_child(&self.id, "hint")).tooltip_label(tr("release_notes_close")).child(
            ButtonUi::new(self.id)
                .background_color(transparent_black())
                .height(34.0)
                .fit_content(true)
                .padding_horizontal(11.0)
                .on_press(move |window, cx| on_press(window, cx))
                .children(IconUi::sized("xmark", px(13.0), self.theme.text.secondary, false)),
        )
    }
}

/// title row shared by both decks
/// title keeps every line it needs and stays clear of the close button
/// on a short window the deck copy gives way
#[derive(IntoElement)]
pub struct SlideDeckHeader {
    id: ElementId,
    title: SharedString,
    theme: AppTheme,
    on_close: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl SlideDeckHeader {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        theme: AppTheme,
        on_close: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    ) -> Self {
        Self { id: id.into(), title: title.into(), theme, on_close }
    }
}

impl RenderOnce for SlideDeckHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let right_to_left = layout_direction(cx) == LayoutDirection::RightToLeft;
        let room = if self.on_close.is_some() { SlideDeckMetrics::CLOSE_BUTTON_ROOM } else { 0.0 };
        let close = self.on_close.map(|on_close| {
            div()
                .absolute()
                .top_0()
                .when(right_to_left, |slot| slot.left_0())
                .when(!right_to_left, |slot| slot.right_0())
                .child(SlideDeckCloseButton::new(named_child(&self.id, "close"), self.theme.clone(), move |window, cx| {
                    on_close(window, cx)
                }))
        });
        div().flex_none().w_full().px(px(18.0)).child(
            div()
                .relative()
                .w_full()
                .px(px(room))
                .child(
                    MainText::new(self.title)
                        .px(22.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::BOLD)
                        .w_full(),
                )
                .children(close),
        )
    }
}

#[derive(IntoElement)]
struct SlideDeckPagerHeight {
    sizer: AnyElement,
    overlay: AnyElement,
}

impl RenderOnce for SlideDeckPagerHeight {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .relative()
            .w_full()
            .min_h_0()
            .overflow_hidden()
            .child(self.sizer)
            .child(div().absolute().inset_0().child(self.overlay))
    }
}

#[derive(IntoElement)]
struct SlideDeckPage<Slide: SlideDeckSlide> {
    id: ElementId,
    slide: Slide,
    renders_media: bool,
    is_current: bool,
    shows_media_box: bool,
    media_height: f32,
    theme: AppTheme,
    on_natural_aspect: Rc<dyn Fn(f32, &mut App)>,
}

impl<Slide: SlideDeckSlide> SlideDeckPage<Slide> {
    fn media(&self) -> AnyElement {
        let media_url = self.slide.media_url();
        let media_kind = self.slide.media_kind();
        if media_kind == "emoji" && !media_url.is_empty() {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(16.0))
                .bg(self.theme.bg._300)
                .child(MainText::new(media_url.to_owned()).px(96.0).minimum_scale_factor(0.4))
                .into_any_element()
        } else if self.renders_media && !media_url.is_empty() && media_kind == "video" {
            div()
                .size_full()
                .rounded(px(16.0))
                .overflow_hidden()
                .child(ReleaseSlideVideo::new(named_child(&self.id, "video"), media_url.to_owned(), self.is_current))
                .into_any_element()
        } else if self.renders_media && !media_url.is_empty() && media_kind != "none" {
            let on_natural_aspect = self.on_natural_aspect.clone();
            div()
                .size_full()
                .child(
                    CleverImageUi::new(media_url.to_owned())
                        .id(named_child(&self.id, "image"))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(16.0)
                        .persist_to_disk(true)
                        .on_natural_size(move |size, _, cx| {
                            if size.height > 0.0 {
                                on_natural_aspect(size.width / size.height, cx);
                            }
                        }),
                )
                .into_any_element()
        } else {
            div().size_full().rounded(px(16.0)).bg(self.theme.bg._300).into_any_element()
        }
    }
}

impl<Slide: SlideDeckSlide> RenderOnce for SlideDeckPage<Slide> {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let media = self.shows_media_box.then(|| {
            div().flex_none().w_full().h(px(self.media_height)).overflow_hidden().child(self.media())
        });
        vstack(16.0)
            .size_full()
            .min_h_0()
            .px(px(SlideDeckMetrics::PAGE_HORIZONTAL_PADDING))
            .pt(px(14.0))
            .children(media)
            .child(
                scroll_view(named_child(&self.id, "copy"))
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(SlideDeckCopy { slide: self.slide }),
            )
            .child(div().flex_none().h(px(0.0)))
    }
}

/// heading and text of a slide, shared by the page and the hidden sizer
/// so they cant measure differently
#[derive(IntoElement)]
struct SlideDeckCopy<Slide: SlideDeckSlide> {
    slide: Slide,
}

impl<Slide: SlideDeckSlide> RenderOnce for SlideDeckCopy<Slide> {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let title = self.slide.title();
        let body = self.slide.body();
        vstack(7.0)
            .w_full()
            .px(px(6.0))
            .when(!title.is_empty(), |copy| {
                copy.child(
                    MainText::new(title.to_owned())
                        .px(17.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .w_full(),
                )
            })
            .when(!body.is_empty(), |copy| {
                copy.child(SecondaryText::new(body.to_owned()).px(14.0).tac(TextAlignT::Center).w_full())
            })
    }
}

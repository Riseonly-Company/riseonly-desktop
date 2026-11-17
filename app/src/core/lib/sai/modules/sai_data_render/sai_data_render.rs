use std::panic::Location;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ElementId, FontWeight, IntoElement, ParentElement, Pixels,
    RenderOnce, Styled, Window, container_query, div, px,
};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::environment::{EnvKey, EnvScope, env, env_scope};
use rise_ui::swiftui::measure::measure;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellCoordinateSpace, ListShellFloatingTopInsetKey};

/// builds a state view on demand
pub type SaiDataRenderBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

type SaiDataRenderContent = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;

#[derive(IntoElement)]
pub struct SaiDataRender {
    status: SaiWsSliceStatus,
    is_empty: bool,
    no_data_text: Option<String>,
    error_text: Option<String>,
    need_pending: bool,
    message_height_percent: f32,
    no_data_height_percent: f32,
    is_skeleton: bool,
    content: SaiDataRenderContent,
    loading_builder: Option<SaiDataRenderBuilder>,
    error_builder: Option<SaiDataRenderBuilder>,
    empty_builder: Option<SaiDataRenderBuilder>,
    geometry_key: ElementId,
}

impl SaiDataRender {
    #[track_caller]
    pub fn new<E: IntoElement>(
        status: SaiWsSliceStatus,
        content: impl FnOnce(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            status,
            is_empty: false,
            no_data_text: None,
            error_text: None,
            need_pending: true,
            message_height_percent: 50.0,
            no_data_height_percent: 15.0,
            is_skeleton: false,
            content: Box::new(move |window, cx| content(window, cx).into_any_element()),
            loading_builder: None,
            error_builder: None,
            empty_builder: None,
            geometry_key: ElementId::CodeLocation(*Location::caller()),
        }
    }

    /// status and error come from the slice
    /// an explicit error_text set after this wins over the slice error
    #[track_caller]
    pub fn slice<E: IntoElement>(
        slice: &dyn SaiSliceState,
        content: impl FnOnce(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        let mut render = Self::new(slice.status(), content);
        render.error_text = slice.error();
        render
    }

    /// key of the empty view frame state, defaults to the call site
    /// two renders built at one call site under one parent (pager pages) need distinct ids
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.geometry_key = id.into();
        self
    }

    pub fn is_empty(mut self, is_empty: bool) -> Self {
        self.is_empty = is_empty;
        self
    }

    pub fn no_data_text(mut self, no_data_text: impl Into<Option<String>>) -> Self {
        self.no_data_text = no_data_text.into();
        self
    }

    pub fn error_text(mut self, error_text: impl Into<Option<String>>) -> Self {
        if let Some(error_text) = error_text.into() {
            self.error_text = Some(error_text);
        }
        self
    }

    pub fn need_pending(mut self, need_pending: bool) -> Self {
        self.need_pending = need_pending;
        self
    }

    pub fn message_height_percent(mut self, message_height_percent: f32) -> Self {
        self.message_height_percent = message_height_percent;
        self
    }

    pub fn no_data_height_percent(mut self, no_data_height_percent: f32) -> Self {
        self.no_data_height_percent = no_data_height_percent;
        self
    }

    pub fn is_skeleton(mut self, is_skeleton: bool) -> Self {
        self.is_skeleton = is_skeleton;
        self
    }

    pub fn sai_data_render_loading<E: IntoElement>(
        mut self,
        make: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.loading_builder = Some(Rc::new(move |window, cx| make(window, cx).into_any_element()));
        self
    }

    pub fn sai_data_render_error<E: IntoElement>(
        mut self,
        make: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.error_builder = Some(Rc::new(move |window, cx| make(window, cx).into_any_element()));
        self
    }

    pub fn sai_data_render_empty<E: IntoElement>(
        mut self,
        make: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.empty_builder = Some(Rc::new(move |window, cx| make(window, cx).into_any_element()));
        self
    }

    fn effective_status(&self) -> SaiWsSliceStatus {
        if !self.need_pending {
            return SaiWsSliceStatus::Fulfilled;
        }
        self.status
    }

    fn render_loading(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if let Some(builder) = &self.loading_builder {
            return builder(window, cx);
        }
        if let Some(builder) = env::<SaiDataRenderLoadingKey>(cx) {
            return builder(window, cx);
        }
        SaiDataRenderDefaultsLoading {
            message_height_percent: self.message_height_percent,
            is_skeleton: self.is_skeleton,
        }
        .into_any_element()
    }

    fn render_error(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if let Some(builder) = &self.error_builder {
            return builder(window, cx);
        }
        if let Some(builder) = env::<SaiDataRenderErrorKey>(cx) {
            return builder(window, cx);
        }
        SaiDataRenderDefaultsError {
            text: self.error_text.clone().unwrap_or_else(|| tr("error_fetching_data")),
            message_height_percent: self.message_height_percent,
        }
        .into_any_element()
    }

    fn render_empty(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if let Some(builder) = &self.empty_builder {
            return builder(window, cx);
        }
        if let Some(builder) = env::<SaiDataRenderEmptyKey>(cx) {
            return builder(window, cx);
        }
        SaiDataRenderDefaultsEmpty {
            text: self.no_data_text.clone().unwrap_or_else(|| tr("no_data")),
            no_data_height_percent: self.no_data_height_percent,
            geometry_key: self.geometry_key.clone(),
        }
        .into_any_element()
    }
}

impl RenderOnce for SaiDataRender {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        match self.effective_status() {
            SaiWsSliceStatus::Idle | SaiWsSliceStatus::Pending => self.render_loading(window, cx),
            SaiWsSliceStatus::Rejected => self.render_error(window, cx),
            SaiWsSliceStatus::Fulfilled => {
                if self.is_empty {
                    self.render_empty(window, cx)
                } else {
                    (self.content)(window, cx)
                }
            }
        }
    }
}

const SAI_DATA_RENDER_SKELETON_ROW_HEIGHT: f32 = 60.0;

#[derive(IntoElement)]
pub struct SaiDataRenderDefaultsLoading {
    pub message_height_percent: f32,
    pub is_skeleton: bool,
}

impl RenderOnce for SaiDataRenderDefaultsLoading {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let message_height_percent = self.message_height_percent;
        if self.is_skeleton {
            return div()
                .size_full()
                .overflow_hidden()
                .child(container_query(move |size, _, _| {
                    let rows = ((f32::from(size.height) / SAI_DATA_RENDER_SKELETON_ROW_HEIGHT).ceil()
                        as usize)
                        .max(3);
                    div()
                        .size_full()
                        .child(SkeletonListUi { rows, ..SkeletonListUi::default() })
                }))
                .into_any_element();
        }
        container_query(move |size, _, _| {
            let y_offset = (f32::from(size.height) * (message_height_percent / 100.0) - 14.0).max(0.0);
            div().relative().size_full().child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(y_offset))
                    .flex()
                    .justify_center()
                    .child(LoaderUi::new().size(LoaderUiSize::Large)),
            )
        })
        .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct SaiDataRenderDefaultsError {
    pub text: String,
    pub message_height_percent: f32,
}

impl RenderOnce for SaiDataRenderDefaultsError {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_primary = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
        let text = self.text;
        let message_height_percent = self.message_height_percent;
        container_query(move |size, _, _| {
            let y_offset = (f32::from(size.height) * (message_height_percent / 100.0) - 10.0).max(0.0);
            div().relative().size_full().child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(y_offset))
                    .px(px(24.0))
                    .flex()
                    .justify_center()
                    .child(
                        MainText::new(text)
                            .min_w_0()
                            .px(16.0)
                            .tac(TextAlignT::Center)
                            .color(text_primary),
                    ),
            )
        })
    }
}

struct SaiDataRenderGeometry(Option<Bounds<Pixels>>);

#[derive(IntoElement)]
pub struct SaiDataRenderDefaultsEmpty {
    pub text: String,
    pub no_data_height_percent: f32,
    pub geometry_key: ElementId,
}

impl RenderOnce for SaiDataRenderDefaultsEmpty {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_primary = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
        let floating_top_inset = env::<ListShellFloatingTopInsetKey>(cx);
        let content_space = ListShellCoordinateSpace::Content.frame(cx);
        let state = window.use_keyed_state(self.geometry_key.clone(), cx, |_, _| SaiDataRenderGeometry(None));
        let frame = state.read(cx).0;
        let min_y = frame.map(|frame| {
            let space_top = content_space.map_or(Pixels::ZERO, |space| space.origin.y);
            f32::from(frame.origin.y - space_top)
        });
        let text = self.text;
        let no_data_height_percent = self.no_data_height_percent;
        let reader = measure(self.geometry_key, move |bounds, _, cx| {
            state.update(cx, |geometry, cx| {
                let wakes = geometry.0.map(|frame| frame.origin.y) != Some(bounds.origin.y);
                geometry.0 = Some(bounds);
                if wakes {
                    cx.notify();
                }
            });
        });
        div()
            .relative()
            .size_full()
            .child(container_query(move |size, _, _| {
                let top_overlap = match (floating_top_inset > 0.0, min_y) {
                    (true, Some(min_y)) => (floating_top_inset - min_y).max(0.0),
                    _ => 0.0,
                };
                let available_height = (f32::from(size.height) - top_overlap).max(0.0);
                let y_offset = top_overlap + (available_height * (no_data_height_percent / 100.0)).max(0.0);
                div().relative().size_full().child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(y_offset))
                        .px(px(24.0))
                        .flex()
                        .justify_center()
                        .child(
                            MainText::new(text)
                                .min_w_0()
                                .px(17.0)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::BOLD)
                                .color(text_primary),
                        ),
                )
            }))
            .child(reader)
    }
}

pub struct SaiDataRenderLoadingKey;

impl EnvKey for SaiDataRenderLoadingKey {
    type Value = Option<SaiDataRenderBuilder>;

    fn default_value() -> Option<SaiDataRenderBuilder> {
        None
    }
}

pub struct SaiDataRenderErrorKey;

impl EnvKey for SaiDataRenderErrorKey {
    type Value = Option<SaiDataRenderBuilder>;

    fn default_value() -> Option<SaiDataRenderBuilder> {
        None
    }
}

pub struct SaiDataRenderEmptyKey;

impl EnvKey for SaiDataRenderEmptyKey {
    type Value = Option<SaiDataRenderBuilder>;

    fn default_value() -> Option<SaiDataRenderBuilder> {
        None
    }
}

/// loading view builder from the environment, if set
pub fn sai_data_render_loading(cx: &App) -> Option<SaiDataRenderBuilder> {
    env::<SaiDataRenderLoadingKey>(cx)
}

/// error view builder from the environment, if set
pub fn sai_data_render_error(cx: &App) -> Option<SaiDataRenderBuilder> {
    env::<SaiDataRenderErrorKey>(cx)
}

/// empty view builder from the environment, if set
pub fn sai_data_render_empty(cx: &App) -> Option<SaiDataRenderBuilder> {
    env::<SaiDataRenderEmptyKey>(cx)
}

pub trait SaiDataRenderModifiers: IntoElement + Sized {
    fn sai_data_render_default_loading<E: IntoElement>(
        self,
        make: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> EnvScope<SaiDataRenderLoadingKey> {
        let builder: SaiDataRenderBuilder = Rc::new(move |window, cx| make(window, cx).into_any_element());
        env_scope::<SaiDataRenderLoadingKey>(Some(builder)).child(self)
    }

    fn sai_data_render_default_error<E: IntoElement>(
        self,
        make: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> EnvScope<SaiDataRenderErrorKey> {
        let builder: SaiDataRenderBuilder = Rc::new(move |window, cx| make(window, cx).into_any_element());
        env_scope::<SaiDataRenderErrorKey>(Some(builder)).child(self)
    }

    fn sai_data_render_default_empty<E: IntoElement>(
        self,
        make: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> EnvScope<SaiDataRenderEmptyKey> {
        let builder: SaiDataRenderBuilder = Rc::new(move |window, cx| make(window, cx).into_any_element());
        env_scope::<SaiDataRenderEmptyKey>(Some(builder)).child(self)
    }
}

impl<E: IntoElement> SaiDataRenderModifiers for E {}

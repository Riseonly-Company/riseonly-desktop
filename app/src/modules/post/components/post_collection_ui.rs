use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Pixels, Point, RenderOnce, SharedString, Size, Styled, Task, Window, container_query, div, px, size,
};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{ThemeStore, alpha};
use rise_ui::animated_tabs_ui::animated_tabs_types::AnimatedTabsMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::rich_text::rich_text_markup::RichTextMarkup;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::{ButtonAction, button_chrome, plain};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiAlignedFrame, SwiftUiFrame};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::material::Material;
use rise_ui::swiftui::modifiers::{ModifiedContent, ModifierExt, ViewModifier};
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::stacks::{Alignment, hstack, vstack};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::text_sizing::{GridTileTextFit, grid_title_font_size};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RISE_ENGINE_CONNECTION_READY;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::{SharePayload, ShareSheetStore};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouter;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::engine::rise_post_engine_models::RisePostPageStatus;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedCollectionElement<Element> {
    pub offset: i64,
    pub element: Element,
}

/// base is a slice here, every call site hands an array
#[derive(Clone, Copy, Debug)]
pub struct IndexedRandomAccessCollection<'a, Element> {
    base: &'a [Element],
}

impl<'a, Element> IndexedRandomAccessCollection<'a, Element> {
    pub fn new(base: &'a [Element]) -> Self {
        Self { base }
    }

    pub fn start_index(&self) -> usize {
        0
    }

    pub fn end_index(&self) -> usize {
        self.base.len()
    }

    /// item at position, none past the end
    pub fn at(&self, position: usize) -> Option<IndexedCollectionElement<&'a Element>> {
        self.base.get(position).map(|element| IndexedCollectionElement { offset: position as i64, element })
    }

    pub fn index_after(&self, index: usize) -> usize {
        index + 1
    }

    pub fn index_before(&self, index: usize) -> usize {
        index.saturating_sub(1)
    }

    pub fn index_offset_by(&self, index: usize, distance: i64) -> usize {
        index.saturating_add_signed(distance as isize)
    }

    pub fn distance(&self, start: usize, end: usize) -> i64 {
        end as i64 - start as i64
    }

    pub fn iter(&self) -> impl Iterator<Item = IndexedCollectionElement<&'a Element>> + use<'a, Element> {
        self.base
            .iter()
            .enumerate()
            .map(|(offset, element)| IndexedCollectionElement { offset: offset as i64, element })
    }
}

pub type PostPageStateUiView = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;

fn empty_view() -> PostPageStateUiView {
    Box::new(|_: &mut Window, _: &mut App| div().into_any_element())
}

struct PostPageStateConnectionObserver {
    status: RisePostPageStatus,
    on_retry: Rc<dyn Fn(&mut App)>,
    _subscription: RiseSubscription,
    _delivery: Task<()>,
}

impl PostPageStateConnectionObserver {
    fn new(status: RisePostPageStatus, on_retry: Rc<dyn Fn(&mut App)>, cx: &mut Context<Self>) -> Self {
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let subscription = RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
            let _ = sender.unbounded_send(());
        });
        let delivery = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                let alive = this
                    .update(cx, |observer, cx| {
                        if observer.status != RisePostPageStatus::Failed {
                            return;
                        }
                        let on_retry = observer.on_retry.clone();
                        cx.defer(move |cx| on_retry(cx));
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        });
        Self { status, on_retry, _subscription: subscription, _delivery: delivery }
    }
}

#[derive(IntoElement)]
pub struct PostPageStateUi {
    id: ElementId,
    pub status: RisePostPageStatus,
    pub has_items: bool,
    pub has_subject: bool,
    pub empty_icon: String,
    pub empty_text: String,
    pub error_text: String,
    pub on_retry: Rc<dyn Fn(&mut App)>,
    pub loading: PostPageStateUiView,
    pub content: PostPageStateUiView,
}

impl PostPageStateUi {
    /// new feed status view
    /// loading and content builders are set by the builder methods below
    pub fn new(
        id: impl Into<ElementId>,
        status: RisePostPageStatus,
        has_items: bool,
        empty_icon: impl Into<String>,
        empty_text: impl Into<String>,
        error_text: impl Into<String>,
        on_retry: impl Fn(&mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            status,
            has_items,
            has_subject: true,
            empty_icon: empty_icon.into(),
            empty_text: empty_text.into(),
            error_text: error_text.into(),
            on_retry: Rc::new(on_retry),
            loading: empty_view(),
            content: empty_view(),
        }
    }

    pub fn has_subject(mut self, has_subject: bool) -> Self {
        self.has_subject = has_subject;
        self
    }

    pub fn loading<E: IntoElement>(mut self, loading: impl FnOnce(&mut Window, &mut App) -> E + 'static) -> Self {
        self.loading = Box::new(move |window: &mut Window, cx: &mut App| loading(window, cx).into_any_element());
        self
    }

    pub fn content<E: IntoElement>(mut self, content: impl FnOnce(&mut Window, &mut App) -> E + 'static) -> Self {
        self.content = Box::new(move |window: &mut Window, cx: &mut App| content(window, cx).into_any_element());
        self
    }

    fn state_body(self, window: &mut Window, cx: &mut App) -> AnyElement {
        if !self.has_subject && !self.has_items {
            return ChatProfileEmptyState::new(self.empty_icon, self.empty_text).into_any_element();
        }
        if self.has_items {
            let content = (self.content)(window, cx);
            let is_failed = self.status == RisePostPageStatus::Failed;
            let refresh_id = child_id(&self.id, "refresh-error");
            let on_retry = self.on_retry;
            let error_text = self.error_text;
            return div()
                .relative()
                .w_full()
                .child(content)
                .when(is_failed, move |page| {
                    page.child(
                        div().absolute().top_0().left_0().right_0().flex().justify_center().pt(px(8.0)).child(
                            PostPageRefreshErrorUi::new(refresh_id, error_text, move |cx: &mut App| on_retry(cx)),
                        ),
                    )
                })
                .into_any_element();
        }
        match self.status {
            RisePostPageStatus::Idle | RisePostPageStatus::Loading => (self.loading)(window, cx),
            RisePostPageStatus::Loaded => {
                ChatProfileEmptyState::new(self.empty_icon, self.empty_text).into_any_element()
            }
            RisePostPageStatus::Failed => {
                let on_retry = self.on_retry;
                PostPageErrorUi::new(child_id(&self.id, "page-error"), self.error_text, move |cx: &mut App| on_retry(cx))
                    .into_any_element()
            }
        }
    }
}

impl RenderOnce for PostPageStateUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let status = self.status;
        let initial_retry = self.on_retry.clone();
        let observer = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            PostPageStateConnectionObserver::new(status, initial_retry, cx)
        });
        let on_retry = self.on_retry.clone();
        observer.update(cx, |observer, _| {
            observer.status = status;
            observer.on_retry = on_retry;
        });
        self.state_body(window, cx)
    }
}

#[derive(IntoElement)]
pub struct PostPageErrorUi {
    id: ElementId,
    pub text: String,
    pub on_retry: Rc<dyn Fn(&mut App)>,
}

impl PostPageErrorUi {
    pub fn new(id: impl Into<ElementId>, text: impl Into<String>, on_retry: impl Fn(&mut App) + 'static) -> Self {
        Self { id: id.into(), text: text.into(), on_retry: Rc::new(on_retry) }
    }
}

impl RenderOnce for PostPageErrorUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let secondary = theme.text.secondary;
        let primary = theme.primary._100;
        let on_retry = self.on_retry;
        vstack(12.0)
            .children(IconUi::sized("wifi.exclamationmark", px(36.0), secondary.opacity(0.6), false))
            .child(MainText::new(self.text).px(14.0).tac(TextAlignT::Center).color(secondary).max_w(px(280.0)))
            .child(
                plain(self.id)
                    .on_press(move |_: &mut Window, cx: &mut App| on_retry(cx))
                    .padding_edges(EdgeSet::HORIZONTAL, 18.0)
                    .padding_edges(EdgeSet::VERTICAL, 9.0)
                    .bg(primary)
                    .capsule()
                    .child(
                        MainText::new(tr("common_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.content_on(primary)),
                    ),
            )
            .padding_edges(EdgeSet::HORIZONTAL, 24.0)
            .padding_edges(EdgeSet::VERTICAL, 56.0)
            .frame_max_width(Alignment::Center)
    }
}

#[derive(IntoElement)]
pub struct PostPageRefreshErrorUi {
    id: ElementId,
    pub text: String,
    pub on_retry: Rc<dyn Fn(&mut App)>,
}

impl PostPageRefreshErrorUi {
    pub fn new(id: impl Into<ElementId>, text: impl Into<String>, on_retry: impl Fn(&mut App) + 'static) -> Self {
        Self { id: id.into(), text: text.into(), on_retry: Rc::new(on_retry) }
    }
}

impl RenderOnce for PostPageRefreshErrorUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let text_primary = theme.text.primary;
        let border = theme.border._200;
        let material = alpha(theme.bg._100, Material::UltraThin.painted_opacity());
        let on_retry = self.on_retry;
        plain(self.id)
            .on_press(move |_: &mut Window, cx: &mut App| on_retry(cx))
            .relative()
            .padding_edges(EdgeSet::HORIZONTAL, 12.0)
            .padding_edges(EdgeSet::VERTICAL, 8.0)
            .bg(material)
            .capsule()
            .child(
                hstack(7.0)
                    .children(IconUi::sized("arrow.clockwise", px(12.0), text_primary, false))
                    .child(
                        MainText::new(self.text)
                            .px(12.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(text_primary)
                            .number_of_lines(1),
                    ),
            )
            .child(centred_stroke(border, 0.5, Shape::Capsule))
    }
}

#[derive(IntoElement)]
pub struct PostLayoutToggleUi {
    id: ElementId,
    pub uses_grid: Binding<bool>,
}

impl PostLayoutToggleUi {
    pub fn new(id: impl Into<ElementId>, uses_grid: Binding<bool>) -> Self {
        Self { id: id.into(), uses_grid }
    }

    fn toggle(id: ElementId, icon: &str, label: String, active: bool, action: ButtonAction, cx: &mut App) -> AnyElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let icon_color = if active { theme.primary._100 } else { theme.text.secondary };
        let side = AnimatedTabsMetrics::BAR_HEIGHT;
        let button = div()
            .id(id)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(side)
            .circle()
            .bg(theme.bg._200)
            .children(IconUi::sized(icon, px(14.0), icon_color, false))
            .child(centred_stroke(theme.border._200, 0.5, Shape::Circle));
        let label = SharedString::from(label);
        button_chrome(button, false, Some(action))
            .tooltip_label(label)
            .into_any_element()
    }
}

impl RenderOnce for PostLayoutToggleUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let uses_grid = self.uses_grid.get(cx);
        let to_grid = self.uses_grid.clone();
        let to_list = self.uses_grid;
        let grid = Self::toggle(
            child_id(&self.id, "grid"),
            "square.grid.3x3",
            tr("post_layout_grid"),
            uses_grid,
            Rc::new(move |window: &mut Window, cx: &mut App| to_grid.set(true, window, cx)),
            cx,
        );
        let list = Self::toggle(
            child_id(&self.id, "list"),
            "list.bullet",
            tr("chat_search_show_as_list"),
            !uses_grid,
            Rc::new(move |window: &mut Window, cx: &mut App| to_list.set(false, window, cx)),
            cx,
        );
        hstack(6.0).w(AnimatedTabsMetrics::BAR_HEIGHT * 2.0 + px(6.0)).child(grid).child(list)
    }
}

#[derive(IntoElement)]
pub struct PostGridTextUi {
    pub post: Rc<GetPostFeedResponse>,
}

impl PostGridTextUi {
    const PADDING: f32 = 10.0;
    const TRACK_BADGE_TOP_INSET: f32 = 34.0;

    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into() }
    }

    fn tile(post: &GetPostFeedResponse, tile_size: Size<Pixels>, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let text_secondary = theme.text.secondary;
        let background = theme.bg._200;
        let delta = ThemeStore::shared(cx).read(cx).text_delta();
        let title_preview = RichTextMarkup::preview_text(&post.title);
        let title = trimming_whitespaces_and_newlines(&title_preview);
        let content_preview = RichTextMarkup::preview_text(&post.content);
        let content = trimming_whitespaces_and_newlines(&content_preview);
        let primary = if title.is_empty() { content } else { title };
        let shows_content = !content.is_empty() && content != primary;
        let top_inset = if post.track_id.is_some() { Self::TRACK_BADGE_TOP_INSET } else { Self::PADDING };
        let inset = Self::PADDING * 2.0;
        let height_share: f32 = if shows_content { 0.5 } else { 1.0 };
        let fitted = GridTileTextFit::font_size(
            primary,
            grid_title_font_size(if primary.is_empty() { None } else { Some(primary) }) + px(delta),
            px(10.0 + delta),
            FontWeight::SEMIBOLD,
            size(
                tile_size.width - px(inset),
                (tile_size.height - px(top_inset) - px(Self::PADDING)) * height_share,
            ),
            window,
        );
        let title_px = f32::from(fitted) - delta;

        vstack(7.0)
            .when(!primary.is_empty(), |stack| {
                stack.child(
                    MainText::new(primary.to_owned())
                        .px(title_px)
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(if shows_content { 3 } else { 5 })
                        .w_full()
                        .flex_shrink_0(),
                )
            })
            .when(shows_content, |stack| {
                stack.child(
                    MainText::new(content.to_owned())
                        .px((title_px - 3.0).max(10.0))
                        .color(text_secondary)
                        .number_of_lines(4)
                        .w_full()
                        .min_h_0()
                        .overflow_hidden(),
                )
            })
            .padding_edges(EdgeSet::HORIZONTAL, Self::PADDING)
            .padding_edges(EdgeSet::TOP, top_inset)
            .padding_edges(EdgeSet::BOTTOM, Self::PADDING)
            .size_full()
            .bg(background)
            .into_any_element()
    }
}

impl RenderOnce for PostGridTextUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let post = self.post;
        container_query(move |tile_size, window, cx| Self::tile(&post, tile_size, window, cx))
    }
}

#[derive(IntoElement)]
pub struct PostHoldContextMenu {
    id: ElementId,
    pub post: Rc<GetPostFeedResponse>,
    pub treat_as_own_post: bool,
    pub on_open: Rc<dyn Fn(&mut Window, &mut App)>,
    pub content: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>,
}

impl PostHoldContextMenu {
    pub fn new<E: IntoElement>(
        id: impl Into<ElementId>,
        post: impl Into<Rc<GetPostFeedResponse>>,
        on_open: impl Fn(&mut Window, &mut App) + 'static,
        content: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            post: post.into(),
            treat_as_own_post: false,
            on_open: Rc::new(on_open),
            content: Rc::new(move |window: &mut Window, cx: &mut App| content(window, cx).into_any_element()),
        }
    }

    pub fn treat_as_own_post(mut self, treat_as_own_post: bool) -> Self {
        self.treat_as_own_post = treat_as_own_post;
        self
    }

    fn actions(post: &GetPostFeedResponse, mine: bool, acts_as_author: bool) -> Vec<HoldContextMenuAction> {
        let mut values = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if !mine {
            values.push(
                HoldContextMenuAction::new(
                    "repost",
                    if post.is_reposted { tr("repost_delete_action") } else { tr("repost_action") },
                )
                .system_image("arrow.2.squarepath"),
            );
        }
        if acts_as_author {
            values.push(HoldContextMenuAction::new("stats", tr("post_stats_menu_item")).system_image("chart.bar"));
            values.push(HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"));
            values.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        }
        if !mine {
            values.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        values
    }

    fn handle_action(
        id: &str,
        post: &GetPostFeedResponse,
        nav_store: Option<&Entity<NavigationStore>>,
        on_open: &Rc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match id {
            "open" => on_open(window, cx),
            "copy" => {
                if let Some(url) = SharePayload::Post(post.clone()).share_url() {
                    UIPasteboard::general().set_string(url.absolute_string(), cx);
                }
                Notifier::copied(cx);
            }
            "share" => {
                let post = post.clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
            }
            "repost" => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_repost(post, None, cx));
            }
            "stats" => {
                if let Some(nav_store) = nav_store {
                    let route = RootRoute::PostStats { post_id: post.rise_view_target_post_id() };
                    nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            }
            "edit" => {
                if let Some(nav_store) = nav_store {
                    let route = RootRoute::EditPost { post: post.clone() };
                    nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            }
            "delete" => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_own_post(post, None, cx));
            }
            "report" => {
                if let Some(post_id) = int_from_string(&post.id) {
                    ReportSheetStore::shared(cx)
                        .update(cx, |sheet, cx| sheet.present(ReportTarget::Post(post_id), None, None, cx));
                }
            }
            _ => {}
        }
    }
}

impl RenderOnce for PostHoldContextMenu {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mine = self.treat_as_own_post || PostMediaCachePolicy::is_own_post(&self.post, cx);
        let acts_as_author = mine || AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author();
        let nav_store = env::<NavigationStoreKey>(cx);

        let actions_post = self.post.clone();
        let action_post = self.post;
        let action_open = self.on_open.clone();
        let on_open = self.on_open;
        HoldContextMenuContainer::new(
            self.id,
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| Self::actions(&actions_post, mine, acts_as_author)),
            self.content,
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
            Self::handle_action(&id, &action_post, nav_store.as_ref(), &action_open, window, cx);
        }))
        .on_tap(Rc::new(move |point: Point<Pixels>, window: &mut Window, cx: &mut App| {
            if BubbleTapRouter::dispatch(point, None, window, cx) {
                return;
            }
            on_open(window, cx);
        }))
    }
}

#[derive(Clone)]
pub struct PostPaginationTrigger {
    id: ElementId,
    pub index: i64,
    pub item_count: i64,
    pub has_more: bool,
    pub is_loading_more: bool,
    pub threshold: i64,
    pub load_more: Rc<dyn Fn(&mut App)>,
}

impl PostPaginationTrigger {
    fn load_if_near_tail(&self, cx: &mut App) {
        if !self.has_more || self.is_loading_more {
            return;
        }
        if self.index < (self.item_count - self.threshold).max(0) {
            return;
        }
        (self.load_more)(cx);
    }
}

impl ViewModifier for PostPaginationTrigger {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        let after_count = self.clone();
        on_change(
            child_id(&self.id, "item-count"),
            self.item_count,
            false,
            window,
            cx,
            move |_, _, _, cx| after_count.load_if_near_tail(cx),
        );
        let after_more = self.clone();
        on_change(
            child_id(&self.id, "has-more"),
            self.has_more,
            false,
            window,
            cx,
            move |_, _, _, cx| after_more.load_if_near_tail(cx),
        );
        let watches_tail = self.has_more && self.index >= (self.item_count - self.threshold).max(0);
        let tail_connected = watches_tail && RiseTransportStatus::shown(cx).is_connected();
        let after_connect = self.clone();
        on_change(
            child_id(&self.id, "tail-connected"),
            tail_connected,
            false,
            window,
            cx,
            move |_, is_connected, _, cx| {
                if *is_connected {
                    after_connect.load_if_near_tail(cx);
                }
            },
        );
        let appear_id = child_id(&self.id, "appear");
        content
            .on_appear(appear_id, move |_: &mut Window, cx: &mut App| self.load_if_near_tail(cx))
            .into_any_element()
    }
}

pub trait PostPaginationTriggerModifiers: IntoElement + Sized {
    fn post_pagination_trigger(
        self,
        id: impl Into<ElementId>,
        index: i64,
        item_count: i64,
        has_more: bool,
        is_loading_more: bool,
        load_more: impl Fn(&mut App) + 'static,
    ) -> ModifiedContent<PostPaginationTrigger> {
        self.post_pagination_trigger_threshold(id, index, item_count, has_more, is_loading_more, 6, load_more)
    }

    #[allow(clippy::too_many_arguments)]
    fn post_pagination_trigger_threshold(
        self,
        id: impl Into<ElementId>,
        index: i64,
        item_count: i64,
        has_more: bool,
        is_loading_more: bool,
        threshold: i64,
        load_more: impl Fn(&mut App) + 'static,
    ) -> ModifiedContent<PostPaginationTrigger> {
        self.modifier(PostPaginationTrigger {
            id: id.into(),
            index,
            item_count,
            has_more,
            is_loading_more,
            threshold,
            load_more: Rc::new(load_more),
        })
    }
}

impl<T: IntoElement> PostPaginationTriggerModifiers for T {}

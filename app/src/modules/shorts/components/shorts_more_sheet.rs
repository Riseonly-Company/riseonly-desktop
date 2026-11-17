use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Empty, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Render, RenderOnce, Rgba,
    StatefulInteractiveElement, Styled, Subscription, Window, div, hsla, px,
};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{env, reduce_motion};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::shapes::SwiftUiShapes;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::{SharePayload, ShareSheetStore};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::collaborators::post_collaborator_menu::PostCollaboratorMenu;
use crate::modules::post::components::download::post_download_store::PostDownloadSheetStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::{
    PostInteractionsStore, PostInteractionsStoreShareReceiptChannel,
};
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::stores::shorts::shorts_interactions::shorts_interactions::ShortsInteractionsStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

/// more sheet store, hosted at the root like other global sheets
pub struct ShortsMoreSheetStore {
    is_presented: bool,
    post_id: Option<String>,
    reads_engine_first: bool,
    media_index: i64,
    row_taps_open_at: Duration,
}

impl ShortsMoreSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_presented: false,
            post_id: None,
            reads_engine_first: false,
            media_index: 0,
            row_taps_open_at: Duration::ZERO,
        }
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn media_index(&self) -> i64 {
        self.media_index
    }

    pub fn post_id(&self) -> Option<&str> {
        self.post_id.as_deref()
    }

    pub fn reads_engine_first(&self) -> bool {
        self.reads_engine_first
    }

    pub fn accepts_row_taps(&self) -> bool {
        RiseClock::monotonic() >= self.row_taps_open_at
    }

    /// reads_engine_first says which copy the cell draws,
    /// the riffs tab and the viewer use different ones that can lag on counters
    pub fn present(
        &mut self,
        post: &GetPostFeedResponse,
        reads_engine_first: bool,
        media_index: i64,
        cx: &mut Context<Self>,
    ) {
        if post.is_temp {
            return;
        }
        self.post_id = Some(post.id.clone());
        self.media_index = media_index;
        self.reads_engine_first = reads_engine_first;
        if !self.is_presented {
            self.row_taps_open_at = if reduce_motion(cx) {
                Duration::ZERO
            } else {
                RiseClock::monotonic() + Duration::from_secs_f64(BottomSheetMetrics::PRESENT_DURATION)
            };
        }
        self.is_presented = true;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.is_presented = false;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.is_presented = false;
        self.post_id = None;
        self.reads_engine_first = false;
        cx.notify();
    }

    /// re-read at action time, a stale row would toggle the opposite of what the reader sees
    pub fn live_post(post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        let short = ShortsServicesStore::shared(cx).update(cx, |service, cx| service.resolved_short(post_id, cx));
        let engine = PostServicesStore::shared(cx).update(cx, |service, cx| service.resolved_post(post_id, cx));
        if Self::shared(cx).read(cx).reads_engine_first {
            engine.or(short)
        } else {
            short.or(engine)
        }
    }
}

pub struct ShortsMoreSheetHost {
    store: Entity<ShortsMoreSheetStore>,
    sheet_fraction: f32,
    _observations: [Subscription; 3],
}

impl ShortsMoreSheetHost {
    const MINIMUM_FRACTION: f32 = 0.3;
    const FRACTION_TOLERANCE: f32 = 0.005;

    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ShortsMoreSheetStore::shared(cx);
        let shorts_service = ShortsServicesStore::shared(cx);
        let post_service = PostServicesStore::shared(cx);
        let observations = [
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&shorts_service, |host, _, cx| host.notify_if_presented(cx)),
            cx.observe(&post_service, |host, _, cx| host.notify_if_presented(cx)),
        ];
        Self {
            store,
            sheet_fraction: Self::MINIMUM_FRACTION,
            _observations: observations,
        }
    }

    fn notify_if_presented(&mut self, cx: &mut Context<Self>) {
        if self.store.read(cx).is_presented() {
            cx.notify();
        }
    }

    fn adopt_content_height(&mut self, height: f32, window: &Window, cx: &mut Context<Self>) {
        let target = BottomSheetMeasuredFraction::value(height, Self::MINIMUM_FRACTION, window);
        if (target - self.sheet_fraction).abs() <= Self::FRACTION_TOLERANCE {
            return;
        }
        self.sheet_fraction = target;
        cx.notify();
    }
}

impl Render for ShortsMoreSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let navigation_store = env::<NavigationStoreKey>(cx);
        let read = self.store.clone();
        let write = self.store.clone();
        let content_store = self.store.clone();
        let host = cx.entity().downgrade();
        let visible = Binding::new(
            move |cx| read.read(cx).is_presented(),
            move |value, _, cx| {
                if !value {
                    write.update(cx, |store, cx| store.dismiss(cx));
                }
            },
        );
        div().size_full().child(
            BottomSheetUi::new(
                "shorts_more_sheet",
                visible,
                move |_, cx| {
                    let post_id = content_store.read(cx).post_id().map(str::to_owned);
                    let post = post_id.and_then(|post_id| ShortsMoreSheetStore::live_post(&post_id, cx));
                    let Some(post) = post else {
                        return Empty.into_any_element();
                    };
                    let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
                    let measured_host = host.clone();
                    let dismiss_store = content_store.clone();
                    ShortsMoreSheetContent::new(
                        post,
                        theme,
                        navigation_store.clone(),
                        move |height, window, cx| {
                            measured_host
                                .update(cx, |host, cx| host.adopt_content_height(height, window, cx))
                                .ok();
                        },
                        move |_, cx| dismiss_store.update(cx, |store, cx| store.dismiss(cx)),
                    )
                    .into_any_element()
                },
                |_, _| Empty,
            )
            .snap_fractions(vec![self.sheet_fraction])
            .content_scrollable(true)
            .sheet_background(Some(background))
            .footer_background(Some(background)),
        )
    }
}

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;
type MeasuredHeight = Rc<dyn Fn(f32, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ShortsMoreSheetContent {
    post: Rc<GetPostFeedResponse>,
    theme: AppTheme,
    nav_store: Option<Entity<NavigationStore>>,
    on_measured_height: MeasuredHeight,
    dismiss: SheetAction,
    allows_actions: bool,
}

impl ShortsMoreSheetContent {
    const BOTTOM_ALLOWANCE: f32 = 28.0;
    const COLLABORATOR_ICON_BACKGROUND: &'static str = "#7E57F7";

    pub fn new(
        post: GetPostFeedResponse,
        theme: AppTheme,
        nav_store: Option<Entity<NavigationStore>>,
        on_measured_height: impl Fn(f32, &mut Window, &mut App) + 'static,
        dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            post: Rc::new(post),
            theme,
            nav_store,
            on_measured_height: Rc::new(on_measured_height),
            dismiss: Rc::new(dismiss),
            allows_actions: true,
        }
    }

    pub fn allows_actions(mut self, allows_actions: bool) -> Self {
        self.allows_actions = allows_actions;
        self
    }

    fn performer(&self) -> ShortsMoreSheetPerformer {
        ShortsMoreSheetPerformer {
            post: self.post.clone(),
            allows_actions: self.allows_actions,
            dismiss: self.dismiss.clone(),
        }
    }

    fn row(
        id: impl Into<gpui::SharedString>,
        icon: impl Into<gpui::SharedString>,
        icon_background: Option<Hsla>,
        text: String,
        destructive: bool,
        callback: impl Fn(&mut Window, &mut App) + 'static,
    ) -> GroupedBtn {
        GroupedBtn::new(id, text)
            .icon(icon)
            .icon_background(icon_background)
            .destructive(destructive)
            .show_chevron(false)
            .callback(callback)
    }

    /// same coauthor items as the post menu so reel and feed offer the same set
    fn collaborator_items(&self, acts_as_author: bool, cx: &mut App) -> Vec<GroupedBtn> {
        PostCollaboratorMenu::items(&self.post, acts_as_author, cx)
            .into_iter()
            .map(|item| {
                let performer = self.performer();
                let destructive = item.destructive;
                let action = item.action;
                let icon_background = (!destructive).then(|| color(Self::COLLABORATOR_ICON_BACKGROUND));
                Self::row(item.id, item.system_image, icon_background, item.title, destructive, move |window, cx| {
                    let action = action.clone();
                    performer.perform(
                        destructive.then_some(NotificationKind::Warning),
                        move |_, cx| action(cx),
                        window,
                        cx,
                    );
                })
            })
            .collect()
    }

    fn counters_strip(&self, viewer: Option<&str>, cx: &mut App) -> AnyElement {
        let views = PostServicesStore::shared(cx).update(cx, |service, cx| service.post_view_count(&self.post, cx));
        let mut strip = div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .px(px(16.0))
            .w_full()
            .child(self.counter_pill("eye.fill", views, tr("post_stats_views")));
        if self.post.shows_count(|visibility| visibility.likes, viewer) {
            strip = strip.child(self.counter_pill("heart.fill", self.post.likes_count, tr("post_stats_likes")));
        }
        if self.post.shows_count(|visibility| visibility.comments, viewer) {
            strip = strip.child(self.counter_pill(
                "bubble.right.fill",
                self.post.comments_count,
                tr("post_stats_comments"),
            ));
        }
        strip.into_any_element()
    }

    fn counter_pill(&self, system_image: &'static str, value: i64, label: String) -> impl IntoElement {
        let theme = &self.theme;
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .py(px(10.0))
            .rounded_rect(14.0)
            .bg(theme.bg._200)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .children(IconUi::sized(system_image, px(12.0), theme.text.secondary, true))
                    .child(
                        MainText::new(RiseFormat::compact_count(value))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    ),
            )
            .child(
                MainText::new(label)
                    .px(11.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            )
    }

    fn primary_items(&self, cx: &mut App) -> Vec<GroupedBtn> {
        let mut items = self.base_primary_items();
        if self.post.allows_download_entry(cx) {
            let index = items.iter().position(|item| item.id.as_ref() == "share").map_or(items.len(), |index| index + 1);
            items.insert(index, self.download_item());
        }
        items
    }

    fn download_item(&self) -> GroupedBtn {
        Self::row(
            "download",
            "arrow.down.to.line",
            Some(SettingsIconPalette::memory_videos()),
            tr("post_download_action"),
            false,
            {
                let performer = self.performer();
                move |window, cx| {
                    let media_index = usize::try_from(ShortsMoreSheetStore::shared(cx).read(cx).media_index()).unwrap_or(0);
                    performer.perform_with_live_post(
                        None,
                        move |post, _, cx| {
                            PostDownloadSheetStore::shared(cx)
                                .update(cx, |store, cx| store.start(post, media_index, false, 0.0, cx));
                        },
                        window,
                        cx,
                    );
                }
            },
        )
    }

    fn base_primary_items(&self) -> Vec<GroupedBtn> {
        let post = &self.post;
        vec![
            Self::row("copy_link", "link", Some(color("#2F8FE5")), tr("ctx_copy_link"), false, {
                let performer = self.performer();
                move |window, cx| {
                    performer.perform_with_live_post(
                        None,
                        |post, _, cx| {
                            let link = SharePayload::Post(post.clone())
                                .share_url()
                                .map(|url| url.absolute_string().to_owned())
                                .unwrap_or_default();
                            UIPasteboard::general().set_string(&link, cx);
                            let _ = notification(NotificationKind::Success);
                            PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.record_share(&post, PostInteractionsStoreShareReceiptChannel::LinkCopy, cx);
                            });
                        },
                        window,
                        cx,
                    );
                }
            }),
            Self::row("share", "square.and.arrow.up", Some(color("#34A853")), tr("ctx_share"), false, {
                let performer = self.performer();
                move |window, cx| {
                    performer.perform_with_live_post(
                        None,
                        |post, _, cx| {
                            ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
                        },
                        window,
                        cx,
                    );
                }
            }),
            Self::row(
                "repost",
                "arrow.2.squarepath",
                Some(color(Self::COLLABORATOR_ICON_BACKGROUND)),
                tr(if post.is_reposted { "repost_delete_action" } else { "repost_action" }),
                false,
                {
                    let performer = self.performer();
                    move |window, cx| {
                        performer.perform_with_live_post(
                            None,
                            |post, _, cx| {
                                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                                    store.toggle_repost(
                                        &post,
                                        Some(Box::new(|message: String, cx: &mut App| {
                                            Notifier::send(
                                                message,
                                                NotifierSendOptions {
                                                    r#type: NotifierType::Error,
                                                    position: NotifierPosition::Bottom,
                                                    ..Default::default()
                                                },
                                                cx,
                                            );
                                        })),
                                        cx,
                                    );
                                });
                            },
                            window,
                            cx,
                        );
                    }
                },
            ),
            Self::row(
                "favorite",
                if post.is_favorited { "bookmark.fill" } else { "bookmark" },
                Some(Self::system_orange(&self.theme)),
                tr(if post.is_favorited { "favourite_remove" } else { "favourite_add" }),
                false,
                {
                    let performer = self.performer();
                    move |window, cx| {
                        performer.perform_with_live_post(
                            None,
                            |post, _, cx| {
                                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&post, cx));
                            },
                            window,
                            cx,
                        );
                    }
                },
            ),
            Self::row(
                "like",
                if post.is_liked { "heart.fill" } else { "heart" },
                Some(self.theme.primary._100),
                tr(if post.is_liked { "ctx_unlike" } else { "shorts_more_like" }),
                false,
                {
                    let performer = self.performer();
                    move |window, cx| {
                        performer.perform_with_live_post(
                            None,
                            |post, _, cx| {
                                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&post, cx));
                            },
                            window,
                            cx,
                        );
                    }
                },
            ),
        ]
    }

    fn owner_items(&self) -> Vec<GroupedBtn> {
        let stats_background: Hsla = Rgba { r: 0.2, g: 0.5, b: 0.9, a: 1.0 }.into();
        vec![
            Self::row("stats", "chart.bar", Some(stats_background), tr("post_stats_menu_item"), false, {
                let performer = self.performer();
                let nav_store = self.nav_store.clone();
                move |window, cx| {
                    let nav_store = nav_store.clone();
                    performer.perform_with_live_post(
                        None,
                        move |post, _, cx| {
                            if let Some(nav_store) = nav_store {
                                let route = RootRoute::PostStats { post_id: post.rise_view_target_post_id() };
                                nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                            }
                        },
                        window,
                        cx,
                    );
                }
            }),
            Self::row("edit", "pencil", Some(Self::neutral_icon_background()), tr("ctx_edit"), false, {
                let performer = self.performer();
                let nav_store = self.nav_store.clone();
                move |window, cx| {
                    let nav_store = nav_store.clone();
                    performer.perform_with_live_post(
                        None,
                        move |post, _, cx| {
                            if let Some(nav_store) = nav_store {
                                let route = RootRoute::EditPost { post };
                                nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                            }
                        },
                        window,
                        cx,
                    );
                }
            }),
            Self::row("delete", "trash", None, tr("ctx_delete"), true, {
                let performer = self.performer();
                move |window, cx| {
                    performer.perform_with_live_post(
                        Some(NotificationKind::Warning),
                        |post, _, cx| {
                            PostInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.delete_own_post(&post, None, cx));
                        },
                        window,
                        cx,
                    );
                }
            }),
        ]
    }

    fn audience_items(&self) -> Vec<GroupedBtn> {
        vec![
            Self::row(
                "not_interested",
                "hand.thumbsdown.fill",
                Some(Self::neutral_icon_background()),
                tr("shorts_more_not_interested"),
                false,
                {
                    let performer = self.performer();
                    move |window, cx| {
                        performer.perform_with_live_post(
                            None,
                            |post, _, cx| {
                                ShortsInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.not_interested(&post, cx));
                            },
                            window,
                            cx,
                        );
                    }
                },
            ),
            Self::row("report", "exclamationmark.bubble.fill", None, tr("shorts_more_report"), true, {
                let performer = self.performer();
                move |window, cx| {
                    performer.perform_with_live_post(
                        Some(NotificationKind::Warning),
                        |post, _, cx| {
                            let Some(post_id) = int_from_string(&post.rise_view_target_post_id()) else {
                                return;
                            };
                            ReportSheetStore::shared(cx)
                                .update(cx, |sheet, cx| sheet.present(ReportTarget::Post(post_id), None, None, cx));
                        },
                        window,
                        cx,
                    );
                }
            }),
        ]
    }

    fn neutral_icon_background() -> Hsla {
        hsla(0.0, 0.0, 0.42, 1.0)
    }

    fn system_orange(theme: &AppTheme) -> Hsla {
        UIColor::system_orange().resolved_color(theme.appearance)
    }
}

impl RenderOnce for ShortsMoreSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let is_mine = RiseIdentity::is_own(
            &RiseIdentity::author(Some(self.post.author_id.as_str()), None, self.post.author.id.as_deref()),
            viewer.as_deref(),
            None,
        );
        let acts_as_author = is_mine || AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author();
        let role_items = if is_mine {
            self.owner_items()
        } else if acts_as_author {
            let mut items = self.owner_items();
            items.extend(self.audience_items());
            items
        } else {
            self.audience_items()
        };
        let collaborators = self.collaborator_items(acts_as_author, cx);
        let counters = self.counters_strip(viewer.as_deref(), cx);
        let on_measured_height = self.on_measured_height.clone();

        let mut column = div()
            .relative()
            .flex_none()
            .w_full()
            .pt(px(8.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(counters)
            .child(GroupedBtns::new("shorts_more_primary", self.primary_items(cx)))
            .child(GroupedBtns::new("shorts_more_role", role_items));
        if !collaborators.is_empty() {
            column = column.child(GroupedBtns::new("shorts_more_collaborators", collaborators));
        }
        column = column.child(div().flex_none().h(px(Self::BOTTOM_ALLOWANCE))).child(measure(
            "shorts_more_content_height",
            move |bounds, window, cx| on_measured_height(f32::from(bounds.size.height), window, cx),
        ));

        div().id("shorts_more_scroll").size_full().overflow_y_scroll().child(column)
    }
}

/// closure captured half of the sheet, every row runs through it
struct ShortsMoreSheetPerformer {
    post: Rc<GetPostFeedResponse>,
    allows_actions: bool,
    dismiss: SheetAction,
}

impl ShortsMoreSheetPerformer {
    fn accepts_actions(&self, cx: &mut App) -> bool {
        self.allows_actions && ShortsMoreSheetStore::shared(cx).read(cx).accepts_row_taps()
    }

    /// dismiss first and act once the frame commits it,
    /// so a sheet the action opens never races the closing one
    fn perform(
        &self,
        haptic: Option<NotificationKind>,
        action: impl FnOnce(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.accepts_actions(cx) {
            return;
        }
        (self.dismiss)(window, cx);
        if let Some(haptic) = haptic {
            let _ = notification(haptic);
        }
        window.defer(cx, action);
    }

    fn perform_with_live_post(
        &self,
        haptic: Option<NotificationKind>,
        action: impl FnOnce(GetPostFeedResponse, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut App,
    ) {
        let fallback = self.post.clone();
        self.perform(
            haptic,
            move |window, cx| {
                let post =
                    ShortsMoreSheetStore::live_post(&fallback.id, cx).unwrap_or_else(|| fallback.as_ref().clone());
                action(post, window, cx);
            },
            window,
            cx,
        );
    }
}

use std::rc::Rc;

use gpui::{
    App, Context, Div, ElementId, Empty, Entity, FontWeight, Pixels, SharedString, Size, Subscription, Window,
    transparent_black,
};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::bottom_sheet_interaction_policy::BottomSheetInteractionStyle;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderToolbarSlot};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;

use crate::app::router::navigation_store::{BotMiniAppPresentation, NavigationStore};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::bot::components::bot_ui_components::BotStateView;
use crate::modules::bot::pages::bot_mini_app_screen::BotMiniAppWebView;
use crate::modules::bot::shared::bot_ui_validation::{BotMiniAppInitData, BotMiniAppURLPolicy};
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::shared::chat_profile_report_hooks::ChatProfileReportTarget;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

fn sheet_id(presentation_id: Uuid, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("bot-mini-app-{}-{name}", presentation_id.uuid_string())))
}

pub enum BotMiniAppSheetPolicy {}

impl BotMiniAppSheetPolicy {
    pub const SNAP_DURATION: f64 = 0.45;

    pub fn detent_fractions(size: Size<Pixels>) -> Vec<f32> {
        let width = f32::from(size.width);
        let height = f32::from(size.height);
        if width <= 0.0 || height <= 0.0 {
            return vec![0.7512, 1.0];
        }
        if height < width || width >= 600.0 {
            return vec![1.0];
        }

        let factor = if width <= 320.0 { 0.15 } else { 0.2488 };
        let top_inset = (width.max(height) * factor).floor();
        let collapsed = ((height - top_inset) / height).clamp(0.5, 0.9);
        vec![collapsed, 1.0]
    }
}

/// view for the mini app presentation, the retained one has to outlive the store value so the
/// sheet can finish dismissing
pub struct BotMiniAppPresentationHost {
    navigation_store: Entity<NavigationStore>,
    retained_presentation: Option<BotMiniAppPresentation>,
    is_presented: bool,
    sheet: Option<Entity<BotMiniAppSheet>>,
    _navigation_subscription: Subscription,
}

impl BotMiniAppPresentationHost {
    pub fn new(navigation_store: Entity<NavigationStore>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&navigation_store, |host, store, cx| {
            let presentation = store.read(cx).bot_mini_app_presentation().cloned();
            host.reconcile_presentation(presentation, cx);
        });
        let appear_store = navigation_store.clone();
        let host = cx.weak_entity();
        cx.defer(move |cx| {
            let presentation = appear_store.read(cx).bot_mini_app_presentation().cloned();
            let _ = host.update(cx, |host, cx| host.reconcile_presentation(presentation, cx));
        });
        Self {
            navigation_store,
            retained_presentation: None,
            is_presented: false,
            sheet: None,
            _navigation_subscription: subscription,
        }
    }

    fn presentation_binding(&self, presentation: &BotMiniAppPresentation, cx: &mut Context<Self>) -> Binding<bool> {
        let id = presentation.id;
        Binding::entity(
            cx.entity(),
            move |host: &Self| {
                host.is_presented
                    && host.retained_presentation.as_ref().is_some_and(|retained| retained.id == id)
            },
            move |host, open, cx| {
                host.is_presented = open;
                if !open {
                    host.navigation_store.update(cx, |store, cx| store.dismiss_bot_mini_app(id, cx));
                }
                cx.notify();
            },
        )
    }

    fn present(&mut self, presentation: BotMiniAppPresentation, cx: &mut Context<Self>) {
        let is_presented = self.presentation_binding(&presentation, cx);
        let send_store = self.navigation_store.clone();
        let send_presentation = presentation.clone();
        let on_send_data: Rc<dyn Fn(String, &mut App)> = Rc::new(move |data: String, cx: &mut App| {
            if send_presentation.is_web_link || !send_store.read(cx).is_bot_mini_app_presentation_active(send_presentation.id)
            {
                return;
            }
            BotMiniAppDataSender::send(
                send_presentation.chat_id.as_deref(),
                &data,
                &BotMiniAppInitData::extract(&send_presentation.url),
                cx,
            );
        });
        let id = presentation.id;
        let host = cx.weak_entity();
        let on_did_dismiss: SheetAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = host.update(cx, |host, cx| host.complete_dismissal(id, cx));
        });
        let sheet_presentation = presentation.clone();
        self.sheet =
            Some(cx.new(|_| BotMiniAppSheet::new(is_presented, sheet_presentation, on_send_data, on_did_dismiss)));
        self.retained_presentation = Some(presentation);
        self.is_presented = true;
        cx.notify();
    }

    fn reconcile_presentation(&mut self, presentation: Option<BotMiniAppPresentation>, cx: &mut Context<Self>) {
        let retained_id = self.retained_presentation.as_ref().map(|retained| retained.id);
        if retained_id == presentation.as_ref().map(|presentation| presentation.id) {
            if presentation.is_some() && self.retained_presentation != presentation {
                self.retained_presentation = presentation;
                cx.notify();
            }
            return;
        }

        if self.retained_presentation.is_some() {
            if self.is_presented {
                self.is_presented = false;
                cx.notify();
            }
            return;
        }

        if let Some(presentation) = presentation {
            self.present(presentation, cx);
        }
    }

    fn complete_dismissal(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.retained_presentation.as_ref().map(|retained| retained.id) != Some(id) {
            return;
        }
        self.retained_presentation = None;
        self.sheet = None;
        self.is_presented = false;
        cx.notify();

        let Some(latest_id) =
            self.navigation_store.read(cx).bot_mini_app_presentation().map(|presentation| presentation.id)
        else {
            return;
        };
        let host = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = host.update(cx, |host, cx| {
                if host.retained_presentation.is_some() {
                    return;
                }
                let latest = host.navigation_store.read(cx).bot_mini_app_presentation().cloned();
                if let Some(latest) = latest.filter(|latest| latest.id == latest_id) {
                    host.present(latest, cx);
                }
            });
        });
    }
}

impl Render for BotMiniAppPresentationHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.sheet.clone())
    }
}

/// one entity per presentation
pub struct BotMiniAppSheet {
    is_presented: Binding<bool>,
    presentation: BotMiniAppPresentation,
    on_send_data: Rc<dyn Fn(String, &mut App)>,
    on_did_dismiss: SheetAction,
    selected_snap_fraction: f32,
    progress: f64,
    can_go_back: bool,
    web_error: Option<String>,
    reload_trigger: i64,
    go_back_trigger: i64,
    geometry_size: Option<Size<Pixels>>,
    is_actions_menu_presented: bool,
}

impl BotMiniAppSheet {
    fn new(
        is_presented: Binding<bool>,
        presentation: BotMiniAppPresentation,
        on_send_data: Rc<dyn Fn(String, &mut App)>,
        on_did_dismiss: SheetAction,
    ) -> Self {
        Self {
            is_presented,
            presentation,
            on_send_data,
            on_did_dismiss,
            selected_snap_fraction: 1.0,
            progress: 0.0,
            can_go_back: false,
            web_error: None,
            reload_trigger: 0,
            go_back_trigger: 0,
            geometry_size: None,
            is_actions_menu_presented: false,
        }
    }

    fn set_geometry_size(&mut self, new_size: Size<Pixels>, cx: &mut Context<Self>) {
        match self.geometry_size.replace(new_size) {
            Some(old_size) if old_size != new_size => self.update_detent_for_layout_change(old_size, new_size),
            Some(_) => {}
            None => {
                self.selected_snap_fraction =
                    BotMiniAppSheetPolicy::detent_fractions(new_size).first().copied().unwrap_or(1.0);
            }
        }
        cx.notify();
    }

    fn update_detent_for_layout_change(&mut self, old_size: Size<Pixels>, new_size: Size<Pixels>) {
        let old_detents = BotMiniAppSheetPolicy::detent_fractions(old_size);
        let new_detents = BotMiniAppSheetPolicy::detent_fractions(new_size);
        let (Some(old_collapsed), Some(new_collapsed)) = (old_detents.first(), new_detents.first()) else {
            return;
        };

        if new_detents.len() == 1 || (self.selected_snap_fraction - old_collapsed).abs() < 0.02 {
            self.selected_snap_fraction = *new_collapsed;
        } else {
            self.selected_snap_fraction = new_detents.last().copied().unwrap_or(*new_collapsed);
        }
    }

    fn handle_back_or_close(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        if this.read(cx).can_go_back {
            this.update(cx, |sheet, cx| {
                sheet.go_back_trigger += 1;
                cx.notify();
            });
        } else {
            Self::request_dismiss(this, window, cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.progress = 0.0;
        self.web_error = None;
        self.reload_trigger += 1;
        cx.notify();
    }

    fn expand(&mut self, cx: &mut Context<Self>) {
        self.selected_snap_fraction = 1.0;
        cx.notify();
    }

    fn request_dismiss(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let is_presented = this.read(cx).is_presented.clone();
        is_presented.set(false, window, cx);
    }

    /// the page belongs to a bot so its reported as that bot
    /// the bot chat stands in when the button carried no bot id, nobody reports their own bot
    fn report_target(presentation: &BotMiniAppPresentation, cx: &mut App) -> Option<ReportTarget> {
        if let Some(bot_id) = presentation.bot_id.as_deref().filter(|bot_id| !bot_id.is_empty()) {
            if BotServicesStore::bot(bot_id, cx).is_some_and(|bot| bot.is_owned_by_requester) {
                return None;
            }
            return Some(ReportTarget::Bot(bot_id.to_owned()));
        }
        let chat_id = presentation.chat_id.as_deref().filter(|chat_id| !chat_id.is_empty())?;
        let chats = ChatServicesStore::shared(cx);
        let chat = chats.read(cx).chat_info(chat_id, cx)?;
        if !chat.is_bot_chat() {
            return None;
        }
        ChatProfileReportTarget::resolve(&chat, cx)
    }

    fn actions_glyph(theme: &AppTheme) -> AnyElement {
        PageHeaderToolbarSlot::new(div().children(IconUi::sized("ellipsis", px(17.0), theme.text.primary, false)))
            .into_any_element()
    }

    fn actions_menu(
        this: &Entity<Self>,
        presentation_id: Uuid,
        url: Url,
        report_target: Option<ReportTarget>,
        theme: &AppTheme,
    ) -> AnyElement {
        let open_this = this.clone();
        let has_report_target = report_target.is_some();
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| {
            let mut actions = vec![
                HoldContextMenuAction::new("reload", tr("bot_mini_app_reload")).system_image("arrow.clockwise"),
                HoldContextMenuAction::new("open_in_browser", tr("bot_mini_app_open_in_browser")).system_image("safari"),
            ];
            if has_report_target {
                actions.push(
                    HoldContextMenuAction::new("report", tr("ctx_report"))
                        .system_image("exclamationmark.bubble")
                        .destructive(true),
                );
            }
            actions.push(HoldContextMenuAction::new("close", tr("close")).system_image("xmark").destructive(true));
            actions
        });
        let action_this = this.clone();
        let on_action: HoldContextMenuPresentationOnAction =
            Rc::new(move |id: String, window: &mut Window, cx: &mut App| match id.as_str() {
                "reload" => action_this.update(cx, |sheet, cx| sheet.reload(cx)),
                "report" => {
                    if let Some(report_target) = report_target.clone() {
                        ReportSheetStore::shared(cx)
                            .update(cx, |sheet, cx| sheet.present(report_target, None, None, cx));
                    }
                }
                "open_in_browser" => {
                    let _ = rise_platform::open_url::open_url(&url, cx);
                }
                "close" => Self::request_dismiss(&action_this, window, cx),
                _ => {}
            });
        let source_theme = theme.clone();
        let source: HoldContextMenuTapSource =
            Rc::new(move |_: &mut Window, _: &mut App| Self::actions_glyph(&source_theme));
        let is_presented = Binding::entity(
            this.clone(),
            |sheet: &Self| sheet.is_actions_menu_presented,
            |sheet, value, cx| {
                sheet.is_actions_menu_presented = value;
                cx.notify();
            },
        );
        plain(sheet_id(presentation_id, "menu"))
            .on_press(move |_: &mut Window, cx: &mut App| {
                open_this.update(cx, |sheet, cx| {
                    sheet.is_actions_menu_presented = true;
                    cx.notify();
                });
            })
            .child(Self::actions_glyph(theme))
            .liquid_chrome_capsule_with(0.12, true, Some(0.84), false)
            .tap_context_menu(
                sheet_id(presentation_id, "menu-anchor"),
                is_presented,
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            )
            .into_any_element()
    }

    fn header(
        this: &Entity<Self>,
        report_target: Option<ReportTarget>,
        theme: &AppTheme,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let sheet = this.read(cx);
        let is_web_link = sheet.presentation.is_web_link;
        let presentation_id = sheet.presentation.id;
        let can_go_back = sheet.can_go_back;
        let progress = sheet.progress;
        let title = sheet.presentation.title.clone();
        let url = sheet.presentation.url.clone();
        let back_this = this.clone();

        div()
            .relative()
            .w_full()
            .px(px(12.0))
            .py(px(5.0))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .px(px(76.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(1.0))
                    .child(
                        MainText::new(title)
                            .px(16.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .when(!is_web_link, |titles| {
                        titles.child(
                            SecondaryText::new(tr("bot_mini_app_subtitle"))
                                .px(11.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.secondary),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        PageHeaderIconButton::new(
                            sheet_id(presentation_id, "back"),
                            if can_go_back { "chevron.backward" } else { "xmark" },
                            move |window: &mut Window, cx: &mut App| {
                                Self::handle_back_or_close(&back_this, window, cx);
                            },
                        )
                        .font_size(if can_go_back { 16.0 } else { 15.0 }),
                    )
                    .child(Self::actions_menu(this, presentation_id, url, report_target, theme)),
            )
            .when(progress > 0.0 && progress < 1.0, |header| {
                header.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(2.0))
                        .child(progress_view_value(progress).tint(theme.primary._100)),
                )
            })
    }

    fn page(this: &Entity<Self>, theme: &AppTheme, cx: &App) -> Div {
        let sheet = this.read(cx);
        let presentation_id = sheet.presentation.id;
        let page = div().relative().flex_1().min_h_0().w_full().bg(theme.bg._100);
        if !BotMiniAppURLPolicy::allows(&sheet.presentation.url, sheet.presentation.allows_development_http) {
            return page.child(
                BotStateView::new(
                    sheet_id(presentation_id, "blocked"),
                    "lock.trianglebadge.exclamationmark.fill",
                    tr("bot_mini_app_url_blocked"),
                )
                .message(tr("bot_mini_app_url_policy_hint")),
            );
        }

        let close_this = this.clone();
        let expand_this = this.clone();
        let send_this = this.clone();
        let retry_this = this.clone();
        let web_view = BotMiniAppWebView::new(
            sheet_id(presentation_id, "web-view"),
            sheet.presentation.url.clone(),
            sheet.presentation.allows_development_http,
            Binding::entity(this.clone(), |sheet: &Self| sheet.progress, |sheet, value, cx| {
                sheet.progress = value;
                cx.notify();
            }),
            Binding::entity(this.clone(), |sheet: &Self| sheet.can_go_back, |sheet, value, cx| {
                sheet.can_go_back = value;
                cx.notify();
            }),
            Binding::entity(this.clone(), |sheet: &Self| sheet.web_error.clone(), |sheet, value, cx| {
                sheet.web_error = value;
                cx.notify();
            }),
            sheet.reload_trigger,
            sheet.go_back_trigger,
            move |window: &mut Window, cx: &mut App| Self::request_dismiss(&close_this, window, cx),
            move |data: String, window: &mut Window, cx: &mut App| {
                let on_send_data = send_this.read(cx).on_send_data.clone();
                on_send_data(data, cx);
                Self::request_dismiss(&send_this, window, cx);
            },
        )
        .on_expand(move |_: &mut Window, cx: &mut App| {
            expand_this.update(cx, |sheet, cx| sheet.expand(cx));
        })
        .is_covered(sheet.is_actions_menu_presented || sheet.progress < 1.0);

        page.child(web_view)
            .when(sheet.progress < 1.0 && sheet.web_error.is_none(), |page| {
                page.child(
                    div().absolute().inset_0().child(BotMiniAppWebSkeleton),
                )
            })
            .when_some(sheet.web_error.clone(), |page, web_error| {
                page.child(
                    div().absolute().inset_0().bg(theme.bg._100).child(
                        BotStateView::new(
                            sheet_id(presentation_id, "web-error"),
                            "wifi.exclamationmark",
                            tr("bot_mini_app_load_error"),
                        )
                        .message(web_error)
                        .action_title(tr("bot_retry"))
                        .action(move |_: &mut Window, cx: &mut App| {
                            retry_this.update(cx, |sheet, cx| sheet.reload(cx));
                        }),
                    ),
                )
            })
    }

    /// desktop window has no sensor housing or home indicator so safe insets are zero and no
    /// padding is added
    fn sheet_content(this: &Entity<Self>, cx: &mut App) -> Div {
        let theme = rise_ui::theme(cx).clone();
        let presentation = this.read(cx).presentation.clone();
        let report_target = Self::report_target(&presentation, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::header(this, report_target, &theme, cx))
            .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.35)))
            .child(Self::page(this, &theme, cx))
    }
}

impl Render for BotMiniAppSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let presentation_id = self.presentation.id;
        let background = rise_ui::theme(cx).bg._100;
        let measure_this = this.clone();
        let geometry = measure(sheet_id(presentation_id, "geometry"), move |bounds, _, cx| {
            measure_this.update(cx, |sheet, cx| sheet.set_geometry_size(bounds.size, cx));
        })
        .absolute()
        .size_full();

        let sheet = self.geometry_size.map(|size| {
            let content_this = this.clone();
            let on_did_dismiss = self.on_did_dismiss.clone();
            BottomSheetUi::new(
                sheet_id(presentation_id, "sheet"),
                self.is_presented.clone(),
                move |_: &mut Window, cx: &mut App| Self::sheet_content(&content_this, cx),
                |_: &mut Window, _: &mut App| Empty,
            )
            .snap_fractions(BotMiniAppSheetPolicy::detent_fractions(size))
            .selected_snap_fraction(Some(Binding::entity(
                this.clone(),
                |sheet: &Self| sheet.selected_snap_fraction,
                |sheet, value, cx| {
                    sheet.selected_snap_fraction = value;
                    cx.notify();
                },
            )))
            .snap_animation_duration(BotMiniAppSheetPolicy::SNAP_DURATION)
            .interaction_style(BottomSheetInteractionStyle::TelegramWebApp)
            .content_is_edge_to_edge(true)
            .experimental_bottom_sheet(true)
            .experimental_uses_liquid_glass_surface(false)
            .full_page_allows_swipe_dismiss(true)
            .dismisses_on_backdrop_tap(true)
            .full_page_shows_drag_indicator(true)
            .full_page_hides_footer(true)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .sheet_background(Some(background))
            .footer_background(Some(transparent_black()))
            .on_did_dismiss(move |window, cx| on_did_dismiss(window, cx))
        });

        div().relative().size_full().child(geometry).children(sheet)
    }
}

#[derive(IntoElement)]
struct BotMiniAppWebSkeleton;

impl RenderOnce for BotMiniAppWebSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().size_full().child(SkeletonBlock::new(None::<f32>, None::<f32>, 0.0).skeleton_shimmer(None))
    }
}

pub enum BotMiniAppDataSender {}

impl BotMiniAppDataSender {
    pub fn send(chat_id: Option<&str>, data: &str, init_data: &str, cx: &mut App) {
        let Some(chat_id) = chat_id.filter(|chat_id| !chat_id.is_empty() && !init_data.is_empty()) else {
            Notifier::send(
                tr("bot_callback_failed"),
                NotifierSendOptions {
                    content: Some(tr("bot_button_not_supported")),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..Default::default()
                },
                cx,
            );
            return;
        };

        let temp_id = MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.send_web_app_data(
                chat_id,
                data,
                init_data,
                None,
                Some(Box::new(|result: anyhow::Result<()>, cx: &mut App| {
                    if let Err(error) = result {
                        Notifier::send(
                            tr("bot_callback_failed"),
                            NotifierSendOptions {
                                content: Some(error.to_string()),
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Bottom,
                                ..Default::default()
                            },
                            cx,
                        );
                    }
                })),
                cx,
            )
        });
        if temp_id.is_none() {
            Notifier::send(
                tr("bot_callback_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..Default::default()
                },
                cx,
            );
        }
    }
}

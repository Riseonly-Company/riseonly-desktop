use std::collections::HashMap;
use std::rc::Rc;

use gpui::{AnyWindowHandle, App, Context, ElementId, Entity, FontWeight, Hsla, Pixels, Point, SharedString, Subscription, Task, Window, white};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{characters, drop_first_characters, trimming_whitespaces_and_newlines};
use rise_i18n::{FormatArg, tr, tr_args, tr_plural};
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::system_share::ShareItem;
use rise_theme::AppTheme;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::switch_ui::SwitchRowUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::swiftui::tooltip_label::{TooltipLabelModifiers, TooltipLabelPageMotion};
use rise_ui::layer_keys::LayerKeysExt;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::input_ui::InputUi;
use crate::core::ui::qr_code::rise_qr_code_card::RiseQRCodeSubject;
use crate::core::ui::qr_code::rise_qr_code_modal::RiseQRCodePresentation;
use crate::core::widgets::notifier::notifier_host::NotifierHost;
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_avatar_palette::AvatarPaletteChat;
use crate::modules::chat::stores::chats::chat_actions::chat_types::InviteLink;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const SUBTITLE_SEPARATOR: &str = " \u{b7} ";
const VALUE_NAME: &str = "name";
const VALUE_LIMIT: &str = "limit";

fn page_id(chat_id: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("chat-settings-links-{chat_id}-{name}")))
}

fn load_invite_links(chat_id: &str, cx: &mut App) {
    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_invite_links(chat_id, cx));
}

fn copy(value: &str, cx: &mut App) {
    UIPasteboard::general().set_string(value, cx);
    let _ = ImpactFeedbackGenerator::new(ImpactStyle::Light).impact_occurred();
}

fn invite_share_url(link: &InviteLink) -> String {
    RiseonlyPublicLink::join_url(&link.link, AppEnvironment::current())
        .map(|url| url.absolute_string().to_owned())
        .unwrap_or_else(|| link.link.clone())
}

fn open_qr_code(link: &InviteLink, chat_id: &str, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
    let Some(url) = RiseonlyPublicLink::join_url(&link.link, AppEnvironment::current()) else {
        return;
    };
    let chat = ChatServicesStore::shared(cx).update(cx, |services, cx| services.chat_info(chat_id, cx));
    let subject = RiseQRCodeSubject::new(
        url,
        None,
        chat.as_ref().map(|chat| chat.title.clone()).unwrap_or_default(),
        chat.as_ref().and_then(AvatarPalette::resolved_logo_url_for_chat),
        Some(chat_id.to_owned()),
    );
    RiseQRCodePresentation::present(subject, nav_store.cloned(), cx);
}

fn link_subtitle(link: &InviteLink) -> String {
    let mut parts: Vec<String> = Vec::new();
    if link.used_count > 0 {
        parts.push(tr_plural("chat_links_joined_count", link.used_count));
    } else {
        parts.push(tr("chat_links_no_joins"));
    }
    if link.is_active()
        && let Some(limit) = link.usage_limit
        && limit > 0
    {
        parts.push(tr_args("chat_links_remaining", &[FormatArg::Int((limit - link.used_count).max(0))]));
    }
    if link.creates_join_request {
        parts.push(tr("chat_links_requires_approval"));
    }
    if link.is_revoked {
        parts.push(tr("chat_links_revoked"));
    } else if link.is_exhausted() {
        parts.push(tr("chat_links_expired"));
    }
    parts.join(SUBTITLE_SEPARATOR)
}

fn link_actions(link: &InviteLink) -> Vec<HoldContextMenuAction> {
    let mut actions = vec![HoldContextMenuAction::new("copy", tr("chat_links_copy")).system_image("doc.on.doc")];
    if link.is_active() {
        actions.push(HoldContextMenuAction::new("qr", tr("qr_code_title")).system_image("qrcode"));
    }
    if !link.is_revoked {
        actions.push(HoldContextMenuAction::new("revoke", tr("chat_links_revoke")).system_image("trash").destructive(true));
    }
    actions
}

fn card_button_label(icon: &'static str, title: String, theme: &AppTheme) -> Div {
    hstack(7.0)
        .w_full()
        .min_h(px(44.0))
        .justify_center()
        .bg(theme.primary._100)
        .capsule()
        .children(IconUi::sized(icon, px(15.0), theme.content_on(theme.primary._100), false))
        .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100)))
}

fn link_row(link: &InviteLink, theme: &AppTheme) -> Div {
    let tile: Hsla = if link.is_active() { ChatTileColor::blue() } else { ChatTileColor::gray() };
    let title = match link.name.as_deref() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => invite_share_url(link),
    };
    hstack(12.0)
        .w_full()
        .px(px(16.0))
        .py(px(11.0))
        .child(
            div()
                .size(px(29.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_rect(8.0)
                .bg(tile)
                .children(IconUi::sized("link", px(14.0), white(), false)),
        )
        .child(
            vstack(2.0)
                .flex_1()
                .min_w_0()
                .align(HorizontalAlignment::Leading)
                .child(MainText::new(title).px(15.0).color(theme.text.primary).number_of_lines(1))
                .child(MainText::new(link_subtitle(link)).px(12.0).color(theme.text.secondary).number_of_lines(1)),
        )
        .child(spacer_min_w(4.0).flex_none())
}

struct ChatSettingsLinksState {
    show_create: bool,
    _subscriptions: [Subscription; 3],
}

impl ChatSettingsLinksState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let actions = ChatActionsStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self {
            show_create: false,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
struct ChatSettingsLinksBody {
    chat_id: String,
    state: Entity<ChatSettingsLinksState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ChatSettingsLinksBody {
    fn load_error_state(chat_id: &str, cx: &mut App) -> Div {
        let theme = rise_ui::theme(cx).clone();
        let failure = ChatActionsStore::shared(cx).read(cx).get_invite_links_sai.failure();
        let text = failure.as_ref().and_then(CanonErrors::text_for).unwrap_or_else(|| tr("error_fetching_data"));
        let retry_chat_id = chat_id.to_owned();
        let retry_title = tr("common_retry");
        vstack(14.0)
            .size_full()
            .justify_center()
            .px(px(24.0))
            .child(MainText::new(text).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
            .child(
                LiquidGlassButton::new(page_id(chat_id, "retry"), LiquidGlassButtonShape::Capsule)
                    .accessibility_label(Some(retry_title.clone()))
                    .on_press(move |_, cx| load_invite_links(&retry_chat_id, cx))
                    .child(MainText::new(retry_title).px(14.0).color(theme.text.primary)),
            )
    }

    fn primary_card(&self, link: &InviteLink, theme: &AppTheme) -> Div {
        let url = invite_share_url(link);
        let copy_url = url.clone();
        let share_url = url.clone();
        let qr_link = link.clone();
        let qr_chat_id = self.chat_id.clone();
        let qr_store = self.nav_store.clone();
        vstack(12.0)
            .w_full()
            .px(px(12.0))
            .child(
                hstack(0.0)
                    .w_full()
                    .px(px(4.0))
                    .bg(theme.bg._200)
                    .rounded_rect(14.0)
                    .child(div().size(px(44.0)).flex_none())
                    .child(
                        div().flex_1().min_w_0().py(px(13.0)).child(
                            MainText::new(url)
                                .px(15.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.primary)
                                .number_of_lines(1),
                        ),
                    )
                    .child(
                        div().id(page_id(&self.chat_id, "qr-label")).tooltip_label(tr("qr_code_title")).flex_none().child(
                            plain(page_id(&self.chat_id, "qr"))
                                .size(px(44.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .on_press(move |_, cx| open_qr_code(&qr_link, &qr_chat_id, qr_store.as_ref(), cx))
                                .children(IconUi::sized("qrcode", px(20.0), theme.primary._100, false)),
                        ),
                    ),
            )
            .child(
                hstack(10.0)
                    .w_full()
                    .child(
                        div().flex_1().min_w_0().child(
                            plain(page_id(&self.chat_id, "copy"))
                                .w_full()
                                .on_press(move |_, cx| copy(&copy_url, cx))
                                .child(card_button_label("doc.on.doc", tr("chat_links_copy"), theme)),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            plain(page_id(&self.chat_id, "share"))
                                .w_full()
                                .on_press(move |_, cx| {
                                    SystemShare::present(vec![ShareItem::Link(share_url.clone())], None, cx);
                                })
                                .child(card_button_label("square.and.arrow.up", tr("share_action"), theme)),
                        ),
                    ),
            )
            .when(link.used_count > 0, |card| {
                card.child(
                    MainText::new(tr_plural("chat_links_joined_count", link.used_count))
                        .px(13.0)
                        .color(theme.primary._100),
                )
            })
    }

    fn link_action(link: &InviteLink, chat_id: &str, id: &str, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        match id {
            "copy" => copy(&invite_share_url(link), cx),
            "qr" => open_qr_code(link, chat_id, nav_store, cx),
            "revoke" => {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.revoke_invite_link(chat_id, &link.id, cx));
            }
            _ => {}
        }
    }

    fn links_list(&self, links: Vec<InviteLink>, title: Option<String>, list_name: &str, theme: &AppTheme) -> Div {
        let count = links.len();
        let rows = links.into_iter().enumerate().map(|(index, link)| {
            let actions = link_actions(&link);
            let content_link = link.clone();
            let content_theme = theme.clone();
            let action_link = link.clone();
            let action_chat_id = self.chat_id.clone();
            let action_store = self.nav_store.clone();
            let tap_url = invite_share_url(&link);
            div()
                .w_full()
                .child(
                    HoldContextMenuContainer::new(
                        page_id(&self.chat_id, &format!("{list_name}-hold-{}", link.id)),
                        HoldContextMenuAlignment::Leading,
                        Rc::new(move |_: &App| actions.clone()),
                        Rc::new(move |_: &mut Window, _: &mut App| {
                            link_row(&content_link, &content_theme).into_any_element()
                        }),
                    )
                    .show_emoji_list(false)
                    .lifted_background(Some(theme.bg._200))
                    .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                        Self::link_action(&action_link, &action_chat_id, &id, action_store.as_ref(), cx);
                    }))
                    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| copy(&tap_url, cx))),
                )
                .when(index + 1 < count, |row| {
                    row.child(
                        div().w_full().pl(px(52.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))),
                    )
                })
        });
        vstack(8.0)
            .w_full()
            .px(px(12.0))
            .align(HorizontalAlignment::Leading)
            .when_some(title, |list, title| {
                list.child(div().px(px(16.0)).child(MainText::new(title).px(12.0).color(theme.text.secondary)))
            })
            .child(vstack(0.0).w_full().bg(theme.bg._200).rounded_rect(20.0).overflow_hidden().children(rows))
    }
}

impl RenderOnce for ChatSettingsLinksBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id.clone();
        let all_links = ChatServicesStore::shared(cx).update(cx, |services, cx| services.invite_links(&chat_id, cx));
        let primary = ChatServicesStore::shared(cx).update(cx, |services, cx| services.primary_invite_link(&chat_id, cx));
        let slice = ChatActionsStore::shared(cx).read(cx).get_invite_links_sai.clone();
        let need_pending = all_links.is_empty();
        let error_chat_id = chat_id.clone();
        let this_state = self.state.clone();

        SaiDataRender::slice(&*slice, move |_, cx| {
            let theme = rise_ui::theme(cx).clone();
            let primary_id = primary.as_ref().map(|link| link.id.clone());
            let additional: Vec<InviteLink> = all_links
                .iter()
                .filter(|link| link.is_active() && Some(&link.id) != primary_id.as_ref())
                .cloned()
                .collect();
            let inactive: Vec<InviteLink> = all_links.iter().filter(|link| !link.is_active()).cloned().collect();
            let create_state = this_state.clone();
            let refresh_chat_id = chat_id.clone();
            scroll_view(page_id(&chat_id, "scroll"))
                .size_full()
                .scroll_disabled(HoldContextMenuPresenter::has_active_request(cx))
                .list_shell_scroll_under_chrome(cx)
                .bg(theme.bg._100)
                .refreshable(move |_, cx| {
                    load_invite_links(&refresh_chat_id, cx);
                    Task::ready(())
                })
                .child(
                    vstack(18.0)
                        .w_full()
                        .pt(px(12.0))
                        .children(primary.as_ref().map(|link| self.primary_card(link, &theme)))
                        .child(
                            GroupedBtns::new(
                                page_id(&chat_id, "create-group"),
                                vec![
                                    GroupedBtn::new("create", tr("chat_links_create"))
                                        .icon("plus")
                                        .icon_background(ChatTileColor::blue())
                                        .show_chevron(false)
                                        .callback(move |_, cx| {
                                            create_state.update(cx, |state, cx| {
                                                state.show_create = true;
                                                cx.notify();
                                            });
                                        }),
                                ],
                            )
                            .title(tr("chat_links_additional_section")),
                        )
                        .when(!additional.is_empty(), |stack| {
                            stack.child(self.links_list(additional, None, "additional", &theme))
                        })
                        .when(!inactive.is_empty(), |stack| {
                            stack.child(self.links_list(
                                inactive,
                                Some(tr("chat_links_inactive_section")),
                                "inactive",
                                &theme,
                            ))
                        })
                        .child(div().w_full().min_h(px(80.0))),
                )
        })
        .id(page_id(&error_chat_id, "render"))
        .is_empty(false)
        .need_pending(need_pending)
        .sai_data_render_error(move |_, cx| Self::load_error_state(&error_chat_id, cx))
    }
}

#[derive(IntoElement)]
pub struct ChatSettingsLinks {
    pub chat_id: Option<String>,
}

impl ChatSettingsLinks {
    pub fn new(chat_id: Option<String>) -> Self {
        Self { chat_id }
    }

    fn resolved_chat_id(&self) -> String {
        self.chat_id.clone().unwrap_or_default()
    }
}

impl RenderOnce for ChatSettingsLinks {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.resolved_chat_id();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(page_id(&chat_id, "state"), cx, |_, cx| ChatSettingsLinksState::new(cx));

        let task_chat_id = chat_id.clone();
        use_task(page_id(&chat_id, "task"), (), window, cx, async move |cx| {
            let settle = cx.update(|_, cx| TooltipLabelPageMotion::remaining(cx)).unwrap_or_default();
            if !settle.is_zero() {
                cx.background_executor().timer(settle).await;
            }
            let _ = cx.update(|_, cx| load_invite_links(&task_chat_id, cx));
        });

        let back_store = nav_store.clone();
        let keys_chat_id = chat_id.clone();
        let sheet_chat_id = chat_id.clone();
        let shell = SettingsWrapperUi::new(tr("chat_settings_links_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(ChatSettingsLinksBody { chat_id: chat_id.clone(), state: state.clone(), nav_store });

        div()
            .relative()
            .size_full()
            .child(PageRefreshKeys::new(
                page_id(&chat_id, "keys"),
                move |_, cx| load_invite_links(&keys_chat_id, cx),
                shell,
            ))
            .child(
                NativeBottomSheetPresentationHost::new(
                    page_id(&chat_id, "create-sheet"),
                    Binding::entity(
                        state,
                        |state: &ChatSettingsLinksState| state.show_create,
                        |state, value, _| state.show_create = value,
                    ),
                    move |_, _| CreateInviteLinkSheet { chat_id: sheet_chat_id.clone() },
                )
                .presentation_detents([PresentationDetent::Large]),
            )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExpiryPreset {
    None,
    Hour,
    Day,
    Week,
}

impl ExpiryPreset {
    const ALL_CASES: [Self; 4] = [Self::None, Self::Hour, Self::Day, Self::Week];

    fn title_key(self) -> &'static str {
        match self {
            Self::None => "chat_links_expiry_none",
            Self::Hour => "chat_links_expiry_hour",
            Self::Day => "chat_links_expiry_day",
            Self::Week => "chat_links_expiry_week",
        }
    }

    fn seconds(self) -> Option<f64> {
        match self {
            Self::None => None,
            Self::Hour => Some(3600.0),
            Self::Day => Some(86_400.0),
            Self::Week => Some(604_800.0),
        }
    }
}

/// state of the sheet, gone when the sheet stops rendering so a new presentation starts clean
struct CreateInviteLinkSheetState {
    values: HashMap<String, String>,
    expiry: ExpiryPreset,
    requires_approval: bool,
    _subscription: Subscription,
}

impl CreateInviteLinkSheetState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = ChatActionsStore::shared(cx);
        Self {
            values: HashMap::new(),
            expiry: ExpiryPreset::None,
            requires_approval: false,
            _subscription: cx.observe(&actions, |_, _, cx| cx.notify()),
        }
    }

    fn value(&self, name: &str) -> &str {
        self.values.get(name).map(String::as_str).unwrap_or_default()
    }

    /// digits past int range parse as nil and would silently make an unlimited link
    fn usage_limit_error(&self) -> Option<String> {
        let digits = self.value(VALUE_LIMIT);
        if digits.is_empty() {
            return None;
        }
        let ceiling = RiseLimits::service_value("max_invite_link_usage_limit");
        match digits.parse::<i64>() {
            Ok(limit) if limit <= ceiling => None,
            _ => Some(tr_plural("people_picker_limit_reached", ceiling)),
        }
    }
}

#[derive(IntoElement)]
struct CreateInviteLinkSheet {
    chat_id: String,
}

impl CreateInviteLinkSheet {
    fn sentence_case(text: &str) -> String {
        match characters(text).next() {
            Some(first) => first.to_uppercase() + drop_first_characters(text, 1),
            None => text.to_owned(),
        }
    }

    fn close(dismiss_action: &DismissAction, window_handle: AnyWindowHandle, cx: &mut App) {
        let dismiss_action = dismiss_action.clone();
        cx.defer(move |cx| {
            window_handle.update(cx, |_, window, cx| dismiss_action.call_as_function(window, cx)).ok();
        });
    }

    fn create(
        chat_id: &str,
        state: &Entity<CreateInviteLinkSheetState>,
        dismiss_action: &DismissAction,
        window: &Window,
        cx: &mut App,
    ) {
        let sheet = state.read(cx);
        if sheet.usage_limit_error().is_some() {
            return;
        }
        let name = trimming_whitespaces_and_newlines(sheet.value(VALUE_NAME)).to_owned();
        let limit = sheet.value(VALUE_LIMIT).parse::<i64>().ok();
        let expires_at_ms =
            sheet.expiry.seconds().map(|seconds| ((Date::now().time_interval_since_1970() + seconds) * 1000.0) as i64);
        let requires_approval = sheet.requires_approval;
        let window_handle = window.window_handle();
        let revoked_dismiss = dismiss_action.clone();
        let result_dismiss = dismiss_action.clone();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.create_invite_link(
                chat_id,
                (!name.is_empty()).then_some(name.as_str()),
                expires_at_ms,
                limit,
                requires_approval,
                None,
                Some(Box::new(move |cx: &mut App| Self::close(&revoked_dismiss, window_handle, cx))),
                None,
                Box::new(move |ok: bool, cx: &mut App| {
                    if ok {
                        Self::close(&result_dismiss, window_handle, cx);
                    }
                }),
                cx,
            );
        });
    }

    fn toolbar_button(id: ElementId, title: String, color: Hsla, weight: FontWeight) -> PlainButton {
        plain(id).child(MainText::new(title).px(17.0).font_weight(weight).color(color).number_of_lines(1))
    }
}

impl RenderOnce for CreateInviteLinkSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let chat_id = self.chat_id;
        let state = window.use_keyed_state(page_id(&chat_id, "create-state"), cx, |_, cx| CreateInviteLinkSheetState::new(cx));
        let notifier = window.use_keyed_state(page_id(&chat_id, "create-notifier"), cx, |_, cx| NotifierHost::new(cx));
        let dismiss_action = dismiss(cx);
        let is_pending = ChatActionsStore::shared(cx).read(cx).create_invite_link_sai.is_pending();

        let sheet = state.read(cx);
        let values = sheet.values.clone();
        let selected_index = ExpiryPreset::ALL_CASES.iter().position(|preset| *preset == sheet.expiry).unwrap_or(0);
        let usage_limit_error = sheet.usage_limit_error();
        let done_enabled = usage_limit_error.is_none();

        let set_name = state.clone();
        let set_limit = state.clone();
        let set_expiry = state.clone();
        let cancel_dismiss = dismiss_action.clone();
        let done_state = state.clone();
        let done_chat_id = chat_id.clone();
        let (submit_state, submit_chat_id, submit_dismiss) = (state.clone(), chat_id.clone(), dismiss_action.clone());

        let toolbar_trailing = if is_pending {
            progress_view().into_any_element()
        } else {
            Self::toolbar_button(
                page_id(&chat_id, "create-done"),
                tr("common_done"),
                if done_enabled { theme.primary._100 } else { theme.text.secondary },
                FontWeight::SEMIBOLD,
            )
            .disabled(!done_enabled)
            .on_press(move |window, cx| Self::create(&done_chat_id, &done_state, &dismiss_action, window, cx))
            .into_any_element()
        };

        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .form_primary(done_enabled && !is_pending, move |window, cx| {
                Self::create(&submit_chat_id, &submit_state, &submit_dismiss, window, cx)
            })
            .child(
                vstack(0.0)
                    .size_full()
                    .child(
                        hstack(8.0)
                            .w_full()
                            .h(px(52.0))
                            .px(px(16.0))
                            .child(
                                div().flex_1().min_w_0().flex().justify_start().child(
                                    Self::toolbar_button(
                                        page_id(&chat_id, "create-cancel"),
                                        tr("common_cancel"),
                                        theme.primary._100,
                                        FontWeight::NORMAL,
                                    )
                                    .on_press(move |window, cx| cancel_dismiss.call_as_function(window, cx)),
                                ),
                            )
                            .child(
                                MainText::new(tr("chat_links_create"))
                                    .px(17.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.primary)
                                    .number_of_lines(1),
                            )
                            .child(div().flex_1().min_w_0().flex().justify_end().child(toolbar_trailing)),
                    )
                    .child(
                        div().flex_1().min_h_0().w_full().child(
                            scroll_view("create-invite-link-scroll").size_full().bg(theme.bg._100).child(
                                vstack(22.0)
                                    .w_full()
                                    .p(px(16.0))
                                    .align(HorizontalAlignment::Leading)
                                    .child(
                                        InputUi::new("create-invite-link-name")
                                            .values(values.clone())
                                            .name(VALUE_NAME)
                                            .placeholder(tr("chat_links_name_placeholder"))
                                            .rule(RiseFieldRule::new("chat_create_invite_link", "/name"))
                                            .set_value(move |_, value, _, cx| {
                                                set_name.update(cx, |sheet, cx| {
                                                    sheet.values.insert(VALUE_NAME.to_owned(), value.to_owned());
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        vstack(8.0)
                                            .w_full()
                                            .align(HorizontalAlignment::Leading)
                                            .child(
                                                MainText::new(tr("chat_links_expiry_title"))
                                                    .px(13.0)
                                                    .color(theme.text.secondary),
                                            )
                                            .child(SegmentedPicker::new(
                                                "create-invite-link-expiry",
                                                ExpiryPreset::ALL_CASES
                                                    .iter()
                                                    .map(|preset| SharedString::from(tr(preset.title_key())))
                                                    .collect(),
                                                selected_index,
                                                move |index, _, cx| {
                                                    set_expiry.update(cx, |sheet, cx| {
                                                        sheet.expiry = ExpiryPreset::ALL_CASES
                                                            .get(index)
                                                            .copied()
                                                            .unwrap_or(ExpiryPreset::None);
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .child(
                                        InputUi::new("create-invite-link-limit")
                                            .values(values)
                                            .error_message(usage_limit_error)
                                            .name(VALUE_LIMIT)
                                            .placeholder(tr("chat_links_usage_limit_placeholder"))
                                            .format(|raw, _| raw.chars().filter(|character| character.is_numeric()).collect())
                                            .set_value(move |_, value, _, cx| {
                                                set_limit.update(cx, |sheet, cx| {
                                                    sheet.values.insert(VALUE_LIMIT.to_owned(), value.to_owned());
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(div().w_full().child(div().mx(px(-16.0)).child(SwitchRowUi::new(
                                        "create-invite-link-approval",
                                        Self::sentence_case(&tr("chat_links_requires_approval")),
                                        Binding::entity(
                                            state.clone(),
                                            |sheet: &CreateInviteLinkSheetState| sheet.requires_approval,
                                            |sheet, value, _| sheet.requires_approval = value,
                                        ),
                                    )))),
                            ),
                        ),
                    ),
            )
            .child(div().absolute().inset_0().child(notifier))
    }
}

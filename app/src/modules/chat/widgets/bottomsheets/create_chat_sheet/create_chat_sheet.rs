use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Axis, Context, ElementId, Empty, Entity, FontWeight, Hsla, Window, transparent_black};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::tr;
use rise_theme::{AppTheme, color};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetNavigationStateKey, BottomSheetScreenItem,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::layer_keys::{declare_form_primary, form_scope};
use rise_ui::keyboard_dismiss::{KeyboardDismiss, SheetFocusKey};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::Shape;
use rise_ui::swiftui::stacks::{hstack, spacer_min_h, vstack};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_pages_ui::BottomSheetPagesUi;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::select_media_ui::SelectMediaUi;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::widgets::sheets::people_picker_sheet::{
    PeoplePickerHostContext, PeoplePickerMode, PeoplePickerSheet, PeoplePickerStore,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::components::bot_ui_components::{BotOneTimeTokenSheet, BotSettingsFieldRow, BotToggleRow};
use crate::modules::bot::shared::bot_ui_validation::{BotCreateFlowState, BotFieldRules, BotTagAvailabilityState};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::{BotActionsStore, BotCreateResult};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotModel;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::{ChatActionTextCallback, ChatActionsStore};
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::{
    ChatFieldRules, ChatPrivacyType, CreateChatSheetState,
};

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

const ORANGE_HEX: &str = "#FF9500";
const INDIGO_HEX: &str = "#5856D6";
const BLUE_HEX: &str = "#007AFF";
const GREEN_HEX: &str = "#34C759";
const TAG_STATUS_ICON_SIDE: f32 = 17.0;

/// keeps only what an ascii on-screen keyboard could have produced
/// a hardware keyboard can type any layout
fn ascii_capable(raw: &str) -> String {
    raw.chars().filter(char::is_ascii).collect()
}

fn navigate(
    navigation: &Option<Entity<BottomSheetNavigationState>>,
    screen_id: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(navigation) = navigation {
        BottomSheetNavigationState::navigate(navigation, screen_id, window, cx);
    }
}

#[derive(IntoElement, Default)]
pub struct CreateChatSheet;

impl CreateChatSheet {
    pub fn new() -> Self {
        Self
    }

    fn menu_fraction(screen_height: f32) -> f32 {
        let row = GroupedCardMetrics::ICON_SIDE + GroupedCardMetrics::ROW_VERTICAL_PADDING * 2.0;
        let handle = BottomSheetMetrics::HANDLE_TOP_PADDING
            + BottomSheetMetrics::HANDLE_HEIGHT
            + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;
        let header = PageHeaderMetrics::TOOLBAR_BUTTON_SIZE + 20.0;
        let content = handle + header + 4.0 + row * 3.0 + 1.0 + 16.0;
        let screen = screen_height.max(1.0);
        ((content + BottomSheetPagesUi::window_bottom_inset()) / screen).max(0.36)
    }

    fn pages(theme: &AppTheme, screen_height: f32) -> Vec<BottomSheetScreenItem> {
        let menu_theme = theme.clone();
        let group_theme = theme.clone();
        let group_settings_theme = theme.clone();
        let channel_theme = theme.clone();
        let channel_settings_theme = theme.clone();
        let bot_theme = theme.clone();
        let bot_settings_theme = theme.clone();
        vec![
            BottomSheetScreenItem::new(
                "menu",
                tr("create_chat_sheet_title"),
                Self::menu_fraction(screen_height),
                move |_: &mut Window, _: &mut App| CreateChatMenuView { theme: menu_theme.clone() },
            ),
            BottomSheetScreenItem::new(
                "group",
                tr("create_group_chat"),
                0.54,
                move |_: &mut Window, _: &mut App| CreateGroupChatView { theme: group_theme.clone() },
            ),
            BottomSheetScreenItem::new(
                "groupSettings",
                tr("group_settings"),
                0.93,
                move |_: &mut Window, _: &mut App| CreateGroupSettingsView { theme: group_settings_theme.clone() },
            ),
            BottomSheetScreenItem::new(
                "groupMembers",
                tr("chat_settings_members"),
                0.93,
                |_: &mut Window, _: &mut App| CreateGroupMembersView,
            ),
            BottomSheetScreenItem::new(
                "channel",
                tr("create_channel_chat"),
                0.62,
                move |_: &mut Window, _: &mut App| CreateChannelChatView { theme: channel_theme.clone() },
            ),
            BottomSheetScreenItem::new(
                "channelSettings",
                tr("channel_settings"),
                0.93,
                move |_: &mut Window, _: &mut App| CreateChannelSettingsView { theme: channel_settings_theme.clone() },
            ),
            BottomSheetScreenItem::new(
                "bot",
                tr("bot_create"),
                0.67,
                move |_: &mut Window, _: &mut App| CreateBotFirstStepView { theme: bot_theme.clone() },
            ),
            BottomSheetScreenItem::new(
                "botSettingsCreate",
                tr("bot_create_settings"),
                0.72,
                move |_: &mut Window, _: &mut App| CreateBotSettingsStepView { theme: bot_settings_theme.clone() },
            ),
        ]
    }

    fn token_sheet(
        state: &Entity<CreateChatSheetState>,
        bot_flow: &Entity<BotCreateFlowState>,
        background: Hsla,
    ) -> impl IntoElement + use<> {
        let is_presented = Binding::entity(bot_flow.clone(),
            |flow: &BotCreateFlowState| flow.token_presented,
            |flow: &mut BotCreateFlowState, value: bool, _: &mut Context<BotCreateFlowState>| {
                flow.token_presented = value;
            },
        );
        let content_flow = bot_flow.clone();
        let dismissed_flow = bot_flow.clone();
        let dismissed_state = state.clone();
        BottomSheetUi::new(
            "create-chat-bot-token-sheet",
            is_presented,
            move |_: &mut Window, cx: &mut App| {
                let flow = content_flow.read(cx);
                let (Some(bot), Some(token)) = (flow.created_bot.as_ref(), flow.one_time_token.clone()) else {
                    return Empty.into_any_element();
                };
                let closing = content_flow.clone();
                BotOneTimeTokenSheet::new("create-chat-bot-token-sheet", bot.name.clone(), token, move |_: &mut Window, cx: &mut App| {
                    closing.update(cx, |flow, cx| flow.clear_creation_result(cx));
                })
                .into_any_element()
            },
            |_: &mut Window, _: &mut App| Empty,
        )
        .fits_content(true)
        .dismisses_on_backdrop_tap(false)
        .full_page_allows_swipe_dismiss(false)
        .full_page_shows_drag_indicator(false)
        .full_page_hides_footer(true)
        .sheet_background(Some(background))
        .on_did_dismiss(move |_: &mut Window, cx: &mut App| {
            dismissed_flow.update(cx, |flow, cx| flow.clear_creation_result(cx));
            dismissed_state.update(cx, |state, cx| state.close(cx));
        })
    }
}

impl RenderOnce for CreateChatSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = CreateChatSheetState::shared(cx);
        let bot_flow = BotCreateFlowState::shared(cx);
        let nav_store = env::<NavigationStoreKey>(cx);
        let screen_height = window.use_keyed_state("create-chat-sheet-screen-height", cx, |_, _| 0.0_f32);
        let pages = Self::pages(&theme, *screen_height.read(cx));

        let read_state = state.clone();
        let write_state = state.clone();
        let is_bottom_sheet = Binding::new(
            move |cx: &App| read_state.read(cx).is_open(),
            move |value: bool, _: &mut Window, cx: &mut App| {
                write_state.update(cx, |state, cx| {
                    if value {
                        state.set_is_open(true, cx);
                    } else {
                        state.close(cx);
                    }
                });
            },
        );

        let presenting_flow = bot_flow.clone();
        let dismissed_flow = bot_flow.clone();
        let dismissed_state = state.clone();
        let screen_state = state.clone();
        let measured_height = screen_height.clone();
        let on_menu = matches!(state.read(cx).current_page(), "" | "menu");
        div()
            .absolute()
            .inset_0()
            .child(measure("create-chat-sheet-screen", move |bounds, _, cx| {
                measured_height.update(cx, |height, cx| {
                    *height = f32::from(bounds.size.height);
                    cx.notify();
                });
            }))
            .child(
                BottomSheetPagesUi::new("create-chat-sheet", is_bottom_sheet, pages, "menu")
                    .sheet_background(Some(theme.bg._100))
                    .modal_sized_by_detent(on_menu)
                    .on_present(move |_: &mut Window, cx: &mut App| {
                        presenting_flow.update(cx, |flow, cx| flow.reset_form(cx));
                    })
                    .on_did_dismiss(move |_: &mut Window, cx: &mut App| {
                        dismissed_flow.update(cx, |flow, cx| flow.present_pending_token(cx));
                        let pending = dismissed_state.update(cx, |state, _| state.take_pending_open_chat_id());
                        if let (Some(chat_id), Some(nav_store)) = (pending, nav_store.as_ref()) {
                            nav_store.update(cx, |nav_store, cx| {
                                let route = RootRoute::Chat { chat_id: Some(chat_id) };
                                if nav_store.chat_list_beside_stack()
                                    && nav_store.current_tab == MainTab::Chats
                                    && !nav_store.path.is_empty()
                                {
                                    nav_store.open_from_chat_list(route, cx);
                                } else {
                                    nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                                }
                            });
                        }
                        dismissed_state.update(cx, |state, cx| state.sheet_did_dismiss(cx));
                    })
                    .on_screen_change(Some(move |page: &str, _: &mut Window, cx: &mut App| {
                        screen_state.update(cx, |state, cx| state.set_current_page(page, cx));
                    })),
            )
            .child(Self::token_sheet(&state, &bot_flow, theme.bg._100))
    }
}

#[derive(IntoElement)]
struct CreateChatScreenLayout {
    content: AnyElement,
    footer: AnyElement,
    content_scrolls: bool,
}

impl CreateChatScreenLayout {
    fn new(content: impl IntoElement, footer: impl IntoElement) -> Self {
        Self { content: content.into_any_element(), footer: footer.into_any_element(), content_scrolls: false }
    }

    fn scrolling(content: impl IntoElement, footer: impl IntoElement) -> Self {
        Self { content_scrolls: true, ..Self::new(content, footer) }
    }
}

impl RenderOnce for CreateChatScreenLayout {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let spacer = if self.content_scrolls { div().flex_none().h(px(12.0)) } else { spacer_min_h(12.0) };
        let sheet_focus = env::<SheetFocusKey>(cx);
        form_scope(
            vstack(0.0)
                .size_full()
                .items_stretch()
                .capture_any_mouse_down(move |_, window, cx| {
                    KeyboardDismiss::release_field_focus(sheet_focus.as_ref(), window, cx);
                })
                .child(self.content)
                .child(spacer)
                .child(self.footer),
        )
    }
}

#[derive(IntoElement)]
struct CreateChatPrimaryFooter {
    id: &'static str,
    theme: AppTheme,
    title: String,
    disabled: bool,
    is_pending: bool,
    guide_anchor: Option<RiseGuideAnchor>,
    action: SheetAction,
}

impl RenderOnce for CreateChatPrimaryFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let action = self.action.clone();
        declare_form_primary(!self.disabled && !self.is_pending, self.action.clone(), cx);
        let bottom = if BottomSheetPagesUi::window_bottom_inset() > 0.0 { 4.0 } else { 16.0 };
        div().w_full().pb(px(bottom)).child(
            ButtonUi::new(self.id)
                .disabled(self.disabled)
                .is_pending(self.is_pending)
                .background_color(self.theme.primary._100)
                .height(48.0)
                .on_press(move |window: &mut Window, cx: &mut App| action(window, cx))
                .child(MainText::new(self.title).px(16.0).font_weight(FontWeight::SEMIBOLD).color(self.theme.content_on(self.theme.primary._100)))
                .guide_anchor(self.guide_anchor),
        )
    }
}

/// error alert for chat creation, shows when an error appears and its button clears it
#[derive(IntoElement)]
struct CreateChatErrorAlert {
    id: &'static str,
    is_channel: bool,
    content: AnyElement,
}

impl CreateChatErrorAlert {
    fn body(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let state = CreateChatSheetState::shared(cx);
        let snapshot = state.read(cx);
        let message = if snapshot.create_error_is_channel == self.is_channel {
            snapshot.create_error.clone()
        } else {
            None
        };
        let is_channel = self.is_channel;
        on_change(self.id, message, false, window, cx, move |_, message, _, cx| {
            let Some(message) = message else {
                return;
            };
            let title = tr(if is_channel { "create_channel_error" } else { "create_group_error" });
            NativeConfirmAlert::present_notice(
                &title,
                message,
                None,
                None,
                move |cx: &mut App| {
                    state.update(cx, |state, cx| {
                        state.create_error = None;
                        cx.notify();
                    });
                },
                cx,
            );
        });
        self.content
    }
}

impl RenderOnce for CreateChatErrorAlert {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.body(window, cx)
    }
}

#[derive(IntoElement)]
struct CreateChatGroupFooter {
    theme: AppTheme,
}

impl RenderOnce for CreateChatGroupFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = CreateChatSheetState::shared(cx).read(cx).group_form_disabled();
        let navigation = env::<BottomSheetNavigationStateKey>(cx);
        CreateChatPrimaryFooter {
            id: "create-chat-group-next",
            theme: self.theme,
            title: tr("next"),
            disabled,
            is_pending: false,
            guide_anchor: Some(RiseGuideAnchor::CreateChatNext),
            action: Rc::new(move |window: &mut Window, cx: &mut App| {
                navigate(&navigation, "groupSettings", window, cx);
            }),
        }
    }
}

#[derive(IntoElement)]
struct CreateChatChannelFooter {
    theme: AppTheme,
}

impl RenderOnce for CreateChatChannelFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = CreateChatSheetState::shared(cx);
        let disabled = state.read(cx).channel_form_disabled();
        let is_pending = state.read(cx).channel_tag_validating(cx);
        let navigation = env::<BottomSheetNavigationStateKey>(cx);
        CreateChatPrimaryFooter {
            id: "create-chat-channel-next",
            theme: self.theme,
            title: tr("next"),
            disabled,
            is_pending,
            guide_anchor: None,
            action: Rc::new(move |window: &mut Window, cx: &mut App| {
                navigate(&navigation, "channelSettings", window, cx);
            }),
        }
    }
}

#[derive(IntoElement)]
struct CreateChatGroupSubmitFooter {
    theme: AppTheme,
}

impl CreateChatGroupSubmitFooter {
    fn submit_group(cx: &mut App) {
        let state = CreateChatSheetState::shared(cx);
        state.update(cx, |state, cx| {
            state.is_creating = true;
            state.create_error = None;
            state.create_error_is_channel = false;
            cx.notify();
        });
        let (on_success, on_error) = create_chat_callbacks(&state);
        ChatActionsStore::shared(cx).update(cx, |store, cx| {
            store.create_group_action(None, Some(on_success), Some(on_error), cx);
        });
    }
}

impl RenderOnce for CreateChatGroupSubmitFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = CreateChatSheetState::shared(cx);
        let snapshot = state.read(cx);
        let logo_uploading = snapshot.logo_uploading;
        let is_pending = snapshot.is_creating || logo_uploading;
        let disabled = snapshot.group_settings_form_disabled(cx);
        CreateChatErrorAlert {
            id: "create-chat-group-error",
            is_channel: false,
            content: CreateChatPrimaryFooter {
                id: "create-chat-group-submit",
                theme: self.theme,
                title: tr(if logo_uploading { "music_uploading" } else { "create_group" }),
                disabled,
                is_pending,
                guide_anchor: Some(RiseGuideAnchor::CreateChatSubmit),
                action: Rc::new(|_: &mut Window, cx: &mut App| Self::submit_group(cx)),
            }
            .into_any_element(),
        }
    }
}

#[derive(IntoElement)]
struct CreateChatChannelSubmitFooter {
    theme: AppTheme,
}

impl CreateChatChannelSubmitFooter {
    fn submit_channel(cx: &mut App) {
        let state = CreateChatSheetState::shared(cx);
        state.update(cx, |state, cx| {
            state.is_creating = true;
            state.create_error = None;
            state.create_error_is_channel = true;
            cx.notify();
        });
        let (on_success, on_error) = create_chat_callbacks(&state);
        ChatActionsStore::shared(cx).update(cx, |store, cx| {
            store.create_channel_action(None, Some(on_success), Some(on_error), cx);
        });
    }
}

impl RenderOnce for CreateChatChannelSubmitFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = CreateChatSheetState::shared(cx);
        let snapshot = state.read(cx);
        let logo_uploading = snapshot.logo_uploading;
        let is_pending = snapshot.is_creating || logo_uploading;
        let disabled = snapshot.channel_settings_form_disabled(cx);
        CreateChatErrorAlert {
            id: "create-chat-channel-error",
            is_channel: true,
            content: CreateChatPrimaryFooter {
                id: "create-chat-channel-submit",
                theme: self.theme,
                title: tr(if logo_uploading { "music_uploading" } else { "create_channel" }),
                disabled,
                is_pending,
                guide_anchor: None,
                action: Rc::new(|_: &mut Window, cx: &mut App| Self::submit_channel(cx)),
            }
            .into_any_element(),
        }
    }
}

fn create_chat_callbacks(state: &Entity<CreateChatSheetState>) -> (ChatActionTextCallback, ChatActionTextCallback) {
    let succeeded = state.clone();
    let failed = state.clone();
    (
        Rc::new(move |chat_id: String, cx: &mut App| {
            succeeded.update(cx, |state, cx| state.close_after_creating(&chat_id, cx));
        }),
        Rc::new(move |message: String, cx: &mut App| {
            failed.update(cx, |state, cx| {
                state.create_error = Some(message);
                cx.notify();
            });
        }),
    )
}

#[derive(IntoElement)]
struct CreateChatMenuView {
    theme: AppTheme,
}

impl RenderOnce for CreateChatMenuView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let primary = self.theme.primary._100;
        let orange = color(ORANGE_HEX);
        let indigo = color(INDIGO_HEX);
        let navigation = env::<BottomSheetNavigationStateKey>(cx);
        let group_navigation = navigation.clone();
        let channel_navigation = navigation.clone();
        div().w_full().px(px(15.0)).pt(px(4.0)).pb(px(8.0)).child(GroupedBtns::new(
            "create-chat-menu",
            vec![
                GroupedBtn::new("group", tr("create_group_chat"))
                    .icon("person.3.fill")
                    .icon_background(primary.opacity(0.18))
                    .icon_foreground(primary)
                    .guide_anchor(RiseGuideAnchor::CreateChatGroup)
                    .callback(move |window: &mut Window, cx: &mut App| {
                        navigate(&group_navigation, "group", window, cx);
                    }),
                GroupedBtn::new("channel", tr("create_channel_chat"))
                    .icon("megaphone.fill")
                    .icon_background(orange.opacity(0.18))
                    .icon_foreground(orange)
                    .callback(move |window: &mut Window, cx: &mut App| {
                        navigate(&channel_navigation, "channel", window, cx);
                    }),
                GroupedBtn::new("bot", tr("bot_create"))
                    .icon("command.circle.fill")
                    .icon_background(indigo.opacity(0.18))
                    .icon_foreground(indigo)
                    .guide_anchor(RiseGuideAnchor::CreateChatBot)
                    .callback(move |window: &mut Window, cx: &mut App| {
                        navigate(&navigation, "bot", window, cx);
                    }),
            ],
        ))
    }
}

#[derive(IntoElement)]
struct CreateGroupChatView {
    theme: AppTheme,
}

impl RenderOnce for CreateGroupChatView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = CreateChatSheetState::shared(cx);
        let snapshot = state.read(cx);
        let upload_progress = snapshot.logo_upload_progress;
        let is_uploading = snapshot.logo_uploading && snapshot.group_local_logo_file.is_some();
        CreateChatScreenLayout::new(
            div().w_full().px(px(15.0)).pt(px(4.0)).child(CreateChatGroupedForm {
                id: ElementId::from("create-group-form"),
                theme: self.theme.clone(),
                local_file: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.group_local_logo_file.clone(),
                    |state, value, _| state.group_local_logo_file = value,
                ),
                preview: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.group_logo_preview.clone(),
                    |state, value, _| state.group_logo_preview = value,
                ),
                title: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.group_title().to_owned(),
                    |state, value: String, cx| state.set_group_title(&ChatFieldRules::title().clamped(&value), cx),
                ),
                title_placeholder: tr("create_group_chat_name_placeholder"),
                title_guide_anchor: Some(RiseGuideAnchor::CreateChatNameField),
                description: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.group_description.clone(),
                    |state, value: String, _| state.group_description = ChatFieldRules::description().clamped(&value),
                ),
                description_placeholder: tr("create_group_chat_description_placeholder"),
                tag: None,
                tag_placeholder: String::new(),
                tag_error: String::new(),
                tag_checking: false,
                upload_progress,
                is_uploading,
            }),
            div().w_full().px(px(15.0)).child(CreateChatGroupFooter { theme: self.theme }),
        )
    }
}

#[derive(IntoElement)]
struct CreateGroupSettingsView {
    theme: AppTheme,
}

impl RenderOnce for CreateGroupSettingsView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = CreateChatSheetState::shared(cx);
        let snapshot = state.read(cx);
        let members_count = snapshot.group_members.len();
        let is_public = snapshot.group_privacy == ChatPrivacyType::PublicChat;
        let tag_error = snapshot.group_tag_error.clone();
        let tag_checking = snapshot.group_tag_checking;
        let navigation = env::<BottomSheetNavigationStateKey>(cx);

        let mut members = GroupedBtn::new("members", tr("chat_settings_members"))
            .icon("person.2.fill")
            .callback(move |window: &mut Window, cx: &mut App| {
                navigate(&navigation, "groupMembers", window, cx);
            });
        if members_count > 0 {
            members = members.trailing(members_count.to_string());
        }

        CreateChatScreenLayout::scrolling(
            scroll_view("create-group-settings-scroll").shows_indicators(false).flex_1().min_h_0().w_full().px(px(15.0)).child(
                vstack(40.0)
                    .w_full()
                    .items_stretch()
                    .pt(px(15.0))
                    .pb(px(16.0))
                    .child(GroupedBtns::new("create-group-members", vec![members]))
                    .child(CreateChatPrivacyGroupedBtns {
                        id: "create-group-privacy",
                        theme: self.theme.clone(),
                        selection: Binding::entity(state.clone(),
                            |state: &CreateChatSheetState| state.group_privacy,
                            |state, value, _| state.group_privacy = value,
                        ),
                    })
                    .when(is_public, |column| {
                        column.child(CreateChatPublicTagField {
                            id: "create-group-public-tag",
                            theme: self.theme.clone(),
                            tag: Binding::entity(state.clone(),
                                |state: &CreateChatSheetState| state.group_tag.clone(),
                                |state, value: String, cx| state.set_group_tag(&ascii_capable(&value), cx),
                            ),
                            error: tag_error,
                            checking: tag_checking,
                            required: false,
                        })
                    })
                    .child(CreateChatForwardGroupedBtns::new(
                        false,
                        Binding::entity(state.clone(),
                            |state: &CreateChatSheetState| state.forward_in_group_blocked,
                            |state, value, _| state.forward_in_group_blocked = value,
                        ),
                    )),
            ),
            div().w_full().px(px(15.0)).child(CreateChatGroupSubmitFooter { theme: self.theme.clone() }),
        )
    }
}

#[derive(IntoElement)]
struct CreateGroupMembersView;

impl RenderOnce for CreateGroupMembersView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let navigation = env::<BottomSheetNavigationStateKey>(cx);
        let window_handle = window.window_handle();
        PeoplePickerSheet::new()
            .embedded(true)
            .on_appear("create-group-members-appear", move |_, cx| {
                let state = CreateChatSheetState::shared(cx);
                let preselected = state.read(cx).group_members.clone();
                let excluded_ids: HashSet<String> = AuthServicesStore::shared(cx)
                    .read(cx)
                    .profile()
                    .map(|profile| profile.id.clone())
                    .into_iter()
                    .collect();
                PeoplePickerStore::shared(cx).update(cx, |picker, cx| {
                    picker.end_embedded(cx);
                    picker.present(
                        PeoplePickerMode::SearchAll,
                        0,
                        preselected,
                        excluded_ids,
                        tr("chat_settings_members"),
                        None,
                        PeoplePickerHostContext::Embedded,
                        move |picked, cx: &mut App| {
                            state.update(cx, |state, cx| {
                                state.group_members = picked;
                                cx.notify();
                            });
                            cx.defer(move |cx| {
                                let _ = window_handle.update(cx, |_, window, cx| {
                                    if let Some(navigation) = navigation.as_ref() {
                                        BottomSheetNavigationState::go_back(navigation, window, cx);
                                    }
                                });
                            });
                        },
                        cx,
                    );
                });
            })
            .and_on_disappear(|_, cx| {
                PeoplePickerStore::shared(cx).update(cx, |picker, cx| picker.end_embedded(cx));
            })
    }
}

#[derive(IntoElement)]
struct CreateChannelChatView {
    theme: AppTheme,
}

impl RenderOnce for CreateChannelChatView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = CreateChatSheetState::shared(cx);
        let snapshot = state.read(cx);
        let upload_progress = snapshot.logo_upload_progress;
        let is_uploading = snapshot.logo_uploading && snapshot.channel_local_logo_file.is_some();
        CreateChatScreenLayout::new(
            div().w_full().px(px(15.0)).pt(px(4.0)).child(CreateChatGroupedForm {
                id: ElementId::from("create-channel-form"),
                theme: self.theme.clone(),
                local_file: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.channel_local_logo_file.clone(),
                    |state, value, _| state.channel_local_logo_file = value,
                ),
                preview: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.channel_logo_preview.clone(),
                    |state, value, _| state.channel_logo_preview = value,
                ),
                title: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.channel_title.clone(),
                    |state, value: String, _| state.channel_title = ChatFieldRules::title().clamped(&value),
                ),
                title_placeholder: tr("create_channel_chat_name_placeholder"),
                title_guide_anchor: None,
                description: Binding::entity(state.clone(),
                    |state: &CreateChatSheetState| state.channel_description.clone(),
                    |state, value: String, _| state.channel_description = ChatFieldRules::description().clamped(&value),
                ),
                description_placeholder: tr("create_channel_chat_description_placeholder"),
                tag: None,
                tag_placeholder: String::new(),
                tag_error: String::new(),
                tag_checking: false,
                upload_progress,
                is_uploading,
            }),
            div().w_full().px(px(15.0)).child(CreateChatChannelFooter { theme: self.theme }),
        )
    }
}

#[derive(IntoElement)]
struct CreateChannelSettingsView {
    theme: AppTheme,
}

impl RenderOnce for CreateChannelSettingsView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = CreateChatSheetState::shared(cx);
        let snapshot = state.read(cx);
        let is_public = snapshot.channel_privacy == ChatPrivacyType::PublicChat;
        let tag_error = snapshot.channel_tag_error.clone();
        let tag_checking = snapshot.channel_tag_checking;

        CreateChatScreenLayout::scrolling(
            scroll_view("create-channel-settings-scroll").shows_indicators(false).flex_1().min_h_0().w_full().px(px(15.0)).child(
                vstack(40.0)
                    .w_full()
                    .items_stretch()
                    .pt(px(15.0))
                    .pb(px(16.0))
                    .child(CreateChatPrivacyGroupedBtns {
                        id: "create-channel-privacy",
                        theme: self.theme.clone(),
                        selection: Binding::entity(state.clone(),
                            |state: &CreateChatSheetState| state.channel_privacy,
                            |state, value, _| state.channel_privacy = value,
                        ),
                    })
                    .when(is_public, |column| {
                        column.child(CreateChatPublicTagField {
                            id: "create-channel-public-tag",
                            theme: self.theme.clone(),
                            tag: Binding::entity(state.clone(),
                                |state: &CreateChatSheetState| state.channel_tag.clone(),
                                |state, value: String, cx| state.set_channel_tag(&ascii_capable(&value), cx),
                            ),
                            error: tag_error,
                            checking: tag_checking,
                            required: true,
                        })
                    })
                    .child(CreateChatForwardGroupedBtns::new(
                        true,
                        Binding::entity(state.clone(),
                            |state: &CreateChatSheetState| state.forward_in_channel_blocked,
                            |state, value, _| state.forward_in_channel_blocked = value,
                        ),
                    )),
            ),
            div().w_full().px(px(15.0)).child(CreateChatChannelSubmitFooter { theme: self.theme.clone() }),
        )
    }
}

#[derive(IntoElement)]
struct CreateBotFirstStepView {
    theme: AppTheme,
}

impl CreateBotFirstStepView {
    fn tag_status_icon(&self, availability: &BotTagAvailabilityState) -> Option<AnyElement> {
        match availability {
            BotTagAvailabilityState::Checking => {
                Some(progress_view().control_size(ControlSize::Small).into_any_element())
            }
            BotTagAvailabilityState::Available => {
                IconUi::sized("checkmark.circle.fill", px(TAG_STATUS_ICON_SIDE), color(GREEN_HEX), true)
                    .map(IntoElement::into_any_element)
            }
            BotTagAvailabilityState::Invalid(_)
            | BotTagAvailabilityState::Unavailable { .. }
            | BotTagAvailabilityState::Failed => IconUi::sized(
                "exclamationmark.circle.fill",
                px(TAG_STATUS_ICON_SIDE),
                self.theme.semantic.error_100,
                true,
            )
            .map(IntoElement::into_any_element),
            BotTagAvailabilityState::Idle => None,
        }
    }

    fn status_text(availability: &BotTagAvailabilityState) -> String {
        match availability {
            BotTagAvailabilityState::Idle => tr("bot_tag_suffix_hint"),
            BotTagAvailabilityState::Checking => tr("bot_tag_checking"),
            BotTagAvailabilityState::Available => tr("bot_tag_available"),
            BotTagAvailabilityState::Unavailable { suggestions } => match suggestions.first() {
                Some(suggestion) if !suggestion.is_empty() => {
                    format!("{} @{}", tr("bot_tag_unavailable"), suggestion)
                }
                _ => tr("bot_tag_unavailable"),
            },
            BotTagAvailabilityState::Failed => tr("bot_tag_check_failed"),
            BotTagAvailabilityState::Invalid(issue) => tr(&issue.message_key),
        }
    }

    fn status_color(&self, availability: &BotTagAvailabilityState) -> Hsla {
        match availability {
            BotTagAvailabilityState::Available => color(GREEN_HEX),
            BotTagAvailabilityState::Invalid(_)
            | BotTagAvailabilityState::Unavailable { .. }
            | BotTagAvailabilityState::Failed => self.theme.semantic.error_100,
            BotTagAvailabilityState::Idle | BotTagAvailabilityState::Checking => self.theme.text.secondary,
        }
    }
}

impl RenderOnce for CreateBotFirstStepView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let flow = BotCreateFlowState::shared(cx);
        let snapshot = flow.read(cx);
        let availability = snapshot.tag_availability.clone();
        let first_step_valid = snapshot.first_step_valid();
        let navigation = env::<BottomSheetNavigationStateKey>(cx);

        on_change(
            "create-bot-first-step-valid",
            first_step_valid,
            true,
            window,
            cx,
            |_, valid, _, cx| GuideFacts::set(RiseGuideSignal::CreateBotFormValid, *valid, cx),
        );

        let mut tag_row = BotSettingsFieldRow::new(
            "create-bot-tag",
            "at",
            color(BLUE_HEX),
            tr("bot_tag_placeholder"),
            Binding::entity(flow.clone(),
                |flow: &BotCreateFlowState| flow.tag.clone(),
                |flow, value: String, cx| flow.set_tag(&ascii_capable(&value), cx),
            ),
        );
        if let Some(icon) = self.tag_status_icon(&availability) {
            tag_row = tag_row.trailing(icon);
        }

        CreateChatScreenLayout::scrolling(
            scroll_view("create-bot-first-step-scroll").flex_1().min_h_0().w_full().child(
                vstack(12.0)
                    .w_full()
                    .items_stretch()
                    .pt(px(4.0))
                    .pb(px(12.0))
                    .child(
                        GroupedBtns::new(
                            "create-bot-fields",
                            vec![
                                GroupedBtn::new("name", "").content_slot(BotSettingsFieldRow::new(
                                    "create-bot-name",
                                    "command.circle.fill",
                                    color(INDIGO_HEX),
                                    tr("bot_name_placeholder"),
                                    Binding::entity(flow.clone(),
                                        |flow: &BotCreateFlowState| flow.name.clone(),
                                        |flow, value: String, _| flow.name = BotFieldRules::name().clamped(&value),
                                    ),
                                )),
                                GroupedBtn::new("tag", "").content_slot(tag_row),
                                GroupedBtn::new("description", "").content_slot(
                                    BotSettingsFieldRow::new(
                                        "create-bot-description",
                                        "text.alignleft",
                                        SettingsIconPalette::privacy_description(),
                                        tr("bot_description_placeholder"),
                                        Binding::entity(flow.clone(),
                                            |flow: &BotCreateFlowState| flow.description.clone(),
                                            |flow, value: String, _| {
                                                flow.description = BotFieldRules::description().clamped(&value);
                                            },
                                        ),
                                    )
                                    .axis(Axis::Vertical),
                                ),
                            ],
                        )
                        .corner_radius(26.0)
                        .uses_lazy_rows(false)
                        .guide_anchor(RiseGuideAnchor::CreateBotFields),
                    )
                    .child(
                        div().w_full().px(px(18.0)).child(
                            MainText::new(Self::status_text(&availability))
                                .px(12.0)
                                .color(self.status_color(&availability)),
                        ),
                    ),
            ),
            div().w_full().px(px(15.0)).child(CreateChatPrimaryFooter {
                id: "create-bot-next",
                theme: self.theme.clone(),
                title: tr("next"),
                disabled: !first_step_valid,
                is_pending: availability == BotTagAvailabilityState::Checking,
                guide_anchor: Some(RiseGuideAnchor::CreateBotNext),
                action: Rc::new(move |window: &mut Window, cx: &mut App| {
                    navigate(&navigation, "botSettingsCreate", window, cx);
                }),
            }),
        )
    }
}

#[derive(IntoElement)]
struct CreateBotSettingsStepView {
    theme: AppTheme,
}

impl CreateBotSettingsStepView {
    fn create_bot(flow: &Entity<BotCreateFlowState>, cx: &mut App) {
        let snapshot = flow.read(cx);
        let name = snapshot.name.clone();
        let tag = snapshot.tag.clone();
        let description = snapshot.description.clone();
        let can_join_groups = snapshot.can_join_groups;
        let privacy_mode = snapshot.privacy_mode;
        let finished = flow.clone();
        let on_result: BotCreateResult = Rc::new(move |bot: BotModel, token: String, cx: &mut App| {
            finished.update(cx, |flow, cx| flow.complete_creation(bot, token, cx));
            let sheet = CreateChatSheetState::shared(cx);
            if sheet.read(cx).is_open() {
                sheet.update(cx, |sheet, cx| sheet.close(cx));
            } else {
                finished.update(cx, |flow, cx| flow.present_pending_token(cx));
            }
        });
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.create_bot(&name, &tag, &description, "", "", can_join_groups, privacy_mode, Some(on_result), cx);
        });
    }
}

impl RenderOnce for CreateBotSettingsStepView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let flow = BotCreateFlowState::shared(cx);
        let first_step_valid = flow.read(cx).first_step_valid();
        let actions = BotActionsStore::shared(cx);
        let create_bot_sai = actions.read(cx).create_bot_sai.clone();
        let error = create_bot_sai.error();
        let is_pending = create_bot_sai.is_pending();
        let submitting = flow.clone();
        let secondary = self.theme.text.secondary;
        let error_color = self.theme.semantic.error_100;

        CreateChatScreenLayout::scrolling(
            scroll_view("create-bot-settings-scroll").flex_1().min_h_0().w_full().child(
                vstack(12.0)
                    .w_full()
                    .items_stretch()
                    .pt(px(4.0))
                    .pb(px(12.0))
                    .child(
                        GroupedBtns::new(
                            "create-bot-settings",
                            vec![
                                GroupedBtn::new("groups", "").content_slot(BotToggleRow::new(
                                    "create-bot-groups",
                                    "person.3.fill",
                                    color(INDIGO_HEX),
                                    tr("bot_can_join_groups"),
                                    tr("bot_can_join_groups_hint"),
                                    Binding::entity(flow.clone(),
                                        |flow: &BotCreateFlowState| flow.can_join_groups,
                                        |flow, value, _| flow.can_join_groups = value,
                                    ),
                                )),
                                GroupedBtn::new("privacy", "").content_slot(BotToggleRow::new(
                                    "create-bot-privacy",
                                    "lock.shield.fill",
                                    color(GREEN_HEX),
                                    tr("bot_privacy_mode"),
                                    tr("bot_privacy_mode_hint"),
                                    Binding::entity(flow.clone(),
                                        |flow: &BotCreateFlowState| flow.privacy_mode,
                                        |flow, value, _| flow.privacy_mode = value,
                                    ),
                                )),
                            ],
                        )
                        .corner_radius(26.0)
                        .guide_anchor(RiseGuideAnchor::CreateBotSettings),
                    )
                    .child(
                        div()
                            .px(px(18.0))
                            .child(MainText::new(tr("bot_create_token_hint")).px(13.0).color(secondary)),
                    )
                    .when_some(error, |column, error| {
                        column.child(
                            div().w_full().px(px(18.0)).child(MainText::new(error).px(13.0).color(error_color)),
                        )
                    }),
            ),
            div().w_full().px(px(15.0)).child(CreateChatPrimaryFooter {
                id: "create-bot-submit",
                theme: self.theme.clone(),
                title: tr("bot_create"),
                disabled: !first_step_valid,
                is_pending,
                guide_anchor: Some(RiseGuideAnchor::CreateBotSubmit),
                action: Rc::new(move |_: &mut Window, cx: &mut App| Self::create_bot(&submitting, cx)),
            }),
        )
    }
}

#[derive(IntoElement)]
struct CreateChatGroupedForm {
    id: ElementId,
    theme: AppTheme,
    local_file: Binding<Option<SaiWsOptimisticLocalFile>>,
    preview: Binding<Option<CleverImage>>,
    title: Binding<String>,
    title_placeholder: String,
    title_guide_anchor: Option<RiseGuideAnchor>,
    description: Binding<String>,
    description_placeholder: String,
    tag: Option<Binding<String>>,
    tag_placeholder: String,
    tag_error: String,
    tag_checking: bool,
    upload_progress: f64,
    is_uploading: bool,
}

impl CreateChatGroupedForm {
    fn remaining_title_character_count(&self, cx: &App) -> usize {
        ChatFieldRules::title().remaining(&self.title.get(cx)).unwrap_or(0)
    }

    fn form_rows(self, cx: &App) -> Div {
        let theme = self.theme.clone();
        let id = self.id.clone();
        let remaining = self.remaining_title_character_count(cx);
        let secondary = theme.text.secondary;
        let primary = theme.text.primary;

        vstack(0.0)
            .w_full()
            .items_stretch()
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .min_h(px(64.0))
                    .child(
                        div().id((id.clone(), "avatar")).flex_none().tooltip_label(tr("create_chat_avatar_picker_title")).child(
                            SelectMediaUi::new((id.clone(), "avatar-picker"), self.local_file, self.preview)
                                .size(42.0)
                                .placeholder_system_image("camera.fill")
                                .cover_upload_progress(self.upload_progress)
                                .show_upload_overlay(self.is_uploading)
                                .placeholder_background(theme.primary._100)
                                .placeholder_foreground(theme.content_on(theme.primary._100)),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            ShapedTextField::new((id.clone(), "title"), self.title_placeholder, self.title)
                                .auto_focus(true)
                                .placeholder_color(secondary)
                                .font_size(16.0)
                                .text_color(primary)
                                .on_submit(|window: &mut Window, cx: &mut App| window.focus_next(cx))
                                .guide_anchor(self.title_guide_anchor),
                        ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .swiftui_font(Font::system_weight(12.0, FontWeight::MEDIUM).monospaced())
                            .text_color(secondary)
                            .child(remaining.to_string()),
                    ),
            )
            .child(Self::form_separator(&theme, 70.0))
            .when_some(self.tag, |rows, tag| {
                let tag_error = self.tag_error;
                let tag_checking = self.tag_checking;
                let tag_read = tag.clone();
                let tag_binding = Binding::new(
                    move |cx: &App| tag_read.get(cx),
                    move |raw: String, window: &mut Window, cx: &mut App| tag.set(ascii_capable(&raw), window, cx),
                );
                rows.child(
                    hstack(12.0)
                        .w_full()
                        .px(px(16.0))
                        .py(px(10.0))
                        .min_h(px(58.0))
                        .child(Self::field_icon(&theme, "at"))
                        .child(
                            vstack(2.0)
                                .flex_1()
                                .min_w_0()
                                .items_start()
                                .child(
                                    ShapedTextField::new(
                                        (id.clone(), "tag"),
                                        self.tag_placeholder,
                                        tag_binding,
                                    )
                                    .placeholder_color(secondary)
                                    .font_size(16.0)
                                    .text_color(primary)
                                    .on_submit(|window: &mut Window, cx: &mut App| window.focus_next(cx)),
                                )
                                .when(!tag_error.is_empty(), |column| {
                                    column.child(MainText::new(tag_error).px(11.0).color(theme.semantic.error_100))
                                }),
                        )
                        .when(tag_checking, |row| row.child(progress_view().control_size(ControlSize::Small))),
                )
                .child(Self::form_separator(&theme, 57.0))
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .items_start()
                    .px(px(16.0))
                    .py(px(14.0))
                    .min_h(px(58.0))
                    .child(div().pt(px(2.0)).child(Self::field_icon(&theme, "text.alignleft")))
                    .child(
                        div().flex_1().min_w_0().child(
                            ShapedTextField::new((id, "description"), self.description_placeholder, self.description)
                                .axis(Axis::Vertical)
                                .max_visible_lines(3)
                                .placeholder_color(secondary)
                                .font_size(16.0)
                                .text_color(primary),
                        ),
                    ),
            )
    }

    fn field_icon(theme: &AppTheme, system_name: &'static str) -> impl IntoElement + use<> {
        Shape::continuous(8.0).round(
            div()
                .flex_none()
                .size(px(29.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg._300)
                .children(IconUi::sized(system_name, px(14.0), theme.text.secondary, false)),
        )
    }

    fn form_separator(theme: &AppTheme, leading: f32) -> Div {
        div().w_full().pl(px(leading)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
    }
}

impl RenderOnce for CreateChatGroupedForm {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let group_id = (self.id.clone(), "group");
        GroupedBtns::new(
            group_id,
            vec![GroupedBtn::new("createChatForm", "").content_slot(self.form_rows(cx))],
        )
        .corner_radius(28.0)
        .horizontal_padding(0.0)
        .separator_leading_padding(0.0)
        .uses_lazy_rows(false)
    }
}

#[derive(IntoElement)]
struct CreateChatPrivacyGroupedBtns {
    id: &'static str,
    theme: AppTheme,
    selection: Binding<ChatPrivacyType>,
}

impl RenderOnce for CreateChatPrivacyGroupedBtns {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let selected = self.selection.get(cx);
        let primary = self.theme.primary._100;
        let unselected_background = self.theme.bg._300;
        let items = [ChatPrivacyType::PublicChat, ChatPrivacyType::PrivateChat]
            .into_iter()
            .map(|privacy| {
                let is_selected = selected == privacy;
                let is_public = privacy == ChatPrivacyType::PublicChat;
                let selection = self.selection.clone();
                GroupedBtn::new(
                    if is_public { "public" } else { "private" },
                    tr(if is_public { "public_type" } else { "private_type" }),
                )
                .icon(if is_selected { "checkmark" } else { "circle" })
                .icon_background(if is_selected { primary.opacity(0.15) } else { unselected_background })
                .icon_foreground(if is_selected { primary } else { transparent_black() })
                .show_chevron(false)
                .callback(move |window: &mut Window, cx: &mut App| selection.set(privacy, window, cx))
            })
            .collect();
        vstack(8.0)
            .w_full()
            .items_stretch()
            .child(GroupedBtns::new(self.id, items).title(tr("link")).uses_lazy_rows(false))
            .child(
                div().w_full().px(px(16.0)).child(
                    MainText::new(tr(if selected == ChatPrivacyType::PublicChat {
                        "chat_settings_type_public_hint"
                    } else {
                        "chat_settings_type_private_hint"
                    }))
                    .px(14.0)
                    .color(self.theme.text.secondary),
                ),
            )
    }
}

#[derive(IntoElement)]
struct CreateChatPublicTagField {
    id: &'static str,
    theme: AppTheme,
    tag: Binding<String>,
    error: String,
    checking: bool,
    required: bool,
}

impl RenderOnce for CreateChatPublicTagField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let id = ElementId::from(self.id);
        let hint_color = if self.error.is_empty() { theme.text.secondary } else { theme.semantic.error_100 };
        let hint = if self.error.is_empty() {
            tr(if self.required {
                "chat_settings_public_link_required_hint"
            } else {
                "chat_settings_public_link_optional_hint"
            })
        } else {
            self.error
        };
        vstack(8.0)
            .w_full()
            .items_stretch()
            .child(
                GroupedBtns::new(
                    (id.clone(), "group"),
                    vec![GroupedBtn::new("public-tag", "").content_slot(
                        hstack(0.0)
                            .w_full()
                            .px(px(16.0))
                            .min_h(px(56.0))
                            .child(
                                MainText::new(format!("{}/", AppEnvironment::current().public_host()))
                                    .px(15.0)
                                    .color(theme.text.secondary),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    ShapedTextField::new(
                                        (id, "field"),
                                        tr("chat_settings_public_link_placeholder"),
                                        self.tag,
                                    )
                                    .font_size(15.0)
                                    .text_color(theme.text.primary),
                                ),
                            )
                            .when(self.checking, |row| row.child(progress_view().control_size(ControlSize::Small))),
                    )],
                )
                .title(tr("create_chat_public_link_title"))
                .uses_lazy_rows(false),
            )
            .child(
                div()
                    .w_full()
                    .px(px(16.0))
                    .child(MainText::new(hint).px(13.0).color(hint_color).number_of_lines(0)),
            )
    }
}

/// also the forwarding row of the chat type settings page so both read the same
#[derive(IntoElement)]
pub struct CreateChatForwardGroupedBtns {
    is_channel: bool,
    is_blocked: Binding<bool>,
}

impl CreateChatForwardGroupedBtns {
    pub fn new(is_channel: bool, is_blocked: Binding<bool>) -> Self {
        Self { is_channel, is_blocked }
    }

    fn explanation_key(&self, is_blocked: bool) -> &'static str {
        match (self.is_channel, is_blocked) {
            (true, false) => "forward_in_chat_block_explanation",
            (true, true) => "forward_in_chat_blocked_explanation",
            (false, false) => "forward_in_group_allowed_explanation",
            (false, true) => "forward_in_group_blocked_explanation",
        }
    }
}

impl RenderOnce for CreateChatForwardGroupedBtns {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let explanation = tr(self.explanation_key(self.is_blocked.get(cx)));
        let toggled = self.is_blocked.clone();
        GroupedBtns::new(
            "create-chat-forward",
            vec![
                GroupedBtn::new("forward", tr("forward_in_chat_block"))
                    .show_chevron(false)
                    .trailing_slot(SwitchUi::new("create-chat-forward-switch", self.is_blocked))
                    .callback(move |window: &mut Window, cx: &mut App| toggled.toggle(window, cx)),
            ],
        )
        .title(tr("forward_in_chat_block_title"))
        .end_group_title(explanation)
    }
}

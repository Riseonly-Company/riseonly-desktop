use std::rc::Rc;

use gpui::{App, FontWeight, Pixels, Point, SharedString, Window};
use rise_i18n::tr;
use rise_ui::bot_icon::BotIcon;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

type RowTapHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type RowActionHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct GlobalSearchProfileResultRow {
    pub row: Rc<SearchEntityRow>,
    pub on_tap: RowTapHandler,
    pub on_action: RowActionHandler,
}

impl GlobalSearchProfileResultRow {
    pub fn new(
        row: SearchEntityRow,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
        on_action: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { row: Rc::new(row), on_tap: Rc::new(on_tap), on_action: Rc::new(on_action) }
    }

    fn menu_actions(row: &SearchEntityRow, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut actions = vec![
            HoldContextMenuAction::new("open", tr("ctx_open_profile")).system_image("person.crop.circle"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
        ];
        let auth = AuthServicesStore::shared(cx);
        let me = auth.read(cx).profile().map(|profile| profile.id.as_str());
        if let Some(user_id) = row.user_id.as_deref()
            && !user_id.is_empty()
            && Some(user_id) != me
        {
            actions.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        actions
    }
}

impl RenderOnce for GlobalSearchProfileResultRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let actions = Self::menu_actions(&self.row, cx);
        let row = self.row;
        let on_tap = self.on_tap;
        let on_action = self.on_action;
        let id = SharedString::from(format!("global-search-profile-result-{}", row.id));
        let content_row = row;
        let container = HoldContextMenuContainer::new(
            id,
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let theme = rise_ui::theme(cx).clone();
                let row = &content_row;
                let name_color = if row.is_premium { PremiumNameColor::parse(row.name_color.as_deref()) } else { None };
                hstack(10.0)
                    .w_full()
                    .px(px(12.0))
                    .py(px(9.0))
                    .bg(theme.bg._100)
                    .hover({
                        let hover = row_hover_fill(&theme);
                        move |style| style.bg(hover)
                    })
                    .child(
                        UserLogoUi::new(SharedString::from(format!("global-search-profile-logo-{}", row.id)))
                            .source((!row.logo_url.is_empty()).then(|| SharedString::from(row.logo_url.clone())))
                            .name(SharedString::from(row.display_name.clone()))
                            .user_id(row.user_id.clone().map(SharedString::from))
                            .size(44.0),
                    )
                    .child(
                        vstack(2.0)
                            .items_start()
                            .min_w(px(0.0))
                            .child(
                                hstack(6.0)
                                    .min_w(px(0.0))
                                    .child(
                                        MainText::new(row.display_name.clone())
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(name_color)
                                            .number_of_lines(1),
                                    )
                                    .when(row.is_bot_user(), |line| {
                                        line.child(BotIcon::new(theme.text.primary).size(12.0))
                                    })
                                    .when(row.is_official, |line| line.child(OfficialBadge::new().size(px(15.0))))
                                    .when(row.is_premium, |line| {
                                        line.child(
                                            PremiumBadge::new()
                                                .user_id(row.user_id.clone().map(SharedString::from))
                                                .emoji(row.premium_badge_emoji.clone().map(SharedString::from))
                                                .size(15.0),
                                        )
                                    }),
                            )
                            .when(!row.tag_line.is_empty(), |column| {
                                column.child(
                                    MainText::new(row.tag_line.clone())
                                        .px(12.0)
                                        .color(theme.text.secondary)
                                        .number_of_lines(1),
                                )
                            }),
                    )
                    .child(spacer_min_w(0.0))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| on_action(id, window, cx)))
        .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| on_tap(window, cx)));
        div().w_full().child(container)
    }
}

#[derive(IntoElement)]
pub struct GlobalSearchChatResultRow {
    pub row: Rc<SearchEntityRow>,
    pub on_tap: RowTapHandler,
    pub on_action: RowActionHandler,
}

impl GlobalSearchChatResultRow {
    pub fn new(
        row: SearchEntityRow,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
        on_action: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { row: Rc::new(row), on_tap: Rc::new(on_tap), on_action: Rc::new(on_action) }
    }
}

impl RenderOnce for GlobalSearchChatResultRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let actions = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
        ];
        let on_tap = self.on_tap;
        let on_action = self.on_action;
        let content_row = self.row;
        let id = SharedString::from(format!("global-search-chat-result-{}", content_row.id));
        let container = HoldContextMenuContainer::new(
            id,
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let theme = rise_ui::theme(cx).clone();
                let row = &content_row;
                let is_forum = row
                    .chat_id
                    .as_deref()
                    .is_some_and(|chat_id| ChatServicesStore::shared(cx).read(cx).chat_topics_enabled(chat_id, cx));
                let symbol = if row.is_channel() {
                    "megaphone.fill"
                } else if is_forum {
                    "person.3.fill"
                } else {
                    "person.2.fill"
                };
                hstack(10.0)
                    .w_full()
                    .px(px(12.0))
                    .py(px(9.0))
                    .bg(theme.bg._100)
                    .hover({
                        let hover = row_hover_fill(&theme);
                        move |style| style.bg(hover)
                    })
                    .child(
                        UserLogoUi::new(SharedString::from(format!("global-search-chat-logo-{}", row.id)))
                            .source((!row.logo_url.is_empty()).then(|| SharedString::from(row.logo_url.clone())))
                            .name(SharedString::from(row.display_name.clone()))
                            .user_id(SharedString::from(row.chat_id.clone().unwrap_or_else(|| row.id.clone())))
                            .size(44.0)
                            .show_story_ring(false)
                            .open_story_on_press(false),
                    )
                    .child(
                        vstack(2.0)
                            .items_start()
                            .min_w(px(0.0))
                            .child(
                                MainText::new(row.display_name.clone())
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .number_of_lines(1),
                            )
                            .when(!row.tag_line.is_empty(), |column| {
                                column.child(
                                    MainText::new(row.tag_line.clone())
                                        .px(12.0)
                                        .color(theme.text.secondary)
                                        .number_of_lines(1),
                                )
                            }),
                    )
                    .child(spacer_min_w(0.0))
                    .children(IconUi::sized(symbol, px(13.0), theme.text.secondary, true))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| on_action(id, window, cx)))
        .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| on_tap(window, cx)));
        div().w_full().child(container)
    }
}

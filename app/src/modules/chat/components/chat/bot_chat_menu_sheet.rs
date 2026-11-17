use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, ListAlignment, ListState, MouseButton, SharedString, Window,
    list,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::{AppTheme, alpha};
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::button_ui::ButtonUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::bot::components::bot_ui_components::BotStateView;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotCommandModel, BotMiniAppModel};
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatBottomBarMetrics;

pub enum BotChatMenuPresentationContract {}

impl BotChatMenuPresentationContract {
    pub const BUTTON_SIZE: f32 = ChatBottomBarMetrics::CIRCLE;
    pub const COLLAPSED_FRACTION: f32 = 0.52;
    pub const EXPANDED_FRACTION: f32 = 0.93;
    pub const ROW_HEIGHT: f32 = 54.0;

    pub fn starts_expanded(system_keyboard_height: f32) -> bool {
        system_keyboard_height > 1.0
    }

    pub fn initial_snap_fraction(starts_expanded: bool) -> f32 {
        if starts_expanded { Self::EXPANDED_FRACTION } else { Self::COLLAPSED_FRACTION }
    }

    pub fn header_accessory_opacity(is_presented: bool) -> f64 {
        if is_presented { 0.0 } else { 1.0 }
    }
}

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

pub type BotChatMenuCommandAction = Rc<dyn Fn(&BotCommandModel, &mut Window, &mut App)>;
pub type BotChatMenuMiniAppAction = Rc<dyn Fn(&BotMiniAppModel, &mut Window, &mut App)>;
pub type BotChatMenuAction = Rc<dyn Fn(&mut Window, &mut App)>;

const BOTTOM_CLEARANCE: f32 = 6.0;


/// desktop placement of the bot menu, a popover over the composer under its button
pub enum BotChatMenuPopoverMetrics {}

impl BotChatMenuPopoverMetrics {
    pub const WIDTH: f32 = 340.0;
    pub const MAX_HEIGHT: f32 = 440.0;
    pub const CORNER_RADIUS: f32 = 16.0;
    pub const STATE_VERTICAL_PADDING: f32 = 22.0;
    pub const OPEN_DURATION: f32 = 0.16;
    pub const CLOSED_SCALE: f32 = 0.92;

    /// list height for rows plus the pagination row as the menu draws them
    pub fn list_height(rows: usize, has_more: bool) -> f32 {
        if rows == 0 {
            return 0.0;
        }
        rows as f32 * BotChatMenuPresentationContract::ROW_HEIGHT
            + (rows - 1) as f32 * 0.5
            + 2.0
            + if has_more { 48.0 } else { 0.0 }
            + BOTTOM_CLEARANCE
    }
}

/// bot menu of a bot chat, opens as a popover over the composer sized to its rows
/// has compact empty, loading and error states
#[derive(IntoElement)]
pub struct BotChatMenuSheet {
    pub id: ElementId,
    pub commands: Vec<BotCommandModel>,
    pub mini_apps: Vec<BotMiniAppModel>,
    pub has_more_mini_apps: bool,
    pub is_loading_more_mini_apps: bool,
    pub pagination_cache_key: String,
    pub relative_id: Option<String>,
    pub status: SaiWsSliceStatus,
    pub error: Option<String>,
    pub starts_expanded: bool,
    pub bottom_inset: f32,
    pub leading_inset: f32,
    pub available_height: f32,
    pub on_command: BotChatMenuCommandAction,
    pub on_mini_app: BotChatMenuMiniAppAction,
    pub on_retry: BotChatMenuAction,
    pub on_load_more_mini_apps: BotChatMenuAction,
    pub is_presented: Binding<bool>,
}

struct BotChatMenuListState {
    list: ListState,
    row_count: usize,
}

impl RenderOnce for BotChatMenuSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let is_presented = self.is_presented.get(cx);
        let appeared = window.use_keyed_state(child_id(&self.id, "appeared"), cx, |_, _| Animated::new(0.0_f32));
        let focus_handle = window.use_keyed_state(child_id(&self.id, "focus"), cx, |_, cx| cx.focus_handle());
        let focus_handle = focus_handle.read(cx).clone();

        let now = RiseClock::monotonic();
        let target = if is_presented { 1.0 } else { 0.0 };
        appeared.update(cx, |appeared, cx| {
            appeared.set_in(target, ease_out(f64::from(BotChatMenuPopoverMetrics::OPEN_DURATION)), now, cx);
        });
        let fraction = appeared.read(cx).frame_value(now, window).clamp(0.0, 1.0);

        let focus_on_open = focus_handle.clone();
        on_change(child_id(&self.id, "is-presented"), is_presented, false, window, cx, move |_, open, window, cx| {
            if *open {
                window.focus(&focus_on_open, cx);
            }
        });

        if !is_presented && fraction <= 0.0 {
            return div().into_any_element();
        }

        let max_height = self.available_height.clamp(0.0, BotChatMenuPopoverMetrics::MAX_HEIGHT);
        let rows = self.commands.len() + self.mini_apps.len();
        let list_height = (rows > 0)
            .then(|| BotChatMenuPopoverMetrics::list_height(rows, self.has_more_mini_apps).min(max_height));
        let bottom_inset = self.bottom_inset;
        let leading_inset = self.leading_inset;
        let backdrop_binding = self.is_presented.clone();
        let escape_binding = self.is_presented.clone();
        let content = self.content(&theme, window, cx);
        let body = match list_height {
            Some(height) => div().w_full().h(px(height)).child(content),
            None => div()
                .w_full()
                .max_h(px(max_height))
                .padding_edges(EdgeSet::VERTICAL, BotChatMenuPopoverMetrics::STATE_VERTICAL_PADDING)
                .child(content),
        };
        let card = div()
            .id(child_id(&self.id, "popover"))
            .w(px(BotChatMenuPopoverMetrics::WIDTH))
            .rounded(px(BotChatMenuPopoverMetrics::CORNER_RADIUS))
            .overflow_hidden()
            .bg(theme.bg._200)
            .occlude()
            .child(body)
            .liquid_chrome_rounded_rectangle(BotChatMenuPopoverMetrics::CORNER_RADIUS);
        let scale = BotChatMenuPopoverMetrics::CLOSED_SCALE + (1.0 - BotChatMenuPopoverMetrics::CLOSED_SCALE) * fraction;

        div()
            .absolute()
            .inset_0()
            .track_focus(&focus_handle)
            .layer_keys(LayerCancel::dismiss(move |window, cx| escape_binding.set(false, window, cx)))
            .child(
                div()
                    .id(child_id(&self.id, "backdrop"))
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom(px(bottom_inset))
                    .when(is_presented, |backdrop| {
                        backdrop.occlude().on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            backdrop_binding.set(false, window, cx);
                        })
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(leading_inset))
                    .bottom(px(bottom_inset))
                    .opacity(fraction)
                    .child(div().child(card).scale_effect_anchored(scale, UnitPoint::BOTTOM_LEADING)),
            )
            .into_any_element()
    }
}

impl BotChatMenuSheet {
    fn content(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let is_empty = self.commands.is_empty() && self.mini_apps.is_empty();
        if is_empty && matches!(self.status, SaiWsSliceStatus::Pending | SaiWsSliceStatus::Idle) {
            return vstack(10.)
                .w_full()
                .items_center()
                .child(progress_view())
                .child(SecondaryText::new(tr("bot_chat_menu_loading")).px(13.))
                .into_any_element();
        }
        if is_empty && (self.status == SaiWsSliceStatus::Rejected || self.error.is_some()) {
            let on_retry = self.on_retry.clone();
            return BotStateView::new(
                child_id(&self.id, "error"),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("bot_load_error"),
            )
            .message(self.error.clone())
            .action_title(Some(tr("bot_retry")))
            .action(move |window, cx| on_retry(window, cx))
            .into_any_element();
        }
        if is_empty {
            return BotStateView::new(child_id(&self.id, "empty"), "command.circle", tr("bot_chat_menu_empty"))
                .message(Some(tr("bot_chat_menu_empty_hint")))
                .into_any_element();
        }
        self.rows(theme, window, cx)
    }

    fn rows(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let row_count = self.commands.len() + self.mini_apps.len() + usize::from(self.has_more_mini_apps);
        let overdraw = theme.list.overdraw;
        let state = window.use_keyed_state(child_id(&self.id, "rows"), cx, |_, _| BotChatMenuListState {
            list: ListState::new(row_count, ListAlignment::Top, overdraw),
            row_count,
        });
        let list_state = state.update(cx, |state, _| {
            if state.row_count != row_count {
                if row_count > state.row_count {
                    state.list.splice(state.row_count..state.row_count, row_count - state.row_count);
                } else {
                    state.list.reset(row_count);
                }
                state.row_count = row_count;
            }
            state.list.clone()
        });

        let menu = Rc::new(BotChatMenuRows {
            id: self.id.clone(),
            commands: self.commands.clone(),
            mini_apps: self.mini_apps.clone(),
            has_more_mini_apps: self.has_more_mini_apps,
            is_loading_more_mini_apps: self.is_loading_more_mini_apps,
            pagination_cache_key: self.pagination_cache_key.clone(),
            relative_id: self.relative_id.clone(),
            on_command: self.on_command.clone(),
            on_mini_app: self.on_mini_app.clone(),
            on_load_more_mini_apps: self.on_load_more_mini_apps.clone(),
            theme: theme.clone(),
        });
        list(list_state, move |index, _, _| menu.row(index)).size_full().into_any_element()
    }
}

struct BotChatMenuRows {
    id: ElementId,
    commands: Vec<BotCommandModel>,
    mini_apps: Vec<BotMiniAppModel>,
    has_more_mini_apps: bool,
    is_loading_more_mini_apps: bool,
    pagination_cache_key: String,
    relative_id: Option<String>,
    on_command: BotChatMenuCommandAction,
    on_mini_app: BotChatMenuMiniAppAction,
    on_load_more_mini_apps: BotChatMenuAction,
    theme: AppTheme,
}

impl BotChatMenuRows {
    fn row(&self, index: usize) -> AnyElement {
        let command_count = self.commands.len();
        let mini_app_count = self.mini_apps.len();
        let top = if index == 0 { 2.0 } else { 0.0 };
        let is_last = index + 1 == command_count + mini_app_count + usize::from(self.has_more_mini_apps);
        let bottom = if is_last { BOTTOM_CLEARANCE } else { 0.0 };
        let row = if index < command_count {
            self.command_row(index)
        } else if index < command_count + mini_app_count {
            self.mini_app_row(index - command_count)
        } else {
            div()
                .w_full()
                .flex()
                .justify_center()
                .padding_edges(EdgeSet::VERTICAL, 14.)
                .opacity(if self.is_loading_more_mini_apps { 1.0 } else { 0.7 })
                .child(progress_view())
                .into_any_element()
        };
        div()
            .w_full()
            .padding_edges(EdgeSet::TOP, top)
            .padding_edges(EdgeSet::BOTTOM, bottom)
            .child(row)
            .into_any_element()
    }

    /// row frame with horizontal padding 18 and min row height, centers the content vertically
    fn row_frame(row: Div) -> Div {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .padding_edges(EdgeSet::HORIZONTAL, 18.)
            .min_h(px(BotChatMenuPresentationContract::ROW_HEIGHT))
            .child(row)
    }

    fn command_row(&self, index: usize) -> AnyElement {
        let command = self.commands[index].clone();
        let title = if command.description.is_empty() {
            format!("/{}", command.command)
        } else {
            command.description.clone()
        };
        let on_command = self.on_command.clone();
        let pressed = command.clone();
        let button = plain(child_id(&self.id, format!("command-{}", command.id())))
            .on_press(move |window, cx| on_command(&pressed, window, cx))
            .w_full()
            .child(
                Self::row_frame(
                    hstack(12.)
                        .align(VerticalAlignment::FirstTextBaseline)
                        .w_full()
                        .child(
                            div().flex_1().min_w_0().mr(px(12.)).child(
                                MainText::new(title).px(16.).color(self.theme.text.primary).number_of_lines(1),
                            ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .child(SecondaryText::new(format!("/{}", command.command)).px(15.).line_limit(1)),
                        ),
                ),
            );
        let has_separator = index + 1 < self.commands.len() || !self.mini_apps.is_empty();
        vstack(0.).w_full().child(button).when(has_separator, |row| row.child(self.separator())).into_any_element()
    }

    fn mini_app_row(&self, index: usize) -> AnyElement {
        let mini_app = self.mini_apps[index].clone();
        let on_mini_app = self.on_mini_app.clone();
        let pressed = mini_app.clone();
        let on_load_more = self.on_load_more_mini_apps.clone();
        let row_id = child_id(&self.id, format!("mini-app-{}", mini_app.id));
        let button = plain(row_id.clone())
            .on_press(move |window, cx| on_mini_app(&pressed, window, cx))
            .w_full()
            .child(Self::row_frame(
                hstack(12.)
                    .align(VerticalAlignment::FirstTextBaseline)
                    .w_full()
                    .child(
                        div().flex_1().min_w_0().mr(px(12.)).child(
                            MainText::new(mini_app.title.clone())
                                .px(16.)
                                .color(self.theme.text.primary)
                                .number_of_lines(1),
                        ),
                    )
                    .child(div().flex_none().child(SecondaryText::new(tr("ctx_open")).px(15.).line_limit(1))),
            ))
            .sai_pagination_prefetch(
                child_id(&row_id, format!("prefetch-{}", mini_app.id)),
                SaiPaginationPrefetchOptions::new(
                    index,
                    self.mini_apps.len(),
                    self.pagination_cache_key.clone(),
                    self.relative_id.clone(),
                    self.has_more_mini_apps,
                )
                .threshold(8),
                move |window, cx| on_load_more(window, cx),
            );
        let has_separator = index + 1 < self.mini_apps.len();
        vstack(0.).w_full().child(button).when(has_separator, |row| row.child(self.separator())).into_any_element()
    }

    fn separator(&self) -> AnyElement {
        div()
            .w_full()
            .padding_edges(EdgeSet::LEADING, 18.)
            .child(div().w_full().h(px(0.5)).bg(alpha(self.theme.border._200, 0.38)))
            .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct BotChatStartCard {
    pub id: ElementId,
    pub name: String,
    pub tag: String,
    pub bot_user_id: String,
    pub description: String,
    pub avatar_url: Option<String>,
    pub can_manage: bool,
    pub on_start: BotChatMenuAction,
    pub on_manage: BotChatMenuAction,
}

impl RenderOnce for BotChatStartCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let on_start = self.on_start.clone();
        let on_manage = self.on_manage.clone();
        let logo_user_id = if self.bot_user_id.is_empty() { self.tag.clone() } else { self.bot_user_id.clone() };
        let card = vstack(12.)
            .padding_edges(EdgeSet::HORIZONTAL, 22.)
            .padding_edges(EdgeSet::VERTICAL, 20.)
            .max_w(px(330.))
            .child(
                UserLogoUi::new(child_id(&self.id, "logo"))
                    .source(self.avatar_url.clone().map(SharedString::from))
                    .name(Some(SharedString::from(self.name.clone())))
                    .user_id(Some(SharedString::from(logo_user_id)))
                    .size(82.),
            )
            .child(
                vstack(4.)
                    .child(
                        MainText::new(self.name.clone())
                            .px(21.)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD)
                            .number_of_lines(2),
                    )
                    .child(SecondaryText::new(format!("@{}", self.tag)).px(13.).tac(TextAlignT::Center)),
            )
            .when(!self.description.is_empty(), |card| {
                card.child(
                    div()
                        .padding_edges(EdgeSet::HORIZONTAL, 10.)
                        .child(SecondaryText::new(self.description.clone()).px(14.).tac(TextAlignT::Center).line_limit(4)),
                )
            })
            .child(
                ButtonUi::new(child_id(&self.id, "start"))
                    .fit_content(true)
                    .padding_horizontal(34.)
                    .on_press(move |window, cx| on_start(window, cx))
                    .child(
                        hstack(7.)
                            .children(IconUi::sized("play.fill", px(12.), theme.content_on(theme.primary._100), true))
                            .child(
                                MainText::new(tr("bot_start")).px(15.).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100)),
                            ),
                    ),
            )
            .when(self.can_manage, |card| {
                card.child(
                    plain(child_id(&self.id, "manage")).on_press(move |window, cx| on_manage(window, cx)).child(
                        hstack(6.)
                            .children(IconUi::sized("gearshape.fill", px(14.), theme.primary._100, true))
                            .child(
                                MainText::new(tr("bot_manage"))
                                    .px(14.)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.primary._100),
                            ),
                    ),
                )
            });
        card.liquid_chrome_rounded_rectangle(24.)
    }
}

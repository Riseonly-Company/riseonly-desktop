use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, Hsla, Pixels, Point, SharedString, Subscription, Window};
use rise_canon::rise_key::user as user_key;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAction, HoldContextMenuAlignment, HoldContextMenuLocksScrollKey,
};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, theme};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::guides::shared::guide_sandbox_settings::GuideSandboxAchievementsScene;
use crate::modules::user::components::achievements::achievement_card::AchievementCard;
use crate::modules::user::components::achievements::achievement_detail_sheet::AchievementDetailSheet;
use crate::modules::user::components::achievements::achievements_grid_skeleton::AchievementsGridSkeleton;
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::engine::rise_user_achievements_presentation::RiseUserAchievementsPresentation;
use crate::modules::user::stores::user_achievements::user_achievements_interactions::user_achievements_interactions::UserAchievementsInteractionsStore;
use crate::modules::user::stores::user_achievements::user_achievements_services::user_achievements_services::UserAchievementsServicesStore;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// page state plus the achievements presentation and hold menu it reads
struct AchievementsState {
    detail_achievement: Option<RiseAchievement>,
    detail_content_height: f32,
    grid_width: f32,
    _subscriptions: [Subscription; 2],
}

impl AchievementsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseUserAchievementsPresentation::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self {
            detail_achievement: None,
            detail_content_height: 0.0,
            grid_width: 0.0,
            _subscriptions: [
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn set_detail_achievement(&mut self, achievement: Option<RiseAchievement>, cx: &mut Context<Self>) {
        self.detail_achievement = achievement;
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct Achievements {
    id: ElementId,
    user_id: Option<String>,
}

impl Default for Achievements {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Achievements {
    const GRID_COLUMNS: usize = 4;
    const GRID_SPACING: f32 = 12.0;
    const ROW_SPACING: f32 = 16.0;

    pub fn new(user_id: Option<String>) -> Self {
        Self { id: ElementId::Name(SharedString::new_static("achievements")), user_id }
    }

    fn target_user_id(user_id: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(user_id?);
        if value.is_empty() { None } else { Some(value.to_owned()) }
    }

    fn cell_side(width: f32) -> f32 {
        let columns = Self::GRID_COLUMNS as f32;
        let available = (width - 32.0 - Self::GRID_SPACING * (columns - 1.0)).max(160.0);
        (available / columns).floor()
    }

    fn detail_sheet(
        id: ElementId,
        state: Entity<AchievementsState>,
        can_share: bool,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> BottomSheetUi {
        let detail_content_height = state.read(cx).detail_content_height;
        let fraction = BottomSheetMeasuredFraction::attached(detail_content_height, 0.0, 0.4, window).unwrap_or(0.66);

        let read_state = state.clone();
        let write_state = state.clone();
        let is_bottom_sheet = Binding::new(
            move |cx: &App| read_state.read(cx).detail_achievement.is_some(),
            move |is_presented: bool, _: &mut Window, cx: &mut App| {
                if !is_presented {
                    write_state.update(cx, |state, cx| state.set_detail_achievement(None, cx));
                }
            },
        );
        let content_id = named_child(&id, "content");

        BottomSheetUi::new(
            id,
            is_bottom_sheet,
            move |_, cx| {
                let Some(achievement) = state.read(cx).detail_achievement.clone() else {
                    return Empty.into_any_element();
                };
                let dismiss_state = state.clone();
                let height_state = state.clone();
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .w_full()
                            .flex_none()
                            .relative()
                            .child(AchievementDetailSheet::new(achievement, can_share).on_dismiss(move |_, cx| {
                                dismiss_state.update(cx, |state, cx| state.set_detail_achievement(None, cx));
                            }))
                            .child(measure(content_id.clone(), move |bounds, _, cx| {
                                let height = f32::from(bounds.size.height);
                                height_state.update(cx, |state, cx| {
                                    state.detail_content_height = height;
                                    cx.notify();
                                });
                            })),
                    )
                    .into_any_element()
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![fraction])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
    }
}

impl RenderOnce for Achievements {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| AchievementsState::new(cx));
        let theme = theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let target_user_id = Self::target_user_id(self.user_id.as_deref());
        let is_own_list = target_user_id.is_none();

        let detail_open = state.read(cx).detail_achievement.is_some();
        on_change(named_child(&self.id, "detail-open"), detail_open, false, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::AchievementDetailOpen, *open, cx);
        });

        let back_store = nav_store.clone();
        let appear_target = target_user_id.clone();
        let wrapper = SettingsWrapperUi::new(tr("achievements_page_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(AchievementsBody {
            id: named_child(&self.id, "body"),
            target_user_id,
            nav_store,
            state: state.clone(),
        })
        .on_appear(named_child(&self.id, "appear"), move |_, cx| {
            UserAchievementsInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.open_achievements(appear_target.as_deref(), cx);
                if appear_target.is_none() {
                    store.mark_visible_achievements_seen(cx);
                }
            });
        })
        .and_on_disappear(|_, cx| GuideFacts::set(RiseGuideSignal::AchievementDetailOpen, false, cx));

        let sheet = Self::detail_sheet(named_child(&self.id, "detail"), state, is_own_list, &theme, window, cx);

        div().size_full().relative().child(wrapper).child(sheet)
    }
}

#[derive(IntoElement)]
struct AchievementsBody {
    id: ElementId,
    target_user_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
    state: Entity<AchievementsState>,
}

impl AchievementsBody {
    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        named_child(&self.id, name)
    }

    fn is_own_list(&self) -> bool {
        self.target_user_id.is_none()
    }

    fn scroll(&self, side: f32, theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let locks_scroll = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let refresh_target = self.target_user_id.clone();

        let mut column = vstack(20.0).w_full().align(HorizontalAlignment::Leading).pt(px(12.0));
        if let Some(message) = self.inline_error_message(cx) {
            column = column.child(Self::inline_error(message, theme));
        }
        column = column.child(self.sections(side, theme, cx));
        if self.is_own_list() {
            column = column.child(self.privacy_row());
        }
        column = column.child(spacer_min_h(80.0));

        env_scope::<HoldContextMenuLocksScrollKey>(locks_scroll).child(
            scroll_view(self.child_id("scroll"))
                .shows_indicators(false)
                .scroll_disabled(locks_scroll)
                .refreshable(move |_, cx| {
                    let user_id = refresh_target.clone();
                    cx.spawn(async move |cx| {
                        UserAchievementsInteractionsStore::refresh_achievements(user_id, cx).await;
                    })
                })
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(column),
        )
    }

    fn sections(&self, side: f32, theme: &AppTheme, cx: &mut App) -> AnyElement {
        if self.shows_skeleton(cx) {
            return div()
                .w_full()
                .px(px(16.0))
                .child(
                    AchievementsGridSkeleton::new(side)
                        .columns(Achievements::GRID_COLUMNS)
                        .spacing(Achievements::GRID_SPACING)
                        .row_spacing(Achievements::ROW_SPACING),
                )
                .into_any_element();
        }
        if self.shows_blocking_error(cx) {
            return self.error_state(theme).into_any_element();
        }
        if self.shows_hidden_state(cx) {
            return Self::hidden_state(theme).into_any_element();
        }
        if self.shows_empty_state(cx) {
            return self.empty_state(theme).into_any_element();
        }
        if self.is_own_list() {
            return self.own_sections(side, theme, cx).into_any_element();
        }
        let rows = self.foreign_rows(cx);
        self.grid("foreign", rows, side, cx).into_any_element()
    }

    fn own_sections(&self, side: f32, theme: &AppTheme, cx: &mut App) -> Div {
        let visible = UserAchievementsServicesStore::visible_achievements(cx);
        let hidden = UserAchievementsServicesStore::hidden_achievements(cx);
        let progress_title = Self::progress_title(cx);

        let mut column = vstack(20.0).w_full().align(HorizontalAlignment::Leading);
        if !visible.is_empty() {
            column = column.child(self.section("visible", progress_title, visible, side, theme, cx));
        } else if !hidden.is_empty() {
            column = column.child(Self::section_title(progress_title, theme));
        }
        if !hidden.is_empty() {
            column = column.child(self.section(
                "hidden",
                tr("achievements_section_hidden"),
                hidden,
                side,
                theme,
                cx,
            ));
        }
        column
    }

    fn section(
        &self,
        scope: &'static str,
        title: impl Into<SharedString>,
        rows: Vec<RiseAchievement>,
        side: f32,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Div {
        vstack(12.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(Self::section_title(title, theme))
            .child(self.grid(scope, rows, side, cx))
    }

    fn section_title(text: impl Into<SharedString>, theme: &AppTheme) -> Div {
        div().px(px(20.0)).child(MainText::new(text).px(13.0).tac(TextAlignT::Left).color(theme.text.secondary))
    }

    /// denominator is the catalogue size, never a hint which achievements are unearned
    fn progress_title(cx: &mut App) -> String {
        tr("achievements_progress")
            .replace("{earned}", &UserAchievementsServicesStore::earned_count(cx).to_string())
            .replace("{total}", &UserAchievementsServicesStore::progress_total(cx).to_string())
    }

    fn grid(&self, scope: &'static str, rows: Vec<RiseAchievement>, side: f32, cx: &mut App) -> Div {
        let mut grid = div()
            .w_full()
            .px(px(16.0))
            .grid()
            .grid_cols(Achievements::GRID_COLUMNS as u16)
            .gap_x(px(Achievements::GRID_SPACING))
            .gap_y(px(Achievements::ROW_SPACING));
        for achievement in rows {
            grid = grid.child(self.cell(scope, achievement, side, cx));
        }
        grid
    }

    fn cell(
        &self,
        scope: &'static str,
        achievement: RiseAchievement,
        side: f32,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let is_pending = UserAchievementsServicesStore::is_mutating(achievement.id, cx);
        let actions = Self::hold_actions(&achievement, self.is_own_list(), is_pending);
        let anchor = (achievement.id == GuideSandboxAchievementsScene::PRACTICE_ID)
            .then_some(RiseGuideAnchor::AchievementsPractice);

        let action_achievement = achievement.clone();
        let tap_achievement = achievement.clone();
        let tap_state = self.state.clone();
        let content = achievement.clone();

        div()
            .w(px(side))
            .child(
                HoldContextMenuContainer::new(
                    self.child_id(format!("hold-{scope}-{}", achievement.id)),
                    HoldContextMenuAlignment::Leading,
                    Rc::new(move |_: &App| actions.clone()),
                    Rc::new(move |_: &mut Window, _: &mut App| {
                        AchievementCard::new(content.clone(), side).is_pending(is_pending).into_any_element()
                    }),
                )
                .show_emoji_list(false)
                .on_action(Rc::new(move |action_id: String, _: &mut Window, cx: &mut App| {
                    Self::handle(&action_id, &action_achievement, cx);
                }))
                .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                    let achievement = tap_achievement.clone();
                    tap_state.update(cx, |state, cx| state.set_detail_achievement(Some(achievement), cx));
                })),
            )
            .guide_anchor(anchor)
    }

    fn privacy_row(&self) -> GroupedBtns {
        let nav_store = self.nav_store.clone();
        GroupedBtns::new(
            self.child_id("privacy"),
            vec![
                GroupedBtn::new("achievements_privacy", tr("achievements_privacy_row"))
                    .icon("lock.fill")
                    .icon_background(SettingsIconPalette::privacy_achievements())
                    .callback(move |_, cx| {
                        if let Some(nav_store) = &nav_store {
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(
                                    RootRoute::PrivacySetting { r#type: Some(user_key::ACHIEVEMENTS_RULE.to_owned()) },
                                    NavigationStoreSettlingPush::Drop,
                                    cx,
                                );
                            });
                        }
                    }),
            ],
        )
    }

    fn state_icon(symbol: &str, side: f32, color: Hsla) -> Option<impl IntoElement> {
        IconUi::sized(symbol, px(side), color, false)
    }

    fn empty_state(&self, theme: &AppTheme) -> Div {
        let mut column = vstack(10.0)
            .w_full()
            .px(px(40.0))
            .py(px(48.0))
            .children(Self::state_icon("trophy", 42.0, theme.text.secondary.opacity(0.45)))
            .child(
                MainText::new(if self.is_own_list() {
                    tr("achievements_empty_title")
                } else {
                    tr("achievements_foreign_empty_title")
                })
                .px(17.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::SEMIBOLD),
            );
        if self.is_own_list() {
            column = column.child(
                SecondaryText::new(tr("achievements_empty_subtitle"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            );
        }
        column
    }

    fn hidden_state(theme: &AppTheme) -> Div {
        vstack(10.0)
            .w_full()
            .px(px(40.0))
            .py(px(48.0))
            .children(Self::state_icon("lock", 42.0, theme.text.secondary.opacity(0.45)))
            .child(
                MainText::new(tr("profile_section_hidden"))
                    .px(17.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD),
            )
    }

    fn error_state(&self, theme: &AppTheme) -> Div {
        let retry_target = self.target_user_id.clone();
        vstack(14.0)
            .w_full()
            .px(px(40.0))
            .py(px(44.0))
            .children(Self::state_icon("exclamationmark.triangle", 38.0, theme.text.secondary.opacity(0.5)))
            .child(
                MainText::new(tr("achievements_error_title"))
                    .px(16.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                ButtonUi::new(self.child_id("retry"))
                    .background_color(theme.primary._100)
                    .height(44.0)
                    .fit_content(true)
                    .padding_horizontal(20.0)
                    .on_press(move |_, cx| {
                        let target = retry_target.clone();
                        UserAchievementsInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.retry_achievements(target.as_deref(), cx);
                        });
                    })
                    .child(MainText::new(tr("common_retry")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100))),
            )
    }

    fn inline_error(message: String, theme: &AppTheme) -> Div {
        div().w_full().px(px(16.0)).child(
            hstack(8.0)
                .w_full()
                .px(px(12.0))
                .py(px(10.0))
                .bg(theme.bg._200)
                .clip_shape(Shape::continuous(12.0))
                .children(Self::state_icon("exclamationmark.circle", 14.0, theme.text.secondary))
                .child(SecondaryText::new(message).px(13.0).tac(TextAlignT::Left).color(theme.text.secondary))
                .child(spacer_min_w(0.0)),
        )
    }

    fn hold_actions(achievement: &RiseAchievement, is_own_list: bool, is_pending: bool) -> Vec<HoldContextMenuAction> {
        let mut items = Vec::new();

        if is_own_list {
            items.push(
                HoldContextMenuAction::new(
                    if achievement.hidden { "unhide" } else { "hide" },
                    if achievement.hidden { tr("achievements_unhide_action") } else { tr("achievements_hide_action") },
                )
                .system_image(if achievement.hidden { "eye" } else { "eye.slash" })
                .disabled(is_pending)
                .guide_anchor(RiseGuideAnchor::AchievementMenuVisibility),
            );
        }

        items.push(
            HoldContextMenuAction::new("copy_title", tr("achievements_copy_title_action")).system_image("doc.on.doc"),
        );

        items
    }

    fn handle(action_id: &str, achievement: &RiseAchievement, cx: &mut App) {
        match action_id {
            "hide" | "unhide" => {
                let hidden = action_id == "hide";
                UserAchievementsInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.set_hidden(achievement, hidden, cx);
                });
            }
            "copy_title" => {
                UIPasteboard::general().set_string(&achievement.title, cx);
                haptics::selection_changed();
            }
            _ => {}
        }
    }

    fn foreign_rows(&self, cx: &mut App) -> Vec<RiseAchievement> {
        match &self.target_user_id {
            Some(target) => UserAchievementsServicesStore::showcase_achievements(target, cx),
            None => Vec::new(),
        }
    }

    fn shows_skeleton(&self, cx: &mut App) -> bool {
        match &self.target_user_id {
            Some(target) => UserAchievementsServicesStore::shows_showcase_skeleton(target, cx),
            None => UserAchievementsServicesStore::shows_initial_skeleton(cx),
        }
    }

    fn shows_blocking_error(&self, cx: &mut App) -> bool {
        match &self.target_user_id {
            Some(target) => UserAchievementsServicesStore::shows_showcase_error(target, cx),
            None => UserAchievementsServicesStore::shows_blocking_error(cx),
        }
    }

    fn shows_hidden_state(&self, cx: &mut App) -> bool {
        match &self.target_user_id {
            Some(target) => UserAchievementsServicesStore::shows_showcase_hidden(target, cx),
            None => false,
        }
    }

    fn shows_empty_state(&self, cx: &mut App) -> bool {
        match &self.target_user_id {
            Some(target) => UserAchievementsServicesStore::shows_showcase_empty_state(target, cx),
            None => UserAchievementsServicesStore::shows_empty_state(cx),
        }
    }

    fn inline_error_message(&self, cx: &mut App) -> Option<String> {
        if self.target_user_id.is_some() {
            return None;
        }
        UserAchievementsServicesStore::inline_error(cx)?;
        Some(tr("achievements_error_title"))
    }
}

impl RenderOnce for AchievementsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx).clone();
        let width = self.state.read(cx).grid_width;

        let width_state = self.state.clone();
        let mut root = div().size_full().relative().child(measure(self.child_id("width"), move |bounds, _, cx| {
            let width = f32::from(bounds.size.width);
            width_state.update(cx, |state, cx| {
                state.grid_width = width;
                cx.notify();
            });
        }));
        if width > 0.0 {
            root = root.child(self.scroll(Achievements::cell_side(width), &theme, cx));
        }
        root
    }
}

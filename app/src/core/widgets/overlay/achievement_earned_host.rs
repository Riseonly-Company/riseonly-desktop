use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, RenderOnce,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use rise_canon::rise_key;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_i18n::{tr, tr_plural};
use rise_platform::power::system_low_power_mode;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::{
    BottomSheetMeasuredFraction, BottomSheetMeasuredFractionModifiers,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::reduce_motion;

use crate::core::animations::components::confetti_animation::ConfettiAnimation;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::overlay::achievement_earned_store::AchievementEarnedStore;
use crate::modules::user::components::achievements::achievement_artwork::{
    AchievementArtwork, AchievementArtworkPolicy,
};
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

const SHEET_ID: &str = "achievement_earned_sheet";

const INITIAL_FRACTION: f32 = 0.72;
const MINIMUM_FRACTION: f32 = 0.4;

pub struct AchievementEarnedHost {
    store: Entity<AchievementEarnedStore>,
    waits_for_contact_sheet: bool,
    _store_observation: Subscription,
    _user_services_observation: Subscription,
}

impl AchievementEarnedHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = AchievementEarnedStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        let user_services_observation = cx.observe(&user_services, |host, user_services, cx| {
            let waits = user_services.read(cx).should_present_contact_sync_onboarding();
            if waits != host.waits_for_contact_sheet {
                host.waits_for_contact_sheet = waits;
                cx.notify();
            }
        });
        let waits_for_contact_sheet = user_services.read(cx).should_present_contact_sync_onboarding();
        Self {
            store,
            waits_for_contact_sheet,
            _store_observation: store_observation,
            _user_services_observation: user_services_observation,
        }
    }
}

impl Render for AchievementEarnedHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let waits_for_contact_sheet = self.waits_for_contact_sheet;
        let read = self.store.clone();
        let write = self.store.clone();
        let done = self.store.clone();
        AchievementEarnedSheet::new(
            Binding::new(
                move |cx| read.read(cx).is_visible() && !waits_for_contact_sheet,
                move |is_presented, _, cx| {
                    if !is_presented {
                        write.update(cx, |store, cx| store.hide(cx));
                    }
                },
            ),
            self.store.read(cx).current().map(|grant| grant.achievement.clone()),
            move |_, cx| done.update(cx, |store, cx| store.hide(cx)),
        )
    }
}

#[derive(IntoElement)]
pub struct AchievementEarnedSheet {
    is_presented: Binding<bool>,
    achievement: Option<RiseAchievement>,
    on_dismiss: SheetAction,
}

impl AchievementEarnedSheet {
    pub fn new(
        is_presented: Binding<bool>,
        achievement: Option<RiseAchievement>,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            is_presented,
            achievement,
            on_dismiss: Rc::new(on_dismiss),
        }
    }

    fn content(
        achievement: Option<&RiseAchievement>,
        is_presented: bool,
        decorative_motion_allowed: bool,
        theme: &AppTheme,
    ) -> AnyElement {
        let mut column = div().w_full().pt_2().pb_6().flex().flex_col().items_center().gap_5();

        if let Some(achievement) = achievement {
            column = column.child(AchievementArtwork::new(achievement.clone(), 148.0).loops(decorative_motion_allowed));

            let description = Self::description(achievement);
            let mut lines = div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(
                    MainText::new(tr("achievement_earned_title"))
                        .px(26.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::BOLD),
                )
                .child(
                    MainText::new(achievement.title.clone())
                        .px(19.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(AchievementArtworkPolicy::accent_color(achievement).unwrap_or(theme.primary._100)),
                );
            if !description.is_empty() {
                lines = lines.child(
                    div().px_5().child(
                        MainText::new(description)
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
                );
            }
            column = column.child(lines);

            let how_to_earn = Self::how_to_earn(achievement);
            if !how_to_earn.is_empty() {
                column = column.child(GroupedBtns::new(
                    "achievement_earned_how_to_earn",
                    vec![
                        GroupedBtn::new(rise_key::events::HOW_TO_EARN, how_to_earn)
                            .icon("target")
                            .icon_background(SettingsIconPalette::achievements())
                            .pretitle(tr("achievements_how_to_earn_title"))
                            .show_chevron(false),
                    ],
                ));
            }

            if achievement.granted_count > 0 {
                column = column.child(
                    div().w_full().child(
                        SecondaryText::new(tr_plural("achievements_holders_count", achievement.granted_count))
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
                );
            }
        }

        let mut framed = div().relative().w_full().overflow_hidden().child(column);
        if is_presented && decorative_motion_allowed {
            framed = framed.child(
                div()
                    .absolute()
                    .top(px(-54.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(ConfettiAnimation::new("achievement_earned_confetti").size(480.0).looping(false)),
            );
        }
        framed.into_any_element()
    }

    fn footer(on_dismiss: &SheetAction, theme: &AppTheme) -> AnyElement {
        let on_dismiss = on_dismiss.clone();
        div()
            .w_full()
            .px(px(6.0))
            .child(
                ButtonUi::new("achievement_earned_continue")
                    .background_color(theme.primary._100)
                    .height(52.0)
                    .on_press(move |window, cx| on_dismiss(window, cx))
                    .child(
                        MainText::new(tr("achievement_earned_continue"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.content_on(theme.primary._100)),
                    ),
            )
            .into_any_element()
    }

    fn description(achievement: &RiseAchievement) -> String {
        trimming_whitespaces_and_newlines(&achievement.description).to_owned()
    }

    fn how_to_earn(achievement: &RiseAchievement) -> String {
        trimming_whitespaces_and_newlines(&achievement.how_to_earn).to_owned()
    }

    /// artwork already freezes under reduce motion so this gate adds low power mode and power saving
    fn decorative_motion_allowed(cx: &mut App) -> bool {
        !reduce_motion(cx)
            && !system_low_power_mode()
            && !RiseEnergySavingStore::shared(cx)
                .read(cx)
                .reduces(RiseEnergySavingFeature::InterfaceEffects)
    }
}

impl RenderOnce for AchievementEarnedSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content_height = window.use_keyed_state((ElementId::from(SHEET_ID), "content_height"), cx, |_, _| 0.0_f32);
        let footer_height = window.use_keyed_state((ElementId::from(SHEET_ID), "footer_height"), cx, |_, _| 0.0_f32);
        let sheet_fraction = BottomSheetMeasuredFraction::attached(
            *content_height.read(cx),
            *footer_height.read(cx),
            MINIMUM_FRACTION,
            window,
        )
        .unwrap_or(INITIAL_FRACTION);
        let report_content = Rc::new(store_height(content_height));
        let report_footer = Rc::new(store_height(footer_height));
        let sheet_background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let is_presented = self.is_presented.get(cx);
        let decorative_motion_allowed = Self::decorative_motion_allowed(cx);
        let achievement = self.achievement;
        let on_dismiss = self.on_dismiss;

        div().size_full().child(
            BottomSheetUi::new(
                SHEET_ID,
                self.is_presented,
                move |_, cx| {
                    let theme_store = ThemeStore::shared(cx);
                    let theme = theme_store.read(cx).current_theme();
                    div()
                        .id("achievement_earned_scroll")
                        .size_full()
                        .overflow_y_scroll()
                        .child(
                            Self::content(achievement.as_ref(), is_presented, decorative_motion_allowed, theme)
                                .on_sheet_part_height((ElementId::from(SHEET_ID), "content_part"), {
                                    let report = report_content.clone();
                                    move |height: f32, window: &mut Window, cx: &mut App| report(height, window, cx)
                                }),
                        )
                },
                move |_, cx| {
                    let theme_store = ThemeStore::shared(cx);
                    let theme = theme_store.read(cx).current_theme();
                    Self::footer(&on_dismiss, theme).on_sheet_part_height((ElementId::from(SHEET_ID), "footer_part"), {
                        let report = report_footer.clone();
                        move |height: f32, window: &mut Window, cx: &mut App| report(height, window, cx)
                    })
                },
            )
            .snap_fractions(vec![sheet_fraction])
            .full_page(false)
            .dismisses_on_backdrop_tap(true)
            .full_page_shows_drag_indicator(true)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .sheet_background(Some(sheet_background))
            .footer_background(Some(sheet_background)),
        )
    }
}

fn store_height(height: Entity<f32>) -> impl Fn(f32, &mut Window, &mut App) + 'static {
    move |value, _, cx| {
        height.update(cx, |height, cx| {
            *height = value;
            cx.notify();
        });
    }
}

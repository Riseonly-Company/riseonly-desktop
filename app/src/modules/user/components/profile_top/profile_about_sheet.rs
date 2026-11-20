use std::rc::Rc;

use gpui::{AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, MouseButton, Subscription, Window};
use rise_canon::{RiseLimits, RisePrivacyRule};
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_theme::ui_color_system::UIColor;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::{ModalSheetFrame, ModalSheetMetrics};
use rise_ui::expandable_text_ui::{EdgeInsets, ExpandableTextUi};
use rise_ui::icon_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::rtc::media::rise_rtc_audio_settings::RiseRTCStreamerMode;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::profile_field_formatting::{BirthdayFormatting, PhoneNumberFormatting};
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::modules::user::components::achievements::achievement_card::{AchievementCard, AchievementSquareGeometry};
use crate::modules::user::components::profile_top::highlight_create_sheet::{child_id, resolved_appearance};
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::engine::rise_user_achievements_presentation::RiseUserAchievementsPresentation;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;
use crate::modules::user::stores::user_achievements::user_achievements_interactions::user_achievements_interactions::UserAchievementsInteractionsStore;
use crate::modules::user::stores::user_achievements::user_achievements_services::user_achievements_services::UserAchievementsServicesStore;

#[derive(IntoElement)]
pub struct ProfileAboutSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    profile: ProfileModel,
    achievements_user_id: Option<String>,
}

struct ProfileAboutSheetState {
    available_width: Option<f32>,
    description_expanded: bool,
    phone_revealed: bool,
    _presentation_observation: Subscription,
}

/// what a rows tap needs after the rows are built
#[derive(Clone)]
struct ProfileAboutSheetActions {
    is_presented: Binding<bool>,
    achievements_user_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
    description_expanded: Binding<bool>,
    phone_revealed: Binding<bool>,
}

impl ProfileAboutSheetActions {
    /// opens achievements,
    /// dismissal is committed first since a push under the sheet lands on a page nobody sees
    fn open_achievements(&self, window: &mut Window, cx: &mut App) {
        self.is_presented.set(false, window, cx);
        if let Some(nav_store) = self.nav_store.as_ref() {
            let route = RootRoute::Achievements { user_id: self.achievements_user_id.clone() };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

impl ProfileAboutSheet {
    const ACHIEVEMENTS_GRID_COLUMNS: usize = 4;
    const ACHIEVEMENTS_GRID_SPACING: f32 = 10.0;

    fn achievements_preview_limit() -> usize {
        usize::try_from(RiseLimits::client_value("profile_about_sheet_achievements_preview_limit")).unwrap_or(0)
    }

    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>, profile: ProfileModel) -> Self {
        Self { id: id.into(), is_presented, profile, achievements_user_id: None }
    }

    pub fn achievements_user_id(mut self, achievements_user_id: Option<String>) -> Self {
        self.achievements_user_id = achievements_user_id;
        self
    }

    fn description_row(&self, desc: &str, actions: &ProfileAboutSheetActions, theme: &AppTheme) -> AnyElement {
        let expanded = actions.description_expanded.clone();
        let toggle = expanded.clone();
        hstack(GroupedCardMetrics::ICON_SPACING)
            .align(VerticalAlignment::Top)
            .w_full()
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
            .child(
                zstack(Alignment::Center)
                    .flex_none()
                    .size(px(GroupedCardMetrics::ICON_SIDE))
                    .rounded(px(11.0))
                    .bg(UIColor::system_indigo().resolved_color(theme.appearance).opacity(0.85))
                    .children(IconUi::sized("text.alignleft", px(17.0), gpui::white(), false)),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(MainText::new(tr("profile_field_description")).px(11.0).color(theme.text.secondary))
                    .child(
                        ExpandableTextUi::new(
                            child_id(&self.id, "description-text"),
                            desc.to_owned(),
                            theme.text.primary,
                            theme.primary._100,
                        )
                        .font_size(15.0)
                        .line_limit(5)
                        .is_expanded(expanded)
                        .collapse_control_insets(EdgeInsets::new(4.0, 0.0, 0.0, 48.0))
                        .parent_handles_taps(true),
                    ),
            )
            .id(child_id(&self.id, "description-row"))
            .on_click(move |_, window, cx| ExpandableTextUi::toggle_animated(&toggle, window, cx))
            .into_any_element()
    }

    fn rows(&self, actions: &ProfileAboutSheetActions, available_width: f32, cx: &mut App) -> Vec<GroupedBtn> {
        let theme = rise_ui::theme(cx).clone();
        let appearance = resolved_appearance(cx);
        let system = |color: UIColor, opacity: f32| -> Hsla { color.resolved_color(appearance).opacity(opacity) };
        let mut rows = Vec::new();
        let is_own = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().as_deref()
            == Some(self.profile.id.as_str());
        let veiled = |rule: RisePrivacyRule, cx: &mut App| is_own && RiseRTCStreamerMode::conceals(rule, cx);

        if veiled(RisePrivacyRule::Achievements, cx) {
            rows.push(Self::veiled_row("achievements", "trophy.fill", SettingsIconPalette::achievements(), tr("profile_achievements_row"), RisePrivacyRule::Achievements, &theme));
        } else {
            let earned = self.earned_achievements(cx);
            let overflow = earned.len().saturating_sub(Self::achievements_preview_limit());
            let open = actions.clone();
            let mut achievements = GroupedBtn::new("achievements", tr("profile_achievements_row"))
                .icon("trophy.fill")
                .icon_background(SettingsIconPalette::achievements())
                .callback(move |window, cx| open.open_achievements(window, cx));
            if overflow > 0 {
                achievements = achievements.trailing(format!("+{overflow}"));
            }
            if let Some(slot) = self.achievements_slot(&earned, actions, available_width, cx) {
                achievements = achievements.expanded_slot(slot);
            }
            rows.push(achievements);
        }

        let tag = trimming_whitespaces_and_newlines(&self.profile.tag);
        if !tag.is_empty() {
            let value = format!("@{tag}");
            let copied = value.clone();
            rows.push(
                GroupedBtn::new("tag", value)
                    .icon("at")
                    .icon_background(theme.primary._100)
                    .pretitle(tr("profile_field_tag"))
                    .show_chevron(false)
                    .callback(move |_, cx| Self::copy(&copied, cx)),
            );
        }

        let aliases: Vec<String> = self
            .profile
            .alias_tags
            .iter()
            .map(|alias| trimming_whitespaces_and_newlines(alias))
            .filter(|alias| !alias.is_empty())
            .map(|alias| format!("@{alias}"))
            .collect();
        if !aliases.is_empty() {
            let joined = aliases.join(", ");
            let copied = joined.clone();
            rows.push(
                GroupedBtn::new("alias-tags", joined)
                    .icon("at.circle.fill")
                    .icon_background(theme.primary._100.opacity(0.75))
                    .pretitle(tr("profile_field_alias_tags"))
                    .show_chevron(false)
                    .callback(move |_, cx| Self::copy(&copied, cx)),
            );
        }

        if let Some(description) = self
            .profile
            .description
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|description| !description.is_empty())
        {
            if veiled(RisePrivacyRule::Description, cx) {
                let background = UIColor::system_indigo().resolved_color(theme.appearance).opacity(0.85);
                rows.push(Self::veiled_row("description", "text.alignleft", background, tr("profile_field_description"), RisePrivacyRule::Description, &theme));
            } else {
                rows.push(
                    GroupedBtn::new("description", description.to_owned())
                        .show_chevron(false)
                        .content_slot(self.description_row(description, actions, &theme)),
                );
            }
        }

        if let Some((date, age)) = self
            .profile
            .hb
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|hb| !hb.is_empty())
            .and_then(BirthdayFormatting::parse)
        {
            let background = UIColor::system_pink().resolved_color(theme.appearance).opacity(0.9);
            if veiled(RisePrivacyRule::Hb, cx) {
                rows.push(Self::veiled_row("birthday", "gift.fill", background, tr("profile_field_birthday"), RisePrivacyRule::Hb, &theme));
            } else {
                let mut birthday = GroupedBtn::new("birthday", date)
                    .icon("gift.fill")
                    .icon_background(background)
                    .pretitle(tr("profile_field_birthday"))
                    .show_chevron(false);
                if let Some(age) = age {
                    birthday = birthday.trailing(age);
                }
                rows.push(birthday);
            }
        }

        if let Some(gender) = Self::localized_gender(self.profile.gender.as_deref()) {
            rows.push(
                GroupedBtn::new("gender", gender)
                    .icon("person.fill")
                    .icon_background(UIColor::system_blue().resolved_color(theme.appearance).opacity(0.85))
                    .pretitle(tr("profile_field_gender"))
                    .show_chevron(false),
            );
        }

        if let Some(phone) = self
            .profile
            .more
            .phone
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|phone| !phone.is_empty())
        {
            if is_own {
                rows.push(Self::own_phone_row(phone, &actions.phone_revealed, &theme, cx));
            } else {
                let formatted = PhoneNumberFormatting::format(phone);
                let copied = formatted.clone();
                rows.push(
                    GroupedBtn::new("phone", formatted)
                        .icon("phone.fill")
                        .icon_background(UIColor::system_green().resolved_color(theme.appearance).opacity(0.85))
                        .pretitle(tr("profile_field_phone"))
                        .show_chevron(false)
                        .callback(move |_, cx| Self::copy(&copied, cx)),
                );
            }
        }

        let languages: Vec<&str> = self
            .profile
            .more
            .p_lang
            .iter()
            .map(String::as_str)
            .filter(|language| !trimming_whitespaces(language).is_empty())
            .collect();
        if !languages.is_empty() {
            rows.push(
                GroupedBtn::new("languages", languages.join(", "))
                    .icon("globe")
                    .icon_background(UIColor::system_teal().resolved_color(theme.appearance).opacity(0.85))
                    .pretitle(tr("profile_field_languages"))
                    .show_chevron(false),
            );
        }

        rows
    }

    fn achievements_slot(
        &self,
        earned: &[RiseAchievement],
        actions: &ProfileAboutSheetActions,
        available_width: f32,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let side = Self::achievement_cell_side(available_width);
        let preview = &earned[..earned.len().min(Self::achievements_preview_limit())];
        if !preview.is_empty() {
            return Some(Self::achievements_grid(preview, actions, side).into_any_element());
        }
        if self.shows_achievements_skeleton(cx) {
            return Some(Self::achievements_skeleton(side).into_any_element());
        }
        None
    }

    fn achievements_grid(earned: &[RiseAchievement], actions: &ProfileAboutSheetActions, side: f32) -> Div {
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .gap(px(Self::ACHIEVEMENTS_GRID_SPACING))
            .px(px(16.0))
            .pb(px(14.0))
            .children(earned.iter().map(|achievement| {
                let open = actions.clone();
                plain(("profile-about-achievement", achievement.id as u64))
                    .on_press(move |window, cx| open.open_achievements(window, cx))
                    .child(AchievementCard::new(achievement.clone(), side).shows_title(false))
            }))
    }

    fn achievements_skeleton(side: f32) -> impl IntoElement {
        pass_through()
            .w_full()
            .px(px(16.0))
            .pb(px(14.0))
            .flex()
            .flex_row()
            .gap(px(Self::ACHIEVEMENTS_GRID_SPACING))
            .children((0..Self::ACHIEVEMENTS_GRID_COLUMNS).map(|_| {
                SkeletonBlock::new(side, side, AchievementSquareGeometry::corner_radius(side))
            }))
            .skeleton_shimmer(None)
    }

    fn earned_achievements(&self, cx: &mut App) -> Vec<RiseAchievement> {
        match self.normalized_achievements_user_id() {
            Some(user_id) => UserAchievementsServicesStore::showcase_achievements(user_id, cx),
            None => UserAchievementsServicesStore::visible_achievements(cx),
        }
    }

    fn shows_achievements_skeleton(&self, cx: &mut App) -> bool {
        match self.normalized_achievements_user_id() {
            Some(user_id) => UserAchievementsServicesStore::shows_showcase_skeleton(user_id, cx),
            None => UserAchievementsServicesStore::shows_initial_skeleton(cx),
        }
    }

    fn achievement_cell_side(available_width: f32) -> f32 {
        let insets = (GroupedCardMetrics::HORIZONTAL_PADDING + 16.0) * 2.0;
        let spacing = Self::ACHIEVEMENTS_GRID_SPACING * (Self::ACHIEVEMENTS_GRID_COLUMNS as f32 - 1.0);
        let available = available_width - insets - spacing;
        (available / Self::ACHIEVEMENTS_GRID_COLUMNS as f32).floor().max(44.0)
    }

    fn normalized_achievements_user_id(&self) -> Option<&str> {
        self.achievements_user_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|value| !value.is_empty())
    }

    fn load_achievements(user_id: Option<&str>, cx: &mut App) {
        UserAchievementsInteractionsStore::shared(cx).update(cx, |store, cx| store.open_achievements(user_id, cx));
    }

    fn localized_gender(raw: Option<&str>) -> Option<String> {
        match trimming_whitespaces_and_newlines(raw.unwrap_or("")).to_lowercase().as_str() {
            "male" => Some(tr("gender_male")),
            "female" => Some(tr("gender_female")),
            "other" => Some(tr("gender_other")),
            _ => None,
        }
    }

    fn own_phone_row(phone: &str, revealed: &Binding<bool>, theme: &AppTheme, cx: &App) -> GroupedBtn {
        let is_revealed = revealed.get(cx);
        let formatted = PhoneNumberFormatting::format(phone);
        let shown = if is_revealed { formatted.clone() } else { RiseFormat::masked_phone(phone) };
        let label = if is_revealed { tr("contact_phone_hide_action") } else { tr("contact_phone_show_action") };
        let toggle = revealed.clone();
        let eye = div()
            .id("profile-about-phone-eye")
            .aria_label(label.clone())
            .tooltip_label(label)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                plain("profile-about-phone-eye-button")
                    .size(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_press(move |window, cx| {
                        let shows = toggle.get(cx);
                        toggle.set(!shows, window, cx);
                    })
                    .children(IconUi::sized(
                        if is_revealed { "eye.slash" } else { "eye" },
                        px(15.0),
                        theme.text.secondary,
                        false,
                    )),
            );
        let row = GroupedBtn::new("phone", shown)
            .icon("phone.fill")
            .icon_background(UIColor::system_green().resolved_color(theme.appearance).opacity(0.85))
            .pretitle(tr("profile_field_phone"))
            .show_chevron(false)
            .trailing_slot(eye);
        if is_revealed { row.callback(move |_, cx| Self::copy(&formatted, cx)) } else { row }
    }

    fn veiled_row(
        id: &'static str,
        icon: &'static str,
        icon_background: Hsla,
        pretitle: String,
        rule: RisePrivacyRule,
        theme: &AppTheme,
    ) -> GroupedBtn {
        GroupedBtn::new(id, tr("desktop_streamer_mode_hidden"))
            .icon(icon)
            .icon_background(icon_background)
            .pretitle(pretitle)
            .show_chevron(false)
            .trailing_slot(RiseRTCStreamerMode::reveal_button(gpui::SharedString::from(format!("profile-about-{id}-show")), rule, theme))
    }

    fn copy(value: &str, cx: &mut App) {
        UIPasteboard::general().set_string(value, cx);
        Notifier::copied(cx);
    }
}

impl RenderOnce for ProfileAboutSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let host = window.current_view();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, move |_, cx: &mut Context<ProfileAboutSheetState>| {
            let presentation = RiseUserAchievementsPresentation::shared(cx);
            ProfileAboutSheetState {
                available_width: None,
                description_expanded: false,
                phone_revealed: false,
                _presentation_observation: cx.observe(&presentation, move |_, _, cx| {
                    cx.notify();
                    App::notify(cx, host);
                }),
            }
        });
        let available_width = state.read(cx).available_width.unwrap_or_else(|| {
            ModalSheetFrame::for_window(window, cx).width - ModalSheetMetrics::CONTENT_INSET * 2.0
        });
        let actions = ProfileAboutSheetActions {
            is_presented: self.is_presented.clone(),
            achievements_user_id: self.achievements_user_id.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
            description_expanded: Binding::entity(
                state.clone(),
                |state: &ProfileAboutSheetState| state.description_expanded,
                |state: &mut ProfileAboutSheetState, expanded, _| state.description_expanded = expanded,
            ),
            phone_revealed: Binding::entity(
                state.clone(),
                |state: &ProfileAboutSheetState| state.phone_revealed,
                |state: &mut ProfileAboutSheetState, revealed, _| state.phone_revealed = revealed,
            ),
        };
        let rows = self.rows(&actions, available_width, cx);
        let load_user_id: Option<Rc<str>> = self.normalized_achievements_user_id().map(Rc::from);
        let appear_id = ElementId::NamedChild(
            std::sync::Arc::new(self.id.clone()),
            gpui::SharedString::from(format!("appear-{}", load_user_id.as_deref().unwrap_or(""))),
        );
        let measured = state;

        vstack(0.0)
            .relative()
            .w_full()
            .bg(background)
            .child(measure(child_id(&self.id, "width"), move |bounds, _, cx| {
                measured.update(cx, |state, cx| {
                    state.available_width = Some(f32::from(bounds.size.width));
                    cx.notify();
                });
                cx.notify(host);
            }))
            .child(
                hstack(0.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(MainText::new(tr("profile_about_title")).px(17.0).font_weight(FontWeight::SEMIBOLD))
                    .child(spacer_min_w(0.0)),
            )
            .child(div().w_full().pt(px(4.0)).pb(px(24.0)).child(GroupedBtns::new(child_id(&self.id, "rows"), rows)))
            .on_appear(appear_id, move |_, cx| {
                Self::load_achievements(load_user_id.as_deref(), cx);
            })
    }
}

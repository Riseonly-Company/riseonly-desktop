use gpui::{App, Context, Entity, FontWeight, SharedString, Subscription, Window};
use rise_i18n::tr;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_theme::AppTheme;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::section_reveal_ui::SectionRevealUi;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::components::premium_badge_picker::premium_badge_emoji_picker::PremiumBadgeEmojiPicker;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const MAX_PICKER_VIEWPORT_HEIGHT: f32 = 376.0;
const PICKER_VIEWPORT_SCREEN_SHARE: f32 = 0.44;

struct PremiumBadgeSettingsState {
    picker_open: bool,
    _observations: [Subscription; 2],
}

impl PremiumBadgeSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let user_service = UserServicesStore::shared(cx);
        Self {
            picker_open: false,
            _observations: [
                cx.observe(&auth, |_, _, cx| cx.notify()),
                cx.observe(&user_service, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement, Default)]
pub struct PremiumBadgeSettings;

impl PremiumBadgeSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PremiumBadgeSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("premium-badge-settings-state", cx, |_, cx| PremiumBadgeSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);

        SettingsWrapperUi::new(tr("settings_premium_badge_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PremiumBadgeSettingsBody { state })
        .on_appear("premium-badge-settings-appear", |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_premium_badge_failure(cx));
        })
    }
}

#[derive(IntoElement)]
struct PremiumBadgeSettingsBody {
    state: Entity<PremiumBadgeSettingsState>,
}

impl PremiumBadgeSettingsBody {
    fn preview_section(name: String, badge_emoji: Option<String>, theme: &AppTheme) -> impl IntoElement {
        vstack(10.0)
            .w_full()
            .child(
                div().w_full().px(px(12.0)).child(
                    hstack(8.0)
                        .w_full()
                        .justify_center()
                        .py(px(26.0))
                        .bg(theme.bg._200)
                        .continuous_corners(22.0)
                        .child(MainText::new(name).px(22.0).font_weight(FontWeight::BOLD).number_of_lines(1))
                        .child(PremiumBadge::new().emoji(badge_emoji.map(SharedString::from)).size(24.0)),
                ),
            )
            .child(
                div()
                    .w_full()
                    .px(px(16.0))
                    .child(SecondaryText::new(tr("settings_premium_badge_hint")).px(14.0).tac(TextAlignT::Left)),
            )
    }

    fn selection_trailing(pending: bool, badge_emoji: Option<String>, theme: &AppTheme) -> impl IntoElement {
        hstack(8.0)
            .when(pending, |stack| {
                stack.child(progress_view().control_size(ControlSize::Small).tint(theme.text.secondary))
            })
            .child(PremiumBadge::new().emoji(badge_emoji.map(SharedString::from)).size(26.0))
    }

    fn emoji_picker(state: &Entity<PremiumBadgeSettingsState>, viewport_height: f32, picker_open: bool) -> impl IntoElement {
        let committed = state.clone();
        SectionRevealUi::new(
            "premium-badge-picker-reveal",
            picker_open,
            viewport_height + 8.0,
            div().w_full().px(px(8.0)).pb(px(8.0)).child(div().w_full().h(px(viewport_height)).child(
                PremiumBadgeEmojiPicker::new().on_committed(move |_, cx| {
                    committed.update(cx, |state, cx| {
                        state.picker_open = false;
                        cx.notify();
                    });
                }),
            )),
        )
    }
}

impl RenderOnce for PremiumBadgeSettingsBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = self.state;
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile();
        let name = profile.map(|profile| profile.name.clone()).unwrap_or_default();
        let badge_emoji = profile.and_then(|profile| profile.more.premium_badge_emoji.clone());
        let has_custom_badge = !badge_emoji.as_deref().unwrap_or_default().is_empty();
        let user_service = UserServicesStore::shared(cx);
        let mutation_pending = user_service.read(cx).premium_badge_mutation_pending;
        let mutation_failed = user_service.read(cx).premium_badge_mutation_failed;
        let picker_open = state.read(cx).picker_open;
        let viewport_height =
            MAX_PICKER_VIEWPORT_HEIGHT.min(f32::from(window.viewport_size().height) * PICKER_VIEWPORT_SCREEN_SHARE);

        let toggled = state.clone();
        let mut items = vec![
            GroupedBtn::new("premium_badge_picker", tr("settings_premium_badge_choose"))
                .trailing_slot(Self::selection_trailing(mutation_pending, badge_emoji.clone(), &theme))
                .expanded_slot(Self::emoji_picker(&state, viewport_height, picker_open))
                .callback(move |_, cx| {
                    let _ = SelectionFeedbackGenerator::new().selection_changed();
                    toggled.update(cx, |state, cx| {
                        state.picker_open = !state.picker_open;
                        cx.notify();
                    });
                }),
        ];
        if has_custom_badge {
            items.push(
                GroupedBtn::new("premium_badge_reset", tr("settings_premium_badge_reset"))
                    .icon("arrow.uturn.backward")
                    .icon_background(SettingsIconPalette::privacy_messages())
                    .show_chevron(false)
                    .callback(|_, cx| {
                        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.set_premium_badge(None, cx));
                    }),
            );
        }

        scroll_view("premium-badge-settings-scroll")
            .size_full()
            .bg(theme.bg._100)
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(16.0)
                    .w_full()
                    .pt(px(12.0))
                    .child(Self::preview_section(name, badge_emoji, &theme))
                    .child(
                        vstack(8.0)
                            .w_full()
                            .align(HorizontalAlignment::Leading)
                            .child(GroupedBtns::new("premium-badge-selection", items).corner_radius(24.0))
                            .when(mutation_failed, |stack| {
                                stack.child(
                                    div().w_full().px(px(16.0)).child(
                                        MainText::new(tr("settings_premium_badge_save_failed"))
                                            .px(13.0)
                                            .color(theme.semantic.error_100),
                                    ),
                                )
                            }),
                    )
                    .child(div().flex_none().h(px(80.0))),
            )
    }
}

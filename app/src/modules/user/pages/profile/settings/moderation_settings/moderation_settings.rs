use gpui::{App, Context, Entity, Subscription, Window};
use rise_i18n::tr;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::moderation::engine::rise_moderation_presentation::{ModerationLoadState, RiseModerationPresentation};
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportStatus;
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

struct ModerationSettingsState {
    _subscriptions: Vec<Subscription>,
}

impl ModerationSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let roles = ModerationServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let presentation = RiseModerationPresentation::shared(cx);
        Self {
            _subscriptions: vec![
                cx.observe(&roles, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
            ],
        }
    }
}

fn push_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

#[derive(IntoElement)]
pub struct ModerationSettings;

impl ModerationSettings {
    pub fn new() -> Self {
        Self
    }

    fn staff_section(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) -> Option<AnyElement> {
        let items = Self::staff_items(nav_store, cx);
        if items.is_empty() {
            return None;
        }
        Some(
            div()
                .w_full()
                .child(GroupedBtns::new("moderation-settings-staff", items).title(tr("moderation_section_staff")))
                .into_any_element(),
        )
    }

    fn staff_items(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) -> Vec<GroupedBtn> {
        let mut items = Vec::new();
        if ModerationServicesStore::can_enter_moderation(cx) {
            let presentation = RiseModerationPresentation::shared(cx);
            let presentation = presentation.read(cx);
            let open_count = (matches!(presentation.queue_state(), ModerationLoadState::Loaded)
                && presentation.queue_filter().status == ReportStatus::Active)
                .then(|| RiseFormat::compact_count(presentation.queue_total()));
            let nav_store = nav_store.clone();
            let mut item = GroupedBtn::new("reportsQueue", tr("moderation_queue_title"))
                .icon("tray.full.fill")
                .icon_background(SettingsIconPalette::moderation_become())
                .subtitle(tr("moderation_queue_subtitle"))
                .callback(move |_, cx| push_route(&nav_store, RootRoute::ReportsQueue, cx));
            if let Some(open_count) = open_count {
                item = item.trailing(open_count);
            }
            items.push(item);
        }
        if ModerationServicesStore::can_resolve_appeals(cx) {
            let nav_store = nav_store.clone();
            items.push(
                GroupedBtn::new("appealsReview", tr("appeals_review_title"))
                    .icon("arrow.uturn.backward.circle.fill")
                    .icon_background(SettingsIconPalette::moderation_appeals())
                    .subtitle(tr("appeals_review_subtitle"))
                    .callback(move |_, cx| push_route(&nav_store, RootRoute::AppealsReview, cx)),
            );
        }
        if ModerationServicesStore::is_admin(cx) {
            let nav_store = nav_store.clone();
            items.push(
                GroupedBtn::new("allModerators", tr("moderator_all_title"))
                    .icon("person.3.fill")
                    .icon_background(SettingsIconPalette::moderation_become())
                    .subtitle(tr("moderator_all_subtitle"))
                    .callback(move |_, cx| push_route(&nav_store, RootRoute::AllModerators, cx)),
            );
        }
        items
    }

    fn personal_section(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) -> AnyElement {
        let presentation = RiseModerationPresentation::shared(cx);
        let active_blocks = presentation.read(cx).active_blocks().len() as i64;
        let reports_nav_store = nav_store.clone();
        let blocks_nav_store = nav_store.clone();
        let mut my_blocks = GroupedBtn::new("myBlocks", tr("blocks_page_title"))
            .icon("hand.raised.fill")
            .icon_background(SettingsIconPalette::blocks())
            .subtitle(tr("blocks_page_subtitle"))
            .callback(move |_, cx| push_route(&blocks_nav_store, RootRoute::MyBlocks, cx));
        if active_blocks > 0 {
            my_blocks = my_blocks.trailing(RiseFormat::compact_count(active_blocks));
        }
        let items = vec![
            GroupedBtn::new("myReports", tr("report_my_reports_title"))
                .icon("flag.fill")
                .icon_background(SettingsIconPalette::moderation_become())
                .subtitle(tr("report_my_reports_subtitle"))
                .callback(move |_, cx| push_route(&reports_nav_store, RootRoute::MyReports, cx)),
            my_blocks,
        ];
        div()
            .w_full()
            .child(GroupedBtns::new("moderation-settings-personal", items).title(tr("moderation_section_personal")))
            .into_any_element()
    }
}

impl Default for ModerationSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for ModerationSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let _state = window.use_keyed_state("moderation-settings-state", cx, |_, cx| ModerationSettingsState::new(cx));
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let can_enter_moderation = ModerationServicesStore::can_enter_moderation(cx);
        use_task("moderation-settings-queue", can_enter_moderation, window, cx, async move |cx| {
            if !can_enter_moderation {
                return;
            }
            let _ = cx.update(|_, cx| {
                ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_queue_loaded(cx));
            });
        });

        let staff = Self::staff_section(&nav_store, cx);
        let personal = Self::personal_section(&nav_store, cx);
        let sections = vstack(14.0).w_full().pt(px(12.0)).children(staff).child(personal).child(div().min_h(px(80.0)));

        SettingsWrapperUi::new(tr("settings_moderations_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("moderation-settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(sections),
        )
        .on_appear("moderation-settings-appear", |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_my_profile_loaded(cx));
            ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.ensure_moderator_profile_loaded(cx);
                store.ensure_blocks_loaded(cx);
            });
        })
    }
}

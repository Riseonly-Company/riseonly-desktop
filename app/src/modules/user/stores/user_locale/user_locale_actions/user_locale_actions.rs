use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::rise_request;

use crate::modules::user::engine::rise_user_domain::RiseUserActionExecutor;

pub struct UserLocaleActionsStore {}

impl UserLocaleActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn load_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.load_locale_settings(force).await;
        })
        .detach();
    }

    pub fn update_region_action(&mut self, region: String, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let _ = domain
                .update_locale_settings(rise_request::user::UpdateLocaleSettings {
                    region_code: Some(region),
                    ..Default::default()
                })
                .await;
        })
        .detach();
    }

    pub fn update_app_language_action(&mut self, language: String, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let _ = domain
                .update_locale_settings(rise_request::user::UpdateLocaleSettings {
                    app_language: Some(language),
                    ..Default::default()
                })
                .await;
        })
        .detach();
    }

    pub fn update_notification_language_action(&mut self, language: Option<String>, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let follows_app = if language.is_none() { Some(true) } else { None };
            let _ = domain
                .update_locale_settings(rise_request::user::UpdateLocaleSettings {
                    notification_language: language,
                    notification_language_follows_app: follows_app,
                    ..Default::default()
                })
                .await;
        })
        .detach();
    }
}

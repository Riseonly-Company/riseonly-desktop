use gpui::{AnyView, Context, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListReusableView;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListHostedEnvironment, PostListHostedEnvironmentModifiers,
};

pub struct PostListHeaderRowView {
    hosted_view: Option<AnyView>,
    environment: Option<PostListHostedEnvironment>,
    installed_id: Option<String>,
    installed_generation: Option<u64>,
}

impl PostListHeaderRowView {
    pub const REUSE_IDENTIFIER: &'static str = "post.header";

    pub fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        Self { hosted_view: None, environment: None, installed_id: None, installed_generation: None }
    }

    pub fn configure(
        &mut self,
        id: &str,
        generation: u64,
        content: impl FnOnce(&mut Context<Self>) -> AnyView,
        environment: &PostListHostedEnvironment,
        cx: &mut Context<Self>,
    ) {
        if self.hosted_view.is_some()
            && self.installed_id.as_deref() == Some(id)
            && self.installed_generation == Some(generation)
        {
            return;
        }
        self.installed_id = Some(id.to_owned());
        self.installed_generation = Some(generation);
        self.hosted_view = Some(content(cx));
        self.environment = Some(environment.clone());
        cx.notify();
    }
}

impl RiseManualListReusableView for PostListHeaderRowView {
    fn prepare_for_manual_list_reuse(&mut self, _cx: &mut Context<Self>) {}
}

impl Render for PostListHeaderRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let hosted = match (&self.hosted_view, &self.environment) {
            (Some(view), Some(environment)) => {
                Some(view.clone().applying_post_list_hosted_environment(environment))
            }
            _ => None,
        };
        div().size_full().children(hosted)
    }
}

use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::{rise_request, rise_wire_methods};

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsRequestOptions, SaiWsRequestOptionsInit};

pub struct PostWatchActionsStore {
    pub posts_watch_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub posts_unwatch_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
}

impl PostWatchActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self { posts_watch_sai: SaiTypedSlice::new(cx), posts_unwatch_sai: SaiTypedSlice::new(cx) }
    }

    pub fn watch_action(&mut self, post_ids: Vec<i64>, cx: &mut Context<Self>) {
        if post_ids.is_empty() {
            return;
        }
        SaiWs::request_slice(
            &self.posts_watch_sai,
            rise_request::gateway::PostsWatch { post_ids: Some(post_ids) },
            Self::options(rise_wire_methods::gateway::POSTS_WATCH),
            Some(Self::ignore_success()),
            Some(Self::ignore_error()),
            cx,
        );
    }

    pub fn unwatch_action(&mut self, post_ids: Vec<i64>, cx: &mut Context<Self>) {
        if post_ids.is_empty() {
            return;
        }
        SaiWs::request_slice(
            &self.posts_unwatch_sai,
            rise_request::gateway::PostsUnwatch { post_ids: Some(post_ids) },
            Self::options(rise_wire_methods::gateway::POSTS_UNWATCH),
            Some(Self::ignore_success()),
            Some(Self::ignore_error()),
            cx,
        );
    }

    fn options(method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::gateway::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn ignore_success() -> SaiWsOnSuccess {
        Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {})
    }

    fn ignore_error() -> SaiWsOnError {
        Rc::new(|_: &anyhow::Error, _: &mut App| {})
    }
}

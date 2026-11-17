use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_request::event::TrackEvents;
use rise_canon::rise_wire_methods;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementEvent;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;

pub struct EngagementActionsStore {
    pub track_events_sai: Rc<SaiTypedSlice<rise_canon::rise_response::event::TrackEvents>>,
}

impl EngagementActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            track_events_sai: SaiTypedSlice::new(cx),
        }
    }

    pub fn track_events_action(
        &self,
        events: &[EngagementEvent],
        session_id: &str,
        on_complete: impl Fn(bool, &mut App) + 'static,
        cx: &mut App,
    ) {
        if events.is_empty() {
            on_complete(true, cx);
            return;
        }

        let body = TrackEvents {
            device: Some(RiseReleaseNotesDeviceContext::platform().to_owned()),
            events: Some(events.iter().map(EngagementEvent::wire_event).collect()),
            session_id: Some(session_id.to_owned()),
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::event::SERVICE.to_owned(),
            method: rise_wire_methods::event::TRACK_EVENTS.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_complete: Rc<dyn Fn(bool, &mut App)> = Rc::new(on_complete);
        let on_success_complete = Rc::clone(&on_complete);
        let on_success: SaiWsOnSuccess =
            Rc::new(move |_response: Option<Box<dyn Any>>, cx: &mut App| on_success_complete(true, cx));
        let on_error: SaiWsOnError =
            Rc::new(move |_error: &anyhow::Error, cx: &mut App| on_complete(false, cx));

        SaiWs::request_slice(
            &self.track_events_sai,
            body,
            options,
            Some(on_success),
            Some(on_error),
            cx,
        );
    }
}

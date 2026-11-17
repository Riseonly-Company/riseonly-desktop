use std::any::Any;
use std::rc::Rc;

use gpui::{App, AsyncApp, Entity};
use rise_canon::RiseRequestBody;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsResponseDebtCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_cache_refresher::SaiWsCacheRefresherRule;
use crate::core::lib::sai::modules::sai_ws::sai_ws_cache_refresher_coordinator::SaiWsCacheRefresherCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_coalescing::{
    SaiWsCoalescingOptions, SaiWsCoalescingRegistry,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_event_registry::{
    SaiWsEventHandlerRegistration, SaiWsEventRegistry,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationState, SaiWsPaginationStateStore,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;

const LOG_NAME: &str = "SaiWs";

/// completion for a send request, called once on the main thread
pub type SaiWsSendRequestCompletion = Box<dyn FnOnce(anyhow::Result<()>, &mut App)>;
/// success callback, gets the decoded slice payload or the raw value at the take path
pub type SaiWsOnSuccess = Rc<dyn Fn(Option<Box<dyn Any>>, &mut App)>;
/// error callback for a ws request
pub type SaiWsOnError = Rc<dyn Fn(&anyhow::Error, &mut App)>;

/// ws request entry points, all run on the main thread
/// label overloads carry the label in the name and async ones end in _async
pub enum SaiWs {}

impl SaiWs {
    pub fn manager(cx: &mut App) -> Entity<SaiWebSocketManager> {
        SaiWebSocketManager::shared(cx)
    }

    pub fn pagination_state(for_cache_key: &str, cx: &mut App) -> Entity<SaiWsPaginationState> {
        SaiWsPaginationStateStore::state(for_cache_key, cx)
    }

    pub fn is_offline_ack_debt_enabled() -> bool {
        SaiWsResponseDebtCoordinator::is_feature_enabled()
    }

    pub fn set_is_offline_ack_debt_enabled(new_value: bool) {
        SaiWsResponseDebtCoordinator::set_is_feature_enabled(new_value);
    }

    pub fn connect(access_token: &str, cx: &mut App) {
        Sai::log(LOG_NAME, || "connect(accessToken:)".to_owned(), SaiLogKind::Websocket, cx);
        Self::manager(cx).update(cx, |manager, cx| manager.connect(access_token, cx));
    }

    pub fn disconnect(cx: &mut App) {
        Sai::log(LOG_NAME, || "disconnect()".to_owned(), SaiLogKind::Websocket, cx);
        Self::manager(cx).update(cx, |manager, cx| manager.disconnect(cx));
    }

    pub fn connect_if_needed(access_token: Option<&str>, cx: &mut App) {
        Self::manager(cx).update(cx, |manager, cx| manager.connect_if_needed(access_token, cx));
    }

    pub fn set_application_active(active: bool, access_token: Option<String>, cx: &mut App) {
        Self::manager(cx)
            .update(cx, |manager, cx| manager.set_application_active(active, access_token, cx));
    }

    pub fn set_transport_hold(owner: &str, active: bool, cx: &mut App) {
        Self::manager(cx).update(cx, |manager, cx| manager.set_transport_hold(owner, active, cx));
    }

    pub fn configure_cache_refresher(rules: Vec<SaiWsCacheRefresherRule>, cx: &mut App) {
        SaiWsCacheRefresherCoordinator::configure(rules, cx);
    }

    pub fn notify_active_screen_changed(cx: &mut App) {
        SaiWsCacheRefresherCoordinator::notify_active_screen_changed(cx);
    }

    pub fn record_coalesced_mutation(coalescing: &SaiWsCoalescingOptions, cx: &mut App) {
        SaiWsCoalescingRegistry::bump(coalescing, cx);
    }

    pub fn discard_coalesced_mutation(coalescing: &SaiWsCoalescingOptions, cx: &mut App) {
        SaiWsCoalescingRegistry::discard(coalescing, cx);
    }

    pub fn register_event_handlers(handlers: Vec<SaiWsEventHandlerRegistration>, cx: &mut App) {
        SaiWsEventRegistry::shared(cx).update(cx, |registry, cx| registry.register(handlers, cx));
    }

    pub fn register_event_handler(registration: SaiWsEventHandlerRegistration, cx: &mut App) {
        SaiWsEventRegistry::shared(cx)
            .update(cx, |registry, cx| registry.register(vec![registration], cx));
    }

    pub fn unregister_event_handler(r#type: &str, cx: &mut App) {
        SaiWsEventRegistry::shared(cx).update(cx, |registry, cx| registry.unregister(r#type, cx));
    }

    pub fn unregister_all_event_handlers(cx: &mut App) {
        SaiWsEventRegistry::shared(cx).update(cx, |registry, cx| registry.remove_all(cx));
    }

    pub fn request_raw_body(
        raw_body: Map<String, Value>,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let account_user_id = options.account_user_id.clone();
            let account_epoch = options.account_epoch.clone();
            let result = SaiWebSocketManager::send_service_call_raw_body(
                manager,
                raw_body,
                options,
                on_success,
                on_error.clone(),
                cx.clone(),
            )
            .await;
            if let Err(error) = result {
                cx.update(|cx| {
                    Self::report_request_failure(
                        &error,
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                        on_error.as_ref(),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    /// sends a request into a slice, the body knows its own service and action
    pub fn request_slice<T: DeserializeOwned + 'static, Body: RiseRequestBody + 'static>(
        slice: &Rc<SaiTypedSlice<T>>,
        body: Body,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let mut o = Self::bound::<Body>(options);
        o.sai_slice = Some(Self::slice_state(slice));
        Self::request(body, o, on_success, on_error, cx);
    }

    fn bound<Body: RiseRequestBody>(options: SaiWsRequestOptions) -> SaiWsRequestOptions {
        debug_assert!(
            options.method.is_empty() || options.method == Body::ACTION,
            "{}: options name {}.{}, the body belongs to {}.{}",
            Body::CONTRACT,
            options.service,
            options.method,
            Body::SERVICE,
            Body::ACTION
        );
        let mut o = options;
        o.service = Body::SERVICE.to_owned();
        o.method = Body::ACTION.to_owned();
        o
    }

    pub fn request_slice_encodable<T: DeserializeOwned + 'static, Body: Serialize + 'static>(
        slice: &Rc<SaiTypedSlice<T>>,
        body: Body,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let mut o = options;
        o.sai_slice = Some(Self::slice_state(slice));
        Self::request(body, o, on_success, on_error, cx);
    }

    pub fn request_slice_raw_body<T: DeserializeOwned + 'static>(
        slice: &Rc<SaiTypedSlice<T>>,
        raw_body: Map<String, Value>,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let mut o = options;
        o.sai_slice = Some(Self::slice_state(slice));
        Self::request_raw_body(raw_body, o, on_success, on_error, cx);
    }

    /// sends a serializable body as a request
    pub fn request<Body: Serialize + 'static>(
        body: Body,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let account_user_id = options.account_user_id.clone();
            let account_epoch = options.account_epoch.clone();
            let result = SaiWebSocketManager::send_service_call_body(
                manager,
                &body,
                options,
                on_success,
                on_error.clone(),
                cx.clone(),
            )
            .await;
            if let Err(error) = result {
                cx.update(|cx| {
                    Self::report_request_failure(
                        &error,
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                        on_error.as_ref(),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub fn request_raw_body_options_dictionary(
        raw_body: Map<String, Value>,
        options: Map<String, Value>,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let result = match Self::ws_options(&options) {
                Ok(mut parsed) => {
                    parsed.account_user_id = account_user_id.clone();
                    parsed.account_epoch = account_epoch.clone();
                    SaiWebSocketManager::send_service_call_raw_body(
                        manager,
                        raw_body,
                        parsed,
                        on_success,
                        on_error.clone(),
                        cx.clone(),
                    )
                    .await
                    .map(|_| ())
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                cx.update(|cx| {
                    Self::report_request_failure(
                        &error,
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                        on_error.as_ref(),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub fn request_options_dictionary<Body: Serialize + 'static>(
        body: Body,
        options: Map<String, Value>,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let result = match Self::ws_options(&options) {
                Ok(mut parsed) => {
                    parsed.account_user_id = account_user_id.clone();
                    parsed.account_epoch = account_epoch.clone();
                    SaiWebSocketManager::send_service_call_body(
                        manager,
                        &body,
                        parsed,
                        on_success,
                        on_error.clone(),
                        cx.clone(),
                    )
                    .await
                    .map(|_| ())
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                cx.update(|cx| {
                    Self::report_request_failure(
                        &error,
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                        on_error.as_ref(),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub fn request_service<Body: Serialize + 'static>(
        body: Body,
        service: &str,
        method: &str,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let options = Self::service_options(service, method, SaiWsRequestOptionsInit::default());
        Self::request(body, options, on_success, on_error, cx);
    }

    pub fn request_raw_body_service(
        raw_body: Map<String, Value>,
        service: &str,
        method: &str,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let options = Self::service_options(service, method, SaiWsRequestOptionsInit::default());
        Self::request_raw_body(raw_body, options, on_success, on_error, cx);
    }

    pub fn send_request<Body: Serialize + 'static>(
        body: Body,
        options: SaiWsRequestOptions,
        completion: Option<SaiWsSendRequestCompletion>,
        cx: &mut App,
    ) {
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let account_user_id = options.account_user_id.clone();
            let account_epoch = options.account_epoch.clone();
            let result = SaiWebSocketManager::send_service_call_body(
                manager,
                &body,
                options,
                None,
                None,
                cx.clone(),
            )
            .await
            .map(|_| ());
            cx.update(|cx| {
                Self::finish_send_request(
                    result,
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                    completion,
                    cx,
                );
            });
        })
        .detach();
    }

    pub fn send_request_raw_body(
        raw_body: Map<String, Value>,
        options: SaiWsRequestOptions,
        completion: Option<SaiWsSendRequestCompletion>,
        cx: &mut App,
    ) {
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let account_user_id = options.account_user_id.clone();
            let account_epoch = options.account_epoch.clone();
            let result = SaiWebSocketManager::send_service_call_raw_body(
                manager,
                raw_body,
                options,
                None,
                None,
                cx.clone(),
            )
            .await
            .map(|_| ());
            cx.update(|cx| {
                Self::finish_send_request(
                    result,
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                    completion,
                    cx,
                );
            });
        })
        .detach();
    }

    pub fn send_request_options_dictionary<Body: Serialize + 'static>(
        body: Body,
        options: Map<String, Value>,
        completion: Option<SaiWsSendRequestCompletion>,
        cx: &mut App,
    ) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let result = match Self::ws_options(&options) {
                Ok(mut parsed) => {
                    parsed.account_user_id = account_user_id.clone();
                    parsed.account_epoch = account_epoch.clone();
                    SaiWebSocketManager::send_service_call_body(
                        manager,
                        &body,
                        parsed,
                        None,
                        None,
                        cx.clone(),
                    )
                    .await
                    .map(|_| ())
                }
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                Self::finish_send_request(
                    result,
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                    completion,
                    cx,
                );
            });
        })
        .detach();
    }

    pub fn send_request_raw_body_options_dictionary(
        raw_body: Map<String, Value>,
        options: Map<String, Value>,
        completion: Option<SaiWsSendRequestCompletion>,
        cx: &mut App,
    ) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let manager = Self::manager(cx);
        cx.spawn(async move |cx| {
            let result = match Self::ws_options(&options) {
                Ok(mut parsed) => {
                    parsed.account_user_id = account_user_id.clone();
                    parsed.account_epoch = account_epoch.clone();
                    SaiWebSocketManager::send_service_call_raw_body(
                        manager,
                        raw_body,
                        parsed,
                        None,
                        None,
                        cx.clone(),
                    )
                    .await
                    .map(|_| ())
                }
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                Self::finish_send_request(
                    result,
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                    completion,
                    cx,
                );
            });
        })
        .detach();
    }

    pub fn send_request_service<Body: Serialize + 'static>(
        body: Body,
        service: &str,
        method: &str,
        args: SaiWsRequestOptionsInit,
        completion: Option<SaiWsSendRequestCompletion>,
        cx: &mut App,
    ) {
        let options = Self::service_options(service, method, args);
        Self::send_request(body, options, completion, cx);
    }

    pub fn send_request_raw_body_service(
        raw_body: Map<String, Value>,
        service: &str,
        method: &str,
        args: SaiWsRequestOptionsInit,
        completion: Option<SaiWsSendRequestCompletion>,
        cx: &mut App,
    ) {
        let options = Self::service_options(service, method, args);
        Self::send_request_raw_body(raw_body, options, completion, cx);
    }

    pub async fn request_slice_async<T: DeserializeOwned + 'static, Body: Serialize>(
        slice: &Rc<SaiTypedSlice<T>>,
        body: &Body,
        options: SaiWsRequestOptions,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let mut o = options;
        o.sai_slice = Some(Self::slice_state(slice));
        Self::request_async(body, o, cx).await
    }

    pub async fn request_slice_raw_body_async<T: DeserializeOwned + 'static>(
        slice: &Rc<SaiTypedSlice<T>>,
        raw_body: Map<String, Value>,
        options: SaiWsRequestOptions,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let mut o = options;
        o.sai_slice = Some(Self::slice_state(slice));
        Self::request_raw_body_async(raw_body, o, cx).await
    }

    /// async version of request, awaits the result
    pub async fn request_async<Body: Serialize>(
        body: &Body,
        options: SaiWsRequestOptions,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let manager = cx.update(Self::manager);
        SaiWebSocketManager::send_service_call_body(manager, body, options, None, None, cx)
            .await
            .map(|_| ())
    }

    pub async fn request_raw_body_async(
        raw_body: Map<String, Value>,
        options: SaiWsRequestOptions,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let manager = cx.update(Self::manager);
        SaiWebSocketManager::send_service_call_raw_body(manager, raw_body, options, None, None, cx)
            .await
            .map(|_| ())
    }

    pub async fn request_options_dictionary_async<Body: Serialize>(
        body: &Body,
        options: &Map<String, Value>,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let parsed = Self::ws_options(options)?;
        Self::request_async(body, parsed, cx).await
    }

    pub async fn request_raw_body_options_dictionary_async(
        raw_body: Map<String, Value>,
        options: &Map<String, Value>,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let parsed = Self::ws_options(options)?;
        Self::request_raw_body_async(raw_body, parsed, cx).await
    }

    pub async fn request_service_async<Body: Serialize>(
        body: &Body,
        service: &str,
        method: &str,
        args: SaiWsRequestOptionsInit,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let options = Self::service_options(service, method, args);
        Self::request_async(body, options, cx).await
    }

    pub async fn request_raw_body_service_async(
        raw_body: Map<String, Value>,
        service: &str,
        method: &str,
        args: SaiWsRequestOptionsInit,
        cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let options = Self::service_options(service, method, args);
        Self::request_raw_body_async(raw_body, options, cx).await
    }

    fn service_options(
        service: &str,
        method: &str,
        args: SaiWsRequestOptionsInit,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: service.to_owned(),
            method: method.to_owned(),
            ..args
        })
    }

    fn slice_state<T: DeserializeOwned + 'static>(slice: &Rc<SaiTypedSlice<T>>) -> Rc<dyn SaiSliceState> {
        slice.clone()
    }

    fn ws_options(options: &Map<String, Value>) -> anyhow::Result<SaiWsRequestOptions> {
        Ok(SaiWsRequestOptions::from_options_dictionary(options)?)
    }

    fn report_request_failure(
        error: &anyhow::Error,
        user_id: Option<&str>,
        epoch: Option<&str>,
        on_error: Option<&SaiWsOnError>,
        cx: &mut App,
    ) {
        if !Self::is_current_account(user_id, epoch) {
            return;
        }
        if let Some(on_error) = on_error {
            on_error(error, cx);
        }
        let description = error.to_string();
        Sai::log(
            LOG_NAME,
            move || format!("request failed: {description}"),
            SaiLogKind::Websocket,
            cx,
        );
    }

    fn finish_send_request(
        result: anyhow::Result<()>,
        user_id: Option<&str>,
        epoch: Option<&str>,
        completion: Option<SaiWsSendRequestCompletion>,
        cx: &mut App,
    ) {
        if !Self::is_current_account(user_id, epoch) {
            return;
        }
        match result {
            Ok(()) => {
                if let Some(completion) = completion {
                    completion(Ok(()), cx);
                }
            }
            Err(error) => {
                let description = error.to_string();
                Sai::log(
                    LOG_NAME,
                    move || format!("sendRequest failed: {description}"),
                    SaiLogKind::Websocket,
                    cx,
                );
                if let Some(completion) = completion {
                    completion(Err(error), cx);
                }
            }
        }
    }

    fn is_current_account(user_id: Option<&str>, epoch: Option<&str>) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(user_id, epoch)
    }
}

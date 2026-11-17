use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_validate::TAG;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct TagValidationError {
    pub message: String,
}

impl TagValidationError {
    pub fn error_description(&self) -> Option<String> {
        Some(self.message.clone())
    }
}

pub type CheckTagExistGatewayDTO = rise_canon::rise_view::tag::TagAvailability;

pub type GlobalTagChatDTO = rise_canon::rise_proto::tag::SearchChatResult;
pub type GlobalTagUserDTO = rise_canon::rise_proto::tag::SearchUserResult;

pub type GlobalTagResultDTO = rise_canon::rise_proto::tag::SearchResultItem;
pub type GlobalTagSearchGatewayDTO = rise_canon::rise_view::tag::SearchPage;

pub struct TagActionsStore {
    pub check_tag_exist_sai: Rc<SaiTypedSlice<CheckTagExistGatewayDTO>>,
    pub global_tag_search_sai: Rc<SaiTypedSlice<GlobalTagSearchGatewayDTO>>,
}

impl TagActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            check_tag_exist_sai: SaiTypedSlice::new(cx),
            global_tag_search_sai: SaiTypedSlice::new(cx),
        }
    }

    pub fn check_tag_exist_action(
        &self,
        tag: &str,
        on_success: impl Fn(CheckTagExistGatewayDTO, &mut App) + 'static,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let trimmed = TAG.normalize(tag);
        if let Some(message) = TAG.error_text(&trimmed) {
            if let Some(on_error) = &on_error {
                on_error(&anyhow::Error::new(TagValidationError { message }), cx);
            }
            return;
        }

        let body = rise_canon::rise_request::tag::CheckTagExist {
            tag: Some(trimmed.clone()),
        };
        let cache_key = SaiCache::new(["checkTagExist", trimmed.as_str()]).key;
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::tag::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::tag::CHECK_TAG_EXIST.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_error_logged: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log(
                "Tag",
                || format!("check_tag_exist({trimmed}) error: {error}"),
                SaiLogKind::Network,
                cx,
            );
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });

        SaiWs::request_slice(
            &self.check_tag_exist_sai,
            body,
            options,
            Some(Self::decoded_success(on_success)),
            Some(on_error_logged),
            cx,
        );
    }

    pub fn resolve_public_tag_action(
        &self,
        tag: &str,
        on_success: impl Fn(GlobalTagSearchGatewayDTO, &mut App) + 'static,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        let normalized = trimming_prefix(trimming_whitespaces_and_newlines(tag), '@').to_lowercase();
        if normalized.is_empty() {
            return;
        }

        let body = rise_canon::rise_request::tag::GlobalSearchByTag {
            limit: Some(10),
            tag: Some(normalized),
            ..rise_canon::rise_request::tag::GlobalSearchByTag::default()
        };
        let on_error_forwarded: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(
            &self.global_tag_search_sai,
            body,
            SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
                service: rise_canon::rise_wire_methods::tag::SERVICE.to_owned(),
                method: rise_canon::rise_wire_methods::tag::GLOBAL_SEARCH_BY_TAG.to_owned(),
                reload_if_cached: false,
                allow_concurrent_while_pending: true,
                reflect_pending_in_ui: false,
                send_immediately: true,
                store_response_in_cache: false,
                persist_cache_to_disk: false,
                ..SaiWsRequestOptionsInit::default()
            }),
            Some(Self::decoded_success(on_success)),
            Some(on_error_forwarded),
            cx,
        );
    }

    fn decoded_success<T: DeserializeOwned + 'static>(
        on_success: impl Fn(T, &mut App) + 'static,
    ) -> SaiWsOnSuccess {
        Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(payload) = payload else {
                return;
            };
            match payload.downcast::<T>() {
                Ok(dto) => on_success(*dto, cx),
                Err(payload) => {
                    if let Some(dto) = SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()) {
                        on_success(dto, cx);
                    }
                }
            }
        })
    }
}

fn trimming_prefix(string: &str, prefix: char) -> &str {
    string.strip_prefix(prefix).unwrap_or(string)
}

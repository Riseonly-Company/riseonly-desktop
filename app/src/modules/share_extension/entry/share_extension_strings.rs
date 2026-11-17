use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode};
use rise_i18n::{FormatArg, format_printf, tr};

use crate::modules::share_extension::entry::rise_share_wire::RiseShareTransportError;
use crate::modules::share_extension::entry::share_extension_models::{
    ShareExtensionError, SharePartialDeliveryError,
};
use crate::modules::share_extension::shared::rise_share_container::RiseShareContainerError;

/// share layer strings, read per call because the app language can change while the layer is open
pub enum ShareStrings {}

impl ShareStrings {
    pub fn title() -> String {
        Self::value("share_title")
    }

    pub fn select_chats() -> String {
        Self::value("share_select_chats")
    }

    pub fn search() -> String {
        Self::value("share_search")
    }

    pub fn clear_search() -> String {
        Self::value("share_clear_search")
    }

    pub fn cancel() -> String {
        Self::value("share_cancel")
    }

    pub fn cancel_hint() -> String {
        Self::value("share_cancel_hint")
    }

    pub fn cancelled() -> String {
        Self::value("share_cancelled")
    }

    pub fn back() -> String {
        Self::value("share_back")
    }

    pub fn recent() -> String {
        Self::value("share_recent")
    }

    pub fn no_chats() -> String {
        Self::value("share_no_chats")
    }

    pub fn no_chats_detail() -> String {
        Self::value("share_no_chats_detail")
    }

    pub fn no_search_results() -> String {
        Self::value("share_no_search_results")
    }

    pub fn try_another_search() -> String {
        Self::value("share_try_another_search")
    }

    pub fn retry() -> String {
        Self::value("share_retry")
    }

    pub fn refreshing_chats() -> String {
        Self::value("share_refreshing_chats")
    }

    pub fn select_topic() -> String {
        Self::value("share_select_topic")
    }

    pub fn no_topics() -> String {
        Self::value("share_no_topics")
    }

    pub fn no_topics_detail() -> String {
        Self::value("share_no_topics_detail")
    }

    pub fn general() -> String {
        Self::value("share_general")
    }

    pub fn comment() -> String {
        Self::value("share_comment")
    }

    pub fn comment_hint() -> String {
        Self::value("share_comment_hint")
    }

    pub fn send() -> String {
        Self::value("share_send")
    }

    pub fn sending() -> String {
        Self::value("share_sending")
    }

    pub fn sent() -> String {
        Self::value("share_sent")
    }

    pub fn share_to_story() -> String {
        Self::value("share_to_story")
    }

    pub fn opening_story_editor() -> String {
        Self::value("share_opening_story_editor")
    }

    pub fn signed_out_title() -> String {
        Self::value("share_signed_out_title")
    }

    pub fn signed_out_story_message() -> String {
        Self::value("share_signed_out_story_message")
    }

    pub fn open_app_title() -> String {
        Self::value("share_open_app_title")
    }

    pub fn open_app_story_message() -> String {
        Self::value("share_open_app_story_message")
    }

    pub fn opening_app_for_large_share() -> String {
        Self::value("share_opening_app_large_share")
    }

    pub fn error_title() -> String {
        Self::value("share_error_title")
    }

    pub fn dismiss() -> String {
        Self::value("share_dismiss")
    }

    pub fn selected() -> String {
        Self::value("share_selected")
    }

    pub fn account() -> String {
        Self::value("share_account")
    }

    pub fn account_format() -> String {
        Self::value("share_account_format")
    }

    pub fn untitled_chat() -> String {
        Self::value("share_untitled_chat")
    }

    pub fn saved_messages() -> String {
        Self::value("share_saved_messages")
    }

    pub fn select_at_least_one() -> String {
        Self::value("share_select_at_least_one")
    }

    pub fn invalid_selection() -> String {
        Self::value("share_invalid_selection")
    }

    pub fn configuration_missing() -> String {
        Self::value("share_error_configuration_missing")
    }

    pub fn app_group_unavailable() -> String {
        Self::value("share_error_app_group_unavailable")
    }

    pub fn snapshot_unavailable() -> String {
        Self::value("share_error_snapshot_unavailable")
    }

    pub fn snapshot_invalid() -> String {
        Self::value("share_error_snapshot_invalid")
    }

    pub fn authentication_unavailable() -> String {
        Self::value("share_error_authentication_unavailable")
    }

    pub fn account_changed() -> String {
        Self::value("share_error_account_changed")
    }

    pub fn unsupported_content() -> String {
        Self::value("share_error_unsupported_content")
    }

    pub fn file_too_large() -> String {
        Self::value("share_error_file_too_large")
    }

    pub fn file_unavailable() -> String {
        Self::value("share_error_file_unavailable")
    }

    pub fn invalid_server_response() -> String {
        Self::value("share_error_invalid_server_response")
    }

    pub fn request_timed_out() -> String {
        Self::value("share_error_request_timed_out")
    }

    pub fn session_expired() -> String {
        Self::value("share_error_session_expired")
    }

    pub fn send_failed() -> String {
        Self::value("share_error_send_failed")
    }

    pub fn network_unavailable() -> String {
        Self::value("share_error_network_unavailable")
    }

    pub fn service_unavailable() -> String {
        Self::value("share_error_service_unavailable")
    }

    pub fn story_unavailable() -> String {
        Self::value("share_error_story_unavailable")
    }

    pub fn app_open_failed() -> String {
        Self::value("share_error_app_open_failed")
    }

    pub fn send_count(count: i64) -> String {
        Self::format("share_send_count", count)
    }

    pub fn selected_count(count: i64) -> String {
        Self::format("share_selected_count", count)
    }

    pub fn progress(percent: i64) -> String {
        Self::format("share_progress", percent)
    }

    pub fn too_many_items(maximum: i64) -> String {
        Self::format("share_error_too_many_items", maximum)
    }

    pub fn content_too_long(maximum: i64) -> String {
        Self::format("share_error_content_too_long", maximum)
    }

    pub fn comment_too_long(maximum: i64) -> String {
        Self::format("share_error_comment_too_long", maximum)
    }

    pub fn too_many_destinations(maximum: i64) -> String {
        Self::format("share_error_too_many_destinations", maximum)
    }

    pub fn too_many_destinations_title(maximum: i64) -> String {
        Self::format("share_error_too_many_destinations_title", maximum)
    }

    pub fn partial_delivery(delivered: i64, failed: i64) -> String {
        format_printf(
            &Self::value("share_partial_delivery"),
            &[FormatArg::Int(delivered), FormatArg::Int(failed)],
        )
    }

    fn format(key: &str, value: i64) -> String {
        format_printf(&Self::value(key), &[FormatArg::Int(value)])
    }

    fn value(key: &str) -> String {
        tr(key)
    }
}

/// turns errors into share layer messages
pub enum ShareErrorPresenter {}

impl ShareErrorPresenter {
    /// message for an error by downcasting it, no keychain branch on desktop since its one process
    pub fn message(error: &anyhow::Error) -> String {
        if let Some(partial) = error.downcast_ref::<SharePartialDeliveryError>() {
            return partial.localized_description();
        }
        if let Some(share) = error.downcast_ref::<ShareExtensionError>() {
            return share.localized_description();
        }
        if let Some(transport) = error.downcast_ref::<RiseShareTransportError>() {
            return transport.localized_description();
        }
        if let Some(url) = error.downcast_ref::<URLError>() {
            return match url.code {
                URLErrorCode::TimedOut => ShareStrings::request_timed_out(),
                URLErrorCode::NotConnectedToInternet
                | URLErrorCode::NetworkConnectionLost
                | URLErrorCode::CannotConnectToHost
                | URLErrorCode::CannotFindHost
                | URLErrorCode::DnsLookupFailed => ShareStrings::network_unavailable(),
                _ => ShareStrings::service_unavailable(),
            };
        }
        if error.downcast_ref::<serde_json::Error>().is_some() {
            return ShareStrings::invalid_server_response();
        }
        if let Some(container) = error.downcast_ref::<RiseShareContainerError>() {
            return match container {
                RiseShareContainerError::AppGroupUnavailable => ShareStrings::app_group_unavailable(),
                RiseShareContainerError::AccountMismatch => ShareStrings::account_changed(),
                RiseShareContainerError::InvalidFile => ShareStrings::file_unavailable(),
                _ => ShareStrings::snapshot_invalid(),
            };
        }
        ShareStrings::send_failed()
    }
}

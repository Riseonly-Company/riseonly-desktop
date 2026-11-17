use std::path::PathBuf;

use rise_canon::rise_upload::PIPELINE_MAXIMUM_FILE_BYTES;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_limits::service_limit::GET_SHARE_TARGETS_LIMIT_CLAMP;
use rise_platform::AppPaths;

use crate::core::config::app_environment::AppEnvironment;
use crate::modules::share_extension::entry::share_extension_models::ShareExtensionError;

/// settings of the share layer
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShareExtensionConfiguration {
    pub app_group_identifier: String,
    pub story_url_scheme: String,
    pub maximum_provider_count: i64,
    pub maximum_file_attachment_count: i64,
    pub maximum_file_byte_count: i64,
    pub maximum_content_length: i64,
    pub maximum_comment_length: i64,
    pub maximum_destination_count: i64,
    pub foreground_upload_byte_cap: i64,
    pub recent_destination_limit: i64,
    pub search_result_limit: i64,
}

impl ShareExtensionConfiguration {
    pub const APP_GROUP_INFO_KEY: &'static str = "RiseOnlyAppGroupIdentifier";
    pub const STORY_SCHEME_INFO_KEY: &'static str = "RiseOnlyStoryURLScheme";

    pub const DEFAULT_MAXIMUM_PROVIDER_COUNT: i64 = 64;
    pub const DEFAULT_MAXIMUM_FILE_ATTACHMENT_COUNT: i64 = 10;
    pub const DEFAULT_MAXIMUM_CONTENT_LENGTH: i64 = 4_096;
    pub const DEFAULT_MAXIMUM_COMMENT_LENGTH: i64 = 1_024;
    pub const DEFAULT_MAXIMUM_DESTINATION_COUNT: i64 = 20;
    pub const DEFAULT_FOREGROUND_UPLOAD_BYTE_CAP: i64 = 32 * 1_024 * 1_024;

    /// server page limit for share targets, deeper chats come through next_offset
    pub fn share_targets_page_limit() -> i64 {
        i64::try_from(GET_SHARE_TARGETS_LIMIT_CLAMP).unwrap_or(1)
    }

    pub fn default_maximum_file_byte_count() -> i64 {
        i64::try_from(PIPELINE_MAXIMUM_FILE_BYTES).unwrap_or(i64::MAX)
    }

    /// no defaults here, so pass the DEFAULT constants and the two canon readers or use with_defaults
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app_group_identifier: String,
        story_url_scheme: String,
        maximum_provider_count: i64,
        maximum_file_attachment_count: i64,
        maximum_file_byte_count: i64,
        maximum_content_length: i64,
        maximum_comment_length: i64,
        maximum_destination_count: i64,
        foreground_upload_byte_cap: i64,
        recent_destination_limit: i64,
        search_result_limit: i64,
    ) -> Self {
        Self {
            app_group_identifier,
            story_url_scheme,
            maximum_provider_count,
            maximum_file_attachment_count,
            maximum_file_byte_count,
            maximum_content_length,
            maximum_comment_length,
            maximum_destination_count,
            foreground_upload_byte_cap,
            recent_destination_limit,
            search_result_limit,
        }
    }

    pub fn with_defaults(app_group_identifier: String, story_url_scheme: String) -> Self {
        Self::new(
            app_group_identifier,
            story_url_scheme,
            Self::DEFAULT_MAXIMUM_PROVIDER_COUNT,
            Self::DEFAULT_MAXIMUM_FILE_ATTACHMENT_COUNT,
            Self::default_maximum_file_byte_count(),
            Self::DEFAULT_MAXIMUM_CONTENT_LENGTH,
            Self::DEFAULT_MAXIMUM_COMMENT_LENGTH,
            Self::DEFAULT_MAXIMUM_DESTINATION_COUNT,
            Self::DEFAULT_FOREGROUND_UPLOAD_BYTE_CAP,
            Self::share_targets_page_limit(),
            Self::share_targets_page_limit(),
        )
    }

    /// loads config from the compiled environment, no app group on desktop
    /// container is the apps own data dir
    pub fn load() -> Result<Self, ShareExtensionError> {
        let environment = AppEnvironment::current();
        let Some(app_group_identifier) = Self::resolved_value(Some(environment.as_str())) else {
            return Err(ShareExtensionError::ConfigurationMissing);
        };
        let Some(story_scheme) = Self::resolved_value(Some(environment.url_scheme())) else {
            return Err(ShareExtensionError::ConfigurationMissing);
        };
        Ok(Self::with_defaults(app_group_identifier, story_scheme))
    }

    pub fn shared_container_url(&self) -> Result<PathBuf, ShareExtensionError> {
        let environment = AppEnvironment::current();
        if environment.as_str() != self.app_group_identifier {
            return Err(ShareExtensionError::AppGroupUnavailable);
        }
        AppPaths::resolve(environment.canon())
            .map(|paths| paths.data_dir().to_path_buf())
            .map_err(|_| ShareExtensionError::AppGroupUnavailable)
    }

    fn resolved_value(value: Option<&str>) -> Option<String> {
        let value = value?;
        let trimmed = trimming_whitespaces_and_newlines(value);
        if trimmed.is_empty() || trimmed.contains("$(") {
            return None;
        }
        Some(trimmed.to_owned())
    }
}

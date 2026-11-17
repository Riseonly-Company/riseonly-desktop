use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::BackgroundExecutor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_access::{SecurityScopedAccess, volume_available_capacity_for_important_usage};
use rise_platform::file_manager::{FileAttributeType, FileManager};
use rise_platform::uniform_type::UTType;

use crate::modules::share_extension::entry::share_extension_configuration::ShareExtensionConfiguration;
use crate::modules::share_extension::entry::share_extension_models::{
    ShareAttachment, ShareAttachmentKind, ShareExtensionError, SharePayload,
};
use crate::modules::share_extension::entry::share_temporary_file_store::{ShareFileName, ShareTemporaryFileStore};

const TEXT_URI_LIST_MIME_TYPE: &str = "text/uri-list";
const TEXT_PLAIN_MIME_TYPE: &str = "text/plain";
const OCTET_STREAM_MIME_TYPE: &str = "application/octet-stream";
const FREE_SPACE_RESERVE_BYTES: i64 = 50 * 1_024 * 1_024;

/// one shared file or one piece of text
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareItemProvider {
    File { path: PathBuf, suggested_name: Option<String> },
    Text(String),
}

impl ShareItemProvider {
    pub fn registered_type_identifiers(&self) -> Vec<String> {
        match self {
            Self::File { path, .. } => {
                let mut identifiers = Vec::new();
                if let Some(content_type) = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .and_then(UTType::from_filename_extension)
                {
                    identifiers.push(content_type.identifier());
                }
                identifiers.push(UTType::file_url().identifier());
                identifiers
            }
            Self::Text(text) => {
                let mut identifiers = Vec::new();
                if ShareItemLoader::is_web_url(trimming_whitespaces_and_newlines(text)) {
                    identifiers.push(UTType::url().identifier());
                }
                identifiers.push(UTType::plain_text().identifier());
                identifiers
            }
        }
    }

    pub fn suggested_name(&self) -> Option<String> {
        match self {
            Self::File { suggested_name, .. } => suggested_name.clone(),
            Self::Text(_) => None,
        }
    }
}

/// providers of one shared item plus its text
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareExtensionItem {
    pub attachments: Option<Vec<ShareItemProvider>>,
    pub attributed_content_text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShareItemLoaderCandidateKind {
    File,
    Url,
    Text,
}

#[derive(Clone, Debug)]
struct ShareItemLoaderCandidate {
    kind: ShareItemLoaderCandidateKind,
    r#type: UTType,
    identifier: String,
}

#[derive(Clone, Debug)]
struct ShareItemLoaderLoadedFile {
    url: PathBuf,
    name: String,
    byte_count: i64,
    kind: ShareAttachmentKind,
    type_identifier: String,
    mime_type: String,
}

/// loads shared items into files and text
pub struct ShareItemLoader {
    configuration: ShareExtensionConfiguration,
    temporary_file_store: Arc<ShareTemporaryFileStore>,
}

impl ShareItemLoader {
    pub fn new(configuration: ShareExtensionConfiguration, temporary_file_store: Arc<ShareTemporaryFileStore>) -> Self {
        Self {
            configuration,
            temporary_file_store,
        }
    }

    /// dropping the future cancels the load, a copy already running finishes into the session folder
    /// the layer removes that folder on close
    pub async fn load(
        &self,
        executor: BackgroundExecutor,
        input_items: Vec<ShareExtensionItem>,
    ) -> anyhow::Result<SharePayload> {
        let extension_items = input_items;
        let providers: Vec<ShareItemProvider> = extension_items
            .iter()
            .flat_map(|item| item.attachments.clone().unwrap_or_default())
            .collect();
        let has_attributed_text = extension_items
            .iter()
            .any(|item| item.attributed_content_text.as_deref().is_some_and(|text| !text.is_empty()));
        if providers.is_empty() && !has_attributed_text {
            return Err(ShareExtensionError::UnsupportedContent.into());
        }
        if Self::count(providers.len()) > self.configuration.maximum_provider_count {
            return Err(ShareExtensionError::TooManyItems(self.configuration.maximum_provider_count).into());
        }

        let provider_candidates: Vec<(ShareItemProvider, ShareItemLoaderCandidate)> = providers
            .iter()
            .filter_map(|provider| Self::best_candidate(provider).map(|candidate| (provider.clone(), candidate)))
            .collect();
        let proposed_file_count = provider_candidates
            .iter()
            .filter(|(_, candidate)| {
                candidate.kind == ShareItemLoaderCandidateKind::File || candidate.r#type.conforms_to(&UTType::file_url())
            })
            .count();
        if Self::count(proposed_file_count) > self.configuration.maximum_file_attachment_count {
            return Err(ShareExtensionError::TooManyItems(self.configuration.maximum_file_attachment_count).into());
        }

        let mut attachments: Vec<ShareAttachment> =
            Vec::with_capacity(provider_candidates.len() + extension_items.len());
        for (index, (provider, candidate)) in provider_candidates.into_iter().enumerate() {
            let source_index = Self::count(index);
            match candidate.kind {
                ShareItemLoaderCandidateKind::File => {
                    let loaded = self.load_file(executor.clone(), &provider, &candidate).await?;
                    attachments.push(Self::file_attachment(loaded, source_index));
                }
                ShareItemLoaderCandidateKind::Url => {
                    let Some(value) = Self::load_string(&provider, &candidate.identifier) else {
                        continue;
                    };
                    let file_url = Url::from_string(&value)
                        .filter(Url::is_file_url)
                        .and_then(|url| url.to_file_path());
                    if let Some(file_url) = file_url {
                        let suggested_name = provider.suggested_name();
                        let directory = self.temporary_file_store.directory_url.clone();
                        let maximum_bytes = self.configuration.maximum_file_byte_count;
                        let r#type = candidate.r#type;
                        let loaded = executor
                            .spawn(async move {
                                Self::copy_source(&file_url, suggested_name.as_deref(), r#type, &directory, maximum_bytes)
                            })
                            .await?;
                        attachments.push(Self::file_attachment(loaded, source_index));
                    } else {
                        let trimmed = trimming_whitespaces_and_newlines(&value);
                        if trimmed.is_empty() {
                            continue;
                        }
                        attachments.push(Self::text_attachment(trimmed, candidate.identifier.clone(), source_index));
                    }
                }
                ShareItemLoaderCandidateKind::Text => {
                    let Some(value) = Self::load_string(&provider, &candidate.identifier) else {
                        continue;
                    };
                    let value = trimming_whitespaces_and_newlines(&value);
                    if value.is_empty() {
                        continue;
                    }
                    attachments.push(Self::text_attachment(value, candidate.identifier.clone(), source_index));
                }
            }
        }

        let mut next_index = Self::count(providers.len());
        let mut existing_texts: HashSet<String> =
            attachments.iter().filter_map(|attachment| attachment.text.clone()).collect();
        for item in &extension_items {
            let value = item
                .attributed_content_text
                .as_deref()
                .map(trimming_whitespaces_and_newlines)
                .unwrap_or("");
            if value.is_empty() || existing_texts.contains(value) {
                continue;
            }
            let is_web_url = Self::is_web_url(value);
            attachments.push(ShareAttachment::new(
                Uuid::new_v4(),
                if is_web_url { ShareAttachmentKind::Link } else { ShareAttachmentKind::Text },
                UTType::plain_text().identifier(),
                TEXT_PLAIN_MIME_TYPE.to_owned(),
                None,
                None,
                Some(value.to_owned()),
                None,
                next_index,
            ));
            existing_texts.insert(value.to_owned());
            next_index += 1;
        }

        if attachments.is_empty() {
            return Err(ShareExtensionError::UnsupportedContent.into());
        }
        let file_backed_count = attachments.iter().filter(|attachment| attachment.is_file_backed()).count();
        if Self::count(file_backed_count) > self.configuration.maximum_file_attachment_count {
            return Err(ShareExtensionError::TooManyItems(self.configuration.maximum_file_attachment_count).into());
        }

        attachments.sort_by_key(|attachment| attachment.source_index);
        let payload = SharePayload {
            session_id: Uuid::new_v4(),
            session_directory_url: self.temporary_file_store.directory_url.clone(),
            attachments,
        };
        if Self::count(payload.textual_content().chars().count()) > self.configuration.maximum_content_length {
            return Err(ShareExtensionError::ContentTooLong(self.configuration.maximum_content_length).into());
        }
        Ok(payload)
    }

    fn count(value: usize) -> i64 {
        i64::try_from(value).unwrap_or(i64::MAX)
    }

    fn file_attachment(loaded: ShareItemLoaderLoadedFile, source_index: i64) -> ShareAttachment {
        ShareAttachment::new(
            Uuid::new_v4(),
            loaded.kind,
            loaded.type_identifier,
            loaded.mime_type,
            Some(loaded.name),
            Some(loaded.url),
            None,
            Some(loaded.byte_count),
            source_index,
        )
    }

    fn text_attachment(value: &str, content_type_identifier: String, source_index: i64) -> ShareAttachment {
        let is_web_url = Self::is_web_url(value);
        ShareAttachment::new(
            Uuid::new_v4(),
            if is_web_url { ShareAttachmentKind::Link } else { ShareAttachmentKind::Text },
            content_type_identifier,
            if is_web_url { TEXT_URI_LIST_MIME_TYPE } else { TEXT_PLAIN_MIME_TYPE }.to_owned(),
            None,
            None,
            Some(value.to_owned()),
            None,
            source_index,
        )
    }

    fn best_candidate(provider: &ShareItemProvider) -> Option<ShareItemLoaderCandidate> {
        let registered: Vec<(String, UTType)> = provider
            .registered_type_identifiers()
            .into_iter()
            .filter_map(|identifier| UTType::from_identifier(&identifier).map(|r#type| (identifier, r#type)))
            .collect();

        let first_conforming = |r#type: UTType, kind: ShareItemLoaderCandidateKind| {
            registered
                .iter()
                .find(|(_, registered_type)| registered_type.conforms_to(&r#type))
                .map(|(identifier, registered_type)| ShareItemLoaderCandidate {
                    kind,
                    r#type: *registered_type,
                    identifier: identifier.clone(),
                })
        };
        let matching = |r#type: UTType| first_conforming(r#type, ShareItemLoaderCandidateKind::File);

        matching(UTType::movie())
            .or_else(|| matching(UTType::image()))
            .or_else(|| matching(UTType::audio()))
            .or_else(|| first_conforming(UTType::file_url(), ShareItemLoaderCandidateKind::Url))
            .or_else(|| first_conforming(UTType::url(), ShareItemLoaderCandidateKind::Url))
            .or_else(|| first_conforming(UTType::plain_text(), ShareItemLoaderCandidateKind::Text))
            .or_else(|| matching(UTType::content()))
            .or_else(|| matching(UTType::data()))
    }

    async fn load_file(
        &self,
        executor: BackgroundExecutor,
        provider: &ShareItemProvider,
        candidate: &ShareItemLoaderCandidate,
    ) -> anyhow::Result<ShareItemLoaderLoadedFile> {
        let ShareItemProvider::File { path, .. } = provider else {
            return Err(ShareExtensionError::FileUnavailable.into());
        };
        let source = path.clone();
        let directory = self.temporary_file_store.directory_url.clone();
        let maximum_bytes = self.configuration.maximum_file_byte_count;
        let suggested_name = provider.suggested_name();
        let r#type = candidate.r#type;
        executor
            .spawn(async move { Self::copy_source(&source, suggested_name.as_deref(), r#type, &directory, maximum_bytes) })
            .await
    }

    /// a file answers its file url, text answers itself
    fn load_string(provider: &ShareItemProvider, _type_identifier: &str) -> Option<String> {
        match provider {
            ShareItemProvider::File { path, .. } => {
                Url::from_file_path(path).map(|url| url.absolute_string().to_owned())
            }
            ShareItemProvider::Text(text) => Some(text.clone()),
        }
    }

    /// copies the source file into the directory with a size cap, blocking file io
    fn copy_source(
        source: &Path,
        suggested_name: Option<&str>,
        r#type: UTType,
        directory: &Path,
        maximum_bytes: i64,
    ) -> anyhow::Result<ShareItemLoaderLoadedFile> {
        let _access = SecurityScopedAccess::start(source);

        let attributes = FileManager.attributes_of_item(source)?;
        if attributes.r#type != FileAttributeType::TypeRegular {
            return Err(ShareExtensionError::FileUnavailable.into());
        }
        let source_content_type = Self::content_type_of(source);
        let byte_count = attributes.size;
        if byte_count <= 0 {
            return Err(ShareExtensionError::FileUnavailable.into());
        }
        if byte_count > maximum_bytes {
            return Err(ShareExtensionError::FileTooLarge.into());
        }

        if let Some(capacity) = volume_available_capacity_for_important_usage(directory)
            && capacity < byte_count.saturating_add(FREE_SPACE_RESERVE_BYTES)
        {
            return Err(ShareExtensionError::FileUnavailable.into());
        }

        let preferred_extension = r#type.preferred_filename_extension();
        let source_name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = ShareFileName::safe_name(
            Some(suggested_name.unwrap_or(&source_name)),
            preferred_extension.as_deref(),
        );
        let destination = ShareFileName::unique_url(directory, Some(&name), preferred_extension.as_deref());
        FileManager.copy_item(source, &destination)?;
        let destination_attributes = FileManager.attributes_of_item(&destination)?;
        if destination_attributes.r#type != FileAttributeType::TypeRegular {
            let _ = FileManager.remove_item(&destination);
            return Err(ShareExtensionError::FileUnavailable.into());
        }
        let copied_bytes = destination_attributes.size;
        if copied_bytes != byte_count {
            let _ = FileManager.remove_item(&destination);
            return Err(ShareExtensionError::FileUnavailable.into());
        }

        let inferred_type = source_content_type
            .or_else(|| UTType::from_filename_extension(ShareFileName::path_extension(&name)))
            .unwrap_or_else(|| {
                if r#type.conforms_to(&UTType::file_url()) {
                    UTType::data()
                } else {
                    r#type
                }
            });
        Ok(ShareItemLoaderLoadedFile {
            url: destination,
            name,
            byte_count: copied_bytes,
            kind: Self::attachment_kind(inferred_type),
            type_identifier: inferred_type.identifier(),
            mime_type: inferred_type
                .preferred_mime_type()
                .unwrap_or_else(|| OCTET_STREAM_MIME_TYPE.to_owned()),
        })
    }

    fn content_type_of(source: &Path) -> Option<UTType> {
        source
            .extension()
            .and_then(|value| value.to_str())
            .and_then(UTType::from_filename_extension)
    }

    fn attachment_kind(r#type: UTType) -> ShareAttachmentKind {
        if r#type.conforms_to(&UTType::image()) {
            return ShareAttachmentKind::Image;
        }
        if r#type.conforms_to(&UTType::movie()) {
            return ShareAttachmentKind::Video;
        }
        if r#type.conforms_to(&UTType::audio()) {
            return ShareAttachmentKind::Audio;
        }
        ShareAttachmentKind::File
    }

    fn is_web_url(value: &str) -> bool {
        let Some(url) = Url::from_string(value) else {
            return false;
        };
        let Some(scheme) = url.scheme().map(str::to_lowercase) else {
            return false;
        };
        scheme == "https" || scheme == "http"
    }
}

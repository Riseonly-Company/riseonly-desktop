use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rise_core::foundation::foundation_file_name::host_file_name;
use rise_core::foundation::foundation_file_url::PathFlavor;
use rise_core::foundation::foundation_text::{character_count, character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};

use crate::modules::share_extension::shared::rise_share_container::{RiseShareContainer, RiseShareContainerError};

/// temp file store, an atomic flag makes removal happen exactly once from any thread
pub struct ShareTemporaryFileStore {
    pub directory_url: PathBuf,
    has_removed_directory: AtomicBool,
}

impl ShareTemporaryFileStore {
    /// makes the store, blocking file io so call it off the main thread
    pub fn new() -> Result<Self, RiseShareContainerError> {
        RiseShareContainer::remove_expired_files();
        let directory_url = RiseShareContainer::make_temporary_directory()?;
        let resource_values = URLResourceValues {
            is_excluded_from_backup: Some(true),
        };
        let _ = set_resource_values(&directory_url, &resource_values);
        Ok(Self {
            directory_url,
            has_removed_directory: AtomicBool::new(false),
        })
    }

    pub fn remove_session_files(&self) {
        if self.has_removed_directory.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = FileManager.remove_item(&self.directory_url);
    }
}

pub enum ShareFileName {}

impl ShareFileName {
    const FALLBACK_NAME: &'static str = "Shared item";
    const MAXIMUM_NAME_CHARACTERS: usize = 180;
    const TRIMMED_STEM_CHARACTERS: usize = 150;
    const MAXIMUM_EXTENSION_CHARACTERS: usize = 12;

    pub fn safe_name(suggested_name: Option<&str>, fallback_extension: Option<&str>) -> String {
        let raw = trimming_whitespaces_and_newlines(suggested_name.unwrap_or(""));
        let leaf = Self::last_path_component(raw);
        let sanitized_scalars: String = leaf
            .chars()
            .map(|scalar| {
                if matches!(scalar, '/' | '\\' | ':' | '\0') || u32::from(scalar) < 32 {
                    '_'
                } else {
                    scalar
                }
            })
            .collect();
        let mut sanitized = trimming_whitespaces_and_newlines(&sanitized_scalars).to_owned();
        sanitized = host_file_name(&sanitized, PathFlavor::HOST);
        if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
            sanitized = Self::FALLBACK_NAME.to_owned();
        }
        if character_count(&sanitized) > Self::MAXIMUM_NAME_CHARACTERS {
            let path_extension = Self::path_extension(&sanitized).to_owned();
            let stem = Self::deleting_path_extension(&sanitized).to_owned();
            sanitized = character_prefix(&stem, Self::TRIMMED_STEM_CHARACTERS).to_owned();
            if !path_extension.is_empty() {
                sanitized.push('.');
                sanitized.push_str(character_prefix(&path_extension, Self::MAXIMUM_EXTENSION_CHARACTERS));
            }
        }
        if Self::path_extension(&sanitized).is_empty() {
            if let Some(fallback_extension) = fallback_extension.filter(|value| !value.is_empty()) {
                sanitized.push('.');
                sanitized.push_str(fallback_extension);
            }
        }
        sanitized
    }

    pub fn unique_url(directory: &Path, suggested_name: Option<&str>, fallback_extension: Option<&str>) -> PathBuf {
        let name = Self::safe_name(suggested_name, fallback_extension);
        directory.join(format!("{}-{}", Self::lowercased_uuid(), name))
    }

    fn lowercased_uuid() -> String {
        Uuid::new_v4().uuid_string().to_lowercase()
    }

    /// part after the last slash, trailing slashes ignored
    fn last_path_component(value: &str) -> &str {
        let trimmed = value.trim_end_matches('/');
        if trimmed.is_empty() {
            return if value.is_empty() { "" } else { "/" };
        }
        match trimmed.rfind('/') {
            Some(index) => &trimmed[index + 1..],
            None => trimmed,
        }
    }

    /// text after the last dot, empty if none or if it has a space
    pub fn path_extension(name: &str) -> &str {
        match name.rfind('.') {
            Some(index) if index > 0 && index + 1 < name.len() => {
                let candidate = &name[index + 1..];
                if candidate.contains(' ') { "" } else { candidate }
            }
            _ => "",
        }
    }

    pub fn deleting_path_extension(name: &str) -> &str {
        let path_extension = Self::path_extension(name);
        if path_extension.is_empty() {
            return name;
        }
        &name[..name.len() - path_extension.len() - 1]
    }
}

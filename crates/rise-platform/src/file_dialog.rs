//! Native open and save dialogs, and the file-type filtering gpui does not do.
//!
//! No backend hangs allowed types on the panel, so a request for "images only" still shows
//! the user everything and the filter is applied to the paths that come back. A pick
//! therefore carries its rejects rather than discarding them silently.

use std::future::Future;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileDialogError {
    #[error("the file dialog went away before it answered")]
    Abandoned,
    #[error("the system file dialog failed: {0}")]
    System(String),
}

/// Extensions a request will accept, matched against what the dialog returns.
///
/// An empty extension list means "anything": [`FileFilter::any`] accepts a file with
/// no extension at all, while every named filter rejects it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileFilter {
    label: String,
    extensions: Vec<String>,
}

impl FileFilter {
    /// Entries are normalised, so `".PNG"`, `"PNG"` and `"png"` are one filter. An entry
    /// may itself contain a dot (`"tar.gz"`), which then has to match as a whole suffix.
    pub fn new<E: AsRef<str>>(
        label: impl Into<String>,
        extensions: impl IntoIterator<Item = E>,
    ) -> Self {
        let mut normalised: Vec<String> = Vec::new();

        for entry in extensions {
            let trimmed = entry.as_ref().trim().trim_matches('.');
            if trimmed.is_empty() {
                continue;
            }

            let extension = trimmed.to_ascii_lowercase();
            if !normalised.contains(&extension) {
                normalised.push(extension);
            }
        }

        Self {
            label: label.into(),
            extensions: normalised,
        }
    }

    pub fn any() -> Self {
        Self {
            label: "All files".into(),
            extensions: Vec::new(),
        }
    }

    pub fn images() -> Self {
        Self::new(
            "Images",
            ["jpg", "jpeg", "png", "heic", "heif", "gif", "webp"],
        )
    }

    pub fn video() -> Self {
        Self::new("Video", ["mp4", "mov", "m4v", "qt", "webm"])
    }

    pub fn audio() -> Self {
        Self::new("Audio", ["mp3", "m4a", "aac", "flac", "ogg", "wav"])
    }

    /// Images and video together, which is narrower than "any file".
    pub fn media() -> Self {
        let mut extensions = Self::images().extensions;
        extensions.extend(Self::video().extensions);

        Self {
            label: "Media".into(),
            extensions,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    pub fn accepts_everything(&self) -> bool {
        self.extensions.is_empty()
    }

    /// The extension used to complete a save name the user left bare: the first one listed.
    pub fn default_extension(&self) -> Option<&str> {
        self.extensions.first().map(String::as_str)
    }

    /// Matches the final dot-separated suffix, so `gz` takes `archive.tar.gz` while `png`
    /// refuses `photo.png.exe`. A leading-dot name (`.gitignore`) has no extension, as
    /// `Path::extension` reads it, and a name that is not valid UTF-8 is refused.
    pub fn accepts(&self, path: &Path) -> bool {
        if self.accepts_everything() {
            return true;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let name = name.to_ascii_lowercase();

        self.extensions
            .iter()
            .any(|extension| ends_with_extension(&name, extension))
    }

    /// Input order is preserved in both halves.
    pub fn partition(&self, paths: impl IntoIterator<Item = PathBuf>) -> FilteredSelection {
        let mut selection = FilteredSelection::default();

        for path in paths {
            if self.accepts(&path) {
                selection.accepted.push(path);
            } else {
                selection.rejected.push(path);
            }
        }

        selection
    }

    /// Appends [`Self::default_extension`] to a name the filter rejects.
    ///
    /// A name that already satisfies the filter keeps its own spelling and case; one that
    /// does not gains the extension rather than losing what the user typed.
    pub fn complete_extension(&self, path: PathBuf) -> PathBuf {
        if self.accepts(&path) {
            return path;
        }

        let Some(extension) = self.default_extension() else {
            return path;
        };

        let completed = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("{name}.{extension}"));

        match completed {
            Some(completed) => path.with_file_name(completed),
            None => path,
        }
    }
}

fn ends_with_extension(name: &str, extension: &str) -> bool {
    // Strictly longer (the +1 is the dot) so `.gz` is not read as a gz file with an empty stem.
    name.len() > extension.len() + 1
        && name.ends_with(extension)
        && name.as_bytes()[name.len() - extension.len() - 1] == b'.'
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FilteredSelection {
    pub accepted: Vec<PathBuf>,
    pub rejected: Vec<PathBuf>,
}

impl FilteredSelection {
    pub fn total(&self) -> usize {
        self.accepted.len() + self.rejected.len()
    }

    pub fn has_rejections(&self) -> bool {
        !self.rejected.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionTarget {
    Files,
    Directories,
    FilesOrDirectories,
}

impl SelectionTarget {
    pub fn allows_files(self) -> bool {
        matches!(self, Self::Files | Self::FilesOrDirectories)
    }

    pub fn allows_directories(self) -> bool {
        matches!(self, Self::Directories | Self::FilesOrDirectories)
    }

    /// What this target becomes once the host's dialog is taken into account.
    ///
    /// `FilesOrDirectories` drops to `Files` off macOS, where asking for directories sets
    /// `FOS_PICKFOLDERS` (or the portal's `directory`) and hides every file.
    pub fn resolve(self, host: HostOs) -> Self {
        match self {
            Self::FilesOrDirectories if !supports_mixed_selection(host) => Self::Files,
            resolved => resolved,
        }
    }
}

/// Whether one dialog can offer files and directories at once. True only on macOS,
/// whose open panel has independent `canChooseFiles` and `canChooseDirectories` flags.
pub fn supports_mixed_selection(host: HostOs) -> bool {
    match host {
        HostOs::MacOs => true,
        HostOs::Windows | HostOs::Linux => false,
    }
}

/// Whether the host's dialog can hide the files a filter excludes.
///
/// Unsupported everywhere: a screen that wants the user to see only images has to say
/// so in its own wording, because the picker will not.
pub fn dialog_enforces_filter(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Windows | HostOs::Linux => PlatformSupport::Unsupported,
    }
}

/// What to ask the system picker for.
///
/// Built through the constructors so a directory request cannot carry a named filter:
/// a directory has no extension, so the filter would reject every folder chosen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenRequest {
    target: SelectionTarget,
    multiple: bool,
    prompt: Option<String>,
    filter: FileFilter,
}

impl OpenRequest {
    pub fn files(filter: FileFilter) -> Self {
        Self {
            target: SelectionTarget::Files,
            multiple: false,
            prompt: None,
            filter,
        }
    }

    pub fn directories() -> Self {
        Self {
            target: SelectionTarget::Directories,
            multiple: false,
            prompt: None,
            filter: FileFilter::any(),
        }
    }

    pub fn files_or_directories(filter: FileFilter) -> Self {
        Self {
            target: SelectionTarget::FilesOrDirectories,
            multiple: false,
            prompt: None,
            filter,
        }
    }

    pub fn allowing_multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// The label on the confirm button, not a window title.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    pub fn target(&self) -> SelectionTarget {
        self.target
    }

    pub fn is_multiple(&self) -> bool {
        self.multiple
    }

    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    pub fn filter(&self) -> &FileFilter {
        &self.filter
    }

    fn prompt_options(&self, host: HostOs) -> gpui::PathPromptOptions {
        let target = self.target.resolve(host);

        gpui::PathPromptOptions {
            files: target.allows_files(),
            directories: target.allows_directories(),
            multiple: self.multiple,
            prompt: self.prompt.clone().map(gpui::SharedString::from),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SaveRequest {
    pub directory: PathBuf,
    pub suggested_name: Option<String>,
    pub filter: FileFilter,
}

impl SaveRequest {
    pub fn new(directory: impl Into<PathBuf>, filter: FileFilter) -> Self {
        Self {
            directory: directory.into(),
            suggested_name: None,
            filter,
        }
    }

    pub fn with_suggested_name(mut self, name: impl Into<String>) -> Self {
        self.suggested_name = Some(name.into());
        self
    }
}

/// The three ways an open dialog ends. A failure is not a cancel: a dialog that never
/// opened still owes the user a message.
#[derive(Debug, PartialEq, Eq)]
pub enum Selection {
    Picked(FilteredSelection),
    Cancelled,
    Failed(FileDialogError),
}

impl Selection {
    /// `None` and an empty list both mean cancel — the backends disagree about which
    /// they send. A selection in which every path fails the filter is still a pick.
    pub fn from_paths(paths: Option<Vec<PathBuf>>, filter: &FileFilter) -> Self {
        match paths {
            None => Self::Cancelled,
            Some(paths) if paths.is_empty() => Self::Cancelled,
            Some(paths) => Self::Picked(filter.partition(paths)),
        }
    }

    pub fn picked(&self) -> Option<&FilteredSelection> {
        match self {
            Self::Picked(selection) => Some(selection),
            Self::Cancelled | Self::Failed(_) => None,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Chosen(PathBuf),
    Cancelled,
    Failed(FileDialogError),
}

impl SaveOutcome {
    pub fn from_path(path: Option<PathBuf>, filter: &FileFilter) -> Self {
        match path {
            None => Self::Cancelled,
            Some(path) => Self::Chosen(filter.complete_extension(path)),
        }
    }

    pub fn chosen(&self) -> Option<&Path> {
        match self {
            Self::Chosen(path) => Some(path),
            Self::Cancelled | Self::Failed(_) => None,
        }
    }
}

/// Opens the picker and resolves once the user is done with it.
///
/// The dialog is raised before the future is built, so the future borrows nothing and can
/// be handed to a spawned task. The filter never reaches the panel: the result is
/// partitioned afterwards.
pub fn open(app: &gpui::App, request: &OpenRequest) -> impl Future<Output = Selection> + use<> {
    let options = request.prompt_options(HostOs::current());
    let receiver = app.prompt_for_paths(options);
    let filter = request.filter.clone();

    async move {
        match receiver.await {
            Ok(Ok(paths)) => Selection::from_paths(paths, &filter),
            Ok(Err(error)) => Selection::Failed(FileDialogError::System(error.to_string())),
            Err(_) => Selection::Failed(FileDialogError::Abandoned),
        }
    }
}

/// Opens the save panel and resolves once the user is done with it.
///
/// The chosen path is completed against the filter: a name typed without an extension
/// comes back from every backend exactly as typed.
pub fn save(app: &gpui::App, request: &SaveRequest) -> impl Future<Output = SaveOutcome> + use<> {
    let suggested = request.suggested_name.as_deref();
    let receiver = app.prompt_for_new_path(&request.directory, suggested);
    let filter = request.filter.clone();

    async move {
        match receiver.await {
            Ok(Ok(path)) => SaveOutcome::from_path(path, &filter),
            Ok(Err(error)) => SaveOutcome::Failed(FileDialogError::System(error.to_string())),
            Err(_) => SaveOutcome::Failed(FileDialogError::Abandoned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().copied().map(PathBuf::from).collect()
    }

    #[test]
    fn an_extension_a_camera_wrote_in_capitals_is_still_an_image() {
        let images = FileFilter::images();
        assert!(images.accepts(Path::new("IMG_0001.JPG")));
        assert!(images.accepts(Path::new("IMG_0002.HeIc")));
    }

    #[test]
    fn a_file_with_no_extension_is_refused_by_a_named_filter() {
        assert!(!FileFilter::images().accepts(Path::new("README")));
        assert!(!FileFilter::audio().accepts(Path::new("Makefile")));
    }

    #[test]
    fn a_filter_that_allows_everything_allows_a_file_with_no_extension() {
        let any = FileFilter::any();
        assert!(any.accepts_everything());
        assert!(any.accepts(Path::new("README")));
        assert!(any.accepts(Path::new("photo.jpg")));
        assert!(any.accepts(Path::new("installer.exe")));
    }

    #[test]
    fn a_compound_name_is_matched_by_its_last_extension() {
        let gz = FileFilter::new("Archives", ["gz"]);
        assert!(gz.accepts(Path::new("archive.tar.gz")));

        let tar_gz = FileFilter::new("Archives", ["tar.gz"]);
        assert!(tar_gz.accepts(Path::new("archive.tar.gz")));
        assert!(
            !tar_gz.accepts(Path::new("archive.gz")),
            "an entry with a dot in it has to match as a whole suffix"
        );
    }

    #[test]
    fn a_second_extension_cannot_smuggle_a_program_past_an_image_filter() {
        let images = FileFilter::images();
        assert!(!images.accepts(Path::new("photo.png.exe")));
        assert!(!images.accepts(Path::new("photo.png.")));
    }

    #[test]
    fn a_leading_dot_name_counts_as_having_no_extension() {
        assert!(!FileFilter::images().accepts(Path::new(".png")));

        let git = FileFilter::new("Git", ["gitignore"]);
        assert!(
            !git.accepts(Path::new(".gitignore")),
            "a dotfile is a stem, which is how Path::extension reads it too"
        );
    }

    #[test]
    fn dots_and_case_in_a_filter_entry_are_normalised_away() {
        let filter = FileFilter::new("Images", [".PNG", "png", "  .JpG. "]);
        assert_eq!(filter.extensions().len(), 2, "png was written twice");
        assert_eq!(filter.default_extension(), Some("png"));
        assert!(filter.accepts(Path::new("a.png")));
        assert!(filter.accepts(Path::new("b.JPG")));
    }

    #[test]
    fn a_filter_whose_entries_all_normalise_away_allows_everything() {
        let filter = FileFilter::new("Broken", ["", "  ", "."]);
        assert!(filter.accepts_everything());
        assert!(filter.default_extension().is_none());
        assert!(
            filter.accepts(Path::new("anything")),
            "a filter nobody can satisfy would lock the user out of the dialog"
        );
    }

    #[test]
    fn the_named_filters_cover_what_the_reference_media_picker_opens() {
        let images = FileFilter::images();
        let video = FileFilter::video();
        let audio = FileFilter::audio();

        assert!(images.accepts(Path::new("a.webp")));
        assert!(images.accepts(Path::new("a.heif")));
        assert!(video.accepts(Path::new("a.webm")));
        assert!(video.accepts(Path::new("a.mov")));
        assert!(audio.accepts(Path::new("a.flac")));
        assert!(audio.accepts(Path::new("a.m4a")));

        assert!(!images.accepts(Path::new("a.mp4")));
        assert!(!video.accepts(Path::new("a.jpg")));
        assert!(
            !audio.accepts(Path::new("a.mp4")),
            "mp4 is the video container here, m4a is the audio one"
        );

        assert_eq!(images.label(), "Images");
    }

    #[test]
    fn media_is_images_and_video_but_not_every_file() {
        let media = FileFilter::media();
        assert!(media.accepts(Path::new("a.jpg")));
        assert!(media.accepts(Path::new("a.mp4")));
        assert!(!media.accepts(Path::new("a.pdf")));
        assert!(!media.accepts(Path::new("a.mp3")));
    }

    #[test]
    fn a_mixed_selection_is_split_in_the_order_the_user_chose_it() {
        let chosen = paths(&["a.png", "b.pdf", "c.JPG", "d.txt", "e.zip"]);
        let selection = FileFilter::images().partition(chosen);

        assert_eq!(selection.accepted, paths(&["a.png", "c.JPG"]));
        assert_eq!(selection.rejected, paths(&["b.pdf", "d.txt", "e.zip"]));
        assert_eq!(selection.total(), 5);
        assert!(selection.has_rejections());
        assert!(!selection.is_empty());
    }

    #[test]
    fn a_selection_where_nothing_qualifies_is_a_pick_not_a_cancel() {
        let chosen = paths(&["a.pdf", "b.txt"]);
        let selection = Selection::from_paths(Some(chosen), &FileFilter::images());

        let picked = selection.picked().expect("the user did choose files");
        assert!(picked.accepted.is_empty());
        assert_eq!(picked.rejected.len(), 2, "say which two failed");
        assert!(!selection.is_cancelled());
    }

    #[test]
    fn nothing_and_an_empty_list_both_read_as_a_cancel() {
        let filter = FileFilter::any();
        let empty = Selection::from_paths(Some(Vec::new()), &filter);

        assert!(Selection::from_paths(None, &filter).is_cancelled());
        assert!(empty.is_cancelled());
    }

    #[test]
    fn a_failed_dialog_is_never_mistaken_for_a_cancel_or_a_pick() {
        let failed = Selection::Failed(FileDialogError::Abandoned);
        assert!(!failed.is_cancelled(), "a failure needs its own message");
        assert!(failed.picked().is_none());

        let dead = SaveOutcome::Failed(FileDialogError::System("no portal".into()));
        assert!(dead.chosen().is_none());
        assert_ne!(dead, SaveOutcome::Cancelled);
    }

    #[test]
    fn only_macos_can_offer_files_and_directories_in_one_dialog() {
        assert!(supports_mixed_selection(HostOs::MacOs));
        assert!(!supports_mixed_selection(HostOs::Windows));
        assert!(!supports_mixed_selection(HostOs::Linux));
    }

    #[test]
    fn a_mixed_request_drops_to_files_where_the_dialog_cannot_do_both() {
        for host in HostOs::ALL {
            let resolved = SelectionTarget::FilesOrDirectories.resolve(host);
            let expected = if host == HostOs::MacOs {
                SelectionTarget::FilesOrDirectories
            } else {
                SelectionTarget::Files
            };
            assert_eq!(resolved, expected, "on {host:?}");
        }
    }

    #[test]
    fn a_single_purpose_request_is_never_rewritten_for_any_host() {
        let files = SelectionTarget::Files;
        let directories = SelectionTarget::Directories;

        for host in HostOs::ALL {
            assert_eq!(files.resolve(host), files, "on {host:?}");
            assert_eq!(
                directories.resolve(host),
                directories,
                "asking for a folder must still ask for a folder on {host:?}"
            );
        }
    }

    #[test]
    fn asking_for_a_file_never_asks_the_picker_for_a_folder() {
        let request = OpenRequest::files(FileFilter::images());

        for host in HostOs::ALL {
            let options = request.prompt_options(host);

            assert!(options.files);
            assert!(
                !options.directories,
                "that flag becomes FOS_PICKFOLDERS and hides every file on {host:?}"
            );
        }
    }

    #[test]
    fn a_mixed_request_only_asks_for_folders_where_that_does_not_hide_files() {
        let request = OpenRequest::files_or_directories(FileFilter::any());

        for host in HostOs::ALL {
            let options = request.prompt_options(host);

            assert!(options.files);
            assert_eq!(options.directories, host == HostOs::MacOs, "on {host:?}");
        }
    }

    #[test]
    fn a_directory_request_asks_only_for_directories() {
        let request = OpenRequest::directories();
        assert_eq!(request.target(), SelectionTarget::Directories);

        for host in HostOs::ALL {
            let options = request.prompt_options(host);
            assert!(options.directories, "on {host:?}");
            assert!(!options.files, "on {host:?}");
        }
    }

    #[test]
    fn a_directory_request_cannot_carry_a_file_filter() {
        assert!(
            OpenRequest::directories().filter().accepts_everything(),
            "a folder has no extension, so a named filter would reject the pick"
        );
    }

    #[test]
    fn the_multiple_flag_and_the_prompt_label_reach_gpui() {
        let request = OpenRequest::files(FileFilter::media())
            .allowing_multiple(true)
            .with_prompt("Attach");
        let options = request.prompt_options(HostOs::current());
        let label = options.prompt.as_ref().map(gpui::SharedString::as_str);

        assert!(options.multiple);
        assert_eq!(label, Some("Attach"));
        assert!(request.is_multiple());
        assert_eq!(request.prompt(), Some("Attach"));
    }

    #[test]
    fn a_request_selects_one_path_unless_it_was_told_otherwise() {
        let request = OpenRequest::files(FileFilter::any());
        assert!(!request.is_multiple());
        assert!(!request.prompt_options(HostOs::current()).multiple);
        assert!(request.prompt().is_none());
    }

    #[test]
    fn no_backend_narrows_the_dialog_itself() {
        for host in HostOs::ALL {
            assert!(
                !dialog_enforces_filter(host).is_supported(),
                "a screen promising 'images only' has to say so itself on {host:?}"
            );
        }
    }

    #[test]
    fn a_save_name_typed_without_an_extension_gains_the_filters_default() {
        let filter = FileFilter::new("Images", ["png", "jpg"]);
        let typed = Some(PathBuf::from("/a/holiday"));
        let outcome = SaveOutcome::from_path(typed, &filter);

        assert_eq!(outcome.chosen(), Some(Path::new("/a/holiday.png")));
    }

    #[test]
    fn a_save_name_that_already_matches_keeps_its_own_case() {
        let images = FileFilter::images();
        let upper = images.complete_extension(PathBuf::from("/a/HOLIDAY.PNG"));
        let jpeg = images.complete_extension(PathBuf::from("/a/holiday.jpeg"));

        assert_eq!(upper, PathBuf::from("/a/HOLIDAY.PNG"));
        assert_eq!(jpeg, PathBuf::from("/a/holiday.jpeg"));
    }

    #[test]
    fn a_save_name_with_the_wrong_extension_keeps_what_the_user_typed() {
        let typed = PathBuf::from("/a/notes.txt");
        let completed = FileFilter::images().complete_extension(typed);

        assert_eq!(
            completed,
            PathBuf::from("/a/notes.txt.jpg"),
            "renaming notes.txt to notes.jpg would lose which file it was"
        );
    }

    #[test]
    fn a_save_with_no_filter_writes_exactly_what_was_typed() {
        let typed = PathBuf::from("/a/notes");
        let completed = FileFilter::any().complete_extension(typed);

        assert_eq!(completed, PathBuf::from("/a/notes"));
    }

    #[test]
    fn completing_a_name_never_touches_the_directory_it_sits_in() {
        let bare = PathBuf::from("/a/b.c/track");
        let completed = FileFilter::audio().complete_extension(bare);

        assert_eq!(completed, PathBuf::from("/a/b.c/track.mp3"));
    }

    #[test]
    fn a_save_dialog_that_was_dismissed_yields_no_path() {
        let outcome = SaveOutcome::from_path(None, &FileFilter::images());

        assert_eq!(outcome, SaveOutcome::Cancelled);
        assert!(outcome.chosen().is_none());
    }

    #[test]
    fn a_save_request_carries_the_name_it_was_given() {
        let bare = SaveRequest::new("/a", FileFilter::images());
        let request = bare.with_suggested_name("holiday");

        assert_eq!(request.directory, PathBuf::from("/a"));
        assert_eq!(request.suggested_name.as_deref(), Some("holiday"));
    }
}
